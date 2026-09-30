// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Functional tests through the packaged CLI and library: unknown tools, weak
// opt-in, composition conflicts, unavailable revisions, architecture
// selection, multi-match, integrity tamper, and dependency cycles.
using System.Text.Json.Nodes;
using Xunit;
using static Microsoft.Mxc.PolicyCatalog.FunctionalTests.Fx;

namespace Microsoft.Mxc.PolicyCatalog.FunctionalTests;

public sealed class CliCommandTests
{
    private static string[] Split(string joined) => joined.Length == 0 ? Array.Empty<string>() : joined.Split('|');

    [Fact]
    public void InspectReportsMetadataOnly()
    {
        var result = Cli("inspect");
        Assert.Equal(0, result.Status);
        Assert.Equal(InstalledManifest["defaultRevision"]!.GetValue<string>(), result.Json!["info"]!["catalogRevision"]!.GetValue<string>());
        Assert.Equal(new[] { "tool:git", "tool:node", "tool:npm" }, result.Json["entries"]!.AsArray().Select(e => e!["entryId"]!.GetValue<string>()));
        Assert.DoesNotContain("readwritePaths", result.Stdout, StringComparison.Ordinal);
        Assert.DoesNotContain("${", result.Stdout, StringComparison.Ordinal);
        Assert.EndsWith("}\n", result.Stdout, StringComparison.Ordinal);
    }

    [Fact]
    public void ValidateBundledCatalog()
    {
        var result = Cli("validate");
        Assert.Equal(0, result.Status);
        Assert.True(result.Json!["ok"]!.GetValue<bool>());
        Assert.Empty(result.Json["errors"]!.AsArray());
        Assert.Null(result.Json["baseRef"]);
    }

    [Theory]
    [InlineData("")]
    [InlineData("bogus")]
    [InlineData("info")]
    [InlineData("resolve|--platform")]
    [InlineData("resolve|--symbol|noequals|git")]
    [InlineData("resolve|--symbol|=x|git")]
    [InlineData("resolve|--purl|pkg:npm/npm")]
    [InlineData("resolve|--base-ref|HEAD|git")]
    [InlineData("resolve|--platform|--allow-weak|git")]
    [InlineData("inspect|extra")]
    [InlineData("validate|extra")]
    [InlineData("validate|--base-ref")]
    [InlineData("resolve|--catalog")]
    public void UsageErrorsExit2WithoutStdout(string joined)
    {
        var result = Cli(Split(joined));
        Assert.Equal(2, result.Status);
        Assert.Equal(string.Empty, result.Stdout);
        Assert.StartsWith("policy-catalog: ", result.Stderr, StringComparison.Ordinal);
        Assert.EndsWith("usage: policy-catalog <resolve|inspect|validate> [options]\n", result.Stderr.Replace("\r\n", "\n"), StringComparison.Ordinal);
    }

    [Theory]
    [InlineData("resolve|--platform|plan9|git")]
    [InlineData("resolve|--architecture|mips|git")]
    [InlineData("resolve|--platform|linux|--architecture|x64|--allow-weak|--symbol|node_prefix=relative|node")]
    [InlineData("resolve|--platform|linux|--architecture|x64|--symbol|unknown_symbol=/x|git")]
    [InlineData("resolve|--platform|linux|--architecture|x64|--symbol|__proto__=/x|git")]
    [InlineData("resolve|--platform|linux|--architecture|x64|--purl|not-a-purl|npm")]
    [InlineData("resolve|--platform|linux|--architecture|x64|./bin/git")]
    public void InvalidCallerInputIsInvalidContext(string joined)
    {
        var result = Cli(Split(joined));
        Assert.Equal(1, result.Status);
        Assert.Equal("malformed_request", result.Json!["error"]!["code"]!.GetValue<string>());
        Assert.Equal("invalid_context", result.ErrorReason);
    }

    public static TheoryData<string, string> PlatformArch()
    {
        var data = new TheoryData<string, string>();
        foreach (var p in Platforms)
        {
            foreach (var a in Architectures)
            {
                data.Add(p, a);
            }
        }

        return data;
    }

