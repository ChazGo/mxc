// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Runtime lookup and inspection (design §4.3–§5.2): per input, match one
//! entry, select its version and intent, and compose every contributing
//! (tool, intent) pair and its dependencies into one candidate floor.

use crate::policy_store::catalog::{
    entry_index, CatalogEntry, Dependency, IdentityPredicate, IntentDefinition, Overlay,
    SymbolSource,
};
use crate::policy_store::compose::{component_symbols, compose_check, compose_policy, Component};
use crate::policy_store::effective::{
    has_arch_specific, materialize, select_platform_variant, Closure, IntentChoice, Node,
};
use crate::policy_store::errors::{invalid_catalog, invalid_context, ErrorReason, PolicyCatalogError, Result};
use crate::policy_store::host::{HostEnvironment, SystemHost};
use crate::policy_store::json::cmp_utf16;
use crate::policy_store::model::{
    Architecture, CatalogAdditionsMetadata, CatalogEntryMetadata, CatalogIdentityMetadata,
    CatalogInfo, CatalogIntentMetadata, DefaultMetadata, Diagnostics, EntryMatchRecord, IntentMode,
    IntentSelection, MatchedIdentity, Platform, PlatformVariantMetadata, Provenance,
    ResolveContext, SandboxConfigResolution, SandboxPolicy, ToolCandidate, ToolInputs, ToolRecord,
    ToolResolutionStatus, ToolResolutionWarning, ToolWarningCode, VersionSelection, VersionStatus,
    VersionVariantMetadata, Warning,
};
use crate::policy_store::paths::{case_key, is_absolute_path};
use crate::policy_store::purl::{parse_purl, ParsedPurl};
use crate::policy_store::store::{bundled_catalog_store, CatalogStore};
use crate::policy_store::text::replace_symbols;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

fn validate_candidate(candidate: &ToolCandidate, index: usize) -> Result<()> {
    if candidate.invocation_name.is_empty() {
        return Err(invalid_context(format!(
            "tool input {index}: invocationName must be a non-empty string"
        )));
    }
    if candidate.invocation_name.contains(['/', '\\']) {
        return Err(invalid_context(format!(
            "tool input {index}: invocationName must be a bare name, not a path"
        )));
    }
    for (key, value) in [
        ("packageUrl", &candidate.package_url),
        ("detectedVersion", &candidate.detected_version),
        ("intent", &candidate.intent),
    ] {
        if value.as_deref() == Some("") {
            return Err(invalid_context(format!(
                "tool input {index}: {key} must be a non-empty string when present"
            )));
        }
    }
    Ok(())
}

fn describe_input(index: usize, tool: &ToolCandidate) -> String {
    format!("input {index} ('{}')", tool.invocation_name)
}

fn tool_warning(
    code: ToolWarningCode,
    input_index: usize,
    entry_id: Option<&str>,
    tool: &ToolCandidate,
    message: String,
) -> Warning {
    Warning::Tool(ToolResolutionWarning {
        code,
        input_index,
        entry_id: entry_id.map(str::to_string),
        detected_version: tool.detected_version.clone(),
        intent: tool.intent.clone(),
        message,
    })
}

fn intent_metadata(list: &[(String, IntentDefinition)]) -> Vec<CatalogIntentMetadata> {
    list.iter()
        .map(|(name, definition)| CatalogIntentMetadata {
            name: name.clone(),
            example_subcommands: definition.example_subcommands.clone(),
            dependency_entry_ids: dependency_ids(&definition.dependencies),
        })
        .collect()
}

fn dependency_ids(list: &[Dependency]) -> Vec<String> {
    list.iter().map(|d| d.entry_id.clone()).collect()
}

fn additions_metadata(overlay: &Overlay) -> CatalogAdditionsMetadata {
    CatalogAdditionsMetadata {
        dependency_entry_ids: dependency_ids(&overlay.dependencies),
        intent_additions: intent_metadata(&overlay.intent_additions),
        new_intents: intent_metadata(&overlay.new_intents),
    }
}

