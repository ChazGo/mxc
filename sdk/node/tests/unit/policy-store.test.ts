// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert';
import { mkdirSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { after, describe, it } from 'node:test';
import { MxcError } from '../../src/v1/errors.js';
import { findMxcFfiLibrary } from '../../src/native-library.js';
import type { ContainerRequest } from '../../src/v1/types.js';
import {
  getCatalogInfo,
  listCatalogEntries,
  resolveToolRequirements,
  resolveToolRequirementsWithDiagnostics,
  type ContainerRequirements,
  type ResolveContext,
  type ToolCandidate,
} from '../../src/v1/index.js';
import { validateResolution } from '../../src/v1/tool-requirements.js';

// The policy store runs in-process through mxc_ffi. Skip, rather than fail,
// where the native library has not been built.
const skip = findMxcFfiLibrary() === null ? 'mxc_ffi is not built' : false;

// Lookups target the current host, whose filesystem object identity is
// examined, so every symbol names an existing directory.
const root = mkdtempSync(join(tmpdir(), 'mxc-policy-store-'));
const dir = (...parts: string[]): string => {
  const path = join(root, ...parts);
  mkdirSync(path, { recursive: true });
  return path;
};
const paths = {
  project: dir('repo'),
  node: dir('node'),
  npm: dir('npm'),
  npmCache: dir('npm-cache'),
  git: dir('git'),
  ssh: dir('ssh'),
  temp: dir('temp'),
  programData: dir('pd'),
};
dir('pd', 'Git');

const context: ResolveContext = {
  projectRoot: paths.project,
  symbols: {
    node_prefix: paths.node,
    npm_prefix: paths.npm,
    npm_cache: paths.npmCache,
    git_prefix: paths.git,
    ssh_prefix: paths.ssh,
    temp_dir: paths.temp,
    programData: paths.programData,
  },
};

const git = (detectedVersion?: string, intent?: string): ToolCandidate => ({
  invocationName: 'git',
  packageUrl: 'pkg:generic/git',
  ...(detectedVersion === undefined ? {} : { detectedVersion }),
  ...(intent === undefined ? {} : { intent }),
});

describe('policy store (prototype)', { skip }, () => {
  after(() => rmSync(root, { recursive: true, force: true }));

  it('reports the bundled catalog and SDK contract version', () => {
    const info = getCatalogInfo();
    assert.strictEqual(info.catalogSchemaVersion, '1');
    assert.strictEqual(info.sdkContractVersion, '1.0.0');
    assert.match(info.catalogRevision, /\S/);
  });

  it('lists entry metadata without requirements bodies', () => {
    const entries = listCatalogEntries();
    const ids = entries.map((entry) => entry.entryId).sort();
    assert.ok(ids.includes('tool:npm'), `entries: ${ids.join(', ')}`);
    for (const entry of entries) {
      assert.ok(!('requirements' in entry.default));
    }
    const gitEntry = entries.find((entry) => entry.entryId === 'tool:git');
    assert.ok(gitEntry !== undefined);
    assert.strictEqual(gitEntry.versionScheme, 'intdot');
    assert.deepStrictEqual(
      gitEntry.default.intents.map((intent) => intent.name).sort(),
      ['fetch', 'local', 'push'],
    );
    assert.deepStrictEqual(
      gitEntry.versionVariants.map((variant) => variant.versionRange),
      ['vers:intdot/>=2.40|<2.50', 'vers:intdot/>=2.50|<3'],
    );
    assert.deepStrictEqual(
      gitEntry.versionVariants[1].newIntents.map((intent) => intent.name),
      ['bundle-fetch'],
    );
  });

  it('resolves command-free requirements that combine with a command', async () => {
    const pending = resolveToolRequirements(
      { invocationName: 'npm', packageUrl: 'pkg:npm/npm' },
      context,
    );
    assert.ok(pending instanceof Promise);
    const requirements = await pending;
    assert.ok(requirements !== undefined);
    assert.ok(!('command' in requirements));
    assert.ok(!('version' in requirements));
    assert.ok((requirements.filesystem?.readonlyPaths?.length ?? 0) > 0);
    const approved: ContainerRequirements = requirements;
    const request: ContainerRequest = { ...approved, command: 'npm --version' };
    assert.strictEqual(request.command, 'npm --version');
  });

  it('returns undefined for an unknown tool', async () => {
    assert.strictEqual(await resolveToolRequirements('no-such-tool', context), undefined);
  });

  it('requires the weak-identity opt-in for name-only matches', async () => {
    const withoutOptIn = await resolveToolRequirementsWithDiagnostics('git', context);
    assert.strictEqual(withoutOptIn.requirements, undefined);

    const withOptIn = await resolveToolRequirementsWithDiagnostics(['git'], {
      ...context,
      allowWeakIdentityFallback: true,
    });
    assert.ok(withOptIn.requirements !== undefined);
    const [tool] = withOptIn.diagnostics.tools;
    assert.strictEqual(tool.inputIndex, 0);
    assert.strictEqual(tool.contributes, true);
    assert.strictEqual(tool.selection?.entryId, 'tool:git');
    assert.strictEqual(tool.selection?.matchedIdentities[0].strength, 'weak');
    const weak = withOptIn.diagnostics.warnings.find((w) => w.code === 'weak_identity');
    assert.ok(weak !== undefined && weak.code === 'weak_identity');
    assert.strictEqual(weak.invocationName, 'git');
  });

  it('selects a version range and an intent, attributing dependencies', async () => {
    const push = await resolveToolRequirementsWithDiagnostics(git('2.45.1', 'push'), context);
    const [tool] = push.diagnostics.tools;
    assert.strictEqual(tool.status, 'matched_version');
    assert.strictEqual(
      tool.selection?.versionSelection.selectedVersionRange,
      'vers:intdot/>=2.40|<2.50',
    );
    assert.deepStrictEqual(tool.selection?.intentSelection, {
      requested: 'push',
      mode: 'named',
      selected: ['push'],
    });
    const [ssh] = push.diagnostics.resolvedDependencies;
    assert.strictEqual(ssh.entryId, 'tool:ssh');
    assert.deepStrictEqual(ssh.inputIndexes, [0]);
    assert.deepStrictEqual(ssh.intentSelection, { mode: 'none', selected: [] });
    assert.ok(push.requirements?.filesystem?.readonlyPaths?.includes(paths.ssh));
    assert.strictEqual(push.requirements?.network?.egress?.allow?.length, 1);
  });

  it('reports per-pair outcomes with structured warnings', async () => {
    const outOfRange = await resolveToolRequirementsWithDiagnostics(git('2.30.0', 'fetch'), context);
    assert.strictEqual(outOfRange.diagnostics.tools[0].status, 'version_out_of_range');
    assert.ok(outOfRange.requirements !== undefined);
    const warning = outOfRange.diagnostics.warnings[0];
    assert.strictEqual(warning.code, 'version_out_of_range');
    assert.ok(warning.code === 'version_out_of_range');
    assert.strictEqual(warning.entryId, 'tool:git');
    assert.strictEqual(warning.detectedVersion, '2.30.0');

    const unparseable = await resolveToolRequirementsWithDiagnostics(git('banana', 'fetch'), context);
    assert.strictEqual(unparseable.diagnostics.tools[0].status, 'version_unparseable');
    assert.strictEqual(unparseable.requirements, undefined);
    // Selection metadata survives the failure without implying contribution.
    assert.strictEqual(unparseable.diagnostics.tools[0].contributes, false);
    assert.strictEqual(unparseable.diagnostics.tools[0].selection?.entryId, 'tool:git');

    const unsupported = await resolveToolRequirementsWithDiagnostics(git(undefined, 'bundle-fetch'), context);
    assert.strictEqual(unsupported.diagnostics.tools[0].status, 'intent_unsupported');
    assert.strictEqual(unsupported.requirements, undefined);

    const bundle = await resolveToolRequirementsWithDiagnostics(git('2.55', 'bundle-fetch'), context);
    assert.strictEqual(bundle.diagnostics.tools[0].status, 'matched_version');
    assert.deepStrictEqual(bundle.diagnostics.resolvedDependencies, []);

    const purl = await resolveToolRequirementsWithDiagnostics(
      { invocationName: 'npm', packageUrl: 'not a purl' },
      context,
    );
    assert.strictEqual(purl.diagnostics.tools[0].status, 'tool_unmatched');
    assert.strictEqual(purl.diagnostics.warnings[0].code, 'purl_invalid');

    const ignored = await resolveToolRequirementsWithDiagnostics(
      { invocationName: 'npm', packageUrl: 'pkg:npm/npm@10.0.0' },
      context,
    );
    const first = ignored.diagnostics.warnings[0];
    assert.ok(first.code === 'purl_components_ignored');
    assert.deepStrictEqual(first.ignoredComponents, ['version']);
  });

  it('composes pairs: a tool without network does not veto another', async () => {
    const resolution = await resolveToolRequirementsWithDiagnostics(
      [git(undefined, 'local'), git(undefined, 'fetch'), 'no-such-tool'],
      context,
    );
    assert.strictEqual(resolution.requirements?.network?.egress?.allow?.length, 1);
    assert.strictEqual(resolution.diagnostics.tools[2].status, 'tool_unmatched');
    assert.deepStrictEqual(
      resolution.diagnostics.tools.map((tool) => tool.contributes),
      [true, true, false],
    );
    assert.strictEqual(resolution.diagnostics.tools[2].selection, undefined);
    const unmatched = resolution.diagnostics.warnings.find((w) => w.code === 'tool_unmatched');
    assert.ok(unmatched !== undefined && unmatched.code === 'tool_unmatched');
    assert.strictEqual(unmatched.inputIndex, 2);
    assert.strictEqual(unmatched.invocationName, 'no-such-tool');
  });

  it('rejects with MxcError and a stable reason for an invalid context', async () => {
    await assert.rejects(
      resolveToolRequirements('npm', { ...context, platform: 'plan9' as never }),
      (error: unknown) => {
        assert.ok(error instanceof MxcError);
        assert.strictEqual(error.code, 'malformed_request');
        assert.strictEqual(error.details?.reason, 'invalid_context');
        return true;
      },
    );
    await assert.rejects(
      resolveToolRequirements(git(undefined, ''), context),
      (error: unknown) => error instanceof MxcError && error.details?.reason === 'invalid_context',
    );
  });

  it('binds project_root through either context form and rejects differing values', async () => {
    const { projectRoot, ...rest } = context;
    const viaSymbol = await resolveToolRequirements(git(undefined, 'local'), {
      ...rest,
      symbols: { ...rest.symbols, project_root: projectRoot ?? '' },
    });
    assert.deepStrictEqual(viaSymbol, await resolveToolRequirements(git(undefined, 'local'), context));
    await assert.rejects(
      resolveToolRequirements([], {
        ...context,
        symbols: { ...context.symbols, project_root: `${paths.project}x` },
      }),
      (error: unknown) =>
        error instanceof MxcError &&
        error.code === 'malformed_request' &&
        error.details?.reason === 'invalid_context',
    );
  });
});

describe('policy store diagnostics boundary validation (prototype)', () => {
  const valid = () => ({
    requirements: {},
    diagnostics: {
      catalogRevision: 'r',
      tools: [{ inputIndex: 0, contributes: true, status: 'matched_default' }],
      resolvedDependencies: [],
      warnings: [],
    },
  });

  it('accepts a well-formed result, including an unfamiliar status', () => {
    validateResolution(valid());
    const result = valid();
    result.diagnostics.tools[0].status = 'some_future_status';
    validateResolution(result);
  });

  it('rejects a missing or non-boolean contribution flag', () => {
    for (const contributes of [undefined, 'true', 1]) {
      const result = valid() as { diagnostics: { tools: Array<Record<string, unknown>> } };
      result.diagnostics.tools[0].contributes = contributes;
      assert.throws(
        () => validateResolution(result),
        (error: unknown) => error instanceof MxcError && error.code === 'backend_error',
      );
    }
  });

  it('rejects output that disagrees with the contribution flags', () => {
    const withoutOutput = valid() as Record<string, unknown>;
    delete withoutOutput.requirements;
    assert.throws(() => validateResolution(withoutOutput), MxcError);
    const noContributor = valid();
    noContributor.diagnostics.tools[0].contributes = false;
    assert.throws(() => validateResolution(noContributor), MxcError);
  });
});
