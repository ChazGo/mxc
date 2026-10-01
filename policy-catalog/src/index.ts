// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Runtime lookup (design §5.1) and setup/inspection (design §5.2) are
// separate entry points. None of them launches a sandbox, contacts a network
// service, or writes consumer state.
export {
  resolveSandboxPolicy,
  resolveSandboxPolicyWithDiagnostics,
  listCatalogEntries,
  getCatalogInfo,
  PolicyCatalog,
} from './resolver.js';

export { nodeHostEnvironment, type HostEnvironment } from './host.js';

export {
  CatalogStore,
  loadCatalogDirectory,
  bundledCatalogStore,
  type CatalogSource,
  type CatalogManifest,
  type ManifestRevision,
} from './store.js';

export {
  PolicyCatalogError,
  ERROR_CODE_FOR_REASON,
  type PolicyCatalogErrorCode,
  type PolicyCatalogErrorReason,
} from './errors.js';

export type {
  CatalogArchitecture,
  CatalogEntryMetadata,
  CatalogIdentityMetadata,
  CatalogInfo,
  CatalogNetworkRule,
  CatalogPlatform,
  CatalogSandboxPolicy,
  IdentityStrength,
  ResolveContext,
  SandboxConfigResolution,
  ToolCandidate,
  ToolInput,
} from './types.js';
