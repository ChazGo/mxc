// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { PolicyCatalogError } from './errors.js';
import { isValidVersionRange } from './version-range.js';
import { parsePurl } from './purl.js';
import type {
  CatalogArchitecture,
  CatalogPlatform,
  CatalogSandboxPolicy,
} from './types.js';

export const CATALOG_SCHEMA_VERSION = '1';
export const PLATFORMS: readonly CatalogPlatform[] = ['windows', 'linux', 'macos'];
export const ARCHITECTURES: readonly CatalogArchitecture[] = ['x64', 'arm64'];

/** Fields that the v1 contract can compose across entries (spec §4.5). */
export const COMPOSABLE_FILESYSTEM_FIELDS = ['deniedPaths', 'readonlyPaths', 'readwritePaths'] as const;
export type AccessClass = (typeof COMPOSABLE_FILESYSTEM_FIELDS)[number];

export type SymbolSource = 'context' | 'caller' | 'host';

export interface CatalogContract {
  catalogSchemaVersion: string;
  sandboxPolicyVersions: string[];
  symbols: Record<string, { source: SymbolSource; description: string }>;
}

export type IdentityPredicate =
  | { kind: 'purl'; value: string; versionRange?: string }
  | { kind: 'invocation-name'; names: string[] };

export interface PlatformVariant {
  when: { platform: CatalogPlatform; architecture?: CatalogArchitecture };
  dependencies?: Array<{ entryId: string; versionRange?: string }>;
  sandboxPolicy: CatalogSandboxPolicy;
}

export interface CatalogEntry {
  entryId: string;
  entryRevision: number;
  displayName: string;
  identity: IdentityPredicate[];
  platformVariants: PlatformVariant[];
  provenance: { method: string; sourceRevision: string };
}

export interface CatalogRevision {
  catalogSchemaVersion: string;
  catalogRevision: string;
  entries: CatalogEntry[];
}

/** Strength of an identity predicate kind. Order in this map is strongest first. */
export const IDENTITY_STRENGTH: Record<IdentityPredicate['kind'], 'strong' | 'weak'> = {
  purl: 'strong',
  'invocation-name': 'weak',
};

const REVISION_PATTERN = /^(\d{4}-\d{2}-\d{2})\.([1-9]\d*)$/;
const ENTRY_ID_PATTERN = /^[a-z][a-z0-9-]*:[a-z0-9][a-z0-9._-]*$/;
const SYMBOL_PATTERN = /\$\{([a-z][a-z0-9_]*)\}/g;
const ANCHORED_SYMBOL = /^\$\{([a-z][a-z0-9_]*)\}(?:[\\/]|$)/;

