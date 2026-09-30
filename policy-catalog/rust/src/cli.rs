// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! `policy-catalog resolve | inspect | validate`, output-equivalent to the
//! TypeScript CLI (`src/cli.ts`).
//!
//! ```text
//! policy-catalog resolve [--catalog DIR] [--diagnostics] [--platform P]
//!     [--architecture A] [--revision R] [--project-root PATH]
//!     [--symbol name=value]... [--allow-weak]
//!     [--purl URL] [--detected-version V] <tool>...
//! policy-catalog inspect [--catalog DIR]
//! policy-catalog validate [--catalog DIR] [--base-ref REF]
//! ```
//!
//! Exit codes: 0 success (including "no policy"), 1 library failure or
//! failed validation, 2 usage error. Output is JSON on stdout.

use crate::errors::PolicyCatalogError;
use crate::json::{Json, JsonObject};
use crate::model::{ResolveContext, SymbolMap, ToolCandidate, ToolInput, ToolInputs};
use crate::resolver::PolicyCatalog;
use crate::store::{bundled_catalog_store, load_catalog_directory, CatalogStore};
use crate::validate::{resolve_path, validate_bundled_catalog, validate_catalog_directory};
use std::sync::Arc;

pub const USAGE: &str = "usage: policy-catalog <resolve|inspect|validate> [options]";

enum Failure {
    Usage(String),
    Library(PolicyCatalogError),
}

impl From<PolicyCatalogError> for Failure {
    fn from(error: PolicyCatalogError) -> Self {
        Failure::Library(error)
    }
}

/// Captured process output of one CLI invocation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CliOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

fn take_value(args: &[String], index: usize, flag: &str) -> Result<String, Failure> {
    match args.get(index + 1) {
        Some(value) if !value.starts_with("--") => Ok(value.clone()),
        _ => Err(Failure::Usage(format!("{flag} requires a value"))),
    }
}

fn store_from(catalog_dir: &Option<String>) -> Result<Arc<CatalogStore>, PolicyCatalogError> {
    match catalog_dir {
        None => bundled_catalog_store(),
        Some(dir) => Ok(Arc::new(load_catalog_directory(resolve_path(dir))?)),
    }
}

fn print(value: &Json, out: &mut String) {
    out.push_str(&value.to_pretty_string());
    out.push('\n');
}

fn reject_rest(args: &[String], command: &str) -> Result<(), Failure> {
    match args.first() {
        Some(first) => Err(Failure::Usage(format!("{command}: unexpected argument '{first}'"))),
        None => Ok(()),
    }
}

/// Runs the CLI over `argv` (without the program name) and returns its output.
pub fn run(argv: &[String]) -> CliOutput {
    let mut output = CliOutput::default();
    match main_inner(argv, &mut output.stdout) {
        Ok(code) => output.exit_code = code,
        Err(Failure::Usage(message)) => {
            output.stdout.clear();
            output.stderr = format!("policy-catalog: {message}\n{USAGE}\n");
            output.exit_code = 2;
        }
        Err(Failure::Library(error)) => {
            let mut details = JsonObject::new();
            details.insert("reason", error.reason().as_str().into());
            let mut body = JsonObject::new();
            body.insert("code", error.code().as_str().into());
            body.insert("message", error.message().into());
            body.insert("details", Json::Object(details));
            let mut root = JsonObject::new();
            root.insert("error", Json::Object(body));
            output.stdout.clear();
            print(&Json::Object(root), &mut output.stdout);
            output.exit_code = 1;
        }
    }
    output
}

