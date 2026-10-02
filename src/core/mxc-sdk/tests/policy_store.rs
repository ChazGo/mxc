// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! The prototype policy store surface of `mxc-sdk`: typed SDK policies, and
//! the JSON binding path used by `mxc_ffi`, checked against the store's
//! bundled-catalog conformance fixtures.

use mxc_sdk::policy_store::{
    binding, get_catalog_info, list_catalog_entries, resolve_sandbox_policy,
    resolve_sandbox_policy_with_diagnostics, Architecture, ErrorReason, IntentMode, Platform,
    ResolveContext, ToolCandidate, ToolResolutionStatus, VersionStatus, Warning,
};
use serde_json::Value;
use std::path::PathBuf;

fn linux_context() -> ResolveContext {
    ResolveContext::new()
        .platform(Platform::Linux)
        .architecture(Architecture::X64)
        .project_root("/work/repo")
        .symbol("git_prefix", "/usr/bin")
}

#[test]
fn resolves_to_the_sdk_sandbox_policy_type() {
    let ctx = linux_context()
        .symbol("node_prefix", "/opt/node/bin")
        .symbol("npm_prefix", "/opt/node/bin")
        .symbol("npm_cache", "/home/u/.npm");
    let policy = resolve_sandbox_policy(
        ToolCandidate::new("npm").with_package_url("pkg:npm/npm"),
        &ctx,
    )
    .unwrap()
    .expect("npm resolves by its strong identity");
    assert_eq!(policy.version, "0.9.0-alpha");
    let filesystem = policy.filesystem.expect("filesystem section");
    assert_eq!(filesystem.readonly_paths, ["/opt/node/bin"]);
    assert_eq!(filesystem.readwrite_paths, ["/work/repo", "/home/u/.npm"]);
    assert!(filesystem.denied_paths.is_empty());
    assert!(policy.network.is_none());
    assert!(policy.ui.is_none());
    assert_eq!(policy.timeout_ms, None);
}

#[test]
fn diagnostics_carry_the_same_policy_and_attribution() {
    let ctx = linux_context().allow_weak(true);
    let resolution = resolve_sandbox_policy_with_diagnostics("git", &ctx).unwrap();
    let policy = resolution.policy.expect("git resolves");
    let plain = resolve_sandbox_policy("git", &ctx).unwrap().unwrap();
    assert_eq!(
        policy.filesystem.unwrap().readonly_paths,
        plain.filesystem.unwrap().readonly_paths
    );
    assert_eq!(resolution.diagnostics.tools.len(), 1);
    assert_eq!(
        resolution.diagnostics.tools[0].matches[0].entry_id,
        "tool:git"
    );
}

#[test]
fn version_and_intent_select_the_effective_policy() {
    let ctx = linux_context().symbol("ssh_prefix", "/usr/lib/ssh");
    let git = |version: &str, intent: &str| {
        ToolCandidate::new("git")
            .with_package_url("pkg:generic/git")
            .with_detected_version(version)
            .with_intent(intent)
    };
    let push = resolve_sandbox_policy_with_diagnostics(git("2.45.1", "push"), &ctx).unwrap();
    let tool = &push.diagnostics.tools[0];
    assert_eq!(
        tool.status,
        ToolResolutionStatus::Version(VersionStatus::MatchedVersion)
    );
    assert_eq!(
        tool.matches[0].intent_selection.as_ref().unwrap().mode,
        IntentMode::Named
    );
    assert_eq!(
        push.diagnostics.resolved_dependencies[0].entry_id,
        "tool:ssh"
    );
    let policy = push.policy.unwrap();
    assert_eq!(
        policy.filesystem.unwrap().readonly_paths,
        ["/usr/bin", "/usr/lib/ssh"]
    );
    assert_eq!(
        policy.network.unwrap().egress.unwrap().allow.unwrap().len(),
        1
    );

    let unsupported =
        resolve_sandbox_policy_with_diagnostics(git("2.45.1", "bundle-fetch"), &ctx).unwrap();
    assert!(unsupported.policy.is_none());
    assert_eq!(
        unsupported.diagnostics.tools[0].status,
        ToolResolutionStatus::IntentUnsupported
    );
    assert!(matches!(
        &unsupported.diagnostics.warnings[0],
        Warning::Tool(w) if w.code.as_str() == "intent_unsupported"
    ));
}

