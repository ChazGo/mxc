// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using Xunit;

namespace Microsoft.Mxc.PolicyCatalog.Tests;

/// <summary>Port of tests/unit/resolver.test.ts.</summary>
public sealed class ResolverTests
{
    private static readonly ResolveContext Weak = new() { AllowWeakIdentityFallback = true };

    private static PolicyCatalog ArchCatalog(IHostEnvironment? host = null) => TestData.CatalogFor(TestData.Revision(new[]
    {
        TestData.Entry("tool:a", """
            { "identity": [{ "kind": "purl", "value": "pkg:npm/a" }, { "kind": "invocation-name", "names": ["a"] }],
              "platformVariants": [
                { "when": { "platform": "windows", "architecture": "x64" }, "sandboxPolicy": { "version": "0.9.0-alpha", "timeoutMs": 1 } },
                { "when": { "platform": "windows", "architecture": "arm64" }, "sandboxPolicy": { "version": "0.9.0-alpha", "timeoutMs": 2 } } ] }
            """),
    }), host ?? new FixedHost("windows", "arm64"));

    private static string Warnings(SandboxConfigResolution result) => string.Join("\n", result.Diagnostics.Warnings);

    [Fact]
    public void OmittedContextUsesHostDefaultsAndNoWeakFallback()
    {
        var catalog = ArchCatalog();
        Assert.Null(catalog.ResolveSandboxPolicy("a"));
        var result = catalog.ResolveSandboxPolicyWithDiagnostics(new ToolInput("a") { PackageUrl = "pkg:npm/a" });
        Assert.Equal(2.0, result.Policy?.TimeoutMs);
        Assert.Equal("2000-01-01.1", result.Diagnostics.CatalogRevision);
        Assert.Contains("native system architecture 'arm64'; the tool's architecture was not verified", Warnings(result), StringComparison.Ordinal);
    }

    [Fact]
    public void ExplicitArchitectureWinsAndSuppressesTheWarning()
    {
        var result = ArchCatalog().ResolveSandboxPolicyWithDiagnostics(new ToolInput("a") { PackageUrl = "pkg:npm/a" }, new ResolveContext { Architecture = "x64" });
        Assert.Equal(1.0, result.Policy?.TimeoutMs);
        Assert.Empty(result.Diagnostics.Warnings);
    }

    [Fact]
    public void AnotherArchitectureIsNeverAFallback()
    {
        var catalog = TestData.CatalogFor(TestData.Revision(new[]
        {
            TestData.Entry("tool:a", """{ "platformVariants": [{ "when": { "platform": "linux", "architecture": "arm64" }, "sandboxPolicy": { "version": "0.9.0-alpha" } }] }"""),
        }));
        var result = catalog.ResolveSandboxPolicyWithDiagnostics("a", Weak with { Architecture = "x64" });
        Assert.Null(result.Policy);
        Assert.Equal("input 0 ('a') matched no eligible catalog entry: tool:a has no variant for linux/x64", result.Diagnostics.Warnings[0]);
    }

    [Fact]
    public void ArchitectureDetectionFailureIsALibraryErrorAndIsLazy()
    {
        var host = new FixedHost("linux", () => throw new PolicyCatalogException(PolicyCatalogErrorReason.UnsupportedHost, "unknown machine"));
        var catalog = new PolicyCatalog(TestData.StoreFor(new[] { TestData.Revision(new[] { TestData.Entry("tool:t") }) }), host);
        var error = TestData.Failure(() => catalog.ResolveSandboxPolicy("t", Weak with { ProjectRoot = "/p" }));
        Assert.Equal(("unsupported_containment", "unsupported_host"), (error.Code, error.Reason));
        Assert.Null(catalog.ResolveSandboxPolicy("nothing", Weak));
        var explicitArch = catalog.ResolveSandboxPolicy("t", Weak with { Architecture = "x64", ProjectRoot = "/p" });
        Assert.Equal(new[] { "/p" }, explicitArch?.Filesystem?.ReadwritePaths);
    }

    [Fact]
    public void NeverFabricatesSymbols()
    {
        var result = TestData.Bundled().ResolveSandboxPolicyWithDiagnostics("git", Weak with { Architecture = "x64" });
        Assert.Null(result.Policy);
        Assert.Matches("(?s)required symbol 'git_prefix'.*required symbol 'project_root'", Warnings(result));
    }

