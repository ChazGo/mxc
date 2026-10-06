# mxc-sdk policy_store — PROTOTYPE, pending API review

> **Prototype.** This API is proposed and pending API review and sign-off
> before check-in. Names, shapes, and catalog contents may change. It is not
> part of MXC 1.0; a later SDK release is targeted.

The MXC policy store resolves known tools to command-free
`ContainerRequirements`: a **best-effort floor** of the access a known tool
typically needs, which a caller reviews and constrains before adding its own
command. It is not a guarantee, and it is complementary to Learning Mode
rather than a replacement for it.

This directory holds the store's data. The resolver is the internal
`policy_store` module in `../src/policy_store/`, which embeds the catalog at
build time; nothing is downloaded.

| SDK | Entry point |
|-----|-------------|
| Rust (`mxc-sdk`) | `mxc_sdk::v1::{resolve_tool_requirements, resolve_tool_requirements_with_diagnostics, get_catalog_info, list_catalog_entries}` |
| Node (`@microsoft/mxc-sdk/v1`) | `resolveToolRequirements`, `resolveToolRequirementsWithDiagnostics` (promise-based), `getCatalogInfo`, `listCatalogEntries` through `mxc_ffi` |
| C# (`Microsoft.Mxc.Sdk.V1`) | `MxcContainer.ResolveToolRequirements`, `ResolveToolRequirementsWithDiagnostics` (plus `Async`), `GetCatalogInfo`, `ListCatalogEntries` through `mxc_ffi` |

## Layout

| Path | Contents |
|------|----------|
| `catalog/` | Contract, manifest, immutable published revisions (V1 data), and generated views (`views/`: reviewer view and exact MXC 1.0.0 requests) |
| `schema/` | JSON Schemas for the catalog and manifest (editor validation) |
| `conformance/` | Language-neutral fixtures and path/canonical-JSON vectors |

## Build and test

From `src/`:

```sh
cargo test -p mxc-sdk --lib policy_store
cargo test -p mxc-sdk --test policy_store_library --test policy_store_conformance --test policy_store_catalog_validation --test policy_store_vectors --test policy_store_sdk
cargo test -p mxc_ffi --test policy_store
```

`tests/policy_store_catalog_validation.rs` is the contribution gate: contract,
entry-revision history, every materialized effective policy validated as an
exact MXC 1.0.0 request, deterministic resolution, conformance coverage, and
current views (`MXC_POLICY_STORE_UPDATE_VIEWS=1` regenerates `catalog/views/`).
Set `MXC_POLICY_STORE_BASE_REF=<ref>` (for example `origin/main`) to also
check that already-published revisions are unchanged.

## Contributing a revision

Published revisions are immutable. See
[`docs/policy-store/README.md`](../../../docs/policy-store/README.md#changing-the-catalog)
for the steps, and [`docs/policy-store/design.md`](../../../docs/policy-store/design.md)
for the prototype's implementation notes.
