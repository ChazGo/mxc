// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Runs the language-neutral conformance fixtures (conformance/fixtures/*.json).
// Other language bindings run the same files with the same expectations.
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { bundledCatalog, catalogFor, errorCategory, fixtureFiles, fixturesDir, readJson } from './helpers.js';

for (const name of fixtureFiles()) {
  const fixture = readJson(new URL(name, fixturesDir));
  describe(`conformance: ${name}`, () => {
    const catalog = fixture.catalog === 'bundled' ? bundledCatalog() : catalogFor(fixture.catalog);
    for (const testCase of fixture.cases) {
      it(testCase.name, () => {
        if (testCase.expectError !== undefined) {
          assert.equal(errorCategory(() => catalog.resolveCatalogEntry(testCase.tool, testCase.context)), testCase.expectError);
          return;
        }
        const actual = catalog.resolveCatalogEntry(testCase.tool, testCase.context);
        if (testCase.expect === null) {
          assert.equal(actual, undefined);
        } else {
          assert.deepEqual(actual, testCase.expect);
        }
      });
    }
  });
}