    [Fact]
    public void HostSymbolsOnlyForTheCurrentHostPlatform()
    {
        var revision = TestData.Revision(new[]
        {
            TestData.Entry("tool:t", """
                { "platformVariants": [
                  { "when": { "platform": "linux" }, "sandboxPolicy": { "version": "0.9.0-alpha", "filesystem": { "readonlyPaths": ["${user_home}/.cfg"] } } },
                  { "when": { "platform": "macos" }, "sandboxPolicy": { "version": "0.9.0-alpha", "filesystem": { "readonlyPaths": ["${user_home}/.cfg"] } } } ] }
                """),
        });
        var catalog = TestData.CatalogFor(revision, new FixedHost("linux", "x64", new Dictionary<string, string> { ["user_home"] = "/home/me" }));
        Assert.Equal(new[] { "/home/me/.cfg" }, catalog.ResolveSandboxPolicy("t", Weak)?.Filesystem?.ReadonlyPaths);
        Assert.Null(catalog.ResolveSandboxPolicy("t", Weak with { Platform = "macos", Architecture = "arm64" }));
        Assert.Equal(new[] { "/srv/u/.cfg" }, catalog.ResolveSandboxPolicy("t", Weak with { Symbols = new Dictionary<string, string?> { ["user_home"] = "/srv/u" } })?.Filesystem?.ReadonlyPaths);
    }

    [Fact]
    public void StringObjectAndOneElementListAreEquivalent()
    {
        var catalog = TestData.Bundled();
        var ctx = Weak with { Architecture = "x64", ProjectRoot = "/p", Symbols = new Dictionary<string, string?> { ["git_prefix"] = "/g" } };
        var a = PolicyCatalogJson.Serialize(catalog.ResolveSandboxPolicyWithDiagnostics("git", ctx));
        Assert.Equal(a, PolicyCatalogJson.Serialize(catalog.ResolveSandboxPolicyWithDiagnostics(new ToolInput("git"), ctx)));
        Assert.Equal(a, PolicyCatalogJson.Serialize(catalog.ResolveSandboxPolicyWithDiagnostics(new ToolInput[] { "git" }, ctx)));
        Assert.Null(catalog.ResolveSandboxPolicy("git", ctx with { AllowWeakIdentityFallback = false }));
    }

    public static TheoryData<string, ToolInput?, ResolveContext> InvalidInputs() => new()
    {
        { "empty name", "", new ResolveContext() },
        { "path", "/usr/bin/git", new ResolveContext() },
        { "bad purl", new ToolInput("npm") { PackageUrl = "npm" }, new ResolveContext() },
        { "empty version", new ToolInput("npm") { DetectedVersion = "" }, new ResolveContext() },
        { "null input", null, new ResolveContext() },
        { "platform", "git", new ResolveContext { Platform = "plan9" } },
        { "architecture", "git", new ResolveContext { Architecture = "x86" } },
        { "unknown symbol", "git", new ResolveContext { Symbols = new Dictionary<string, string?> { ["nope"] = "/x" } } },
        { "context symbol", "git", new ResolveContext { Symbols = new Dictionary<string, string?> { ["project_root"] = "/x" } } },
        { "null symbol", "git", new ResolveContext { Symbols = new Dictionary<string, string?> { ["git_prefix"] = null } } },
        { "proto symbol", "git", new ResolveContext { Symbols = new Dictionary<string, string?> { ["__proto__"] = "/x" } } },
        { "empty root", "git", new ResolveContext { ProjectRoot = "" } },
    };

    [Theory]
    [MemberData(nameof(InvalidInputs))]
    public void InvalidInputsAreLibraryFailures(string label, ToolInput? tool, ResolveContext ctx)
    {
        Assert.NotEmpty(label);
        Assert.Equal("invalid_context", TestData.ErrorReason(() => TestData.Bundled().ResolveSandboxPolicy(new[] { tool! }, ctx)));
    }

    [Fact]
    public void RelativeOrTemplatedSymbolValuesAreRejected()
    {
        var catalog = TestData.Bundled();
        var ctx = Weak with { Platform = "linux", Architecture = "x64" };
        var error = TestData.Failure(() => catalog.ResolveSandboxPolicy("node", ctx with { Symbols = new Dictionary<string, string?> { ["node_prefix"] = "bin" } }));
        Assert.Equal("[malformed_request] symbol 'node_prefix' must resolve to an absolute linux path", error.Message);
        Assert.Equal("invalid_context", TestData.ErrorReason(() => catalog.ResolveSandboxPolicy("node", ctx with { Symbols = new Dictionary<string, string?> { ["node_prefix"] = "/x/${git_prefix}" } })));
    }

