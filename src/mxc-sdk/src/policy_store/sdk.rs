// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! The public v1 surface (API spec §1): typed [`ContainerRequirements`] over
//! the bundled catalog, with failures mapped to the SDK's [`Error`].
//!
//! **Prototype, pending API review.** Rust `Error` carries no
//! `details.reason`; the code alone is authoritative (the reason is optional
//! by design). The C ABI, Node, and .NET surfaces carry the reason.

use crate::mxc_engine::{Error, ErrorCode};
use crate::policy::ContainerRequirements;
use crate::policy_store::errors::{self, PolicyCatalogError};
use crate::policy_store::exact::to_container_requirements;
use crate::policy_store::model::{
    CatalogEntryMetadata, CatalogInfo, Diagnostics, ResolveContext, ToolCandidate,
};
use crate::policy_store::resolver;

/// Composed requirements and the attribution and warnings from the same
/// resolution pass.
#[derive(Clone, Debug)]
pub struct ToolRequirementsResolution {
    /// `None` when no pair contributed or a required symbol is unresolved.
    pub requirements: Option<ContainerRequirements>,
    pub diagnostics: Diagnostics,
}

fn to_error(error: PolicyCatalogError) -> Error {
    let code = match error.code() {
        errors::ErrorCode::PolicyValidation => ErrorCode::PolicyValidation,
        errors::ErrorCode::MalformedRequest => ErrorCode::MalformedRequest,
        errors::ErrorCode::UnsupportedContainment => ErrorCode::UnsupportedContainment,
        errors::ErrorCode::BackendError => ErrorCode::BackendError,
    };
    Error::new(code, error.detail())
}

fn typed(
    requirements: Option<crate::policy_store::model::Requirements>,
) -> Result<Option<ContainerRequirements>, Error> {
    requirements
        .map(|r| to_container_requirements(&r))
        .transpose()
        .map_err(|problem| Error::new(ErrorCode::PolicyValidation, problem))
}

/// Resolves one tool or several to command-free container requirements: a
/// best-effort floor that callers review and constrain before adding a
/// command with [`crate::v1::ContainerRequest::from_requirements`]. `None`
/// means no requirements were resolved; library failures are errors.
///
/// One tool is a one-element slice; `None` selects the documented context
/// defaults (API spec §6).
pub fn resolve_tool_requirements(
    tools: &[ToolCandidate],
    context: Option<&ResolveContext>,
) -> Result<Option<ContainerRequirements>, Error> {
    let default = ResolveContext::default();
    typed(resolver::resolve_requirements(tools, context.unwrap_or(&default)).map_err(to_error)?)
}

/// [`resolve_tool_requirements`] plus per-input statuses, dependency
/// attribution, and structured warnings from the same pass.
pub fn resolve_tool_requirements_with_diagnostics(
    tools: &[ToolCandidate],
    context: Option<&ResolveContext>,
) -> Result<ToolRequirementsResolution, Error> {
    let default = ResolveContext::default();
    let resolution =
        resolver::resolve_requirements_with_diagnostics(tools, context.unwrap_or(&default))
            .map_err(to_error)?;
    Ok(ToolRequirementsResolution {
        requirements: typed(resolution.requirements)?,
        diagnostics: resolution.diagnostics,
    })
}

/// Metadata for every entry of the installed catalog revision.
pub fn list_catalog_entries() -> Result<Vec<CatalogEntryMetadata>, Error> {
    resolver::list_catalog_entries().map_err(to_error)
}

/// The bundled catalog schema version, revision, and SDK contract version.
pub fn get_catalog_info() -> Result<CatalogInfo, Error> {
    resolver::get_catalog_info().map_err(to_error)
}

/// JSON entry points for the Node and C# SDKs, which reach the store through
/// `mxc_ffi`. Not a stable API; the C ABI and its bindings are co-versioned.
/// Failures keep the store's `details.reason`.
#[doc(hidden)]
pub mod binding {
    use crate::policy_store::errors::{ErrorReason, PolicyCatalogError, Result};
    use crate::policy_store::exact::to_container_requirements;
    use crate::policy_store::json::{Json, JsonObject};
    use crate::policy_store::model::Requirements;
    use crate::policy_store::request::parse_resolve_request;
    use crate::policy_store::resolver;

    /// Proves the composed value converts to the typed v1 sections before it
    /// crosses the boundary, as the Rust surface does.
    fn checked(requirements: &Requirements) -> Result<Json> {
        to_container_requirements(requirements)
            .map_err(|problem| PolicyCatalogError::new(ErrorReason::InvalidCatalog, problem))?;
        Ok(requirements.to_json())
    }

    /// `{"tools", "context"?}` → `{"requirements"?}`; `requirements` is
    /// omitted when nothing resolves, so absence survives the boundary.
    pub fn resolve_tool_requirements_json(request: &str) -> Result<String> {
        let (tools, context) = parse_resolve_request(request)?;
        let mut out = JsonObject::new();
        if let Some(requirements) = resolver::resolve_requirements(tools, &context)? {
            out.insert("requirements", checked(&requirements)?);
        }
        Ok(Json::Object(out).to_compact_string())
    }

    /// `{"tools", "context"?}` → `{"requirements"?, "diagnostics"}`.
    pub fn resolve_tool_requirements_with_diagnostics_json(request: &str) -> Result<String> {
        let (tools, context) = parse_resolve_request(request)?;
        let resolution = resolver::resolve_requirements_with_diagnostics(tools, &context)?;
        if let Some(requirements) = &resolution.requirements {
            checked(requirements)?;
        }
        Ok(resolution.to_json().to_compact_string())
    }

    /// `{"catalogSchemaVersion", "catalogRevision", "sdkContractVersion"}`.
    pub fn catalog_info_json() -> Result<String> {
        Ok(resolver::get_catalog_info()?.to_json().to_compact_string())
    }

    /// An array of entry metadata objects.
    pub fn list_catalog_entries_json() -> Result<String> {
        let entries = resolver::list_catalog_entries()?;
        Ok(Json::Array(entries.iter().map(|e| e.to_json()).collect()).to_compact_string())
    }
}