fn main_inner(argv: &[String], out: &mut String) -> Result<i32, Failure> {
    let command = argv.first().map(String::as_str);
    let rest = argv.get(1..).unwrap_or_default();
    let mut catalog_dir: Option<String> = None;
    let mut base_ref: Option<String> = None;
    let mut args: Vec<String> = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == "--catalog" {
            catalog_dir = Some(take_value(rest, i, "--catalog")?);
            i += 1;
        } else if rest[i] == "--base-ref" && command == Some("validate") {
            base_ref = Some(take_value(rest, i, "--base-ref")?);
            i += 1;
        } else {
            args.push(rest[i].clone());
        }
        i += 1;
    }
    match command {
        Some("inspect") => {
            reject_rest(&args, "inspect")?;
            let catalog = PolicyCatalog::new(store_from(&catalog_dir)?);
            let info = catalog.get_catalog_info()?;
            let entries = catalog.list_catalog_entries()?;
            let mut root = JsonObject::new();
            root.insert("info", info.to_json());
            root.insert("entries", Json::Array(entries.iter().map(|e| e.to_json()).collect()));
            print(&Json::Object(root), out);
            Ok(0)
        }
        Some("validate") => {
            reject_rest(&args, "validate")?;
            let report = match &catalog_dir {
                Some(dir) => validate_catalog_directory(dir, base_ref.as_deref()),
                None => validate_bundled_catalog(base_ref.as_deref()),
            };
            print(&report.to_json(), out);
            Ok(if report.ok { 0 } else { 1 })
        }
        Some("resolve") => resolve_command(&args, &catalog_dir, out),
        None => Err(Failure::Usage("missing command".to_string())),
        Some(other) => Err(Failure::Usage(format!("unknown command '{other}'"))),
    }
}

fn resolve_command(args: &[String], catalog_dir: &Option<String>, out: &mut String) -> Result<i32, Failure> {
    let mut ctx = ResolveContext::new();
    let mut symbols = SymbolMap::new();
    let mut tools: Vec<ToolInput> = Vec::new();
    let mut diagnostics = false;
    let mut purl: Option<String> = None;
    let mut detected_version: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--diagnostics" => diagnostics = true,
            "--allow-weak" => ctx.allow_weak_identity_fallback = true,
            "--platform" => {
                ctx.platform = Some(take_value(args, i, arg)?);
                i += 1;
            }
            "--architecture" => {
                ctx.architecture = Some(take_value(args, i, arg)?);
                i += 1;
            }
            "--revision" => {
                ctx.catalog_revision = Some(take_value(args, i, arg)?);
                i += 1;
            }
            "--project-root" => {
                ctx.project_root = Some(take_value(args, i, arg)?);
                i += 1;
            }
            "--symbol" => {
                let pair = take_value(args, i, arg)?;
                i += 1;
                match pair.find('=') {
                    Some(eq) if eq > 0 => symbols.insert(&pair[..eq], &pair[eq + 1..]),
                    _ => return Err(Failure::Usage("--symbol expects name=value".to_string())),
                }
            }
            "--purl" => {
                purl = Some(take_value(args, i, arg)?);
                i += 1;
            }
            "--detected-version" => {
                detected_version = Some(take_value(args, i, arg)?);
                i += 1;
            }
            _ => {
                if arg.starts_with("--") {
                    return Err(Failure::Usage(format!("unknown option '{arg}'")));
                }
                // --purl / --detected-version apply to the next tool name only.
                if purl.is_none() && detected_version.is_none() {
                    tools.push(ToolInput::Name(arg.to_string()));
                } else {
                    tools.push(ToolInput::Candidate(ToolCandidate {
                        invocation_name: arg.to_string(),
                        package_url: purl.take(),
                        detected_version: detected_version.take(),
                    }));
                }
            }
        }
        i += 1;
    }
    if purl.is_some() || detected_version.is_some() {
        return Err(Failure::Usage(
            "--purl/--detected-version must precede a tool name".to_string(),
        ));
    }
    if !symbols.is_empty() {
        ctx.symbols = Some(symbols);
    }
    let catalog = PolicyCatalog::new(store_from(catalog_dir)?);
    let tools = ToolInputs(tools);
    if diagnostics {
        print(
            &catalog.get_sandbox_config_with_diagnostics(tools, &ctx)?.to_json(),
            out,
        );
    } else {
        let policy = catalog.get_sandbox_config(tools, &ctx)?;
        print(&policy.map_or(Json::Null, |p| p.to_json()), out);
    }
    Ok(0)
}