    [Theory]
    [MemberData(nameof(PlatformArch))]
    public void ResolvesFullToolSetDeterministically(string platform, string architecture)
    {
        var c = PlatformContext[platform];
        var first = Cli(Args("resolve", "--diagnostics", FullContext(platform, architecture), "git", "npm", "node"));
        var second = Cli(Args("resolve", "--diagnostics", FullContext(platform, architecture), "git", "npm", "node"));
        Assert.Equal(0, first.Status);
        Assert.Equal(first.Stdout, second.Stdout);
        var policy = first.Json!["policy"]!;
        Assert.Equal(new[] { c.Root, c.Cache }, policy["filesystem"]!["readwritePaths"]!.AsArray().Select(p => p!.GetValue<string>()));
        Assert.Equal(new[] { c.Prefix }, policy["filesystem"]!["readonlyPaths"]!.AsArray().Select(p => p!.GetValue<string>()));
        Assert.Null(policy["network"]);
        Assert.Equal("""[{"entryId":"tool:node","entryRevision":1}]""", first.Json["diagnostics"]!["resolvedDependencies"]!.ToJsonString(Relaxed));
        Assert.Contains($"tool:git uses its architecture-neutral {platform} variant; no {architecture}-specific variant exists", first.Warnings, StringComparison.Ordinal);
        Assert.DoesNotContain("architecture was not specified", first.Warnings, StringComparison.Ordinal);
        var plain = Cli(Args("resolve", FullContext(platform, architecture), "git", "npm", "node"));
        Assert.Equal(policy.ToJsonString(Relaxed), plain.Json!.ToJsonString(Relaxed));
    }

    [Fact]
    public void EmptyInputAndUnresolvedSymbolYieldNoPolicy()
    {
        Assert.Equal("null\n", Cli("resolve", "--platform", "linux", "--architecture", "x64").Stdout);
        var empty = Cli("resolve", "--diagnostics", "--platform", "linux", "--architecture", "x64");
        Assert.Equal(0, empty.Status);
        Assert.Null(empty.Json!["policy"]);
        Assert.DoesNotContain("\"policy\"", empty.Stdout, StringComparison.Ordinal);
        var unresolved = Cli("resolve", "--diagnostics", "--platform", "linux", "--architecture", "x64", "--allow-weak", "--symbol", "git_prefix=/usr/bin", "git");
        Assert.Contains("required symbol 'project_root' (needed by tool:git) is unresolved; supply ResolveContext.projectRoot; no policy was returned", unresolved.Warnings, StringComparison.Ordinal);
    }
}

public sealed class ResolutionTests : WorkDirTest
{
    private string MultiCatalog() => WriteCatalog(NewDir("multi-"), new[]
    {
        Revision(new JsonNode[]
        {
            Entry("tool:app", """{ "identity": [{ "kind": "purl", "value": "pkg:npm/app" }, { "kind": "invocation-name", "names": ["app"] }] }"""),
            Entry("tool:app-plugin", """{ "identity": [{ "kind": "invocation-name", "names": ["app"] }] }"""),
        }),
    });

    [Fact]
    public void UnknownToolYieldsNoPolicyWithWarning()
    {
        var result = Cli(Args("resolve", "--diagnostics", FullContext("linux", "x64"), "cargo"));
        Assert.Equal(0, result.Status);
        Assert.Null(result.Json!["policy"]);
        Assert.Equal("""[{"inputIndex":0,"matches":[]}]""", result.Json["diagnostics"]!["tools"]!.ToJsonString(Relaxed));
        Assert.Equal("input 0 ('cargo') matched no eligible catalog entry", result.Warnings);
        Assert.Equal("null\n", Cli(Args("resolve", FullContext("linux", "x64"), "cargo")).Stdout);
        var library = BundledPolicyCatalog.GetSandboxConfigWithDiagnostics("cargo", new ResolveContext { Platform = "linux", Architecture = "x64" });
        Assert.Null(library.Policy);
        Assert.Equal(new[] { "input 0 ('cargo') matched no eligible catalog entry" }, library.Diagnostics.Warnings);
    }

