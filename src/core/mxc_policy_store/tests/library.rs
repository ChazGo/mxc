// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Resolver, validation, and store suites for the entry model in the Policy
//! Store design: one unversioned default per entry, additive platform and
//! version overlays, and intent selection.

mod common;

use common::*;
use mxc_policy_store::tooling::{
    canonical_json, check_entry_revisions, check_published_immutability, check_store_history,
    validate_catalog_revision, validate_contract, Json, PublishedRevision, PublishedState,
};
use mxc_policy_store::{
    get_catalog_info, list_catalog_entries, resolve_sandbox_policy,
    resolve_sandbox_policy_with_diagnostics, Architecture, ErrorReason, FixedHost, HostEnvironment,
    IntentMode, Platform, PolicyCatalog, PolicyCatalogError, ResolveContext,
    SandboxConfigResolution, ToolCandidate, ToolInput, ToolInputs, ToolResolutionStatus,
    VersionStatus, Warning,
};
use std::collections::HashMap;
use std::sync::Arc;

const V: &str = "0.9.0-alpha";

fn weak() -> ResolveContext {
    ResolveContext::new().allow_weak(true)
}

fn validate(entries: Vec<Json>) -> Result<(), PolicyCatalogError> {
    let contract = validate_contract(&contract()).unwrap();
    validate_catalog_revision(&revision(entries), &contract).map(|_| ())
}

fn validate_err(entries: Vec<Json>, needle: &str) {
    let error = validate(entries).expect_err(needle);
    assert_eq!(error.reason(), ErrorReason::InvalidCatalog);
    assert!(
        error.message().contains(needle),
        "{} does not contain {needle}",
        error.message()
    );
}

