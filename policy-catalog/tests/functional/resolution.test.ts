// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Functional coverage of the resolution behaviors the design calls out
// (design 4.3, 4.4, 4.5, 5.1): additive multi-entry matching, weak-identity
// opt-in, unknown tools, composition conflicts, unavailable revisions,
// neutral-variant fallback, and the architecture-not-verified warning. Each
// case runs through the installed CLI and, where the injected host matters,
// through the installed library.
import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  ARCHITECTURES,
  PLATFORMS,
  PLATFORM_CONTEXT,
  cli,
  entry,
  fullContext,
  lib,
  revision,
  symbolArgs,
  warningsOf,
  writeCatalog,
  type Platform,
} from './helpers.js';

const NOT_VERIFIED = /architecture was not specified; variants were selected for the native system architecture '(x64|arm64)'; the tool's architecture was not verified/;

let work: string;
before(() => {
  work = mkdtempSync(join(tmpdir(), 'policy-catalog-functional-resolution-'));
});
after(() => {
  rmSync(work, { recursive: true, force: true });
});

describe('one input matching several entries', () => {
  // 'app' is claimed by two entries: tool:app (purl + name) and tool:app-plugin (name only).
  const catalog = () => writeCatalog(mkdtempSync(join(work, 'multi-')), [revision([
    entry('tool:app', {
      identity: [{ kind: 'purl', value: 'pkg:npm/app' }, { kind: 'invocation-name', names: ['app'] }],
    }),
    entry('tool:app-plugin', {
      identity: [{ kind: 'invocation-name', names: ['app'] }],
    }),
  ])]);

  it('composes every eligible match and warns, naming the input and the entries', () => {
    const dir = catalog();
    const result = cli('resolve', '--catalog', dir, '--diagnostics', '--platform', 'linux', '--architecture', 'x64',
      '--allow-weak', '--symbol', 'git_prefix=/opt', 'app');
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(result.json.diagnostics.tools, [{
      inputIndex: 0,
      matches: [
        { entryId: 'tool:app', entryRevision: 1, matchedIdentities: [{ kind: 'invocation-name', strength: 'weak' }] },
        { entryId: 'tool:app-plugin', entryRevision: 1, matchedIdentities: [{ kind: 'invocation-name', strength: 'weak' }] },
      ],
    }]);
    assert.match(warningsOf(result), /input 0 \('app'\) matched 2 entries \(tool:app, tool:app-plugin\); all contribute/);
    assert.deepEqual(result.json.policy.filesystem.readonlyPaths, ['/opt/app', '/opt/app-plugin']);
  });

  it('a strong match does not suppress the other entry\'s eligible weak match', () => {
    const dir = catalog();
    const result = cli('resolve', '--catalog', dir, '--diagnostics', '--platform', 'linux', '--architecture', 'x64',
      '--allow-weak', '--symbol', 'git_prefix=/opt', '--purl', 'pkg:npm/app@1.0.0', 'app');
    assert.equal(result.status, 0, result.stderr);
    const [app, plugin] = result.json.diagnostics.tools[0].matches;
    assert.deepEqual(app.matchedIdentities.map((i: any) => i.strength), ['strong', 'weak']);
    assert.equal(plugin.entryId, 'tool:app-plugin');
    assert.match(warningsOf(result), /matched 2 entries/);
  });

  it('without the weak opt-in only the strongly identified entry contributes and no multi-match warning is issued', () => {
    const dir = catalog();
    const result = cli('resolve', '--catalog', dir, '--diagnostics', '--platform', 'linux', '--architecture', 'x64',
      '--symbol', 'git_prefix=/opt', '--purl', 'pkg:npm/app', 'app');
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(result.json.diagnostics.tools[0].matches.map((m: any) => m.entryId), ['tool:app']);
    assert.doesNotMatch(warningsOf(result), /matched 2 entries/);
    assert.deepEqual(result.json.policy.filesystem.readonlyPaths, ['/opt/app']);
  });
});

