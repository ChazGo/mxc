// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Typed v1 conversion and exact SDK-contract validation (design §4.2).
//!
//! Composed requirements stay in the catalog's JSON model inside the store.
//! This module converts them to `ContainerRequirements` (the v1 section
//! types) and validates them against the SDK target contract: the typed
//! sections build the exact `OneShotRequest` through the SDK hook, a
//! disposable copy is normalized, and an independent exact JSON serialization
//! (mapping `allowInputInjection` to wire `injection` and supplying the
//! SDK-owned wire version) must parse to the same request. The fixture
//! command is never executed or returned.

use crate::policy::{
    ClipboardPolicy, ContainerRequirements, FilesystemPolicy, NetworkAction, NetworkEgressPolicy,
    NetworkIngressPolicy, NetworkPeerPolicy, NetworkPolicy, NetworkPortPolicy, NetworkProtocol,
    NetworkRulePolicy, UiPolicy,
};
use crate::policy_store::catalog::{CatalogContract, SDK_CONTRACT_VERSION};
use crate::policy_store::effective::Materialized;
use crate::policy_store::json::{Json, JsonObject};
use crate::policy_store::model::{Platform, Requirements};
use crate::policy_store::text::replace_symbols;

/// The validation-only command bound into exact requests.
pub const VALIDATION_COMMAND: &str = "mxc-policy-store-validation";
const VALIDATION_CONTAINER_ID: &str = "mxc-policy-store-validation";

fn action(value: Option<&Json>, at: &str) -> Result<Option<NetworkAction>, String> {
    match value.map(Json::as_str) {
        None => Ok(None),
        Some(Some("allow")) => Ok(Some(NetworkAction::Allow)),
        Some(Some("deny")) => Ok(Some(NetworkAction::Deny)),
        Some(_) => Err(format!("'{at}' is not a network action")),
    }
}

fn strings(value: Option<&Json>) -> Vec<String> {
    value
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
        .filter_map(Json::as_str)
        .map(str::to_string)
        .collect()
}

fn port(value: Option<&Json>, at: &str) -> Result<Option<u16>, String> {
    match value {
        None => Ok(None),
        Some(n) => n
            .as_f64()
            .filter(|n| n.fract() == 0.0 && (1.0..=65535.0).contains(n))
            .map(|n| Some(n as u16))
            .ok_or_else(|| format!("'{at}' is not a port")),
    }
}

fn rules(value: Option<&Json>, at: &str) -> Result<Option<Vec<NetworkRulePolicy>>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let items = value
        .as_array()
        .ok_or_else(|| format!("'{at}' must be an array"))?;
    items
        .iter()
        .enumerate()
        .map(|(index, rule)| {
            let at = format!("{at}[{index}]");
            let to = rule.get("to").and_then(Json::as_array).map(|peers| {
                peers
                    .iter()
                    .map(|peer| NetworkPeerPolicy {
                        cidr: peer
                            .get("cidr")
                            .and_then(Json::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        except: peer.get("except").map(|e| strings(Some(e))),
                    })
                    .collect()
            });
            let ports = match rule.get("ports").and_then(Json::as_array) {
                None => None,
                Some(ports) => Some(
                    ports
                        .iter()
                        .enumerate()
                        .map(|(i, p)| {
                            let at = format!("{at}.ports[{i}]");
                            let protocol = match p.get("protocol").map(Json::as_str) {
                                None => None,
                                Some(Some("tcp")) => Some(NetworkProtocol::Tcp),
                                Some(Some("udp")) => Some(NetworkProtocol::Udp),
                                Some(Some("icmp")) => Some(NetworkProtocol::Icmp),
                                Some(Some("any")) => Some(NetworkProtocol::Any),
                                Some(_) => return Err(format!("'{at}.protocol' is unsupported")),
                            };
                            Ok(NetworkPortPolicy {
                                protocol,
                                port: port(p.get("port"), &format!("{at}.port"))?,
                                end_port: port(p.get("endPort"), &format!("{at}.endPort"))?,
                            })
                        })
                        .collect::<Result<Vec<_>, String>>()?,
                ),
            };
            Ok(NetworkRulePolicy { to, ports })
        })
        .collect::<Result<Vec<_>, String>>()
        .map(Some)
}

