// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Embeds the catalog into the crate.
//!
//! Source of truth: `../catalog` (the repository's `policy-catalog/catalog`).
//! It is used when present and this is not a packaged crate. A packaged
//! crate (identified by the `Cargo.toml.orig` cargo writes into every
//! `.crate`) always uses its crate-local `catalog/`, which
//! `scripts/sync-catalog.mjs` copies from `../catalog` before
//! `cargo package`; that directory is git-ignored and listed in
//! `Cargo.toml` `include` so it ships in the `.crate`.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let repo_catalog = manifest_dir.join("..").join("catalog");
    let local_catalog = manifest_dir.join("catalog");
    let packaged = manifest_dir.join("Cargo.toml.orig").exists();

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", local_catalog.display());
    println!("cargo:rerun-if-changed={}", repo_catalog.display());

    let source = if !packaged && repo_catalog.join("manifest.json").is_file() {
        repo_catalog
    } else if local_catalog.join("manifest.json").is_file() {
        local_catalog
    } else {
        panic!(
            "no catalog to embed: expected {} (repository) or {} (run `node scripts/sync-catalog.mjs` before `cargo package`)",
            repo_catalog.display(),
            local_catalog.display()
        );
    };
    let source = fs::canonicalize(&source).expect("canonicalize catalog directory");
    let display = strip_verbatim(&source);

    let revisions_dir = source.join("revisions");
    let mut revisions: Vec<String> = fs::read_dir(&revisions_dir)
        .expect("read catalog/revisions")
        .map(|e| e.expect("dir entry").file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".json"))
        .collect();
    revisions.sort();

    let mut code = String::new();
    let lit = |p: &Path| format!("{:?}", p.to_string_lossy());
    for file in ["contract.v1.json", "manifest.json"] {
        println!("cargo:rerun-if-changed={}", source.join(file).display());
    }
    writeln!(code, "/// Directory the catalog was embedded from.").unwrap();
    writeln!(code, "pub const SOURCE_DIR: &str = {:?};", display).unwrap();
    writeln!(
        code,
        "pub const CONTRACT: &str = include_str!({});",
        lit(&source.join("contract.v1.json"))
    )
    .unwrap();
    writeln!(
        code,
        "pub const MANIFEST: &str = include_str!({});",
        lit(&source.join("manifest.json"))
    )
    .unwrap();
    writeln!(code, "pub const REVISIONS: &[(&str, &str)] = &[").unwrap();
    for name in &revisions {
        let path = revisions_dir.join(name);
        println!("cargo:rerun-if-changed={}", path.display());
        writeln!(
            code,
            "    ({:?}, include_str!({})),",
            format!("revisions/{name}"),
            lit(&path)
        )
        .unwrap();
    }
    writeln!(code, "];").unwrap();

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("bundled_catalog.rs");
    fs::write(out, code).expect("write bundled_catalog.rs");
}

fn strip_verbatim(path: &Path) -> String {
    let text = path.to_string_lossy().into_owned();
    if let Some(rest) = text.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{rest}")
    } else if let Some(rest) = text.strip_prefix("\\\\?\\") {
        rest.to_string()
    } else {
        text
    }
}
