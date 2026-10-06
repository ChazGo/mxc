# Policy Store prototype: implementation notes

> **PROTOTYPE, pending API review.** Not part of MXC 1.0; a later MXC SDK
> release is targeted. API names and shapes may change before sign-off.

The design spec is `docs/mxc-policy-store.md` in
[microsoft/mxc#1309](https://github.com/microsoft/mxc/pull/1309). This
prototype tracks spec head `f7a450c`. The spec is the source of truth; this
page is not a copy of it. It records only how the prototype implements the
spec, the decisions it made where the spec leaves room, and its known gaps.
The [README](README.md) describes the API and the catalog workflow.

## Shape

- One resolver, an internal `policy_store` module of `mxc-sdk`
  (`src/mxc-sdk/src/policy_store/`), with the V1 catalog compiled in from
  `src/mxc-sdk/policy_store/catalog/`. There is no separate crate, library,
  or CLI.
- Catalog sources follow spec §6.3: one editable file per tool in
  `catalog/entries/`. `assemble_revision` (a checked generator, like the
  reviewer views) builds the default revision snapshot in `entryId` order and
  rejects duplicate IDs and dangling dependencies; a cargo test fails when the
  checked-in snapshot is stale. The static, manifest-selected embedding is
  unchanged.
- Rust: `mxc_sdk::v1::{resolve_tool_requirements,
  resolve_tool_requirements_with_diagnostics, get_catalog_info,
  list_catalog_entries}` return `ContainerRequirements` built from the v1
  section types. Lookup types live in `mxc_sdk::v1::tool_requirements`.
- Node and .NET call the same resolver through panic-contained `mxc_ffi`
  exports. Node resolution is promise-based (`koffi` async calls, so the
  event loop is not blocked); catalog inspection is synchronous. .NET exposes
  `MxcContainer.ResolveToolRequirements` and
  `ResolveToolRequirementsWithDiagnostics` with `Async` forms.
- Catalog revision `2026-10-06.1` (`sdkContractVersion` `1.0.0`). Every
  effective policy is materialized at build-test time, bound to fixture
  symbols and a validation-only command, and validated as an exact MXC 1.0.0
  `ContainerRequest`; `catalog/views/<revision>.requests.json` records those
  requests and `<revision>.md` is the reviewer view.

## Decisions where the spec leaves room

| Topic | Prototype behavior |
|---|---|
| purl parsing | An `@` before the last `/` is an unencoded npm scope, not a version (`pkg:npm/@scope/name`). Matching compares type, namespace, and name only; other components produce `purl_components_ignored`. |
| purl name normalization | No per-type normalization beyond the spec's comparison rule; open in the spec. |
| Empty `packageUrl` | `malformed_request` / `invalid_context`, like other invalid inputs. |
| Filesystem identity on a foreign platform | A lookup for a platform other than the host cannot examine object identity, so affected pairs fail closed. Tests resolve for the host platform against existing temporary directories. |
| Identity read failures | A discovery read failure maps to `unsupported_containment` / `unsupported_host`. |
| Read-only alias promotion | Reported as `readonly_superseded`. Alias paths are kept as written. |
| Lookup-time exact validation | A composed result that fails exact MXC 1.0.0 validation maps to `policy_validation` / `composition_conflict`. |
| `architecture_fallback` | Also emitted for dependencies and for the default selection. |
| `npm_cache` symbol | Discovery is limited; callers pass it in `symbols`. |
| Error reason | Node `details.reason` and .NET `MxcException.Reason` carry the stable reason. The Rust `Error` carries the code only. |
| FFI request | The `{"tools", "context"?}` lookup request is catalog-lookup input, not policy ingress, so it is not an exact versioned contract. |
| .NET runner contract | The lookup is on `MxcContainer` per the spec but is not part of `IContainerRunner`, which models container execution. |

## Known gaps

- Final API names are pending API review.
- The exact policy validator and version mapping, and purl normalization and
  equality, follow the spec's open items and may change.
- The JSON Schemas in `policy_store/schema/` are editor aids; the Rust
  catalog-validation tests are authoritative.

## Tests

From `src/`: `cargo test -p mxc-sdk --lib policy_store`, the
`policy_store_*` integration tests in `src/mxc-sdk/tests/`, and
`cargo test -p mxc_ffi --test policy_store`. Node:
`sdk/node/tests/unit/policy-store.test.ts`. .NET:
`sdk/dotnet/Microsoft.Mxc.Sdk.Tests/V1/ToolRequirementsTests.cs`.
