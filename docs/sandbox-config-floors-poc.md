# Sandbox config floors proof of concept

This experimental SDK feature is the first reviewable implementation seam for
the config-floor proposal in
[microsoft/mxc#779](https://github.com/microsoft/mxc/pull/779). It provides a
small MXC-owned, repository-reviewed catalog and a read-only TypeScript lookup:

```typescript
getSandboxConfigForTool(
  tools: readonly string[],
  context?: ResolveContext,
): SandboxPolicy | undefined
```

Catalog entries embed the existing `SandboxPolicy` vocabulary. Symbols exist
only in catalog source; the resolver returns a literal policy. The proof includes
`git`, `node`, and `npm`, with `npm` depending on `node`. Resolution supports
multiple requested tools, transitive dependency closure, cycle safety,
deterministic permission union, and project/npm symbol resolution. An all-unknown
lookup returns `undefined`, while a mixed lookup returns the known requirements.

## Ownership and security boundary

Config floors describe minimum access a tool likely needs. They are compatibility
input, not authorization, and they are not a Microsoft certification of the tool
or its requirements. Invocation names are lookup hints, not trustworthy tool
identity. Runtime consumers are read-only; catalog changes happen through normal
repository review.

The resolver does not accept, inspect, mutate, merge with, or override a
host-authored policy. Hosts such as OpenClaw own writable user, learned, and
invocation-specific state; approval; restrictive policy composition; and final
enforcement under non-overridable OS, enterprise, and device constraints.

## Proof-of-concept limits

The catalog schema deliberately accepts only the currently exercised
`SandboxPolicy` subset and rejects unsupported fields rather than silently
dropping them. All seed entries use one policy version; cross-version policy
migration is deferred. Tool discovery is limited to executable directories on
`PATH`; npm cache and registry discovery uses environment settings plus
platform defaults rather than executing npm or fully parsing its config chain.

Production work remains for:

- stronger tool identity beyond invocation names;
- signed or otherwise integrity-protected feeds;
- dynamic distribution, versioning, rollback, and revocation;
- private and enterprise catalog overlays;
- catalog ownership, review, and governance policy;
- broader cross-platform symbol resolution and floor authoring;
- Rust and C# SDK parity;
- safe cross-version policy migration and shared composition semantics;
- any future Microsoft certification program.