    [Fact]
    public void WeakIdentityRequiresOptIn()
    {
        var common = Args("--diagnostics", "--platform", "linux", "--architecture", "x64", SymbolArgs("linux"));
        var off = Cli(Args("resolve", common, "git"));
        Assert.Equal(0, off.Status);
        Assert.Null(off.Json!["policy"]);
        Assert.Equal("input 0 ('git') matched no eligible catalog entry: tool:git matched only by invocation name and allowWeakIdentityFallback is not enabled", off.Warnings);
        var on = Cli(Args("resolve", common, "--allow-weak", "git"));
        Assert.NotNull(on.Json!["policy"]);
        Assert.Contains("input 0 ('git') matched tool:git only by invocation name (weak identity)", on.Warnings, StringComparison.Ordinal);

        var catalog = new PolicyCatalog(CatalogStore.Bundled());
        var ctx = new ResolveContext { Platform = "linux", Architecture = "x64", ProjectRoot = "/p", Symbols = new Dictionary<string, string?> { ["git_prefix"] = "/usr/bin" } };
        Assert.Null(catalog.GetSandboxConfig("git", ctx));
        Assert.NotNull(catalog.GetSandboxConfig(new ToolInput("git"), ctx with { AllowWeakIdentityFallback = true }));

        var strong = Cli(Args("resolve", common, "--purl", "pkg:npm/npm@11.0.0", "npm"));
        Assert.NotNull(strong.Json!["policy"]);
        Assert.DoesNotContain("weak identity", strong.Warnings, StringComparison.Ordinal);
    }

    [Fact]
    public void MultiMatchWarnsAndComposesEveryEntry()
    {
        var dir = MultiCatalog();
        var result = Cli("resolve", "--catalog", dir, "--diagnostics", "--platform", "linux", "--architecture", "x64", "--allow-weak", "--symbol", "git_prefix=/opt", "app");
        Assert.Equal(0, result.Status);
        Assert.Contains("input 0 ('app') matched 2 entries (tool:app, tool:app-plugin); all contribute", result.Warnings, StringComparison.Ordinal);
        Assert.Equal("""["/opt/app","/opt/app-plugin"]""", result.Json!["policy"]!["filesystem"]!["readonlyPaths"]!.ToJsonString(Relaxed));
        var strongOnly = Cli("resolve", "--catalog", dir, "--diagnostics", "--platform", "linux", "--architecture", "x64", "--symbol", "git_prefix=/opt", "--purl", "pkg:npm/app", "app");
        Assert.DoesNotContain("matched 2 entries", strongOnly.Warnings, StringComparison.Ordinal);
    }

    [Fact]
    public void CompositionConflicts()
    {
        var overlap = Cli("resolve", "--platform", "linux", "--architecture", "x64", "--allow-weak", "--project-root", "/opt", "--symbol", "git_prefix=/usr/bin", "--symbol", "node_prefix=/opt/node", "git", "node");
        Assert.Equal(1, overlap.Status);
        Assert.Equal("composition_conflict", overlap.ErrorReason);
        Assert.Contains("overlap across access classes", overlap.ErrorMessage, StringComparison.Ordinal);

        var dir = WriteCatalog(NewDir("compose-"), new[]
        {
            Revision(new JsonNode[]
            {
                Entry("tool:a"),
                Entry("tool:b", """{ "platformVariants": [{ "when": { "platform": "linux" }, "sandboxPolicy": { "version": "0.9.0-alpha", "network": { "egress": { "default": "deny" } } } }] }"""),
            }),
        });
        Assert.Equal(0, Cli("validate", "--catalog", dir).Status);
        var alone = Cli("resolve", "--catalog", dir, "--platform", "linux", "--architecture", "x64", "--allow-weak", "b");
        Assert.Equal("""{"egress":{"default":"deny"}}""", alone.Json!["network"]!.ToJsonString(Relaxed));
        var both = Cli("resolve", "--catalog", dir, "--platform", "linux", "--architecture", "x64", "--allow-weak", "--symbol", "git_prefix=/opt", "a", "b");
        Assert.Equal(1, both.Status);
        Assert.Equal("[policy_validation] selected entries cannot be composed: 'tool:b' uses 'network', which has no v1 cross-entry composition rule", both.ErrorMessage);
    }

