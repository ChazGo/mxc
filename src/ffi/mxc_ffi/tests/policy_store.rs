// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! The prototype policy store C ABI. Lookups target the current host so
//! filesystem object identity can be established for existing paths.

use std::ffi::{CStr, CString};
use std::mem::MaybeUninit;
use std::ptr;

use mxc_ffi::*;

fn take(result: &mut MxcPolicyStoreResult) -> (Option<String>, Option<String>, Option<String>) {
    let read = |p: *mut std::ffi::c_char| {
        (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
    };
    let values = (
        read(result.json_utf8),
        read(result.reason_utf8),
        read(result.error.message_utf8),
    );
    unsafe {
        mxc_policy_store_result_free(result);
        // Idempotent.
        mxc_policy_store_result_free(result);
    }
    assert!(result.json_utf8.is_null() && result.reason_utf8.is_null());
    values
}

fn call(
    f: unsafe extern "C" fn(*const std::ffi::c_char, *mut MxcPolicyStoreResult) -> i32,
    request: &str,
) -> (i32, Option<String>, Option<String>, Option<String>) {
    let request = CString::new(request).unwrap();
    let mut out = MaybeUninit::<MxcPolicyStoreResult>::uninit();
    let status = unsafe { f(request.as_ptr(), out.as_mut_ptr()) };
    let mut out = unsafe { out.assume_init() };
    assert_eq!(out.status, status);
    let (json, reason, message) = take(&mut out);
    (status, json, reason, message)
}

/// Existing host directories for the git entry's symbols.
struct Dirs {
    _root: tempfile::TempDir,
    project: String,
    git: String,
    ssh: String,
    program_data: String,
}

fn dirs() -> Dirs {
    let root = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        let path = root.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path.to_str().unwrap().to_owned()
    };
    let (project, git, ssh, program_data) = (make("w"), make("git"), make("ssh"), make("pd"));
    std::fs::create_dir_all(root.path().join("pd").join("Git")).unwrap();
    Dirs {
        project,
        git,
        ssh,
        program_data,
        _root: root,
    }
}

fn request(tools: serde_json::Value, dirs: &Dirs, weak: bool) -> String {
    serde_json::json!({
        "tools": tools,
        "context": {
            "allowWeakIdentityFallback": weak,
            "projectRoot": dirs.project,
            "symbols": {
                "git_prefix": dirs.git,
                "ssh_prefix": dirs.ssh,
                "programData": dirs.program_data,
            }
        }
    })
    .to_string()
}

fn git(dirs: &Dirs) -> String {
    request(serde_json::json!("git"), dirs, true)
}

#[test]
fn resolves_requirements() {
    let dirs = dirs();
    let (status, json, reason, message) = call(mxc_resolve_tool_requirements_json, &git(&dirs));
    assert_eq!(status, MXC_STATUS_SUCCESS, "{message:?}");
    assert_eq!(reason, None);
    let json: serde_json::Value = serde_json::from_str(&json.unwrap()).unwrap();
    let requirements = &json["requirements"];
    assert!(requirements.get("version").is_none());
    assert!(requirements.get("command").is_none());
    assert_eq!(
        requirements["filesystem"]["readonlyPaths"][0],
        dirs.git.as_str()
    );
    assert_eq!(
        requirements["filesystem"]["readwritePaths"][0],
        dirs.project.as_str()
    );
}

#[test]
fn absence_omits_the_requirements() {
    let (status, json, _, _) = call(
        mxc_resolve_tool_requirements_json,
        r#"{"tools":"unknown-tool"}"#,
    );
    assert_eq!(status, MXC_STATUS_SUCCESS);
    assert_eq!(json.as_deref(), Some("{}"));
}

#[test]
fn diagnostics_include_attribution() {
    let dirs = dirs();
    let (status, json, _, _) = call(
        mxc_resolve_tool_requirements_with_diagnostics_json,
        &git(&dirs),
    );
    assert_eq!(status, MXC_STATUS_SUCCESS);
    let json: serde_json::Value = serde_json::from_str(&json.unwrap()).unwrap();
    assert_eq!(
        json["diagnostics"]["tools"][0]["matches"][0]["entryId"],
        "tool:git"
    );
    assert!(json["requirements"].is_object());
}

#[test]
fn intent_and_version_cross_the_boundary() {
    let dirs = dirs();
    let (status, json, _, message) = call(
        mxc_resolve_tool_requirements_with_diagnostics_json,
        &request(
            serde_json::json!({"invocationName":"git","packageUrl":"pkg:generic/git","detectedVersion":"2.30.0","intent":"fetch"}),
            &dirs,
            false,
        ),
    );
    assert_eq!(status, MXC_STATUS_SUCCESS, "{message:?}");
    let json: serde_json::Value = serde_json::from_str(&json.unwrap()).unwrap();
    let tool = &json["diagnostics"]["tools"][0];
    assert_eq!(tool["status"], "version_out_of_range");
    assert_eq!(tool["matches"][0]["intentSelection"]["requested"], "fetch");
    let warning = &json["diagnostics"]["warnings"][0];
    assert_eq!(warning["code"], "version_out_of_range");
    assert_eq!(warning["inputIndex"], 0);
    assert_eq!(warning["detectedVersion"], "2.30.0");
    assert_eq!(
        json["requirements"]["network"]["egress"]["allow"][0]["ports"][0]["port"],
        443
    );
}

