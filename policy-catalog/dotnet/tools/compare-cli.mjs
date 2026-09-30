#!/usr/bin/env node
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Spot comparison of the .NET CLI against the TypeScript CLI (node dist/cli.js).
// Feeds identical arguments to both, then compares exit codes, stderr (usage
// errors), and stdout JSON canonicalized with sorted keys, after replacing the
// absolute catalog directory with a placeholder and truncating OS I/O error
// text after "could not be read: ".
//
//   npm run build   (once, from policy-catalog/)
//   dotnet build dotnet/Microsoft.Mxc.PolicyCatalog.slnx -c Release
//   node dotnet/tools/compare-cli.mjs
import { spawnSync } from 'node:child_process';
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync, mkdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { canonicalJson, canonicalSha256 } from '../../dist/canonical-json.js';

const root = fileURLToPath(new URL('../../', import.meta.url));
const tsCli = join(root, 'dist', 'cli.js');
const csCli = join(root, 'dotnet', 'Microsoft.Mxc.PolicyCatalog.Cli', 'bin', 'Release', 'net8.0', 'policy-catalog.dll');
const work = mkdtempSync(join(tmpdir(), 'policy-catalog-compare-'));

function writeCatalog(dir, revisions) {
  mkdirSync(join(dir, 'revisions'), { recursive: true });
  cpSync(join(root, 'catalog', 'contract.v1.json'), join(dir, 'contract.v1.json'));
  const manifest = { catalogSchemaVersion: '1', defaultRevision: revisions.at(-1).catalogRevision, revisions: [] };
  for (const r of revisions) {
    const file = `revisions/${r.catalogRevision}.json`;
    writeFileSync(join(dir, file), JSON.stringify(r, null, 2));
    manifest.revisions.push({ catalogRevision: r.catalogRevision, file, sha256: canonicalSha256(r) });
  }
  writeFileSync(join(dir, 'manifest.json'), JSON.stringify(manifest, null, 2));
  return dir;
}
const entry = (id, deps = []) => ({
  entryId: id, entryRevision: 1, displayName: id, identity: [{ kind: 'invocation-name', names: [id.split(':')[1]] }],
  platformVariants: [{ when: { platform: 'linux' }, ...(deps.length ? { dependencies: deps.map(entryId => ({ entryId })) } : {}), sandboxPolicy: { version: '0.9.0-alpha', filesystem: { readonlyPaths: [`\${git_prefix}/${id.split(':')[1]}`] } } }],
  provenance: { method: 'cmp', sourceRevision: 'cmp' },
});

const tampered = join(work, 'tampered');
cpSync(join(root, 'catalog'), tampered, { recursive: true });
const revFile = join(tampered, 'revisions', '2026-09-29.1.json');
const data = JSON.parse(readFileSync(revFile, 'utf8'));
data.entries[0].platformVariants[0].sandboxPolicy.filesystem.readwritePaths.push('${user_home}');
writeFileSync(revFile, JSON.stringify(data));
const cyclic = writeCatalog(join(work, 'cyclic'), [{ catalogSchemaVersion: '1', catalogRevision: '2000-01-01.1', entries: [entry('tool:a', ['tool:b']), entry('tool:b', ['tool:c']), entry('tool:c', ['tool:a'])] }]);
const missing = join(work, 'missing');
cpSync(join(root, 'catalog'), missing, { recursive: true });
rmSync(join(missing, 'revisions', '2026-09-29.1.json'));