    [Fact]
    public void ResultsAreDeterministicCopies()
    {
        var catalog = TestData.Bundled();
        var ctx = new ResolveContext { Architecture = "x64", ProjectRoot = "/p", Symbols = new Dictionary<string, string?> { ["npm_prefix"] = "/n", ["npm_cache"] = "/c", ["node_prefix"] = "/n" } };
        var tool = new ToolInput("npm") { PackageUrl = "pkg:npm/npm" };
        var first = PolicyCatalogJson.Serialize(catalog.ResolveSandboxPolicyWithDiagnostics(tool, ctx));
        Assert.Equal(first, PolicyCatalogJson.Serialize(catalog.ResolveSandboxPolicyWithDiagnostics(tool, ctx)));
        var policy = catalog.ResolveSandboxPolicy(tool, ctx)!;
        Assert.Equal(new[] { "/n" }, policy.Filesystem!.ReadonlyPaths);
        Assert.Equal(new[] { "/p", "/c" }, policy.Filesystem!.ReadwritePaths);
    }

    [Fact]
    public void DeduplicatesWithPlatformPathRules()
    {
        var windows = TestData.CatalogFor(TestData.Revision(new[]
        {
            TestData.Entry("tool:w", """{ "platformVariants": [{ "when": { "platform": "windows" }, "sandboxPolicy": { "version": "0.9.0-alpha", "filesystem": { "readonlyPaths": ["${git_prefix}", "${node_prefix}\\"] } } }] }"""),
        }));
        var policy = windows.ResolveSandboxPolicy("w", Weak with { Platform = "windows", Architecture = "x64", Symbols = new Dictionary<string, string?> { ["git_prefix"] = "C:\\Tools", ["node_prefix"] = "c:\\tools" } });
        Assert.Equal(new[] { "C:\\Tools" }, policy?.Filesystem?.ReadonlyPaths);
        var linux = TestData.CatalogFor(TestData.Revision(new[]
        {
            TestData.Entry("tool:l", """{ "platformVariants": [{ "when": { "platform": "linux" }, "sandboxPolicy": { "version": "0.9.0-alpha", "filesystem": { "readonlyPaths": ["${git_prefix}", "${node_prefix}/"] } } }] }"""),
        }));
        Assert.Equal(new[] { "/Tools", "/tools" }, linux.ResolveSandboxPolicy("l", Weak with { Symbols = new Dictionary<string, string?> { ["git_prefix"] = "/Tools", ["node_prefix"] = "/tools" } })?.Filesystem?.ReadonlyPaths);
    }

    [Fact]
    public void MacOSFoldsPathCaseForOverlap()
    {
        string Entry(string platform) => $$"""{ "platformVariants": [{ "when": { "platform": "{{platform}}" }, "sandboxPolicy": { "version": "0.9.0-alpha", "filesystem": { "readonlyPaths": ["${git_prefix}"], "readwritePaths": ["${project_root}"] } } }] }""";
        var mac = TestData.CatalogFor(TestData.Revision(new[] { TestData.Entry("tool:m", Entry("macos")) }));
        var ctx = Weak with { Platform = "macos", Architecture = "arm64", ProjectRoot = "/tools/work", Symbols = new Dictionary<string, string?> { ["git_prefix"] = "/Tools" } };
        var error = TestData.Failure(() => mac.ResolveSandboxPolicy("m", ctx));
        Assert.Equal("[policy_validation] resolved paths overlap across access classes: '/Tools' (readonlyPaths) overlaps '/tools/work' (readwritePaths)", error.Message);
        var linux = TestData.CatalogFor(TestData.Revision(new[] { TestData.Entry("tool:m", Entry("linux")) }));
        Assert.NotNull(linux.ResolveSandboxPolicy("m", ctx with { Platform = "linux", Architecture = "x64" }));
    }

    [Fact]
    public void InvocationNameCasingFollowsThePlatform()
    {
        var catalog = TestData.CatalogFor(TestData.Revision(new[]
        {
            TestData.Entry("tool:gh", """
                { "platformVariants": [
                  { "when": { "platform": "linux" }, "sandboxPolicy": { "version": "0.9.0-alpha" } },
                  { "when": { "platform": "macos" }, "sandboxPolicy": { "version": "0.9.0-alpha" } },
                  { "when": { "platform": "windows" }, "sandboxPolicy": { "version": "0.9.0-alpha" } } ] }
                """),
        }));
        IEnumerable<string> On(string platform, string name) =>
            catalog.ResolveSandboxPolicyWithDiagnostics(name, Weak with { Platform = platform, Architecture = "x64" }).Diagnostics.Tools[0].Matches.Select(m => m.EntryId);
        Assert.Equal(new[] { "tool:gh" }, On("linux", "gh"));
        Assert.Empty(On("linux", "GH"));
        Assert.Equal(new[] { "tool:gh" }, On("macos", "GH"));
        Assert.Equal(new[] { "tool:gh" }, On("windows", "Gh"));
    }

