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
      assert.ok(!('sandboxPolicy' in entry.default));
      assert.match(entry.default.sandboxPolicyVersion, /\S/);
    }
    const git = entries.find((entry) => entry.entryId === 'tool:git');
    assert.ok(git !== undefined);
    assert.strictEqual(git.versionScheme, 'intdot');
    assert.deepStrictEqual(
      git.default.intents.map((intent) => intent.name).sort(),
      ['fetch', 'local', 'push'],
    );
    assert.deepStrictEqual(
      git.versionVariants.map((variant) => variant.versionRange),
      ['vers:intdot/>=2.40|<2.50', 'vers:intdot/>=2.50|<3'],
    );
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

  const git = (detectedVersion: string | undefined, intent: string | undefined) => ({
    invocationName: 'git',
    packageUrl: 'pkg:generic/git',
    ...(detectedVersion === undefined ? {} : { detectedVersion }),
    ...(intent === undefined ? {} : { intent }),
  });
  const gitContext = {
    ...context,
    symbols: { git_prefix: '/usr/bin', ssh_prefix: '/usr/lib/ssh', temp_dir: '/tmp' },
  };

  it('selects a version range and an intent', () => {
    const push = resolveSandboxPolicyWithDiagnostics(git('2.45.1', 'push'), gitContext);
    const tool = push.diagnostics.tools[0];
    assert.strictEqual(tool.status, 'matched_version');
    assert.strictEqual(tool.matches[0].versionSelection.selectedVersionRange, 'vers:intdot/>=2.40|<2.50');
    assert.deepStrictEqual(tool.matches[0].intentSelection, {
      requested: 'push',
      mode: 'named',
      selected: ['push'],
    });
    assert.strictEqual(push.diagnostics.resolvedDependencies[0].entryId, 'tool:ssh');
    assert.deepStrictEqual(push.policy?.filesystem?.readonlyPaths, ['/usr/bin', '/usr/lib/ssh']);
    assert.strictEqual(push.policy?.network?.egress?.allow?.length, 1);
  });

  it('reports out-of-range, unparseable, and unsupported pairs with structured warnings', () => {
    const outOfRange = resolveSandboxPolicyWithDiagnostics(git('2.30.0', 'fetch'), gitContext);
    assert.strictEqual(outOfRange.diagnostics.tools[0].status, 'version_out_of_range');
    assert.ok(outOfRange.policy !== undefined);
    const warning = outOfRange.diagnostics.warnings[0];
    assert.ok(typeof warning === 'object');
    assert.strictEqual(warning.code, 'version_out_of_range');
    assert.strictEqual(warning.entryId, 'tool:git');

    const unparseable = resolveSandboxPolicyWithDiagnostics(git('banana', 'fetch'), gitContext);
    assert.strictEqual(unparseable.diagnostics.tools[0].status, 'version_unparseable');
    assert.strictEqual(unparseable.policy, undefined);

    const unsupported = resolveSandboxPolicyWithDiagnostics(
      git('2.45.1', 'bundle-fetch'),
      gitContext,
    );
    assert.strictEqual(unsupported.diagnostics.tools[0].status, 'intent_unsupported');
    assert.strictEqual(
      unsupported.diagnostics.tools[0].matches[0].versionSelection.status,
      'matched_version',
    );
    assert.strictEqual(unsupported.policy, undefined);
  });

  it('composes pairs: a tool without network does not veto another', () => {
    const resolution = resolveSandboxPolicyWithDiagnostics(
      [git(undefined, 'local'), git(undefined, 'fetch'), 'no-such-tool'],
      gitContext,
    );
    assert.strictEqual(resolution.policy?.network?.egress?.allow?.length, 1);
    assert.strictEqual(resolution.diagnostics.tools[2].status, 'tool_unmatched');
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
    assert.throws(
      () => resolveSandboxPolicy(git(undefined, ''), gitContext),
      (error: unknown) => error instanceof MxcError && error.details?.reason === 'invalid_context',
    );
  });
});
