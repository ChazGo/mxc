// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using Microsoft.Mxc.Sdk.Native;

namespace Microsoft.Mxc.Sdk;

/// <summary>
/// <b>PROTOTYPE, pending API review.</b> The MXC policy store: resolve known
/// tools to a candidate floor <see cref="SandboxPolicy"/> from the policy
/// catalog bundled statically in the native <c>mxc_ffi</c> library.
/// </summary>
/// <remarks>
/// <para>
/// The result is a best-effort floor, not a guarantee: the access a known tool
/// typically needs, which a caller composes with its own policy. It is
/// complementary to Learning Mode, not a replacement. Resolution never grants
/// access, launches a sandbox, contacts a network service, or writes state; the
/// V1 catalog is compiled in and nothing is downloaded.
/// </para>
/// <para>
/// Names and shapes are proposed and may change before sign-off (for example,
/// the names may drop "Sandbox", and lookup may gain an intent such as
/// <c>git pull</c> versus <c>git push</c>). This API is not part of MXC 1.0.
/// </para>
/// <para>
/// Failures throw <see cref="MxcException"/>: <see cref="MxcException.Code"/>
/// carries the typed <see cref="ErrorCode"/> and
/// <see cref="MxcException.Reason"/> carries the store's stable failure reason
/// (for example <c>invalid_context</c>).
/// </para>
/// </remarks>
public static class MxcPolicyStore
{
    static MxcPolicyStore()
    {
        NativeLibraryResolver.Initialize();
    }

    private static readonly JsonSerializerOptions RequestJsonOptions = new()
    {
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
    };

