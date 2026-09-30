// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Functional test runner. Tests the package a consumer would install, not the
// source tree:
//
//   1. `npm pack` the package (its dist/ must already be built).
//   2. Install the tarball, offline, into a fresh consumer project in a
//      temporary directory outside the repository.
//   3. Run the compiled *.test.js files in this directory's dist/ with
//      POLICY_CATALOG_CONSUMER_DIR pointing at that consumer. The tests load
//      the library and CLI only from the consumer's node_modules.
//
// The final step mirrors MXC's sdk/node/tests/integration/run-tests.js
// (`node --test --test-reporter spec --test-force-exit`).
//
//   npm run test:functional
import { readdirSync, existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const dist = fileURLToPath(new URL('./dist/', import.meta.url));
const packageDir = fileURLToPath(new URL('../../', import.meta.url));

const files = existsSync(dist)
  ? readdirSync(dist, { withFileTypes: true })
    .filter(entry => entry.isFile() && entry.name.endsWith('.test.js'))
    .map(entry => join(dist, entry.name))
    .sort()
  : [];
if (!files.length) {
  console.error(`No test files found in ${dist}`);
  process.exit(1);
}

// Run npm through its JS entry point so no shell is involved on any platform.
const npmCli = process.env.npm_execpath;
if (!npmCli) {
  console.error('Run the functional suite through `npm run test:functional` so npm can pack and install the package.');
  process.exit(2);
}
const npm = (args, cwd) => execFileSync(process.execPath, [npmCli, ...args], { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] });

const work = mkdtempSync(join(tmpdir(), 'policy-catalog-functional-'));
let status = 0;
try {
  const [packed] = JSON.parse(npm(['pack', '--json', '--pack-destination', work], packageDir));
  const tarball = join(work, packed.filename);
  const consumer = join(work, 'consumer');
  mkdirSync(consumer);
  writeFileSync(join(consumer, 'package.json'), `${JSON.stringify({ name: 'policy-catalog-functional-consumer', private: true, type: 'module' }, null, 2)}\n`);
  // The package has no runtime dependencies, so the install needs no network.
  npm(['install', '--offline', '--no-audit', '--no-fund', '--ignore-scripts', tarball], consumer);
  console.log(`Installed ${packed.name}@${packed.version} (${packed.entryCount} files) from ${packed.filename} into ${consumer}\n`);

  try {
    execFileSync(process.execPath, ['--test', '--test-reporter', 'spec', '--test-force-exit', ...files], {
      stdio: 'inherit',
      env: { ...process.env, POLICY_CATALOG_CONSUMER_DIR: consumer },
    });
  } catch (error) {
    status = typeof error.status === 'number' && error.status !== 0 ? error.status : 1;
  }
} finally {
  rmSync(work, { recursive: true, force: true });
}
process.exit(status);
