// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using Microsoft.Mxc.Sdk.V1;
using Xunit;

namespace Microsoft.Mxc.Sdk.Tests.V1;

/// <summary>
/// The prototype tool-requirements lookup (pending API review). Lookups target
/// the current host, whose filesystem object identity is examined, so every
/// symbol names an existing temporary directory.
/// </summary>
public sealed class ToolRequirementsTests : IDisposable
{
    private readonly string _root;
    private readonly string _ssh;
    private readonly ResolveContext _context;

    public ToolRequirementsTests()
    {
        _root = Path.Combine(Path.GetTempPath(), "mxc-policy-store-" + Guid.NewGuid().ToString("N"));
        string Dir(params string[] parts)
        {
            var path = Path.Combine(new[] { _root }.Concat(parts).ToArray());
            Directory.CreateDirectory(path);
            return path;
        }

        _ssh = Dir("ssh");
        Dir("pd", "Git");
        _context = new ResolveContext
        {
            ProjectRoot = Dir("repo"),
            Symbols = new Dictionary<string, string>
            {
                ["node_prefix"] = Dir("node"),
                ["npm_prefix"] = Dir("npm"),
                ["npm_cache"] = Dir("npm-cache"),
                ["git_prefix"] = Dir("git"),
                ["ssh_prefix"] = _ssh,
                ["temp_dir"] = Dir("temp"),
                ["programData"] = Dir("pd"),
            },
        };
    }

    public void Dispose()
    {
        try
        {
            Directory.Delete(_root, recursive: true);
        }
        catch (IOException)
        {
        }
    }

    private static ToolCandidate Git(string? version = null, string? intent = null) =>
        new("git") { PackageUrl = "pkg:generic/git", DetectedVersion = version, Intent = intent };

    [Fact]
    public void GetCatalogInfo_ReportsTheBundledCatalogAndContract()
    {
        var info = MxcContainer.GetCatalogInfo();

        Assert.Equal("1", info.CatalogSchemaVersion);
        Assert.Equal("1.0.0", info.SdkContractVersion);
        Assert.False(string.IsNullOrWhiteSpace(info.CatalogRevision));
    }

    [Fact]
    public void ListCatalogEntries_ReturnsMetadata()
    {
        var entries = MxcContainer.ListCatalogEntries();

        Assert.Contains(entries, entry => entry.EntryId == "tool:npm");
        var git = Assert.Single(entries, entry => entry.EntryId == "tool:git");
        Assert.Equal("intdot", git.VersionScheme);
        Assert.Equal(new[] { "fetch", "local", "push" }, git.Default.Intents.Select(i => i.Name).Order());
        Assert.Equal(
            new[] { "vers:intdot/>=2.40|<2.50", "vers:intdot/>=2.50|<3" },
            git.VersionVariants.Select(v => v.VersionRange));
        Assert.Equal("bundle-fetch", Assert.Single(git.VersionVariants[1].NewIntents).Name);
    }

    [Fact]
    public async Task ResolveToolRequirementsAsync_ReturnsCommandFreeRequirements()
    {
        var requirements = await MxcContainer.ResolveToolRequirementsAsync(
            new ToolCandidate("npm") { PackageUrl = "pkg:npm/npm" },
            _context,
            TestContext.Current.CancellationToken);

        Assert.NotNull(requirements);
        Assert.NotEmpty(requirements.Filesystem!.ReadonlyPaths);

        var request = ContainerRequest.FromRequirements(requirements, "npm --version");
        Assert.Equal("npm --version", request.Command);
        Assert.Same(requirements.Filesystem, request.Filesystem);
    }

    [Fact]
    public void ResolveToolRequirements_ReturnsNullForAnUnknownTool()
    {
        Assert.Null(MxcContainer.ResolveToolRequirements("no-such-tool", _context));
    }

    [Fact]
    public void WeakIdentity_RequiresTheOptIn()
    {
        Assert.Null(MxcContainer.ResolveToolRequirementsWithDiagnostics("git", _context).Requirements);

        var resolution = MxcContainer.ResolveToolRequirementsWithDiagnostics(
            new ToolCandidate[] { "git" },
            _context with { AllowWeakIdentityFallback = true });

        Assert.NotNull(resolution.Requirements);
        var tool = Assert.Single(resolution.Diagnostics.Tools);
        Assert.Equal(0, tool.InputIndex);
        Assert.Equal("tool:git", tool.Matches[0].EntryId);
        Assert.Equal("weak", tool.Matches[0].MatchedIdentities[0].Strength);
        var weak = Assert.IsType<ToolResolutionWarning>(
            Assert.Single(resolution.Diagnostics.Warnings, w => w.Code == "weak_identity"));
        Assert.Equal("git", weak.InvocationName);
    }

