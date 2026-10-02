# MXC Policy Store (prototype)

> **PROTOTYPE, pending API review.** The API names and shapes here are proposed
> and may change before sign-off. The policy store is not part of MXC 1.0; a
> later MXC SDK release is targeted.

The policy store resolves known tools (for example `git`, `node`, `npm`) to a
candidate **floor** `SandboxPolicy` from a reviewed catalog bundled statically
in the MXC SDKs. A floor is a best-effort starting point, not a guarantee, and
it never grants access by itself: the caller reviews it, composes it with its
own policy and ceilings, and passes the result to the SDK's existing sandbox
APIs. It is complementary to Learning Mode, not a replacement.

- [design.md](design.md) — the design proposal, updated for the design review
  outcome.

## Where it lives

| Layer | Location | API |
|---|---|---|
| Resolver and V1 data | `src/core/mxc_policy_store` | Single implementation; owns `catalog/`, `schema/`, and `conformance/` |
| Rust SDK | `src/core/mxc-sdk/src/policy_store.rs` | `mxc_sdk::policy_store::{resolve_sandbox_policy, resolve_sandbox_policy_with_diagnostics, get_catalog_info, list_catalog_entries}` |
| C ABI | `src/ffi/mxc_ffi/src/policy_store.rs` | `mxc_resolve_sandbox_policy_json`, `mxc_resolve_sandbox_policy_with_diagnostics_json`, `mxc_policy_catalog_info_json`, `mxc_list_policy_catalog_entries_json`, `mxc_policy_store_result_free` |
| Node SDK | `sdk/node/src/policy-store.ts` | `resolveSandboxPolicy`, `resolveSandboxPolicyWithDiagnostics`, `getCatalogInfo`, `listCatalogEntries` |
| C# SDK | `sdk/dotnet/Microsoft.Mxc.Sdk/MxcPolicyStore.cs` | `MxcPolicyStore.ResolveSandboxPolicy`, `ResolveSandboxPolicyWithDiagnostics`, `GetCatalogInfo`, `ListCatalogEntries` |

There is one resolver. `build.rs` compiles the catalog into the
`mxc_policy_store` crate, so the data ships inside `mxc_ffi` and every SDK
that loads it; nothing is downloaded. The Rust SDK converts the store's policy
into `mxc_sdk::SandboxPolicy`. The Node and C# SDKs send a JSON request
(`{"tools", "context"?}`) through `mxc_ffi` and map the result onto their own
`SandboxPolicy` types. Resolution stays outside `mxc_engine`: it never selects
a backend or launches a sandbox.

## Resolution pipeline

One call resolves one input or an array of inputs in a single pass:

1. **Validate inputs and context.** Invalid input fails as
   `malformed_request` (`invalid_context`), never as absence.
2. **Load the revision** — the requested `catalogRevision`, or the bundled
   default. A missing revision is `backend_error` (`revision_unavailable`); a
   digest mismatch is `backend_error` (`integrity`).
3. **Match each input additively** in `entryId` order. A `purl` match is
   strong; an `invocation-name` match is weak and needs
   `allowWeakIdentityFallback`. Names and paths fold case on Windows and macOS
   and are exact on Linux.
4. **Select a variant.** The exact architecture wins over the
   architecture-neutral variant; another architecture is never a fallback. An
   omitted architecture uses the device's native system architecture, not
   the process architecture.
5. **Close over dependencies**, depth-first, rejecting cycles.
6. **Apply the composition limits**: one `sandboxPolicy.version`, and only
   filesystem lists when more than one entry is selected.
7. **Resolve symbols.** `project_root` comes only from `projectRoot`. A
   missing required symbol yields no policy plus a warning naming it; the
   resolver never returns a partial policy.
8. **Compose.** Paths are substituted, normalized, and de-duplicated per
   access class. Overlapping paths in different classes fail as
   `policy_validation` (`composition_conflict`).

Verifying that the executable really carries the strong identity passed in
`packageUrl` is the caller's job.

## Failures

Failures reuse MXC's error codes, with a stable sub-reason: `details.reason`
on the Node `MxcError`, `PolicyStoreException.Reason` in C#, and
`PolicyCatalogError::reason()` in Rust.

| Code | Reason | Meaning |
|---|---|---|
| `policy_validation` | `invalid_catalog` | Catalog data violates the contract, or a resolved field does not fit the SDK `SandboxPolicy`. |
| `policy_validation` | `composition_conflict` | The selected entries cannot be composed under the V1 rules. |
| `malformed_request` | `invalid_context` | The caller's inputs or context are invalid. |
| `unsupported_containment` | `unsupported_host` | The host platform or native architecture cannot be determined. |
| `backend_error` | `integrity` | Catalog data does not match its published digest. |
| `backend_error` | `revision_unavailable` | An explicitly requested revision is not bundled. |

Absence is not a failure: the resolve call returns no policy.

## Changing the catalog

Published revisions are immutable. Never edit a file under
`src/core/mxc_policy_store/catalog/revisions/` that is already listed in
`catalog/manifest.json`; publish a new revision instead.

1. Copy the latest revision to `catalog/revisions/<YYYY-MM-DD.N>.json` and set
   its `catalogRevision`.
2. Make the change. Bump `entryRevision` for every entry that changes.
3. Append `{ catalogRevision, file, sha256 }` to the manifest and point
   `defaultRevision` at it. To get the digest, enter 64 zeros, run the tests
   below, and copy the actual digest from the integrity failure.
4. Add or update a case in `conformance/fixtures/bundled-catalog.json`. Every
   entry in the default revision needs one.
5. Run the checks (from `src/`):

   ```text
   cargo test -p mxc_policy_store
   cargo test -p mxc-sdk --test policy_store
   cargo test -p mxc_ffi --test policy_store
   ```

   Set `MXC_POLICY_STORE_BASE_REF=origin/main` to also check that no
   published revision changed.

Each new entry or requirement expansion needs identity evidence, supported
version ranges, per-platform evidence, a minimized requirement set (captured
with Learning Mode before the pull request), fixtures, and `provenance`. See
[design §7](design.md#7-contribution-and-review).

The JSON Schemas in `schema/` describe the catalog shape for editors. They are
not run in CI; the Rust validation in `tests/catalog_validation.rs` covers the
same rules and more.
