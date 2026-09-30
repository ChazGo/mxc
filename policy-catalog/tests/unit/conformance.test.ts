// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Runs the language-neutral conformance fixtures (conformance/fixtures/*.json).
// Other language bindings must run the same files with the same expectations.
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { bundledCatalog, catalogFor, errorCategory, fixedHost, fixtureFiles, fixturesDir, readJson } from './helpers.js';

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
          assert.equal(errorCategory(() => catalog.getSandboxConfigWithDiagnostics(testCase.tools, testCase.context)), testCase.expectError);
          assert.equal(errorCategory(() => catalog.getSandboxConfig(testCase.tools, testCase.context)), testCase.expectError);
          return;
        }
        const expected = toExpected(testCase.expect);
        assert.deepEqual(catalog.getSandboxConfigWithDiagnostics(testCase.tools, testCase.context), expected);
        // The policy-only operation returns the same policy from the same logic.
        assert.deepEqual(catalog.getSandboxConfig(testCase.tools, testCase.context), expected.policy);
        // A single input is exactly a one-element array.
        if (!Array.isArray(testCase.tools)) {
          assert.deepEqual(catalog.getSandboxConfigWithDiagnostics([testCase.tools], testCase.context), expected);
        }
      });
    }
  });
}