describe('weak-identity opt-in', () => {
  const base = ['--diagnostics', '--platform', 'linux', '--architecture', 'x64', ...symbolArgs('linux')];

  it('off (default): an invocation-name-only match yields no policy and says why', () => {
    const result = cli('resolve', ...base, 'git');
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.json.policy, undefined);
    assert.deepEqual(result.json.diagnostics.tools, [{ inputIndex: 0, matches: [] }]);
    assert.match(warningsOf(result), /input 0 \('git'\) matched no eligible catalog entry: tool:git matched only by invocation name and allowWeakIdentityFallback is not enabled/);
    assert.equal(cli('resolve', '--platform', 'linux', '--architecture', 'x64', ...symbolArgs('linux'), 'git').json, null);
  });

  it('on: the same match contributes and is flagged as weak identity', () => {
    const result = cli('resolve', ...base, '--allow-weak', 'git');
    assert.equal(result.status, 0, result.stderr);
    assert.notEqual(result.json.policy, undefined);
    assert.deepEqual(result.json.diagnostics.tools[0].matches[0].matchedIdentities, [{ kind: 'invocation-name', strength: 'weak' }]);
    assert.match(warningsOf(result), /input 0 \('git'\) matched tool:git only by invocation name \(weak identity\)/);
  });

  it('a strong package-URL match does not need the opt-in and is not flagged as weak', () => {
    const result = cli('resolve', ...base, '--purl', 'pkg:npm/npm@10.9.0', 'npm');
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(result.json.diagnostics.tools[0].matches[0].matchedIdentities.map((i: any) => i.strength), ['strong', 'weak']);
    assert.notEqual(result.json.policy, undefined);
    assert.doesNotMatch(warningsOf(result), /weak identity/);
  });

  it('string shorthand and object input obey the same option in the installed library', () => {
    const catalog = new lib.PolicyCatalog(lib.bundledCatalogStore());
    const ctx = { platform: 'linux' as const, architecture: 'x64' as const, projectRoot: '/p', symbols: { git_prefix: '/usr/bin' } };
    assert.equal(catalog.resolveSandboxPolicy('git', ctx), undefined);
    assert.equal(catalog.resolveSandboxPolicy({ invocationName: 'git' }, ctx), undefined);
    const on = { ...ctx, allowWeakIdentityFallback: true };
    assert.deepEqual(catalog.resolveSandboxPolicy('git', on), catalog.resolveSandboxPolicy({ invocationName: 'git' }, on));
    assert.notEqual(catalog.resolveSandboxPolicy('git', on), undefined);
  });
});

describe('unknown tools', () => {
  it('an unknown tool yields no policy, never an empty policy, with a per-input warning', () => {
    const result = cli('resolve', '--diagnostics', ...fullContext('linux', 'x64'), 'cargo');
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.json.policy, undefined);
    assert.deepEqual(result.json.diagnostics.tools, [{ inputIndex: 0, matches: [] }]);
    assert.match(warningsOf(result), /input 0 \('cargo'\) matched no eligible catalog entry$/m);
    assert.equal(cli('resolve', ...fullContext('linux', 'x64'), 'cargo').json, null);
  });

  it('known and unknown inputs compose the known requirements and report the unknown one', () => {
    const mixed = cli('resolve', '--diagnostics', ...fullContext('linux', 'x64'), 'cargo', 'git');
    const known = cli('resolve', '--diagnostics', ...fullContext('linux', 'x64'), 'git');
    assert.equal(mixed.status, 0, mixed.stderr);
    assert.deepEqual(mixed.json.policy, known.json.policy);
    assert.deepEqual(mixed.json.diagnostics.tools.map((t: any) => [t.inputIndex, t.matches.length]), [[0, 0], [1, 1]]);
    assert.match(warningsOf(mixed), /input 0 \('cargo'\) matched no eligible catalog entry/);
  });
});

