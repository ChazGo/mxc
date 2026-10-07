// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Runtime lookup and inspection (design §4.3–§4.5; API spec §1, §3): per input, match one
//! entry, select its version and intent, and compose every contributing
//! (tool, intent) pair and its dependencies into one candidate floor.

use crate::policy_store::catalog::{
    entry_index, CatalogEntry, Dependency, IdentityPredicate, IntentDefinition, Overlay,
    SymbolSource,
};
use crate::policy_store::compose::{
    compose, compose_check, contribution_symbols, Contribution, IdentityOracle,
};
use crate::policy_store::effective::{
    has_arch_specific, materialize, select_platform_variant, Closure, IntentChoice, LayerSet,
    PlatformSelection,
};
use crate::policy_store::errors::{
    invalid_catalog, invalid_context, ErrorReason, PolicyCatalogError, Result,
};
use crate::policy_store::exact::validate_exact;
use crate::policy_store::host::{object_within, HostEnvironment, ObjectRelation, SystemHost};
use crate::policy_store::json::cmp_utf16;
use crate::policy_store::model::{
    Architecture, ArchitectureFallback, CatalogAdditionsMetadata, CatalogEntryMetadata,
    CatalogIdentityMetadata, CatalogInfo, CatalogIntentMetadata, DefaultMetadata,
    DetailWarningKind, Diagnostics, EntryMatchRecord, IntentMode, IntentSelection, MatchedIdentity,
    Platform, PlatformVariantMetadata, Provenance, PurlComponent, Requirements,
    RequirementsResolution, ResolutionDetailWarning, ResolveContext, SymbolValueSource,
    ToolCandidate, ToolInputs, ToolRecord, ToolResolutionStatus, ToolResolutionWarning,
    ToolWarningKind, VersionSelection, VersionStatus, VersionVariantMetadata, Warning,
};
use crate::policy_store::paths::{case_key, is_absolute_path, normalize_path};
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

fn tool_warning(input_index: usize, kind: ToolWarningKind, message: String) -> Warning {
    Warning::Tool(ToolResolutionWarning {
        input_index,
        kind,
        message,
    })
}

fn union_into(target: &mut Vec<usize>, extra: &[usize]) {
    target.extend_from_slice(extra);
    target.sort_unstable();
    target.dedup();
}

