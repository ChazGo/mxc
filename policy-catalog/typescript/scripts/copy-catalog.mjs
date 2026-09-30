// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Bundles the repository's catalog data into the package (`<pkg>/catalog/`),
// so the library resolves locally without network access (spec §6.1).
import { cpSync, rmSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const source = fileURLToPath(new URL('../../catalog/', import.meta.url));
const target = fileURLToPath(new URL('../catalog/', import.meta.url));
rmSync(target, { recursive: true, force: true });
cpSync(source, target, { recursive: true });
