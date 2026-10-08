// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Catalog contract, entry model (one unversioned default plus additive
//! platform and version overlays), and contract/revision validation. The
//! effective-policy materialization lives in [`crate::policy_store::effective`].

use crate::policy_store::errors::{invalid_catalog, Result};
use crate::policy_store::json::{js_number_to_string, js_to_string, Json, JsonObject};
use crate::policy_store::model::{Architecture, IdentityStrength, Platform};
use crate::policy_store::paths::is_absolute_path;
use crate::policy_store::purl::parse_purl;
use crate::policy_store::text::{is_symbol_name, js_to_lower, replace_symbols, symbol_matches};
use crate::policy_store::vers::{VersRange, VersionScheme};
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
    /// Documented per-platform default templates (design §4.2). They may
    /// reference only host-known symbols.
    pub defaults: Vec<(Platform, String)>,
}

impl SymbolDefinition {
    pub fn default_for(&self, platform: Platform) -> Option<&str> {
        self.defaults
            .iter()
            .find(|(p, _)| *p == platform)
            .map(|(_, template)| template.as_str())
    }
}

/// The SDK contract version this SDK build emits for `ContainerRequest`
/// (`schemas/schema-version.json` `sdkMajorTargets`). Every catalog
/// revision this SDK bundles must target it.
pub const SDK_CONTRACT_VERSION: &str = "1.0.0";

/// Validated catalog contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogContract {
    pub catalog_schema_version: String,
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

/// Embedded, validated `default.requirements` (the four access fields of a
/// v1 `ContainerRequest`). The reviewed JSON object is kept as written so
/// field order and values are preserved exactly.
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogPolicy {
    raw: JsonObject,
}

impl CatalogPolicy {
    pub fn raw(&self) -> &JsonObject {
        &self.raw
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

/// One identity predicate of an entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityPredicate {
    Purl { value: String },
    InvocationName { names: Vec<String> },
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
    /// A `vers` range in the target entry's scheme. It is recorded in
    /// diagnostics; it does not select a version overlay.
    pub version_range: Option<String>,
    /// Intents of the target whose additions the reference adds to the
    /// target's base. `None` contributes the base only.
    pub intents: Option<Vec<String>>,
}

/// Additive policy data (`policyAdditions`): v1 allows only read-only and
/// read-write paths and outbound allow rules.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Additions {
    pub readonly_paths: Vec<String>,
    pub readwrite_paths: Vec<String>,
    pub egress_allow: Vec<Json>,
}

impl Additions {
    pub fn is_empty(&self) -> bool {
        self.readonly_paths.is_empty()
            && self.readwrite_paths.is_empty()
            && self.egress_allow.is_empty()
    }
}

/// An intent declaration, or an extension of an inherited intent.
#[derive(Clone, Debug, PartialEq)]
pub struct IntentDefinition {
    pub example_subcommands: Option<Vec<String>>,
    pub additions: Additions,
    pub dependencies: Vec<Dependency>,
}

/// The additions a platform or version overlay makes to the default.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Overlay {
    pub policy_additions: Additions,
    pub dependencies: Vec<Dependency>,
    /// Extensions of intents the default declares.
    pub intent_additions: Vec<(String, IntentDefinition)>,
    /// Intents the default does not declare (`newIntents`).
    pub new_intents: Vec<(String, IntentDefinition)>,
}