    // Strict on the way back in: a policy field the SDK model cannot hold must
    // fail loudly rather than be silently dropped from the floor.
    private static readonly JsonSerializerOptions ResultJsonOptions = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        UnmappedMemberHandling = JsonUnmappedMemberHandling.Disallow,
        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.CamelCase) },
    };

    /// <summary>
    /// <b>PROTOTYPE, pending API review.</b> Resolve one tool to a floor policy.
    /// </summary>
    /// <returns>The policy, or <see langword="null"/> when none can be resolved.</returns>
    /// <exception cref="MxcException">The input, context, or catalog is invalid.</exception>
    public static SandboxPolicy? ResolveSandboxPolicy(ToolInput tool, ResolveContext? context = null)
    {
        ArgumentNullException.ThrowIfNull(tool);
        return ResolveSandboxPolicy(new[] { tool }, context);
    }

    /// <summary>
    /// <b>PROTOTYPE, pending API review.</b> Resolve a list of tools to a single
    /// composed floor policy.
    /// </summary>
    /// <returns>The policy, or <see langword="null"/> when none can be resolved.</returns>
    /// <exception cref="MxcException">The input, context, or catalog is invalid.</exception>
    public static SandboxPolicy? ResolveSandboxPolicy(
        IReadOnlyList<ToolInput> tools,
        ResolveContext? context = null)
    {
        var json = CallResolve(Request(tools, context), withDiagnostics: false);
        return ParseResolution(json).Policy;
    }

    /// <summary>
    /// <b>PROTOTYPE, pending API review.</b> Like
    /// <see cref="ResolveSandboxPolicy(ToolInput, ResolveContext?)"/>, and also
    /// reports which catalog entries matched, the dependencies they pulled in,
    /// and warnings.
    /// </summary>
    /// <exception cref="MxcException">The input, context, or catalog is invalid.</exception>
    public static SandboxConfigResolution ResolveSandboxPolicyWithDiagnostics(
        ToolInput tool,
        ResolveContext? context = null)
    {
        ArgumentNullException.ThrowIfNull(tool);
        return ResolveSandboxPolicyWithDiagnostics(new[] { tool }, context);
    }

    /// <summary>
    /// <b>PROTOTYPE, pending API review.</b> Like
    /// <see cref="ResolveSandboxPolicy(IReadOnlyList{ToolInput}, ResolveContext?)"/>,
    /// and also reports which catalog entries matched each input, the
    /// dependencies they pulled in, and warnings.
    /// </summary>
    /// <exception cref="MxcException">The input, context, or catalog is invalid.</exception>
    public static SandboxConfigResolution ResolveSandboxPolicyWithDiagnostics(
        IReadOnlyList<ToolInput> tools,
        ResolveContext? context = null)
    {
        var json = CallResolve(Request(tools, context), withDiagnostics: true);
        var resolution = ParseResolution(json);
        if (ReferenceEquals(resolution.Diagnostics, EmptyDiagnostics))
        {
            throw new MxcException(
                ErrorCode.BackendError,
                "The policy store returned no diagnostics.");
        }

        return resolution;
    }

    /// <summary>
    /// <b>PROTOTYPE, pending API review.</b> The bundled catalog's schema
    /// version and default revision.
    /// </summary>
    public static CatalogInfo GetCatalogInfo()
    {
        var json = CallInspect(listEntries: false);
        return Deserialize<CatalogInfo>(json);
    }

    /// <summary>
    /// <b>PROTOTYPE, pending API review.</b> Metadata for every entry in the
    /// bundled catalog's default revision. It never exposes a policy body.
    /// </summary>
    public static IReadOnlyList<CatalogEntryMetadata> ListCatalogEntries()
    {
        var json = CallInspect(listEntries: true);
        return Deserialize<CatalogEntryMetadata[]>(json);
    }

    private static byte[] Request(IReadOnlyList<ToolInput> tools, ResolveContext? context)
    {
        ArgumentNullException.ThrowIfNull(tools);
        var request = new NativeRequest(
            tools.Select(tool => tool ?? throw new ArgumentException(
                "A tool input is null.", nameof(tools))).ToArray(),
            context);
        var json = JsonSerializer.Serialize(request, RequestJsonOptions);
        var bytes = new byte[Encoding.UTF8.GetByteCount(json) + 1];
        Encoding.UTF8.GetBytes(json, 0, json.Length, bytes, 0);
        return bytes;
    }

    private static unsafe string CallResolve(byte[] request, bool withDiagnostics)
    {
        fixed (byte* requestPtr = request)
        {
            MxcPolicyStoreResult result = default;
            var status = withDiagnostics
                ? NativeMethods.mxc_resolve_sandbox_policy_with_diagnostics_json(requestPtr, &result)
                : NativeMethods.mxc_resolve_sandbox_policy_json(requestPtr, &result);
            return TakeResult(status, &result);
        }
    }

    private static unsafe string CallInspect(bool listEntries)
    {
        MxcPolicyStoreResult result = default;
        var status = listEntries
            ? NativeMethods.mxc_list_policy_catalog_entries_json(&result)
            : NativeMethods.mxc_policy_catalog_info_json(&result);
        return TakeResult(status, &result);
    }

    private static unsafe string TakeResult(int status, MxcPolicyStoreResult* result)
    {
        try
        {
            if (status != (int)ErrorCode.Success)
            {
                var error = NativeError.ToException(status, result->error, "unknown error");
                error.Reason = NativeError.ToStringOrNull(result->reason_utf8);
                throw error;
            }

            return NativeError.ToStringOrNull(result->json_utf8)
                ?? throw new MxcException(
                    ErrorCode.BackendError,
                    "The policy store returned no result.");
        }
        finally
        {
            NativeMethods.mxc_policy_store_result_free(result);
        }
    }

    private static T Deserialize<T>(string json) =>
        JsonSerializer.Deserialize<T>(json, ResultJsonOptions)
            ?? throw new JsonException("The policy store returned null JSON.");

    /// <summary>
    /// Maps the store's policy JSON onto the SDK <see cref="SandboxPolicy"/>.
    /// A field the SDK model cannot hold is a catalog-contract violation, as in
    /// the Rust SDK, not a silently narrowed floor.
    /// </summary>
    internal static SandboxConfigResolution ParseResolution(string json)
    {
        try
        {
            var resolution = Deserialize<NativeResolution>(json);
            return new SandboxConfigResolution(
                resolution.Policy,
                resolution.Diagnostics ?? EmptyDiagnostics);
        }
        catch (JsonException error)
        {
            throw new MxcException(
                ErrorCode.PolicyValidation,
                $"The resolved policy is not a valid SDK SandboxPolicy: {error.Message}",
                error)
            {
                Reason = "invalid_catalog",
            };
        }
    }

    private static readonly ResolutionDiagnostics EmptyDiagnostics =
        new(string.Empty, Array.Empty<ToolDiagnostics>(), Array.Empty<ResolvedDependency>(), Array.Empty<string>());

    private sealed record NativeRequest(
        [property: JsonPropertyName("tools")] IReadOnlyList<ToolInput> Tools,
        [property: JsonPropertyName("context")] ResolveContext? Context);

    private sealed record NativeResolution(
        [property: JsonPropertyName("policy")] SandboxPolicy? Policy,
        [property: JsonPropertyName("diagnostics")] ResolutionDiagnostics? Diagnostics);
}

/// <summary>Catalog platform selector values.</summary>
public static class CatalogPlatforms
{
    /// <summary>Windows.</summary>
    public const string Windows = "windows";

    /// <summary>Linux.</summary>
    public const string Linux = "linux";

    /// <summary>macOS.</summary>
    public const string MacOS = "macos";
}

/// <summary>Catalog architecture selector values.</summary>
public static class CatalogArchitectures
{
    /// <summary>x64 (AMD64 / x86_64).</summary>
    public const string X64 = "x64";

    /// <summary>ARM64 (aarch64).</summary>
    public const string Arm64 = "arm64";
}

