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
 * Each catalog entry has one unversioned default plus additive platform,
 * version (purl `vers` range), and intent overlays. A {@link ToolCandidate}
 * may carry a `detectedVersion` and an `intent` (for example `fetch` versus
 * `push` for git); each tool and intent pair resolves independently, and the
 * pairs compose into one floor.
 *
 * Names and shapes are proposed and may change before sign-off (for example,
 * the names may drop "Sandbox"). This API is not part of MXC 1.0.
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
  /**
   * The command name, for example `git`. A weak identity, used only when
   * `allowWeakIdentityFallback` is set. Matching is case-insensitive on
   * Windows and macOS and exact on Linux.
   */
  invocationName: string;
  /**
   * A Package URL without a version, for example `pkg:npm/npm`. A strong
   * identity. A version embedded in the purl is ignored with a warning.
   */
  packageUrl?: string;
  /**
   * The tool version the caller detected. It selects at most one reviewed
   * version range; no other version evidence is inferred.
   */
  detectedVersion?: string;
  /**
   * The intended operation, for example `fetch` or `push`. When omitted, the
   * base policy plus every intent of the effective policy applies. An intent
   * the effective policy does not define contributes nothing.
   */
  intent?: string;
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

/** How a detected version selected the effective policy. */
export type VersionStatus =
  | 'matched_default'
  | 'matched_version'
  | 'version_out_of_range'
  | 'version_unparseable';

/** Per-input status: the version status, or why the input contributes nothing. */
export type ToolResolutionStatus = VersionStatus | 'intent_unsupported' | 'tool_unmatched';

/** Version selection for one matched entry. */
export interface VersionSelection {
  status: VersionStatus;
  detectedVersion?: string;
  /** Present only for `matched_version`. */
  selectedVersionRange?: string;
}

/**
 * Intent selection for one matched entry or resolved dependency. A
 * dependency reports `none` (base only) unless its reference names intents,
 * which report `named`.
 */
export interface IntentSelection {
  requested?: string;
  mode: 'named' | 'all' | 'none' | 'unsupported';
  /** Selected intent names, sorted; empty when unsupported or none. */
  selected: string[];
}

/** A structured per-input warning. */
export interface ToolResolutionWarning {
  code: 'version_out_of_range' | 'version_unparseable' | 'intent_unsupported' | 'tool_unmatched';
  inputIndex: number;
  entryId?: string;
  detectedVersion?: string;
  intent?: string;
  message: string;
}

/** A diagnostics warning: structured per input, or free text. */
export type PolicyResolutionWarning = string | ToolResolutionWarning;

/** Match attribution and warnings from one resolution pass. */
export interface PolicyResolutionDiagnostics {
  catalogRevision: string;
  tools: Array<{
    inputIndex: number;
    status: ToolResolutionStatus;
    /** At most one entry; empty for `tool_unmatched`. */
    matches: Array<{
      entryId: string;
      entryRevision: number;
      matchedIdentities: Array<{ kind: string; strength: IdentityStrength }>;
      versionSelection: VersionSelection;
      /** Absent when the version could not be parsed. */
      intentSelection?: IntentSelection;
    }>;
  }>;
  resolvedDependencies: Array<{
    entryId: string;
    entryRevision: number;
    requiredVersionRange?: string;
    versionSelection: VersionSelection;
    intentSelection: IntentSelection;
  }>;
  warnings: PolicyResolutionWarning[];
}

/** Result of {@link resolveSandboxPolicyWithDiagnostics}. */
export interface SandboxConfigResolution {
  /** The composed policy, or `undefined` when nothing could be resolved. */
  policy: SandboxPolicy | undefined;
  diagnostics: PolicyResolutionDiagnostics;
}

/** One identity predicate of a catalog entry. */
export type CatalogIdentityMetadata =
  | { kind: 'purl'; value: string }
  | { kind: 'invocation-name'; names: string[] };

/** The version scheme an entry's `vers` ranges use. */
export type CatalogVersionScheme = 'npm' | 'semver' | 'pypi' | 'nuget' | 'intdot';

/** An intent an entry or overlay defines or extends. */
export interface CatalogIntentMetadata {
  name: string;
  exampleSubcommands?: string[];
  dependencyEntryIds: string[];
}

/** What an overlay adds. Overlays are additive only. */
export interface CatalogAdditionsMetadata {
  dependencyEntryIds: string[];
  /** Extensions of intents the default declares. */
  intentAdditions: CatalogIntentMetadata[];
  /** Intents the overlay introduces. */
  newIntents: CatalogIntentMetadata[];
}

/** Inspection metadata for one catalog entry. It never exposes a policy body. */
export interface CatalogEntryMetadata {
  catalogRevision: string;
  entryId: string;
  entryRevision: number;
  displayName: string;
  versionScheme: CatalogVersionScheme;
  identity: CatalogIdentityMetadata[];
  default: {
    dependencyEntryIds: string[];
    sandboxPolicyVersion: string;
    intents: CatalogIntentMetadata[];
  };
  platformVariants: Array<
    { platform: CatalogPlatform; architecture?: CatalogArchitecture } & CatalogAdditionsMetadata
  >;
  versionVariants: Array<{ versionRange: string } & CatalogAdditionsMetadata>;
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
 *   (`invalid_catalog`, `composition_conflict`, `ambiguous_match`), or `backend_error` (`integrity`,
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