function fail(message: string): never {
  throw new PolicyCatalogError('validation', message);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function onlyFields(value: Record<string, unknown>, allowed: readonly string[], at: string): void {
  for (const key of Object.keys(value)) {
    if (!allowed.includes(key)) {
      fail(`unsupported field '${at}.${key}'`);
    }
  }
}

function nonEmptyString(value: unknown, at: string): string {
  if (typeof value !== 'string' || value.length === 0) {
    fail(`'${at}' must be a non-empty string`);
  }
  return value;
}

function stringArray(value: unknown, at: string, minItems = 0): string[] {
  if (!Array.isArray(value) || value.length < minItems) {
    fail(`'${at}' must be an array with at least ${minItems} item(s)`);
  }
  return value.map((item, index) => nonEmptyString(item, `${at}[${index}]`));
}

/** Compares two catalog revision identifiers (`YYYY-MM-DD.N`). */
export function compareCatalogRevisions(left: string, right: string): number {
  const a = REVISION_PATTERN.exec(left);
  const b = REVISION_PATTERN.exec(right);
  if (!a || !b) {
    fail(`cannot compare malformed catalog revisions '${left}' and '${right}'`);
  }
  if (a[1] !== b[1]) {
    return a[1] < b[1] ? -1 : 1;
  }
  return Math.sign(Number(a[2]) - Number(b[2]));
}

export function isCatalogRevisionId(value: string): boolean {
  return REVISION_PATTERN.test(value);
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

export function validateContract(raw: unknown): CatalogContract {
  if (!isRecord(raw)) {
    fail('contract root must be an object');
  }
  onlyFields(raw, ['$comment', 'catalogSchemaVersion', 'sandboxPolicyVersions', 'symbols'], 'contract');
  if (raw.catalogSchemaVersion !== CATALOG_SCHEMA_VERSION) {
    fail(`contract.catalogSchemaVersion must be '${CATALOG_SCHEMA_VERSION}'`);
  }
  const sandboxPolicyVersions = stringArray(raw.sandboxPolicyVersions, 'contract.sandboxPolicyVersions', 1);
  if (!isRecord(raw.symbols)) {
    fail('contract.symbols must be an object');
  }
  const symbols: CatalogContract['symbols'] = {};
  for (const [name, definition] of Object.entries(raw.symbols)) {
    if (!/^[a-z][a-z0-9_]*$/.test(name) || !isRecord(definition)) {
      fail(`contract.symbols.${name} is malformed`);
    }
    onlyFields(definition, ['source', 'description'], `contract.symbols.${name}`);
    if (definition.source !== 'context' && definition.source !== 'caller' && definition.source !== 'host') {
      fail(`contract.symbols.${name}.source is unsupported`);
    }
    symbols[name] = {
      source: definition.source,
      description: nonEmptyString(definition.description, `contract.symbols.${name}.description`),
    };
  }
  return { catalogSchemaVersion: CATALOG_SCHEMA_VERSION, sandboxPolicyVersions, symbols };
}

// ---------------------------------------------------------------------------
// Paths and symbols
// ---------------------------------------------------------------------------

function validateTemplatePath(value: string, at: string, contract: CatalogContract): void {
  if (/[*?]/.test(value)) {
    fail(`'${at}' contains a wildcard`);
  }
  if (value.replace(SYMBOL_PATTERN, '').includes('${')) {
    fail(`'${at}' contains malformed symbol syntax`);
  }
  for (const match of value.matchAll(SYMBOL_PATTERN)) {
    if (!(match[1] in contract.symbols)) {
      fail(`'${at}' references unknown symbol '${match[1]}'`);
    }
  }
  // Catalog data never ships a literal, machine-specific path (spec §4.2):
  // every filesystem requirement must be anchored at a declared symbol.
  if (!ANCHORED_SYMBOL.test(value)) {
    fail(`'${at}' must start with a declared symbol; literal paths are not allowed`);
  }
  if (value.split(/[\\/]/).some(segment => segment === '..')) {
    fail(`'${at}' must not contain '..' segments`);
  }
}

/** Returns the symbols a policy references, in first-seen order. */
export function policySymbols(policy: CatalogSandboxPolicy): string[] {
  const seen = new Set<string>();
  for (const field of COMPOSABLE_FILESYSTEM_FIELDS) {
    for (const value of policy.filesystem?.[field] ?? []) {
      for (const match of value.matchAll(SYMBOL_PATTERN)) {
        seen.add(match[1]);
      }
    }
  }
  return [...seen];
}

/** Splits a path into comparable segments using the platform's path rules. */
export function pathKeySegments(value: string, platform: CatalogPlatform): string[] {
  const separators = platform === 'windows' ? /[\\/]+/ : /\/+/;
  const segments = value.split(separators).filter((segment, index) => segment !== '.' && (segment !== '' || index === 0));
  const normalized = segments.length > 1 && segments[segments.length - 1] === '' ? segments.slice(0, -1) : segments;
  return platform === 'windows' ? normalized.map(segment => segment.toLowerCase()) : normalized;
}

function isSameOrNested(left: string[], right: string[]): boolean {
  const shorter = left.length <= right.length ? left : right;
  const longer = shorter === left ? right : left;
  return shorter.every((segment, index) => longer[index] === segment);
}

/**
 * Finds the first equal or ancestor/descendant pair of paths that appear in
 * different access classes. Returns a description or `undefined`.
 */
export function findCrossClassOverlap(
  classes: Partial<Record<AccessClass, readonly string[]>>,
  platform: CatalogPlatform,
): string | undefined {
  const fields = COMPOSABLE_FILESYSTEM_FIELDS.filter(field => (classes[field]?.length ?? 0) > 0);
  for (let i = 0; i < fields.length; i += 1) {
    for (let j = i + 1; j < fields.length; j += 1) {
      for (const left of classes[fields[i]] ?? []) {
        for (const right of classes[fields[j]] ?? []) {
          if (isSameOrNested(pathKeySegments(left, platform), pathKeySegments(right, platform))) {
            return `'${left}' (${fields[i]}) overlaps '${right}' (${fields[j]})`;
          }
        }
      }
    }
  }
  return undefined;
}

// ---------------------------------------------------------------------------
// Embedded SandboxPolicy (catalog-supported subset)
// ---------------------------------------------------------------------------

const BACKEND_KEYS = ['containment', 'processContainer', 'appContainer', 'lxc', 'seatbelt', 'wslc', 'hyperlight', 'bwrap'];

function validateNetworkRules(value: unknown, at: string, denyList: boolean): void {
  if (!Array.isArray(value)) {
    fail(`'${at}' must be an array`);
  }
  value.forEach((rule, index) => {
    const ruleAt = `${at}[${index}]`;
    if (!isRecord(rule)) {
      fail(`'${ruleAt}' must be an object`);
    }
    onlyFields(rule, ['to', 'ports'], ruleAt);
    if (rule.to === undefined && !denyList) {
      fail(`'${ruleAt}' has no 'to'; wildcard network grants are not allowed`);
    }
    if (rule.to !== undefined) {
      if (!Array.isArray(rule.to) || rule.to.length === 0) {
        fail(`'${ruleAt}.to' must be a non-empty array`);
      }
      rule.to.forEach((peer, peerIndex) => {
        const peerAt = `${ruleAt}.to[${peerIndex}]`;
        if (!isRecord(peer)) {
          fail(`'${peerAt}' must be an object`);
        }
        onlyFields(peer, ['cidr', 'except'], peerAt);
        const cidr = nonEmptyString(peer.cidr, `${peerAt}.cidr`);
        if (!denyList && /\/0$/.test(cidr)) {
          fail(`'${peerAt}.cidr' is a wildcard network grant`);
        }
        if (peer.except !== undefined) {
          stringArray(peer.except, `${peerAt}.except`);
        }
      });
    }
    if (rule.ports !== undefined) {
      if (!Array.isArray(rule.ports) || rule.ports.length === 0) {
        fail(`'${ruleAt}.ports' must be a non-empty array`);
      }
      rule.ports.forEach((port, portIndex) => {
        const portAt = `${ruleAt}.ports[${portIndex}]`;
        if (!isRecord(port)) {
          fail(`'${portAt}' must be an object`);
        }
        onlyFields(port, ['protocol', 'port', 'endPort'], portAt);
        if (port.protocol !== undefined && !['tcp', 'udp', 'icmp', 'any'].includes(port.protocol as string)) {
          fail(`'${portAt}.protocol' is unsupported`);
        }
        for (const key of ['port', 'endPort'] as const) {
          const n = port[key];
          if (n !== undefined && (typeof n !== 'number' || !Number.isInteger(n) || n < 1 || n > 65535)) {
            fail(`'${portAt}.${key}' must be an integer in 1..65535`);
          }
        }
        if (port.endPort !== undefined && (port.port === undefined || (port.endPort as number) < (port.port as number))) {
          fail(`'${portAt}.endPort' requires a lower or equal 'port'`);
        }
      });
    }
  });
}

function validateSandboxPolicy(raw: unknown, at: string, contract: CatalogContract): CatalogSandboxPolicy {
  if (!isRecord(raw)) {
    fail(`'${at}' must be an object`);
  }
  for (const key of BACKEND_KEYS) {
    if (key in raw) {
      fail(`'${at}.${key}' names a containment backend; platform variants must stay backend-neutral`);
    }
  }
  onlyFields(raw, ['version', 'filesystem', 'network', 'ui', 'timeoutMs'], at);
  const version = nonEmptyString(raw.version, `${at}.version`);
  if (!contract.sandboxPolicyVersions.includes(version)) {
    fail(`'${at}.version' '${version}' is not a SandboxPolicy version registered in the catalog contract`);
  }
  if (raw.filesystem !== undefined) {
    if (!isRecord(raw.filesystem)) {
      fail(`'${at}.filesystem' must be an object`);
    }
    onlyFields(raw.filesystem, COMPOSABLE_FILESYSTEM_FIELDS, `${at}.filesystem`);
    for (const field of COMPOSABLE_FILESYSTEM_FIELDS) {
      if (raw.filesystem[field] !== undefined) {
        stringArray(raw.filesystem[field], `${at}.filesystem.${field}`).forEach((value, index) =>
          validateTemplatePath(value, `${at}.filesystem.${field}[${index}]`, contract));
      }
    }
  }
  if (raw.network !== undefined) {
    if (!isRecord(raw.network)) {
      fail(`'${at}.network' must be an object`);
    }
    onlyFields(raw.network, ['egress', 'ingress'], `${at}.network`);
    const { egress, ingress } = raw.network;
    if (egress !== undefined) {
      if (!isRecord(egress)) {
        fail(`'${at}.network.egress' must be an object`);
      }
      onlyFields(egress, ['default', 'allow', 'deny'], `${at}.network.egress`);
      if (egress.default !== undefined && egress.default !== 'deny') {
        fail(`'${at}.network.egress.default' must be 'deny'; a default-allow grant is a wildcard`);
      }
      if (egress.allow !== undefined) {
        validateNetworkRules(egress.allow, `${at}.network.egress.allow`, false);
      }
      if (egress.deny !== undefined) {
        validateNetworkRules(egress.deny, `${at}.network.egress.deny`, true);
      }
    }
    if (ingress !== undefined) {
      if (!isRecord(ingress)) {
        fail(`'${at}.network.ingress' must be an object`);
      }
      onlyFields(ingress, ['default', 'hostLoopback'], `${at}.network.ingress`);
      if (ingress.default !== undefined && ingress.default !== 'deny') {
        fail(`'${at}.network.ingress.default' must be 'deny'; a default-allow grant is a wildcard`);
      }
      if (ingress.hostLoopback !== undefined && ingress.hostLoopback !== 'allow' && ingress.hostLoopback !== 'deny') {
        fail(`'${at}.network.ingress.hostLoopback' is unsupported`);
      }
    }
  }
  if (raw.ui !== undefined) {
    if (!isRecord(raw.ui)) {
      fail(`'${at}.ui' must be an object`);
    }
    onlyFields(raw.ui, ['allowWindows', 'clipboard', 'allowInputInjection'], `${at}.ui`);
    for (const key of ['allowWindows', 'allowInputInjection'] as const) {
      if (raw.ui[key] !== undefined && typeof raw.ui[key] !== 'boolean') {
        fail(`'${at}.ui.${key}' must be a boolean`);
      }
    }
    if (raw.ui.clipboard !== undefined && !['none', 'read', 'write', 'all'].includes(raw.ui.clipboard as string)) {
      fail(`'${at}.ui.clipboard' is unsupported`);
    }
  }
  if (raw.timeoutMs !== undefined
    && (typeof raw.timeoutMs !== 'number' || !Number.isInteger(raw.timeoutMs) || raw.timeoutMs < 1)) {
    fail(`'${at}.timeoutMs' must be a positive integer`);
  }
  return raw as unknown as CatalogSandboxPolicy;
}

// ---------------------------------------------------------------------------
// Entries
// ---------------------------------------------------------------------------

function validateIdentity(raw: unknown, at: string): IdentityPredicate[] {
  if (!Array.isArray(raw) || raw.length === 0) {
    fail(`'${at}' must be a non-empty array`);
  }
  const predicates = raw.map((item, index): IdentityPredicate => {
    const itemAt = `${at}[${index}]`;
    if (!isRecord(item)) {
      fail(`'${itemAt}' must be an object`);
    }
    if (item.kind === 'purl') {
      onlyFields(item, ['kind', 'value', 'versionRange'], itemAt);
      const value = nonEmptyString(item.value, `${itemAt}.value`);
      const parsed = parsePurl(value);
      if (!parsed) {
        fail(`'${itemAt}.value' is not a valid package URL`);
      }
      if (parsed.version !== undefined) {
        fail(`'${itemAt}.value' must not pin a version; use 'versionRange'`);
      }
      if (item.versionRange !== undefined && !isValidVersionRange(nonEmptyString(item.versionRange, `${itemAt}.versionRange`))) {
        fail(`'${itemAt}.versionRange' is not a valid version range`);
      }
      return { kind: 'purl', value, ...(item.versionRange !== undefined ? { versionRange: item.versionRange as string } : {}) };
    }
    if (item.kind === 'invocation-name') {
      onlyFields(item, ['kind', 'names'], itemAt);
      const names = stringArray(item.names, `${itemAt}.names`, 1);
      for (const name of names) {
        if (/[\\/]/.test(name)) {
          fail(`'${itemAt}.names' entry '${name}' must be a bare invocation name, not a path`);
        }
      }
      return { kind: 'invocation-name', names };
    }
    fail(`'${itemAt}.kind' is not a supported identity kind`);
  });
  // identity is ordered strongest to weakest (spec §4.3).
  let sawWeak = false;
  predicates.forEach((predicate, index) => {
    const weak = IDENTITY_STRENGTH[predicate.kind] === 'weak';
    if (!weak && sawWeak) {
      fail(`'${at}[${index}]' is a strong predicate after a weak one; identity must be ordered strongest first`);
    }
    sawWeak ||= weak;
  });
  return predicates;
}

function validateVariants(raw: unknown, at: string, contract: CatalogContract): PlatformVariant[] {
  if (!Array.isArray(raw) || raw.length === 0) {
    fail(`'${at}' must be a non-empty array`);
  }
  const selectors = new Set<string>();
  return raw.map((item, index): PlatformVariant => {
    const itemAt = `${at}[${index}]`;
    if (!isRecord(item)) {
      fail(`'${itemAt}' must be an object`);
    }
    onlyFields(item, ['when', 'dependencies', 'sandboxPolicy'], itemAt);
    if (!isRecord(item.when)) {
      fail(`'${itemAt}.when' must be an object`);
    }
    onlyFields(item.when, ['platform', 'architecture'], `${itemAt}.when`);
    const platform = item.when.platform as CatalogPlatform;
    if (!PLATFORMS.includes(platform)) {
      fail(`'${itemAt}.when.platform' must be one of ${PLATFORMS.join(', ')}`);
    }
    const architecture = item.when.architecture as CatalogArchitecture | undefined;
    if (architecture !== undefined && !ARCHITECTURES.includes(architecture)) {
      fail(`'${itemAt}.when.architecture' must be one of ${ARCHITECTURES.join(', ')}`);
    }
    const selector = `${platform}/${architecture ?? '*'}`;
    if (selectors.has(selector)) {
      fail(architecture === undefined
        ? `'${itemAt}' is a second architecture-neutral variant for '${platform}'`
        : `'${itemAt}' duplicates selector '${selector}'`);
    }
    selectors.add(selector);
    let dependencies: PlatformVariant['dependencies'];
    if (item.dependencies !== undefined) {
      if (!Array.isArray(item.dependencies)) {
        fail(`'${itemAt}.dependencies' must be an array`);
      }
      const seen = new Set<string>();
      dependencies = item.dependencies.map((dependency, depIndex) => {
        const depAt = `${itemAt}.dependencies[${depIndex}]`;
        if (!isRecord(dependency)) {
          fail(`'${depAt}' must be an object`);
        }
        onlyFields(dependency, ['entryId', 'versionRange'], depAt);
        const entryId = nonEmptyString(dependency.entryId, `${depAt}.entryId`);
        if (seen.has(entryId)) {
          fail(`'${depAt}.entryId' '${entryId}' is listed twice`);
        }
        seen.add(entryId);
        if (dependency.versionRange !== undefined
          && !isValidVersionRange(nonEmptyString(dependency.versionRange, `${depAt}.versionRange`))) {
          fail(`'${depAt}.versionRange' is not a valid version range`);
        }
        return { entryId, ...(dependency.versionRange !== undefined ? { versionRange: dependency.versionRange as string } : {}) };
      });
    }
    return {
      when: { platform, ...(architecture !== undefined ? { architecture } : {}) },
      ...(dependencies !== undefined ? { dependencies } : {}),
      sandboxPolicy: validateSandboxPolicy(item.sandboxPolicy, `${itemAt}.sandboxPolicy`, contract),
    };
  });
}

function validateEntry(raw: unknown, at: string, contract: CatalogContract): CatalogEntry {
  if (!isRecord(raw)) {
    fail(`'${at}' must be an object`);
  }
  onlyFields(raw, ['entryId', 'entryRevision', 'displayName', 'identity', 'platformVariants', 'provenance'], at);
  const entryId = nonEmptyString(raw.entryId, `${at}.entryId`);
  if (!ENTRY_ID_PATTERN.test(entryId)) {
    fail(`'${at}.entryId' '${entryId}' must be namespaced, e.g. 'tool:name'`);
  }
  if (typeof raw.entryRevision !== 'number' || !Number.isInteger(raw.entryRevision) || raw.entryRevision < 1) {
    fail(`'${at}.entryRevision' must be a positive integer`);
  }
  if (!isRecord(raw.provenance)) {
    fail(`'${at}.provenance' must be an object`);
  }
  onlyFields(raw.provenance, ['method', 'sourceRevision'], `${at}.provenance`);
  return {
    entryId,
    entryRevision: raw.entryRevision,
    displayName: nonEmptyString(raw.displayName, `${at}.displayName`),
    identity: validateIdentity(raw.identity, `${at}.identity`),
    platformVariants: validateVariants(raw.platformVariants, `${at}.platformVariants`, contract),
    provenance: {
      method: nonEmptyString(raw.provenance.method, `${at}.provenance.method`),
      sourceRevision: nonEmptyString(raw.provenance.sourceRevision, `${at}.provenance.sourceRevision`),
    },
  };
}

// ---------------------------------------------------------------------------
// Variant selection and dependency closure (shared by validation and resolver)
// ---------------------------------------------------------------------------

export interface VariantSelection {
  variant: PlatformVariant;
  exact: boolean;
}

/** Selects the variant for a host: exact architecture first, then neutral (spec §4.4). */
export function selectVariant(
  entry: CatalogEntry,
  platform: CatalogPlatform,
  architecture: CatalogArchitecture,
): VariantSelection | undefined {
  const forPlatform = entry.platformVariants.filter(variant => variant.when.platform === platform);
  const exact = forPlatform.find(variant => variant.when.architecture === architecture);
  if (exact) {
    return { variant: exact, exact: true };
  }
  const neutral = forPlatform.find(variant => variant.when.architecture === undefined);
  return neutral ? { variant: neutral, exact: false } : undefined;
}

export interface ClosureNode {
  entry: CatalogEntry;
  variant: PlatformVariant;
  /** Range from the first edge that reached this node, in traversal order. */
  requiredVersionRange?: string;
}

export type ClosureResult =
  | { ok: true; nodes: ClosureNode[] }
  | { ok: false; reason: 'cycle' | 'missing-entry' | 'unsupported-dependency'; detail: string };

/**
 * Deterministic depth-first dependency closure. The root is first; each
 * dependency follows in declaration order and appears once. Cycles are
 * reported, never silently broken.
 */
export function dependencyClosure(
  root: CatalogEntry,
  rootVariant: PlatformVariant,
  byId: ReadonlyMap<string, CatalogEntry>,
  platform: CatalogPlatform,
  architecture: CatalogArchitecture,
): ClosureResult {
  const nodes: ClosureNode[] = [];
  const done = new Set<string>();
  const stack: string[] = [];
  let failure: ClosureResult | undefined;

  const visit = (entry: CatalogEntry, variant: PlatformVariant, range: string | undefined): void => {
    if (failure) {
      return;
    }
    if (stack.includes(entry.entryId)) {
      failure = { ok: false, reason: 'cycle', detail: [...stack, entry.entryId].join(' -> ') };
      return;
    }
    if (done.has(entry.entryId)) {
      return;
    }
    stack.push(entry.entryId);
    nodes.push({ entry, variant, ...(range !== undefined ? { requiredVersionRange: range } : {}) });
    for (const dependency of variant.dependencies ?? []) {
      const target = byId.get(dependency.entryId);
      if (!target) {
        failure = { ok: false, reason: 'missing-entry', detail: `${entry.entryId} -> ${dependency.entryId}` };
        return;
      }
      const selected = selectVariant(target, platform, architecture);
      if (!selected) {
        failure = {
          ok: false,
          reason: 'unsupported-dependency',
          detail: `${entry.entryId} -> ${dependency.entryId} has no ${platform}/${architecture} variant`,
        };
        return;
      }
      visit(target, selected.variant, dependency.versionRange);
    }
    stack.pop();
    done.add(entry.entryId);
  };

  visit(root, rootVariant, undefined);
  return failure ?? { ok: true, nodes };
}

/**
 * Composition limits for the v1 vocabulary (spec §4.5): returns a violation
 * description, or `undefined` when the closure can be composed.
 */
export function compositionViolation(nodes: readonly ClosureNode[]): string | undefined {
  const versions = new Set(nodes.map(node => node.variant.sandboxPolicy.version));
  if (versions.size > 1) {
    return `mixed sandboxPolicy.version values (${[...versions].sort().join(', ')})`;
  }
  if (nodes.length < 2) {
    return undefined;
  }
  for (const node of nodes) {
    const policy = node.variant.sandboxPolicy;
    for (const key of Object.keys(policy)) {
      if (key !== 'version' && key !== 'filesystem') {
        return `'${node.entry.entryId}' uses '${key}', which has no v1 cross-entry composition rule`;
      }
    }
  }
  return undefined;
}

// ---------------------------------------------------------------------------
// Revision-level validation
// ---------------------------------------------------------------------------

function identityKeys(entry: CatalogEntry): string[] {
  const keys: string[] = [];
  for (const predicate of entry.identity) {
    if (predicate.kind === 'purl') {
      keys.push(`purl:${parsePurl(predicate.value)!.key}`);
    } else {
      // Invocation names are unique case-insensitively so matching is
      // unambiguous on every platform, including Windows.
      keys.push(...predicate.names.map(name => `invocation-name:${name.toLowerCase()}`));
    }
  }
  return keys;
}

/**
 * Validates one catalog revision against the v1 contract: shape, identity
 * uniqueness and ordering, platform selectors, symbols, unsafe paths, backend
 * neutrality, dependency closure and cycle-freedom for every supported
 * platform/architecture, and the v1 composition limits.
 */
export function validateCatalogRevision(raw: unknown, contract: CatalogContract): CatalogRevision {
  if (!isRecord(raw)) {
    fail('catalog root must be an object');
  }
  onlyFields(raw, ['catalogSchemaVersion', 'catalogRevision', 'entries'], 'catalog');
  if (raw.catalogSchemaVersion !== contract.catalogSchemaVersion) {
    fail(`catalog.catalogSchemaVersion must be '${contract.catalogSchemaVersion}'`);
  }
  const catalogRevision = nonEmptyString(raw.catalogRevision, 'catalog.catalogRevision');
  if (!isCatalogRevisionId(catalogRevision)) {
    fail(`catalog.catalogRevision '${catalogRevision}' must match YYYY-MM-DD.N`);
  }
  if (!Array.isArray(raw.entries)) {
    fail('catalog.entries must be an array');
  }
  const entries = raw.entries.map((entry, index) => validateEntry(entry, `entries[${index}]`, contract));

  const byId = new Map<string, CatalogEntry>();
  const identityOwners = new Map<string, string>();
  for (const entry of entries) {
    if (byId.has(entry.entryId)) {
      fail(`duplicate entryId '${entry.entryId}'`);
    }
    byId.set(entry.entryId, entry);
    for (const key of identityKeys(entry)) {
      const owner = identityOwners.get(key);
      if (owner !== undefined && owner !== entry.entryId) {
        fail(`identity '${key}' is claimed by both '${owner}' and '${entry.entryId}'`);
      }
      identityOwners.set(key, entry.entryId);
    }
  }

  for (const entry of entries) {
    for (const variant of entry.platformVariants) {
      for (const dependency of variant.dependencies ?? []) {
        if (!byId.has(dependency.entryId)) {
          fail(`'${entry.entryId}' depends on unknown entry '${dependency.entryId}' in this revision`);
        }
        if (dependency.entryId === entry.entryId) {
          fail(`'${entry.entryId}' depends on itself`);
        }
      }
    }
    for (const platform of PLATFORMS) {
      for (const architecture of ARCHITECTURES) {
        const selected = selectVariant(entry, platform, architecture);
        if (!selected) {
          continue;
        }
        const closure = dependencyClosure(entry, selected.variant, byId, platform, architecture);
        if (!closure.ok) {
          fail(`'${entry.entryId}' on ${platform}/${architecture}: ${closure.reason} (${closure.detail})`);
        }
        const violation = compositionViolation(closure.nodes);
        if (violation) {
          fail(`'${entry.entryId}' on ${platform}/${architecture}: ${violation}`);
        }
        const classes: Partial<Record<AccessClass, string[]>> = {};
        for (const field of COMPOSABLE_FILESYSTEM_FIELDS) {
          classes[field] = closure.nodes.flatMap(node => node.variant.sandboxPolicy.filesystem?.[field] ?? []);
        }
        const overlap = findCrossClassOverlap(classes, platform);
        if (overlap) {
          fail(`'${entry.entryId}' on ${platform}/${architecture}: ${overlap}`);
        }
      }
    }
  }

  return { catalogSchemaVersion: contract.catalogSchemaVersion, catalogRevision, entries };
}
