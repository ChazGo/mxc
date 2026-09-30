// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Contribution/CI validation pipeline for the catalog (spec §7, §12 "Data").
//
//   1. JSON Schema conformance of contract, manifest, and every revision.
//   2. Integrity: every revision matches its published canonical SHA-256.
//   3. Semantic validation shared with the resolver: registered
//      SandboxPolicy versions, identity uniqueness/ordering, selectors,
//      dependency closure and cycle freedom, symbol validity, no literal or
//      user-specific paths, no wildcard grants, unsupported-field rejection,
//      backend neutrality, and v1 composition limits.
//   4. Entry-revision monotonicity across consecutive published revisions.
//   5. Immutability of already-published revisions against a base git ref
//      (POLICY_CATALOG_BASE_REF, e.g. origin/main), when provided.
//   6. Deterministic resolution: every entry resolves identically twice for
//      every platform/architecture selector.
//   7. Package inclusion: the bundled default revision equals the repository
//      manifest default.
//   8. Every entry has a conformance fixture case.
import { execFileSync } from 'node:child_process';
import { readFileSync, readdirSync } from 'node:fs';
import { createRequire } from 'node:module';
import { join, relative } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { PolicyCatalog, loadCatalogDirectory, bundledCatalogStore } from '../dist/index.js';
import {
  ARCHITECTURES,
  PLATFORMS,
  checkPublishedImmutability,
  checkStoreHistory,
  selectVariant,
} from '../dist/tooling.js';

const require = createRequire(import.meta.url);
const Ajv2020 = require('ajv/dist/2020');

const root = fileURLToPath(new URL('../../', import.meta.url));
const catalogDir = join(root, 'catalog');
const schemaDir = join(root, 'schema');
const readJson = file => JSON.parse(readFileSync(file, 'utf8'));
const errors = [];
const step = (name, fn) => {
  try {
    const found = fn() ?? [];
    errors.push(...found.map(message => `${name}: ${message}`));
    console.log(`${found.length === 0 ? 'ok  ' : 'FAIL'} ${name}`);
  } catch (error) {
    errors.push(`${name}: ${error.message}`);
    console.log(`FAIL ${name}`);
  }
};

const ajv = new Ajv2020({ allErrors: true, strict: true });
const catalogSchema = ajv.compile(readJson(join(schemaDir, 'catalog.v1.schema.json')));
const manifestSchema = ajv.compile(readJson(join(schemaDir, 'manifest.v1.schema.json')));
const describe = validate => (validate.errors ?? []).map(e => `${e.instancePath || '/'} ${e.message}`);

const manifest = readJson(join(catalogDir, 'manifest.json'));

step('manifest schema', () => (manifestSchema(manifest) ? [] : describe(manifestSchema)));

step('revision schema', () => {
  const found = [];
  const listed = new Set(manifest.revisions.map(r => r.file));
  for (const name of readdirSync(join(catalogDir, 'revisions'))) {
    const file = `revisions/${name}`;
    if (!listed.has(file)) {
      found.push(`${file} is not listed in manifest.json`);
      continue;
    }
    if (!catalogSchema(readJson(join(catalogDir, file)))) {
      found.push(...describe(catalogSchema).map(m => `${file} ${m}`));
    }
  }
  return found;
});

const store = loadCatalogDirectory(pathToFileURL(`${catalogDir}/`));

step('integrity, contract, and entry-revision history', () => checkStoreHistory(store));

step('published revision immutability', () => {
  const baseRef = process.env.POLICY_CATALOG_BASE_REF;
  if (!baseRef) {
    console.log('     (skipped: set POLICY_CATALOG_BASE_REF to compare against a base ref)');
    return [];
  }
  const repoRoot = execFileSync('git', ['-C', root, 'rev-parse', '--show-toplevel'], { encoding: 'utf8' }).trim();
  const prefix = relative(repoRoot, catalogDir).replaceAll('\\', '/');
  const show = path => {
    try {
      return execFileSync('git', ['-C', repoRoot, 'show', `${baseRef}:${prefix}/${path}`], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] });
    } catch {
      return undefined;
    }
  };
  const baseManifestText = show('manifest.json');
  if (baseManifestText === undefined) {
    console.log(`     (no catalog at ${baseRef}; nothing published yet)`);
    return [];
  }
  const baseManifest = JSON.parse(baseManifestText);
  const baseFiles = new Map();
  for (const r of baseManifest.revisions) {
    const text = show(r.file);
    if (text !== undefined) baseFiles.set(r.file, text);
  }
  const proposedFiles = new Map(manifest.revisions.map(r => [r.file, readFileSync(join(catalogDir, r.file), 'utf8')]));
  return checkPublishedImmutability({ manifest: baseManifest, files: baseFiles }, { manifest, files: proposedFiles });
});

step('deterministic resolution', () => {
  const found = [];
  const catalog = new PolicyCatalog(store);
  const symbols = Object.fromEntries(
    Object.entries(store.contract.symbols).filter(([, d]) => d.source !== 'context').map(([name]) => [name, name]),
  );
  for (const revisionId of store.availableRevisions) {
    for (const entry of store.revision(revisionId).entries) {
      const name = entry.identity.find(p => p.kind === 'invocation-name')?.names[0];
      const purl = entry.identity.find(p => p.kind === 'purl')?.value;
      for (const platform of PLATFORMS) {
        for (const architecture of ARCHITECTURES) {
          if (!selectVariant(entry, platform, architecture)) continue;
          const base = platform === 'windows' ? 'C:\\ci' : '/ci';
          const sep = platform === 'windows' ? '\\' : '/';
          const ctx = {
            platform,
            architecture,
            catalogRevision: revisionId,
            allowWeakIdentityFallback: true,
            projectRoot: `${base}${sep}project`,
            symbols: Object.fromEntries(Object.keys(symbols).map(s => [s, `${base}${sep}${s}`])),
          };
          const tool = { invocationName: name ?? 'unused', ...(purl ? { packageUrl: purl } : {}) };
          const first = JSON.stringify(catalog.resolveCatalogEntry(tool, ctx));
          const second = JSON.stringify(catalog.resolveCatalogEntry(tool, ctx));
          if (first === undefined || first !== second) {
            found.push(`${revisionId} ${entry.entryId} ${platform}/${architecture} did not resolve deterministically`);
          }
        }
      }
    }
  }
  return found;
});

step('package inclusion', () => {
  const bundled = bundledCatalogStore();
  return bundled.defaultRevision === store.defaultRevision
    && JSON.stringify(bundled.availableRevisions) === JSON.stringify(store.availableRevisions)
    ? []
    : ['bundled package catalog does not match the repository catalog; rebuild the package'];
});

step('entry fixtures', () => {
  const fixtureDir = join(root, 'conformance', 'fixtures');
  const covered = new Set();
  for (const name of readdirSync(fixtureDir)) {
    const fixture = readJson(join(fixtureDir, name));
    if (fixture.catalog !== 'bundled') continue;
    for (const c of fixture.cases) {
      if (c.expect?.entryId) covered.add(c.expect.entryId);
    }
  }
  const found = [];
  for (const entry of store.revision().entries) {
    if (!covered.has(entry.entryId)) {
      found.push(`${entry.entryId} has no bundled-catalog conformance case in conformance/fixtures/`);
    }
  }
  return found;
});

if (errors.length > 0) {
  console.error('\nPolicy catalog validation FAILED:');
  for (const message of errors) console.error(`  - ${message}`);
  process.exit(1);
}
console.log(`\nPolicy catalog validation passed (${relative(process.cwd(), catalogDir) || '.'}, default revision ${store.defaultRevision}).`);