/// The unversioned default every entry has exactly once.
#[derive(Clone, Debug, PartialEq)]
pub struct EntryDefault {
    pub requirements: CatalogPolicy,
    pub dependencies: Vec<Dependency>,
    pub intents: Vec<(String, IntentDefinition)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlatformVariant {
    pub platform: Platform,
    pub architecture: Option<Architecture>,
    pub overlay: Overlay,
}

#[derive(Clone, Debug)]
pub struct VersionVariant {
    pub version_range: VersRange,
    pub overlay: Overlay,
}

impl PartialEq for VersionVariant {
    fn eq(&self, other: &Self) -> bool {
        self.version_range.as_str() == other.version_range.as_str() && self.overlay == other.overlay
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CatalogEntry {
    pub entry_id: String,
    pub entry_revision: f64,
    pub display_name: String,
    pub version_scheme: VersionScheme,
    pub identity: Vec<IdentityPredicate>,
    pub default: EntryDefault,
    pub platform_variants: Vec<PlatformVariant>,
    pub version_variants: Vec<VersionVariant>,
    pub provenance_method: String,
    pub provenance_source_revision: String,
    /// The validated entry as JSON (history comparisons use its canonical form).
    pub(crate) raw: Json,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CatalogRevision {
    pub catalog_schema_version: String,
    pub catalog_revision: String,
    /// The SDK `ContainerRequest` contract version the revision targets.
    pub sdk_contract_version: String,
    pub entries: Vec<CatalogEntry>,
}

pub(crate) fn fail<T>(message: impl AsRef<str>) -> Result<T> {
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
        &["$comment", "catalogSchemaVersion", "symbols"],
        "contract",
    )?;
    if raw.get("catalogSchemaVersion").and_then(Json::as_str) != Some(CATALOG_SCHEMA_VERSION) {
        return fail(format!(
            "contract.catalogSchemaVersion must be '{CATALOG_SCHEMA_VERSION}'"
        ));
    }
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
            &["source", "description", "defaults"],
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
        let mut defaults = Vec::new();
        if let Some(raw_defaults) = definition.get("defaults") {
            let at = format!("contract.symbols.{name}.defaults");
            let Some(raw_defaults) = raw_defaults.as_object() else {
                return fail(format!("{at} must be an object"));
            };
            if source == SymbolSource::Context {
                return fail(format!(
                    "{at}: a context symbol comes only from ResolveContext.projectRoot"
                ));
            }
            for (platform, template) in raw_defaults.iter() {
                let Some(parsed) = Platform::parse(platform) else {
                    return fail(format!("{at}.{platform} is not a catalog platform"));
                };
                let template = non_empty_string(Some(template), &format!("{at}.{platform}"))?;
                defaults.push((parsed, template));
            }
        }
        symbols.push((
            name.to_string(),
            SymbolDefinition {
                source,
                description,
                defaults,
            },
        ));
    }
    let contract = CatalogContract {
        catalog_schema_version: CATALOG_SCHEMA_VERSION.to_string(),
        symbols,
    };
    // Default templates may reference only host-known symbols and must form
    // an absolute path on their platform once those are substituted.
    for (name, definition) in contract.symbols() {
        for (platform, template) in &definition.defaults {
            let at = format!("contract.symbols.{name}.defaults.{platform}");
            validate_template_path(template, &at, &contract)?;
            for (_, _, referenced) in symbol_matches(template) {
                if contract.symbol(referenced).map(|d| d.source) != Some(SymbolSource::Host) {
                    return fail(format!(
                        "'{at}' references '{referenced}', which is not a host-known symbol"
                    ));
                }
            }
            let probe = replace_symbols(template, |_| match platform {
                Platform::Windows => "C:\\x".to_string(),
                _ => "/x".to_string(),
            });
            if !is_absolute_path(&probe, *platform) {
                return fail(format!("'{at}' does not form an absolute {platform} path"));
            }
        }
    }
    Ok(contract)
}

// ---------------------------------------------------------------------------
// Paths and symbols
// ---------------------------------------------------------------------------

pub(crate) fn validate_template_path(
    value: &str,
    at: &str,
    contract: &CatalogContract,
) -> Result<()> {
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

// ---------------------------------------------------------------------------
// Embedded requirements (v1 ContainerRequest access fields)
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
                let Some(block) = crate::policy_store::netrule::parse_cidr(&cidr) else {
                    return fail(format!("'{peer_at}.cidr' '{cidr}' is not a valid CIDR"));
                };
                if !deny_list && cidr.ends_with("/0") {
                    return fail(format!("'{peer_at}.cidr' is a wildcard network grant"));
                }
                if let Some(except) = peer.get("except") {
                    let except_at = format!("{peer_at}.except");
                    for (i, value) in string_array(Some(except), &except_at, 0)?
                        .iter()
                        .enumerate()
                    {
                        let inside = crate::policy_store::netrule::parse_cidr(value)
                            .is_some_and(|excluded| block.contains(excluded));
                        if !inside {
                            return fail(format!(
                                "'{except_at}[{i}]' '{value}' must be a CIDR of the same family within '{cidr}'"
                            ));
                        }
                    }
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
                    if protocol.as_str() == Some("icmp")
                        && (port.contains_key("port") || port.contains_key("endPort"))
                    {
                        return fail(format!("'{port_at}' must not set a port for icmp"));
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

pub(crate) fn validate_requirements(
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
    for (key, reason) in [
        ("command", "commands are supplied by the caller"),
        ("version", "the SDK owns the wire version"),
        ("lifecycle", "lifecycle settings are caller-owned"),
        ("environment", "environment data is caller-owned"),
        ("env", "environment data is caller-owned"),
    ] {
        if raw.contains_key(key) {
            return fail(format!("'{at}.{key}' is not an access field; {reason}"));
        }
    }
    only_fields(raw, &["filesystem", "network", "ui", "timeoutMs"], at)?;
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
        if network.contains_key("runtimeConfig") || network.contains_key("proxy") {
            return fail(format!(
                "'{at}.network' must not carry runtime proxy values; they are caller-owned"
            ));
        }
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
            &["disable", "clipboard", "allowInputInjection"],
            &format!("{at}.ui"),
        )?;
        if ui.get("disable").and_then(Json::as_bool).is_none() {
            return fail(format!(
                "'{at}.ui.disable' is required and must be a boolean"
            ));
        }
        if let Some(value) = ui.get("allowInputInjection") {
            if value.as_bool().is_none() {
                return fail(format!("'{at}.ui.allowInputInjection' must be a boolean"));
            }
        }
        if let Some(clipboard) = ui.get("clipboard") {
            if !matches!(clipboard.as_str(), Some("none" | "read" | "write" | "all")) {
                return fail(format!("'{at}.ui.clipboard' is unsupported"));
            }
        }
    }
    if let Some(timeout) = raw.get("timeoutMs") {
        let valid = timeout
            .as_f64()
            .is_some_and(|n| is_integer(n) && (1.0..=f64::from(u32::MAX)).contains(&n));
        if !valid {
            return fail(format!(
                "'{at}.timeoutMs' must be a positive unsigned 32-bit integer"
            ));
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
        IdentityPredicate::Purl { value } => {
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
                only_fields(item, &["kind", "value"], &item_at)?;
                let value = non_empty_string(item.get("value"), &format!("{item_at}.value"))?;
                let Some(parsed) = parse_purl(&value) else {
                    return fail(format!("'{item_at}.value' is not a valid package URL"));
                };
                if parsed.version.is_some() || parsed.has_qualifiers || parsed.has_subpath {
                    return fail(format!(
                        "'{item_at}.value' must name only type, namespace, and name; versions select 'versionVariants', and qualifiers and subpaths are not identity"
                    ));
                }
                predicates.push(IdentityPredicate::Purl { value });
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
            let shown: Vec<&str> = key.split('\0').filter(|s| !s.is_empty()).collect();
            return fail(format!("'{at}' repeats identity '{}'", shown.join("/")));
        }
    }
    Ok(predicates)
}

fn validate_dependencies(raw: Option<&Json>, at: &str) -> Result<Vec<Dependency>> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let Some(items) = raw.as_array() else {
        return fail(format!("'{at}' must be an array"));
    };
    let mut seen = HashSet::new();
    let mut list = Vec::new();
    for (index, dependency) in items.iter().enumerate() {
        let dep_at = format!("{at}[{index}]");
        let Some(dependency) = dependency.as_object() else {
            return fail(format!("'{dep_at}' must be an object"));
        };
        only_fields(dependency, &["entryId", "versionRange", "intents"], &dep_at)?;
        let entry_id = non_empty_string(dependency.get("entryId"), &format!("{dep_at}.entryId"))?;
        if !seen.insert(entry_id.clone()) {
            return fail(format!("'{dep_at}.entryId' '{entry_id}' is listed twice"));
        }
        let mut version_range = None;
        if let Some(range) = dependency.get("versionRange") {
            let range = non_empty_string(Some(range), &format!("{dep_at}.versionRange"))?;
            if let Err(reason) = VersRange::parse(&range) {
                return fail(format!(
                    "'{dep_at}.versionRange' is not a valid vers range: {reason}"
                ));
            }
            version_range = Some(range);
        }
        let intents = match dependency.get("intents") {
            None => None,
            Some(value) => {
                let names = string_array(Some(value), &format!("{dep_at}.intents"), 1)?;
                let mut unique = HashSet::new();
                for name in &names {
                    if !is_intent_name(name) {
                        return fail(format!(
                            "'{dep_at}.intents' entry '{name}' is not a valid intent name"
                        ));
                    }
                    if !unique.insert(name.as_str()) {
                        return fail(format!("'{dep_at}.intents' lists '{name}' twice"));
                    }
                }
                Some(names)
            }
        };
        list.push(Dependency {
            entry_id,
            version_range,
            intents,
        });
    }
    Ok(list)
}

const ADDITIONS_RULE: &str =
    "additions may only use filesystem.readonlyPaths, filesystem.readwritePaths, and network.egress.allow";

fn validate_additions(
    raw: Option<&Json>,
    at: &str,
    contract: &CatalogContract,
) -> Result<Additions> {
    let Some(raw) = raw else {
        return Ok(Additions::default());
    };
    let Some(object) = raw.as_object() else {
        return fail(format!("'{at}' must be an object"));
    };
    let mut additions = Additions::default();
    for (key, value) in object.iter() {
        match key {
            "filesystem" => {
                let Some(filesystem) = value.as_object() else {
                    return fail(format!("'{at}.filesystem' must be an object"));
                };
                for (field, values) in filesystem.iter() {
                    let field_at = format!("{at}.filesystem.{field}");
                    let target = match field {
                        "readonlyPaths" => &mut additions.readonly_paths,
                        "readwritePaths" => &mut additions.readwrite_paths,
                        _ => {
                            return fail(format!("'{field_at}' is not additive; {ADDITIONS_RULE}"))
                        }
                    };
                    for (index, path) in string_array(Some(values), &field_at, 0)?
                        .into_iter()
                        .enumerate()
                    {
                        validate_template_path(&path, &format!("{field_at}[{index}]"), contract)?;
                        target.push(path);
                    }
                }
            }
            "network" => {
                let Some(network) = value.as_object() else {
                    return fail(format!("'{at}.network' must be an object"));
                };
                for (field, value) in network.iter() {
                    if field != "egress" {
                        return fail(format!(
                            "'{at}.network.{field}' is not additive; {ADDITIONS_RULE}"
                        ));
                    }
                    let Some(egress) = value.as_object() else {
                        return fail(format!("'{at}.network.egress' must be an object"));
                    };
                    for (field, rules) in egress.iter() {
                        let rules_at = format!("{at}.network.egress.{field}");
                        if field != "allow" {
                            return fail(format!("'{rules_at}' is not additive; {ADDITIONS_RULE}"));
                        }
                        validate_network_rules(rules, &rules_at, false)?;
                        additions
                            .egress_allow
                            .extend(rules.as_array().into_iter().flatten().cloned());
                    }
                }
            }
            _ => return fail(format!("'{at}.{key}' is not additive; {ADDITIONS_RULE}")),
        }
    }
    Ok(additions)
}

/// `/^[a-z][a-z0-9_-]*$/`
fn is_intent_name(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn validate_intents(
    raw: Option<&Json>,
    at: &str,
    contract: &CatalogContract,
) -> Result<Vec<(String, IntentDefinition)>> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let Some(object) = raw.as_object() else {
        return fail(format!("'{at}' must be an object"));
    };
    let mut intents = Vec::new();
    for (name, definition) in object.iter() {
        let intent_at = format!("{at}.{name}");
        if !is_intent_name(name) {
            return fail(format!(
                "'{intent_at}' is not a valid intent name (lower-case letters, digits, '-', '_')"
            ));
        }
        let Some(definition) = definition.as_object() else {
            return fail(format!("'{intent_at}' must be an object"));
        };
        only_fields(
            definition,
            &["exampleSubcommands", "policyAdditions", "dependencies"],
            &intent_at,
        )?;
        let example_subcommands = match definition.get("exampleSubcommands") {
            None => None,
            Some(value) => Some(string_array(
                Some(value),
                &format!("{intent_at}.exampleSubcommands"),
                1,
            )?),
        };
        intents.push((
            name.to_string(),
            IntentDefinition {
                example_subcommands,
                additions: validate_additions(
                    definition.get("policyAdditions"),
                    &format!("{intent_at}.policyAdditions"),
                    contract,
                )?,
                dependencies: validate_dependencies(
                    definition.get("dependencies"),
                    &format!("{intent_at}.dependencies"),
                )?,
            },
        ));
    }
    Ok(intents)
}

fn validate_overlay(
    item: &JsonObject,
    at: &str,
    default_intents: &[(String, IntentDefinition)],
    contract: &CatalogContract,
) -> Result<Overlay> {
    let intent_additions = validate_intents(
        item.get("intentAdditions"),
        &format!("{at}.intentAdditions"),
        contract,
    )?;
    for (name, _) in &intent_additions {
        if !default_intents.iter().any(|(n, _)| n == name) {
            return fail(format!(
                "'{at}.intentAdditions.{name}' extends an intent the default does not declare; use 'newIntents' to declare a new one"
            ));
        }
    }
    let new_intents = validate_intents(
        item.get("newIntents"),
        &format!("{at}.newIntents"),
        contract,
    )?;
    for (name, _) in &new_intents {
        if default_intents.iter().any(|(n, _)| n == name) {
            return fail(format!(
                "'{at}.newIntents.{name}' redeclares an inherited intent; use 'intentAdditions' to extend it"
            ));
        }
    }
    Ok(Overlay {
        policy_additions: validate_additions(
            item.get("policyAdditions"),
            &format!("{at}.policyAdditions"),
            contract,
        )?,
        dependencies: validate_dependencies(
            item.get("dependencies"),
            &format!("{at}.dependencies"),
        )?,
        intent_additions,
        new_intents,
    })
}

const OVERLAY_FIELDS: [&str; 4] = [
    "policyAdditions",
    "dependencies",
    "intentAdditions",
    "newIntents",
];

fn validate_default(
    raw: Option<&Json>,
    at: &str,
    contract: &CatalogContract,
) -> Result<EntryDefault> {
    let Some(item) = record(raw) else {
        return fail(format!(
            "'{at}' must be an object; every entry has exactly one unversioned default"
        ));
    };
    only_fields(item, &["requirements", "dependencies", "intents"], at)?;
    Ok(EntryDefault {
        requirements: validate_requirements(
            item.get("requirements"),
            &format!("{at}.requirements"),
            contract,
        )?,
        dependencies: validate_dependencies(
            item.get("dependencies"),
            &format!("{at}.dependencies"),
        )?,
        intents: validate_intents(item.get("intents"), &format!("{at}.intents"), contract)?,
    })
}

fn validate_platform_variants(
    raw: Option<&Json>,
    at: &str,
    default: &EntryDefault,
    contract: &CatalogContract,
) -> Result<Vec<PlatformVariant>> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let Some(items) = raw.as_array() else {
        return fail(format!("'{at}' must be an array"));
    };
    let mut selectors = HashSet::new();
    let mut variants = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let item_at = format!("{at}[{index}]");
        let Some(item) = item.as_object() else {
            return fail(format!("'{item_at}' must be an object"));
        };
        let mut allowed = vec!["when"];
        allowed.extend(OVERLAY_FIELDS);
        only_fields(item, &allowed, &item_at)?;
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
        variants.push(PlatformVariant {
            platform,
            architecture,
            overlay: validate_overlay(item, &item_at, &default.intents, contract)?,
        });
    }
    Ok(variants)
}

fn validate_version_variants(
    raw: Option<&Json>,
    at: &str,
    scheme: VersionScheme,
    default: &EntryDefault,
    contract: &CatalogContract,
) -> Result<Vec<VersionVariant>> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let Some(items) = raw.as_array() else {
        return fail(format!("'{at}' must be an array"));
    };
    let mut variants: Vec<VersionVariant> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let item_at = format!("{at}[{index}]");
        let Some(item) = item.as_object() else {
            return fail(format!("'{item_at}' must be an object"));
        };
        let mut allowed = vec!["versionRange"];
        allowed.extend(OVERLAY_FIELDS);
        only_fields(item, &allowed, &item_at)?;
        let text = non_empty_string(item.get("versionRange"), &format!("{item_at}.versionRange"))?;
        let range = match VersRange::parse(&text) {
            Ok(range) => range,
            Err(reason) => {
                return fail(format!(
                    "'{item_at}.versionRange' is not a valid vers range: {reason}"
                ))
            }
        };
        if range.scheme() != scheme {
            return fail(format!(
                "'{item_at}.versionRange' uses '{}', but the entry's versionScheme is '{scheme}'",
                range.scheme()
            ));
        }
        for (other_index, other) in variants.iter().enumerate() {
            if range.overlaps(&other.version_range) {
                return fail(format!(
                    "'{item_at}.versionRange' '{text}' overlaps '{at}[{other_index}].versionRange' '{}'; version ranges must not overlap",
                    other.version_range
                ));
            }
        }
        variants.push(VersionVariant {
            version_range: range,
            overlay: validate_overlay(item, &item_at, &default.intents, contract)?,
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
            "versionScheme",
            "identity",
            "default",
            "platformVariants",
            "versionVariants",
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
    let Some(version_scheme) = object
        .get("versionScheme")
        .and_then(Json::as_str)
        .and_then(VersionScheme::parse)
    else {
        return fail(format!(
            "'{at}.versionScheme' must be one of npm, semver, pypi, nuget, intdot"
        ));
    };
    let identity = validate_identity(object.get("identity"), &format!("{at}.identity"))?;
    let default = validate_default(object.get("default"), &format!("{at}.default"), contract)?;
    let platform_variants = validate_platform_variants(
        object.get("platformVariants"),
        &format!("{at}.platformVariants"),
        &default,
        contract,
    )?;
    let version_variants = validate_version_variants(
        object.get("versionVariants"),
        &format!("{at}.versionVariants"),
        version_scheme,
        &default,
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
        version_scheme,
        identity,
        default,
        platform_variants,
        version_variants,
        provenance_method,
        provenance_source_revision,
        raw: raw.clone(),
    })
}

impl CatalogEntry {
    /// Every dependency edge the entry declares anywhere, with where it is.
    pub(crate) fn all_dependencies(&self) -> Vec<&Dependency> {
        fn intents(list: &[(String, IntentDefinition)]) -> impl Iterator<Item = &Dependency> {
            list.iter().flat_map(|(_, d)| &d.dependencies)
        }
        let mut out: Vec<&Dependency> = Vec::new();
        out.extend(&self.default.dependencies);
        out.extend(intents(&self.default.intents));
        let overlays = self
            .platform_variants
            .iter()
            .map(|v| &v.overlay)
            .chain(self.version_variants.iter().map(|v| &v.overlay));
        for overlay in overlays {
            out.extend(&overlay.dependencies);
            out.extend(intents(&overlay.intent_additions));
            out.extend(intents(&overlay.new_intents));
        }
        out
    }
}

pub type EntryIndex<'a> = HashMap<&'a str, &'a CatalogEntry>;

pub fn entry_index(entries: &[CatalogEntry]) -> EntryIndex<'_> {
    entries.iter().map(|e| (e.entry_id.as_str(), e)).collect()
}