describe('composition conflicts', () => {
  it('overlapping resolved paths across access classes are a policy_validation (composition_conflict) failure, not a choice', () => {
    // project_root=/opt (read-write for git) contains node_prefix=/opt/node (read-only for node).
    const result = cli('resolve', '--platform', 'linux', '--architecture', 'x64', '--allow-weak', '--project-root', '/opt',
      '--symbol', 'git_prefix=/usr/bin', '--symbol', 'node_prefix=/opt/node', 'git', 'node');
    assert.equal(result.status, 1, result.stderr);
    assert.equal(result.json.error.details.reason, 'composition_conflict');
    assert.match(result.json.error.message, /overlap across access classes/);
  });

  it('two independently matched entries that would need a network merge are rejected', () => {
    const dir = writeCatalog(mkdtempSync(join(work, 'compose-')), [revision([
      entry('tool:a'),
      entry('tool:b', {
        platformVariants: [{
          when: { platform: 'linux' },
          sandboxPolicy: { version: '0.9.0-alpha', network: { egress: { default: 'deny' } } },
        }],
      }),
    ])]);
    // Each entry alone is valid; the conflict exists only for this input set.
    assert.equal(cli('validate', '--catalog', dir).status, 0);
    const alone = cli('resolve', '--catalog', dir, '--platform', 'linux', '--architecture', 'x64', '--allow-weak', 'b');
    assert.equal(alone.status, 0, alone.stderr);
    assert.deepEqual(alone.json.network, { egress: { default: 'deny' } });
    const both = cli('resolve', '--catalog', dir, '--platform', 'linux', '--architecture', 'x64', '--allow-weak', '--symbol', 'git_prefix=/opt', 'a', 'b');
    assert.equal(both.status, 1, both.stderr);
    assert.equal(both.json.error.details.reason, 'composition_conflict');
    assert.match(both.json.error.message, /'tool:b' uses 'network', which has no v1 cross-entry composition rule/);
  });

  it('mixed sandboxPolicy versions across selected entries are rejected', () => {
    const dir = writeCatalog(mkdtempSync(join(work, 'versions-')), [revision([
      entry('tool:a'),
      entry('tool:b', {
        platformVariants: [{ when: { platform: 'linux' }, sandboxPolicy: { version: '0.8.0-alpha', filesystem: { readonlyPaths: ['${git_prefix}/b'] } } }],
      }),
    ])]);
    const both = cli('resolve', '--catalog', dir, '--platform', 'linux', '--architecture', 'x64', '--allow-weak', '--symbol', 'git_prefix=/opt', 'a', 'b');
    assert.equal(both.status, 1, both.stderr);
    assert.equal(both.json.error.details.reason, 'composition_conflict');
    assert.match(both.json.error.message, /mixed sandboxPolicy\.version values/);
  });
});

describe('catalog revisions', () => {
  it('an unavailable explicit revision is a backend_error (revision_unavailable) failure, never a substitution', () => {
    const result = cli('resolve', '--revision', '1999-01-01.1', ...fullContext('linux', 'x64'), 'git');
    assert.equal(result.status, 1, result.stderr);
    assert.equal(result.json.error.details.reason, 'revision_unavailable');
    assert.match(result.json.error.message, /'1999-01-01\.1' is not installed/);
    assert.throws(
      () => lib.resolveSandboxPolicy('git', { catalogRevision: '1999-01-01.1', platform: 'linux', architecture: 'x64' }),
      (error: any) => error instanceof lib.PolicyCatalogError && error.reason === 'revision_unavailable',
    );
  });

  it('an older installed revision stays addressable after a newer default is published', () => {
    const r1 = revision([entry('tool:a')], '2000-01-01.1');
    const r2 = revision([entry('tool:a', {
      entryRevision: 2,
      platformVariants: [{ when: { platform: 'linux' }, sandboxPolicy: { version: '0.9.0-alpha', filesystem: { readonlyPaths: ['${git_prefix}/v2'] } } }],
    })], '2000-01-02.1');
    const dir = writeCatalog(mkdtempSync(join(work, 'revisions-')), [r1, r2]);
    assert.equal(cli('validate', '--catalog', dir).status, 0);
    const args = ['--catalog', dir, '--diagnostics', '--platform', 'linux', '--architecture', 'x64', '--allow-weak', '--symbol', 'git_prefix=/opt', 'a'];
    const latest = cli('resolve', ...args);
    const older = cli('resolve', '--revision', '2000-01-01.1', ...args);
    assert.equal(latest.json.diagnostics.catalogRevision, '2000-01-02.1');
    assert.deepEqual(latest.json.policy.filesystem.readonlyPaths, ['/opt/v2']);
    assert.equal(older.json.diagnostics.catalogRevision, '2000-01-01.1');
    assert.deepEqual(older.json.policy.filesystem.readonlyPaths, ['/opt/a']);
    const missing = cli('resolve', '--revision', '2000-01-03.1', ...args);
    assert.equal(missing.json.error.details.reason, 'revision_unavailable');
  });
});

