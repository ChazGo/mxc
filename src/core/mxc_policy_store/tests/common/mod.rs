// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Shared helpers for the integration tests. Tests read the crate's
//! `conformance/` and `catalog/` through `CARGO_MANIFEST_DIR`.

#![allow(dead_code)]

use mxc_policy_store::tooling::{Json, JsonObject};
use mxc_policy_store::{
    Architecture, CatalogStore, ErrorReason, FixedHost, MemorySource, Platform, PolicyCatalog,
    PolicyCatalogError, ResolveContext, ToolInputs,
};
use std::path::PathBuf;
use std::sync::Arc;

pub fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn read_json(path: PathBuf) -> Json {
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    Json::parse(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// The contract shipped with the embedded catalog.
pub fn contract() -> Json {
    let files = mxc_policy_store::tooling::bundled_catalog_files();
    let text = files
        .iter()
        .find(|(name, _)| *name == "contract.v1.json")
        .unwrap()
        .1;
    Json::parse(text).unwrap()
}

pub fn fixed_host(platform: Platform, architecture: Architecture) -> Arc<FixedHost> {
    Arc::new(FixedHost::new(platform, architecture))
}

pub fn linux_x64() -> Arc<FixedHost> {
    fixed_host(Platform::Linux, Architecture::X64)
}

pub fn store_for(revisions: &[Json]) -> Result<CatalogStore, PolicyCatalogError> {
    store_with(revisions, None)
}

pub fn store_with(
    revisions: &[Json],
    default_revision: Option<&str>,
) -> Result<CatalogStore, PolicyCatalogError> {
    CatalogStore::new(MemorySource::publishing(
        contract(),
        revisions,
        default_revision,
    ))
}

pub fn catalog_for(revision: Json, host: Arc<FixedHost>) -> PolicyCatalog {
    PolicyCatalog::with_host(Arc::new(store_for(&[revision]).unwrap()), host)
}

pub fn bundled_catalog(host: Arc<FixedHost>) -> PolicyCatalog {
    PolicyCatalog::with_host(mxc_policy_store::bundled_catalog_store().unwrap(), host)
}

pub fn j(text: &str) -> Json {
    Json::parse(text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

pub fn revision_with(entries: Vec<Json>, id: &str) -> Json {
    let mut o = JsonObject::new();
    o.insert("catalogSchemaVersion", "1".into());
    o.insert("catalogRevision", id.into());
    o.insert("entries", Json::Array(entries));
    Json::Object(o)
}

pub fn revision(entries: Vec<Json>) -> Json {
    revision_with(entries, "2000-01-01.1")
}

/// Minimal valid entry (linux, readwrite `${project_root}`) with overrides.
pub fn entry(entry_id: &str, overrides: &str) -> Json {
    let name = entry_id.split(':').nth(1).unwrap();
    let mut base = j(&format!(
        r#"{{"entryId":"{entry_id}","entryRevision":1,"displayName":"{name}",
            "identity":[{{"kind":"invocation-name","names":["{name}"]}}],
            "platformVariants":[{{"when":{{"platform":"linux"}},"sandboxPolicy":{{"version":"0.9.0-alpha","filesystem":{{"readwritePaths":["${{project_root}}"]}}}}}}],
            "provenance":{{"method":"test","sourceRevision":"test"}}}}"#
    ));
    if !overrides.is_empty() {
        if let (Json::Object(target), Json::Object(extra)) = (&mut base, j(overrides)) {
            for (k, v) in extra.iter() {
                target.insert(k, v.clone());
            }
        }
    }
    base
}

pub fn reason_of<T: std::fmt::Debug>(result: Result<T, PolicyCatalogError>) -> ErrorReason {
    let error = result.expect_err("expected a PolicyCatalogError");
    assert_eq!(error.code(), error.reason().code());
    assert!(error
        .message()
        .starts_with(&format!("[{}] ", error.code().as_str())));
    error.reason()
}

/// Fixture `tools` (single input or array), through the binding parser.
pub fn tools_from(value: &Json) -> ToolInputs {
    mxc_policy_store::request::parse_tool_inputs(value).expect("fixture tools parse")
}

/// Fixture `context`, through the binding parser.
pub fn context_from(value: Option<&Json>) -> ResolveContext {
    mxc_policy_store::request::parse_resolve_context(value).expect("fixture context parses")
}