// ---------------------------------------------------------------------------
// Revision-level validation
// ---------------------------------------------------------------------------

/// Validates one catalog revision against the v1 contract (design §7),
/// including every effective policy each entry can produce.
pub fn validate_catalog_revision(
    raw: &Json,
    contract: &CatalogContract,
) -> Result<CatalogRevision> {
    let Some(object) = raw.as_object() else {
        return fail("catalog root must be an object");
    };
    only_fields(
        object,
        &[
            "catalogSchemaVersion",
            "catalogRevision",
            "sdkContractVersion",
            "entries",
        ],
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
    let sdk_contract_version = non_empty_string(
        object.get("sdkContractVersion"),
        "catalog.sdkContractVersion",
    )?;
    if sdk_contract_version != SDK_CONTRACT_VERSION {
        return fail(format!(
            "catalog.sdkContractVersion '{sdk_contract_version}' is not this SDK's ContainerRequest contract version '{SDK_CONTRACT_VERSION}'"
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
        for dependency in entry.all_dependencies() {
            let Some(target) = by_id.get(dependency.entry_id.as_str()) else {
                return fail(format!(
                    "'{}' depends on unknown entry '{}' in this revision",
                    entry.entry_id, dependency.entry_id
                ));
            };
            if dependency.entry_id == entry.entry_id {
                return fail(format!("'{}' depends on itself", entry.entry_id));
            }
            if let Some(range) = &dependency.version_range {
                let scheme = VersRange::parse(range).map(|r| r.scheme()).ok();
                if scheme != Some(target.version_scheme) {
                    return fail(format!(
                        "'{}' requires '{}' with range '{range}', which does not use that entry's versionScheme '{}'",
                        entry.entry_id, dependency.entry_id, target.version_scheme
                    ));
                }
            }
        }
    }

    crate::policy_store::effective::validate_materializations(&entries, &by_id, contract)?;

    Ok(CatalogRevision {
        catalog_schema_version: contract.catalog_schema_version.clone(),
        catalog_revision,
        sdk_contract_version,
        entries,
    })
}

/// Canonical JSON of an entry without `entryRevision` (history comparisons).
pub(crate) fn entry_semantic_key(entry: &CatalogEntry) -> String {
    let mut raw = entry.raw.clone();
    if let Json::Object(object) = &mut raw {
        object.remove("entryRevision");
    }
    crate::policy_store::json::canonical_json(&raw)
}

/// Formats an entry revision number as TypeScript would interpolate it.
pub(crate) fn revision_number(value: f64) -> String {
    js_number_to_string(value)
}

#[allow(dead_code)]
pub(crate) fn describe(value: &Json) -> String {
    js_to_string(value)
}
