# MXC Policy Store (prototype)

> **PROTOTYPE, pending API review.** The API names and shapes here are proposed
> and may change before sign-off. The policy store is not part of MXC 1.0; a
> later MXC SDK release is targeted.

The policy store resolves known tools (for example `git`, `node`, `npm`) to
command-free **`ContainerRequirements`** from a reviewed catalog bundled
statically in the MXC SDKs. The result is a best-effort floor, not a
guarantee, and it never grants access by itself: the caller reviews and
constrains it, adds its own command (object spread in Node,
`ContainerRequest::from_requirements` in Rust,
`ContainerRequest.FromRequirements` in C#), and passes the request to the
SDK's existing v1 container APIs. It is complementary to Learning Mode, not a
replacement. Lookup never creates a container, contacts a network service,
writes state, or downloads data.

- The spec is in [microsoft/mxc#1309](https://github.com/microsoft/mxc/pull/1309),
  head `5c2ba8e`: `docs/mxc-policy-store-api.md` is the caller-facing API
  contract (types, behavior, errors, examples), and
  `docs/mxc-policy-store.md` the catalog design (data model, packaging,
  source layout, contribution).
- [design.md](design.md) records the prototype's implementation decisions and
  its known gaps against that spec.
- [`catalog/views/`](../../src/mxc-sdk/policy_store/catalog/views/) holds the
  generated reviewer view and exact-request view for each revision.

## Where it lives

| Layer | Location | API |
|---|---|---|
| Resolver and V1 data | `src/mxc-sdk/src/policy_store/` (internal `mxc-sdk` module) and `src/mxc-sdk/policy_store/` (catalog, schemas, conformance) | Single implementation |
| Rust SDK | `mxc_sdk::v1` | `resolve_tool_requirements`, `resolve_tool_requirements_with_diagnostics`, `get_catalog_info`, `list_catalog_entries`; inputs and diagnostics in `mxc_sdk::v1::tool_requirements` |
| C ABI | `src/ffi/mxc_ffi/src/policy_store.rs` | `mxc_resolve_tool_requirements_json`, `mxc_resolve_tool_requirements_with_diagnostics_json`, `mxc_policy_catalog_info_json`, `mxc_list_policy_catalog_entries_json`, `mxc_policy_store_result_free` |
| Node SDK | `sdk/node/src/v1/tool-requirements.ts` (`@microsoft/mxc-sdk/v1`) | `resolveToolRequirements`, `resolveToolRequirementsWithDiagnostics` (both return a `Promise`), `getCatalogInfo`, `listCatalogEntries` |
| C# SDK | `sdk/dotnet/Microsoft.Mxc.Sdk/V1/MxcContainer.ToolRequirements.cs` | `MxcContainer.ResolveToolRequirements`, `ResolveToolRequirementsWithDiagnostics` (plus `Async` forms), `GetCatalogInfo`, `ListCatalogEntries` |

There is one resolver. The catalog is compiled into `mxc-sdk`, so the data
ships inside `mxc_ffi` and every SDK that loads it. The Rust SDK returns
`ContainerRequirements` built from the existing v1 section types. The Node and
C# SDKs send a JSON lookup request (`{"tools", "context"?}`) through
`mxc_ffi` and map the result onto their own v1 section types. The SDK owns
the wire version: the catalog authors access only (`default.requirements`,
never a `version` or `command`). Resolution stays outside `mxc_engine`: it never
selects a backend or launches a container.

## Example (Node)

```ts
import {
  ContainerRequest,
  resolveToolRequirementsWithDiagnostics,
} from '@microsoft/mxc-sdk/v1';

const { requirements, diagnostics } = await resolveToolRequirementsWithDiagnostics(
  { invocationName: 'git', packageUrl: 'pkg:generic/git', detectedVersion: '2.45.1', intent: 'push' },
  { projectRoot: '/work/repo', symbols: { git_prefix: '/usr', ssh_prefix: '/usr' } },
);
// diagnostics.tools[0].status === 'matched_version'
// diagnostics.resolvedDependencies[0].entryId === 'tool:ssh'
if (requirements) {
  const request: ContainerRequest = { ...requirements, command: 'git push' };
}
```

## Entry model

Each catalog entry has one unversioned **default** (`requirements` holding
container access sections, its dependencies, and named **intents** such as
`fetch` and `push`) and additive **overlays**: `platformVariants` for a
platform or architecture, and `versionVariants` keyed by a purl `vers` range
in the entry's required `versionScheme` (`npm`, `semver`, `pypi`, `nuget`, or
`intdot`). Overlays use `policyAdditions`, `intentAdditions` (extending
intents the default declares), and `newIntents`; they can never remove or
narrow anything. The effective policy is the default plus the platform
overlay plus at most one version overlay, and an intent then selects from it.

## Resolution

One call resolves one input or an array in a single pass. Each input is a
tool candidate (`invocationName`, optional `packageUrl`, `detectedVersion`,
and `intent`), or a bare name.

- A `purl` match (on type, namespace, and name) is strong; an
  `invocation-name` match is weak and needs `allowWeakIdentityFallback`.
  Names compare case-insensitively on Windows and macOS and exactly on Linux.
  Purl components other than type/namespace/name are ignored with a
  `purl_components_ignored` warning; an unparseable purl is `purl_invalid`.
- Per-input statuses:

  | Situation | Status | Contributes |
  |---|---|---|
  | No version | `matched_default` | default (+ platform) |
  | Version in a range | `matched_version` | default + that overlay |
  | Valid version in no range | `version_out_of_range` | default (+ platform) |
  | Unparseable version | `version_unparseable` | nothing |
  | Intent not defined | `intent_unsupported` | nothing |
  | No eligible entry | `tool_unmatched` | nothing |

  No intent selects the base plus every intent. Other inputs still resolve.
- Dependencies contribute their default plus platform base additions; a
  reference naming intents adds those intents. `resolvedDependencies` is
  de-duplicated and attributes each dependency to every requesting input
  (`inputIndexes`).
- Composition satisfies every contributing pair, and never more: paths are
  substituted, normalized, and de-duplicated per layer; read-write supersedes
  read-only; overlapping catalog denies are removed with structured
  warnings. Alias paths are preserved as written. Paths whose filesystem
  object identity cannot be established fail closed for that pair. The
  composed result is validated as an exact MXC 1.0.0 request before it is
  returned.
- Warnings are structured objects with a stable `code` and code-specific
  fields (for example `entryId`, `detectedVersion`, `ignoredComponents`).

Verifying that the executable really carries the strong identity passed in
`packageUrl`, and that `detectedVersion` is accurate, is the caller's job.
The caller's own restrictions always win.

## Failures

Failures reuse MXC's error codes, with a stable sub-reason: `details.reason`
on the Node `MxcError` and `MxcException.Reason` in C#. The Rust `Error`
carries the code only.

| Code | Reason | Meaning |
|---|---|---|
| `policy_validation` | `invalid_catalog` | Catalog data violates the contract, or a resolved field does not fit the v1 section types. |
| `policy_validation` | `composition_conflict` | The contributing pairs cannot be composed without granting broader access. |
| `policy_validation` | `ambiguous_match` | An input matches more than one entry at the same rank. |
| `malformed_request` | `invalid_context` | The caller's inputs or context are invalid. |
| `unsupported_containment` | `unsupported_host` | The host platform, or a needed native architecture, cannot be determined. |
| `backend_error` | `integrity` | Bundled catalog data cannot be read or declares the wrong revision. |
| `backend_error` | `revision_unavailable` | An explicitly requested revision is not bundled. |

Absence is not a failure: the resolve call yields no requirements.

## Changing the catalog

Authors edit one JSON file per tool under
`src/mxc-sdk/policy_store/catalog/entries/`. Each file holds that tool's full
entry: identity, default, intents, every platform and version variant,
dependencies, and provenance. Optional category subdirectories are
organizational only; a file's path never affects identity, lookup, or
composition. `revisions/<catalogRevision>.json` and `views/` are generated,
immutable published snapshots: never edit them by hand, and never rewrite one
that is already published.

1. Add a new `{ catalogRevision, file }` (`<YYYY-MM-DD.N>`,
   `revisions/<YYYY-MM-DD.N>.json`) to `catalog/manifest.json` and point
   `defaultRevision` at it. The data is compiled into the native library and
   inherits MXC package signing.
2. Edit the entry files. Bump `entryRevision` for every entry that changes.
   Every entry needs a `versionScheme` and one `default`; overlays are
   additive only, and version ranges in one entry must not overlap.
3. Generate the revision snapshot from the entries with
   `MXC_POLICY_STORE_UPDATE_REVISION=1`, then the views with
   `MXC_POLICY_STORE_UPDATE_VIEWS=1` (both run
   `cargo test -p mxc-sdk --test policy_store_catalog_validation`), and review
   them. The assembler orders entries by `entryId` and rejects duplicate
   `entryId` values and dependencies on entries no file defines.
4. Add or update a case in `conformance/fixtures/bundled-catalog.json`. Every
   entry in the default revision needs one. To record actual outcomes for
   review, run the conformance test with
   `MXC_POLICY_STORE_UPDATE_FIXTURES=<output-dir>`, then merge them by hand.
5. Run the checks (from `src/`). They fail if a generated snapshot or view is
   stale against its inputs:

   ```text
   cargo test -p mxc-sdk --lib policy_store
   cargo test -p mxc-sdk --test policy_store_library --test policy_store_conformance --test policy_store_catalog_validation --test policy_store_vectors --test policy_store_sdk
   cargo test -p mxc_ffi --test policy_store
   ```

   Set `MXC_POLICY_STORE_BASE_REF=origin/main` to also check that no
   published revision changed.

The JSON Schemas in `src/mxc-sdk/policy_store/schema/` describe the catalog
shape for editors. The Rust validation in
`tests/policy_store_catalog_validation.rs` covers the same rules and more,
including every materialized effective policy as an exact MXC 1.0.0 request.
