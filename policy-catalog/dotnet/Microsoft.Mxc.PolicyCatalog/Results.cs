// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

namespace Microsoft.Mxc.PolicyCatalog;

// Numbers are JSON numbers with JavaScript semantics (IEEE doubles) so every
// value round-trips exactly as the TypeScript reference emits it. The catalog
// contract only admits integers where these appear.

/// <summary>
/// The catalog-supported subset of MXC's <c>SandboxPolicy</c> authoring contract. Structurally compatible
/// with the MXC SDK policy; backend-specific keys are never part of it. A candidate lower bound, not authorization.
/// </summary>
/// <param name="Version">The <c>SandboxPolicy</c> contract version.</param>
public sealed record CatalogSandboxPolicy(string Version)
{
    /// <summary>Filesystem requirements, or <c>null</c> when absent.</summary>
    public CatalogFilesystemPolicy? Filesystem { get; init; }

    /// <summary>Network requirements, or <c>null</c> when absent.</summary>
    public CatalogNetworkPolicy? Network { get; init; }

    /// <summary>UI requirements, or <c>null</c> when absent.</summary>
    public CatalogUiPolicy? Ui { get; init; }

    /// <summary>Timeout in milliseconds, or <c>null</c> when absent.</summary>
    public double? TimeoutMs { get; init; }
}

/// <summary>Filesystem access classes. A <c>null</c> list is absent from the JSON form.</summary>
public sealed record CatalogFilesystemPolicy
{
    /// <summary>Paths the tool must not access.</summary>
    public IReadOnlyList<string>? DeniedPaths { get; init; }

    /// <summary>Paths the tool reads.</summary>
    public IReadOnlyList<string>? ReadonlyPaths { get; init; }

    /// <summary>Paths the tool reads and writes.</summary>
    public IReadOnlyList<string>? ReadwritePaths { get; init; }
}

/// <summary>Network requirements.</summary>
public sealed record CatalogNetworkPolicy
{
    /// <summary>Egress rules.</summary>
    public CatalogEgressPolicy? Egress { get; init; }

    /// <summary>Ingress rules.</summary>
    public CatalogIngressPolicy? Ingress { get; init; }
}

/// <summary>Egress rules.</summary>
public sealed record CatalogEgressPolicy
{
    /// <summary><c>deny</c> (the only default the catalog admits) or <c>null</c>.</summary>
    public string? Default { get; init; }

    /// <summary>Allowed peers.</summary>
    public IReadOnlyList<CatalogNetworkRule>? Allow { get; init; }

    /// <summary>Denied peers.</summary>
    public IReadOnlyList<CatalogNetworkRule>? Deny { get; init; }
}

/// <summary>Ingress rules.</summary>
public sealed record CatalogIngressPolicy
{
    /// <summary><c>deny</c> or <c>null</c>.</summary>
    public string? Default { get; init; }

    /// <summary><c>allow</c>, <c>deny</c>, or <c>null</c>.</summary>
    public string? HostLoopback { get; init; }
}

/// <summary>One network rule.</summary>
public sealed record CatalogNetworkRule
{
    /// <summary>Peers.</summary>
    public IReadOnlyList<CatalogNetworkPeer>? To { get; init; }

    /// <summary>Ports.</summary>
    public IReadOnlyList<CatalogNetworkPort>? Ports { get; init; }
}

/// <summary>A network peer.</summary>
/// <param name="Cidr">The peer CIDR.</param>
public sealed record CatalogNetworkPeer(string Cidr)
{
    /// <summary>Excluded CIDRs.</summary>
    public IReadOnlyList<string>? Except { get; init; }
}

/// <summary>A port or port range.</summary>
public sealed record CatalogNetworkPort
{
    /// <summary><c>tcp</c>, <c>udp</c>, <c>icmp</c>, <c>any</c>, or <c>null</c>.</summary>
    public string? Protocol { get; init; }

    /// <summary>First port.</summary>
    public double? Port { get; init; }

    /// <summary>Last port of a range.</summary>
    public double? EndPort { get; init; }
}

/// <summary>UI requirements.</summary>
public sealed record CatalogUiPolicy
{
    /// <summary>Whether windows are allowed.</summary>
    public bool? AllowWindows { get; init; }

    /// <summary><c>none</c>, <c>read</c>, <c>write</c>, <c>all</c>, or <c>null</c>.</summary>
    public string? Clipboard { get; init; }

    /// <summary>Whether input injection is allowed.</summary>
    public bool? AllowInputInjection { get; init; }
}

/// <summary>Strength of a matched identity predicate kind.</summary>
/// <param name="Kind"><c>purl</c> or <c>invocation-name</c>.</param>
/// <param name="Strength"><c>strong</c> or <c>weak</c>.</param>
public sealed record MatchedIdentity(string Kind, string Strength);

/// <summary>One entry matched by one input.</summary>
/// <param name="EntryId">The entry ID.</param>
/// <param name="EntryRevision">The entry revision.</param>
/// <param name="MatchedIdentities">Satisfied predicates, in declaration order.</param>
public sealed record ToolMatch(string EntryId, double EntryRevision, IReadOnlyList<MatchedIdentity> MatchedIdentities);

