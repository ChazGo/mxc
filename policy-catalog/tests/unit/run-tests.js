// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Unit test runner. Mirrors MXC's sdk/node/tests/integration/run-tests.js:
// runs the compiled *.test.js files in this directory's dist/ with the spec
// reporter, and fails when there is nothing to run.
//
//   npm run test:unit
import { readdirSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const dist = fileURLToPath(new URL('./dist/', import.meta.url));
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

try {
  execFileSync(process.execPath, ['--test', '--test-reporter', 'spec', '--test-force-exit', ...files], { stdio: 'inherit' });
} catch (error) {
  process.exit(typeof error.status === 'number' && error.status !== 0 ? error.status : 1);
}
