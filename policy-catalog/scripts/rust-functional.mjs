#!/usr/bin/env node
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Functional tests of the PACKAGED Rust crate (rust/). Cross-platform driver:
//
//   1. copies ../catalog into rust/catalog (rust/scripts/sync-catalog.mjs)
//   2. `cargo package` in rust/ (verifies the packaged crate builds in isolation)
//   3. extracts target/package/mxc-policy-catalog-*.crate into a fresh temp
//      directory outside the repository (os.tmpdir())
//   4. builds the packaged `policy-catalog` binary from the extracted crate
//   5. creates a consumer crate from rust/functional/ (Cargo.toml.in, src/,
//      tests/) that depends on the extracted crate by `path`, and runs its tests
//
// Every build uses a CARGO_TARGET_DIR inside the temp directory, so nothing
// collides with rust/target. Prints test counts; exits nonzero on failure.
//
//   node scripts/rust-functional.mjs [--allow-dirty] [--keep]
import { execFileSync, spawnSync } from 'node:child_process';
import { copyFileSync, cpSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, relative, isAbsolute } from 'node:path';
import { fileURLToPath } from 'node:url';

const allowDirty = process.argv.includes('--allow-dirty');
const keep = process.argv.includes('--keep');
const crateDir = fileURLToPath(new URL('../rust/', import.meta.url));
const repoDir = fileURLToPath(new URL('../', import.meta.url));
const exe = process.platform === 'win32' ? '.exe' : '';

function run(cmd, args, options = {}) {
  console.log(`> ${cmd} ${args.join(' ')}${options.cwd ? `   (in ${options.cwd})` : ''}`);
  const result = spawnSync(cmd, args, { stdio: 'inherit', ...options });
  if (result.status !== 0) {
    throw new Error(`${cmd} ${args.join(' ')} failed with exit code ${result.status ?? result.signal}`);
  }
}

const work = mkdtempSync(join(tmpdir(), 'policy-catalog-rust-functional-'));
const rel = relative(repoDir, work);
if (!(rel.startsWith('..') || isAbsolute(rel))) {
  throw new Error(`temp directory ${work} is inside the repository`);
}

let exitCode = 1;
try {
  run(process.execPath, [join(crateDir, 'scripts', 'sync-catalog.mjs')]);
  run('cargo', ['package', ...(allowDirty ? ['--allow-dirty'] : [])], { cwd: crateDir });

  const packageDir = join(crateDir, 'target', 'package');
  const crates = readdirSync(packageDir)
    .filter(name => /^mxc-policy-catalog-.*\.crate$/.test(name))
    .map(name => join(packageDir, name))
    .sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs);
  if (crates.length === 0) {
    throw new Error(`no .crate in ${packageDir}`);
  }
  const crate = crates[0];
  console.log(`Packaged crate: ${crate}`);

  // A .crate is a gzip'd tarball with one top-level `<name>-<version>/` directory.
  const extractRoot = join(work, 'extracted');
  rmSync(extractRoot, { recursive: true, force: true });
  execFileSync('tar', ['-xzf', crate, '-C', work.replaceAll('\\', '/')], { stdio: 'inherit', cwd: work });
  const extracted = readdirSync(work)
    .filter(name => name.startsWith('mxc-policy-catalog-') && statSync(join(work, name)).isDirectory())
    .map(name => join(work, name))[0];
  if (!extracted) {
    throw new Error('the .crate did not contain a mxc-policy-catalog-* directory');
  }
  console.log(`Extracted to: ${extracted}`);

  // Build the packaged binary from the extracted crate.
  const crateTarget = join(work, 'target-crate');
  run('cargo', ['build', '--locked', '--bin', 'policy-catalog'], {
    cwd: extracted,
    env: { ...process.env, CARGO_TARGET_DIR: crateTarget },
  });
  const bin = join(crateTarget, 'debug', `policy-catalog${exe}`);

  // Consumer crate from the template.
  const consumer = join(work, 'consumer');
  const template = join(crateDir, 'functional');
  cpSync(join(template, 'src'), join(consumer, 'src'), { recursive: true });
  cpSync(join(template, 'tests'), join(consumer, 'tests'), { recursive: true });
  const manifest = readFileSync(join(template, 'Cargo.toml.in'), 'utf8').replaceAll('@CRATE_PATH@', extracted.replaceAll('\\', '/'));
  writeFileSync(join(consumer, 'Cargo.toml'), manifest);
  // Pin dependency versions to the packaged crate's lock file and toolchain.
  copyFileSync(join(extracted, 'Cargo.lock'), join(consumer, 'Cargo.lock'));
  copyFileSync(join(extracted, 'rust-toolchain.toml'), join(consumer, 'rust-toolchain.toml'));

  const testWork = join(work, 'test-work');
  const result = spawnSync('cargo', ['test', '--', '--test-threads=4'], {
    cwd: consumer,
    encoding: 'utf8',
    env: {
      ...process.env,
      CARGO_TARGET_DIR: join(work, 'target-consumer'),
      POLICY_CATALOG_BIN: bin,
      POLICY_CATALOG_WORK: testWork,
    },
  });
  process.stdout.write(result.stdout ?? '');
  process.stderr.write(result.stderr ?? '');
  let passed = 0;
  let failed = 0;
  for (const match of (result.stdout ?? '').matchAll(/test result: \w+\. (\d+) passed; (\d+) failed/g)) {
    passed += Number(match[1]);
    failed += Number(match[2]);
  }
  console.log(`\nrust functional (packaged crate): ${passed} passed, ${failed} failed`);
  exitCode = result.status === 0 && failed === 0 && passed > 0 ? 0 : 1;
} catch (error) {
  console.error(`rust-functional: ${error.message}`);
  exitCode = 1;
} finally {
  if (keep) {
    console.log(`Kept ${work}`);
  } else {
    rmSync(work, { recursive: true, force: true });
  }
}
process.exitCode = exitCode;
