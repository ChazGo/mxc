// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Contract and revision validation, variant selection, dependency closure,
//! and composition limits. Error messages match the original TypeScript
//! prototype byte for byte, so the frozen conformance vectors still apply.

use crate::errors::{invalid_catalog, Result};
use crate::json::{cmp_utf16, js_number_to_string, js_to_string, Json, JsonObject};
use crate::model::{Architecture, IdentityStrength, Platform};
use crate::paths::path_key_segments;
use crate::purl::parse_purl;
use crate::text::{is_symbol_name, js_to_lower, replace_symbols, symbol_matches};
use crate::version_range::is_valid_version_range;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

pub const CATALOG_SCHEMA_VERSION: &str = "1";

/// Fields the v1 contract can compose across entries (design §4.5), in order.
pub const COMPOSABLE_FILESYSTEM_FIELDS: [&str; 3] =
    ["deniedPaths", "readonlyPaths", "readwritePaths"];

const BACKEND_KEYS: [&str; 8] = [
    "containment",
    "processContainer",
    "appContainer",
    "lxc",
    "seatbelt",
    "wslc",
    "hyperlight",
    "bwrap",
];

/// Where a symbol's value comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolSource {
    Context,
    Caller,
    Host,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolDefinition {
    pub source: SymbolSource,
    pub description: String,
}

/// Validated catalog contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogContract {
    pub catalog_schema_version: String,
    pub sandbox_policy_versions: Vec<String>,
    symbols: Vec<(String, SymbolDefinition)>,
}

impl CatalogContract {
    /// Own-key symbol lookup.
    pub fn symbol(&self, name: &str) -> Option<&SymbolDefinition> {
        self.symbols.iter().find(|(n, _)| n == name).map(|(_, d)| d)
    }

    pub fn symbols(&self) -> impl Iterator<Item = (&str, &SymbolDefinition)> {
        self.symbols.iter().map(|(n, d)| (n.as_str(), d))
    }
}

/// One identity predicate of an entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityPredicate {
    Purl {
        value: String,
        version_range: Option<String>,
    },
    InvocationName {
        names: Vec<String>,
    },
}

