// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Effective policies (design §4.3–§4.4): the default plus the selected
//! platform overlay, at most one version variant, and the selected intents;
//! dependency closure; and the build-time materialization of every effective
//! policy the catalog can produce.

use crate::policy_store::catalog::{
    fail, validate_sandbox_policy, Additions, CatalogContract, CatalogEntry, Dependency,
    EntryIndex, IntentDefinition, Overlay, PlatformVariant, VersionVariant,
};
use crate::policy_store::compose::{compose_check, compose_policy, Component};
use crate::policy_store::errors::{invalid_catalog, Result};
use crate::policy_store::json::{cmp_utf16, Json};
use crate::policy_store::model::{
    Architecture, DependencyRecord, IntentMode, IntentSelection, Platform, SandboxPolicy,
    VersionSelection,
};
use std::collections::{HashMap, HashSet};

/// The platform overlay selected for an entry.
#[derive(Clone, Copy, Debug)]
pub struct PlatformSelection<'a> {
    pub index: usize,
    pub variant: &'a PlatformVariant,
    /// False for the architecture-neutral fallback.
    pub exact: bool,
}

/// Exact architecture first, then the platform's neutral overlay, else none
/// (the common default). Another architecture's overlay is never selected.
/// `architecture` is consulted only when the platform has
/// architecture-specific overlays.
pub fn select_platform_variant<'a>(
    entry: &'a CatalogEntry,
    platform: Platform,
    architecture: &dyn Fn() -> Result<Architecture>,
) -> Result<Option<PlatformSelection<'a>>> {
    let candidates: Vec<(usize, &PlatformVariant)> = entry
        .platform_variants
        .iter()
        .enumerate()
        .filter(|(_, v)| v.platform == platform)
        .collect();
    if candidates.iter().any(|(_, v)| v.architecture.is_some()) {
        let arch = architecture()?;
        if let Some((index, variant)) = candidates
            .iter()
            .find(|(_, v)| v.architecture == Some(arch))
        {
            return Ok(Some(PlatformSelection {
                index: *index,
                variant,
                exact: true,
            }));
        }
    }
    Ok(candidates
        .iter()
        .find(|(_, v)| v.architecture.is_none())
        .map(|(index, variant)| PlatformSelection {
            index: *index,
            variant,
            exact: false,
        }))
}

/// Whether the platform has architecture-specific overlays (so a neutral
/// selection is a fallback worth reporting).
pub fn has_arch_specific(entry: &CatalogEntry, platform: Platform) -> bool {
    entry
        .platform_variants
        .iter()
        .any(|v| v.platform == platform && v.architecture.is_some())
}

#[derive(Clone, Debug)]
pub struct EffectiveIntent<'a> {
    pub name: String,
    pub additions: Vec<&'a Additions>,
    pub dependencies: Vec<&'a Dependency>,
}

/// One effective policy before intent selection.
#[derive(Clone, Debug)]
pub struct Effective<'a> {
    pub entry: &'a CatalogEntry,
    pub base_additions: Vec<&'a Additions>,
    pub base_dependencies: Vec<&'a Dependency>,
    /// Sorted by name.
    pub intents: Vec<EffectiveIntent<'a>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntentChoice<'s> {
    /// Base plus every effective intent (no intent requested).
    All,
    /// Base plus one named intent.
    Named(&'s str),
    /// Base only (dependencies, and comparing a default with a new intent).
    BaseOnly,
    /// Base plus the named intents (a dependency reference naming intents).
    Set(&'s [String]),
}

/// The selected base plus intents of one effective policy.
#[derive(Clone, Debug)]
pub struct Selected<'a> {
    pub additions: Vec<&'a Additions>,
    pub dependencies: Vec<&'a Dependency>,
    pub intent_names: Vec<String>,
}

fn add_intents<'a>(
    intents: &mut Vec<EffectiveIntent<'a>>,
    list: &'a [(String, IntentDefinition)],
    extend: bool,
    origin: &str,
) -> std::result::Result<(), String> {
    for (name, definition) in list {
        match intents.iter_mut().find(|i| &i.name == name) {
            Some(existing) if extend => {
                existing.additions.push(&definition.additions);
                existing.dependencies.extend(&definition.dependencies);
            }
            Some(_) => {
                return Err(format!(
                    "intent '{name}' from {origin} is already declared by another overlay"
                ))
            }
            None if extend => {
                return Err(format!(
                    "{origin} extends intent '{name}', which is not declared"
                ))
            }
            None => intents.push(EffectiveIntent {
                name: name.clone(),
                additions: vec![&definition.additions],
                dependencies: definition.dependencies.iter().collect(),
            }),
        }
    }
    Ok(())
}

