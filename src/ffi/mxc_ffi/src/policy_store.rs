// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! **PROTOTYPE, pending API review.** The policy store over the C ABI, for
//! the Node and C# SDKs.
//!
//! Each entry point fills a caller-owned [`MxcPolicyStoreResult`]: on success
//! `json_utf8` holds the result document; on failure `reason_utf8` holds the
//! stable sub-reason (`invalid_context`, `integrity`, ...) and `error` the
//! message. Release every result with [`mxc_policy_store_result_free`].
//!
//! The resolve entry points take `{"tools": <input | input[]>, "context"?}`
//! (see `mxc_policy_store::request`). `mxc_resolve_sandbox_policy_json`
//! returns `{"policy"?}`, omitting `policy` when nothing resolves; the
//! diagnostics variant returns `{"policy"?, "diagnostics"}`. The policy JSON
//! is the SDK `SandboxPolicy` shape.

use std::ffi::c_char;
use std::panic::catch_unwind;
use std::ptr;

use mxc_sdk::policy_store::{binding, ErrorCode, PolicyCatalogError};

use crate::{
    alloc_cstring, cstr_to_str, free_cstr, report_panic, MxcErrorDetail, MXC_STATUS_BACKEND_ERROR,
    MXC_STATUS_INVALID_UTF8, MXC_STATUS_MALFORMED_REQUEST, MXC_STATUS_NULL_ARGUMENT,
    MXC_STATUS_PANIC, MXC_STATUS_POLICY_VALIDATION, MXC_STATUS_SUCCESS,
    MXC_STATUS_UNSUPPORTED_CONTAINMENT,
};

/// The result of a policy store entry point.
///
/// All non-null pointers, including those inside `error`, are owned by the
/// caller and must be released with [`mxc_policy_store_result_free`].
#[repr(C)]
pub struct MxcPolicyStoreResult {
    /// `0` on success; otherwise one of the `MXC_STATUS_*` codes.
    pub status: i32,
    /// The result document (UTF-8 JSON) on success, or null.
    pub json_utf8: *mut c_char,
    /// The stable failure sub-reason (`details.reason`) on a store failure,
    /// or null.
    pub reason_utf8: *mut c_char,
    /// Why the call failed, when `status != 0`; all-null otherwise.
    pub error: MxcErrorDetail,
}

impl MxcPolicyStoreResult {
    fn success(json: &str) -> Self {
        Self {
            status: MXC_STATUS_SUCCESS,
            json_utf8: alloc_cstring(json.as_bytes()),
            reason_utf8: ptr::null_mut(),
            error: MxcErrorDetail::none(),
        }
    }

    fn failure(status: i32, message: impl Into<String>) -> Self {
        Self {
            status,
            json_utf8: ptr::null_mut(),
            reason_utf8: ptr::null_mut(),
            error: MxcErrorDetail::from_message(message),
        }
    }

    fn from_store_error(error: &PolicyCatalogError) -> Self {
        Self {
            reason_utf8: alloc_cstring(error.reason().as_str().as_bytes()),
            ..Self::failure(status_from_store_code(error.code()), error.message())
        }
    }
}

fn status_from_store_code(code: ErrorCode) -> i32 {
    match code {
        ErrorCode::PolicyValidation => MXC_STATUS_POLICY_VALIDATION,
        ErrorCode::MalformedRequest => MXC_STATUS_MALFORMED_REQUEST,
        ErrorCode::UnsupportedContainment => MXC_STATUS_UNSUPPORTED_CONTAINMENT,
        ErrorCode::BackendError => MXC_STATUS_BACKEND_ERROR,
    }
}

fn complete(result: Result<String, PolicyCatalogError>) -> MxcPolicyStoreResult {
    match result {
        Ok(json) => MxcPolicyStoreResult::success(&json),
        Err(error) => MxcPolicyStoreResult::from_store_error(&error),
    }
}

/// Runs `body` panic-safely and writes its result to `out`.
///
/// # Safety
/// `out` must be null or point to writable [`MxcPolicyStoreResult`] storage.
unsafe fn write_result(
    operation: &str,
    out: *mut MxcPolicyStoreResult,
    body: impl FnOnce() -> MxcPolicyStoreResult + std::panic::UnwindSafe,
) -> i32 {
    if out.is_null() {
        return MXC_STATUS_NULL_ARGUMENT;
    }
    let result = catch_unwind(body).unwrap_or_else(|panic| {
        report_panic(operation, &*panic);
        MxcPolicyStoreResult::failure(MXC_STATUS_PANIC, "the mxc policy store panicked")
    });
    let status = result.status;
    // SAFETY: `out` is non-null and caller-guaranteed writable.
    unsafe { ptr::write(out, result) };
    status
}

