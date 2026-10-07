// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! **PROTOTYPE, pending API review.** The MXC policy store: resolves known
//! tools to command-free MXC 1.0 container requirements (filesystem, network,
//! UI, timeout) from a policy catalog that is bundled statically in
//! `mxc-sdk`.
//!
//! The API names, shapes, and catalog contents are proposed and may change
//! before sign-off. The feature is not part of MXC 1.0; it targets a later SDK
//! release and builds the SDK's own v1 `ContainerRequest` types.
//!
//! References to "design §N" are to `docs/mxc-policy-store.md` (catalog
//! design) and to "API spec §N" are to `docs/mxc-policy-store-api.md` (the
//! caller-facing contract), both in microsoft/mxc#1309.
//!
//! The public surface is `crate::v1::resolve_tool_requirements` and
//! `crate::v1::resolve_tool_requirements_with_diagnostics` (plus the
//! metadata calls). The internal lookup ([`resolve_requirements`],
//! [`resolve_requirements_with_diagnostics`], [`get_catalog_info`],
//! [`list_catalog_entries`], and [`PolicyCatalog`]) returns a **best-effort
//! floor**, not a guarantee: the access a known tool typically needs, which
//! callers review and constrain before adding a command. It is complementary to Learning
//! Mode, not a replacement. It never grants access, launches a sandbox,
//! contacts a network service, or writes consumer state. The V1 catalog is
//! compiled in and validated on first use; nothing is downloaded.
//!
//! This is an internal `mxc-sdk` module. The Node and C# SDKs reach it through
//! `mxc_ffi`. [`tooling`] exposes the catalog contribution rules (validation,
//! history, canonical JSON, paths) that the `policy_store_*` tests enforce.

pub mod assemble;
pub mod catalog;
pub mod compose;
pub mod effective;
pub mod errors;
pub mod exact;
pub mod history;
pub mod host;
pub mod json;
pub mod model;
pub mod netrule;
pub mod paths;
pub mod purl;
pub mod request;
pub mod resolver;
pub mod sdk;
pub mod store;
mod text;
pub mod validate;
pub mod vers;
pub mod view;

pub use crate::mxc_common::filesystem_object::ExistingObjectComparison;
pub use errors::{ErrorCode, ErrorReason, PolicyCatalogError};
pub use host::{FixedHost, HostEnvironment, SystemHost};
pub use model::{
    Architecture, ArchitectureFallback, CatalogAdditionsMetadata, CatalogEntryMetadata,
    CatalogIdentityMetadata, CatalogInfo, CatalogIntentMetadata, DefaultMetadata, DependencyRecord,
    DetailWarningKind, Diagnostics, EntryMatchRecord, FilesystemRequirements, IdentityStrength,
    IntentMode, IntentSelection, MatchedIdentity, NetworkRequirement, PathAccess, PathRequirement,
    Platform, PlatformVariantMetadata, Provenance, PurlComponent, Requirements,
    RequirementsResolution, ResolutionDetailWarning, ResolveContext, SymbolMap, SymbolValueSource,
    ToolCandidate, ToolInput, ToolInputs, ToolRecord, ToolResolutionStatus, ToolResolutionWarning,
    ToolWarningKind, VersionSelection, VersionStatus, VersionVariantMetadata, Warning,
};
pub use resolver::{
    get_catalog_info, list_catalog_entries, resolve_requirements,
    resolve_requirements_with_diagnostics, PolicyCatalog,
};
pub use store::{
    bundled_catalog_store, load_catalog_directory, CatalogManifest, CatalogSource, CatalogStore,
    DirectorySource, ManifestRevision, MemorySource,
};

/// Contribution and CI tooling (not part of the runtime lookup API).
pub mod tooling {
    pub use crate::policy_store::assemble::{
        assemble_revision, collect_entry_sources, render_revision, EntrySource,
    };
    pub use crate::policy_store::catalog::{
        compare_catalog_revisions, validate_catalog_revision, validate_contract, Additions,
        CatalogContract, CatalogEntry, CatalogRevision, Dependency, EntryDefault,
        IdentityPredicate, IntentDefinition, Overlay, PlatformVariant, SymbolSource,
        VersionVariant,
    };
    pub use crate::policy_store::effective::{
        materialize, materialize_entry, select_platform_variant, Effective, IntentChoice,
        Materialized, PlatformSelection,
    };
    pub use crate::policy_store::exact::{
        bind_fixture_symbols, exact_document, to_container_requirements, validate_exact,
        VALIDATION_COMMAND,
    };
    pub use crate::policy_store::history::{
        check_entry_revisions, check_published_immutability, check_store_history,
        PublishedRevision, PublishedState,
    };
    pub use crate::policy_store::host::{architecture_from_machine, object_within, ObjectRelation};
    pub use crate::policy_store::json::{canonical_json, js_number_to_string, Json, JsonObject};
    pub use crate::policy_store::paths::{
        case_key, folds_case, is_absolute_path, normalize_path, path_key_segments,
    };
    pub use crate::policy_store::purl::{parse_purl, ParsedPurl};
    pub use crate::policy_store::store::{
        bundled_catalog_files, bundled_catalog_source, validate_manifest,
    };
    pub use crate::policy_store::validate::{
        check_against_base_ref, read_published_state_at_ref, validate_bundled_catalog,
        validate_catalog_directory, CatalogValidationReport,
    };
    pub use crate::policy_store::view::{render_exact_requests, render_reviewer_view};
}

pub use vers::{VersRange, Version, VersionScheme};
