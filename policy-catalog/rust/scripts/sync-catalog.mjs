#!/usr/bin/env node
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Copies the catalog source of truth (../../catalog, i.e. policy-catalog/catalog)
// into the crate-local rust/catalog/ so `cargo package` can ship it inside
// the .crate. rust/catalog/ is git-ignored and listed in Cargo.toml `include`.
// The repository build never reads it: build.rs embeds ../catalog directly
// whenever it is present and the crate is not a packaged copy.
//
//   node scripts/sync-catalog.mjs
import { cpSync, existsSync, rmSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const source = fileURLToPath(new URL('../../catalog/', import.meta.url));
const target = fileURLToPath(new URL('../catalog/', import.meta.url));

if (!existsSync(`${source}manifest.json`)) {
  console.error(`sync-catalog: ${source} has no manifest.json`);
  process.exit(1);
}
rmSync(target, { recursive: true, force: true });
cpSync(source, target, {
  recursive: true,
  filter: path => !/\.(md|txt)$/i.test(path),
});
console.log(`sync-catalog: copied ${source} -> ${target}`);
