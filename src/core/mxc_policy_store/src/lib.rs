// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! **PROTOTYPE, pending API review.** The MXC policy store: resolves known
//! tools to a candidate floor [`SandboxPolicy`] from a policy catalog that is
//! bundled statically in this crate.
//!
//! The API names, shapes, and catalog contents are proposed and may change
//! before sign-off (for example, the names may drop "Sandbox", and lookup may
//! gain an intent such as `git pull` versus `git push`). The feature is not
//! part of MXC 1.0.
//!
//! The runtime lookup API ([`resolve_sandbox_policy`],
//! [`resolve_sandbox_policy_with_diagnostics`], [`get_catalog_info`],
//! [`list_catalog_entries`], and [`PolicyCatalog`]) returns a **best-effort
//! floor**, not a guarantee: the access a known tool typically needs, which
//! callers compose with their own policy. It is complementary to Learning
//! Mode, not a replacement. It never grants access, launches a sandbox,
//! contacts a network service, or writes consumer state. The V1 catalog is
//! compiled in and integrity-checked on first use; nothing is downloaded.
//!
//! Most callers reach this crate through an MXC SDK: `mxc_sdk::policy_store`
//! in Rust, and the Node and C# SDKs through `mxc_ffi`. [`tooling`] exposes
//! the catalog contribution rules (validation, history, canonical JSON,
//! paths) that the crate's own tests enforce.

pub mod catalog;
pub mod errors;
pub mod history;
pub mod host;
pub mod json;
pub mod model;
pub mod paths;
pub mod purl;
pub mod request;
pub mod resolver;
pub mod store;
mod text;
pub mod validate;
pub mod vers;
pub mod version_range;

pub use errors::{ErrorCode, ErrorReason, PolicyCatalogError};
pub use host::{FixedHost, HostEnvironment, SystemHost};
pub use model::{
    Architecture, CatalogEntryMetadata, CatalogIdentityMetadata, CatalogInfo, DependencyRecord,
    Diagnostics, EntryMatchRecord, FilesystemPolicy, IdentityStrength, MatchedIdentity, Platform,
    PlatformVariantMetadata, Provenance, ResolveContext, SandboxConfigResolution, SandboxPolicy,
    SymbolMap, ToolCandidate, ToolInput, ToolInputs, ToolRecord,
};
pub use resolver::{
    get_catalog_info, list_catalog_entries, resolve_sandbox_policy,
    resolve_sandbox_policy_with_diagnostics, PolicyCatalog,
};
pub use store::{
    bundled_catalog_store, load_catalog_directory, CatalogManifest, CatalogSource, CatalogStore,
    DirectorySource, ManifestRevision, MemorySource,
};

/// Contribution and CI tooling (not part of the runtime lookup API).
pub mod tooling {
    pub use crate::catalog::{
        compare_catalog_revisions, select_variant, validate_catalog_revision, validate_contract,
        CatalogContract, CatalogEntry, CatalogRevision, IdentityPredicate, PlatformVariant,
        SymbolSource,
    };
    pub use crate::history::{
        check_entry_revisions, check_published_immutability, check_store_history,
        PublishedRevision, PublishedState,
    };
    pub use crate::host::architecture_from_machine;
    pub use crate::json::{canonical_json, js_number_to_string, Json, JsonObject};
    pub use crate::paths::{
        case_key, folds_case, is_absolute_path, normalize_path, path_key_segments,
    };
    pub use crate::purl::{parse_purl, ParsedPurl};
    pub use crate::store::{bundled_catalog_files, bundled_catalog_source, validate_manifest};
    pub use crate::validate::{
        check_against_base_ref, read_published_state_at_ref, validate_bundled_catalog,
        validate_catalog_directory, CatalogValidationReport,
    };
    pub use crate::version_range::{is_valid_version_range, satisfies_version_range};
}

pub use vers::{VersRange, Version, VersionScheme};
