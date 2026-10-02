// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! The prototype policy store C ABI.

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

const GIT_LINUX: &str = r#"{"tools":"git","context":{"platform":"linux","architecture":"x64","allowWeakIdentityFallback":true,"projectRoot":"/w","symbols":{"git_prefix":"/usr/bin"}}}"#;

#[test]
fn resolves_a_policy() {
    let (status, json, reason, message) = call(mxc_resolve_sandbox_policy_json, GIT_LINUX);
    assert_eq!(status, MXC_STATUS_SUCCESS, "{message:?}");
    assert_eq!(reason, None);
    let json: serde_json::Value = serde_json::from_str(&json.unwrap()).unwrap();
    assert_eq!(json["policy"]["version"], "0.9.0-alpha");
    assert_eq!(json["policy"]["filesystem"]["readonlyPaths"][0], "/usr/bin");
}

#[test]
fn absence_omits_the_policy() {
    let (status, json, _, _) = call(
        mxc_resolve_sandbox_policy_json,
        r#"{"tools":"unknown-tool","context":{"platform":"linux","architecture":"x64"}}"#,
    );
    assert_eq!(status, MXC_STATUS_SUCCESS);
    assert_eq!(json.as_deref(), Some("{}"));
}

#[test]
fn diagnostics_include_attribution() {
    let (status, json, _, _) = call(mxc_resolve_sandbox_policy_with_diagnostics_json, GIT_LINUX);
    assert_eq!(status, MXC_STATUS_SUCCESS);
    let json: serde_json::Value = serde_json::from_str(&json.unwrap()).unwrap();
    assert_eq!(
        json["diagnostics"]["tools"][0]["matches"][0]["entryId"],
        "tool:git"
    );
    assert!(json["policy"].is_object());
}

#[test]
fn intent_and_version_cross_the_boundary() {
    let (status, json, _, message) = call(
        mxc_resolve_sandbox_policy_with_diagnostics_json,
        r#"{"tools":{"invocationName":"git","packageUrl":"pkg:generic/git","detectedVersion":"2.30.0","intent":"fetch"},"context":{"platform":"linux","architecture":"x64","projectRoot":"/w","symbols":{"git_prefix":"/usr/bin"}}}"#,
    );
    assert_eq!(status, MXC_STATUS_SUCCESS, "{message:?}");
    let json: serde_json::Value = serde_json::from_str(&json.unwrap()).unwrap();
    let tool = &json["diagnostics"]["tools"][0];
    assert_eq!(tool["status"], "version_out_of_range");
    assert_eq!(tool["matches"][0]["intentSelection"]["requested"], "fetch");
    assert_eq!(
        json["diagnostics"]["warnings"][0]["code"],
        "version_out_of_range"
    );
    assert_eq!(
        json["policy"]["network"]["egress"]["allow"][0]["ports"][0]["port"],
        443
    );
}

#[test]
fn store_failures_carry_status_and_reason() {
    let (status, json, reason, message) = call(
        mxc_resolve_sandbox_policy_json,
        r#"{"tools":"git","context":{"platform":"solaris"}}"#,
    );
    assert_eq!(status, MXC_STATUS_MALFORMED_REQUEST);
    assert_eq!(json, None);
    assert_eq!(reason.as_deref(), Some("invalid_context"));
    assert!(message.unwrap().starts_with("[malformed_request] "));

    let (status, _, reason, _) = call(
        mxc_resolve_sandbox_policy_json,
        r#"{"tools":"git","context":{"catalogRevision":"1999-01-01.1"}}"#,
    );
    assert_eq!(status, MXC_STATUS_BACKEND_ERROR);
    assert_eq!(reason.as_deref(), Some("revision_unavailable"));

    let (status, _, reason, _) = call(mxc_resolve_sandbox_policy_json, "not json");
    assert_eq!(status, MXC_STATUS_MALFORMED_REQUEST);
    assert_eq!(reason.as_deref(), Some("invalid_context"));
}

#[test]
fn null_and_non_utf8_arguments_are_rejected() {
    let mut out = MaybeUninit::<MxcPolicyStoreResult>::uninit();
    let status = unsafe { mxc_resolve_sandbox_policy_json(ptr::null(), out.as_mut_ptr()) };
    assert_eq!(status, MXC_STATUS_NULL_ARGUMENT);
    let mut out = unsafe { out.assume_init() };
    let (_, reason, _) = take(&mut out);
    assert_eq!(reason, None);

    let bad = CString::new(vec![0xffu8, 0xfe]).unwrap();
    let mut out = MaybeUninit::<MxcPolicyStoreResult>::uninit();
    let status = unsafe { mxc_resolve_sandbox_policy_json(bad.as_ptr(), out.as_mut_ptr()) };
    assert_eq!(status, MXC_STATUS_INVALID_UTF8);
    take(&mut unsafe { out.assume_init() });

    let request = CString::new("{}").unwrap();
    let status = unsafe { mxc_resolve_sandbox_policy_json(request.as_ptr(), ptr::null_mut()) };
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

    let mut out = MaybeUninit::<MxcPolicyStoreResult>::uninit();
    assert_eq!(
        unsafe { mxc_list_policy_catalog_entries_json(out.as_mut_ptr()) },
        MXC_STATUS_SUCCESS
    );
    let (json, _, _) = take(&mut unsafe { out.assume_init() });
    let entries: serde_json::Value = serde_json::from_str(&json.unwrap()).unwrap();
    assert!(entries.as_array().unwrap().len() >= 3);
}
