<!--
Adapted from the Known-tool Policy Floors / Policy Store design proposal,
docs/mxc-policy-store.md in Chaz Gordish's "chazgo-vigilant-enigma" MXC
worktree (base commit 34537f85384c00f72564f931077d1790c5a86a02,
microsoft/mxc#1309, plus uncommitted edits as of 2026-09-29).

Updated for the 2026-10-01 Policy Store design review outcome: the store ships
inside MXC as new APIs in the existing MXC SDKs, not as a separate repository,
standalone library, or CLI; it is not part of MXC 1.0; V1 data is bundled
statically; a resolved policy is a best-effort floor; Learning Mode is
complementary; and the API remains pending API review. Changed sections:
Status, §1 (feature impact), §2, §5 (introduction), §6, §7, §8, §11, §12,
§13, and §14.

Updated again on 2026-10-02 for the entry model the prototype now implements:
one unversioned default per entry with additive platform, version, and intent
overlays; per tool-and-intent resolution statuses; and composition that
satisfies every requested pair. Changed sections: §4, §5.1, §5.2, §7, §9,
§12, and §13. The canonical design document remains the source of truth; this
copy summarizes the prototype's behavior and is not a substitute for it.
-->
# Feature Spec: Known-tool Policy Floors

**Status:** Prototype, pending API review and sign-off before check-in. The
policy store ships inside MXC as new APIs in the existing MXC SDKs. It is not
part of MXC 1.0; a later MXC SDK release is targeted. The API names in this
document are the current proposal and may change in review (see
[§13](#13-open-questions)).

**IMPORTANT NOTE:** A resolved policy is a best-effort floor, not a guarantee.
Applying it does not guarantee that a tool's end-to-end workflow will work
under process containment. The policy store is complementary to Learning Mode,
not a replacement for it (see [§8](#8-relationship-to-learning-mode)).

---

## 1. Problem Statement

Developers frequently disable process isolation after enabling it breaks tools
needed for their workflow. This makes the first-run experience for process
containment poor and reduces adoption before developers can identify the
missing policy.

The near-term mitigation is a public, reviewable set of known per-tool policy
floors. Consumers can apply a candidate floor instead of starting from no tool
knowledge, and contributors can iterate on the data as failures are found. A
floor only describes a known minimum requirement. It does not prove that every
process, dependency, credential, service, or network interaction in an
end-to-end workflow is covered.

[#779](https://github.com/microsoft/mxc/pull/779) proposed the initial config
floor data model and SDK resolver. This document narrows that proposal into a
temporary catalog contract with explicit identity, platform, dependency,
revision, and inspection behavior. It reuses #779's model where possible and
calls out differences directly.

This document does not restate general MXC sandboxing concepts already covered
by [`docs/sandbox-policy/0.8.0/policy.md`](../sandbox-policy/0.8.0/policy.md) or
[`docs/versioning.md`](../versioning.md). It covers only what a policy store adds.

### Non-goals

- This does not change what the sandbox backend enforces, or `SandboxPolicy` /
  `ContainerConfig` schema semantics. A catalog entry embeds an existing
  `SandboxPolicy`; it does not define a parallel vocabulary.
- This is not a trust or attestation mechanism, and it does not authorize
  anything. See [§9](#9-trust-model).
- This is not a guarantee that a complete tool workflow will succeed under
  process containment.
- This does not define how any specific consumer stores, displays, or lets a
  user approve requirements. See [§2](#2-ownership-boundary).
- This does not define Learning Mode's candidate-generation or review UX. See
  [§8](#8-relationship-to-learning-mode).

### MXC feature impact and defaults

This adds known-tool lookup APIs to the existing MXC SDKs. Following the feature-impact
checklist in [`docs/authoring-a-new-feature.md`](../authoring-a-new-feature.md):

- **Policy changes:** None. Catalog entries embed an existing, registered
  `SandboxPolicy`.
- **ContainerConfig changes:** None. The catalog does not add configuration
  fields or change omission behavior in an existing contract.
- **OS and backend changes:** None. Backends continue to validate whether they
  can enforce the resolved policy.
- **MXC SDK changes:** Additive. The Node (`@microsoft/mxc-sdk`), Rust
  (`mxc-sdk`), and C# (`Microsoft.Mxc.Sdk`) SDKs gain catalog lookup and
  inspection APIs that return each SDK's existing `SandboxPolicy` type. Existing
  APIs and behavior are unchanged. See
  [§6](#6-packaging-inside-the-mxc-sdks).
- **No separate deliverable:** There is no separate repository, standalone
  library, or CLI utility.

The policy store does not add an MXC schema feature, activate the
`--experimental` runtime gate, or change executor behavior.

Defaults and omission behavior are:

- Existing callers do not perform catalog lookup automatically. A consumer
  must explicitly enable or invoke it.
- Omitted `ResolveContext.platform` uses the current host platform.
- Omitted `ResolveContext.architecture` uses the device's native system
  architecture, not the architecture of the calling process or a detected
  tool build. Explicit caller selection takes precedence. See the selection
  rules and emulation risk in [§4.4](#44-platform-and-architecture).
- Omitted `ResolveContext.catalogRevision` uses the catalog revision bundled
  with the SDK.
- Omitted `ResolveContext.allowWeakIdentityFallback` is `false`.
- Omitted `projectRoot` and `symbols` provide no caller overrides. The resolver
  may use approved host-known symbols, but it does not invent machine-specific
  values. A selected entry with an unresolved required symbol is not
  resolvable.
- Omitted `packageUrl` or `detectedVersion` supplies no matching evidence. The
  resolver does not fabricate either value or infer a version from anything
  else. Omitted `intent` selects the base policy plus every intent.
- If no policy can be resolved, `resolveSandboxPolicy` returns `undefined`.
  `resolveSandboxPolicyWithDiagnostics` instead returns a result whose `policy` is
  `undefined`, preserving the diagnostics. The consumer's restrictive baseline
  remains unchanged.

## 2. Ownership boundary

MXC owns a validated, versioned, read-only data set of known-tool
sandbox requirements, the resolver that reads it, and the SDK APIs that expose
it, alongside the existing `SandboxPolicy` contract. The data is reviewed and
released with the MXC SDKs.

The catalog states a candidate minimum that a tool needs. It does not grant
access, modify caller state, create a sandbox, or guarantee workflow success.

Everything else is a consumer decision:

- Whether automatic catalog lookup is enabled at all.
- Access-profile mapping, elevation preference, and per-tool authorization.
- Persistence of accepted requirements (which tools, which catalog/entry
  revision, when).
- Composition with the consumer's own user, learned, and invocation-specific
  policy layers, and with non-overridable OS/enterprise/device ceilings.
- Approval UX, audit, and the final call into `createConfigFromPolicy()` /
  sandbox creation.

A catalog lookup can only ever narrow what a consumer still has to decide for
itself. `resolveSandboxPolicy` returns a candidate composed requirement or
`undefined`; its diagnostics counterpart also reports how that result was
obtained. The consumer decides whether and how to act on it. This mirrors
#779's floor/policy distinction, discussed further in
[§3](#3-relationship-to-the-config-floors-proposal): a resolved entry is a
lower bound asserted by the tool ecosystem, never an upper bound the host is
required to grant.

## 3. Relationship to the config-floors proposal

| #779 (config floors) | This document (policy store) |
|---|---|
| One `schemaVersion` for the whole table | Separate version dimensions: `catalogSchemaVersion`, `catalogRevision`, per-entry `entryRevision`, the default's `sandboxPolicy.version`, and a per-entry tool `versionScheme` ([§4.1](#41-versions)) |
| Strongest satisfied identity predicate describes a match | Each input selects at most one ranked entry; a tie fails rather than guessing ([§4.3](#43-identity)) |
| One `sandboxPolicy` per entry; `when.platform` only conditions dependencies | One unversioned default per entry plus additive platform, version, and intent overlays; nothing can name a containment backend ([§4.2](#42-entry-shape)) |
| `requires` composition unspecified beyond "union" | Explicit composition rules that satisfy every requested pair; anything inexpressible fails ([§4.6](#46-composition)) |
| `getSandboxConfigForTool(tools: string[])` returns one composed policy | `resolveSandboxPolicy` accepts one tool or an array and returns one policy; `resolveSandboxPolicyWithDiagnostics` adds attribution, with catalog inspection kept separate ([§5](#5-api-surface)) |
| No revision/publication model | Immutable published catalog revisions; corrections publish a new revision ([§10](#10-immutable-revisions)) |

The data model, the floor/policy direction argument, multi-tool composition,
and the identity layering problem (invocation name vs. launcher artifact vs.
executing image) build on #779. The matching, composition, and trust refinements
are stated here rather than implied by that reference.

## 4. Data model

### 4.1 Versions

| Field | Meaning |
|---|---|
| `catalogSchemaVersion` | Version of the catalog JSON shape itself. |
| `catalogRevision` | Immutable identifier for one published, fully reviewed catalog. |
| `entryRevision` | Monotonic revision of a single entry, for cache invalidation and audit comparison. |
| `sandboxPolicy.version` | The exact registered `SandboxPolicy` contract version of an entry's default. |
| `versionScheme` | Required per entry: how its tool versions are parsed and compared (`npm`, `semver`, `pypi`, `nuget`, or `intdot`). |

These identifiers serve separate purposes and do not advance in lockstep.
Registering a new `SandboxPolicy` contract does not change existing catalog
data. Migrating an entry to that contract changes its content, so the
migration increments both `entryRevision` and `catalogRevision`. Tool
version ranges are an orthogonal axis: they describe which builds of a tool
the reviewed evidence covers, not anything about the catalog.

### 4.2 Entry shape

An entry has exactly one unversioned **default** and any number of
**overlays**. Overlays only add; none can remove or narrow what the default or
another overlay grants. Abbreviated from the bundled `tool:git` entry:

```json
{
  "entryId": "tool:git",
  "entryRevision": 1,
  "displayName": "Git",
  "versionScheme": "intdot",
  "identity": [
    { "kind": "purl", "value": "pkg:generic/git" },
    { "kind": "invocation-name", "names": ["git", "git.exe"] }
  ],
  "default": {
    "sandboxPolicy": {
      "version": "0.9.0-alpha",
      "filesystem": { "readonlyPaths": ["${git_prefix}"], "readwritePaths": ["${project_root}"] }
    },
    "intents": {
      "local": { "exampleSubcommands": ["status", "commit", "log"] },
      "fetch": { "policyAdditions": { "network": { "egress": { "allow": ["…tcp/443…"] } } } },
      "push":  { "policyAdditions": { "network": { "egress": { "allow": ["…tcp/22…"] } } } }
    }
  },
  "platformVariants": [
    { "when": { "platform": "windows" },
      "policyAdditions": { "filesystem": { "readonlyPaths": ["${programData}/Git"] } } }
  ],
  "versionVariants": [
    { "versionRange": "vers:intdot/>=2.40|<2.50",
      "intentAdditions": { "push": { "dependencies": [{ "entryId": "tool:ssh" }] } } },
    { "versionRange": "vers:intdot/>=2.50|<3",
      "intents": { "bundle-fetch": { "policyAdditions": { "…": "…" } } } }
  ],
  "provenance": { "method": "design-example", "sourceRevision": "…" }
}
```

The bundled revision (`2026-10-02.1`) carries `tool:git`, `tool:node`,
`tool:npm`, and `tool:ssh`. Its network addresses are documentation ranges
(RFC 5737), not real endpoints.

- **Default.** One complete `sandboxPolicy`, the default's dependencies, and
  its named intents. It applies whenever the entry matches.
- **Platform overlays** (`platformVariants`) add filesystem paths, outbound
  allow rules, dependencies, intent extensions (`intentAdditions`), or new
  intents for one platform, optionally one architecture. An exact
  architecture overlay is preferred over the platform's architecture-neutral
  overlay; another architecture's overlay is never used. Selectors must be
  unique, with at most one neutral overlay per platform.
- **Version overlays** (`versionVariants`) carry a purl `vers` range in the
  entry's `versionScheme`. Ranges within an entry must not overlap, so at most
  one applies. A version overlay adds the same kinds of things a platform
  overlay can.
- **Intents** are lower-case names (`[a-z][a-z0-9_-]*`). An intent adds
  access and dependencies on top of the base policy. `intentAdditions` may
  extend only intents the default declares; `intents` may introduce only
  names no other applicable overlay declares.
- **Not additive**, and therefore rejected in an overlay: `deniedPaths`,
  `network.egress.deny`, `network.egress.default`, ingress, `ui`, and
  `timeoutMs`. Those may appear only in the default's `sandboxPolicy`.

The **effective policy** for one platform, architecture, and version is the
default plus the selected platform overlay plus at most one version overlay;
an intent then selects from the effective policy's intents. Symbols
(`${project_root}`, `${npm_cache}`, …) are resolved before a policy is
returned; catalog data never ships a literal, machine-specific path.

### 4.3 Identity

A purl predicate is a **strong** identity; an invocation-name predicate is a
**weak** fallback that participates only with `allowWeakIdentityFallback`.
Catalog purls never pin a version, and a version embedded in a caller's
`packageUrl` is ignored with a warning: the only version evidence is
`detectedVersion`. Invocation names compare case-insensitively on Windows and
macOS and exactly on Linux.

Each input selects at most one entry. Among eligible entries the resolver
prefers a strong match, then an entry whose applicable overlays declare the
requested intent, then an exact-architecture overlay over a neutral one over
none. A tie at the top rank fails as `policy_validation`
(`ambiguous_match`) rather than guessing. The resolver does not verify that
the executable really carries the identity the caller passed.

### 4.4 Platform and architecture

Supported platforms are `windows`, `linux`, and `macos`; architectures are
`x64` and `arm64`. An omitted `ResolveContext.architecture` uses the device's
native system architecture (not the process architecture), detected only when
an entry has architecture-specific overlays for the platform; when it is
used, a warning says the tool's architecture was not verified. Falling back
to a platform's neutral overlay because no exact overlay exists also produces
a warning. A failure to detect the native architecture when it is needed is
an error, not a guess.

An x64 tool running under emulation on an ARM64 device may need the x64
overlay. The resolver never inspects or runs the tool; callers that know the
tool's architecture should pass it explicitly.

### 4.5 Resolution per tool and intent

Each input is one tool-and-intent pair, resolved independently:

| Situation | Status | Contribution |
|---|---|---|
| No detected version | `matched_default` | the effective policy without a version overlay |
| Version inside a range | `matched_version` | default plus that version overlay |
| Valid version in no range (including an entry with no ranges) | `version_out_of_range` | as `matched_default`, with a structured warning |
| Version the scheme cannot parse | `version_unparseable` | nothing |
| Intent not defined by the effective policy | `intent_unsupported` | nothing; the version status stays in `versionSelection` |
| No intent | (version status) | base policy plus every intent of the effective policy |
| No eligible entry | `tool_unmatched` | nothing |

There is no wildcard fallback and no "require all" option: other inputs still
resolve, so a result may cover only some of the requested tools.
Dependencies are materialized with their default, the platform overlay, and
all intents; a dependency's `versionRange` (in the target entry's scheme) is
recorded as unevaluated metadata.

### 4.6 Composition

All contributing pairs and their dependency closure compose into one policy:

1. Every contribution must declare the same `sandboxPolicy.version`.
2. Paths are substituted and normalized with the selected platform's rules,
   then de-duplicated exactly. Case-only differences are kept and warned
   about, because the filesystem's case sensitivity is not determined.
3. Read-write supersedes read-only: a read-only path at or under a read-write
   path is omitted, with a warning.
4. A catalog deny that overlaps any required read-only or read-write path is
   removed entirely, with a warning naming its scope. Non-conflicting denies
   are kept.
5. Outbound allow rules from every pair are unioned under a default-deny
   egress posture, so a pair without network needs never vetoes another
   pair's. If only one contribution uses network, its section passes through
   unchanged.
6. A single contribution with no additions passes through whole, including
   fields such as `ui` and `timeoutMs`. Anything else the model cannot
   express across contributions (for example a catalog egress deny alongside
   another contribution's network, or `timeoutMs` in a composed set) fails as
   `policy_validation` (`composition_conflict`) rather than granting broader
   access.

The caller's own restrictions still win: the floor is input to the caller's
composition, never a ceiling override.

### 4.7 Build-time validation

Catalog validation (run by `cargo test -p mxc_policy_store`) checks the
contract and identity rules, the required `versionScheme` and single
default, unique selectors, non-overlapping ranges in the entry's scheme,
additive-only overlays, dependency targets and their ranges, and cycles. It
then materializes every platform × architecture × version × intent effective
policy (including the "all intents" case) with its dependency closure,
validates each composed policy, and checks that the default is a subset of
each. A Markdown reviewer view of every effective policy is generated into
`catalog/views/<revision>.md` and must stay current.

## 5. API surface

The MXC SDKs separate runtime resolution from catalog inspection.
Resolution accepts one tool or an array and composes all applicable matching
entries and dependencies into one `SandboxPolicy`. Callers choose a policy-only
operation or a diagnostic operation over the same resolution logic. Neither
implicitly returns the whole catalog. These are in-process SDK calls, not a
hosted service, and they never launch a sandbox.

The signatures below use TypeScript to describe the shared contract. Rust and
C# expose the same operations and metadata with idiomatic names and types.
TypeScript and C# expose single-tool and array overloads; Rust uses an idiomatic
one-or-many input type because it does not support function overloading. An
absent policy is `undefined` in TypeScript/JavaScript, `None` in Rust, and
`null` in C#. Failures remain distinct from policy absence.

### 5.1 Runtime lookup

```ts
interface ToolCandidate {
  invocationName: string;
  packageUrl?: string;      // strong identity; any embedded version is ignored
  detectedVersion?: string; // the only version evidence
  intent?: string;          // for example "fetch" or "push"
}

type ToolInput = string | ToolCandidate;

interface ResolveContext {
  projectRoot?: string;
  symbols?: Record<string, string>;
  platform?: "windows" | "linux" | "macos";
  architecture?: "x64" | "arm64";
  catalogRevision?: string;
  allowWeakIdentityFallback?: boolean;
}

type VersionStatus =
  | "matched_default" | "matched_version"
  | "version_out_of_range" | "version_unparseable";

interface VersionSelection {
  status: VersionStatus;
  detectedVersion?: string;
  selectedVersionRange?: string; // matched_version only
}

interface IntentSelection {
  requested?: string;
  mode: "named" | "all" | "unsupported";
  selected: string[];
}

interface ToolResolutionWarning {
  code: "version_out_of_range" | "version_unparseable"
      | "intent_unsupported" | "tool_unmatched";
  inputIndex: number;
  entryId?: string;
  detectedVersion?: string;
  intent?: string;
  message: string;
}

interface SandboxConfigResolution {
  policy: SandboxPolicy | undefined;
  diagnostics: {
    catalogRevision: string;
    tools: Array<{
      inputIndex: number;
      status: VersionStatus | "intent_unsupported" | "tool_unmatched";
      matches: Array<{ // at most one
        entryId: string;
        entryRevision: number;
        matchedIdentities: Array<{ kind: string; strength: "strong" | "weak" }>;
        versionSelection: VersionSelection;
        intentSelection?: IntentSelection; // absent when unparseable
      }>;
    }>;
    resolvedDependencies: Array<{
      entryId: string;
      entryRevision: number;
      requiredVersionRange?: string;
      versionSelection: VersionSelection;
      intentSelection: IntentSelection;
    }>;
    warnings: Array<string | ToolResolutionWarning>;
  };
}

declare function resolveSandboxPolicy(
  tools: ToolInput | readonly ToolInput[], ctx?: ResolveContext): SandboxPolicy | undefined;
declare function resolveSandboxPolicyWithDiagnostics(
  tools: ToolInput | readonly ToolInput[], ctx?: ResolveContext): SandboxConfigResolution;
```

C# exposes single-tool and list overloads; Rust takes an idiomatic
one-or-many input. A string input is shorthand for `{ invocationName }` and
carries no package, version, or intent evidence. A one-element array is
equivalent to a single input, with `inputIndex` `0`.

```ts
const ctx = { projectRoot: "/work/repo", symbols: { git_prefix: "/usr/bin" } };
const git = { invocationName: "git", packageUrl: "pkg:generic/git", detectedVersion: "2.45.1" };
// One floor for a fetch followed by a push.
const policy = resolveSandboxPolicy(
  [{ ...git, intent: "fetch" }, { ...git, intent: "push" }], ctx);
```

`resolveSandboxPolicy` returns the composed `SandboxPolicy` directly;
`resolveSandboxPolicyWithDiagnostics` returns the same policy with
attribution and warnings from the same pass. An empty or entirely
non-contributing lookup returns no policy, not an empty policy. An unresolved
required symbol in a contributing entry prevents a policy and names the
symbol in a warning; the resolver never returns a partial policy. Free-text
warnings cover weak matches, host-derived architecture and symbols,
neutral-overlay fallback, ignored purl versions, and composition adjustments;
per-input outcomes use structured warnings.

### 5.2 Setup and inspection

```ts
type CatalogPlatform = "windows" | "linux" | "macos";
type CatalogArchitecture = "x64" | "arm64";

type CatalogIdentityMetadata =
  | { kind: "purl"; value: string }
  | { kind: "invocation-name"; names: string[] };

interface CatalogIntentMetadata {
  name: string;
  exampleSubcommands?: string[];
  dependencyEntryIds: string[];
}

interface CatalogAdditionsMetadata {
  dependencyEntryIds: string[];
  intentAdditions: CatalogIntentMetadata[];
  intents: CatalogIntentMetadata[];
}

interface CatalogEntryMetadata {
  catalogRevision: string;
  entryId: string;
  entryRevision: number;
  displayName: string;
  versionScheme: "npm" | "semver" | "pypi" | "nuget" | "intdot";
  identity: CatalogIdentityMetadata[];
  default: {
    dependencyEntryIds: string[];
    sandboxPolicyVersion: string;
    intents: CatalogIntentMetadata[];
  };
  platformVariants: Array<
    { platform: CatalogPlatform; architecture?: CatalogArchitecture } & CatalogAdditionsMetadata>;
  versionVariants: Array<{ versionRange: string } & CatalogAdditionsMetadata>;
  provenance: {
    method: string;
    sourceRevision: string;
  };
}

listCatalogEntries(): CatalogEntryMetadata[];
getCatalogInfo(): { catalogSchemaVersion: string; catalogRevision: string };
```

This supports setup UI, catalog browsing, and update decisions without paying
the cost of policy resolution, and keeps "give me everything" out of the
runtime lookup path entirely. Metadata exposes selectors, version ranges,
intent names, dependency IDs, and provenance, but not an unresolved or
resolved policy body.

### 5.3 Consumer obligations

A consumer that uses this API:

1. Decides whether automatic lookup is enabled at all.
2. When persisting an accepted policy, retains its `catalogRevision` and
   contributing entry IDs/revisions from diagnostics, whether the policy
   covers one tool or several.
3. Keeps catalog-derived requirements in a layer separate from its own user,
   learned, and invocation-specific policy.
4. Applies its own authorization, elevation, and restrictive-composition
   rules on top.
5. Enforces its OS, enterprise, device, and backend ceilings regardless of
   what the catalog returned.
6. Fails closed when a required entry cannot be realized on the current
   host/backend. It falls back to its own restrictive baseline and does not
   run uncontained.
7. Uses the diagnostics operation when attribution or audit is needed, and
   records matched identities, catalog/entry revisions, warnings, and approval
   state in its own audit trail.

The policy store APIs never write a consumer's own policy state. A consumer's own
capability observation (see [§8](#8-relationship-to-learning-mode)) can produce
candidate evidence for a future contribution to this catalog; it is not a
mechanism for mutating the catalog at request time.

## 6. Packaging inside the MXC SDKs

The policy store ships inside MXC as new APIs in the existing MXC SDKs. There
is no separate repository, standalone library, package, or CLI utility. A
consumer that already uses an MXC SDK resolves a floor and passes its final,
authorized policy to that SDK's existing sandbox APIs.

### 6.1 SDK surface and bundled data

| Language | Package | API |
|---|---|---|
| TypeScript / JavaScript | `@microsoft/mxc-sdk` | `resolveSandboxPolicy`, `resolveSandboxPolicyWithDiagnostics`, `getCatalogInfo`, `listCatalogEntries` |
| Rust | `mxc-sdk` (`mxc_sdk::policy_store`) | `resolve_sandbox_policy`, `resolve_sandbox_policy_with_diagnostics`, `get_catalog_info`, `list_catalog_entries` |
| C# / .NET | `Microsoft.Mxc.Sdk` (`MxcPolicyStore`) | `ResolveSandboxPolicy`, `ResolveSandboxPolicyWithDiagnostics`, `GetCatalogInfo`, `ListCatalogEntries` |

Each SDK returns its own existing `SandboxPolicy` type, not a parallel
catalog-only type.

The V1 policy data is bundled statically in each SDK package. It is not
downloaded. Lookup is local and does not contact a hosted service or run the
candidate tool. `ResolveContext.catalogRevision` selects a bundled revision,
not a network lookup; an explicitly requested revision that is unavailable is
an error, not a substitution with a different revision. An omitted revision
uses the SDK's bundled default.

A consumer:

1. Calls `getCatalogInfo()` or `listCatalogEntries()` for inspection.
   `resolveSandboxPolicy()` returns a policy for one tool or an array;
   `resolveSandboxPolicyWithDiagnostics()` adds match attribution and warnings.
2. Handles policy absence without widening its restrictive baseline. It
   reviews the composed policy and uses the diagnostics operation when it
   needs contributing identities, revisions, and warnings, applying the
   consumer obligations in [§5.3](#53-consumer-obligations).
3. Supplies its final policy to the SDK's existing sandbox APIs. The lookup
   APIs do not launch a sandbox.

An SDK release reports its bundled `catalogRevision`; the SDK version does not
identify the catalog revision or embedded `SandboxPolicy.version`. Updating
the SDK does not rewrite a consumer's previously accepted per-tool policies.

### 6.2 Implementation and cross-language consistency

There is one resolver implementation, in Rust (`src/core/mxc_policy_store`),
which also owns the bundled catalog data, its JSON schemas, and the shared
conformance fixtures. The Rust SDK re-exports it. The Node and C# SDKs reach it
through the existing `mxc_ffi` C ABI, the same native library they already
load, so all three SDKs agree on matching, variant selection, dependency
metadata, resolved policy, warnings, and failure categories by construction.
Language-specific absence and error types preserve those distinctions.

Resolution stays outside `mxc_engine`: it never selects a backend or launches
a sandbox, so it does not belong in the execution engine.

Shared fixtures cover platform path semantics as well as ordinary lookup, and
each SDK's own test suite exercises its binding against them.

## 7. Contribution and review

- Catalog contributions are pull requests against `microsoft/mxc`. No client
  or SDK can write a catalog entry at runtime.
- Every entry change includes identity evidence, supported tool version
  range(s), platform evidence, a minimized requirement set, test fixtures,
  and provenance.
- CI validates the catalog contract, exact `SandboxPolicy` version
  registration, entry-ID uniqueness, one default per entry, non-overlapping
  version ranges, additive-only overlays, dependency closure and
  cycle-freedom, symbol validity, absence of unsafe user-specific literal
  paths, unsupported-field rejection, and deterministic resolution. It
  materializes and validates every effective policy and checks the rendered
  reviewer view ([§4.7](#47-build-time-validation)).
- A new entry or a requirement expansion requires one catalog-owner approval
  and one security/policy-reviewer approval, plus tool- or scenario-owner
  evidence where available.
- A requirement reduction requires regression evidence that every supported
  tool version still functions under the narrower requirement.
- SDK API changes follow MXC's normal API review. These contribution
  requirements do not require applications to seek maintainer approval to use
  the bundled catalog.

## 8. Relationship to Learning Mode

Learning Mode is complementary to the policy store, not a replacement for it.
The policy store supplies a reviewed, best-effort starting floor for known
tools before anything has run; Learning Mode observes what a specific workload
actually touches and helps author policy for it, including for tools the
catalog does not know.

MXC's learning-mode capabilities (`learningModeLogging`,
`permissiveLearningMode`, `captureDenials`; see
[`docs/learning-mode/capabilities.md`](../learning-mode/capabilities.md)) are the
substrate a contributor can use to observe what a tool actually touches, the
same way [#779 §5.1](https://github.com/microsoft/mxc/pull/779) describes for
config floors. That observation workflow is unchanged by this document and
remains **a contributor step that happens before a pull request**, not
consumer runtime behavior and not a catalog-mutation path.

Whether and how a consumer turns its own runtime capability observations into
a candidate catalog contribution or a locally scoped policy suggestion is
that consumer's design. No runtime submission hook is proposed here.

## 9. Trust model

This document tightens #779's trust framing rather than replacing it. Entries
assert *need*, not authorization, but incorrect data has two different
outcomes:

- An understated floor omits a requirement and can cause the tool or its
  end-to-end workflow to fail under the resulting policy.
- An overstated floor can fail against a narrower consumer ceiling. If a
  consumer instead approves or adopts it and its ceiling permits the request,
  the effective policy contains unnecessary capability.

What changes from #779 is the review bar. #779 described community-contributed,
unsigned, unwarranted data. This contract requires named-role approval
([§7](#7-contribution-and-review)) before an entry publishes, and publishes
under an immutable revision that CI checks against published history
([§10](#10-immutable-revisions)).
That raises confidence in the data; it does not change what the data *is*. The
catalog still carries no security guarantee or independent authority. A
consumer must review the requirement and intersect it with its own policy
rather than adopt it outright. The catalog can influence a consumer's
decision, so consumer approval and restrictive ceilings remain required even
though the catalog cannot grant capability by itself. See
[#779 §2.1](https://github.com/microsoft/mxc/pull/779) for why that layering is
honest about what such a choice costs.

## 10. Immutable revisions

Published catalog revisions are immutable. A correction, including a security
fix to an over-broad entry, publishes a new `catalogRevision` and bumps the
affected `entryRevision`; it never rewrites a revision a consumer may already
have cached or recorded in an audit trail.

## 11. Backward compatibility

- No change to `SandboxPolicy` or `ContainerConfig` schema.
- No change to executor behavior.
- The MXC SDK changes are additive. Catalog lookup requires an explicit call
  to a new SDK API; existing MXC callers see no behavior change.
- The policy store is not part of MXC 1.0. Once it ships, the catalog schema
  stays compatible within MXC 1.x.

## 12. Test plan

**Resolver (Rust, exercised directly and through each SDK binding)**

- shared conformance fixtures produce the same results and failure reasons
  across bindings (`mxc_policy_store` and the C# tests replay them; the
  `mxc-sdk`, `mxc_ffi`, and Node tests cover the same paths through each
  binding)
- a single input and a one-element array are equivalent; the policy-only API
  returns the diagnostic API's policy
- every row of the per-pair table in [§4.5](#45-resolution-per-tool-and-intent),
  including an out-of-range version with an unsupported intent
- `vers` parsing and comparison for all five schemes
- an embedded purl version is ignored with a warning
- strong identity outranks weak, intent support breaks ties, exact
  architecture outranks neutral, and remaining ties fail as `ambiguous_match`
- invocation-name casing per platform
- native-architecture default with its warning; explicit architecture; no
  cross-architecture fallback; neutral fallback warning
- unresolved symbols prevent policy output; host-derived symbols only for the
  current host platform, overridable by the caller
- composition: read-write supersedes read-only, conflicting denies removed
  with diagnostics, outbound allows unioned without a network veto, mixed
  policy versions and inexpressible combinations rejected
- dependency chains compose each entry once; dependency records keep distinct
  required ranges

**Data (CI)**

- the contract and every entry validate, and every effective policy is
  materialized and validated ([§4.7](#47-build-time-validation))
- the reviewer view is current; every entry has a bundled conformance case
- catalog and entry revision monotonicity; published revisions unchanged when
  `MXC_POLICY_STORE_BASE_REF` is set

**Integration**

- each SDK performs lookup without launching a sandbox or selecting a
  backend; lookup requires no network access
- the bundled catalog revision matches `getCatalogInfo()`; selecting an
  unavailable revision fails explicitly
- an SDK update leaves previously accepted consumer policies unchanged
- a representative tool that fails under a minimal consumer policy succeeds
  once its resolved floor is composed in, and still fails when the consumer's
  policy forbids what the floor requests

## 13. Open questions

Recommended answers are proposals for review, not decisions.

| Question | Status |
|---|---|
| Final API names? | Pending API review. Current names are kept; they may drop "Sandbox" (for example, `resolvePolicy`). |
| Exact policy validator and `SandboxPolicy` version mapping used at build time | Open. The prototype validates composed effective policies with the catalog contract's own subset validator. |
| Purl normalization and equality, including purl-embedded versions | Open. The prototype lower-cases the type, compares namespace and name as written, and ignores an embedded version with a warning. |
| `vers` conformance for `pypi` and `nuget` | The prototype implements practical subsets of PEP 440 and NuGet versioning. Exact conformance is open. |
| How does the catalog schema evolve? | Keep it compatible within MXC 1.x; a breaking change waits for a major version. |
| Is invocation-name-only identity accepted automatically? | No: it requires `allowWeakIdentityFallback`. |
| Are private or enterprise catalog overlays in scope? | Deferred until the shared catalog contract and API are stable. |
| Who owns catalog schema, data, and API review? | MXC, through its normal API review, with named catalog and security reviewers. |

## 14. Related work

- [`microsoft/mxc#779`](https://github.com/microsoft/mxc/pull/779) - Sandbox
  Config Floors feature spec. This document's data model, floor/policy
  direction argument, and identity-layering analysis build directly on it.
- [`ChazGo/mxc#1`](https://github.com/ChazGo/mxc/pull/1) - the prototype of
  this design inside the MXC SDKs, pending API review.
- [`docs/sandbox-policy/0.8.0/policy.md`](../sandbox-policy/0.8.0/policy.md) -
  the `SandboxPolicy` contract every catalog entry embeds.
- [`docs/versioning.md`](../versioning.md) - the versioning model
  [§4.1](#41-versions) builds on.