describe('platform and architecture selection', () => {
  for (const platform of PLATFORMS) {
    for (const architecture of ARCHITECTURES) {
      it(`${platform}/${architecture}: explicit architecture warns about neutral fallback but not about verification`, () => {
        const result = cli('resolve', '--diagnostics', ...fullContext(platform, architecture), 'git');
        assert.equal(result.status, 0, result.stderr);
        assert.match(warningsOf(result), new RegExp(`tool:git uses its architecture-neutral ${platform} variant; no ${architecture}-specific variant exists`));
        assert.doesNotMatch(warningsOf(result), NOT_VERIFIED);
      });

      it(`${platform}/${architecture}: omitted architecture uses the injected native architecture and warns that the tool's was not verified`, () => {
        // The installed library with a host whose native architecture is fixed,
        // so every combination is exercised on any CI machine.
        const host = { platform: (): Platform => platform, nativeArchitecture: () => architecture, symbol: () => undefined };
        const catalog = new lib.PolicyCatalog(lib.bundledCatalogStore(), host);
        const c = PLATFORM_CONTEXT[platform];
        const ctx = { allowWeakIdentityFallback: true, projectRoot: c.root, symbols: { git_prefix: c.prefix } };
        const omitted = catalog.resolveSandboxPolicyWithDiagnostics('git', ctx);
        const warnings = omitted.diagnostics.warnings.join('\n');
        assert.match(warnings, NOT_VERIFIED);
        assert.match(warnings, new RegExp(`native system architecture '${architecture}'`));
        assert.match(warnings, new RegExp(`no ${architecture}-specific variant exists`));
        const explicit = catalog.resolveSandboxPolicyWithDiagnostics('git', { ...ctx, architecture, platform });
        assert.doesNotMatch(explicit.diagnostics.warnings.join('\n'), NOT_VERIFIED);
        assert.deepEqual(explicit.policy, omitted.policy);
      });
    }

    it(`${platform}: the installed CLI on this host warns when architecture is omitted`, () => {
      const result = cli('resolve', '--diagnostics', '--platform', platform, '--allow-weak', ...symbolArgs(platform), 'git');
      assert.equal(result.status, 0, result.stderr);
      assert.match(warningsOf(result), NOT_VERIFIED);
    });
  }

  it('no warning about architecture is issued when nothing was selected', () => {
    const result = cli('resolve', '--diagnostics', '--platform', 'linux', 'cargo');
    assert.equal(result.status, 0, result.stderr);
    assert.doesNotMatch(warningsOf(result), /architecture/);
  });

  it('an exact-architecture variant wins and emits no neutral-fallback warning; another architecture is never a fallback', () => {
    const dir = writeCatalog(mkdtempSync(join(work, 'arch-')), [revision([
      entry('tool:a', {
        platformVariants: [
          { when: { platform: 'windows', architecture: 'arm64' }, sandboxPolicy: { version: '0.9.0-alpha', filesystem: { readonlyPaths: ['${git_prefix}\\arm64'] } } },
          { when: { platform: 'windows', architecture: 'x64' }, sandboxPolicy: { version: '0.9.0-alpha', filesystem: { readonlyPaths: ['${git_prefix}\\x64'] } } },
          { when: { platform: 'linux', architecture: 'x64' }, sandboxPolicy: { version: '0.9.0-alpha', filesystem: { readonlyPaths: ['${git_prefix}/x64'] } } },
        ],
      }),
    ])]);
    const common = ['--catalog', dir, '--diagnostics', '--allow-weak'];
    for (const architecture of ARCHITECTURES) {
      const result = cli('resolve', ...common, '--platform', 'windows', '--architecture', architecture, '--symbol', 'git_prefix=C:\\t', 'a');
      assert.equal(result.status, 0, result.stderr);
      assert.deepEqual(result.json.policy.filesystem.readonlyPaths, [`C:\\t\\${architecture}`]);
      assert.doesNotMatch(warningsOf(result), /architecture-neutral/);
    }
    const noArm = cli('resolve', ...common, '--platform', 'linux', '--architecture', 'arm64', '--symbol', 'git_prefix=/t', 'a');
    assert.equal(noArm.status, 0, noArm.stderr);
    assert.equal(noArm.json.policy, undefined);
    assert.match(warningsOf(noArm), /tool:a has no variant for linux\/arm64/);
  });
});
