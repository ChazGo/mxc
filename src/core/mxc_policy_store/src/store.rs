// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Manifest validation and lazy, digest-checked, validated revision loading
//! (TypeScript `src/store.ts`), plus the catalog bundled into this crate.

use crate::catalog::{
    compare_catalog_revisions, is_catalog_revision_id, validate_catalog_revision,
    validate_contract, CatalogContract, CatalogRevision, CATALOG_SCHEMA_VERSION,
};
use crate::errors::{invalid_catalog, ErrorReason, PolicyCatalogError, Result};
use crate::json::{canonical_sha256, Json, JsonObject};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// One published revision as listed in `manifest.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestRevision {
    pub catalog_revision: String,
    pub file: String,
    pub sha256: String,
}

/// A validated manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogManifest {
    pub catalog_schema_version: String,
    pub default_revision: String,
    pub revisions: Vec<ManifestRevision>,
}

/// Source of catalog data: the parsed contract and manifest, and a reader
/// for revision files named by the manifest.
pub trait CatalogSource: Send + Sync {
    fn contract(&self) -> &Json;
    fn manifest(&self) -> &Json;
    /// Reads and parses one manifest `file` entry. The error text follows
    /// `could not be read: ` in the resulting integrity error.
    fn read_revision(&self, file: &str) -> std::result::Result<Json, String>;
}

/// In-memory catalog source (fixtures, tests, embedded data).
#[derive(Clone, Debug)]
pub struct MemorySource {
    pub contract: Json,
    pub manifest: Json,
    pub files: HashMap<String, Json>,
}

impl MemorySource {
    /// A source whose manifest publishes `revisions` with correct canonical
    /// digests; the default is the last revision unless `default_revision`
    /// is given. `digest_overrides` replaces digests by revision id.
    pub fn publishing(
        contract: Json,
        revisions: &[Json],
        default_revision: Option<&str>,
        digest_overrides: &HashMap<String, String>,
    ) -> Self {
        let mut files = HashMap::new();
        let mut listed = Vec::new();
        for revision in revisions {
            let id = revision
                .get("catalogRevision")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_string();
            let file = format!("revisions/{id}.json");
            files.insert(file.clone(), revision.clone());
            let mut entry = JsonObject::new();
            entry.insert("catalogRevision", Json::String(id.clone()));
            entry.insert("file", Json::String(file));
            let digest = digest_overrides
                .get(&id)
                .cloned()
                .unwrap_or_else(|| canonical_sha256(revision));
            entry.insert("sha256", Json::String(digest));
            listed.push(Json::Object(entry));
        }
        let default = default_revision.map(str::to_string).unwrap_or_else(|| {
            revisions
                .last()
                .and_then(|r| r.get("catalogRevision"))
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_string()
        });
        let mut manifest = JsonObject::new();
        manifest.insert(
            "catalogSchemaVersion",
            Json::String(CATALOG_SCHEMA_VERSION.into()),
        );
        manifest.insert("defaultRevision", Json::String(default));
        manifest.insert("revisions", Json::Array(listed));
        Self {
            contract,
            manifest: Json::Object(manifest),
            files,
        }
    }
}

impl CatalogSource for MemorySource {
    fn contract(&self) -> &Json {
        &self.contract
    }

    fn manifest(&self) -> &Json {
        &self.manifest
    }

    fn read_revision(&self, file: &str) -> std::result::Result<Json, String> {
        self.files
            .get(file)
            .cloned()
            .ok_or_else(|| format!("no such file '{file}'"))
    }
}

