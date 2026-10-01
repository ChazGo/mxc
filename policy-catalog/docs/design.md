<!--
Verbatim copy of the Known-tool Policy Floors / Policy Store design proposal.

Source: docs/mxc-policy-store.md in Chaz Gordish's "chazgo-vigilant-enigma"
MXC worktree: base commit 34537f85384c00f72564f931077d1790c5a86a02
(microsoft/mxc#1309) plus uncommitted edits as of 2026-09-29.
Everything below the "BEGIN VERBATIM COPY" marker is byte-identical to that
file after git's CRLF-to-LF checkout normalization. No text or links were
changed.

Relative links in the copy point into the MXC repository (docs/), not into
this directory. Absolute equivalents:
- authoring-a-new-feature.md -> https://github.com/microsoft/mxc/blob/main/docs/authoring-a-new-feature.md
- learning-mode/capabilities.md -> https://github.com/microsoft/mxc/blob/main/docs/learning-mode/capabilities.md
- sandbox-policy/0.8.0/policy.md -> https://github.com/microsoft/mxc/blob/main/docs/sandbox-policy/0.8.0/policy.md
- versioning.md -> https://github.com/microsoft/mxc/blob/main/docs/versioning.md
-->
<!-- BEGIN VERBATIM COPY -->
# Feature Spec: Known-tool Policy Floors

**Status:** Proposed public preview catalog. This is not an approved, shipped, or
implemented catalog.

**IMPORTANT NOTE:** This is a time-limited bridge, not a long-term supported
Microsoft product. Applying a published floor does not guarantee that a tool's
end-to-end workflow will work under process containment. The catalog and its
dedicated repository are expected to be retired when Learning Mode provides
the replacement workflow.

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
by [`docs/sandbox-policy/0.8.0/policy.md`](sandbox-policy/0.8.0/policy.md) or
[`docs/versioning.md`](versioning.md). It covers only what a policy store adds.

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

This is a standalone catalog and library proposal. Following the feature-impact
checklist in [`docs/authoring-a-new-feature.md`](authoring-a-new-feature.md):

- **Policy changes:** None. Catalog entries embed an existing, registered
  `SandboxPolicy`.
- **ContainerConfig changes:** None. The catalog does not add configuration
  fields or change omission behavior in an existing contract.
- **OS and backend changes:** None. Backends continue to validate whether they
  can enforce the resolved policy.
- **MXC SDK changes:** None. This proposal does not add catalog lookup, types,
  or resolver code to the MXC SDKs.
- **Standalone libraries:** The dedicated catalog repository provides its own
  library API for TypeScript/JavaScript, Rust, and C#/.NET. See
  [§6](#6-intended-repository-and-packaging-boundary).

The proposed **public preview** designation describes the catalog's support
status. It does not add an MXC schema feature, activate the
`--experimental` runtime gate, or change executor behavior.

Defaults and omission behavior are:

- Existing callers do not perform catalog lookup automatically. A consumer
  must explicitly enable or invoke it.
- Omitted `ResolveContext.platform` uses the current host platform.
- Omitted `ResolveContext.architecture` uses the device's native system
  architecture, not the architecture of the library's process or a detected
  tool build. Explicit caller selection takes precedence. See the selection
  rules and emulation risk in [§4.4](#44-platform-variants).
- Omitted `ResolveContext.catalogRevision` uses the currently installed
  catalog revision.
- Omitted `ResolveContext.allowWeakIdentityFallback` is `false`.
- Omitted `projectRoot` and `symbols` provide no caller overrides. The resolver
  may use approved host-known symbols, but it does not invent machine-specific
  values. A selected entry with an unresolved required symbol is not
  resolvable.
- Omitted `packageUrl` or `detectedVersion` supplies no matching evidence. The
  resolver does not fabricate either value.
- If no policy can be resolved, `resolveSandboxPolicy` returns `undefined`.
  `resolveSandboxPolicyWithDiagnostics` instead returns a result whose `policy` is
  `undefined`, preserving the diagnostics. The consumer's restrictive baseline
  remains unchanged.

## 2. Ownership boundary

The proposed dedicated catalog project owns an integrity-validated, versioned,
read-only data set of known-tool sandbox requirements, its resolver libraries,
and their public APIs. MXC continues to own the existing `SandboxPolicy`
contract, but not the catalog entries, libraries, repository, or publication
lifecycle.

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
| One `schemaVersion` for the whole table | Four separate version dimensions: `catalogSchemaVersion`, `catalogRevision`, per-entry `entryRevision`, and per-variant `sandboxPolicy.version` ([§4.1](#41-versions)) |
| Strongest satisfied identity predicate describes a match | All eligible matching entries contribute; identity evidence is retained without stronger matches suppressing weaker ones ([§4.3](#43-identity)) |
| One `sandboxPolicy` per entry; `when.platform` only conditions dependencies | One complete `SandboxPolicy` per platform variant; a variant cannot name a containment backend ([§4.4](#44-platform-variants)) |
| `requires` composition unspecified beyond "union" | Composition limited to a small, explicit, field-by-field set for the first contract version; everything else is rejected until a rule exists ([§4.5](#45-dependencies-and-composition)) |
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
| `sandboxPolicy.version` | The exact registered `SandboxPolicy` contract version used by one platform variant. |

These identifiers serve separate purposes and do not advance in lockstep.
Registering a new `SandboxPolicy` contract does not change existing catalog
data and therefore does not require a new catalog revision. Migrating a
platform variant to that contract changes the entry's content, so publication
of that migration must increment both `entryRevision` and `catalogRevision`.
A catalog revision may still change without incrementing unaffected entries.
Tool version constraints (`versionRange`, below) are a fifth, orthogonal axis.
They describe which builds of a tool an entry was observed against, not
anything about the catalog.

### 4.2 Entry shape

```json
{
  "entryId": "tool:npm",
  "entryRevision": 3,
  "displayName": "npm / npx",
  "identity": [
    { "kind": "purl", "value": "pkg:npm/npm", "versionRange": ">=10 <12" },
    { "kind": "invocation-name", "names": ["npm", "npm.cmd", "npx", "npx.cmd"] }
  ],
  "platformVariants": [
    {
      "when": { "platform": "windows" },
      "dependencies": [{ "entryId": "tool:node", "versionRange": ">=22" }],
      "sandboxPolicy": {
        "version": "0.9.0-alpha",
        "filesystem": {
          "readonlyPaths": ["${npm_prefix}"],
          "readwritePaths": ["${project_root}", "${npm_cache}"]
        }
      }
    }
  ],
  "provenance": { "method": "reviewed-observation", "sourceRevision": "opaque-review-reference" }
}
```

Invariants:

- `entryId` is stable, unique, namespaced, and is the only key `requires`/
  dependency edges may reference.
- `entryRevision` increases on every semantic change to the entry.
- Variant selection follows the deterministic rules in
  [§4.4](#44-platform-variants). No selected variant means the tool is
  unsupported on that platform, not that it needs an empty policy, and not
  `undefined` conflated with "requires nothing" (see [#779, "Defaults and
  omission"](https://github.com/microsoft/mxc/pull/779)).
- Symbols (`${project_root}`, `${npm_cache}`, OS well-known folders) are
  resolved by the resolver before a policy is returned; catalog data never
  ships a literal, machine-specific path. This is unchanged from #779.
- An embedded `sandboxPolicy` is validated against the real `SandboxPolicy`
  schema for its declared `version`. The catalog schema does not duplicate
  that validation.

### 4.3 Identity

`identity` describes the predicates a candidate can satisfy for an entry,
using the layering #779 §3.1 establishes (invocation name vs. launcher artifact
vs. executing image; falsifiable-against-a-local-artifact as the admission test
for a new kind).

Matching is additive across entries. For each input tool, the resolver collects
every entry with a satisfied, eligible identity predicate and an applicable
platform/architecture variant. It does not choose a single winning entry:

- A package-identity match does not suppress another entry's eligible
  invocation-name match. Equal-strength matches to different entries also
  contribute; they are not ambiguity errors.
- Identity strength describes the matching evidence, not precedence between
  entries. Diagnostics retain all satisfied identity predicates for each
  contributing entry.
- Multiple predicates matching the same entry do not add its policy multiple
  times. Entries shared across input tools or dependency chains likewise
  contribute once, while diagnostics preserve the per-input matches.
- Multiple matching entries for one input produce a diagnostic warning, not
  a refusal. Their policies must still satisfy the composition rules in
  [§4.5](#45-dependencies-and-composition).
- Diagnostic tool records follow input order, matches are ordered by
  `entryId`, and matched predicates follow their declaration order within the
  entry. Catalog file order does not select or exclude a match.

The existing matching qualifications remain:

- A version range on an identity predicate is advisory matching evidence, not
  a gate. A detected mismatch returns a diagnostic alongside the resolved
  policy rather than silently degrading precision, and the consumer decides
  what to do with the mismatch.
- Invocation-name-only identity is the always-available fallback, not the
  default outcome. Whether a consumer accepts an invocation-name-only match
  automatically, or requires opt-in, is unresolved. See
  [§13](#13-open-questions). Under the current proposed default, an entry
  matched only by invocation name participates when
  `allowWeakIdentityFallback` is `true`. Compose-all-matches does not bypass
  that option.

### 4.4 Platform variants

Supported platforms are `windows`, `linux`, and `macos`. Supported architecture
selectors are `x64` and `arm64`. A variant selector has this closed shape:

```ts
interface PlatformVariantSelector {
  platform: "windows" | "linux" | "macos";
  architecture?: "x64" | "arm64";
}
```

A platform variant is a complete requirement statement: one full
`SandboxPolicy`, not a patch applied to a base policy, plus any
platform-specific dependencies. Variants are never merged. Architecture is a
catalog selector, not a field added to the embedded `SandboxPolicy`. Omitting
`when.architecture` makes a catalog variant architecture-neutral; omitting the
caller's `ResolveContext.architecture` instead requests the host default.

Selection first filters by platform, then uses the following precedence:

| Caller context | Preferred variant | Fallback |
|---|---|---|
| Explicit `architecture: "x64"` | x64 for the selected platform | Architecture-neutral for that platform |
| Explicit `architecture: "arm64"` | ARM64 for the selected platform | Architecture-neutral for that platform |
| Architecture omitted | Device's native system architecture for the selected platform | Architecture-neutral for that platform |

The native system architecture is the architecture reported by the host OS,
not the architecture of the process hosting the library. For example, on an
ARM64 device with both x64 and ARM64 catalog variants and no neutral variant,
omitting architecture selects ARM64. An explicit `architecture: "x64"` selects
x64 on that same device. The resolver does not require a neutral variant to
return a result when the effective architecture has an exact match.

Catalog validation rejects duplicate exact selectors and more than one
architecture-neutral variant for the same platform. If neither an exact nor
architecture-neutral variant exists, that entry contributes no match, not an
empty policy or a variant for a different architecture. A failure to determine
the native system architecture when it is needed is a library error, not a
guessed selection.

**Emulation risk:** A host-derived default does not establish the architecture
of the installed tool. An x64 tool running under emulation on an ARM64 device
may need the x64 variant rather than the default ARM64 variant. The resolver
does not inspect or run the tool to discover its architecture. Callers that
know the relevant tool and runtime requirements should select architecture
explicitly and remain responsible for deciding whether the result applies.
Neither explicit selection nor a host default guarantees that the returned
policy is sufficient or minimal. Host-derived selection and neutral fallback
are surfaced through `diagnostics.warnings` by the diagnostics API
([§5.1](#51-runtime-lookup)).

A platform variant must not name a specific MXC containment backend. Policies
stay backend-neutral; the selected backend still decides whether a stated
requirement can be realized on that host.

### 4.5 Dependencies and composition

Dependencies reference another entry's `entryId` and live inside the platform
variant when platform-specific. An optional `versionRange` records which
dependency versions supplied the reviewed evidence. The v1 resolver has no
dependency inventory, so it does not evaluate that range or use it for
matching. It returns the range as unevaluated metadata for consumer inspection.
Catalog validation checks only that the range is syntactically valid.
Resolution is otherwise transitive, cycle-rejecting, and deterministic, and
the diagnostics API returns the resolved dependency metadata alongside the
policy.

The same composition rules apply to all entries matched by one tool, entries
matched by different tools in an array, and their transitive dependencies.
Each selected entry contributes its policy once, even when reached through
multiple inputs or dependency edges. Repeated contribution is de-duplicated
by entry ID within the selected catalog revision, not by discarding match
attribution. A shared `ResolveContext` applies to the whole lookup.

Unlike #779, this document does not treat "union the policies" as sufficient
composition. Silently unioning arbitrary `SandboxPolicy` objects across a
dependency chain hides exactly the kind of conflicting-field problem that
made #779 exclude `proxy` from the embedded object. For the first contract
version, cross-entry composition is limited to the exact
`filesystem.deniedPaths`, `filesystem.readonlyPaths`, and
`filesystem.readwritePaths` fields:

1. Every policy in the selected entries and their dependency closure must
   declare the same `sandboxPolicy.version`.
2. Paths are resolved, normalized using the selected platform's path rules,
   and de-duplicated within the same access class.
3. Catalog validation rejects equal or ancestor/descendant paths that occur in
   different access classes. It never chooses between denied, read-only, and
   read-write access implicitly.
4. The non-conflicting, normalized lists are merged into the returned policy.

The v1 contract does not compose `network`. In particular, it defines no merge
for `network.egress.default`, `network.egress.allow`,
`network.egress.deny`, `network.ingress.default`, or
`network.ingress.hostLoopback`. Composition rejects a selected set of entries
where policies from more than one entry would require composing any `network`
field. The same rejection applies to timeout, clipboard, lifecycle, UI, proxy,
and every other policy field without an explicit cross-entry rule. A lookup
resolving to only one entry without dependencies may still use
catalog-supported policy fields because no cross-entry merge occurs.

## 5. API surface

The standalone libraries separate runtime resolution from catalog inspection.
Resolution accepts one tool or an array and composes all applicable matching
entries and dependencies into one `SandboxPolicy`. Callers choose a policy-only
operation or a diagnostic operation over the same resolution logic. Neither
implicitly returns the whole catalog. These are in-process library calls, not
a hosted service or additions to the MXC SDKs.

The signatures below use TypeScript to describe the shared contract. Rust and
C# expose the same operations and metadata with idiomatic names and types.
TypeScript and C# expose single-tool and array overloads; Rust uses an idiomatic
one-or-many input type because it does not support function overloading. An
absent policy is `undefined` in TypeScript/JavaScript, `None` in Rust, and
`null` in C#. Library failures remain distinct from policy absence.

### 5.1 Runtime lookup

```ts
interface ToolCandidate {
  invocationName: string;
  packageUrl?: string;
  detectedVersion?: string;
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

interface SandboxConfigResolution {
  policy: SandboxPolicy | undefined;
  diagnostics: {
    catalogRevision: string;
    tools: Array<{
      inputIndex: number;
      matches: Array<{
        entryId: string;
        entryRevision: number;
        matchedIdentities: Array<{
          kind: string;
          strength: "strong" | "weak";
        }>;
      }>;
    }>;
    resolvedDependencies: Array<{
      entryId: string;
      entryRevision: number;
      requiredVersionRange?: string;
    }>;
    warnings: string[];
  };
}

declare function resolveSandboxPolicy(
  tool: ToolInput,
  ctx?: ResolveContext
): SandboxPolicy | undefined;

declare function resolveSandboxPolicy(
  tools: readonly ToolInput[],
  ctx?: ResolveContext
): SandboxPolicy | undefined;

declare function resolveSandboxPolicyWithDiagnostics(
  tool: ToolInput,
  ctx?: ResolveContext
): SandboxConfigResolution;

declare function resolveSandboxPolicyWithDiagnostics(
  tools: readonly ToolInput[],
  ctx?: ResolveContext
): SandboxConfigResolution;
```

A string input is shorthand for `{ invocationName: tool }`; it supplies no
package or version evidence and follows the same weak-identity option as an
object input. For example, name-only lookup under the current proposed opt-in
rule is:

```ts
const ctx = { allowWeakIdentityFallback: true };
const policy = resolveSandboxPolicy("npm", ctx);
const combinedPolicy = resolveSandboxPolicy(["git", "npm"], ctx);
const result = resolveSandboxPolicyWithDiagnostics("npm", ctx);
const combinedResult =
  resolveSandboxPolicyWithDiagnostics(["git", "npm"], ctx);
```

Single-tool lookup is equivalent to a one-element array; its diagnostic
`inputIndex` is `0`. A caller retaining separate policies per tool can use
single-tool calls. A caller wanting one sandbox for several tools passes an
array. Both forms compose every eligible matching entry, not just the
strongest match, and the selected dependencies.

`resolveSandboxPolicy` returns the composed `SandboxPolicy` directly, not a wrapper
or a `ContainerConfig`. `resolveSandboxPolicyWithDiagnostics` returns that same
policy with attribution and warnings from the same resolution pass. Callers
choose one operation; retrieving diagnostics does not require a second lookup
or process-global "last result" state.

Following #779, an unmatched input contributes no requirements while matched
inputs still contribute. Each input has a diagnostic record; an unmatched
input has an empty `matches` list and a warning. An empty input array or an
all-unmatched lookup produces no policy, not an empty policy:
`resolveSandboxPolicy` returns `undefined`, while the diagnostics operation returns
a `SandboxConfigResolution` with `policy: undefined`. An empty array has no
per-input records. Unresolved required symbols in selected entries prevent a
policy from being returned and produce diagnostics; they are not grounds for
silently omitting a selected requirement to produce a partial policy.

Multiple matching entries for one input are listed in `matches`, with a
warning identifying that input and the contributing entry IDs. Shared entries
remain attributed to every matching input even though their policy is
composed once. Dependency diagnostics retain each distinct
entry/revision/required-version-range combination, ordered by those fields;
repeated metadata does not mean repeated policy contribution.

When architecture is omitted, diagnostics include a warning naming the
effective native system architecture and stating that the tool's architecture
was not verified. Architecture-neutral fallback is also identified. These
diagnostics describe selection; they do not attest to the installed tool's
architecture. The policy-only operation does not expose warnings or
attribution; consumers needing them use `resolveSandboxPolicyWithDiagnostics`.

### 5.2 Setup and inspection

```ts
type CatalogPlatform = "windows" | "linux" | "macos";
type CatalogArchitecture = "x64" | "arm64";

type CatalogIdentityMetadata =
  | { kind: "purl"; value: string; versionRange?: string }
  | { kind: "invocation-name"; names: string[] };

interface CatalogEntryMetadata {
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
runtime lookup path entirely. Metadata exposes selectors, dependency IDs, and
provenance, but not an unresolved or resolved policy body.

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

The catalog libraries never write a consumer's policy store. A consumer's own
capability observation (see [§8](#8-relationship-to-learning-mode)) can produce
candidate evidence for a future contribution to this catalog; it is not a
mechanism for mutating the catalog at request time.

## 6. Intended repository and packaging boundary

The catalog is intended to live in a new public repository outside
`microsoft/mxc`. Its schema, entries, resolver libraries, contribution history,
validation, and publication workflow belong there. This specification remains
in MXC while the proposed contract is reviewed. No catalog repository or
package is created by this proposal.

MXC retains the existing `SandboxPolicy` contract. The catalog libraries
produce policy data conforming to that contract; they do not require an MXC
executor or execution library to perform lookup. A consumer that uses MXC
passes its final, authorized policy to an existing MXC SDK separately.
Catalog and library releases do not require an MXC SDK release or changes to
MXC repository governance.

### 6.1 Library distribution and consumption

The initial library language coverage matches MXC's current first-party SDK
languages, but the packages are owned and released by the catalog project:

| Language | Distribution | API form |
|---|---|---|
| TypeScript / JavaScript | npm package | JavaScript library with TypeScript declarations |
| Rust | Cargo crate | Public Rust library API |
| C# / .NET | NuGet package | Managed library API |

Repository and package names remain to be selected. This language match does
not require copying MXC's native-binding architecture or exposing sandbox
execution operations.

A consumer:

1. Installs and pins the standalone library package for its language. Each
   package includes a reviewed default catalog revision for local use.
2. Calls `getCatalogInfo()` or `listCatalogEntries()` for inspection.
   `resolveSandboxPolicy()` returns a policy for one tool or an array;
   `resolveSandboxPolicyWithDiagnostics()` adds match attribution and warnings.
3. Handles policy absence without widening its restrictive baseline. It
   reviews the composed policy and uses the diagnostics operation when it
   needs contributing identities, revisions, and warnings, applying the
   consumer obligations in [§5.3](#53-consumer-obligations).
4. Supplies its final policy to its chosen execution integration. The catalog
   library does not launch a sandbox.

Lookup is local and does not download updates, contact a hosted service, or
run the candidate tool. `ResolveContext.catalogRevision` selects an available
local revision, not a network lookup; an explicitly requested revision that
is unavailable is an error, not a substitution with a different revision.
An omitted revision uses the library's installed default.

Catalog revisions are also published as immutable, language-neutral data
artifacts. A library package version identifies the library release, not the
catalog revision or embedded `SandboxPolicy.version`; it declares the catalog
schema and policy versions it supports and reports its bundled
`catalogRevision`. Publishing newer data can update the packages' bundled
revision without changing resolver behavior. Installing an update does not
rewrite a consumer's previously accepted per-tool policies.

### 6.2 Cross-language consistency and support

All three libraries use the same catalog format and shared conformance
fixtures. Given the same catalog revision, tool inputs, and explicit resolution
context, they must agree on matching, variant selection, dependency metadata,
resolved policy, warnings, and failure categories. Language-specific absence
and error types must preserve those distinctions.

Shared fixtures cover platform path semantics as well as ordinary lookup;
matching function names alone is not compatibility. Package CI must also
exercise installation, public API usage, and host-derived defaults on the
supported platforms. Implementation sharing between languages is a separate
engineering decision, not a requirement to depend on MXC's engine.

Supporting three languages includes maintaining parity, dependencies,
documentation, and releases, not only writing the initial implementations.
The libraries and catalog have the same limited public-preview horizon and
are intended to retire together when Learning Mode replaces this workflow.

## 7. Contribution and review

- Catalog contributions are pull requests against the dedicated catalog
  repository. No client or SDK can write a catalog entry at runtime.
- Every entry change includes identity evidence, supported tool version
  range(s), platform evidence, a minimized requirement set, test fixtures,
  and provenance.
- CI validates schema conformance, exact `SandboxPolicy` version registration,
  entry-ID uniqueness, dependency closure and cycle-freedom, symbol validity,
  absence of unsafe user-specific literal paths, unsupported-field rejection,
  deterministic resolution, and package inclusion.
- A new entry or a requirement expansion requires one catalog-owner approval
  and one security/policy-reviewer approval, plus tool- or scenario-owner
  evidence where available.
- A requirement reduction requires regression evidence that every supported
  tool version still functions under the narrower requirement.
- Library API and implementation contributions are reviewed in the dedicated
  catalog repository. These contribution requirements do not require
  applications to seek maintainer approval to use the public catalog or
  libraries.

## 8. Relationship to Learning Mode

Learning Mode is the intended long-term solution. The known-tool catalog only
reduces immediate first-run failures while that workflow is completed. It is
not a parallel long-term policy platform.

MXC's learning-mode capabilities (`learningModeLogging`,
`permissiveLearningMode`, `captureDenials`; see
[`docs/learning-mode/capabilities.md`](learning-mode/capabilities.md)) are the
substrate a contributor can use to observe what a tool actually touches, the
same way [#779 §5.1](https://github.com/microsoft/mxc/pull/779) describes for
config floors. That observation workflow is unchanged by this document and
remains **a contributor step that happens before a pull request**, not
consumer runtime behavior and not a catalog-mutation path.

Whether and how a consumer turns its own runtime capability observations into
a candidate catalog contribution or a locally scoped policy suggestion is
that consumer's design. No runtime submission hook is proposed here. When
Learning Mode can provide the required observation and policy-authoring
experience directly, this catalog should be retired rather than promoted into
a durable platform.

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
under an immutable, integrity-validated revision ([§10](#10-immutable-revisions)).
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
- No change to the MXC SDK APIs or dependencies. Catalog lookup requires an
  explicit call to a standalone library; existing MXC callers see no behavior
  change.
- Catalog schema and API compatibility are limited to the stopgap's support
  horizon. Retirement in favor of Learning Mode is an expected outcome, not a
  normal promotion milestone.

## 12. Test plan

**Resolver libraries (TypeScript/JavaScript, Rust, and C#/.NET)**

- shared conformance fixtures produce equivalent results and failure
  categories in all three languages
- one-tool and one-element-array overloads produce equivalent policies and
  diagnostics; the simple API returns the same policy as the diagnostic API
- multiple input tools compose all matching entries and dependencies into one
  policy; repeated inputs or shared dependencies do not duplicate contributions
- known and unknown inputs compose the known requirements and report each
  unmatched input; empty and all-unmatched arrays return no policy, never an
  empty policy, while the diagnostic API preserves the resolution metadata
- omitted context uses host platform and native system architecture, the
  installed catalog revision, no caller symbol overrides, and no weak-identity
  fallback
- an unresolved required symbol prevents policy output, with diagnostics,
  rather than silently omitting selected requirements
- one input matching several entries composes all eligible matches, including
  equal-strength matches; a stronger match does not suppress a weaker eligible
  match, and diagnostics preserve all matching entries and identity evidence
- multiple predicates matching the same entry contribute that policy once;
  file order does not change matching or composition
- string shorthand and object inputs obey the same weak-identity fallback
  option; additive matching does not bypass it
- version-range mismatch produces a warning, not a refusal
- exact-architecture variant precedes the platform-only variant; duplicate
  selectors are rejected; no matching variant produces `undefined`
- on an ARM64 host with both architecture-specific variants and no neutral
  variant, omitted architecture selects ARM64; explicit x64 selects x64
- a library process running as x64 under emulation on an ARM64 host still
  defaults to the native ARM64 system architecture, not its process
  architecture
- a missing exact variant falls back to the platform's neutral variant;
  a different architecture's variant is never used as a fallback
- successful host-derived selection and neutral fallback produce the
  diagnostics specified in [§5.1](#51-runtime-lookup); host-architecture
  detection failure produces a library error, not a guessed match
- dependency chain resolution, including cycles (terminate, no duplication)
- dependency `versionRange` is returned as unevaluated metadata and never used
  for v1 resolver matching
- restricted composition rules ([§4.5](#45-dependencies-and-composition)):
  same-class path de-duplication; cross-class path overlap, mixed policy
  versions, network fields, and other unsupported composed fields are rejected
  at validation time rather than resolved
- symbol resolution on Windows, Linux, and macOS

**Data (CI)**

- every entry and platform variant validates against the catalog schema and
  the `SandboxPolicy` schema for its declared version
- `dependencies[].entryId` references resolve within the same catalog revision
- no literal absolute user-specific paths; no wildcard filesystem/network grants
- catalog/entry revision monotonicity across a proposed change

**Integration**

- each package installs and performs lookup without an MXC executor or
  execution library; lookup requires no network access
- the bundled catalog revision matches `getCatalogInfo()`; selecting an
  unavailable revision fails explicitly, without falling back to another
  revision
- a package update leaves previously accepted consumer policies unchanged
- a representative tool that fails under a minimal consumer policy succeeds
  once its resolved entry is composed in
- the same tool still fails when the consumer's policy forbids what the entry
  requests (the floor never widens the consumer's ceiling)

## 13. Open questions

Recommended answers are proposals for review, not decisions.

| Question | Recommended answer |
|---|---|
| What is the dedicated repository name and owning team? | Use a public repository outside `microsoft/mxc`; publish a separately versioned artifact so catalog updates are not coupled to SDK releases. |
| Is invocation-name-only identity accepted automatically, or does it require explicit consumer opt-in? | Treat it as a fallback requiring explicit opt-in (`allowWeakIdentityFallback`), not the default. |
| What happens on a detected tool-version mismatch: `undefined`, or a warning-bearing result the consumer may still use? | Return the resolved result with a warning; refusing outright removes information the consumer needs to decide for itself. |
| Are private or enterprise catalog overlays in scope, and if so with what precedence? | Defer until the shared catalog contract and its API are stable; define precedence explicitly before any library implementation adds overlay support. |
| Should the first contract version's composition vocabulary expand beyond [§4.5](#45-dependencies-and-composition) before implementation? | No. Start with conflict-rejecting filesystem composition and expand only with an explicit, reviewed rule per field. |
| Who owns catalog schema, data, and library API review? | Assign catalog, library, and security reviewers in the dedicated repository; no MXC SDK integration is proposed. |
| Should the libraries share a resolver implementation or implement the contract independently? | Choose based on dependency footprint and maintenance cost, with shared conformance fixtures required either way. |

## 14. Related work

- [`microsoft/mxc#779`](https://github.com/microsoft/mxc/pull/779) - Sandbox
  Config Floors feature spec. This document's data model, floor/policy
  direction argument, and identity-layering analysis build directly on it.
- [`ChazGo/mxc#1`](https://github.com/ChazGo/mxc/pull/1) - draft SDK resolver
  and catalog prototype exercising lookup, dependency closure, and symbol
  resolution against an earlier version of this shape.
- [`docs/sandbox-policy/0.8.0/policy.md`](sandbox-policy/0.8.0/policy.md) -
  the `SandboxPolicy` contract every catalog entry embeds.
- [`docs/versioning.md`](versioning.md) - the versioning model
  [§4.1](#41-versions) builds on.
