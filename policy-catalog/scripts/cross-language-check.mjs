#!/usr/bin/env node
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Cross-language check: identical inputs must give byte-identical normalized
// JSON output from the TypeScript, Rust, and C# `policy-catalog` CLIs, for
// resolve, inspect, validate, and the error cases.
//
//   npm run build
//   node scripts/cross-language-check.mjs [--no-build] [--only ts,rust,dotnet]
//
// By default the Rust and .NET CLIs are built from rust/ and dotnet/ first
// (release configuration, their own output directories). Override the
// commands with POLICY_CATALOG_RUST_CLI / POLICY_CATALOG_DOTNET_CLI (a path to
// an executable, or to a .dll that is run with `dotnet`).
//
// Normalization, applied identically to every language:
//   - stdout is parsed as JSON and re-serialized with object keys sorted
//     (key order is not part of the contract; everything else is);
//   - absolute paths of the temporary catalog directories are replaced with
//     placeholders;
//   - OS I/O error text after "could not be read: " is truncated, because it
//     comes from each runtime's file API;
//   - `validate` without --catalog reports where each package installed its
//     bundled catalog (npm package dir, Rust source tree, .NET output dir), so
//     that `catalogDir` must end in `catalog` and is then replaced with
//     `<bundled>/catalog`;
//   - for usage errors (exit 2) stdout must be empty and stderr must end with
//     the shared usage line.
// Exit code must match exactly.
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { canonicalSha256 } from '../dist/canonical-json.js';

const root = fileURLToPath(new URL('..', import.meta.url));
const args = process.argv.slice(2);
const noBuild = args.includes('--no-build');
const onlyAt = args.indexOf('--only');
const only = onlyAt >= 0 ? new Set(args[onlyAt + 1].split(',')) : new Set(['ts', 'rust', 'dotnet']);
const exe = process.platform === 'win32' ? '.exe' : '';

function run(file, argv, cwd = root) {
  execFileSync(file, argv, { cwd, stdio: 'inherit' });
}

/** Language -> function(args) => { status, stdout, stderr }. */
const clis = {};
if (only.has('ts')) {
  clis.ts = [process.execPath, join(root, 'dist', 'cli.js')];
}
if (only.has('rust')) {
  let cli = process.env.POLICY_CATALOG_RUST_CLI;
  if (!cli) {
    const targetDir = join(root, 'rust', 'target');
    if (!noBuild) {
      run('cargo', ['build', '--release', '--locked', '--bin', 'policy-catalog', '--target-dir', targetDir], join(root, 'rust'));
    }
    cli = join(targetDir, 'release', `policy-catalog${exe}`);
  }
  clis.rust = cli.endsWith('.dll') ? ['dotnet', cli] : [cli];
}
if (only.has('dotnet')) {
  let cli = process.env.POLICY_CATALOG_DOTNET_CLI;
  if (!cli) {
    const project = join(root, 'dotnet', 'Microsoft.Mxc.PolicyCatalog.Cli');
    if (!noBuild) {
      run('dotnet', ['build', join(project, 'Microsoft.Mxc.PolicyCatalog.Cli.csproj'), '-c', 'Release', '--nologo', '-v', 'quiet'], root);
    }
    cli = join(project, 'bin', 'Release', 'net8.0', 'policy-catalog.dll');
  }
  clis.dotnet = cli.endsWith('.dll') ? ['dotnet', cli] : [cli];
}
for (const [lang, [cmd]] of Object.entries(clis)) {
  if (cmd !== 'dotnet' && !existsSync(cmd)) {
    console.error(`${lang}: CLI not found at ${cmd}`);
    process.exit(2);
  }
}

// ---------------------------------------------------------------------------
// Synthetic catalogs (written once, shared by every language)
// ---------------------------------------------------------------------------

const work = mkdtempSync(join(tmpdir(), 'policy-catalog-xlang-'));
const placeholders = new Map();
const contract = readFileSync(join(root, 'catalog', 'contract.v1.json'), 'utf8');