const ctx = {
  windows: ['--project-root', 'C:\\work\\app', '--symbol', 'git_prefix=C:\\tools', '--symbol', 'node_prefix=C:\\tools', '--symbol', 'npm_prefix=C:\\tools', '--symbol', 'npm_cache=C:\\cache\\npm', '--symbol', 'user_home=C:\\Users\\u', '--symbol', 'temp_dir=C:\\t'],
  linux: ['--project-root', '/work/app', '--symbol', 'git_prefix=/opt/tools', '--symbol', 'node_prefix=/opt/tools', '--symbol', 'npm_prefix=/opt/tools', '--symbol', 'npm_cache=/var/cache/npm', '--symbol', 'user_home=/home/u', '--symbol', 'temp_dir=/tmp'],
  macos: ['--project-root', '/Users/dev/app', '--symbol', 'git_prefix=/opt/homebrew/bin', '--symbol', 'node_prefix=/opt/homebrew/bin', '--symbol', 'npm_prefix=/opt/homebrew/bin', '--symbol', 'npm_cache=/Users/dev/.npm', '--symbol', 'user_home=/Users/dev', '--symbol', 'temp_dir=/tmp'],
};
const cases = [
  ['inspect'], ['validate'], ['validate', '--catalog', join(root, 'catalog')],
  ['validate', '--catalog', tampered], ['inspect', '--catalog', tampered], ['resolve', '--catalog', tampered, '--platform', 'linux', '--architecture', 'x64', '--allow-weak', ...ctx.linux, 'git'],
  ['validate', '--catalog', cyclic], ['resolve', '--catalog', cyclic, '--platform', 'linux', '--architecture', 'x64', '--allow-weak', '--symbol', 'git_prefix=/opt', 'a'],
  ['validate', '--catalog', missing], ['inspect', '--catalog', missing], ['inspect', '--catalog', join(work, 'nope')], ['validate', '--catalog', join(work, 'nope')],
  ['validate', '--base-ref', 'HEAD'], ['validate', '--base-ref', '-h'], ['validate', '--base-ref', 'no-such-ref-xyz'], ['validate', '--catalog', cyclic, '--base-ref', 'HEAD'],
];
for (const platform of ['windows', 'linux', 'macos']) {
  for (const architecture of ['x64', 'arm64']) {
    for (const diag of [[], ['--diagnostics']]) {
      cases.push(['resolve', ...diag, '--platform', platform, '--architecture', architecture, '--allow-weak', ...ctx[platform], 'git', 'npm', 'node']);
      cases.push(['resolve', ...diag, '--platform', platform, '--architecture', architecture, ...ctx[platform], 'git', 'npm', 'node']);
    }
  }
  cases.push(['resolve', '--diagnostics', '--platform', platform, '--allow-weak', ...ctx[platform], 'git', 'npm', 'node']);
}
for (const diag of [[], ['--diagnostics']]) {
  cases.push(
    ['resolve', ...diag, '--platform', 'linux', '--architecture', 'x64', ...ctx.linux, '--purl', 'pkg:npm/npm@11.0.0', 'npm'],
    ['resolve', ...diag, '--platform', 'linux', '--architecture', 'x64', ...ctx.linux, '--purl', 'pkg:npm/npm@11.0.0', '--detected-version', 'nightly', 'npm'],
    ['resolve', ...diag, '--platform', 'linux', '--architecture', 'x64', '--allow-weak', ...ctx.linux, 'cargo'],
    ['resolve', ...diag, '--revision', '2099-01-01.1', '--platform', 'linux', '--architecture', 'x64', 'git'],
    ['resolve', ...diag, '--platform', 'linux', '--architecture', 'mips', 'git'],
    ['resolve', ...diag, '--platform', 'plan9', 'git'],
    ['resolve', ...diag, '--platform', 'linux', '--architecture', 'x64', '--allow-weak', '--symbol', 'node_prefix=bin', 'node'],
    ['resolve', ...diag, '--platform', 'linux', '--architecture', 'x64', '--allow-weak', '--symbol', '__proto__=/x', 'node'],
    ['resolve', ...diag, '--platform', 'linux', '--architecture', 'x64', '--allow-weak', '--symbol', 'project_root=/x', 'git'],
    ['resolve', ...diag, '--platform', 'linux', '--architecture', 'x64', '--allow-weak', '--symbol', 'git_prefix=/g', 'git'],
    ['resolve', ...diag, '--allow-weak', ...ctx.windows, 'git', 'npm', 'node'],
    ['resolve', ...diag, '--platform', 'linux', '--architecture', 'x64', '--allow-weak', '--project-root', '/opt', '--symbol', 'node_prefix=/opt/node', 'git', 'node'],
    ['resolve', ...diag, '--platform', 'linux', '--architecture', 'x64', '--allow-weak', '--project-root', '/opt', '--symbol', 'git_prefix=/usr/bin', '--symbol', 'node_prefix=/opt/node', 'git', 'node'],
    ['resolve', ...diag, '--platform', 'windows', '--architecture', 'x64', '--allow-weak', '--project-root', 'C:\\W', '--symbol', 'git_prefix=c:\\w\\bin', 'GIT.EXE'],
    ['resolve', ...diag, '--platform', 'linux', '--architecture', 'x64', '--allow-weak', ...ctx.linux, 'GIT'],
    ['resolve', ...diag, '--platform', 'linux', '--architecture', 'x64', '--purl', 'not-a-purl', 'npm'],
    ['resolve', ...diag, '--platform', 'linux', '--architecture', 'x64', '--purl', 'pkg:npm/npm@%E0%A4%A', 'npm'],
    ['resolve', ...diag, '--platform', 'linux', '--architecture', 'x64', './bin/git'],
    ['resolve', ...diag, '--platform', 'linux', '--architecture', 'x64'],
  );
}
cases.push(
  [], ['bogus'], ['info'], ['resolve', '--platform'], ['resolve', '--symbol', 'noequals', 'git'], ['resolve', '--symbol', '=x', 'git'],
  ['resolve', '--purl', 'pkg:npm/npm'], ['resolve', '--base-ref', 'HEAD', 'git'], ['inspect', 'extra'], ['validate', 'extra'],
  ['validate', '--base-ref'], ['resolve', '--catalog'], ['resolve', '--platform', '--allow-weak', 'git'], ['resolve', '--bogus'],
  ['inspect', '--base-ref', 'HEAD'],
);

