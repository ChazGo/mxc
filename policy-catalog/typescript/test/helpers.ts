// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { readFileSync, readdirSync } from 'node:fs';
import {
  CatalogStore,
  PolicyCatalog,
  PolicyCatalogError,
  bundledCatalogStore,
  type HostEnvironment,
} from '../dist/index.js';
import { canonicalSha256 } from '../dist/tooling.js';

export const fixturesDir = new URL('../../conformance/fixtures/', import.meta.url);
export const catalogDir = new URL('../../catalog/', import.meta.url);

export function readJson(url: URL): any {
  return JSON.parse(readFileSync(url, 'utf8'));
}

export const contract = readJson(new URL('contract.v1.json', catalogDir));

/** Host with fixed facts, so tests never depend on the machine running them. */
export function fixedHost(
  platform: 'windows' | 'linux' | 'macos' = 'linux',
  architecture: 'x64' | 'arm64' = 'x64',
  symbols: Record<string, string> = {},
): HostEnvironment {
  return {
    platform: () => platform,
    architecture: () => architecture,
    symbol: name => symbols[name],
  };
}

/** Builds an in-memory store whose manifest publishes the given revisions. */
export function storeFor(revisions: any[], options: { defaultRevision?: string; digests?: Record<string, string> } = {}): CatalogStore {
  const files = new Map<string, unknown>();
  const manifest = {
    catalogSchemaVersion: '1',
    defaultRevision: options.defaultRevision ?? revisions[revisions.length - 1].catalogRevision,
    revisions: revisions.map(revision => {
      const file = `revisions/${revision.catalogRevision}.json`;
      files.set(file, revision);
      return {
        catalogRevision: revision.catalogRevision,
        file,
        sha256: options.digests?.[revision.catalogRevision] ?? canonicalSha256(revision),
      };
    }),
  };
  return new CatalogStore({
    contract,
    manifest,
    readRevision: file => structuredClone(files.get(file)),
  });
}

export function catalogFor(revision: any, host: HostEnvironment = fixedHost()): PolicyCatalog {
  return new PolicyCatalog(storeFor([revision]), host);
}

export function bundledCatalog(host: HostEnvironment = fixedHost()): PolicyCatalog {
  return new PolicyCatalog(bundledCatalogStore(), host);
}

export function fixtureFiles(): string[] {
  return readdirSync(fixturesDir).filter(name => name.endsWith('.json')).sort();
}

export function errorCategory(fn: () => unknown): string | undefined {
  try {
    fn();
  } catch (error) {
    if (error instanceof PolicyCatalogError) {
      return error.category;
    }
    throw error;
  }
  return undefined;
}

/** Minimal valid revision with one entry, for mutation in validation tests. */
export function revisionWith(entries: any[], catalogRevision = '2000-01-01.1'): any {
  return { catalogSchemaVersion: '1', catalogRevision, entries };
}

export function entry(entryId: string, overrides: Record<string, unknown> = {}): any {
  const name = entryId.split(':')[1];
  return {
    entryId,
    entryRevision: 1,
    displayName: name,
    identity: [{ kind: 'invocation-name', names: [name] }],
    platformVariants: [
      {
        when: { platform: 'linux' },
        sandboxPolicy: { version: '0.9.0-alpha', filesystem: { readwritePaths: ['${project_root}'] } },
      },
    ],
    provenance: { method: 'test', sourceRevision: 'test' },
    ...overrides,
  };
}
