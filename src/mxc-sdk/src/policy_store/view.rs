// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! The rendered reviewer view of a catalog revision (design §7): every
//! effective policy, by entry, version option, platform, architecture, and
//! intent, so reviewers see what each combination actually grants.

use crate::policy_store::catalog::{entry_index, CatalogContract, CatalogRevision};
use crate::policy_store::effective::{materialize_entry, Materialized};
use crate::policy_store::errors::Result;
use crate::policy_store::exact::{bind_fixture_symbols, exact_document};
use crate::policy_store::json::{canonical_json, cmp_utf16, Json, JsonObject};
use crate::policy_store::model::{Architecture, FilesystemRequirements, Platform};

fn cell(values: &[String]) -> String {
    if values.is_empty() {
        "—".to_string()
    } else {
        values
            .iter()
            .map(|v| format!("`{v}`"))
            .collect::<Vec<_>>()
            .join("<br>")
    }
}

fn paths(
    m: &Materialized,
    field: fn(&FilesystemRequirements) -> &Option<Vec<String>>,
) -> Vec<String> {
    m.requirements
        .filesystem
        .as_ref()
        .and_then(|fs| field(fs).clone())
        .unwrap_or_default()
}

fn describe_rule(rule: &Json) -> String {
    let peers: Vec<String> = rule
        .get("to")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
        .map(|peer| {
            let cidr = peer.get("cidr").and_then(Json::as_str).unwrap_or("?");
            let except: Vec<&str> = peer
                .get("except")
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
                .filter_map(Json::as_str)
                .collect();
            if except.is_empty() {
                cidr.to_string()
            } else {
                format!("{cidr} except {}", except.join(","))
            }
        })
        .collect();
    let ports: Vec<String> = rule
        .get("ports")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
        .map(|port| {
            let protocol = port.get("protocol").and_then(Json::as_str).unwrap_or("any");
            let number = |key| port.get(key).and_then(Json::as_f64).map(|n| n.to_string());
            match (number("port"), number("endPort")) {
                (Some(start), Some(end)) => format!("{protocol}/{start}-{end}"),
                (Some(start), None) => format!("{protocol}/{start}"),
                _ => protocol.to_string(),
            }
        })
        .collect();
    let peers = if peers.is_empty() {
        "*".to_string()
    } else {
        peers.join("+")
    };
    if ports.is_empty() {
        format!("{peers} any port")
    } else {
        format!("{peers} {}", ports.join("+"))
    }
}

fn egress(m: &Materialized) -> Vec<String> {
    let Some(network) = &m.requirements.network else {
        return Vec::new();
    };
    let allow: Vec<String> = network
        .get("egress")
        .and_then(|e| e.get("allow"))
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
        .map(describe_rule)
        .collect();
    if allow.is_empty() {
        vec![format!("{network}")]
    } else {
        allow
    }
}

type PathField = fn(&FilesystemRequirements) -> &Option<Vec<String>>;

/// What the row adds to the common default for the same intent: paths,
/// outbound rules, and dependencies the default does not have.
fn added(m: &Materialized) -> Vec<String> {
    let baseline = Materialized {
        requirements: m.default_requirements.clone(),
        dependency_entry_ids: m.default_dependency_entry_ids.clone(),
        ..m.clone()
    };
    let mut out = Vec::new();
    let classes: [(&str, PathField); 2] = [
        ("ro", |f| &f.readonly_paths),
        ("rw", |f| &f.readwrite_paths),
    ];
    for (label, field) in classes {
        let before = paths(&baseline, field);
        out.extend(
            paths(m, field)
                .into_iter()
                .filter(|p| !before.contains(p))
                .map(|p| format!("+{label} {p}")),
        );
    }
    let before = egress(&baseline);
    out.extend(
        egress(m)
            .into_iter()
            .filter(|r| !before.contains(r))
            .map(|r| format!("+allow {r}")),
    );
    out.extend(
        m.dependency_entry_ids
            .iter()
            .filter(|d| !baseline.dependency_entry_ids.contains(d))
            .map(|d| format!("+dependency {d}")),
    );
    out
}

fn row_cells(m: &Materialized) -> [String; 6] {
    [
        cell(&paths(m, |f| &f.readonly_paths)),
        cell(&paths(m, |f| &f.readwrite_paths)),
        cell(&paths(m, |f| &f.denied_paths)),
        cell(&egress(m)),
        cell(&m.dependency_entry_ids),
        cell(&added(m)),
    ]
}

