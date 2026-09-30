// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// The shared vectors in conformance/vectors/ are generated from this
// implementation (scripts/generate-vectors.mjs) and consumed by the Rust and
// C# bindings. This suite fails when the TypeScript behavior drifts from the
// committed vectors, so a behavior change must regenerate them deliberately.
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { canonicalJson, canonicalSha256, isAbsolutePath, normalizePath, parsePurl, pathKeySegments } from '@mxc-prototype/policy-catalog/tooling';
import { readJson } from './helpers.js';

const vectorsDir = new URL('../../../conformance/vectors/', import.meta.url);

describe('shared vectors: canonical JSON', () => {
  const { cases } = readJson(new URL('canonical-json.json', vectorsDir));
  for (const [index, c] of cases.entries()) {
    it(`case ${index}: ${c.json.slice(0, 40)}`, () => {
      const value = JSON.parse(c.json);
      assert.equal(canonicalJson(value), c.canonical);
      assert.equal(canonicalSha256(value), c.sha256);
    });
  }
});

describe('shared vectors: paths', () => {
  const vectors = readJson(new URL('paths.json', vectorsDir));
  for (const platform of ['windows', 'linux', 'macos'] as const) {
    it(`${platform}: isAbsolute, normalize, and case-folded key segments`, () => {
      assert.ok(vectors[platform].length > 5);
      for (const v of vectors[platform]) {
        assert.equal(isAbsolutePath(v.input, platform), v.absolute, v.input);
        if (v.normalized !== null) {
          assert.equal(normalizePath(v.input, platform), v.normalized, v.input);
        }
        assert.deepEqual(pathKeySegments(v.input, platform), v.keySegments, v.input);
      }
    });
  }
});

describe('package URLs', () => {
  it('malformed percent-encoding is an invalid package URL, not an exception', () => {
    assert.equal(parsePurl('pkg:npm/npm@%E0%A4%A'), undefined);
    assert.deepEqual(parsePurl('pkg:npm/npm@10.9.0'), { key: 'npm/npm', version: '10.9.0' });
  });
});
