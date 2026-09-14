// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { diagLog } from './diagnostic.js';
import { SandboxPolicy } from './types.js';

type Platform = 'windows' | 'linux' | 'macos';

interface InvocationNameIdentity {
  kind: 'invocation-name';
  names: string[];
}

interface ConfigFloorEntry {
  tool: string;
  description?: string;
  identity: InvocationNameIdentity[];
  requires?: string[];
  sandboxPolicy: SandboxPolicy;
}

interface ConfigFloorCatalog {
  schemaVersion: '1';
  entries: ConfigFloorEntry[];
}

/**
 * Inputs used to resolve machine-specific symbols in sandbox config floors.
 *
 * @experimental Config floors are an early proof of concept and may change.
 */
export interface ResolveContext {
  /** Project root for project-relative requirements. Defaults to `process.cwd()`. */
  projectRoot?: string;
  /** Overrides for catalog symbols, without the `${` and `}` delimiters. */
  symbols?: Record<string, string>;
}

const FLOOR_POLICY_FIELDS = new Set(['version', 'filesystem', 'network', 'ui', 'timeoutMs']);
const FILESYSTEM_FIELDS = new Set(['readonlyPaths', 'readwritePaths']);
const NETWORK_FIELDS = new Set([
  'allowOutbound',
  'allowLocalNetwork',
  'allowedHosts',
]);
const UI_FIELDS = new Set(['allowWindows', 'clipboard', 'allowInputInjection']);
const CLIPBOARD_VALUES = new Set(['none', 'read', 'write', 'all']);
const POLICY_VERSIONS = new Set([
  '0.6.0-alpha',
  '0.7.0-alpha',
  '0.8.0-alpha',
  '0.9.0-alpha',
]);
const KNOWN_SYMBOLS = new Set([
  'project_root',
  'git_prefix',
  'node_prefix',
  'npm_prefix',
  'npm_cache',
  'npm_registry_host',
]);
const SYMBOL_PATTERN = /\$\{([a-z][a-z0-9_]*)\}/g;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function assertOnlyFields(
  value: Record<string, unknown>,
  allowed: ReadonlySet<string>,
  location: string,
): void {
  const unsupported = Object.keys(value).find(field => !allowed.has(field));
  if (unsupported) {
    throw new Error(`Invalid config floor catalog: unsupported field '${location}.${unsupported}'`);
  }
}

function requireString(value: unknown, location: string): string {
  if (typeof value !== 'string' || value.length === 0) {
    throw new Error(`Invalid config floor catalog: '${location}' must be a non-empty string`);
  }
  return value;
}

function requireStringArray(value: unknown, location: string): string[] {
  if (!Array.isArray(value) || value.some(item => typeof item !== 'string' || item.length === 0)) {
    throw new Error(`Invalid config floor catalog: '${location}' must be an array of non-empty strings`);
  }
  return value;
}

function validateOptionalBoolean(value: unknown, location: string): void {
  if (value !== undefined && typeof value !== 'boolean') {
    throw new Error(`Invalid config floor catalog: '${location}' must be a boolean`);
  }
}

function isUnsafeUserSpecificAbsolutePath(value: string): boolean {
  return /^[a-z]:\\users\\[^\\]+(?:\\|$)/i.test(value)
    || /^\/(?:home|Users)\/[^/]+(?:\/|$)/.test(value);
}

function validateCatalogString(value: string, location: string): void {
  if (/[*?]/.test(value)) {
    throw new Error(`Invalid config floor catalog: '${location}' contains a wildcard`);
  }
  const symbolsRemoved = value.replace(SYMBOL_PATTERN, '');
  if (symbolsRemoved.includes('${')) {
    throw new Error(`Invalid config floor catalog: '${location}' contains malformed symbol syntax`);
  }
  for (const match of value.matchAll(SYMBOL_PATTERN)) {
    if (!KNOWN_SYMBOLS.has(match[1])) {
      throw new Error(
        `Invalid config floor catalog: '${location}' contains unknown symbol '${match[1]}'`,
      );
    }
  }
}

function validateCatalogPath(value: string, location: string): void {
  validateCatalogString(value, location);
  if (isUnsafeUserSpecificAbsolutePath(value)) {
    throw new Error(
      `Invalid config floor catalog: '${location}' contains a user-specific absolute path`,
    );
  }
}