/// `{"default":{"sandboxPolicy":<policy>}<extra>}` as entry overrides.
fn with_default(policy: &str, extra: &str) -> String {
    format!(r#"{{"default":{{"sandboxPolicy":{policy}}}{extra}}}"#)
}

fn fs_json(policy: &Option<mxc_policy_store::SandboxPolicy>) -> Json {
    policy
        .as_ref()
        .expect("a policy")
        .filesystem
        .as_ref()
        .expect("a filesystem section")
        .to_json()
}

fn messages(warnings: &[Warning]) -> String {
    warnings
        .iter()
        .map(Warning::message)
        .collect::<Vec<_>>()
        .join("\n")
}

fn egress(result: &SandboxConfigResolution) -> Option<Json> {
    result
        .policy
        .as_ref()
        .and_then(|p| p.network.clone())
        .and_then(|n| n.get("egress").and_then(|e| e.get("allow")).cloned())
}

fn tcp(cidr: &str, port: u16) -> String {
    format!(r#"{{"to":[{{"cidr":"{cidr}"}}],"ports":[{{"protocol":"tcp","port":{port}}}]}}"#)
}

// ---------------------------------------------------------------------------
// Platform and architecture selection
// ---------------------------------------------------------------------------

fn arch_catalog(host: Arc<FixedHost>) -> PolicyCatalog {
    catalog_for(
        revision(vec![entry(
            "tool:a",
            r#"{"identity":[{"kind":"purl","value":"pkg:npm/a"},{"kind":"invocation-name","names":["a"]}],
                "platformVariants":[
                  {"when":{"platform":"windows","architecture":"x64"},"policyAdditions":{"filesystem":{"readonlyPaths":["${git_prefix}/x64"]}}},
                  {"when":{"platform":"windows","architecture":"arm64"},"policyAdditions":{"filesystem":{"readonlyPaths":["${git_prefix}/arm64"]}}}]}"#,
        )]),
        host,
    )
}

#[test]
fn omitted_architecture_uses_the_native_system_architecture() {
    let catalog = arch_catalog(fixed_host(Platform::Windows, Architecture::Arm64));
    let ctx = ResolveContext::new()
        .project_root("C:\\p")
        .symbol("git_prefix", "C:\\g");
    assert_eq!(catalog.resolve_sandbox_policy("a", &ctx).unwrap(), None);
    let result = catalog
        .resolve_sandbox_policy_with_diagnostics(
            ToolCandidate::new("a").with_package_url("pkg:npm/a"),
            &ctx,
        )
        .unwrap();
    assert_eq!(
        fs_json(&result.policy),
        j(r#"{"readonlyPaths":["C:\\g\\arm64"],"readwritePaths":["C:\\p"]}"#)
    );
    assert_eq!(result.diagnostics.catalog_revision, "2000-01-01.1");
    assert!(messages(&result.diagnostics.warnings)
        .contains("native system architecture 'arm64'; the tool's architecture was not verified"));
}

#[test]
fn explicit_architecture_wins_and_suppresses_the_host_default_warning() {
    let catalog = arch_catalog(fixed_host(Platform::Windows, Architecture::Arm64));
    let result = catalog
        .resolve_sandbox_policy_with_diagnostics(
            ToolCandidate::new("a").with_package_url("pkg:npm/a"),
            &ResolveContext::new()
                .architecture(Architecture::X64)
                .project_root("C:\\p")
                .symbol("git_prefix", "C:\\g"),
        )
        .unwrap();
    assert_eq!(
        fs_json(&result.policy),
        j(r#"{"readonlyPaths":["C:\\g\\x64"],"readwritePaths":["C:\\p"]}"#)
    );
    assert!(result.diagnostics.warnings.is_empty());
}

#[test]
fn another_architecture_is_never_used_and_neutral_additions_are_reported() {
    let catalog = catalog_for(
        revision(vec![
            entry(
                "tool:a",
                r#"{"platformVariants":[{"when":{"platform":"linux","architecture":"arm64"},"policyAdditions":{"filesystem":{"readonlyPaths":["${git_prefix}"]}}}]}"#,
            ),
            entry(
                "tool:b",
                r#"{"platformVariants":[
                    {"when":{"platform":"linux"},"policyAdditions":{"filesystem":{"readonlyPaths":["${node_prefix}"]}}},
                    {"when":{"platform":"linux","architecture":"arm64"},"policyAdditions":{"filesystem":{"readonlyPaths":["${git_prefix}"]}}}]}"#,
            ),
        ]),
        linux_x64(),
    );
    let ctx = weak()
        .architecture(Architecture::X64)
        .project_root("/p")
        .symbol("node_prefix", "/n");
    let a = catalog
        .resolve_sandbox_policy_with_diagnostics("a", &ctx)
        .unwrap();
    assert_eq!(fs_json(&a.policy), j(r#"{"readwritePaths":["/p"]}"#));
    assert!(!messages(&a.diagnostics.warnings).contains("architecture"));
    let b = catalog
        .resolve_sandbox_policy_with_diagnostics("b", &ctx)
        .unwrap();
    assert_eq!(
        fs_json(&b.policy),
        j(r#"{"readonlyPaths":["/n"],"readwritePaths":["/p"]}"#)
    );
    assert!(messages(&b.diagnostics.warnings).contains(
        "tool:b uses its architecture-neutral linux additions; no x64-specific overlay exists"
    ));
}

struct FailingHost;
impl HostEnvironment for FailingHost {
    fn platform(&self) -> Result<Platform, PolicyCatalogError> {
        Ok(Platform::Linux)
    }
    fn native_architecture(&self) -> Result<Architecture, PolicyCatalogError> {
        Err(PolicyCatalogError::new(
            ErrorReason::UnsupportedHost,
            "unknown machine",
        ))
    }
    fn symbol(&self, _: &str) -> Option<String> {
        None
    }
}

#[test]
fn host_architecture_is_detected_only_when_an_overlay_needs_it() {
    let store = Arc::new(
        store_for(&[revision(vec![
            entry("tool:t", ""),
            entry(
                "tool:u",
                r#"{"platformVariants":[{"when":{"platform":"linux","architecture":"x64"}}]}"#,
            ),
        ])])
        .unwrap(),
    );
    let catalog = PolicyCatalog::with_host(store, Arc::new(FailingHost));
    let error = catalog
        .resolve_sandbox_policy("u", &weak().project_root("/p"))
        .unwrap_err();
    assert!(error.message().contains("unknown machine"));
    assert_eq!(error.reason(), ErrorReason::UnsupportedHost);
    let result = catalog
        .resolve_sandbox_policy_with_diagnostics("t", &weak().project_root("/p"))
        .unwrap();
    assert_eq!(fs_json(&result.policy), j(r#"{"readwritePaths":["/p"]}"#));
    assert!(!messages(&result.diagnostics.warnings).contains("architecture"));
    assert_eq!(
        catalog.resolve_sandbox_policy("nothing", &weak()).unwrap(),
        None
    );
    assert!(catalog
        .resolve_sandbox_policy(
            "u",
            &weak().architecture(Architecture::X64).project_root("/p")
        )
        .unwrap()
        .is_some());
}

// ---------------------------------------------------------------------------
// Symbols
// ---------------------------------------------------------------------------

#[test]
fn never_fabricates_project_root_or_caller_symbols() {
    let result = bundled_catalog(linux_x64())
        .resolve_sandbox_policy_with_diagnostics("git", &weak().architecture(Architecture::X64))
        .unwrap();
    assert_eq!(result.policy, None);
    let warnings = messages(&result.diagnostics.warnings);
    let a = warnings.find("required symbol 'git_prefix'").unwrap();
    let b = warnings.find("required symbol 'project_root'").unwrap();
    assert!(a < b);
}

#[test]
fn host_symbols_only_for_the_current_host_platform_and_caller_overrides_win() {
    let rev = revision(vec![entry(
        "tool:t",
        &with_default(
            &format!(
                r#"{{"version":"{V}","filesystem":{{"readonlyPaths":["${{user_home}}/.cfg"]}}}}"#
            ),
            "",
        ),
    )]);
    let host = Arc::new(
        FixedHost::new(Platform::Linux, Architecture::X64).with_symbol("user_home", "/home/me"),
    );
    let catalog = catalog_for(rev, host);
    let derived = catalog
        .resolve_sandbox_policy_with_diagnostics("t", &weak())
        .unwrap();
    assert_eq!(
        fs_json(&derived.policy),
        j(r#"{"readonlyPaths":["/home/me/.cfg"]}"#)
    );
    assert!(messages(&derived.diagnostics.warnings)
        .contains("symbol 'user_home' resolved from the host environment to '/home/me'"));
    assert_eq!(
        catalog
            .resolve_sandbox_policy(
                "t",
                &weak()
                    .platform(Platform::Macos)
                    .architecture(Architecture::Arm64)
            )
            .unwrap(),
        None
    );
    let overridden = catalog
        .resolve_sandbox_policy_with_diagnostics("t", &weak().symbol("user_home", "/srv/u"))
        .unwrap();
    assert_eq!(
        fs_json(&overridden.policy),
        j(r#"{"readonlyPaths":["/srv/u/.cfg"]}"#)
    );
    assert!(!messages(&overridden.diagnostics.warnings).contains("host environment"));
}

#[test]
fn windows_overlay_adds_program_data_git() {
    let ctx = ResolveContext::new()
        .platform(Platform::Windows)
        .architecture(Architecture::X64)
        .project_root("C:\\p")
        .symbol("git_prefix", "C:\\Program Files\\Git")
        .symbol("programData", "C:\\ProgramData");
    let policy = bundled_catalog(linux_x64())
        .resolve_sandbox_policy(
            ToolCandidate::new("git")
                .with_package_url("pkg:generic/git")
                .with_intent("local"),
            &ctx,
        )
        .unwrap();
    assert_eq!(
        fs_json(&policy),
        j(
            r#"{"readonlyPaths":["C:\\Program Files\\Git","C:\\ProgramData\\Git"],"readwritePaths":["C:\\p"]}"#
        )
    );
}

// ---------------------------------------------------------------------------
// Inputs and failures
// ---------------------------------------------------------------------------

fn git_ctx() -> ResolveContext {
    weak()
        .architecture(Architecture::X64)
        .project_root("/p")
        .symbol("git_prefix", "/g")
}

#[test]
fn string_object_and_one_element_array_are_equivalent() {
    let catalog = bundled_catalog(linux_x64());
    let ctx = git_ctx();
    let a = catalog
        .resolve_sandbox_policy_with_diagnostics("git", &ctx)
        .unwrap();
    assert_eq!(
        catalog
            .resolve_sandbox_policy_with_diagnostics(ToolCandidate::new("git"), &ctx)
            .unwrap(),
        a
    );
    assert_eq!(
        catalog
            .resolve_sandbox_policy_with_diagnostics(vec!["git"], &ctx)
            .unwrap(),
        a
    );
    assert_eq!(a.diagnostics.tools[0].input_index, 0);
    let strict = ResolveContext {
        allow_weak_identity_fallback: false,
        ..ctx
    };
    assert_eq!(
        catalog.resolve_sandbox_policy("git", &strict).unwrap(),
        None
    );
    assert_eq!(
        catalog
            .resolve_sandbox_policy(ToolCandidate::new("git").with_intent("fetch"), &strict)
            .unwrap(),
        None
    );
}

#[test]
fn invalid_context_and_inputs_are_failures_not_absence() {
    let catalog = bundled_catalog(linux_x64());
    let cases: Vec<(ToolInputs, ResolveContext)> = vec![
        ("".into(), ResolveContext::new()),
        ("/usr/bin/git".into(), ResolveContext::new()),
        (
            ToolCandidate::new("npm").with_package_url("npm").into(),
            ResolveContext::new(),
        ),
        (
            ToolCandidate::new("npm").with_detected_version("").into(),
            ResolveContext::new(),
        ),
        (
            ToolCandidate::new("npm").with_package_url("").into(),
            ResolveContext::new(),
        ),
        (
            ToolCandidate::new("git").with_intent("").into(),
            ResolveContext::new(),
        ),
        (
            "git".into(),
            ResolveContext {
                platform: Some("plan9".into()),
                ..Default::default()
            },
        ),
        (
            "git".into(),
            ResolveContext {
                architecture: Some("x86".into()),
                ..Default::default()
            },
        ),
        ("git".into(), ResolveContext::new().symbol("nope", "/x")),
        (
            "git".into(),
            ResolveContext::new().symbol("project_root", "/x"),
        ),
        (
            "git".into(),
            ResolveContext::new().symbol("__proto__", "/x"),
        ),
        ("git".into(), ResolveContext::new().project_root("")),
    ];
    for (tools, ctx) in cases {
        assert_eq!(
            reason_of(catalog.resolve_sandbox_policy(tools.clone(), &ctx)),
            ErrorReason::InvalidContext,
            "{tools:?} {ctx:?}"
        );
    }
    for value in ["bin", "/x/${node_prefix}"] {
        let ctx = weak()
            .architecture(Architecture::X64)
            .symbol("node_prefix", value);
        let error = catalog.resolve_sandbox_policy("node", &ctx).unwrap_err();
        assert_eq!(
            error.message(),
            "[malformed_request] symbol 'node_prefix' must resolve to an absolute linux path"
        );
    }
}

#[test]
fn results_are_caller_owned_and_deterministic() {
    let catalog = bundled_catalog(linux_x64());
    let ctx = ResolveContext::new()
        .architecture(Architecture::X64)
        .project_root("/p")
        .symbol("npm_prefix", "/n")
        .symbol("npm_cache", "/c")
        .symbol("node_prefix", "/node");
    let tool = ToolCandidate::new("npm").with_package_url("pkg:npm/npm");
    let mut first = catalog
        .resolve_sandbox_policy_with_diagnostics(tool.clone(), &ctx)
        .unwrap();
    first
        .policy
        .as_mut()
        .unwrap()
        .filesystem
        .as_mut()
        .unwrap()
        .readwrite_paths
        .as_mut()
        .unwrap()
        .push("/mutated".into());
    first
        .diagnostics
        .warnings
        .push("mutated".to_string().into());
    let second = catalog
        .resolve_sandbox_policy_with_diagnostics(tool, &ctx)
        .unwrap();
    assert_eq!(
        fs_json(&second.policy),
        j(r#"{"readonlyPaths":["/n","/node"],"readwritePaths":["/p","/c"]}"#)
    );
    assert!(!messages(&second.diagnostics.warnings).contains("mutated"));
}

#[test]
fn invocation_name_casing_per_platform() {
    let catalog = catalog_for(revision(vec![entry("tool:gh", "")]), linux_x64());
    let on = |platform: Platform, name: &str| -> ToolResolutionStatus {
        catalog
            .resolve_sandbox_policy_with_diagnostics(
                name,
                &weak()
                    .platform(platform)
                    .architecture(Architecture::X64)
                    .project_root(if platform == Platform::Windows {
                        "C:\\p"
                    } else {
                        "/p"
                    }),
            )
            .unwrap()
            .diagnostics
            .tools[0]
            .status
    };
    let matched = ToolResolutionStatus::Version(VersionStatus::MatchedDefault);
    assert_eq!(on(Platform::Linux, "gh"), matched);
    assert_eq!(
        on(Platform::Linux, "GH"),
        ToolResolutionStatus::ToolUnmatched
    );
    assert_eq!(on(Platform::Macos, "GH"), matched);
    assert_eq!(on(Platform::Windows, "Gh"), matched);
}

// ---------------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------------

#[test]
fn package_identity_then_intent_then_architecture_and_ties_are_ambiguous() {
    let named = |id: &str, extra: &str| {
        entry(
            id,
            &format!(r#"{{"identity":[{{"kind":"invocation-name","names":["x"]}}]{extra}}}"#),
        )
    };
    let entries = vec![
        named("tool:a", ""),
        named(
            "tool:b",
            r#","default":{"sandboxPolicy":{"version":"0.9.0-alpha"},"intents":{"build":{}}}"#,
        ),
        entry(
            "tool:c",
            r#"{"identity":[{"kind":"purl","value":"pkg:npm/x"},{"kind":"invocation-name","names":["x"]}],"default":{"sandboxPolicy":{"version":"0.9.0-alpha"}}}"#,
        ),
    ];
    let mut reversed = entries.clone();
    reversed.reverse();
    for list in [entries, reversed] {
        let catalog = catalog_for(revision(list), linux_x64());
        let ctx = weak().project_root("/p");
        let chosen = |tool: ToolCandidate| {
            catalog
                .resolve_sandbox_policy_with_diagnostics(tool, &ctx)
                .unwrap()
                .diagnostics
                .tools[0]
                .matches[0]
                .entry_id
                .clone()
        };
        assert_eq!(
            chosen(ToolCandidate::new("x").with_package_url("pkg:npm/x")),
            "tool:c"
        );
        assert_eq!(
            chosen(ToolCandidate::new("x").with_intent("build")),
            "tool:b"
        );
        let error = catalog.resolve_sandbox_policy("x", &ctx).unwrap_err();
        assert_eq!(error.reason(), ErrorReason::AmbiguousMatch);
        assert_eq!(
            error.message(),
            "[policy_validation] input 0 ('x') matches 3 entries with equal rank (tool:a, tool:b, tool:c); no entry was selected"
        );
    }
}

#[test]
fn exact_architecture_outranks_neutral_and_default() {
    let catalog = catalog_for(
        revision(vec![
            entry(
                "tool:a",
                r#"{"identity":[{"kind":"invocation-name","names":["x"]}],"platformVariants":[{"when":{"platform":"linux"}}]}"#,
            ),
            entry(
                "tool:b",
                r#"{"identity":[{"kind":"invocation-name","names":["x"]}],"platformVariants":[{"when":{"platform":"linux","architecture":"x64"}}]}"#,
            ),
        ]),
        linux_x64(),
    );
    let result = catalog
        .resolve_sandbox_policy_with_diagnostics("x", &weak().project_root("/p"))
        .unwrap();
    assert_eq!(result.diagnostics.tools[0].matches[0].entry_id, "tool:b");
}

#[test]
fn package_url_versions_are_ignored_with_a_warning() {
    let result = bundled_catalog(linux_x64())
        .resolve_sandbox_policy_with_diagnostics(
            ToolCandidate::new("git").with_package_url("pkg:generic/git@2.45.0"),
            &git_ctx(),
        )
        .unwrap();
    let tool = &result.diagnostics.tools[0];
    assert_eq!(
        tool.status,
        ToolResolutionStatus::Version(VersionStatus::MatchedDefault)
    );
    assert!(messages(&result.diagnostics.warnings).contains(
        "input 0 ('git'): the version '2.45.0' embedded in packageUrl is not version evidence and was ignored; supply detectedVersion"
    ));
}

// ---------------------------------------------------------------------------
// Version and intent selection
// ---------------------------------------------------------------------------

fn git(version: Option<&str>, intent: Option<&str>) -> ToolCandidate {
    let mut tool = ToolCandidate::new("git").with_package_url("pkg:generic/git");
    tool.detected_version = version.map(str::to_string);
    tool.intent = intent.map(str::to_string);
    tool
}

fn resolve_git(tools: impl Into<ToolInputs>) -> SandboxConfigResolution {
    bundled_catalog(linux_x64())
        .resolve_sandbox_policy_with_diagnostics(
            tools,
            &git_ctx()
                .symbol("ssh_prefix", "/ssh")
                .symbol("temp_dir", "/tmp"),
        )
        .unwrap()
}

#[test]
fn version_selection_statuses() {
    let none = resolve_git(git(None, Some("push")));
    let record = &none.diagnostics.tools[0].matches[0];
    assert_eq!(
        none.diagnostics.tools[0].status,
        ToolResolutionStatus::Version(VersionStatus::MatchedDefault)
    );
    assert_eq!(record.version_selection.detected_version, None);
    assert!(none.diagnostics.resolved_dependencies.is_empty());

    let inside = resolve_git(git(Some("2.45.1"), Some("push")));
    let record = &inside.diagnostics.tools[0].matches[0];
    assert_eq!(
        record.version_selection.status,
        VersionStatus::MatchedVersion
    );
    assert_eq!(
        record.version_selection.selected_version_range.as_deref(),
        Some("vers:intdot/>=2.40|<2.50")
    );
    assert_eq!(
        inside.diagnostics.resolved_dependencies[0].entry_id,
        "tool:ssh"
    );
    assert_eq!(
        fs_json(&inside.policy),
        j(r#"{"readonlyPaths":["/g","/ssh"],"readwritePaths":["/p"]}"#)
    );
    assert_eq!(
        egress(&inside),
        Some(j(&format!("[{}]", tcp("192.0.2.10/32", 22))))
    );

    let outside = resolve_git(git(Some("2.30.0"), Some("push")));
    assert_eq!(
        outside.diagnostics.tools[0].status,
        ToolResolutionStatus::Version(VersionStatus::VersionOutOfRange)
    );
    assert_eq!(outside.policy, none.policy);
    let Warning::Tool(warning) = &outside.diagnostics.warnings[0] else {
        panic!("structured warning expected");
    };
    assert_eq!(warning.code.as_str(), "version_out_of_range");
    assert_eq!(warning.entry_id.as_deref(), Some("tool:git"));
    assert_eq!(warning.detected_version.as_deref(), Some("2.30.0"));
    assert_eq!(warning.intent.as_deref(), Some("push"));

    let bad = resolve_git(git(Some("banana"), Some("push")));
    assert_eq!(bad.policy, None);
    assert_eq!(
        bad.diagnostics.tools[0].status,
        ToolResolutionStatus::Version(VersionStatus::VersionUnparseable)
    );
    assert_eq!(bad.diagnostics.tools[0].matches[0].intent_selection, None);
}

#[test]
fn intent_selection_and_unsupported_intents() {
    let all = resolve_git(git(Some("2.55"), None));
    let selection = all.diagnostics.tools[0].matches[0]
        .intent_selection
        .clone()
        .unwrap();
    assert_eq!(selection.mode, IntentMode::All);
    assert_eq!(
        selection.selected,
        ["bundle-fetch", "fetch", "local", "push"]
    );
    assert_eq!(
        fs_json(&all.policy),
        j(r#"{"readonlyPaths":["/g"],"readwritePaths":["/p","/tmp/git-bundles"]}"#)
    );
    assert_eq!(
        egress(&all),
        Some(j(&format!(
            "[{},{},{}]",
            tcp("198.51.100.20/32", 443),
            tcp("192.0.2.10/32", 443),
            tcp("192.0.2.10/32", 22)
        )))
    );

    let local = resolve_git(git(None, Some("local")));
    assert_eq!(local.policy.as_ref().unwrap().network, None);

    let unsupported = resolve_git(git(Some("2.45"), Some("bundle-fetch")));
    let tool = &unsupported.diagnostics.tools[0];
    assert_eq!(tool.status, ToolResolutionStatus::IntentUnsupported);
    assert_eq!(
        tool.matches[0].version_selection.status,
        VersionStatus::MatchedVersion
    );
    assert_eq!(
        tool.matches[0].intent_selection.as_ref().unwrap().mode,
        IntentMode::Unsupported
    );
    assert_eq!(unsupported.policy, None);

    let both = resolve_git(git(Some("2.30"), Some("bundle-fetch")));
    let tool = &both.diagnostics.tools[0];
    assert_eq!(tool.status, ToolResolutionStatus::IntentUnsupported);
    assert_eq!(
        tool.matches[0].version_selection.status,
        VersionStatus::VersionOutOfRange
    );
    let codes: Vec<&str> = both
        .diagnostics
        .warnings
        .iter()
        .filter_map(|w| match w {
            Warning::Tool(t) => Some(t.code.as_str()),
            Warning::Text(_) => None,
        })
        .collect();
    assert_eq!(codes, ["version_out_of_range", "intent_unsupported"]);
}

#[test]
fn pairs_compose_and_a_tool_without_network_does_not_veto_another() {
    let result = resolve_git(vec![
        ToolInput::from(git(None, Some("local"))),
        ToolInput::from(git(None, Some("fetch"))),
        ToolInput::from(git(None, Some("push"))),
        ToolInput::from(git(None, Some("fetch"))),
        ToolInput::from("definitely-unknown"),
    ]);
    assert_eq!(
        egress(&result),
        Some(j(&format!(
            "[{},{}]",
            tcp("192.0.2.10/32", 443),
            tcp("192.0.2.10/32", 22)
        )))
    );
    assert_eq!(
        result.diagnostics.tools[4].status,
        ToolResolutionStatus::ToolUnmatched
    );
    assert_eq!(
        fs_json(&result.policy),
        j(r#"{"readonlyPaths":["/g"],"readwritePaths":["/p"]}"#)
    );
}

// ---------------------------------------------------------------------------
// Composition
// ---------------------------------------------------------------------------

fn paths_entry(id: &str, policy_fs: &str) -> Json {
    entry(
        id,
        &with_default(
            &format!(r#"{{"version":"{V}","filesystem":{policy_fs}}}"#),
            "",
        ),
    )
}

fn compose(entries: Vec<Json>, names: Vec<&str>) -> SandboxConfigResolution {
    catalog_for(revision(entries), linux_x64())
        .resolve_sandbox_policy_with_diagnostics(
            names,
            &weak()
                .project_root("/work")
                .symbol("git_prefix", "/data")
                .symbol("node_prefix", "/secrets"),
        )
        .unwrap()
}

#[test]
fn filesystem_floor_composition_rules() {
    let ro_rw = compose(
        vec![
            paths_entry("tool:a", r#"{"readonlyPaths":["${project_root}"]}"#),
            paths_entry("tool:b", r#"{"readwritePaths":["${project_root}"]}"#),
        ],
        vec!["a", "b"],
    );
    assert_eq!(fs_json(&ro_rw.policy), j(r#"{"readwritePaths":["/work"]}"#));
    assert!(messages(&ro_rw.diagnostics.warnings).contains(
        "read-only '/work' (tool:a) is covered by read-write '/work' (tool:b); the read-only entry was omitted"
    ));

    let nested_ro = compose(
        vec![paths_entry(
            "tool:a",
            r#"{"readwritePaths":["${project_root}"],"readonlyPaths":["${project_root}/tools"]}"#,
        )],
        vec!["a"],
    );
    assert_eq!(
        fs_json(&nested_ro.policy),
        j(r#"{"readwritePaths":["/work"]}"#)
    );

    let nested_rw = compose(
        vec![paths_entry(
            "tool:a",
            r#"{"readonlyPaths":["${project_root}"],"readwritePaths":["${project_root}/cache"]}"#,
        )],
        vec!["a"],
    );
    assert_eq!(
        fs_json(&nested_rw.policy),
        j(r#"{"readonlyPaths":["/work"],"readwritePaths":["/work/cache"]}"#)
    );

    let denies = compose(
        vec![
            paths_entry(
                "tool:a",
                r#"{"deniedPaths":["${git_prefix}","${node_prefix}"]}"#,
            ),
            paths_entry(
                "tool:b",
                r#"{"readwritePaths":["${project_root}","${git_prefix}/cache"]}"#,
            ),
        ],
        vec!["a", "b"],
    );
    assert_eq!(
        fs_json(&denies.policy),
        j(r#"{"deniedPaths":["/secrets"],"readwritePaths":["/work","/data/cache"]}"#)
    );
    assert!(messages(&denies.diagnostics.warnings).contains(
        "removed catalog deny '/data' (tool:a) because it overlaps required read-write '/data/cache' (tool:b); the entire deny scope '/data' was removed, so other grants may now apply throughout it"
    ));
}

#[test]
fn unknown_case_sensitivity_keeps_case_variants_with_a_diagnostic() {
    let catalog = catalog_for(
        revision(vec![paths_entry(
            "tool:w",
            r#"{"readonlyPaths":["${git_prefix}","${node_prefix}\\"]}"#,
        )]),
        linux_x64(),
    );
    let ctx = weak()
        .platform(Platform::Windows)
        .architecture(Architecture::X64)
        .project_root("C:\\p")
        .symbol("git_prefix", "C:\\Tools")
        .symbol("node_prefix", "c:\\tools");
    let result = catalog
        .resolve_sandbox_policy_with_diagnostics("w", &ctx)
        .unwrap();
    assert_eq!(
        fs_json(&result.policy),
        j(r#"{"readonlyPaths":["C:\\Tools","c:\\tools"]}"#)
    );
    assert!(messages(&result.diagnostics.warnings).contains(
        "read-only 'C:\\Tools' (tool:w) and read-only 'c:\\tools' (tool:w) differ only by case; filesystem case sensitivity was not determined, so they were compared case-sensitively and kept distinct"
    ));
    let same = catalog
        .resolve_sandbox_policy(
            "w",
            &ctx.symbol("git_prefix", "C:\\tools")
                .symbol("node_prefix", "C:/tools/"),
        )
        .unwrap();
    assert_eq!(fs_json(&same), j(r#"{"readonlyPaths":["C:\\tools"]}"#));
}

#[test]
fn dependency_chain_composes_each_entry_once_in_order() {
    let node = |deps: &[&str], path: &str| {
        let deps = deps
            .iter()
            .map(|d| format!(r#"{{"entryId":"{d}"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            r#"{{"default":{{"sandboxPolicy":{{"version":"{V}","filesystem":{{"readonlyPaths":["${{project_root}}/{path}"]}}}},"dependencies":[{deps}]}}}}"#
        )
    };
    let catalog = catalog_for(
        revision(vec![
            entry("tool:top", &node(&["tool:left", "tool:right"], "top")),
            entry("tool:left", &node(&["tool:leaf"], "left")),
            entry("tool:right", &node(&["tool:leaf"], "right")),
            entry("tool:leaf", &node(&[], "leaf")),
        ]),
        linux_x64(),
    );
    let result = catalog
        .resolve_sandbox_policy_with_diagnostics("top", &weak().project_root("/r"))
        .unwrap();
    let deps: Vec<&str> = result
        .diagnostics
        .resolved_dependencies
        .iter()
        .map(|d| d.entry_id.as_str())
        .collect();
    assert_eq!(deps, ["tool:leaf", "tool:left", "tool:right"]);
    assert_eq!(
        fs_json(&result.policy),
        j(r#"{"readonlyPaths":["/r/top","/r/left","/r/leaf","/r/right"]}"#)
    );
}

#[test]
fn dependency_diagnostics_keep_distinct_ranges_sorted() {
    let a = format!(
        r#"{{"default":{{"sandboxPolicy":{{"version":"{V}"}},"dependencies":[{{"entryId":"tool:c","versionRange":"vers:semver/>=2.0.0"}}]}}}}"#
    );
    let b = format!(
        r#"{{"default":{{"sandboxPolicy":{{"version":"{V}"}},"dependencies":[{{"entryId":"tool:c"}}]}}}}"#
    );
    let catalog = catalog_for(
        revision(vec![
            entry("tool:a", &a),
            entry("tool:b", &b),
            entry(
                "tool:c",
                &format!(
                    r#"{{"default":{{"sandboxPolicy":{{"version":"{V}"}},"intents":{{"run":{{}}}}}}}}"#
                ),
            ),
        ]),
        linux_x64(),
    );
    let result = catalog
        .resolve_sandbox_policy_with_diagnostics(vec!["a", "b", "a"], &weak())
        .unwrap();
    assert_eq!(
        result
            .to_json()
            .get("diagnostics")
            .unwrap()
            .get("resolvedDependencies")
            .unwrap(),
        &j(r#"[
            {"entryId":"tool:c","entryRevision":1,"versionSelection":{"status":"matched_default"},"intentSelection":{"mode":"all","selected":["run"]}},
            {"entryId":"tool:c","entryRevision":1,"requiredVersionRange":"vers:semver/>=2.0.0","versionSelection":{"status":"matched_default"},"intentSelection":{"mode":"all","selected":["run"]}}]"#)
    );
    assert_eq!(
        result.policy.unwrap().to_json(),
        j(&format!(r#"{{"version":"{V}"}}"#))
    );
}

#[test]
fn unsupported_combinations_fail_rather_than_broaden() {
    let catalog = catalog_for(
        revision(vec![
            entry("tool:a", &with_default(r#"{"version":"0.8.0-alpha"}"#, "")),
            entry(
                "tool:b",
                &with_default(&format!(r#"{{"version":"{V}"}}"#), ""),
            ),
            entry(
                "tool:d",
                &with_default(
                    &format!(
                        r#"{{"version":"{V}","network":{{"egress":{{"deny":[{{"ports":[{{"port":22}}]}}]}}}}}}"#
                    ),
                    "",
                ),
            ),
            entry(
                "tool:n",
                &with_default(
                    &format!(
                        r#"{{"version":"{V}","network":{{"egress":{{"default":"deny","allow":[{}]}}}}}}"#,
                        tcp("192.0.2.1/32", 443)
                    ),
                    "",
                ),
            ),
            entry(
                "tool:u",
                &with_default(&format!(r#"{{"version":"{V}","timeoutMs":5}}"#), ""),
            ),
        ]),
        linux_x64(),
    );
    let error = catalog
        .resolve_sandbox_policy(vec!["a", "b"], &weak())
        .unwrap_err();
    assert_eq!(
        error.message(),
        "[policy_validation] selected entries cannot be composed: mixed sandboxPolicy.version values (0.8.0-alpha, 0.9.0-alpha)"
    );
    assert_eq!(
        reason_of(catalog.resolve_sandbox_policy(vec!["d", "n"], &weak())),
        ErrorReason::CompositionConflict
    );
    assert_eq!(
        reason_of(catalog.resolve_sandbox_policy(vec!["u", "b"], &weak())),
        ErrorReason::CompositionConflict
    );
    // A single selected policy without additions passes through whole.
    assert_eq!(
        catalog
            .resolve_sandbox_policy("u", &weak())
            .unwrap()
            .unwrap()
            .to_json(),
        j(&format!(r#"{{"version":"{V}","timeoutMs":5}}"#))
    );
    // Only one component needs network: its network passes through.
    assert_eq!(
        catalog
            .resolve_sandbox_policy(vec!["d", "b"], &weak())
            .unwrap()
            .unwrap()
            .network,
        Some(j(r#"{"egress":{"deny":[{"ports":[{"port":22}]}]}}"#))
    );
}

// ---------------------------------------------------------------------------
// Setup and inspection
// ---------------------------------------------------------------------------

#[test]
fn inspection_over_the_bundled_catalog() {
    let info = get_catalog_info().unwrap();
    assert_eq!(
        info.to_json(),
        j(r#"{"catalogSchemaVersion":"1","catalogRevision":"2026-10-02.1"}"#)
    );
    let entries = list_catalog_entries().unwrap();
    let ids: Vec<&str> = entries.iter().map(|e| e.entry_id.as_str()).collect();
    assert_eq!(ids, ["tool:git", "tool:node", "tool:npm", "tool:ssh"]);
    let git = entries
        .iter()
        .find(|e| e.entry_id == "tool:git")
        .unwrap()
        .to_json();
    assert_eq!(git.get("versionScheme").unwrap(), &j(r#""intdot""#));
    assert_eq!(
        git.get("versionVariants").unwrap(),
        &j(r#"[
          {"versionRange":"vers:intdot/>=2.40|<2.50","dependencyEntryIds":[],"intentAdditions":[{"name":"push","dependencyEntryIds":["tool:ssh"]}],"intents":[]},
          {"versionRange":"vers:intdot/>=2.50|<3","dependencyEntryIds":[],"intentAdditions":[],"intents":[{"name":"bundle-fetch","exampleSubcommands":["fetch --bundle-uri"],"dependencyEntryIds":[]}]}]"#)
    );
    let text = Json::Array(entries.iter().map(|e| e.to_json()).collect()).to_compact_string();
    assert!(!text.contains("Paths") && !text.contains("${") && !text.contains("sandboxPolicy\""));
}

#[test]
fn module_level_functions_use_the_bundled_catalog() {
    let ctx = weak()
        .platform(Platform::Linux)
        .architecture(Architecture::X64);
    assert_eq!(
        resolve_sandbox_policy("definitely-unknown-tool", &ctx).unwrap(),
        None
    );
    let result = resolve_sandbox_policy_with_diagnostics(
        vec![ToolInput::from("definitely-unknown-tool")],
        &ctx,
    )
    .unwrap();
    assert_eq!(result.policy, None);
    assert_eq!(
        result.diagnostics.to_json().get("tools").unwrap(),
        &j(r#"[{"inputIndex":0,"status":"tool_unmatched","matches":[]}]"#)
    );
    assert_eq!(
        reason_of(resolve_sandbox_policy(
            "git",
            &ctx.catalog_revision("1999-01-01.1")
        )),
        ErrorReason::RevisionUnavailable
    );
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

fn dep(id: &str) -> String {
    format!(
        r#"{{"default":{{"sandboxPolicy":{{"version":"{V}"}},"dependencies":[{{"entryId":"{id}"}}]}}}}"#
    )
}

#[test]
fn validation_accepts_minimal_and_rejects_cycles_and_missing_dependencies() {
    validate(vec![entry("tool:a", "")]).unwrap();
    let error = validate(vec![
        entry("tool:a", &dep("tool:b")),
        entry("tool:b", &dep("tool:c")),
        entry("tool:c", &dep("tool:a")),
    ])
    .unwrap_err();
    assert_eq!(
        error.message(),
        "[policy_validation] 'tool:a' on windows/x64 (default) intent (all): dependency resolution failed: cycle (tool:a -> tool:b -> tool:c -> tool:a)"
    );
    validate_err(vec![entry("tool:a", &dep("tool:a"))], "depends on itself");
    validate_err(
        vec![entry("tool:a", &dep("tool:missing"))],
        "unknown entry 'tool:missing'",
    );
    let ranged = |range: &str| {
        format!(
            r#"{{"default":{{"sandboxPolicy":{{"version":"{V}"}},"dependencies":[{{"entryId":"tool:b","versionRange":"{range}"}}]}}}}"#
        )
    };
    validate_err(
        vec![entry("tool:a", &ranged("^1.x")), entry("tool:b", "")],
        "is not a valid vers range",
    );
    validate_err(
        vec![
            entry("tool:a", &ranged("vers:npm/>=1.0.0")),
            entry("tool:b", ""),
        ],
        "does not use that entry's versionScheme 'semver'",
    );
    validate(vec![
        entry("tool:a", &ranged("vers:semver/>=1.0.0")),
        entry("tool:b", ""),
    ])
    .unwrap();
}

#[test]
fn validation_requires_exactly_one_default_and_a_scheme() {
    let mut no_default = entry("tool:a", "");
    if let Json::Object(o) = &mut no_default {
        o.remove("default");
    }
    validate_err(
        vec![no_default],
        "every entry has exactly one unversioned default",
    );
    validate_err(
        vec![entry("tool:a", r#"{"versionScheme":"maven"}"#)],
        "'entries[0].versionScheme' must be one of npm, semver, pypi, nuget, intdot",
    );
    validate_err(
        vec![entry(
            "tool:a",
            r#"{"default":{"sandboxPolicy":{"version":"0.9.0-alpha"},"versionRange":"vers:semver/>=1"}}"#,
        )],
        "unsupported field 'entries[0].default.versionRange'",
    );
}

#[test]
fn validation_selectors() {
    let v = |arch: &str| {
        let a = if arch.is_empty() {
            String::new()
        } else {
            format!(r#","architecture":"{arch}""#)
        };
        format!(r#"{{"when":{{"platform":"linux"{a}}}}}"#)
    };
    let variants = |list: &[String]| format!(r#"{{"platformVariants":[{}]}}"#, list.join(","));
    validate_err(
        vec![entry("tool:a", &variants(&[v("x64"), v("x64")]))],
        "duplicates selector",
    );
    validate_err(
        vec![entry("tool:a", &variants(&[v(""), v("")]))],
        "second architecture-neutral",
    );
    validate(vec![entry(
        "tool:a",
        &variants(&[v(""), v("x64"), v("arm64")]),
    )])
    .unwrap();
    validate_err(
        vec![entry("tool:a", &variants(&[v("riscv")]))],
        "'entries[0].platformVariants[0].when.architecture' must be one of x64, arm64",
    );
    validate_err(
        vec![entry(
            "tool:a",
            &format!(
                r#"{{"platformVariants":[{{"when":{{"platform":"linux"}},"sandboxPolicy":{{"version":"{V}"}}}}]}}"#
            ),
        )],
        "unsupported field 'entries[0].platformVariants[0].sandboxPolicy'",
    );
}

#[test]
fn validation_policy_rules() {
    let p = |policy: &str| vec![entry("tool:a", &with_default(policy, ""))];
    validate_err(
        p(&format!(r#"{{"version":"{V}","processContainer":{{}}}}"#)),
        "containment backend",
    );
    validate_err(
        p(&format!(r#"{{"version":"{V}","containment":"lxc"}}"#)),
        "containment backend",
    );
    validate_err(
        p(r#"{"version":"0.9.0"}"#),
        "not a SandboxPolicy version registered",
    );
    validate_err(
        vec![entry("tool:a", r#"{"extra":1}"#)],
        "unsupported field 'entries[0].extra'",
    );
    validate_err(
        p(&format!(
            r#"{{"version":"{V}","filesystem":{{"clearPolicyOnExit":true}}}}"#
        )),
        "unsupported field",
    );
    let path = |value: &str| {
        p(&format!(
            r#"{{"version":"{V}","filesystem":{{"readonlyPaths":[{}]}}}}"#,
            Json::from(value)
        ))
    };
    validate_err(path("/home/alice/.npm"), "literal paths are not allowed");
    validate_err(path("C:\\Users\\alice"), "literal paths are not allowed");
    validate_err(path("${project_root}/*"), "wildcard");
    validate_err(path("${project_root}/../x"), "'..'");
    validate_err(path("${unknown_thing}"), "unknown symbol 'unknown_thing'");
    validate_err(path("${project_root"), "malformed symbol");
    validate_err(path("${project_root}x"), "literal paths are not allowed");
    validate(path("${project_root}/node_modules")).unwrap();
    validate(path("${programData}\\Git")).unwrap();
    let net = |n: &str| p(&format!(r#"{{"version":"{V}","network":{n}}}"#));
    validate_err(net(r#"{"egress":{"default":"allow"}}"#), "default-allow");
    validate_err(net(r#"{"ingress":{"default":"allow"}}"#), "default-allow");
    validate_err(
        net(r#"{"egress":{"allow":[{"ports":[{"port":443}]}]}}"#),
        "wildcard network grant",
    );
    validate_err(
        net(r#"{"egress":{"allow":[{"to":[{"cidr":"0.0.0.0/0"}]}]}}"#),
        "wildcard network grant",
    );
    validate_err(
        net(
            r#"{"egress":{"allow":[{"to":[{"cidr":"10.0.0.0/8"}],"ports":[{"port":10,"endPort":5}]}]}}"#,
        ),
        "requires a lower or equal 'port'",
    );
    validate(net(r#"{"egress":{"deny":[{"ports":[{"port":22}]}]}}"#)).unwrap();
    validate_err(
        p(&format!(r#"{{"version":"{V}","timeoutMs":1.5}}"#)),
        "timeoutMs' must be a positive integer",
    );
    validate_err(
        p(&format!(r#"{{"version":"{V}","ui":{{"clipboard":"x"}}}}"#)),
        "clipboard' is unsupported",
    );
}

#[test]
fn validation_identity_rules() {
    validate(vec![
        entry("tool:a", ""),
        entry(
            "tool:b",
            r#"{"identity":[{"kind":"invocation-name","names":["A"]}]}"#,
        ),
    ])
    .unwrap();
    validate_err(
        vec![entry(
            "tool:a",
            r#"{"identity":[{"kind":"invocation-name","names":["a","A"]}]}"#,
        )],
        "repeats identity 'invocation-name:a'",
    );
    validate_err(
        vec![entry(
            "tool:a",
            r#"{"identity":[{"kind":"purl","value":"pkg:npm/a"},{"kind":"purl","value":"pkg:NPM/a"}]}"#,
        )],
        "repeats identity 'purl:npm/a'",
    );
    validate_err(
        vec![entry(
            "tool:a",
            r#"{"identity":[{"kind":"purl","value":"pkg:npm/a@1.0.0"}]}"#,
        )],
        "must not pin a version",
    );
    validate_err(
        vec![entry(
            "tool:a",
            r#"{"identity":[{"kind":"purl","value":"pkg:npm/a","versionRange":">=1"}]}"#,
        )],
        "unsupported field 'entries[0].identity[0].versionRange'",
    );
    validate_err(
        vec![entry(
            "tool:a",
            r#"{"identity":[{"kind":"invocation-name","names":["bin/a"]}]}"#,
        )],
        "bare invocation name",
    );
    validate_err(
        vec![
            entry("tool:a", ""),
            entry(
                "tool:a",
                r#"{"identity":[{"kind":"invocation-name","names":["z"]}]}"#,
            ),
        ],
        "duplicate entryId",
    );
    validate_err(
        vec![entry("tool:a", r#"{"entryId":"noNamespace"}"#)],
        "must be namespaced",
    );
}

#[test]
fn validation_overlays_are_additive_only() {
    let overlay = |body: &str| {
        vec![entry(
            "tool:a",
            &format!(
                r#"{{"default":{{"sandboxPolicy":{{"version":"{V}"}},"intents":{{"run":{{}}}}}},"platformVariants":[{{"when":{{"platform":"linux"}},{body}}}]}}"#
            ),
        )]
    };
    validate(overlay(
        r#""policyAdditions":{"filesystem":{"readonlyPaths":["${git_prefix}"]}}"#,
    ))
    .unwrap();
    validate_err(
        overlay(r#""policyAdditions":{"filesystem":{"deniedPaths":["${git_prefix}"]}}"#),
        "'entries[0].platformVariants[0].policyAdditions.filesystem.deniedPaths' is not additive",
    );
    validate_err(
        overlay(r#""policyAdditions":{"network":{"egress":{"deny":[]}}}"#),
        "'entries[0].platformVariants[0].policyAdditions.network.egress.deny' is not additive",
    );
    validate_err(
        overlay(r#""policyAdditions":{"timeoutMs":5}"#),
        "is not additive",
    );
    validate_err(
        overlay(r#""intentAdditions":{"other":{}}"#),
        "extends an intent the default does not declare",
    );
    validate_err(
        overlay(r#""intents":{"run":{}}"#),
        "redeclares an inherited intent",
    );
    validate_err(
        overlay(r#""intents":{"Run":{}}"#),
        "is not a valid intent name",
    );
    validate_err(
        overlay(r#""intents":{"x":{"exampleSubcommands":[]}}"#),
        "exampleSubcommands",
    );
}

#[test]
fn validation_version_variants() {
    let ranges = |list: &[&str]| {
        let variants = list
            .iter()
            .map(|r| format!(r#"{{"versionRange":"{r}"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        vec![entry(
            "tool:a",
            &format!(r#"{{"versionVariants":[{variants}]}}"#),
        )]
    };
    validate(ranges(&[
        "vers:semver/>=1.0.0|<2.0.0",
        "vers:semver/>=2.0.0",
    ]))
    .unwrap();
    validate_err(
        ranges(&["vers:semver/>=1.0.0|<2.0.0", "vers:semver/>=1.5.0"]),
        "overlaps 'entries[0].versionVariants[0].versionRange'",
    );
    validate_err(
        ranges(&["vers:npm/>=1.0.0"]),
        "uses 'npm', but the entry's versionScheme is 'semver'",
    );
    validate_err(ranges(&[">=1.0.0"]), "is not a valid vers range");
    validate_err(
        vec![entry(
            "tool:a",
            r#"{"platformVariants":[{"when":{"platform":"linux"},"intents":{"x":{}}}],
                "versionVariants":[{"versionRange":"vers:semver/>=1.0.0","intents":{"x":{}}}]}"#,
        )],
        "'tool:a' on linux/x64 (vers:semver/>=1.0.0): intent 'x' from version range 'vers:semver/>=1.0.0' is already declared by another overlay",
    );
}

#[test]
fn validation_materializes_composition_limits() {
    let err = |entries: Vec<Json>, needle: &str| {
        let m = validate(entries).unwrap_err().message().to_string();
        assert!(m.contains(needle), "{m}");
    };
    err(
        vec![
            entry("tool:a", &dep("tool:b")),
            entry("tool:b", &with_default(r#"{"version":"0.8.0-alpha"}"#, "")),
        ],
        "mixed sandboxPolicy.version",
    );
    err(
        vec![
            entry("tool:a", &dep("tool:b")),
            entry(
                "tool:b",
                &with_default(&format!(r#"{{"version":"{V}","timeoutMs":5}}"#), ""),
            ),
        ],
        "'tool:b' uses 'timeoutMs', which has no v1 cross-policy composition rule",
    );
    err(
        vec![entry(
            "tool:a",
            &format!(
                r#"{{"default":{{"sandboxPolicy":{{"version":"{V}","network":{{"egress":{{"deny":[{{"ports":[{{"port":22}}]}}]}}}}}},
                     "intents":{{"net":{{"policyAdditions":{{"network":{{"egress":{{"allow":[{}]}}}}}}}}}}}}}}"#,
                tcp("192.0.2.1/32", 443)
            ),
        )],
        "'tool:a' uses 'network.egress.deny', which cannot be combined with other network requirements",
    );
}

// ---------------------------------------------------------------------------
// Store and history
// ---------------------------------------------------------------------------

#[test]
fn bundled_store_verifies_and_canonical_json_ignores_formatting() {
    let store = mxc_policy_store::bundled_catalog_store().unwrap();
    assert!(check_store_history(&store).is_empty());
    assert_eq!(
        store.revision(None).unwrap().catalog_revision,
        store.default_revision()
    );
    assert_eq!(
        canonical_json(&j(r#"{"b":1,"a":[2,{"d":3,"c":4}]}"#)),
        r#"{"a":[2,{"c":4,"d":3}],"b":1}"#
    );
    assert_eq!(
        canonical_json(&j(r#"{"a":1,"b":2}"#)),
        canonical_json(&j(r#"{"b":2,"a":1}"#))
    );
}

#[test]
fn unreadable_revision_is_an_integrity_error_everywhere() {
    let source = mxc_policy_store::MemorySource {
        files: HashMap::new(),
        ..mxc_policy_store::MemorySource::publishing(
            contract(),
            &[revision(vec![entry("tool:a", "")])],
            None,
        )
    };
    let store = Arc::new(mxc_policy_store::CatalogStore::new(source).unwrap());
    assert_eq!(reason_of(store.revision(None)), ErrorReason::Integrity);
    let catalog = PolicyCatalog::with_host(store, linux_x64());
    assert_eq!(
        reason_of(catalog.resolve_sandbox_policy("a", &weak())),
        ErrorReason::Integrity
    );
    assert_eq!(
        reason_of(catalog.resolve_sandbox_policy_with_diagnostics("a", &weak())),
        ErrorReason::Integrity
    );
    assert_eq!(
        reason_of(catalog.list_catalog_entries()),
        ErrorReason::Integrity
    );
    assert_eq!(
        reason_of(catalog.get_catalog_info()),
        ErrorReason::Integrity
    );
}

#[test]
fn revision_id_mismatch_and_invalid_manifest() {
    let other = revision_with(vec![entry("tool:a", "")], "2000-01-02.1");
    let mut relabeled = other.clone();
    if let Json::Object(o) = &mut relabeled {
        o.insert("catalogRevision", "2000-01-01.1".into());
    }
    // A file that declares another revision id → integrity.
    let source = mxc_policy_store::MemorySource {
        files: HashMap::from([("revisions/2000-01-01.1.json".to_string(), other.clone())]),
        ..mxc_policy_store::MemorySource::publishing(contract(), &[relabeled], None)
    };
    let error = mxc_policy_store::CatalogStore::new(source)
        .unwrap()
        .revision(None)
        .unwrap_err();
    assert_eq!(
        error.message(),
        "[backend_error] file 'revisions/2000-01-01.1.json' declares revision '2000-01-02.1', expected '2000-01-01.1'"
    );
    let r = revision(vec![entry("tool:a", "")]);
    assert_eq!(
        reason_of(store_with(std::slice::from_ref(&r), Some("2001-01-01.1"))),
        ErrorReason::InvalidCatalog
    );
    assert_eq!(
        reason_of(store_for(&[
            revision_with(vec![entry("tool:a", "")], "2000-01-02.1"),
            r
        ])),
        ErrorReason::InvalidCatalog
    );
}

#[test]
fn explicit_revisions_are_never_substituted() {
    let r1 = revision_with(
        vec![entry("tool:a", ""), entry("tool:b", "")],
        "2000-01-01.1",
    );
    let r2 = revision_with(
        vec![
            entry("tool:a", r#"{"entryRevision":2,"displayName":"renamed"}"#),
            entry("tool:b", ""),
        ],
        "2000-01-02.1",
    );
    let catalog = PolicyCatalog::with_host(Arc::new(store_for(&[r1, r2]).unwrap()), linux_x64());
    let ctx = weak().project_root("/p");
    let of = |ctx: &ResolveContext| {
        let r = catalog
            .resolve_sandbox_policy_with_diagnostics("a", ctx)
            .unwrap();
        (
            r.diagnostics.catalog_revision.clone(),
            r.diagnostics.tools[0].matches[0].entry_revision,
        )
    };
    assert_eq!(of(&ctx), ("2000-01-02.1".into(), 2.0));
    assert_eq!(
        of(&ctx.clone().catalog_revision("2000-01-01.1")),
        ("2000-01-01.1".into(), 1.0)
    );
    let error = catalog
        .resolve_sandbox_policy("a", &ctx.catalog_revision("2000-01-03.1"))
        .unwrap_err();
    assert_eq!(
        error.message(),
        "[backend_error] catalog revision '2000-01-03.1' is not installed"
    );
}

#[test]
fn entry_revision_history() {
    let contract = validate_contract(&contract()).unwrap();
    let v = |r: Json| validate_catalog_revision(&r, &contract).unwrap();
    let base = v(revision_with(
        vec![entry("tool:a", ""), entry("tool:b", "")],
        "2000-01-01.1",
    ));
    let changed = v(revision_with(
        vec![
            entry("tool:a", r#"{"displayName":"x"}"#),
            entry("tool:b", ""),
        ],
        "2000-01-02.1",
    ));
    let bumped = v(revision_with(
        vec![
            entry("tool:a", ""),
            entry("tool:b", r#"{"entryRevision":2}"#),
        ],
        "2000-01-02.1",
    ));
    let ok = v(revision_with(
        vec![
            entry("tool:a", r#"{"displayName":"x","entryRevision":2}"#),
            entry("tool:b", ""),
        ],
        "2000-01-02.1",
    ));
    assert_eq!(
        check_entry_revisions(&base, &changed),
        ["2000-01-02.1: 'tool:a' changed but entryRevision did not increase (1 -> 1)"]
    );
    assert_eq!(
        check_entry_revisions(&base, &bumped),
        ["2000-01-02.1: 'tool:b' is unchanged but entryRevision moved (1 -> 2)"]
    );
    assert!(check_entry_revisions(&base, &ok).is_empty());
    assert!(check_entry_revisions(&ok, &base)[0].contains("must be newer"));
}

#[test]
fn published_immutability() {
    let published = PublishedRevision::new("2000-01-01.1", "revisions/2000-01-01.1.json");
    let files = HashMap::from([(
        "revisions/2000-01-01.1.json".to_string(),
        "{\"x\":1}\n".to_string(),
    )]);
    let base = PublishedState {
        revisions: vec![published.clone()],
        files: files.clone(),
    };
    let appended = PublishedState {
        revisions: vec![
            published.clone(),
            PublishedRevision::new("2000-01-02.1", "revisions/2000-01-02.1.json"),
        ],
        files: HashMap::from([(
            "revisions/2000-01-01.1.json".to_string(),
            "{\r\n \"x\": 1}".to_string(),
        )]),
    };
    assert!(check_published_immutability(&base, &appended).is_empty());
    let edited = PublishedState {
        revisions: vec![published.clone()],
        files: HashMap::from([(
            "revisions/2000-01-01.1.json".to_string(),
            "{\"x\":2}\n".to_string(),
        )]),
    };
    assert_eq!(
        check_published_immutability(&base, &edited),
        ["published revision file 'revisions/2000-01-01.1.json' was modified; publish a new revision instead"]
    );
    let moved = PublishedState {
        revisions: vec![PublishedRevision::new(
            "2000-01-01.1",
            "revisions/moved.json",
        )],
        files,
    };
    assert_eq!(
        check_published_immutability(&base, &moved),
        ["published revision '2000-01-01.1' manifest entry was modified"]
    );
    let removed = PublishedState::default();
    assert_eq!(
        check_published_immutability(&base, &removed),
        ["published revision '2000-01-01.1' was removed or reordered in the manifest"]
    );
}

#[test]
fn error_code_for_every_reason() {
    for reason in ErrorReason::ALL {
        let error = PolicyCatalogError::new(reason, "m");
        assert_eq!(error.message(), format!("[{}] m", reason.code().as_str()));
    }
}
