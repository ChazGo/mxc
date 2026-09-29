# Feature Spec: Known-tool Policy Floors

**Status:** Proposed experimental stopgap. This is not an approved, shipped, or
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

## 2. Ownership boundary

The proposed dedicated catalog project owns an integrity-validated, versioned,
read-only data set of known-tool sandbox requirements. MXC continues to own
`SandboxPolicy`. If an MXC SDK consumption API is approved, MXC also owns that
API, but not the catalog entries, repository, or publication lifecycle.

The catalog states a candidate minimum that a tool needs. It does not grant
access, modify caller state, create a sandbox, or guarantee workflow success.

Everything else is a consumer decision:

- Whether automatic catalog lookup is enabled at all.
- Access-profile mapping, elevation preference, and per-tool authorization.
- Persistence of accepted requirements (which tool, which catalog/entry
  revision, when).
- Composition with the consumer's own user, learned, and invocation-specific
  policy layers, and with non-overridable OS/enterprise/device ceilings.
- Approval UX, audit, and the final call into `createConfigFromPolicy()` /
  sandbox creation.

A catalog lookup can only ever narrow what a consumer still has to decide for
itself. The resolver returns a candidate requirement or `undefined`; the
consumer decides whether and how to act on it. This mirrors #779's floor/policy
distinction, discussed further in
[§3](#3-relationship-to-the-config-floors-proposal): a resolved entry is a
lower bound asserted by the tool ecosystem, never an upper bound the host is
required to grant.

## 3. Relationship to the config-floors proposal

| #779 (config floors) | This document (policy store) |
|---|---|
| One `schemaVersion` for the whole table | Four separate version dimensions: `catalogSchemaVersion`, `catalogRevision`, per-entry `entryRevision`, and per-variant `sandboxPolicy.version` ([§4.1](#41-versions)) |
| `identity` predicates, unordered | `identity` explicitly ordered strongest to weakest, with defined match/fallback behavior ([§4.3](#43-identity)) |
| One `sandboxPolicy` per entry; `when.platform` only conditions dependencies | One complete `SandboxPolicy` per platform variant; a variant cannot name a containment backend ([§4.4](#44-platform-variants)) |
| `requires` composition unspecified beyond "union" | Composition limited to a small, explicit, field-by-field set for the first contract version; everything else is rejected until a rule exists ([§4.5](#45-dependencies-and-composition)) |
| Single resolver function, no separate catalog-inspection API | Resolver split from a separate metadata/inspection API ([§5](#5-api-surface)) |
| No revision/publication model | Immutable published catalog revisions; corrections publish a new revision ([§10](#10-immutable-revisions)) |

The data model, the floor/policy direction argument, the identity layering
problem (invocation name vs. launcher artifact vs. executing image), and the
trust framing all carry forward from #779 essentially unchanged; this document
does not re-derive them, and cites the relevant #779 section instead.

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

`identity` is an ordered list, strongest predicate first, per the layering
#779 §3.1 establishes (invocation name vs. launcher artifact vs. executing
image; falsifiable-against-a-local-artifact as the admission test for a new
kind). This document adds:

- A version range on an identity predicate is advisory matching evidence, not
  a gate. A detected mismatch returns a diagnostic alongside the resolved
  policy rather than silently degrading precision, and the consumer decides
  what to do with the mismatch.
- Invocation-name-only identity is the always-available fallback, not the
  default outcome. Whether a consumer accepts an invocation-name-only match
  automatically, or requires opt-in, is unresolved. See
  [§13](#13-open-questions).

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
platform-specific dependencies. Variants are never merged. Selection first
filters by `platform`, then prefers an exact `architecture` match over a
variant that omits `architecture`. Catalog validation rejects duplicate exact
selectors and more than one architecture-neutral variant for the same
platform. If neither an exact nor architecture-neutral variant exists, the
entry is unsupported on that host.

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
the resolver returns the full dependency chain alongside the result.

Unlike #779, this document does not treat "union the policies" as sufficient
composition. Silently unioning arbitrary `SandboxPolicy` objects across a
dependency chain hides exactly the kind of conflicting-field problem that
made #779 exclude `proxy` from the embedded object. For the first contract
version, cross-entry composition is limited to the exact
`filesystem.deniedPaths`, `filesystem.readonlyPaths`, and
`filesystem.readwritePaths` fields:

1. Every policy in the dependency closure must declare the same
   `sandboxPolicy.version`.
2. Paths are resolved, normalized using the selected platform's path rules,
   and de-duplicated within the same access class.
3. Catalog validation rejects equal or ancestor/descendant paths that occur in
   different access classes. It never chooses between denied, read-only, and
   read-write access implicitly.
4. The non-conflicting, normalized lists are merged into the returned policy.

The v1 contract does not compose `network`. In particular, it defines no merge
for `network.egress.default`, `network.egress.allow`,
`network.egress.deny`, `network.ingress.default`, or
`network.ingress.hostLoopback`. Catalog validation rejects a dependency closure
where policies from more than one entry would require composing any `network`
field. The same rejection applies to timeout, clipboard, lifecycle, UI, proxy,
and every other policy field without an explicit cross-entry rule. Entries
without dependencies may still use catalog-supported policy fields because no
cross-entry merge occurs.

## 5. API surface

Two APIs, kept deliberately separate so that "resolve one tool's requirement"
stays a cheap, hot-path-safe call and never implicitly returns the whole
catalog.

### 5.1 Runtime lookup

```ts
interface ToolCandidate {
  invocationName: string;
  packageUrl?: string;
  detectedVersion?: string;
}

interface ResolveContext {
  projectRoot?: string;
  symbols?: Record<string, string>;
  platform?: "windows" | "linux" | "macos";
  architecture?: "x64" | "arm64";
  catalogRevision?: string;
  allowWeakIdentityFallback?: boolean;
}

interface ResolvedToolEntry {
  entryId: string;
  entryRevision: number;
  catalogRevision: string;
  matchedIdentity: { kind: string; strength: "strong" | "weak" };
  resolvedDependencies: Array<{
    entryId: string;
    entryRevision: number;
    requiredVersionRange?: string;
  }>;
  policy: SandboxPolicy;
  warnings: string[];
}

resolveCatalogEntry(
  tool: ToolCandidate,
  ctx?: ResolveContext
): ResolvedToolEntry | undefined;
```

`resolveCatalogEntry` is singular by design, not an array-in/array-out call:
a caller composing and persisting requirements per tool (rather than as one
opaque merged blob) needs each result independently addressable and
independently attributable. A caller resolving several tools calls it once
per tool. `undefined` means no acceptable identity/platform match, never an
empty policy (same distinction #779 makes; see [§4.2](#42-entry-shape)).

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
2. Stores any accepted result per tool, keyed by `entryId`, `entryRevision`,
   and `catalogRevision`, never as an unattributed merged policy blob.
3. Keeps catalog-derived requirements in a layer separate from its own user,
   learned, and invocation-specific policy.
4. Applies its own authorization, elevation, and restrictive-composition
   rules on top.
5. Enforces its OS, enterprise, device, and backend ceilings regardless of
   what the catalog returned.
6. Fails closed when a required entry cannot be realized on the current
   host/backend. It falls back to its own restrictive baseline and does not
   run uncontained.
7. Records matched identity, catalog/entry revision, warnings, and approval
   state in its own audit trail.

MXC never writes a consumer's policy store. A consumer's own capability
observation (see [§8](#8-relationship-to-learning-mode)) can produce candidate
evidence for a future contribution to this catalog; it is not a mechanism for
mutating the catalog at request time.

## 6. Intended repository and packaging boundary

The catalog is intended to live in a new public repository outside
`microsoft/mxc`. Its schema, entries, contribution history, validation, and
publication workflow belong there. This specification remains in MXC only
while the contract and optional consumption boundary are reviewed. No catalog
repository is created by this proposal.

MXC retains the existing `SandboxPolicy` contract. If approved, MXC may also
retain SDK types and resolver code that consume a separately versioned catalog
artifact. Catalog releases must not require an MXC SDK release, and catalog
governance must not become part of MXC repository governance. The dedicated
repository and artifact have the same limited horizon as the stopgap and may
be retired when Learning Mode replaces them.

## 7. Contribution and review

- Catalog contributions are pull requests against the dedicated catalog
  repository. No client or SDK can write a catalog entry at runtime.
- Every entry change includes identity evidence, supported tool version
  range(s), platform evidence, a minimized requirement set, test fixtures,
  and provenance.
- CI validates schema conformance, exact `SandboxPolicy` version registration,
  identity uniqueness, dependency closure and cycle-freedom, symbol validity,
  absence of unsafe user-specific literal paths, unsupported-field rejection,
  deterministic resolution, and package inclusion.
- A new entry or a requirement expansion requires one catalog-owner approval
  and one security/policy-reviewer approval, plus tool- or scenario-owner
  evidence where available.
- A requirement reduction requires regression evidence that every supported
  tool version still functions under the narrower requirement.
- Any MXC SDK consumption change is reviewed separately in `microsoft/mxc`.

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
- Any SDK consumption surface is opt-in and experimental. Existing callers
  that never call it see no behavior change.
- Catalog schema and API compatibility are limited to the stopgap's support
  horizon. Retirement in favor of Learning Mode is an expected outcome, not a
  normal promotion milestone.

## 12. Test plan

**Resolver (SDK unit tests)**

- single tool returns the expected entry; unknown tool returns `undefined`,
  never an empty policy
- identity match strength selection and weak-identity fallback behavior
- version-range mismatch produces a warning, not a refusal
- exact-architecture variant precedes the platform-only variant; duplicate
  selectors are rejected; no matching variant produces `undefined`
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
| Are private or enterprise catalog overlays in scope, and if so with what precedence? | Defer until the shared catalog contract and its API are stable; define precedence explicitly before any SDK implementation adds overlay support. |
| Should the first contract version's composition vocabulary expand beyond [§4.5](#45-dependencies-and-composition) before implementation? | No. Start with conflict-rejecting filesystem composition and expand only with an explicit, reviewed rule per field. |
| Who owns catalog schema/data review versus optional MXC SDK integration? | Assign catalog and security owners in the dedicated repository; keep MXC SDK review with existing MXC owners. |

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
