// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Contribution validation for the bundled catalog. These checks replace the
//! prototype's npm `validate` pipeline:
//!
//! - integrity, contract, and entry-revision history of every revision;
//! - immutability of published revisions against a base git ref, when
//!   `MXC_POLICY_STORE_BASE_REF` names one (for example `origin/main`);
//! - every revision file is listed in the manifest;
//! - every entry resolves deterministically, through its own identity, for
//!   every platform and architecture (every effective policy is also
//!   materialized and validated by `validate_catalog_revision`);
//! - the rendered reviewer view and exact-request bundle in `catalog/views/`
//!   are current (CI validates the bundle against the SDK target schema);
//! - every entry has a bundled-catalog conformance case, as a match or a
//!   resolved dependency.
//!
//! The JSON Schemas in `schema/` document the same shape for editors. The
//! semantic validation exercised here is a superset of them.

#[path = "policy_store_common/mod.rs"]
mod common;

use common::{crate_dir, read_json};
use mxc_sdk::__policy_store::tooling::{
    canonical_json, render_exact_requests, render_reviewer_view, validate_bundled_catalog,
    IdentityPredicate, Json,
};
use mxc_sdk::__policy_store::{
    bundled_catalog_store, Architecture, FixedHost, Platform, PolicyCatalog, ResolveContext,
    ToolCandidate,
};
use std::collections::HashSet;
use std::sync::Arc;

#[test]
fn bundled_catalog_passes_contribution_validation() {
    let base_ref = std::env::var("MXC_POLICY_STORE_BASE_REF")
        .ok()
        .filter(|r| !r.is_empty());
    let report = validate_bundled_catalog(base_ref.as_deref());
    assert!(report.ok, "catalog validation failed: {:#?}", report.errors);
    if let Some(base_ref) = base_ref {
        let (reported, _) = report.base_ref.expect("base ref was checked");
        assert_eq!(reported, base_ref);
    }
}

#[test]
fn every_revision_file_is_listed_in_the_manifest() {
    let store = bundled_catalog_store().unwrap();
    let listed: HashSet<&str> = store
        .manifest()
        .revisions
        .iter()
        .map(|r| r.file.as_str())
        .collect();
    for entry in std::fs::read_dir(crate_dir().join("catalog").join("revisions")).unwrap() {
        let file = format!("revisions/{}", entry.unwrap().file_name().to_string_lossy());
        assert!(
            listed.contains(file.as_str()),
            "{file} is not listed in manifest.json"
        );
    }
}

fn context_for(
    store: &mxc_sdk::__policy_store::CatalogStore,
    platform: Platform,
    revision: &str,
) -> ResolveContext {
    let (base, sep) = match platform {
        Platform::Windows => ("C:\\ci", "\\"),
        _ => ("/ci", "/"),
    };
    let mut ctx = ResolveContext::new()
        .catalog_revision(revision)
        .allow_weak(true)
        .project_root(format!("{base}{sep}project"));
    for (name, definition) in store.contract().symbols() {
        if !matches!(
            definition.source,
            mxc_sdk::__policy_store::tooling::SymbolSource::Context
        ) {
            ctx = ctx.symbol(name, format!("{base}{sep}{name}"));
        }
    }
    ctx
}

