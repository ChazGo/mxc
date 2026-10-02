// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! **PROTOTYPE, pending API review.** The MXC policy store: resolve known
//! tools to a candidate floor [`SandboxPolicy`] from the policy catalog
//! bundled statically in this SDK.
//!
//! The result is a **best-effort floor**, not a guarantee: the access a known
//! tool typically needs, which a caller composes with its own policy. It is
//! complementary to Learning Mode, not a replacement. Resolution never grants
//! access, launches a sandbox, contacts a network service, or writes state;
//! the V1 catalog is compiled in and nothing is downloaded.
//!
//! Each catalog entry has one unversioned default plus additive platform,
//! version (`vers` range), and intent overlays. A [`ToolCandidate`] names the
//! tool by package URL (strong) or invocation name (weak fallback), and may
//! carry a `detected_version` and an `intent` such as `fetch` or `push`.
//!
//! The names and shapes here are proposed and may change before sign-off (for
//! example, the names may drop "Sandbox"). This API is not part of MXC 1.0.
//!
//! ```no_run
//! use mxc_sdk::policy_store::{resolve_sandbox_policy, ResolveContext, ToolCandidate};
//!
//! let ctx = ResolveContext::new()
//!     .project_root("/work/repo")
//!     .symbol("git_prefix", "/usr/bin");
//! let git = ToolCandidate::new("git")
//!     .with_package_url("pkg:generic/git")
//!     .with_detected_version("2.45.1")
//!     .with_intent("fetch");
//! if let Some(policy) = resolve_sandbox_policy(git, &ctx)? {
//!     // Compose `policy` with the caller's own policy, then build a request.
//!     let _ = policy;
//! }
//! # Ok::<(), mxc_sdk::policy_store::PolicyCatalogError>(())
//! ```

use crate::SandboxPolicy;
use mxc_policy_store::tooling::Json;

pub use mxc_policy_store::{
    Architecture, CatalogAdditionsMetadata, CatalogEntryMetadata, CatalogIdentityMetadata,
    CatalogInfo, CatalogIntentMetadata, DefaultMetadata, DependencyRecord, Diagnostics,
    EntryMatchRecord, ErrorCode, ErrorReason, IdentityStrength, IntentMode, IntentSelection,
    MatchedIdentity, Platform, PlatformVariantMetadata, PolicyCatalogError, Provenance,
    ResolveContext, SymbolMap, ToolCandidate, ToolInput, ToolInputs, ToolRecord,
    ToolResolutionStatus, ToolResolutionWarning, ToolWarningCode, VersionScheme, VersionSelection,
    VersionStatus, VersionVariantMetadata, Warning,
};

/// Result of [`resolve_sandbox_policy_with_diagnostics`]: the same policy
/// [`resolve_sandbox_policy`] returns, plus match attribution and warnings
/// from the same resolution pass.
#[derive(Debug, Clone)]
pub struct SandboxConfigResolution {
    /// The composed policy, or `None` when nothing could be resolved.
    pub policy: Option<SandboxPolicy>,
    /// Which entries matched, the dependencies they pulled in, and warnings.
    pub diagnostics: Diagnostics,
}

/// Resolves one tool or a list of tools to a single composed floor policy
/// from the bundled catalog. Returns `None` when no policy can be resolved.
pub fn resolve_sandbox_policy(
    tools: impl Into<ToolInputs>,
    context: &ResolveContext,
) -> Result<Option<SandboxPolicy>, PolicyCatalogError> {
    mxc_policy_store::resolve_sandbox_policy(tools, context)?
        .map(|policy| to_sdk_policy(&policy))
        .transpose()
}

/// Like [`resolve_sandbox_policy`], and also reports which catalog entries
/// matched each input, the dependencies they pulled in, and warnings.
pub fn resolve_sandbox_policy_with_diagnostics(
    tools: impl Into<ToolInputs>,
    context: &ResolveContext,
) -> Result<SandboxConfigResolution, PolicyCatalogError> {
    let resolution = mxc_policy_store::resolve_sandbox_policy_with_diagnostics(tools, context)?;
    Ok(SandboxConfigResolution {
        policy: resolution.policy.as_ref().map(to_sdk_policy).transpose()?,
        diagnostics: resolution.diagnostics,
    })
}

/// The bundled catalog's schema version and default revision.
pub fn get_catalog_info() -> Result<CatalogInfo, PolicyCatalogError> {
    mxc_policy_store::get_catalog_info()
}

/// Metadata for every entry in the bundled catalog's default revision. It
/// never exposes a policy body; use the resolve functions for policies.
pub fn list_catalog_entries() -> Result<Vec<CatalogEntryMetadata>, PolicyCatalogError> {
    mxc_policy_store::list_catalog_entries()
}

