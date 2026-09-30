#!/usr/bin/env node
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Extractability gate: copies the git-tracked contents of this directory
// (what `git filter-repo --subdirectory-filter policy-catalog` would keep)
// into a fresh temporary git repository and runs the full check there. It
// fails if anything reaches outside the directory: imports, relative links,
// scripts, or an undeclared dependency on the enclosing checkout.
//
//   npm run check:extract            # uses committed files (HEAD)
//   npm run check:extract -- --worktree   # uses tracked files as they are on disk
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const packageDir = fileURLToPath(new URL('..', import.meta.url));
const git = (args, cwd) => execFileSync('git', args, { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] });
const repoRoot = git(['rev-parse', '--show-toplevel'], packageDir).trim();
const prefix = relative(repoRoot, packageDir).replaceAll('\\', '/');
const useWorktree = process.argv.includes('--worktree');

const work = mkdtempSync(join(tmpdir(), 'policy-catalog-extract-'));
const target = join(work, 'repo');
try {
  if (useWorktree) {
    // Tracked plus untracked-but-not-ignored: exactly what the next commit holds.
    const files = git(['ls-files', '-co', '--exclude-standard', '-z', '--', '.'], packageDir).split('\0').filter(Boolean);
    for (const file of files) {
      const destination = join(target, file);
      mkdirSync(dirname(destination), { recursive: true });
      copyFileSync(join(packageDir, file), destination);
    }
    console.log(`Copied ${files.length} files from the working tree.`);
  } else {
    // `git archive HEAD:<prefix>` yields exactly the subdirectory-filtered tree.
    const tar = join(work, 'tree.tar');
    git(['archive', '--format=tar', '-o', tar, `HEAD:${prefix}`], repoRoot);
    mkdirSync(target);
    execFileSync('tar', ['-xf', tar, '-C', target], { stdio: 'inherit' });
    console.log(`Extracted HEAD:${prefix}.`);
  }

  // A fresh repository, so git-dependent checks run against the extracted root.
  git(['init', '-q'], target);
  git(['-c', 'user.name=extract-check', '-c', 'user.email=extract-check@invalid', 'add', '-A'], target);
  git(['-c', 'user.name=extract-check', '-c', 'user.email=extract-check@invalid', 'commit', '-q', '-m', 'extract'], target);

  // Relative Markdown links must resolve inside the extracted tree.
  const brokenLinks = [];
  // docs/design.md is a verbatim copy of the MXC proposal. Its relative links
  // point into the MXC repository and are deliberately not rewritten; each
  // one must instead have an absolute equivalent listed in the file's header.
  const VERBATIM = 'docs/design.md';
  const VERBATIM_MARKER = '<!-- BEGIN VERBATIM COPY -->';
  for (const file of git(['ls-files', '*.md'], target).split('\n').filter(Boolean)) {
    const raw = readFileSync(join(target, file), 'utf8');
    const markerAt = file === VERBATIM ? raw.indexOf(VERBATIM_MARKER) : -1;
    if (file === VERBATIM && markerAt < 0) {
      brokenLinks.push(`${file}: missing '${VERBATIM_MARKER}'`);
      continue;
    }
    const header = markerAt >= 0 ? raw.slice(0, markerAt) : '';
    const text = raw.replace(/```[\s\S]*?```/g, '');
    for (const match of text.matchAll(/\]\(([^)\s]+)\)/g)) {
      const link = match[1].split('#')[0];
      if (!link || /^[a-z][a-z0-9+.-]*:/i.test(link)) continue;
      if (markerAt >= 0) {
        if (!header.includes(`- ${link} -> https://`)) {
          brokenLinks.push(`${file}: MXC-relative link '${match[1]}' has no absolute equivalent in the header`);
        }
        continue;
      }
      const resolved = resolve(dirname(join(target, file)), link);
      if (relative(target, resolved).startsWith('..')) {
        brokenLinks.push(`${file}: '${match[1]}' leaves the repository`);
        continue;
      }
      try {
        readFileSync(resolved);
      } catch (error) {
        if (error.code === 'EISDIR') continue;
        brokenLinks.push(`${file}: '${match[1]}' does not exist`);
      }
    }
  }
  if (brokenLinks.length > 0) {
    console.error('Extractability check FAILED (links):');
    for (const link of brokenLinks) console.error(`  - ${link}`);
    process.exitCode = 1;
  } else {
    const npmCli = process.env.npm_execpath;
    if (!npmCli) throw new Error('run this script through `npm run check:extract`');
    const npm = args => execFileSync(process.execPath, [npmCli, ...args], { cwd: target, stdio: 'inherit' });
    npm(['ci', '--no-audit', '--no-fund']);
    // `check` includes typecheck, unit, functional (from the packed tarball),
    // validation, pack contents, and the install smoke test.
    npm(['run', 'check']);
    // The Rust and .NET libraries, their packaged-artifact functional tests,
    // and the cross-language check must also work from the extracted tree.
    const run = (file, args, cwd = target) => execFileSync(file, args, { cwd, stdio: 'inherit' });
    const rustDir = join(target, 'rust');
    run('cargo', ['fmt', '--check'], rustDir);
    run('cargo', ['clippy', '--locked', '--all-targets', '--all-features', '--', '-D', 'warnings'], rustDir);
    run('cargo', ['test', '--locked'], rustDir);
    run(process.execPath, ['scripts/rust-functional.mjs']);
    const dotnetDir = join(target, 'dotnet');
    run('dotnet', ['build', 'Microsoft.Mxc.PolicyCatalog.slnx', '-c', 'Release'], dotnetDir);
    run('dotnet', ['test', '--solution', 'Microsoft.Mxc.PolicyCatalog.slnx', '-c', 'Release', '--no-build'], dotnetDir);
    run(process.execPath, ['scripts/dotnet-functional.mjs']);
    run(process.execPath, ['scripts/cross-language-check.mjs']);
    console.log('\nExtractability check OK: the directory builds, tests, validates, and packs as a standalone repository (TypeScript, Rust, .NET, cross-language).');
  }
} finally {
  rmSync(work, { recursive: true, force: true });
}