/// Borrows the request, or produces the failure for a null or non-UTF-8 one.
///
/// # Safety
/// `request` must be null or a valid NUL-terminated C string.
unsafe fn request_str<'a>(request: *const c_char) -> Result<&'a str, MxcPolicyStoreResult> {
    match unsafe { cstr_to_str(request) } {
        Some(value) => Ok(value),
        None if request.is_null() => Err(MxcPolicyStoreResult::failure(
            MXC_STATUS_NULL_ARGUMENT,
            "request JSON pointer is null",
        )),
        None => Err(MxcPolicyStoreResult::failure(
            MXC_STATUS_INVALID_UTF8,
            "request JSON is not UTF-8",
        )),
    }
}

/// Resolves tools to one composed floor policy from the bundled catalog.
///
/// # Safety
/// - `request_json_utf8` must be null or valid NUL-terminated UTF-8.
/// - `out` must be null or point to writable [`MxcPolicyStoreResult`] storage,
///   released afterwards with [`mxc_policy_store_result_free`].
#[no_mangle]
pub unsafe extern "C" fn mxc_resolve_sandbox_policy_json(
    request_json_utf8: *const c_char,
    out: *mut MxcPolicyStoreResult,
) -> i32 {
    unsafe {
        write_result(
            "mxc_resolve_sandbox_policy_json",
            out,
            || match request_str(request_json_utf8) {
                Ok(request) => complete(binding::resolve_sandbox_policy_json(request)),
                Err(failure) => failure,
            },
        )
    }
}

/// Like [`mxc_resolve_sandbox_policy_json`], with match attribution,
/// dependencies, and warnings.
///
/// # Safety
/// Same contract as [`mxc_resolve_sandbox_policy_json`].
#[no_mangle]
pub unsafe extern "C" fn mxc_resolve_sandbox_policy_with_diagnostics_json(
    request_json_utf8: *const c_char,
    out: *mut MxcPolicyStoreResult,
) -> i32 {
    unsafe {
        write_result(
            "mxc_resolve_sandbox_policy_with_diagnostics_json",
            out,
            || match request_str(request_json_utf8) {
                Ok(request) => complete(binding::resolve_sandbox_policy_with_diagnostics_json(
                    request,
                )),
                Err(failure) => failure,
            },
        )
    }
}

/// The bundled catalog's `{"catalogSchemaVersion", "catalogRevision"}`.
///
/// # Safety
/// `out` must be null or point to writable [`MxcPolicyStoreResult`] storage,
/// released afterwards with [`mxc_policy_store_result_free`].
#[no_mangle]
pub unsafe extern "C" fn mxc_policy_catalog_info_json(out: *mut MxcPolicyStoreResult) -> i32 {
    unsafe {
        write_result("mxc_policy_catalog_info_json", out, || {
            complete(binding::catalog_info_json())
        })
    }
}

/// Metadata for every entry in the bundled catalog, as a JSON array.
///
/// # Safety
/// `out` must be null or point to writable [`MxcPolicyStoreResult`] storage,
/// released afterwards with [`mxc_policy_store_result_free`].
#[no_mangle]
pub unsafe extern "C" fn mxc_list_policy_catalog_entries_json(
    out: *mut MxcPolicyStoreResult,
) -> i32 {
    unsafe {
        write_result("mxc_list_policy_catalog_entries_json", out, || {
            complete(binding::list_catalog_entries_json())
        })
    }
}

/// Frees the owned strings of an [`MxcPolicyStoreResult`]. Idempotent.
///
/// # Safety
/// `r` must be null or point to a result filled by one of the policy store
/// entry points.
#[no_mangle]
pub unsafe extern "C" fn mxc_policy_store_result_free(r: *mut MxcPolicyStoreResult) {
    if r.is_null() {
        return;
    }
    if let Err(p) = catch_unwind(|| {
        // SAFETY: caller guarantees `r` points to a valid result.
        let r = unsafe { &mut *r };
        free_cstr(&mut r.json_utf8);
        free_cstr(&mut r.reason_utf8);
        r.error.free_strings();
    }) {
        report_panic("mxc_policy_store_result_free", &*p);
    }
}
