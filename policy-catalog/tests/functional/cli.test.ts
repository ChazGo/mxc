// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Functional (end-to-end) tests. They run the compiled CLI (`dist/cli.js`) as a
// separate process against the real bundled catalog and against tampered or
// invalid copies of it, then assert on exit codes and JSON output. Unlike the
// unit tests, they exercise the shipped entry point, file loading, integrity
// verification, and host detection exactly as a consumer would. Mirrors MXC's
// sdk/node/tests/integration layout (compiled node:test files, spec reporter).
import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

// dist-tests/tests/functional/ -> package root
const packageRoot = fileURLToPath(new URL('../../../', import.meta.url));
const cli = join(packageRoot, 'dist', 'cli.js');
const catalogDir = join(packageRoot, 'catalog');
const manifest = JSON.parse(readFileSync(join(catalogDir, 'manifest.json'), 'utf8'));

interface CliResult {
  status: number | null;
  json: any;
  stderr: string;
}

function run(...args: string[]): CliResult {
  const result = spawnSync(process.execPath, [cli, ...args], { encoding: 'utf8' });
  let json: any;
  try {
    json = result.stdout.trim() === '' ? undefined : JSON.parse(result.stdout);
  } catch {
    json = result.stdout;
  }
  return { status: result.status, json, stderr: result.stderr };
}

const PLATFORM_CONTEXT = {
  windows: { root: 'C:\\work\\app', prefix: 'C:\\tools', cache: 'C:\\cache\\npm' },
  linux: { root: '/work/app', prefix: '/opt/tools', cache: '/var/cache/npm' },
  macos: { root: '/Users/dev/app', prefix: '/opt/homebrew/bin', cache: '/Users/dev/.npm' },
} as const;

function fullContext(platform: keyof typeof PLATFORM_CONTEXT, architecture: string): string[] {
  const c = PLATFORM_CONTEXT[platform];
  return [
    '--platform', platform,
    '--architecture', architecture,
    '--allow-weak',
    '--project-root', c.root,
    '--symbol', `git_prefix=${c.prefix}`,
    '--symbol', `node_prefix=${c.prefix}`,
    '--symbol', `npm_prefix=${c.prefix}`,
    '--symbol', `npm_cache=${c.cache}`,
  ];
}