function validatePolicy(rawPolicy: unknown, location: string): SandboxPolicy {
  if (!isRecord(rawPolicy)) {
    throw new Error(`Invalid config floor catalog: '${location}' must be an object`);
  }
  assertOnlyFields(rawPolicy, FLOOR_POLICY_FIELDS, location);
  const version = requireString(rawPolicy.version, `${location}.version`);
  if (!POLICY_VERSIONS.has(version)) {
    throw new Error(
      `Invalid config floor catalog: '${location}.version' is not a registered policy version`,
    );
  }

  if (rawPolicy.filesystem !== undefined) {
    if (!isRecord(rawPolicy.filesystem)) {
      throw new Error(`Invalid config floor catalog: '${location}.filesystem' must be an object`);
    }
    assertOnlyFields(rawPolicy.filesystem, FILESYSTEM_FIELDS, `${location}.filesystem`);
    for (const field of FILESYSTEM_FIELDS) {
      const value = rawPolicy.filesystem[field];
      if (value !== undefined) {
        requireStringArray(value, `${location}.filesystem.${field}`)
          .forEach(item => validateCatalogPath(item, `${location}.filesystem.${field}`));
      }
    }
  }

  if (rawPolicy.network !== undefined) {
    if (!isRecord(rawPolicy.network)) {
      throw new Error(`Invalid config floor catalog: '${location}.network' must be an object`);
    }
    assertOnlyFields(rawPolicy.network, NETWORK_FIELDS, `${location}.network`);
    validateOptionalBoolean(rawPolicy.network.allowOutbound, `${location}.network.allowOutbound`);
    validateOptionalBoolean(
      rawPolicy.network.allowLocalNetwork,
      `${location}.network.allowLocalNetwork`,
    );
    if (rawPolicy.network.allowedHosts !== undefined) {
      requireStringArray(rawPolicy.network.allowedHosts, `${location}.network.allowedHosts`)
        .forEach(item => validateCatalogString(item, `${location}.network.allowedHosts`));
    }
    if (rawPolicy.network.allowedHosts !== undefined && rawPolicy.network.allowOutbound !== true) {
      throw new Error(
        `Invalid config floor catalog: '${location}.network.allowedHosts' requires allowOutbound=true`,
      );
    }
  }

  if (rawPolicy.ui !== undefined) {
    if (!isRecord(rawPolicy.ui)) {
      throw new Error(`Invalid config floor catalog: '${location}.ui' must be an object`);
    }
    assertOnlyFields(rawPolicy.ui, UI_FIELDS, `${location}.ui`);
    validateOptionalBoolean(rawPolicy.ui.allowWindows, `${location}.ui.allowWindows`);
    validateOptionalBoolean(
      rawPolicy.ui.allowInputInjection,
      `${location}.ui.allowInputInjection`,
    );
    if (
      rawPolicy.ui.clipboard !== undefined
      && (
        typeof rawPolicy.ui.clipboard !== 'string'
        || !CLIPBOARD_VALUES.has(rawPolicy.ui.clipboard)
      )
    ) {
      throw new Error(`Invalid config floor catalog: '${location}.ui.clipboard' is unsupported`);
    }
  }

  if (
    rawPolicy.timeoutMs !== undefined
    && (
      typeof rawPolicy.timeoutMs !== 'number'
      || !Number.isInteger(rawPolicy.timeoutMs)
      || rawPolicy.timeoutMs < 0
    )
  ) {
    throw new Error(
      `Invalid config floor catalog: '${location}.timeoutMs' must be a non-negative integer`,
    );
  }

  return rawPolicy as SandboxPolicy;
}

/**
 * Validates untrusted catalog data and returns its typed representation.
 *
 * @internal Exported for repository validation tests; not part of the package root API.
 */
