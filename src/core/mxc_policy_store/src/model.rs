// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Public request and result types (design §5). Every result type renders to
//! the exact JSON shape the original TypeScript prototype produced, which the
//! SDK bindings and conformance vectors rely on (`to_json`), with absent optional fields
//! omitted rather than written as `null`.

use crate::json::{Json, JsonObject};
use std::fmt;

/// Catalog platform selector (design §4.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Platform {
    Windows,
    Linux,
    Macos,
}

impl Platform {
    /// Declaration order used by validation (`PLATFORMS`).
    pub const ALL: [Platform; 3] = [Platform::Windows, Platform::Linux, Platform::Macos];

    pub fn as_str(self) -> &'static str {
        match self {
            Platform::Windows => "windows",
            Platform::Linux => "linux",
            Platform::Macos => "macos",
        }
    }

    pub fn parse(value: &str) -> Option<Platform> {
        Platform::ALL.into_iter().find(|p| p.as_str() == value)
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Catalog architecture selector (design §4.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Architecture {
    X64,
    Arm64,
}

impl Architecture {
    /// Declaration order used by validation (`ARCHITECTURES`).
    pub const ALL: [Architecture; 2] = [Architecture::X64, Architecture::Arm64];

    pub fn as_str(self) -> &'static str {
        match self {
            Architecture::X64 => "x64",
            Architecture::Arm64 => "arm64",
        }
    }

    pub fn parse(value: &str) -> Option<Architecture> {
        Architecture::ALL.into_iter().find(|a| a.as_str() == value)
    }
}

impl fmt::Display for Architecture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Runtime lookup input (design §5.1).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ToolCandidate {
    pub invocation_name: String,
    pub package_url: Option<String>,
    pub detected_version: Option<String>,
}

impl ToolCandidate {
    pub fn new(invocation_name: impl Into<String>) -> Self {
        Self {
            invocation_name: invocation_name.into(),
            ..Self::default()
        }
    }

    pub fn with_package_url(mut self, value: impl Into<String>) -> Self {
        self.package_url = Some(value.into());
        self
    }

    pub fn with_detected_version(mut self, value: impl Into<String>) -> Self {
        self.detected_version = Some(value.into());
        self
    }
}

/// A bare name is shorthand for `ToolCandidate { invocation_name }`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolInput {
    Name(String),
    Candidate(ToolCandidate),
}

impl ToolInput {
    pub(crate) fn to_candidate(&self) -> ToolCandidate {
        match self {
            ToolInput::Name(name) => ToolCandidate::new(name.clone()),
            ToolInput::Candidate(candidate) => candidate.clone(),
        }
    }
}

impl From<&str> for ToolInput {
    fn from(value: &str) -> Self {
        ToolInput::Name(value.to_string())
    }
}

impl From<String> for ToolInput {
    fn from(value: String) -> Self {
        ToolInput::Name(value)
    }
}

impl From<ToolCandidate> for ToolInput {
    fn from(value: ToolCandidate) -> Self {
        ToolInput::Candidate(value)
    }
}

/// One tool or a list of tools. A single input is exactly a one-element list.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ToolInputs(pub Vec<ToolInput>);

impl From<ToolInput> for ToolInputs {
    fn from(value: ToolInput) -> Self {
        ToolInputs(vec![value])
    }
}

impl From<&str> for ToolInputs {
    fn from(value: &str) -> Self {
        ToolInputs(vec![value.into()])
    }
}

impl From<String> for ToolInputs {
    fn from(value: String) -> Self {
        ToolInputs(vec![value.into()])
    }
}

impl From<ToolCandidate> for ToolInputs {
    fn from(value: ToolCandidate) -> Self {
        ToolInputs(vec![value.into()])
    }
}

impl From<Vec<ToolInput>> for ToolInputs {
    fn from(value: Vec<ToolInput>) -> Self {
        ToolInputs(value)
    }
}

impl From<&[ToolInput]> for ToolInputs {
    fn from(value: &[ToolInput]) -> Self {
        ToolInputs(value.to_vec())
    }
}

impl From<Vec<&str>> for ToolInputs {
    fn from(value: Vec<&str>) -> Self {
        ToolInputs(value.into_iter().map(ToolInput::from).collect())
    }
}

impl<const N: usize> From<[&str; N]> for ToolInputs {
    fn from(value: [&str; N]) -> Self {
        ToolInputs(value.into_iter().map(ToolInput::from).collect())
    }
}

/// Caller-supplied symbol values, keyed by symbol name. Key order follows
/// ECMAScript property order (the order TypeScript enumerates them), and a
/// repeated name keeps its first position and takes the last value.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SymbolMap(JsonObject);

impl SymbolMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.0.insert(name, Json::String(value.into()));
    }

    /// Own-key lookup (never consults anything like a prototype chain).
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).and_then(Json::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .map(|(k, v)| (k, v.as_str().unwrap_or_default()))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<K: Into<String>, V: Into<String>> FromIterator<(K, V)> for SymbolMap {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let mut map = SymbolMap::new();
        for (k, v) in iter {
            map.insert(k, v);
        }
        map
    }
}