#[test]
fn every_entry_resolves_deterministically_through_its_own_identity() {
    let store = bundled_catalog_store().unwrap();
    let mut failures = Vec::new();
    for revision_id in store.available_revisions() {
        let revision = store.revision(Some(&revision_id)).unwrap();
        for entry in &revision.entries {
            let mut candidate = ToolCandidate::new("unused");
            for predicate in &entry.identity {
                match predicate {
                    IdentityPredicate::InvocationName { names } => {
                        candidate.invocation_name = names[0].clone()
                    }
                    IdentityPredicate::Purl { value } => {
                        candidate.package_url = Some(value.clone())
                    }
                }
            }
            for platform in Platform::ALL {
                for architecture in Architecture::ALL {
                    let catalog = PolicyCatalog::with_host(
                        store.clone(),
                        Arc::new(FixedHost::new(platform, architecture)),
                    );
                    let ctx = context_for(&store, platform, &revision_id)
                        .platform(platform)
                        .architecture(architecture);
                    let label =
                        format!("{revision_id} {} {platform}/{architecture}", entry.entry_id);
                    let first =
                        catalog.resolve_requirements_with_diagnostics(candidate.clone(), &ctx);
                    let second =
                        catalog.resolve_requirements_with_diagnostics(candidate.clone(), &ctx);
                    match (first, second) {
                        (Ok(first), Ok(second)) => {
                            if first.requirements.is_none() {
                                failures.push(format!(
                                    "{label} produced no requirements: {:?}",
                                    first.diagnostics.warnings
                                ));
                            } else if first.to_json() != second.to_json() {
                                failures.push(format!("{label} did not resolve deterministically"));
                            } else if !first.diagnostics.tools[0]
                                .matches
                                .iter()
                                .any(|m| m.entry_id == entry.entry_id)
                            {
                                failures
                                    .push(format!("{label} was not matched by its own identity"));
                            }
                        }
                        (Err(error), _) | (_, Err(error)) => {
                            failures.push(format!("{label} failed: {error}"))
                        }
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn every_entry_has_a_bundled_conformance_case() {
    let mut covered = HashSet::new();
    for file in std::fs::read_dir(crate_dir().join("conformance").join("fixtures")).unwrap() {
        let fixture = read_json(file.unwrap().path());
        if fixture.get("catalog").and_then(Json::as_str) != Some("bundled") {
            continue;
        }
        for case in fixture
            .get("cases")
            .and_then(Json::as_array)
            .into_iter()
            .flatten()
        {
            let diagnostics = case.get("expect").and_then(|e| e.get("diagnostics"));
            for dependency in diagnostics
                .and_then(|d| d.get("resolvedDependencies"))
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(id) = dependency.get("entryId").and_then(Json::as_str) {
                    covered.insert(id.to_string());
                }
            }
            let tools = diagnostics
                .and_then(|d| d.get("tools"))
                .and_then(Json::as_array);
            for tool in tools.into_iter().flatten() {
                for m in tool
                    .get("matches")
                    .and_then(Json::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(id) = m.get("entryId").and_then(Json::as_str) {
                        covered.insert(id.to_string());
                    }
                }
            }
        }
    }
    let store = bundled_catalog_store().unwrap();
    let missing: Vec<String> = store
        .revision(None)
        .unwrap()
        .entries
        .iter()
        .filter(|e| !covered.contains(&e.entry_id))
        .map(|e| e.entry_id.clone())
        .collect();
    assert!(
        missing.is_empty(),
        "entries without a bundled conformance case: {missing:?}"
    );
}

/// The reviewer view and exact-request bundle are generated; set
/// `MXC_POLICY_STORE_UPDATE_VIEWS=1` to rewrite them after a catalog change.
#[test]
fn reviewer_views_are_current() {
    let store = bundled_catalog_store().unwrap();
    let dir = crate_dir().join("catalog").join("views");
    let update = std::env::var("MXC_POLICY_STORE_UPDATE_VIEWS").is_ok_and(|v| v == "1");
    for revision_id in store.available_revisions() {
        let revision = store.revision(Some(&revision_id)).unwrap();
        let rendered = render_reviewer_view(&revision, store.contract())
            .unwrap()
            .replace('\n', "\r\n");
        let path = dir.join(format!("{revision_id}.md"));
        let requests = render_exact_requests(&revision, store.contract()).unwrap();
        let requests_path = dir.join(format!("{revision_id}.requests.json"));
        if update {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, &rendered).unwrap();
            std::fs::write(&requests_path, format!("{}\n", requests.to_pretty_string())).unwrap();
            continue;
        }
        let committed_requests = read_json(requests_path.clone());
        assert_eq!(
            canonical_json(&committed_requests),
            canonical_json(&requests),
            "{} is stale; regenerate with MXC_POLICY_STORE_UPDATE_VIEWS=1",
            requests_path.display()
        );
        let committed = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        assert_eq!(
            committed.replace("\r\n", "\n"),
            rendered.replace("\r\n", "\n"),
            "{} is stale; regenerate with MXC_POLICY_STORE_UPDATE_VIEWS=1",
            path.display()
        );
    }
}