/// Converts composed requirements to the v1 section types.
pub fn to_container_requirements(
    requirements: &Requirements,
) -> Result<ContainerRequirements, String> {
    let filesystem = requirements.filesystem.as_ref().map(|fs| FilesystemPolicy {
        readwrite_paths: fs.readwrite_paths.clone().unwrap_or_default(),
        readonly_paths: fs.readonly_paths.clone().unwrap_or_default(),
        denied_paths: fs.denied_paths.clone().unwrap_or_default(),
        clear_policy_on_exit: None,
    });
    let network = match &requirements.network {
        None => None,
        Some(network) => Some(NetworkPolicy {
            egress: match network.get("egress") {
                None => None,
                Some(egress) => Some(NetworkEgressPolicy {
                    default: action(egress.get("default"), "network.egress.default")?,
                    allow: rules(egress.get("allow"), "network.egress.allow")?,
                    deny: rules(egress.get("deny"), "network.egress.deny")?,
                }),
            },
            ingress: match network.get("ingress") {
                None => None,
                Some(ingress) => Some(NetworkIngressPolicy {
                    default: action(ingress.get("default"), "network.ingress.default")?,
                    host_loopback: action(
                        ingress.get("hostLoopback"),
                        "network.ingress.hostLoopback",
                    )?,
                }),
            },
            runtime_config: None,
        }),
    };
    let ui = match &requirements.ui {
        None => None,
        Some(ui) => Some(UiPolicy {
            disable: ui
                .get("disable")
                .and_then(Json::as_bool)
                .ok_or("'ui.disable' is required")?,
            clipboard: match ui.get("clipboard").map(Json::as_str) {
                None | Some(Some("none")) => ClipboardPolicy::None,
                Some(Some("read")) => ClipboardPolicy::Read,
                Some(Some("write")) => ClipboardPolicy::Write,
                Some(Some("all")) => ClipboardPolicy::All,
                Some(_) => return Err("'ui.clipboard' is unsupported".to_string()),
            },
            allow_input_injection: ui
                .get("allowInputInjection")
                .and_then(Json::as_bool)
                .unwrap_or(false),
        }),
    };
    Ok(ContainerRequirements {
        filesystem,
        network,
        ui,
        timeout_ms: requirements.timeout_ms,
    })
}

/// The exact SDK target one-shot document for `requirements`, as the SDK
/// would serialize it with the validation command.
pub fn exact_document(requirements: &Requirements) -> Json {
    let mut root = JsonObject::new();
    root.insert("version", SDK_CONTRACT_VERSION.into());
    root.insert("containerId", VALIDATION_CONTAINER_ID.into());
    root.insert("containment", "process".into());
    let mut lifecycle = JsonObject::new();
    lifecycle.insert("destroyOnExit", Json::Bool(true));
    lifecycle.insert("preservePolicy", Json::Bool(false));
    root.insert("lifecycle", Json::Object(lifecycle));
    let mut process = JsonObject::new();
    process.insert("commandLine", VALIDATION_COMMAND.into());
    process.insert(
        "timeout",
        Json::Number(f64::from(requirements.timeout_ms.unwrap_or(0))),
    );
    root.insert("process", Json::Object(process));
    let fs = requirements.filesystem.clone().unwrap_or_default();
    let mut filesystem = JsonObject::new();
    let list = |v: &Option<Vec<String>>| {
        Json::Array(
            v.iter()
                .flatten()
                .map(|p| Json::String(p.clone()))
                .collect(),
        )
    };
    filesystem.insert("readwritePaths", list(&fs.readwrite_paths));
    filesystem.insert("readonlyPaths", list(&fs.readonly_paths));
    filesystem.insert("deniedPaths", list(&fs.denied_paths));
    root.insert("filesystem", Json::Object(filesystem));
    if let Some(network) = &requirements.network {
        root.insert("network", network.clone());
    }
    if let Some(ui) = &requirements.ui {
        let mut wire = JsonObject::new();
        wire.insert(
            "disable",
            ui.get("disable").cloned().unwrap_or(Json::Bool(true)),
        );
        wire.insert(
            "clipboard",
            ui.get("clipboard")
                .cloned()
                .unwrap_or_else(|| "none".into()),
        );
        wire.insert(
            "injection",
            ui.get("allowInputInjection")
                .cloned()
                .unwrap_or(Json::Bool(false)),
        );
        root.insert("ui", Json::Object(wire));
    }
    Json::Object(root)
}

/// Validates concrete (symbol-free) requirements against the SDK target.
pub fn validate_exact(requirements: &Requirements) -> Result<(), String> {
    let typed = to_container_requirements(requirements)?;
    let wire = crate::policy_store::json::canonical_json(&exact_document(requirements));
    crate::policy::check_requirements_exact(
        &typed,
        &wire,
        VALIDATION_COMMAND,
        VALIDATION_CONTAINER_ID,
    )
}

/// A fixture value for `symbol` on `platform`.
pub fn fixture_symbol(symbol: &str, platform: Platform) -> String {
    match platform {
        Platform::Windows => format!("C:\\mxc-policy-store-fixture\\{symbol}"),
        _ => format!("/mxc-policy-store-fixture/{symbol}"),
    }
}

/// Binds fixture symbols into symbolic requirements.
pub fn bind_fixture_symbols(requirements: &Requirements, platform: Platform) -> Requirements {
    let mut bound = requirements.clone();
    if let Some(fs) = &mut bound.filesystem {
        for list in [
            &mut fs.denied_paths,
            &mut fs.readonly_paths,
            &mut fs.readwrite_paths,
        ]
        .into_iter()
        .flatten()
        {
            for path in list.iter_mut() {
                let substituted = replace_symbols(path, |name| fixture_symbol(name, platform));
                *path = crate::policy_store::paths::normalize_path(&substituted, platform);
            }
        }
    }
    bound
}

/// Validates one materialized combination with fixture symbols bound.
pub fn validate_materialized(
    materialized: &Materialized,
    _contract: &CatalogContract,
) -> Result<(), String> {
    validate_exact(&bind_fixture_symbols(
        &materialized.requirements,
        materialized.platform,
    ))
}
