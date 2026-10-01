// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Runs the language-neutral conformance fixtures (conformance/fixtures/*.json).
// Other language bindings must run the same files with the same expectations.
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { PolicyCatalogError } from '@mxc-prototype/policy-catalog';
import { bundledCatalog, catalogFor, fixedHost, fixtureFiles, fixturesDir, readJson } from './helpers.js';

function failure(fn: () => unknown): { code: string; reason: string } | undefined {
  try {
    fn();
  } catch (error) {
    if (error instanceof PolicyCatalogError) {
      return { code: error.code, reason: error.details.reason };
    }
    throw error;
  }
  return undefined;
}

function toExpected(expect: any): any {
  return { ...expect, policy: expect.policy ?? undefined };
}

for (const name of fixtureFiles()) {
  const fixture = readJson(new URL(name, fixturesDir));
  describe(`conformance: ${name}`, () => {
    for (const testCase of fixture.cases) {
      it(testCase.name, () => {
        const host = fixedHost(testCase.host?.platform ?? 'linux', testCase.host?.nativeArchitecture ?? 'x64');
        const catalog = fixture.catalog === 'bundled' ? bundledCatalog(host) : catalogFor(fixture.catalog, host);
        if (testCase.expectError !== undefined) {
          assert.deepEqual(failure(() => catalog.resolveSandboxPolicyWithDiagnostics(testCase.tools, testCase.context)), testCase.expectError);
          assert.deepEqual(failure(() => catalog.resolveSandboxPolicy(testCase.tools, testCase.context)), testCase.expectError);
          return;
        }
        const expected = toExpected(testCase.expect);
        assert.deepEqual(catalog.resolveSandboxPolicyWithDiagnostics(testCase.tools, testCase.context), expected);
        // The policy-only operation returns the same policy from the same logic.
        assert.deepEqual(catalog.resolveSandboxPolicy(testCase.tools, testCase.context), expected.policy);
        // A single input is exactly a one-element array.
        if (!Array.isArray(testCase.tools)) {
          assert.deepEqual(catalog.resolveSandboxPolicyWithDiagnostics([testCase.tools], testCase.context), expected);
        }
      });
    }
  });
}
