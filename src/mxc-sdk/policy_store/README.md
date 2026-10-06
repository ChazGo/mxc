# mxc-sdk policy_store — PROTOTYPE, pending API review

> **Prototype.** This API is proposed and pending API review and sign-off
> before check-in. Names, shapes, and catalog contents may change (for
> example, the names may drop "Sandbox"). It is not part of MXC 1.0; a later
> SDK release is targeted.

The MXC policy store resolves known tools to a **best-effort floor**
`SandboxPolicy`: the access a known tool typically needs, which a caller
composes with its own policy. It is not a guarantee, and it is complementary
to Learning Mode rather than a replacement for it.

The V1 catalog in `catalog/` is the single source of truth. `build/build_policy_store.rs` embeds
it with `include_str!`; nothing is downloaded. Each entry has one unversioned
default plus additive platform, version (purl `vers` ranges in the entry's
`versionScheme`), and intent overlays. A lookup takes tool candidates with an
optional `detectedVersion` and `intent`, and reports a per-input status
(`matched_default`, `matched_version`, `version_out_of_range`,
`version_unparseable`, `intent_unsupported`, or `tool_unmatched`). Every
MXC SDK uses this crate:

| SDK | Entry point |
|-----|-------------|
| Rust (`mxc-sdk`) | `mxc_sdk::policy_store::{resolve_sandbox_policy, resolve_sandbox_policy_with_diagnostics, get_catalog_info, list_catalog_entries}` |
| Node (`@microsoft/mxc-sdk`) | `resolveSandboxPolicy`, `resolveSandboxPolicyWithDiagnostics`, `getCatalogInfo`, `listCatalogEntries` through `mxc_ffi` |
| C# (`Microsoft.Mxc.Sdk`) | `MxcPolicyStore.ResolveSandboxPolicy`, `MxcPolicyStore.ResolveSandboxPolicyWithDiagnostics`, `MxcPolicyStore.GetCatalogInfo`, `MxcPolicyStore.ListCatalogEntries` through `mxc_ffi` |

This crate returns its own catalog-shaped policy model. `mxc-sdk` converts it
to `mxc_sdk::policy::SandboxPolicy`, and `mxc_ffi` hands the same JSON to the
Node and C# SDKs.

## Layout

| Path | Contents |
|------|----------|
| `catalog/` | Contract, manifest, immutable published revisions (V1 data), and generated reviewer views (`views/`) |
| `schema/` | JSON Schemas for the catalog and manifest (editor validation) |
| `conformance/` | Language-neutral fixtures and path/canonical-JSON vectors |
| `src/` | Resolver, effective-policy builder, composition, `vers` schemes, validation, and path rules |
| `tests/` | Library, conformance, vector, and catalog contribution tests |

## Build and test

From `src/`:

```sh
cargo test -p mxc_policy_store
cargo clippy -p mxc_policy_store --all-targets -- -D warnings
```

`tests/policy_store_catalog_validation.rs` is the contribution gate: contract,
entry-revision history, every materialized effective policy, deterministic
resolution, conformance coverage, and a current reviewer view
(`MXC_POLICY_STORE_UPDATE_VIEWS=1` regenerates `catalog/views/`).
Set `MXC_POLICY_STORE_BASE_REF=<ref>` (for example `origin/main`) to also
check that already-published revisions are unchanged.

## Contributing a revision

Published revisions are immutable. See
[`docs/policy-store/README.md`](../../../docs/policy-store/README.md#changing-the-catalog)
for the steps, and [`docs/policy-store/design.md`](../../../docs/policy-store/design.md)
for the design.
