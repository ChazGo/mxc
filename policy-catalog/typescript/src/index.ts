// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Runtime lookup (spec §5.1) and setup/inspection (spec §5.2) are separate
// entry points. Neither launches a sandbox, contacts a network service, or
// writes consumer state.
export {
  resolveCatalogEntry,
  listCatalogEntries,
  getCatalogInfo,
  PolicyCatalog,
  nodeHostEnvironment,
  type HostEnvironment,
} from './resolver.js';

export {
  CatalogStore,
  loadCatalogDirectory,
  bundledCatalogStore,
  type CatalogSource,
  type CatalogManifest,
  type ManifestRevision,
} from './store.js';

export { PolicyCatalogError, type PolicyCatalogErrorCategory } from './errors.js';

export type {
  CatalogArchitecture,
  CatalogEntryMetadata,
  CatalogIdentityMetadata,
  CatalogInfo,
  CatalogNetworkRule,
  CatalogPlatform,
  CatalogSandboxPolicy,
  ResolveContext,
  ResolvedToolEntry,
  ToolCandidate,
} from './types.js';
