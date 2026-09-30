# Microsoft.Mxc.PolicyCatalog (.NET) — PROTOTYPE

Native C# implementation of the policy catalog library and the `policy-catalog`
CLI, behavior-equivalent to the TypeScript reference in `../src`. It targets
`net8.0` and depends only on inbox libraries (`System.Text.Json`,
`Microsoft.Win32.Registry` on Windows). The bundled catalog is embedded from
`../catalog/` (the single source of truth; nothing is copied into this folder).

`global.json` pins SDK `10.0.401` (`rollForward: latestFeature`) with the
Microsoft.Testing.Platform test runner. SDK 8.0.x cannot run MTP-mode
`dotnet test` or read `.slnx` solutions. The projects still target `net8.0`.

Run everything from this directory (`dotnet/`):

```sh
# Build (warnings are errors)
dotnet build Microsoft.Mxc.PolicyCatalog.slnx -c Release

# Unit, conformance (../conformance/fixtures), and vector (../conformance/vectors) tests
dotnet test --solution Microsoft.Mxc.PolicyCatalog.slnx -c Release

# Pack the library
dotnet pack Microsoft.Mxc.PolicyCatalog/Microsoft.Mxc.PolicyCatalog.csproj -c Release -o <tmp-dir>

# CLI (from source)
dotnet Microsoft.Mxc.PolicyCatalog.Cli/bin/Release/net8.0/policy-catalog.dll inspect
```

## Functional tests (packaged artifact)

```sh
# from policy-catalog/
node scripts/dotnet-functional.mjs
```

The driver packs the library into a local feed under `os.tmpdir()`, then
builds two consumers outside the repository against that package:

- the CLI (`functional/policy-catalog-cli-consumer` + `Microsoft.Mxc.PolicyCatalog.Cli/Program.cs`)
- the functional tests (`functional/Microsoft.Mxc.PolicyCatalog.FunctionalTests`)

Each consumer gets a generated `nuget.config` and an isolated `NUGET_PACKAGES`
directory. The tests call the packaged library directly and run the packaged
CLI in a separate process. The functional project is not part of the solution.

Third-party packages (xunit.v3) come from nuget.org. Where nuget.org is not
directly reachable, set `POLICY_CATALOG_NUGET_UPSTREAM` to a mirror feed URL.
Set `POLICY_CATALOG_KEEP_TEMP=1` to keep the temporary directory.