fn detail_warning(
    mut input_indexes: Vec<usize>,
    mut entry_ids: Vec<String>,
    kind: DetailWarningKind,
    message: String,
) -> Warning {
    union_into(&mut input_indexes, &[]);
    entry_ids.sort_by(|a, b| cmp_utf16(a, b));
    entry_ids.dedup();
    Warning::Detail(ResolutionDetailWarning {
        input_indexes,
        entry_ids,
        kind,
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
    // Setup and inspection (API spec §1)
    // -----------------------------------------------------------------------

    pub fn get_catalog_info(&self) -> Result<CatalogInfo> {
        let revision = self.store.revision(None)?;
        Ok(CatalogInfo {
            catalog_schema_version: revision.catalog_schema_version.clone(),
            catalog_revision: revision.catalog_revision.clone(),
            sdk_contract_version: revision.sdk_contract_version.clone(),
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
    // Runtime lookup (API spec §1, §3)
    // -----------------------------------------------------------------------

    /// The composed requirements, or `None` when none can be resolved.
    pub fn resolve_requirements(
        &self,
        tools: impl Into<ToolInputs>,
        ctx: &ResolveContext,
    ) -> Result<Option<Requirements>> {
        Ok(self.resolve(&tools.into(), ctx)?.requirements)
    }

    /// Resolves one tool or a list of tools in one pass: the composed
    /// requirements with attribution and warnings. Library failures are
    /// errors, never absence. The result is a best-effort floor, not
    /// authorization.
    pub fn resolve_requirements_with_diagnostics(
        &self,
        tools: impl Into<ToolInputs>,
        ctx: &ResolveContext,
    ) -> Result<RequirementsResolution> {
        self.resolve(&tools.into(), ctx)
    }

    fn resolve(&self, tools: &ToolInputs, ctx: &ResolveContext) -> Result<RequirementsResolution> {
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

        let mut set = LayerSet::default();
        let mut closure = Closure::new(&by_id, platform, &architecture);
        // Root entries with their platform selection and requesting inputs.
        let mut roots: Vec<(&CatalogEntry, Option<PlatformSelection<'_>>, Vec<usize>)> = Vec::new();

        for (input_index, tool) in candidates.iter().enumerate() {
            let unmatched = |records: &mut Vec<ToolRecord>| {
                records.push(ToolRecord {
                    input_index,
                    status: ToolResolutionStatus::ToolUnmatched,
                    matches: Vec::new(),
                });
            };
            // Identity (design §4.3): an invalid candidate PURL leaves the
            // pair unmatched, with no invocation-name retry.
            let purl = match &tool.package_url {
                None => None,
                Some(package_url) => match parse_purl(package_url) {
                    None => {
                        warnings.push(tool_warning(
                            input_index,
                            ToolWarningKind::PurlInvalid {
                                package_url: package_url.clone(),
                            },
                            format!(
                                "{}: '{package_url}' is not a valid package URL; the input was not matched by invocation name either",
                                describe_input(input_index, tool)
                            ),
                        ));
                        unmatched(&mut tool_records);
                        continue;
                    }
                    Some(purl) => {
                        let mut ignored = Vec::new();
                        if purl.version.is_some() {
                            ignored.push(PurlComponent::Version);
                        }
                        if purl.has_qualifiers {
                            ignored.push(PurlComponent::Qualifiers);
                        }
                        if purl.has_subpath {
                            ignored.push(PurlComponent::Subpath);
                        }
                        if !ignored.is_empty() {
                            let names: Vec<&str> = ignored.iter().map(|c| c.as_str()).collect();
                            warnings.push(tool_warning(
                                input_index,
                                ToolWarningKind::PurlComponentsIgnored {
                                    package_url: package_url.clone(),
                                    ignored_components: ignored,
                                },
                                format!(
                                    "{}: packageUrl {} ignored for matching; it is not version evidence (supply detectedVersion)",
                                    describe_input(input_index, tool),
                                    names.join(", ")
                                ),
                            ));
                        }
                        Some(purl)
                    }
                },
            };
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
                        input_index,
                        ToolWarningKind::ToolUnmatched {
                            invocation_name: tool.invocation_name.clone(),
                        },
                        format!(
                            "{} matched no eligible catalog entry{suffix}",
                            describe_input(input_index, tool)
                        ),
                    ));
                    unmatched(&mut tool_records);
                    continue;
                }
            };
            let entry = choice.entry;
            if !choice.strong {
                warnings.push(tool_warning(
                    input_index,
                    ToolWarningKind::WeakIdentity {
                        entry_id: entry.entry_id.clone(),
                        invocation_name: tool.invocation_name.clone(),
                    },
                    format!(
                        "{} matched {} only by invocation name (weak identity)",
                        describe_input(input_index, tool),
                        entry.entry_id
                    ),
                ));
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
                            input_index,
                            ToolWarningKind::VersionUnparseable {
                                entry_id: entry.entry_id.clone(),
                                detected_version: detected.clone(),
                            },
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
                                    input_index,
                                    ToolWarningKind::VersionOutOfRange {
                                        entry_id: entry.entry_id.clone(),
                                        detected_version: detected.clone(),
                                    },
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
                    input_index,
                    ToolWarningKind::IntentUnsupported {
                        entry_id: entry.entry_id.clone(),
                        intent: requested.clone(),
                    },
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
            match roots.iter_mut().find(|(e, _, _)| std::ptr::eq(*e, entry)) {
                Some((_, _, owners)) => owners.push(input_index),
                None => roots.push((entry, selection, vec![input_index])),
            }
            closure.add_layers(entry, &selected.layers, &[input_index], &mut set)?;
        }

        let mut diagnostics = Diagnostics {
            catalog_revision: revision.catalog_revision.clone(),
            tools: tool_records,
            resolved_dependencies: Vec::new(),
            warnings: Vec::new(),
        };
        if set.is_empty() {
            diagnostics.warnings = warnings;
            return Ok(RequirementsResolution {
                requirements: None,
                diagnostics,
            });
        }

        // Architecture diagnostics (design §4.2): every entry whose platform
        // has architecture-specific overlays consulted the architecture.
        let mut consulted: Vec<(&CatalogEntry, Option<PlatformSelection<'_>>, Vec<usize>)> =
            Vec::new();
        for (entry, selection, owners) in roots
            .iter()
            .cloned()
            .chain(closure.selections.iter().cloned())
        {
            if !has_arch_specific(entry, platform) {
                continue;
            }
            match consulted
                .iter_mut()
                .find(|(e, _, _)| std::ptr::eq(*e, entry))
            {
                Some((_, _, existing)) => union_into(existing, &owners),
                None => {
                    let mut owners = owners;
                    union_into(&mut owners, &[]);
                    consulted.push((entry, selection, owners));
                }
            }
        }
        consulted.sort_by(|a, b| cmp_utf16(&a.0.entry_id, &b.0.entry_id));
        if ctx_architecture.is_none() && !consulted.is_empty() {
            if let Some(native) = lazy.value.get() {
                let mut owners = Vec::new();
                for (_, _, o) in &consulted {
                    union_into(&mut owners, o);
                }
                warnings.push(detail_warning(
                    owners,
                    consulted.iter().map(|(e, _, _)| e.entry_id.clone()).collect(),
                    DetailWarningKind::ArchitectureDefault {
                        platform,
                        architecture: native,
                    },
                    format!(
                        "architecture was not specified; overlays were selected for the native system architecture '{native}'; the tool's architecture was not verified"
                    ),
                ));
            }
        }
        for (entry, selection, owners) in &consulted {
            if selection.is_some_and(|s| s.exact) {
                continue;
            }
            let arch = architecture()?;
            let selected = if selection.is_some() {
                ArchitectureFallback::Platform
            } else {
                ArchitectureFallback::Default
            };
            warnings.push(detail_warning(
                owners.clone(),
                vec![entry.entry_id.clone()],
                DetailWarningKind::ArchitectureFallback {
                    platform,
                    architecture: arch,
                    selected,
                },
                format!(
                    "{} has no {arch}-specific {platform} overlay; its architecture-neutral {} data was used",
                    entry.entry_id,
                    match selected {
                        ArchitectureFallback::Platform => "platform",
                        ArchitectureFallback::Default => "default",
                    }
                ),
            ));
        }

        let excluded_none = HashSet::new();
        if let Err(violation) = compose_check(&set.contributions(&excluded_none)) {
            return Err(PolicyCatalogError::new(
                ErrorReason::CompositionConflict,
                format!("selected entries cannot be composed: {violation}"),
            ));
        }

        let Some(symbols) = self.resolve_symbols(
            &set.contributions(&excluded_none),
            ctx,
            platform,
            &mut warnings,
        )?
        else {
            diagnostics.resolved_dependencies = closure.sorted_records();
            diagnostics.warnings = warnings;
            return Ok(RequirementsResolution {
                requirements: None,
                diagnostics,
            });
        };
        let resolve = |template: &str| replace_symbols(template, |name| symbols[name].clone());

        // Filesystem object identity (design §4.5): a relation whose identity
        // is unknown fails closed for every pair that owns either side; the
        // remaining layers are composed again.
        let identity = HostIdentity {
            host: self.host.as_ref(),
            local: self.host.platform().ok() == Some(platform),
        };
        let mut excluded: HashSet<usize> = HashSet::new();
        let composed = loop {
            let contributions = set.contributions(&excluded);
            if contributions.is_empty() {
                break None;
            }
            let composed = compose(&contributions, &resolve, platform, &identity);
            if composed.identity_failures.is_empty() {
                break Some(composed);
            }
            for failure in &composed.identity_failures {
                excluded.extend(failure.owners.iter().copied());
                warnings.push(detail_warning(
                    failure.owners.clone(),
                    failure.entry_ids.clone(),
                    DetailWarningKind::FilesystemIdentityUnresolved {
                        paths: failure.paths.clone(),
                        platform,
                    },
                    format!(
                        "filesystem object identity of {} could not be established on {platform}; the requesting tools contribute nothing",
                        failure
                            .paths
                            .iter()
                            .map(|p| format!("'{p}'"))
                            .collect::<Vec<_>>()
                            .join(" and ")
                    ),
                ));
            }
        };
        for record in &mut diagnostics.tools {
            if excluded.contains(&record.input_index) {
                record.status = ToolResolutionStatus::FilesystemIdentityUnresolved;
            }
        }
        diagnostics.resolved_dependencies = closure
            .sorted_records()
            .into_iter()
            .filter_map(|mut record| {
                record.input_indexes.retain(|i| !excluded.contains(i));
                (!record.input_indexes.is_empty()).then_some(record)
            })
            .collect();
        let requirements = match composed {
            None => None,
            Some(composed) => {
                warnings.extend(composed.warnings);
                if let Err(problem) = validate_exact(&composed.requirements) {
                    return Err(PolicyCatalogError::new(
                        ErrorReason::CompositionConflict,
                        format!("the composed requirements fail SDK target validation: {problem}"),
                    ));
                }
                Some(composed.requirements)
            }
        };
        diagnostics.warnings = warnings;
        Ok(RequirementsResolution {
            requirements,
            diagnostics,
        })
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

    /// Resolves every symbol the contributions need (design §4.2 precedence:
    /// caller values, then host-local discovery, then the contract's platform
    /// default); `None` (with warnings) when any is unresolved. Never a
    /// partial result.
    fn resolve_symbols(
        &self,
        contributions: &[Contribution<'_>],
        ctx: &ResolveContext,
        platform: Platform,
        warnings: &mut Vec<Warning>,
    ) -> Result<Option<HashMap<String, String>>> {
        let contract = self.store.contract();
        let on_host = self.host.platform().ok() == Some(platform);
        let supplied = |name: &str| {
            ctx.symbols
                .as_ref()
                .and_then(|s| s.get(name))
                .map(str::to_string)
        };
        let mut values: HashMap<String, String> = HashMap::new();
        let mut missing: Vec<(String, Vec<String>, Vec<usize>)> = Vec::new();
        for (name, entry_ids, owners) in contribution_symbols(contributions) {
            let definition = contract
                .symbol(&name)
                .expect("validated revisions reference declared symbols");
            let mut value: Option<(String, SymbolValueSource)> =
                if definition.source == SymbolSource::Context {
                    ctx.project_root
                        .clone()
                        .map(|v| (v, SymbolValueSource::Caller))
                } else {
                    supplied(&name).map(|v| (v, SymbolValueSource::Caller))
                };
            if value.is_none() && on_host {
                value = match definition.source {
                    SymbolSource::Host => self
                        .host
                        .symbol(&name)
                        .map(|v| (v, SymbolValueSource::Host)),
                    SymbolSource::Caller => self
                        .host
                        .discover(&name)?
                        .map(|v| (v, SymbolValueSource::Discovery)),
                    SymbolSource::Context => None,
                };
            }
            if value.is_none() {
                if let Some(template) = definition.default_for(platform) {
                    let mut complete = true;
                    let expanded = replace_symbols(template, |inner| {
                        match supplied(inner)
                            .or_else(|| on_host.then(|| self.host.symbol(inner)).flatten())
                        {
                            Some(v) => v,
                            None => {
                                complete = false;
                                String::new()
                            }
                        }
                    });
                    if complete {
                        value = Some((
                            normalize_path(&expanded, platform),
                            SymbolValueSource::Default,
                        ));
                    }
                }
            }
            let Some((value, source)) = value else {
                missing.push((name, entry_ids, owners));
                continue;
            };
            if value.contains("${") || !is_absolute_path(&value, platform) {
                return Err(invalid_context(format!(
                    "symbol '{name}' must resolve to an absolute {platform} path (from {} value '{value}')",
                    source.as_str()
                )));
            }
            if source != SymbolValueSource::Caller {
                warnings.push(detail_warning(
                    owners,
                    entry_ids,
                    DetailWarningKind::SymbolResolved {
                        symbol: name.clone(),
                        value: value.clone(),
                        source,
                    },
                    format!(
                        "symbol '{name}' resolved to '{value}' from {}",
                        match source {
                            SymbolValueSource::Discovery => "host-local discovery",
                            SymbolValueSource::Host => "the host environment",
                            SymbolValueSource::Default => "the contract's platform default",
                            SymbolValueSource::Caller => "the caller",
                        }
                    ),
                ));
            }
            values.insert(name, value);
        }
        if !missing.is_empty() {
            missing.sort_by(|a, b| cmp_utf16(&a.0, &b.0));
            for (name, entry_ids, owners) in missing {
                let hint =
                    if contract.symbol(&name).map(|d| d.source) == Some(SymbolSource::Context) {
                        "ResolveContext.projectRoot".to_string()
                    } else {
                        format!("ResolveContext.symbols.{name}")
                    };
                let message = format!(
                    "required symbol '{name}' (needed by {}) is unresolved; supply {hint}; no requirements were returned",
                    entry_ids.join(", ")
                );
                warnings.push(detail_warning(
                    owners,
                    entry_ids,
                    DetailWarningKind::SymbolUnresolved { symbol: name },
                    message,
                ));
            }
            return Ok(None);
        }
        Ok(Some(values))
    }
}

/// Host object identity, available only when the lookup targets the host's
/// own platform; other targets fail closed.
struct HostIdentity<'h> {
    host: &'h dyn HostEnvironment,
    local: bool,
}

impl IdentityOracle for HostIdentity<'_> {
    fn within(&self, inner: &str, outer: &str) -> ObjectRelation {
        if self.local {
            object_within(self.host, inner, outer)
        } else {
            ObjectRelation::Unknown
        }
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

/// Composed requirements for one or several tools, from the bundled catalog.
pub fn resolve_requirements(
    tools: impl Into<ToolInputs>,
    ctx: &ResolveContext,
) -> Result<Option<Requirements>> {
    bundled()?.resolve_requirements(tools, ctx)
}

/// Composed requirements plus attribution, from the bundled catalog.
pub fn resolve_requirements_with_diagnostics(
    tools: impl Into<ToolInputs>,
    ctx: &ResolveContext,
) -> Result<RequirementsResolution> {
    bundled()?.resolve_requirements_with_diagnostics(tools, ctx)
}

/// Bundled catalog entry metadata.
pub fn list_catalog_entries() -> Result<Vec<CatalogEntryMetadata>> {
    bundled()?.list_catalog_entries()
}

/// Bundled catalog schema version, revision, and SDK contract version.
pub fn get_catalog_info() -> Result<CatalogInfo> {
    bundled()?.get_catalog_info()
}
