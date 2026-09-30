// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

/**
 * Language-neutral library failure categories. Shared conformance fixtures
 * assert on these values, so every language binding must preserve them.
 * A library failure is never a "no match" result (spec §5).
 */
export type PolicyCatalogErrorCategory =
  /** Bundled data does not match its published integrity digest. */
  | 'integrity'
  /** Catalog, manifest, or contract data violates the catalog contract. */
  | 'validation'
  /** An explicitly requested catalog revision is not installed. */
  | 'revision-unavailable'
  /** The caller supplied an invalid ResolveContext or ToolCandidate. */
  | 'invalid-context'
  /** The host platform/architecture cannot be mapped to a catalog selector. */
  | 'unsupported-host'
  /** Resolved paths would require choosing between access classes. */
  | 'composition-conflict';

export class PolicyCatalogError extends Error {
  readonly category: PolicyCatalogErrorCategory;

  constructor(category: PolicyCatalogErrorCategory, message: string) {
    super(`[${category}] ${message}`);
    this.name = 'PolicyCatalogError';
    this.category = category;
  }
}