export function validateConfigFloorCatalog(rawCatalog: unknown): ConfigFloorCatalog {
  if (!isRecord(rawCatalog)) {
    throw new Error('Invalid config floor catalog: root must be an object');
  }
  assertOnlyFields(rawCatalog, new Set(['schemaVersion', 'entries']), 'catalog');
  if (rawCatalog.schemaVersion !== '1') {
    throw new Error("Invalid config floor catalog: 'schemaVersion' must be '1'");
  }
  if (!Array.isArray(rawCatalog.entries)) {
    throw new Error("Invalid config floor catalog: 'entries' must be an array");
  }

  const entries = rawCatalog.entries.map((rawEntry, index): ConfigFloorEntry => {
    const location = `entries[${index}]`;
    if (!isRecord(rawEntry)) {
      throw new Error(`Invalid config floor catalog: '${location}' must be an object`);
    }
    assertOnlyFields(
      rawEntry,
      new Set(['tool', 'description', 'identity', 'requires', 'sandboxPolicy']),
      location,
    );
    const tool = requireString(rawEntry.tool, `${location}.tool`);
    if (rawEntry.description !== undefined) {
      requireString(rawEntry.description, `${location}.description`);
    }
    if (!Array.isArray(rawEntry.identity) || rawEntry.identity.length === 0) {
      throw new Error(`Invalid config floor catalog: '${location}.identity' must not be empty`);
    }
    const identity = rawEntry.identity.map((rawIdentity, identityIndex): InvocationNameIdentity => {
      const identityLocation = `${location}.identity[${identityIndex}]`;
      if (!isRecord(rawIdentity)) {
        throw new Error(`Invalid config floor catalog: '${identityLocation}' must be an object`);
      }
      assertOnlyFields(rawIdentity, new Set(['kind', 'names']), identityLocation);
      if (rawIdentity.kind !== 'invocation-name') {
        throw new Error(`Invalid config floor catalog: '${identityLocation}.kind' is unsupported`);
      }
      return {
        kind: 'invocation-name',
        names: requireStringArray(rawIdentity.names, `${identityLocation}.names`),
      };
    });
    const requires = rawEntry.requires === undefined
      ? undefined
      : requireStringArray(rawEntry.requires, `${location}.requires`);
    return {
      tool,
      description: rawEntry.description as string | undefined,
      identity,
      requires,
      sandboxPolicy: validatePolicy(rawEntry.sandboxPolicy, `${location}.sandboxPolicy`),
    };
  });

  const tools = new Set<string>();
  const invocationNames = new Set<string>();
  for (const entry of entries) {
    if (tools.has(entry.tool)) {
      throw new Error(`Invalid config floor catalog: duplicate tool '${entry.tool}'`);
    }
    tools.add(entry.tool);
    for (const identity of entry.identity) {
      for (const name of identity.names) {
        const normalized = name.toLowerCase();
        if (invocationNames.has(normalized)) {
          throw new Error(`Invalid config floor catalog: duplicate invocation name '${name}'`);
        }
        invocationNames.add(normalized);
      }
    }
  }
  for (const entry of entries) {
    for (const dependency of entry.requires ?? []) {
      if (!tools.has(dependency)) {
        throw new Error(
          `Invalid config floor catalog: tool '${entry.tool}' requires unknown tool '${dependency}'`,
        );
      }
    }
  }

  return { schemaVersion: '1', entries };
}

const catalogJson: unknown = JSON.parse(
  fs.readFileSync(new URL('./config-floors/catalog.json', import.meta.url), 'utf8'),
);
const CATALOG = validateConfigFloorCatalog(catalogJson);

function currentPlatform(): Platform {
  switch (os.platform()) {
    case 'win32':
      return 'windows';
    case 'darwin':
      return 'macos';
    default:
      return 'linux';
  }
}

function resolveSymbol(
  name: string,
  context: ResolveContext,
  platform: Platform,
): string | undefined {
  if (context.symbols?.[name] !== undefined) {
    return context.symbols[name];
  }
  if (name === 'project_root') {
    return path.resolve(context.projectRoot ?? process.cwd());
  }
  if (name === 'git_prefix') {
    return resolveExecutableDirectory(platform === 'windows' ? ['git.exe', 'git'] : ['git']);
  }
  if (name === 'node_prefix') {
    return resolveExecutableDirectory(platform === 'windows' ? ['node.exe', 'node'] : ['node'])
      ?? path.dirname(process.execPath);
  }
  if (name === 'npm_prefix') {
    return resolveExecutableDirectory(
      platform === 'windows' ? ['npm.cmd', 'npm.exe', 'npm'] : ['npm'],
    );
  }
  if (name === 'npm_cache') {
    const home = os.homedir();
    const npmCache = process.env['NPM_CONFIG_CACHE']
      ?? (platform === 'windows'
        ? path.join(process.env['LOCALAPPDATA'] ?? home, 'npm-cache')
        : path.join(home, '.npm'));
    return path.resolve(npmCache);
  }
  if (name === 'npm_registry_host') {
    const registry = process.env['NPM_CONFIG_REGISTRY'] ?? 'https://registry.npmjs.org';
    try {
      return new URL(registry).hostname;
    } catch {
      throw new Error(
        "Cannot resolve config floor symbol 'npm_registry_host': invalid npm registry URL",
      );
    }
  }
  return undefined;
}

function resolveExecutableDirectory(names: readonly string[]): string | undefined {
  const pathDirectories = (process.env['PATH'] ?? process.env['Path'] ?? '')
    .split(path.delimiter)
    .filter(Boolean);
  for (const directory of pathDirectories) {
    for (const name of names) {
      const candidate = path.join(directory, name);
      try {
        if (fs.statSync(candidate).isFile()) {
          return path.resolve(directory);
        }
      } catch {
        // Continue searching PATH. An unreadable candidate does not resolve the symbol.
      }
    }
  }
  return undefined;
}

