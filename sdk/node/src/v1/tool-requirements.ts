// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

/**
 * **PROTOTYPE, pending API review.** The MXC policy store: resolve known tools
 * to command-free {@link ContainerRequirements} from the catalog bundled
 * statically in this SDK's native library.
 *
 * The result is a best-effort floor, not a guarantee: the access a known tool
 * typically needs, which a caller reviews, constrains, and combines with its
 * own command (`{ ...requirements, command }`) to build a
 * {@link ContainerRequest}. It is complementary to Learning Mode, not a
 * replacement. Resolution never grants access, launches a container, contacts
 * a network service, or writes state; the catalog is compiled into `mxc_ffi`
 * and nothing is downloaded.
 *
 * Each catalog entry has one unversioned default plus additive platform,
 * version (purl `vers` range), and intent overlays. A {@link ToolCandidate}
 * may carry a `detectedVersion` and an `intent` (for example `fetch`
 * versus `push` for git); each tool and intent pair resolves independently,
 * and the pairs compose into one set of requirements.
 *
 * Names and shapes are proposed and may change before sign-off. This API is
 * not part of MXC 1.0. Resolution returns a Promise because it examines host
 * filesystem objects; metadata inspection is synchronous. Every function runs
 * in-process through `mxc_ffi`.
 *
 * @module
 */

import { callPolicyStoreAsync, inspectPolicyStore } from '../bindings/policy-store.js';
import type { ContainerRequest, NetworkRuleConfig } from './types.js';

/** The access fields of a {@link ContainerRequest}, without a command. */
export type ContainerRequirements = Pick<
  ContainerRequest,
  'filesystem' | 'network' | 'ui' | 'timeoutMs'
>;

/** Catalog platform selector values. */
export type CatalogPlatform = 'windows' | 'linux' | 'macos';

/** Catalog architecture selector values. */
export type CatalogArchitecture = 'x64' | 'arm64';

/** The closed selector of a catalog platform variant. */
export interface PlatformVariantSelector {
  platform: CatalogPlatform;
  architecture?: CatalogArchitecture;
}

/** A tool to look up, identified by invocation name and optional identity. */
export interface ToolCandidate {
  /**
   * The command name, for example `git`. A weak identity, used only when
   * `allowWeakIdentityFallback` is set. Matching is case-insensitive on
   * Windows and macOS and exact on Linux.
   */
  invocationName: string;
  /**
   * A Package URL, for example `pkg:npm/npm`. A strong identity matched on
   * type, namespace, and name; a version, qualifiers, or subpath are ignored
   * with a `purl_components_ignored` warning. An invalid PURL makes the pair
   * `tool_unmatched` with a `purl_invalid` warning.
   */
  packageUrl?: string;
  /**
   * The tool version the caller detected. It selects at most one reviewed
   * version range; no other version evidence is inferred.
   */
  detectedVersion?: string;
  /**
   * The intended operation, for example `fetch` or `push`. When omitted,
   * the base plus every intent of the effective entry applies. An intent the
   * effective entry does not define contributes nothing.
   */
  intent?: string;
}

/** A string is shorthand for `{ invocationName: tool }`. */
export type ToolInput = string | ToolCandidate;

