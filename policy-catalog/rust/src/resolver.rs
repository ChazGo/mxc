// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Runtime lookup and inspection (TypeScript `src/resolver.ts`).

use crate::catalog::{
    composition_violation, dependency_closure, entry_index, find_cross_class_overlap, policy_symbols, select_variant,
    CatalogEntry, ClosureNode, IdentityPredicate, SymbolSource, VariantSelection, COMPOSABLE_FILESYSTEM_FIELDS,
};
use crate::errors::{invalid_context, ErrorReason, PolicyCatalogError, Result};
use crate::host::{HostEnvironment, SystemHost};
use crate::json::cmp_utf16;
use crate::model::{
    Architecture, CatalogEntryMetadata, CatalogIdentityMetadata, CatalogInfo, DependencyRecord, Diagnostics,
    EntryMatchRecord, FilesystemPolicy, MatchedIdentity, Platform, PlatformVariantMetadata, Provenance, ResolveContext,
    SandboxConfigResolution, SandboxPolicy, ToolCandidate, ToolInputs, ToolRecord,
};
use crate::paths::{case_key, is_absolute_path, normalize_path, path_key_segments};
use crate::purl::{parse_purl, ParsedPurl};
use crate::store::{bundled_catalog_store, CatalogStore};
use crate::text::replace_symbols;
use crate::version_range::satisfies_version_range;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

