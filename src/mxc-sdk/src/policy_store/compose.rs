// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Floor composition of selected layers (design §4.5): every selected base,
//! its selected additions, and its dependencies combine into one
//! `ContainerRequirements` that preserves the access each requested pair
//! needs.

use crate::policy_store::catalog::{Additions, CatalogPolicy};
use crate::policy_store::effective::LayerBody;
use crate::policy_store::host::ObjectRelation;
use crate::policy_store::json::{canonical_json, cmp_utf16, Json, JsonObject};
use crate::policy_store::model::{
    DetailWarningKind, FilesystemRequirements, NetworkRequirement, PathAccess, PathRequirement,
    Platform, Requirements, ResolutionDetailWarning, Warning,
};
use crate::policy_store::netrule::{describe_rule, rules_overlap};
use crate::policy_store::paths::{normalize_path, path_exact_segments, path_key_segments};
use crate::policy_store::text::symbol_matches;

/// One selected layer and the input indexes that require it.
#[derive(Clone, Debug)]
pub struct Contribution<'a> {
    pub entry_id: &'a str,
    pub body: LayerBody<'a>,
    pub owners: Vec<usize>,
}

impl Contribution<'_> {
    fn paths(&self, class: PathAccess) -> Vec<&str> {
        match self.body {
            LayerBody::Base(base) => base.filesystem_field(field(class)),
            LayerBody::Additions(additions) => match class {
                PathAccess::Denied => Vec::new(),
                PathAccess::Readonly => additions
                    .readonly_paths
                    .iter()
                    .map(String::as_str)
                    .collect(),
                PathAccess::Readwrite => additions
                    .readwrite_paths
                    .iter()
                    .map(String::as_str)
                    .collect(),
            },
        }
    }
}

fn field(class: PathAccess) -> &'static str {
    match class {
        PathAccess::Denied => "deniedPaths",
        PathAccess::Readonly => "readonlyPaths",
        PathAccess::Readwrite => "readwritePaths",
    }
}

fn label(class: PathAccess) -> &'static str {
    match class {
        PathAccess::Denied => "denied",
        PathAccess::Readonly => "read-only",
        PathAccess::Readwrite => "read-write",
    }
}

const CLASSES: [PathAccess; 3] = [
    PathAccess::Denied,
    PathAccess::Readonly,
    PathAccess::Readwrite,
];

/// The contributions of one entry.
struct Group<'a> {
    entry_id: &'a str,
    base: Option<&'a CatalogPolicy>,
    additions: Vec<&'a Additions>,
}

impl Group<'_> {
    fn has_additions(&self) -> bool {
        self.additions.iter().any(|a| !a.is_empty())
    }

    fn needs_network(&self) -> bool {
        self.base.is_some_and(|b| b.field("network").is_some())
            || self.additions.iter().any(|a| !a.egress_allow.is_empty())
    }
}

fn groups<'a>(contributions: &[Contribution<'a>]) -> Vec<Group<'a>> {
    let mut out: Vec<Group<'a>> = Vec::new();
    for contribution in contributions {
        let index = match out.iter().position(|g| g.entry_id == contribution.entry_id) {
            Some(index) => index,
            None => {
                out.push(Group {
                    entry_id: contribution.entry_id,
                    base: None,
                    additions: Vec::new(),
                });
                out.len() - 1
            }
        };
        match contribution.body {
            LayerBody::Base(base) => out[index].base = Some(base),
            LayerBody::Additions(additions) => out[index].additions.push(additions),
        }
    }
    out
}