/// Default + platform additions + at most one version variant (all additive).
pub fn materialize<'a>(
    entry: &'a CatalogEntry,
    platform: Option<&'a PlatformVariant>,
    version: Option<&'a VersionVariant>,
) -> std::result::Result<Effective<'a>, String> {
    let mut effective = Effective {
        entry,
        base_additions: Vec::new(),
        base_dependencies: entry.default.dependencies.iter().collect(),
        intents: Vec::new(),
    };
    add_intents(
        &mut effective.intents,
        &entry.default.intents,
        false,
        "the default",
    )?;
    let overlays: [(Option<&'a Overlay>, String); 2] = [
        (
            platform.map(|p| &p.overlay),
            platform.map_or_else(String::new, |p| {
                format!(
                    "the {}/{} overlay",
                    p.platform,
                    p.architecture.map_or("*", Architecture::as_str)
                )
            }),
        ),
        (
            version.map(|v| &v.overlay),
            version.map_or_else(String::new, |v| {
                format!("version range '{}'", v.version_range)
            }),
        ),
    ];
    for (overlay, origin) in overlays {
        let Some(overlay) = overlay else { continue };
        effective.base_additions.push(&overlay.policy_additions);
        effective.base_dependencies.extend(&overlay.dependencies);
        add_intents(
            &mut effective.intents,
            &overlay.intent_additions,
            true,
            &origin,
        )?;
        add_intents(&mut effective.intents, &overlay.new_intents, false, &origin)?;
    }
    effective
        .intents
        .sort_by(|a, b| cmp_utf16(&a.name, &b.name));
    Ok(effective)
}

impl<'a> Effective<'a> {
    pub fn has_intent(&self, name: &str) -> bool {
        self.intents.iter().any(|i| i.name == name)
    }

    pub fn intent_names(&self) -> Vec<String> {
        self.intents.iter().map(|i| i.name.clone()).collect()
    }

    /// `None` when a named intent is not defined for this effective policy.
    pub fn select(&self, choice: IntentChoice<'_>) -> Option<Selected<'a>> {
        let chosen: Vec<&EffectiveIntent<'a>> = match choice {
            IntentChoice::All => self.intents.iter().collect(),
            IntentChoice::BaseOnly => Vec::new(),
            IntentChoice::Named(name) => vec![self.intents.iter().find(|i| i.name == name)?],
            IntentChoice::Set(names) => self
                .intents
                .iter()
                .filter(|i| names.contains(&i.name))
                .collect(),
        };
        if let IntentChoice::Set(names) = choice {
            if chosen.len() != names.len() {
                return None;
            }
        }
        let mut additions = self.base_additions.clone();
        let mut dependencies = self.base_dependencies.clone();
        for intent in &chosen {
            additions.extend(&intent.additions);
            dependencies.extend(&intent.dependencies);
        }
        Some(Selected {
            additions,
            dependencies,
            intent_names: chosen.iter().map(|i| i.name.clone()).collect(),
        })
    }
}

/// A node of the composed set: the component and how its platform overlay
/// was selected.
#[derive(Clone, Debug)]
pub struct Node<'a> {
    pub component: Component<'a>,
    /// Neutral fallback on a platform with architecture-specific overlays.
    pub neutral_fallback: Option<Platform>,
}

/// Accumulates the dependency closure of every contribution in one lookup.
/// A dependency contributes its default base plus its platform overlay's base
/// additions; a reference naming intents also adds those intents (design
/// §4.5). Each (entry, base or intent) component contributes once.
pub struct Closure<'a, 'm> {
    pub by_id: &'m EntryIndex<'a>,
    pub platform: Platform,
    pub architecture: &'m dyn Fn() -> Result<Architecture>,
    pub nodes: Vec<Node<'a>>,
    pub records: Vec<DependencyRecord>,
    /// Node index per dependency entry, and the intents already added to it.
    done: HashMap<&'a str, (usize, HashSet<String>)>,
}

