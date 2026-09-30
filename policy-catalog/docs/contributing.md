# Contributing

This page describes the contribution flow for catalog data and for the library
([design §7](design.md#7-contribution-and-review)). No client or library can
write a catalog entry at runtime. Every change is a pull request.

## Catalog changes

Published revisions are immutable
([design §10](design.md#10-immutable-revisions)). Never edit a file under
`catalog/revisions/` that is already listed in `catalog/manifest.json`. That
includes corrections and security fixes: publish a new revision instead.

1. Copy the latest revision to `catalog/revisions/<YYYY-MM-DD.N>.json` and set
   `catalogRevision` to the same identifier.
2. Make the change. Bump `entryRevision` for every entry that changes
   semantically, and leave unchanged entries alone. Validation enforces both
   rules.
3. Build, compute the new file's digest, and append it to the manifest:

   ```text
   npm run build
   npm run digest -- catalog/revisions/<YYYY-MM-DD.N>.json
   ```

   Add `{ catalogRevision, file, sha256 }` to the end of `revisions` and point
   `defaultRevision` at it.
4. Add or update cases in `conformance/fixtures/`. Every entry in the default
   revision must appear in at least one bundled-catalog case.
5. Run the full check:

   ```text
   npm run check
   npm run validate -- --base-ref=origin/main
   npx policy-catalog validate --base-ref origin/main   # same check through the CLI
   ```

### Entry requirements

Each new entry, or each requirement expansion, needs the following
([design §7](design.md#7-contribution-and-review)):

- identity evidence and the supported tool version ranges
- evidence for each platform variant
- a minimized requirement set; capture it with MXC Learning Mode before the
  pull request ([design §8](design.md#8-relationship-to-learning-mode))
- test fixtures
- `provenance` that references the review evidence

A requirement reduction needs regression evidence that every supported tool
version still works under the narrower requirement.

Review requires one catalog-owner approval and one security/policy-reviewer
approval, plus tool- or scenario-owner evidence where available. CI cannot
enforce this; the repository's branch rules and CODEOWNERS must.

### Rules enforced by `npm run validate`

- JSON Schema conformance (`schema/`)
- digest integrity for every revision in the manifest
- exact registered `SandboxPolicy` versions (`catalog/contract.v1.json`)
- entry-ID uniqueness, with no repeated identity predicate within one entry
- valid platform and architecture selectors: no duplicate exact selector and
  at most one architecture-neutral variant per platform
- no backend-specific keys in a variant
- dependency closure within the same revision, with cycles rejected
- only declared symbols; every path is anchored at a symbol, with no literal,
  user-specific, wildcard, or `..` paths
- no wildcard network grants: no default-allow, no allow rule without `to`,
  no `/0` CIDR
- unsupported-field rejection
- v1 composition limits within every dependency closure
- `entryRevision` monotonicity across consecutive revisions
- published-revision immutability against `--base-ref=<ref>` (or `POLICY_CATALOG_BASE_REF`); an unknown ref fails
- deterministic, file-order-independent resolution for every selector
- package inclusion
- fixture coverage

## Library changes

Changes to the library API or behavior must keep the shared contract intact:

- Any behavior visible across languages goes into `conformance/fixtures/`,
  and the change must describe what the Rust and C# bindings need to match
  ([design §6.2](design.md#62-cross-language-consistency-and-support)).
- Error codes, `details.reason` values, and absence semantics are part of the
  contract. Changing them is a breaking change.
- Run `npm run check` and `npm run check:extract -- --worktree` before
  opening a pull request. If you touch `rust/`, `dotnet/`, or anything the
  CLIs print, also run the Rust and .NET commands below and
  `node scripts/cross-language-check.mjs` (after `npm run build`).

## Rust and .NET bindings

The Rust crate `mxc-policy-catalog` (`rust/`, its own Cargo `[workspace]`)
and the .NET package `Microsoft.Mxc.PolicyCatalog` (`dotnet/`, net8.0) are
native implementations of the same behavior, not wrappers around the
TypeScript library. Each one embeds the bundled catalog, passes every case in
`conformance/fixtures/` and `conformance/vectors/`, and ships a
`policy-catalog` CLI whose output must match the TypeScript CLI.

```text
# Rust (toolchain pinned by rust/rust-toolchain.toml)
cd rust
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cd ..
node scripts/rust-functional.mjs      # cargo package, then a consumer crate outside the repo

# .NET (SDK pinned by dotnet/global.json)
cd dotnet
dotnet build Microsoft.Mxc.PolicyCatalog.slnx -c Release
dotnet test --solution Microsoft.Mxc.PolicyCatalog.slnx -c Release --no-build
cd ..
node scripts/dotnet-functional.mjs    # dotnet pack, then a consumer project on a local feed outside the repo

# All three CLIs must print byte-identical normalized JSON
npm run build
node scripts/cross-language-check.mjs
```

When `src/` behavior changes, change the Rust and .NET code in the same pull
request, add or update a conformance fixture, and add a cross-language case
if the CLI output changes.

## Test layout

| Suite | Location | Command |
|---|---|---|
| Unit and conformance | `tests/unit/*.test.ts` | `npm run test:unit` |
| Functional (installed package, end to end) | `tests/functional/*.test.ts` | `npm run test:functional` |
| Catalog validation | `scripts/validate-catalog.mjs` | `npm run validate` |
| Package contents | `scripts/check-pack.mjs` | `npm run check:pack` |
| Offline install smoke test | `scripts/package-smoke.mjs` | `npm run smoke:package` |
| Standalone extraction | `scripts/check-extractable.mjs` | `npm run check:extract` |
| Rust unit and conformance | `rust/src`, `rust/tests` | `cargo test --locked` (in `rust/`) |
| Rust functional (packaged `.crate`) | `scripts/rust-functional.mjs` | `node scripts/rust-functional.mjs` |
| .NET unit and conformance | `dotnet/Microsoft.Mxc.PolicyCatalog.Tests` | `dotnet test` (in `dotnet/`) |
| .NET functional (packed `.nupkg`) | `scripts/dotnet-functional.mjs` | `node scripts/dotnet-functional.mjs` |
| Cross-language CLI parity | `scripts/cross-language-check.mjs` | `node scripts/cross-language-check.mjs` |

The layout mirrors MXC's `sdk/node/tests/integration`. Each suite directory
has its own `tsconfig.json` that compiles `*.test.ts` into that directory's
`dist/`, and a `run-tests.js` that runs the compiled `*.test.js` files with
`node --test --test-reporter spec --test-force-exit`. Before either runner,
`scripts/check-tests-present.mjs` (after MXC's
`scripts/versioning/check-tests-present.js`) fails the run when the compiled
directory holds no test files.

- Unit tests import the package by its own name, so they exercise the
  compiled `dist/` output that ships.
- Functional tests never use the repository's `dist/` or `catalog/`.
  `tests/functional/run-tests.js` runs `npm pack`, installs the tarball
  offline into a new consumer project in a temporary directory outside the
  repository, and passes that directory to the tests. The tests load the
  library through the installed package's `exports` and run the installed
  `bin`, in a separate process. `helpers.ts` refuses to run if the
  installed package resolves inside the source tree.
- Functional tests that need a defective catalog write a synthetic or
  tampered copy into a temporary directory and pass it with `--catalog`.
