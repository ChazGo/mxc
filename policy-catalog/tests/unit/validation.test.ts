// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { validateCatalogRevision, validateContract } from '@mxc-prototype/policy-catalog/tooling';
import { contract, entry, revisionWith } from './helpers.js';

const parsedContract = validateContract(contract);
const validate = (entries: any[]) => () => validateCatalogRevision(revisionWith(entries), parsedContract);
const linux = (sandboxPolicy: any, extra: Record<string, unknown> = {}) => ({ when: { platform: 'linux' }, ...extra, sandboxPolicy });
const v = '0.9.0-alpha';

describe('catalog validation (contribution/CI rules)', () => {
  it('accepts a minimal valid revision', () => {
    assert.doesNotThrow(validate([entry('tool:a')]));
  });

  it('rejects dependency cycles, including indirect ones', () => {
    const dep = (id: string) => [linux({ version: v }, { dependencies: [{ entryId: id }] })];
    assert.throws(validate([
      entry('tool:a', { platformVariants: dep('tool:b') }),
      entry('tool:b', { platformVariants: dep('tool:c') }),
      entry('tool:c', { platformVariants: dep('tool:a') }),
    ]), /cycle \(tool:a -> tool:b -> tool:c -> tool:a\)/);
    assert.throws(validate([entry('tool:a', { platformVariants: dep('tool:a') })]), /depends on itself/);
  });

  it('rejects dependencies outside the same catalog revision', () => {
    assert.throws(validate([entry('tool:a', { platformVariants: [linux({ version: v }, { dependencies: [{ entryId: 'tool:missing' }] })] })]), /unknown entry 'tool:missing'/);
  });

  it('rejects syntactically invalid dependency version ranges but never evaluates them', () => {
    assert.throws(validate([
      entry('tool:a', { platformVariants: [linux({ version: v }, { dependencies: [{ entryId: 'tool:b', versionRange: '^1.x' }] })] }),
      entry('tool:b'),
    ]), /versionRange/);
  });

  it('rejects duplicate exact selectors and a second architecture-neutral variant', () => {
    const variant = (architecture?: string) => ({ when: { platform: 'linux', ...(architecture ? { architecture } : {}) }, sandboxPolicy: { version: v } });
    assert.throws(validate([entry('tool:a', { platformVariants: [variant('x64'), variant('x64')] })]), /duplicates selector/);
    assert.throws(validate([entry('tool:a', { platformVariants: [variant(), variant()] })]), /second architecture-neutral/);
    assert.doesNotThrow(validate([entry('tool:a', { platformVariants: [variant(), variant('x64'), variant('arm64')] })]));
    assert.throws(validate([entry('tool:a', { platformVariants: [{ when: { platform: 'linux', architecture: 'riscv' }, sandboxPolicy: { version: v } }] })]), /architecture/);
  });

  it('rejects variants that name a containment backend', () => {
    assert.throws(validate([entry('tool:a', { platformVariants: [linux({ version: v, processContainer: {} })] })]), /containment backend/);
    assert.throws(validate([entry('tool:a', { platformVariants: [linux({ version: v, containment: 'lxc' })] })]), /containment backend/);
  });

  it('requires exact registered SandboxPolicy versions', () => {
    assert.throws(validate([entry('tool:a', { platformVariants: [linux({ version: '0.9.0' })] })]), /not a SandboxPolicy version registered/);
    assert.throws(validate([entry('tool:a', { platformVariants: [linux({ version: '0.10.0-alpha' })] })]), /not a SandboxPolicy version registered/);
  });

  it('rejects unsupported fields at every level', () => {
    assert.throws(validate([entry('tool:a', { extra: 1 })]), /unsupported field/);
    assert.throws(validate([entry('tool:a', { platformVariants: [linux({ version: v, filesystem: { clearPolicyOnExit: true } })] })]), /unsupported field/);
    assert.throws(validate([entry('tool:a', { platformVariants: [linux({ version: v, network: { allowOutbound: true } })] })]), /unsupported field/);
    assert.throws(validate([entry('tool:a', { platformVariants: [linux({ version: v, telemetry: {} })] })]), /unsupported field/);
  });

  it('rejects literal, user-specific, wildcard, traversal, and unknown-symbol paths', () => {
    const path = (p: string) => validate([entry('tool:a', { platformVariants: [linux({ version: v, filesystem: { readonlyPaths: [p] } })] })]);
    assert.throws(path('/home/alice/.npm'), /literal paths are not allowed/);
    assert.throws(path('C:\\Users\\alice'), /literal paths are not allowed/);
    assert.throws(path('/usr/lib'), /literal paths are not allowed/);
    assert.throws(path('${project_root}/*'), /wildcard/);
    assert.throws(path('${project_root}/../x'), /'\.\.'/);
    assert.throws(path('${unknown_thing}'), /unknown symbol/);
    assert.throws(path('${project_root'), /malformed symbol/);
    assert.doesNotThrow(path('${project_root}/node_modules'));
  });

  it('rejects wildcard network grants', () => {
    const net = (network: any) => validate([entry('tool:a', { platformVariants: [linux({ version: v, network })] })]);
    assert.throws(net({ egress: { default: 'allow' } }), /default-allow/);
    assert.throws(net({ ingress: { default: 'allow' } }), /default-allow/);
    assert.throws(net({ egress: { allow: [{ ports: [{ port: 443 }] }] } }), /wildcard network grant/);
    assert.throws(net({ egress: { allow: [{ to: [{ cidr: '0.0.0.0/0' }] }] } }), /wildcard network grant/);
    assert.doesNotThrow(net({ egress: { default: 'deny', allow: [{ to: [{ cidr: '192.0.2.0/24' }], ports: [{ protocol: 'tcp', port: 443 }] }] } }));
  });

  it('allows overlapping identities across entries (additive matching) but not repeats within one entry', () => {
    // Design §4.3: equal-strength matches to different entries both contribute.
    assert.doesNotThrow(validate([entry('tool:a'), entry('tool:b', { identity: [{ kind: 'invocation-name', names: ['A'] }] })]));
    assert.doesNotThrow(validate([
      entry('tool:a', { identity: [{ kind: 'purl', value: 'pkg:npm/x' }] }),
      entry('tool:b', { identity: [{ kind: 'purl', value: 'pkg:NPM/x' }] }),
    ]));
    // Predicate order within an entry carries no precedence.
    assert.doesNotThrow(validate([entry('tool:a', { identity: [{ kind: 'invocation-name', names: ['a'] }, { kind: 'purl', value: 'pkg:npm/a' }] })]));
    assert.throws(validate([entry('tool:a', { identity: [{ kind: 'invocation-name', names: ['a', 'A'] }] })]), /repeats identity/);
    assert.throws(validate([entry('tool:a', { identity: [{ kind: 'purl', value: 'pkg:npm/a' }, { kind: 'purl', value: 'pkg:npm/a' }] })]), /repeats identity/);
    assert.throws(validate([entry('tool:a', { identity: [{ kind: 'purl', value: 'pkg:npm/a@1.0.0' }] })]), /must not pin a version/);
    assert.throws(validate([entry('tool:a', { identity: [{ kind: 'invocation-name', names: ['bin/a'] }] })]), /bare invocation name/);
    assert.throws(validate([entry('tool:a', { identity: [{ kind: 'sha256', value: 'x' }] })]), /not a supported identity kind/);
    assert.throws(validate([entry('tool:a'), entry('tool:a', { identity: [{ kind: 'invocation-name', names: ['z'] }] })]), /duplicate entryId/);
    assert.throws(validate([entry('tool:a', { entryId: 'noNamespace' })]), /namespaced/);
  });

  describe('v1 composition vocabulary', () => {
    const withDep = (policy: any, depPolicy: any) => validate([
      entry('tool:a', { platformVariants: [linux(policy, { dependencies: [{ entryId: 'tool:b' }] })] }),
      entry('tool:b', { platformVariants: [linux(depPolicy)] }),
    ]);

    it('composes filesystem path classes and same-class duplicates', () => {
      assert.doesNotThrow(withDep(
        { version: v, filesystem: { readwritePaths: ['${project_root}'], readonlyPaths: ['${node_prefix}'] } },
        { version: v, filesystem: { readonlyPaths: ['${node_prefix}', '${git_prefix}'], deniedPaths: ['${user_home}/.ssh'] } },
      ));
    });

    it('rejects cross-class equal or ancestor/descendant paths', () => {
      assert.throws(withDep(
        { version: v, filesystem: { readwritePaths: ['${project_root}'] } },
        { version: v, filesystem: { readonlyPaths: ['${project_root}'] } },
      ), /overlaps/);
      assert.throws(withDep(
        { version: v, filesystem: { readwritePaths: ['${project_root}'] } },
        { version: v, filesystem: { deniedPaths: ['${project_root}/secrets'] } },
      ), /overlaps/);
    });

    it('rejects mixed sandboxPolicy versions', () => {
      assert.throws(withDep({ version: v }, { version: '0.8.0-alpha' }), /mixed sandboxPolicy\.version/);
    });

    it('rejects network and other fields without a cross-entry rule, but allows them on standalone entries', () => {
      const network = { egress: { default: 'deny' } };
      assert.throws(withDep({ version: v, network }, { version: v }), /'network'/);
      assert.throws(withDep({ version: v }, { version: v, timeoutMs: 5 }), /'timeoutMs'/);
      assert.throws(withDep({ version: v }, { version: v, ui: { clipboard: 'read' } }), /'ui'/);
      assert.doesNotThrow(validate([entry('tool:a', { platformVariants: [linux({ version: v, network, ui: { clipboard: 'read' }, timeoutMs: 5 })] })]));
    });
  });
});
