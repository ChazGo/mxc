// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { copyFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const outputRoot = process.argv[2];
if (!outputRoot) {
  throw new Error('Usage: node scripts/copy-config-floors.mjs <output-root>');
}

const packageRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
const sourceRoot = join(packageRoot, 'src', 'config-floors');
const destinationRoot = join(packageRoot, outputRoot, 'config-floors');
mkdirSync(destinationRoot, { recursive: true });

for (const file of ['catalog.json', 'schema.json']) {
  copyFileSync(join(sourceRoot, file), join(destinationRoot, file));
}