impl IdentityPredicate {
    pub fn kind(&self) -> &'static str {
        match self {
            IdentityPredicate::Purl { .. } => "purl",
            IdentityPredicate::InvocationName { .. } => "invocation-name",
        }
    }

    pub fn strength(&self) -> IdentityStrength {
        match self {
            IdentityPredicate::Purl { .. } => IdentityStrength::Strong,
            IdentityPredicate::InvocationName { .. } => IdentityStrength::Weak,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dependency {
    pub entry_id: String,
    pub version_range: Option<String>,
}

/// An embedded, validated `SandboxPolicy`. The reviewed JSON object is kept
/// as written so field order and values are preserved exactly.
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogPolicy {
    raw: JsonObject,
}

impl CatalogPolicy {
    pub fn raw(&self) -> &JsonObject {
        &self.raw
    }

    pub fn version(&self) -> &str {
        self.raw
            .get("version")
            .and_then(Json::as_str)
            .unwrap_or_default()
    }

    pub fn has_filesystem(&self) -> bool {
        self.raw.contains_key("filesystem")
    }

    /// The templates of one filesystem access class (empty when absent).
    pub fn filesystem_field(&self, field: &str) -> Vec<&str> {
        self.raw
            .get("filesystem")
            .and_then(|fs| fs.get(field))
            .and_then(Json::as_array)
            .map(|items| items.iter().filter_map(Json::as_str).collect())
            .unwrap_or_default()
    }

    pub fn field(&self, key: &str) -> Option<&Json> {
        self.raw.get(key)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlatformVariant {
    pub platform: Platform,
    pub architecture: Option<Architecture>,
    pub dependencies: Option<Vec<Dependency>>,
    pub sandbox_policy: CatalogPolicy,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CatalogEntry {
    pub entry_id: String,
    pub entry_revision: f64,
    pub display_name: String,
    pub identity: Vec<IdentityPredicate>,
    pub platform_variants: Vec<PlatformVariant>,
    pub provenance_method: String,
    pub provenance_source_revision: String,
    /// The validated entry as JSON (history comparisons use its canonical form).
    pub(crate) raw: Json,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CatalogRevision {
    pub catalog_schema_version: String,
    pub catalog_revision: String,
    pub entries: Vec<CatalogEntry>,
}

fn fail<T>(message: impl AsRef<str>) -> Result<T> {
    Err(invalid_catalog(message))
}

fn record(value: Option<&Json>) -> Option<&JsonObject> {
    value.and_then(Json::as_object)
}

fn only_fields(value: &JsonObject, allowed: &[&str], at: &str) -> Result<()> {
    for key in value.keys() {
        if !allowed.contains(&key) {
            return fail(format!("unsupported field '{at}.{key}'"));
        }
    }
    Ok(())
}

fn non_empty_string(value: Option<&Json>, at: &str) -> Result<String> {
    match value.and_then(Json::as_str) {
        Some(s) if !s.is_empty() => Ok(s.to_string()),
        _ => fail(format!("'{at}' must be a non-empty string")),
    }
}

fn string_array(value: Option<&Json>, at: &str, min_items: usize) -> Result<Vec<String>> {
    let items = match value.and_then(Json::as_array) {
        Some(items) if items.len() >= min_items => items,
        _ => {
            return fail(format!(
                "'{at}' must be an array with at least {min_items} item(s)"
            ))
        }
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| non_empty_string(Some(item), &format!("{at}[{index}]")))
        .collect()
}

fn is_integer(value: f64) -> bool {
    value.is_finite() && value.fract() == 0.0
}

/// `/^(\d{4}-\d{2}-\d{2})\.([1-9]\d*)$/` → (date, sequence).
fn parse_revision_id(value: &str) -> Option<(&str, &str)> {
    let b = value.as_bytes();
    if b.len() < 12 {
        return None;
    }
    let digit = |i: usize| b[i].is_ascii_digit();
    let date_ok = (0..4).all(digit)
        && b[4] == b'-'
        && (5..7).all(digit)
        && b[7] == b'-'
        && (8..10).all(digit);
    if !date_ok
        || b[10] != b'.'
        || !(b'1'..=b'9').contains(&b[11])
        || !b[12..].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    Some((&value[..10], &value[11..]))
}

/// Compares two catalog revision identifiers (`YYYY-MM-DD.N`).
pub fn compare_catalog_revisions(left: &str, right: &str) -> Result<Ordering> {
    match (parse_revision_id(left), parse_revision_id(right)) {
        (Some((ld, ln)), Some((rd, rn))) => {
            if ld != rd {
                return Ok(ld.cmp(rd));
            }
            let (l, r): (f64, f64) = (
                ln.parse().unwrap_or(f64::NAN),
                rn.parse().unwrap_or(f64::NAN),
            );
            Ok(l.partial_cmp(&r).unwrap_or(Ordering::Equal))
        }
        _ => fail(format!(
            "cannot compare malformed catalog revisions '{left}' and '{right}'"
        )),
    }
}

pub fn is_catalog_revision_id(value: &str) -> bool {
    parse_revision_id(value).is_some()
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

pub fn validate_contract(raw: &Json) -> Result<CatalogContract> {
    let Some(raw) = raw.as_object() else {
        return fail("contract root must be an object");
    };
    only_fields(
        raw,
        &[
            "$comment",
            "catalogSchemaVersion",
            "sandboxPolicyVersions",
            "symbols",
        ],
        "contract",
    )?;
    if raw.get("catalogSchemaVersion").and_then(Json::as_str) != Some(CATALOG_SCHEMA_VERSION) {
        return fail(format!(
            "contract.catalogSchemaVersion must be '{CATALOG_SCHEMA_VERSION}'"
        ));
    }
    let sandbox_policy_versions = string_array(
        raw.get("sandboxPolicyVersions"),
        "contract.sandboxPolicyVersions",
        1,
    )?;
    let Some(raw_symbols) = record(raw.get("symbols")) else {
        return fail("contract.symbols must be an object");
    };
    let mut symbols = Vec::new();
    for (name, definition) in raw_symbols.iter() {
        let definition = match definition.as_object() {
            Some(d) if is_symbol_name(name) => d,
            _ => return fail(format!("contract.symbols.{name} is malformed")),
        };
        only_fields(
            definition,
            &["source", "description"],
            &format!("contract.symbols.{name}"),
        )?;
        let source = match definition.get("source").and_then(Json::as_str) {
            Some("context") => SymbolSource::Context,
            Some("caller") => SymbolSource::Caller,
            Some("host") => SymbolSource::Host,
            _ => return fail(format!("contract.symbols.{name}.source is unsupported")),
        };
        let description = non_empty_string(
            definition.get("description"),
            &format!("contract.symbols.{name}.description"),
        )?;
        symbols.push((
            name.to_string(),
            SymbolDefinition {
                source,
                description,
            },
        ));
    }
    Ok(CatalogContract {
        catalog_schema_version: CATALOG_SCHEMA_VERSION.to_string(),
        sandbox_policy_versions,
        symbols,
    })
}

// ---------------------------------------------------------------------------
// Paths and symbols
// ---------------------------------------------------------------------------

fn validate_template_path(value: &str, at: &str, contract: &CatalogContract) -> Result<()> {
    if value.contains(['*', '?']) {
        return fail(format!("'{at}' contains a wildcard"));
    }
    if replace_symbols(value, |_| String::new()).contains("${") {
        return fail(format!("'{at}' contains malformed symbol syntax"));
    }
    let matches = symbol_matches(value);
    for (_, _, name) in &matches {
        if contract.symbol(name).is_none() {
            return fail(format!("'{at}' references unknown symbol '{name}'"));
        }
    }
    let anchored = matches.first().is_some_and(|(start, end, _)| {
        *start == 0 && (value.len() == *end || value[*end..].starts_with(['/', '\\']))
    });
    if !anchored {
        return fail(format!(
            "'{at}' must start with a declared symbol; literal paths are not allowed"
        ));
    }
    if value.split(['/', '\\']).any(|segment| segment == "..") {
        return fail(format!("'{at}' must not contain '..' segments"));
    }
    Ok(())
}

/// The symbols a policy references, in first-seen order.
pub fn policy_symbols(policy: &CatalogPolicy) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for field in COMPOSABLE_FILESYSTEM_FIELDS {
        for value in policy.filesystem_field(field) {
            for (_, _, name) in symbol_matches(value) {
                if !seen.iter().any(|s| s == name) {
                    seen.push(name.to_string());
                }
            }
        }
    }
    seen
}

fn is_same_or_nested(left: &[String], right: &[String]) -> bool {
    let (shorter, longer) = if left.len() <= right.len() {
        (left, right)
    } else {
        (right, left)
    };
    shorter.iter().zip(longer).all(|(a, b)| a == b)
}

/// Finds the first equal or ancestor/descendant pair of paths in different
/// access classes. `classes` is in [`COMPOSABLE_FILESYSTEM_FIELDS`] order.
pub fn find_cross_class_overlap(
    classes: &[(&str, Vec<String>)],
    platform: Platform,
) -> Option<String> {
    let fields: Vec<&(&str, Vec<String>)> = classes
        .iter()
        .filter(|(_, values)| !values.is_empty())
        .collect();
    for i in 0..fields.len() {
        for j in i + 1..fields.len() {
            for left in &fields[i].1 {
                for right in &fields[j].1 {
                    if is_same_or_nested(
                        &path_key_segments(left, platform),
                        &path_key_segments(right, platform),
                    ) {
                        return Some(format!(
                            "'{left}' ({}) overlaps '{right}' ({})",
                            fields[i].0, fields[j].0
                        ));
                    }
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Embedded SandboxPolicy
// ---------------------------------------------------------------------------

fn validate_network_rules(value: &Json, at: &str, deny_list: bool) -> Result<()> {
    let Some(rules) = value.as_array() else {
        return fail(format!("'{at}' must be an array"));
    };
    for (index, rule) in rules.iter().enumerate() {
        let rule_at = format!("{at}[{index}]");
        let Some(rule) = rule.as_object() else {
            return fail(format!("'{rule_at}' must be an object"));
        };
        only_fields(rule, &["to", "ports"], &rule_at)?;
        if !rule.contains_key("to") && !deny_list {
            return fail(format!(
                "'{rule_at}' has no 'to'; wildcard network grants are not allowed"
            ));
        }
        if let Some(to) = rule.get("to") {
            let peers = match to.as_array() {
                Some(peers) if !peers.is_empty() => peers,
                _ => return fail(format!("'{rule_at}.to' must be a non-empty array")),
            };
            for (peer_index, peer) in peers.iter().enumerate() {
                let peer_at = format!("{rule_at}.to[{peer_index}]");
                let Some(peer) = peer.as_object() else {
                    return fail(format!("'{peer_at}' must be an object"));
                };
                only_fields(peer, &["cidr", "except"], &peer_at)?;
                let cidr = non_empty_string(peer.get("cidr"), &format!("{peer_at}.cidr"))?;
                if !deny_list && cidr.ends_with("/0") {
                    return fail(format!("'{peer_at}.cidr' is a wildcard network grant"));
                }
                if let Some(except) = peer.get("except") {
                    string_array(Some(except), &format!("{peer_at}.except"), 0)?;
                }
            }
        }
        if let Some(ports) = rule.get("ports") {
            let ports = match ports.as_array() {
                Some(ports) if !ports.is_empty() => ports,
                _ => return fail(format!("'{rule_at}.ports' must be a non-empty array")),
            };
            for (port_index, port) in ports.iter().enumerate() {
                let port_at = format!("{rule_at}.ports[{port_index}]");
                let Some(port) = port.as_object() else {
                    return fail(format!("'{port_at}' must be an object"));
                };
                only_fields(port, &["protocol", "port", "endPort"], &port_at)?;
                if let Some(protocol) = port.get("protocol") {
                    if !matches!(protocol.as_str(), Some("tcp" | "udp" | "icmp" | "any")) {
                        return fail(format!("'{port_at}.protocol' is unsupported"));
                    }
                }
                for key in ["port", "endPort"] {
                    if let Some(n) = port.get(key) {
                        let ok = n
                            .as_f64()
                            .is_some_and(|n| is_integer(n) && (1.0..=65535.0).contains(&n));
                        if !ok {
                            return fail(format!(
                                "'{port_at}.{key}' must be an integer in 1..65535"
                            ));
                        }
                    }
                }
                if let Some(end) = port.get("endPort").and_then(Json::as_f64) {
                    let bad = match port.get("port").and_then(Json::as_f64) {
                        None => true,
                        Some(start) => end < start,
                    };
                    if bad {
                        return fail(format!(
                            "'{port_at}.endPort' requires a lower or equal 'port'"
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_sandbox_policy(
    raw: Option<&Json>,
    at: &str,
    contract: &CatalogContract,
) -> Result<CatalogPolicy> {
    let Some(raw) = record(raw) else {
        return fail(format!("'{at}' must be an object"));
    };
    for key in BACKEND_KEYS {
        if raw.contains_key(key) {
            return fail(format!(
                "'{at}.{key}' names a containment backend; platform variants must stay backend-neutral"
            ));
        }
    }
    only_fields(
        raw,
        &["version", "filesystem", "network", "ui", "timeoutMs"],
        at,
    )?;
    let version = non_empty_string(raw.get("version"), &format!("{at}.version"))?;
    if !contract.sandbox_policy_versions.contains(&version) {
        return fail(format!(
            "'{at}.version' '{version}' is not a SandboxPolicy version registered in the catalog contract"
        ));
    }
    if let Some(filesystem) = raw.get("filesystem") {
        let Some(filesystem) = filesystem.as_object() else {
            return fail(format!("'{at}.filesystem' must be an object"));
        };
        only_fields(
            filesystem,
            &COMPOSABLE_FILESYSTEM_FIELDS,
            &format!("{at}.filesystem"),
        )?;
        for field in COMPOSABLE_FILESYSTEM_FIELDS {
            if let Some(values) = filesystem.get(field) {
                let field_at = format!("{at}.filesystem.{field}");
                for (index, value) in string_array(Some(values), &field_at, 0)?.iter().enumerate() {
                    validate_template_path(value, &format!("{field_at}[{index}]"), contract)?;
                }
            }
        }
    }
    if let Some(network) = raw.get("network") {
        let Some(network) = network.as_object() else {
            return fail(format!("'{at}.network' must be an object"));
        };
        only_fields(network, &["egress", "ingress"], &format!("{at}.network"))?;
        if let Some(egress) = network.get("egress") {
            let Some(egress) = egress.as_object() else {
                return fail(format!("'{at}.network.egress' must be an object"));
            };
            only_fields(
                egress,
                &["default", "allow", "deny"],
                &format!("{at}.network.egress"),
            )?;
            if let Some(default) = egress.get("default") {
                if default.as_str() != Some("deny") {
                    return fail(format!(
                        "'{at}.network.egress.default' must be 'deny'; a default-allow grant is a wildcard"
                    ));
                }
            }
            if let Some(allow) = egress.get("allow") {
                validate_network_rules(allow, &format!("{at}.network.egress.allow"), false)?;
            }
            if let Some(deny) = egress.get("deny") {
                validate_network_rules(deny, &format!("{at}.network.egress.deny"), true)?;
            }
        }
        if let Some(ingress) = network.get("ingress") {
            let Some(ingress) = ingress.as_object() else {
                return fail(format!("'{at}.network.ingress' must be an object"));
            };
            only_fields(
                ingress,
                &["default", "hostLoopback"],
                &format!("{at}.network.ingress"),
            )?;
            if let Some(default) = ingress.get("default") {
                if default.as_str() != Some("deny") {
                    return fail(format!(
                        "'{at}.network.ingress.default' must be 'deny'; a default-allow grant is a wildcard"
                    ));
                }
            }
            if let Some(loopback) = ingress.get("hostLoopback") {
                if !matches!(loopback.as_str(), Some("allow" | "deny")) {
                    return fail(format!(
                        "'{at}.network.ingress.hostLoopback' is unsupported"
                    ));
                }
            }
        }
    }
    if let Some(ui) = raw.get("ui") {
        let Some(ui) = ui.as_object() else {
            return fail(format!("'{at}.ui' must be an object"));
        };
        only_fields(
            ui,
            &["allowWindows", "clipboard", "allowInputInjection"],
            &format!("{at}.ui"),
        )?;
        for key in ["allowWindows", "allowInputInjection"] {
            if let Some(value) = ui.get(key) {
                if value.as_bool().is_none() {
                    return fail(format!("'{at}.ui.{key}' must be a boolean"));
                }
            }
        }
        if let Some(clipboard) = ui.get("clipboard") {
            if !matches!(clipboard.as_str(), Some("none" | "read" | "write" | "all")) {
                return fail(format!("'{at}.ui.clipboard' is unsupported"));
            }
        }
    }
    if let Some(timeout) = raw.get("timeoutMs") {
        if !timeout.as_f64().is_some_and(|n| is_integer(n) && n >= 1.0) {
            return fail(format!("'{at}.timeoutMs' must be a positive integer"));
        }
    }
    Ok(CatalogPolicy { raw: raw.clone() })
}

// ---------------------------------------------------------------------------
// Entries
// ---------------------------------------------------------------------------

/// Stable comparison keys for one predicate; invocation names always fold.
fn identity_keys(predicate: &IdentityPredicate) -> Vec<String> {
    match predicate {
        IdentityPredicate::Purl { value, .. } => {
            vec![format!(
                "purl:{}",
                parse_purl(value).map(|p| p.key).unwrap_or_default()
            )]
        }
        IdentityPredicate::InvocationName { names } => names
            .iter()
            .map(|name| format!("invocation-name:{}", js_to_lower(name)))
            .collect(),
    }
}

fn validate_identity(raw: Option<&Json>, at: &str) -> Result<Vec<IdentityPredicate>> {
    let items = match raw.and_then(Json::as_array) {
        Some(items) if !items.is_empty() => items,
        _ => return fail(format!("'{at}' must be a non-empty array")),
    };
    let mut predicates = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let item_at = format!("{at}[{index}]");
        let Some(item) = item.as_object() else {
            return fail(format!("'{item_at}' must be an object"));
        };
        match item.get("kind").and_then(Json::as_str) {
            Some("purl") => {
                only_fields(item, &["kind", "value", "versionRange"], &item_at)?;
                let value = non_empty_string(item.get("value"), &format!("{item_at}.value"))?;
                let Some(parsed) = parse_purl(&value) else {
                    return fail(format!("'{item_at}.value' is not a valid package URL"));
                };
                if parsed.version.is_some() {
                    return fail(format!(
                        "'{item_at}.value' must not pin a version; use 'versionRange'"
                    ));
                }
                let mut version_range = None;
                if let Some(range) = item.get("versionRange") {
                    let range = non_empty_string(Some(range), &format!("{item_at}.versionRange"))?;
                    if !is_valid_version_range(&range) {
                        return fail(format!(
                            "'{item_at}.versionRange' is not a valid version range"
                        ));
                    }
                    version_range = Some(range);
                }
                predicates.push(IdentityPredicate::Purl {
                    value,
                    version_range,
                });
            }
            Some("invocation-name") => {
                only_fields(item, &["kind", "names"], &item_at)?;
                let names = string_array(item.get("names"), &format!("{item_at}.names"), 1)?;
                for name in &names {
                    if name.contains(['/', '\\']) {
                        return fail(format!(
                            "'{item_at}.names' entry '{name}' must be a bare invocation name, not a path"
                        ));
                    }
                }
                predicates.push(IdentityPredicate::InvocationName { names });
            }
            _ => return fail(format!("'{item_at}.kind' is not a supported identity kind")),
        }
    }
    let mut seen = HashSet::new();
    for key in predicates.iter().flat_map(identity_keys) {
        if !seen.insert(key.clone()) {
            return fail(format!("'{at}' repeats identity '{key}'"));
        }
    }
    Ok(predicates)
}

fn validate_variants(
    raw: Option<&Json>,
    at: &str,
    contract: &CatalogContract,
) -> Result<Vec<PlatformVariant>> {
    let items = match raw.and_then(Json::as_array) {
        Some(items) if !items.is_empty() => items,
        _ => return fail(format!("'{at}' must be a non-empty array")),
    };
    let mut selectors = HashSet::new();
    let mut variants = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let item_at = format!("{at}[{index}]");
        let Some(item) = item.as_object() else {
            return fail(format!("'{item_at}' must be an object"));
        };
        only_fields(item, &["when", "dependencies", "sandboxPolicy"], &item_at)?;
        let Some(when) = record(item.get("when")) else {
            return fail(format!("'{item_at}.when' must be an object"));
        };
        only_fields(
            when,
            &["platform", "architecture"],
            &format!("{item_at}.when"),
        )?;
        let Some(platform) = when
            .get("platform")
            .and_then(Json::as_str)
            .and_then(Platform::parse)
        else {
            return fail(format!(
                "'{item_at}.when.platform' must be one of windows, linux, macos"
            ));
        };
        let architecture = match when.get("architecture") {
            None => None,
            Some(value) => match value.as_str().and_then(Architecture::parse) {
                Some(arch) => Some(arch),
                None => {
                    return fail(format!(
                        "'{item_at}.when.architecture' must be one of x64, arm64"
                    ))
                }
            },
        };
        let selector = format!(
            "{platform}/{}",
            architecture.map_or("*", Architecture::as_str)
        );
        if !selectors.insert(selector.clone()) {
            return fail(if architecture.is_none() {
                format!("'{item_at}' is a second architecture-neutral variant for '{platform}'")
            } else {
                format!("'{item_at}' duplicates selector '{selector}'")
            });
        }
        let mut dependencies = None;
        if let Some(raw_dependencies) = item.get("dependencies") {
            let Some(raw_dependencies) = raw_dependencies.as_array() else {
                return fail(format!("'{item_at}.dependencies' must be an array"));
            };
            let mut seen = HashSet::new();
            let mut list = Vec::new();
            for (dep_index, dependency) in raw_dependencies.iter().enumerate() {
                let dep_at = format!("{item_at}.dependencies[{dep_index}]");
                let Some(dependency) = dependency.as_object() else {
                    return fail(format!("'{dep_at}' must be an object"));
                };
                only_fields(dependency, &["entryId", "versionRange"], &dep_at)?;
                let entry_id =
                    non_empty_string(dependency.get("entryId"), &format!("{dep_at}.entryId"))?;
                if !seen.insert(entry_id.clone()) {
                    return fail(format!("'{dep_at}.entryId' '{entry_id}' is listed twice"));
                }
                let mut version_range = None;
                if let Some(range) = dependency.get("versionRange") {
                    let range = non_empty_string(Some(range), &format!("{dep_at}.versionRange"))?;
                    if !is_valid_version_range(&range) {
                        return fail(format!(
                            "'{dep_at}.versionRange' is not a valid version range"
                        ));
                    }
                    version_range = Some(range);
                }
                list.push(Dependency {
                    entry_id,
                    version_range,
                });
            }
            dependencies = Some(list);
        }
        let sandbox_policy = validate_sandbox_policy(
            item.get("sandboxPolicy"),
            &format!("{item_at}.sandboxPolicy"),
            contract,
        )?;
        variants.push(PlatformVariant {
            platform,
            architecture,
            dependencies,
            sandbox_policy,
        });
    }
    Ok(variants)
}

/// `/^[a-z][a-z0-9-]*:[a-z0-9][a-z0-9._-]*$/`
fn is_entry_id(value: &str) -> bool {
    let Some((namespace, name)) = value.split_once(':') else {
        return false;
    };
    let mut ns = namespace.chars();
    let mut nm = name.chars();
    ns.next().is_some_and(|c| c.is_ascii_lowercase())
        && ns.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && nm
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && nm.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

fn validate_entry(raw: &Json, at: &str, contract: &CatalogContract) -> Result<CatalogEntry> {
    let Some(object) = raw.as_object() else {
        return fail(format!("'{at}' must be an object"));
    };
    only_fields(
        object,
        &[
            "entryId",
            "entryRevision",
            "displayName",
            "identity",
            "platformVariants",
            "provenance",
        ],
        at,
    )?;
    let entry_id = non_empty_string(object.get("entryId"), &format!("{at}.entryId"))?;
    if !is_entry_id(&entry_id) {
        return fail(format!(
            "'{at}.entryId' '{entry_id}' must be namespaced, e.g. 'tool:name'"
        ));
    }
    let entry_revision = match object.get("entryRevision").and_then(Json::as_f64) {
        Some(n) if is_integer(n) && n >= 1.0 => n,
        _ => return fail(format!("'{at}.entryRevision' must be a positive integer")),
    };
    let Some(provenance) = record(object.get("provenance")) else {
        return fail(format!("'{at}.provenance' must be an object"));
    };
    only_fields(
        provenance,
        &["method", "sourceRevision"],
        &format!("{at}.provenance"),
    )?;
    let display_name = non_empty_string(object.get("displayName"), &format!("{at}.displayName"))?;
    let identity = validate_identity(object.get("identity"), &format!("{at}.identity"))?;
    let platform_variants = validate_variants(
        object.get("platformVariants"),
        &format!("{at}.platformVariants"),
        contract,
    )?;
    let provenance_method =
        non_empty_string(provenance.get("method"), &format!("{at}.provenance.method"))?;
    let provenance_source_revision = non_empty_string(
        provenance.get("sourceRevision"),
        &format!("{at}.provenance.sourceRevision"),
    )?;
    Ok(CatalogEntry {
        entry_id,
        entry_revision,
        display_name,
        identity,
        platform_variants,
        provenance_method,
        provenance_source_revision,
        raw: raw.clone(),
    })
}

// ---------------------------------------------------------------------------
// Variant selection and dependency closure
// ---------------------------------------------------------------------------

/// A selected variant; `exact` is false for the architecture-neutral fallback.
#[derive(Clone, Copy, Debug)]
pub struct VariantSelection<'a> {
    pub variant: &'a PlatformVariant,
    pub exact: bool,
}

/// Selects the exact architecture first, then the platform's neutral variant.
/// Another architecture's variant is never a fallback.
pub fn select_variant(
    entry: &CatalogEntry,
    platform: Platform,
    architecture: Architecture,
) -> Option<VariantSelection<'_>> {
    let for_platform = || {
        entry
            .platform_variants
            .iter()
            .filter(move |v| v.platform == platform)
    };
    if let Some(variant) = for_platform().find(|v| v.architecture == Some(architecture)) {
        return Some(VariantSelection {
            variant,
            exact: true,
        });
    }
    for_platform()
        .find(|v| v.architecture.is_none())
        .map(|variant| VariantSelection {
            variant,
            exact: false,
        })
}

#[derive(Clone, Copy, Debug)]
pub struct ClosureNode<'a> {
    pub entry: &'a CatalogEntry,
    pub variant: &'a PlatformVariant,
    pub exact: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosureFailure {
    /// `cycle`, `missing-entry`, or `unsupported-dependency`.
    pub reason: &'static str,
    pub detail: String,
}

pub type EntryIndex<'a> = HashMap<&'a str, &'a CatalogEntry>;

pub fn entry_index(entries: &[CatalogEntry]) -> EntryIndex<'_> {
    entries.iter().map(|e| (e.entry_id.as_str(), e)).collect()
}

struct Closure<'a, 'm> {
    by_id: &'m EntryIndex<'a>,
    platform: Platform,
    architecture: Architecture,
    nodes: Vec<ClosureNode<'a>>,
    done: HashSet<&'a str>,
    stack: Vec<&'a str>,
}

impl<'a> Closure<'a, '_> {
    fn visit(
        &mut self,
        entry: &'a CatalogEntry,
        selection: VariantSelection<'a>,
    ) -> std::result::Result<(), ClosureFailure> {
        if self.stack.contains(&entry.entry_id.as_str()) {
            let mut path: Vec<&str> = self.stack.clone();
            path.push(&entry.entry_id);
            return Err(ClosureFailure {
                reason: "cycle",
                detail: path.join(" -> "),
            });
        }
        if self.done.contains(entry.entry_id.as_str()) {
            return Ok(());
        }
        self.stack.push(&entry.entry_id);
        self.nodes.push(ClosureNode {
            entry,
            variant: selection.variant,
            exact: selection.exact,
        });
        for dependency in selection.variant.dependencies.iter().flatten() {
            let Some(target) = self.by_id.get(dependency.entry_id.as_str()).copied() else {
                return Err(ClosureFailure {
                    reason: "missing-entry",
                    detail: format!("{} -> {}", entry.entry_id, dependency.entry_id),
                });
            };
            let Some(selected) = select_variant(target, self.platform, self.architecture) else {
                return Err(ClosureFailure {
                    reason: "unsupported-dependency",
                    detail: format!(
                        "{} -> {} has no {}/{} variant",
                        entry.entry_id, dependency.entry_id, self.platform, self.architecture
                    ),
                });
            };
            self.visit(target, selected)?;
        }
        self.stack.pop();
        self.done.insert(&entry.entry_id);
        Ok(())
    }
}

/// Deterministic depth-first dependency closure; the root first, each
/// dependency in declaration order and once. Cycles are reported.
pub fn dependency_closure<'a>(
    root: &'a CatalogEntry,
    root_selection: VariantSelection<'a>,
    by_id: &EntryIndex<'a>,
    platform: Platform,
    architecture: Architecture,
) -> std::result::Result<Vec<ClosureNode<'a>>, ClosureFailure> {
    let mut closure = Closure {
        by_id,
        platform,
        architecture,
        nodes: Vec::new(),
        done: HashSet::new(),
        stack: Vec::new(),
    };
    closure.visit(root, root_selection)?;
    Ok(closure.nodes)
}

/// Composition limits for the v1 vocabulary (design §4.5).
pub fn composition_violation(nodes: &[ClosureNode<'_>]) -> Option<String> {
    let mut versions: Vec<&str> = Vec::new();
    for node in nodes {
        let version = node.variant.sandbox_policy.version();
        if !versions.contains(&version) {
            versions.push(version);
        }
    }
    if versions.len() > 1 {
        versions.sort_by(|a, b| cmp_utf16(a, b));
        return Some(format!(
            "mixed sandboxPolicy.version values ({})",
            versions.join(", ")
        ));
    }
    if nodes.len() < 2 {
        return None;
    }
    for node in nodes {
        for key in node.variant.sandbox_policy.raw().keys() {
            if key != "version" && key != "filesystem" {
                return Some(format!(
                    "'{}' uses '{key}', which has no v1 cross-entry composition rule",
                    node.entry.entry_id
                ));
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Revision-level validation
// ---------------------------------------------------------------------------

/// Validates one catalog revision against the v1 contract (design §7).
pub fn validate_catalog_revision(
    raw: &Json,
    contract: &CatalogContract,
) -> Result<CatalogRevision> {
    let Some(object) = raw.as_object() else {
        return fail("catalog root must be an object");
    };
    only_fields(
        object,
        &["catalogSchemaVersion", "catalogRevision", "entries"],
        "catalog",
    )?;
    if object.get("catalogSchemaVersion").and_then(Json::as_str)
        != Some(contract.catalog_schema_version.as_str())
    {
        return fail(format!(
            "catalog.catalogSchemaVersion must be '{}'",
            contract.catalog_schema_version
        ));
    }
    let catalog_revision =
        non_empty_string(object.get("catalogRevision"), "catalog.catalogRevision")?;
    if !is_catalog_revision_id(&catalog_revision) {
        return fail(format!(
            "catalog.catalogRevision '{catalog_revision}' must match YYYY-MM-DD.N"
        ));
    }
    let Some(raw_entries) = object.get("entries").and_then(Json::as_array) else {
        return fail("catalog.entries must be an array");
    };
    let entries = raw_entries
        .iter()
        .enumerate()
        .map(|(index, entry)| validate_entry(entry, &format!("entries[{index}]"), contract))
        .collect::<Result<Vec<_>>>()?;

    let mut by_id: EntryIndex<'_> = HashMap::new();
    for entry in &entries {
        if by_id.insert(&entry.entry_id, entry).is_some() {
            return fail(format!("duplicate entryId '{}'", entry.entry_id));
        }
    }

    for entry in &entries {
        for variant in &entry.platform_variants {
            for dependency in variant.dependencies.iter().flatten() {
                if !by_id.contains_key(dependency.entry_id.as_str()) {
                    return fail(format!(
                        "'{}' depends on unknown entry '{}' in this revision",
                        entry.entry_id, dependency.entry_id
                    ));
                }
                if dependency.entry_id == entry.entry_id {
                    return fail(format!("'{}' depends on itself", entry.entry_id));
                }
            }
        }
        for platform in Platform::ALL {
            for architecture in Architecture::ALL {
                let Some(selected) = select_variant(entry, platform, architecture) else {
                    continue;
                };
                let nodes =
                    match dependency_closure(entry, selected, &by_id, platform, architecture) {
                        Ok(nodes) => nodes,
                        Err(failure) => {
                            return fail(format!(
                                "'{}' on {platform}/{architecture}: {} ({})",
                                entry.entry_id, failure.reason, failure.detail
                            ))
                        }
                    };
                if let Some(violation) = composition_violation(&nodes) {
                    return fail(format!(
                        "'{}' on {platform}/{architecture}: {violation}",
                        entry.entry_id
                    ));
                }
                let classes: Vec<(&str, Vec<String>)> = COMPOSABLE_FILESYSTEM_FIELDS
                    .iter()
                    .map(|field| {
                        let values = nodes
                            .iter()
                            .flat_map(|node| node.variant.sandbox_policy.filesystem_field(field))
                            .map(str::to_string)
                            .collect();
                        (*field, values)
                    })
                    .collect();
                if let Some(overlap) = find_cross_class_overlap(&classes, platform) {
                    return fail(format!(
                        "'{}' on {platform}/{architecture}: {overlap}",
                        entry.entry_id
                    ));
                }
            }
        }
    }

    Ok(CatalogRevision {
        catalog_schema_version: contract.catalog_schema_version.clone(),
        catalog_revision,
        entries,
    })
}

/// Canonical JSON of an entry without `entryRevision` (history comparisons).
pub(crate) fn entry_semantic_key(entry: &CatalogEntry) -> String {
    let mut raw = entry.raw.clone();
    if let Json::Object(object) = &mut raw {
        object.remove("entryRevision");
    }
    crate::json::canonical_json(&raw)
}

/// Formats an entry revision number as TypeScript would interpolate it.
pub(crate) fn revision_number(value: f64) -> String {
    js_number_to_string(value)
}

#[allow(dead_code)]
pub(crate) fn describe(value: &Json) -> String {
    js_to_string(value)
}