/// The symbols the contributions reference, first-seen order, with the
/// entries and input indexes that need each.
pub fn contribution_symbols(
    contributions: &[Contribution<'_>],
) -> Vec<(String, Vec<String>, Vec<usize>)> {
    let mut out: Vec<(String, Vec<String>, Vec<usize>)> = Vec::new();
    for contribution in contributions {
        for class in CLASSES {
            for template in contribution.paths(class) {
                for (_, _, name) in symbol_matches(template) {
                    let index = match out.iter().position(|(n, _, _)| n == name) {
                        Some(index) => index,
                        None => {
                            out.push((name.to_string(), Vec::new(), Vec::new()));
                            out.len() - 1
                        }
                    };
                    let (_, ids, owners) = &mut out[index];
                    add_id(ids, contribution.entry_id);
                    add_owners(owners, &contribution.owners);
                }
            }
        }
    }
    out
}

fn add_id(ids: &mut Vec<String>, id: &str) {
    if !ids.iter().any(|x| x == id) {
        ids.push(id.to_string());
        ids.sort_by(|a, b| cmp_utf16(a, b));
    }
}

fn add_owners(owners: &mut Vec<usize>, extra: &[usize]) {
    owners.extend_from_slice(extra);
    owners.sort_unstable();
    owners.dedup();
}

/// Rejects combinations the v1 composition rules cannot express (design
/// §4.5), rather than approximating them with broader access.
pub fn compose_check(contributions: &[Contribution<'_>]) -> Result<(), String> {
    let groups = groups(contributions);
    if is_passthrough(&groups) {
        return Ok(());
    }
    for group in &groups {
        for key in group.base.into_iter().flat_map(|b| b.raw().keys()) {
            if !matches!(key, "filesystem" | "network" | "ui" | "timeoutMs") {
                return Err(format!(
                    "'{}' uses '{key}', which has no v1 cross-policy composition rule",
                    group.entry_id
                ));
            }
        }
    }
    // API spec §4: one source's `ui`/`timeoutMs` is kept; two distinct
    // sources supplying the same field conflict, even with equal values.
    for field in SINGLE_SOURCE_FIELDS {
        let sources: Vec<&str> = groups
            .iter()
            .filter(|g| g.base.is_some_and(|b| b.field(field).is_some()))
            .map(|g| g.entry_id)
            .collect();
        if sources.len() > 1 {
            return Err(format!(
                "'{field}' is supplied by {}; distinct sources cannot both supply it",
                sources.join(" and ")
            ));
        }
    }
    if groups.iter().filter(|g| g.needs_network()).count() <= 1 {
        return Ok(());
    }
    for group in &groups {
        let Some(network) = group
            .base
            .and_then(|b| b.field("network"))
            .and_then(Json::as_object)
        else {
            continue;
        };
        for (key, value) in network.iter() {
            if key != "egress" {
                return Err(format!(
                    "'{}' uses 'network.{key}', which cannot be combined with other network requirements",
                    group.entry_id
                ));
            }
            for (field, _) in value.as_object().into_iter().flat_map(|o| o.iter()) {
                if !matches!(field, "default" | "allow" | "deny") {
                    return Err(format!(
                        "'{}' uses 'network.egress.{field}', which cannot be combined with other network requirements",
                        group.entry_id
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Fields kept from at most one deduplicated source, never merged.
const SINGLE_SOURCE_FIELDS: [&str; 2] = ["ui", "timeoutMs"];

/// The one base that supplies `field`, if any (`compose_check` rejects more).
fn single_source<'a>(groups: &[Group<'a>], field: &str) -> Option<&'a Json> {
    groups
        .iter()
        .find_map(|g| g.base.and_then(|b| b.field(field)))
}

/// A single selected policy without additions or dependencies keeps every
/// catalog-supported field without cross-policy composition.
fn is_passthrough(groups: &[Group<'_>]) -> bool {
    groups.len() == 1 && !groups[0].has_additions()
}

/// Filesystem object identity used during composition.
pub trait IdentityOracle {
    /// Whether `inner` names `outer`'s object or an object beneath it.
    fn within(&self, inner: &str, outer: &str) -> ObjectRelation;
}

/// Lexical rules only (symbolic catalog validation and the reviewer view):
/// paths that are not lexically related are treated as distinct objects.
pub struct LexicalIdentity;

impl IdentityOracle for LexicalIdentity {
    fn within(&self, _inner: &str, _outer: &str) -> ObjectRelation {
        ObjectRelation::NotWithin
    }
}

/// An access relationship whose object identity could not be established.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityFailure {
    pub paths: Vec<String>,
    pub owners: Vec<usize>,
    pub entry_ids: Vec<String>,
}

/// The composed requirements, composition diagnostics, and any relation
/// whose identity is unknown. When `identity_failures` is non-empty the
/// caller drops their owners and composes again.
#[derive(Clone, Debug)]
pub struct Composed {
    pub requirements: Requirements,
    pub warnings: Vec<Warning>,
    pub identity_failures: Vec<IdentityFailure>,
}

#[derive(Clone, Debug)]
struct PathItem {
    path: String,
    exact: Vec<String>,
    folded: Vec<String>,
    entry_ids: Vec<String>,
    owners: Vec<usize>,
}

impl PathItem {
    fn requirement(&self, access: PathAccess) -> PathRequirement {
        PathRequirement {
            path: self.path.clone(),
            access,
            entry_ids: self.entry_ids.clone(),
        }
    }
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

fn detail(
    owners: &[&[usize]],
    entry_ids: &[&[String]],
    kind: DetailWarningKind,
    message: String,
) -> Warning {
    let mut input_indexes = Vec::new();
    for o in owners {
        add_owners(&mut input_indexes, o);
    }
    let mut all_ids = Vec::new();
    for list in entry_ids {
        for id in list.iter() {
            add_id(&mut all_ids, id);
        }
    }
    Warning::Detail(ResolutionDetailWarning {
        input_indexes,
        entry_ids: all_ids,
        kind,
        message,
    })
}

fn failure(a: &PathItem, b: &PathItem) -> IdentityFailure {
    let mut owners = a.owners.clone();
    add_owners(&mut owners, &b.owners);
    let mut entry_ids = a.entry_ids.clone();
    for id in &b.entry_ids {
        add_id(&mut entry_ids, id);
    }
    IdentityFailure {
        paths: vec![a.path.clone(), b.path.clone()],
        owners,
        entry_ids,
    }
}

/// Composes contributions that passed [`compose_check`]. `resolve` maps a
/// path template to its substituted value (identity for symbolic validation).
pub fn compose(
    contributions: &[Contribution<'_>],
    resolve: &dyn Fn(&str) -> String,
    platform: Platform,
    identity: &dyn IdentityOracle,
) -> Composed {
    let mut warnings = Vec::new();
    let mut failures = Vec::new();
    let mut classes: Vec<Vec<PathItem>> = Vec::new();
    for class in CLASSES {
        let mut items: Vec<PathItem> = Vec::new();
        for contribution in contributions {
            for template in contribution.paths(class) {
                let path = normalize_path(&resolve(template), platform);
                let exact = path_exact_segments(&path, platform);
                let index = match items.iter().position(|i| i.exact == exact) {
                    Some(index) => index,
                    None => {
                        items.push(PathItem {
                            folded: path_key_segments(&path, Platform::Windows),
                            path,
                            exact,
                            entry_ids: Vec::new(),
                            owners: Vec::new(),
                        });
                        items.len() - 1
                    }
                };
                add_id(&mut items[index].entry_ids, contribution.entry_id);
                add_owners(&mut items[index].owners, &contribution.owners);
            }
        }
        classes.push(items);
    }

    // Case sensitivity of the target filesystem is not determined, so paths
    // compare case-sensitively; report pairs that differ only by case.
    let all: Vec<(PathAccess, &PathItem)> = CLASSES
        .iter()
        .zip(&classes)
        .flat_map(|(class, items)| items.iter().map(move |i| (*class, i)))
        .collect();
    for (i, (left_class, left)) in all.iter().enumerate() {
        for (right_class, right) in &all[i + 1..] {
            if is_same_or_nested(&left.folded, &right.folded)
                && !is_same_or_nested(&left.exact, &right.exact)
            {
                warnings.push(detail(
                    &[&left.owners, &right.owners],
                    &[&left.entry_ids, &right.entry_ids],
                    DetailWarningKind::FilesystemCaseAssumed {
                        paths: vec![left.path.clone(), right.path.clone()],
                    },
                    format!(
                        "{} '{}' ({}) and {} '{}' ({}) differ only by case; filesystem case sensitivity was not determined, so they were compared case-sensitively and kept distinct",
                        label(*left_class),
                        left.path,
                        ids(left),
                        label(*right_class),
                        right.path,
                        ids(right)
                    ),
                ));
            }
        }
    }

    let [denied, readonly, mut readwrite] = <[Vec<PathItem>; 3]>::try_from(classes)
        .unwrap_or_else(|_| unreachable!("three access classes"));

    // Read-only requirements equal to or within a read-write subtree are
    // satisfied by it. A read-only alias of a read-write object keeps its
    // pathname with read-write access.
    let mut kept_readonly = Vec::new();
    let mut promoted = Vec::new();
    for ro in readonly {
        let covering: Vec<&PathItem> = readwrite
            .iter()
            .filter(|rw| is_within(&ro.exact, &rw.exact))
            .collect();
        if !covering.is_empty() {
            let owners: Vec<&[usize]> = std::iter::once(ro.owners.as_slice())
                .chain(covering.iter().map(|rw| rw.owners.as_slice()))
                .collect();
            let entry_ids: Vec<&[String]> = std::iter::once(ro.entry_ids.as_slice())
                .chain(covering.iter().map(|rw| rw.entry_ids.as_slice()))
                .collect();
            warnings.push(detail(
                &owners,
                &entry_ids,
                DetailWarningKind::ReadonlySuperseded {
                    removed: ro.requirement(PathAccess::Readonly),
                    required_by: covering
                        .iter()
                        .map(|rw| rw.requirement(PathAccess::Readwrite))
                        .collect(),
                },
                format!(
                    "read-only '{}' ({}) is covered by read-write '{}' ({}); the read-only entry was omitted",
                    ro.path,
                    ids(&ro),
                    covering[0].path,
                    ids(covering[0])
                ),
            ));
            continue;
        }
        let mut aliases: Vec<&PathItem> = Vec::new();
        for rw in &readwrite {
            match identity.within(&ro.path, &rw.path) {
                ObjectRelation::Within => aliases.push(rw),
                ObjectRelation::Unknown => failures.push(failure(&ro, rw)),
                ObjectRelation::NotWithin => {}
            }
        }
        if aliases.is_empty() {
            kept_readonly.push(ro);
            continue;
        }
        let owners: Vec<&[usize]> = std::iter::once(ro.owners.as_slice())
            .chain(aliases.iter().map(|rw| rw.owners.as_slice()))
            .collect();
        let entry_ids: Vec<&[String]> = std::iter::once(ro.entry_ids.as_slice())
            .chain(aliases.iter().map(|rw| rw.entry_ids.as_slice()))
            .collect();
        warnings.push(detail(
            &owners,
            &entry_ids,
            DetailWarningKind::ReadonlySuperseded {
                removed: ro.requirement(PathAccess::Readonly),
                required_by: aliases
                    .iter()
                    .map(|rw| rw.requirement(PathAccess::Readwrite))
                    .collect(),
            },
            format!(
                "read-only '{}' ({}) names the same filesystem object as read-write '{}' ({}) or one beneath it; the alias pathname was retained with read-write access",
                ro.path,
                ids(&ro),
                aliases[0].path,
                ids(aliases[0])
            ),
        ));
        promoted.push(ro);
    }
    readwrite.extend(promoted);
    let readonly = kept_readonly;

    // A catalog deny overlapping any required read-only or read-write path is
    // removed in full.
    let mut kept_denied = Vec::new();
    for deny in denied {
        let grants: Vec<(PathAccess, &PathItem)> = readwrite
            .iter()
            .map(|g| (PathAccess::Readwrite, g))
            .chain(readonly.iter().map(|g| (PathAccess::Readonly, g)))
            .collect();
        let mut overlapping: Vec<(PathAccess, &PathItem)> = grants
            .iter()
            .filter(|(_, g)| is_same_or_nested(&deny.exact, &g.exact))
            .copied()
            .collect();
        if overlapping.is_empty() {
            for (access, grant) in &grants {
                let forward = identity.within(&deny.path, &grant.path);
                let backward = identity.within(&grant.path, &deny.path);
                if forward == ObjectRelation::Within || backward == ObjectRelation::Within {
                    overlapping.push((*access, grant));
                } else if forward == ObjectRelation::Unknown || backward == ObjectRelation::Unknown
                {
                    failures.push(failure(&deny, grant));
                }
            }
        }
        if overlapping.is_empty() {
            kept_denied.push(deny);
            continue;
        }
        let owners: Vec<&[usize]> = std::iter::once(deny.owners.as_slice())
            .chain(overlapping.iter().map(|(_, g)| g.owners.as_slice()))
            .collect();
        let entry_ids: Vec<&[String]> = std::iter::once(deny.entry_ids.as_slice())
            .chain(overlapping.iter().map(|(_, g)| g.entry_ids.as_slice()))
            .collect();
        let (first_access, first) = overlapping[0];
        warnings.push(detail(
            &owners,
            &entry_ids,
            DetailWarningKind::FilesystemDenyRemoved {
                removed: deny.requirement(PathAccess::Denied),
                required_by: overlapping
                    .iter()
                    .map(|(access, g)| g.requirement(*access))
                    .collect(),
            },
            format!(
                "removed catalog deny '{}' ({}) because it overlaps required {} '{}' ({}); the entire deny scope '{}' was removed, so other grants may now apply throughout it",
                deny.path,
                ids(&deny),
                label(first_access),
                first.path,
                ids(first),
                deny.path
            ),
        ));
    }

    let has_filesystem = contributions.iter().any(|c| match c.body {
        LayerBody::Base(base) => base.has_filesystem(),
        LayerBody::Additions(a) => !a.readonly_paths.is_empty() || !a.readwrite_paths.is_empty(),
    });
    let pick = |items: Vec<PathItem>| {
        Some(items.into_iter().map(|i| i.path).collect::<Vec<_>>()).filter(|v| !v.is_empty())
    };
    let filesystem = has_filesystem.then(|| FilesystemRequirements {
        denied_paths: pick(kept_denied),
        readonly_paths: pick(readonly),
        readwrite_paths: pick(readwrite),
    });

    let groups = groups(contributions);
    let network = compose_network(contributions, &groups, &mut warnings);
    Composed {
        requirements: Requirements {
            filesystem,
            network,
            ui: single_source(&groups, "ui").cloned(),
            timeout_ms: single_source(&groups, "timeoutMs")
                .and_then(Json::as_f64)
                .map(|n| n as u32),
        },
        warnings,
        identity_failures: failures,
    }
}

/// Outbound rules keyed by canonical JSON, with their contributors.
#[derive(Default)]
struct Rules {
    items: Vec<RuleItem>,
}

struct RuleItem {
    key: String,
    rule: Json,
    entry_ids: Vec<String>,
    owners: Vec<usize>,
}

impl RuleItem {
    fn requirement(&self) -> NetworkRequirement {
        NetworkRequirement {
            rule: self.rule.clone(),
            entry_ids: self.entry_ids.clone(),
        }
    }
}

impl Rules {
    fn add(&mut self, rule: &Json, contribution: &Contribution<'_>) {
        let key = canonical_json(rule);
        let index = match self.items.iter().position(|i| i.key == key) {
            Some(index) => index,
            None => {
                self.items.push(RuleItem {
                    key,
                    rule: rule.clone(),
                    entry_ids: Vec::new(),
                    owners: Vec::new(),
                });
                self.items.len() - 1
            }
        };
        add_id(&mut self.items[index].entry_ids, contribution.entry_id);
        add_owners(&mut self.items[index].owners, &contribution.owners);
    }

    fn json(&self) -> Option<Json> {
        (!self.items.is_empty())
            .then(|| Json::Array(self.items.iter().map(|i| i.rule.clone()).collect()))
    }
}

fn egress_rules<'j>(network: Option<&'j Json>, field: &str) -> impl Iterator<Item = &'j Json> {
    network
        .and_then(|n| n.get("egress"))
        .and_then(|e| e.get(field))
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
}

/// Composes network requirements (design §4.5). A single entry needing
/// network keeps its own network section, plus its outbound additions.
/// Several such entries union their outbound allow rules and catalog egress
/// denies under a deny-by-default egress. Either way, a catalog deny
/// overlapping any required allow rule is removed in full, with a warning.
fn compose_network(
    contributions: &[Contribution<'_>],
    groups: &[Group<'_>],
    warnings: &mut Vec<Warning>,
) -> Option<Json> {
    let needing: Vec<&Group<'_>> = groups.iter().filter(|g| g.needs_network()).collect();
    if needing.is_empty() {
        return None;
    }
    let mut allow = Rules::default();
    let mut deny = Rules::default();
    for contribution in contributions {
        match contribution.body {
            LayerBody::Base(base) => {
                let network = base.field("network");
                for rule in egress_rules(network, "allow") {
                    allow.add(rule, contribution);
                }
                for rule in egress_rules(network, "deny") {
                    deny.add(rule, contribution);
                }
            }
            LayerBody::Additions(additions) => {
                for rule in &additions.egress_allow {
                    allow.add(rule, contribution);
                }
            }
        }
    }
    deny.items.retain(|denied| {
        let required: Vec<&RuleItem> = allow
            .items
            .iter()
            .filter(|a| rules_overlap(&a.rule, &denied.rule))
            .collect();
        if required.is_empty() {
            return true;
        }
        let owners: Vec<&[usize]> = std::iter::once(denied.owners.as_slice())
            .chain(required.iter().map(|a| a.owners.as_slice()))
            .collect();
        let entry_ids: Vec<&[String]> = std::iter::once(denied.entry_ids.as_slice())
            .chain(required.iter().map(|a| a.entry_ids.as_slice()))
            .collect();
        warnings.push(detail(
            &owners,
            &entry_ids,
            DetailWarningKind::NetworkDenyRemoved {
                removed: denied.requirement(),
                required_by: required.iter().map(|a| a.requirement()).collect(),
            },
            format!(
                "removed catalog egress deny {} ({}) because it overlaps required egress allow {} ({}); the entire deny rule was removed, so other grants may now apply throughout {}",
                describe_rule(&denied.rule),
                denied.entry_ids.join(", "),
                describe_rule(&required[0].rule),
                required[0].entry_ids.join(", "),
                describe_rule(&denied.rule)
            ),
        ));
        false
    });

    let single_base = match needing.as_slice() {
        [only] => only
            .base
            .and_then(|b| b.field("network"))
            .and_then(Json::as_object),
        _ => None,
    };
    let mut egress = JsonObject::new();
    match single_base.and_then(|b| b.get("egress")) {
        Some(base_egress) => {
            if let Some(default) = base_egress.get("default") {
                egress.insert("default", default.clone());
            }
        }
        None if single_base.is_some() => {}
        None => egress.insert("default", "deny".into()),
    }
    if let Some(rules) = allow.json() {
        egress.insert("allow", rules);
    }
    if let Some(rules) = deny.json() {
        egress.insert("deny", rules);
    }
    let mut network = JsonObject::new();
    if !egress.is_empty() || single_base.is_some_and(|b| b.contains_key("egress")) {
        network.insert("egress", Json::Object(egress));
    }
    for (key, value) in single_base.into_iter().flat_map(|b| b.iter()) {
        if key != "egress" {
            network.insert(key, value.clone());
        }
    }
    Some(Json::Object(network))
}
