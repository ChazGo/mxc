// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  PolicyCatalog,
  getCatalogInfo,
  resolveSandboxPolicy,
  resolveSandboxPolicyWithDiagnostics,
  listCatalogEntries,
} from '@mxc-prototype/policy-catalog';
import { architectureFromMachine } from '@mxc-prototype/policy-catalog/tooling';
import { bundledCatalog, catalogFor, entry, errorReason, fixedHost, revisionWith, storeFor } from './helpers.js';

const v = '0.9.0-alpha';
const weak = { allowWeakIdentityFallback: true } as const;

describe('runtime lookup: defaults (design §1)', () => {
  const archCatalog = (host = fixedHost('windows', 'arm64')) => catalogFor(revisionWith([
    entry('tool:a', {
      identity: [{ kind: 'purl', value: 'pkg:npm/a' }, { kind: 'invocation-name', names: ['a'] }],
      platformVariants: [
        { when: { platform: 'windows', architecture: 'x64' }, sandboxPolicy: { version: v, timeoutMs: 1 } },
        { when: { platform: 'windows', architecture: 'arm64' }, sandboxPolicy: { version: v, timeoutMs: 2 } },
      ],
    }),
  ]), host);

  it('omitted context: host platform, native architecture, installed revision, no weak fallback', () => {
    const catalog = archCatalog();
    assert.equal(catalog.resolveSandboxPolicy('a'), undefined, 'weak identity requires opt-in');
    const result = catalog.resolveSandboxPolicyWithDiagnostics({ invocationName: 'a', packageUrl: 'pkg:npm/a' });
    assert.equal(result.policy?.timeoutMs, 2, 'native arm64 selected');
    assert.equal(result.diagnostics.catalogRevision, '2000-01-01.1');
    assert.match(result.diagnostics.warnings.join('\n'), /native system architecture 'arm64'; the tool's architecture was not verified/);
  });

  it('explicit architecture takes precedence and suppresses the host-default warning', () => {
    const result = archCatalog().resolveSandboxPolicyWithDiagnostics({ invocationName: 'a', packageUrl: 'pkg:npm/a' }, { architecture: 'x64' });
    assert.equal(result.policy?.timeoutMs, 1);
    assert.deepEqual(result.diagnostics.warnings, []);
  });

  it('a different architecture variant is never a fallback', () => {
    const catalog = catalogFor(revisionWith([
      entry('tool:a', { platformVariants: [{ when: { platform: 'linux', architecture: 'arm64' }, sandboxPolicy: { version: v } }] }),
    ]));
    const result = catalog.resolveSandboxPolicyWithDiagnostics('a', { ...weak, architecture: 'x64' });
    assert.equal(result.policy, undefined);
    assert.match(result.diagnostics.warnings[0], /tool:a has no variant for linux\/x64/);
  });

  it('host architecture detection failure is a library error, not a guessed selection', () => {
    const host = { ...fixedHost(), nativeArchitecture: (): never => { throw new Error('unknown machine'); } };
    const catalog = new PolicyCatalog(storeFor([revisionWith([entry('tool:t')])]), host);
    assert.throws(() => catalog.resolveSandboxPolicy('t', { ...weak, projectRoot: '/p' }), /unknown machine/);
    // Detection is not needed when nothing matches, so it is not attempted.
    assert.equal(catalog.resolveSandboxPolicy('nothing', weak), undefined);
    // An explicit architecture never consults the host.
    assert.deepEqual(catalog.resolveSandboxPolicy('t', { ...weak, architecture: 'x64', projectRoot: '/p' })?.filesystem, { readwritePaths: ['/p'] });
  });

  it('maps OS machine names to selectors and rejects unknown ones', () => {
    assert.equal(architectureFromMachine('AMD64'), 'x64');
    assert.equal(architectureFromMachine('x86_64'), 'x64');
    assert.equal(architectureFromMachine('ARM64'), 'arm64');
    assert.equal(architectureFromMachine('aarch64'), 'arm64');
    assert.equal(architectureFromMachine('riscv64'), undefined);
    assert.equal(architectureFromMachine('x86'), undefined);
  });

  it('never fabricates projectRoot or caller symbols', () => {
    const result = bundledCatalog().resolveSandboxPolicyWithDiagnostics('git', { ...weak, architecture: 'x64' });
    assert.equal(result.policy, undefined);
    assert.match(result.diagnostics.warnings.join('\n'), /required symbol 'git_prefix'.*required symbol 'project_root'/s);
  });

  it('derives host symbols only for the current host platform, and caller values override them', () => {
    const revision = revisionWith([
      entry('tool:t', {
        platformVariants: [
          { when: { platform: 'linux' }, sandboxPolicy: { version: v, filesystem: { readonlyPaths: ['${user_home}/.cfg'] } } },
          { when: { platform: 'macos' }, sandboxPolicy: { version: v, filesystem: { readonlyPaths: ['${user_home}/.cfg'] } } },
        ],
      }),
    ]);
    const catalog = catalogFor(revision, fixedHost('linux', 'x64', { user_home: '/home/me' }));
    assert.deepEqual(catalog.resolveSandboxPolicy('t', weak)?.filesystem, { readonlyPaths: ['/home/me/.cfg'] });
    assert.equal(catalog.resolveSandboxPolicy('t', { ...weak, platform: 'macos', architecture: 'arm64' }), undefined);
    assert.deepEqual(catalog.resolveSandboxPolicy('t', { ...weak, symbols: { user_home: '/srv/u' } })?.filesystem, { readonlyPaths: ['/srv/u/.cfg'] });
  });
});

describe('runtime lookup: inputs and failures (design §5.1)', () => {
  it('string shorthand, object input, and a one-element array are equivalent', () => {
    const catalog = bundledCatalog();
    const ctx = { ...weak, architecture: 'x64' as const, projectRoot: '/p', symbols: { git_prefix: '/g' } };
    const a = catalog.resolveSandboxPolicyWithDiagnostics('git', ctx);
    assert.deepEqual(catalog.resolveSandboxPolicyWithDiagnostics({ invocationName: 'git' }, ctx), a);
    assert.deepEqual(catalog.resolveSandboxPolicyWithDiagnostics(['git'], ctx), a);
    assert.equal(a.diagnostics.tools[0].inputIndex, 0);
  });

  it('string shorthand obeys the weak-identity option exactly like an object input', () => {
    const catalog = bundledCatalog();
    const ctx = { architecture: 'x64' as const, projectRoot: '/p', symbols: { git_prefix: '/g' } };
    assert.equal(catalog.resolveSandboxPolicy('git', ctx), undefined);
    assert.equal(catalog.resolveSandboxPolicy({ invocationName: 'git' }, ctx), undefined);
  });

  it('rejects invalid context and inputs as library failures, not absence', () => {
    const catalog = bundledCatalog();
    const cases: Array<[unknown, unknown]> = [
      ['', {}],
      ['/usr/bin/git', {}],
      [{ invocationName: 'npm', packageUrl: 'npm' }, {}],
      [{ invocationName: 'npm', detectedVersion: '' }, {}],
      [[42], {}],
      ['git', { platform: 'plan9' }],
      ['git', { architecture: 'x86' }],
      ['git', { symbols: { nope: '/x' } }],
      ['git', { symbols: { project_root: '/x' } }],
      ['git', { projectRoot: '' }],
    ];
    for (const [tools, ctx] of cases) {
      assert.equal(errorReason(() => catalog.resolveSandboxPolicy(tools as any, ctx as any)), 'invalid_context', JSON.stringify([tools, ctx]));
    }
  });

  it('is deterministic and returns caller-owned copies', () => {
    const catalog = bundledCatalog();
    const ctx = { architecture: 'x64' as const, projectRoot: '/p', symbols: { npm_prefix: '/n', npm_cache: '/c', node_prefix: '/n' } };
    const tool = { invocationName: 'npm', packageUrl: 'pkg:npm/npm' };
    const first = catalog.resolveSandboxPolicyWithDiagnostics(tool, ctx);
    first.policy!.filesystem!.readwritePaths!.push('/mutated');
    first.diagnostics.warnings.push('mutated');
    first.diagnostics.tools[0].matches.length = 0;
    const second = catalog.resolveSandboxPolicyWithDiagnostics(tool, ctx);
    assert.deepEqual(second.policy?.filesystem, { readonlyPaths: ['/n'], readwritePaths: ['/p', '/c'] });
    assert.equal(second.diagnostics.tools[0].matches.length, 1);
    assert.ok(!second.diagnostics.warnings.includes('mutated'));
  });

  it('de-duplicates normalized paths within an access class using platform path rules', () => {
    const catalog = catalogFor(revisionWith([
      entry('tool:w', {
        platformVariants: [{ when: { platform: 'windows' }, sandboxPolicy: { version: v, filesystem: { readonlyPaths: ['${git_prefix}', '${node_prefix}\\'] } } }],
      }),
    ]));
    const policy = catalog.resolveSandboxPolicy('w', { platform: 'windows', architecture: 'x64', ...weak, symbols: { git_prefix: 'C:\\Tools', node_prefix: 'c:\\tools' } });
    assert.deepEqual(policy?.filesystem, { readonlyPaths: ['C:\\Tools'] });
    // Linux paths are case-sensitive, so the same values stay distinct there.
    const linux = catalogFor(revisionWith([
      entry('tool:l', { platformVariants: [{ when: { platform: 'linux' }, sandboxPolicy: { version: v, filesystem: { readonlyPaths: ['${git_prefix}', '${node_prefix}/'] } } }] }),
    ]));
    assert.deepEqual(linux.resolveSandboxPolicy('l', { ...weak, symbols: { git_prefix: '/Tools', node_prefix: '/tools' } })?.filesystem, { readonlyPaths: ['/Tools', '/tools'] });
  });

  it('macOS folds path case like Windows (one casing rule per OS)', () => {
    const mac = catalogFor(revisionWith([
      entry('tool:m', { platformVariants: [{ when: { platform: 'macos' }, sandboxPolicy: { version: v, filesystem: { readonlyPaths: ['${git_prefix}', '${node_prefix}/'] } } }] }),
    ]));
    const ctx = { platform: 'macos', architecture: 'arm64', ...weak, symbols: { git_prefix: '/Tools', node_prefix: '/tools' } } as const;
    assert.deepEqual(mac.resolveSandboxPolicy('m', ctx)?.filesystem, { readonlyPaths: ['/Tools'] });
    // Overlap across access classes is also detected case-insensitively on macOS.
    const overlap = catalogFor(revisionWith([
      entry('tool:m', { platformVariants: [{ when: { platform: 'macos' }, sandboxPolicy: { version: v, filesystem: { readonlyPaths: ['${git_prefix}'], readwritePaths: ['${project_root}'] } } }] }),
    ]));
    assert.equal(errorReason(() => overlap.resolveSandboxPolicy('m', { ...ctx, projectRoot: '/tools/work' })), 'composition_conflict');
    // The same values on Linux are distinct paths, so there is no overlap.
    const linux = catalogFor(revisionWith([
      entry('tool:m', { platformVariants: [{ when: { platform: 'linux' }, sandboxPolicy: { version: v, filesystem: { readonlyPaths: ['${git_prefix}'], readwritePaths: ['${project_root}'] } } }] }),
    ]));
    assert.deepEqual(linux.resolveSandboxPolicy('m', { ...weak, platform: 'linux', architecture: 'x64', projectRoot: '/tools/work', symbols: { git_prefix: '/Tools' } })?.filesystem,
      { readonlyPaths: ['/Tools'], readwritePaths: ['/tools/work'] });
  });

  it('invocation names: Linux is exact (gh is not GH); Windows and macOS fold case', () => {
    const catalog = catalogFor(revisionWith([
      entry('tool:gh', { platformVariants: ['linux', 'macos', 'windows'].map(platform => ({ when: { platform }, sandboxPolicy: { version: v } })) }),
    ]));
    const on = (platform: 'linux' | 'macos' | 'windows', name: string) =>
      catalog.resolveSandboxPolicyWithDiagnostics(name, { ...weak, platform, architecture: 'x64' }).diagnostics.tools[0].matches.map(m => m.entryId);
    assert.deepEqual(on('linux', 'gh'), ['tool:gh']);
    assert.deepEqual(on('linux', 'GH'), []);
    assert.deepEqual(on('macos', 'GH'), ['tool:gh']);
    assert.deepEqual(on('windows', 'Gh'), ['tool:gh']);
  });

  it('dependency chain resolution composes each entry once, in deterministic order', () => {
    const variant = (deps: string[], path: string) => ({
      when: { platform: 'linux' },
      ...(deps.length ? { dependencies: deps.map(entryId => ({ entryId })) } : {}),
      sandboxPolicy: { version: v, filesystem: { readonlyPaths: [`\${project_root}/${path}`] } },
    });
    const catalog = catalogFor(revisionWith([
      entry('tool:top', { platformVariants: [variant(['tool:left', 'tool:right'], 'top')] }),
      entry('tool:left', { platformVariants: [variant(['tool:leaf'], 'left')] }),
      entry('tool:right', { platformVariants: [variant(['tool:leaf'], 'right')] }),
      entry('tool:leaf', { platformVariants: [variant([], 'leaf')] }),
    ]));
    const result = catalog.resolveSandboxPolicyWithDiagnostics('top', { ...weak, projectRoot: '/r' });
    assert.deepEqual(result.diagnostics.resolvedDependencies.map(d => d.entryId), ['tool:leaf', 'tool:left', 'tool:right']);
    assert.deepEqual(result.policy?.filesystem?.readonlyPaths, ['/r/top', '/r/left', '/r/leaf', '/r/right']);
  });

  it('dependency diagnostics retain distinct required ranges, sorted', () => {
    const catalog = catalogFor(revisionWith([
      entry('tool:a', { platformVariants: [{ when: { platform: 'linux' }, dependencies: [{ entryId: 'tool:c', versionRange: '>=2' }], sandboxPolicy: { version: v } }] }),
      entry('tool:b', { platformVariants: [{ when: { platform: 'linux' }, dependencies: [{ entryId: 'tool:c' }], sandboxPolicy: { version: v } }] }),
      entry('tool:c', { platformVariants: [{ when: { platform: 'linux' }, sandboxPolicy: { version: v } }] }),
    ]));
    const result = catalog.resolveSandboxPolicyWithDiagnostics(['a', 'b', 'a'], weak);
    assert.deepEqual(result.diagnostics.resolvedDependencies, [
      { entryId: 'tool:c', entryRevision: 1 },
      { entryId: 'tool:c', entryRevision: 1, requiredVersionRange: '>=2' },
    ]);
    assert.deepEqual(result.policy, { version: v });
  });

  it('catalog file order does not change matching, attribution, or composition', () => {
    const entries = [
      entry('tool:b', { identity: [{ kind: 'invocation-name', names: ['x'] }], platformVariants: [{ when: { platform: 'linux' }, sandboxPolicy: { version: v, filesystem: { readonlyPaths: ['${project_root}/b'] } } }] }),
      entry('tool:a', { identity: [{ kind: 'invocation-name', names: ['x'] }], platformVariants: [{ when: { platform: 'linux' }, sandboxPolicy: { version: v, filesystem: { readonlyPaths: ['${project_root}/a'] } } }] }),
    ];
    const forward = catalogFor(revisionWith(entries)).resolveSandboxPolicyWithDiagnostics('x', { ...weak, projectRoot: '/r' });
    const backward = catalogFor(revisionWith([...entries].reverse())).resolveSandboxPolicyWithDiagnostics('x', { ...weak, projectRoot: '/r' });
    assert.deepEqual(forward, backward);
    assert.deepEqual(forward.diagnostics.tools[0].matches.map(m => m.entryId), ['tool:a', 'tool:b']);
    assert.deepEqual(forward.policy?.filesystem?.readonlyPaths, ['/r/a', '/r/b']);
  });

  it('rejects mixed policy versions across inputs at lookup time', () => {
    const catalog = catalogFor(revisionWith([
      entry('tool:a', { platformVariants: [{ when: { platform: 'linux' }, sandboxPolicy: { version: '0.8.0-alpha' } }] }),
      entry('tool:b', { platformVariants: [{ when: { platform: 'linux' }, sandboxPolicy: { version: v } }] }),
    ]));
    assert.equal(errorReason(() => catalog.resolveSandboxPolicy(['a', 'b'], weak)), 'composition_conflict');
    // Each tool on its own is fine.
    assert.deepEqual(catalog.resolveSandboxPolicy('a', weak), { version: '0.8.0-alpha' });
  });
});

describe('setup and inspection (design §5.2)', () => {
  it('reports the bundled revision, which matches the manifest default', () => {
    assert.deepEqual(getCatalogInfo(), { catalogSchemaVersion: '1', catalogRevision: '2026-09-29.1' });
  });

  it('lists metadata ordered by entryId, without any policy body', () => {
    const entries = listCatalogEntries();
    assert.deepEqual(entries.map(e => e.entryId), ['tool:git', 'tool:node', 'tool:npm']);
    const npm = entries.find(e => e.entryId === 'tool:npm')!;
    assert.deepEqual(npm.platformVariants[0], { platform: 'windows', dependencyEntryIds: ['tool:node'], sandboxPolicyVersion: v });
    const text = JSON.stringify(entries);
    assert.ok(!text.includes('Paths'));
    assert.ok(!text.includes('${'));
    assert.ok(!text.includes('sandboxPolicy"'));
  });

  it('module-level functions use the bundled catalog', () => {
    const ctx = { ...weak, platform: 'linux' as const, architecture: 'x64' as const };
    assert.equal(resolveSandboxPolicy('definitely-unknown-tool', ctx), undefined);
    const result = resolveSandboxPolicyWithDiagnostics(['definitely-unknown-tool'], ctx);
    assert.equal(result.policy, undefined);
    assert.deepEqual(result.diagnostics.tools, [{ inputIndex: 0, matches: [] }]);
  });
});