/// Renders the reviewer view (Markdown). Architectures with identical rows
/// collapse into one `any` row.
pub fn render_reviewer_view(
    revision: &CatalogRevision,
    contract: &CatalogContract,
) -> Result<String> {
    let by_id = entry_index(&revision.entries);
    let mut entries: Vec<_> = revision.entries.iter().collect();
    entries.sort_by(|a, b| cmp_utf16(&a.entry_id, &b.entry_id));
    let mut out = String::new();
    out.push_str(&format!(
        "<!-- Generated by mxc_sdk policy_store::tooling::render_reviewer_view. Do not edit. -->\n\n# Catalog revision {} — reviewer view\n\nEvery effective policy each entry can produce (its command-free container requirements): the default, plus platform additions, at most one version variant, and the selected intent. `(all)` means no intent was requested. Paths are unsubstituted templates. *Added to default* lists what the row adds to the common default (no platform or version overlay) for the same intent, or to the default's base for an intent the default does not declare.\n",
        revision.catalog_revision
    ));
    for entry in entries {
        let materialized = materialize_entry(entry, &by_id, contract)?;
        out.push_str(&format!(
            "\n## `{}` — {} (revision {}, `{}` versions)\n",
            entry.entry_id,
            entry.display_name,
            crate::policy_store::catalog::revision_number(entry.entry_revision),
            entry.version_scheme
        ));
        let mut options: Vec<Option<String>> = vec![None];
        options.extend(
            entry
                .version_variants
                .iter()
                .map(|v| Some(v.version_range.as_str().to_string())),
        );
        for option in options {
            out.push_str(&format!(
                "\n### {}\n\n| Platform | Arch | Intent | Read-only | Read-write | Denied | Outbound allow | Dependencies | Added to default |\n|---|---|---|---|---|---|---|---|---|\n",
                option
                    .as_deref()
                    .map_or("Default (no detected version or out of range)".to_string(), |r| {
                        format!("Version `{r}`")
                    })
            ));
            for platform in Platform::ALL {
                let rows: Vec<&Materialized> = materialized
                    .iter()
                    .filter(|m| m.platform == platform && m.version_range == option)
                    .collect();
                let mut intents: Vec<Option<String>> = Vec::new();
                for m in &rows {
                    if !intents.contains(&m.intent) {
                        intents.push(m.intent.clone());
                    }
                }
                for intent in intents {
                    let per_arch: Vec<(&Materialized, [String; 6])> = Architecture::ALL
                        .iter()
                        .filter_map(|arch| {
                            rows.iter()
                                .find(|m| m.architecture == *arch && m.intent == intent)
                        })
                        .map(|m| (*m, row_cells(m)))
                        .collect();
                    let same = per_arch.windows(2).all(|w| w[0].1 == w[1].1)
                        && per_arch.len() == Architecture::ALL.len();
                    let shown: Vec<(String, &[String; 6])> = if same {
                        vec![("any".to_string(), &per_arch[0].1)]
                    } else {
                        per_arch
                            .iter()
                            .map(|(m, c)| (m.architecture.to_string(), c))
                            .collect()
                    };
                    for (arch, cells) in shown {
                        out.push_str(&format!(
                            "| {platform} | {arch} | {} | {} |\n",
                            intent.as_deref().unwrap_or("(all)"),
                            cells.join(" | ")
                        ));
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Every distinct exact SDK target document the revision's materialized
/// combinations produce with fixture symbols and the validation-only command
/// bound (design §4.2), with the combinations that produce it. CI validates
/// each document against the registered schema for its version.
pub fn render_exact_requests(
    revision: &CatalogRevision,
    contract: &CatalogContract,
) -> Result<Json> {
    let by_id = entry_index(&revision.entries);
    let mut entries: Vec<_> = revision.entries.iter().collect();
    entries.sort_by(|a, b| cmp_utf16(&a.entry_id, &b.entry_id));
    let mut requests: Vec<(String, Json, Vec<Json>)> = Vec::new();
    for entry in entries {
        for m in materialize_entry(entry, &by_id, contract)? {
            let document = exact_document(&bind_fixture_symbols(&m.requirements, m.platform));
            let key = canonical_json(&document);
            let combination = Json::String(format!(
                "{} {}/{} {} {}",
                entry.entry_id,
                m.platform,
                m.architecture,
                m.version_range.as_deref().unwrap_or("default"),
                m.intent.as_deref().unwrap_or("(all)")
            ));
            match requests.iter_mut().find(|(k, _, _)| *k == key) {
                Some((_, _, combinations)) => combinations.push(combination),
                None => requests.push((key, document, vec![combination])),
            }
        }
    }
    let mut root = JsonObject::new();
    root.insert(
        "$comment",
        "Generated by mxc_sdk policy_store::tooling::render_exact_requests. Do not edit. Each document is the exact SDK target request for the listed combinations, with fixture symbols and a validation-only command; scripts/versioning/validate-configs.js validates it against the registered schema.".into(),
    );
    root.insert("catalogRevision", revision.catalog_revision.as_str().into());
    root.insert(
        "sdkContractVersion",
        revision.sdk_contract_version.as_str().into(),
    );
    root.insert(
        "requests",
        Json::Array(
            requests
                .into_iter()
                .map(|(_, document, combinations)| {
                    let mut o = JsonObject::new();
                    o.insert("combinations", Json::Array(combinations));
                    o.insert("document", document);
                    Json::Object(o)
                })
                .collect(),
        ),
    );
    Ok(Json::Object(root))
}
