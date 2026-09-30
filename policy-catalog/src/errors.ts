// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

/**
 * Error codes, reused from MXC's closed `MxcError` code set (MXC
 * `sdk/node/src/errors.ts`, Rust `MxcErrorCode`) where the meaning fits.
 * The library adds no code of its own. Warnings stay plain strings, as they
 * do everywhere in MXC.
 */
export type PolicyCatalogErrorCode =
  /** Catalog data or the selected composition violates the catalog contract. */
  | 'policy_validation'
  /** The caller's ResolveContext or ToolCandidate is structurally invalid. */
  | 'malformed_request'
  /** The host platform or native architecture has no catalog selector, or cannot be detected. */
  | 'unsupported_containment'
  /** Installed catalog data is unavailable, unreadable, or fails integrity. */
  | 'backend_error';

/**
 * Stable, language-neutral sub-reason carried in `details.reason`, following
 * MXC's convention of putting structured information in `details`. Shared
 * conformance fixtures and the cross-language check assert on
 * `code` + `details.reason`, so every language binding must preserve both.
 * A library failure is never a "no match" result (design §5).
 */
export type PolicyCatalogErrorReason =
  /** policy_validation: catalog, manifest, or contract data is invalid (including dependency cycles and unknown symbols). */
  | 'invalid_catalog'
  /** policy_validation: selected entries or resolved paths cannot be composed. */
  | 'composition_conflict'
  /** malformed_request: invalid ResolveContext or ToolCandidate. */
  | 'invalid_context'
  /** unsupported_containment: host platform/architecture has no selector or cannot be detected. */
  | 'unsupported_host'
  /** backend_error: bundled data does not match its published digest, or cannot be read. */
  | 'integrity'
  /** backend_error: an explicitly requested catalog revision is not installed. */
  | 'revision_unavailable';

/** The one code each reason maps to. */
export const ERROR_CODE_FOR_REASON: Readonly<Record<PolicyCatalogErrorReason, PolicyCatalogErrorCode>> = {
  invalid_catalog: 'policy_validation',
  composition_conflict: 'policy_validation',
  invalid_context: 'malformed_request',
  unsupported_host: 'unsupported_containment',
  integrity: 'backend_error',
  revision_unavailable: 'backend_error',
};

export class PolicyCatalogError extends Error {
  readonly code: PolicyCatalogErrorCode;
  readonly details: { readonly reason: PolicyCatalogErrorReason };

  constructor(reason: PolicyCatalogErrorReason, message: string) {
    const code = ERROR_CODE_FOR_REASON[reason];
    super(`[${code}] ${message}`);
    this.name = 'PolicyCatalogError';
    this.code = code;
    this.details = Object.freeze({ reason });
  }

  /** The stable sub-reason; shorthand for `details.reason`. */
  get reason(): PolicyCatalogErrorReason {
    return this.details.reason;
  }
}