/// Reads a file as UTF-8 text the way Node's `readFileSync(path, 'utf8')`
/// does (invalid sequences become U+FFFD).
pub(crate) fn read_text(path: &Path) -> std::io::Result<String> {
    std::fs::read(path).map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

fn read_json(path: &Path) -> std::result::Result<Json, String> {
    let text = read_text(path).map_err(|e| e.to_string())?;
    Json::parse(&text).map_err(|e| e.to_string())
}

/// Catalog directory source (`contract.v1.json`, `manifest.json`, `revisions/`).
#[derive(Clone, Debug)]
pub struct DirectorySource {
    directory: PathBuf,
    contract: Json,
    manifest: Json,
}

impl DirectorySource {
    pub fn open(directory: impl Into<PathBuf>) -> Result<Self> {
        let directory = directory.into();
        let read = || -> std::result::Result<(Json, Json), String> {
            Ok((
                read_json(&directory.join("contract.v1.json"))?,
                read_json(&directory.join("manifest.json"))?,
            ))
        };
        match read() {
            Ok((contract, manifest)) => Ok(Self {
                directory,
                contract,
                manifest,
            }),
            Err(message) => Err(PolicyCatalogError::new(
                ErrorReason::Integrity,
                format!("catalog data could not be read: {message}"),
            )),
        }
    }
}

impl CatalogSource for DirectorySource {
    fn contract(&self) -> &Json {
        &self.contract
    }

    fn manifest(&self) -> &Json {
        &self.manifest
    }

    fn read_revision(&self, file: &str) -> std::result::Result<Json, String> {
        read_json(&self.directory.join(file))
    }
}

fn invalid<T>(message: impl AsRef<str>) -> Result<T> {
    Err(invalid_catalog(message))
}

/// Validates `manifest.json` (TypeScript `validateManifest`).
pub fn validate_manifest(raw: &Json) -> Result<CatalogManifest> {
    let Some(object) = raw.as_object() else {
        return invalid("manifest root must be an object");
    };
    for key in object.keys() {
        if !["catalogSchemaVersion", "defaultRevision", "revisions"].contains(&key) {
            return invalid(format!("unsupported field 'manifest.{key}'"));
        }
    }
    if object.get("catalogSchemaVersion").and_then(Json::as_str) != Some(CATALOG_SCHEMA_VERSION) {
        return invalid(format!(
            "manifest.catalogSchemaVersion must be '{CATALOG_SCHEMA_VERSION}'"
        ));
    }
    let items = match object.get("revisions").and_then(Json::as_array) {
        Some(items) if !items.is_empty() => items,
        _ => return invalid("manifest.revisions must be a non-empty array"),
    };
    let mut revisions = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let at = format!("manifest.revisions[{index}]");
        let Some(item) = item.as_object() else {
            return invalid(format!("'{at}' must be an object"));
        };
        for key in item.keys() {
            if !["catalogRevision", "file", "sha256"].contains(&key) {
                return invalid(format!("unsupported field '{at}.{key}'"));
            }
        }
        let catalog_revision = match item.get("catalogRevision").and_then(Json::as_str) {
            Some(id) if is_catalog_revision_id(id) => id.to_string(),
            _ => return invalid(format!("'{at}.catalogRevision' must match YYYY-MM-DD.N")),
        };
        let expected_file = format!("revisions/{catalog_revision}.json");
        if item.get("file").and_then(Json::as_str) != Some(expected_file.as_str()) {
            return invalid(format!("'{at}.file' must be '{expected_file}'"));
        }
        let sha256 = match item.get("sha256").and_then(Json::as_str) {
            Some(d)
                if d.len() == 64
                    && d.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) =>
            {
                d.to_string()
            }
            _ => {
                return invalid(format!(
                    "'{at}.sha256' must be a lower-case hex SHA-256 digest"
                ))
            }
        };
        revisions.push(ManifestRevision {
            catalog_revision,
            file: expected_file,
            sha256,
        });
    }
    for index in 1..revisions.len() {
        if compare_catalog_revisions(
            &revisions[index - 1].catalog_revision,
            &revisions[index].catalog_revision,
        )? != Ordering::Less
        {
            return invalid(format!(
                "manifest.revisions must be strictly increasing ('{}')",
                revisions[index].catalog_revision
            ));
        }
    }
    let default_revision = match object.get("defaultRevision").and_then(Json::as_str) {
        Some(d) if revisions.iter().any(|r| r.catalog_revision == d) => d.to_string(),
        _ => return invalid("manifest.defaultRevision must name a listed revision"),
    };
    Ok(CatalogManifest {
        catalog_schema_version: CATALOG_SCHEMA_VERSION.to_string(),
        default_revision,
        revisions,
    })
}

/// Read-only access to installed, integrity-validated catalog revisions.
/// Revisions load lazily on first use, are verified against the manifest
/// digest and the contract, and are cached (successes only).
pub struct CatalogStore {
    contract: CatalogContract,
    manifest: CatalogManifest,
    source: Box<dyn CatalogSource>,
    loaded: Mutex<HashMap<String, Arc<CatalogRevision>>>,
}

impl std::fmt::Debug for CatalogStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatalogStore")
            .field("manifest", &self.manifest)
            .finish_non_exhaustive()
    }
}

impl CatalogStore {
    /// Validates the contract, then the manifest.
    pub fn new(source: impl CatalogSource + 'static) -> Result<Self> {
        let contract = validate_contract(source.contract())?;
        let manifest = validate_manifest(source.manifest())?;
        Ok(Self {
            contract,
            manifest,
            source: Box::new(source),
            loaded: Mutex::new(HashMap::new()),
        })
    }

    pub fn contract(&self) -> &CatalogContract {
        &self.contract
    }

