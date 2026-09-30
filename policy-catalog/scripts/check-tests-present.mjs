#!/usr/bin/env node
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Fails when a test directory holds no *.test.js files. `node --test <dir>`
// exits 0 when it finds nothing, so a renamed or empty test directory would
// otherwise pass silently. Mirrors MXC's scripts/versioning/check-tests-present.js.
//
//   node scripts/check-tests-present.mjs <compiled-test-dir>
import { existsSync, readdirSync } from 'node:fs';

const dir = process.argv[2];
if (!dir || !existsSync(dir)) {
  console.error(`Test presence check FAILED: '${dir ?? '(none)'}' does not exist.`);
  process.exit(1);
}
// Only regular files count; a directory named `x.test.js` runs nothing.
const files = readdirSync(dir, { withFileTypes: true })
  .filter(entry => entry.isFile() && entry.name.endsWith('.test.js'));
if (files.length === 0) {
  console.error(`Test presence check FAILED: no *.test.js files in '${dir}'.`);
  process.exit(1);
}
console.log(`Test presence check OK: ${files.length} test file(s) in ${dir}.`);