    [Fact]
    public void DependencyChainsComposeEachEntryOnceInOrder()
    {
        static string Variant(string[] deps, string path) => $$"""
            { "platformVariants": [{ "when": { "platform": "linux" }
              {{(deps.Length > 0 ? ", \"dependencies\": [" + string.Join(",", deps.Select(d => $"{{\"entryId\":\"{d}\"}}")) + "]" : string.Empty)}},
              "sandboxPolicy": { "version": "0.9.0-alpha", "filesystem": { "readonlyPaths": ["${project_root}/{{path}}"] } } }] }
            """;
        var catalog = TestData.CatalogFor(TestData.Revision(new[]
        {
            TestData.Entry("tool:top", Variant(new[] { "tool:left", "tool:right" }, "top")),
            TestData.Entry("tool:left", Variant(new[] { "tool:leaf" }, "left")),
            TestData.Entry("tool:right", Variant(new[] { "tool:leaf" }, "right")),
            TestData.Entry("tool:leaf", Variant(Array.Empty<string>(), "leaf")),
        }));
        var result = catalog.ResolveSandboxPolicyWithDiagnostics("top", Weak with { ProjectRoot = "/r" });
        Assert.Equal(new[] { "tool:leaf", "tool:left", "tool:right" }, result.Diagnostics.ResolvedDependencies.Select(d => d.EntryId));
        Assert.Equal(new[] { "/r/top", "/r/left", "/r/leaf", "/r/right" }, result.Policy?.Filesystem?.ReadonlyPaths);
    }

    [Fact]
    public void DependencyDiagnosticsKeepDistinctRangesSorted()
    {
        var catalog = TestData.CatalogFor(TestData.Revision(new[]
        {
            TestData.Entry("tool:a", """{ "platformVariants": [{ "when": { "platform": "linux" }, "dependencies": [{ "entryId": "tool:c", "versionRange": ">=2" }], "sandboxPolicy": { "version": "0.9.0-alpha" } }] }"""),
            TestData.Entry("tool:b", """{ "platformVariants": [{ "when": { "platform": "linux" }, "dependencies": [{ "entryId": "tool:c" }], "sandboxPolicy": { "version": "0.9.0-alpha" } }] }"""),
            TestData.Entry("tool:c", """{ "platformVariants": [{ "when": { "platform": "linux" }, "sandboxPolicy": { "version": "0.9.0-alpha" } }] }"""),
        }));
        var result = catalog.ResolveSandboxPolicyWithDiagnostics(new ToolInput[] { "a", "b", "a" }, Weak);
        Assert.Equal(
            new[] { new ResolvedDependency("tool:c", 1, null), new ResolvedDependency("tool:c", 1, ">=2") },
            result.Diagnostics.ResolvedDependencies);
        Assert.Equal("""{"version":"0.9.0-alpha"}""", PolicyCatalogJson.Serialize(result.Policy));
    }

    [Fact]
    public void CatalogFileOrderDoesNotMatter()
    {
        var entries = new[]
        {
            TestData.Entry("tool:b", """{ "identity": [{ "kind": "invocation-name", "names": ["x"] }], "platformVariants": [{ "when": { "platform": "linux" }, "sandboxPolicy": { "version": "0.9.0-alpha", "filesystem": { "readonlyPaths": ["${project_root}/b"] } } }] }"""),
            TestData.Entry("tool:a", """{ "identity": [{ "kind": "invocation-name", "names": ["x"] }], "platformVariants": [{ "when": { "platform": "linux" }, "sandboxPolicy": { "version": "0.9.0-alpha", "filesystem": { "readonlyPaths": ["${project_root}/a"] } } }] }"""),
        };
        var ctx = Weak with { ProjectRoot = "/r" };
        var forward = TestData.CatalogFor(TestData.Revision(entries.Select(e => e.DeepClone()))).ResolveSandboxPolicyWithDiagnostics("x", ctx);
        var backward = TestData.CatalogFor(TestData.Revision(Enumerable.Reverse(entries).Select(e => e.DeepClone()))).ResolveSandboxPolicyWithDiagnostics("x", ctx);
        Assert.Equal(PolicyCatalogJson.Serialize(forward), PolicyCatalogJson.Serialize(backward));
        Assert.Equal(new[] { "/r/a", "/r/b" }, forward.Policy?.Filesystem?.ReadonlyPaths);
        Assert.Contains("input 0 ('x') matched 2 entries (tool:a, tool:b); all contribute", forward.Diagnostics.Warnings);
    }