describe('policy-catalog CLI: bundled catalog', () => {
  it('info reports the installed default revision', () => {
    const result = run('info');
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(result.json, { catalogSchemaVersion: '1', catalogRevision: manifest.defaultRevision });
  });

  it('verify checks integrity and history of every installed revision', () => {
    const result = run('verify');
    assert.equal(result.status, 0, JSON.stringify(result.json));
    assert.equal(result.json.ok, true);
    assert.deepEqual(result.json.revisions, manifest.revisions.map((r: any) => r.catalogRevision));
  });

  it('list returns metadata only, sorted by entryId', () => {
    const result = run('list');
    assert.equal(result.status, 0);
    const ids = result.json.map((e: any) => e.entryId);
    assert.deepEqual(ids, [...ids].sort());
    assert.ok(!JSON.stringify(result.json).includes('readwritePaths'));
  });

  for (const platform of ['windows', 'linux', 'macos'] as const) {
    for (const architecture of ['x64', 'arm64']) {
      it(`resolves the full tool set on ${platform}/${architecture} deterministically`, () => {
        const c = PLATFORM_CONTEXT[platform];
        const first = run('resolve', '--diagnostics', ...fullContext(platform, architecture), 'git', 'npm', 'node');
        const second = run('resolve', '--diagnostics', ...fullContext(platform, architecture), 'git', 'npm', 'node');
        assert.equal(first.status, 0, first.stderr);
        assert.deepEqual(first.json, second.json);
        const { policy, diagnostics } = first.json;
        assert.equal(policy.version, '0.9.0-alpha');
        assert.deepEqual(policy.filesystem.readwritePaths, [c.root, c.cache]);
        assert.deepEqual(policy.filesystem.readonlyPaths, [c.prefix]);
        assert.equal(policy.network, undefined, 'v1 never composes network');
        assert.deepEqual(diagnostics.tools.map((t: any) => t.matches.map((m: any) => m.entryId)), [['tool:git'], ['tool:npm'], ['tool:node']]);
        assert.deepEqual(diagnostics.resolvedDependencies, [{ entryId: 'tool:node', entryRevision: 1 }]);
      });
    }
  }

  it('policy-only output equals the diagnostics policy', () => {
    const ctx = fullContext('linux', 'x64');
    const plain = run('resolve', ...ctx, 'npm');
    const diag = run('resolve', '--diagnostics', ...ctx, 'npm');
    assert.deepEqual(plain.json, diag.json.policy);
  });

  it('strong package-URL identity does not need the weak-fallback opt-in', () => {
    const c = PLATFORM_CONTEXT.linux;
    const result = run('resolve', '--diagnostics', '--platform', 'linux', '--architecture', 'x64', '--project-root', c.root,
      '--symbol', `npm_prefix=${c.prefix}`, '--symbol', `npm_cache=${c.cache}`, '--symbol', `node_prefix=${c.prefix}`,
      '--purl', 'pkg:npm/npm@10.9.0', 'npm');
    assert.equal(result.status, 0, result.stderr);
    // Every satisfied predicate is attributed in declaration order (design §4.3);
    // the strong one makes the entry eligible without the weak-fallback opt-in.
    assert.deepEqual(result.json.diagnostics.tools[0].matches[0].matchedIdentities.map((i: any) => i.strength), ['strong', 'weak']);
    assert.notEqual(result.json.policy, null);
  });

  it('omitted architecture uses the native system architecture and says so', () => {
    const c = PLATFORM_CONTEXT.linux;
    const result = run('resolve', '--diagnostics', '--platform', 'linux', '--allow-weak', '--project-root', c.root, '--symbol', `git_prefix=${c.prefix}`, 'git');
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.json.diagnostics.warnings.join('\n'), /native system architecture '(x64|arm64)'; the tool's architecture was not verified/);
  });

  it('unknown tools, weak-only matches without opt-in, and empty input produce null, never an empty policy', () => {
    assert.equal(run('resolve', '--platform', 'linux', '--architecture', 'x64', '--allow-weak', 'cargo').json, null);
    assert.equal(run('resolve', ...fullContext('linux', 'x64').filter(a => a !== '--allow-weak'), 'git').json, null);
    const empty = run('resolve', '--diagnostics', '--platform', 'linux', '--architecture', 'x64');
    assert.equal(empty.status, 0);
    assert.deepEqual(empty.json, { diagnostics: { catalogRevision: manifest.defaultRevision, tools: [], resolvedDependencies: [], warnings: [] } });
  });

  it('an unresolved required symbol yields null with an actionable warning', () => {
    const result = run('resolve', '--diagnostics', '--platform', 'linux', '--architecture', 'x64', '--allow-weak', '--symbol', 'git_prefix=/usr/bin', 'git');
    assert.equal(result.status, 0);
    assert.equal(result.json.policy, undefined);
    assert.match(result.json.diagnostics.warnings.join('\n'), /required symbol 'project_root'.*supply ResolveContext\.projectRoot/);
  });

  it('library failures exit 1 with a stable error category', () => {
    const cases: Array<[string[], string]> = [
      [['resolve', '--revision', '1999-01-01.1', 'git'], 'revision-unavailable'],
      [['resolve', '--platform', 'plan9', 'git'], 'invalid-context'],
      [['resolve', '--platform', 'linux', '--architecture', 'x64', '--allow-weak', '--symbol', 'node_prefix=relative', 'node'], 'invalid-context'],
      [['resolve', '--platform', 'linux', '--architecture', 'x64', '--symbol', 'unknown_symbol=/x', 'git'], 'invalid-context'],
      [['resolve', '--platform', 'linux', '--architecture', 'x64', '--allow-weak', '--project-root', '/opt', '--symbol', 'git_prefix=/usr/bin', '--symbol', 'node_prefix=/opt/node', 'git', 'node'], 'composition-conflict'],
    ];
    for (const [args, category] of cases) {
      const result = run(...args);
      assert.equal(result.status, 1, `${args.join(' ')} -> ${result.stderr}`);
      assert.equal(result.json.error.category, category, args.join(' '));
    }
  });

  it('usage errors exit 2 without JSON on stdout', () => {
    for (const args of [[], ['bogus'], ['resolve', '--platform'], ['resolve', '--symbol', 'noequals', 'git'], ['resolve', '--purl', 'pkg:npm/npm']]) {
      const result = run(...args);
      assert.equal(result.status, 2, args.join(' '));
      assert.equal(result.json, undefined);
      assert.match(result.stderr, /usage: policy-catalog/);
    }
  });
});

