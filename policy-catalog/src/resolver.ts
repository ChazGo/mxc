// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import * as path from 'node:path';
import { PolicyCatalogError } from './errors.js';
import { nodeHostEnvironment, type HostEnvironment } from './host.js';
import { parsePurl, type ParsedPurl } from './purl.js';
import { satisfiesVersionRange } from './version-range.js';
import {
  ARCHITECTURES,
  COMPOSABLE_FILESYSTEM_FIELDS,
  IDENTITY_STRENGTH,
  PLATFORMS,
  caseKey,
  compositionViolation,
  dependencyClosure,
  findCrossClassOverlap,
  pathKeySegments,
  policySymbols,
  selectVariant,
  type AccessClass,
  type CatalogEntry,
  type ClosureNode,
  type IdentityPredicate,
  type VariantSelection,
} from './catalog.js';
import { bundledCatalogStore, type CatalogStore } from './store.js';
import type {
  CatalogArchitecture,
  CatalogEntryMetadata,
  CatalogInfo,
  CatalogPlatform,
  CatalogSandboxPolicy,
  ResolveContext,
  SandboxConfigResolution,
  ToolCandidate,
  ToolInput,
} from './types.js';

type Diagnostics = SandboxConfigResolution['diagnostics'];
type ToolRecord = Diagnostics['tools'][number];
type DependencyRecord = Diagnostics['resolvedDependencies'][number];

interface EntryMatch {
  entry: CatalogEntry;
  selection: VariantSelection;
  satisfied: IdentityPredicate[];
}

const SYMBOL_PATTERN = /\$\{([a-z][a-z0-9_]*)\}/g;

function pathApi(platform: CatalogPlatform): path.PlatformPath {
  return platform === 'windows' ? path.win32 : path.posix;
}

function normalizePath(value: string, platform: CatalogPlatform): string {
  const api = pathApi(platform);
  const normalized = api.normalize(value);
  const root = api.parse(normalized).root;
  return normalized.length > root.length ? normalized.replace(/[\\/]+$/, '') : normalized;
}

function compareStrings(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0;
}

function toCandidate(input: ToolInput, index: number): ToolCandidate {
  const candidate = typeof input === 'string' ? { invocationName: input } : input;
  if (candidate === null || typeof candidate !== 'object') {
    throw new PolicyCatalogError('invalid_context', `tool input ${index} must be a string or a ToolCandidate`);
  }
  if (typeof candidate.invocationName !== 'string' || candidate.invocationName.length === 0) {
    throw new PolicyCatalogError('invalid_context', `tool input ${index}: invocationName must be a non-empty string`);
  }
  if (/[\\/]/.test(candidate.invocationName)) {
    throw new PolicyCatalogError('invalid_context', `tool input ${index}: invocationName must be a bare name, not a path`);
  }
  for (const key of ['packageUrl', 'detectedVersion'] as const) {
    if (candidate[key] !== undefined && (typeof candidate[key] !== 'string' || candidate[key].length === 0)) {
      throw new PolicyCatalogError('invalid_context', `tool input ${index}: ${key} must be a non-empty string when present`);
    }
  }
  return candidate;
}

function describeInput(index: number, tool: ToolCandidate): string {
  return `input ${index} ('${tool.invocationName}')`;
}

