// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Catalog source assembly (design §6.3). Authors edit one JSON file per tool
//! under `catalog/entries/`; this module assembles those files into the
//! complete candidate revision that `revisions/<catalogRevision>.json`
//! snapshots. Paths and file order never affect the result: entries are
//! ordered by `entryId`, and a path only appears in error messages.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::policy_store::catalog::SDK_CONTRACT_VERSION;
use crate::policy_store::json::{Json, JsonObject};

/// One editable entry source: a display path (for errors only) and its text.
#[derive(Clone, Debug)]
pub struct EntrySource {
    pub path: String,
    pub text: String,
}

/// Reads every `*.json` file under `dir`, recursively. Category
/// subdirectories are organizational only.
pub fn collect_entry_sources(dir: &Path) -> std::io::Result<Vec<EntrySource>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<EntrySource>) -> std::io::Result<()> {
        for item in std::fs::read_dir(dir)? {
            let path: PathBuf = item?.path();
            if path.is_dir() {
                walk(root, &path, out)?;
            } else if path.extension().is_some_and(|e| e == "json") {
                let display = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push(EntrySource {
                    path: display,
                    text: std::fs::read_to_string(&path)?,
                });
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    Ok(out)
}

/// Every `dependencies[].entryId` anywhere in an entry: the default, intents,
/// and every overlay.
fn dependency_references(value: &Json, out: &mut BTreeSet<String>) {
    match value {
        Json::Object(object) => {
            for (key, child) in object.iter() {
                if key == "dependencies" {
                    for item in child.as_array().into_iter().flatten() {
                        if let Some(id) = item.get("entryId").and_then(Json::as_str) {
                            out.insert(id.to_string());
                        }
                    }
                }
                dependency_references(child, out);
            }
        }
        Json::Array(items) => items
            .iter()
            .for_each(|item| dependency_references(item, out)),
        _ => {}
    }
}

/// Assembles entry sources into the revision document for `catalog_revision`.
///
/// Checks that every source is one entry object with a string `entryId`, that
/// `entryId` values are globally unique, and that every dependency reference
/// names an assembled entry. Full semantic validation of the result remains
/// `validate_catalog_revision`'s job. Errors are sorted for stable output.
pub fn assemble_revision(
    sources: &[EntrySource],
    catalog_schema_version: &str,
    catalog_revision: &str,
) -> Result<Json, Vec<String>> {
    let mut errors = Vec::new();
    let mut entries: BTreeMap<String, (String, Json)> = BTreeMap::new();
    for source in sources {
        let value = match Json::parse(&source.text) {
            Ok(value) => value,
            Err(error) => {
                errors.push(format!("{}: invalid JSON: {error}", source.path));
                continue;
            }
        };
        let Some(id) = value
            .as_object()
            .and_then(|o| o.get("entryId"))
            .and_then(Json::as_str)
            .map(str::to_string)
        else {
            errors.push(format!(
                "{}: must be one catalog entry object with a string 'entryId'",
                source.path
            ));
            continue;
        };
        if let Some((first, _)) = entries.get(&id) {
            let (a, b) = if first.as_str() <= source.path.as_str() {
                (first.as_str(), source.path.as_str())
            } else {
                (source.path.as_str(), first.as_str())
            };
            errors.push(format!("entryId '{id}' is defined in both {a} and {b}"));
            continue;
        }
        entries.insert(id, (source.path.clone(), value));
    }
    for (id, (path, value)) in &entries {
        let mut references = BTreeSet::new();
        dependency_references(value, &mut references);
        for reference in references {
            if !entries.contains_key(&reference) {
                errors.push(format!(
                    "{path}: entry '{id}' depends on '{reference}', which no entry file defines"
                ));
            }
        }
    }
    if !errors.is_empty() {
        errors.sort();
        return Err(errors);
    }
    let mut root = JsonObject::new();
    root.insert("catalogSchemaVersion", catalog_schema_version.into());
    root.insert("catalogRevision", catalog_revision.into());
    root.insert("sdkContractVersion", SDK_CONTRACT_VERSION.into());
    root.insert(
        "entries",
        Json::Array(entries.into_values().map(|(_, value)| value).collect()),
    );
    Ok(Json::Object(root))
}

/// The generated snapshot text for an assembled revision.
pub fn render_revision(revision: &Json) -> String {
    format!("{}\n", revision.to_pretty_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(path: &str, text: &str) -> EntrySource {
        EntrySource {
            path: path.into(),
            text: text.into(),
        }
    }

    fn ids(revision: &Json) -> Vec<String> {
        revision
            .get("entries")
            .and_then(Json::as_array)
            .unwrap()
            .iter()
            .map(|e| e.get("entryId").and_then(Json::as_str).unwrap().to_string())
            .collect()
    }

    const A: &str = r#"{"entryId":"tool:a","default":{"dependencies":[{"entryId":"tool:b"}]}}"#;
    const B: &str = r#"{"entryId":"tool:b","default":{}}"#;

    #[test]
    fn orders_entries_by_id_whatever_the_paths() {
        let forward = assemble_revision(&[source("a.json", A), source("b.json", B)], "1", "r");
        let moved = assemble_revision(
            &[source("x/z/b.json", B), source("y/0-a.json", A)],
            "1",
            "r",
        );
        let forward = render_revision(&forward.unwrap());
        assert_eq!(forward, render_revision(&moved.unwrap()));
        let revision = Json::parse(&forward).unwrap();
        assert_eq!(ids(&revision), ["tool:a", "tool:b"]);
        assert_eq!(
            revision.get("catalogRevision").and_then(Json::as_str),
            Some("r")
        );
        assert_eq!(
            revision.get("sdkContractVersion").and_then(Json::as_str),
            Some(SDK_CONTRACT_VERSION)
        );
    }

    #[test]
    fn rejects_a_duplicate_entry_id() {
        let errors = assemble_revision(
            &[
                source("b.json", B),
                source("dev/b.json", B),
                source("a.json", A),
            ],
            "1",
            "r",
        )
        .unwrap_err();
        assert_eq!(
            errors,
            ["entryId 'tool:b' is defined in both b.json and dev/b.json"]
        );
    }

    #[test]
    fn rejects_a_dangling_dependency_anywhere_in_an_entry() {
        let errors = assemble_revision(&[source("a.json", A)], "1", "r").unwrap_err();
        assert_eq!(
            errors,
            ["a.json: entry 'tool:a' depends on 'tool:b', which no entry file defines"]
        );
        let nested = r#"{"entryId":"tool:c","versionVariants":[{"intentAdditions":
            {"push":{"dependencies":[{"entryId":"tool:missing","intents":["x"]}]}}}]}"#;
        let errors = assemble_revision(&[source("c.json", nested)], "1", "r").unwrap_err();
        assert_eq!(
            errors,
            ["c.json: entry 'tool:c' depends on 'tool:missing', which no entry file defines"]
        );
    }

    #[test]
    fn rejects_a_file_that_is_not_one_entry() {
        let errors = assemble_revision(
            &[source("list.json", "[]"), source("bad.json", "{")],
            "1",
            "r",
        )
        .unwrap_err();
        assert_eq!(errors.len(), 2);
        assert!(errors[0].starts_with("bad.json: invalid JSON"));
        assert!(errors[1].starts_with("list.json: must be one catalog entry object"));
    }
}
