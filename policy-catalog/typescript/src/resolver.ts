// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import * as os from 'node:os';
import * as path from 'node:path';
import { PolicyCatalogError } from './errors.js';
import { parsePurl } from './purl.js';
import { satisfiesVersionRange } from './version-range.js';
import {
  ARCHITECTURES,
  COMPOSABLE_FILESYSTEM_FIELDS,
  IDENTITY_STRENGTH,
  PLATFORMS,
  compositionViolation,
  dependencyClosure,
  findCrossClassOverlap,
  pathKeySegments,
  policySymbols,
  selectVariant,
  type AccessClass,
  type CatalogEntry,
  type CatalogRevision,
  type IdentityPredicate,
} from './catalog.js';
import { bundledCatalogStore, type CatalogStore } from './store.js';
import type {
  CatalogArchitecture,
  CatalogEntryMetadata,
  CatalogInfo,
  CatalogPlatform,
  CatalogSandboxPolicy,
  ResolveContext,
  ResolvedToolEntry,
  ToolCandidate,
} from './types.js';

/** Host facts the resolver may use when the caller omits them. */
export interface HostEnvironment {
  platform(): CatalogPlatform;
  architecture(): CatalogArchitecture;
  /** Approved host-known symbols (`source: "host"` in the contract) for the current host. */
  symbol(name: string): string | undefined;
}

function hostPlatform(): CatalogPlatform {
  switch (os.platform()) {
    case 'win32':
      return 'windows';
    case 'darwin':
      return 'macos';
    case 'linux':
      return 'linux';
    default:
      throw new PolicyCatalogError('unsupported-host', `host platform '${os.platform()}' has no catalog selector`);
  }
}

function hostArchitecture(): CatalogArchitecture {
  // Use the OS-reported machine type rather than the Node.js build
  // architecture (process.arch). Callers that know the tool's architecture
  // should pass ResolveContext.architecture explicitly.
  const machine = os.machine().toLowerCase();
  if (machine === 'x86_64' || machine === 'amd64' || machine === 'x64') {
    return 'x64';
  }
  if (machine === 'arm64' || machine === 'aarch64') {
    return 'arm64';
  }
  throw new PolicyCatalogError('unsupported-host', `host architecture '${machine}' has no catalog selector`);
}

/** Default host environment backed by Node.js `os`. */
export const nodeHostEnvironment: HostEnvironment = {
  platform: hostPlatform,
  architecture: hostArchitecture,
  symbol(name) {
    switch (name) {
      case 'user_home':
        return os.homedir() || undefined;
      case 'temp_dir':
        return os.tmpdir() || undefined;
      default:
        return undefined;
    }
  },
};

interface IdentityMatch {
  entry: CatalogEntry;
  predicate: IdentityPredicate;
  warnings: string[];
}

function pathApi(platform: CatalogPlatform): path.PlatformPath {
  return platform === 'windows' ? path.win32 : path.posix;
}

function normalizePath(value: string, platform: CatalogPlatform): string {
  const api = pathApi(platform);
  const normalized = api.normalize(value);
  const root = api.parse(normalized).root;
  return normalized.length > root.length ? normalized.replace(/[\\/]+$/, '') : normalized;
}

function validateContext(ctx: ResolveContext): void {
  if (ctx.platform !== undefined && !PLATFORMS.includes(ctx.platform)) {
    throw new PolicyCatalogError('invalid-context', `ResolveContext.platform '${String(ctx.platform)}' is unsupported`);
  }
  if (ctx.architecture !== undefined && !ARCHITECTURES.includes(ctx.architecture)) {
    throw new PolicyCatalogError('invalid-context', `ResolveContext.architecture '${String(ctx.architecture)}' is unsupported`);
  }
}

function validateTool(tool: ToolCandidate): void {
  if (typeof tool?.invocationName !== 'string' || tool.invocationName.length === 0) {
    throw new PolicyCatalogError('invalid-context', 'ToolCandidate.invocationName must be a non-empty string');
  }
  if (/[\\/]/.test(tool.invocationName)) {
    throw new PolicyCatalogError('invalid-context', 'ToolCandidate.invocationName must be a bare name, not a path');
  }
}

