// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Prints the canonical SHA-256 digest of a catalog revision file, for use in
// catalog/manifest.json when publishing a NEW revision. Never recompute the
// digest of an already-published revision; publish a new revision instead.
//
// Usage: node scripts/catalog-digest.mjs ../catalog/revisions/<revision>.json
import { readFileSync } from 'node:fs';
import { canonicalSha256 } from '../dist/tooling.js';

const file = process.argv[2];
if (!file) {
  console.error('usage: node scripts/catalog-digest.mjs <revision.json>');
  process.exit(2);
}
console.log(canonicalSha256(JSON.parse(readFileSync(file, 'utf8'))));
