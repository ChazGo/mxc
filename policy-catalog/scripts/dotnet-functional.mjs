#!/usr/bin/env node
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// .NET functional test driver. Tests the package a consumer would install,
// not the source tree:
//
//   1. `dotnet pack` Microsoft.Mxc.PolicyCatalog into a local feed in a
//      temporary directory outside the repository (os.tmpdir()).
//   2. Copy the CLI (dotnet/functional/policy-catalog-cli-consumer + the CLI
//      Program.cs) and the functional test project
//      (dotnet/functional/Microsoft.Mxc.PolicyCatalog.FunctionalTests) into
//      temporary consumer directories with a generated nuget.config
//      (local feed + nuget.org) and an isolated NUGET_PACKAGES directory, so a
//      stale cached package can never be used.
//   3. Build the CLI against the package, then build and run the tests with
//      POLICY_CATALOG_CLI_DLL pointing at that CLI build.
//
//   node scripts/dotnet-functional.mjs        (from policy-catalog/)
//
// Exits nonzero on any failure. Set POLICY_CATALOG_KEEP_TEMP=1 to keep the
// temporary directory for inspection. Third-party packages (xunit.v3) come
// from nuget.org; set POLICY_CATALOG_NUGET_UPSTREAM to a nuget.org mirror URL
// where nuget.org is not directly reachable.
import { spawnSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, relative, isAbsolute } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const dotnetDir = join(root, 'dotnet');
const version = '0.0.0-prototype';
const upstream = process.env.POLICY_CATALOG_NUGET_UPSTREAM || 'https://api.nuget.org/v3/index.json';

const work = mkdtempSync(join(tmpdir(), 'policy-catalog-dotnet-functional-'));
const outside = relative(root, work);
if (!(outside.startsWith('..') || isAbsolute(outside))) {
  console.error(`temporary directory ${work} is inside the repository`);
  process.exit(1);
}
const feed = join(work, 'feed');
const packages = join(work, 'nuget-packages');
mkdirSync(feed);
mkdirSync(packages);

const env = {
  ...process.env,
  NUGET_PACKAGES: packages,
  DOTNET_CLI_TELEMETRY_OPTOUT: '1',
  DOTNET_NOLOGO: '1',
  DOTNET_SKIP_FIRST_TIME_EXPERIENCE: '1',
  // Consumers must not inherit the repository's global.json or Directory.* files.
  MSBUILDDISABLENODEREUSE: '1',
};

function dotnet(args, cwd, extraEnv = {}) {
  console.log(`\n> dotnet ${args.join(' ')}   (cwd ${cwd})`);
  const result = spawnSync('dotnet', args, { cwd, env: { ...env, ...extraEnv }, stdio: 'inherit' });
  if (result.error) {
    throw result.error;
  }
  return result.status ?? 1;
}

function must(status, what) {
  if (status !== 0) {
    throw new Error(`${what} failed with exit code ${status}`);
  }
}

const nugetConfig = `<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <packageSources>
    <clear />
    <add key="local-policy-catalog" value="${feed}" />
    <add key="upstream" value="${upstream}" />
  </packageSources>
  <packageSourceMapping>
    <packageSource key="local-policy-catalog">
      <package pattern="Microsoft.Mxc.PolicyCatalog" />
    </packageSource>
    <packageSource key="upstream">
      <package pattern="*" />
    </packageSource>
  </packageSourceMapping>
</configuration>
`;
// The consumers pin the same SDK policy as the repository (MTP test runner).
const globalJson = `${JSON.stringify({ sdk: { version: '10.0.401', rollForward: 'latestFeature' }, test: { runner: 'Microsoft.Testing.Platform' } }, null, 2)}\n`;

function consumer(name, template) {
  const dir = join(work, name);
  cpSync(template, dir, {
    recursive: true,
    filter: source => !/[\\/](bin|obj|TestResults)([\\/]|$)/.test(relative(template, source)),
  });
  writeFileSync(join(dir, 'nuget.config'), nugetConfig);
  writeFileSync(join(dir, 'global.json'), globalJson);
  // Stop MSBuild from importing Directory.Build.* from parent directories.
  writeFileSync(join(dir, 'Directory.Build.props'), '<Project />\n');
  writeFileSync(join(dir, 'Directory.Build.targets'), '<Project />\n');
  return dir;
}

let status = 0;
try {
  // 1. Pack.
  must(dotnet(['pack', join(dotnetDir, 'Microsoft.Mxc.PolicyCatalog', 'Microsoft.Mxc.PolicyCatalog.csproj'), '-c', 'Release', '-o', feed, '-p:ContinuousIntegrationBuild=true'], dotnetDir), 'dotnet pack');
  const nupkgs = readdirSync(feed).filter(f => f.endsWith('.nupkg'));
  if (!nupkgs.includes(`Microsoft.Mxc.PolicyCatalog.${version}.nupkg`)) {
    throw new Error(`expected Microsoft.Mxc.PolicyCatalog.${version}.nupkg in ${feed}, found ${nupkgs.join(', ')}`);
  }
  console.log(`\nPacked ${nupkgs.join(', ')} into ${feed}`);

  // 2a. CLI consumer, built against the package.
  const cliDir = consumer('cli', join(dotnetDir, 'functional', 'policy-catalog-cli-consumer'));
  cpSync(join(dotnetDir, 'Microsoft.Mxc.PolicyCatalog.Cli', 'Program.cs'), join(cliDir, 'Program.cs'));
  must(dotnet(['build', '-c', 'Release', '-o', join(cliDir, 'out')], cliDir), 'CLI consumer build');
  const cliDll = join(cliDir, 'out', 'policy-catalog.dll');
  if (!existsSync(cliDll) || !existsSync(join(cliDir, 'out', 'catalog', 'manifest.json'))) {
    throw new Error(`CLI consumer build did not produce ${cliDll} with its catalog directory`);
  }

  // 2b. Functional tests consumer.
  const testDir = consumer('tests', join(dotnetDir, 'functional', 'Microsoft.Mxc.PolicyCatalog.FunctionalTests'));
  must(dotnet(['build', '-c', 'Release'], testDir), 'functional test build');

  // 3. Run. xUnit v3 test projects are executables; MTP prints the counts.
  status = dotnet(['test', '--project', join(testDir, 'Microsoft.Mxc.PolicyCatalog.FunctionalTests.csproj'), '-c', 'Release', '--no-build'], testDir, {
    POLICY_CATALOG_CLI_DLL: cliDll,
    POLICY_CATALOG_REPO_ROOT: root,
  });
  console.log(status === 0 ? '\n.NET functional tests passed.' : `\n.NET functional tests FAILED (exit ${status}).`);
} catch (error) {
  console.error(`\n${error.message}`);
  status = 1;
} finally {
  if (process.env.POLICY_CATALOG_KEEP_TEMP === '1') {
    console.log(`kept ${work}`);
  } else {
    rmSync(work, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  }
}
process.exit(status);
