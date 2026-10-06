// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Runs every language-neutral conformance fixture in
//! `../conformance/fixtures/*.json`. The mxc-sdk and mxc_ffi suites replay
//! the same fixtures through their binding entry points.
//!
//! `MXC_POLICY_STORE_UPDATE_FIXTURES=<dir>` records each case's actual
//! outcome as `<dir>/<fixture>` (one compact JSON array) instead of
//! asserting, so a catalog or resolver change can be reviewed and merged into
//! the fixtures by hand.

#[path = "policy_store_common/mod.rs"]
mod common;

use common::*;
use mxc_sdk::__policy_store::tooling::{Json, JsonObject};
use mxc_sdk::__policy_store::{Architecture, FixedHost, Platform, PolicyCatalogError};
use std::sync::Arc;

/// The fixed host a case describes: symbols, discovery results, object
/// aliases, and paths whose identity cannot be established.
fn host_from(
    host: Option<&Json>,
    platform: Platform,
    architecture: Architecture,
) -> Arc<FixedHost> {
    let mut fixed = FixedHost::new(platform, architecture);
    let Some(host) = host else {
        return Arc::new(fixed);
    };
    let pairs = |key: &str| -> Vec<(String, String)> {
        host.get(key)
            .and_then(Json::as_object)
            .map(|o| {
                o.iter()
                    .map(|(k, v)| (k.to_string(), v.as_str().unwrap().to_string()))
                    .collect()
            })
            .unwrap_or_default()
    };
    for (name, value) in pairs("symbols") {
        fixed = fixed.with_symbol(name, value);
    }
    for (name, value) in pairs("discovered") {
        fixed = fixed.with_discovered(name, value);
    }
    for alias in host
        .get("aliases")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
    {
        let alias = alias.as_array().unwrap();
        fixed = fixed.with_alias(alias[0].as_str().unwrap(), alias[1].as_str().unwrap());
    }
    for path in host
        .get("unknownPaths")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
    {
        fixed = fixed.with_unknown_path(path.as_str().unwrap());
    }
    Arc::new(fixed)
}

fn failure<T>(result: Result<T, PolicyCatalogError>) -> Option<(String, String)> {
    result.err().map(|e| {
        (
            e.code().as_str().to_string(),
            e.reason().as_str().to_string(),
        )
    })
}

#[test]
fn all_conformance_fixtures() {
    let dir = crate_dir().join("conformance").join("fixtures");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".json"))
        .collect();
    names.sort();
    assert!(names.len() >= 2, "fixtures present");
    let update_dir = std::env::var("MXC_POLICY_STORE_UPDATE_FIXTURES")
        .ok()
        .filter(|d| !d.is_empty());
    let mut cases = 0;
    for name in names {
        let fixture = read_json(dir.join(&name));
        let mut recorded = Vec::new();
        for case in fixture.get("cases").and_then(Json::as_array).unwrap() {
            let case_name = format!(
                "{name}: {}",
                case.get("name").and_then(Json::as_str).unwrap()
            );
            // Object identity is examined only on the target's own host, so
            // the fixed host defaults to the requested platform.
            let host_platform = case
                .get("host")
                .and_then(|h| h.get("platform"))
                .or_else(|| case.get("context").and_then(|c| c.get("platform")))
                .and_then(Json::as_str)
                .map_or(Platform::Linux, |p| Platform::parse(p).unwrap());
            let host_arch = case
                .get("host")
                .and_then(|h| h.get("nativeArchitecture"))
                .and_then(Json::as_str)
                .map_or(Architecture::X64, |a| Architecture::parse(a).unwrap());
            let host = host_from(case.get("host"), host_platform, host_arch);
            let catalog = match fixture.get("catalog") {
                Some(Json::String(s)) if s == "bundled" => bundled_catalog(host),
                Some(revision) => catalog_for(revision.clone(), host),
                None => panic!("fixture without catalog"),
            };
            let raw_tools = case.get("tools").unwrap();
            let tools = tools_from(raw_tools);
            let ctx = context_from(case.get("context"));
            if update_dir.is_some() {
                let mut outcome = JsonObject::new();
                outcome.insert("name", case.get("name").unwrap().clone());
                match catalog.resolve_requirements_with_diagnostics(tools, &ctx) {
                    Ok(result) => outcome.insert("expect", result.to_json()),
                    Err(error) => {
                        let mut e = JsonObject::new();
                        e.insert("code", error.code().as_str().into());
                        e.insert("reason", error.reason().as_str().into());
                        outcome.insert("expectError", Json::Object(e));
                    }
                }
                recorded.push(Json::Object(outcome));
                continue;
            }
            if let Some(expect_error) = case.get("expectError") {
                let expected = Some((
                    expect_error
                        .get("code")
                        .and_then(Json::as_str)
                        .unwrap()
                        .to_string(),
                    expect_error
                        .get("reason")
                        .and_then(Json::as_str)
                        .unwrap()
                        .to_string(),
                ));
                assert_eq!(
                    failure(catalog.resolve_requirements_with_diagnostics(tools.clone(), &ctx)),
                    expected,
                    "{case_name}"
                );
                assert_eq!(
                    failure(catalog.resolve_requirements(tools, &ctx)),
                    expected,
                    "{case_name}"
                );
                cases += 1;
                continue;
            }
            let expect = case.get("expect").unwrap();
            let mut expected = JsonObject::new();
            let expected_requirements =
                expect.get("requirements").filter(|p| !p.is_null()).cloned();
            if let Some(requirements) = &expected_requirements {
                expected.insert("requirements", requirements.clone());
            }
            expected.insert("diagnostics", expect.get("diagnostics").unwrap().clone());
            let expected = Json::Object(expected);

            let actual = catalog
                .resolve_requirements_with_diagnostics(tools.clone(), &ctx)
                .unwrap();
            assert_eq!(
                actual.to_json(),
                expected,
                "{case_name}\nactual: {}",
                actual.to_json()
            );
            let requirements = catalog.resolve_requirements(tools.clone(), &ctx).unwrap();
            assert_eq!(
                requirements.map(|p| p.to_json()),
                expected_requirements,
                "{case_name}"
            );
            if !matches!(raw_tools, Json::Array(_)) {
                let as_array = tools_from(&Json::Array(vec![raw_tools.clone()]));
                let again = catalog
                    .resolve_requirements_with_diagnostics(as_array, &ctx)
                    .unwrap();
                assert_eq!(
                    again, actual,
                    "{case_name}: single input == one-element array"
                );
            }
            cases += 1;
        }
        if let Some(update_dir) = &update_dir {
            std::fs::create_dir_all(update_dir).unwrap();
            std::fs::write(
                std::path::Path::new(update_dir).join(&name),
                Json::Array(recorded).to_compact_string(),
            )
            .unwrap();
        }
    }
    if update_dir.is_some() {
        return;
    }
    assert!(cases >= 20, "ran {cases} conformance cases");
}
