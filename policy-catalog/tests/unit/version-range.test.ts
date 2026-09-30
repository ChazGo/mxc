// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { isValidVersionRange, parsePurl, satisfiesVersionRange } from '@mxc-prototype/policy-catalog/tooling';

describe('version ranges', () => {
  it('accepts the v1 grammar and rejects everything else', () => {
    for (const range of ['>=10 <12', '22', '>=1.2.3', '=1.0.0', '<2 || >=4']) {
      assert.ok(isValidVersionRange(range), range);
    }
    for (const range of ['', '^1.2.3', '~1', '1.x', '>= 1', '1 ||', 'latest']) {
      assert.ok(!isValidVersionRange(range), range);
    }
  });

  it('evaluates evidence versions and reports incomparable evidence', () => {
    assert.equal(satisfiesVersionRange('10.9.0', '>=10 <12'), true);
    assert.equal(satisfiesVersionRange('v12.0.0', '>=10 <12'), false);
    assert.equal(satisfiesVersionRange('22.3.1', '22'), true);
    assert.equal(satisfiesVersionRange('23.0.0', '22'), false);
    assert.equal(satisfiesVersionRange('3.0.0', '<2 || >=3'), true);
    assert.equal(satisfiesVersionRange('10.0.0-rc.1', '>=10'), true);
    assert.equal(satisfiesVersionRange('nightly', '>=10'), undefined);
  });
});

describe('package URLs', () => {
  it('extracts a version-free key and optional version', () => {
    assert.deepEqual(parsePurl('pkg:npm/npm@10.9.0'), { key: 'npm/npm', version: '10.9.0' });
    assert.deepEqual(parsePurl('pkg:NPM/%40scope/pkg@1.0.0?x=y#sub'), { key: 'npm/%40scope/pkg', version: '1.0.0' });
    assert.deepEqual(parsePurl('pkg:npm/npm'), { key: 'npm/npm' });
    assert.equal(parsePurl('npm/npm'), undefined);
    assert.equal(parsePurl('pkg:npm'), undefined);
  });
});