struct EntryMatch<'a> {
    entry: &'a CatalogEntry,
    selection: VariantSelection<'a>,
    satisfied: Vec<&'a IdentityPredicate>,
}

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
                identity: entry
                    .identity
                    .iter()
                    .map(|predicate| match predicate {
                        IdentityPredicate::Purl { value, version_range } => CatalogIdentityMetadata::Purl {
                            value: value.clone(),
                            version_range: version_range.clone(),
                        },
                        IdentityPredicate::InvocationName { names } => {
                            CatalogIdentityMetadata::InvocationName { names: names.clone() }
                        }
                    })
                    .collect(),
                platform_variants: entry
                    .platform_variants
                    .iter()
                    .map(|variant| PlatformVariantMetadata {
                        platform: variant.platform,
                        architecture: variant.architecture,
                        dependency_entry_ids: variant
                            .dependencies
                            .iter()
                            .flatten()
                            .map(|d| d.entry_id.clone())
                            .collect(),
                        sandbox_policy_version: variant.sandbox_policy.version().to_string(),
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
    /// errors, never absence. The result is a candidate lower bound, not
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
        let architecture = LazyArchitecture {
            host: self.host.as_ref(),
            value: Cell::new(ctx_architecture),
        };

        let mut warnings: Vec<String> = Vec::new();
        let mut tool_records = Vec::new();
        let mut selected: Vec<ClosureNode<'_>> = Vec::new();
        let mut selected_ids: HashSet<&str> = HashSet::new();
        let by_id = entry_index(&revision.entries);
        let mut ordered: Vec<&CatalogEntry> = revision.entries.iter().collect();
        ordered.sort_by(|a, b| cmp_utf16(&a.entry_id, &b.entry_id));

        for (input_index, tool) in candidates.iter().enumerate() {
            let matches = self.match_tool(
                &ordered,
                tool,
                input_index,
                platform,
                allow_weak,
                &architecture,
                &mut warnings,
            )?;
            tool_records.push(ToolRecord {
                input_index,
                matches: matches
                    .iter()
                    .map(|m| EntryMatchRecord {
                        entry_id: m.entry.entry_id.clone(),
                        entry_revision: m.entry.entry_revision,
                        matched_identities: m
                            .satisfied
                            .iter()
                            .map(|p| MatchedIdentity {
                                kind: p.kind().to_string(),
                                strength: p.strength(),
                            })
                            .collect(),
                    })
                    .collect(),
            });
            if matches.len() > 1 {
                let ids: Vec<&str> = matches.iter().map(|m| m.entry.entry_id.as_str()).collect();
                warnings.push(format!(
                    "{} matched {} entries ({}); all contribute",
                    describe_input(input_index, tool),
                    matches.len(),
                    ids.join(", ")
                ));
            }
            for m in &matches {
                let nodes =
                    dependency_closure(m.entry, m.selection, &by_id, platform, architecture.get()?).map_err(|f| {
                        PolicyCatalogError::new(
                            ErrorReason::InvalidCatalog,
                            format!("dependency resolution failed: {} ({})", f.reason, f.detail),
                        )
                    })?;
                for node in nodes {
                    if selected_ids.insert(node.entry.entry_id.as_str()) {
                        selected.push(node);
                    }
                }
            }
        }

        let mut diagnostics = Diagnostics {
            catalog_revision: revision.catalog_revision.clone(),
            tools: tool_records,
            resolved_dependencies: dependency_records(&selected, &by_id),
            warnings: Vec::new(),
        };
        if selected.is_empty() {
            diagnostics.warnings = warnings;
            return Ok(SandboxConfigResolution {
                policy: None,
                diagnostics,
            });
        }

        if ctx_architecture.is_none() {
            warnings.push(format!(
                "architecture was not specified; variants were selected for the native system architecture '{}'; the tool's architecture was not verified",
                architecture.get()?
            ));
        }
        for node in &selected {
            if !node.exact {
                warnings.push(format!(
                    "{} uses its architecture-neutral {platform} variant; no {}-specific variant exists",
                    node.entry.entry_id,
                    architecture.get()?
                ));
            }
        }

        if let Some(violation) = composition_violation(&selected) {
            return Err(PolicyCatalogError::new(
                ErrorReason::CompositionConflict,
                format!("selected entries cannot be composed: {violation}"),
            ));
        }

        let symbols = self.resolve_symbols(&selected, ctx, platform, &mut warnings)?;
        let policy = match symbols {
            None => None,
            Some(symbols) => Some(compose_policy(&selected, &symbols, platform)?),
        };
        diagnostics.warnings = warnings;
        Ok(SandboxConfigResolution { policy, diagnostics })
    }

    #[allow(clippy::too_many_arguments)]
    fn match_tool<'a>(
        &self,
        ordered: &[&'a CatalogEntry],
        tool: &ToolCandidate,
        input_index: usize,
        platform: Platform,
        allow_weak: bool,
        architecture: &LazyArchitecture<'_>,
        warnings: &mut Vec<String>,
    ) -> Result<Vec<EntryMatch<'a>>> {
        let mut purl: Option<ParsedPurl> = None;
        if let Some(package_url) = &tool.package_url {
            purl = parse_purl(package_url);
            if purl.is_none() {
                return Err(invalid_context(format!(
                    "{}: '{package_url}' is not a valid package URL",
                    describe_input(input_index, tool)
                )));
            }
        }
        let invocation = case_key(&tool.invocation_name, platform);

        let mut matches = Vec::new();
        let mut skipped: Vec<String> = Vec::new();
        for entry in ordered {
            let satisfied: Vec<&IdentityPredicate> = entry
                .identity
                .iter()
                .filter(|predicate| match predicate {
                    IdentityPredicate::Purl { value, .. } => match &purl {
                        Some(purl) => parse_purl(value).is_some_and(|p| p.key == purl.key),
                        None => false,
                    },
                    IdentityPredicate::InvocationName { names } => {
                        names.iter().any(|name| case_key(name, platform) == invocation)
                    }
                })
                .collect();
            if satisfied.is_empty() {
                continue;
            }
            let strong = satisfied.iter().any(|p| matches!(p, IdentityPredicate::Purl { .. }));
            if !strong && !allow_weak {
                skipped.push(format!(
                    "{} matched only by invocation name and allowWeakIdentityFallback is not enabled",
                    entry.entry_id
                ));
                continue;
            }
            let Some(selection) = select_variant(entry, platform, architecture.get()?) else {
                skipped.push(format!(
                    "{} has no variant for {platform}/{}",
                    entry.entry_id,
                    architecture.get()?
                ));
                continue;
            };
            for predicate in &satisfied {
                let IdentityPredicate::Purl {
                    version_range: Some(range),
                    ..
                } = predicate
                else {
                    continue;
                };
                let evidence = tool
                    .detected_version
                    .clone()
                    .or_else(|| purl.as_ref().and_then(|p| p.version.clone()));
                let Some(evidence) = evidence else {
                    continue;
                };
                let in_range = satisfies_version_range(&evidence, range);
                if in_range != Some(true) {
                    warnings.push(format!(
                        "{}: detected version '{evidence}' {} the reviewed range '{range}' for {}",
                        describe_input(input_index, tool),
                        if in_range == Some(false) {
                            "is outside"
                        } else {
                            "could not be compared with"
                        },
                        entry.entry_id
                    ));
                }
            }
            if !strong {
                warnings.push(format!(
                    "{} matched {} only by invocation name (weak identity)",
                    describe_input(input_index, tool),
                    entry.entry_id
                ));
            }
            matches.push(EntryMatch {
                entry,
                selection,
                satisfied,
            });
        }
        if matches.is_empty() {
            let suffix = if skipped.is_empty() {
                String::new()
            } else {
                format!(": {}", skipped.join("; "))
            };
            warnings.push(format!(
                "{} matched no eligible catalog entry{suffix}",
                describe_input(input_index, tool)
            ));
        }
        Ok(matches)
    }

    fn validate_context(&self, ctx: &ResolveContext) -> Result<(Option<Platform>, Option<Architecture>)> {
        let platform = match &ctx.platform {
            None => None,
            Some(value) => Some(
                Platform::parse(value)
                    .ok_or_else(|| invalid_context(format!("ResolveContext.platform '{value}' is unsupported")))?,
            ),
        };
        let architecture = match &ctx.architecture {
            None => None,
            Some(value) => Some(
                Architecture::parse(value)
                    .ok_or_else(|| invalid_context(format!("ResolveContext.architecture '{value}' is unsupported")))?,
            ),
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

    /// Resolves every symbol the selected entries need; `None` (with
    /// warnings) when any is unresolved. Never a partial policy.
    fn resolve_symbols(
        &self,
        nodes: &[ClosureNode<'_>],
        ctx: &ResolveContext,
        platform: Platform,
        warnings: &mut Vec<String>,
    ) -> Result<Option<HashMap<String, String>>> {
        let contract = self.store.contract();
        let mut values: HashMap<String, String> = HashMap::new();
        let mut missing: Vec<(String, Vec<String>)> = Vec::new();
        for node in nodes {
            for name in policy_symbols(&node.variant.sandbox_policy) {
                if values.contains_key(&name) {
                    continue;
                }
                let definition = contract
                    .symbol(&name)
                    .expect("validated revisions reference declared symbols");
                let value = if definition.source == SymbolSource::Context {
                    ctx.project_root.clone()
                } else {
                    let mut value = ctx.symbols.as_ref().and_then(|s| s.get(&name)).map(str::to_string);
                    if value.is_none()
                        && definition.source == SymbolSource::Host
                        && self.host.platform().ok() == Some(platform)
                    {
                        value = self.host.symbol(&name);
                    }
                    value
                };
                let Some(value) = value else {
                    match missing.iter_mut().find(|(n, _)| *n == name) {
                        Some((_, ids)) => ids.push(node.entry.entry_id.clone()),
                        None => missing.push((name, vec![node.entry.entry_id.clone()])),
                    }
                    continue;
                };
                if value.contains("${") || !is_absolute_path(&value, platform) {
                    return Err(invalid_context(format!(
                        "symbol '{name}' must resolve to an absolute {platform} path"
                    )));
                }
                values.insert(name, value);
            }
        }
        if !missing.is_empty() {
            missing.sort_by(|a, b| cmp_utf16(&a.0, &b.0));
            for (name, entry_ids) in missing {
                let hint = if contract.symbol(&name).map(|d| d.source) == Some(SymbolSource::Context) {
                    "ResolveContext.projectRoot".to_string()
                } else {
                    format!("ResolveContext.symbols.{name}")
                };
                warnings.push(format!(
                    "required symbol '{name}' (needed by {}) is unresolved; supply {hint}; no policy was returned",
                    entry_ids.join(", ")
                ));
            }
            return Ok(None);
        }
        Ok(Some(values))
    }
}

/// Distinct dependency edges among the selected entries, ordered by
/// entryId, entryRevision, then requiredVersionRange (absent first).
fn dependency_records(nodes: &[ClosureNode<'_>], by_id: &HashMap<&str, &CatalogEntry>) -> Vec<DependencyRecord> {
    let mut records: Vec<DependencyRecord> = Vec::new();
    for node in nodes {
        for dependency in node.variant.dependencies.iter().flatten() {
            let target = by_id[dependency.entry_id.as_str()];
            let record = DependencyRecord {
                entry_id: target.entry_id.clone(),
                entry_revision: target.entry_revision,
                required_version_range: dependency.version_range.clone(),
            };
            if !records.contains(&record) {
                records.push(record);
            }
        }
    }
    records.sort_by(|a, b| {
        cmp_utf16(&a.entry_id, &b.entry_id)
            .then(
                a.entry_revision
                    .partial_cmp(&b.entry_revision)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then_with(|| match (&a.required_version_range, &b.required_version_range) {
                (None, None) => std::cmp::Ordering::Equal,
                (None, Some(_)) => std::cmp::Ordering::Less,
                (Some(_), None) => std::cmp::Ordering::Greater,
                (Some(x), Some(y)) => cmp_utf16(x, y),
            })
    });
    records
}

/// Composes the v1 vocabulary (design §4.5).
fn compose_policy(
    nodes: &[ClosureNode<'_>],
    symbols: &HashMap<String, String>,
    platform: Platform,
) -> Result<SandboxPolicy> {
    let mut classes: Vec<(&str, Vec<String>)> = Vec::new();
    for field in COMPOSABLE_FILESYSTEM_FIELDS {
        let mut seen: HashSet<Vec<String>> = HashSet::new();
        let mut out = Vec::new();
        for node in nodes {
            for template in node.variant.sandbox_policy.filesystem_field(field) {
                let substituted = replace_symbols(template, |name| symbols[name].clone());
                let resolved = normalize_path(&substituted, platform);
                if seen.insert(path_key_segments(&resolved, platform)) {
                    out.push(resolved);
                }
            }
        }
        classes.push((field, out));
    }
    if let Some(overlap) = find_cross_class_overlap(&classes, platform) {
        return Err(PolicyCatalogError::new(
            ErrorReason::CompositionConflict,
            format!("resolved paths overlap across access classes: {overlap}"),
        ));
    }

    let root = &nodes[0].variant.sandbox_policy;
    let filesystem = nodes
        .iter()
        .any(|n| n.variant.sandbox_policy.has_filesystem())
        .then(|| {
            let pick = |index: usize| Some(classes[index].1.clone()).filter(|v| !v.is_empty());
            FilesystemPolicy {
                denied_paths: pick(0),
                readonly_paths: pick(1),
                readwrite_paths: pick(2),
            }
        });
    Ok(SandboxPolicy {
        version: root.version().to_string(),
        filesystem,
        network: root.field("network").cloned(),
        ui: root.field("ui").cloned(),
        timeout_ms: root.field("timeoutMs").and_then(|v| v.as_f64()),
    })
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
pub fn resolve_sandbox_policy(tools: impl Into<ToolInputs>, ctx: &ResolveContext) -> Result<Option<SandboxPolicy>> {
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