/// The eligible entry chosen for one input.
struct Choice<'a> {
    entry: &'a CatalogEntry,
    satisfied: Vec<&'a IdentityPredicate>,
    strong: bool,
}

/// Lazily determined architecture: detected only when first needed.
struct LazyArchitecture<'h> {
    host: &'h dyn HostEnvironment,
    value: Cell<Option<Architecture>>,
}

impl LazyArchitecture<'_> {
    fn get(&self) -> Result<Architecture> {
        if let Some(value) = self.value.get() {
            return Ok(value);
        }
        let value = self.host.native_architecture()?;
        self.value.set(Some(value));
        Ok(value)
    }
}

/// Read-only view over one catalog store with an injectable host.
pub struct PolicyCatalog {
    store: Arc<CatalogStore>,
    host: Arc<dyn HostEnvironment>,
}

impl std::fmt::Debug for PolicyCatalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PolicyCatalog")
            .field("store", &self.store)
            .finish_non_exhaustive()
    }
}

impl PolicyCatalog {
    /// A catalog over `store` using the real host ([`SystemHost`]).
    pub fn new(store: Arc<CatalogStore>) -> Self {
        Self::with_host(store, Arc::new(SystemHost))
    }

    pub fn with_host(store: Arc<CatalogStore>, host: Arc<dyn HostEnvironment>) -> Self {
        Self { store, host }
    }

    pub fn store(&self) -> &CatalogStore {
        &self.store
    }

    // -----------------------------------------------------------------------
    // Setup and inspection (design §5.2)
    // -----------------------------------------------------------------------

    pub fn get_catalog_info(&self) -> Result<CatalogInfo> {
        let revision = self.store.revision(None)?;
        Ok(CatalogInfo {
            catalog_schema_version: revision.catalog_schema_version.clone(),
            catalog_revision: revision.catalog_revision.clone(),
        })
    }

    /// Metadata for every entry of the installed revision, ordered by `entryId`.
    pub fn list_catalog_entries(&self) -> Result<Vec<CatalogEntryMetadata>> {
        let revision = self.store.revision(None)?;
        let mut entries: Vec<&CatalogEntry> = revision.entries.iter().collect();
        entries.sort_by(|a, b| cmp_utf16(&a.entry_id, &b.entry_id));
        Ok(entries
            .into_iter()
            .map(|entry| CatalogEntryMetadata {
                catalog_revision: revision.catalog_revision.clone(),
                entry_id: entry.entry_id.clone(),
                entry_revision: entry.entry_revision,
                display_name: entry.display_name.clone(),
                version_scheme: entry.version_scheme,
                identity: entry
                    .identity
                    .iter()
                    .map(|predicate| match predicate {
                        IdentityPredicate::Purl { value } => CatalogIdentityMetadata::Purl {
                            value: value.clone(),
                        },
                        IdentityPredicate::InvocationName { names } => {
                            CatalogIdentityMetadata::InvocationName {
                                names: names.clone(),
                            }
                        }
                    })
                    .collect(),
                default: DefaultMetadata {
                    dependency_entry_ids: dependency_ids(&entry.default.dependencies),
                    sandbox_policy_version: entry.default.sandbox_policy.version().to_string(),
                    intents: intent_metadata(&entry.default.intents),
                },
                platform_variants: entry
                    .platform_variants
                    .iter()
                    .map(|variant| PlatformVariantMetadata {
                        platform: variant.platform,
                        architecture: variant.architecture,
                        additions: additions_metadata(&variant.overlay),
                    })
                    .collect(),
                version_variants: entry
                    .version_variants
                    .iter()
                    .map(|variant| VersionVariantMetadata {
                        version_range: variant.version_range.as_str().to_string(),
                        additions: additions_metadata(&variant.overlay),
                    })
                    .collect(),
                provenance: Provenance {
                    method: entry.provenance_method.clone(),
                    source_revision: entry.provenance_source_revision.clone(),
                },
            })
            .collect())
    }

