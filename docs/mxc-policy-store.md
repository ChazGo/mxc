# Feature Spec: MXC Policy Store

**Status:** Proposed — review-ready draft, targeting sign-off around October 2,
2026. This is a proposed contract for review, not an approved, shipped, or
implemented catalog, and October 2 is a review-readiness target rather than a
delivery or personal commitment.

---

## 1. Problem Statement

[#779](https://github.com/microsoft/mxc/pull/779) proposed **config floors**: a
repository-hosted table of minimum sandbox requirements per known tool, plus an
SDK resolver, so a host does not have to discover by hand what a tool needs to
run inside a sandbox. That proposal intentionally left several things loose —
one version number for the whole table, name-only identity as the common case,
one policy per entry regardless of platform, and no distinction between "look up
one tool's requirement" and "inspect the whole catalog."

Turning that proposal into something MXC can host and consumers can build
against requires tightening exactly those points into a contract: independently
versioned catalog and entries, ordered identity strength, complete per-platform
requirement statements, deterministic dependency resolution, a resolver API
that is safe to call in a hot path, immutable published revisions, and a
reviewed contribution pipeline. This document is that contract. It reuses
[#779](https://github.com/microsoft/mxc/pull/779)'s data model and API shape
almost entirely; where it diverges, it says so.

This document does not restate general MXC sandboxing concepts already covered
by [`docs/sandbox-policy/0.8.0/policy.md`](sandbox-policy/0.8.0/policy.md) or
[`docs/versioning.md`](versioning.md). It covers only what a policy store adds.

### Non-goals

- This does not change what the sandbox backend enforces, or `SandboxPolicy` /
  `ContainerConfig` schema semantics. A catalog entry embeds an existing
  `SandboxPolicy`; it does not define a parallel vocabulary.
- This is not a trust or attestation mechanism, and it does not authorize
  anything. See [§9](#9-trust-model).
- This does not define how any specific consumer stores, displays, or lets a
  user approve requirements. See [§2](#2-ownership-boundary).
- This does not define Learning Mode's candidate-generation or review UX. See
  [§8](#8-relationship-to-learning-mode).

## 2. Ownership boundary

MXC owns exactly one thing here: an integrity-validated, versioned, read-only
catalog of known-tool sandbox requirements, and the SDK surface that resolves
it. The catalog states what a tool needs. It does not grant access, does not
modify caller state, and does not create a sandbox.

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
itself: MXC returns a candidate requirement or `undefined`; the consumer decides
whether, and how, to act on it. This mirrors #779's floor/policy distinction,
discussed further in [§3](#3-relationship-to-the-config-floors-proposal):
a resolved entry is a lower bound asserted by the tool ecosystem, never an
upper bound the host is required to grant.

## 3. Relationship to the config-floors proposal

| #779 (config floors) | This document (policy store) |
|---|---|
| One `schemaVersion` for the whole table | Four independent versions: `catalogSchemaVersion`, `catalogRevision`, per-entry `entryRevision`, and per-variant `sandboxPolicy.version` ([§4.1](#41-versions)) |
| `identity` predicates, unordered | `identity` explicitly ordered strongest → weakest, with defined match/fallback behavior ([§4.3](#43-identity)) |
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

These move independently. A `SandboxPolicy` version bump does not require a new
catalog revision, and a catalog revision does not require every entry to bump.
Tool version constraints (`versionRange`, below) are a fifth, orthogonal axis —
they describe which builds of the *tool* an entry was observed against, not
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
- Exactly one most-specific `platformVariants` entry may match a given
  platform/architecture. No matching variant means the tool is unsupported on
  that platform — not that it needs an empty policy, and not `undefined`
  conflated with "requires nothing" (see [#779, "Defaults and
  omission"](https://github.com/microsoft/mxc/pull/779)).
- Symbols (`${project_root}`, `${npm_cache}`, OS well-known folders) are
  resolved by the resolver before a policy is returned; catalog data never
  ships a literal, machine-specific path. This is unchanged from #779.
- An embedded `sandboxPolicy` is validated against the real `SandboxPolicy`
  schema for its declared `version` — the catalog schema does not duplicate
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
  automatically, or requires opt-in, is unresolved — see [§13](#13-open-questions).

### 4.4 Platform variants

Supported platforms are `windows`, `linux`, and `macos`. A platform variant is
a complete requirement statement — one full `SandboxPolicy`, not a patch
applied to a base policy — plus any platform-specific dependencies. Variants
are never merged. This closes #779's open question about entries that need a
genuinely different policy per platform, not just different symbol resolution:
they now can, by declaring more than one variant.

A platform variant must not name a specific MXC containment backend. Policies
stay backend-neutral; the selected backend still decides whether a stated
requirement can be realized on that host.

### 4.5 Dependencies and composition

Dependencies reference another entry's `entryId`, with an optional tool
`versionRange`, and live inside the platform variant when platform-specific.
Resolution is transitive, cycle-rejecting, and deterministic, and the resolver
returns the full dependency chain alongside the result.

Unlike #779, this document does not treat "union the policies" as sufficient
composition. Silently unioning arbitrary `SandboxPolicy` objects across a
dependency chain hides exactly the kind of conflicting-field problem that
made #779 exclude `proxy` from the embedded object. For the first contract
version, composition across a dependency chain is limited to fields where the
merge rule is unambiguous:

- filesystem path lists, normalized and de-duplicated;
- network host lists, normalized and de-duplicated;
- boolean capability requirements, where `true` always means "this dependency
  requires the capability."

Timeout, clipboard, lifecycle, UI, and proxy fields are excluded from
cross-entry composition until each has an explicit rule; catalog validation
rejects a dependency combination that would require merging one of them,
rather than picking an implicit answer.

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
  architecture?: string;
  catalogRevision?: string;
  allowWeakIdentityFallback?: boolean;
}

interface ResolvedToolEntry {
  entryId: string;
  entryRevision: number;
  catalogRevision: string;
  matchedIdentity: { kind: string; strength: "strong" | "weak" };
  resolvedDependencies: Array<{ entryId: string; entryRevision: number }>;
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
per tool. `undefined` means no acceptable identity/platform match — never an
empty policy (same distinction #779 makes; see [§4.2](#42-entry-shape)).

### 5.2 Setup and inspection

```ts
listCatalogEntries(): CatalogEntryMetadata[];
getCatalogInfo(): { catalogSchemaVersion: string; catalogRevision: string };
```

This supports setup UI, catalog browsing, and update decisions without paying
the cost of policy resolution, and keeps "give me everything" out of the
runtime lookup path entirely.

### 5.3 Consumer obligations

A consumer that uses this API:

1. Decides whether automatic lookup is enabled at all.
2. Stores any accepted result per tool, keyed by `entryId`, `entryRevision`,
   and `catalogRevision` — never as an unattributed merged policy blob.
3. Keeps catalog-derived requirements in a layer separate from its own user,
   learned, and invocation-specific policy.
4. Applies its own authorization, elevation, and restrictive-composition
   rules on top.
5. Enforces its OS, enterprise, device, and backend ceilings regardless of
   what the catalog returned.
6. Fails closed — falls back to its own restrictive baseline, and does not
   run uncontained — when a required entry cannot be realized on the current
   host/backend.
7. Records matched identity, catalog/entry revision, warnings, and approval
   state in its own audit trail.

MXC never writes a consumer's policy store. A consumer's own capability
observation (see [§8](#8-relationship-to-learning-mode)) can produce candidate
evidence for a future contribution to this catalog; it is not a mechanism for
mutating the catalog at request time.

## 6. Packaging and repository ownership

Canonical source lives in `microsoft/mxc`, with schema, semantic validation,
and generated package artifacts, following this repo's existing schema
codegen model ([`docs/schema-codegen.md`](schema-codegen.md)). Whether the
catalog ships inside each SDK package or as a separately versioned artifact
consumed by all SDKs is open; a separately versioned artifact is recommended
so catalog updates are not coupled to SDK release cadence. See
[§13](#13-open-questions).

## 7. Contribution and review

- Contributions are pull requests against `microsoft/mxc`. No client or SDK
  can write a catalog entry at runtime.
- Every entry change includes identity evidence, supported tool version
  range(s), platform evidence, a minimized requirement set, test fixtures,
  and provenance.
- CI validates schema conformance, exact `SandboxPolicy` version registration,
  identity uniqueness, dependency closure and cycle-freedom, symbol validity,
  absence of unsafe user-specific literal paths, unsupported-field rejection,
  deterministic resolution, and package inclusion.
- A new entry or a requirement expansion requires one MXC SDK/catalog-owner
  approval and one MXC security/policy-reviewer approval, plus tool- or
  scenario-owner evidence where available.
- A requirement reduction requires regression evidence that every supported
  tool version still functions under the narrower requirement.

## 8. Relationship to Learning Mode

MXC's upstream learning-mode capabilities (`learningModeLogging`,
`permissiveLearningMode`, `captureDenials` — see
[`docs/learning-mode/capabilities.md`](learning-mode/capabilities.md)) are the
substrate a contributor can use to observe what a tool actually touches, the
same way [#779 §5.1](https://github.com/microsoft/mxc/pull/779) describes for
config floors. That observation workflow is unchanged by this document and
remains **a contributor step that happens before a pull request**, not
consumer runtime behavior and not a catalog-mutation path.

Whether and how a consumer turns its own runtime capability observations into
a candidate catalog contribution, or into a locally-scoped policy suggestion
for its own user, is that consumer's design — most likely deferred to the
consumer, and out of scope for the catalog contract itself unless a future
revision of this contract needs to define hooks for submitting observation
evidence. No such hook is proposed here.

## 9. Trust model

This document tightens #779's trust framing rather than replacing it: entries
still assert *need*, not authorization, and a wrong or malicious entry can
only overstate need, which surfaces as a tool that fails under the consumer's
existing policy — never as an authority the consumer did not already grant.

What changes from #779 is the review bar. #779 described community-contributed,
unsigned, unwarranted data. This contract requires named-role approval
([§7](#7-contribution-and-review)) before an entry publishes, and publishes
under an immutable, integrity-validated revision ([§10](#10-immutable-revisions)).
That raises confidence in the data; it does not change what the data *is*. The
catalog still carries no security guarantee, and a consumer must still
intersect a resolved entry with its own policy rather than adopt it as policy
outright (though nothing prevents that choice — see [#779 §2.1](https://github.com/microsoft/mxc/pull/779)
for why that layering is honest about what such a choice costs).

## 10. Immutable revisions

Published catalog revisions are immutable. A correction — including a security
fix to an over-broad entry — publishes a new `catalogRevision` and bumps the
affected `entryRevision`; it never rewrites a revision a consumer may already
have cached or recorded in an audit trail.

## 11. Backward compatibility

- No change to `SandboxPolicy` or `ContainerConfig` schema.
- No change to executor behavior.
- New SDK surface only; existing callers that never call it see no behavior
  change.
- Given the schema is expected to move as open questions resolve, the catalog
  and resolver should land under the experimental surface and promote through
  this repo's normal promotion process once the shape has settled, per
  [`docs/authoring-a-new-feature.md`](authoring-a-new-feature.md).

## 12. Test plan

**Resolver (SDK unit tests)**

- single tool → expected entry; unknown tool → `undefined`, never an empty policy
- identity match strength selection and weak-identity fallback behavior
- version-range mismatch produces a warning, not a refusal
- exactly one platform variant selected; no matching variant → `undefined`
- dependency chain resolution, including cycles (terminate, no duplication)
- restricted composition rules ([§4.5](#45-dependencies-and-composition)):
  path/host de-duplication and boolean-OR merge; a dependency requiring an
  unsupported composed field is rejected at validation time, not resolved
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
| Repository and package ownership: does the catalog ship inside each SDK package, or as a separately versioned artifact? | Canonical source in `microsoft/mxc`; prefer a separately versioned generated artifact to decouple catalog updates from SDK releases. |
| Is invocation-name-only identity accepted automatically, or does it require explicit consumer opt-in? | Treat it as a fallback requiring explicit opt-in (`allowWeakIdentityFallback`), not the default. |
| What happens on a detected tool-version mismatch — `undefined`, or a warning-bearing result the consumer may still use? | Return the resolved result with a warning; refusing outright removes information the consumer needs to decide for itself. |
| Are private or enterprise catalog overlays in scope, and if so with what precedence? | Defer until the shared catalog contract and its API are stable; define precedence explicitly before any SDK implementation adds overlay support. |
| Should the first contract version's composition vocabulary expand beyond [§4.5](#45-dependencies-and-composition) before implementation? | No — ship the restricted vocabulary first; expand only with an explicit, reviewed composition rule per field. |
| Who are the named MXC owners for schema/API review vs. policy/security review? | To be assigned before this document is finalized; not a contract-shape question. |

## 14. Related work

- [`microsoft/mxc#779`](https://github.com/microsoft/mxc/pull/779) — Sandbox
  Config Floors feature spec. This document's data model, floor/policy
  direction argument, and identity-layering analysis build directly on it.
- [`ChazGo/mxc#1`](https://github.com/ChazGo/mxc/pull/1) — draft SDK resolver
  and catalog prototype exercising lookup, dependency closure, and symbol
  resolution against an earlier version of this shape.
- [`docs/sandbox-policy/0.8.0/policy.md`](sandbox-policy/0.8.0/policy.md) —
  the `SandboxPolicy` contract every catalog entry embeds.
- [`docs/versioning.md`](versioning.md) — the versioning model
  [§4.1](#41-versions) builds on.
