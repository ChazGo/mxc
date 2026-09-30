// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Shared helpers for the integration tests. Tests read the repository's
//! `../conformance` and `../catalog` through `CARGO_MANIFEST_DIR`; they do
//! not ship in the `.crate`.

#![allow(dead_code)]

use mxc_policy_catalog::tooling::{Json, JsonObject};
use mxc_policy_catalog::{
    Architecture, CatalogStore, ErrorReason, FixedHost, MemorySource, Platform, PolicyCatalog, PolicyCatalogError,
    ResolveContext, SymbolMap, ToolCandidate, ToolInput, ToolInputs,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub fn repo_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

pub fn read_json(path: PathBuf) -> Json {
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    Json::parse(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// The contract shipped with the embedded catalog.
pub fn contract() -> Json {
    let files = mxc_policy_catalog::tooling::bundled_catalog_files();
    let text = files.iter().find(|(name, _)| *name == "contract.v1.json").unwrap().1;
    Json::parse(text).unwrap()
}

pub fn fixed_host(platform: Platform, architecture: Architecture) -> Arc<FixedHost> {
    Arc::new(FixedHost::new(platform, architecture))
}

pub fn linux_x64() -> Arc<FixedHost> {
    fixed_host(Platform::Linux, Architecture::X64)
}

pub fn store_for(revisions: &[Json]) -> Result<CatalogStore, PolicyCatalogError> {
    store_with(revisions, None, &HashMap::new())
}

pub fn store_with(
    revisions: &[Json],
    default_revision: Option<&str>,
    digests: &HashMap<String, String>,
) -> Result<CatalogStore, PolicyCatalogError> {
    CatalogStore::new(MemorySource::publishing(
        contract(),
        revisions,
        default_revision,
        digests,
    ))
}

pub fn catalog_for(revision: Json, host: Arc<FixedHost>) -> PolicyCatalog {
    PolicyCatalog::with_host(Arc::new(store_for(&[revision]).unwrap()), host)
}

pub fn bundled_catalog(host: Arc<FixedHost>) -> PolicyCatalog {
    PolicyCatalog::with_host(mxc_policy_catalog::bundled_catalog_store().unwrap(), host)
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
    assert!(error.message().starts_with(&format!("[{}] ", error.code().as_str())));
    error.reason()
}

fn candidate(value: &Json) -> ToolInput {
    match value {
        Json::String(s) => ToolInput::Name(s.clone()),
        Json::Object(o) => ToolInput::Candidate(ToolCandidate {
            invocation_name: o.get("invocationName").and_then(Json::as_str).unwrap().to_string(),
            package_url: o.get("packageUrl").and_then(Json::as_str).map(str::to_string),
            detected_version: o.get("detectedVersion").and_then(Json::as_str).map(str::to_string),
        }),
        other => panic!("unsupported tool input {other}"),
    }
}

/// Fixture `tools` (single input or array) as library inputs.
pub fn tools_from(value: &Json) -> ToolInputs {
    match value {
        Json::Array(items) => ToolInputs(items.iter().map(candidate).collect()),
        single => ToolInputs(vec![candidate(single)]),
    }
}

/// Fixture `context` as a library context.
pub fn context_from(value: Option<&Json>) -> ResolveContext {
    let mut ctx = ResolveContext::new();
    let Some(value) = value else { return ctx };
    let s = |k: &str| value.get(k).and_then(Json::as_str).map(str::to_string);
    ctx.project_root = s("projectRoot");
    ctx.platform = s("platform");
    ctx.architecture = s("architecture");
    ctx.catalog_revision = s("catalogRevision");
    ctx.allow_weak_identity_fallback = value.get("allowWeakIdentityFallback").and_then(Json::as_bool) == Some(true);
    if let Some(symbols) = value.get("symbols").and_then(Json::as_object) {
        let mut map = SymbolMap::new();
        for (k, v) in symbols.iter() {
            map.insert(k, v.as_str().unwrap());
        }
        ctx.symbols = Some(map);
    }
    ctx
}
