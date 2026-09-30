// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Functional tests for the installed `policy-catalog` CLI: the three commands
// (resolve, inspect, validate), exit codes, JSON output, and the bundled
// catalog shipped in the tarball.
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { ARCHITECTURES, PLATFORMS, PLATFORM_CONTEXT, cli, cliViaBin, fullContext, installedManifest } from './helpers.js';

describe('installed CLI: inspect and validate', () => {
  it('inspect reports catalog info and metadata-only entries sorted by entryId', () => {
    const result = cli('inspect');
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(result.json.info, { catalogSchemaVersion: '1', catalogRevision: installedManifest.defaultRevision });
    const ids = result.json.entries.map((e: any) => e.entryId);
    assert.deepEqual(ids, ['tool:git', 'tool:node', 'tool:npm']);
    assert.ok(!result.stdout.includes('readwritePaths'), 'inspect must never return a policy body');
    assert.ok(!result.stdout.includes('${'), 'inspect must never return unresolved policy templates');
  });

  it('validate checks integrity, contract, and history of every installed revision', () => {
    const result = cli('validate');
    assert.equal(result.status, 0, result.stdout);
    assert.equal(result.json.ok, true);
    assert.deepEqual(result.json.errors, []);
    assert.equal(result.json.defaultRevision, installedManifest.defaultRevision);
    assert.deepEqual(result.json.revisions, installedManifest.revisions.map((r: any) => r.catalogRevision));
    assert.equal(result.json.baseRef, undefined);
  });

  it('the npm-installed bin shim runs the same CLI', () => {
    const result = cliViaBin('inspect');
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(result.json, cli('inspect').json);
  });

  it('removed command names are usage errors, not aliases', () => {
    for (const command of ['info', 'list', 'verify']) {
      const result = cli(command);
      assert.equal(result.status, 2, command);
      assert.match(result.stderr, /unknown command/);
      assert.match(result.stderr, /usage: policy-catalog <resolve\|inspect\|validate>/);
    }
  });

  it('usage errors exit 2 without JSON on stdout', () => {
    const cases = [
      [],
      ['bogus'],
      ['resolve', '--platform'],
      ['resolve', '--symbol', 'noequals', 'git'],
      ['resolve', '--purl', 'pkg:npm/npm'],
      ['resolve', '--base-ref', 'HEAD', 'git'],
      ['inspect', 'extra'],
      ['validate', 'extra'],
      ['validate', '--base-ref'],
      ['resolve', '--catalog'],
    ];
    for (const args of cases) {
      const result = cli(...args);
      assert.equal(result.status, 2, args.join(' '));
      assert.equal(result.json, undefined, args.join(' '));
      assert.match(result.stderr, /usage: policy-catalog/);
    }
  });
});

describe('installed CLI: resolve against the bundled catalog', () => {
  for (const platform of PLATFORMS) {
    for (const architecture of ARCHITECTURES) {
      it(`resolves the full tool set on ${platform}/${architecture} deterministically`, () => {
        const c = PLATFORM_CONTEXT[platform];
        const first = cli('resolve', '--diagnostics', ...fullContext(platform, architecture), 'git', 'npm', 'node');
        const second = cli('resolve', '--diagnostics', ...fullContext(platform, architecture), 'git', 'npm', 'node');
        assert.equal(first.status, 0, first.stderr);
        assert.deepEqual(first.json, second.json);
        const { policy, diagnostics } = first.json;
        assert.equal(policy.version, '0.9.0-alpha');
        assert.deepEqual(policy.filesystem.readwritePaths, [c.root, c.cache]);
        assert.deepEqual(policy.filesystem.readonlyPaths, [c.prefix]);
        assert.equal(policy.network, undefined, 'v1 never composes network');
        assert.deepEqual(diagnostics.tools.map((t: any) => t.matches.map((m: any) => m.entryId)), [['tool:git'], ['tool:npm'], ['tool:node']]);
        assert.deepEqual(diagnostics.resolvedDependencies, [{ entryId: 'tool:node', entryRevision: 1 }]);
      });
    }
  }

  it('policy-only output equals the diagnostics policy', () => {
    const ctx = fullContext('linux', 'x64');
    const plain = cli('resolve', ...ctx, 'npm');
    const diag = cli('resolve', '--diagnostics', ...ctx, 'npm');
    assert.equal(plain.status, 0, plain.stderr);
    assert.deepEqual(plain.json, diag.json.policy);
  });

  it('an unresolved required symbol yields null with an actionable warning', () => {
    const result = cli('resolve', '--diagnostics', '--platform', 'linux', '--architecture', 'x64', '--allow-weak', '--symbol', 'git_prefix=/usr/bin', 'git');
    assert.equal(result.status, 0);
    assert.equal(result.json.policy, undefined);
    assert.match(result.json.diagnostics.warnings.join('\n'), /required symbol 'project_root'.*supply ResolveContext\.projectRoot/);
  });

  it('empty input produces no policy, never an empty policy', () => {
    assert.equal(cli('resolve', '--platform', 'linux', '--architecture', 'x64').json, null);
    const empty = cli('resolve', '--diagnostics', '--platform', 'linux', '--architecture', 'x64');
    assert.equal(empty.status, 0);
    assert.deepEqual(empty.json, { diagnostics: { catalogRevision: installedManifest.defaultRevision, tools: [], resolvedDependencies: [], warnings: [] } });
  });

  it('invalid caller input is a malformed_request (invalid_context) library failure (exit 1)', () => {
    const cases: string[][] = [
      ['resolve', '--platform', 'plan9', 'git'],
      ['resolve', '--architecture', 'mips', 'git'],
      ['resolve', '--platform', 'linux', '--architecture', 'x64', '--allow-weak', '--symbol', 'node_prefix=relative', 'node'],
      ['resolve', '--platform', 'linux', '--architecture', 'x64', '--symbol', 'unknown_symbol=/x', 'git'],
      ['resolve', '--platform', 'linux', '--architecture', 'x64', '--purl', 'not-a-purl', 'npm'],
      ['resolve', '--platform', 'linux', '--architecture', 'x64', './bin/git'],
    ];
    for (const args of cases) {
      const result = cli(...args);
      assert.equal(result.status, 1, `${args.join(' ')} -> ${result.stderr}`);
      assert.equal(result.json.error.details.reason, 'invalid_context', args.join(' '));
    }
  });
});
