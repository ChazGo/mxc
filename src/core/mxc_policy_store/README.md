# mxc_policy_store — PROTOTYPE, pending API review

> **Prototype.** This API is proposed and pending API review and sign-off
> before check-in. Names, shapes, and catalog contents may change (for
> example, the names may drop "Sandbox", and lookup may gain an intent such
> as `git pull` versus `git push`). It is not part of MXC 1.0; a later SDK
> release is targeted.

The MXC policy store resolves known tools to a **best-effort floor**
`SandboxPolicy`: the access a known tool typically needs, which a caller
composes with its own policy. It is not a guarantee, and it is complementary
to Learning Mode rather than a replacement for it.

The V1 catalog in `catalog/` is the single source of truth. `build.rs` embeds
it with `include_str!`, and the store checks each revision's published
SHA-256 on first use. Nothing is downloaded. Every MXC SDK uses this crate:

| SDK | Entry point |
|-----|-------------|
| Rust (`mxc-sdk`) | `mxc_sdk::policy_store::{resolve_sandbox_policy, resolve_sandbox_policy_with_diagnostics, get_catalog_info, list_catalog_entries}` |
| Node (`@microsoft/mxc-sdk`) | `resolveSandboxPolicy`, `resolveSandboxPolicyWithDiagnostics`, `getCatalogInfo`, `listCatalogEntries` through `mxc_ffi` |
| C# (`Microsoft.Mxc.Sdk`) | `PolicyStore.ResolveSandboxPolicy`, `PolicyStore.ResolveSandboxPolicyWithDiagnostics`, `PolicyStore.GetCatalogInfo`, `PolicyStore.ListCatalogEntries` through `mxc_ffi` |

This crate returns its own catalog-shaped policy model. `mxc-sdk` converts it
to `mxc_sdk::policy::SandboxPolicy`, and `mxc_ffi` hands the same JSON to the
Node and C# SDKs.

## Layout

| Path | Contents |
|------|----------|
| `catalog/` | Contract, manifest, and immutable published revisions (V1 data) |
| `schema/` | JSON Schemas for the catalog and manifest (editor validation) |
| `conformance/` | Language-neutral fixtures and path/canonical-JSON vectors |
| `src/` | Resolver, store, integrity, validation, and path rules |
| `tests/` | Library, conformance, vector, and catalog contribution tests |

## Build and test

From `src/`:

```sh
cargo test -p mxc_policy_store
cargo clippy -p mxc_policy_store --all-targets -- -D warnings
```

`tests/catalog_validation.rs` is the contribution gate: integrity, contract,
entry-revision history, deterministic resolution, and conformance coverage.
Set `MXC_POLICY_STORE_BASE_REF=<ref>` (for example `origin/main`) to also
check that already-published revisions are unchanged.

## Contributing a revision

Add a new file under `catalog/revisions/`, list it with its canonical SHA-256
in `catalog/manifest.json`, add a bundled-catalog conformance case under
`conformance/fixtures/`, and run the tests above. Published revisions are
immutable.