/**
 * Read-only view over one catalog store. Resolution and inspection are
 * separate operations; neither mutates catalog data or any consumer state.
 */
export class PolicyCatalog {
  private readonly store: CatalogStore;
  private readonly host: HostEnvironment;

  constructor(store: CatalogStore, host: HostEnvironment = nodeHostEnvironment) {
    this.store = store;
    this.host = host;
  }

  // -------------------------------------------------------------------------
  // Setup and inspection (spec §5.2)
  // -------------------------------------------------------------------------

  getCatalogInfo(): CatalogInfo {
    const revision = this.store.revision();
    return { catalogSchemaVersion: revision.catalogSchemaVersion, catalogRevision: revision.catalogRevision };
  }

  /** Metadata for every entry in the installed revision, ordered by `entryId`. Never includes a policy body. */
  listCatalogEntries(): CatalogEntryMetadata[] {
    const revision = this.store.revision();
    return [...revision.entries]
      .sort((a, b) => (a.entryId < b.entryId ? -1 : a.entryId > b.entryId ? 1 : 0))
      .map(entry => ({
        catalogRevision: revision.catalogRevision,
        entryId: entry.entryId,
        entryRevision: entry.entryRevision,
        displayName: entry.displayName,
        identity: entry.identity.map(predicate => (predicate.kind === 'purl'
          ? { kind: 'purl' as const, value: predicate.value, ...(predicate.versionRange !== undefined ? { versionRange: predicate.versionRange } : {}) }
          : { kind: 'invocation-name' as const, names: [...predicate.names] })),
        platformVariants: entry.platformVariants.map(variant => ({
          platform: variant.when.platform,
          ...(variant.when.architecture !== undefined ? { architecture: variant.when.architecture } : {}),
          dependencyEntryIds: (variant.dependencies ?? []).map(dependency => dependency.entryId),
          sandboxPolicyVersion: variant.sandboxPolicy.version,
        })),
        provenance: { ...entry.provenance },
      }));
  }

  // -------------------------------------------------------------------------
  // Runtime lookup (spec §5.1)
  // -------------------------------------------------------------------------

  /**
   * Resolves one tool's candidate minimum requirement.
   *
   * Returns `undefined` when there is no acceptable identity match, no
   * applicable platform variant, or a required symbol is unresolved. Throws
   * {@link PolicyCatalogError} for library failures (integrity, unavailable
   * revision, invalid context, composition conflict), which are never
   * reported as "no match".
   *
   * The result is a candidate lower bound, not authorization: consumers apply
   * their own authorization, ceilings, persistence, composition, and audit.
   */
  resolveCatalogEntry(tool: ToolCandidate, ctx: ResolveContext = {}): ResolvedToolEntry | undefined {
    validateTool(tool);
    validateContext(ctx);
    this.validateContextSymbols(ctx);
    const revision = this.store.revision(ctx.catalogRevision);

    const platform = ctx.platform ?? this.host.platform();
    const architecture = ctx.architecture ?? this.host.architecture();

    const match = this.matchIdentity(revision, tool, ctx.allowWeakIdentityFallback === true, platform);
    if (!match) {
      return undefined;
    }

    const selected = selectVariant(match.entry, platform, architecture);
    if (!selected) {
      return undefined;
    }

    const byId = new Map(revision.entries.map(entry => [entry.entryId, entry]));
    const closure = dependencyClosure(match.entry, selected.variant, byId, platform, architecture);
    if (!closure.ok) {
      // A validated revision cannot reach this; treat it as corrupt data.
      throw new PolicyCatalogError('validation', `dependency resolution failed: ${closure.reason} (${closure.detail})`);
    }
    const violation = compositionViolation(closure.nodes);
    if (violation) {
      throw new PolicyCatalogError('validation', `composition rejected: ${violation}`);
    }

    const symbols = this.resolveSymbols(closure.nodes.flatMap(node => policySymbols(node.variant.sandboxPolicy)), ctx, platform);
    if (!symbols) {
      return undefined;
    }

    const policy = this.composePolicy(closure.nodes.map(node => node.variant.sandboxPolicy), symbols, platform);

    return {
      entryId: match.entry.entryId,
      entryRevision: match.entry.entryRevision,
      catalogRevision: revision.catalogRevision,
      matchedIdentity: { kind: match.predicate.kind, strength: IDENTITY_STRENGTH[match.predicate.kind] },
      resolvedDependencies: closure.nodes.slice(1).map(node => ({
        entryId: node.entry.entryId,
        entryRevision: node.entry.entryRevision,
        ...(node.requiredVersionRange !== undefined ? { requiredVersionRange: node.requiredVersionRange } : {}),
      })),
      policy,
      warnings: match.warnings,
    };
  }

