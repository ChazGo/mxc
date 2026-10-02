// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using System.Text.Json;
using System.Text.Json.Serialization;
using Microsoft.Mxc.Sdk;
using Xunit;

namespace Microsoft.Mxc.Sdk.Tests;

public class MxcPolicyStoreTests
{
    private static readonly ResolveContext LinuxContext = new()
    {
        Platform = CatalogPlatforms.Linux,
        Architecture = CatalogArchitectures.X64,
        ProjectRoot = "/work/repo",
        Symbols = new Dictionary<string, string>
        {
            ["git_prefix"] = "/usr",
            ["node_prefix"] = "/opt/node",
            ["npm_prefix"] = "/opt/npm",
            ["npm_cache"] = "/home/u/.npm",
        },
    };

    private static readonly ToolInput Npm = new("npm") { PackageUrl = "pkg:npm/npm" };

    [Fact]
    public void GetCatalogInfo_ReportsTheBundledCatalog()
    {
        var info = MxcPolicyStore.GetCatalogInfo();

        Assert.Equal("1", info.CatalogSchemaVersion);
        Assert.False(string.IsNullOrWhiteSpace(info.CatalogRevision));
    }

    [Fact]
    public void ListCatalogEntries_ReturnsMetadata()
    {
        var entries = MxcPolicyStore.ListCatalogEntries();

        var npm = Assert.Single(entries, entry => entry.EntryId == "tool:npm");
        Assert.Contains(npm.Identity, identity => identity.Kind == "purl" && identity.Value == "pkg:npm/npm");
        Assert.Equal("npm", npm.VersionScheme);
        Assert.Contains("tool:node", npm.Default.DependencyEntryIds);

        var git = Assert.Single(entries, entry => entry.EntryId == "tool:git");
        Assert.Equal("intdot", git.VersionScheme);
        Assert.Equal(new[] { "fetch", "local", "push" }, git.Default.Intents.Select(i => i.Name).Order());
        var windows = Assert.Single(git.PlatformVariants);
        Assert.Equal(CatalogPlatforms.Windows, windows.Platform);
        Assert.Null(windows.Architecture);
        Assert.Collection(
            git.VersionVariants,
            older =>
            {
                Assert.Equal("vers:intdot/>=2.40|<2.50", older.VersionRange);
                var push = Assert.Single(older.IntentAdditions);
                Assert.Equal("push", push.Name);
                Assert.Equal(new[] { "tool:ssh" }, push.DependencyEntryIds);
            },
            newer => Assert.Equal("bundle-fetch", Assert.Single(newer.Intents).Name));
    }

    [Fact]
    public void ResolveSandboxPolicy_ReturnsTheSdkPolicyType()
    {
        SandboxPolicy? policy = MxcPolicyStore.ResolveSandboxPolicy(Npm, LinuxContext);

        Assert.NotNull(policy);
        Assert.Equal("0.9.0-alpha", policy.Version);
        Assert.NotNull(policy.Filesystem);
        Assert.Contains("/opt/npm", policy.Filesystem.ReadonlyPaths);
        Assert.Contains("/opt/node", policy.Filesystem.ReadonlyPaths);
        Assert.Contains("/work/repo", policy.Filesystem.ReadwritePaths);
    }

    [Fact]
    public void ResolveSandboxPolicy_ReturnsNullForAnUnknownTool()
    {
        Assert.Null(MxcPolicyStore.ResolveSandboxPolicy("no-such-tool", LinuxContext));
    }

    [Fact]
    public void ResolveSandboxPolicy_ComposesAListOfTools()
    {
        var policy = MxcPolicyStore.ResolveSandboxPolicy(
            new ToolInput[] { "git", Npm },
            LinuxContext with { AllowWeakIdentityFallback = true });

        Assert.NotNull(policy);
        Assert.Contains("/usr", policy.Filesystem!.ReadonlyPaths);
        Assert.Contains("/opt/npm", policy.Filesystem.ReadonlyPaths);
    }

    [Fact]
    public void ResolveSandboxPolicyWithDiagnostics_ReportsWeakMatchesOnlyWhenAllowed()
    {
        var withoutOptIn = MxcPolicyStore.ResolveSandboxPolicyWithDiagnostics("git", LinuxContext);
        Assert.Null(withoutOptIn.Policy);

        var withOptIn = MxcPolicyStore.ResolveSandboxPolicyWithDiagnostics(
            "git",
            LinuxContext with { AllowWeakIdentityFallback = true });

        Assert.NotNull(withOptIn.Policy);
        var tool = Assert.Single(withOptIn.Diagnostics.Tools);
        Assert.Equal(0, tool.InputIndex);
        var match = Assert.Single(tool.Matches);
        Assert.Equal("tool:git", match.EntryId);
        Assert.Equal("weak", Assert.Single(match.MatchedIdentities).Strength);
    }

