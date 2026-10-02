// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert';
import { describe, it } from 'node:test';
import { MxcError } from '../../src/errors.js';
import { findMxcFfiLibrary } from '../../src/native-library.js';
import {
  getCatalogInfo,
  listCatalogEntries,
  resolveSandboxPolicy,
  resolveSandboxPolicyWithDiagnostics,
} from '../../src/policy-store.js';

// The policy store runs in-process through mxc_ffi. Skip, rather than fail,
// where the native library has not been built.
const skip = findMxcFfiLibrary() === null ? 'mxc_ffi is not built' : false;

const context = {
  platform: 'linux' as const,
  architecture: 'x64' as const,
  projectRoot: '/work/repo',
  symbols: {
    node_prefix: '/opt/node',
    npm_prefix: '/opt/npm',
    npm_cache: '/home/u/.npm',
  },
};

describe('policy store (prototype)', { skip }, () => {
  it('reports the bundled catalog', () => {
    const info = getCatalogInfo();
    assert.strictEqual(info.catalogSchemaVersion, '1');
    assert.match(info.catalogRevision, /\S/);
  });

  it('lists entry metadata without policy bodies', () => {
    const entries = listCatalogEntries();
    const ids = entries.map((entry) => entry.entryId).sort();
    assert.ok(ids.includes('tool:npm'), `entries: ${ids.join(', ')}`);
    for (const entry of entries) {
      assert.ok(!('policy' in entry));
      assert.ok(entry.platformVariants.length > 0);
    }
  });

  it('resolves a strong identity to an SDK SandboxPolicy', () => {
    const policy = resolveSandboxPolicy(
      { invocationName: 'npm', packageUrl: 'pkg:npm/npm' },
      context,
    );
    assert.ok(policy !== undefined);
    assert.strictEqual(policy.version, '0.9.0-alpha');
    assert.ok((policy.filesystem?.readonlyPaths?.length ?? 0) > 0);
  });

  it('returns undefined for an unknown tool', () => {
    assert.strictEqual(resolveSandboxPolicy('no-such-tool', context), undefined);
  });

  it('requires the weak-identity opt-in for name-only matches', () => {
    const withoutOptIn = resolveSandboxPolicyWithDiagnostics('git', {
      ...context,
      symbols: { git_prefix: '/usr' },
    });
    assert.strictEqual(withoutOptIn.policy, undefined);

    const withOptIn = resolveSandboxPolicyWithDiagnostics('git', {
      ...context,
      symbols: { git_prefix: '/usr' },
      allowWeakIdentityFallback: true,
    });
    assert.ok(withOptIn.policy !== undefined);
    assert.strictEqual(withOptIn.diagnostics.tools[0].inputIndex, 0);
    assert.strictEqual(withOptIn.diagnostics.tools[0].matches[0].entryId, 'tool:git');
    assert.strictEqual(
      withOptIn.diagnostics.tools[0].matches[0].matchedIdentities[0].strength,
      'weak',
    );
  });

  it('reports dependencies pulled in by a match', () => {
    const resolution = resolveSandboxPolicyWithDiagnostics(
      { invocationName: 'npm', packageUrl: 'pkg:npm/npm' },
      context,
    );
    assert.ok(
      resolution.diagnostics.resolvedDependencies.some((dep) => dep.entryId === 'tool:node'),
    );
  });

  it('throws MxcError with a stable reason for an invalid context', () => {
    assert.throws(
      () =>
        resolveSandboxPolicy('npm', {
          ...context,
          platform: 'plan9' as never,
        }),
      (error: unknown) => {
        assert.ok(error instanceof MxcError);
        assert.strictEqual(error.code, 'malformed_request');
        assert.strictEqual(error.details?.reason, 'invalid_context');
                return true;
      },
    );
  });
});
