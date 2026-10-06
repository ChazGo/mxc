// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Error codes reused from MXC's closed `MxcError` code set, plus a stable,
//! language-neutral sub-reason carried in `details.reason` by the SDKs. Warnings stay plain strings.

use std::fmt;

/// MXC error code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    /// Catalog data or the selected composition violates the catalog contract.
    PolicyValidation,
    /// The caller's context or tool input is structurally invalid.
    MalformedRequest,
    /// The host platform or native architecture has no selector or cannot be detected.
    UnsupportedContainment,
    /// Installed catalog data is unavailable, unreadable, or fails integrity.
    BackendError,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::PolicyValidation => "policy_validation",
            ErrorCode::MalformedRequest => "malformed_request",
            ErrorCode::UnsupportedContainment => "unsupported_containment",
            ErrorCode::BackendError => "backend_error",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Stable sub-reason (`details.reason`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorReason {
    InvalidCatalog,
    CompositionConflict,
    AmbiguousMatch,
    InvalidContext,
    UnsupportedHost,
    Integrity,
    RevisionUnavailable,
}

impl ErrorReason {
    pub const ALL: [ErrorReason; 7] = [
        ErrorReason::InvalidCatalog,
        ErrorReason::CompositionConflict,
        ErrorReason::AmbiguousMatch,
        ErrorReason::InvalidContext,
        ErrorReason::UnsupportedHost,
        ErrorReason::Integrity,
        ErrorReason::RevisionUnavailable,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ErrorReason::InvalidCatalog => "invalid_catalog",
            ErrorReason::CompositionConflict => "composition_conflict",
            ErrorReason::AmbiguousMatch => "ambiguous_match",
            ErrorReason::InvalidContext => "invalid_context",
            ErrorReason::UnsupportedHost => "unsupported_host",
            ErrorReason::Integrity => "integrity",
            ErrorReason::RevisionUnavailable => "revision_unavailable",
        }
    }

    /// The one code each reason maps to (`ERROR_CODE_FOR_REASON`).
    pub fn code(self) -> ErrorCode {
        match self {
            ErrorReason::InvalidCatalog
            | ErrorReason::CompositionConflict
            | ErrorReason::AmbiguousMatch => ErrorCode::PolicyValidation,
            ErrorReason::InvalidContext => ErrorCode::MalformedRequest,
            ErrorReason::UnsupportedHost => ErrorCode::UnsupportedContainment,
            ErrorReason::Integrity | ErrorReason::RevisionUnavailable => ErrorCode::BackendError,
        }
    }
}

impl fmt::Display for ErrorReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A library failure. `message()` is `[<code>] <message>`, exactly as in the original TypeScript prototype.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyCatalogError {
    reason: ErrorReason,
    message: String,
}

impl PolicyCatalogError {
    pub fn new(reason: ErrorReason, message: impl AsRef<str>) -> Self {
        Self {
            reason,
            message: format!("[{}] {}", reason.code(), message.as_ref()),
        }
    }

    pub fn code(&self) -> ErrorCode {
        self.reason.code()
    }

    pub fn reason(&self) -> ErrorReason {
        self.reason
    }

    /// Full message including the `[code] ` prefix.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The message without the `[code] ` prefix.
    pub fn detail(&self) -> &str {
        self.message
            .split_once("] ")
            .map_or(self.message.as_str(), |(_, rest)| rest)
    }
}

impl fmt::Display for PolicyCatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PolicyCatalogError {}

pub type Result<T, E = PolicyCatalogError> = std::result::Result<T, E>;

pub(crate) fn invalid_catalog(message: impl AsRef<str>) -> PolicyCatalogError {
    PolicyCatalogError::new(ErrorReason::InvalidCatalog, message)
}

pub(crate) fn invalid_context(message: impl AsRef<str>) -> PolicyCatalogError {
    PolicyCatalogError::new(ErrorReason::InvalidContext, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_is_prefixed_with_code() {
        let error = PolicyCatalogError::new(ErrorReason::RevisionUnavailable, "x");
        assert_eq!(error.message(), "[backend_error] x");
        assert_eq!(error.code(), ErrorCode::BackendError);
        let codes: Vec<&str> = ErrorReason::ALL.iter().map(|r| r.code().as_str()).collect();
        assert_eq!(
            codes,
            [
                "policy_validation",
                "policy_validation",
                "policy_validation",
                "malformed_request",
                "unsupported_containment",
                "backend_error",
                "backend_error"
            ]
        );
    }
}
