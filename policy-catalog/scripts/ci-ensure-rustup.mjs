#!/usr/bin/env node
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// CI helper: makes sure `rustup` and the toolchain pinned by
// rust/rust-toolchain.toml are available. Most GitHub-hosted images ship
// rustup; some (for example windows-11-arm) do not, and then this installs it
// from the official rustup-init for the runner's architecture. Local
// developers install Rust themselves; this script only prints what it finds
// when rustup is already present.
import { execFileSync, spawnSync } from 'node:child_process';
import { appendFileSync, mkdtempSync, writeFileSync } from 'node:fs';
import { homedir, machine, tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const rustDir = fileURLToPath(new URL('../rust/', import.meta.url));
const has = cmd => spawnSync(cmd, ['--version'], { stdio: 'ignore' }).status === 0;

if (!has('rustup')) {
  if (!process.env.CI) {
    console.error('rustup is not installed; install Rust from https://rustup.rs and rerun.');
    process.exit(1);
  }
  const arch = machine() === 'arm64' || machine() === 'aarch64' ? 'aarch64' : 'x86_64';
  const target = process.platform === 'win32' ? `${arch}-pc-windows-msvc`
    : process.platform === 'darwin' ? `${arch}-apple-darwin` : `${arch}-unknown-linux-gnu`;
  const exe = process.platform === 'win32' ? '.exe' : '';
  const url = `https://static.rust-lang.org/rustup/dist/${target}/rustup-init${exe}`;
  console.log(`rustup not found; installing from ${url}`);
  const response = await fetch(url);
  if (!response.ok) throw new Error(`download failed: ${response.status}`);
  const init = join(mkdtempSync(join(tmpdir(), 'rustup-')), `rustup-init${exe}`);
  writeFileSync(init, Buffer.from(await response.arrayBuffer()), { mode: 0o755 });
  execFileSync(init, ['-y', '--profile', 'minimal', '--default-toolchain', 'none', '--no-modify-path'], { stdio: 'inherit' });
  const bin = join(process.env.CARGO_HOME ?? join(homedir(), '.cargo'), 'bin');
  if (process.env.GITHUB_PATH) appendFileSync(process.env.GITHUB_PATH, `${bin}\n`);
  process.env.PATH = `${bin}${process.platform === 'win32' ? ';' : ':'}${process.env.PATH}`;
}

// Resolving the active toolchain in rust/ installs the pinned channel and its
// components (rustup >= 1.28 no longer installs it implicitly on `show`).
execFileSync('rustup', ['toolchain', 'install'], { cwd: rustDir, stdio: 'inherit' });
execFileSync('rustc', ['-vV'], { cwd: rustDir, stdio: 'inherit' });
