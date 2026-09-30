// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Functional tests for catalog integrity and contract enforcement through the
// installed CLI: digest tampering, missing files, invalid manifests,
// dependency cycles, and the --base-ref immutability check. Every catalog
// here is a copy or a synthetic directory in a temporary folder; the
// installed package is never modified.
import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { cli, entry, fullContext, installedCatalogDir, installedManifest, revision, writeCatalog } from './helpers.js';

let work: string;
before(() => {
  work = mkdtempSync(join(tmpdir(), 'policy-catalog-functional-integrity-'));
});
after(() => {
  rmSync(work, { recursive: true, force: true });
});

const copyInstalled = (): string => {
  const dir = mkdtempSync(join(work, 'catalog-'));
  cpSync(installedCatalogDir, dir, { recursive: true });
  return dir;
};
const latestFile = (dir: string) => join(dir, installedManifest.revisions.at(-1).file);

describe('integrity of a copied catalog', () => {
  it('an untouched copy validates and resolves like the bundled catalog', () => {
    const dir = copyInstalled();
    assert.equal(cli('validate', '--catalog', dir).status, 0);
    const ctx = fullContext('linux', 'x64');
    assert.deepEqual(cli('resolve', '--catalog', dir, ...ctx, 'git').json, cli('resolve', ...ctx, 'git').json);
  });

  it('formatting-only edits keep the canonical digest valid', () => {
    const dir = copyInstalled();
    const file = latestFile(dir);
    writeFileSync(file, JSON.stringify(JSON.parse(readFileSync(file, 'utf8'))).replaceAll(',', ',\r\n'));
    const result = cli('validate', '--catalog', dir);
    assert.equal(result.status, 0, result.stdout);
  });

  it('a semantic edit to a published revision fails integrity in validate, resolve, and inspect', () => {
    const dir = copyInstalled();
    const file = latestFile(dir);
    const data = JSON.parse(readFileSync(file, 'utf8'));
    // Widen a requirement: the kind of tamper integrity checking exists to stop.
    data.entries[0].platformVariants[0].sandboxPolicy.filesystem.readwritePaths.push('${user_home}');
    writeFileSync(file, JSON.stringify(data, null, 2));

    const validate = cli('validate', '--catalog', dir);
    assert.equal(validate.status, 1, validate.stderr);
    assert.equal(validate.json?.ok, false, `validate produced no JSON report; stderr: ${validate.stderr}`);
    assert.match(validate.json.errors.join('\n'), /\[integrity\] catalog revision '[^']+' digest [0-9a-f]{64} does not match the published digest/);

    const resolve = cli('resolve', '--catalog', dir, ...fullContext('windows', 'x64'), 'git');
    assert.equal(resolve.status, 1, resolve.stderr);
    assert.equal(resolve.json.error.category, 'integrity');
    assert.equal(cli('inspect', '--catalog', dir).json.error.category, 'integrity');
  });

  it('a missing revision file is an integrity failure', () => {
    const dir = copyInstalled();
    rmSync(latestFile(dir));
    const result = cli('inspect', '--catalog', dir);
    assert.equal(result.status, 1);
    assert.equal(result.json.error.category, 'integrity');
    assert.match(cli('validate', '--catalog', dir).json.errors.join('\n'), /\[integrity\].*could not be read/);
  });

  it('a manifest whose default names an unlisted revision is a validation failure', () => {
    const dir = copyInstalled();
    writeFileSync(join(dir, 'manifest.json'), JSON.stringify({ ...installedManifest, defaultRevision: '2099-01-01.1' }));
    const result = cli('inspect', '--catalog', dir);
    assert.equal(result.status, 1);
    assert.equal(result.json.error.category, 'validation');
    const validate = cli('validate', '--catalog', dir);
    assert.equal(validate.status, 1);
    assert.match(validate.json.errors.join('\n'), /\[validation\] manifest\.defaultRevision must name a listed revision/);
  });
});

describe('dependency cycles', () => {
  const dependsOn = (target: string) => ({
    platformVariants: [{
      when: { platform: 'linux' },
      dependencies: [{ entryId: target }],
      sandboxPolicy: { version: '0.9.0-alpha', filesystem: { readonlyPaths: [`\${git_prefix}/${target.split(':')[1]}-dep`] } },
    }],
  });

  it('validate rejects a correctly digested catalog whose entries form a cycle', () => {
    // Digests are correct, so the only defect is the cycle a -> b -> c -> a.
    const dir = writeCatalog(mkdtempSync(join(work, 'cycle-')), [revision([
      entry('tool:a', dependsOn('tool:b')),
      entry('tool:b', dependsOn('tool:c')),
      entry('tool:c', dependsOn('tool:a')),
    ])]);
    const result = cli('validate', '--catalog', dir);
    assert.equal(result.status, 1, result.stderr);
    assert.equal(result.json?.ok, false, `validate produced no JSON report; stderr: ${result.stderr}`);
    assert.match(result.json.errors.join('\n'), /\[validation\] 'tool:a' on linux\/x64: cycle \(tool:a -> tool:b -> tool:c -> tool:a\)/);
  });

  it('resolve never returns a policy from a cyclic catalog', () => {
    const dir = writeCatalog(mkdtempSync(join(work, 'cycle-')), [revision([
      entry('tool:a', dependsOn('tool:b')),
      entry('tool:b', dependsOn('tool:a')),
    ])]);
    const result = cli('resolve', '--catalog', dir, '--platform', 'linux', '--architecture', 'x64', '--allow-weak', '--symbol', 'git_prefix=/opt', 'a');
    assert.equal(result.status, 1, result.stderr);
    assert.equal(result.json?.error?.category, 'validation', `stderr: ${result.stderr}`);
    assert.match(result.json.error.message, /cycle \(tool:a -> tool:b -> tool:a\)/);
  });

  it('a diamond (shared dependency, no cycle) is valid and contributes the shared entry once', () => {
    const dir = writeCatalog(mkdtempSync(join(work, 'diamond-')), [revision([
      entry('tool:a', { platformVariants: [{ when: { platform: 'linux' }, dependencies: [{ entryId: 'tool:b' }, { entryId: 'tool:c' }], sandboxPolicy: { version: '0.9.0-alpha', filesystem: { readonlyPaths: ['${git_prefix}/a'] } } }] }),
      entry('tool:b', dependsOn('tool:d')),
      entry('tool:c', dependsOn('tool:d')),
      entry('tool:d'),
    ])]);
    assert.equal(cli('validate', '--catalog', dir).status, 0);
    const result = cli('resolve', '--catalog', dir, '--diagnostics', '--platform', 'linux', '--architecture', 'x64', '--allow-weak', '--symbol', 'git_prefix=/opt', 'a');
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(result.json.policy.filesystem.readonlyPaths, ['/opt/a', '/opt/d-dep', '/opt/d']);
    assert.deepEqual(result.json.diagnostics.resolvedDependencies.map((d: any) => d.entryId), ['tool:b', 'tool:c', 'tool:d']);
  });
});

