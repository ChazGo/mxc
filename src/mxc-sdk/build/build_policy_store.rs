// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Embeds the V1 policy catalog into the crate.
//!
//! `catalog/` beside this file is the single source of truth. Every MXC SDK
//! reaches it through this crate: Rust directly, and Node and C# through the
//! `mxc_ffi` native library they already ship.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

pub fn run() {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let source = manifest_dir.join("policy_store").join("catalog");

        println!("cargo:rerun-if-changed={}", source.display());

    assert!(
        source.join("manifest.json").is_file(),
        "no catalog to embed: expected {}",
        source.display()
    );
    let source = fs::canonicalize(&source).expect("canonicalize catalog directory");
    let display = strip_verbatim(&source);

    let revisions_dir = source.join("revisions");
    let mut revisions: Vec<String> = fs::read_dir(&revisions_dir)
        .expect("read catalog/revisions")
        .map(|e| {
            e.expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
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

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("policy_store_catalog.rs");
    fs::write(out, code).expect("write policy_store_catalog.rs");
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
