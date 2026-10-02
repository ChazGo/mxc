// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Runs every language-neutral conformance fixture in
//! `../conformance/fixtures/*.json`. The mxc-sdk and mxc_ffi suites replay
//! the same fixtures through their binding entry points.

mod common;

use common::*;
use mxc_policy_store::tooling::{Json, JsonObject};
use mxc_policy_store::{Architecture, Platform, PolicyCatalogError};

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
    let mut cases = 0;
    for name in names {
        let fixture = read_json(dir.join(&name));
        for case in fixture.get("cases").and_then(Json::as_array).unwrap() {
            let case_name = format!(
                "{name}: {}",
                case.get("name").and_then(Json::as_str).unwrap()
            );
            let host_platform = case
                .get("host")
                .and_then(|h| h.get("platform"))
                .and_then(Json::as_str)
                .map_or(Platform::Linux, |p| Platform::parse(p).unwrap());
            let host_arch = case
                .get("host")
                .and_then(|h| h.get("nativeArchitecture"))
                .and_then(Json::as_str)
                .map_or(Architecture::X64, |a| Architecture::parse(a).unwrap());
            let host = fixed_host(host_platform, host_arch);
            let catalog = match fixture.get("catalog") {
                Some(Json::String(s)) if s == "bundled" => bundled_catalog(host),
                Some(revision) => catalog_for(revision.clone(), host),
                None => panic!("fixture without catalog"),
            };
            let raw_tools = case.get("tools").unwrap();
            let tools = tools_from(raw_tools);
            let ctx = context_from(case.get("context"));
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
                    failure(catalog.resolve_sandbox_policy_with_diagnostics(tools.clone(), &ctx)),
                    expected,
                    "{case_name}"
                );
                assert_eq!(
                    failure(catalog.resolve_sandbox_policy(tools, &ctx)),
                    expected,
                    "{case_name}"
                );
                cases += 1;
                continue;
            }
            let expect = case.get("expect").unwrap();
            let mut expected = JsonObject::new();
            let expected_policy = expect.get("policy").filter(|p| !p.is_null()).cloned();
            if let Some(policy) = &expected_policy {
                expected.insert("policy", policy.clone());
            }
            expected.insert("diagnostics", expect.get("diagnostics").unwrap().clone());
            let expected = Json::Object(expected);

            let actual = catalog
                .resolve_sandbox_policy_with_diagnostics(tools.clone(), &ctx)
                .unwrap();
            assert_eq!(
                actual.to_json(),
                expected,
                "{case_name}\nactual: {}",
                actual.to_json()
            );
            let policy = catalog.resolve_sandbox_policy(tools.clone(), &ctx).unwrap();
            assert_eq!(policy.map(|p| p.to_json()), expected_policy, "{case_name}");
            if !matches!(raw_tools, Json::Array(_)) {
                let as_array = tools_from(&Json::Array(vec![raw_tools.clone()]));
                let again = catalog
                    .resolve_sandbox_policy_with_diagnostics(as_array, &ctx)
                    .unwrap();
                assert_eq!(
                    again, actual,
                    "{case_name}: single input == one-element array"
                );
            }
            cases += 1;
        }
    }
    assert!(cases >= 20, "ran {cases} conformance cases");
}