    [Fact]
    public void ResolveSandboxPolicyWithDiagnostics_ReportsDependencies()
    {
        var resolution = MxcPolicyStore.ResolveSandboxPolicyWithDiagnostics(Npm, LinuxContext);

        Assert.Contains(resolution.Diagnostics.ResolvedDependencies, dep => dep.EntryId == "tool:node");
    }

    private static readonly ResolveContext GitContext = LinuxContext with
    {
        Symbols = new Dictionary<string, string>
        {
            ["git_prefix"] = "/usr/bin",
            ["ssh_prefix"] = "/usr/lib/ssh",
            ["temp_dir"] = "/tmp",
        },
    };

    private static ToolInput Git(string? version, string? intent) =>
        new("git") { PackageUrl = "pkg:generic/git", DetectedVersion = version, Intent = intent };

    [Fact]
    public void ResolveSandboxPolicyWithDiagnostics_SelectsAVersionRangeAndAnIntent()
    {
        var push = MxcPolicyStore.ResolveSandboxPolicyWithDiagnostics(Git("2.45.1", "push"), GitContext);

        var tool = Assert.Single(push.Diagnostics.Tools);
        Assert.Equal("matched_version", tool.Status);
        var match = Assert.Single(tool.Matches);
        Assert.Equal("vers:intdot/>=2.40|<2.50", match.VersionSelection.SelectedVersionRange);
        Assert.Equal("push", match.IntentSelection!.Requested);
        Assert.Equal("named", match.IntentSelection.Mode);
        Assert.Equal("tool:ssh", Assert.Single(push.Diagnostics.ResolvedDependencies).EntryId);
        Assert.Equal(new[] { "/usr/bin", "/usr/lib/ssh" }, push.Policy!.Filesystem!.ReadonlyPaths);
        Assert.Single(push.Policy.Network!.Egress!.Allow!);
    }

    [Fact]
    public void ResolveSandboxPolicyWithDiagnostics_ReportsStructuredWarnings()
    {
        var outOfRange = MxcPolicyStore.ResolveSandboxPolicyWithDiagnostics(Git("2.30.0", "bundle-fetch"), GitContext);

        Assert.Null(outOfRange.Policy);
        var tool = Assert.Single(outOfRange.Diagnostics.Tools);
        Assert.Equal("intent_unsupported", tool.Status);
        Assert.Equal("version_out_of_range", Assert.Single(tool.Matches).VersionSelection.Status);
        Assert.Collection(
            outOfRange.Diagnostics.Warnings,
            warning =>
            {
                Assert.Equal("version_out_of_range", warning.Code);
                Assert.Equal(0, warning.InputIndex);
                Assert.Equal("tool:git", warning.EntryId);
                Assert.Equal("2.30.0", warning.DetectedVersion);
            },
            warning => Assert.Equal("intent_unsupported", warning.Code));

        var weak = MxcPolicyStore.ResolveSandboxPolicyWithDiagnostics(
            "git",
            GitContext with { AllowWeakIdentityFallback = true });
        var text = Assert.Single(weak.Diagnostics.Warnings);
        Assert.Null(text.Code);
        Assert.Contains("weak identity", text.Message);
    }

    [Fact]
    public void ResolveSandboxPolicy_PairsCompose()
    {
        var policy = MxcPolicyStore.ResolveSandboxPolicy(
            new[] { Git(null, "local"), Git(null, "fetch"), Git(null, "push") },
            GitContext);

        Assert.Equal(2, policy!.Network!.Egress!.Allow!.Count);
    }

    [Fact]
    public void ResolveSandboxPolicy_EmptyIntentIsInvalid()
    {
        var error = Assert.Throws<MxcException>(() =>
            MxcPolicyStore.ResolveSandboxPolicy(Git(null, string.Empty), GitContext));

        Assert.Equal("invalid_context", error.Reason);
    }

