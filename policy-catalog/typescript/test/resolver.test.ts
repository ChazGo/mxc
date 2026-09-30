// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  PolicyCatalog,
  getCatalogInfo,
  listCatalogEntries,
  resolveCatalogEntry,
} from '../dist/index.js';
import { bundledCatalog, catalogFor, entry, errorCategory, fixedHost, revisionWith, storeFor } from './helpers.js';

describe('runtime lookup', () => {
  it('omitted context uses host platform/architecture, installed revision, and no weak fallback', () => {
    const host = fixedHost('windows', 'arm64');
    const catalog = catalogFor(revisionWith([
      entry('tool:a', {
        identity: [{ kind: 'purl', value: 'pkg:npm/a' }, { kind: 'invocation-name', names: ['a'] }],
        platformVariants: [
          { when: { platform: 'windows', architecture: 'x64' }, sandboxPolicy: { version: '0.9.0-alpha', timeoutMs: 1 } },
          { when: { platform: 'windows', architecture: 'arm64' }, sandboxPolicy: { version: '0.9.0-alpha', timeoutMs: 2 } },
        ],
      }),
    ]), host);
    assert.equal(catalog.resolveCatalogEntry({ invocationName: 'a' }), undefined, 'weak identity requires opt-in');
    const result = catalog.resolveCatalogEntry({ invocationName: 'a', packageUrl: 'pkg:npm/a' });
    assert.equal(result?.policy.timeoutMs, 2, 'host arm64 selected');
    assert.equal(result?.catalogRevision, '2000-01-01.1');
    const explicit = catalog.resolveCatalogEntry({ invocationName: 'a', packageUrl: 'pkg:npm/a' }, { architecture: 'x64' });
    assert.equal(explicit?.policy.timeoutMs, 1, 'explicit architecture wins');
  });

  it('never fabricates projectRoot or caller symbols', () => {
    const catalog = bundledCatalog();
    assert.equal(catalog.resolveCatalogEntry({ invocationName: 'git' }, { allowWeakIdentityFallback: true }), undefined);
  });

  it('derives host symbols only for the current host platform, and caller values override them', () => {
    const revision = revisionWith([
      entry('tool:t', {
        platformVariants: [
          { when: { platform: 'linux' }, sandboxPolicy: { version: '0.9.0-alpha', filesystem: { readonlyPaths: ['${user_home}/.cfg'] } } },
          { when: { platform: 'macos' }, sandboxPolicy: { version: '0.9.0-alpha', filesystem: { readonlyPaths: ['${user_home}/.cfg'] } } },
        ],
      }),
    ]);
    const catalog = catalogFor(revision, fixedHost('linux', 'x64', { user_home: '/home/me' }));
    const weak = { allowWeakIdentityFallback: true };
    assert.deepEqual(catalog.resolveCatalogEntry({ invocationName: 't' }, weak)?.policy.filesystem, { readonlyPaths: ['/home/me/.cfg'] });
    assert.equal(catalog.resolveCatalogEntry({ invocationName: 't' }, { ...weak, platform: 'macos' }), undefined);
    assert.deepEqual(
      catalog.resolveCatalogEntry({ invocationName: 't' }, { ...weak, symbols: { user_home: '/srv/u' } })?.policy.filesystem,
      { readonlyPaths: ['/srv/u/.cfg'] },
    );
  });

  it('rejects invalid context and inputs as library failures, not no-match', () => {
    const catalog = bundledCatalog();
    assert.equal(errorCategory(() => catalog.resolveCatalogEntry({ invocationName: '' })), 'invalid-context');
    assert.equal(errorCategory(() => catalog.resolveCatalogEntry({ invocationName: '/usr/bin/git' })), 'invalid-context');
    assert.equal(errorCategory(() => catalog.resolveCatalogEntry({ invocationName: 'git' }, { platform: 'plan9' as any })), 'invalid-context');
    assert.equal(errorCategory(() => catalog.resolveCatalogEntry({ invocationName: 'git' }, { symbols: { nope: '/x' } })), 'invalid-context');
    assert.equal(errorCategory(() => catalog.resolveCatalogEntry({ invocationName: 'git' }, { symbols: { project_root: '/x' } })), 'invalid-context');
    assert.equal(errorCategory(() => catalog.resolveCatalogEntry({ invocationName: 'npm', packageUrl: 'npm' })), 'invalid-context');
  });

  it('host detection failure is a library error, not a guessed selection', () => {
    const host = { ...fixedHost(), architecture: (): never => { throw new Error('unknown machine'); } };
    const catalog = new PolicyCatalog(storeFor([revisionWith([entry('tool:t')])]), host);
    assert.throws(() => catalog.resolveCatalogEntry({ invocationName: 't' }, { allowWeakIdentityFallback: true }), /unknown machine/);
  });

  it('is deterministic and returns caller-owned copies', () => {
    const catalog = bundledCatalog();
    const ctx = {
      platform: 'linux' as const,
      architecture: 'x64' as const,
      projectRoot: '/p',
      symbols: { npm_prefix: '/n', npm_cache: '/c', node_prefix: '/n' },
    };
    const tool = { invocationName: 'npm', packageUrl: 'pkg:npm/npm' };
    const first = catalog.resolveCatalogEntry(tool, ctx)!;
    first.policy.filesystem!.readwritePaths!.push('/mutated');
    first.warnings.push('mutated');
    const second = catalog.resolveCatalogEntry(tool, ctx)!;
    assert.deepEqual(second.policy.filesystem, { readonlyPaths: ['/n'], readwritePaths: ['/p', '/c'] });
    assert.deepEqual(second.warnings, []);
  });

  it('de-duplicates normalized paths within an access class using platform path rules', () => {
    const catalog = catalogFor(revisionWith([
      entry('tool:w', {
        platformVariants: [{
          when: { platform: 'windows' },
          sandboxPolicy: { version: '0.9.0-alpha', filesystem: { readonlyPaths: ['${git_prefix}', '${node_prefix}\\'] } },
        }],
      }),
    ]));
    const result = catalog.resolveCatalogEntry(
      { invocationName: 'w' },
      { platform: 'windows', architecture: 'x64', allowWeakIdentityFallback: true, symbols: { git_prefix: 'C:\\Tools', node_prefix: 'c:\\tools' } },
    );
    assert.deepEqual(result?.policy.filesystem, { readonlyPaths: ['C:\\Tools'] });
  });

  it('dependency chain resolution composes each entry once, in declaration order', () => {
    const variant = (deps: string[], path: string) => ({
      when: { platform: 'linux' },
      ...(deps.length ? { dependencies: deps.map(entryId => ({ entryId })) } : {}),
      sandboxPolicy: { version: '0.9.0-alpha', filesystem: { readonlyPaths: [`\${project_root}/${path}`] } },
    });
    const catalog = catalogFor(revisionWith([
      entry('tool:top', { platformVariants: [variant(['tool:left', 'tool:right'], 'top')] }),
      entry('tool:left', { platformVariants: [variant(['tool:leaf'], 'left')] }),
      entry('tool:right', { platformVariants: [variant(['tool:leaf'], 'right')] }),
      entry('tool:leaf', { platformVariants: [variant([], 'leaf')] }),
    ]));
    const result = catalog.resolveCatalogEntry({ invocationName: 'top' }, { allowWeakIdentityFallback: true, projectRoot: '/r' })!;
    assert.deepEqual(result.resolvedDependencies.map(d => d.entryId), ['tool:left', 'tool:leaf', 'tool:right']);
    assert.deepEqual(result.policy.filesystem?.readonlyPaths, ['/r/top', '/r/left', '/r/leaf', '/r/right']);
  });

  it('dependency without a variant for the host makes the entry invalid at validation time', () => {
    const store = storeFor([revisionWith([
      entry('tool:a', { platformVariants: [{ when: { platform: 'linux' }, dependencies: [{ entryId: 'tool:b' }], sandboxPolicy: { version: '0.9.0-alpha' } }] }),
      entry('tool:b', { platformVariants: [{ when: { platform: 'windows' }, sandboxPolicy: { version: '0.9.0-alpha' } }] }),
    ])]);
    assert.throws(() => store.revision(), /unsupported-dependency/);
    assert.equal(errorCategory(() => store.revision()), 'validation');
  });
});

describe('setup and inspection', () => {
  it('reports the bundled revision and matches the manifest default', () => {
    assert.deepEqual(getCatalogInfo(), { catalogSchemaVersion: '1', catalogRevision: '2026-09-29.1' });
  });

  it('lists metadata ordered by entryId without any policy body', () => {
    const entries = listCatalogEntries();
    assert.deepEqual(entries.map(e => e.entryId), ['tool:git', 'tool:node', 'tool:npm']);
    const npm = entries.find(e => e.entryId === 'tool:npm')!;
    assert.deepEqual(npm.platformVariants[0], {
      platform: 'windows',
      dependencyEntryIds: ['tool:node'],
      sandboxPolicyVersion: '0.9.0-alpha',
    });
    assert.ok(!JSON.stringify(entries).includes('readwritePaths'));
    assert.ok(!JSON.stringify(entries).includes('${'));
  });

  it('module-level resolveCatalogEntry uses the bundled catalog', () => {
    assert.equal(resolveCatalogEntry({ invocationName: 'definitely-unknown-tool' }, { allowWeakIdentityFallback: true }), undefined);
  });
});