    [Fact]
    public void UnavailableRevisionIsNeverSubstituted()
    {
        var result = Cli(Args("resolve", "--revision", "2099-01-01.1", FullContext("linux", "x64"), "git"));
        Assert.Equal(1, result.Status);
        Assert.Equal("""{"code":"backend_error","message":"[backend_error] catalog revision '2099-01-01.1' is not installed","details":{"reason":"revision_unavailable"}}""", result.Json!["error"]!.ToJsonString(Relaxed));
        var error = Assert.Throws<PolicyCatalogException>(() => BundledPolicyCatalog.GetSandboxConfig("git", new ResolveContext { CatalogRevision = "2099-01-01.1", Platform = "linux", Architecture = "x64" }));
        Assert.Equal("revision_unavailable", error.Reason);

        var r1 = Revision(new JsonNode[] { Entry("tool:a") }, "2000-01-01.1");
        var r2 = Revision(new JsonNode[] { Entry("tool:a", """{ "entryRevision": 2, "platformVariants": [{ "when": { "platform": "linux" }, "sandboxPolicy": { "version": "0.9.0-alpha", "filesystem": { "readonlyPaths": ["${git_prefix}/v2"] } } }] }""") }, "2000-01-02.1");
        var dir = WriteCatalog(NewDir("revisions-"), new[] { r1, r2 });
        Assert.Equal(0, Cli("validate", "--catalog", dir).Status);
        var common = new[] { "--catalog", dir, "--platform", "linux", "--architecture", "x64", "--allow-weak", "--symbol", "git_prefix=/opt", "a" };
        Assert.Equal("""["/opt/v2"]""", Cli(Args("resolve", common)).Json!["filesystem"]!["readonlyPaths"]!.ToJsonString(Relaxed));
        Assert.Equal("""["/opt/a"]""", Cli(Args("resolve", "--revision", "2000-01-01.1", common)).Json!["filesystem"]!["readonlyPaths"]!.ToJsonString(Relaxed));
    }

    [Theory]
    [MemberData(nameof(CliCommandTests.PlatformArch), MemberType = typeof(CliCommandTests))]
    public void HostDefaultArchitectureAndNeutralFallbackWarnings(string platform, string architecture)
    {
        var catalog = new PolicyCatalog(CatalogStore.Bundled(), new FixedHost(platform, architecture));
        var c = PlatformContext[platform];
        var ctx = new ResolveContext { AllowWeakIdentityFallback = true, ProjectRoot = c.Root, Symbols = new Dictionary<string, string?> { ["git_prefix"] = c.Prefix } };
        var omitted = catalog.GetSandboxConfigWithDiagnostics("git", ctx);
        Assert.Equal(
            new[]
            {
                "input 0 ('git') matched tool:git only by invocation name (weak identity)",
                $"architecture was not specified; variants were selected for the native system architecture '{architecture}'; the tool's architecture was not verified",
                $"tool:git uses its architecture-neutral {platform} variant; no {architecture}-specific variant exists",
            },
            omitted.Diagnostics.Warnings);
        var explicitArch = catalog.GetSandboxConfigWithDiagnostics("git", ctx with { Platform = platform, Architecture = architecture });
        Assert.DoesNotContain(explicitArch.Diagnostics.Warnings, w => w.StartsWith("architecture was not specified", StringComparison.Ordinal));
        Assert.Equal(PolicyCatalogJson.Serialize(omitted.Policy), PolicyCatalogJson.Serialize(explicitArch.Policy));

        // The packaged CLI detects this machine's native architecture.
        var cli = Cli(Args("resolve", "--diagnostics", "--platform", platform, "--allow-weak", SymbolArgs(platform), "git"));
        Assert.Equal(0, cli.Status);
        Assert.Matches("architecture was not specified; variants were selected for the native system architecture '(x64|arm64)'; the tool's architecture was not verified", cli.Warnings);
    }