function writeCatalog(name, revisions, { tamper, dropFile } = {}) {
  const dir = join(work, name);
  const files = {};
  const manifest = {
    catalogSchemaVersion: '1',
    defaultRevision: revisions[revisions.length - 1].catalogRevision,
    revisions: revisions.map(r => {
      const file = `revisions/${r.catalogRevision}.json`;
      files[file] = r;
      return { catalogRevision: r.catalogRevision, file, sha256: canonicalSha256(r) };
    }),
  };
  mkdirSync(join(dir, 'revisions'), { recursive: true });
  writeFileSync(join(dir, 'contract.v1.json'), contract);
  writeFileSync(join(dir, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
  for (const [file, value] of Object.entries(files)) {
    if (file === dropFile) continue;
    const body = tamper ? tamper(structuredClone(value)) : value;
    writeFileSync(join(dir, file), `${JSON.stringify(body, null, 2)}\n`);
  }
  placeholders.set(dir, `<${name}>`);
  // Runtimes may report the same directory in another spelling (on Windows,
  // an 8.3 short temp path such as RUNNER~1 versus its long form).
  const real = realpathSync.native(dir);
  if (real !== dir) placeholders.set(real, `<${name}>`);
  return dir;
}

const entry = (entryId, overrides = {}) => {
  const name = entryId.split(':')[1];
  return {
    entryId,
    entryRevision: 1,
    displayName: name,
    identity: [{ kind: 'invocation-name', names: [name] }],
    platformVariants: [{ when: { platform: 'linux' }, sandboxPolicy: { version: '0.9.0-alpha', filesystem: { readonlyPaths: [`\${git_prefix}/${name}`] } } }],
    provenance: { method: 'xlang', sourceRevision: 'xlang' },
    ...overrides,
  };
};
const revision = (entries, catalogRevision = '2000-01-01.1') => ({ catalogSchemaVersion: '1', catalogRevision, entries });
const dependsOn = (id, dep) => entry(id, {
  platformVariants: [{ when: { platform: 'linux' }, dependencies: [{ entryId: dep }], sandboxPolicy: { version: '0.9.0-alpha' } }],
});

const bundledRevision = JSON.parse(readFileSync(join(root, 'catalog', 'revisions', '2026-09-29.1.json'), 'utf8'));
const good = writeCatalog('good', [revision([entry('tool:a'), entry('tool:b')]), revision([entry('tool:a', { entryRevision: 2, displayName: 'a2' }), entry('tool:b')], '2000-01-02.1')]);
const tampered = writeCatalog('tampered', [bundledRevision], { tamper: r => { r.entries[0].displayName = 'Tampered'; return r; } });
const missing = writeCatalog('missing', [bundledRevision], { dropFile: 'revisions/2026-09-29.1.json' });
const cyclic = writeCatalog('cyclic', [revision([dependsOn('tool:a', 'tool:b'), dependsOn('tool:b', 'tool:c'), dependsOn('tool:c', 'tool:a')])]);
const multi = writeCatalog('multi', [revision([
  entry('tool:app', { identity: [{ kind: 'purl', value: 'pkg:npm/app', versionRange: '>=2 <3' }, { kind: 'invocation-name', names: ['app'] }] }),
  entry('tool:app-plugin', { identity: [{ kind: 'invocation-name', names: ['app'] }] }),
  entry('tool:net', {
    identity: [{ kind: 'invocation-name', names: ['net'] }],
    platformVariants: [{ when: { platform: 'linux' }, sandboxPolicy: { version: '0.9.0-alpha', network: { egress: { default: 'deny', allow: [{ to: [{ cidr: '192.0.2.0/24' }] }] } } } }],
  }),
  entry('tool:arm', {
    identity: [{ kind: 'invocation-name', names: ['arm'] }],
    platformVariants: [
      { when: { platform: 'linux', architecture: 'arm64' }, sandboxPolicy: { version: '0.9.0-alpha', filesystem: { readonlyPaths: ['${git_prefix}/arm64'] } } },
      { when: { platform: 'windows' }, sandboxPolicy: { version: '0.9.0-alpha', filesystem: { readonlyPaths: ['${git_prefix}\\neutral'] } } },
    ],
  }),
])]);

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

const syms = {
  windows: ['--project-root', 'C:\\work\\app', '--symbol', 'git_prefix=C:\\tools', '--symbol', 'node_prefix=C:\\tools', '--symbol', 'npm_prefix=C:\\tools', '--symbol', 'npm_cache=C:\\cache\\npm'],
  linux: ['--project-root', '/work/app', '--symbol', 'git_prefix=/opt/tools', '--symbol', 'node_prefix=/opt/tools', '--symbol', 'npm_prefix=/opt/tools', '--symbol', 'npm_cache=/var/cache/npm'],
  macos: ['--project-root', '/Users/dev/app', '--symbol', 'git_prefix=/opt/homebrew/bin', '--symbol', 'node_prefix=/opt/homebrew/bin', '--symbol', 'npm_prefix=/opt/homebrew/bin', '--symbol', 'npm_cache=/Users/dev/.npm'],
};
const cases = [
  ['inspect'],
  ['inspect', '--catalog', good],
  ['validate'],
  ['validate', '--catalog', good],
  ['validate', '--catalog', tampered],
  ['validate', '--catalog', missing],
  ['validate', '--catalog', cyclic],
  ['inspect', '--catalog', tampered],
  ['inspect', '--catalog', cyclic],
];
for (const platform of ['windows', 'linux', 'macos']) {
  for (const architecture of ['x64', 'arm64']) {
    const ctx = ['--platform', platform, '--architecture', architecture, '--allow-weak', ...syms[platform]];
    cases.push(['resolve', ...ctx, 'git', 'npm', 'node']);
    cases.push(['resolve', '--diagnostics', ...ctx, 'git', 'npm', 'node', 'cargo']);
    cases.push(['resolve', '--diagnostics', ...ctx, 'GIT', 'Npm.CMD']);
  }
}
const linux = ['--platform', 'linux', '--architecture', 'x64'];
cases.push(
  // weak opt-in off / on
  ['resolve', '--diagnostics', ...linux, ...syms.linux, 'npm'],
  ['resolve', ...linux, ...syms.linux, 'npm'],
  ['resolve', '--diagnostics', ...linux, '--allow-weak', ...syms.linux, 'npm'],
  // strong identity and version-range warnings
  ['resolve', '--diagnostics', ...linux, ...syms.linux, '--purl', 'pkg:npm/npm@10.9.0', 'npx'],
  ['resolve', '--diagnostics', ...linux, ...syms.linux, '--purl', 'pkg:npm/npm', '--detected-version', 'v11.0.0-rc.1', 'npm'],
  // unknown tool, empty input, unresolved symbol
  ['resolve', '--diagnostics', ...linux, '--allow-weak', ...syms.linux, 'no-such-tool'],
  ['resolve', ...linux],
  ['resolve', '--diagnostics', ...linux],
  ['resolve', '--diagnostics', ...linux, '--allow-weak', '--symbol', 'git_prefix=/usr/bin', 'git'],
  // casing: Linux exact, Windows/macOS fold
  ['resolve', '--diagnostics', ...linux, '--allow-weak', ...syms.linux, 'GIT'],
  ['resolve', '--diagnostics', '--platform', 'macos', '--architecture', 'arm64', '--allow-weak', '--project-root', '/Tools/w', '--symbol', 'git_prefix=/tools', 'Git'],
  ['resolve', '--diagnostics', '--platform', 'windows', '--architecture', 'x64', '--allow-weak', '--project-root', 'c:\\tools', '--symbol', 'git_prefix=C:\\Tools\\', 'git'],
  // revisions
  ['resolve', '--diagnostics', '--catalog', good, ...linux, '--allow-weak', '--symbol', 'git_prefix=/g', 'a'],
  ['resolve', '--diagnostics', '--catalog', good, '--revision', '2000-01-01.1', ...linux, '--allow-weak', '--symbol', 'git_prefix=/g', 'a'],
  ['resolve', ...linux, '--revision', '2099-01-01.1', 'git'],
  // integrity and cycles
  ['resolve', '--catalog', tampered, ...linux, 'git'],
  ['resolve', '--catalog', missing, ...linux, 'git'],
  ['resolve', '--catalog', cyclic, ...linux, '--allow-weak', 'a'],
  // multi-match, composition conflicts, architecture
  ['resolve', '--diagnostics', '--catalog', multi, ...linux, '--allow-weak', '--symbol', 'git_prefix=/g', 'app'],
  ['resolve', '--diagnostics', '--catalog', multi, ...linux, '--allow-weak', '--symbol', 'git_prefix=/g', '--purl', 'pkg:npm/app@3.1.0', 'app'],
  ['resolve', '--catalog', multi, ...linux, '--allow-weak', '--symbol', 'git_prefix=/g', 'net', 'app'],
  ['resolve', ...linux, '--allow-weak', '--project-root', '/opt', '--symbol', 'git_prefix=/usr/bin', '--symbol', 'node_prefix=/opt/node', 'git', 'node'],
  ['resolve', '--diagnostics', '--catalog', multi, ...linux, '--allow-weak', '--symbol', 'git_prefix=/g', 'arm'],
  ['resolve', '--diagnostics', '--catalog', multi, '--platform', 'linux', '--architecture', 'arm64', '--allow-weak', '--symbol', 'git_prefix=/g', 'arm'],
  ['resolve', '--diagnostics', '--catalog', multi, '--platform', 'windows', '--architecture', 'arm64', '--allow-weak', '--symbol', 'git_prefix=C:\\g', 'arm'],
  // invalid context (malformed_request)
  ['resolve', '--platform', 'plan9', 'git'],
  ['resolve', '--architecture', 'mips', 'git'],
  ['resolve', ...linux, '--allow-weak', '--symbol', 'node_prefix=relative', 'node'],
  ['resolve', ...linux, '--symbol', 'unknown_symbol=/x', 'git'],
  ['resolve', ...linux, '--symbol', '__proto__=/x', 'git'],
  ['resolve', ...linux, '--symbol', 'project_root=/x', 'git'],
  ['resolve', ...linux, '--purl', 'not-a-purl', 'npm'],
  ['resolve', ...linux, '--purl', 'pkg:npm/npm@%E0%A4%A', 'npm'],
  ['resolve', ...linux, './bin/git'],
  // usage errors
  [],
  ['bogus'],
  ['info'],
  ['resolve', '--platform'],
  ['resolve', '--symbol', 'noequals', 'git'],
  ['resolve', '--purl', 'pkg:npm/npm'],
  ['resolve', '--base-ref', 'HEAD', 'git'],
  ['resolve', '--nope'],
  ['inspect', 'extra'],
  ['validate', 'extra'],
  ['validate', '--base-ref'],
  ['resolve', '--catalog'],
);

// ---------------------------------------------------------------------------
// Run and compare
// ---------------------------------------------------------------------------

function sortKeys(value) {
  if (Array.isArray(value)) return value.map(sortKeys);
  if (value !== null && typeof value === 'object') {
    return Object.fromEntries(Object.keys(value).sort().map(key => [key, sortKeys(value[key])]));
  }
  return value;
}

function normalizeText(text) {
  let out = text;
  for (const [dir, name] of placeholders) {
    for (const form of [dir, dir.replaceAll('\\', '/'), JSON.stringify(dir).slice(1, -1)]) {
      out = out.split(form).join(name);
    }
  }
  return out;
}

function normalize(result, argv) {
  const status = result.status;
  if (status === 2) {
    const lines = result.stderr.trim().split(/\r?\n/);
    return {
      status,
      stdout: result.stdout,
      stderrUsage: lines[lines.length - 1],
      stderrMessage: lines[0].startsWith('policy-catalog: ') ? lines[0] : `(bad prefix) ${lines[0]}`,
    };
  }
  let json;
  try {
    json = JSON.parse(result.stdout);
  } catch {
    return { status, stdout: `(not JSON) ${result.stdout}`, stderr: result.stderr };
  }
  if (argv[0] === 'validate' && !argv.includes('--catalog') && typeof json?.catalogDir === 'string') {
    json.catalogDir = /[\\/]catalog$/.test(json.catalogDir) ? '<bundled>/catalog' : `(unexpected) ${json.catalogDir}`;
  }
  const text = normalizeText(JSON.stringify(sortKeys(json), null, 2))
    .replace(/(could not be read: )[^"]*/g, '$1<io-error>');
  return { status, stdout: text };
}

const languages = Object.keys(clis);
let failures = 0;
let compared = 0;
try {
  for (const argv of cases) {
    const results = {};
    for (const lang of languages) {
      const [cmd, ...pre] = clis[lang];
      const raw = spawnSync(cmd, [...pre, ...argv], { cwd: work, encoding: 'utf8' });
      if (raw.error) throw raw.error;
      results[lang] = JSON.stringify(normalize(raw, argv), null, 2);
    }
    compared += 1;
    const reference = results[languages[0]];
    const differing = languages.filter(lang => results[lang] !== reference);
    const label = normalizeText(argv.join(' ')) || '(no arguments)';
    if (differing.length === 0) {
      console.log(`ok   ${label}`);
    } else {
      failures += 1;
      console.log(`FAIL ${label}`);
      for (const lang of languages) console.log(`  --- ${lang}\n${results[lang].replace(/^/gm, '  ')}`);
    }
  }
} finally {
  rmSync(work, { recursive: true, force: true });
}
console.log(`\ncross-language check: ${compared} cases x ${languages.length} languages (${languages.join(', ')}), ${compared - failures} identical, ${failures} different`);
process.exit(failures === 0 ? 0 : 1);
