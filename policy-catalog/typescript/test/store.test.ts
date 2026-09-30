// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { PolicyCatalog, bundledCatalogStore } from '../dist/index.js';
import {
  canonicalJson,
  canonicalSha256,
  checkEntryRevisions,
  checkPublishedImmutability,
  checkStoreHistory,
  validateContract,
  validateCatalogRevision,
} from '../dist/tooling.js';
import { contract, entry, errorCategory, fixedHost, revisionWith, storeFor } from './helpers.js';

const parsedContract = validateContract(contract);

describe('integrity', () => {
  it('bundled catalog verifies against its published digest', () => {
    const store = bundledCatalogStore();
    assert.deepEqual(checkStoreHistory(store), []);
    assert.equal(store.revision().catalogRevision, store.defaultRevision);
  });

  it('canonical digest ignores formatting and key order but not content', () => {
    assert.equal(canonicalJson({ b: 1, a: [2, { d: 3, c: 4 }] }), '{"a":[2,{"c":4,"d":3}],"b":1}');
    assert.equal(canonicalSha256({ a: 1, b: 2 }), canonicalSha256({ b: 2, a: 1 }));
    assert.notEqual(canonicalSha256({ a: 1 }), canonicalSha256({ a: 2 }));
  });

  it('a tampered revision fails with an integrity error, never a no-match', () => {
    const revision = revisionWith([entry('tool:a')]);
    const store = storeFor([revision], { digests: { '2000-01-01.1': '0'.repeat(64) } });
    assert.equal(errorCategory(() => store.revision()), 'integrity');
    const catalog = new PolicyCatalog(store, fixedHost());
    assert.equal(errorCategory(() => catalog.resolveCatalogEntry({ invocationName: 'a' }, { allowWeakIdentityFallback: true })), 'integrity');
    assert.equal(errorCategory(() => catalog.getCatalogInfo()), 'integrity');
  });

  it('a revision file that declares a different revision id fails integrity', () => {
    const revision = revisionWith([entry('tool:a')], '2000-01-02.1');
    const digest = canonicalSha256(revision);
    const store = storeFor([{ ...revision, catalogRevision: '2000-01-01.1' }], { digests: { '2000-01-01.1': digest } });
    // The digest was computed for other content, so this is caught as tampering first.
    assert.equal(errorCategory(() => store.revision()), 'integrity');
  });

  it('invalid manifest data is a validation error', () => {
    const revision = revisionWith([entry('tool:a')]);
    assert.equal(errorCategory(() => storeFor([revision], { defaultRevision: '2001-01-01.1' })), 'validation');
    assert.equal(errorCategory(() => storeFor([revisionWith([entry('tool:a')], '2000-01-02.1'), revision])), 'validation');
  });
});

describe('versioning and immutable revisions', () => {
  const r1 = revisionWith([entry('tool:a'), entry('tool:b')], '2000-01-01.1');

  it('selects an explicit installed revision and never substitutes an unavailable one', () => {
    const r2 = revisionWith([entry('tool:a', { entryRevision: 2, displayName: 'renamed' }), entry('tool:b')], '2000-01-02.1');
    const catalog = new PolicyCatalog(storeFor([r1, r2]), fixedHost());
    const ctx = { allowWeakIdentityFallback: true, projectRoot: '/p' };
    assert.equal(catalog.resolveCatalogEntry({ invocationName: 'a' }, ctx)?.entryRevision, 2);
    assert.equal(catalog.resolveCatalogEntry({ invocationName: 'a' }, { ...ctx, catalogRevision: '2000-01-01.1' })?.entryRevision, 1);
    assert.equal(errorCategory(() => catalog.resolveCatalogEntry({ invocationName: 'a' }, { ...ctx, catalogRevision: '2000-01-03.1' })), 'revision-unavailable');
  });

  it('loaded revisions are deeply frozen (read-only)', () => {
    const store = storeFor([r1]);
    const loaded = store.revision();
    assert.ok(Object.isFrozen(loaded.entries[0].platformVariants[0].sandboxPolicy));
    assert.throws(() => { (loaded.entries as any[]).push({}); }, TypeError);
  });

  it('requires entryRevision to increase exactly when an entry changes', () => {
    const changedNoBump = revisionWith([entry('tool:a', { displayName: 'x' }), entry('tool:b')], '2000-01-02.1');
    const bumpedNoChange = revisionWith([entry('tool:a'), entry('tool:b', { entryRevision: 2 })], '2000-01-02.1');
    const ok = revisionWith([entry('tool:a', { displayName: 'x', entryRevision: 2 }), entry('tool:b')], '2000-01-02.1');
    const [a, b, c] = [changedNoBump, bumpedNoChange, ok].map(r => validateCatalogRevision(r, parsedContract));
    const base = validateCatalogRevision(r1, parsedContract);
    assert.match(checkEntryRevisions(base, a).join('\n'), /'tool:a' changed but entryRevision did not increase/);
    assert.match(checkEntryRevisions(base, b).join('\n'), /'tool:b' is unchanged but entryRevision moved/);
    assert.deepEqual(checkEntryRevisions(base, c), []);
    assert.match(checkEntryRevisions(c, base).join('\n'), /must be newer/);
  });

  it('rejects edits to, or removal of, an already-published revision', () => {
    const published = { catalogRevision: '2000-01-01.1', file: 'revisions/2000-01-01.1.json', sha256: 'a'.repeat(64) };
    const base = { manifest: { revisions: [published] }, files: new Map([[published.file, '{"x":1}\n']]) };
    const appended = {
      manifest: { revisions: [published, { catalogRevision: '2000-01-02.1', file: 'revisions/2000-01-02.1.json', sha256: 'b'.repeat(64) }] },
      files: new Map([[published.file, '{"x":1}\n']]),
    };
    assert.deepEqual(checkPublishedImmutability(base, appended), []);
    const edited = { manifest: { revisions: [published] }, files: new Map([[published.file, '{"x":2}\n']]) };
    assert.match(checkPublishedImmutability(base, edited).join('\n'), /was modified; publish a new revision/);
    const redigested = { manifest: { revisions: [{ ...published, sha256: 'c'.repeat(64) }] }, files: base.files };
    assert.match(checkPublishedImmutability(base, redigested).join('\n'), /manifest entry was modified/);
    const removed = { manifest: { revisions: [] }, files: new Map() };
    assert.match(checkPublishedImmutability(base, removed).join('\n'), /removed or reordered/);
  });
});