    [Fact]
    public void ArchitectureMismatchIsNoMatchWithSkippedReason()
    {
        var dir = WriteCatalog(NewDir("arch-"), new[]
        {
            Revision(new JsonNode[]
            {
                Entry("tool:a", """
                    { "platformVariants": [
                      { "when": { "platform": "windows", "architecture": "arm64" }, "sandboxPolicy": { "version": "0.9.0-alpha", "filesystem": { "readonlyPaths": ["${git_prefix}\\arm64"] } } },
                      { "when": { "platform": "windows", "architecture": "x64" }, "sandboxPolicy": { "version": "0.9.0-alpha", "filesystem": { "readonlyPaths": ["${git_prefix}\\x64"] } } },
                      { "when": { "platform": "linux", "architecture": "x64" }, "sandboxPolicy": { "version": "0.9.0-alpha", "filesystem": { "readonlyPaths": ["${git_prefix}/x64"] } } } ] }
                    """),
            }),
        });
        foreach (var architecture in Architectures)
        {
            var result = Cli("resolve", "--catalog", dir, "--diagnostics", "--allow-weak", "--platform", "windows", "--architecture", architecture, "--symbol", "git_prefix=C:\\t", "a");
            Assert.Equal($"[\"C:\\\\t\\\\{architecture}\"]", result.Json!["policy"]!["filesystem"]!["readonlyPaths"]!.ToJsonString(Relaxed));
            Assert.DoesNotContain("architecture-neutral", result.Warnings, StringComparison.Ordinal);
        }

        var noArm = Cli("resolve", "--catalog", dir, "--diagnostics", "--allow-weak", "--platform", "linux", "--architecture", "arm64", "--symbol", "git_prefix=/t", "a");
        Assert.Equal(0, noArm.Status);
        Assert.Null(noArm.Json!["policy"]);
        Assert.Equal("input 0 ('a') matched no eligible catalog entry: tool:a has no variant for linux/arm64", noArm.Warnings);
    }
}

public sealed class IntegrityTests : WorkDirTest
{
    private string CopyInstalled()
    {
        var dir = NewDir("catalog-");
        CopyDirectory(InstalledCatalogDir, dir);
        return dir;
    }

    private static string LatestFile(string dir) =>
        Path.Combine(dir, InstalledManifest["revisions"]!.AsArray()[^1]!["file"]!.GetValue<string>());

    [Fact]
    public void UntouchedCopyValidatesAndFormattingOnlyEditsKeepTheDigest()
    {
        var dir = CopyInstalled();
        Assert.Equal(0, Cli("validate", "--catalog", dir).Status);
        var ctx = FullContext("linux", "x64");
        Assert.Equal(Cli(Args("resolve", ctx, "git")).Stdout, Cli(Args("resolve", "--catalog", dir, ctx, "git")).Stdout);
        var file = LatestFile(dir);
        File.WriteAllText(file, JsonNode.Parse(File.ReadAllText(file))!.ToJsonString(Relaxed).Replace(",", ",\r\n"));
        Assert.Equal(0, Cli("validate", "--catalog", dir).Status);
    }

    [Fact]
    public void TamperedRevisionFailsIntegrityEverywhere()
    {
        var dir = CopyInstalled();
        var file = LatestFile(dir);
        var data = JsonNode.Parse(File.ReadAllText(file))!;
        data["entries"]![0]!["platformVariants"]![0]!["sandboxPolicy"]!["filesystem"]!["readwritePaths"]!.AsArray().Add("${user_home}");
        File.WriteAllText(file, data.ToJsonString(Relaxed));

        var validate = Cli("validate", "--catalog", dir);
        Assert.Equal(1, validate.Status);
        Assert.False(validate.Json!["ok"]!.GetValue<bool>());
        Assert.Matches("\\[backend_error\\] catalog revision '[^']+' digest [0-9a-f]{64} does not match the published digest", validate.Json["errors"]!.ToJsonString(Relaxed));

        var resolve = Cli(Args("resolve", "--catalog", dir, FullContext("windows", "x64"), "git"));
        Assert.Equal(1, resolve.Status);
        Assert.Equal("integrity", resolve.ErrorReason);
        Assert.Equal("backend_error", resolve.Json!["error"]!["code"]!.GetValue<string>());
        Assert.Equal("integrity", Cli("inspect", "--catalog", dir).ErrorReason);

        var error = Assert.Throws<PolicyCatalogException>(() => new PolicyCatalog(CatalogStore.FromDirectory(dir)).GetCatalogInfo());
        Assert.Equal(("backend_error", "integrity"), (error.Code, error.Reason));
    }

