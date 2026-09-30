# Architecture

This page describes how the TypeScript prototype implements the
[design proposal](design.md). The design is authoritative. Where this page and
the design disagree, the design wins and this page is a bug.

## Boundary

The library owns a read-only, integrity-validated catalog and a resolver that
turns tool inputs into a **candidate lower-bound** `SandboxPolicy`
([design §2](design.md#2-ownership-boundary)).

The library does not:

- grant access or authorize anything
- create or launch a sandbox
- contact the network or download catalog updates
- run or inspect the candidate tool
- write any consumer state
- import from, or require, the MXC SDKs or executors

Consumers keep every decision the design assigns to them: whether lookup is
enabled, authorization and elevation, persistence, composition with their own
layers and ceilings, approval, audit, and the final sandbox creation.

## Modules

| Module | Responsibility |
|---|---|
| `src/types.ts` | Public types from design §5 (`ToolInput`, `ResolveContext`, `SandboxConfigResolution`, metadata). |
| `src/errors.ts` | `PolicyCatalogError` and its stable failure categories. |
| `src/canonical-json.ts` | Canonical JSON and SHA-256 used for revision integrity. |
| `src/catalog.ts` | Contract and revision validation, variant selection, dependency closure, composition limits. |
| `src/store.ts` | Manifest validation; lazy, digest-checked, deep-frozen revision loading. |
| `src/host.ts` | Host platform, native system architecture, and approved host symbols. |
| `src/resolver.ts` | `getSandboxConfig`, `getSandboxConfigWithDiagnostics`, `getCatalogInfo`, `listCatalogEntries`. |
| `src/history.ts` | `entryRevision` monotonicity and published-revision immutability checks. |
| `src/validate.ts` | Catalog-directory validation and the `--base-ref` immutability check, shared by `policy-catalog validate` and `scripts/validate-catalog.mjs`. Tooling only; the runtime lookup path never runs git. |
| `src/cli.ts` | `policy-catalog resolve \| inspect \| validate` command-line harness over the public API (functional tests, CI, contributors). |
| `src/tooling.ts` | `./tooling` subpath exports for validation scripts and tests. It is not part of the runtime API. |

## Resolution pipeline

One call resolves one input or an array of inputs in a single pass. A single
input is treated exactly like a one-element array.

1. **Validate inputs and context.** Invalid input throws `invalid-context`. It
   is never reported as absence.
2. **Load the revision.** Use the requested `catalogRevision`, or the
   installed default. A missing revision is `revision-unavailable`, never a
   substitution. A digest mismatch is `integrity`.
3. **Match each input additively** (design §4.3). Entries are visited in
   `entryId` order, so catalog file order is irrelevant. For each entry, every
   satisfied predicate is collected in declaration order:
   - A `purl` match is strong.
   - An `invocation-name` match is weak and needs `allowWeakIdentityFallback`.
   - Invocation names compare case-insensitively on Windows and macOS, and
     case-sensitively on Linux.
   - A strong match never suppresses another entry's eligible weak match.
4. **Select a variant** (design §4.4). The exact architecture wins over the
   architecture-neutral variant, and another architecture is never used as a
   fallback. An omitted architecture uses the native system architecture (see
   below), which is detected lazily, only when a candidate entry needs it.
5. **Close over dependencies.** Traversal is depth-first in declaration order
   and rejects cycles. Every selected entry is keyed by `entryId`, so it
   contributes once no matter how many inputs or edges reach it.
6. **Apply the composition limits** (design §4.5) to the whole selected set:
   - every policy declares the same `sandboxPolicy.version`
   - with more than one entry, only `filesystem.{deniedPaths,readonlyPaths,readwritePaths}`
     may appear
7. **Resolve symbols.** `project_root` comes only from `projectRoot`. Other
   symbols come from `symbols`; `host` symbols are derived only for the current
   host platform. If any required symbol is missing, the result is
   `policy: undefined` plus a warning that names the symbol. The resolver never
   returns a partial policy.
8. **Compose.** Paths are substituted, normalized with the platform's path
   rules, and de-duplicated per access class in first-seen order. Equal or
   ancestor/descendant paths in different classes throw
   `composition-conflict`. The resolver never picks an access class.

Diagnostics follow the design exactly:

- tool records in input order
- matches ordered by `entryId`
- matched predicates in declaration order
- dependency metadata de-duplicated and ordered by
  entryId/entryRevision/requiredVersionRange
- warnings for weak-only matches, multi-entry matches, version-range
  mismatches, unmatched inputs, the host-default architecture, and neutral
  fallback

## Native system architecture

The spec requires the device's native architecture rather than the process
architecture ([design §4.4](design.md#44-platform-variants)).
`nodeHostEnvironment.nativeArchitecture()` reads the following:

- **Windows:** the machine-wide `PROCESSOR_ARCHITECTURE` in
  `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment`, read
  with `%SystemRoot%\System32\reg.exe`. An x64 process emulated on ARM64 sees
  `AMD64` in its own environment and from `os.machine()`, but the machine-wide
  value is `ARM64`.
- **macOS:** `sysctl -n hw.optional.arm64`, read with `/usr/sbin/sysctl`. It
  reports Apple silicon even under Rosetta. Intel Macs lack the key.
- **Linux:** `os.machine()`, the kernel machine type.

Executables are invoked by absolute path. An unknown or unreadable value throws
`unsupported-host`. The resolver never guesses. The result is cached for the
process.

## Integrity and immutability

`catalog/manifest.json` lists every published revision with a SHA-256 over its
canonical JSON form (keys sorted recursively, no insignificant whitespace), so
formatting or line-ending changes never invalidate a digest. Loaded revisions
are deep-frozen. The validation pipeline compares proposed changes against a
base git ref and rejects any change to a published revision's manifest entry
or content ([design §10](design.md#10-immutable-revisions)).

## Failure categories

| Category | Meaning |
|---|---|
| `integrity` | Catalog data cannot be read or does not match its published digest. |
| `validation` | Catalog, manifest, or contract data violates the catalog contract. |
| `revision-unavailable` | An explicitly requested revision is not installed. |
| `invalid-context` | The caller's inputs or `ResolveContext` are invalid. |
| `unsupported-host` | The host platform or native architecture cannot be mapped or determined. |
| `composition-conflict` | The selected entries cannot be composed under the v1 rules. |

Absence is not a failure. `getSandboxConfig` returns `undefined`, and
`getSandboxConfigWithDiagnostics` returns `policy: undefined` with its
diagnostics.
