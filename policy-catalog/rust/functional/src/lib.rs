// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Helpers for the packaged-crate functional tests. Everything resolves the
//! library from the extracted `.crate` (a `path` dependency outside the
//! repository) and the CLI from the binary built from that same crate
//! (`POLICY_CATALOG_BIN`). Nothing reads the repository.

use mxc_policy_catalog::tooling::{bundled_catalog_files, canonical_sha256, Json, JsonObject};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Result of one CLI run.
#[derive(Debug)]
pub struct CliResult {
    pub status: i32,
    pub json: Option<Json>,
    pub stdout: String,
    pub stderr: String,
}

impl CliResult {
    pub fn warnings(&self) -> String {
        self.json
            .as_ref()
            .and_then(|j| j.get("diagnostics"))
            .and_then(|d| d.get("warnings"))
            .and_then(Json::as_array)
            .map(|w| w.iter().filter_map(Json::as_str).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default()
    }

    pub fn error_reason(&self) -> Option<String> {
        self.json
            .as_ref()?
            .get("error")?
            .get("details")?
            .get("reason")?
            .as_str()
            .map(str::to_string)
    }

    pub fn errors(&self) -> String {
        self.json
            .as_ref()
            .and_then(|j| j.get("errors"))
            .and_then(Json::as_array)
            .map(|e| e.iter().filter_map(Json::as_str).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default()
    }
}

/// The packaged `policy-catalog` binary.
pub fn cli_path() -> PathBuf {
    PathBuf::from(std::env::var("POLICY_CATALOG_BIN").expect("POLICY_CATALOG_BIN is set by scripts/rust-functional.mjs"))
}

/// Runs the packaged CLI in a separate process.
pub fn cli(args: &[&str]) -> CliResult {
    let output = Command::new(cli_path()).args(args).output().expect("run policy-catalog");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let json = if stdout.trim().is_empty() { None } else { Json::parse(&stdout).ok() };
    CliResult {
        status: output.status.code().unwrap_or(-1),
        json,
        stdout,
        stderr,
    }
}

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A fresh directory under the driver's work directory (outside the repository).
pub fn work_dir(prefix: &str) -> PathBuf {
    let base = std::env::var("POLICY_CATALOG_WORK").map_or_else(|_| std::env::temp_dir(), PathBuf::from);
    let dir = base.join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn j(text: &str) -> Json {
    Json::parse(text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

pub fn s(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Writes the catalog shipped inside the packaged crate, byte for byte.
pub fn copy_bundled(dir: &Path) -> PathBuf {
    for (name, text) in bundled_catalog_files() {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    dir.to_path_buf()
}

/// Writes a complete catalog directory (packaged contract, correct digests),
/// so the only defect is the one a test introduces.
pub fn write_catalog(dir: &Path, revisions: &[Json]) -> PathBuf {
    std::fs::create_dir_all(dir.join("revisions")).unwrap();
    let contract = bundled_catalog_files().into_iter().find(|(n, _)| *n == "contract.v1.json").unwrap().1;
    std::fs::write(dir.join("contract.v1.json"), contract).unwrap();
    let mut listed = Vec::new();
    for revision in revisions {
        let id = revision.get("catalogRevision").and_then(Json::as_str).unwrap();
        let file = format!("revisions/{id}.json");
        std::fs::write(dir.join(&file), format!("{}\n", revision.to_pretty_string())).unwrap();
        listed.push(j(&format!(
            r#"{{"catalogRevision":"{id}","file":"{file}","sha256":"{}"}}"#,
            canonical_sha256(revision)
        )));
    }
    let mut manifest = JsonObject::new();
    manifest.insert("catalogSchemaVersion", "1".into());
    let last = revisions.last().unwrap().get("catalogRevision").unwrap().clone();
    manifest.insert("defaultRevision", last);
    manifest.insert("revisions", Json::Array(listed));
    std::fs::write(dir.join("manifest.json"), Json::Object(manifest).to_pretty_string()).unwrap();
    dir.to_path_buf()
}

/// Minimal valid entry (linux, readonly `${git_prefix}/<name>`) with overrides.
pub fn entry(entry_id: &str, overrides: &str) -> Json {
    let name = entry_id.split(':').nth(1).unwrap();
    let mut base = j(&format!(
        r#"{{"entryId":"{entry_id}","entryRevision":1,"displayName":"{name}",
            "identity":[{{"kind":"invocation-name","names":["{name}"]}}],
            "platformVariants":[{{"when":{{"platform":"linux"}},"sandboxPolicy":{{"version":"0.9.0-alpha","filesystem":{{"readonlyPaths":["${{git_prefix}}/{name}"]}}}}}}],
            "provenance":{{"method":"functional-test","sourceRevision":"functional-test"}}}}"#
    ));
    if !overrides.is_empty() {
        if let (Json::Object(target), Json::Object(extra)) = (&mut base, j(overrides)) {
            for (k, v) in extra.iter() {
                target.insert(k, v.clone());
            }
        }
    }
    base
}

pub fn revision(entries: Vec<Json>, id: &str) -> Json {
    let mut o = JsonObject::new();
    o.insert("catalogSchemaVersion", "1".into());
    o.insert("catalogRevision", id.into());
    o.insert("entries", Json::Array(entries));
    Json::Object(o)
}

/// Resolve flags with every bundled symbol for `platform`.
pub fn full_context(platform: &str, architecture: &str) -> Vec<String> {
    let (root, prefix, cache) = match platform {
        "windows" => ("C:\\work\\app", "C:\\tools", "C:\\cache\\npm"),
        "macos" => ("/Users/dev/app", "/opt/homebrew/bin", "/Users/dev/.npm"),
        _ => ("/work/app", "/opt/tools", "/var/cache/npm"),
    };
    let mut args: Vec<String> = vec!["--platform".into(), platform.into()];
    if !architecture.is_empty() {
        args.extend(["--architecture".into(), architecture.into()]);
    }
    args.extend(
        [
            "--allow-weak",
            "--project-root",
            root,
            "--symbol",
            &format!("git_prefix={prefix}"),
            "--symbol",
            &format!("node_prefix={prefix}"),
            "--symbol",
            &format!("npm_prefix={prefix}"),
            "--symbol",
            &format!("npm_cache={cache}"),
        ]
        .map(str::to_string),
    );
    args
}

pub fn args<'a>(parts: &'a [&'a str], extra: &'a [String]) -> Vec<&'a str> {
    let mut all: Vec<&str> = parts.to_vec();
    all.extend(extra.iter().map(String::as_str));
    all
}