    [Fact]
    public void MixedVersionsAcrossInputsAreACompositionConflict()
    {
        var catalog = TestData.CatalogFor(TestData.Revision(new[]
        {
            TestData.Entry("tool:a", """{ "platformVariants": [{ "when": { "platform": "linux" }, "sandboxPolicy": { "version": "0.8.0-alpha" } }] }"""),
            TestData.Entry("tool:b", """{ "platformVariants": [{ "when": { "platform": "linux" }, "sandboxPolicy": { "version": "0.9.0-alpha" } }] }"""),
        }));
        var error = TestData.Failure(() => catalog.ResolveSandboxPolicy(new ToolInput[] { "a", "b" }, Weak));
        Assert.Equal("[policy_validation] selected entries cannot be composed: mixed sandboxPolicy.version values (0.8.0-alpha, 0.9.0-alpha)", error.Message);
        Assert.Equal("""{"version":"0.8.0-alpha"}""", PolicyCatalogJson.Serialize(catalog.ResolveSandboxPolicy("a", Weak)));
    }

    [Fact]
    public void VersionRangeWarnings()
    {
        var catalog = TestData.CatalogFor(TestData.Revision(new[]
        {
            TestData.Entry("tool:app", """{ "identity": [{ "kind": "purl", "value": "pkg:npm/app", "versionRange": ">=2 <3" }] }"""),
        }));
        var ctx = new ResolveContext { ProjectRoot = "/p" };
        Assert.DoesNotContain(catalog.ResolveSandboxPolicyWithDiagnostics(new ToolInput("app") { PackageUrl = "pkg:npm/app@2.1.0" }, ctx).Diagnostics.Warnings, w => w.Contains("range", StringComparison.Ordinal));
        Assert.Contains(
            "input 0 ('app'): detected version 'nightly' could not be compared with the reviewed range '>=2 <3' for tool:app",
            catalog.ResolveSandboxPolicyWithDiagnostics(new ToolInput("app") { PackageUrl = "pkg:npm/app@3.0.0", DetectedVersion = "nightly" }, ctx).Diagnostics.Warnings);
    }

    [Fact]
    public void InspectionListsMetadataOnly()
    {
        Assert.Equal(new CatalogInfo("1", "2026-09-29.1"), BundledPolicyCatalog.GetCatalogInfo());
        var entries = BundledPolicyCatalog.ListCatalogEntries();
        Assert.Equal(new[] { "tool:git", "tool:node", "tool:npm" }, entries.Select(e => e.EntryId));
        var npm = entries.Single(e => e.EntryId == "tool:npm");
        Assert.Equal("windows", npm.PlatformVariants[0].Platform);
        Assert.Null(npm.PlatformVariants[0].Architecture);
        Assert.Equal(new[] { "tool:node" }, npm.PlatformVariants[0].DependencyEntryIds);
        var text = PolicyCatalogJson.Serialize(entries);
        Assert.DoesNotContain("Paths", text, StringComparison.Ordinal);
        Assert.DoesNotContain("${", text, StringComparison.Ordinal);
    }

    [Fact]
    public void BundledConvenienceFunctions()
    {
        var ctx = Weak with { Platform = "linux", Architecture = "x64" };
        Assert.Null(BundledPolicyCatalog.ResolveSandboxPolicy("definitely-unknown-tool", ctx));
        var result = BundledPolicyCatalog.ResolveSandboxPolicyWithDiagnostics(new ToolInput[] { "definitely-unknown-tool" }, ctx);
        Assert.Null(result.Policy);
        Assert.Equal("""{"diagnostics":{"catalogRevision":"2026-09-29.1","tools":[{"inputIndex":0,"matches":[]}],"resolvedDependencies":[],"warnings":["input 0 ('definitely-unknown-tool') matched no eligible catalog entry"]}}""", PolicyCatalogJson.Serialize(result));
        Assert.Equal("revision_unavailable", TestData.ErrorReason(() => BundledPolicyCatalog.ResolveSandboxPolicy("git", ctx with { CatalogRevision = "1999-01-01.1" })));
    }
}
