// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Catalog-directory validation and the `--base-ref` published-revision
//! immutability check Tooling only: the
//! runtime lookup path never runs git.

use crate::errors::{invalid_catalog, PolicyCatalogError};
use crate::history::{
    check_published_immutability, check_store_history, PublishedRevision, PublishedState,
};
use crate::json::{Json, JsonObject};
use crate::model::Platform;
use crate::paths::{is_absolute_path, normalize_path};
use crate::store::{read_text, CatalogManifest, CatalogStore, DirectorySource};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Result of [`validate_catalog_directory`].
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogValidationReport {
    /// True only when every requested check ran and passed.
    pub ok: bool,
    pub catalog_dir: String,
    pub default_revision: Option<String>,
    pub revisions: Option<Vec<String>>,
    /// `(ref, comparedRevisions)`, present only when a base ref was requested.
    pub base_ref: Option<(String, usize)>,
    pub errors: Vec<String>,
}

impl CatalogValidationReport {
    /// The report as JSON (absent fields omitted).
    pub fn to_json(&self) -> Json {
        let mut o = JsonObject::new();
        o.insert("ok", Json::Bool(self.ok));
        o.insert("catalogDir", Json::String(self.catalog_dir.clone()));
        if let Some(d) = &self.default_revision {
            o.insert("defaultRevision", Json::String(d.clone()));
        }
        if let Some(r) = &self.revisions {
            o.insert(
                "revisions",
                Json::Array(r.iter().map(|s| Json::String(s.clone())).collect()),
            );
        }
        if let Some((reference, compared)) = &self.base_ref {
            let mut b = JsonObject::new();
            b.insert("ref", Json::String(reference.clone()));
            b.insert("comparedRevisions", Json::Number(*compared as f64));
            o.insert("baseRef", Json::Object(b));
        }
        o.insert(
            "errors",
            Json::Array(
                self.errors
                    .iter()
                    .map(|s| Json::String(s.clone()))
                    .collect(),
            ),
        );
        Json::Object(o)
    }
}

fn host_path_platform() -> Platform {
    if cfg!(windows) {
        Platform::Windows
    } else {
        Platform::Linux
    }
}

/// Node `path.resolve(value)` against the current directory.
pub fn resolve_path(value: &str) -> String {
    let platform = host_path_platform();
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let sep = if cfg!(windows) { "\\" } else { "/" };
    let joined = if value.is_empty() {
        cwd
    } else if is_absolute_path(value, platform) {
        let bytes = value.as_bytes();
        let rooted_without_drive = cfg!(windows)
            && (bytes[0] == b'\\' || bytes[0] == b'/')
            && !(bytes.len() > 1 && (bytes[1] == b'\\' || bytes[1] == b'/'));
        if rooted_without_drive && cwd.len() >= 2 && cwd.as_bytes()[1] == b':' {
            format!("{}{value}", &cwd[..2])
        } else {
            value.to_string()
        }
    } else {
        format!("{cwd}{sep}{value}")
    };
    normalize_path(&joined, platform)
}

/// `fs.realpathSync.native` (without the Windows verbatim prefix).
fn realpath(path: &str) -> std::io::Result<String> {
    let canonical = std::fs::canonicalize(path)?;
    let text = canonical.to_string_lossy().into_owned();
    Ok(if let Some(rest) = text.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{rest}")
    } else if let Some(rest) = text.strip_prefix("\\\\?\\") {
        rest.to_string()
    } else {
        text
    })
}

fn git(args: &[&str], cwd: &str) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn git_succeeds(args: &[&str], cwd: &str) -> bool {
    git(args, cwd).is_ok()
}

/// A failure reported as `[policy_validation] <message>` (TypeScript wraps
/// non-library exceptions that way, which equals an invalid_catalog message).
fn validation_error(message: impl Into<String>) -> PolicyCatalogError {
    invalid_catalog(message.into())
}

fn segments(path: &str) -> Vec<&str> {
    let separators: &[char] = if cfg!(windows) { &['\\', '/'] } else { &['/'] };
    path.split(separators).filter(|s| !s.is_empty()).collect()
}

/// Node `path.relative(top, dir)` for a `dir` inside `top`, with `/`
/// separators; `None` when `dir` is outside `top`. Windows compares
/// case-insensitively, like Node's win32 `relative`.
fn relative_inside(top: &str, dir: &str) -> Option<String> {
    let platform = host_path_platform();
    let top_n = normalize_path(top, platform);
    let dir_n = normalize_path(dir, platform);
    let top_s = segments(&top_n);
    let dir_s = segments(&dir_n);
    let same = |a: &str, b: &str| {
        if cfg!(windows) {
            a.to_lowercase() == b.to_lowercase()
        } else {
            a == b
        }
    };
    if top_s.len() > dir_s.len() || top_s.iter().zip(&dir_s).any(|(a, b)| !same(a, b)) {
        return None;
    }
    Some(dir_s[top_s.len()..].join("/"))
}

