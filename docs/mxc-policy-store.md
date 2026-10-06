# Feature Spec: Known-tool Policy Floors

**Status:** Under review. Pending API sign-off from the designated MXC SDK
reviewers before merge and implementation. Not part of MXC 1.0; no later release
is committed.

**IMPORTANT NOTE:** The Policy Store provides best-effort baseline requirements
believed necessary for representative tool workflows. Floors may broaden file
access to unblock tools, but guarantee neither success nor safety. Users and
clients must review that access; their settings and enterprise policies can
further constrain or override the recommendation.

---

## 1. Problem Statement

When users first enable sandboxing without known-good configurations, tools
can break and users spend time debugging or disable containment. The catalog
supplies a suggested starting policy intended to make requested tools work
out of the box when the user has not configured the sandbox and no enterprise
policy overrides it.

Clients can configure the sandbox and override these defaults; user and
enterprise policy can further restrict them, including later changes. The
catalog is a best-effort suggestion, not authorization or a guarantee of
success or safety.

[#779](https://github.com/microsoft/mxc/pull/779) proposed the initial config
floor data model and SDK resolver. This document develops that proposal into
an SDK catalog contract with identity, platform, dependency, revision, and
inspection behavior. It reuses #779's model where possible and calls out
differences directly.

This document does not restate general MXC sandboxing concepts already covered
by the [v1 SDK reference](https://github.com/microsoft/mxc/blob/894f4c159705f5f470727e4fa1e363a2abec88f1/docs/reference/node/v1/README.md) or
[`docs/versioning.md`](versioning.md). It covers only what a policy store adds.

### Non-goals

- This does not change backend enforcement or the MXC 1.x request contract.
  Catalog data uses the access fields of `ContainerRequest`; it does not
  define a parallel policy vocabulary or supply commands.
- This is not a trust or attestation mechanism, and it does not authorize
  anything. See [§9](#9-trust-model).
- This is not a guarantee that a complete tool workflow will succeed under
  process containment.
- This does not define how any specific consumer stores, displays, or lets a
  user approve requirements. See [§2](#2-ownership-boundary).
- This does not define Learning Mode's candidate-generation or review UX. See
  [§8](#8-relationship-to-learning-mode).

### MXC feature impact and defaults

**MXC 1.0 alignment:** Command-free lookup and composition are unchanged.
`ContainerRequirements` reuses the filesystem, network, UI, and timeout types
from the [v1 request](https://github.com/microsoft/mxc/blob/894f4c159705f5f470727e4fa1e363a2abec88f1/sdk/node/src/v1/types.ts#L549-L597);
the caller adds execution settings later. The SDK owns wire version selection.

| Earlier spec | MXC 1.x alignment |
|---|---|
| `SandboxPolicy`, `resolveSandboxPolicy` | `ContainerRequirements`, `resolveToolRequirements` |
| `SandboxConfigResolution.policy` | `ToolRequirementsResolution.requirements` |
| `default.sandboxPolicy` and its `version` | Access-only `default.requirements`; catalog `sdkContractVersion` records validation target |
| `ui.allowWindows` | `ui.disable` with inverse meaning; `clipboard` and `allowInputInjection` retain their meanings |
| Synchronous Node resolution | Promise-returning plain verbs, with no `Async` suffix |

This is an MXC SDK API, not a command-line utility. Following the feature-impact
checklist in
[`docs/authoring-a-new-feature.md`](authoring-a-new-feature.md):

- **Policy changes:** None. Entries use the existing v1 request access fields.
- **ContainerConfig changes:** None. The catalog does not add configuration
  fields or change omission behavior in an existing contract.
- **OS and backend changes:** None. Backends continue to validate whether they
  can enforce the resolved policy.
- **MXC SDK changes:** Add policy-resolution and inspection APIs to the existing
  TypeScript/JavaScript, Rust, and C#/.NET SDKs, returning `ContainerRequirements`.
- **Delivery:** V1 policy data is embedded in the MXC native library at build
  time and ships in the existing MXC SDK packages. See
  [§6](#6-intended-repository-and-packaging-boundary).

Defaults and omission behavior are:

- `ResolveContext` is optional and carries lookup context only. No command or
  execution settings are required, inferred, or returned by the resolver.
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
  uses supported local discovery and shared documented defaults for required
  symbols as specified in [§4.2](#42-entry-shape). It does not invent a project
  root or an installation path. A selected entry with an unresolved required
  symbol is not resolvable.
- The resolver does not fabricate an omitted `packageUrl` or `detectedVersion`.
- Omitted `ToolCandidate.detectedVersion` selects the unversioned default,
  without a version warning.
- Omitted `ToolCandidate.intent` selects the base plus all intents of the
  effective version policy. An unsupported intent contributes no policy and
  produces an `intent_unsupported` warning.
- If no policy can be resolved, `resolveToolRequirements` yields `undefined`.
  `resolveToolRequirementsWithDiagnostics` yields a result whose `requirements` is
  `undefined`, preserving the diagnostics. The consumer's restrictive baseline
  remains unchanged.

## 2. Ownership boundary

MXC owns the reviewed, versioned, read-only catalog, its resolution and
inspection APIs, and their SDK publication lifecycle in `microsoft/mxc`.
The requirements type and its underlying request field types remain MXC-owned as well.

The catalog states a best-effort baseline for a tool. It does not grant
access, modify caller state, create a sandbox, or guarantee workflow success.
Filesystem and network composition combine lower-bound requirements using the
least restrictive access needed by the selected tools. This is distinct from
the consumer's restrictive composition with its own security ceilings.

Everything else is a consumer decision:

- Whether automatic catalog lookup is enabled at all.
- Access-profile mapping, elevation preference, and per-tool authorization.
- Persistence of accepted requirements (which tools, which catalog/entry
  revision, when).
- Composition with the consumer's own user, learned, and invocation-specific
  policy layers, and with non-overridable OS/enterprise/device ceilings.
- Approval UX, audit, and the final call to the v1 `run` / `spawn` operations
  (or their language equivalents).

A catalog lookup can only ever narrow what a consumer still has to decide for
itself. `resolveToolRequirements` yields candidate requirements or
`undefined`; its diagnostics counterpart also reports how that result was
obtained. The consumer decides whether and how to act on it. This mirrors
#779's floor/policy distinction, discussed further in
[§3](#3-relationship-to-the-config-floors-proposal): a resolved entry is a
lower bound asserted by the tool ecosystem, never an upper bound the host is
required to grant.

## 3. Relationship to the config-floors proposal

| #779 (config floors) | This document (policy store) |
|---|---|
| One `schemaVersion` for the whole table | Separate catalog shape, catalog/entry revisions, and SDK-owned validation target ([§4.1](#41-versions)) |
| Strongest satisfied identity predicate describes a match | Select the most specific identity/intent/architecture match per tool; tied matches are errors ([§4.3](#43-identity)) |
| One `sandboxPolicy` per entry; `when.platform` only conditions dependencies | One unversioned default per entry, with platform/intent data and optional additive version overlays ([§4.2](#42-entry-shape)) |
| `requires` composition unspecified beyond "union" | Composition limited to a small, explicit, field-by-field set for the first contract version; everything else is rejected until a rule exists ([§4.5](#45-dependencies-and-composition)) |
| `getSandboxConfigForTool(tools: string[])` returns one composed policy | Replaced by `resolveToolRequirements`, accepting one tool or an array and optional lookup context; `resolveToolRequirementsWithDiagnostics` adds attribution ([§5](#5-api-surface)) |
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
| `sdkContractVersion` | Catalog-revision metadata recording its published validation target (`1.0.0` initially), not a caller-selected wire version. |

These identifiers serve separate purposes and do not advance in lockstep.
Registering a new exact request contract does not change existing catalog
data. A semantic entry change increments its `entryRevision` and the containing
`catalogRevision`; changing only catalog metadata, including its published
validation target, increments only `catalogRevision`. Revalidating unchanged
data does not rewrite its published metadata or either revision.
Tool versions and `versionRange` values are distinct from catalog revisions.

Entries use MXC's v1 request access fields; breaking changes require a new
major surface. No entry or caller chooses a wire version. At SDK build time,
every bundled revision is revalidated against that SDK's exact
`SDK_CONTRACT_VERSION` and its matching schema, not automatically accepted by
semver range. An older published validation target remains selectable when
it is an exact registered stable target in the same major and the current
build has revalidated its data successfully. Other revisions are not bundled.
At lookup, select only those build-validated revisions; never dispatch their
historical target or substitute another revision. The SDK owns wire version
selection as described in [versioning.md](versioning.md).

### 4.2 Entry shape

```json
{
  "entryId": "tool:git",
  "entryRevision": 3,
  "displayName": "Git",
  "versionScheme": "intdot",
  "identity": [
    { "kind": "purl", "value": "pkg:generic/git" },
    { "kind": "invocation-name", "names": ["git", "git.exe"] }
  ],
  "default": {
    "requirements": {
      "filesystem": {
        "readonlyPaths": ["${git_prefix}"],
        "readwritePaths": ["${project_root}"]
      }
    },
    "intents": {
      "local": {
        "exampleSubcommands": ["status", "diff"],
        "policyAdditions": {}
      },
      "fetch": {
        "exampleSubcommands": ["fetch", "pull"],
        "policyAdditions": {
          "network": {
            "egress": {
              "allow": [
                { "to": [{ "cidr": "192.0.2.10/32" }], "ports": [{ "protocol": "tcp", "port": 443 }] }
              ]
            }
          }
        }
      },
      "push": {
        "exampleSubcommands": ["push"],
        "policyAdditions": {
          "network": {
            "egress": {
              "allow": [
                { "to": [{ "cidr": "192.0.2.10/32" }], "ports": [{ "protocol": "tcp", "port": 22 }] }
              ]
            }
          }
        }
      }
    }
  },
  "platformVariants": [
    {
      "when": { "platform": "windows" },
      "policyAdditions": {
        "filesystem": { "readonlyPaths": ["${programData}/Git"] }
      }
    }
  ],
  "versionVariants": [
    {
      "versionRange": "vers:intdot/>=2.40|<2.50",
      "intentAdditions": {
        "push": { "dependencies": [{ "entryId": "tool:ssh" }] }
      }
    },
    {
      "versionRange": "vers:intdot/>=2.50|<3",
      "newIntents": {
        "bundle-fetch": {
          "exampleSubcommands": ["clone --bundle-uri=<uri>"],
          "policyAdditions": {
            "filesystem": { "readwritePaths": ["${temp_dir}/git-bundles"] },
            "network": {
              "egress": {
                "allow": [
                  { "to": [{ "cidr": "198.51.100.20/32" }], "ports": [{ "protocol": "tcp", "port": 443 }] }
                ]
              }
            }
          }
        }
      }
    }
  ],
  "provenance": { "method": "reviewed-observation", "sourceRevision": "opaque-review-reference" }
}
```

Each entry has exactly one unversioned `default`, containing the conservative
subset common to all tool versions and platforms, not the newest version's
behavior. It contains minimal base access fields in `requirements` and intent
additions. Neither stored nor resolved requirements contain execution settings.
The caller supplies a command later when constructing a `ContainerRequest`.
Unversioned means no tool-version selector, not a caller-selectable wire version.

`versionVariants` is an optional list of non-overlapping VERS ranges using the
entry's `versionScheme`. Effective policy data is `default` plus the selected
platform/architecture overlay and at most one selected version overlay.
Neither dimension cascades across multiple variants. In either overlay,
`policyAdditions` and `dependencies` add to the base, `intentAdditions` adds to
intents the default declares, and `newIntents` defines intents the default
does not declare. Additions use only fields
with defined monotone composition: no replacements, deletions, narrowing,
negative operations, or removal/renaming of inherited intents.
In v1, access additions are read-only/read-write paths and outbound allow
rules. Overlay deny rules, scalar replacements, and delete/rename operations
are invalid.

To narrow requirements for newer versions, remove the access from the default
and add it only to the older version ranges that need it. Each effective
variant must retain all default access and intents. Intent bodies themselves
remain additions to their effective base, not full policy copies.

The Git example illustrates the structure, not a verified Git compatibility
matrix. `local` adds nothing; default fetch and push add their network needs.
The first range adds an SSH dependency to push through `intentAdditions`. The
second defines `bundle-fetch` through `newIntents`, without inheriting the
first range's SSH dependency. The endpoints are
documentation addresses. The Windows overlay adds `${programData}/Git`;
`programData` is the Windows common application-data directory and is resolved
only when that overlay is selected.

Intent names are exact identifiers scoped to the tool: Git's `local`, `fetch`,
and `push` are supplied as `intent: "local"`, `"fetch"`, or `"push"`.
`exampleSubcommands` contains non-normative hints for callers. The caller maps
command lines to intents; the catalog does not parse or execute command lines.

Invariants:

- `entryId` is stable, unique, namespaced, and is the only key `dependencies`
  edges may reference.
- `entryRevision` increases on every semantic change to the entry.
- Exactly one `default` is required, including when no version variants exist.
  Entries with only versioned variants, multiple defaults, or a variant tagged
  as default are invalid.
- Each entry declares a supported `versionScheme`. Version ranges are valid,
  non-overlapping, and use that scheme. Overlap is a catalog error, not a
  precedence choice.
- Intent names are unique in each materialized default-plus-overlays result.
  `intentAdditions` may name only intents the default declares. `newIntents`
  cannot reuse a default intent name or a name the other selected overlay
  defines. All additions and dependencies are versioned with the containing
  entry.
- Variant selection follows the deterministic rules in
  [§4.4](#44-platform-variants). No matching platform overlay leaves the common
  default unchanged; it does not produce an empty policy or a no-match result.
- Symbols (`${project_root}`, `${npm_cache}`, OS well-known folders) are
  resolved by the resolver before a policy is returned; catalog data never
  ships a literal, machine-specific path. This is unchanged from #779.
- Embedded request access fields use the v1 SDK contract and the validation
  pipeline below; there is no standalone `SandboxPolicy` schema.

`default.requirements` is closed to the three filesystem path lists,
directional network policy, v1 UI fields, and unsigned 32-bit `timeoutMs`.
Commands, wire versions, backend configuration, runtime proxy values,
lifecycle/cleanup settings, environment data, and unknown fields are rejected,
including inside overlays. `policyAdditions` retains its access-only meaning.
When UI is present, `disable` is required; `clipboard` and `allowInputInjection`
are optional. UI/timeout composition and overlay limits remain those of §4.5.

Build validation materializes every platform/architecture/version/intent
combination, enforcing the closed field set, selectors, and additive rules.
Bind fixture symbols and a validation-only command, then expose the SDK's exact
[`OneShotRequest` before normalization](https://github.com/microsoft/mxc/blob/894f4c159705f5f470727e4fa1e363a2abec88f1/src/mxc-sdk/src/policy/exact/v1_0.rs#L184).
This hook must be implemented: `prepare_request` returns normalized data.
Validate the exact serialization against the SDK target's schema, initially
[`mxc-config.schema.1.0.0.json`](https://github.com/microsoft/mxc/blob/894f4c159705f5f470727e4fa1e363a2abec88f1/schemas/stable/mxc-config.schema.1.0.0.json),
mapping `allowInputInjection` to wire `injection` and supplying the SDK-owned
wire version. Check CIDR/exclusion containment, protocol/port relationships,
numeric bounds, and unknown fields. Run semantic normalization on a disposable
copy; its restrictive path precedence must not alter the original floors.
The fixture command is never executed or returned. Lookup repeats validation
with real symbols and the object-identity checks in §4.5. Neither stage probes
or launches a backend; enforcement and capability checks still occur at execution.

Symbol definitions are shared across entries and versioned with the selected
catalog revision. Each definition describes the symbol's permitted sources
and may include a `defaults` map from platform to path template. This extends
the catalog contract, not `ContainerRequest`. Entries continue to reference
symbols rather than repeat defaults. For example, default metadata in the
shared symbol registry can include:

```json
{
  "symbols": {
    "npm_cache": {
      "defaults": {
        "linux": "${user_home}/.npm",
        "macos": "${user_home}/.npm"
      }
    },
    "programData": {
      "source": "host",
      "description": "Windows common application-data directory."
    }
  }
}
```

This is a metadata fragment, not a complete symbol definition. Default
templates may reference approved host-known symbols; they cannot contain
commands or executable discovery logic. Shared definitions and defaults are
embedded with the entries and cannot change behind a pinned catalog revision.

For symbols required by selected entries and dependencies, precedence is:

1. Explicit caller values from `projectRoot` or `symbols`, as applicable.
2. Supported local discovery, including `PATH`, known host locations, and
   relevant tool configuration overrides.
3. A documented default for the selected platform, if applicable.
4. Unresolved, with diagnostics and no partial policy.

Tool-specific discovery runs in the library, not in catalog-supplied code.
It does not execute candidate tools, install software, or contact the network.
Automatic discovery and host-derived values describe the current host and
environment; callers targeting another execution environment supply overrides.
A failed configuration read is an explicit library error, not evidence that
no override exists and a default should be used. Discovery does not verify
tool identity. The diagnostics operation reports the source and resolved value
of each discovered or defaulted symbol through `diagnostics.warnings`.

### 4.3 Identity

Lookup uses tool identity, an optional detected version, and optional
tool-defined intent. Package URL
provides strong identity and invocation name is the opt-in fallback. Raw
command lines and calling-application identity are not lookup keys; the caller
maps operations to intents and applies its own restrictions. Platform and
architecture remain resolution context.

`identity` describes the predicates a candidate can satisfy for an entry,
using the layering #779 §3.1 establishes (invocation name vs. launcher artifact
vs. executing image; falsifiable-against-a-local-artifact as the admission test
for a new kind).

Caller-supplied identity is not verified identity. A `packageUrl` match does
not prove that the installed tool belongs to that package; the caller is
responsible for verifying that association. The library does not inspect the
tool to verify it.

Parse candidate and catalog package URLs using the
[PURL component rules](https://github.com/package-url/purl-spec/blob/7cd2d3442fb9c88155db17ada7c911b40ec22d41/docs/specification/standard/Clause-5-Package-URL-Specification.md)
and applicable type definitions, including percent-decoding before comparison.
Compare only type, namespace, and name. Catalog comparison is
locale-independent and case-insensitive for type and namespace; the name
follows its package type's normalization and case rules. Namespace folding is
a catalog lookup rule, not a claim that every ecosystem treats namespaces
case-insensitively.

Ignore candidate version, qualifiers, and subpath for matching and record
`purl_components_ignored` when any is present. Only `detectedVersion` supplies
tool-version evidence. Validate the complete PURL before ignoring components.
An invalid candidate PURL makes that pair `tool_unmatched` with `purl_invalid`;
do not repair it, retry invocation-name matching, or fail other pairs.
Invalid catalog PURLs are catalog validation errors.

Invocation-name matching is locale-independent and case-insensitive on Windows
and macOS, and exact on Linux. This does not change the caller's command or
package-identity matching.

For each input tool, consider entries with an eligible identity predicate and
their applicable platform additions. Rank matches in this order:

1. Package URL match over invocation-name-only match.
2. A requested intent declared in the default or applicable overlays over no
   matching intent declaration.
3. Exact architecture over architecture-neutral additions or the common default.

Select the unique highest-ranked match. Two distinct matches tied after all
three comparisons produce an error, not a union or a file-order tie-break.
Multiple predicates satisfied within the same entry count as one identity
match, ranked by its strongest satisfied predicate.

After selecting the entry, resolve its version and then its intent as below.
Version ranges do not choose a different tool entry.
Failure for that pair does not retry another entry or a wildcard entry.
Composition includes only contributing pairs in an array request.

Composition applies across tools in an array request, not across competing
identity matches for one tool. Diagnostics follow input order and identify the
single selected entry, satisfied identity predicates, and selected intents.

Invocation-name-only matching requires explicit opt-in
(`allowWeakIdentityFallback: true`). Intent selection does not bypass that
option.

`versionRange` uses the Package-URL project's
[VERS syntax](https://github.com/package-url/vers-spec/blob/797c842a4afebf258e6710a68cd60306afc36708/docs/specification/standard/Clause-5-VERS-Specification.md):
`vers:<type>/<constraints>`, for example `vers:npm/>=10.0.0|<12.0.0`.
V1 supports these [version types](https://github.com/package-url/vers-spec/blob/797c842a4afebf258e6710a68cd60306afc36708/docs/types/vers-types.md):

| VERS type | Version parsing and comparison |
|---|---|
| `npm` | node-semver version rules, as referenced by the VERS npm definition |
| `semver` | Semantic Versioning 2.0.0 |
| `pypi` | PEP 440 |
| `nuget` | NuGet version normalization and comparison |
| `intdot` | VERS dotted-integer comparison for numeric tool versions such as Git `2.40` |

VERS defines the range syntax and interval evaluation; the named type defines
version parsing, normalization, and ordering, including prereleases and
accepted prefixes. Do not apply npm's native range syntax to other types or
strip version prefixes independently of the named rules. `generic` is not
supported while its upstream comparison algorithm is unspecified.

Catalog authoring/build validation rejects malformed VERS strings, invalid
constraints, and unsupported types as invalid catalog data.

For each requested tool/intent pair, parse `detectedVersion` under the selected
entry's `versionScheme`. Use only that scheme's specified parsing and
normalization; do not repair rejected input, try other schemes, or perform
fuzzy matching.

| Version input | Effective policy data | Version status / warning |
|---|---|---|
| Omitted | Default plus platform additions | `matched_default`; no version warning |
| Valid, inside exactly one range | Default plus platform and selected version additions | `matched_version`; record the selected range |
| Valid, in no range | Default plus platform additions | `version_out_of_range` warning |
| Unparseable under the entry's scheme | None for this pair | `version_unparseable` warning |

An out-of-range version never selects the nearest, highest, or broadest variant.
An unparseable version contributes nothing, while other requested pairs still
resolve. After version selection, a named intent must exist in the effective
policy. Otherwise emit `intent_unsupported` and contribute nothing for that
pair, not the base or an all-intents fallback. An out-of-range version requesting
a newer-only intent emits both warnings and contributes nothing. With no
intent, combine the effective base with all of its effective intents.

No catalog match yields `tool_unmatched`, no contribution, and no wildcard
fallback. Diagnostics preserve input order. These per-pair outcomes are not
whole-request failures.

### 4.4 Platform variants

Supported platforms are `windows`, `linux`, and `macos`. Supported architecture
selectors are `x64` and `arm64`. A variant selector has this closed shape:

```ts
interface PlatformVariantSelector {
  platform: "windows" | "linux" | "macos";
  architecture?: "x64" | "arm64";
}
```

A platform variant contains only additions to the common default, using the
same `policyAdditions`, `dependencies`, `intentAdditions`, and `newIntents`
fields as a version overlay. It never removes, narrows, or replaces default
requirements. Select at most one platform/architecture overlay, then combine
its additions with the default and the selected version overlay.
Architecture is a catalog selector, not a field added to `ContainerRequest`. Omitting
`when.architecture` makes a catalog variant architecture-neutral; omitting the
caller's `ResolveContext.architecture` instead requests the host default.

Selection first filters by platform. After identity and intent specificity
([§4.3](#43-identity)), architecture selection uses the following precedence:

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
architecture-neutral variant exists, retain the common default without platform
additions; never select another architecture's overlay. A failure to determine
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

Dependencies reference another entry's `entryId` and can belong to the base or
to an intent, in the default or a selected overlay. Include base dependencies
and only the selected intent dependencies. A dependency contributes only its
unversioned default base plus its applicable platform overlay's base
additions; it never selects a version overlay or includes intents. This
differs from a requested tool with no intent, which includes all effective
intents. A reference may name dependency intents, for example
`{ "entryId": "tool:ssh", "intents": ["connect"] }`; those intents, including
their applicable platform `intentAdditions`, are then added. Catalog validation
rejects a named dependency intent that is missing from any materialized
platform combination where the reference applies. An optional dependency
`versionRange` uses the same VERS syntax and authoring/build validation;
a range is not a detected version and does not select a version overlay.
Resolution is otherwise transitive, cycle-rejecting, and deterministic, and
the diagnostics API returns the resolved dependency metadata alongside the
policy.

Compose each selected base with its selected intent additions, then combine
the results for different requested tools and their transitive dependencies.
De-duplicate each source contribution layer independently within an entry and
catalog revision. The default base contributes once, and each selected platform
base layer contributes once, regardless of requested version or intent.
Default intent additions are keyed by intent; platform intent additions by
platform selector and intent; version base additions by range; and version
intent additions by range and intent. Include new-intent definitions under
their owning overlay. Never attach the requesting pair's version or intent to
a shared base-layer key. Repeated identical layers contribute once; distinct
version and intent additions selected by different pairs are retained.
Preserve per-input attribution. A shared `ResolveContext` applies to the whole
lookup; each tool candidate carries its own intent.

Following #779's floor semantics, filesystem composition preserves the access
required by all selected tools rather than intersecting their requirements.
An overlapping read-only requirement must not suppress another tool's needed
write access, and a catalog-provided deny must not block a required read or
write path. This is not a rule for merging consumer authorization policy.
Filesystem composition uses the exact
`filesystem.deniedPaths`, `filesystem.readonlyPaths`, and
`filesystem.readwritePaths` fields:

1. All selected entries and additions use the containing catalog revision's
   validated SDK target; no stored request contains a `version`.
2. At lookup time, resolve all required symbols and normalize paths using the
   selected platform's path rules before comparing equal or
   ancestor/descendant paths. Combine the selected entries and dependencies,
   de-duplicating equivalent pathnames within each access class, not distinct
   required alias locations.
3. Preserve every required read-write subtree. Remove read-only entries equal
   to or contained within a read-write subtree, since read-write already
   satisfies their read requirement. Retain a read-only ancestor of a
   read-write subtree without promoting the whole ancestor to read-write.
4. Remove each catalog-provided deny that overlaps any required read-only or
   read-write path, whether equal, an ancestor, or a descendant. Remove the
   entire deny entry, not an invented exception beneath it. Retain
   non-overlapping denies.
5. Return the composed policy and make the access changes available through
   diagnostics. The same rules apply to overlaps within one selected entry
   and across multiple entries; matching or traversal order must not change
   the effective access.

Filesystem comparison is separate from invocation-name matching. Equality,
de-duplication, and ancestor checks honor the applicable filesystem and
directory case-sensitivity, not a blanket OS assumption. When that information
cannot be determined, compare case-sensitively, preserve differently cased
paths for comparison, and report the assumption in diagnostics. Returned paths
retain their casing; comparison must not lowercase the policy paths. Another
target environment must not inherit this host's filesystem case rules.

Resolve actual filesystem object identity using MXC's
[object comparison primitives](https://github.com/microsoft/mxc/blob/894f4c159705f5f470727e4fa1e363a2abec88f1/src/mxc-sdk/src/core/mxc_common/filesystem_object.rs#L100-L265).
Symlink, junction, hard-link, bind-mount, and 8.3 aliases must participate in
the same catalog access composition, not silently cause a required read-write
path to become read-only or denied when MXC later normalizes the request.
Keep this floor composition separate from the runner's restrictive enforcement.

Object identity reconciles access, not pathname reachability. Retain every
required alias location even when multiple paths name the same object. If a
read-only alias names an object required read-write through another path,
retain that alias with the composed read-write access instead of deleting it.
Do not collapse same-class aliases merely because their object identities
match: path-based backends still need each required mount or pathname.

If necessary identity cannot be established for the target, that pair
contributes nothing and reports `filesystem_identity_unresolved`; other
independently resolved pairs can still contribute. This includes unresolved
aliases introduced by dependencies or cross-pair composition. Do not treat
unknown identity as proof that paths differ, bypass this check with lexical
case rules, or weaken MXC enforcement.

V1 inspects local host-side source paths, not a remote or guest filesystem.
Windows identity uses volume serial number and file ID through
`CreateFileW`/`FileIdInfo`; Unix uses `stat` device/inode (following links).
Compare opened/resolved objects, preserve every required alias pathname, and
compare existing ancestor identities for subtree overlap. A cleanly missing
suffix is compared relative to its deepest resolvable existing ancestor; it
is not treated as an existing alias. An unreadable component, broken link
whose target cannot be established, or different target filesystem is unknown
and drops every input whose access relationship depends on that fact. Never
drop only the restrictive side of an unresolved relation to retain its grant.
Case-sensitive comparison is only a lexical aid when case sensitivity is
unknown, not permission to skip object checks. Apply the same analysis across
pair/dependency boundaries. Discard all layers solely owned by excluded pairs,
then compose the remaining complete pairs; retain shared layers only for
remaining owners. No lookup result bypasses the runner's authoritative
enforcement-time checks or claims protection from later filesystem changes.

| Resolved requirements | Composed filesystem policy |
|---|---|
| Read-only `/work` and read-write `/work` | Read-write `/work`; omit read-only `/work` |
| Read-write `/work` and read-only `/work/tools` | Read-write `/work`; omit read-only `/work/tools` |
| Read-only `/work` and read-write `/work/cache` | Retain read-only `/work` and read-write `/work/cache`; do not make all of `/work` writable |
| Denied `/data` and read-write `/data/cache` | Remove denied `/data`; retain read-write `/data/cache` and report that the entire `/data` deny was removed |
| Denied `/secrets` and read-write `/work` | Retain both non-overlapping entries |

These are composition rules for the returned `ContainerRequirements`, not changes
to MXC's enforcement precedence. Simply concatenating a conflicting deny or
read-only entry with a grant is insufficient: the restrictive entry could
still prevent the access the composed floor is intended to request.

Removing a parent deny removes its protection for the entire subtree, not
just the overlapping required path. It does not itself add a grant to that
subtree, but other grants can now apply there. Diagnostics must identify the
removed deny and this broader effect. Caller-owned denies and other user,
enterprise, device, or backend restrictions are never inputs to this
least-restrictive catalog composition and must not be removed by it.

Publication checks validate policy shapes, symbols, and supported composition
fields and exercise the rules with known paths and fixtures. Equal or nested
filesystem requirements are not by themselves invalid catalog data. Caller
symbol values can introduce additional overlaps, so the resolver must always
apply these rules after substitution and normalization at lookup time.
An overlap covered by these rules is not a composition error; missing required
symbols or unsupported composed fields retain their existing failure behavior.

Network requirements also combine across selected tools, intents, and
dependencies. A tool that needs no network contributes no network access; it
does not veto access required by another requested tool. For example, Git's
`local` intent alone needs no network, while a request combining it with a
tool needing HTTPS access includes that tool's HTTPS requirement.

When only one selected component requires network, preserve its supported
network requirements; components with no grants do not force an additional
network merge. For multiple scoped outbound requirements in v1, retain
`network.egress.default: "deny"` and union the selected
`network.egress.allow` and catalog `network.egress.deny` rules,
de-duplicating identical rules.
Preserve each whole rule's destination, exclusions, protocol, and port
relationships; never form a cross-product of destinations and ports. Missing
network fields or deny-by-default contribute no grants, not a restriction on
another component's required grants. Do not introduce unrestricted outbound
access or include unselected version or intent additions. An omitted intent
selects all intents in the effective version policy; an unsupported intent
contributes no access.

Catalog egress denies follow the filesystem deny rule. Remove each catalog
deny rule that overlaps any required allow rule, meaning their destination
CIDRs intersect after `except` exclusions and their protocol/port selectors
intersect. Remove the entire deny rule, not an invented exception within it.
Retain non-overlapping deny rules. The same rule applies within one entry and
across entries, and a conflicting deny never fails the request.
Diagnostics identify the removed rule, its full destination and port scope,
and the contributing entries.

This combined access belongs to the shared sandbox, not to isolated
permissions per tool. Caller-owned network denies and other user, enterprise,
device, or backend restrictions are never inputs to this composition and are
never removed by it. Other network configuration, including allow-by-default,
non-default ingress, and proxy configuration, is rejected rather than
approximated with broader access when more than one selected policy sets it.
The same rejection
applies to timeout, clipboard, lifecycle, UI, and every other field without an
explicit composition rule. A single selected policy without additions or
dependencies may use catalog-supported fields without cross-policy composition.

## 5. API surface

The MXC SDK APIs separate runtime resolution from catalog inspection.
Resolution accepts one tool or an array, selects the most specific match for
each input, and composes its effective base, selected intents, and dependencies into one
`ContainerRequirements`. Callers choose a requirements-only operation or a diagnostic
operation over the same resolution logic. Neither
implicitly returns the whole catalog. These are SDK library calls, not a
hosted service or a command-line utility.

The signatures below use TypeScript to describe the shared contract. Rust and
C# expose the same operations and metadata with idiomatic names and types.
TypeScript and C# expose single-tool and array overloads; Rust uses an idiomatic
one-or-many input type because it does not support function overloading. An
absent policy is `undefined` in TypeScript/JavaScript, `None` in Rust, and
`null` in C#. Library failures remain distinct from policy absence.

### 5.1 Runtime lookup

`ContainerRequirements` preserves the earlier four-field scope: filesystem,
network, UI, and timeout, with the composition rules in §4.5. It reuses the
SDK's nested types and optionality; `Pick` is not runtime validation or an
expansion of the catalog's supported fields (§4.2). Rust/.NET use an equivalent
four-field aggregate, not duplicate nested models.

Context is optional and lookup-only. Command, containment, name, working
directory, environment, cleanup, proxy, and operation options remain caller-owned,
not resolver inputs or outputs. Intent selects requirements, not a command.
Resolve again or supply overrides if the eventual execution environment differs
from discovery. Requirements do not certify coverage of an arbitrary command.

Node resolution uses Promise-returning plain verbs, matching the v1
[run/spawn convention](https://github.com/microsoft/mxc/blob/894f4c159705f5f470727e4fa1e363a2abec88f1/sdk/node/src/v1/container.ts#L428-L477);
filesystem work must not block the event loop. Rust exposes
`v1::resolve_tool_requirements` / `resolve_tool_requirements_with_diagnostics`
as `Result<Option<ContainerRequirements>, Error>` / `Result<ToolRequirementsResolution, Error>`.
.NET exposes `MxcContainer.ResolveToolRequirements` and
`ResolveToolRequirementsWithDiagnostics` (plus `Async` Task forms) in `V1`.
Metadata inspection stays synchronous; no operation creates a container.

Failures reuse existing MXC error codes, which are sufficient for normal
programmatic handling. An optional `details.reason` may provide a stable,
catalog-specific distinction for logging, investigation, or finer handling
when the code alone is too broad. Callers need not branch on it; an absent or
unrecognized reason retains the same handling as the primary code. A reason
must not duplicate a distinction already expressed by an existing MXC code.

The following primary-code mappings apply across all language bindings.
When `details.reason` is supplied for these failures, it uses the listed value;
callers may ignore it.

| Failure | MXC error code | Optional `details.reason` |
|---|---|---|
| Invalid tool input or resolution context | `malformed_request` | `invalid_context` |
| Invalid catalog data, including invalid dependency references or cycles | `policy_validation` | `invalid_catalog` |
| Distinct matches for one tool tied at the highest identity/intent/architecture rank | `policy_validation` | `ambiguous_match` |
| Unsupported composition, including incompatible SDK target metadata or fields without a composition rule | `policy_validation` | `composition_conflict` |
| Unsupported or undetectable host platform or architecture | `unsupported_containment` | `unsupported_host` |
| Bundled catalog content cannot be read | `backend_error` | `integrity` |
| Explicitly requested catalog revision is not installed | `backend_error` | `revision_unavailable` |

Filesystem and network overlaps handled by
[§4.5](#45-dependencies-and-composition) are not composition failures. Ordinary no-match results remain policy absence, not an
error from this table. Invalid candidate PURLs, well-typed but unparseable
version strings, unsupported intents, and unresolved filesystem identity are
per-pair diagnostic outcomes, not whole-request errors from this table.

```ts
import type { ContainerRequest, NetworkRuleConfig } from "@microsoft/mxc-sdk/v1";

export type ContainerRequirements = Pick<ContainerRequest,
  "filesystem" | "network" | "ui" | "timeoutMs">;

interface ToolCandidate {
  invocationName: string;
  packageUrl?: string;
  detectedVersion?: string;
  intent?: string;
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

interface IntentSelection {
  requested?: string;
  mode: "named" | "all" | "none" | "unsupported";
  selected: string[];
}

type VersionStatus =
  | "matched_default"
  | "matched_version"
  | "version_out_of_range"
  | "version_unparseable";

interface VersionSelection {
  status: VersionStatus;
  detectedVersion?: string;
  selectedVersionRange?: string;
}

type ToolResolutionStatus =
  | VersionStatus
  | "intent_unsupported"
  | "tool_unmatched"
  | "filesystem_identity_unresolved";

type InputWarning = { inputIndex: number; message: string };
type ToolResolutionWarning = InputWarning & (
  | { code: "version_out_of_range" | "version_unparseable";
      entryId: string; detectedVersion: string }
  | { code: "intent_unsupported"; entryId: string; intent: string }
  | { code: "tool_unmatched"; invocationName: string }
  | { code: "purl_invalid"; packageUrl: string }
  | { code: "purl_components_ignored"; packageUrl: string;
      ignoredComponents: Array<"version" | "qualifiers" | "subpath"> }
  | { code: "weak_identity"; entryId: string; invocationName: string }
);

interface WarningScope {
  inputIndexes: number[];
  entryIds: string[];
  message: string;
}

interface PathRequirement {
  path: string;
  access: "denied" | "readonly" | "readwrite";
  entryIds: string[];
}

type EgressRule =
  NetworkRuleConfig;

interface NetworkRequirement {
  rule: EgressRule;
  entryIds: string[];
}

type ResolutionDetailWarning = WarningScope & (
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

type PolicyResolutionWarning = ToolResolutionWarning | ResolutionDetailWarning;

interface ToolRequirementsResolution {
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

export declare function resolveToolRequirements(
  tool: ToolInput,
  ctx?: ResolveContext
): Promise<ContainerRequirements | undefined>;

export declare function resolveToolRequirements(
  tools: readonly ToolInput[],
  ctx?: ResolveContext
): Promise<ContainerRequirements | undefined>;

export declare function resolveToolRequirementsWithDiagnostics(
  tool: ToolInput,
  ctx?: ResolveContext
): Promise<ToolRequirementsResolution>;

export declare function resolveToolRequirementsWithDiagnostics(
  tools: readonly ToolInput[],
  ctx?: ResolveContext
): Promise<ToolRequirementsResolution>;
```

A string input is shorthand for `{ invocationName: tool }`; it supplies no
intent, package, or version evidence and follows the same weak-identity option
as an object input. For example, name-only lookup under the opt-in
rule is:

```ts
const ctx: ResolveContext = {
  platform: "windows",
  architecture: "x64",
  allowWeakIdentityFallback: true,
  projectRoot: String.raw`D:\work\repo`,
  symbols: {
    git_prefix: String.raw`D:\tools\git`,
    ssh_prefix: String.raw`D:\tools\ssh`,
    programData: String.raw`C:\ProgramData`,
    temp_dir: String.raw`D:\temp`,
  },
};
const push = await resolveToolRequirementsWithDiagnostics(
  { invocationName: "git", detectedVersion: "2.45", intent: "push" }, ctx);
const bundle = await resolveToolRequirementsWithDiagnostics(
  { invocationName: "git", detectedVersion: "2.55", intent: "bundle-fetch" }, ctx);
const noVersionBundle = await resolveToolRequirementsWithDiagnostics(
  { invocationName: "git", intent: "bundle-fetch" }, ctx);
const combinedRequirements = await resolveToolRequirements(
  [{ invocationName: "git", detectedVersion: "2.45", intent: "push" }, "node"], ctx);
for (const warning of push.diagnostics.warnings) {
  if (warning.code === "filesystem_deny_removed") {
    console.log(warning.removed.path, warning.requiredBy, warning.entryIds);
  }
}
```

Later, reviewed and constrained requirements combine with a command without
converting their nested types:

```ts
function createRequest(
  approvedRequirements: ContainerRequirements,
  command: string,
): ContainerRequest {
  return { ...approvedRequirements, command };
}
```

For the Git entry above, with referenced dependencies available:

| Request | Selected version data | Result |
|---|---|---|
| `2.45` + `push` | Default + Windows additions + `vers:intdot/>=2.40\|<2.50` | `matched_version`; push policy plus the SSH dependency's default and Windows base additions |
| `2.55` + `bundle-fetch` | Default + Windows additions + `vers:intdot/>=2.50\|<3` | `matched_version`; new bundle-fetch policy, no inherited SSH addition |
| No version + `bundle-fetch` | Default + Windows additions | `intent_unsupported`; `requirements` is `undefined` for this single-pair call |

Single-tool lookup is equivalent to a one-element array; its diagnostic
`inputIndex` is `0`. A caller retaining separate policies per tool can use
single-tool calls. A caller wanting one sandbox for several tools passes an
array. Both forms select one most-specific match per input, then compose the
contributing requirements. A string input uses the unversioned default and all
its effective intents, including applicable platform additions.

Both operations yield the same requirements; the diagnostics form adds
attribution and warnings from that resolution pass. No second lookup or
process-global "last result" state is needed.

**The returned requirements may cover only a subset of the requested tools.**
Partial results are intentional in both APIs. `tool_unmatched`,
`version_unparseable`, `intent_unsupported`, and
`filesystem_identity_unresolved` pairs contribute nothing; other pairs still
resolve. The requirements-only call makes no coverage promise, even when its result
is non-`undefined`. Callers needing to know which pairs contributed use
`resolveToolRequirementsWithDiagnostics` and inspect per-input statuses.
No wildcard entry fills a missing match, and there is no `requireAllMatches`
option.

Each input has a diagnostic record in input order. A `tool_unmatched` input
has an empty `matches` list and a warning. An empty array or a lookup with no
contributing pairs produces no requirements, not an empty requirements object:
`resolveToolRequirements` yields `undefined`, while the diagnostics operation yields
a `ToolRequirementsResolution` with `requirements: undefined`. An empty array has no
per-input records. Unresolved required symbols in selected entries prevent a
policy from being returned and produce diagnostics; they are not grounds for
silently omitting a selected requirement to produce a partial policy.

Each input's `matches` contains at most one identity-matched entry, including
when its version or intent prevents contribution. Equally specific matches
remain an ambiguity error. `versionSelection` reports the supplied version,
version status, and selected range only for `matched_version`.

`intentSelection` reports `mode: "named"` or `"all"` and sorted selected names.
An unsupported name uses `"unsupported"` and an empty list. Version parse
failure skips intent resolution. With no intent and no effective intents,
`"all"` has an empty list and the effective base still contributes.

A contributing pair's status is its version status. Unresolved target object
identity makes its status `filesystem_identity_unresolved` while preserving
any completed version and intent selection. Unsupported intent makes
the pair's status `intent_unsupported` while retaining the version status in
`versionSelection`. Structured warnings preserve input order; for an
out-of-range version and unsupported intent, emit `version_out_of_range` then
`intent_unsupported`. All warnings are structured records; `code` selects the
category's fields and `message` is human-readable, not a parsing contract.
These statuses never silently select another variant or entry.

Per-input warning fields retain the prototype's `inputIndex`, `entryId`,
`detectedVersion`, `intent`, and `message` vocabulary where applicable.
Warnings about shared contributions use sorted, distinct `inputIndexes` and
`entryIds`; each path/rule contribution also identifies its source entries.
`removed` always contains the complete original deny path or egress rule,
including exclusions and protocol/port selectors, not merely the intersection.
Removing it can affect its entire scope wherever other grants apply.

Dependency metadata includes version and intent selections. A dependency
always reports `matched_default`, and `mode: "none"` with an empty list unless
its reference names intents, which report `"named"`. De-duplicate identical
entry/revision/range/selection records and sort by those fields. Shared
contributions retain sorted, distinct `inputIndexes` for every contributing
requester, including transitive dependencies. Union these indexes when
de-duplicating identical records; requester indexes are not part of the
dependency-record identity.

When architecture is omitted, diagnostics include a warning naming the
effective native system architecture and stating that the tool's architecture
was not verified. Architecture-neutral fallback is also identified. These
diagnostics describe selection; they do not attest to the installed tool's
architecture. The requirements-only operation does not expose warnings or
attribution; consumers needing them use `resolveToolRequirementsWithDiagnostics`.

Composition diagnostics report read-only requirements superseded by
read-write requirements, and catalog filesystem and egress denies removed to
satisfy required access. Each warning identifies the resolved paths or rules,
access classes, and contributing entry IDs. A removed deny warning names the
full removed scope and explains that other grants may now apply throughout
it, not only at the overlap. Both APIs return the same composed policy;
callers needing to review these adjustments use
`resolveToolRequirementsWithDiagnostics`.

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
  newIntents: CatalogIntentMetadata[];
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

export declare function listCatalogEntries(): CatalogEntryMetadata[];
export declare function getCatalogInfo(): {
  catalogSchemaVersion: string;
  catalogRevision: string;
  sdkContractVersion: string;
};
```

This supports setup UI, catalog browsing, and update decisions without paying
the cost of policy resolution, and keeps "give me everything" out of the
runtime lookup path entirely. Metadata exposes the default and overlay
selectors, inherited/new intent names, subcommand hints, dependency IDs, and
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

The catalog APIs never write a consumer's policy store. A consumer's own
capability observation (see [§8](#8-relationship-to-learning-mode)) can produce
candidate evidence for a future contribution to this catalog; it is not a
mechanism for mutating the catalog at request time.

## 6. Intended repository and packaging boundary

The Policy Store lives in `microsoft/mxc` and ships through the existing MXC
SDK packages, not a separate repository, package, or command-line utility.
Catalog data and resolver APIs follow the MXC contribution and release process.

One internal `policy_store` module in `mxc-sdk` uses the SDK's v1 types and
common identity helpers. Rust calls it directly; Node/.NET use panic-contained
`mxc_ffi` library exports, not launch/probe APIs. V1 data is embedded at build
time, with no dynamic fetching; that is a possible V2 capability.

### 6.1 Library distribution and consumption

The MXC SDKs expose requirements using their v1 request field types:

| Language | Existing MXC SDK | Requirements type |
|---|---|---|
| TypeScript / JavaScript | `@microsoft/mxc-sdk/v1` | `ContainerRequirements` (`Pick<ContainerRequest, ...>`) |
| Rust | `mxc-sdk` | `mxc_sdk::v1::ContainerRequirements`, using v1 section types |
| C# / .NET | `Microsoft.Mxc.Sdk.V1` | `ContainerRequirements`, using v1 section types |

Returned requirements use the containing SDK's supported fields. Unsupported
data must not be silently dropped to fit its types.

A consumer:

1. Installs an MXC SDK release with its bundled catalog.
2. Inspects the catalog or resolves requirements using §5's APIs.
3. Reviews access and coverage, preserving its restrictive baseline on absence
   and applying [§5.3](#53-consumer-obligations).
4. Supplies command/execution settings later, applies its restrictions, and
   passes the resulting `ContainerRequest` to MXC.

The SDK API reference and repository/package READMEs must state:

> Returns a best-effort MXC policy baseline for representative tool workflows,
> not authorization or a guarantee of success or safety. It may request broader
> access. Callers and users review that access and may further constrain or
> override the recommendation; enterprise and device restrictions remain
> authoritative. Diagnostics explain the contributing requirements and changes.
> A returned policy may cover only a subset of requested tools; inspect
> per-input statuses to determine coverage.

Lookup is local and does not download updates, contact a hosted service, or
run the candidate tool. `ResolveContext.catalogRevision` selects a revision
included in the installed SDK; an unavailable revision is an error, not a
request to download or substitute data. An omitted revision uses the bundled
default.

V1 catalog updates ship with an MXC release. Revision metadata can identify the
bundled data independently of the SDK package version without implying a
separate artifact delivery channel. Installing an SDK update does not rewrite
a consumer's previously accepted policies.

### 6.2 Cross-language consistency and support

All three SDKs use the same catalog format and shared conformance
fixtures. Given the same catalog revision, tool inputs, explicit resolution
context, and relevant host/filesystem observations, they must agree on matching,
variant selection, dependency metadata, effective policy, diagnostic meaning,
and failure categories. This is semantic consistency, not byte-identical
output or reproduction of another language's runtime behavior.

Each binding uses consistent, idiomatic result and error handling for its
language. Equivalent failures map to the corresponding existing MXC error
codes, while error types, message wording, and language-specific representations
may differ. Callers need not understand another binding or parse message text.
Shared fixtures compare policy semantics and required diagnostic information,
including prescribed ordering, rather than identical warning prose or
incidental serialization.

Shared fixtures cover platform path semantics as well as ordinary lookup;
matching function names alone is not compatibility. Package CI must also
exercise installation, public API usage, and host-derived defaults on the
supported platforms. Binding tests exercise the shared native resolver rather
than independent implementations of the resolution rules.

Policy Store is an ongoing SDK capability. Language parity,
documentation, and maintenance belong to the MXC SDK release process.

## 7. Contribution and review

- Catalog and API contributions are pull requests in `microsoft/mxc`.
  No client or SDK can write a catalog entry at runtime.
- Every entry change includes identity evidence, supported tool version
  range(s), platform evidence, a minimized requirement set, test fixtures,
  and provenance.
- Entry and dependency `versionRange` values pass VERS syntax and supported-type
  validation during authoring/build validation. An entry's version ranges must
  use its declared scheme and must not overlap.
- Keep one unversioned default common to all versions and platforms. Define an
  intent only when its additions materially differ in network access,
  credentials, or writes outside the
  workspace. Do not create one intent per subcommand. An empty `local` intent
  identifies base-only use separately from omitted-intent aggregation.
- Intent dependencies and access additions must be justified by that intent.
  Example subcommands are caller guidance, not executable matching rules.
- CI must validate the agreed policy contract and version mapping,
  entry-ID uniqueness, dependency closure and cycle-freedom, symbol validity,
  absence of unsafe user-specific literal paths, unsupported-field rejection,
  deterministic resolution, and package inclusion.
- Build validation rejects missing/multiple defaults, version-only entries,
  variants tagged as default, and non-additive platform/version operations.
  Materialize and schema-validate every platform x architecture x version
  variant x intent combination, including omitted-intent aggregation.
- Verify default base requirements are a subset of each effective base, and
  each inherited default intent's policy/dependencies are a subset of the same
  effective intent. Overlays may add names through `newIntents`, never delete,
  rename, or redefine inherited intents. Validate dependency closures
  introduced by additions too.
- Generate rendered effective-policy views and diffs from the default for
  catalog reviewers; do not require reviewers to mentally expand overlays.
- A new entry or a requirement expansion requires one catalog-owner approval
  and one security/policy-reviewer approval, plus tool- or scenario-owner
  evidence where available.
- A requirement reduction needs regression evidence for the representative
  scenarios used to justify the entry, not a universal workflow guarantee.
- Catalog data and SDK changes follow MXC's contribution and release review.
  These requirements do not require applications to seek maintainer approval
  to use the SDK APIs.

## 8. Relationship to Learning Mode

Learning Mode complements Policy Store; it does not replace it. Policy Store
is a long-lived source of best-effort baseline requirements.
Learning Mode is developer tooling for discovering requirements and
productizing them as reviewed policy data.

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
that consumer's design. The SDK provides no runtime submission hook or
automatic mutation of the release-bundled catalog.

## 9. Trust model

This document tightens #779's trust framing rather than replacing it. Entries
assert *need*, not authorization, but incorrect data has two different
outcomes:

- An understated floor omits a requirement and can cause the tool or its
  end-to-end workflow to fail under the resulting policy.
- An overstated floor can fail against a narrower consumer ceiling. If a
  consumer instead approves or adopts it and its ceiling permits the request,
  the effective policy contains unnecessary capability.

Even correct entries can yield a broader combined request than any one tool
needs. Read-write requirements supersede overlapping catalog read-only
requirements, and a conflicting catalog filesystem or egress deny is removed
in full under
[§4.5](#45-dependencies-and-composition). These changes are intentional and
reported by the diagnostics API; they are not permission to remove a
consumer's own restrictions.

What changes from #779 is the review bar. #779 described community-contributed,
unsigned, unwarranted data. This contract requires named-role approval
([§7](#7-contribution-and-review)) before an entry publishes, and publishes
under an immutable revision through MXC's package distribution
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

V1 embeds entries and shared symbol definitions in the native library at build
time and inherits MXC package signing and distribution integrity. It has no
separate catalog digest or runtime checksum. Schema validation still applies.

## 11. Backward compatibility

- No change to the v1 `ContainerRequest` or exact `ContainerConfig` schema.
- No change to executor behavior.
- New opt-in APIs in existing MXC SDKs; callers that do not use policy lookup
  see no behavior change from this feature.
- No breaking policy-schema changes within MXC 1.x; breaking changes require
  2.x. Bundled entries follow the containing SDK's compatible policy contract.

## 12. Test plan

**SDK resolver APIs (TypeScript/JavaScript, Rust, and C#/.NET)**

- shared conformance fixtures produce equivalent results and failure
  categories in all three languages without requiring identical message text
  or language-specific representations; each binding's error handling is
  consistent and uses the corresponding MXC error codes
- failure cases use the primary-code mappings in [§5.1](#51-runtime-lookup);
  any supplied reason uses its listed value, and callers can handle the code alone
- one-tool and one-element-array overloads produce equivalent requirements and
  diagnostics; the simple API yields the same requirements as the diagnostic API
- lookup is command-free and returns only §4.2's allowed fields; reject old
  UI names and caller execution fields rather than silently accepting them
- Node resolution does not block the event loop; Rust/.NET preserve their
  corresponding idiomatic result/error types
- multiple input tools select one match each and compose their bases, selected
  intent additions, and dependencies; repeated inputs and shared components
  do not duplicate contributions
- two versions or intents of one entry share the default/platform base layers
  once, preserving distinct additions and per-input attribution; identical
  base fields do not create false unsupported-composition conflicts
- a known intent selects only its effective base and additions; omitted intent
  combines all effective intents, while unsupported intent contributes nothing
  with `intent_unsupported`
- omitted version selects default with `matched_default` and no version warning;
  a version in one range adds only that overlay with `matched_version`
- a valid version outside all ranges uses default with `version_out_of_range`;
  never select the nearest, highest, or broadest variant
- an unparseable version contributes nothing with `version_unparseable`;
  an out-of-range version plus a newer-only intent emits both warnings
- unparseable versions, unsupported intents, and unmatched tools skip that
  pair's dependencies and symbols while other pairs still resolve
- every input has an ordered status record; `tool_unmatched` never falls back
  to a wildcard entry; mixed requests return only contributing policies
- requirements-only and diagnostic APIs retain the same partial-result behavior;
  only the diagnostic API reports coverage, without a `requireAllMatches` option
- different tools in one request carry independent intents; two intents of
  the same tool share the base without losing either set of additions
- unselected intent dependencies and symbols are not resolved; selected
  intent dependencies participate in cycle detection and attribution
- shared and transitive dependency records retain the union of requesting
  `inputIndexes`, including successful resolutions that emit no warnings
- a dependency contributes only its default and applicable platform base
  additions, never a version overlay or intents, unless its reference names
  dependency intents; named intents are added with their platform additions
- known and unknown inputs compose the known requirements and report each
  unmatched input; empty and all-unmatched arrays return no policy, never an
  empty policy, while the diagnostic API preserves the resolution metadata
- omitted optional context fields use host platform and native system architecture, the
  installed catalog revision, no caller symbol overrides, and no weak-identity
  fallback
- an unresolved required symbol prevents policy output, with diagnostics,
  rather than silently omitting selected requirements
- caller symbol overrides precede discovery, which precedes documented
  defaults; only required symbols are resolved, with source/value diagnostics
- failed configuration reads are errors, not default selection; another
  target environment does not inherit this host's discovered paths
- pinned catalog revisions retain their shared symbol definitions and defaults;
  unsupported default templates and executable discovery data are rejected
- package identity precedes invocation name, then declared intent specificity,
  then exact architecture over platform-only or common default;
  distinct matches tied at the highest rank produce `ambiguous_match`
- intent declaration may identify an entry before version selection, but a
  version lacking that intent still contributes nothing, not another entry
- multiple predicates matching the same entry contribute that policy once;
  file order does not change matching or composition
- string shorthand and object inputs obey the same weak-identity fallback
  option; intent selection does not bypass it
- invocation names compare case-insensitively on Windows/macOS and exactly on
  Linux, without changing command spelling or package-identity matching
- PURL type and namespace compare case-insensitively, names follow their
  type's rules, and equivalent percent-encodings compare after parsing
- candidate PURL version/qualifiers/subpath are ignored with structured
  warnings; malformed components leave only that pair unmatched, with no
  invocation-name retry even when weak matching is enabled
- filesystem equality, de-duplication, and ancestor checks follow the actual
  case rules, including case-sensitive macOS volumes and Windows directories;
  unknown sensitivity preserves differently cased paths with a diagnostic
- exact-architecture additions precede platform-only additions; duplicate
  selectors are rejected; no matching overlay retains the common default
- on an ARM64 host with both architecture-specific variants and no neutral
  variant, omitted architecture selects ARM64; explicit x64 selects x64
- a library process running as x64 under emulation on an ARM64 host still
  defaults to the native ARM64 system architecture, not its process
  architecture
- a missing exact overlay falls back to the platform's neutral additions,
  otherwise to the common default; never use another architecture's overlay
- successful host-derived selection and neutral fallback produce the
  diagnostics specified in [§5.1](#51-runtime-lookup); host-architecture
  detection failure produces a library error, not a guessed match
- dependency chain resolution, including cycles (terminate, no duplication)
- filesystem floor composition ([§4.5](#45-dependencies-and-composition)):
  same-class de-duplication; equal read-only/read-write paths become read-write;
  read-write ancestors subsume read-only descendants, while read-only
  ancestors remain read-only outside required writable subtrees
- literal paths and distinct symbols that resolve to equal or nested paths
  follow the same lookup-time composition rules; catalog publication checks
  do not substitute for this runtime pass
- symlink, junction, hard-link, bind-mount, and 8.3 aliases use established
  target object identity in floor composition; the returned policy must not
  silently lose required writes to a more restrictive catalog alias
- same-object aliases preserve every required pathname; a read-write `/data`
  and read-only bind-mount alias `/alias` remain accessible through both names
  with the composed access, and same-class aliases are not collapsed
- unestablished necessary target identity fails closed for affected pairs,
  including dependency aliases, with structured diagnostics; unrelated
  resolved pairs still contribute and MXC enforcement is not weakened
- warnings for architecture, symbols, composition, removed denies, versions,
  intents, identity, and unmatched pairs have discriminated codes and required
  data fields; consumers read paths/rules and source entries without parsing
  messages
- catalog denies equal to, above, or below required read/write paths are
  removed; non-overlapping denies remain; warnings identify source entries,
  paths, access changes, and the full scope of removed parent denies
- overlaps within one entry, across matched inputs, and through dependencies
  behave identically; both APIs return equivalent composed policies, and
  entry/input traversal order does not change effective access
- a no-network tool does not veto another selected tool's network requirement;
  unselected version/platform/intent additions add no access; omitted intent
  selects all effective intents, while unsupported intent contributes nothing
- composed outbound rules retain destination/port pairings and exclusions;
  no network requirement means no grants, not unrestricted access
- a catalog egress deny rule overlapping another selected tool/intent's
  required allow rule is removed in full, within one entry or across entries,
  without failing the request; non-overlapping deny rules remain; warnings
  identify the rule, its full scope, and source entries
- incompatible SDK target metadata and network fields other than egress allow/deny rules,
  or other fields outside the supported composition rules, remain rejected
  rather than broadly approximated
- symbol resolution on Windows, Linux, and macOS

**Data (CI)**

- every default and materialized platform/architecture/version/intent
  combination passes the closed catalog checks, typed v1 builder, semantic
  validation, and exact schema validation; no backend/probe is needed
- validation captures the exact request before normalization and never feeds
  the normalized restrictive copy back into floor composition
- a newer SDK exact target revalidates all bundled catalog revisions using
  its own schema; unchanged older-major-line revisions retain their IDs when
  revalidation passes, while unvalidated or cross-major data is not bundled
- catalog PURLs pass complete syntax/type validation before identity indexing
- exactly one unversioned default is required; missing/multiple defaults,
  version-only entries, and a version variant tagged as default are rejected
- version ranges use the entry's scheme and do not overlap, including at
  inclusive boundaries; adjacent non-overlapping ranges are accepted
- platform, version, and intent additions share the SDK target and cannot
  remove, narrow, or replace inherited requirements; defaults remain subsets
  of effective variants and inherited intent names are never deleted or renamed
- rendered effective policies and default-relative diffs include base/intent
  access and dependencies for reviewer inspection
- malformed VERS syntax, invalid constraints, and unsupported version types
  fail catalog validation; valid examples cover npm, semver, pypi, nuget, and
  intdot, including their scheme-specific version syntax
- metadata distinguishes default, platform, and version additions and lists
  intent names, subcommand hints, and dependencies without resolving policy bodies
- `dependencies[].entryId` references resolve within the same catalog revision;
  named dependency intents exist in every applicable platform combination
- no literal absolute user-specific paths; no wildcard filesystem/network grants
- catalog/entry revision monotonicity across a change

**Integration**

- each MXC SDK resolves through the shared Rust implementation with build-time
  embedded data; lookup performs no dynamic fetching or sandbox execution
- compiled consumer examples in all three languages pass both the direct
  result's fields and the diagnostic result's requirements fields into
  `ContainerRequest` with a caller-supplied command, without nested-type
  conversion, casts, or serialization
- the bundled catalog revision matches `getCatalogInfo()`; selecting an
  unavailable revision fails explicitly, without falling back to another
  revision
- an MXC SDK update leaves previously accepted consumer policies unchanged
- a representative tool that fails under a minimal consumer policy succeeds
  once its resolved entry is composed in
- the composed MXC policy realizes the documented read-only/read-write
  nesting without unnecessarily promoting a read-only parent to read-write;
  retained backend precedence does not reintroduce removed catalog conflicts
- the same tool still fails when the consumer's policy forbids what the entry
  requests (the floor never widens the consumer's ceiling)
- a caller-owned filesystem or egress deny still prevents access even when an
  overlapping catalog-provided deny was removed during floor composition
- floor alias reconciliation and MXC's final object-based normalization agree
  on required access; a resolver check never replaces enforcement-time checks

## 13. Open questions

| Maintainer sign-off | Recommended answer |
|---|---|
| Approve the command-free requirements API surface? | `resolveToolRequirements` / `resolveToolRequirementsWithDiagnostics` return `ContainerRequirements` using existing v1 section types, with optional lookup context, Promise-based Node resolution, corresponding Rust/.NET bindings, and the documented partial-result contract. |

## 14. Related work

- [`microsoft/mxc#779`](https://github.com/microsoft/mxc/pull/779) - Sandbox
  Config Floors feature spec. This document's data model, floor/policy
  direction argument, and identity-layering analysis build directly on it.
- [`ChazGo/mxc#1`](https://github.com/ChazGo/mxc/pull/1) - draft SDK resolver
  and catalog prototype exercising lookup, dependency closure, and symbol
  resolution against an earlier version of this shape.
- [Node v1 types](https://github.com/microsoft/mxc/blob/894f4c159705f5f470727e4fa1e363a2abec88f1/docs/reference/node/v1/types.md) -
  `ContainerRequest` and its access sections. Rust and .NET use their
  corresponding v1 SDK types.
- [`docs/versioning.md`](versioning.md) - the versioning model
  [§4.1](#41-versions) builds on.
- [Package-URL VERS specification](https://github.com/package-url/vers-spec) -
  version-range syntax and supported version-type comparison references.