    // -----------------------------------------------------------------------
    // Runtime lookup (design §5.1)
    // -----------------------------------------------------------------------

    /// The composed candidate policy, or `None` when no policy can be resolved.
    pub fn resolve_sandbox_policy(
        &self,
        tools: impl Into<ToolInputs>,
        ctx: &ResolveContext,
    ) -> Result<Option<SandboxPolicy>> {
        Ok(self.resolve(&tools.into(), ctx)?.policy)
    }

    /// Resolves one tool or a list of tools in one pass: the composed
    /// candidate policy with attribution and warnings. Library failures are
    /// errors, never absence. The result is a best-effort floor, not
    /// authorization.
    pub fn resolve_sandbox_policy_with_diagnostics(
        &self,
        tools: impl Into<ToolInputs>,
        ctx: &ResolveContext,
    ) -> Result<SandboxConfigResolution> {
        self.resolve(&tools.into(), ctx)
    }

    fn resolve(&self, tools: &ToolInputs, ctx: &ResolveContext) -> Result<SandboxConfigResolution> {
        let candidates: Vec<ToolCandidate> = tools.0.iter().map(|t| t.to_candidate()).collect();
        for (index, candidate) in candidates.iter().enumerate() {
            validate_candidate(candidate, index)?;
        }
        let (ctx_platform, ctx_architecture) = self.validate_context(ctx)?;
        let revision = self.store.revision(ctx.catalog_revision.as_deref())?;
        let platform = match ctx_platform {
            Some(platform) => platform,
            None => self.host.platform()?,
        };
        let allow_weak = ctx.allow_weak_identity_fallback;
        let lazy = LazyArchitecture {
            host: self.host.as_ref(),
            value: Cell::new(ctx_architecture),
        };
        let architecture = || lazy.get();

        let mut warnings: Vec<Warning> = Vec::new();
        let mut tool_records = Vec::new();
        let by_id = entry_index(&revision.entries);
        let mut ordered: Vec<&CatalogEntry> = revision.entries.iter().collect();
        ordered.sort_by(|a, b| cmp_utf16(&a.entry_id, &b.entry_id));

        let mut roots: Vec<Node<'_>> = Vec::new();
        let mut contributed: HashSet<(String, String, Option<usize>, String)> = HashSet::new();
        let mut closure = Closure::new(&by_id, platform, &architecture);

        for (input_index, tool) in candidates.iter().enumerate() {
            let purl = self.parse_input_purl(tool, input_index, &mut warnings)?;
            let choice = match self.match_tool(
                &ordered,
                tool,
                purl.as_ref(),
                input_index,
                platform,
                allow_weak,
                &architecture,
            )? {
                Ok(choice) => choice,
                Err(skipped) => {
                    let suffix = if skipped.is_empty() {
                        String::new()
                    } else {
                        format!(": {}", skipped.join("; "))
                    };
                    warnings.push(tool_warning(
                        ToolWarningCode::ToolUnmatched,
                        input_index,
                        None,
                        tool,
                        format!(
                            "{} matched no eligible catalog entry{suffix}",
                            describe_input(input_index, tool)
                        ),
                    ));
                    tool_records.push(ToolRecord {
                        input_index,
                        status: ToolResolutionStatus::ToolUnmatched,
                        matches: Vec::new(),
                    });
                    continue;
                }
            };
            let entry = choice.entry;
            if !choice.strong {
                warnings.push(Warning::Text(format!(
                    "{} matched {} only by invocation name (weak identity)",
                    describe_input(input_index, tool),
                    entry.entry_id
                )));
            }
            let matched_identities: Vec<MatchedIdentity> = choice
                .satisfied
                .iter()
                .map(|p| MatchedIdentity {
                    kind: p.kind().to_string(),
                    strength: p.strength(),
                })
                .collect();
            let mut record = EntryMatchRecord {
                entry_id: entry.entry_id.clone(),
                entry_revision: entry.entry_revision,
                matched_identities,
                version_selection: VersionSelection {
                    status: VersionStatus::MatchedDefault,
                    detected_version: tool.detected_version.clone(),
                    selected_version_range: None,
                },
                intent_selection: None,
            };

            // Version selection (design §4.3).
            let mut version_variant = None;
            if let Some(detected) = &tool.detected_version {
                match entry.version_scheme.parse_version(detected) {
                    None => {
                        record.version_selection.status = VersionStatus::VersionUnparseable;
                        warnings.push(tool_warning(
                            ToolWarningCode::VersionUnparseable,
                            input_index,
                            Some(&entry.entry_id),
                            tool,
                            format!(
                                "{}: detected version '{detected}' is not a valid {} version for {}; the input contributes nothing",
                                describe_input(input_index, tool),
                                entry.version_scheme,
                                entry.entry_id
                            ),
                        ));
                        tool_records.push(ToolRecord {
                            input_index,
                            status: ToolResolutionStatus::Version(
                                VersionStatus::VersionUnparseable,
                            ),
                            matches: vec![record],
                        });
                        continue;
                    }
                    Some(version) => {
                        version_variant = entry
                            .version_variants
                            .iter()
                            .find(|v| v.version_range.contains(&version));
                        match version_variant {
                            Some(variant) => {
                                record.version_selection.status = VersionStatus::MatchedVersion;
                                record.version_selection.selected_version_range =
                                    Some(variant.version_range.as_str().to_string());
                            }
                            None => {
                                record.version_selection.status = VersionStatus::VersionOutOfRange;
                                warnings.push(tool_warning(
                                    ToolWarningCode::VersionOutOfRange,
                                    input_index,
                                    Some(&entry.entry_id),
                                    tool,
                                    format!(
                                        "{}: detected version '{detected}' is outside every reviewed version range for {}; its unversioned default applies",
                                        describe_input(input_index, tool),
                                        entry.entry_id
                                    ),
                                ));
                            }
                        }
                    }
                }
            }

            let selection = select_platform_variant(entry, platform, &architecture)?;
            let effective = materialize(entry, selection.map(|s| s.variant), version_variant)
                .map_err(|e| invalid_catalog(format!("'{}': {e}", entry.entry_id)))?;

            // Intent selection (design §4.4).
            let intent_choice = match &tool.intent {
                Some(name) => IntentChoice::Named(name),
                None => IntentChoice::All,
            };
            let Some(selected) = effective.select(intent_choice) else {
                let requested = tool.intent.clone().unwrap_or_default();
                record.intent_selection = Some(IntentSelection {
                    requested: Some(requested.clone()),
                    mode: IntentMode::Unsupported,
                    selected: Vec::new(),
                });
                let available = effective.intent_names();
                warnings.push(tool_warning(
                    ToolWarningCode::IntentUnsupported,
                    input_index,
                    Some(&entry.entry_id),
                    tool,
                    format!(
                        "{}: intent '{requested}' is not defined for the selected policy of {} (available: {}); the input contributes nothing",
                        describe_input(input_index, tool),
                        entry.entry_id,
                        if available.is_empty() {
                            "none".to_string()
                        } else {
                            available.join(", ")
                        }
                    ),
                ));
                tool_records.push(ToolRecord {
                    input_index,
                    status: ToolResolutionStatus::IntentUnsupported,
                    matches: vec![record],
                });
                continue;
            };
            record.intent_selection = Some(IntentSelection {
                requested: tool.intent.clone(),
                mode: if tool.intent.is_some() {
                    IntentMode::Named
                } else {
                    IntentMode::All
                },
                selected: selected.intent_names.clone(),
            });
            let status = ToolResolutionStatus::Version(record.version_selection.status);
            tool_records.push(ToolRecord {
                input_index,
                status,
                matches: vec![record],
            });

            let key = (
                entry.entry_id.clone(),
                version_variant
                    .map(|v| v.version_range.as_str().to_string())
                    .unwrap_or_default(),
                selection.map(|s| s.index),
                tool.intent.clone().unwrap_or_else(|| "*".to_string()),
            );
            if contributed.insert(key) {
                roots.push(Node {
                    component: Component {
                        entry_id: &entry.entry_id,
                        base: &entry.default.sandbox_policy,
                        additions: selected.additions.clone(),
                    },
                    neutral_fallback: selection
                        .filter(|s| !s.exact && has_arch_specific(entry, platform))
                        .map(|_| platform),
                });
                closure.add_dependencies(entry, &selected.dependencies)?;
            }
        }

        let resolved_dependencies = closure.sorted_records();
        let mut nodes = roots;
        nodes.extend(closure.nodes);
        let mut diagnostics = Diagnostics {
            catalog_revision: revision.catalog_revision.clone(),
            tools: tool_records,
            resolved_dependencies,
            warnings: Vec::new(),
        };
        if nodes.is_empty() {
            diagnostics.warnings = warnings;
            return Ok(SandboxConfigResolution {
                policy: None,
                diagnostics,
            });
        }

        if ctx_architecture.is_none() {
            if let Some(native) = lazy.value.get() {
                warnings.push(Warning::Text(format!(
                    "architecture was not specified; overlays were selected for the native system architecture '{native}'; the tool's architecture was not verified"
                )));
            }
        }
        let mut reported: HashSet<&str> = HashSet::new();
        for node in &nodes {
            if let Some(platform) = node.neutral_fallback {
                if reported.insert(node.component.entry_id) {
                    warnings.push(Warning::Text(format!(
                        "{} uses its architecture-neutral {platform} additions; no {}-specific overlay exists",
                        node.component.entry_id,
                        architecture()?
                    )));
                }
            }
        }

        let components: Vec<Component<'_>> = nodes.into_iter().map(|n| n.component).collect();
        if let Err(violation) = compose_check(&components) {
            return Err(PolicyCatalogError::new(
                ErrorReason::CompositionConflict,
                format!("selected entries cannot be composed: {violation}"),
            ));
        }

        let symbols = self.resolve_symbols(&components, ctx, platform, &mut warnings)?;
        let policy = match symbols {
            None => None,
            Some(symbols) => {
                let composed = compose_policy(
                    &components,
                    &|template: &str| replace_symbols(template, |name| symbols[name].clone()),
                    platform,
                );
                warnings.extend(composed.warnings.into_iter().map(Warning::Text));
                Some(composed.policy)
            }
        };
        diagnostics.warnings = warnings;
        Ok(SandboxConfigResolution {
            policy,
            diagnostics,
        })
    }