    [Fact]
    public void MissingRevisionFileAndBadManifest()
    {
        var dir = CopyInstalled();
        File.Delete(LatestFile(dir));
        Assert.Equal("integrity", Cli("inspect", "--catalog", dir).ErrorReason);
        Assert.Contains("could not be read", Cli("validate", "--catalog", dir).Json!["errors"]!.ToJsonString(Relaxed), StringComparison.Ordinal);

        var bad = CopyInstalled();
        var manifest = InstalledManifest;
        manifest["defaultRevision"] = "2099-01-01.1";
        File.WriteAllText(Path.Combine(bad, "manifest.json"), manifest.ToJsonString(Relaxed));
        Assert.Equal("invalid_catalog", Cli("inspect", "--catalog", bad).ErrorReason);
        Assert.Equal(
            """["[policy_validation] manifest.defaultRevision must name a listed revision"]""",
            Cli("validate", "--catalog", bad).Json!["errors"]!.ToJsonString(Relaxed));
    }

    [Fact]
    public void ValidateRejectsADependencyCycle()
    {
        var dir = WriteCatalog(NewDir("cycle-"), new[]
        {
            Revision(new JsonNode[] { DependsOn("tool:a", "tool:b"), DependsOn("tool:b", "tool:c"), DependsOn("tool:c", "tool:a") }),
        });
        var result = Cli("validate", "--catalog", dir);
        Assert.Equal(1, result.Status);
        Assert.False(result.Json!["ok"]!.GetValue<bool>());
        Assert.Contains("[policy_validation] 'tool:a' on linux/x64: cycle (tool:a -> tool:b -> tool:c -> tool:a)", result.Json["errors"]![0]!.GetValue<string>(), StringComparison.Ordinal);
    }

    [Fact]
    public void ResolveNeverReturnsAPolicyFromACyclicCatalog()
    {
        var dir = WriteCatalog(NewDir("cycle-"), new[] { Revision(new JsonNode[] { DependsOn("tool:a", "tool:b"), DependsOn("tool:b", "tool:a") }) });
        var result = Cli("resolve", "--catalog", dir, "--platform", "linux", "--architecture", "x64", "--allow-weak", "--symbol", "git_prefix=/opt", "a");
        Assert.Equal(1, result.Status);
        Assert.Equal("policy_validation", result.Json?["error"]?["code"]?.GetValue<string>());
        Assert.Equal("invalid_catalog", result.ErrorReason);
        Assert.Contains("cycle (tool:a -> tool:b -> tool:a)", result.ErrorMessage, StringComparison.Ordinal);
        var error = Assert.Throws<PolicyCatalogException>(() => new PolicyCatalog(CatalogStore.FromDirectory(dir)).GetSandboxConfig("a", new ResolveContext { AllowWeakIdentityFallback = true, Platform = "linux", Architecture = "x64" }));
        Assert.Equal("invalid_catalog", error.Reason);
    }

    [Fact]
    public void DiamondIsValidAndContributesTheSharedEntryOnce()
    {
        var dir = WriteCatalog(NewDir("diamond-"), new[]
        {
            Revision(new JsonNode[]
            {
                Entry("tool:a", """{ "platformVariants": [{ "when": { "platform": "linux" }, "dependencies": [{ "entryId": "tool:b" }, { "entryId": "tool:c" }], "sandboxPolicy": { "version": "0.9.0-alpha", "filesystem": { "readonlyPaths": ["${git_prefix}/a"] } } }] }"""),
                DependsOn("tool:b", "tool:d"),
                DependsOn("tool:c", "tool:d"),
                Entry("tool:d"),
            }),
        });
        Assert.Equal(0, Cli("validate", "--catalog", dir).Status);
        var result = Cli("resolve", "--catalog", dir, "--diagnostics", "--platform", "linux", "--architecture", "x64", "--allow-weak", "--symbol", "git_prefix=/opt", "a");
        Assert.Equal("""["/opt/a","/opt/d-dep","/opt/d"]""", result.Json!["policy"]!["filesystem"]!["readonlyPaths"]!.ToJsonString(Relaxed));
    }
}
