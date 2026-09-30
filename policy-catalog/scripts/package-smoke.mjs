// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Package-level integration smoke test (design §12 "Integration"): packs the
// library, installs the tarball into an empty temporary project with no
// network access and no MXC SDK or executor, and exercises the public API.
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const packageDir = fileURLToPath(new URL('..', import.meta.url));
// Run npm through its JS entry point so no shell is involved on any platform.
const npmCli = process.env.npm_execpath;
if (!npmCli) {
  throw new Error('run this script through `npm run smoke:package`');
}
const run = (args, cwd) => execFileSync(process.execPath, [npmCli, ...args], { cwd, encoding: 'utf8' });
const work = mkdtempSync(join(tmpdir(), 'policy-catalog-smoke-'));
try {
  const packed = JSON.parse(run(['pack', '--json', '--pack-destination', work], packageDir));
  const tarball = join(work, packed[0].filename);
  const files = packed[0].files.map(f => f.path);
  // Exact package contents are gated by check-pack.mjs; this checks the install.
  if (!files.includes('catalog/manifest.json')) throw new Error('package is missing the bundled catalog');
  const consumer = join(work, 'consumer');
  mkdirSync(consumer);
  writeFileSync(join(consumer, 'package.json'), JSON.stringify({ name: 'consumer', private: true, type: 'module' }));
  run(['install', '--offline', '--no-audit', '--no-fund', tarball], consumer);
  const manifest = JSON.parse(readFileSync(join(packageDir, 'catalog', 'manifest.json'), 'utf8'));
  writeFileSync(join(consumer, 'smoke.mjs'), `
    import { getCatalogInfo, listCatalogEntries, getSandboxConfig, getSandboxConfigWithDiagnostics, PolicyCatalogError } from '@mxc-prototype/policy-catalog';
    import { execFileSync } from 'node:child_process';
    import assert from 'node:assert/strict';
    assert.equal(getCatalogInfo().catalogRevision, ${JSON.stringify(manifest.defaultRevision)});
    assert.ok(listCatalogEntries().length > 0);
    // Weak identity requires opt-in, so a bare name yields no policy.
    assert.equal(getSandboxConfig('git', { platform: 'linux', architecture: 'x64' }), undefined);
    const ctx = { platform: 'linux', architecture: 'x64', allowWeakIdentityFallback: true,
      projectRoot: '/p', symbols: { git_prefix: '/usr/bin', node_prefix: '/n', npm_prefix: '/n', npm_cache: '/c' } };
    assert.deepEqual(getSandboxConfig(['git', 'npm'], ctx).filesystem.readwritePaths, ['/p', '/c']);
    const r = getSandboxConfigWithDiagnostics('git', ctx);
    assert.equal(r.diagnostics.tools[0].matches[0].entryId, 'tool:git');
    assert.throws(() => getSandboxConfig('git', { catalogRevision: '1999-01-01.1' }), PolicyCatalogError);
    // The packaged CLI runs from the installed tree.
    const inspected = JSON.parse(execFileSync(process.execPath, ['node_modules/@mxc-prototype/policy-catalog/dist/cli.js', 'inspect'], { encoding: 'utf8' }));
    assert.equal(inspected.info.catalogRevision, ${JSON.stringify(manifest.defaultRevision)});
    // On POSIX, npm installs the bin as a symlink; running through it must still work.
    if (process.platform !== 'win32') {
      const viaBin = JSON.parse(execFileSync(process.execPath, ['node_modules/.bin/policy-catalog', 'inspect'], { encoding: 'utf8' }));
      assert.deepEqual(viaBin, inspected);
    }
    // validate checks the installed catalog's integrity and contract.
    const validated = JSON.parse(execFileSync(process.execPath, ['node_modules/@mxc-prototype/policy-catalog/dist/cli.js', 'validate'], { encoding: 'utf8' }));
    assert.equal(validated.ok, true);
    console.log('package smoke ok');
  `);
  process.stdout.write(execFileSync(process.execPath, ['smoke.mjs'], { cwd: consumer, encoding: 'utf8' }));
} finally {
  rmSync(work, { recursive: true, force: true });
}
