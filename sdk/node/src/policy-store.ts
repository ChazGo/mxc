// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

/**
 * **PROTOTYPE, pending API review.** The MXC policy store: resolve known tools
 * to a candidate floor {@link SandboxPolicy} from the policy catalog bundled
 * statically in this SDK's native library.
 *
 * The result is a best-effort floor, not a guarantee: the access a known tool
 * typically needs, which a caller composes with its own policy. It is
 * complementary to Learning Mode, not a replacement. Resolution never grants
 * access, launches a sandbox, contacts a network service, or writes state; the
 * V1 catalog is compiled into `mxc_ffi` and nothing is downloaded.
 *
 * Names and shapes are proposed and may change before sign-off (for example,
 * the names may drop "Sandbox", and lookup may gain an intent such as
 * `git pull` versus `git push`). This API is not part of MXC 1.0.
 *
 * All four functions run in-process through `mxc_ffi`, so they require the
 * native library that ships with this package.
 *
 * @module
 */

import { callPolicyStore, inspectPolicyStore } from './bindings/policy-store.js';
import type { SandboxPolicy } from './types.js';

/** Catalog platform selector values. */
export type CatalogPlatform = 'windows' | 'linux' | 'macos';

/** Catalog architecture selector values. */
export type CatalogArchitecture = 'x64' | 'arm64';

/** A tool to look up, identified by invocation name and optional identity. */
export interface ToolCandidate {
  /** The command name, for example `git`. */
  invocationName: string;
  /** A Package URL, for example `pkg:npm/npm`. A strong identity. */
  packageUrl?: string;
  /** The detected tool version, used to filter `packageUrl` version ranges. */
  detectedVersion?: string;
}

/** A string is shorthand for `{ invocationName: tool }`. */
export type ToolInput = string | ToolCandidate;

/** Lookup context shared by every input in one lookup. */
export interface ResolveContext {
  /** Substituted for the `project_root` catalog symbol. */
  projectRoot?: string;
  /** Caller-supplied catalog symbol values, for example `git_prefix`. */
  symbols?: Record<string, string>;
  /** Defaults to the current host platform. */
  platform?: CatalogPlatform;
  /** Defaults to the current host architecture. */
  architecture?: CatalogArchitecture;
  /** Defaults to the bundled catalog's default revision. */
  catalogRevision?: string;
  /** Allow entries matched only by invocation name. Defaults to `false`. */
  allowWeakIdentityFallback?: boolean;
}

/** Strength of a matched identity predicate. */
export type IdentityStrength = 'strong' | 'weak';

/** Match attribution and warnings from one resolution pass. */
export interface PolicyResolutionDiagnostics {
  catalogRevision: string;
  tools: Array<{
    inputIndex: number;
    matches: Array<{
      entryId: string;
      entryRevision: number;
      matchedIdentities: Array<{ kind: string; strength: IdentityStrength }>;
    }>;
  }>;
  resolvedDependencies: Array<{
    entryId: string;
    entryRevision: number;
    requiredVersionRange?: string;
  }>;
  warnings: string[];
}

/** Result of {@link resolveSandboxPolicyWithDiagnostics}. */
export interface SandboxConfigResolution {
  /** The composed policy, or `undefined` when nothing could be resolved. */
  policy: SandboxPolicy | undefined;
  diagnostics: PolicyResolutionDiagnostics;
}

/** One identity predicate of a catalog entry. */
export type CatalogIdentityMetadata =
  | { kind: 'purl'; value: string; versionRange?: string }
  | { kind: 'invocation-name'; names: string[] };

/** Inspection metadata for one catalog entry. It never exposes a policy body. */
export interface CatalogEntryMetadata {
  catalogRevision: string;
  entryId: string;
  entryRevision: number;
  displayName: string;
  identity: CatalogIdentityMetadata[];
  platformVariants: Array<{
    platform: CatalogPlatform;
    architecture?: CatalogArchitecture;
    dependencyEntryIds: string[];
    sandboxPolicyVersion: string;
  }>;
  provenance: { method: string; sourceRevision: string };
}

/** The bundled catalog's schema version and default revision. */
export interface CatalogInfo {
  catalogSchemaVersion: string;
  catalogRevision: string;
}

/**
 * **PROTOTYPE, pending API review.** Resolves one tool or a list of tools to a
 * single composed floor policy from the bundled catalog.
 *
 * @returns The policy, or `undefined` when no policy can be resolved.
 * @throws {MxcError} with the stable failure reason in `details.reason`:
 *   `malformed_request` (`invalid_context`) for invalid input or context,
 *   `unsupported_containment` (`unsupported_host`), `policy_validation`
 *   (`invalid_catalog`, `composition_conflict`), or `backend_error` (`integrity`,
 *   `revision_unavailable`).
 */
export function resolveSandboxPolicy(
  tools: ToolInput | ToolInput[],
  context: ResolveContext = {},
): SandboxPolicy | undefined {
  const result = callPolicyStore('mxc_resolve_sandbox_policy_json', {
    tools,
    context,
  }) as { policy?: SandboxPolicy };
  return result.policy;
}

/**
 * **PROTOTYPE, pending API review.** Like {@link resolveSandboxPolicy}, and
 * also reports which catalog entries matched each input, the dependencies
 * they pulled in, and warnings.
 */
export function resolveSandboxPolicyWithDiagnostics(
  tools: ToolInput | ToolInput[],
  context: ResolveContext = {},
): SandboxConfigResolution {
  const result = callPolicyStore('mxc_resolve_sandbox_policy_with_diagnostics_json', {
    tools,
    context,
  }) as { policy?: SandboxPolicy; diagnostics: PolicyResolutionDiagnostics };
  return { policy: result.policy, diagnostics: result.diagnostics };
}

/** **PROTOTYPE, pending API review.** The bundled catalog's schema version and default revision. */
export function getCatalogInfo(): CatalogInfo {
  return inspectPolicyStore('mxc_policy_catalog_info_json') as CatalogInfo;
}

/**
 * **PROTOTYPE, pending API review.** Metadata for every entry in the bundled
 * catalog's default revision. It never exposes a policy body.
 */
export function listCatalogEntries(): CatalogEntryMetadata[] {
  return inspectPolicyStore('mxc_list_policy_catalog_entries_json') as CatalogEntryMetadata[];
}