/**
 * Read-only view over one catalog store. Runtime resolution and inspection are
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
  // Setup and inspection (design §5.2)
  // -------------------------------------------------------------------------

  getCatalogInfo(): CatalogInfo {
    const revision = this.store.revision();
    return { catalogSchemaVersion: revision.catalogSchemaVersion, catalogRevision: revision.catalogRevision };
  }

  /** Metadata for every entry in the installed revision, ordered by `entryId`. Never includes a policy body. */
  listCatalogEntries(): CatalogEntryMetadata[] {
    const revision = this.store.revision();
    return [...revision.entries]
      .sort((a, b) => compareStrings(a.entryId, b.entryId))
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
  // Runtime lookup (design §5.1)
  // -------------------------------------------------------------------------

  /** Returns the composed candidate policy, or `undefined` when no policy can be resolved. */
  getSandboxConfig(tools: ToolInput | readonly ToolInput[], ctx: ResolveContext = {}): CatalogSandboxPolicy | undefined {
    return this.resolve(tools, ctx).policy;
  }

  /**
   * Resolves one tool or an array of tools in one pass and returns the
   * composed candidate policy with attribution and warnings.
   *
   * `policy` is `undefined` for an empty input, when no input matched, or
   * when a selected entry requires an unresolved symbol. It is never an empty
   * stand-in policy. Library failures (integrity, unavailable revision,
   * invalid context, host detection, composition conflict) throw
   * {@link PolicyCatalogError}; they are never reported as absence.
   *
   * The result is a candidate lower bound, not authorization. Consumers keep
   * authorization, ceilings, persistence, composition, approval, and audit.
   */
  getSandboxConfigWithDiagnostics(tools: ToolInput | readonly ToolInput[], ctx: ResolveContext = {}): SandboxConfigResolution {
    return this.resolve(tools, ctx);
  }

  private resolve(tools: ToolInput | readonly ToolInput[], ctx: ResolveContext): SandboxConfigResolution {
    // A single input is exactly a one-element array (design §5.1).
    const inputs = (Array.isArray(tools) ? tools : [tools]) as readonly ToolInput[];
    const candidates = inputs.map(toCandidate);
    this.validateContext(ctx);
    const revision = this.store.revision(ctx.catalogRevision);
    const platform = ctx.platform ?? this.host.platform();
    const allowWeak = ctx.allowWeakIdentityFallback === true;

    // The native architecture is resolved lazily: a lookup that never needs
    // variant selection must not fail on host detection (design §4.4).
    let architecture = ctx.architecture;
    const effectiveArchitecture = (): CatalogArchitecture => {
      architecture ??= this.host.nativeArchitecture();
      return architecture;
    };

    const warnings: string[] = [];
    const toolRecords: ToolRecord[] = [];
    const selected = new Map<string, ClosureNode>();
    const byId = new Map(revision.entries.map(entry => [entry.entryId, entry]));
    // Entry order in the catalog file never selects or excludes a match.
    const ordered = [...revision.entries].sort((a, b) => compareStrings(a.entryId, b.entryId));

    candidates.forEach((tool, inputIndex) => {
      const matches = this.matchTool(ordered, tool, inputIndex, platform, allowWeak, effectiveArchitecture, warnings);
      toolRecords.push({
        inputIndex,
        matches: matches.map(match => ({
          entryId: match.entry.entryId,
          entryRevision: match.entry.entryRevision,
          matchedIdentities: match.satisfied.map(predicate => ({ kind: predicate.kind, strength: IDENTITY_STRENGTH[predicate.kind] })),
        })),
      });
      if (matches.length > 1) {
        warnings.push(`${describeInput(inputIndex, tool)} matched ${matches.length} entries (${matches.map(m => m.entry.entryId).join(', ')}); all contribute`);
      }
      for (const match of matches) {
        const closure = dependencyClosure(match.entry, match.selection, byId, platform, effectiveArchitecture());
        if (!closure.ok) {
          // A validated revision cannot reach this; treat it as corrupt data.
          throw new PolicyCatalogError('invalid_catalog', `dependency resolution failed: ${closure.reason} (${closure.detail})`);
        }
        for (const node of closure.nodes) {
          if (!selected.has(node.entry.entryId)) {
            selected.set(node.entry.entryId, node);
          }
        }
      }
    });

    const diagnostics: Diagnostics = {
      catalogRevision: revision.catalogRevision,
      tools: toolRecords,
      resolvedDependencies: dependencyRecords([...selected.values()], byId),
      warnings,
    };
    const nodes = [...selected.values()];
    if (nodes.length === 0) {
      return { policy: undefined, diagnostics };
    }

    if (ctx.architecture === undefined) {
      warnings.push(`architecture was not specified; variants were selected for the native system architecture '${effectiveArchitecture()}'; the tool's architecture was not verified`);
    }
    for (const node of nodes) {
      if (!node.exact) {
        warnings.push(`${node.entry.entryId} uses its architecture-neutral ${platform} variant; no ${effectiveArchitecture()}-specific variant exists`);
      }
    }

    const violation = compositionViolation(nodes);
    if (violation) {
      throw new PolicyCatalogError('composition_conflict', `selected entries cannot be composed: ${violation}`);
    }

    const symbols = this.resolveSymbols(nodes, ctx, platform, warnings);
    if (!symbols) {
      return { policy: undefined, diagnostics };
    }
    return { policy: composePolicy(nodes, symbols, platform), diagnostics };
  }

  /**
   * Collects every eligible entry for one input, ordered by `entryId`
   * (design §4.3). Matching is additive: a strong match never suppresses
   * another entry's eligible weak match.
   */
  private matchTool(
    ordered: readonly CatalogEntry[],
    tool: ToolCandidate,
    inputIndex: number,
    platform: CatalogPlatform,
    allowWeak: boolean,
    architecture: () => CatalogArchitecture,
    warnings: string[],
  ): EntryMatch[] {
    let purl: ParsedPurl | undefined;
    if (tool.packageUrl !== undefined) {
      purl = parsePurl(tool.packageUrl);
      if (!purl) {
        throw new PolicyCatalogError('invalid_context', `${describeInput(inputIndex, tool)}: '${tool.packageUrl}' is not a valid package URL`);
      }
    }
    // The same casing rule as path comparison (catalog.ts `foldsCase`).
    const invocation = caseKey(tool.invocationName, platform);

    const matches: EntryMatch[] = [];
    const skipped: string[] = [];
    for (const entry of ordered) {
      const satisfied = entry.identity.filter(predicate => (predicate.kind === 'purl'
        ? purl !== undefined && parsePurl(predicate.value)?.key === purl.key
        : predicate.names.some(name => caseKey(name, platform) === invocation)));
      if (satisfied.length === 0) {
        continue;
      }
      const strong = satisfied.some(predicate => IDENTITY_STRENGTH[predicate.kind] === 'strong');
      if (!strong && !allowWeak) {
        skipped.push(`${entry.entryId} matched only by invocation name and allowWeakIdentityFallback is not enabled`);
        continue;
      }
      const selection = selectVariant(entry, platform, architecture());
      if (!selection) {
        skipped.push(`${entry.entryId} has no variant for ${platform}/${architecture()}`);
        continue;
      }
      for (const predicate of satisfied) {
        if (predicate.kind !== 'purl' || predicate.versionRange === undefined) {
          continue;
        }
        const evidence = tool.detectedVersion ?? purl?.version;
        if (evidence === undefined) {
          continue;
        }
        const inRange = satisfiesVersionRange(evidence, predicate.versionRange);
        if (inRange !== true) {
          warnings.push(`${describeInput(inputIndex, tool)}: detected version '${evidence}' ${inRange === false ? 'is outside' : 'could not be compared with'} the reviewed range '${predicate.versionRange}' for ${entry.entryId}`);
        }
      }
      if (!strong) {
        warnings.push(`${describeInput(inputIndex, tool)} matched ${entry.entryId} only by invocation name (weak identity)`);
      }
      matches.push({ entry, selection, satisfied });
    }
    if (matches.length === 0) {
      warnings.push(`${describeInput(inputIndex, tool)} matched no eligible catalog entry${skipped.length > 0 ? `: ${skipped.join('; ')}` : ''}`);
    }
    return matches;
  }

  private validateContext(ctx: ResolveContext): void {
    if (ctx === null || typeof ctx !== 'object') {
      throw new PolicyCatalogError('invalid_context', 'ResolveContext must be an object');
    }
    if (ctx.platform !== undefined && !PLATFORMS.includes(ctx.platform)) {
      throw new PolicyCatalogError('invalid_context', `ResolveContext.platform '${String(ctx.platform)}' is unsupported`);
    }
    if (ctx.architecture !== undefined && !ARCHITECTURES.includes(ctx.architecture)) {
      throw new PolicyCatalogError('invalid_context', `ResolveContext.architecture '${String(ctx.architecture)}' is unsupported`);
    }
    if (ctx.projectRoot !== undefined && (typeof ctx.projectRoot !== 'string' || ctx.projectRoot.length === 0)) {
      throw new PolicyCatalogError('invalid_context', 'ResolveContext.projectRoot must be a non-empty string when present');
    }
    const contract = this.store.contract;
    for (const [name, value] of Object.entries(ctx.symbols ?? {})) {
      const definition = contract.symbols[name];
      if (!definition) {
        throw new PolicyCatalogError('invalid_context', `ResolveContext.symbols.${name} is not a catalog symbol`);
      }
      if (definition.source === 'context') {
        throw new PolicyCatalogError('invalid_context', `symbol '${name}' is supplied through ResolveContext.projectRoot, not symbols`);
      }
      if (typeof value !== 'string') {
        throw new PolicyCatalogError('invalid_context', `ResolveContext.symbols.${name} must be a string`);
      }
    }
  }

  /**
   * Resolves every symbol the selected entries need. Returns `undefined`
   * (with warnings) when any required symbol is unresolved: a selected
   * requirement is never silently dropped to produce a partial policy.
   */
  private resolveSymbols(
    nodes: readonly ClosureNode[],
    ctx: ResolveContext,
    platform: CatalogPlatform,
    warnings: string[],
  ): Map<string, string> | undefined {
    const contract = this.store.contract;
    const api = pathApi(platform);
    const values = new Map<string, string>();
    const missing = new Map<string, string[]>();
    for (const node of nodes) {
      for (const name of policySymbols(node.variant.sandboxPolicy)) {
        if (values.has(name)) {
          continue;
        }
        const definition = contract.symbols[name];
        let value: string | undefined;
        if (definition.source === 'context') {
          value = ctx.projectRoot;
        } else {
          value = ctx.symbols?.[name];
          // Host-derived symbols describe the current host only; they are
          // never derived for an explicitly different target platform.
          if (value === undefined && definition.source === 'host' && platform === this.host.platform()) {
            value = this.host.symbol(name);
          }
        }
        if (value === undefined) {
          missing.set(name, [...(missing.get(name) ?? []), node.entry.entryId]);
          continue;
        }
        if (value.includes('${') || !api.isAbsolute(value)) {
          throw new PolicyCatalogError('invalid_context', `symbol '${name}' must resolve to an absolute ${platform} path`);
        }
        values.set(name, value);
      }
    }
    if (missing.size > 0) {
      for (const [name, entryIds] of [...missing].sort(([a], [b]) => compareStrings(a, b))) {
        const hint = contract.symbols[name].source === 'context' ? 'ResolveContext.projectRoot' : `ResolveContext.symbols.${name}`;
        warnings.push(`required symbol '${name}' (needed by ${entryIds.join(', ')}) is unresolved; supply ${hint}; no policy was returned`);
      }
      return undefined;
    }
    return values;
  }
}