/** Lookup context shared by every input in one lookup. */
export interface ResolveContext {
  /** Substituted for the `project_root` catalog symbol. */
  projectRoot?: string;
  /**
   * Catalog symbol values, for example `git_prefix`. They take precedence
   * over host values, discovery, and contract defaults.
   */
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

/** How a detected version selected the entry's version data. */
export type VersionStatus =
  | 'matched_default'
  | 'matched_version'
  | 'version_out_of_range'
  | 'version_unparseable';

/** Per-input status: the version status, or why the pair contributes nothing. */
export type ToolResolutionStatus =
  | VersionStatus
  | 'intent_unsupported'
  | 'tool_unmatched'
  | 'filesystem_identity_unresolved';

/** Version selection for one matched entry or dependency. */
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

/** Fields every per-input warning carries. */
export interface InputWarning {
  inputIndex: number;
  /** Human-readable; not a parsing contract. */
  message: string;
}

/** A structured warning about one input; `code` selects its fields. */
export type ToolResolutionWarning = InputWarning &
  (
    | {
        code: 'version_out_of_range' | 'version_unparseable';
        entryId: string;
        detectedVersion: string;
      }
    | { code: 'intent_unsupported'; entryId: string; intent: string }
    | { code: 'tool_unmatched'; invocationName: string }
    | { code: 'purl_invalid'; packageUrl: string }
    | {
        code: 'purl_components_ignored';
        packageUrl: string;
        ignoredComponents: Array<'version' | 'qualifiers' | 'subpath'>;
      }
    | { code: 'weak_identity'; entryId: string; invocationName: string }
  );

/** Fields every shared-contribution warning carries. */
export interface WarningScope {
  /** Sorted, distinct contributing input indexes. */
  inputIndexes: number[];
  /** Sorted, distinct contributing entry IDs. */
  entryIds: string[];
  /** Human-readable; not a parsing contract. */
  message: string;
}

/** A resolved path with its access class and source entries. */
export interface PathRequirement {
  path: string;
  access: 'denied' | 'readonly' | 'readwrite';
  entryIds: string[];
}

/** An outbound rule the composed requirements carry. */
export type EgressRule = NetworkRuleConfig;

/** An egress rule with its source entries. */
export interface NetworkRequirement {
  rule: EgressRule;
  entryIds: string[];
}

/** A structured warning about shared contributions; `code` selects its fields. */
export type ResolutionDetailWarning = WarningScope &
  (
    | { code: 'architecture_default'; platform: CatalogPlatform; architecture: CatalogArchitecture }
    | {
        code: 'architecture_fallback';
        platform: CatalogPlatform;
        architecture: CatalogArchitecture;
        selected: 'platform' | 'default';
      }
    | {
        code: 'symbol_resolved';
        symbol: string;
        value: string;
        source: 'caller' | 'discovery' | 'host' | 'default';
      }
    | { code: 'symbol_unresolved'; symbol: string }
    | { code: 'filesystem_case_assumed'; paths: string[]; comparison: 'case_sensitive' }
    | { code: 'filesystem_identity_unresolved'; paths: string[]; platform: CatalogPlatform }
    | { code: 'readonly_superseded'; removed: PathRequirement; requiredBy: PathRequirement[] }
    | { code: 'filesystem_deny_removed'; removed: PathRequirement; requiredBy: PathRequirement[] }
    | {
        code: 'network_deny_removed';
        removed: NetworkRequirement;
        requiredBy: NetworkRequirement[];
      }
  );

/** Any diagnostics warning. */
export type PolicyResolutionWarning = ToolResolutionWarning | ResolutionDetailWarning;

/** Match attribution and warnings from one resolution pass. */
export interface ToolRequirementsDiagnostics {
  catalogRevision: string;
  tools: Array<{
    inputIndex: number;
    status: ToolResolutionStatus;
    /** At most one entry; empty for `tool_unmatched`. */
    matches: Array<{
      entryId: string;
      entryRevision: number;
      matchedIdentities: Array<{ kind: string; strength: 'strong' | 'weak' }>;
      versionSelection: VersionSelection;
      /** Absent when the version could not be parsed. */
      intentSelection?: IntentSelection;
    }>;
  }>;
  resolvedDependencies: Array<{
    entryId: string;
    entryRevision: number;
    /** Sorted, distinct indexes of every input that required this record. */
    inputIndexes: number[];
    requiredVersionRange?: string;
    versionSelection: VersionSelection;
    intentSelection: IntentSelection;
  }>;
  warnings: PolicyResolutionWarning[];
}

/** Result of {@link resolveToolRequirementsWithDiagnostics}. */
export interface ToolRequirementsResolution {
  /** The composed requirements, or `undefined` when none were resolved. */
  requirements: ContainerRequirements | undefined;
  diagnostics: ToolRequirementsDiagnostics;
}

/** One identity predicate of a catalog entry. */
export type CatalogIdentityMetadata =
  | { kind: 'purl'; value: string }
  | { kind: 'invocation-name'; names: string[] };

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

/** Inspection metadata for one catalog entry. It never exposes a requirements body. */
export interface CatalogEntryMetadata {
  catalogRevision: string;
  entryId: string;
  entryRevision: number;
  displayName: string;
  versionScheme: 'npm' | 'semver' | 'pypi' | 'nuget' | 'intdot';
  identity: CatalogIdentityMetadata[];
  default: {
    dependencyEntryIds: string[];
    intents: CatalogIntentMetadata[];
  };
  platformVariants: Array<
    CatalogAdditionsMetadata & PlatformVariantSelector
  >;
  versionVariants: Array<CatalogAdditionsMetadata & { versionRange: string }>;
  provenance: { method: string; sourceRevision: string };
}

/** The bundled catalog's schema version, default revision, and SDK contract version. */
export interface CatalogInfo {
  catalogSchemaVersion: string;
  catalogRevision: string;
  sdkContractVersion: string;
}

/**
 * **PROTOTYPE, pending API review.** Resolves one tool or a list of tools to
 * composed, command-free container requirements from the bundled catalog.
 * The result may cover only some of the requested tools; use
 * {@link resolveToolRequirementsWithDiagnostics} to see which contributed.
 *
 * @returns The requirements, or `undefined` when none were resolved.
 * @throws {MxcError} with the optional stable reason in `details.reason`:
 *   `malformed_request` (`invalid_context`), `unsupported_containment`
 *   (`unsupported_host`), `policy_validation` (`invalid_catalog`,
 *   `ambiguous_match`, `composition_conflict`), or `backend_error`
 *   (`integrity`, `revision_unavailable`).
 */
export function resolveToolRequirements(
  tool: ToolInput,
  context?: ResolveContext,
): Promise<ContainerRequirements | undefined>;
export function resolveToolRequirements(
  tools: readonly ToolInput[],
  context?: ResolveContext,
): Promise<ContainerRequirements | undefined>;
export async function resolveToolRequirements(
  tools: ToolInput | readonly ToolInput[],
  context: ResolveContext = {},
): Promise<ContainerRequirements | undefined> {
  const result = (await callPolicyStoreAsync('mxc_resolve_tool_requirements_json', {
    tools,
    context,
  })) as { requirements?: ContainerRequirements };
  return result.requirements;
}

/**
 * **PROTOTYPE, pending API review.** Like {@link resolveToolRequirements},
 * and also reports per-input statuses, the catalog entries and dependencies
 * that contributed, and structured warnings from the same pass.
 */
export function resolveToolRequirementsWithDiagnostics(
  tool: ToolInput,
  context?: ResolveContext,
): Promise<ToolRequirementsResolution>;
export function resolveToolRequirementsWithDiagnostics(
  tools: readonly ToolInput[],
  context?: ResolveContext,
): Promise<ToolRequirementsResolution>;
export async function resolveToolRequirementsWithDiagnostics(
  tools: ToolInput | readonly ToolInput[],
  context: ResolveContext = {},
): Promise<ToolRequirementsResolution> {
  const result = (await callPolicyStoreAsync(
    'mxc_resolve_tool_requirements_with_diagnostics_json',
    { tools, context },
  )) as { requirements?: ContainerRequirements; diagnostics: ToolRequirementsDiagnostics };
  return { requirements: result.requirements, diagnostics: result.diagnostics };
}

/**
 * **PROTOTYPE, pending API review.** The bundled catalog's schema version,
 * default revision, and SDK contract version.
 */
export function getCatalogInfo(): CatalogInfo {
  return inspectPolicyStore('mxc_policy_catalog_info_json') as CatalogInfo;
}

/**
 * **PROTOTYPE, pending API review.** Metadata for every entry in the bundled
 * catalog's default revision. It never exposes a requirements body.
 */
export function listCatalogEntries(): CatalogEntryMetadata[] {
  return inspectPolicyStore('mxc_list_policy_catalog_entries_json') as CatalogEntryMetadata[];
}
