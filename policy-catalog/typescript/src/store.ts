// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { readFileSync } from 'node:fs';
import { canonicalSha256 } from './canonical-json.js';
import { PolicyCatalogError } from './errors.js';
import {
  CATALOG_SCHEMA_VERSION,
  compareCatalogRevisions,
  isCatalogRevisionId,
  validateCatalogRevision,
  validateContract,
  type CatalogContract,
  type CatalogRevision,
} from './catalog.js';

/** One published revision as listed in `catalog/manifest.json`. */
export interface ManifestRevision {
  catalogRevision: string;
  file: string;
  sha256: string;
}

export interface CatalogManifest {
  catalogSchemaVersion: string;
  defaultRevision: string;
  revisions: ManifestRevision[];
}

/** In-memory source for a catalog store. Used by the file loader and by fixtures. */
export interface CatalogSource {
  contract: unknown;
  manifest: unknown;
  /** Reads the raw JSON of a manifest `file` entry. */
  readRevision(file: string): unknown;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function invalid(message: string): never {
  throw new PolicyCatalogError('validation', message);
}

function deepFreeze<T>(value: T): T {
  if (value !== null && typeof value === 'object' && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const child of Object.values(value as Record<string, unknown>)) {
      deepFreeze(child);
    }
  }
  return value;
}

export function validateManifest(raw: unknown): CatalogManifest {
  if (!isRecord(raw)) {
    invalid('manifest root must be an object');
  }
  for (const key of Object.keys(raw)) {
    if (!['catalogSchemaVersion', 'defaultRevision', 'revisions'].includes(key)) {
      invalid(`unsupported field 'manifest.${key}'`);
    }
  }
  if (raw.catalogSchemaVersion !== CATALOG_SCHEMA_VERSION) {
    invalid(`manifest.catalogSchemaVersion must be '${CATALOG_SCHEMA_VERSION}'`);
  }
  if (!Array.isArray(raw.revisions) || raw.revisions.length === 0) {
    invalid('manifest.revisions must be a non-empty array');
  }
  const revisions = raw.revisions.map((item, index): ManifestRevision => {
    const at = `manifest.revisions[${index}]`;
    if (!isRecord(item)) {
      invalid(`'${at}' must be an object`);
    }
    for (const key of Object.keys(item)) {
      if (!['catalogRevision', 'file', 'sha256'].includes(key)) {
        invalid(`unsupported field '${at}.${key}'`);
      }
    }
    const { catalogRevision, file, sha256 } = item;
    if (typeof catalogRevision !== 'string' || !isCatalogRevisionId(catalogRevision)) {
      invalid(`'${at}.catalogRevision' must match YYYY-MM-DD.N`);
    }
    if (file !== `revisions/${catalogRevision}.json`) {
      invalid(`'${at}.file' must be 'revisions/${catalogRevision}.json'`);
    }
    if (typeof sha256 !== 'string' || !/^[0-9a-f]{64}$/.test(sha256)) {
      invalid(`'${at}.sha256' must be a lower-case hex SHA-256 digest`);
    }
    return { catalogRevision, file, sha256 };
  });
  // Published revisions are append-only and strictly increasing.
  for (let index = 1; index < revisions.length; index += 1) {
    if (compareCatalogRevisions(revisions[index - 1].catalogRevision, revisions[index].catalogRevision) >= 0) {
      invalid(`manifest.revisions must be strictly increasing ('${revisions[index].catalogRevision}')`);
    }
  }
  const defaultRevision = raw.defaultRevision;
  if (typeof defaultRevision !== 'string' || !revisions.some(r => r.catalogRevision === defaultRevision)) {
    invalid('manifest.defaultRevision must name a listed revision');
  }
  return { catalogSchemaVersion: CATALOG_SCHEMA_VERSION, defaultRevision, revisions };
}

/**
 * Read-only access to the locally installed, integrity-validated catalog
 * revisions. Revisions are loaded lazily, verified against the manifest
 * digest, validated against the contract, and deep-frozen.
 */
export class CatalogStore {
  readonly contract: CatalogContract;
  readonly manifest: CatalogManifest;
  private readonly source: CatalogSource;
  private readonly loaded = new Map<string, CatalogRevision>();

  constructor(source: CatalogSource) {
    this.source = source;
    this.contract = deepFreeze(validateContract(source.contract));
    this.manifest = deepFreeze(validateManifest(source.manifest));
  }

  get defaultRevision(): string {
    return this.manifest.defaultRevision;
  }

  get availableRevisions(): string[] {
    return this.manifest.revisions.map(revision => revision.catalogRevision);
  }

  /**
   * Returns the requested revision, or the installed default when omitted.
   * An explicitly requested revision that is not installed is an error; it is
   * never substituted with a different revision.
   */
  revision(catalogRevision?: string): CatalogRevision {
    const id = catalogRevision ?? this.manifest.defaultRevision;
    const cached = this.loaded.get(id);
    if (cached) {
      return cached;
    }
    const listed = this.manifest.revisions.find(revision => revision.catalogRevision === id);
    if (!listed) {
      throw new PolicyCatalogError('revision-unavailable', `catalog revision '${id}' is not installed`);
    }
    let raw: unknown;
    try {
      raw = this.source.readRevision(listed.file);
    } catch (error) {
      throw new PolicyCatalogError('integrity', `catalog revision '${id}' could not be read: ${(error as Error).message}`);
    }
    const digest = canonicalSha256(raw);
    if (digest !== listed.sha256) {
      throw new PolicyCatalogError(
        'integrity',
        `catalog revision '${id}' digest ${digest} does not match the published digest ${listed.sha256}`,
      );
    }
    const revision = validateCatalogRevision(raw, this.contract);
    if (revision.catalogRevision !== id) {
      throw new PolicyCatalogError('integrity', `file '${listed.file}' declares revision '${revision.catalogRevision}', expected '${id}'`);
    }
    deepFreeze(revision);
    this.loaded.set(id, revision);
    return revision;
  }
}

function readJson(url: URL): unknown {
  return JSON.parse(readFileSync(url, 'utf8'));
}

/** Loads a store from a catalog directory (`contract.v1.json`, `manifest.json`, `revisions/`). */
export function loadCatalogDirectory(directory: URL): CatalogStore {
  const base = directory.href.endsWith('/') ? directory : new URL(`${directory.href}/`);
  let contract: unknown;
  let manifest: unknown;
  try {
    contract = readJson(new URL('contract.v1.json', base));
    manifest = readJson(new URL('manifest.json', base));
  } catch (error) {
    throw new PolicyCatalogError('integrity', `catalog data could not be read: ${(error as Error).message}`);
  }
  return new CatalogStore({
    contract,
    manifest,
    readRevision: file => readJson(new URL(file, base)),
  });
}

let bundled: CatalogStore | undefined;

/**
 * The catalog bundled with this package. The build copies the repository's
 * `catalog/` directory next to the compiled output (`<out>/../catalog/`).
 */
export function bundledCatalogStore(): CatalogStore {
  bundled ??= loadCatalogDirectory(new URL('../catalog/', import.meta.url));
  return bundled;
}