    [Fact]
    public async Task VersionAndIntent_SelectARangeAndAttributeDependencies()
    {
        var push = await MxcContainer.ResolveToolRequirementsWithDiagnosticsAsync(
            Git("2.45.1", "push"),
            _context,
            TestContext.Current.CancellationToken);

        var tool = Assert.Single(push.Diagnostics.Tools);
        Assert.Equal("matched_version", tool.Status);
        Assert.Equal("vers:intdot/>=2.40|<2.50", tool.Matches[0].VersionSelection.SelectedVersionRange);
        Assert.Equal("named", tool.Matches[0].IntentSelection!.Mode);
        Assert.Equal("push", tool.Matches[0].IntentSelection!.Requested);
        Assert.Equal(new[] { "push" }, tool.Matches[0].IntentSelection!.Selected);
        var ssh = Assert.Single(push.Diagnostics.ResolvedDependencies);
        Assert.Equal("tool:ssh", ssh.EntryId);
        Assert.Equal(new[] { 0 }, ssh.InputIndexes);
        Assert.Equal("none", ssh.IntentSelection.Mode);
        Assert.Contains(_ssh, push.Requirements!.Filesystem!.ReadonlyPaths);
        Assert.Single(push.Requirements.Network!.Egress!.Allow!);
    }

    [Fact]
    public void PerPairOutcomes_CarryStructuredWarnings()
    {
        var outOfRange = MxcContainer.ResolveToolRequirementsWithDiagnostics(Git("2.30.0", "fetch"), _context);
        Assert.Equal("version_out_of_range", outOfRange.Diagnostics.Tools[0].Status);
        Assert.NotNull(outOfRange.Requirements);
        var warning = Assert.IsType<ToolResolutionWarning>(outOfRange.Diagnostics.Warnings[0]);
        Assert.Equal("version_out_of_range", warning.Code);
        Assert.Equal("tool:git", warning.EntryId);
        Assert.Equal("2.30.0", warning.DetectedVersion);

        var unparseable = MxcContainer.ResolveToolRequirementsWithDiagnostics(Git("banana", "fetch"), _context);
        Assert.Equal("version_unparseable", unparseable.Diagnostics.Tools[0].Status);
        Assert.Null(unparseable.Requirements);

        var unsupported = MxcContainer.ResolveToolRequirementsWithDiagnostics(Git(intent: "bundle-fetch"), _context);
        Assert.Equal("intent_unsupported", unsupported.Diagnostics.Tools[0].Status);
        Assert.Null(unsupported.Requirements);

        var bundle = MxcContainer.ResolveToolRequirementsWithDiagnostics(Git("2.55", "bundle-fetch"), _context);
        Assert.Equal("matched_version", bundle.Diagnostics.Tools[0].Status);
        Assert.Empty(bundle.Diagnostics.ResolvedDependencies);

        var invalidPurl = MxcContainer.ResolveToolRequirementsWithDiagnostics(
            new ToolCandidate("npm") { PackageUrl = "not a purl" },
            _context);
        Assert.Equal("tool_unmatched", invalidPurl.Diagnostics.Tools[0].Status);
        Assert.Equal("purl_invalid", invalidPurl.Diagnostics.Warnings[0].Code);

        var ignored = MxcContainer.ResolveToolRequirementsWithDiagnostics(
            new ToolCandidate("npm") { PackageUrl = "pkg:npm/npm@10.0.0" },
            _context);
        var components = Assert.IsType<ToolResolutionWarning>(ignored.Diagnostics.Warnings[0]);
        Assert.Equal("purl_components_ignored", components.Code);
        Assert.Equal(new[] { "version" }, components.IgnoredComponents);
    }

    [Fact]
    public void Composition_AToolWithoutNetworkDoesNotVetoAnother()
    {
        var resolution = MxcContainer.ResolveToolRequirementsWithDiagnostics(
            new ToolCandidate[] { Git(intent: "local"), Git(intent: "fetch"), "no-such-tool" },
            _context);

        Assert.Single(resolution.Requirements!.Network!.Egress!.Allow!);
        Assert.Equal("tool_unmatched", resolution.Diagnostics.Tools[2].Status);
    }

    [Fact]
    public async Task InvalidContext_ThrowsMxcExceptionWithAReason()
    {
        var platform = Assert.Throws<MxcException>(
            () => MxcContainer.ResolveToolRequirements("npm", _context with { Platform = "plan9" }));
        Assert.Equal(ErrorCode.MalformedRequest, platform.Code);
        Assert.Equal("invalid_context", platform.Reason);

        var intent = await Assert.ThrowsAsync<MxcException>(
            () => MxcContainer.ResolveToolRequirementsAsync(
                Git(intent: string.Empty),
                _context,
                TestContext.Current.CancellationToken));
        Assert.Equal("invalid_context", intent.Reason);
    }

    [Fact]
    public void ToolRequirementsRequest_OmitsUnsetFields()
    {
        var bytes = MxcContainer.ToolRequirementsRequest(new ToolCandidate[] { "git" }, null);

        Assert.Equal(0, bytes[^1]);
        Assert.Equal(
            """{"tools":[{"invocationName":"git"}]}""",
            System.Text.Encoding.UTF8.GetString(bytes, 0, bytes.Length - 1));
    }
}
