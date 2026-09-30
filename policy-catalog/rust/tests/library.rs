// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Ports of the TypeScript unit suites (`tests/unit/resolver.test.ts`,
//! `validation.test.ts`, `store.test.ts`).

mod common;

use common::*;
use mxc_policy_catalog::tooling::{
    canonical_json, canonical_sha256, check_entry_revisions, check_published_immutability, check_store_history,
    validate_catalog_revision, validate_contract, Json, PublishedRevision, PublishedState,
};
use mxc_policy_catalog::{
    get_catalog_info, get_sandbox_config, get_sandbox_config_with_diagnostics, list_catalog_entries, Architecture,
    ErrorReason, FixedHost, HostEnvironment, Platform, PolicyCatalog, PolicyCatalogError, ResolveContext,
    ToolCandidate, ToolInput, ToolInputs,
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

fn linux(policy: &str, extra: &str) -> String {
    let extra = if extra.is_empty() {
        String::new()
    } else {
        format!(",{extra}")
    };
    format!(r#"{{"when":{{"platform":"linux"}}{extra},"sandboxPolicy":{policy}}}"#)
}

fn variants(list: &[String]) -> String {
    format!(r#"{{"platformVariants":[{}]}}"#, list.join(","))
}

fn fs_paths(policy: &Option<mxc_policy_catalog::SandboxPolicy>) -> Json {
    policy.as_ref().unwrap().filesystem.as_ref().unwrap().to_json()
}

// ---------------------------------------------------------------------------
// Runtime lookup: defaults
// ---------------------------------------------------------------------------

fn arch_catalog(host: Arc<FixedHost>) -> PolicyCatalog {
    catalog_for(
        revision(vec![entry(
            "tool:a",
            &format!(
                r#"{{"identity":[{{"kind":"purl","value":"pkg:npm/a"}},{{"kind":"invocation-name","names":["a"]}}],
                "platformVariants":[
                  {{"when":{{"platform":"windows","architecture":"x64"}},"sandboxPolicy":{{"version":"{V}","timeoutMs":1}}}},
                  {{"when":{{"platform":"windows","architecture":"arm64"}},"sandboxPolicy":{{"version":"{V}","timeoutMs":2}}}}]}}"#
            ),
        )]),
        host,
    )
}

#[test]
fn omitted_context_uses_host_platform_native_arch_default_revision_no_weak() {
    let catalog = arch_catalog(fixed_host(Platform::Windows, Architecture::Arm64));
    assert_eq!(catalog.get_sandbox_config("a", &ResolveContext::new()).unwrap(), None);
    let result = catalog
        .get_sandbox_config_with_diagnostics(
            ToolCandidate::new("a").with_package_url("pkg:npm/a"),
            &ResolveContext::new(),
        )
        .unwrap();
    assert_eq!(result.policy.unwrap().timeout_ms, Some(2.0));
    assert_eq!(result.diagnostics.catalog_revision, "2000-01-01.1");
    assert!(result
        .diagnostics
        .warnings
        .join("\n")
        .contains("native system architecture 'arm64'; the tool's architecture was not verified"));
}

#[test]
fn explicit_architecture_wins_and_suppresses_host_default_warning() {
    let catalog = arch_catalog(fixed_host(Platform::Windows, Architecture::Arm64));
    let result = catalog
        .get_sandbox_config_with_diagnostics(
            ToolCandidate::new("a").with_package_url("pkg:npm/a"),
            &ResolveContext::new().architecture(Architecture::X64),
        )
        .unwrap();
    assert_eq!(result.policy.unwrap().timeout_ms, Some(1.0));
    assert!(result.diagnostics.warnings.is_empty());
}

#[test]
fn another_architecture_is_never_a_fallback() {
    let catalog = catalog_for(
        revision(vec![entry(
            "tool:a",
            &format!(
                r#"{{"platformVariants":[{{"when":{{"platform":"linux","architecture":"arm64"}},"sandboxPolicy":{{"version":"{V}"}}}}]}}"#
            ),
        )]),
        linux_x64(),
    );
    let result = catalog
        .get_sandbox_config_with_diagnostics("a", &weak().architecture(Architecture::X64))
        .unwrap();
    assert_eq!(result.policy, None);
    assert!(result.diagnostics.warnings[0].contains("tool:a has no variant for linux/x64"));
}

struct FailingHost;
impl HostEnvironment for FailingHost {
    fn platform(&self) -> Result<Platform, PolicyCatalogError> {
        Ok(Platform::Linux)
    }
    fn native_architecture(&self) -> Result<Architecture, PolicyCatalogError> {
        Err(PolicyCatalogError::new(ErrorReason::UnsupportedHost, "unknown machine"))
    }
    fn symbol(&self, _: &str) -> Option<String> {
        None
    }
}

#[test]
fn host_detection_failure_is_an_error_and_only_attempted_when_needed() {
    let store = Arc::new(store_for(&[revision(vec![entry("tool:t", "")])]).unwrap());
    let catalog = PolicyCatalog::with_host(store, Arc::new(FailingHost));
    let error = catalog.get_sandbox_config("t", &weak().project_root("/p")).unwrap_err();
    assert!(error.message().contains("unknown machine"));
    assert_eq!(error.reason(), ErrorReason::UnsupportedHost);
    assert_eq!(catalog.get_sandbox_config("nothing", &weak()).unwrap(), None);
    let policy = catalog
        .get_sandbox_config("t", &weak().architecture(Architecture::X64).project_root("/p"))
        .unwrap();
    assert_eq!(fs_paths(&policy), j(r#"{"readwritePaths":["/p"]}"#));
}

#[test]
fn never_fabricates_project_root_or_caller_symbols() {
    let result = bundled_catalog(linux_x64())
        .get_sandbox_config_with_diagnostics("git", &weak().architecture(Architecture::X64))
        .unwrap();
    assert_eq!(result.policy, None);
    let warnings = result.diagnostics.warnings.join("\n");
    let a = warnings.find("required symbol 'git_prefix'").unwrap();
    let b = warnings.find("required symbol 'project_root'").unwrap();
    assert!(a < b);
}

#[test]
fn host_symbols_only_for_current_host_platform_and_caller_overrides() {
    let rev = revision(vec![entry(
        "tool:t",
        &format!(
            r#"{{"platformVariants":[
            {{"when":{{"platform":"linux"}},"sandboxPolicy":{{"version":"{V}","filesystem":{{"readonlyPaths":["${{user_home}}/.cfg"]}}}}}},
            {{"when":{{"platform":"macos"}},"sandboxPolicy":{{"version":"{V}","filesystem":{{"readonlyPaths":["${{user_home}}/.cfg"]}}}}}}]}}"#
        ),
    )]);
    let host = Arc::new(FixedHost::new(Platform::Linux, Architecture::X64).with_symbol("user_home", "/home/me"));
    let catalog = catalog_for(rev, host);
    assert_eq!(
        fs_paths(&catalog.get_sandbox_config("t", &weak()).unwrap()),
        j(r#"{"readonlyPaths":["/home/me/.cfg"]}"#)
    );
    assert_eq!(
        catalog
            .get_sandbox_config("t", &weak().platform(Platform::Macos).architecture(Architecture::Arm64))
            .unwrap(),
        None
    );
    assert_eq!(
        fs_paths(
            &catalog
                .get_sandbox_config("t", &weak().symbol("user_home", "/srv/u"))
                .unwrap()
        ),
        j(r#"{"readonlyPaths":["/srv/u/.cfg"]}"#)
    );
}

// ---------------------------------------------------------------------------
// Inputs and failures
// ---------------------------------------------------------------------------

#[test]
fn string_object_and_one_element_array_are_equivalent() {
    let catalog = bundled_catalog(linux_x64());
    let ctx = weak()
        .architecture(Architecture::X64)
        .project_root("/p")
        .symbol("git_prefix", "/g");
    let a = catalog.get_sandbox_config_with_diagnostics("git", &ctx).unwrap();
    assert_eq!(
        catalog
            .get_sandbox_config_with_diagnostics(ToolCandidate::new("git"), &ctx)
            .unwrap(),
        a
    );
    assert_eq!(
        catalog.get_sandbox_config_with_diagnostics(vec!["git"], &ctx).unwrap(),
        a
    );
    assert_eq!(a.diagnostics.tools[0].input_index, 0);
    let strict = ResolveContext::new()
        .architecture(Architecture::X64)
        .project_root("/p")
        .symbol("git_prefix", "/g");
    assert_eq!(catalog.get_sandbox_config("git", &strict).unwrap(), None);
    assert_eq!(
        catalog.get_sandbox_config(ToolCandidate::new("git"), &strict).unwrap(),
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
        ("git".into(), ResolveContext::new().symbol("project_root", "/x")),
        ("git".into(), ResolveContext::new().symbol("__proto__", "/x")),
        ("git".into(), ResolveContext::new().project_root("")),
    ];
    for (tools, ctx) in cases {
        assert_eq!(
            reason_of(catalog.get_sandbox_config(tools.clone(), &ctx)),
            ErrorReason::InvalidContext,
            "{tools:?} {ctx:?}"
        );
    }
    // A relative or `${`-containing symbol value is rejected once it is needed.
    for value in ["bin", "/x/${node_prefix}"] {
        let ctx = weak().architecture(Architecture::X64).symbol("node_prefix", value);
        let error = catalog.get_sandbox_config("node", &ctx).unwrap_err();
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
        .symbol("node_prefix", "/n");
    let tool = ToolCandidate::new("npm").with_package_url("pkg:npm/npm");
    let mut first = catalog.get_sandbox_config_with_diagnostics(tool.clone(), &ctx).unwrap();
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
    first.diagnostics.warnings.push("mutated".into());
    let second = catalog.get_sandbox_config_with_diagnostics(tool, &ctx).unwrap();
    assert_eq!(
        fs_paths(&second.policy),
        j(r#"{"readonlyPaths":["/n"],"readwritePaths":["/p","/c"]}"#)
    );
    assert!(!second.diagnostics.warnings.contains(&"mutated".to_string()));
}

#[test]
fn dedupes_normalized_paths_with_platform_casing() {
    let windows = catalog_for(
        revision(vec![entry(
            "tool:w",
            &format!(
                r#"{{"platformVariants":[{{"when":{{"platform":"windows"}},"sandboxPolicy":{{"version":"{V}","filesystem":{{"readonlyPaths":["${{git_prefix}}","${{node_prefix}}\\"]}}}}}}]}}"#
            ),
        )]),
        linux_x64(),
    );
    let ctx = weak()
        .platform(Platform::Windows)
        .architecture(Architecture::X64)
        .symbol("git_prefix", "C:\\Tools")
        .symbol("node_prefix", "c:\\tools");
    assert_eq!(
        fs_paths(&windows.get_sandbox_config("w", &ctx).unwrap()),
        j(r#"{"readonlyPaths":["C:\\Tools"]}"#)
    );
    let tmpl = |platform: &str, id: &str| {
        entry(
            id,
            &format!(
                r#"{{"platformVariants":[{{"when":{{"platform":"{platform}"}},"sandboxPolicy":{{"version":"{V}","filesystem":{{"readonlyPaths":["${{git_prefix}}","${{node_prefix}}/"]}}}}}}]}}"#
            ),
        )
    };
    let linux = catalog_for(revision(vec![tmpl("linux", "tool:l")]), linux_x64());
    let ctx = weak().symbol("git_prefix", "/Tools").symbol("node_prefix", "/tools");
    assert_eq!(
        fs_paths(&linux.get_sandbox_config("l", &ctx).unwrap()),
        j(r#"{"readonlyPaths":["/Tools","/tools"]}"#)
    );
    let mac = catalog_for(revision(vec![tmpl("macos", "tool:m")]), linux_x64());
    let ctx = ctx.platform(Platform::Macos).architecture(Architecture::Arm64);
    assert_eq!(
        fs_paths(&mac.get_sandbox_config("m", &ctx).unwrap()),
        j(r#"{"readonlyPaths":["/Tools"]}"#)
    );
}

#[test]
fn overlap_detection_follows_platform_casing() {
    let make = |platform: &str| {
        catalog_for(
            revision(vec![entry(
                "tool:m",
                &format!(
                    r#"{{"platformVariants":[{{"when":{{"platform":"{platform}"}},"sandboxPolicy":{{"version":"{V}","filesystem":{{"readonlyPaths":["${{git_prefix}}"],"readwritePaths":["${{project_root}}"]}}}}}}]}}"#
                ),
            )]),
            linux_x64(),
        )
    };
    let ctx = weak()
        .architecture(Architecture::X64)
        .project_root("/tools/work")
        .symbol("git_prefix", "/Tools");
    assert_eq!(
        reason_of(make("macos").get_sandbox_config("m", &ctx.clone().platform(Platform::Macos))),
        ErrorReason::CompositionConflict
    );
    assert_eq!(
        fs_paths(
            &make("linux")
                .get_sandbox_config("m", &ctx.platform(Platform::Linux))
                .unwrap()
        ),
        j(r#"{"readonlyPaths":["/Tools"],"readwritePaths":["/tools/work"]}"#)
    );
}

#[test]
fn invocation_name_casing_per_platform() {
    let variants_all = ["linux", "macos", "windows"]
        .iter()
        .map(|p| format!(r#"{{"when":{{"platform":"{p}"}},"sandboxPolicy":{{"version":"{V}"}}}}"#))
        .collect::<Vec<_>>()
        .join(",");
    let catalog = catalog_for(
        revision(vec![entry(
            "tool:gh",
            &format!(r#"{{"platformVariants":[{variants_all}]}}"#),
        )]),
        linux_x64(),
    );
    let on = |platform: Platform, name: &str| -> Vec<String> {
        catalog
            .get_sandbox_config_with_diagnostics(name, &weak().platform(platform).architecture(Architecture::X64))
            .unwrap()
            .diagnostics
            .tools[0]
            .matches
            .iter()
            .map(|m| m.entry_id.clone())
            .collect()
    };
    assert_eq!(on(Platform::Linux, "gh"), ["tool:gh"]);
    assert!(on(Platform::Linux, "GH").is_empty());
    assert_eq!(on(Platform::Macos, "GH"), ["tool:gh"]);
    assert_eq!(on(Platform::Windows, "Gh"), ["tool:gh"]);
}

#[test]
fn dependency_chain_composes_each_entry_once_in_order() {
    let variant = |deps: &[&str], path: &str| {
        let deps = if deps.is_empty() {
            String::new()
        } else {
            format!(
                r#","dependencies":[{}]"#,
                deps.iter()
                    .map(|d| format!(r#"{{"entryId":"{d}"}}"#))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        format!(
            r#"{{"platformVariants":[{{"when":{{"platform":"linux"}}{deps},"sandboxPolicy":{{"version":"{V}","filesystem":{{"readonlyPaths":["${{project_root}}/{path}"]}}}}}}]}}"#
        )
    };
    let catalog = catalog_for(
        revision(vec![
            entry("tool:top", &variant(&["tool:left", "tool:right"], "top")),
            entry("tool:left", &variant(&["tool:leaf"], "left")),
            entry("tool:right", &variant(&["tool:leaf"], "right")),
            entry("tool:leaf", &variant(&[], "leaf")),
        ]),
        linux_x64(),
    );
    let result = catalog
        .get_sandbox_config_with_diagnostics("top", &weak().project_root("/r"))
        .unwrap();
    let deps: Vec<&str> = result
        .diagnostics
        .resolved_dependencies
        .iter()
        .map(|d| d.entry_id.as_str())
        .collect();
    assert_eq!(deps, ["tool:leaf", "tool:left", "tool:right"]);
    assert_eq!(
        fs_paths(&result.policy),
        j(r#"{"readonlyPaths":["/r/top","/r/left","/r/leaf","/r/right"]}"#)
    );
}

#[test]
fn dependency_diagnostics_keep_distinct_ranges_sorted() {
    let catalog = catalog_for(
        revision(vec![
            entry(
                "tool:a",
                &variants(&[linux(
                    &format!(r#"{{"version":"{V}"}}"#),
                    r#""dependencies":[{"entryId":"tool:c","versionRange":">=2"}]"#,
                )]),
            ),
            entry(
                "tool:b",
                &variants(&[linux(
                    &format!(r#"{{"version":"{V}"}}"#),
                    r#""dependencies":[{"entryId":"tool:c"}]"#,
                )]),
            ),
            entry("tool:c", &variants(&[linux(&format!(r#"{{"version":"{V}"}}"#), "")])),
        ]),
        linux_x64(),
    );
    let result = catalog
        .get_sandbox_config_with_diagnostics(vec!["a", "b", "a"], &weak())
        .unwrap();
    assert_eq!(
        result
            .to_json()
            .get("diagnostics")
            .unwrap()
            .get("resolvedDependencies")
            .unwrap(),
        &j(
            r#"[{"entryId":"tool:c","entryRevision":1},{"entryId":"tool:c","entryRevision":1,"requiredVersionRange":">=2"}]"#
        )
    );
    assert_eq!(result.policy.unwrap().to_json(), j(&format!(r#"{{"version":"{V}"}}"#)));
}

#[test]
fn catalog_file_order_does_not_matter() {
    let e = |id: &str, p: &str| {
        entry(
            id,
            &format!(
                r#"{{"identity":[{{"kind":"invocation-name","names":["x"]}}],"platformVariants":[{{"when":{{"platform":"linux"}},"sandboxPolicy":{{"version":"{V}","filesystem":{{"readonlyPaths":["${{project_root}}/{p}"]}}}}}}]}}"#
            ),
        )
    };
    let ctx = weak().project_root("/r");
    let forward = catalog_for(revision(vec![e("tool:b", "b"), e("tool:a", "a")]), linux_x64())
        .get_sandbox_config_with_diagnostics("x", &ctx)
        .unwrap();
    let backward = catalog_for(revision(vec![e("tool:a", "a"), e("tool:b", "b")]), linux_x64())
        .get_sandbox_config_with_diagnostics("x", &ctx)
        .unwrap();
    assert_eq!(forward, backward);
    assert_eq!(fs_paths(&forward.policy), j(r#"{"readonlyPaths":["/r/a","/r/b"]}"#));
}

#[test]
fn mixed_versions_across_inputs_conflict() {
    let catalog = catalog_for(
        revision(vec![
            entry("tool:a", &variants(&[linux(r#"{"version":"0.8.0-alpha"}"#, "")])),
            entry("tool:b", &variants(&[linux(&format!(r#"{{"version":"{V}"}}"#), "")])),
        ]),
        linux_x64(),
    );
    let error = catalog.get_sandbox_config(vec!["a", "b"], &weak()).unwrap_err();
    assert_eq!(
        error.message(),
        "[policy_validation] selected entries cannot be composed: mixed sandboxPolicy.version values (0.8.0-alpha, 0.9.0-alpha)"
    );
    assert_eq!(
        catalog.get_sandbox_config("a", &weak()).unwrap().unwrap().to_json(),
        j(r#"{"version":"0.8.0-alpha"}"#)
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
        j(r#"{"catalogSchemaVersion":"1","catalogRevision":"2026-09-29.1"}"#)
    );
    let entries = list_catalog_entries().unwrap();
    let ids: Vec<&str> = entries.iter().map(|e| e.entry_id.as_str()).collect();
    assert_eq!(ids, ["tool:git", "tool:node", "tool:npm"]);
    let npm = entries.iter().find(|e| e.entry_id == "tool:npm").unwrap().to_json();
    assert_eq!(
        npm.get("platformVariants").unwrap().as_array().unwrap()[0],
        j(&format!(
            r#"{{"platform":"windows","dependencyEntryIds":["tool:node"],"sandboxPolicyVersion":"{V}"}}"#
        ))
    );
    let text = Json::Array(entries.iter().map(|e| e.to_json()).collect()).to_compact_string();
    assert!(!text.contains("Paths") && !text.contains("${") && !text.contains("sandboxPolicy\""));
}

#[test]
fn module_level_functions_use_the_bundled_catalog() {
    let ctx = weak().platform(Platform::Linux).architecture(Architecture::X64);
    assert_eq!(get_sandbox_config("definitely-unknown-tool", &ctx).unwrap(), None);
    let result = get_sandbox_config_with_diagnostics(vec![ToolInput::from("definitely-unknown-tool")], &ctx).unwrap();
    assert_eq!(result.policy, None);
    assert_eq!(
        result.diagnostics.to_json().get("tools").unwrap(),
        &j(r#"[{"inputIndex":0,"matches":[]}]"#)
    );
    assert_eq!(
        reason_of(get_sandbox_config("git", &ctx.catalog_revision("1999-01-01.1"))),
        ErrorReason::RevisionUnavailable
    );
}

// ---------------------------------------------------------------------------
// Validation (validation.test.ts)
// ---------------------------------------------------------------------------

fn dep(id: &str) -> String {
    variants(&[linux(
        &format!(r#"{{"version":"{V}"}}"#),
        &format!(r#""dependencies":[{{"entryId":"{id}"}}]"#),
    )])
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
        "[policy_validation] 'tool:a' on windows/x64: cycle (tool:a -> tool:b -> tool:c -> tool:a)"
            .replace("windows", "linux")
    );
    validate_err(vec![entry("tool:a", &dep("tool:a"))], "depends on itself");
    validate_err(
        vec![entry("tool:a", &dep("tool:missing"))],
        "unknown entry 'tool:missing'",
    );
    validate_err(
        vec![
            entry(
                "tool:a",
                &variants(&[linux(
                    &format!(r#"{{"version":"{V}"}}"#),
                    r#""dependencies":[{"entryId":"tool:b","versionRange":"^1.x"}]"#,
                )]),
            ),
            entry("tool:b", ""),
        ],
        "versionRange",
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
        format!(r#"{{"when":{{"platform":"linux"{a}}},"sandboxPolicy":{{"version":"{V}"}}}}"#)
    };
    validate_err(
        vec![entry("tool:a", &variants(&[v("x64"), v("x64")]))],
        "duplicates selector",
    );
    validate_err(
        vec![entry("tool:a", &variants(&[v(""), v("")]))],
        "second architecture-neutral",
    );
    validate(vec![entry("tool:a", &variants(&[v(""), v("x64"), v("arm64")]))]).unwrap();
    validate_err(
        vec![entry("tool:a", &variants(&[v("riscv")]))],
        "'entries[0].platformVariants[0].when.architecture' must be one of x64, arm64",
    );
}

#[test]
fn validation_policy_rules() {
    let p = |policy: &str| vec![entry("tool:a", &variants(&[linux(policy, "")]))];
    validate_err(
        p(&format!(r#"{{"version":"{V}","processContainer":{{}}}}"#)),
        "containment backend",
    );
    validate_err(
        p(&format!(r#"{{"version":"{V}","containment":"lxc"}}"#)),
        "containment backend",
    );
    validate_err(p(r#"{"version":"0.9.0"}"#), "not a SandboxPolicy version registered");
    validate_err(
        p(r#"{"version":"0.10.0-alpha"}"#),
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
    validate_err(
        p(&format!(r#"{{"version":"{V}","network":{{"allowOutbound":true}}}}"#)),
        "unsupported field",
    );
    validate_err(
        p(&format!(r#"{{"version":"{V}","telemetry":{{}}}}"#)),
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
    validate(path("${project_root}\\x")).unwrap();
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
        net(r#"{"egress":{"allow":[{"to":[{"cidr":"10.0.0.0/8"}],"ports":[{"port":0}]}]}}"#),
        "must be an integer in 1..65535",
    );
    validate_err(
        net(r#"{"egress":{"allow":[{"to":[{"cidr":"10.0.0.0/8"}],"ports":[{"port":10,"endPort":5}]}]}}"#),
        "requires a lower or equal 'port'",
    );
    validate(net(r#"{"egress":{"deny":[{"ports":[{"port":22}]}]}}"#)).unwrap();
    validate(net(r#"{"egress":{"default":"deny","allow":[{"to":[{"cidr":"192.0.2.0/24"}],"ports":[{"protocol":"tcp","port":443}]}]}}"#)).unwrap();
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
        entry("tool:b", r#"{"identity":[{"kind":"invocation-name","names":["A"]}]}"#),
    ])
    .unwrap();
    validate(vec![
        entry("tool:a", r#"{"identity":[{"kind":"purl","value":"pkg:npm/x"}]}"#),
        entry("tool:b", r#"{"identity":[{"kind":"purl","value":"pkg:NPM/x"}]}"#),
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
            r#"{"identity":[{"kind":"invocation-name","names":["\u00c9","\u00e9"]}]}"#,
        )],
        "repeats identity",
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
            r#"{"identity":[{"kind":"invocation-name","names":["bin/a"]}]}"#,
        )],
        "bare invocation name",
    );
    validate_err(
        vec![entry("tool:a", r#"{"identity":[{"kind":"sha256","value":"x"}]}"#)],
        "not a supported identity kind",
    );
    validate_err(
        vec![
            entry("tool:a", ""),
            entry("tool:a", r#"{"identity":[{"kind":"invocation-name","names":["z"]}]}"#),
        ],
        "duplicate entryId",
    );
    validate_err(
        vec![entry("tool:a", r#"{"entryId":"noNamespace"}"#)],
        "must be namespaced",
    );
}

#[test]
fn validation_composition_vocabulary() {
    let with_dep = |policy: &str, dep_policy: &str| {
        validate(vec![
            entry(
                "tool:a",
                &variants(&[linux(policy, r#""dependencies":[{"entryId":"tool:b"}]"#)]),
            ),
            entry("tool:b", &variants(&[linux(dep_policy, "")])),
        ])
    };
    with_dep(
        &format!(r#"{{"version":"{V}","filesystem":{{"readwritePaths":["${{project_root}}"],"readonlyPaths":["${{node_prefix}}"]}}}}"#),
        &format!(r#"{{"version":"{V}","filesystem":{{"readonlyPaths":["${{node_prefix}}","${{git_prefix}}"],"deniedPaths":["${{user_home}}/.ssh"]}}}}"#),
    )
    .unwrap();
    let err = |r: Result<(), PolicyCatalogError>, needle: &str| {
        let m = r.unwrap_err().message().to_string();
        assert!(m.contains(needle), "{m}");
    };
    err(
        with_dep(
            &format!(r#"{{"version":"{V}","filesystem":{{"readwritePaths":["${{project_root}}"]}}}}"#),
            &format!(r#"{{"version":"{V}","filesystem":{{"deniedPaths":["${{project_root}}/secrets"]}}}}"#),
        ),
        "'${project_root}/secrets' (deniedPaths) overlaps '${project_root}' (readwritePaths)",
    );
    err(
        with_dep(&format!(r#"{{"version":"{V}"}}"#), r#"{"version":"0.8.0-alpha"}"#),
        "mixed sandboxPolicy.version",
    );
    err(
        with_dep(
            &format!(r#"{{"version":"{V}","network":{{"egress":{{"default":"deny"}}}}}}"#),
            &format!(r#"{{"version":"{V}"}}"#),
        ),
        "'tool:a' uses 'network', which has no v1 cross-entry composition rule",
    );
    err(
        with_dep(
            &format!(r#"{{"version":"{V}"}}"#),
            &format!(r#"{{"version":"{V}","timeoutMs":5}}"#),
        ),
        "'timeoutMs'",
    );
}

// ---------------------------------------------------------------------------
// Store and history (store.test.ts)
// ---------------------------------------------------------------------------

#[test]
fn bundled_store_verifies_and_canonical_digest_ignores_formatting() {
    let store = mxc_policy_catalog::bundled_catalog_store().unwrap();
    assert!(check_store_history(&store).is_empty());
    assert_eq!(store.revision(None).unwrap().catalog_revision, store.default_revision());
    assert_eq!(
        canonical_json(&j(r#"{"b":1,"a":[2,{"d":3,"c":4}]}"#)),
        r#"{"a":[2,{"c":4,"d":3}],"b":1}"#
    );
    assert_eq!(
        canonical_sha256(&j(r#"{"a":1,"b":2}"#)),
        canonical_sha256(&j(r#"{"b":2,"a":1}"#))
    );
    assert_ne!(canonical_sha256(&j(r#"{"a":1}"#)), canonical_sha256(&j(r#"{"a":2}"#)));
}

#[test]
fn tampered_revision_is_an_integrity_error_everywhere() {
    let digests = HashMap::from([("2000-01-01.1".to_string(), "0".repeat(64))]);
    let store = Arc::new(store_with(&[revision(vec![entry("tool:a", "")])], None, &digests).unwrap());
    assert_eq!(reason_of(store.revision(None)), ErrorReason::Integrity);
    let catalog = PolicyCatalog::with_host(store, linux_x64());
    assert_eq!(
        reason_of(catalog.get_sandbox_config("a", &weak())),
        ErrorReason::Integrity
    );
    assert_eq!(
        reason_of(catalog.get_sandbox_config_with_diagnostics("a", &weak())),
        ErrorReason::Integrity
    );
    assert_eq!(reason_of(catalog.list_catalog_entries()), ErrorReason::Integrity);
    assert_eq!(reason_of(catalog.get_catalog_info()), ErrorReason::Integrity);
}

#[test]
fn revision_id_mismatch_and_invalid_manifest() {
    let other = revision_with(vec![entry("tool:a", "")], "2000-01-02.1");
    let mut relabeled = other.clone();
    if let Json::Object(o) = &mut relabeled {
        o.insert("catalogRevision", "2000-01-01.1".into());
    }
    let digests = HashMap::from([("2000-01-01.1".to_string(), canonical_sha256(&other))]);
    assert_eq!(
        reason_of(store_with(&[relabeled.clone()], None, &digests).unwrap().revision(None)),
        ErrorReason::Integrity
    );
    // Correct digest but wrong declared id → integrity (revision-id check).
    let digests = HashMap::from([("2000-01-01.1".to_string(), canonical_sha256(&other))]);
    let source = mxc_policy_catalog::MemorySource {
        files: HashMap::from([("revisions/2000-01-01.1.json".to_string(), other.clone())]),
        ..mxc_policy_catalog::MemorySource::publishing(contract(), &[relabeled], None, &digests)
    };
    let error = mxc_policy_catalog::CatalogStore::new(source)
        .unwrap()
        .revision(None)
        .unwrap_err();
    assert_eq!(
        error.message(),
        "[backend_error] file 'revisions/2000-01-01.1.json' declares revision '2000-01-02.1', expected '2000-01-01.1'"
    );
    let r = revision(vec![entry("tool:a", "")]);
    assert_eq!(
        reason_of(store_with(
            std::slice::from_ref(&r),
            Some("2001-01-01.1"),
            &HashMap::new()
        )),
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
    let r1 = revision_with(vec![entry("tool:a", ""), entry("tool:b", "")], "2000-01-01.1");
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
        let r = catalog.get_sandbox_config_with_diagnostics("a", ctx).unwrap();
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
        .get_sandbox_config("a", &ctx.catalog_revision("2000-01-03.1"))
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
        vec![entry("tool:a", r#"{"displayName":"x"}"#), entry("tool:b", "")],
        "2000-01-02.1",
    ));
    let bumped = v(revision_with(
        vec![entry("tool:a", ""), entry("tool:b", r#"{"entryRevision":2}"#)],
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
    let published = PublishedRevision::new("2000-01-01.1", "revisions/2000-01-01.1.json", &"a".repeat(64));
    let files = HashMap::from([("revisions/2000-01-01.1.json".to_string(), "{\"x\":1}\n".to_string())]);
    let base = PublishedState {
        revisions: vec![published.clone()],
        files: files.clone(),
    };
    let appended = PublishedState {
        revisions: vec![
            published.clone(),
            PublishedRevision::new("2000-01-02.1", "revisions/2000-01-02.1.json", &"b".repeat(64)),
        ],
        files: HashMap::from([("revisions/2000-01-01.1.json".to_string(), "{\r\n \"x\": 1}".to_string())]),
    };
    assert!(check_published_immutability(&base, &appended).is_empty());
    let edited = PublishedState {
        revisions: vec![published.clone()],
        files: HashMap::from([("revisions/2000-01-01.1.json".to_string(), "{\"x\":2}\n".to_string())]),
    };
    assert_eq!(
        check_published_immutability(&base, &edited),
        ["published revision file 'revisions/2000-01-01.1.json' was modified; publish a new revision instead"]
    );
    let redigested = PublishedState {
        revisions: vec![PublishedRevision::new(
            "2000-01-01.1",
            "revisions/2000-01-01.1.json",
            &"c".repeat(64),
        )],
        files,
    };
    assert_eq!(
        check_published_immutability(&base, &redigested),
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