function normalize(stdout, dirs) {
  if (stdout.trim() === '') {
    return '';
  }
  let text = stdout;
  for (const dir of dirs) {
    text = text.split(JSON.stringify(dir).slice(1, -1)).join('<CATALOG>');
  }
  const value = JSON.parse(text);
  const truncate = v => (typeof v === 'string' && v.includes('could not be read: ') ? v.slice(0, v.indexOf('could not be read: ') + 19) : v);
  const walk = v => (Array.isArray(v) ? v.map(walk) : v && typeof v === 'object' ? Object.fromEntries(Object.entries(v).map(([k, x]) => [k, walk(truncate(x))])) : truncate(v));
  return canonicalJson(walk(value));
}

let mismatches = 0;
const bundledDirs = [join(root, 'catalog'), join(root, 'dotnet', 'Microsoft.Mxc.PolicyCatalog.Cli', 'bin', 'Release', 'net8.0', 'catalog')];
for (const args of cases) {
  const ts = spawnSync(process.execPath, [tsCli, ...args], { cwd: root, encoding: 'utf8' });
  const cs = spawnSync('dotnet', [csCli, ...args], { cwd: root, encoding: 'utf8' });
  const dirs = [...bundledDirs, ...args.filter((_, i) => args[i - 1] === '--catalog').map(d => join(d))].map(d => d.replace(/[\\/]+$/, ''));
  const a = { status: ts.status, out: normalize(ts.stdout, dirs), err: ts.stderr };
  const b = { status: cs.status, out: normalize(cs.stdout, dirs), err: cs.stderr.replace(/\r\n/g, '\n') };
  const same = a.status === b.status && a.out === b.out && a.err === b.err;
  if (!same) {
    mismatches += 1;
    console.log(`MISMATCH: ${args.join(' ')}\n  ts: ${a.status} ${a.out} ${a.err}\n  cs: ${b.status} ${b.out} ${b.err}`);
  }
}
console.log(`${cases.length} cases, ${cases.length - mismatches} identical, ${mismatches} mismatched`);
rmSync(work, { recursive: true, force: true });
process.exit(mismatches === 0 ? 0 : 1);
