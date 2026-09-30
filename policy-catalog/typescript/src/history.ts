// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { canonicalJson } from './canonical-json.js';
import { PolicyCatalogError } from './errors.js';
import { compareCatalogRevisions, type CatalogEntry, type CatalogRevision } from './catalog.js';
import type { CatalogStore } from './store.js';

function entrySemanticKey(entry: CatalogEntry): string {
  const { entryRevision: _ignored, ...rest } = entry;
  return canonicalJson(rest);
}

/**
 * Checks entry-revision monotonicity between two consecutive published
 * catalog revisions (spec §4.1, §10): a semantically changed entry must bump
 * `entryRevision`; an unchanged entry must keep it; it never decreases.
 */
export function checkEntryRevisions(previous: CatalogRevision, next: CatalogRevision): string[] {
  const errors: string[] = [];
  if (compareCatalogRevisions(previous.catalogRevision, next.catalogRevision) >= 0) {
    errors.push(`catalog revision '${next.catalogRevision}' must be newer than '${previous.catalogRevision}'`);
  }
  const before = new Map(previous.entries.map(entry => [entry.entryId, entry]));
  for (const entry of next.entries) {
    const old = before.get(entry.entryId);
    if (!old) {
      continue;
    }
    const changed = entrySemanticKey(old) !== entrySemanticKey(entry);
    if (changed && entry.entryRevision <= old.entryRevision) {
      errors.push(`${next.catalogRevision}: '${entry.entryId}' changed but entryRevision did not increase (${old.entryRevision} -> ${entry.entryRevision})`);
    } else if (!changed && entry.entryRevision !== old.entryRevision) {
      errors.push(`${next.catalogRevision}: '${entry.entryId}' is unchanged but entryRevision moved (${old.entryRevision} -> ${entry.entryRevision})`);
    }
  }
  return errors;
}

/**
 * Validates every installed revision (integrity + contract) and the
 * entry-revision chain between consecutive revisions.
 */
export function checkStoreHistory(store: CatalogStore): string[] {
  const errors: string[] = [];
  let previous: CatalogRevision | undefined;
  for (const id of store.availableRevisions) {
    let current: CatalogRevision;
    try {
      current = store.revision(id);
    } catch (error) {
      errors.push(error instanceof PolicyCatalogError ? error.message : String(error));
      previous = undefined;
      continue;
    }
    if (previous) {
      errors.push(...checkEntryRevisions(previous, current));
    }
    previous = current;
  }
  return errors;
}

/** Raw published state used to compare a proposed change against its base. */
export interface PublishedState {
  manifest: { revisions: Array<{ catalogRevision: string; file: string; sha256: string }> };
  /** Raw file text keyed by manifest `file`. */
  files: ReadonlyMap<string, string>;
}

/**
 * Enforces immutability of published revisions across a proposed change
 * (spec §10): previously published revisions keep their manifest entry,
 * digest, and file bytes, and new revisions are only appended.
 */
export function checkPublishedImmutability(base: PublishedState, proposed: PublishedState): string[] {
  const errors: string[] = [];
  base.manifest.revisions.forEach((published, index) => {
    const now = proposed.manifest.revisions[index];
    if (!now || now.catalogRevision !== published.catalogRevision) {
      errors.push(`published revision '${published.catalogRevision}' was removed or reordered in the manifest`);
      return;
    }
    if (now.file !== published.file || now.sha256 !== published.sha256) {
      errors.push(`published revision '${published.catalogRevision}' manifest entry was modified`);
    }
    // Compare canonical content, not raw bytes, so checkout line-ending
    // conversion cannot produce a false positive.
    const canonical = (text: string | undefined): string | undefined => {
      if (text === undefined) {
        return undefined;
      }
      try {
        return canonicalJson(JSON.parse(text));
      } catch {
        return `invalid:${text}`;
      }
    };
    const baseContent = canonical(base.files.get(published.file));
    const proposedContent = canonical(proposed.files.get(published.file));
    if (baseContent !== undefined && proposedContent !== baseContent) {
      errors.push(`published revision file '${published.file}' was modified; publish a new revision instead`);
    }
  });
  return errors;
}
