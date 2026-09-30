// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Package-level integration smoke test (spec §12 "Integration"): packs the
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
  for (const required of ['catalog/manifest.json', 'catalog/contract.v1.json', 'dist/index.js', 'dist/index.d.ts']) {
    if (!files.includes(required)) throw new Error(`package is missing ${required}`);
  }
  if (files.some(f => f.startsWith('src/') || f.startsWith('test/'))) {
    throw new Error('package unexpectedly includes sources or tests');
  }
  const consumer = join(work, 'consumer');
  mkdirSync(consumer);
  writeFileSync(join(consumer, 'package.json'), JSON.stringify({ name: 'consumer', private: true, type: 'module' }));
  run(['install', '--offline', '--no-audit', '--no-fund', tarball], consumer);
  const manifest = JSON.parse(readFileSync(join(packageDir, '..', 'catalog', 'manifest.json'), 'utf8'));
  writeFileSync(join(consumer, 'smoke.mjs'), `
    import { getCatalogInfo, listCatalogEntries, resolveCatalogEntry, PolicyCatalogError } from '@mxc-prototype/policy-catalog';
    import assert from 'node:assert/strict';
    assert.equal(getCatalogInfo().catalogRevision, ${JSON.stringify(manifest.defaultRevision)});
    assert.ok(listCatalogEntries().length > 0);
    assert.equal(resolveCatalogEntry({ invocationName: 'git' }), undefined);
    const r = resolveCatalogEntry({ invocationName: 'git' }, {
      platform: 'linux', architecture: 'x64', allowWeakIdentityFallback: true,
      projectRoot: '/p', symbols: { git_prefix: '/usr/bin' },
    });
    assert.equal(r.entryId, 'tool:git');
    assert.throws(() => resolveCatalogEntry({ invocationName: 'git' }, { catalogRevision: '1999-01-01.1' }), PolicyCatalogError);
    console.log('package smoke ok');
  `);
  process.stdout.write(execFileSync(process.execPath, ['smoke.mjs'], { cwd: consumer, encoding: 'utf8' }));
} finally {
  rmSync(work, { recursive: true, force: true });
}