#[test]
fn absence_is_none_and_bad_context_is_an_error() {
    assert!(resolve_sandbox_policy("not-a-known-tool", &linux_context())
        .unwrap()
        .is_none());
    let error =
        resolve_sandbox_policy("git", &ResolveContext::new().platform_str("solaris")).unwrap_err();
    assert_eq!(error.reason(), ErrorReason::InvalidContext);
}

trait PlatformStr {
    fn platform_str(self, value: &str) -> Self;
}

impl PlatformStr for ResolveContext {
    fn platform_str(mut self, value: &str) -> Self {
        self.platform = Some(value.to_string());
        self
    }
}

#[test]
fn inspection_reports_the_bundled_catalog() {
    let info = get_catalog_info().unwrap();
    assert_eq!(info.catalog_schema_version, "1");
    let ids: Vec<String> = list_catalog_entries()
        .unwrap()
        .into_iter()
        .map(|e| e.entry_id)
        .collect();
    assert!(ids.contains(&"tool:git".to_string()), "{ids:?}");
}

fn fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../mxc_policy_store/conformance/fixtures/bundled-catalog.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Every host-independent bundled conformance case gives the same result
/// through the JSON binding path the Node and C# SDKs use.
#[test]
fn binding_json_matches_the_bundled_conformance_fixtures() {
    let fixture = fixture();
    let mut ran = 0;
    for case in fixture["cases"].as_array().unwrap() {
        if case.get("host").is_some() {
            continue;
        }
        let name = case["name"].as_str().unwrap();
        let mut request = serde_json::Map::new();
        request.insert("tools".into(), case["tools"].clone());
        if let Some(context) = case.get("context") {
            request.insert("context".into(), context.clone());
        }
        let request = Value::Object(request).to_string();
        let plain = binding::resolve_sandbox_policy_json(&request);
        let full = binding::resolve_sandbox_policy_with_diagnostics_json(&request);
        if let Some(expected) = case.get("expectError") {
            for error in [plain.unwrap_err(), full.unwrap_err()] {
                assert_eq!(error.code().as_str(), expected["code"], "{name}");
                assert_eq!(error.reason().as_str(), expected["reason"], "{name}");
            }
        } else {
            let expect = &case["expect"];
            let plain: Value = serde_json::from_str(&plain.unwrap()).unwrap();
            let full: Value = serde_json::from_str(&full.unwrap()).unwrap();
            let policy = |v: &Value| v.get("policy").cloned().unwrap_or(Value::Null);
            assert_eq!(policy(&plain), expect["policy"], "{name}");
            assert_eq!(policy(&full), expect["policy"], "{name}");
            assert_eq!(full["diagnostics"], expect["diagnostics"], "{name}");
        }
        ran += 1;
    }
    assert!(ran >= 10, "ran {ran} cases");
}

#[test]
fn binding_inspection_and_malformed_requests() {
    let info: Value = serde_json::from_str(&binding::catalog_info_json().unwrap()).unwrap();
    assert_eq!(info["catalogSchemaVersion"], "1");
    let entries: Value =
        serde_json::from_str(&binding::list_catalog_entries_json().unwrap()).unwrap();
    assert!(entries
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["entryId"] == "tool:git"));
    let error =
        binding::resolve_sandbox_policy_json(r#"{"tools":"git","extra":true}"#).unwrap_err();
    assert_eq!(error.reason(), ErrorReason::InvalidContext);
}