#[test]
fn dependency_intent_selection_crosses_the_boundary() {
    let dirs = dirs();
    let (status, json, _, message) = call(
        mxc_resolve_tool_requirements_with_diagnostics_json,
        &request(
            serde_json::json!({"invocationName":"git","packageUrl":"pkg:generic/git","detectedVersion":"2.45.1","intent":"push"}),
            &dirs,
            false,
        ),
    );
    assert_eq!(status, MXC_STATUS_SUCCESS, "{message:?}");
    let json: serde_json::Value = serde_json::from_str(&json.unwrap()).unwrap();
    let dependency = &json["diagnostics"]["resolvedDependencies"][0];
    assert_eq!(dependency["entryId"], "tool:ssh");
    assert_eq!(dependency["inputIndexes"], serde_json::json!([0]));
    assert_eq!(dependency["intentSelection"]["mode"], "none");
    assert_eq!(
        dependency["intentSelection"]["selected"],
        serde_json::json!([])
    );
}

#[test]
fn store_failures_carry_status_and_reason() {
    let (status, json, reason, message) = call(
        mxc_resolve_tool_requirements_json,
        r#"{"tools":"git","context":{"platform":"solaris"}}"#,
    );
    assert_eq!(status, MXC_STATUS_MALFORMED_REQUEST);
    assert_eq!(json, None);
    assert_eq!(reason.as_deref(), Some("invalid_context"));
    assert!(message.unwrap().starts_with("[malformed_request] "));

    let (status, _, reason, _) = call(
        mxc_resolve_tool_requirements_json,
        r#"{"tools":"git","context":{"catalogRevision":"1999-01-01.1"}}"#,
    );
    assert_eq!(status, MXC_STATUS_BACKEND_ERROR);
    assert_eq!(reason.as_deref(), Some("revision_unavailable"));

    let (status, _, reason, _) = call(mxc_resolve_tool_requirements_json, "not json");
    assert_eq!(status, MXC_STATUS_MALFORMED_REQUEST);
    assert_eq!(reason.as_deref(), Some("invalid_context"));
}

#[test]
fn null_and_non_utf8_arguments_are_rejected() {
    let mut out = MaybeUninit::<MxcPolicyStoreResult>::uninit();
    let status = unsafe { mxc_resolve_tool_requirements_json(ptr::null(), out.as_mut_ptr()) };
    assert_eq!(status, MXC_STATUS_NULL_ARGUMENT);
    let mut out = unsafe { out.assume_init() };
    let (_, reason, _) = take(&mut out);
    assert_eq!(reason, None);

    let bad = CString::new(vec![0xffu8, 0xfe]).unwrap();
    let mut out = MaybeUninit::<MxcPolicyStoreResult>::uninit();
    let status = unsafe { mxc_resolve_tool_requirements_json(bad.as_ptr(), out.as_mut_ptr()) };
    assert_eq!(status, MXC_STATUS_INVALID_UTF8);
    take(&mut unsafe { out.assume_init() });

    let request = CString::new("{}").unwrap();
    let status = unsafe { mxc_resolve_tool_requirements_json(request.as_ptr(), ptr::null_mut()) };
    assert_eq!(status, MXC_STATUS_NULL_ARGUMENT);
    unsafe { mxc_policy_store_result_free(ptr::null_mut()) };
}

#[test]
fn inspection_entry_points() {
    let mut out = MaybeUninit::<MxcPolicyStoreResult>::uninit();
    assert_eq!(
        unsafe { mxc_policy_catalog_info_json(out.as_mut_ptr()) },
        MXC_STATUS_SUCCESS
    );
    let (json, _, _) = take(&mut unsafe { out.assume_init() });
    let info: serde_json::Value = serde_json::from_str(&json.unwrap()).unwrap();
    assert_eq!(info["catalogSchemaVersion"], "1");
    assert_eq!(info["sdkContractVersion"], "1.0.0");

    let mut out = MaybeUninit::<MxcPolicyStoreResult>::uninit();
    assert_eq!(
        unsafe { mxc_list_policy_catalog_entries_json(out.as_mut_ptr()) },
        MXC_STATUS_SUCCESS
    );
    let (json, _, _) = take(&mut unsafe { out.assume_init() });
    let entries: serde_json::Value = serde_json::from_str(&json.unwrap()).unwrap();
    assert!(entries.as_array().unwrap().len() >= 3);
}
