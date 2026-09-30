# Known-tool policy catalog (prototype)

> **Prototype.** This directory prototypes the standalone catalog and library
> described in the [Known-tool Policy Floors design proposal](docs/design.md),
> which is proposed in MXC as `docs/mxc-policy-store.md`
> ([microsoft/mxc#1309](https://github.com/microsoft/mxc/pull/1309)). It is not
> an approved, shipped, or supported catalog. The design calls the catalog a
> time-limited bridge that is expected to be retired when Learning Mode
> provides the replacement workflow.

This directory is laid out as its own repository. It has no imports from, and
no build or test dependency on, the enclosing MXC tree. You can extract it
with:

```text
git filter-repo --subdirectory-filter policy-catalog
```

`npm run check:extract` proves this by building and testing a fresh copy.

## Layout

| Path | Contents |
|---|---|
| `docs/design.md` | Verbatim copy of the design proposal (authoritative). Its relative links point into the MXC repository; the header lists absolute equivalents. |
| `docs/architecture.md` | How this prototype implements the design. |
| `docs/contributing.md` | Catalog and library contribution flow, validation rules, and test layout. |
| `catalog/contract.v1.json` | Catalog contract v1: registered `SandboxPolicy` versions and the symbol vocabulary. |
| `catalog/manifest.json` | Append-only list of published revisions, each with its canonical SHA-256, plus the default revision. |
| `catalog/revisions/<YYYY-MM-DD.N>.json` | Immutable published catalog revisions. |
| `schema/` | JSON Schemas for revisions and the manifest. |
| `conformance/fixtures/` | Language-neutral conformance cases that every language binding must pass. |
| `src/` | TypeScript/JavaScript library and the `policy-catalog` CLI. |
| `tests/unit/` | Unit and conformance tests (`node:test`), with their own `tsconfig.json` and `run-tests.js`. |
| `tests/functional/` | End-to-end tests against the packed and installed tarball: the installed CLI and library, the real catalog, and tampered or synthetic catalogs. Own `tsconfig.json` and `run-tests.js`. |
| `scripts/` | Validation, digest, package, and extractability checks. |
| `.github/workflows/ci.yml` | CI for the standalone repository. |

## Library API

Runtime lookup is kept separate from inspection
([design §5](docs/design.md#5-api-surface)):

```ts
import {
  getSandboxConfig,
  getSandboxConfigWithDiagnostics,
  getCatalogInfo,
  listCatalogEntries,
} from '@mxc-prototype/policy-catalog';

const ctx = {
  allowWeakIdentityFallback: true,
  projectRoot: '/work/app',
  symbols: { git_prefix: '/usr/bin', npm_prefix: '/usr/local/bin', npm_cache: '/home/me/.npm', node_prefix: '/usr/local/bin' },
};

getSandboxConfig('npm', ctx);                             // one tool -> SandboxPolicy | undefined
getSandboxConfig(['git', 'npm'], ctx);                    // several tools -> one composed policy
getSandboxConfigWithDiagnostics(['git', 'npm'], ctx);     // same policy plus attribution and warnings
getSandboxConfig({ invocationName: 'npx', packageUrl: 'pkg:npm/npm@10.9.0' }, ctx);

getCatalogInfo();      // { catalogSchemaVersion: '1', catalogRevision: '2026-09-29.1' }
listCatalogEntries();  // metadata only, never policy bodies
```

The same operations are available from the command line. The CLI has
exactly three commands:

| Command | Purpose |
|---|---|
| `policy-catalog resolve [options] <tool>...` | `getSandboxConfig`, or `getSandboxConfigWithDiagnostics` with `--diagnostics`. Options: `--platform`, `--architecture`, `--revision`, `--project-root`, `--symbol name=value`, `--allow-weak`, and `--purl` / `--detected-version` for the next tool. |
| `policy-catalog inspect` | `getCatalogInfo()` and `listCatalogEntries()`: metadata only, never a policy body. |
| `policy-catalog validate [--base-ref REF]` | Integrity, the catalog contract (including dependency cycles), and entry-revision history. With `--base-ref`, also checks that every revision published at `REF` is unchanged. |

Every command accepts `--catalog DIR` to use a catalog directory instead of the
bundled one. Output is JSON on stdout. Exit codes: `0` success (including "no
policy"), `1` library failure or failed validation, `2` usage error.

```text
npx policy-catalog resolve --diagnostics --allow-weak --project-root /work/app \
  --symbol git_prefix=/usr/bin git
npx policy-catalog validate --catalog ./catalog --base-ref origin/main
```

### Results

- The result is a candidate **lower bound**, not authorization. Consumers
  decide whether lookup is enabled, and own authorization, elevation,
  persistence, composition with their own layers and ceilings, approval,
  audit, and sandbox creation
  ([design §2](docs/design.md#2-ownership-boundary),
  [design §5.3](docs/design.md#53-consumer-obligations)).
- Absence is not an empty policy. `getSandboxConfig` returns `undefined` for
  an empty input, when no input matched, or when a selected entry has an
  unresolved required symbol. `getSandboxConfigWithDiagnostics` returns
  `policy: undefined` with the diagnostics.
- Library failures throw `PolicyCatalogError` with an MXC error `code`
  (`policy_validation`, `malformed_request`, `unsupported_containment`,
  `backend_error`) and a stable `details.reason`. Warnings are plain strings.
  See [architecture](docs/architecture.md#failure-codes).

### Matching and selection

- Matching is additive: every eligible matching entry contributes, and each
  contributes once.
- `purl` identity is strong. Invocation-name-only identity is weak and is
  **off by default**; a caller opts in per call with
  `allowWeakIdentityFallback: true` (CLI `--allow-weak`).
- The catalog records strong evidence (package URL, signer). **Verifying that
  the binary about to run actually has that strong identity is the caller's
  job**; the library cannot see the binary.
- Casing follows the target OS: Windows and macOS compare invocation names and
  paths case-insensitively; Linux compares them exactly (`gh` is not `GH`).
  Catalog validation always treats names case-insensitively when it checks for
  duplicates.
- An omitted `platform` uses the host. An omitted `architecture` uses the
  device's native system architecture, not the process architecture, and
  diagnostics warn that the tool's architecture was not verified. An omitted
  `catalogRevision` uses the installed default. An unavailable explicit
  revision is an error.
- Lookup is local and synchronous. It makes no network calls and does not run
  the candidate tool.

## Development

Requires Node.js 24 or later.

```text
npm ci
npm run check             # typecheck, unit, functional (installed tarball), catalog validation, pack contents, install smoke
npm run test:unit
npm run test:functional   # packs, installs the tarball in a temp project, and tests the installed CLI and library
npm run validate          # add -- --base-ref=<ref> (or POLICY_CATALOG_BASE_REF=<ref>) to enforce published-revision immutability
npm run check:pack
npm run check:extract     # build and test HEAD's copy of this directory as a standalone repository
```

See [docs/contributing.md](docs/contributing.md) before changing catalog data.

## Status: implemented and deferred

**Implemented (TypeScript):**

- the four version dimensions
- canonical-digest integrity
- immutable, deep-frozen revisions with CI immutability and monotonicity
  checks
- additive strong/weak identity matching with opt-in weak fallback
- platform and architecture variants with native-architecture defaults and
  diagnostics
- deterministic, cycle-rejecting dependency resolution
- v1 filesystem-only composition across inputs and dependencies
- the policy-only and diagnostics APIs (single tool or array)
- metadata inspection
- the CLI harness (`resolve | inspect | validate`)
- the contribution validation pipeline
- package inclusion and offline install checks
- the extractability check

**Deferred:**

- Rust (Cargo) and C#/.NET (NuGet) libraries
  ([design §6.1](docs/design.md#61-library-distribution-and-consumption)).
  `conformance/fixtures/` is the shared contract they must pass.
- Validation of embedded policies against MXC's real released `SandboxPolicy`
  schemas ([design §4.2](docs/design.md#42-entry-shape)). The prototype
  validates a catalog-supported subset and pins `0.8.0-alpha` and
  `0.9.0-alpha` in `catalog/contract.v1.json`.
- Integration tests that run a real tool under an MXC sandbox
  ([design §12](docs/design.md#12-test-plan), "Integration").
- Verification on real ARM64 and emulated hosts. Native-architecture detection
  is covered with an injected host.
- Private or enterprise overlays
  ([design §13](docs/design.md#13-open-questions)).
- Named-role review enforcement. That belongs to repository governance, not
  CI.
- Reviewed catalog data. `tool:git`, `tool:node`, and `tool:npm` were migrated
  from an earlier prototype and have not been re-observed
  (`provenance.method: prototype-migration`).

## License

MIT. See [LICENSE.md](LICENSE.md).
