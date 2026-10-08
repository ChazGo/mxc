// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! The public `mxc_sdk::v1` tool-requirements surface: error mapping,
//! catalog metadata, and command-free requirements.

use mxc_sdk::v1::tool_requirements::{
    Architecture, Platform, ResolveContext, SymbolMap, ToolCandidate, ToolResolutionStatus,
};
use mxc_sdk::v1::{
    get_catalog_info, list_catalog_entries, resolve_tool_requirements,
    resolve_tool_requirements_with_diagnostics, ErrorCode,
};

#[test]
fn invalid_context_maps_to_malformed_request() {
    let error = resolve_tool_requirements(&["/usr/bin/git".into()], None).unwrap_err();
    assert_eq!(error.code, ErrorCode::MalformedRequest);
}

#[test]
fn name_only_conversion_sets_only_the_invocation_name() {
    let converted: ToolCandidate = "git".into();
    assert_eq!(
        converted,
        ToolCandidate {
            invocation_name: "git".to_owned(),
            package_url: None,
            detected_version: None,
            intent: None,
        }
    );
}

#[test]
fn borrowed_slices_resolve_one_and_many_tools() {
    let context = ResolveContext::new()
        .platform(Platform::Linux)
        .architecture(Architecture::X64)
        .allow_weak(true);
    let selected = |tools: &[ToolCandidate]| -> Vec<String> {
        resolve_tool_requirements_with_diagnostics(tools, Some(&context))
            .unwrap()
            .diagnostics
            .tools
            .iter()
            .filter_map(|t| t.selection.as_ref().map(|s| s.entry_id.clone()))
            .collect()
    };
    assert_eq!(selected(&["git".into()]), ["tool:git"]);
    assert_eq!(
        selected(&["git".into(), "node".into()]),
        ["tool:git", "tool:node"]
    );
    // Name-only input does not opt into weak matching by itself.
    assert!(resolve_tool_requirements(&["git".into()], None)
        .unwrap()
        .is_none());
}

#[test]
fn project_root_and_symbol_form_must_be_identical() {
    let symbols = |value: &str| [("project_root", value)].into_iter().collect::<SymbolMap>();
    let both = |root: &str, symbol: &str| ResolveContext {
        project_root: Some(root.to_owned()),
        symbols: Some(symbols(symbol)),
        ..ResolveContext::default()
    };
    // Rejected before any input is resolved, even with no inputs.
    let error = resolve_tool_requirements(&[], Some(&both("/a", "/b"))).unwrap_err();
    assert_eq!(error.code, ErrorCode::MalformedRequest);
    // No path normalization: a trailing separator is a different string.
    let error = resolve_tool_requirements(&[], Some(&both("/a", "/a/"))).unwrap_err();
    assert_eq!(error.code, ErrorCode::MalformedRequest);
    assert!(resolve_tool_requirements(&[], Some(&both("/a", "/a")))
        .unwrap()
        .is_none());
}

#[test]
fn unmatched_tools_resolve_to_none_with_status() {
    let result =
        resolve_tool_requirements_with_diagnostics(&["no-such-tool-xyz".into()], None).unwrap();
    assert!(result.requirements.is_none());
    let tool = &result.diagnostics.tools[0];
    assert_eq!(tool.status, ToolResolutionStatus::ToolUnmatched);
    assert!(!tool.contributes);
    assert!(tool.selection.is_none());
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
