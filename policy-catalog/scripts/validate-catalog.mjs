// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Contribution/CI validation pipeline for the catalog (design §7, §12 "Data").
//
//   1. JSON Schema conformance of contract, manifest, and every revision.
//   2. Integrity: every revision matches its published canonical SHA-256.
//   3. Semantic validation shared with the resolver: registered
//      SandboxPolicy versions, entry-ID uniqueness, per-entry identity
//      predicates, selectors,
//      dependency closure and cycle freedom, symbol validity, no literal or
//      user-specific paths, no wildcard grants, unsupported-field rejection,
//      backend neutrality, and v1 composition limits.
//   4. Entry-revision monotonicity across consecutive published revisions.
//   5. Immutability of already-published revisions against a base git ref
//      (--base-ref=<ref> or POLICY_CATALOG_BASE_REF, e.g. origin/main), when
//      provided. Shares its implementation with `policy-catalog validate`.
//   6. Deterministic resolution: every entry resolves identically twice for
//      every platform/architecture selector, and the whole-catalog lookup is
//      independent of input order and of catalog file order.
//   7. Package inclusion: the bundled default revision equals the repository
//      manifest default.
//   8. Every entry has a conformance fixture case.
import { readFileSync, readdirSync } from 'node:fs';
import { createRequire } from 'node:module';
import { join, relative } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { CatalogStore, PolicyCatalog, loadCatalogDirectory, bundledCatalogStore } from '../dist/index.js';
import {
  ARCHITECTURES,
  PLATFORMS,
  canonicalSha256,
  checkStoreHistory,
  selectVariant,
  checkAgainstBaseRef,
} from '../dist/tooling.js';

const require = createRequire(import.meta.url);
const Ajv2020 = require('ajv/dist/2020');

const root = fileURLToPath(new URL('../', import.meta.url));
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
  // Same check as `policy-catalog validate --base-ref <ref>`. An unknown ref is
  // an error, never a silent pass.
  const baseRef = process.argv.find(arg => arg.startsWith('--base-ref='))?.slice('--base-ref='.length)
    ?? process.env.POLICY_CATALOG_BASE_REF;
  if (!baseRef) {
    console.log('     (skipped: pass --base-ref=<ref> or set POLICY_CATALOG_BASE_REF to compare against a base ref)');
    return [];
  }
  const { comparedRevisions, errors } = checkAgainstBaseRef(catalogDir, manifest, baseRef);
  console.log(`     (base ref ${baseRef}: ${comparedRevisions} published revision(s) compared)`);
  return errors;
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
          const first = catalog.getSandboxConfigWithDiagnostics(tool, ctx);
          const second = catalog.getSandboxConfigWithDiagnostics(tool, ctx);
          if (first.policy === undefined) {
            found.push(`${revisionId} ${entry.entryId} ${platform}/${architecture} produced no policy: ${first.diagnostics.warnings.join('; ')}`);
          } else if (JSON.stringify(first) !== JSON.stringify(second)) {
            found.push(`${revisionId} ${entry.entryId} ${platform}/${architecture} did not resolve deterministically`);
          } else if (!first.diagnostics.tools[0].matches.some(match => match.entryId === entry.entryId)) {
            found.push(`${revisionId} ${entry.entryId} ${platform}/${architecture} was not matched by its own identity`);
          }
        }
      }
    }
  }
  return found;
});

step('package inclusion', () => {
  // The package ships `catalog/` itself; the bundled loader must see exactly
  // the revisions this pipeline validated.
  const bundled = bundledCatalogStore();
  return bundled.defaultRevision === store.defaultRevision
    && JSON.stringify(bundled.availableRevisions) === JSON.stringify(store.availableRevisions)
    ? []
    : ['bundled catalog does not match the repository catalog'];
});

step('order independence', () => {
  // Catalog file order and input order must not change the composed policy.
  const revision = store.revision();
  const reversed = { ...structuredClone(revision), entries: [...structuredClone(revision.entries)].reverse() };
  const digest = canonicalSha256(reversed);
  const reversedStore = new CatalogStore({
    contract: store.contract,
    manifest: { catalogSchemaVersion: '1', defaultRevision: revision.catalogRevision, revisions: [{ catalogRevision: revision.catalogRevision, file: `revisions/${revision.catalogRevision}.json`, sha256: digest }] },
    readRevision: () => structuredClone(reversed),
  });
  const names = revision.entries.map(e => e.identity.find(p => p.kind === 'invocation-name')?.names[0]).filter(Boolean);
  const found = [];
  for (const platform of PLATFORMS) {
    for (const architecture of ARCHITECTURES) {
      const sep = platform === 'windows' ? '\\' : '/';
      const base = platform === 'windows' ? 'C:\\ci' : '/ci';
      const ctx = {
        platform,
        architecture,
        allowWeakIdentityFallback: true,
        projectRoot: `${base}${sep}project`,
        symbols: Object.fromEntries(Object.entries(store.contract.symbols).filter(([, d]) => d.source !== 'context').map(([s]) => [s, `${base}${sep}${s}`])),
      };
      const run = (catalogStore, tools) => {
        try {
          return JSON.stringify(new PolicyCatalog(catalogStore).getSandboxConfig(tools, ctx));
        } catch (error) {
          return `error:${error.category ?? error.message}`;
        }
      };
      const a = run(store, names);
      const b = run(reversedStore, names);
      const c = run(store, [...names].reverse());
      if (a !== b) found.push(`${platform}/${architecture}: catalog file order changed the result`);
      if (a !== c && !a.startsWith('error:')) {
        // Paths keep first-seen order, so only set equality is required across input order.
        const set = s => JSON.stringify(Object.fromEntries(Object.entries(JSON.parse(s)?.filesystem ?? {}).map(([k, v]) => [k, [...v].sort()])));
        if (set(a) !== set(c)) found.push(`${platform}/${architecture}: input order changed the composed requirement set`);
      }
    }
  }
  return found;
});

step('entry fixtures', () => {
  const fixtureDir = join(root, 'conformance', 'fixtures');
  const covered = new Set();
  for (const name of readdirSync(fixtureDir)) {
    const fixture = readJson(join(fixtureDir, name));
    if (fixture.catalog !== 'bundled') continue;
    for (const c of fixture.cases) {
      for (const tool of c.expect?.diagnostics?.tools ?? []) {
        for (const match of tool.matches) covered.add(match.entryId);
      }
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