  private matchIdentity(
    revision: CatalogRevision,
    tool: ToolCandidate,
    allowWeak: boolean,
    platform: CatalogPlatform,
  ): IdentityMatch | undefined {
    const warnings: string[] = [];
    let strong: IdentityMatch | undefined;

    if (tool.packageUrl !== undefined) {
      const parsed = parsePurl(tool.packageUrl);
      if (!parsed) {
        throw new PolicyCatalogError('invalid-context', `ToolCandidate.packageUrl '${tool.packageUrl}' is not a valid package URL`);
      }
      for (const entry of revision.entries) {
        const predicate = entry.identity.find(p => p.kind === 'purl' && parsePurl(p.value)?.key === parsed.key);
        if (predicate) {
          strong = { entry, predicate, warnings };
          break;
        }
      }
      if (strong && strong.predicate.kind === 'purl' && strong.predicate.versionRange !== undefined) {
        const evidence = tool.detectedVersion ?? parsed.version;
        if (evidence !== undefined) {
          const satisfied = satisfiesVersionRange(evidence, strong.predicate.versionRange);
          if (satisfied === false) {
            warnings.push(`detected version '${evidence}' is outside the reviewed range '${strong.predicate.versionRange}' for ${strong.entry.entryId}`);
          } else if (satisfied === undefined) {
            warnings.push(`detected version '${evidence}' could not be compared with the reviewed range '${strong.predicate.versionRange}' for ${strong.entry.entryId}`);
          }
        }
      }
    }

    const weakEntry = this.matchInvocationName(revision, tool.invocationName, platform);
    if (strong) {
      if (weakEntry && weakEntry.entry.entryId !== strong.entry.entryId) {
        warnings.push(`invocation name '${tool.invocationName}' is listed by ${weakEntry.entry.entryId}; the package URL match ${strong.entry.entryId} takes precedence`);
      }
      return strong;
    }
    if (!allowWeak || !weakEntry) {
      return undefined;
    }
    if (tool.packageUrl !== undefined) {
      warnings.push(`package URL '${tool.packageUrl}' matched no entry; fell back to invocation-name identity`);
    }
    warnings.push(`${weakEntry.entry.entryId} matched only by invocation name '${tool.invocationName}' (weak identity)`);
    return { ...weakEntry, warnings };
  }

  private matchInvocationName(
    revision: CatalogRevision,
    name: string,
    platform: CatalogPlatform,
  ): Omit<IdentityMatch, 'warnings'> | undefined {
    // Windows and default macOS volumes resolve executables case-insensitively;
    // Linux does not. Validation guarantees case-insensitive uniqueness across
    // entries, so either comparison selects at most one entry.
    const fold = platform === 'linux' ? (value: string) => value : (value: string) => value.toLowerCase();
    const key = fold(name);
    for (const entry of revision.entries) {
      const predicate = entry.identity.find(p => p.kind === 'invocation-name' && p.names.some(n => fold(n) === key));
      if (predicate) {
        return { entry, predicate };
      }
    }
    return undefined;
  }

  /** Caller symbol names are checked before matching so an invalid context is never reported as no-match. */
  private validateContextSymbols(ctx: ResolveContext): void {
    const contract = this.store.contract;
    if (ctx.symbols !== undefined) {
      for (const name of Object.keys(ctx.symbols)) {
        const definition = contract.symbols[name];
        if (!definition) {
          throw new PolicyCatalogError('invalid-context', `ResolveContext.symbols.${name} is not a catalog symbol`);
        }
        if (definition.source === 'context') {
          throw new PolicyCatalogError('invalid-context', `symbol '${name}' is supplied through ResolveContext.projectRoot, not symbols`);
        }
      }
    }
  }

