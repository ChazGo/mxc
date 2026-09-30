// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Shared helpers for the functional suite. Everything here resolves the
// library, the CLI, and the bundled catalog from the package that
// run-tests.js packed with `npm pack` and installed into a temporary consumer
// project outside the repository. Nothing reads the repository's dist/ or
// catalog/: the suite tests what a consumer would install.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, readFileSync, realpathSync, writeFileSync } from 'node:fs';
import { isAbsolute, join, relative } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
// Type-only imports are erased at compile time; the runtime modules below are
// loaded from the installed package.
import type * as Library from '@mxc-prototype/policy-catalog';
import type * as Tooling from '@mxc-prototype/policy-catalog/tooling';

const consumerDir = process.env.POLICY_CATALOG_CONSUMER_DIR;
if (!consumerDir) {
  throw new Error('POLICY_CATALOG_CONSUMER_DIR is not set; run the functional suite through tests/functional/run-tests.js');
}

/** The consumer project the tarball was installed into. */
export const consumer = consumerDir;
/** The installed package directory inside the consumer's node_modules. */
export const packageDir = join(consumer, 'node_modules', '@mxc-prototype', 'policy-catalog');
const pkg = JSON.parse(readFileSync(join(packageDir, 'package.json'), 'utf8'));

// Guard: the package under test must not be the source tree.
// tests/functional/dist/helpers.js -> package source root
const sourceRoot = realpathSync.native(fileURLToPath(new URL('../../../', import.meta.url)));
const installedRoot = realpathSync.native(packageDir);
const fromSource = relative(sourceRoot, installedRoot);
assert.ok(fromSource.startsWith('..') || isAbsolute(fromSource), `installed package ${installedRoot} is inside the source tree ${sourceRoot}`);

/** The installed CLI entry point, as declared by the installed package's `bin`. */
export const cliPath = join(packageDir, pkg.bin['policy-catalog']);
/** The catalog directory shipped in the installed package. */
export const installedCatalogDir = join(packageDir, 'catalog');
export const installedManifest = JSON.parse(readFileSync(join(installedCatalogDir, 'manifest.json'), 'utf8'));

/** The installed library, loaded through the installed package's `exports`. */
export const lib: typeof Library = await import(pathToFileURL(join(packageDir, pkg.exports['.'].import)).href);
/** The installed tooling subpath (digests for synthetic catalogs). */
export const tooling: typeof Tooling = await import(pathToFileURL(join(packageDir, pkg.exports['./tooling'].import)).href);

export interface CliResult {
  status: number | null;
  json: any;
  stdout: string;
  stderr: string;
}

function parse(stdout: string): any {
  try {
    return stdout.trim() === '' ? undefined : JSON.parse(stdout);
  } catch {
    return stdout;
  }
}

/** Runs the installed CLI in a separate process from the consumer directory. */
export function cli(...args: string[]): CliResult {
  const result = spawnSync(process.execPath, [cliPath, ...args], { cwd: consumer, encoding: 'utf8' });
  return { status: result.status, json: parse(result.stdout), stdout: result.stdout, stderr: result.stderr };
}

/** Runs the CLI through the npm-installed bin shim (`node_modules/.bin`). */
export function cliViaBin(...args: string[]): CliResult {
  const bin = join(consumer, 'node_modules', '.bin', 'policy-catalog');
  const result = process.platform === 'win32'
    // .cmd shims need a shell on Windows; arguments here are fixed literals.
    ? spawnSync(`"${bin}.cmd" ${args.join(' ')}`, { cwd: consumer, encoding: 'utf8', shell: true })
    : spawnSync(bin, args, { cwd: consumer, encoding: 'utf8' });
  return { status: result.status, json: parse(result.stdout), stdout: result.stdout, stderr: result.stderr };
}

export const PLATFORM_CONTEXT = {
  windows: { root: 'C:\\work\\app', prefix: 'C:\\tools', cache: 'C:\\cache\\npm' },
  linux: { root: '/work/app', prefix: '/opt/tools', cache: '/var/cache/npm' },
  macos: { root: '/Users/dev/app', prefix: '/opt/homebrew/bin', cache: '/Users/dev/.npm' },
} as const;
export type Platform = keyof typeof PLATFORM_CONTEXT;
export const PLATFORMS = Object.keys(PLATFORM_CONTEXT) as Platform[];
export const ARCHITECTURES = ['x64', 'arm64'] as const;

/** Resolve flags with every bundled-catalog symbol supplied for `platform`. */
export function symbolArgs(platform: Platform): string[] {
  const c = PLATFORM_CONTEXT[platform];
  return [
    '--project-root', c.root,
    '--symbol', `git_prefix=${c.prefix}`,
    '--symbol', `node_prefix=${c.prefix}`,
    '--symbol', `npm_prefix=${c.prefix}`,
    '--symbol', `npm_cache=${c.cache}`,
  ];
}

export function fullContext(platform: Platform, architecture: string): string[] {
  return ['--platform', platform, '--architecture', architecture, '--allow-weak', ...symbolArgs(platform)];
}

/** Minimal valid entry for synthetic catalogs. */
export function entry(entryId: string, overrides: Record<string, unknown> = {}): any {
  const name = entryId.split(':')[1];
  return {
    entryId,
    entryRevision: 1,
    displayName: name,
    identity: [{ kind: 'invocation-name', names: [name] }],
    platformVariants: [
      {
        when: { platform: 'linux' },
        sandboxPolicy: { version: '0.9.0-alpha', filesystem: { readonlyPaths: [`\${git_prefix}/${name}`] } },
      },
    ],
    provenance: { method: 'functional-test', sourceRevision: 'functional-test' },
    ...overrides,
  };
}

export function revision(entries: any[], catalogRevision = '2000-01-01.1'): any {
  return { catalogSchemaVersion: '1', catalogRevision, entries };
}

/**
 * Writes a complete catalog directory (contract from the installed package,
 * manifest with correct canonical digests, revision files) so the only
 * defect in it is the one a test introduces on purpose.
 */
export function writeCatalog(dir: string, revisions: any[], options: { defaultRevision?: string } = {}): string {
  mkdirSync(join(dir, 'revisions'), { recursive: true });
  copyFileSync(join(installedCatalogDir, 'contract.v1.json'), join(dir, 'contract.v1.json'));
  const manifest = {
    catalogSchemaVersion: '1',
    defaultRevision: options.defaultRevision ?? revisions[revisions.length - 1].catalogRevision,
    revisions: revisions.map(r => {
      const file = `revisions/${r.catalogRevision}.json`;
      writeFileSync(join(dir, file), `${JSON.stringify(r, null, 2)}\n`);
      return { catalogRevision: r.catalogRevision, file, sha256: tooling.canonicalSha256(r) };
    }),
  };
  writeFileSync(join(dir, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
  return dir;
}

export function warningsOf(result: CliResult): string {
  return (result.json?.diagnostics?.warnings ?? []).join('\n');
}