/// Reads the published catalog state at `base_ref` for the git repository
/// containing `catalog_dir`. `Ok(None)` when the ref has no catalog there.
pub fn read_published_state_at_ref(
    catalog_dir: &str,
    base_ref: &str,
) -> Result<Option<PublishedState>, PolicyCatalogError> {
    if base_ref.is_empty()
        || base_ref.starts_with('-')
        || base_ref
            .chars()
            .any(|c| c == '\0' || crate::text::js_is_space(c))
    {
        return Err(validation_error(format!(
            "base-ref check: '{base_ref}' is not a valid git ref"
        )));
    }
    let dir = realpath(catalog_dir)
        .map_err(|e| validation_error(format!("{e}, realpath '{catalog_dir}'")))?;
    let top = git(&["rev-parse", "--show-toplevel"], &dir)
        .ok()
        .and_then(|out| realpath(out.trim()).ok())
        .ok_or_else(|| {
            validation_error(format!(
                "base-ref check: '{catalog_dir}' is not inside a git work tree"
            ))
        })?;
    let commit = format!("{base_ref}^{{commit}}");
    if !git_succeeds(&["rev-parse", "--verify", "--quiet", &commit], &top) {
        return Err(validation_error(format!(
            "base-ref check: '{base_ref}' does not name a commit"
        )));
    }
    let Some(prefix) = relative_inside(&top, &dir) else {
        return Err(validation_error(format!(
            "base-ref check: '{catalog_dir}' is outside its git work tree"
        )));
    };
    let at = |file: &str| {
        if prefix.is_empty() {
            format!("{base_ref}:{file}")
        } else {
            format!("{base_ref}:{prefix}/{file}")
        }
    };
    if !git_succeeds(&["cat-file", "-e", &at("manifest.json")], &top) {
        return Ok(None);
    }
    let manifest_text = git(&["show", &at("manifest.json")], &top).map_err(validation_error)?;
    let manifest = Json::parse(&manifest_text).map_err(|e| validation_error(e.message))?;
    let revisions = PublishedState::revisions_from_manifest(&manifest);
    let mut files = HashMap::new();
    for revision in &revisions {
        let file = match &revision.file {
            Some(Json::String(s)) => s.clone(),
            Some(other) => crate::json::js_to_string(other),
            None => "undefined".to_string(),
        };
        if git_succeeds(&["cat-file", "-e", &at(&file)], &top) {
            let text = git(&["show", &at(&file)], &top).map_err(validation_error)?;
            if let Some(Json::String(key)) = &revision.file {
                files.insert(key.clone(), text);
            }
        }
    }
    Ok(Some(PublishedState { revisions, files }))
}

/// Compares a catalog directory's manifest and files with the revisions
/// published at `base_ref`. Never fails; problems are returned as errors.
pub fn check_against_base_ref(
    catalog_dir: &str,
    manifest: &CatalogManifest,
    base_ref: &str,
) -> (usize, Vec<String>) {
    let dir = resolve_path(catalog_dir);
    let base = match read_published_state_at_ref(&dir, base_ref) {
        Ok(None) => return (0, Vec::new()),
        Ok(Some(base)) => base,
        Err(error) => return (0, vec![error.message().to_string()]),
    };
    let mut files = HashMap::new();
    let revisions: Vec<PublishedRevision> = manifest
        .revisions
        .iter()
        .map(|r| {
            let text = read_text(&Path::new(&dir).join(&r.file)).unwrap_or_default();
            files.insert(r.file.clone(), text);
            PublishedRevision::new(&r.catalog_revision, &r.file, &r.sha256)
        })
        .collect();
    let proposed = PublishedState { revisions, files };
    let errors = check_published_immutability(&base, &proposed)
        .into_iter()
        .map(|message| format!("[immutability] {message}"))
        .collect();
    (base.revisions.len(), errors)
}

fn run_checks(
    store: Result<CatalogStore, PolicyCatalogError>,
    dir: String,
    base_dir: &str,
    base_ref: Option<&str>,
) -> CatalogValidationReport {
    let mut report = CatalogValidationReport {
        ok: false,
        catalog_dir: dir,
        default_revision: None,
        revisions: None,
        base_ref: None,
        errors: Vec::new(),
    };
    let store = match store {
        Ok(store) => store,
        Err(error) => {
            report.errors.push(error.message().to_string());
            return report;
        }
    };
    report.default_revision = Some(store.default_revision().to_string());
    report.revisions = Some(store.available_revisions());
    report.errors.extend(check_store_history(&store));
    if let Some(base_ref) = base_ref {
        let (compared, errors) = check_against_base_ref(base_dir, store.manifest(), base_ref);
        report.base_ref = Some((base_ref.to_string(), compared));
        report.errors.extend(errors);
    }
    report.ok = report.errors.is_empty();
    report
}

/// Validates a catalog directory: manifest and contract, per-revision
/// integrity, the full contract, entry-revision history, and, with
/// `base_ref`, immutability of every revision published at that ref.
pub fn validate_catalog_directory(
    catalog_dir: &str,
    base_ref: Option<&str>,
) -> CatalogValidationReport {
    let dir = resolve_path(catalog_dir);
    let store = DirectorySource::open(PathBuf::from(&dir)).and_then(CatalogStore::new);
    run_checks(store, dir.clone(), &dir, base_ref)
}

/// Validates the catalog embedded in this crate. `catalogDir` in the report
/// (and the base for `--base-ref`) is the directory it was embedded from.
pub fn validate_bundled_catalog(base_ref: Option<&str>) -> CatalogValidationReport {
    let dir = resolve_path(crate::store::bundled_catalog_source_dir());
    let store = crate::store::bundled_catalog_source().and_then(CatalogStore::new);
    run_checks(store, dir.clone(), &dir, base_ref)
}
