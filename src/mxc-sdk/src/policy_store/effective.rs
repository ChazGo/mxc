// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Effective policies (design §4.3–§4.5): the default plus the selected
//! platform overlay, at most one version variant, and the selected intents,
//! kept as independently de-duplicated source layers; the dependency closure;
//! and the build-time materialization of every effective policy the catalog
//! can produce.

use crate::policy_store::catalog::{
    fail, validate_requirements, Additions, CatalogContract, CatalogEntry, CatalogPolicy,
    Dependency, EntryIndex, IntentDefinition, Overlay, PlatformVariant, VersionVariant,
};
use crate::policy_store::compose::{compose, compose_check, Contribution, LexicalIdentity};
use crate::policy_store::errors::{invalid_catalog, Result};
use crate::policy_store::json::{cmp_utf16, Json};
use crate::policy_store::model::{
    Architecture, DependencyRecord, FilesystemRequirements, IntentMode, IntentSelection, Platform,
    Requirements, VersionSelection,
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

/// Whether the platform has architecture-specific overlays (so the
/// architecture was consulted, and a neutral selection is a fallback).
pub fn has_arch_specific(entry: &CatalogEntry, platform: Platform) -> bool {
    entry
        .platform_variants
        .iter()
        .any(|v| v.platform == platform && v.architecture.is_some())
}

/// Which part of an entry a layer comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LayerSource {
    Default,
    /// A platform overlay, by index in `platformVariants`.
    Platform(usize),
    /// A version overlay, by index in `versionVariants`.
    Version(usize),
}

/// The de-duplication key of one source contribution layer (design §4.5):
/// the default base once; each platform base once; default intents by
/// intent; platform intents by selector and intent; version bases by range;
/// and version intents by range and intent. A requesting pair's version or
/// intent is never part of a shared base-layer key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LayerKey {
    pub entry_id: String,
    pub source: LayerSource,
    /// `None` for a base layer.
    pub intent: Option<String>,
}

/// What a layer contributes.
#[derive(Clone, Copy, Debug)]
pub enum LayerBody<'a> {
    /// The default's `requirements`.
    Base(&'a CatalogPolicy),
    /// Additive data from an intent or overlay.
    Additions(&'a Additions),
}

#[derive(Clone, Debug)]
pub struct Layer<'a> {
    pub key: LayerKey,
    pub entry_id: &'a str,
    pub body: LayerBody<'a>,
    pub dependencies: &'a [Dependency],
}

#[derive(Clone, Debug)]
pub struct EffectiveIntent<'a> {
    pub name: String,
    pub layers: Vec<Layer<'a>>,
}

/// One effective policy before intent selection.
#[derive(Clone, Debug)]
pub struct Effective<'a> {
    pub entry: &'a CatalogEntry,
    pub base_layers: Vec<Layer<'a>>,
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
    pub layers: Vec<Layer<'a>>,
    pub intent_names: Vec<String>,
}

fn layer<'a>(
    entry: &'a CatalogEntry,
    source: LayerSource,
    intent: Option<&str>,
    body: LayerBody<'a>,
    dependencies: &'a [Dependency],
) -> Layer<'a> {
    Layer {
        key: LayerKey {
            entry_id: entry.entry_id.clone(),
            source,
            intent: intent.map(str::to_string),
        },
        entry_id: &entry.entry_id,
        body,
        dependencies,
    }
}

