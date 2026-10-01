#!/usr/bin/env node
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Command-line harness over the public library API. It is intended for
// contributors, CI, and functional tests; it is not a sandbox launcher.
// There are exactly three commands, one per use:
//
//   policy-catalog resolve [--catalog DIR] [--diagnostics] [--platform P]
//       [--architecture A] [--revision R] [--project-root PATH]
//       [--symbol name=value]... [--allow-weak]
//       [--purl URL] [--detected-version V] <tool>...
//   policy-catalog inspect [--catalog DIR]
//   policy-catalog validate [--catalog DIR] [--base-ref REF]
//
// resolve   runs resolveSandboxPolicy / resolveSandboxPolicyWithDiagnostics.
// inspect   prints getCatalogInfo() and listCatalogEntries() (metadata only).
// validate  checks integrity, the catalog contract (including dependency
//           cycles), entry-revision history, and, with --base-ref, that every
//           revision published at REF is unchanged.
//
// --catalog defaults to the catalog bundled with this package.
// Exit codes: 0 success (including "no policy" for resolve), 1 library
// failure (PolicyCatalogError) or failed validation, 2 usage error.
// Output is JSON on stdout.
import { realpathSync } from 'node:fs';
import { resolve as resolvePath } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { PolicyCatalogError } from './errors.js';
import { PolicyCatalog } from './resolver.js';
import { bundledCatalogStore, loadCatalogDirectory, type CatalogStore } from './store.js';
import { validateCatalogDirectory } from './validate.js';
import type { CatalogArchitecture, CatalogPlatform, ResolveContext, ToolCandidate } from './types.js';

const USAGE = 'usage: policy-catalog <resolve|inspect|validate> [options]';

class UsageError extends Error {}

function takeValue(args: string[], index: number, flag: string): string {
  const value = args[index + 1];
  if (value === undefined || value.startsWith('--')) {
    throw new UsageError(`${flag} requires a value`);
  }
  return value;
}

function storeFrom(catalogDir: string | undefined): CatalogStore {
  return catalogDir === undefined
    ? bundledCatalogStore()
    : loadCatalogDirectory(pathToFileURL(`${resolvePath(catalogDir)}/`));
}

function bundledCatalogDir(): string {
  return fileURLToPath(new URL('../catalog/', import.meta.url));
}

function print(value: unknown): void {
  process.stdout.write(`${JSON.stringify(value, null, 2)}\n`);
}

function rejectRest(args: string[], command: string): void {
  if (args.length > 0) {
    throw new UsageError(`${command}: unexpected argument '${args[0]}'`);
  }
}

export function main(argv: string[]): number {
  const [command, ...rest] = argv;
  try {
    let catalogDir: string | undefined;
    let baseRef: string | undefined;
    const args: string[] = [];
    for (let i = 0; i < rest.length; i += 1) {
      if (rest[i] === '--catalog') {
        catalogDir = takeValue(rest, i, '--catalog');
        i += 1;
      } else if (rest[i] === '--base-ref' && command === 'validate') {
        baseRef = takeValue(rest, i, '--base-ref');
        i += 1;
      } else {
        args.push(rest[i]);
      }
    }
    switch (command) {
      case 'inspect': {
        rejectRest(args, command);
        const catalog = new PolicyCatalog(storeFrom(catalogDir));
        print({ info: catalog.getCatalogInfo(), entries: catalog.listCatalogEntries() });
        return 0;
      }
      case 'validate': {
        rejectRest(args, command);
        const report = validateCatalogDirectory(catalogDir ?? bundledCatalogDir(), baseRef === undefined ? {} : { baseRef });
        print(report);
        return report.ok ? 0 : 1;
      }
      case 'resolve':
        return resolveCommand(args, catalogDir);
      default:
        throw new UsageError(command === undefined ? 'missing command' : `unknown command '${command}'`);
    }
  } catch (error) {
    if (error instanceof UsageError) {
      process.stderr.write(`policy-catalog: ${error.message}\n${USAGE}\n`);
      return 2;
    }
    if (error instanceof PolicyCatalogError) {
      print({ error: { code: error.code, message: error.message, details: { reason: error.reason } } });
      return 1;
    }
    throw error;
  }
}

function resolveCommand(args: string[], catalogDir: string | undefined): number {
  const ctx: ResolveContext = {};
  // Null prototype: a '__proto__' name must stay an ordinary (rejected) key.
  const symbols: Record<string, string> = Object.create(null);
  const tools: Array<string | ToolCandidate> = [];
  let diagnostics = false;
  let purl: string | undefined;
  let detectedVersion: string | undefined;
  for (let i = 0; i < args.length; i += 1) {
    const arg = args[i];
    switch (arg) {
      case '--diagnostics':
        diagnostics = true;
        break;
      case '--allow-weak':
        ctx.allowWeakIdentityFallback = true;
        break;
      case '--platform':
        ctx.platform = takeValue(args, i++, arg) as CatalogPlatform;
        break;
      case '--architecture':
        ctx.architecture = takeValue(args, i++, arg) as CatalogArchitecture;
        break;
      case '--revision':
        ctx.catalogRevision = takeValue(args, i++, arg);
        break;
      case '--project-root':
        ctx.projectRoot = takeValue(args, i++, arg);
        break;
      case '--symbol': {
        const pair = takeValue(args, i++, arg);
        const eq = pair.indexOf('=');
        if (eq <= 0) {
          throw new UsageError('--symbol expects name=value');
        }
        symbols[pair.slice(0, eq)] = pair.slice(eq + 1);
        break;
      }
      case '--purl':
        purl = takeValue(args, i++, arg);
        break;
      case '--detected-version':
        detectedVersion = takeValue(args, i++, arg);
        break;
      default:
        if (arg.startsWith('--')) {
          throw new UsageError(`unknown option '${arg}'`);
        }
        // --purl / --detected-version apply to the next tool name only.
        tools.push(purl === undefined && detectedVersion === undefined
          ? arg
          : { invocationName: arg, ...(purl !== undefined ? { packageUrl: purl } : {}), ...(detectedVersion !== undefined ? { detectedVersion } : {}) });
        purl = undefined;
        detectedVersion = undefined;
    }
  }
  if (purl !== undefined || detectedVersion !== undefined) {
    throw new UsageError('--purl/--detected-version must precede a tool name');
  }
  if (Object.keys(symbols).length > 0) {
    ctx.symbols = symbols;
  }
  const catalog = new PolicyCatalog(storeFrom(catalogDir));
  if (diagnostics) {
    print(catalog.resolveSandboxPolicyWithDiagnostics(tools, ctx));
  } else {
    // `null` is JSON's rendering of "no policy"; it is never an empty policy.
    print(catalog.resolveSandboxPolicy(tools, ctx) ?? null);
  }
  return 0;
}

// npm installs the bin as a symlink on POSIX, so compare real paths; a plain
// URL comparison would silently do nothing when run through the symlink.
function invokedDirectly(): boolean {
  const entry = process.argv[1];
  if (entry === undefined) {
    return false;
  }
  try {
    return realpathSync(resolvePath(entry)) === realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
}

if (invokedDirectly()) {
  process.exitCode = main(process.argv.slice(2));
}
