// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! The public `mxc_sdk::v1` tool-requirements surface: error mapping,
//! catalog metadata, and command-free requirements.

use mxc_sdk::v1::tool_requirements::{ResolveContext, ToolCandidate, ToolResolutionStatus};
use mxc_sdk::v1::{
    get_catalog_info, list_catalog_entries, resolve_tool_requirements,
    resolve_tool_requirements_with_diagnostics, ErrorCode,
};

#[test]
fn invalid_context_maps_to_malformed_request() {
    let error = resolve_tool_requirements("/usr/bin/git", &ResolveContext::new()).unwrap_err();
    assert_eq!(error.code, ErrorCode::MalformedRequest);
}

#[test]
fn unmatched_tools_resolve_to_none_with_status() {
    let result = resolve_tool_requirements_with_diagnostics(
        ToolCandidate::new("no-such-tool-xyz"),
        &ResolveContext::new(),
    )
    .unwrap();
    assert!(result.requirements.is_none());
    assert_eq!(
        result.diagnostics.tools[0].status,
        ToolResolutionStatus::ToolUnmatched
    );
}

#[test]
fn catalog_metadata_reports_the_sdk_contract_version() {
    let info = get_catalog_info().unwrap();
    assert_eq!(info.sdk_contract_version, "1.0.0");
    assert!(list_catalog_entries()
        .unwrap()
        .iter()
        .any(|entry| entry.entry_id == "tool:git"));
}