/// Converts the store's catalog-shaped policy to the SDK [`SandboxPolicy`].
/// The store emits the same camelCase JSON shape the SDK accepts, so this is
/// a lossless round trip for every field the catalog contract allows.
fn to_sdk_policy(
    policy: &mxc_policy_store::SandboxPolicy,
) -> Result<SandboxPolicy, PolicyCatalogError> {
    policy_from_json(&policy.to_json())
}

fn policy_from_json(json: &Json) -> Result<SandboxPolicy, PolicyCatalogError> {
    serde_json::from_str(&json.to_compact_string()).map_err(|error| {
        PolicyCatalogError::new(
            ErrorReason::InvalidCatalog,
            format!("the resolved policy is not a valid SDK SandboxPolicy: {error}"),
        )
    })
}

/// JSON entry points for language bindings that reach the store through
/// `mxc_ffi`. Not a stable API; the C ABI and its bindings are co-versioned.
#[doc(hidden)]
pub mod binding {
    use super::{policy_from_json, PolicyCatalogError};
    use mxc_policy_store::request::parse_resolve_request;
    use mxc_policy_store::tooling::{Json, JsonObject};

    /// `{"tools", "context"?}` → `{"policy"?}`. `policy` is omitted when
    /// nothing resolves, so absence survives the boundary.
    pub fn resolve_sandbox_policy_json(request: &str) -> Result<String, PolicyCatalogError> {
        let (tools, context) = parse_resolve_request(request)?;
        let mut out = JsonObject::new();
        if let Some(policy) = mxc_policy_store::resolve_sandbox_policy(tools, &context)? {
            let json = policy.to_json();
            policy_from_json(&json)?;
            out.insert("policy", json);
        }
        Ok(Json::Object(out).to_compact_string())
    }

    /// `{"tools", "context"?}` → `{"policy"?, "diagnostics"}`.
    pub fn resolve_sandbox_policy_with_diagnostics_json(
        request: &str,
    ) -> Result<String, PolicyCatalogError> {
        let (tools, context) = parse_resolve_request(request)?;
        let resolution =
            mxc_policy_store::resolve_sandbox_policy_with_diagnostics(tools, &context)?;
        if let Some(policy) = &resolution.policy {
            policy_from_json(&policy.to_json())?;
        }
        Ok(resolution.to_json().to_compact_string())
    }

    /// `{"catalogSchemaVersion", "catalogRevision"}`.
    pub fn catalog_info_json() -> Result<String, PolicyCatalogError> {
        Ok(mxc_policy_store::get_catalog_info()?
            .to_json()
            .to_compact_string())
    }

    /// An array of entry metadata objects.
    pub fn list_catalog_entries_json() -> Result<String, PolicyCatalogError> {
        let entries = mxc_policy_store::list_catalog_entries()?;
        Ok(Json::Array(entries.iter().map(|e| e.to_json()).collect()).to_compact_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_policy(extra: &str) -> mxc_policy_store::SandboxPolicy {
        let raw = Json::parse(&format!(r#"{{"version":"0.9.0-alpha"{extra}}}"#)).unwrap();
        let get = |key: &str| raw.get(key).cloned();
        mxc_policy_store::SandboxPolicy {
            version: "0.9.0-alpha".into(),
            filesystem: None,
            network: get("network"),
            ui: get("ui"),
            timeout_ms: get("timeoutMs").and_then(|t| t.as_f64()),
        }
    }

    /// Every non-filesystem field the catalog contract admits converts.
    #[test]
    fn catalog_contract_fields_convert_to_the_sdk_policy() {
        let policy = to_sdk_policy(&store_policy(
            r#","network":{"egress":{"default":"deny","allow":[{"to":[{"cidr":"10.0.0.0/8","except":["10.1.0.0/16"]}],"ports":[{"protocol":"tcp","port":443,"endPort":444}]}],"deny":[{"ports":[{"protocol":"any"}]}]},"ingress":{"default":"deny","hostLoopback":"allow"}},"ui":{"allowWindows":true,"clipboard":"read"},"timeoutMs":60000"#,
        ))
        .unwrap();
        let network = policy.network.unwrap();
        let egress = network.egress.unwrap();
        assert_eq!(
            egress.allow.unwrap()[0].ports.as_ref().unwrap()[0].end_port,
            Some(444)
        );
        assert_eq!(egress.deny.unwrap().len(), 1);
        assert_eq!(
            network.ingress.unwrap().host_loopback,
            Some(crate::NetworkAction::Allow)
        );
        let ui = policy.ui.unwrap();
        assert!(ui.allow_windows);
        assert!(!ui.allow_input_injection);
        assert_eq!(policy.timeout_ms, Some(60000));
    }

    /// The catalog admits any positive integer timeout; the SDK holds `u32`.
    #[test]
    fn an_out_of_range_timeout_is_a_policy_validation_error() {
        let error = to_sdk_policy(&store_policy(r#","timeoutMs":5000000000"#)).unwrap_err();
        assert_eq!(error.reason(), ErrorReason::InvalidCatalog);
    }
}