    fn parse_input_purl(
        &self,
        tool: &ToolCandidate,
        input_index: usize,
        warnings: &mut Vec<Warning>,
    ) -> Result<Option<ParsedPurl>> {
        let Some(package_url) = &tool.package_url else {
            return Ok(None);
        };
        let Some(purl) = parse_purl(package_url) else {
            return Err(invalid_context(format!(
                "{}: '{package_url}' is not a valid package URL",
                describe_input(input_index, tool)
            )));
        };
        if let Some(version) = &purl.version {
            warnings.push(Warning::Text(format!(
                "{}: the version '{version}' embedded in packageUrl is not version evidence and was ignored; supply detectedVersion",
                describe_input(input_index, tool)
            )));
        }
        Ok(Some(purl))
    }

    /// The single most specific eligible entry (design §4.3), or the reasons
    /// candidate entries were skipped. Ties at the highest rank are errors.
    #[allow(clippy::too_many_arguments)]
    fn match_tool<'a>(
        &self,
        ordered: &[&'a CatalogEntry],
        tool: &ToolCandidate,
        purl: Option<&ParsedPurl>,
        input_index: usize,
        platform: Platform,
        allow_weak: bool,
        architecture: &dyn Fn() -> Result<Architecture>,
    ) -> Result<std::result::Result<Choice<'a>, Vec<String>>> {
        let invocation = case_key(&tool.invocation_name, platform);
        let mut ranked: Vec<((bool, bool, u8), Choice<'a>)> = Vec::new();
        let mut skipped: Vec<String> = Vec::new();
        for entry in ordered {
            let satisfied: Vec<&IdentityPredicate> = entry
                .identity
                .iter()
                .filter(|predicate| match predicate {
                    IdentityPredicate::Purl { value } => match purl {
                        Some(purl) => parse_purl(value).is_some_and(|p| p.key == purl.key),
                        None => false,
                    },
                    IdentityPredicate::InvocationName { names } => names
                        .iter()
                        .any(|name| case_key(name, platform) == invocation),
                })
                .collect();
            if satisfied.is_empty() {
                continue;
            }
            let strong = satisfied
                .iter()
                .any(|p| matches!(p, IdentityPredicate::Purl { .. }));
            if !strong && !allow_weak {
                skipped.push(format!(
                    "{} matched only by invocation name and allowWeakIdentityFallback is not enabled",
                    entry.entry_id
                ));
                continue;
            }
            let selection = select_platform_variant(entry, platform, architecture)?;
            // Intent specificity uses the overlays that apply to this input:
            // its platform selection and the version variant containing its
            // detected version, if any.
            let version_variant = tool.detected_version.as_deref().and_then(|detected| {
                let version = entry.version_scheme.parse_version(detected)?;
                entry
                    .version_variants
                    .iter()
                    .find(|v| v.version_range.contains(&version))
            });
            let intent_declared = match &tool.intent {
                None => false,
                Some(intent) => materialize(entry, selection.map(|s| s.variant), version_variant)
                    .map_err(|e| invalid_catalog(format!("'{}': {e}", entry.entry_id)))?
                    .has_intent(intent),
            };
            let arch_rank = match selection {
                Some(s) if s.exact => 2,
                Some(_) => 1,
                None => 0,
            };
            ranked.push((
                (strong, intent_declared, arch_rank),
                Choice {
                    entry,
                    satisfied,
                    strong,
                },
            ));
        }
        let Some(best) = ranked.iter().map(|(rank, _)| *rank).max() else {
            return Ok(Err(skipped));
        };
        let mut top: Vec<Choice<'a>> = ranked
            .into_iter()
            .filter(|(rank, _)| *rank == best)
            .map(|(_, choice)| choice)
            .collect();
        if top.len() > 1 {
            let ids: Vec<&str> = top.iter().map(|c| c.entry.entry_id.as_str()).collect();
            return Err(PolicyCatalogError::new(
                ErrorReason::AmbiguousMatch,
                format!(
                    "{} matches {} entries with equal rank ({}); no entry was selected",
                    describe_input(input_index, tool),
                    ids.len(),
                    ids.join(", ")
                ),
            ));
        }
        Ok(Ok(top.remove(0)))
    }

