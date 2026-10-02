// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Floor composition of selected components (design §4.5): every selected
//! base, its selected additions, and its dependencies combine into one
//! `SandboxPolicy` that preserves the access each requested pair needs.

use crate::catalog::{Additions, CatalogPolicy};
use crate::json::{canonical_json, cmp_utf16, Json, JsonObject};
use crate::model::{FilesystemPolicy, Platform, SandboxPolicy};
use crate::paths::{normalize_path, path_exact_segments, path_key_segments};
use crate::text::symbol_matches;
use std::collections::HashSet;

/// One selected (entry, base, additions) contribution.
#[derive(Clone, Debug)]
pub struct Component<'a> {
    pub entry_id: &'a str,
    pub base: &'a CatalogPolicy,
    pub additions: Vec<&'a Additions>,
}

impl Component<'_> {
    fn has_additions(&self) -> bool {
        self.additions.iter().any(|a| !a.is_empty())
    }

    fn needs_network(&self) -> bool {
        self.base.field("network").is_some()
            || self.additions.iter().any(|a| !a.egress_allow.is_empty())
    }

    /// Path templates of one access class, base first, then additions.
    fn paths(&self, class: Class) -> Vec<&str> {
        let mut out = self.base.filesystem_field(class.field());
        for additions in &self.additions {
            let extra = match class {
                Class::Denied => &[][..],
                Class::Readonly => &additions.readonly_paths[..],
                Class::Readwrite => &additions.readwrite_paths[..],
            };
            out.extend(extra.iter().map(String::as_str));
        }
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Denied,
    Readonly,
    Readwrite,
}

impl Class {
    const ALL: [Class; 3] = [Class::Denied, Class::Readonly, Class::Readwrite];

    fn field(self) -> &'static str {
        match self {
            Class::Denied => "deniedPaths",
            Class::Readonly => "readonlyPaths",
            Class::Readwrite => "readwritePaths",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Class::Denied => "denied",
            Class::Readonly => "read-only",
            Class::Readwrite => "read-write",
        }
    }
}

/// The symbols the components reference, first-seen order, with the entries
/// that need each.
pub fn component_symbols(components: &[Component<'_>]) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for component in components {
        for class in Class::ALL {
            for template in component.paths(class) {
                for (_, _, name) in symbol_matches(template) {
                    match out.iter_mut().find(|(n, _)| n == name) {
                        Some((_, ids)) => {
                            if !ids.iter().any(|id| id == component.entry_id) {
                                ids.push(component.entry_id.to_string());
                            }
                        }
                        None => out.push((name.to_string(), vec![component.entry_id.to_string()])),
                    }
                }
            }
        }
    }
    out
}