    private static readonly JsonSerializerOptions FixtureJsonOptions = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.CamelCase) },
    };

    private static ToolInput FixtureTool(JsonElement tool) => tool.ValueKind == JsonValueKind.String
        ? tool.GetString()!
        : tool.Deserialize<ToolInput>(FixtureJsonOptions)!;

    /// <summary>
    /// Every host-independent bundled conformance case gives the same result
    /// through the C# surface as through the Rust crates.
    /// </summary>
    [Fact]
    public void ResolveSandboxPolicyWithDiagnostics_MatchesTheBundledConformanceFixtures()
    {
        using var stream = typeof(MxcPolicyStoreTests).Assembly
            .GetManifestResourceStream("PolicyStore.bundled-catalog.json")!;
        using var fixture = JsonDocument.Parse(stream);
        var ran = 0;
        foreach (var testCase in fixture.RootElement.GetProperty("cases").EnumerateArray())
        {
            if (testCase.TryGetProperty("host", out _))
            {
                continue;
            }

            var name = testCase.GetProperty("name").GetString();
            var raw = testCase.GetProperty("tools");
            var tools = raw.ValueKind == JsonValueKind.Array
                ? raw.EnumerateArray().Select(FixtureTool).ToArray()
                : new[] { FixtureTool(raw) };
            var context = testCase.TryGetProperty("context", out var c)
                ? c.Deserialize<ResolveContext>(FixtureJsonOptions)
                : null;

            if (testCase.TryGetProperty("expectError", out var expectError))
            {
                var error = Assert.Throws<MxcException>(() =>
                    MxcPolicyStore.ResolveSandboxPolicyWithDiagnostics(tools, context));
                Assert.Equal(expectError.GetProperty("reason").GetString(), error.Reason);
            }
            else
            {
                var expected = MxcPolicyStore.ParseResolution(testCase.GetProperty("expect").GetRawText());
                var actual = MxcPolicyStore.ResolveSandboxPolicyWithDiagnostics(tools, context);
                Assert.True(
                    JsonSerializer.Serialize(expected, FixtureJsonOptions) == JsonSerializer.Serialize(actual, FixtureJsonOptions),
                    name);
            }

            ran++;
        }

        Assert.True(ran >= 15, $"ran {ran} cases");
    }

    [Fact]
    public void ResolveSandboxPolicy_InvalidContextThrowsWithStableReason()
    {
        var error = Assert.Throws<MxcException>(() =>
            MxcPolicyStore.ResolveSandboxPolicy("npm", LinuxContext with { Platform = "plan9" }));

        Assert.Equal(ErrorCode.MalformedRequest, error.Code);
        Assert.Equal("invalid_context", error.Reason);
    }

    [Fact]
    public void MxcException_ReasonIsNullOutsideThePolicyStore()
    {
        Assert.Null(new MxcException(ErrorCode.BackendError, "failed").Reason);
    }

    [Fact]
    public void ParseResolution_MapsNetworkUiAndTimeout()
    {
        var resolution = MxcPolicyStore.ParseResolution("""
            {"policy":{"version":"0.9.0-alpha",
              "network":{"egress":{"default":"deny","allow":[{"to":[{"cidr":"10.0.0.0/8","except":["10.1.0.0/16"]}],"ports":[{"protocol":"tcp","port":443}]}]},
                         "ingress":{"default":"deny","hostLoopback":"allow"}},
              "ui":{"clipboard":"read"},
              "timeoutMs":5000}}
            """);

        var policy = Assert.IsType<SandboxPolicy>(resolution.Policy);
        Assert.Equal(NetworkAction.Deny, policy.Network!.Egress!.Default);
        var rule = Assert.Single(policy.Network.Egress.Allow!);
        Assert.Equal("10.0.0.0/8", Assert.Single(rule.To!).Cidr);
        Assert.Equal(NetworkProtocol.Tcp, Assert.Single(rule.Ports!).Protocol);
        Assert.Equal(NetworkAction.Allow, policy.Network.Ingress!.HostLoopback);
        Assert.Equal(ClipboardPolicy.Read, policy.Ui!.Clipboard);
        Assert.False(policy.Ui.AllowWindows);
        Assert.Equal(5000u, policy.TimeoutMs);
    }

    [Fact]
    public void ParseResolution_RejectsFieldsTheSdkModelCannotHold()
    {
        var error = Assert.Throws<MxcException>(() =>
            MxcPolicyStore.ParseResolution("""{"policy":{"version":"0.9.0-alpha","timeoutMs":4294967296}}"""));
        Assert.Equal(ErrorCode.PolicyValidation, error.Code);
        Assert.Equal("invalid_catalog", error.Reason);

        Assert.Throws<MxcException>(() =>
            MxcPolicyStore.ParseResolution("""{"policy":{"version":"0.9.0-alpha","unknownField":true}}"""));
    }
}