    fn validate_context(
        &self,
        ctx: &ResolveContext,
    ) -> Result<(Option<Platform>, Option<Architecture>)> {
        let platform = match &ctx.platform {
            None => None,
            Some(value) => Some(Platform::parse(value).ok_or_else(|| {
                invalid_context(format!("ResolveContext.platform '{value}' is unsupported"))
            })?),
        };
        let architecture = match &ctx.architecture {
            None => None,
            Some(value) => Some(Architecture::parse(value).ok_or_else(|| {
                invalid_context(format!(
                    "ResolveContext.architecture '{value}' is unsupported"
                ))
            })?),
        };
        if ctx.project_root.as_deref() == Some("") {
            return Err(invalid_context(
                "ResolveContext.projectRoot must be a non-empty string when present",
            ));
        }
        let contract = self.store.contract();
        for (name, _) in ctx.symbols.iter().flat_map(|s| s.iter()) {
            let Some(definition) = contract.symbol(name) else {
                return Err(invalid_context(format!(
                    "ResolveContext.symbols.{name} is not a catalog symbol"
                )));
            };
            if definition.source == SymbolSource::Context {
                return Err(invalid_context(format!(
                    "symbol '{name}' is supplied through ResolveContext.projectRoot, not symbols"
                )));
            }
        }
        Ok((platform, architecture))
    }

