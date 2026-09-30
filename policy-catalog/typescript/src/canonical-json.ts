// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { createHash } from 'node:crypto';

/**
 * Canonical JSON used for revision integrity digests: object keys sorted by
 * UTF-16 code unit order at every level, arrays kept in order, and no
 * insignificant whitespace. Formatting-only edits to a revision file therefore
 * do not change its digest, while every semantic edit does.
 */
export function canonicalJson(value: unknown): string {
  if (value === null || typeof value !== 'object') {
    const encoded = JSON.stringify(value);
    if (encoded === undefined) {
      throw new TypeError('canonicalJson: value is not JSON-serializable');
    }
    return encoded;
  }
  if (Array.isArray(value)) {
    return `[${value.map(canonicalJson).join(',')}]`;
  }
  const record = value as Record<string, unknown>;
  const keys = Object.keys(record).sort();
  return `{${keys.map(key => `${JSON.stringify(key)}:${canonicalJson(record[key])}`).join(',')}}`;
}

/** Lower-case hex SHA-256 of the canonical JSON form. */
export function canonicalSha256(value: unknown): string {
  return createHash('sha256').update(canonicalJson(value), 'utf8').digest('hex');
}
