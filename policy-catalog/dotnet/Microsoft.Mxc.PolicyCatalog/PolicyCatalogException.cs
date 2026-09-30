// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

namespace Microsoft.Mxc.PolicyCatalog;

/// <summary>
/// MXC error codes reused by the policy catalog (MXC's closed <c>MxcError</c> code set).
/// The library adds no code of its own. <see cref="PolicyCatalogException.Code"/> carries the
/// snake_case wire form.
/// </summary>
public enum PolicyCatalogErrorCode
{
    /// <summary><c>policy_validation</c>: catalog data or the selected composition violates the catalog contract.</summary>
    PolicyValidation,

    /// <summary><c>malformed_request</c>: the caller's <see cref="ResolveContext"/> or <see cref="ToolInput"/> is structurally invalid.</summary>
    MalformedRequest,

    /// <summary><c>unsupported_containment</c>: the host platform or native architecture has no catalog selector, or cannot be detected.</summary>
    UnsupportedContainment,

    /// <summary><c>backend_error</c>: installed catalog data is unavailable, unreadable, or fails integrity.</summary>
    BackendError,
}

/// <summary>
/// Stable, language-neutral sub-reason carried in <c>details.reason</c>. Every reason maps to exactly one
/// <see cref="PolicyCatalogErrorCode"/>. <see cref="PolicyCatalogException.Reason"/> carries the snake_case wire form.
/// </summary>
public enum PolicyCatalogErrorReason
{
    /// <summary><c>invalid_catalog</c> (<c>policy_validation</c>): catalog, manifest, or contract data is invalid.</summary>
    InvalidCatalog,

    /// <summary><c>composition_conflict</c> (<c>policy_validation</c>): selected entries or resolved paths cannot be composed.</summary>
    CompositionConflict,

    /// <summary><c>invalid_context</c> (<c>malformed_request</c>): invalid resolve context or tool input.</summary>
    InvalidContext,

    /// <summary><c>unsupported_host</c> (<c>unsupported_containment</c>): host platform/architecture has no selector or cannot be detected.</summary>
    UnsupportedHost,

    /// <summary><c>integrity</c> (<c>backend_error</c>): catalog data does not match its published digest, or cannot be read.</summary>
    Integrity,

    /// <summary><c>revision_unavailable</c> (<c>backend_error</c>): an explicitly requested catalog revision is not installed.</summary>
    RevisionUnavailable,
}

/// <summary>A policy catalog library failure. A failure is never a "no match" result.</summary>
public sealed class PolicyCatalogException : Exception
{
    /// <summary>Creates a failure for <paramref name="reason"/>; <see cref="Exception.Message"/> becomes <c>[code] message</c>.</summary>
    /// <param name="reason">The stable sub-reason; it determines the code.</param>
    /// <param name="message">The message text without the code prefix.</param>
    public PolicyCatalogException(PolicyCatalogErrorReason reason, string message)
        : base($"[{ToWire(CodeFor(reason))}] {message}")
    {
        ErrorReason = reason;
        ErrorCode = CodeFor(reason);
    }

    /// <summary>The MXC error code.</summary>
    public PolicyCatalogErrorCode ErrorCode { get; }

    /// <summary>The stable sub-reason.</summary>
    public PolicyCatalogErrorReason ErrorReason { get; }

    /// <summary>The MXC error code in its wire form, e.g. <c>backend_error</c>.</summary>
    public string Code => ToWire(ErrorCode);

    /// <summary>The stable sub-reason in its wire form (<c>details.reason</c>), e.g. <c>integrity</c>.</summary>
    public string Reason => ToWire(ErrorReason);

    /// <summary>The one code each reason maps to (TypeScript <c>ERROR_CODE_FOR_REASON</c>).</summary>
    /// <param name="reason">A sub-reason.</param>
    /// <returns>The code for <paramref name="reason"/>.</returns>
    public static PolicyCatalogErrorCode CodeFor(PolicyCatalogErrorReason reason) => reason switch
    {
        PolicyCatalogErrorReason.InvalidCatalog => PolicyCatalogErrorCode.PolicyValidation,
        PolicyCatalogErrorReason.CompositionConflict => PolicyCatalogErrorCode.PolicyValidation,
        PolicyCatalogErrorReason.InvalidContext => PolicyCatalogErrorCode.MalformedRequest,
        PolicyCatalogErrorReason.UnsupportedHost => PolicyCatalogErrorCode.UnsupportedContainment,
        PolicyCatalogErrorReason.Integrity => PolicyCatalogErrorCode.BackendError,
        PolicyCatalogErrorReason.RevisionUnavailable => PolicyCatalogErrorCode.BackendError,
        _ => throw new ArgumentOutOfRangeException(nameof(reason)),
    };

    /// <summary>The snake_case wire form of a code.</summary>
    /// <param name="code">A code.</param>
    /// <returns>For example <c>policy_validation</c>.</returns>
    public static string ToWire(PolicyCatalogErrorCode code) => code switch
    {
        PolicyCatalogErrorCode.PolicyValidation => "policy_validation",
        PolicyCatalogErrorCode.MalformedRequest => "malformed_request",
        PolicyCatalogErrorCode.UnsupportedContainment => "unsupported_containment",
        PolicyCatalogErrorCode.BackendError => "backend_error",
        _ => throw new ArgumentOutOfRangeException(nameof(code)),
    };

    /// <summary>The snake_case wire form of a reason.</summary>
    /// <param name="reason">A reason.</param>
    /// <returns>For example <c>invalid_catalog</c>.</returns>
    public static string ToWire(PolicyCatalogErrorReason reason) => reason switch
    {
        PolicyCatalogErrorReason.InvalidCatalog => "invalid_catalog",
        PolicyCatalogErrorReason.CompositionConflict => "composition_conflict",
        PolicyCatalogErrorReason.InvalidContext => "invalid_context",
        PolicyCatalogErrorReason.UnsupportedHost => "unsupported_host",
        PolicyCatalogErrorReason.Integrity => "integrity",
        PolicyCatalogErrorReason.RevisionUnavailable => "revision_unavailable",
        _ => throw new ArgumentOutOfRangeException(nameof(reason)),
    };
}
