// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! The public v1 surface (design §5): typed [`ContainerRequirements`] over
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
    CatalogEntryMetadata, CatalogInfo, Diagnostics, ResolveContext, ToolInputs,
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
pub fn resolve_tool_requirements(
    tools: impl Into<ToolInputs>,
    ctx: &ResolveContext,
) -> Result<Option<ContainerRequirements>, Error> {
    typed(resolver::resolve_requirements(tools, ctx).map_err(to_error)?)
}

/// [`resolve_tool_requirements`] plus per-input statuses, dependency
/// attribution, and structured warnings from the same pass.
pub fn resolve_tool_requirements_with_diagnostics(
    tools: impl Into<ToolInputs>,
    ctx: &ResolveContext,
) -> Result<ToolRequirementsResolution, Error> {
    let resolution =
        resolver::resolve_requirements_with_diagnostics(tools, ctx).map_err(to_error)?;
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