  private resolveSymbols(names: string[], ctx: ResolveContext, platform: CatalogPlatform): Map<string, string> | undefined {
    const contract = this.store.contract;
    const api = pathApi(platform);
    const values = new Map<string, string>();
    for (const name of new Set(names)) {
      const definition = contract.symbols[name];
      let value: string | undefined;
      if (definition.source === 'context') {
        value = ctx.projectRoot;
      } else {
        value = ctx.symbols?.[name];
        // Host-derived symbols are only meaningful for the current host; they
        // are never derived for an explicitly different target platform.
        if (value === undefined && definition.source === 'host' && platform === this.host.platform()) {
          value = this.host.symbol(name);
        }
      }
      if (value === undefined) {
        return undefined;
      }
      if (value.includes('${') || !api.isAbsolute(value)) {
        throw new PolicyCatalogError('invalid-context', `symbol '${name}' must resolve to an absolute ${platform} path`);
      }
      values.set(name, value);
    }
    return values;
  }

  private composePolicy(
    policies: CatalogSandboxPolicy[],
    symbols: ReadonlyMap<string, string>,
    platform: CatalogPlatform,
  ): CatalogSandboxPolicy {
    const classes: Partial<Record<AccessClass, string[]>> = {};
    for (const field of COMPOSABLE_FILESYSTEM_FIELDS) {
      const seen = new Set<string>();
      const out: string[] = [];
      for (const policy of policies) {
        for (const template of policy.filesystem?.[field] ?? []) {
          const resolved = normalizePath(
            template.replace(/\$\{([a-z][a-z0-9_]*)\}/g, (_m, name: string) => symbols.get(name)!),
            platform,
          );
          const key = pathKeySegments(resolved, platform).join('\u0000');
          if (!seen.has(key)) {
            seen.add(key);
            out.push(resolved);
          }
        }
      }
      if (out.length > 0) {
        classes[field] = out;
      }
    }
    const overlap = findCrossClassOverlap(classes, platform);
    if (overlap) {
      // Symbol values can make distinct templates collide. The resolver never
      // chooses an access class implicitly.
      throw new PolicyCatalogError('composition-conflict', `resolved paths overlap across access classes: ${overlap}`);
    }

    const root = policies[0];
    const result: CatalogSandboxPolicy = { version: root.version };
    if (root.filesystem !== undefined || Object.keys(classes).length > 0) {
      result.filesystem = {};
      for (const field of COMPOSABLE_FILESYSTEM_FIELDS) {
        if (classes[field]) {
          result.filesystem[field] = classes[field];
        }
      }
    }
    // Composition validation guarantees these fields only exist on a
    // single-entry closure, so they are copied, never merged.
    if (root.network !== undefined) {
      result.network = structuredClone(root.network);
    }
    if (root.ui !== undefined) {
      result.ui = structuredClone(root.ui);
    }
    if (root.timeoutMs !== undefined) {
      result.timeoutMs = root.timeoutMs;
    }
    return result;
  }
}

let defaultCatalog: PolicyCatalog | undefined;

function bundled(): PolicyCatalog {
  defaultCatalog ??= new PolicyCatalog(bundledCatalogStore());
  return defaultCatalog;
}

/** Resolves one tool against the bundled catalog. See {@link PolicyCatalog.resolveCatalogEntry}. */
export function resolveCatalogEntry(tool: ToolCandidate, ctx?: ResolveContext): ResolvedToolEntry | undefined {
  return bundled().resolveCatalogEntry(tool, ctx);
}

/** Lists bundled catalog entry metadata. See {@link PolicyCatalog.listCatalogEntries}. */
export function listCatalogEntries(): CatalogEntryMetadata[] {
  return bundled().listCatalogEntries();
}

/** Reports the bundled catalog schema version and revision. */
export function getCatalogInfo(): CatalogInfo {
  return bundled().getCatalogInfo();
}
