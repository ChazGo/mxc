#!/usr/bin/env node
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Package inclusion gate (design §7): `npm pack --dry-run` must ship the
// compiled library, the CLI, the bundled catalog, and the schemas, and must not
// ship sources, tests, fixtures, or build-only scripts.
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const npmCli = process.env.npm_execpath;
if (!npmCli) {
  console.error('run this script through `npm run check:pack`');
  process.exit(2);
}
const [packed] = JSON.parse(execFileSync(process.execPath, [npmCli, 'pack', '--dry-run', '--json'], { cwd: root, encoding: 'utf8' }));
const files = packed.files.map(file => file.path.replaceAll('\\', '/')).sort();

const manifest = JSON.parse(readFileSync(new URL('../catalog/manifest.json', import.meta.url), 'utf8'));
const required = [
  'LICENSE.md',
  'README.md',
  'package.json',
  'dist/index.js',
  'dist/index.d.ts',
  'dist/tooling.js',
  'dist/tooling.d.ts',
  'dist/cli.js',
  'dist/validate.js',
  'catalog/contract.v1.json',
  'catalog/manifest.json',
  ...manifest.revisions.map(revision => `catalog/${revision.file}`),
  'schema/catalog.v1.schema.json',
  'schema/manifest.v1.schema.json',
];
const forbidden = [/^src\//, /^tests\//, /^conformance\//, /^scripts\//, /^docs\//, /^\.github\//, /^node_modules\//, /\.test\.js$/];

const errors = [
  ...required.filter(file => !files.includes(file)).map(file => `missing ${file}`),
  ...files.filter(file => forbidden.some(pattern => pattern.test(file))).map(file => `unexpected ${file}`),
];
if (errors.length > 0) {
  console.error('Package contents check FAILED:');
  for (const error of errors) console.error(`  - ${error}`);
  process.exit(1);
}
console.log(`Package contents check OK: ${files.length} files, ${packed.size} bytes packed (${packed.name}@${packed.version}).`);
for (const file of files) console.log(`  ${file}`);