/**
 * Distinct dependency edges among the selected entries, ordered by entryId,
 * entryRevision, then requiredVersionRange (absent first) (design §5.1).
 */
function dependencyRecords(nodes: readonly ClosureNode[], byId: ReadonlyMap<string, CatalogEntry>): DependencyRecord[] {
  const records = new Map<string, DependencyRecord>();
  for (const node of nodes) {
    for (const dependency of node.variant.dependencies ?? []) {
      const target = byId.get(dependency.entryId)!;
      const key = JSON.stringify([target.entryId, target.entryRevision, dependency.versionRange ?? null]);
      records.set(key, {
        entryId: target.entryId,
        entryRevision: target.entryRevision,
        ...(dependency.versionRange !== undefined ? { requiredVersionRange: dependency.versionRange } : {}),
      });
    }
  }
  return [...records.values()].sort((a, b) => compareStrings(a.entryId, b.entryId)
    || a.entryRevision - b.entryRevision
    || (a.requiredVersionRange === undefined ? (b.requiredVersionRange === undefined ? 0 : -1)
      : b.requiredVersionRange === undefined ? 1 : compareStrings(a.requiredVersionRange, b.requiredVersionRange)));
}

/** Composes the v1 vocabulary (design §4.5). Callers have already checked `compositionViolation`. */
function composePolicy(
  nodes: readonly ClosureNode[],
  symbols: ReadonlyMap<string, string>,
  platform: CatalogPlatform,
): CatalogSandboxPolicy {
  const classes: Partial<Record<AccessClass, string[]>> = {};
  for (const field of COMPOSABLE_FILESYSTEM_FIELDS) {
    const seen = new Set<string>();
    const out: string[] = [];
    for (const node of nodes) {
      for (const template of node.variant.sandboxPolicy.filesystem?.[field] ?? []) {
        const resolved = normalizePath(template.replace(SYMBOL_PATTERN, (_match, name: string) => symbols.get(name)!), platform);
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
    throw new PolicyCatalogError('composition_conflict', `resolved paths overlap across access classes: ${overlap}`);
  }

  const root = nodes[0].variant.sandboxPolicy;
  const result: CatalogSandboxPolicy = { version: root.version };
  if (nodes.some(node => node.variant.sandboxPolicy.filesystem !== undefined)) {
    result.filesystem = {};
    for (const field of COMPOSABLE_FILESYSTEM_FIELDS) {
      if (classes[field]) {
        result.filesystem[field] = classes[field];
      }
    }
  }
  // compositionViolation guarantees these fields exist only when exactly one
  // entry is selected, so they are copied, never merged.
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

let defaultCatalog: PolicyCatalog | undefined;

function bundled(): PolicyCatalog {
  defaultCatalog ??= new PolicyCatalog(bundledCatalogStore());
  return defaultCatalog;
}

/** Composed candidate policy for one tool, from the bundled catalog. */
export function getSandboxConfig(tool: ToolInput, ctx?: ResolveContext): CatalogSandboxPolicy | undefined;
/** Composed candidate policy for several tools, from the bundled catalog. */
export function getSandboxConfig(tools: readonly ToolInput[], ctx?: ResolveContext): CatalogSandboxPolicy | undefined;
export function getSandboxConfig(tools: ToolInput | readonly ToolInput[], ctx?: ResolveContext): CatalogSandboxPolicy | undefined {
  return bundled().getSandboxConfig(tools, ctx);
}

/** Composed candidate policy plus attribution for one tool. See {@link PolicyCatalog.getSandboxConfigWithDiagnostics}. */
export function getSandboxConfigWithDiagnostics(tool: ToolInput, ctx?: ResolveContext): SandboxConfigResolution;
/** Composed candidate policy plus attribution for several tools. */
export function getSandboxConfigWithDiagnostics(tools: readonly ToolInput[], ctx?: ResolveContext): SandboxConfigResolution;
export function getSandboxConfigWithDiagnostics(tools: ToolInput | readonly ToolInput[], ctx?: ResolveContext): SandboxConfigResolution {
  return bundled().getSandboxConfigWithDiagnostics(tools, ctx);
}

/** Lists bundled catalog entry metadata. See {@link PolicyCatalog.listCatalogEntries}. */
export function listCatalogEntries(): CatalogEntryMetadata[] {
  return bundled().listCatalogEntries();
}

/** Reports the bundled catalog schema version and revision. */
export function getCatalogInfo(): CatalogInfo {
  return bundled().getCatalogInfo();
}
