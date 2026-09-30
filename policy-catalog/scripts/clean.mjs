#!/usr/bin/env node
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Removes build output owned by this package only.
import { rmSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

for (const dir of ['dist', 'dist-tests']) {
  rmSync(fileURLToPath(new URL(`../${dir}/`, import.meta.url)), { recursive: true, force: true });
}