    /// Resolves every symbol the selected components need; `None` (with
    /// warnings) when any is unresolved. Never a partial policy.
    fn resolve_symbols(
        &self,
        components: &[Component<'_>],
        ctx: &ResolveContext,
        platform: Platform,
        warnings: &mut Vec<Warning>,
    ) -> Result<Option<HashMap<String, String>>> {
        let contract = self.store.contract();
        let mut values: HashMap<String, String> = HashMap::new();
        let mut missing: Vec<(String, Vec<String>)> = Vec::new();
        for (name, entry_ids) in component_symbols(components) {
            let definition = contract
                .symbol(&name)
                .expect("validated revisions reference declared symbols");
            let value = if definition.source == SymbolSource::Context {
                ctx.project_root.clone()
            } else {
                let supplied = ctx
                    .symbols
                    .as_ref()
                    .and_then(|s| s.get(&name))
                    .map(str::to_string);
                if supplied.is_none()
                    && definition.source == SymbolSource::Host
                    && self.host.platform().ok() == Some(platform)
                {
                    let discovered = self.host.symbol(&name);
                    if let Some(value) = &discovered {
                        warnings.push(Warning::Text(format!(
                            "symbol '{name}' resolved from the host environment to '{value}'"
                        )));
                    }
                    discovered
                } else {
                    supplied
                }
            };
            let Some(value) = value else {
                missing.push((name, entry_ids));
                continue;
            };
            if value.contains("${") || !is_absolute_path(&value, platform) {
                return Err(invalid_context(format!(
                    "symbol '{name}' must resolve to an absolute {platform} path"
                )));
            }
            values.insert(name, value);
        }
        if !missing.is_empty() {
            missing.sort_by(|a, b| cmp_utf16(&a.0, &b.0));
            for (name, entry_ids) in missing {
                let hint =
                    if contract.symbol(&name).map(|d| d.source) == Some(SymbolSource::Context) {
                        "ResolveContext.projectRoot".to_string()
                    } else {
                        format!("ResolveContext.symbols.{name}")
                    };
                warnings.push(Warning::Text(format!(
                    "required symbol '{name}' (needed by {}) is unresolved; supply {hint}; no policy was returned",
                    entry_ids.join(", ")
                )));
            }
            return Ok(None);
        }
        Ok(Some(values))
    }
}

fn bundled() -> Result<&'static PolicyCatalog> {
    static CATALOG: OnceLock<PolicyCatalog> = OnceLock::new();
    if let Some(catalog) = CATALOG.get() {
        return Ok(catalog);
    }
    let catalog = PolicyCatalog::new(bundled_catalog_store()?);
    Ok(CATALOG.get_or_init(|| catalog))
}

/// Composed candidate policy for one or several tools, from the bundled catalog.
pub fn resolve_sandbox_policy(
    tools: impl Into<ToolInputs>,
    ctx: &ResolveContext,
) -> Result<Option<SandboxPolicy>> {
    bundled()?.resolve_sandbox_policy(tools, ctx)
}

/// Composed candidate policy plus attribution, from the bundled catalog.
pub fn resolve_sandbox_policy_with_diagnostics(
    tools: impl Into<ToolInputs>,
    ctx: &ResolveContext,
) -> Result<SandboxConfigResolution> {
    bundled()?.resolve_sandbox_policy_with_diagnostics(tools, ctx)
}

/// Bundled catalog entry metadata.
pub fn list_catalog_entries() -> Result<Vec<CatalogEntryMetadata>> {
    bundled()?.list_catalog_entries()
}

/// Bundled catalog schema version and revision.
pub fn get_catalog_info() -> Result<CatalogInfo> {
    bundled()?.get_catalog_info()
}
