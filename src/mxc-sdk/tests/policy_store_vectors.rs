// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Frozen vectors in `../conformance/vectors/`. They were generated from the
//! original TypeScript prototype and are now maintained by hand.

#[path = "policy_store_common/mod.rs"]
mod common;

use common::*;
use mxc_sdk::__policy_store::tooling::{
    canonical_json, is_absolute_path, normalize_path, parse_purl, path_key_segments, Json,
};
use mxc_sdk::__policy_store::Platform;

#[test]
fn canonical_json_vectors() {
    let vectors = read_json(
        crate_dir()
            .join("conformance")
            .join("vectors")
            .join("canonical-json.json"),
    );
    let cases = vectors.get("cases").and_then(Json::as_array).unwrap();
    assert!(cases.len() >= 5);
    for case in cases {
        let text = case.get("json").and_then(Json::as_str).unwrap();
        let value = Json::parse(text).unwrap();
        assert_eq!(
            canonical_json(&value),
            case.get("canonical").and_then(Json::as_str).unwrap(),
            "{text}"
        );
    }
}

#[test]
fn path_vectors() {
    let vectors = read_json(
        crate_dir()
            .join("conformance")
            .join("vectors")
            .join("paths.json"),
    );
    for platform in Platform::ALL {
        let cases = vectors
            .get(platform.as_str())
            .and_then(Json::as_array)
            .unwrap();
        assert!(cases.len() > 5);
        for v in cases {
            let input = v.get("input").and_then(Json::as_str).unwrap();
            assert_eq!(
                is_absolute_path(input, platform),
                v.get("absolute").and_then(Json::as_bool).unwrap(),
                "{platform} isAbsolute {input:?}"
            );
            if let Some(expected) = v.get("normalized").and_then(Json::as_str) {
                assert_eq!(
                    normalize_path(input, platform),
                    expected,
                    "{platform} normalize {input:?}"
                );
            }
            let segments: Vec<String> = v
                .get("keySegments")
                .and_then(Json::as_array)
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_string())
                .collect();
            assert_eq!(
                path_key_segments(input, platform),
                segments,
                "{platform} keySegments {input:?}"
            );
        }
    }
}

#[test]
fn malformed_percent_encoding_is_an_invalid_purl() {
    assert_eq!(parse_purl("pkg:npm/npm@%E0%A4%A"), None);
    let parsed = parse_purl("pkg:npm/npm@10.9.0").expect("valid purl");
    assert_eq!(parsed.package_type, "npm");
    assert_eq!(parsed.name, "npm");
    assert_eq!(parsed.version.as_deref(), Some("10.9.0"));
    assert!(!parsed.has_qualifiers && !parsed.has_subpath);
}

/// The embedded catalog must equal `catalog/` on disk (canonical content),
/// so `build.rs` can never silently skip or misorder a revision.
#[test]
fn embedded_catalog_matches_repository_catalog() {
    let dir = crate_dir().join("catalog");
    let embedded = mxc_sdk::__policy_store::tooling::bundled_catalog_files();
    let mut on_disk = vec!["contract.v1.json".to_string(), "manifest.json".to_string()];
    let mut revisions: Vec<String> = std::fs::read_dir(dir.join("revisions"))
        .unwrap()
        .map(|e| format!("revisions/{}", e.unwrap().file_name().to_string_lossy()))
        .filter(|n| n.ends_with(".json"))
        .collect();
    revisions.sort();
    on_disk.extend(revisions);
    let names: Vec<String> = embedded.iter().map(|(n, _)| n.to_string()).collect();
    assert_eq!(names, on_disk);
    for (name, text) in embedded {
        let disk = read_json(dir.join(name));
        assert_eq!(
            canonical_json(&Json::parse(text).unwrap()),
            canonical_json(&disk),
            "{name}"
        );
    }
}
