// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Catalog-directory validation shared by `policy-catalog validate` and the
// contribution pipeline (scripts/validate-catalog.mjs). Tooling only: the
// runtime lookup API never imports this module or runs git.
import { execFileSync } from 'node:child_process';
import { readFileSync, realpathSync } from 'node:fs';
import { isAbsolute, relative, resolve as resolvePath } from 'node:path';
import { pathToFileURL } from 'node:url';
import { PolicyCatalogError } from './errors.js';
import { checkPublishedImmutability, checkStoreHistory, type PublishedState } from './history.js';
import { loadCatalogDirectory, type CatalogStore } from './store.js';

export interface CatalogValidationReport {
  /** True only when every requested check ran and passed. */
  ok: boolean;
  catalogDir: string;
  defaultRevision?: string;
  revisions?: string[];
  /** Present only when a base ref was requested. */
  baseRef?: {
    ref: string;
    /** Published revisions at the base ref that were compared; 0 when the base has no catalog. */
    comparedRevisions: number;
  };
  errors: string[];
}

function git(args: string[], cwd: string): string {
  return execFileSync('git', args, { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true });
}

function gitSucceeds(args: string[], cwd: string): boolean {
  try {
    git(args, cwd);
    return true;
  } catch {
    return false;
  }
}

/**
 * Reads the published catalog state at `baseRef` for the git repository that
 * contains `catalogDir`. Returns `undefined` when the base ref has no catalog
 * at that path (nothing is published yet). Throws a `validation` error when
 * the check cannot be performed, so a typo in the ref or a directory outside
 * git never passes silently.
 */
export function readPublishedStateAtRef(catalogDir: string, baseRef: string): PublishedState | undefined {
  // git receives the ref as an argument; one that starts with '-' would be
  // parsed as an option, so it is rejected rather than passed through.
  if (baseRef.length === 0 || baseRef.startsWith('-') || /[\s\0]/.test(baseRef)) {
    throw new PolicyCatalogError('invalid_catalog', `base-ref check: '${baseRef}' is not a valid git ref`);
  }
  // .native expands Windows 8.3 short names (e.g. RUNNER~1 in CI temp dirs),
  // so the path compares correctly with git's long-form top-level path.
  const dir = realpathSync.native(catalogDir);
  let top: string;
  try {
    top = realpathSync.native(git(['rev-parse', '--show-toplevel'], dir).trim());
  } catch {
    throw new PolicyCatalogError('invalid_catalog', `base-ref check: '${catalogDir}' is not inside a git work tree`);
  }
  if (!gitSucceeds(['rev-parse', '--verify', '--quiet', `${baseRef}^{commit}`], top)) {
    throw new PolicyCatalogError('invalid_catalog', `base-ref check: '${baseRef}' does not name a commit`);
  }
  const prefix = relative(top, dir).replaceAll('\\', '/');
  if (prefix.startsWith('..') || isAbsolute(prefix)) {
    throw new PolicyCatalogError('invalid_catalog', `base-ref check: '${catalogDir}' is outside its git work tree`);
  }
  const at = (file: string): string => `${baseRef}:${prefix === '' ? '' : `${prefix}/`}${file}`;
  if (!gitSucceeds(['cat-file', '-e', at('manifest.json')], top)) {
    return undefined;
  }
  const manifest = JSON.parse(git(['show', at('manifest.json')], top)) as PublishedState['manifest'];
  const files = new Map<string, string>();
  for (const revision of manifest.revisions ?? []) {
    if (gitSucceeds(['cat-file', '-e', at(revision.file)], top)) {
      files.set(revision.file, git(['show', at(revision.file)], top));
    }
  }
  return { manifest, files };
}

/**
 * Compares a catalog directory's manifest and revision files with the
 * revisions published at `baseRef` (design 10): published revisions keep
 * their manifest entry, digest, and content; new revisions are only appended.
 * Returns errors instead of throwing, including when the check cannot run.
 */
export function checkAgainstBaseRef(
  catalogDir: string,
  manifest: PublishedState['manifest'],
  baseRef: string,
): { comparedRevisions: number; errors: string[] } {
  const dir = resolvePath(catalogDir);
  try {
    const base = readPublishedStateAtRef(dir, baseRef);
    if (!base) {
      return { comparedRevisions: 0, errors: [] };
    }
    const files = new Map(manifest.revisions.map(r => {
      let text: string;
      try {
        text = readFileSync(resolvePath(dir, r.file), 'utf8');
      } catch {
        // Reported as a modification: a published file may not disappear.
        text = '';
      }
      return [r.file, text] as const;
    }));
    return {
      comparedRevisions: base.manifest.revisions.length,
      errors: checkPublishedImmutability(base, { manifest, files }).map(message => `[immutability] ${message}`),
    };
  } catch (error) {
    return { comparedRevisions: 0, errors: [describeError(error)] };
  }
}

function describeError(error: unknown): string {
  return error instanceof PolicyCatalogError ? error.message : `[policy_validation] ${(error as Error).message}`;
}

/**
 * Validates a catalog directory: manifest and contract validity, per-revision
 * integrity against the published canonical digest, the full catalog contract
 * (including dependency closure and cycle freedom), entry-revision history,
 * and, when `baseRef` is given, immutability of every revision published at
 * that ref. Never throws for catalog problems; they are reported in `errors`.
 */
export function validateCatalogDirectory(catalogDir: string, options: { baseRef?: string } = {}): CatalogValidationReport {
  const dir = resolvePath(catalogDir);
  // Field order is fixed so the JSON report is stable and readable.
  const report: CatalogValidationReport = { ok: false, catalogDir: dir, defaultRevision: undefined, revisions: undefined, baseRef: undefined, errors: [] };
  let store: CatalogStore;
  try {
    store = loadCatalogDirectory(pathToFileURL(`${dir}/`));
  } catch (error) {
    report.errors.push(describeError(error));
    return report;
  }
  report.defaultRevision = store.defaultRevision;
  report.revisions = store.availableRevisions;
  report.errors.push(...checkStoreHistory(store));
  if (options.baseRef !== undefined) {
    const result = checkAgainstBaseRef(dir, store.manifest, options.baseRef);
    report.baseRef = { ref: options.baseRef, comparedRevisions: result.comparedRevisions };
    report.errors.push(...result.errors);
  }
  report.ok = report.errors.length === 0;
  return report;
}
