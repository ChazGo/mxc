# Policy Store API

**Status:** Under review. Pending MXC SDK API sign-off before implementation.
Not part of MXC 1.0; no later release is committed.

This document defines the caller-facing contract. The
[design spec](mxc-policy-store.md) owns catalog authoring, implementation,
validation, and release mechanics.

Policy Store resolves suggested requirements for one tool or a set of tools
before the execution command is known. Results are best-effort floors, not
authorization or guarantees of success or safety. Clients and users review
the requested access; their restrictions and enterprise policy take precedence.
Lookup never executes a tool, creates a container, downloads catalog data, or
writes consumer state.

## 1. Operations

| Operation | Result | Use |
|---|---|---|
| `resolveToolRequirements` | `ContainerRequirements` or absence | Resolve one tool or compose several tools' requirements |
| `resolveToolRequirementsWithDiagnostics` | Requirements plus diagnostics | Also inspect coverage, selections, dependencies, and warnings |
| `listCatalogEntries` | Metadata array | Inspect entries without resolving policy bodies |
| `getCatalogInfo` | Catalog/version metadata | Identify the installed default revision |

TypeScript signatures below define the shared contract. All named public
types and operations are exported from `@microsoft/mxc-sdk/v1`.

```ts
export declare function resolveToolRequirements(
  tool: ToolInput, ctx?: ResolveContext,
): Promise<ContainerRequirements | undefined>;
export declare function resolveToolRequirements(
  tools: readonly ToolInput[], ctx?: ResolveContext,
): Promise<ContainerRequirements | undefined>;

export declare function resolveToolRequirementsWithDiagnostics(
  tool: ToolInput, ctx?: ResolveContext,
): Promise<ToolRequirementsResolution>;
export declare function resolveToolRequirementsWithDiagnostics(
  tools: readonly ToolInput[], ctx?: ResolveContext,
): Promise<ToolRequirementsResolution>;

export declare function listCatalogEntries(): CatalogEntryMetadata[];
export declare function getCatalogInfo(): {
  catalogSchemaVersion: string;
  catalogRevision: string;
  sdkContractVersion: string;
};
```

Resolution is asynchronous in Node, with plain names and no `Async` suffix;
it must not block the event loop. Inspection is synchronous.

| SDK | Resolution operations | Return convention |
|---|---|---|
| Rust `mxc_sdk::v1` | `resolve_tool_requirements`, `resolve_tool_requirements_with_diagnostics` | `Result<Option<ContainerRequirements>, Error>` or `Result<ToolRequirementsResolution, Error>` |
| .NET `Microsoft.Mxc.Sdk.V1` | `MxcContainer.ResolveToolRequirements`, `ResolveToolRequirementsWithDiagnostics`; corresponding `Async` forms | Nullable requirements or diagnostic result; asynchronous forms return Tasks |
| Node `@microsoft/mxc-sdk/v1` | Signatures above | Promises; absence is `undefined` |

Rust uses an idiomatic one-or-many input rather than overloads. A single-tool
call equals a one-element array. Both resolution operations produce the same
requirements; diagnostics come from that pass, not a second lookup or global
"last result." Metadata inspection exposes selectors and provenance, not policy
bodies, and reports the installed default catalog.

## 2. Types and fields

### Requirements and inputs

```ts
import type { ContainerRequest, NetworkRuleConfig } from "@microsoft/mxc-sdk/v1";

export type ContainerRequirements = Pick<ContainerRequest,
  "filesystem" | "network" | "ui" | "timeoutMs">;

export interface ToolCandidate {
  invocationName: string;
  packageUrl?: string;
  detectedVersion?: string;
  intent?: string;
}

export type ToolInput = string | ToolCandidate;
export type CatalogPlatform = "windows" | "linux" | "macos";
export type CatalogArchitecture = "x64" | "arm64";

export interface PlatformVariantSelector {
  platform: CatalogPlatform;
  architecture?: CatalogArchitecture;
}

export interface ResolveContext {
  projectRoot?: string;
  symbols?: Record<string, string>;
  platform?: CatalogPlatform;
  architecture?: CatalogArchitecture;
  catalogRevision?: string;
  allowWeakIdentityFallback?: boolean;
}
```

