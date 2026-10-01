// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! PROTOTYPE: native Rust implementation of the MXC known-tool policy
//! catalog library, with behavioral parity to the TypeScript implementation
//! in `policy-catalog/src`.
//!
//! The runtime lookup API ([`resolve_sandbox_policy`],
//! [`resolve_sandbox_policy_with_diagnostics`], [`get_catalog_info`],
//! [`list_catalog_entries`], and [`PolicyCatalog`]) returns a **candidate
//! lower-bound** policy. It never grants access, launches a sandbox, contacts
//! a network service, or writes consumer state. The catalog bundled with this
//! crate is compiled in and integrity-checked on first use.
//!
//! [`tooling`] exposes the contribution/CI rules (validation, history,
//! canonical JSON, paths) shared with the `policy-catalog` binary.

pub mod catalog;
pub mod errors;
pub mod history;
pub mod host;
pub mod json;
pub mod model;
pub mod paths;
pub mod purl;
pub mod resolver;
pub mod store;
mod text;
pub mod validate;
pub mod version_range;

pub use errors::{ErrorCode, ErrorReason, PolicyCatalogError};
pub use host::{FixedHost, HostEnvironment, SystemHost};
pub use model::{
    Architecture, CatalogEntryMetadata, CatalogIdentityMetadata, CatalogInfo, DependencyRecord, Diagnostics,
    EntryMatchRecord, FilesystemPolicy, IdentityStrength, MatchedIdentity, Platform, PlatformVariantMetadata,
    Provenance, ResolveContext, SandboxConfigResolution, SandboxPolicy, SymbolMap, ToolCandidate, ToolInput,
    ToolInputs, ToolRecord,
};
pub use resolver::{
    get_catalog_info, list_catalog_entries, resolve_sandbox_policy, resolve_sandbox_policy_with_diagnostics,
    PolicyCatalog,
};
pub use store::{
    bundled_catalog_store, load_catalog_directory, CatalogManifest, CatalogSource, CatalogStore, DirectorySource,
    ManifestRevision, MemorySource,
};

/// Contribution and CI tooling (not part of the runtime lookup API).
pub mod tooling {
    pub use crate::catalog::{
        compare_catalog_revisions, select_variant, validate_catalog_revision, validate_contract, CatalogContract,
        CatalogEntry, CatalogRevision, IdentityPredicate, PlatformVariant,
    };
    pub use crate::history::{
        check_entry_revisions, check_published_immutability, check_store_history, PublishedRevision, PublishedState,
    };
    pub use crate::host::architecture_from_machine;
    pub use crate::json::{canonical_json, canonical_sha256, js_number_to_string, Json, JsonObject};
    pub use crate::paths::{case_key, folds_case, is_absolute_path, normalize_path, path_key_segments};
    pub use crate::purl::{parse_purl, ParsedPurl};
    pub use crate::store::{bundled_catalog_files, bundled_catalog_source, validate_manifest};
    pub use crate::validate::{
        check_against_base_ref, read_published_state_at_ref, validate_bundled_catalog, validate_catalog_directory,
        CatalogValidationReport,
    };
    pub use crate::version_range::{is_valid_version_range, satisfies_version_range};
}

/// The command-line harness behind the `policy-catalog` binary.
pub mod cli;