describe('validate --base-ref (published-revision immutability)', () => {
  const git = (cwd: string, ...args: string[]) =>
    execFileSync('git', ['-c', 'user.name=functional-test', '-c', 'user.email=functional-test@invalid', '-c', 'core.autocrlf=false', ...args], { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });

  const publishedRepo = (): { repo: string; dir: string } => {
    const repo = mkdtempSync(join(work, 'repo-'));
    git(repo, 'init', '-q');
    const dir = join(repo, 'catalog');
    writeCatalog(dir, [revision([entry('tool:a')], '2000-01-01.1')]);
    git(repo, 'add', '-A');
    git(repo, 'commit', '-q', '-m', 'publish 2000-01-01.1');
    return { repo, dir };
  };

  it('an unchanged or appended catalog passes against the base ref', () => {
    const { dir } = publishedRepo();
    const unchanged = cli('validate', '--catalog', dir, '--base-ref', 'HEAD');
    assert.equal(unchanged.status, 0, unchanged.stdout);
    assert.deepEqual(unchanged.json.baseRef, { ref: 'HEAD', comparedRevisions: 1 });
    // Append a new revision, as the contribution flow requires.
    writeCatalog(dir, [
      revision([entry('tool:a')], '2000-01-01.1'),
      revision([entry('tool:a'), entry('tool:b')], '2000-01-02.1'),
    ]);
    const appended = cli('validate', '--catalog', dir, '--base-ref', 'HEAD');
    assert.equal(appended.status, 0, appended.stdout);
  });

  it('rewriting a published revision (with a recomputed digest) fails against the base ref', () => {
    const { dir } = publishedRepo();
    // Consistent digests, so only the immutability check can catch it.
    writeCatalog(dir, [revision([entry('tool:a', { entryRevision: 2, displayName: 'changed' })], '2000-01-01.1')]);
    assert.equal(cli('validate', '--catalog', dir).status, 0, 'without --base-ref the rewrite is internally consistent');
    const result = cli('validate', '--catalog', dir, '--base-ref', 'HEAD');
    assert.equal(result.status, 1, result.stdout);
    const errors = result.json.errors.join('\n');
    assert.match(errors, /\[immutability\] published revision '2000-01-01\.1' manifest entry was modified/);
    assert.match(errors, /\[immutability\] published revision file 'revisions\/2000-01-01\.1\.json' was modified/);
  });

  it('an unknown or option-like base ref, or a directory outside git, is an error, never a silent pass', () => {
    const { dir } = publishedRepo();
    const badRef = cli('validate', '--catalog', dir, '--base-ref', 'no-such-ref');
    assert.equal(badRef.status, 1);
    assert.match(badRef.json.errors.join('\n'), /base-ref check: 'no-such-ref' does not name a commit/);
    const optionLike = cli('validate', '--catalog', dir, '--base-ref', '-h');
    assert.equal(optionLike.status, 1);
    assert.match(optionLike.json.errors.join('\n'), /base-ref check: '-h' is not a valid git ref/);
    const outside = writeCatalog(mkdtempSync(join(work, 'nogit-')), [revision([entry('tool:a')])]);
    const noGit = cli('validate', '--catalog', outside, '--base-ref', 'HEAD');
    assert.equal(noGit.status, 1);
    assert.match(noGit.json.errors.join('\n'), /is not inside a git work tree/);
  });

  it('a base ref with no catalog at that path has nothing published yet', () => {
    const { repo, dir } = publishedRepo();
    // A commit whose tree is empty: git mktree with no input writes the empty tree.
    const emptyTree = git(repo, 'mktree').trim();
    const orphan = git(repo, 'commit-tree', emptyTree, '-m', 'empty').trim();
    const result = cli('validate', '--catalog', dir, '--base-ref', orphan);
    assert.equal(result.status, 0, result.stdout);
    assert.deepEqual(result.json.baseRef, { ref: orphan, comparedRevisions: 0 });
  });
});