`ContainerRequirements` reuses the SDK's existing nested types and optionality.
Rust/.NET use equivalent four-field aggregates, not duplicate nested models.
The resolver returns only catalog-supported fields:

| Field | Supported content |
|---|---|
| `filesystem` | `readonlyPaths`, `readwritePaths`, `deniedPaths` |
| `network` | Directional `egress` and `ingress` policy |
| `ui` | Required `disable` when UI is present; optional `clipboard` and `allowInputInjection` |
| `timeoutMs` | Unsigned 32-bit integer |

`Pick` is not runtime validation or permission to return other nested settings.
Commands, containment, name, working directory, environment, lifecycle/cleanup,
runtime proxy configuration, telemetry and execution options remain caller-owned.
No command is needed for lookup. Intent selects requirements, not a command.

A string input means `{ invocationName: tool }`, with no package/version/intent
evidence. The library does not verify an installed tool's identity or discover
its version from a supplied package URL.

### Results and selections

`IntentSelection.mode` reports how intent requirements were selected; it is not
a caller option. Effective requirements reflect the applicable
[version/platform selection](#version-intent-and-platform-selection) and
[dependency rules](#dependencies-and-composition).

| Mode | When reported | Requirements contributed | `selected` |
|---|---|---|---|
| `named` | The caller requested a supported intent, or a dependency reference explicitly named supported intents | Effective base plus only the named intents' additions | Selected intent names, sorted |
| `all` | A directly requested tool omitted `intent` | Effective base plus all effective intents; the base still contributes when no intents are defined | All effective intent names, sorted; `[]` when none exist |
| `none` | A dependency reference did not name intents | Applicable dependency base only, without intent additions | `[]` |
| `unsupported` | The requested intent is unavailable for the selected tool/version/platform | Nothing from that tool/intent pair; status is `intent_unsupported`, with no base/all-intents fallback | `[]` |

An unparseable version skips intent resolution.

```ts
export interface IntentSelection {
  requested?: string;
  mode: "named" | "all" | "none" | "unsupported";
  selected: string[];
}

export type VersionStatus =
  | "matched_default"
  | "matched_version"
  | "version_out_of_range"
  | "version_unparseable";

export interface VersionSelection {
  status: VersionStatus;
  detectedVersion?: string;
  selectedVersionRange?: string;
}

export type ToolResolutionStatus =
  | VersionStatus
  | "intent_unsupported"
  | "tool_unmatched"
  | "filesystem_identity_unresolved";

export interface ToolRequirementsResolution {
  requirements: ContainerRequirements | undefined;
  diagnostics: {
    catalogRevision: string;
    tools: Array<{
      inputIndex: number;
      status: ToolResolutionStatus;
      matches: Array<{
        entryId: string;
        entryRevision: number;
        matchedIdentities: Array<{
          kind: string;
          strength: "strong" | "weak";
        }>;
        versionSelection: VersionSelection;
        intentSelection?: IntentSelection;
      }>;
    }>;
    resolvedDependencies: Array<{
      entryId: string;
      entryRevision: number;
      inputIndexes: number[];
      requiredVersionRange?: string;
      versionSelection: VersionSelection;
      intentSelection: IntentSelection;
    }>;
    warnings: PolicyResolutionWarning[];
  };
}
```

### Warnings

Each warning has a stable `code`, category-specific fields, and a human
`message`. Consumers use fields, not message parsing.

```ts
export type InputWarning = { inputIndex: number; message: string };
export type ToolResolutionWarning = InputWarning & (
  | { code: "version_out_of_range" | "version_unparseable";
      entryId: string; detectedVersion: string }
  | { code: "intent_unsupported"; entryId: string; intent: string }
  | { code: "tool_unmatched"; invocationName: string }
  | { code: "purl_invalid"; packageUrl: string }
  | { code: "purl_components_ignored"; packageUrl: string;
      ignoredComponents: Array<"version" | "qualifiers" | "subpath"> }
  | { code: "weak_identity"; entryId: string; invocationName: string }
);

export interface WarningScope {
  inputIndexes: number[];
  entryIds: string[];
  message: string;
}

export interface PathRequirement {
  path: string;
  access: "denied" | "readonly" | "readwrite";
  entryIds: string[];
}

export type EgressRule = NetworkRuleConfig;

export interface NetworkRequirement {
  rule: EgressRule;
  entryIds: string[];
}

export type ResolutionDetailWarning = WarningScope & (
  | { code: "architecture_default"; platform: CatalogPlatform;
      architecture: CatalogArchitecture }
  | { code: "architecture_fallback"; platform: CatalogPlatform;
      architecture: CatalogArchitecture; selected: "platform" | "default" }
  | { code: "symbol_resolved"; symbol: string; value: string;
      source: "caller" | "discovery" | "host" | "default" }
  | { code: "symbol_unresolved"; symbol: string }
  | { code: "filesystem_case_assumed"; paths: string[];
      comparison: "case_sensitive" }
  | { code: "filesystem_identity_unresolved"; paths: string[];
      platform: CatalogPlatform }
  | { code: "readonly_superseded"; removed: PathRequirement;
      requiredBy: PathRequirement[] }
  | { code: "filesystem_deny_removed"; removed: PathRequirement;
      requiredBy: PathRequirement[] }
  | { code: "network_deny_removed"; removed: NetworkRequirement;
      requiredBy: NetworkRequirement[] }
);

export type PolicyResolutionWarning = ToolResolutionWarning | ResolutionDetailWarning;
```

Warnings about shared contributions carry sorted, distinct `inputIndexes` and
`entryIds`. Each path/rule identifies its source entries. `removed` contains
the full original path or egress rule, including exclusions/protocol/ports,
not just the overlap. Other grants may apply throughout the removed scope.

### Inspection metadata

```ts
export type CatalogIdentityMetadata =
  | { kind: "purl"; value: string }
  | { kind: "invocation-name"; names: string[] };

export interface CatalogIntentMetadata {
  name: string;
  exampleSubcommands?: string[];
  dependencyEntryIds: string[];
}

export interface CatalogAdditionsMetadata {
  dependencyEntryIds: string[];
  intentAdditions: CatalogIntentMetadata[];
  newIntents: CatalogIntentMetadata[];
}

export interface CatalogEntryMetadata {
  catalogRevision: string;
  entryId: string;
  entryRevision: number;
  displayName: string;
  versionScheme: "npm" | "semver" | "pypi" | "nuget" | "intdot";
  identity: CatalogIdentityMetadata[];
  default: {
    dependencyEntryIds: string[];
    intents: CatalogIntentMetadata[];
  };
  platformVariants: Array<CatalogAdditionsMetadata & {
    platform: CatalogPlatform;
    architecture?: CatalogArchitecture;
  }>;
  versionVariants: Array<CatalogAdditionsMetadata & { versionRange: string }>;
  provenance: {
    method: string;
    sourceRevision: string;
  };
}
```

## 3. Behavior

### Context defaults and symbol resolution

| Omitted input | Behavior |
|---|---|
| `ctx` | No caller overrides; lookup still requires an explicit API call |
| `platform` | Current host platform |
| `architecture` | Native system architecture, not the library process's architecture |
| `catalogRevision` | Installed default revision |
| `allowWeakIdentityFallback` | `false` |
| `projectRoot`, `symbols` | No caller path overrides; no invented project root |
| `packageUrl`, `detectedVersion` | No fabricated identity/version evidence |
| `intent` | All intents of the effective policy, not of other version variants |

Only symbols required by selected entries and dependencies are resolved:
caller value first, supported local discovery second, documented platform
default third, otherwise unresolved. Discovered/defaulted values and their
sources appear in diagnostics. A configuration-read failure is a library error,
not permission to assume a default. An unresolved required symbol prevents
requirements from being returned; it is not silently dropped.

Discovery describes the current host/environment. Callers targeting another
environment supply overrides. Re-resolve or supply appropriate values if the
eventual execution environment differs. The catalog does not run tools to
discover locations or versions. Definitions of supported symbols belong to the
[catalog contract](mxc-policy-store.md#42-entry-shape).

Only bundled revisions are selectable. An unavailable explicit revision fails,
without downloading or substituting another revision. Installing an SDK update
does not rewrite requirements previously accepted by a consumer.

### Identity and match precedence

Lookup uses identity, optional detected version, and optional tool-defined
intent. Raw command lines and calling-application identity are not keys.
Callers map operations to intent names; catalog subcommand examples are hints,
not parsing rules. A caller-supplied identity is not verified identity.

Parse complete PURLs using the pinned
[component rules](https://github.com/package-url/purl-spec/blob/7cd2d3442fb9c88155db17ada7c911b40ec22d41/docs/specification/standard/Clause-5-Package-URL-Specification.md)
and [type definitions](https://github.com/package-url/purl-spec/tree/7cd2d3442fb9c88155db17ada7c911b40ec22d41/types),
including percent-decoding before comparison. Compare only type, namespace,
and name. Type is case-insensitive; namespace/name use their type's rules,
or exact decoded comparison where no special rule exists, never host casing.
Candidate version/qualifiers/subpath are ignored with `purl_components_ignored`.
Only `detectedVersion` supplies version evidence. Invalid PURLs produce
`tool_unmatched` with `purl_invalid` for that pair, without fuzzy repair or
invocation-name retry. Invalid catalog PURLs are authoring errors.

Invocation names compare case-insensitively and locale-independently on
Windows/macOS, and exactly on Linux. Name-only matching requires
`allowWeakIdentityFallback: true`.
For each tool, rank eligible matches in this order:

1. Package URL over invocation-name-only identity.
2. Requested intent declared in the default/applicable overlays over no such declaration.
3. Exact architecture over platform-neutral additions or common default.

One highest-ranked match wins. Distinct ties fail with `ambiguous_match`;
multiple predicates in one entry count as one match at the strongest satisfied
strength. Version ranges do not select a different tool entry. A subsequent
version/intent failure does not retry a weaker entry or wildcard.

### Version, intent, and platform selection

Each entry has one common default and optional additive platform/architecture
and version overlays. Select at most one overlay of each kind; version ranges
do not overlap or cascade. Parse `detectedVersion` using the entry's scheme,
without fuzzy repair or trying other schemes.

| Version input | Effective requirements | Version status |
|---|---|---|
| Omitted | Default + platform additions | `matched_default`; no version warning |
| In one range | Default + platform + that version's additions | `matched_version`; record selected range |
| Valid but in no range | Default + platform additions | `version_out_of_range` warning; never nearest/highest/broadest variant |
| Unparseable | No contribution from the pair | `version_unparseable` warning |

Ranges use [VERS](https://github.com/package-url/vers-spec/blob/797c842a4afebf258e6710a68cd60306afc36708/docs/specification/standard/Clause-5-VERS-Specification.md)
with the pinned [version types](https://github.com/package-url/vers-spec/blob/797c842a4afebf258e6710a68cd60306afc36708/docs/types/vers-types.md):
`npm` (node-semver), `semver` (SemVer 2.0.0), `pypi` (PEP 440),
`nuget` (NuGet rules), and `intdot` (dotted integers). The type governs
normalization, prefixes and prereleases; do not independently strip prefixes
or apply npm range rules to other types. `generic` is unsupported.

Intent behavior is defined by the [selection-mode table](#results-and-selections).
Out-of-range version plus newer-only intent emits both warnings.

After identity/intent specificity, exact architecture additions beat
platform-only additions; otherwise retain the common default. Never select
another architecture. Architecture detection failure is an error.
Host-derived selection does not establish the installed tool's architecture:
an x64 tool on ARM64 may require an explicit x64 selection. Diagnostics report
host-derived selection and neutral/default fallback, not identity verification.

### Dependencies and composition

Compose contributing tools into one shared requirements object. A dependency
uses its unversioned base plus applicable platform base additions, with intent
additions selected by the [selection-mode table](#results-and-selections).
It does not select a version overlay; a dependency range is not detected
version evidence.
Resolution is transitive and cycle-rejecting. Shared base/platform layers
contribute once; distinct selected version/intent additions remain. Per-input
attribution, including transitive dependency requesters, is retained.

Composition combines needed access, not caller authorization:

| Selected filesystem requirements | Result |
|---|---|
| Read-only and read-write at the same path | Read-write |
| Read-write parent and read-only child | Read-write parent; omit redundant read-only child |
| Read-only parent and read-write child | Keep both; do not make the whole parent writable |
| Catalog deny overlapping a required grant | Remove the whole deny; report its full scope |
| Non-overlapping deny and grant | Keep both |

Emit `readonly_superseded` when read-write requirements supersede a read-only
requirement, identifying the paths, access classes, and contributing entries.

Rules apply after symbol substitution to equal/nested paths and established
object aliases. Keep required alias pathnames accessible, including read-only
aliases of an object another tool needs read-write. Use actual filesystem
case rules; if unknown, preserve case for comparison and report the assumption,
but never use lexical comparison as a substitute for necessary object checks.
Lookup inspects local host-side source paths, not remote/guest filesystems.
Unknown necessary identity excludes affected pairs with
`filesystem_identity_unresolved`; never retain a grant by dropping only an
unresolved restrictive side. Runner enforcement remains authoritative.

A no-network tool does not veto another tool's network requirement. One
network-requiring component retains its supported settings. Multiple scoped
egress requirements union whole allow/deny rules under deny-by-default,
preserving destination/exclusion/protocol/port pairings. Remove an entire catalog
deny that overlaps a required allow after CIDR exclusions and protocol/port
intersection; retain other denies. Report removed rules and full affected scope.
Do not introduce unrestricted access or unselected overlay/intent additions.

Allow-by-default, non-default ingress, proxy, UI, timeout, and other fields
without composition rules fail when distinct selected policies require an
undefined merge. Caller-owned restrictions are never removed. Access belongs
to the combined sandbox, not separate permissions for each tool.

### Results, coverage, and attribution

**Both resolution APIs intentionally allow partial results.** A non-absent
requirements-only result makes no coverage promise. Callers needing coverage
use diagnostics; there is no `requireAllMatches` option.

`tool_unmatched`, `version_unparseable`, `intent_unsupported`, and
`filesystem_identity_unresolved` pairs contribute nothing; independent valid
pairs still contribute. No wildcard fills a missing match. Empty/all-noncontributing
input yields absence, not an empty requirements object. Missing required symbols
in otherwise selected requirements prevent output rather than silently dropping
those requirements. Library failures remain distinct from these outcomes.

Diagnostic tool records follow input order. `matches` has at most one selected
entry. Unmatched inputs have an empty list and a warning: `purl_invalid` for an
invalid candidate PURL, otherwise `tool_unmatched`. Version/intent failures can retain
identity attribution without contributing requirements:

- `versionSelection` records supplied version and status; `selectedVersionRange`
  appears only for `matched_version`.
- A contributing pair's status is its version status. Unsupported intent or
  unresolved identity replaces the pair status while preserving completed
  selection metadata. Out-of-range plus unsupported intent emits warnings in
  that order. Structured per-input warnings preserve input order.
- Dependencies report `matched_default`.
  Deduplicate identical entry/revision/range/selection records,
  sorted by those fields, unioning sorted distinct direct/transitive `inputIndexes`.

## 4. Errors and consumer responsibilities

Use the existing MXC primary code for ordinary handling; `details.reason` is
optional extra detail. When supplied, it uses the mapping below. Unknown/absent
reasons retain primary-code handling; callers need not parse messages.

| Failure | MXC code | Optional reason |
|---|---|---|
| Invalid tool input or context | `malformed_request` | `invalid_context` |
| Invalid catalog, missing dependencies, or cycles | `policy_validation` | `invalid_catalog` |
| Tied highest-ranked matches | `policy_validation` | `ambiguous_match` |
| Incompatible SDK target or undefined composition | `policy_validation` | `composition_conflict` |
| Unsupported/undetectable host platform or architecture | `unsupported_containment` | `unsupported_host` |
| Bundled catalog cannot be read | `backend_error` | `integrity` |
| Explicit revision is not installed | `backend_error` | `revision_unavailable` |

Invalid candidate PURLs, unparseable version strings, unsupported intents, and
unresolved filesystem identity are per-pair outcomes, not whole-call failures.
Supported filesystem/network overlaps are resolved, not composition failures.

The consumer controls lookup enablement, acceptance, authorization/elevation,
and execution. Keep catalog requirements separate from user/learned policies;
preserve OS, enterprise, device, and backend ceilings. Failure to realize a
required floor never authorizes uncontained execution. When persisting or
auditing accepted requirements, retain catalog/entry revisions, contributing
inputs, warnings, and approval state. The library never mutates that state.

## 5. Usage examples

Zava Agent is an illustrative agent client. Its requirements advisor queries
the read-only MXC catalog; it does not manage catalog files. Git versions,
intents, and paths below are examples, not a promised production catalog.

### Zava Agent: create a tool request with diagnostics

Zava Agent first checks its own saved settings for the current tool and intent.
Only when none apply and catalog lookup is enabled does it request an MXC floor.
The application hooks below are scoped to the current user, workspace, and target;
the saved-settings helper checks tool identity/version and context before reuse.

```ts
// zava-agent-requirements.ts
import {
  resolveToolRequirementsWithDiagnostics, getCatalogInfo, listCatalogEntries,
  MxcError,
} from "@microsoft/mxc-sdk/v1";
import type {
  ContainerRequirements, ContainerRequest, ResolveContext, ToolCandidate,
  PolicyResolutionWarning, ToolRequirementsResolution, CatalogEntryMetadata,
} from "@microsoft/mxc-sdk/v1";

// Zava Agent-owned settings, logging and error hooks; implementations omitted.
export interface ZavaAgentApp {
  loadApplicableToolRequirements(
    tool: ToolCandidate, context: ResolveContext,
  ): Promise<ContainerRequirements | undefined>;
  isMxcFloorCatalogEnabled(): boolean;
  logResolution(tool: ToolCandidate, diagnostics: ToolRequirementsResolution["diagnostics"]): void;
  showUnpreparedTool(tool: ToolCandidate, diagnostics: ToolRequirementsResolution["diagnostics"]): void;
  // Applies tool settings and client-wide restrictions, including "never allow" paths.
  // May prompt if configured; returns chosen requirements or undefined to stop.
  applyClientSettings(
    tool: ToolCandidate, requirements: ContainerRequirements,
    warnings: PolicyResolutionWarning[],
  ): Promise<ContainerRequirements | undefined>;
  logLibraryFailure(error: MxcError): void;
  showInputCorrection(error: MxcError): void;
  showUnsupportedHost(error: MxcError): void;
  showCatalogUnavailable(error: MxcError): void;
}

export class ZavaAgentRequirements {
  constructor(
    private readonly app: ZavaAgentApp,
    private readonly context: ResolveContext = {},
  ) {}

  inspectCatalog(): {
    info: ReturnType<typeof getCatalogInfo>; entries: CatalogEntryMetadata[];
  } | undefined {
    try {
      return { info: getCatalogInfo(), entries: listCatalogEntries() };
    } catch (error) {
      this.reportFailure(error);
      return;
    }
  }

  private async prepareTool(tool: ToolCandidate): Promise<ContainerRequirements | undefined> {
    const saved = await this.app.loadApplicableToolRequirements(tool, this.context);
    if (saved !== undefined) return this.app.applyClientSettings(tool, saved, []);
    if (!this.app.isMxcFloorCatalogEnabled()) return undefined;

    let result: ToolRequirementsResolution;
    try {
      result = await resolveToolRequirementsWithDiagnostics(tool, this.context);
    } catch (error) {
      this.reportFailure(error);
      return undefined;
    }

    // Keep warning fields, selections, and dependency attribution in the audit record.
    this.app.logResolution(tool, result.diagnostics);
    const status = result.diagnostics.tools[0]?.status;
    const covered = status === "matched_default" || status === "matched_version"
      || status === "version_out_of_range";
    if (result.requirements === undefined || !covered) {
      this.app.showUnpreparedTool(tool, result.diagnostics);
      return undefined;
    }

    // Path discovery is logged; client settings decide how other warnings affect use.
    const relevantWarnings = result.diagnostics.warnings.filter(
      warning => warning.code !== "symbol_resolved");
    return this.app.applyClientSettings(tool, result.requirements, relevantWarnings);
  }

  async createRequestForTool(
    tool: ToolCandidate, command: string,
  ): Promise<ContainerRequest | undefined> {
    const requirements = await this.prepareTool(tool);
    return requirements === undefined ? undefined : { ...requirements, command };
  }

  private reportFailure(error: unknown): void {
    if (!(error instanceof MxcError)) throw error;
    this.app.logLibraryFailure(error); // Include code, message, and details in app logs.
    switch (error.code) {
      case "malformed_request":
        this.app.showInputCorrection(error);
        break;
      case "unsupported_containment":
        this.app.showUnsupportedHost(error);
        break;
      case "policy_validation":
      case "backend_error":
        this.app.showCatalogUnavailable(error);
        break;
      default:
        throw error;
    }
  }
}
```

This client logs every diagnostic; it changes workflow only where needed:

| Diagnostic | Client behavior |
|---|---|
| `symbol_resolved`; successful default/version selections | Log; use the result according to client settings |
| Architecture defaults/fallbacks, assumed casing, ignored PURL components, weak identity, out-of-range version | Pass the uncertainty to `applyClientSettings` |
| Superseded read-only access or removed filesystem/network denies | Pass the full changed scope and source entries to the same helper |
| Unmatched/invalid identity, unparseable version, unsupported intent, unresolved filesystem identity or required symbol | Return no prepared requirements; show corrective context |
| Library failure | Report by primary MXC code; no retry, revision substitution, or uncontained fallback |

The enablement check is a Zava Agent preference, not an MXC API. Saved
tool/intent settings take precedence even when catalog lookup is disabled.
`applyClientSettings` may accept requirements unchanged, further restrict them,
or decline them, including applying client-wide restrictions to saved settings.
The client can allow automatic use or require a prompt; this
example specifies neither UI nor persistence policy. Any decision uses the
structured facts rather than parsing warning messages. These helpers do not
write to the MXC catalog. The caller can also modify its final `ContainerRequest`
before submitting it to MXC; catalog recommendations never override its settings.

With no applicable saved settings, a disabled catalog or an unresolved/rejected
suggestion leaves this sample's action unprepared. The caller does not create
an empty or uncontained fallback. Application-helper failures propagate to the
application error handler rather than being disguised as an absent catalog match.

The caller supplies workspace and target context when constructing the advisor,
then calls `createRequestForTool(tool, command)` for an execution. Its private
`prepareTool` helper obtains requirements and applies client settings. The command
is added only to the returned `ContainerRequest`, never sent to the catalog API.
Catalog inspection is optional; none of these methods creates a container.

```ts
// Configure the advisor for this workspace, then prepare a Git push request.
declare const app: ZavaAgentApp;
const advisor = new ZavaAgentRequirements(app, {
  platform: "windows", architecture: "x64", allowWeakIdentityFallback: true,
  projectRoot: String.raw`D:\work\repo`,
  symbols: {
    git_prefix: String.raw`D:\tools\git`,
    ssh_prefix: String.raw`D:\tools\ssh`,
    node_prefix: String.raw`D:\tools\node`,
    programData: String.raw`C:\ProgramData`,
    temp_dir: String.raw`D:\temp`,
  },
});
const catalogMetadata = advisor.inspectCatalog(); // Optional inspection; no UI is prescribed.
const gitRequest = await advisor.createRequestForTool({
  invocationName: "git", detectedVersion: "2.45", intent: "push",
}, "git push");
// Only a defined request goes to Zava Agent's normal MXC run/spawn path.
```

With the [design's Git fixture](mxc-policy-store.md#42-entry-shape), requesting
`bundle-fetch` without a version, saved settings, or a defined default intent
returns `undefined` when lookup is enabled; no broader intent is substituted.
Git `2.45` + `push` records the SSH dependency in diagnostics. Multiple callers
share the immutable MXC catalog, not their saved settings or approval state.

### A smaller caller: best-effort build workspace without diagnostics

A project runner prepares one shared sandbox for a Node build that invokes Git.
It uses the catalog as a best-effort starting configuration and applies its own
settings, without needing per-tool match details. This Windows example prepares
the request; the runner's normal MXC execution path runs the build.

```ts
import { resolveToolRequirements } from "@microsoft/mxc-sdk/v1";
import type { ContainerRequirements, ContainerRequest, ResolveContext } from "@microsoft/mxc-sdk/v1";

// Application helpers; automatic use, prompting, and persistence are client choices.
declare function applySharedClientSettings(
  requirements: ContainerRequirements,
): Promise<ContainerRequirements | undefined>;
declare function showNoSharedSuggestion(): void;

async function prepareSharedWorkspace(
  context: ResolveContext,
): Promise<ContainerRequirements | undefined> {
  const suggestion = await resolveToolRequirements(["git", "node"], {
    ...context, allowWeakIdentityFallback: true,
  });
  if (suggestion === undefined) {
    showNoSharedSuggestion();
    return undefined;
  }
  // Non-empty is not proof that both tools matched.
  return applySharedClientSettings(suggestion);
}

// The runner supplies installed-tool paths from its own configuration or discovery.
declare const installedToolPaths: ResolveContext["symbols"];
const projectRoot = String.raw`D:\work\repo`;
const selectedRequirements = await prepareSharedWorkspace({
  projectRoot,
  symbols: installedToolPaths,
});
const buildRequest: ContainerRequest | undefined = selectedRequirements === undefined
  ? undefined
  : { ...selectedRequirements, command: "node build.js", workingDirectory: projectRoot };
// Only a defined request goes to the runner's normal MXC execution path.
```

The short `["git", "node"]` input deliberately opts into name-only matching,
uses common defaults without detected versions, and includes all effective
intents. This is a broad starting point, not a build-specific minimum. The same
API accepts tool descriptors with package identity, detected version, and intent
when the runner has that information.

If only Git matches, the result may lack access Node needs; a defined result
does not prove both tools matched. A runner needing that distinction uses
diagnostics. Absence or rejection stops request creation. Library failures
propagate to the application error boundary; execution failures use the runner's
normal error handling, never an automatic expansion of access or uncontained
retry. Sharing one container also shares its combined access, not separate
permissions per tool.
