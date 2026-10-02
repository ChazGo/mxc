// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Entry-revision monotonicity and published-revision immutability

use crate::catalog::{
    compare_catalog_revisions, entry_semantic_key, revision_number, CatalogRevision,
};
use crate::json::{canonical_json, Json};
use crate::store::CatalogStore;
use std::cmp::Ordering;
use std::collections::HashMap;

/// Checks entry-revision monotonicity between two consecutive revisions: a
/// changed entry must bump `entryRevision`; an unchanged one must keep it.
pub fn check_entry_revisions(previous: &CatalogRevision, next: &CatalogRevision) -> Vec<String> {
    let mut errors = Vec::new();
    let newer = compare_catalog_revisions(&previous.catalog_revision, &next.catalog_revision)
        .map(|o| o == Ordering::Less)
        .unwrap_or(false);
    if !newer {
        errors.push(format!(
            "catalog revision '{}' must be newer than '{}'",
            next.catalog_revision, previous.catalog_revision
        ));
    }
    let before: HashMap<&str, _> = previous
        .entries
        .iter()
        .map(|e| (e.entry_id.as_str(), e))
        .collect();
    for entry in &next.entries {
        let Some(old) = before.get(entry.entry_id.as_str()) else {
            continue;
        };
        let changed = entry_semantic_key(old) != entry_semantic_key(entry);
        if changed && entry.entry_revision <= old.entry_revision {
            errors.push(format!(
                "{}: '{}' changed but entryRevision did not increase ({} -> {})",
                next.catalog_revision,
                entry.entry_id,
                revision_number(old.entry_revision),
                revision_number(entry.entry_revision)
            ));
        } else if !changed && entry.entry_revision != old.entry_revision {
            errors.push(format!(
                "{}: '{}' is unchanged but entryRevision moved ({} -> {})",
                next.catalog_revision,
                entry.entry_id,
                revision_number(old.entry_revision),
                revision_number(entry.entry_revision)
            ));
        }
    }
    errors
}

/// Validates every installed revision (integrity + contract) and the
/// entry-revision chain between consecutive revisions.
pub fn check_store_history(store: &CatalogStore) -> Vec<String> {
    let mut errors = Vec::new();
    let mut previous: Option<std::sync::Arc<CatalogRevision>> = None;
    for id in store.available_revisions() {
        match store.revision(Some(&id)) {
            Err(error) => {
                errors.push(error.message().to_string());
                previous = None;
            }
            Ok(current) => {
                if let Some(previous) = &previous {
                    errors.extend(check_entry_revisions(previous, &current));
                }
                previous = Some(current);
            }
        }
    }
    errors
}

/// One manifest revision as read (unvalidated) from a published manifest.
#[derive(Clone, Debug, PartialEq)]
pub struct PublishedRevision {
    pub catalog_revision: Option<Json>,
    pub file: Option<Json>,
}

impl PublishedRevision {
    pub fn new(catalog_revision: &str, file: &str) -> Self {
        Self {
            catalog_revision: Some(Json::from(catalog_revision)),
            file: Some(Json::from(file)),
        }
    }

    fn from_json(value: &Json) -> Self {
        Self {
            catalog_revision: value.get("catalogRevision").cloned(),
            file: value.get("file").cloned(),
        }
    }

    fn file_key(&self) -> Option<String> {
        self.file.as_ref().map(file_key)
    }
}

/// Map key for a manifest `file` value (TypeScript uses it as a Map key).
fn file_key(value: &Json) -> String {
    match value {
        Json::String(s) => s.clone(),
        other => format!("\u{0}{}", other.to_compact_string()),
    }
}

fn describe_js(value: &Option<Json>) -> String {
    match value {
        None => "undefined".to_string(),
        Some(v) => crate::json::js_to_string(v),
    }
}

/// Raw published state compared across a proposed change.
#[derive(Clone, Debug, Default)]
pub struct PublishedState {
    pub revisions: Vec<PublishedRevision>,
    /// Raw file text keyed by manifest `file`.
    pub files: HashMap<String, String>,
}

impl PublishedState {
    /// Reads `manifest.revisions` from a parsed manifest (absent → empty).
    pub fn revisions_from_manifest(manifest: &Json) -> Vec<PublishedRevision> {
        manifest
            .get("revisions")
            .and_then(Json::as_array)
            .map(|items| items.iter().map(PublishedRevision::from_json).collect())
            .unwrap_or_default()
    }

    pub(crate) fn file_text(&self, revision: &PublishedRevision) -> Option<&String> {
        revision.file_key().and_then(|k| self.files.get(&k))
    }
}

fn canonical_text(text: Option<&String>) -> Option<String> {
    let text = text?;
    Some(match Json::parse(text) {
        Ok(value) => canonical_json(&value),
        Err(_) => format!("invalid:{text}"),
    })
}

/// Published revisions keep their manifest entry and content; new
/// revisions are only appended (design §10).
pub fn check_published_immutability(
    base: &PublishedState,
    proposed: &PublishedState,
) -> Vec<String> {
    let mut errors = Vec::new();
    for (index, published) in base.revisions.iter().enumerate() {
        let name = describe_js(&published.catalog_revision);
        let now = proposed.revisions.get(index);
        let Some(now) = now.filter(|now| now.catalog_revision == published.catalog_revision) else {
            errors.push(format!(
                "published revision '{name}' was removed or reordered in the manifest"
            ));
            continue;
        };
        if now.file != published.file {
            errors.push(format!(
                "published revision '{name}' manifest entry was modified"
            ));
        }
        let base_content = canonical_text(base.file_text(published));
        let proposed_content = canonical_text(proposed.file_text(published));
        if base_content.is_some() && proposed_content != base_content {
            errors.push(format!(
                "published revision file '{}' was modified; publish a new revision instead",
                describe_js(&published.file)
            ));
        }
    }
    errors
}