    pub fn manifest(&self) -> &CatalogManifest {
        &self.manifest
    }

    pub fn default_revision(&self) -> &str {
        &self.manifest.default_revision
    }

    pub fn available_revisions(&self) -> Vec<String> {
        self.manifest
            .revisions
            .iter()
            .map(|r| r.catalog_revision.clone())
            .collect()
    }

    /// The requested revision, or the installed default. An explicitly
    /// requested revision that is not installed is an error, never substituted.
    pub fn revision(&self, catalog_revision: Option<&str>) -> Result<Arc<CatalogRevision>> {
        let id = catalog_revision.unwrap_or(&self.manifest.default_revision);
        if let Some(cached) = self.loaded.lock().expect("store cache lock").get(id) {
            return Ok(cached.clone());
        }
        let Some(listed) = self
            .manifest
            .revisions
            .iter()
            .find(|r| r.catalog_revision == id)
        else {
            return Err(PolicyCatalogError::new(
                ErrorReason::RevisionUnavailable,
                format!("catalog revision '{id}' is not installed"),
            ));
        };
        let raw = self.source.read_revision(&listed.file).map_err(|message| {
            PolicyCatalogError::new(
                ErrorReason::Integrity,
                format!("catalog revision '{id}' could not be read: {message}"),
            )
        })?;
        let digest = canonical_sha256(&raw);
        // BREAK-VERIFY(tamper): the published-digest comparison.
        if digest != listed.sha256 {
            return Err(PolicyCatalogError::new(
                ErrorReason::Integrity,
                format!(
                    "catalog revision '{id}' digest {digest} does not match the published digest {}",
                    listed.sha256
                ),
            ));
        }
        let revision = validate_catalog_revision(&raw, &self.contract)?;
        if revision.catalog_revision != id {
            return Err(PolicyCatalogError::new(
                ErrorReason::Integrity,
                format!(
                    "file '{}' declares revision '{}', expected '{id}'",
                    listed.file, revision.catalog_revision
                ),
            ));
        }
        let revision = Arc::new(revision);
        self.loaded
            .lock()
            .expect("store cache lock")
            .insert(id.to_string(), revision.clone());
        Ok(revision)
    }
}

/// Loads a store from a catalog directory.
pub fn load_catalog_directory(directory: impl AsRef<Path>) -> Result<CatalogStore> {
    CatalogStore::new(DirectorySource::open(directory.as_ref())?)
}

mod bundled_data {
    include!(concat!(env!("OUT_DIR"), "/bundled_catalog.rs"));
}

/// The directory the bundled catalog was embedded from at build time. The
/// data itself is compiled into the crate; this path is informational (the
/// `validate` command reports it) and may not exist at run time.
pub fn bundled_catalog_source_dir() -> &'static str {
    bundled_data::SOURCE_DIR
}

/// The embedded catalog as an in-memory source.
pub fn bundled_catalog_source() -> Result<MemorySource> {
    let parse = |name: &str, text: &str| {
        Json::parse(text).map_err(|e| {
            PolicyCatalogError::new(
                ErrorReason::Integrity,
                format!("catalog data could not be read: {name}: {e}"),
            )
        })
    };
    let contract = parse("contract.v1.json", bundled_data::CONTRACT)?;
    let manifest = parse("manifest.json", bundled_data::MANIFEST)?;
    let mut files = HashMap::new();
    for (file, text) in bundled_data::REVISIONS {
        // A revision that fails to parse is left out, so reading it reports
        // an integrity error exactly when it is requested.
        if let Ok(value) = Json::parse(text) {
            files.insert((*file).to_string(), value);
        }
    }
    Ok(MemorySource {
        contract,
        manifest,
        files,
    })
}

/// The catalog bundled with this crate (lazily constructed, then shared).
pub fn bundled_catalog_store() -> Result<Arc<CatalogStore>> {
    static BUNDLED: OnceLock<Arc<CatalogStore>> = OnceLock::new();
    if let Some(store) = BUNDLED.get() {
        return Ok(store.clone());
    }
    let store = Arc::new(CatalogStore::new(bundled_catalog_source()?)?);
    Ok(BUNDLED.get_or_init(|| store).clone())
}

/// Raw text of the embedded files (`contract.v1.json`, `manifest.json`,
/// `revisions/*.json`), for tooling that must see the shipped bytes.
pub fn bundled_catalog_files() -> Vec<(&'static str, &'static str)> {
    let mut files = vec![
        ("contract.v1.json", bundled_data::CONTRACT),
        ("manifest.json", bundled_data::MANIFEST),
    ];
    files.extend(bundled_data::REVISIONS.iter().copied());
    files
}