/// <summary>
/// <b>PROTOTYPE, pending API review.</b> A tool to look up. A plain string
/// converts implicitly to an input with only <see cref="InvocationName"/>.
/// </summary>
/// <param name="InvocationName">The bare invocation name, for example <c>git</c>.</param>
public sealed record ToolInput(
    [property: JsonPropertyName("invocationName")] string InvocationName)
{
    /// <summary>A verified Package URL, for example <c>pkg:npm/npm</c>. A strong identity.</summary>
    [JsonPropertyName("packageUrl")]
    public string? PackageUrl { get; init; }

    /// <summary>The detected tool version, compared with reviewed version ranges.</summary>
    [JsonPropertyName("detectedVersion")]
    public string? DetectedVersion { get; init; }

    /// <summary>String shorthand for <c>new ToolInput(name)</c>.</summary>
    public static implicit operator ToolInput(string invocationName) => new(invocationName);
}

/// <summary><b>PROTOTYPE, pending API review.</b> Lookup context shared by every input in one lookup.</summary>
public sealed record ResolveContext
{
    /// <summary>Substituted for the <c>project_root</c> catalog symbol.</summary>
    [JsonPropertyName("projectRoot")]
    public string? ProjectRoot { get; init; }

    /// <summary>Caller-supplied catalog symbol values, for example <c>git_prefix</c>.</summary>
    [JsonPropertyName("symbols")]
    public IReadOnlyDictionary<string, string>? Symbols { get; init; }

    /// <summary>A <see cref="CatalogPlatforms"/> value; defaults to the host platform.</summary>
    [JsonPropertyName("platform")]
    public string? Platform { get; init; }

    /// <summary>A <see cref="CatalogArchitectures"/> value; defaults to the host architecture.</summary>
    [JsonPropertyName("architecture")]
    public string? Architecture { get; init; }

    /// <summary>Defaults to the bundled catalog's default revision.</summary>
    [JsonPropertyName("catalogRevision")]
    public string? CatalogRevision { get; init; }

    /// <summary>Allow entries matched only by invocation name. Off by default.</summary>
    [JsonPropertyName("allowWeakIdentityFallback")]
    public bool AllowWeakIdentityFallback { get; init; }
}

/// <summary>A satisfied identity predicate.</summary>
/// <param name="Kind"><c>purl</c> or <c>invocation-name</c>.</param>
/// <param name="Strength"><c>strong</c> or <c>weak</c>.</param>
public sealed record MatchedIdentity(string Kind, string Strength);

/// <summary>One entry matched by one input.</summary>
public sealed record ToolMatch(
    string EntryId,
    int EntryRevision,
    IReadOnlyList<MatchedIdentity> MatchedIdentities);

/// <summary>Per-input attribution, in input order.</summary>
public sealed record ToolDiagnostics(int InputIndex, IReadOnlyList<ToolMatch> Matches);

/// <summary>A dependency pulled in by a match.</summary>
public sealed record ResolvedDependency(
    string EntryId,
    int EntryRevision,
    string? RequiredVersionRange = null);

/// <summary>Match attribution and warnings from one resolution pass.</summary>
public sealed record ResolutionDiagnostics(
    string CatalogRevision,
    IReadOnlyList<ToolDiagnostics> Tools,
    IReadOnlyList<ResolvedDependency> ResolvedDependencies,
    IReadOnlyList<string> Warnings);

/// <summary>
/// <b>PROTOTYPE, pending API review.</b> Result of
/// <see cref="MxcPolicyStore.ResolveSandboxPolicyWithDiagnostics(IReadOnlyList{ToolInput}, ResolveContext?)"/>.
/// </summary>
/// <param name="Policy">The composed policy, or <see langword="null"/> when none can be resolved.</param>
/// <param name="Diagnostics">Attribution and warnings.</param>
public sealed record SandboxConfigResolution(SandboxPolicy? Policy, ResolutionDiagnostics Diagnostics);

/// <summary>The bundled catalog's schema version and default revision.</summary>
public sealed record CatalogInfo(string CatalogSchemaVersion, string CatalogRevision);

/// <summary>One identity predicate of a catalog entry.</summary>
/// <param name="Kind"><c>purl</c> or <c>invocation-name</c>.</param>
public sealed record CatalogIdentityMetadata(string Kind)
{
    /// <summary>The package URL, for <c>purl</c>.</summary>
    public string? Value { get; init; }

    /// <summary>The reviewed version range, for <c>purl</c>, when declared.</summary>
    public string? VersionRange { get; init; }

    /// <summary>The names, for <c>invocation-name</c>.</summary>
    public IReadOnlyList<string>? Names { get; init; }
}

/// <summary>Platform variant metadata.</summary>
public sealed record CatalogVariantMetadata(
    string Platform,
    string? Architecture,
    IReadOnlyList<string> DependencyEntryIds,
    string SandboxPolicyVersion);

/// <summary>Entry provenance.</summary>
public sealed record CatalogProvenance(string Method, string SourceRevision);

/// <summary>Inspection metadata for one catalog entry. It never exposes a policy body.</summary>
public sealed record CatalogEntryMetadata(
    string CatalogRevision,
    string EntryId,
    int EntryRevision,
    string DisplayName,
    IReadOnlyList<CatalogIdentityMetadata> Identity,
    IReadOnlyList<CatalogVariantMetadata> PlatformVariants,
    CatalogProvenance Provenance);