/// Runtime lookup context (design §5.1). `platform` and `architecture` are
/// the caller's raw strings so that an unsupported value is reported as
/// `malformed_request` (`invalid_context`) exactly like the original TypeScript prototype.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResolveContext {
    pub project_root: Option<String>,
    pub symbols: Option<SymbolMap>,
    pub platform: Option<String>,
    pub architecture: Option<String>,
    pub catalog_revision: Option<String>,
    pub allow_weak_identity_fallback: bool,
}

impl ResolveContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn platform(mut self, platform: Platform) -> Self {
        self.platform = Some(platform.as_str().to_string());
        self
    }

    pub fn architecture(mut self, architecture: Architecture) -> Self {
        self.architecture = Some(architecture.as_str().to_string());
        self
    }

    pub fn project_root(mut self, value: impl Into<String>) -> Self {
        self.project_root = Some(value.into());
        self
    }

    pub fn symbol(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.symbols
            .get_or_insert_with(SymbolMap::new)
            .insert(name, value);
        self
    }

    pub fn catalog_revision(mut self, value: impl Into<String>) -> Self {
        self.catalog_revision = Some(value.into());
        self
    }

    pub fn allow_weak(mut self, allow: bool) -> Self {
        self.allow_weak_identity_fallback = allow;
        self
    }
}

/// Strength of a matched identity predicate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityStrength {
    Strong,
    Weak,
}

impl IdentityStrength {
    pub fn as_str(self) -> &'static str {
        match self {
            IdentityStrength::Strong => "strong",
            IdentityStrength::Weak => "weak",
        }
    }
}

/// The filesystem part of a composed policy.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FilesystemPolicy {
    pub denied_paths: Option<Vec<String>>,
    pub readonly_paths: Option<Vec<String>>,
    pub readwrite_paths: Option<Vec<String>>,
}

fn strings(values: &[String]) -> Json {
    Json::Array(values.iter().map(|v| Json::String(v.clone())).collect())
}

impl FilesystemPolicy {
    pub fn to_json(&self) -> Json {
        let mut object = JsonObject::new();
        if let Some(v) = &self.denied_paths {
            object.insert("deniedPaths", strings(v));
        }
        if let Some(v) = &self.readonly_paths {
            object.insert("readonlyPaths", strings(v));
        }
        if let Some(v) = &self.readwrite_paths {
            object.insert("readwritePaths", strings(v));
        }
        Json::Object(object)
    }
}

/// The catalog-supported subset of MXC's `SandboxPolicy`, structurally
/// compatible with the MXC SDK type. `network` and `ui` are carried as the
/// reviewed JSON from the catalog.
#[derive(Clone, Debug, PartialEq)]
pub struct SandboxPolicy {
    pub version: String,
    pub filesystem: Option<FilesystemPolicy>,
    pub network: Option<Json>,
    pub ui: Option<Json>,
    pub timeout_ms: Option<f64>,
}