fn add_intents<'a>(
    intents: &mut Vec<EffectiveIntent<'a>>,
    entry: &'a CatalogEntry,
    source: LayerSource,
    list: &'a [(String, IntentDefinition)],
    extend: bool,
    origin: &str,
) -> std::result::Result<(), String> {
    for (name, definition) in list {
        let added = layer(
            entry,
            source,
            Some(name),
            LayerBody::Additions(&definition.additions),
            &definition.dependencies,
        );
        match intents.iter_mut().find(|i| &i.name == name) {
            Some(existing) if extend => existing.layers.push(added),
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
                layers: vec![added],
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
        base_layers: vec![layer(
            entry,
            LayerSource::Default,
            None,
            LayerBody::Base(&entry.default.requirements),
            &entry.default.dependencies,
        )],
        intents: Vec::new(),
    };
    add_intents(
        &mut effective.intents,
        entry,
        LayerSource::Default,
        &entry.default.intents,
        false,
        "the default",
    )?;
    let platform_overlay = platform.map(|p| {
        let index = entry
            .platform_variants
            .iter()
            .position(|v| std::ptr::eq(v, p))
            .expect("the platform overlay belongs to the entry");
        (
            &p.overlay,
            LayerSource::Platform(index),
            format!(
                "the {}/{} overlay",
                p.platform,
                p.architecture.map_or("*", Architecture::as_str)
            ),
        )
    });
    let version_overlay = version.map(|v| {
        let index = entry
            .version_variants
            .iter()
            .position(|x| std::ptr::eq(x, v))
            .expect("the version overlay belongs to the entry");
        (
            &v.overlay,
            LayerSource::Version(index),
            format!("version range '{}'", v.version_range),
        )
    });
    for (overlay, source, origin) in [platform_overlay, version_overlay].into_iter().flatten() {
        let overlay: &'a Overlay = overlay;
        effective.base_layers.push(layer(
            entry,
            source,
            None,
            LayerBody::Additions(&overlay.policy_additions),
            &overlay.dependencies,
        ));
        add_intents(
            &mut effective.intents,
            entry,
            source,
            &overlay.intent_additions,
            true,
            &origin,
        )?;
        add_intents(
            &mut effective.intents,
            entry,
            source,
            &overlay.new_intents,
            false,
            &origin,
        )?;
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
        let mut layers = self.base_layers.clone();
        for intent in &chosen {
            layers.extend(intent.layers.iter().cloned());
        }
        Some(Selected {
            layers,
            intent_names: chosen.iter().map(|i| i.name.clone()).collect(),
        })
    }
}

/// Every selected layer of one lookup, de-duplicated by [`LayerKey`], with
/// the sorted, distinct input indexes (owners) that require it.
#[derive(Default)]
pub struct LayerSet<'a> {
    items: Vec<(Layer<'a>, Vec<usize>)>,
    index: HashMap<LayerKey, usize>,
}

fn union_into(target: &mut Vec<usize>, extra: &[usize]) {
    target.extend_from_slice(extra);
    target.sort_unstable();
    target.dedup();
}

impl<'a> LayerSet<'a> {
    pub fn add(&mut self, layer: &Layer<'a>, owners: &[usize]) {
        match self.index.get(&layer.key) {
            Some(&at) => union_into(&mut self.items[at].1, owners),
            None => {
                self.index.insert(layer.key.clone(), self.items.len());
                let mut owners = owners.to_vec();
                union_into(&mut owners, &[]);
                self.items.push((layer.clone(), owners));
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn layers(&self) -> impl Iterator<Item = (&Layer<'a>, &[usize])> {
        self.items.iter().map(|(l, o)| (l, o.as_slice()))
    }

    /// Contributions of every layer that keeps an owner outside `excluded`,
    /// with only those remaining owners. Layers owned solely by excluded
    /// pairs are discarded.
    pub fn contributions(&self, excluded: &HashSet<usize>) -> Vec<Contribution<'a>> {
        self.items
            .iter()
            .filter_map(|(layer, owners)| {
                let owners: Vec<usize> = owners
                    .iter()
                    .copied()
                    .filter(|o| !excluded.contains(o))
                    .collect();
                (!owners.is_empty()).then_some(Contribution {
                    entry_id: layer.entry_id,
                    body: layer.body,
                    owners,
                })
            })
            .collect()
    }
}

/// Accumulates the dependency closure of every contribution in one lookup.
/// A dependency contributes its default base plus its platform overlay's base
/// additions; a reference naming intents also adds those intents, with their
/// applicable platform intent additions (design §4.5). Dependency layers
/// inherit the owners of the layer that references them.
pub struct Closure<'a, 'm> {
    pub by_id: &'m EntryIndex<'a>,
    pub platform: Platform,
    pub architecture: &'m dyn Fn() -> Result<Architecture>,
    pub records: Vec<DependencyRecord>,
    /// Platform selection of every dependency entry visited, with its owners.
    pub selections: Vec<(&'a CatalogEntry, Option<PlatformSelection<'a>>, Vec<usize>)>,
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
            records: Vec::new(),
            selections: Vec::new(),
        }
    }

    /// Adds the dependencies of `layers` (selected for `root`), transitively.
    pub fn add_layers(
        &mut self,
        root: &'a CatalogEntry,
        layers: &[Layer<'a>],
        owners: &[usize],
        set: &mut LayerSet<'a>,
    ) -> Result<()> {
        let mut stack = vec![root.entry_id.as_str()];
        for layer in layers {
            set.add(layer, owners);
            self.visit_all(layer.dependencies, owners, set, &mut stack)?;
        }
        Ok(())
    }

    fn visit_all(
        &mut self,
        dependencies: &'a [Dependency],
        owners: &[usize],
        set: &mut LayerSet<'a>,
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
            let Some(selected) = effective.select(IntentChoice::Set(&named)) else {
                let missing = named
                    .iter()
                    .find(|n| !effective.has_intent(n))
                    .cloned()
                    .unwrap_or_default();
                return Err(invalid_catalog(format!(
                    "dependency resolution failed: {from} -> {} names intent '{missing}', which {} does not define on {}",
                    target.entry_id, target.entry_id, self.platform
                )));
            };
            let record = DependencyRecord {
                entry_id: target.entry_id.clone(),
                entry_revision: target.entry_revision,
                input_indexes: owners.to_vec(),
                required_version_range: dependency.version_range.clone(),
                version_selection: VersionSelection::default_match(),
                intent_selection: IntentSelection {
                    requested: None,
                    mode: if dependency.intents.is_some() {
                        IntentMode::Named
                    } else {
                        IntentMode::None
                    },
                    selected: named,
                },
            };
            match self.records.iter_mut().find(|r| r.same_record(&record)) {
                Some(existing) => union_into(&mut existing.input_indexes, owners),
                None => {
                    let mut record = record;
                    union_into(&mut record.input_indexes, &[]);
                    self.records.push(record);
                }
            }
            match self
                .selections
                .iter_mut()
                .find(|(entry, _, _)| std::ptr::eq(*entry, target))
            {
                Some((_, _, existing)) => union_into(existing, owners),
                None => self.selections.push((target, selection, owners.to_vec())),
            }
            stack.push(&target.entry_id);
            for layer in &selected.layers {
                set.add(layer, owners);
                self.visit_all(layer.dependencies, owners, set, stack)?;
            }
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
// Build-time materialization (design §4.2, §7)
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
    pub requirements: Requirements,
    pub dependency_entry_ids: Vec<String>,
    pub intent_names: Vec<String>,
    /// The common default (no platform or version overlay) for the same
    /// intent, or its base for an intent the default does not declare.
    pub default_requirements: Requirements,
    pub default_dependency_entry_ids: Vec<String>,
}

fn compose_symbolic<'a>(
    entry: &'a CatalogEntry,
    selected: &Selected<'a>,
    by_id: &EntryIndex<'a>,
    platform: Platform,
    architecture: Architecture,
) -> std::result::Result<(Requirements, Vec<String>), String> {
    let arch = move || Ok(architecture);
    let mut closure = Closure::new(by_id, platform, &arch);
    let mut set = LayerSet::default();
    closure
        .add_layers(entry, &selected.layers, &[0], &mut set)
        .map_err(|e| e.detail().to_string())?;
    let contributions = set.contributions(&HashSet::new());
    compose_check(&contributions)?;
    let composed = compose(
        &contributions,
        &|t: &str| t.to_string(),
        platform,
        &LexicalIdentity,
    );
    let mut deps: Vec<String> = closure.records.iter().map(|r| r.entry_id.clone()).collect();
    deps.sort_by(|a, b| cmp_utf16(a, b));
    deps.dedup();
    Ok((composed.requirements, deps))
}

fn rule_keys(requirements: &Requirements) -> HashSet<String> {
    requirements
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
    requirements: &Requirements,
    field: fn(&FilesystemRequirements) -> &Option<Vec<String>>,
) -> Vec<String> {
    requirements
        .filesystem
        .as_ref()
        .and_then(|fs| field(fs).clone())
        .unwrap_or_default()
}

/// Every access the default grants is still granted by the effective policy.
fn subset_violation(
    default: &Requirements,
    default_deps: &[String],
    effective: &Requirements,
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
/// `entry` (design §7), validating composition, the closed composed fields,
/// and that the default is a subset of each effective policy.
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
                    let (requirements, deps) =
                        compose_symbolic(entry, &selected, by_id, platform, architecture)
                            .map_err(|e| invalid_catalog(format!("{at}: {e}")))?;
                    if let Err(e) =
                        validate_requirements(Some(&requirements.to_json()), "composed", contract)
                    {
                        return fail(format!(
                            "{at}: composed requirements are invalid: {}",
                            e.detail()
                        ));
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
                    let (default_requirements, default_deps) =
                        compose_symbolic(entry, &default_selected, by_id, platform, architecture)
                            .map_err(|e| invalid_catalog(format!("{at} (default): {e}")))?;
                    if let Some(violation) = subset_violation(
                        &default_requirements,
                        &default_deps,
                        &requirements,
                        &deps,
                        platform,
                    ) {
                        return fail(format!(
                            "{at}: the default is not a subset of the effective policy; it {violation}"
                        ));
                    }
                    out.push(Materialized {
                        platform,
                        architecture,
                        version_range: range.clone(),
                        intent: intent.clone(),
                        requirements,
                        dependency_entry_ids: deps,
                        intent_names: selected.intent_names,
                        default_requirements,
                        default_dependency_entry_ids: default_deps,
                    });
                }
            }
        }
    }
    Ok(out)
}

/// Revision-level materialization check, called from catalog validation:
/// every combination composes, and its exact SDK request validates.
pub(crate) fn validate_materializations(
    entries: &[CatalogEntry],
    by_id: &EntryIndex<'_>,
    contract: &CatalogContract,
) -> Result<()> {
    for entry in entries {
        for materialized in materialize_entry(entry, by_id, contract)? {
            crate::policy_store::exact::validate_materialized(&materialized, contract).map_err(
                |e| {
                    invalid_catalog(format!(
                        "'{}' on {}/{} ({}) intent {}: {e}",
                        entry.entry_id,
                        materialized.platform,
                        materialized.architecture,
                        materialized.version_range.as_deref().unwrap_or("default"),
                        materialized.intent.as_deref().unwrap_or("(all)")
                    ))
                },
            )?;
        }
    }
    Ok(())
}