/// Rejects combinations the v1 composition rules cannot express (design
/// §4.5), rather than approximating them with broader access.
pub fn compose_check(components: &[Component<'_>]) -> Result<(), String> {
    let mut versions: Vec<&str> = Vec::new();
    for component in components {
        let version = component.base.version();
        if !versions.contains(&version) {
            versions.push(version);
        }
    }
    if versions.len() > 1 {
        versions.sort_by(|a, b| cmp_utf16(a, b));
        return Err(format!(
            "mixed sandboxPolicy.version values ({})",
            versions.join(", ")
        ));
    }
    if is_passthrough(components) {
        return Ok(());
    }
    for component in components {
        for key in component.base.raw().keys() {
            if !matches!(key, "version" | "filesystem" | "network") {
                return Err(format!(
                    "'{}' uses '{key}', which has no v1 cross-policy composition rule",
                    component.entry_id
                ));
            }
        }
    }
    if verbatim_network(components).is_some() {
        return Ok(());
    }
    for component in components {
        let Some(network) = component.base.field("network").and_then(Json::as_object) else {
            continue;
        };
        for (key, value) in network.iter() {
            if key != "egress" {
                return Err(format!(
                    "'{}' uses 'network.{key}', which cannot be combined with other network requirements",
                    component.entry_id
                ));
            }
            for (field, _) in value.as_object().into_iter().flat_map(|o| o.iter()) {
                if !matches!(field, "default" | "allow") {
                    return Err(format!(
                        "'{}' uses 'network.egress.{field}', which cannot be combined with other network requirements",
                        component.entry_id
                    ));
                }
            }
        }
    }
    Ok(())
}

/// A single selected policy without additions or dependencies keeps every
/// catalog-supported field without cross-policy composition.
fn is_passthrough(components: &[Component<'_>]) -> bool {
    components.len() == 1 && !components[0].has_additions()
}

/// The one component whose base network passes through unchanged: the only
/// component needing network, contributing no outbound additions.
fn verbatim_network<'c, 'a>(components: &'c [Component<'a>]) -> Option<&'c Component<'a>> {
    let mut needing = components.iter().filter(|c| c.needs_network());
    let only = needing.next()?;
    if needing.next().is_some() || only.additions.iter().any(|a| !a.egress_allow.is_empty()) {
        return None;
    }
    Some(only)
}

/// The composed policy and the composition diagnostics.
#[derive(Clone, Debug)]
pub struct Composed {
    pub policy: SandboxPolicy,
    pub warnings: Vec<String>,
}

struct PathItem {
    path: String,
    exact: Vec<String>,
    folded: Vec<String>,
    entry_ids: Vec<String>,
}

fn is_same_or_nested(left: &[String], right: &[String]) -> bool {
    let (shorter, longer) = if left.len() <= right.len() {
        (left, right)
    } else {
        (right, left)
    };
    shorter.iter().zip(longer).all(|(a, b)| a == b)
}

/// `inner` equals `outer` or lies beneath it.
fn is_within(inner: &[String], outer: &[String]) -> bool {
    inner.len() >= outer.len() && outer.iter().zip(inner).all(|(a, b)| a == b)
}

fn ids(item: &PathItem) -> String {
    item.entry_ids.join(", ")
}

/// Composes components that passed [`compose_check`]. `resolve` maps a path
/// template to its substituted value (identity for symbolic validation).
pub fn compose_policy(
    components: &[Component<'_>],
    resolve: &dyn Fn(&str) -> String,
    platform: Platform,
) -> Composed {
    let mut warnings = Vec::new();
    let mut classes: Vec<Vec<PathItem>> = Vec::new();
    for class in Class::ALL {
        let mut items: Vec<PathItem> = Vec::new();
        for component in components {
            for template in component.paths(class) {
                let path = normalize_path(&resolve(template), platform);
                let exact = path_exact_segments(&path, platform);
                match items.iter_mut().find(|i| i.exact == exact) {
                    Some(item) => {
                        if !item.entry_ids.iter().any(|id| id == component.entry_id) {
                            item.entry_ids.push(component.entry_id.to_string());
                        }
                    }
                    None => items.push(PathItem {
                        folded: path_key_segments(&path, Platform::Windows),
                        path,
                        exact,
                        entry_ids: vec![component.entry_id.to_string()],
                    }),
                }
            }
        }
        classes.push(items);
    }

    // Case sensitivity of the target filesystem is not determined, so paths
    // compare case-sensitively; report pairs that differ only by case.
    let all: Vec<(Class, &PathItem)> = Class::ALL
        .iter()
        .zip(&classes)
        .flat_map(|(class, items)| items.iter().map(move |i| (*class, i)))
        .collect();
    for (i, (left_class, left)) in all.iter().enumerate() {
        for (right_class, right) in &all[i + 1..] {
            if is_same_or_nested(&left.folded, &right.folded)
                && !is_same_or_nested(&left.exact, &right.exact)
            {
                warnings.push(format!(
                    "{} '{}' ({}) and {} '{}' ({}) differ only by case; filesystem case sensitivity was not determined, so they were compared case-sensitively and kept distinct",
                    left_class.label(),
                    left.path,
                    ids(left),
                    right_class.label(),
                    right.path,
                    ids(right)
                ));
            }
        }
    }

    let [denied, readonly, readwrite] = <[Vec<PathItem>; 3]>::try_from(classes)
        .unwrap_or_else(|_| unreachable!("three access classes"));

    let readonly: Vec<PathItem> = readonly
        .into_iter()
        .filter(|ro| match readwrite.iter().find(|rw| is_within(&ro.exact, &rw.exact)) {
            Some(rw) => {
                warnings.push(format!(
                    "read-only '{}' ({}) is covered by read-write '{}' ({}); the read-only entry was omitted",
                    ro.path,
                    ids(ro),
                    rw.path,
                    ids(rw)
                ));
                false
            }
            None => true,
        })
        .collect();

    let denied: Vec<PathItem> = denied
        .into_iter()
        .filter(|deny| {
            let grant = readwrite
                .iter()
                .map(|g| (Class::Readwrite, g))
                .chain(readonly.iter().map(|g| (Class::Readonly, g)))
                .find(|(_, g)| is_same_or_nested(&deny.exact, &g.exact));
            match grant {
                Some((class, g)) => {
                    warnings.push(format!(
                        "removed catalog deny '{}' ({}) because it overlaps required {} '{}' ({}); the entire deny scope '{}' was removed, so other grants may now apply throughout it",
                        deny.path,
                        ids(deny),
                        class.label(),
                        g.path,
                        ids(g),
                        deny.path
                    ));
                    false
                }
                None => true,
            }
        })
        .collect();

    let has_filesystem = components.iter().any(|c| {
        c.base.has_filesystem()
            || c.additions
                .iter()
                .any(|a| !a.readonly_paths.is_empty() || !a.readwrite_paths.is_empty())
    });
    let pick = |items: Vec<PathItem>| {
        Some(items.into_iter().map(|i| i.path).collect::<Vec<_>>()).filter(|v| !v.is_empty())
    };
    let filesystem = has_filesystem.then(|| FilesystemPolicy {
        denied_paths: pick(denied),
        readonly_paths: pick(readonly),
        readwrite_paths: pick(readwrite),
    });

    let root = components[0].base;
    let passthrough = is_passthrough(components);
    let network = if passthrough {
        root.field("network").cloned()
    } else if let Some(only) = verbatim_network(components) {
        only.base.field("network").cloned()
    } else {
        compose_network(components)
    };
    Composed {
        policy: SandboxPolicy {
            version: root.version().to_string(),
            filesystem,
            network,
            ui: passthrough.then(|| root.field("ui").cloned()).flatten(),
            timeout_ms: passthrough
                .then(|| root.field("timeoutMs").and_then(Json::as_f64))
                .flatten(),
        },
        warnings,
    }
}

/// Unions the outbound allow rules under a deny-by-default egress.
fn compose_network(components: &[Component<'_>]) -> Option<Json> {
    let mut seen = HashSet::new();
    let mut allow = Vec::new();
    let mut any = false;
    for component in components {
        if let Some(network) = component.base.field("network") {
            any = true;
            let rules = network
                .get("egress")
                .and_then(|e| e.get("allow"))
                .and_then(Json::as_array);
            for rule in rules.into_iter().flatten() {
                if seen.insert(canonical_json(rule)) {
                    allow.push(rule.clone());
                }
            }
        }
        for additions in &component.additions {
            for rule in &additions.egress_allow {
                any = true;
                if seen.insert(canonical_json(rule)) {
                    allow.push(rule.clone());
                }
            }
        }
    }
    if !any {
        return None;
    }
    let mut egress = JsonObject::new();
    egress.insert("default", "deny".into());
    if !allow.is_empty() {
        egress.insert("allow", Json::Array(allow));
    }
    let mut network = JsonObject::new();
    network.insert("egress", Json::Object(egress));
    Some(Json::Object(network))
}