impl<'a, 'm> Closure<'a, 'm> {
    pub fn new(
        by_id: &'m EntryIndex<'a>,
        platform: Platform,
        architecture: &'m dyn Fn() -> Result<Architecture>,
    ) -> Self {
        Self {
            by_id,
            platform,
            architecture,
            nodes: Vec::new(),
            records: Vec::new(),
            done: HashMap::new(),
        }
    }

    /// Adds the dependencies of a contribution of `root`, transitively.
    pub fn add_dependencies(
        &mut self,
        root: &'a CatalogEntry,
        dependencies: &[&'a Dependency],
    ) -> Result<()> {
        let mut stack = vec![root.entry_id.as_str()];
        self.visit_all(dependencies, &mut stack)
    }

    fn visit_all(
        &mut self,
        dependencies: &[&'a Dependency],
        stack: &mut Vec<&'a str>,
    ) -> Result<()> {
        for dependency in dependencies {
            let from = *stack.last().expect("stack is seeded with the root");
            let Some(target) = self.by_id.get(dependency.entry_id.as_str()).copied() else {
                return Err(invalid_catalog(format!(
                    "dependency resolution failed: missing-entry ({from} -> {})",
                    dependency.entry_id
                )));
            };
            if stack.contains(&target.entry_id.as_str()) {
                let mut path = stack.clone();
                path.push(&target.entry_id);
                return Err(invalid_catalog(format!(
                    "dependency resolution failed: cycle ({})",
                    path.join(" -> ")
                )));
            }
            let selection = select_platform_variant(target, self.platform, self.architecture)?;
            let effective = materialize(target, selection.map(|s| s.variant), None)
                .map_err(|e| invalid_catalog(format!("'{}': {e}", target.entry_id)))?;
            let mut named: Vec<String> = dependency.intents.clone().unwrap_or_default();
            named.sort_by(|a, b| cmp_utf16(a, b));
            if let Some(missing) = named.iter().find(|n| !effective.has_intent(n)) {
                return Err(invalid_catalog(format!(
                    "dependency resolution failed: {from} -> {} names intent '{missing}', which {} does not define on {}",
                    target.entry_id, target.entry_id, self.platform
                )));
            }
            let record = DependencyRecord {
                entry_id: target.entry_id.clone(),
                entry_revision: target.entry_revision,
                required_version_range: dependency.version_range.clone(),
                version_selection: VersionSelection::default_match(),
                intent_selection: IntentSelection {
                    requested: None,
                    mode: if dependency.intents.is_some() {
                        IntentMode::Named
                    } else {
                        IntentMode::None
                    },
                    selected: named.clone(),
                },
            };
            if !self.records.contains(&record) {
                self.records.push(record);
            }
            let (new_base, new_intents): (bool, Vec<String>) =
                match self.done.get(target.entry_id.as_str()) {
                    None => (true, named),
                    Some((_, added)) => (
                        false,
                        named.into_iter().filter(|n| !added.contains(n)).collect(),
                    ),
                };
            if !new_base && new_intents.is_empty() {
                continue;
            }
            let base = effective
                .select(IntentChoice::BaseOnly)
                .expect("the base always selects");
            let chosen = effective
                .select(IntentChoice::Set(&new_intents))
                .expect("named intents were checked");
            // `chosen` repeats the base; keep only what is new.
            let base_len = base.additions.len();
            let base_dep_len = base.dependencies.len();
            let mut additions: Vec<&'a Additions> = Vec::new();
            let mut next: Vec<&'a Dependency> = Vec::new();
            if new_base {
                additions.extend(&base.additions);
                next.extend(&base.dependencies);
            }
            additions.extend(&chosen.additions[base_len..]);
            next.extend(&chosen.dependencies[base_dep_len..]);
            match self.done.get_mut(target.entry_id.as_str()) {
                Some((index, added)) => {
                    added.extend(new_intents);
                    self.nodes[*index].component.additions.extend(additions);
                }
                None => {
                    self.done.insert(
                        &target.entry_id,
                        (self.nodes.len(), new_intents.into_iter().collect()),
                    );
                    self.nodes.push(Node {
                        component: Component {
                            entry_id: &target.entry_id,
                            base: &target.default.sandbox_policy,
                            additions,
                        },
                        neutral_fallback: selection
                            .filter(|s| !s.exact && has_arch_specific(target, self.platform))
                            .map(|_| self.platform),
                    });
                }
            }
            stack.push(&target.entry_id);
            self.visit_all(&next, stack)?;
            stack.pop();
        }
        Ok(())
    }

    /// Dependency records sorted by entryId, entryRevision, range (absent
    /// first), then the selected intents.
    pub fn sorted_records(&self) -> Vec<DependencyRecord> {
        let mut records = self.records.clone();
        records.sort_by(|a, b| {
            cmp_utf16(&a.entry_id, &b.entry_id)
                .then(
                    a.entry_revision
                        .partial_cmp(&b.entry_revision)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then_with(
                    || match (&a.required_version_range, &b.required_version_range) {
                        (None, None) => std::cmp::Ordering::Equal,
                        (None, Some(_)) => std::cmp::Ordering::Less,
                        (Some(_), None) => std::cmp::Ordering::Greater,
                        (Some(x), Some(y)) => cmp_utf16(x, y),
                    },
                )
                .then_with(|| {
                    cmp_utf16(
                        &a.intent_selection.selected.join("\u{0}"),
                        &b.intent_selection.selected.join("\u{0}"),
                    )
                })
        });
        records
    }
}

// ---------------------------------------------------------------------------
// Build-time materialization (design §7)
// ---------------------------------------------------------------------------

/// One materialized effective policy, symbolic (templates unsubstituted).
#[derive(Clone, Debug)]
pub struct Materialized {
    pub platform: Platform,
    pub architecture: Architecture,
    /// `None` for the default; otherwise the version range.
    pub version_range: Option<String>,
    /// `None` for all intents.
    pub intent: Option<String>,
    pub policy: SandboxPolicy,
    pub dependency_entry_ids: Vec<String>,
    pub intent_names: Vec<String>,
    /// The common default (no platform or version overlay) for the same
    /// intent, or its base for an intent the default does not declare.
    pub default_policy: SandboxPolicy,
    pub default_dependency_entry_ids: Vec<String>,
}

fn compose_symbolic(
    entry: &CatalogEntry,
    selected: &Selected<'_>,
    by_id: &EntryIndex<'_>,
    platform: Platform,
    architecture: Architecture,
) -> std::result::Result<(SandboxPolicy, Vec<String>), String> {
    let arch = move || Ok(architecture);
    let mut closure = Closure::new(by_id, platform, &arch);
    closure
        .add_dependencies(entry, &selected.dependencies)
        .map_err(|e| e.detail().to_string())?;
    let mut components = vec![Component {
        entry_id: &entry.entry_id,
        base: &entry.default.sandbox_policy,
        additions: selected.additions.clone(),
    }];
    components.extend(closure.nodes.iter().map(|n| n.component.clone()));
    compose_check(&components)?;
    let composed = compose_policy(&components, &|t: &str| t.to_string(), platform);
    let mut deps: Vec<String> = closure.records.iter().map(|r| r.entry_id.clone()).collect();
    deps.sort_by(|a, b| cmp_utf16(a, b));
    deps.dedup();
    Ok((composed.policy, deps))
}

fn rule_keys(policy: &SandboxPolicy) -> HashSet<String> {
    policy
        .network
        .as_ref()
        .and_then(|n| n.get("egress"))
        .and_then(|e| e.get("allow"))
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
        .map(crate::policy_store::json::canonical_json)
        .collect()
}

fn fs_paths(
    policy: &SandboxPolicy,
    field: fn(&crate::policy_store::model::FilesystemPolicy) -> &Option<Vec<String>>,
) -> Vec<String> {
    policy
        .filesystem
        .as_ref()
        .and_then(|fs| field(fs).clone())
        .unwrap_or_default()
}

/// Every access the default grants is still granted by the effective policy.
fn subset_violation(
    default: &SandboxPolicy,
    default_deps: &[String],
    effective: &SandboxPolicy,
    effective_deps: &[String],
    platform: Platform,
) -> Option<String> {
    use crate::policy_store::paths::path_exact_segments as seg;
    let rw = fs_paths(effective, |f| &f.readwrite_paths);
    let ro = fs_paths(effective, |f| &f.readonly_paths);
    let within = |path: &str, list: &[String]| {
        let p = seg(path, platform);
        list.iter().any(|g| {
            let g = seg(g, platform);
            p.len() >= g.len() && g.iter().zip(&p).all(|(a, b)| a == b)
        })
    };
    for path in fs_paths(default, |f| &f.readwrite_paths) {
        if !within(&path, &rw) {
            return Some(format!("loses read-write '{path}'"));
        }
    }
    for path in fs_paths(default, |f| &f.readonly_paths) {
        if !within(&path, &rw) && !ro.contains(&path) {
            return Some(format!("loses read-only '{path}'"));
        }
    }
    let effective_rules = rule_keys(effective);
    if !rule_keys(default).is_subset(&effective_rules) {
        return Some("loses an outbound allow rule".to_string());
    }
    if let Some(dep) = default_deps.iter().find(|d| !effective_deps.contains(d)) {
        return Some(format!("loses dependency '{dep}'"));
    }
    None
}

/// Materializes every platform × architecture × version × intent policy of
/// `entry` (design §7), validating composition, the composed policy, and that
/// the default is a subset of each effective policy.
pub fn materialize_entry(
    entry: &CatalogEntry,
    by_id: &EntryIndex<'_>,
    contract: &CatalogContract,
) -> Result<Vec<Materialized>> {
    let mut out = Vec::new();
    for platform in Platform::ALL {
        for architecture in Architecture::ALL {
            let arch = move || Ok(architecture);
            let selection = select_platform_variant(entry, platform, &arch)?;
            let default_effective = materialize(entry, None, None)
                .map_err(|e| invalid_catalog(format!("'{}': {e}", entry.entry_id)))?;
            let versions: Vec<Option<&VersionVariant>> = std::iter::once(None)
                .chain(entry.version_variants.iter().map(Some))
                .collect();
            for version in versions {
                let range = version.map(|v| v.version_range.as_str().to_string());
                let at = format!(
                    "'{}' on {platform}/{architecture} ({})",
                    entry.entry_id,
                    range.as_deref().unwrap_or("default")
                );
                let effective = match materialize(entry, selection.map(|s| s.variant), version) {
                    Ok(effective) => effective,
                    Err(e) => return fail(format!("{at}: {e}")),
                };
                let intents: Vec<Option<String>> = std::iter::once(None)
                    .chain(effective.intents.iter().map(|i| Some(i.name.clone())))
                    .collect();
                for intent in intents {
                    let choice = intent
                        .as_deref()
                        .map_or(IntentChoice::All, IntentChoice::Named);
                    let at = format!("{at} intent {}", intent.as_deref().unwrap_or("(all)"));
                    let selected = effective.select(choice).expect("listed intents select");
                    let (policy, deps) =
                        compose_symbolic(entry, &selected, by_id, platform, architecture)
                            .map_err(|e| invalid_catalog(format!("{at}: {e}")))?;
                    if let Err(e) =
                        validate_sandbox_policy(Some(&policy.to_json()), "composed", contract)
                    {
                        return fail(format!("{at}: composed policy is invalid: {}", e.detail()));
                    }
                    let default_choice = match choice {
                        IntentChoice::Named(name) if !default_effective.has_intent(name) => {
                            IntentChoice::BaseOnly
                        }
                        other => other,
                    };
                    let default_selected = default_effective
                        .select(default_choice)
                        .expect("default intents select");
                    let (default_policy, default_deps) =
                        compose_symbolic(entry, &default_selected, by_id, platform, architecture)
                            .map_err(|e| invalid_catalog(format!("{at} (default): {e}")))?;
                    if let Some(violation) =
                        subset_violation(&default_policy, &default_deps, &policy, &deps, platform)
                    {
                        return fail(format!(
                            "{at}: the default is not a subset of the effective policy; it {violation}"
                        ));
                    }
                    out.push(Materialized {
                        platform,
                        architecture,
                        version_range: range.clone(),
                        intent: intent.clone(),
                        policy,
                        dependency_entry_ids: deps,
                        intent_names: selected.intent_names,
                        default_policy,
                        default_dependency_entry_ids: default_deps,
                    });
                }
            }
        }
    }
    Ok(out)
}

/// Revision-level materialization check, called from catalog validation.
pub(crate) fn validate_materializations(
    entries: &[CatalogEntry],
    by_id: &EntryIndex<'_>,
    contract: &CatalogContract,
) -> Result<()> {
    for entry in entries {
        materialize_entry(entry, by_id, contract)?;
    }
    Ok(())
}
