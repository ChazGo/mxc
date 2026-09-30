# mxc-policy-catalog (Rust) — PROTOTYPE

A native Rust implementation of the policy catalog library. It behaves the same
as the TypeScript library in `../src`. It is not a wrapper, and it needs no Node
at runtime. The catalog is compiled into the crate.

- Library: `mxc_policy_catalog` provides `get_sandbox_config`, `get_sandbox_config_with_diagnostics`,
  `get_catalog_info`, `list_catalog_entries`, and `PolicyCatalog` (a store plus an injectable
  `HostEnvironment`). Contribution and CI rules are in `mxc_policy_catalog::tooling`.
- Binary: `policy-catalog resolve | inspect | validate`. Its output matches `node dist/cli.js`.

Run every command below from this `rust/` directory. The toolchain is pinned in
`rust-toolchain.toml` (1.93, with clippy and rustfmt).

## Build and test

```sh
cargo build
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test          # unit + ../conformance fixtures + ../conformance/vectors
```

## Bundled catalog

The source of truth is `../catalog`. `build.rs` embeds it with `include_str!`
whenever it is present. Inside a packaged `.crate` (detected by
`Cargo.toml.orig`), `build.rs` embeds the crate-local `catalog/` instead.
`catalog/` is a git-ignored copy that `scripts/sync-catalog.mjs` writes. It is
listed in `Cargo.toml` `include`, so it ships in the `.crate`. A test checks
that the embedded data equals `../catalog` (canonical JSON).

## Package

```sh
node scripts/sync-catalog.mjs
cargo package       # builds and verifies the packaged crate in isolation
```

## Functional tests (packaged crate)

```sh
node ../scripts/rust-functional.mjs
```

The driver syncs the catalog and runs `cargo package`. It then extracts the
`.crate` into a temporary directory outside the repository and builds the
packaged `policy-catalog` binary from the extracted crate. Next, it creates a
consumer crate from `functional/` (`Cargo.toml.in`, `src/`, `tests/`) that
depends on the extracted crate by path. Finally, it runs the consumer's tests,
which use both the library API and the packaged CLI. Every build uses a
`CARGO_TARGET_DIR` inside the temporary directory.
