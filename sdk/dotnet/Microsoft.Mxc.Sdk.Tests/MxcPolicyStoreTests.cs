// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

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
        Assert.Contains(npm.PlatformVariants, variant =>
            variant.Platform == CatalogPlatforms.Linux
            && variant.DependencyEntryIds.Contains("tool:node"));
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