function resolveString(value: string, context: ResolveContext, platform: Platform): string {
  const unresolved: string[] = [];
  const resolved = value.replace(SYMBOL_PATTERN, (_match, name: string) => {
    const replacement = resolveSymbol(name, context, platform);
    if (replacement === undefined) {
      unresolved.push(name);
      return _match;
    }
    return replacement;
  });
  if (unresolved.length > 0) {
    throw new Error(`Cannot resolve config floor symbol '${unresolved[0]}'`);
  }
  if (resolved.includes('${')) {
    throw new Error('A config floor symbol override produced unresolved symbol syntax');
  }
  return resolved;
}

function unionStrings(
  left: string[] | undefined,
  right: string[] | undefined,
): string[] | undefined {
  const values = [...(left ?? []), ...(right ?? [])];
  return values.length === 0 ? undefined : [...new Set(values)].sort();
}

function clipboardRank(value: NonNullable<SandboxPolicy['ui']>['clipboard']): number {
  switch (value) {
    case 'read':
    case 'write':
      return 1;
    case 'all':
      return 2;
    default:
      return 0;
  }
}

function unionClipboard(
  left: NonNullable<SandboxPolicy['ui']>['clipboard'],
  right: NonNullable<SandboxPolicy['ui']>['clipboard'],
): NonNullable<SandboxPolicy['ui']>['clipboard'] {
  if ((left === 'read' && right === 'write') || (left === 'write' && right === 'read')) {
    return 'all';
  }
  return clipboardRank(right) > clipboardRank(left) ? right : left;
}

function mergePolicies(left: SandboxPolicy | undefined, right: SandboxPolicy): SandboxPolicy {
  if (!left) {
    return structuredClone(right);
  }
  if (left.version !== right.version) {
    throw new Error(
      `Cannot merge config floors using policy versions '${left.version}' and '${right.version}'`,
    );
  }

  const readwritePaths = unionStrings(
    left.filesystem?.readwritePaths,
    right.filesystem?.readwritePaths,
  );
  const readwriteSet = new Set(readwritePaths);
  const readonlyCandidates = unionStrings(
    left.filesystem?.readonlyPaths,
    right.filesystem?.readonlyPaths,
  )?.filter(value => !readwriteSet.has(value));
  const readonlyPaths = readonlyCandidates && readonlyCandidates.length > 0
    ? readonlyCandidates
    : undefined;
  const filesystem = readwritePaths || readonlyPaths
    ? {
        ...(readwritePaths ? { readwritePaths } : {}),
        ...(readonlyPaths ? { readonlyPaths } : {}),
      }
    : undefined;

  const allowedHosts = unionStrings(left.network?.allowedHosts, right.network?.allowedHosts);
  const allowOutbound = (left.network?.allowOutbound ?? false)
    || (right.network?.allowOutbound ?? false);
  const hasUnrestrictedOutbound =
    (left.network?.allowOutbound === true && left.network.allowedHosts === undefined)
    || (right.network?.allowOutbound === true && right.network.allowedHosts === undefined);
  const effectiveAllowedHosts = hasUnrestrictedOutbound ? undefined : allowedHosts;
  const allowLocalNetwork = (left.network?.allowLocalNetwork ?? false)
    || (right.network?.allowLocalNetwork ?? false);
  const network = allowOutbound || allowLocalNetwork || effectiveAllowedHosts
    ? {
        ...(allowOutbound ? { allowOutbound } : {}),
        ...(allowLocalNetwork ? { allowLocalNetwork } : {}),
        ...(effectiveAllowedHosts ? { allowedHosts: effectiveAllowedHosts } : {}),
      }
    : undefined;

  const allowWindows = (left.ui?.allowWindows ?? false) || (right.ui?.allowWindows ?? false);
  const clipboard = unionClipboard(left.ui?.clipboard ?? 'none', right.ui?.clipboard ?? 'none');
  const allowInputInjection = (left.ui?.allowInputInjection ?? false)
    || (right.ui?.allowInputInjection ?? false);
  const ui = allowWindows || clipboard !== 'none' || allowInputInjection
    ? {
        ...(allowWindows ? { allowWindows } : {}),
        ...(clipboard !== 'none' ? { clipboard } : {}),
        ...(allowInputInjection ? { allowInputInjection } : {}),
      }
    : undefined;

  const timeoutMs = Math.max(left.timeoutMs ?? 0, right.timeoutMs ?? 0) || undefined;
  return {
    version: left.version,
    ...(filesystem ? { filesystem } : {}),
    ...(network ? { network } : {}),
    ...(ui ? { ui } : {}),
    ...(timeoutMs ? { timeoutMs } : {}),
  };
}

