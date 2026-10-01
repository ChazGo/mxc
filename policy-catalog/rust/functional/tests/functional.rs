// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Functional tests of the PACKAGED crate: the library comes from the
//! extracted `.crate` and the CLI is the binary built from it. Every catalog
//! here is a synthetic directory or a copy in a temporary folder.

use mxc_policy_catalog::tooling::Json;
use mxc_policy_catalog::{
    load_catalog_directory, Architecture, ErrorReason, FixedHost, Platform, PolicyCatalog, ResolveContext,
    ToolCandidate,
};
use policy_catalog_functional_consumer::*;
use std::sync::Arc;

fn catalog_at(dir: &std::path::Path, platform: Platform, architecture: Architecture) -> PolicyCatalog {
    PolicyCatalog::with_host(
        Arc::new(load_catalog_directory(dir).unwrap()),
        Arc::new(FixedHost::new(platform, architecture)),
    )
}

fn depends_on(target: &str) -> String {
    let name = target.split(':').nth(1).unwrap();
    format!(
        r#"{{"platformVariants":[{{"when":{{"platform":"linux"}},"dependencies":[{{"entryId":"{target}"}}],"sandboxPolicy":{{"version":"0.9.0-alpha","filesystem":{{"readonlyPaths":["${{git_prefix}}/{name}-dep"]}}}}}}]}}"#
    )
}

// ---------------------------------------------------------------------------
// Unknown tool
// ---------------------------------------------------------------------------

