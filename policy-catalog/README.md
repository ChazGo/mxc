# Known-tool policy catalog (prototype)

> **Prototype.** This directory prototypes the standalone catalog and library
> proposed in the MXC Policy Store feature spec
> ([microsoft/mxc#1309](https://github.com/microsoft/mxc/pull/1309),
> `docs/mxc-policy-store.md`). It is not an approved, shipped, or supported
> catalog. The spec describes the catalog as a time-limited bridge that is
> expected to be retired when Learning Mode provides the replacement workflow.

The directory is self-contained so it can be moved to its own repository
(spec §6) without changes:

```text
git filter-repo --subdirectory-filter policy-catalog
```

Nothing here imports from the rest of the MXC tree, and no MXC SDK imports
from here. The spec states that the proposal makes no MXC SDK changes.

## Layout

| Path | Contents |
|---|---|
| `catalog/contract.v1.json` | Catalog contract v1: the exact registered `SandboxPolicy` versions entries may embed, plus the symbol vocabulary. |
| `catalog/manifest.json` | Append-only list of published revisions, each with its canonical SHA-256 digest, plus the default revision. |
| `catalog/revisions/<YYYY-MM-DD.N>.json` | Immutable published catalog revisions. |
| `schema/` | JSON Schemas for revisions and the manifest. |
| `conformance/fixtures/` | Language-neutral conformance cases that every language binding must pass. |
| `typescript/` | TypeScript/JavaScript library, tests, and CI/contribution tooling. |
| `.github/workflows/ci.yml` | CI for the future standalone repository. It is inactive while this directory is nested. |

## Library API (TypeScript)

Runtime lookup and inspection are separate (spec §5):

```ts
import { resolveCatalogEntry, listCatalogEntries, getCatalogInfo } from '@mxc-prototype/policy-catalog';

getCatalogInfo();        // { catalogSchemaVersion: '1', catalogRevision: '2026-09-29.1' }
listCatalogEntries();    // metadata only; no policy bodies

const result = resolveCatalogEntry(
  { invocationName: 'npm', packageUrl: 'pkg:npm/npm@10.9.0' },
  { projectRoot: '/work/app', symbols: { npm_prefix: '/usr/local/bin', npm_cache: '/home/me/.npm', node_prefix: '/usr/local/bin' } },
);
// result?.policy is a candidate minimum SandboxPolicy (a lower bound, not authorization).
```

- A no-match result is `undefined`, not an empty policy. Library failures
  throw `PolicyCatalogError` with a stable `category`: `integrity`,
  `validation`, `revision-unavailable`, `invalid-context`, `unsupported-host`,
  or `composition-conflict`.
- Omitted `platform` and `architecture` values use the host. Architecture
  comes from the OS machine type, not `process.arch`. An omitted
  `catalogRevision` uses the installed default. An explicitly requested
  revision that is missing is an error, never a substitution.
- Weak (invocation-name-only) identity matches only with
  `allowWeakIdentityFallback: true`. A package-URL (`purl`) match is strong. A
  version-range mismatch returns a warning, not a refusal.
- `projectRoot` and caller symbols are never fabricated. A required symbol
  that is unresolved produces `undefined`. The resolver can derive only
  `host` symbols (`user_home`, `temp_dir`), and only for the current host
  platform.
- Lookup is local and synchronous. It makes no network calls and does not
  run the tool.

### Consumer boundary

The library does not grant access, create a sandbox, write consumer state, or
compose with consumer policy. Consumers own the following decisions (spec
§2, §5.3):

- whether lookup is enabled
- access-profile mapping, authorization, and elevation
- persistence, keyed by `entryId`, `entryRevision`, and `catalogRevision`
- composition with their own layers and ceilings
- approval UX and audit
- the final call to `createConfigFromPolicy()` or sandbox creation

## Data model and rules implemented

- **Versions (spec §4.1):** The four dimensions are `catalogSchemaVersion`,
  `catalogRevision`, per-entry `entryRevision`, and per-variant
  `sandboxPolicy.version`. The tool `versionRange` is advisory evidence only.
- **Identity (spec §4.3):** Predicates are ordered strongest first and
  validated. `purl` is strong and `invocation-name` is weak. Identity keys
  must be unique across entries; invocation names are compared
  case-insensitively for this check.
- **Platform variants (spec §4.4):** Each variant is a complete policy per
  `windows`/`linux`/`macos`, with an optional `x64`/`arm64` selector. Variants
  are never merged. An exact architecture wins over the neutral variant.
  Duplicate exact selectors and more than one neutral variant are rejected.
  Backend-specific keys are rejected.
- **Dependencies and composition (spec §4.5):** Dependency closure is
  depth-first, deterministic, and rejects cycles. Only
  `filesystem.deniedPaths`, `readonlyPaths`, and `readwritePaths` compose
  across entries: paths are normalized with platform rules and de-duplicated
  per class. Cross-class equal or ancestor/descendant paths are rejected.
  Mixed policy versions are rejected. `network`, `ui`, `timeoutMs`, and every
  other field are rejected in a multi-entry closure. A dependency
  `versionRange` is checked only for syntax and is returned as unevaluated
  metadata.
- **Integrity and immutability (spec §10):** Each revision must match its
  manifest digest, computed over canonical JSON. Loaded data is deep-frozen.
  CI rejects edits to, or removal of, published revisions. A change requires
  a new revision.

## Contribution / CI pipeline (spec §7, §12 "Data")

```text
cd typescript
npm ci
npm test                 # unit + conformance tests
npm run validate         # full catalog validation pipeline
POLICY_CATALOG_BASE_REF=origin/main npm run validate   # + immutability vs base
npm run smoke:package    # pack, offline install, public-API smoke test
```

`validate` checks the following:

- JSON Schema conformance
- manifest integrity digests
- the semantic contract (registered `SandboxPolicy` versions, identity
  uniqueness and ordering, selectors, dependency closure and cycle freedom,
  symbol validity, no literal, user-specific, or `..` paths, no wildcard
  filesystem or network grants, and unsupported-field rejection)
- `entryRevision` monotonicity across revisions
- published-revision immutability against a base ref
- deterministic resolution for every selector
- package inclusion
- a conformance fixture for every entry

### Publishing a new revision

1. Copy the latest revision to `catalog/revisions/<new-id>.json` and edit it.
   Bump `entryRevision` for every entry that changed semantically.
2. Run `npm run build && npm run digest -- ../catalog/revisions/<new-id>.json`.
   Append the result to `manifest.json` and update `defaultRevision`.
3. Add or update conformance fixtures, then run `npm run validate`.

Human review requirements (spec §7) apply to every catalog change. These
include catalog-owner and security-reviewer approval, identity, platform, and
version evidence, and regression evidence for reductions. This prototype
cannot enforce them.

## Deferred / not implemented

- **Rust (Cargo) and C#/.NET (NuGet) libraries (spec §6.1):** Not
  implemented. The fixtures in `conformance/fixtures/` are the shared
  contract those bindings must satisfy.
- **Validation of embedded policies against the real MXC `SandboxPolicy`
  schemas:** This prototype validates a catalog-supported subset of the 0.8
  and 0.9 contracts and pins the registered versions in
  `contract.v1.json`. Full validation against MXC's released schemas would
  need a vendored or published schema artifact after extraction.
- **Cross-platform package CI and host-default tests on real ARM64 or
  emulated hosts:** The workflow runs a Windows/Linux/macOS matrix, but the
  emulation cases are covered only by unit tests with an injected host.
- **Private or enterprise overlays, and a hosted service:** Out of scope
  (spec §13).
- **Catalog entries:** `tool:git`, `tool:node`, and `tool:npm` were migrated
  from the earlier config-floor prototype. They have not been observed or
  reviewed; `provenance.method` is `prototype-migration`.