function resolvePolicy(
  policy: SandboxPolicy,
  context: ResolveContext,
  platform: Platform,
): SandboxPolicy {
  const resolveArray = (values: string[] | undefined): string[] | undefined =>
    values?.map(value => resolveString(value, context, platform));
  const readwritePaths = unionStrings(
    undefined,
    resolveArray(policy.filesystem?.readwritePaths),
  );
  const pathKey = platform === 'windows'
    ? (value: string): string => value.toLowerCase()
    : (value: string): string => value;
  const readwritePathKeys = new Set(readwritePaths?.map(pathKey));
  const readonlyCandidates = unionStrings(
    undefined,
    resolveArray(policy.filesystem?.readonlyPaths),
  )?.filter(value => !readwritePathKeys.has(pathKey(value)));
  const readonlyPaths = readonlyCandidates && readonlyCandidates.length > 0
    ? readonlyCandidates
    : undefined;
  const allowedHosts = unionStrings(undefined, resolveArray(policy.network?.allowedHosts));
  const filesystem = policy.filesystem
    ? {
        ...(readonlyPaths ? { readonlyPaths } : {}),
        ...(readwritePaths ? { readwritePaths } : {}),
      }
    : undefined;
  const network = policy.network
    ? {
        ...policy.network,
        ...(allowedHosts ? { allowedHosts } : {}),
      }
    : undefined;

  return {
    version: policy.version,
    ...(filesystem ? { filesystem } : {}),
    ...(network ? { network } : {}),
    ...(policy.ui ? { ui: structuredClone(policy.ui) } : {}),
    ...(policy.timeoutMs !== undefined ? { timeoutMs: policy.timeoutMs } : {}),
  };
}

function resolveFromCatalog(
  tools: readonly string[],
  context: ResolveContext,
  catalog: ConfigFloorCatalog,
  platform: Platform,
): SandboxPolicy | undefined {
  const byTool = new Map(catalog.entries.map(entry => [entry.tool, entry]));
  const byInvocationName = new Map<string, ConfigFloorEntry>();
  const normalizeInvocation = platform === 'windows'
    ? (value: string): string => value.toLowerCase()
    : (value: string): string => value;
  for (const entry of catalog.entries) {
    for (const identity of entry.identity) {
      for (const name of identity.names) {
        byInvocationName.set(normalizeInvocation(name), entry);
      }
    }
  }

  const visited = new Set<string>();
  let merged: SandboxPolicy | undefined;
  const visit = (entry: ConfigFloorEntry): void => {
    if (visited.has(entry.tool)) {
      return;
    }
    visited.add(entry.tool);
    for (const dependency of entry.requires ?? []) {
      visit(byTool.get(dependency)!);
    }
    merged = mergePolicies(merged, entry.sandboxPolicy);
  };

  for (const tool of tools) {
    const entry = byInvocationName.get(normalizeInvocation(tool));
    if (entry) {
      visit(entry);
    } else {
      diagLog(`getSandboxConfigForTool: no config floor for invocation '${tool}'`);
    }
  }

  if (!merged) {
    return undefined;
  }
  diagLog(`getSandboxConfigForTool: resolved catalog floors for ${[...visited].sort().join(', ')}`);
  return resolvePolicy(merged, context, platform);
}

/**
 * Resolves repository-authored minimum tool requirements to a literal policy.
 *
 * Config floors are compatibility input, not authorization. This function does
 * not inspect, modify, merge with, or override a host-authored policy. Invocation
 * names are a lookup convenience and are not a security identity boundary.
 *
 * @returns `undefined` when every requested tool is unknown.
 * @experimental This proof-of-concept API and catalog may change.
 */
export function getSandboxConfigForTool(
  tools: readonly string[],
  context: ResolveContext = {},
): SandboxPolicy | undefined {
  return resolveFromCatalog(tools, context, CATALOG, currentPlatform());
}

/** @internal Test-only resolver seam for catalog and platform fixtures. */
export function resolveConfigFloorCatalogForTest(
  tools: readonly string[],
  context: ResolveContext,
  rawCatalog: unknown,
  platform: Platform,
): SandboxPolicy | undefined {
  return resolveFromCatalog(
    tools,
    context,
    validateConfigFloorCatalog(rawCatalog),
    platform,
  );
}
