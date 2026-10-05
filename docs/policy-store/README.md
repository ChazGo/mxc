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
  outcome and the entry model below.
- [`catalog/views/`](../../src/core/mxc_policy_store/catalog/views/) — a
  generated reviewer view of every effective policy in each revision.

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

## Entry model

Each catalog entry has one unversioned **default** (a `sandboxPolicy`, its
dependencies, and named **intents** such as `fetch` and `push`) and additive
**overlays**: `platformVariants` for a platform or architecture, and
`versionVariants` keyed by a purl `vers` range in the entry's required
`versionScheme` (`npm`, `semver`, `pypi`, `nuget`, or `intdot`). Overlays
can add paths, outbound allow rules, dependencies, and intents; they can never
remove or narrow anything. The **effective policy** is the default plus the
platform overlay plus at most one version overlay, and an intent then selects
from it. See [design §4](design.md#4-data-model).

## Resolution pipeline

One call resolves one input or an array of inputs in a single pass. Each
input is a tool candidate (`invocationName`, optional `packageUrl`,
`detectedVersion`, and `intent`), or a bare name.

1. **Validate inputs and context.** Invalid input fails as
   `malformed_request` (`invalid_context`), never as absence.
2. **Load the revision** — the requested `catalogRevision`, or the bundled
   default. A missing revision is `backend_error` (`revision_unavailable`);
   unreadable bundled data is `backend_error` (`integrity`).
3. **Match each input to at most one entry.** A `purl` match is strong; an
   `invocation-name` match is weak and needs `allowWeakIdentityFallback`.
   Names compare case-insensitively on Windows and macOS and exactly on Linux.
   A version embedded in `packageUrl` is ignored with a warning. Candidates
   are ranked by strength, then intent support, then architecture
   specificity; a tie fails as `policy_validation` (`ambiguous_match`).
4. **Build the effective policy.** The exact architecture overlay wins over
   the neutral one; another architecture is never a fallback. An omitted
   architecture uses the device's native system architecture, with a warning.
   `detectedVersion` selects at most one version overlay.
5. **Select the intent** and record a per-input status:

   | Situation | Status | Contributes |
   |---|---|---|
   | No version | `matched_default` | default (+ platform) |
   | Version in a range | `matched_version` | default + that overlay |
   | Valid version in no range | `version_out_of_range` | default (+ platform) |
   | Unparseable version | `version_unparseable` | nothing |
   | Intent not defined | `intent_unsupported` | nothing |
   | No eligible entry | `tool_unmatched` | nothing |

   No intent selects the base policy plus every intent. Non-default outcomes
   also appear as structured warnings. Other inputs still resolve.
6. **Close over dependencies**, depth-first, rejecting cycles. A dependency
   contributes only its default base plus its platform overlay's base
   additions; it never selects a version overlay. A reference that names
   intents (`{ "entryId": "tool:ssh", "intents": ["connect"] }`) adds those
   intents too, and catalog validation rejects a named intent the dependency
   does not define. Dependency diagnostics report intent mode `none` or
   `named`.
7. **Resolve symbols.** `project_root` comes only from `projectRoot`. A
   missing required symbol yields no policy plus a warning naming it; the
   resolver never returns a partial policy.
8. **Compose** to satisfy every contributing pair, and never more. Paths are
   substituted, normalized, and de-duplicated; read-write supersedes
   read-only; a catalog filesystem deny that overlaps a required path is
   removed; outbound allow rules are unioned, so a tool without network needs
   never vetoes another's; a catalog egress deny that overlaps any required
   allow rule (destinations intersect after `except`, and protocol/port
   selectors intersect) is removed in full, and non-overlapping denies stay.
   Each adjustment produces a warning naming the full removed scope and the
   contributing entries; a conflicting deny never fails the request. Mixed
   policy versions, non-egress network settings across several network
   requirements, and other fields the model cannot express fail as
   `policy_validation` (`composition_conflict`).

Verifying that the executable really carries the strong identity passed in
`packageUrl`, and that `detectedVersion` is accurate, is the caller's job.
The caller's own restrictions always win over the floor.

## Failures

Failures reuse MXC's error codes, with a stable sub-reason: `details.reason`
on the Node `MxcError`, `MxcException.Reason` in C#, and
`PolicyCatalogError::reason()` in Rust.

| Code | Reason | Meaning |
|---|---|---|
| `policy_validation` | `invalid_catalog` | Catalog data violates the contract, or a resolved field does not fit the SDK `SandboxPolicy`. |
| `policy_validation` | `composition_conflict` | The contributing pairs cannot be composed without granting broader access. |
| `policy_validation` | `ambiguous_match` | An input matches more than one entry at the same rank. |
| `malformed_request` | `invalid_context` | The caller's inputs or context are invalid. |
| `unsupported_containment` | `unsupported_host` | The host platform, or a needed native architecture, cannot be determined. |
| `backend_error` | `integrity` | Bundled catalog data cannot be read or declares the wrong revision. |
| `backend_error` | `revision_unavailable` | An explicitly requested revision is not bundled. |

Absence is not a failure: the resolve call returns no policy.

## Changing the catalog

Published revisions are immutable. Never edit a file under
`src/core/mxc_policy_store/catalog/revisions/` that is already listed in
`catalog/manifest.json`; publish a new revision instead.

1. Copy the latest revision to `catalog/revisions/<YYYY-MM-DD.N>.json` and set
   its `catalogRevision`.
2. Make the change. Bump `entryRevision` for every entry that changes. Every
   entry needs a `versionScheme` and one `default`; overlays are additive
   only, and version ranges in one entry must not overlap.
3. Append `{ catalogRevision, file }` to the manifest and point
   `defaultRevision` at it. There is no separate catalog digest: the data is
   compiled into the native library and inherits MXC package signing.
4. Add or update a case in `conformance/fixtures/bundled-catalog.json`. Every
   entry in the default revision needs one. To record actual outcomes
   for review, run the conformance test with
   `MXC_POLICY_STORE_UPDATE_FIXTURES=<output-dir>`, then merge them into the
   fixtures by hand.
5. Regenerate the reviewer view with `MXC_POLICY_STORE_UPDATE_VIEWS=1`
   (`cargo test -p mxc_policy_store --test catalog_validation`) and review it.
   Its *Added to default* column shows what each platform, version, and
   intent row adds to the common default.
6. Run the checks (from `src/`):

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