/// <summary>Per-input attribution, in input order.</summary>
/// <param name="InputIndex">Zero-based input index.</param>
/// <param name="Matches">Matches ordered by entry ID.</param>
public sealed record ToolDiagnostics(int InputIndex, IReadOnlyList<ToolMatch> Matches);

/// <summary>A dependency edge among the selected entries.</summary>
/// <param name="EntryId">The dependency's entry ID.</param>
/// <param name="EntryRevision">The dependency's entry revision.</param>
/// <param name="RequiredVersionRange">The declared (unevaluated) version range, or <c>null</c>.</param>
public sealed record ResolvedDependency(string EntryId, double EntryRevision, string? RequiredVersionRange);

/// <summary>Attribution and warnings for one lookup (design §5.1).</summary>
/// <param name="CatalogRevision">The revision used.</param>
/// <param name="Tools">Per-input records, in input order.</param>
/// <param name="ResolvedDependencies">Distinct dependency edges, sorted.</param>
/// <param name="Warnings">Plain-text warnings.</param>
public sealed record ResolutionDiagnostics(
    string CatalogRevision,
    IReadOnlyList<ToolDiagnostics> Tools,
    IReadOnlyList<ResolvedDependency> ResolvedDependencies,
    IReadOnlyList<string> Warnings);

/// <summary>Result of <see cref="PolicyCatalog.ResolveSandboxPolicyWithDiagnostics(IReadOnlyList{ToolInput}, ResolveContext?)"/>.</summary>
/// <param name="Policy">The composed candidate policy, or <c>null</c> when none can be resolved.</param>
/// <param name="Diagnostics">Attribution and warnings.</param>
public sealed record SandboxConfigResolution(CatalogSandboxPolicy? Policy, ResolutionDiagnostics Diagnostics);

/// <summary>Result of <see cref="PolicyCatalog.GetCatalogInfo"/>.</summary>
/// <param name="CatalogSchemaVersion">The catalog schema version.</param>
/// <param name="CatalogRevision">The installed default revision.</param>
public sealed record CatalogInfo(string CatalogSchemaVersion, string CatalogRevision);

/// <summary>Identity metadata: either a package URL (<see cref="Value"/>) or invocation names (<see cref="Names"/>).</summary>
/// <param name="Kind"><c>purl</c> or <c>invocation-name</c>.</param>
public sealed record CatalogIdentityMetadata(string Kind)
{
    /// <summary>The package URL for <c>purl</c>.</summary>
    public string? Value { get; init; }

    /// <summary>The reviewed version range for <c>purl</c>, when declared.</summary>
    public string? VersionRange { get; init; }

    /// <summary>The names for <c>invocation-name</c>.</summary>
    public IReadOnlyList<string>? Names { get; init; }
}

/// <summary>Platform variant metadata.</summary>
/// <param name="Platform">The platform selector.</param>
/// <param name="Architecture">The architecture selector, or <c>null</c> for the architecture-neutral variant.</param>
/// <param name="DependencyEntryIds">Declared dependencies.</param>
/// <param name="SandboxPolicyVersion">The embedded policy's contract version.</param>
public sealed record CatalogVariantMetadata(string Platform, string? Architecture, IReadOnlyList<string> DependencyEntryIds, string SandboxPolicyVersion);

/// <summary>Entry provenance.</summary>
/// <param name="Method">How the entry was produced.</param>
/// <param name="SourceRevision">Where it came from.</param>
public sealed record CatalogProvenance(string Method, string SourceRevision);

/// <summary>Inspection metadata (design §5.2). It never exposes a policy body.</summary>
/// <param name="CatalogRevision">The revision.</param>
/// <param name="EntryId">The entry ID.</param>
/// <param name="EntryRevision">The entry revision.</param>
/// <param name="DisplayName">The display name.</param>
/// <param name="Identity">Identity predicates, in declaration order.</param>
/// <param name="PlatformVariants">Variants, in declaration order.</param>
/// <param name="Provenance">Provenance.</param>
public sealed record CatalogEntryMetadata(
    string CatalogRevision,
    string EntryId,
    double EntryRevision,
    string DisplayName,
    IReadOnlyList<CatalogIdentityMetadata> Identity,
    IReadOnlyList<CatalogVariantMetadata> PlatformVariants,
    CatalogProvenance Provenance);

/// <summary>Result of catalog-directory validation (<c>policy-catalog validate</c>).</summary>
/// <param name="Ok">True only when every requested check ran and passed.</param>
/// <param name="CatalogDir">The absolute catalog directory.</param>
/// <param name="DefaultRevision">The default revision, when the manifest loaded.</param>
/// <param name="Revisions">Listed revisions, when the manifest loaded.</param>
/// <param name="BaseRef">The base-ref comparison, when requested.</param>
/// <param name="Errors">Error strings.</param>
public sealed record CatalogValidationReport(
    bool Ok,
    string CatalogDir,
    string? DefaultRevision,
    IReadOnlyList<string>? Revisions,
    BaseRefReport? BaseRef,
    IReadOnlyList<string> Errors);

/// <summary>The base-ref part of a validation report.</summary>
/// <param name="Ref">The requested ref.</param>
/// <param name="ComparedRevisions">Published revisions compared; 0 when the base has no catalog.</param>
public sealed record BaseRefReport(string Ref, int ComparedRevisions);