impl SandboxPolicy {
    pub fn to_json(&self) -> Json {
        let mut object = JsonObject::new();
        object.insert("version", Json::String(self.version.clone()));
        if let Some(fs) = &self.filesystem {
            object.insert("filesystem", fs.to_json());
        }
        if let Some(network) = &self.network {
            object.insert("network", network.clone());
        }
        if let Some(ui) = &self.ui {
            object.insert("ui", ui.clone());
        }
        if let Some(timeout) = self.timeout_ms {
            object.insert("timeoutMs", Json::Number(timeout));
        }
        Json::Object(object)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MatchedIdentity {
    pub kind: String,
    pub strength: IdentityStrength,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EntryMatchRecord {
    pub entry_id: String,
    pub entry_revision: f64,
    pub matched_identities: Vec<MatchedIdentity>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolRecord {
    pub input_index: usize,
    pub matches: Vec<EntryMatchRecord>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DependencyRecord {
    pub entry_id: String,
    pub entry_revision: f64,
    pub required_version_range: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostics {
    pub catalog_revision: String,
    pub tools: Vec<ToolRecord>,
    pub resolved_dependencies: Vec<DependencyRecord>,
    pub warnings: Vec<String>,
}

impl Diagnostics {
    pub fn to_json(&self) -> Json {
        let mut object = JsonObject::new();
        object.insert(
            "catalogRevision",
            Json::String(self.catalog_revision.clone()),
        );
        let tools = self
            .tools
            .iter()
            .map(|tool| {
                let mut record = JsonObject::new();
                record.insert("inputIndex", Json::Number(tool.input_index as f64));
                let matches = tool
                    .matches
                    .iter()
                    .map(|m| {
                        let mut o = JsonObject::new();
                        o.insert("entryId", Json::String(m.entry_id.clone()));
                        o.insert("entryRevision", Json::Number(m.entry_revision));
                        let identities = m
                            .matched_identities
                            .iter()
                            .map(|i| {
                                let mut io = JsonObject::new();
                                io.insert("kind", Json::String(i.kind.clone()));
                                io.insert("strength", Json::String(i.strength.as_str().into()));
                                Json::Object(io)
                            })
                            .collect();
                        o.insert("matchedIdentities", Json::Array(identities));
                        Json::Object(o)
                    })
                    .collect();
                record.insert("matches", Json::Array(matches));
                Json::Object(record)
            })
            .collect();
        object.insert("tools", Json::Array(tools));
        let dependencies = self
            .resolved_dependencies
            .iter()
            .map(|d| {
                let mut o = JsonObject::new();
                o.insert("entryId", Json::String(d.entry_id.clone()));
                o.insert("entryRevision", Json::Number(d.entry_revision));
                if let Some(range) = &d.required_version_range {
                    o.insert("requiredVersionRange", Json::String(range.clone()));
                }
                Json::Object(o)
            })
            .collect();
        object.insert("resolvedDependencies", Json::Array(dependencies));
        object.insert("warnings", strings(&self.warnings));
        Json::Object(object)
    }
}

/// Result of `resolve_sandbox_policy_with_diagnostics` (design §5.1).
#[derive(Clone, Debug, PartialEq)]
pub struct SandboxConfigResolution {
    pub policy: Option<SandboxPolicy>,
    pub diagnostics: Diagnostics,
}

impl SandboxConfigResolution {
    /// `{"policy"?, "diagnostics"}`; `policy` is omitted when absent.
    pub fn to_json(&self) -> Json {
        let mut object = JsonObject::new();
        if let Some(policy) = &self.policy {
            object.insert("policy", policy.to_json());
        }
        object.insert("diagnostics", self.diagnostics.to_json());
        Json::Object(object)
    }
}

/// Inspection metadata for one identity predicate (design §5.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogIdentityMetadata {
    Purl {
        value: String,
        version_range: Option<String>,
    },
    InvocationName {
        names: Vec<String>,
    },
}

impl CatalogIdentityMetadata {
    pub fn to_json(&self) -> Json {
        let mut o = JsonObject::new();
        match self {
            CatalogIdentityMetadata::Purl {
                value,
                version_range,
            } => {
                o.insert("kind", "purl".into());
                o.insert("value", value.as_str().into());
                if let Some(range) = version_range {
                    o.insert("versionRange", range.as_str().into());
                }
            }
            CatalogIdentityMetadata::InvocationName { names } => {
                o.insert("kind", "invocation-name".into());
                o.insert("names", strings(names));
            }
        }
        Json::Object(o)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformVariantMetadata {
    pub platform: Platform,
    pub architecture: Option<Architecture>,
    pub dependency_entry_ids: Vec<String>,
    pub sandbox_policy_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    pub method: String,
    pub source_revision: String,
}

impl Provenance {
    pub fn to_json(&self) -> Json {
        let mut o = JsonObject::new();
        o.insert("method", self.method.as_str().into());
        o.insert("sourceRevision", self.source_revision.as_str().into());
        Json::Object(o)
    }
}

/// Inspection metadata for one entry. It never exposes a policy body.
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogEntryMetadata {
    pub catalog_revision: String,
    pub entry_id: String,
    pub entry_revision: f64,
    pub display_name: String,
    pub identity: Vec<CatalogIdentityMetadata>,
    pub platform_variants: Vec<PlatformVariantMetadata>,
    pub provenance: Provenance,
}

impl CatalogEntryMetadata {
    pub fn to_json(&self) -> Json {
        let mut o = JsonObject::new();
        o.insert("catalogRevision", self.catalog_revision.as_str().into());
        o.insert("entryId", self.entry_id.as_str().into());
        o.insert("entryRevision", Json::Number(self.entry_revision));
        o.insert("displayName", self.display_name.as_str().into());
        o.insert(
            "identity",
            Json::Array(self.identity.iter().map(|i| i.to_json()).collect()),
        );
        let variants = self
            .platform_variants
            .iter()
            .map(|v| {
                let mut vo = JsonObject::new();
                vo.insert("platform", v.platform.as_str().into());
                if let Some(arch) = v.architecture {
                    vo.insert("architecture", arch.as_str().into());
                }
                vo.insert("dependencyEntryIds", strings(&v.dependency_entry_ids));
                vo.insert(
                    "sandboxPolicyVersion",
                    v.sandbox_policy_version.as_str().into(),
                );
                Json::Object(vo)
            })
            .collect();
        o.insert("platformVariants", Json::Array(variants));
        o.insert("provenance", self.provenance.to_json());
        Json::Object(o)
    }
}

/// Result of `get_catalog_info` (design §5.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogInfo {
    pub catalog_schema_version: String,
    pub catalog_revision: String,
}

impl CatalogInfo {
    pub fn to_json(&self) -> Json {
        let mut o = JsonObject::new();
        o.insert(
            "catalogSchemaVersion",
            self.catalog_schema_version.as_str().into(),
        );
        o.insert("catalogRevision", self.catalog_revision.as_str().into());
        Json::Object(o)
    }
}