describe('policy-catalog CLI: integrity failure modes on a copied catalog', () => {
  let work: string;
  const copy = (): string => {
    const dir = mkdtempSync(join(work, 'catalog-'));
    cpSync(catalogDir, dir, { recursive: true });
    return dir;
  };
  const revisionFile = (dir: string) => join(dir, manifest.revisions.at(-1).file);

  before(() => {
    work = mkdtempSync(join(tmpdir(), 'policy-catalog-functional-'));
  });
  after(() => {
    rmSync(work, { recursive: true, force: true });
  });

  it('an untouched copy verifies and resolves like the bundled catalog', () => {
    const dir = copy();
    assert.equal(run('verify', '--catalog', dir).status, 0);
    const ctx = fullContext('linux', 'x64');
    assert.deepEqual(run('resolve', '--catalog', dir, ...ctx, 'git').json, run('resolve', ...ctx, 'git').json);
  });

  it('formatting-only edits keep the canonical digest valid', () => {
    const dir = copy();
    const file = revisionFile(dir);
    writeFileSync(file, JSON.stringify(JSON.parse(readFileSync(file, 'utf8'))));
    assert.equal(run('verify', '--catalog', dir).status, 0);
  });

  it('a semantic edit to a published revision fails integrity, never silently resolving', () => {
    const dir = copy();
    const file = revisionFile(dir);
    const data = JSON.parse(readFileSync(file, 'utf8'));
    data.entries[0].platformVariants[0].sandboxPolicy.filesystem.readwritePaths.push('${user_home}');
    writeFileSync(file, JSON.stringify(data, null, 2));
    const verify = run('verify', '--catalog', dir);
    assert.equal(verify.status, 1, verify.stderr);
    assert.ok(verify.json?.errors, `verify produced no JSON report; stderr: ${verify.stderr}`);
    assert.match(verify.json.errors.join('\n'), /\[integrity\].*does not match the published digest/);
    const resolve = run('resolve', '--catalog', dir, ...fullContext('windows', 'x64'), 'git');
    assert.equal(resolve.status, 1);
    assert.equal(resolve.json.error.category, 'integrity');
  });

  it('a missing revision file is an integrity failure', () => {
    const dir = copy();
    rmSync(revisionFile(dir));
    const result = run('info', '--catalog', dir);
    assert.equal(result.status, 1);
    assert.equal(result.json.error.category, 'integrity');
  });

  it('a manifest whose default names an unlisted revision is a validation failure', () => {
    const dir = copy();
    const file = join(dir, 'manifest.json');
    writeFileSync(file, JSON.stringify({ ...manifest, defaultRevision: '2099-01-01.1' }));
    const result = run('info', '--catalog', dir);
    assert.equal(result.status, 1);
    assert.equal(result.json.error.category, 'validation');
  });
});