#[test]
fn unknown_tool_is_no_policy_with_warning() {
    let ctx = full_context("linux", "x64");
    let result = cli(&args(&["resolve", "--diagnostics"], &ctx).into_iter().chain(["cargo"]).collect::<Vec<_>>());
    assert_eq!(result.status, 0, "{}", result.stderr);
    let json = result.json.as_ref().unwrap();
    assert!(json.get("policy").is_none(), "policy key must be omitted");
    assert_eq!(json.get("diagnostics").unwrap().get("tools").unwrap(), &j(r#"[{"inputIndex":0,"matches":[]}]"#));
    assert!(result.warnings().lines().any(|l| l == "input 0 ('cargo') matched no eligible catalog entry"));
    let plain = cli(&args(&["resolve"], &ctx).into_iter().chain(["cargo"]).collect::<Vec<_>>());
    assert_eq!(plain.stdout, "null\n");
    // Library API over the bundled catalog.
    let r = mxc_policy_catalog::resolve_sandbox_policy_with_diagnostics(
        "cargo",
        &ResolveContext::new().platform(Platform::Linux).architecture(Architecture::X64).allow_weak(true),
    )
    .unwrap();
    assert_eq!(r.policy, None);
}

// ---------------------------------------------------------------------------
// Dependency cycle
// ---------------------------------------------------------------------------

#[test]
fn dependency_cycle_fails_validate_and_resolve() {
    let dir = write_catalog(
        &work_dir("cycle"),
        &[revision(
            vec![
                entry("tool:a", &depends_on("tool:b")),
                entry("tool:b", &depends_on("tool:c")),
                entry("tool:c", &depends_on("tool:a")),
            ],
            "2000-01-01.1",
        )],
    );
    let validate = cli(&["validate", "--catalog", &s(&dir)]);
    assert_eq!(validate.status, 1, "{}", validate.stdout);
    assert_eq!(validate.json.as_ref().unwrap().get("ok"), Some(&Json::Bool(false)));
    assert!(
        validate
            .errors()
            .contains("[policy_validation] 'tool:a' on linux/x64: cycle (tool:a -> tool:b -> tool:c -> tool:a)"),
        "{}",
        validate.errors()
    );
    let resolve = cli(&[
        "resolve", "--catalog", &s(&dir), "--platform", "linux", "--architecture", "x64", "--allow-weak", "--symbol",
        "git_prefix=/opt", "a",
    ]);
    assert_eq!(resolve.status, 1, "{}", resolve.stdout);
    let error = resolve.json.as_ref().unwrap().get("error").unwrap();
    assert_eq!(error.get("code").and_then(Json::as_str), Some("policy_validation"));
    assert_eq!(resolve.error_reason().as_deref(), Some("invalid_catalog"));
    // Library API.
    let catalog = catalog_at(&dir, Platform::Linux, Architecture::X64);
    let err = catalog
        .resolve_sandbox_policy("a", &ResolveContext::new().allow_weak(true).symbol("git_prefix", "/opt"))
        .unwrap_err();
    assert_eq!(err.reason(), ErrorReason::InvalidCatalog);
    assert!(err.message().contains("cycle ("), "{}", err.message());
}

#[test]
fn diamond_is_valid_and_contributes_shared_entry_once() {
    let dir = write_catalog(
        &work_dir("diamond"),
        &[revision(
            vec![
                entry(
                    "tool:a",
                    r#"{"platformVariants":[{"when":{"platform":"linux"},"dependencies":[{"entryId":"tool:b"},{"entryId":"tool:c"}],"sandboxPolicy":{"version":"0.9.0-alpha","filesystem":{"readonlyPaths":["${git_prefix}/a"]}}}]}"#,
                ),
                entry("tool:b", &depends_on("tool:d")),
                entry("tool:c", &depends_on("tool:d")),
                entry("tool:d", ""),
            ],
            "2000-01-01.1",
        )],
    );
    assert_eq!(cli(&["validate", "--catalog", &s(&dir)]).status, 0);
    let r = cli(&[
        "resolve", "--catalog", &s(&dir), "--diagnostics", "--platform", "linux", "--architecture", "x64",
        "--allow-weak", "--symbol", "git_prefix=/opt", "a",
    ]);
    assert_eq!(r.status, 0, "{}", r.stderr);
    let json = r.json.unwrap();
    assert_eq!(
        json.get("policy").unwrap().get("filesystem").unwrap().get("readonlyPaths").unwrap(),
        &j(r#"["/opt/a","/opt/d-dep","/opt/d"]"#)
    );
}

// ---------------------------------------------------------------------------
// Integrity tamper
// ---------------------------------------------------------------------------

#[test]
fn integrity_tamper_fails_resolve_inspect_and_validate() {
    let dir = copy_bundled(&work_dir("tamper"));
    let untouched = cli(&["validate", "--catalog", &s(&dir)]);
    assert_eq!(untouched.status, 0, "{}", untouched.stdout);
    let manifest = j(&std::fs::read_to_string(dir.join("manifest.json")).unwrap());
    let file = manifest.get("revisions").unwrap().as_array().unwrap().last().unwrap().get("file").unwrap();
    let path = dir.join(file.as_str().unwrap());
    // Formatting-only change keeps the digest valid.
    let original = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, j(&original).to_compact_string().replace(',', ",\r\n")).unwrap();
    assert_eq!(cli(&["validate", "--catalog", &s(&dir)]).status, 0);
    // Semantic widening: the tamper integrity checking exists to stop.
    let widened = original.replacen("\"readwritePaths\": [\"${project_root}\"]", "\"readwritePaths\": [\"${project_root}\", \"${user_home}\"]", 1);
    assert_ne!(widened, original, "tamper edit applied");
    std::fs::write(&path, widened).unwrap();

    let validate = cli(&["validate", "--catalog", &s(&dir)]);
    assert_eq!(validate.status, 1, "{}", validate.stdout);
    assert_eq!(validate.json.as_ref().unwrap().get("ok"), Some(&Json::Bool(false)));
    assert!(validate.errors().contains("does not match the published digest"), "{}", validate.errors());
    let ctx = full_context("windows", "x64");
    let resolve = cli(&args(&["resolve", "--catalog", &s(&dir)], &ctx).into_iter().chain(["git"]).collect::<Vec<_>>());
    assert_eq!(resolve.status, 1, "{}", resolve.stdout);
    assert_eq!(resolve.error_reason().as_deref(), Some("integrity"));
    let error = resolve.json.as_ref().unwrap().get("error").unwrap();
    assert_eq!(error.get("code").and_then(Json::as_str), Some("backend_error"));
    let inspect = cli(&["inspect", "--catalog", &s(&dir)]);
    assert_eq!(inspect.status, 1);
    assert_eq!(inspect.error_reason().as_deref(), Some("integrity"));
    // Library API.
    let catalog = catalog_at(&dir, Platform::Linux, Architecture::X64);
    assert_eq!(catalog.list_catalog_entries().unwrap_err().reason(), ErrorReason::Integrity);
}

#[test]
fn missing_revision_file_is_integrity() {
    let dir = copy_bundled(&work_dir("missing"));
    for entry in std::fs::read_dir(dir.join("revisions")).unwrap() {
        std::fs::remove_file(entry.unwrap().path()).unwrap();
    }
    let inspect = cli(&["inspect", "--catalog", &s(&dir)]);
    assert_eq!(inspect.status, 1);
    assert_eq!(inspect.error_reason().as_deref(), Some("integrity"));
    assert!(cli(&["validate", "--catalog", &s(&dir)]).errors().contains("could not be read"));
}

// ---------------------------------------------------------------------------
// Unavailable revision
// ---------------------------------------------------------------------------

#[test]
fn unavailable_revision_is_an_error_never_a_substitution() {
    let ctx = full_context("linux", "x64");
    let r = cli(&args(&["resolve", "--revision", "2099-01-01.1"], &ctx).into_iter().chain(["git"]).collect::<Vec<_>>());
    assert_eq!(r.status, 1);
    assert_eq!(r.error_reason().as_deref(), Some("revision_unavailable"));
    let message = r.json.as_ref().unwrap().get("error").unwrap().get("message").and_then(Json::as_str).unwrap();
    assert_eq!(message, "[backend_error] catalog revision '2099-01-01.1' is not installed");
    let err = mxc_policy_catalog::resolve_sandbox_policy("git", &ResolveContext::new().catalog_revision("1999-01-01.1")).unwrap_err();
    assert_eq!(err.reason(), ErrorReason::RevisionUnavailable);

    // An older installed revision stays addressable.
    let r1 = revision(vec![entry("tool:a", "")], "2000-01-01.1");
    let r2 = revision(
        vec![entry(
            "tool:a",
            r#"{"entryRevision":2,"platformVariants":[{"when":{"platform":"linux"},"sandboxPolicy":{"version":"0.9.0-alpha","filesystem":{"readonlyPaths":["${git_prefix}/v2"]}}}]}"#,
        )],
        "2000-01-02.1",
    );
    let dir = write_catalog(&work_dir("revisions"), &[r1, r2]);
    assert_eq!(cli(&["validate", "--catalog", &s(&dir)]).status, 0);
    let base = ["resolve", "--catalog", &s(&dir), "--platform", "linux", "--architecture", "x64", "--allow-weak", "--symbol", "git_prefix=/opt"];
    let latest = cli(&[&base[..], &["a"]].concat());
    assert_eq!(latest.json.unwrap().get("filesystem").unwrap(), &j(r#"{"readonlyPaths":["/opt/v2"]}"#));
    let older = cli(&[&base[..], &["--revision", "2000-01-01.1", "a"]].concat());
    assert_eq!(older.json.unwrap().get("filesystem").unwrap(), &j(r#"{"readonlyPaths":["/opt/a"]}"#));
    let missing = cli(&[&base[..], &["--revision", "2000-01-03.1", "a"]].concat());
    assert_eq!(missing.error_reason().as_deref(), Some("revision_unavailable"));
}

// ---------------------------------------------------------------------------
// Weak identity opt-in
// ---------------------------------------------------------------------------

#[test]
fn weak_identity_requires_opt_in() {
    let symbols = [
        "--platform", "linux", "--architecture", "x64", "--project-root", "/w", "--symbol", "git_prefix=/usr/bin",
    ];
    let off = cli(&[&["resolve", "--diagnostics"], &symbols[..], &["git"]].concat());
    assert_eq!(off.status, 0);
    assert!(off.json.as_ref().unwrap().get("policy").is_none());
    assert!(off.warnings().contains(
        "input 0 ('git') matched no eligible catalog entry: tool:git matched only by invocation name and allowWeakIdentityFallback is not enabled"
    ));
    assert_eq!(cli(&[&["resolve"], &symbols[..], &["git"]].concat()).stdout, "null\n");
    let on = cli(&[&["resolve", "--diagnostics", "--allow-weak"], &symbols[..], &["git"]].concat());
    assert_eq!(on.status, 0);
    assert!(on.json.as_ref().unwrap().get("policy").is_some());
    assert!(on.warnings().contains("input 0 ('git') matched tool:git only by invocation name (weak identity)"));
    // Strong purl needs no opt-in and is range-checked.
    let strong = cli(&[
        "resolve", "--diagnostics", "--platform", "linux", "--architecture", "x64", "--project-root", "/w", "--symbol",
        "npm_prefix=/n", "--symbol", "npm_cache=/c", "--symbol", "node_prefix=/n", "--purl", "pkg:npm/npm@11.0.0", "npm",
    ]);
    assert_eq!(strong.status, 0, "{}", strong.stderr);
    assert!(strong.json.as_ref().unwrap().get("policy").is_some());
    assert!(!strong.warnings().contains("weak identity"));
    // Library API: string and object inputs obey the option identically.
    let catalog = PolicyCatalog::new(mxc_policy_catalog::bundled_catalog_store().unwrap());
    let ctx = ResolveContext::new()
        .platform(Platform::Linux)
        .architecture(Architecture::X64)
        .project_root("/p")
        .symbol("git_prefix", "/usr/bin");
    assert_eq!(catalog.resolve_sandbox_policy("git", &ctx).unwrap(), None);
    assert_eq!(catalog.resolve_sandbox_policy(ToolCandidate::new("git"), &ctx).unwrap(), None);
    let on = ctx.allow_weak(true);
    assert!(catalog.resolve_sandbox_policy("git", &on).unwrap().is_some());
}

// ---------------------------------------------------------------------------
// Architecture selection
// ---------------------------------------------------------------------------

#[test]
fn arch_mismatch_is_no_match_with_skipped_reason() {
    let dir = write_catalog(
        &work_dir("arch"),
        &[revision(
            vec![entry(
                "tool:a",
                r#"{"platformVariants":[
                  {"when":{"platform":"windows","architecture":"arm64"},"sandboxPolicy":{"version":"0.9.0-alpha","filesystem":{"readonlyPaths":["${git_prefix}\\arm64"]}}},
                  {"when":{"platform":"windows","architecture":"x64"},"sandboxPolicy":{"version":"0.9.0-alpha","filesystem":{"readonlyPaths":["${git_prefix}\\x64"]}}},
                  {"when":{"platform":"linux","architecture":"x64"},"sandboxPolicy":{"version":"0.9.0-alpha","filesystem":{"readonlyPaths":["${git_prefix}/x64"]}}}]}"#,
            )],
            "2000-01-01.1",
        )],
    );
    for arch in ["x64", "arm64"] {
        let r = cli(&[
            "resolve", "--catalog", &s(&dir), "--diagnostics", "--allow-weak", "--platform", "windows",
            "--architecture", arch, "--symbol", "git_prefix=C:\\t", "a",
        ]);
        assert_eq!(r.status, 0, "{}", r.stderr);
        let paths = r.json.as_ref().unwrap().get("policy").unwrap().get("filesystem").unwrap().get("readonlyPaths").unwrap().clone();
        assert_eq!(paths, Json::Array(vec![Json::from(format!("C:\\t\\{arch}"))]));
        assert!(!r.warnings().contains("architecture-neutral"));
    }
    let no_arm = cli(&[
        "resolve", "--catalog", &s(&dir), "--diagnostics", "--allow-weak", "--platform", "linux", "--architecture",
        "arm64", "--symbol", "git_prefix=/t", "a",
    ]);
    assert_eq!(no_arm.status, 0);
    assert!(no_arm.json.as_ref().unwrap().get("policy").is_none());
    assert!(no_arm.warnings().contains("input 0 ('a') matched no eligible catalog entry: tool:a has no variant for linux/arm64"));
}

#[test]
fn host_default_architecture_and_neutral_fallback_warnings() {
    for platform in [Platform::Windows, Platform::Linux, Platform::Macos] {
        for arch in [Architecture::X64, Architecture::Arm64] {
            // Library with an injected native architecture.
            let catalog = PolicyCatalog::with_host(
                mxc_policy_catalog::bundled_catalog_store().unwrap(),
                Arc::new(FixedHost::new(platform, arch)),
            );
            let root = if platform == Platform::Windows { "C:\\w" } else { "/w" };
            let prefix = if platform == Platform::Windows { "C:\\g" } else { "/g" };
            let ctx = ResolveContext::new().allow_weak(true).project_root(root).symbol("git_prefix", prefix);
            let omitted = catalog.resolve_sandbox_policy_with_diagnostics("git", &ctx).unwrap();
            let w = omitted.diagnostics.warnings.join("\n");
            assert!(w.contains(&format!(
                "architecture was not specified; variants were selected for the native system architecture '{arch}'; the tool's architecture was not verified"
            )), "{w}");
            assert!(w.contains(&format!("tool:git uses its architecture-neutral {platform} variant; no {arch}-specific variant exists")));
            let explicit = catalog
                .resolve_sandbox_policy_with_diagnostics("git", &ctx.clone().platform(platform).architecture(arch))
                .unwrap();
            assert!(!explicit.diagnostics.warnings.join("\n").contains("was not verified"));
            assert_eq!(explicit.policy, omitted.policy);

            // Packaged CLI: explicit architecture warns about neutral fallback only.
            let r = cli(&args(&["resolve", "--diagnostics"], &full_context(platform.as_str(), arch.as_str()))
                .into_iter()
                .chain(["git"])
                .collect::<Vec<_>>());
            assert!(r.warnings().contains(&format!("no {arch}-specific variant exists")));
            assert!(!r.warnings().contains("was not verified"));
        }
        // Packaged CLI on this real host: native detection is used and reported.
        let r = cli(&args(&["resolve", "--diagnostics"], &full_context(platform.as_str(), "")).into_iter().chain(["git"]).collect::<Vec<_>>());
        assert_eq!(r.status, 0, "{}", r.stdout);
        assert!(r.warnings().contains("the tool's architecture was not verified"), "{}", r.warnings());
    }
    let nothing = cli(&["resolve", "--diagnostics", "--platform", "linux", "cargo"]);
    assert!(!nothing.warnings().contains("architecture"));
}

// ---------------------------------------------------------------------------
// Multi-match
// ---------------------------------------------------------------------------

#[test]
fn one_input_matching_several_entries_warns_and_composes_all() {
    let dir = write_catalog(
        &work_dir("multi"),
        &[revision(
            vec![
                entry("tool:app", r#"{"identity":[{"kind":"purl","value":"pkg:npm/app"},{"kind":"invocation-name","names":["app"]}]}"#),
                entry("tool:app-plugin", r#"{"identity":[{"kind":"invocation-name","names":["app"]}]}"#),
            ],
            "2000-01-01.1",
        )],
    );
    let base = ["resolve", "--catalog", &s(&dir), "--diagnostics", "--platform", "linux", "--architecture", "x64", "--symbol", "git_prefix=/opt"];
    let r = cli(&[&base[..], &["--allow-weak", "app"]].concat());
    assert_eq!(r.status, 0, "{}", r.stderr);
    assert!(r.warnings().contains("input 0 ('app') matched 2 entries (tool:app, tool:app-plugin); all contribute"));
    assert_eq!(
        r.json.as_ref().unwrap().get("policy").unwrap().get("filesystem").unwrap().get("readonlyPaths").unwrap(),
        &j(r#"["/opt/app","/opt/app-plugin"]"#)
    );
    let strong_only = cli(&[&base[..], &["--purl", "pkg:npm/app", "app"]].concat());
    assert!(!strong_only.warnings().contains("matched 2 entries"));
    // Library API.
    let catalog = catalog_at(&dir, Platform::Linux, Architecture::X64);
    let res = catalog
        .resolve_sandbox_policy_with_diagnostics("app", &ResolveContext::new().allow_weak(true).architecture(Architecture::X64).symbol("git_prefix", "/opt"))
        .unwrap();
    assert_eq!(res.diagnostics.tools[0].matches.len(), 2);
}

// ---------------------------------------------------------------------------
// Composition conflict
// ---------------------------------------------------------------------------

#[test]
fn composition_conflicts_are_errors_not_choices() {
    let r = cli(&[
        "resolve", "--platform", "linux", "--architecture", "x64", "--allow-weak", "--project-root", "/opt", "--symbol",
        "git_prefix=/usr/bin", "--symbol", "node_prefix=/opt/node", "git", "node",
    ]);
    assert_eq!(r.status, 1, "{}", r.stdout);
    assert_eq!(r.error_reason().as_deref(), Some("composition_conflict"));
    let message = r.json.as_ref().unwrap().get("error").unwrap().get("message").and_then(Json::as_str).unwrap().to_string();
    assert!(message.contains("overlap across access classes"), "{message}");

    let dir = write_catalog(
        &work_dir("compose"),
        &[revision(
            vec![
                entry("tool:a", ""),
                entry(
                    "tool:b",
                    r#"{"platformVariants":[{"when":{"platform":"linux"},"sandboxPolicy":{"version":"0.9.0-alpha","network":{"egress":{"default":"deny"}}}}]}"#,
                ),
            ],
            "2000-01-01.1",
        )],
    );
    assert_eq!(cli(&["validate", "--catalog", &s(&dir)]).status, 0);
    let both = cli(&[
        "resolve", "--catalog", &s(&dir), "--platform", "linux", "--architecture", "x64", "--allow-weak", "--symbol",
        "git_prefix=/opt", "a", "b",
    ]);
    assert_eq!(both.status, 1);
    assert_eq!(both.error_reason().as_deref(), Some("composition_conflict"));
    // Library API.
    let catalog = catalog_at(&dir, Platform::Linux, Architecture::X64);
    let err = catalog
        .resolve_sandbox_policy(vec!["a", "b"], &ResolveContext::new().allow_weak(true).symbol("git_prefix", "/opt"))
        .unwrap_err();
    assert_eq!(err.reason(), ErrorReason::CompositionConflict);
}

// ---------------------------------------------------------------------------
// CLI surface
// ---------------------------------------------------------------------------

#[test]
fn cli_usage_errors_and_inspect() {
    for (argv, message) in [
        (vec![], "missing command"),
        (vec!["frob"], "unknown command 'frob'"),
        (vec!["inspect", "x"], "inspect: unexpected argument 'x'"),
        (vec!["resolve", "--base-ref", "HEAD"], "unknown option '--base-ref'"),
        (vec!["resolve", "--symbol", "=x", "a"], "--symbol expects name=value"),
        (vec!["resolve", "--platform", "--allow-weak"], "--platform requires a value"),
        (vec!["resolve", "--purl", "pkg:npm/x"], "--purl/--detected-version must precede a tool name"),
    ] {
        let r = cli(&argv);
        assert_eq!(r.status, 2, "{argv:?}");
        assert_eq!(r.stdout, "");
        assert_eq!(
            r.stderr,
            format!("policy-catalog: {message}\nusage: policy-catalog <resolve|inspect|validate> [options]\n")
        );
    }
    let inspect = cli(&["inspect"]);
    assert_eq!(inspect.status, 0);
    let json = inspect.json.unwrap();
    assert_eq!(json.get("info").unwrap(), &mxc_policy_catalog::get_catalog_info().unwrap().to_json());
    assert_eq!(json.get("entries").unwrap().as_array().unwrap().len(), mxc_policy_catalog::list_catalog_entries().unwrap().len());
    let validate = cli(&["validate"]);
    assert_eq!(validate.status, 0, "{}", validate.stdout);
}
