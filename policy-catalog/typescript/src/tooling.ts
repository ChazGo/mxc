// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Contribution and CI tooling (spec §7). Not part of the runtime lookup API;
// exported under the `./tooling` subpath so validation scripts and tests can
// share the resolver's exact rules.
export { canonicalJson, canonicalSha256 } from './canonical-json.js';
export {
  validateCatalogRevision,
  validateContract,
  compareCatalogRevisions,
  selectVariant,
  PLATFORMS,
  ARCHITECTURES,
  type CatalogContract,
  type CatalogEntry,
  type CatalogRevision,
  type IdentityPredicate,
  type PlatformVariant,
} from './catalog.js';
export { validateManifest } from './store.js';
export {
  checkEntryRevisions,
  checkStoreHistory,
  checkPublishedImmutability,
  type PublishedState,
} from './history.js';
export { isValidVersionRange, satisfiesVersionRange } from './version-range.js';
export { parsePurl } from './purl.js';
