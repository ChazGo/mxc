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
/// Each catalog entry has one unversioned default plus additive platform,
/// version (purl <c>vers</c> range), and intent overlays. A
/// <see cref="ToolInput"/> may carry a <see cref="ToolInput.DetectedVersion"/>
/// and an <see cref="ToolInput.Intent"/> (for example <c>fetch</c> versus
/// <c>push</c> for git); each tool and intent pair resolves independently, and
/// the pairs compose into one floor.
/// </para>
/// <para>
/// Names and shapes are proposed and may change before sign-off (for example,
/// the names may drop "Sandbox"). This API is not part of MXC 1.0.
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
        new(string.Empty, Array.Empty<ToolDiagnostics>(), Array.Empty<ResolvedDependency>(), Array.Empty<ResolutionWarning>());

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
    /// <summary>
    /// A Package URL without a version, for example <c>pkg:npm/npm</c>. A strong
    /// identity. A version embedded in the purl is ignored with a warning.
    /// </summary>
    [JsonPropertyName("packageUrl")]
    public string? PackageUrl { get; init; }

    /// <summary>
    /// The tool version the caller detected. It selects at most one reviewed
    /// version range; no other version evidence is inferred.
    /// </summary>
    [JsonPropertyName("detectedVersion")]
    public string? DetectedVersion { get; init; }

    /// <summary>
    /// The intended operation, for example <c>fetch</c> or <c>push</c>. When
    /// omitted, the base policy plus every intent of the effective policy
    /// applies. An intent the effective policy does not define contributes
    /// nothing.
    /// </summary>
    [JsonPropertyName("intent")]
    public string? Intent { get; init; }

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

/// <summary>How a detected version selected the effective policy.</summary>
/// <param name="Status">
/// <c>matched_default</c>, <c>matched_version</c>, <c>version_out_of_range</c>,
/// or <c>version_unparseable</c>.
/// </param>
public sealed record VersionSelection(string Status)
{
    /// <summary>The caller's detected version, when supplied.</summary>
    public string? DetectedVersion { get; init; }

    /// <summary>The selected <c>vers</c> range; only for <c>matched_version</c>.</summary>
    public string? SelectedVersionRange { get; init; }
}

/// <summary>
/// Which intents of the effective policy were selected. A resolved dependency
/// reports <c>none</c> (base only) unless its reference names intents, which
/// report <c>named</c>.
/// </summary>
/// <param name="Mode"><c>named</c>, <c>all</c>, <c>none</c>, or <c>unsupported</c>.</param>
/// <param name="Selected">Selected intent names, sorted; empty when unsupported or none.</param>
public sealed record IntentSelection(string Mode, IReadOnlyList<string> Selected)
{
    /// <summary>The requested intent, when one was supplied.</summary>
    public string? Requested { get; init; }
}

/// <summary>One entry matched by one input.</summary>
public sealed record ToolMatch(
    string EntryId,
    int EntryRevision,
    IReadOnlyList<MatchedIdentity> MatchedIdentities,
    VersionSelection VersionSelection,
    IntentSelection? IntentSelection = null);

/// <summary>Per-input attribution, in input order.</summary>
/// <param name="InputIndex">The input's position.</param>
/// <param name="Status">
/// The pair's version status (see <see cref="VersionSelection.Status"/>), or
/// <c>intent_unsupported</c> / <c>tool_unmatched</c> when it contributes nothing.
/// </param>
/// <param name="Matches">At most one entry; empty for <c>tool_unmatched</c>.</param>
public sealed record ToolDiagnostics(int InputIndex, string Status, IReadOnlyList<ToolMatch> Matches);

/// <summary>A dependency pulled in by a match.</summary>
public sealed record ResolvedDependency(
    string EntryId,
    int EntryRevision,
    VersionSelection VersionSelection,
    IntentSelection IntentSelection)
{
    /// <summary>The dependency's declared <c>vers</c> range, when one is declared.</summary>
    public string? RequiredVersionRange { get; init; }
}

/// <summary>
/// A diagnostics warning. Free-text warnings carry only
/// <see cref="Message"/>; structured per-input warnings also carry
/// <see cref="Code"/> and <see cref="InputIndex"/>.
/// </summary>
/// <param name="Message">The human-readable warning.</param>
[JsonConverter(typeof(ResolutionWarningConverter))]
public sealed record ResolutionWarning(string Message)
{
    /// <summary>
    /// <c>version_out_of_range</c>, <c>version_unparseable</c>,
    /// <c>intent_unsupported</c>, or <c>tool_unmatched</c>; <see langword="null"/>
    /// for a free-text warning.
    /// </summary>
    public string? Code { get; init; }

    /// <summary>The input the structured warning is about.</summary>
    public int? InputIndex { get; init; }

    /// <summary>The matched entry, when there is one.</summary>
    public string? EntryId { get; init; }

    /// <summary>The caller's detected version, when supplied.</summary>
    public string? DetectedVersion { get; init; }

    /// <summary>The requested intent, when supplied.</summary>
    public string? Intent { get; init; }
}

/// <summary>Reads a warning that is either a string or a structured object.</summary>
internal sealed class ResolutionWarningConverter : JsonConverter<ResolutionWarning>
{
    public override ResolutionWarning Read(
        ref Utf8JsonReader reader,
        Type typeToConvert,
        JsonSerializerOptions options)
    {
        if (reader.TokenType == JsonTokenType.String)
        {
            return new ResolutionWarning(reader.GetString()!);
        }

        if (reader.TokenType != JsonTokenType.StartObject)
        {
            throw new JsonException("A warning must be a string or an object.");
        }

        using var document = JsonDocument.ParseValue(ref reader);
        string? message = null, code = null, entryId = null, detectedVersion = null, intent = null;
        int? inputIndex = null;
        foreach (var property in document.RootElement.EnumerateObject())
        {
            switch (property.Name)
            {
                case "message": message = property.Value.GetString(); break;
                case "code": code = property.Value.GetString(); break;
                case "inputIndex": inputIndex = property.Value.GetInt32(); break;
                case "entryId": entryId = property.Value.GetString(); break;
                case "detectedVersion": detectedVersion = property.Value.GetString(); break;
                case "intent": intent = property.Value.GetString(); break;
                default: throw new JsonException($"Unexpected warning field '{property.Name}'.");
            }
        }

        if (message is null || code is null || inputIndex is null)
        {
            throw new JsonException("A structured warning needs code, inputIndex, and message.");
        }

        return new ResolutionWarning(message)
        {
            Code = code,
            InputIndex = inputIndex,
            EntryId = entryId,
            DetectedVersion = detectedVersion,
            Intent = intent,
        };
    }

    public override void Write(
        Utf8JsonWriter writer,
        ResolutionWarning value,
        JsonSerializerOptions options)
    {
        if (value.Code is null)
        {
            writer.WriteStringValue(value.Message);
            return;
        }

        writer.WriteStartObject();
        writer.WriteString("code", value.Code);
        writer.WriteNumber("inputIndex", value.InputIndex ?? 0);
        if (value.EntryId is not null)
        {
            writer.WriteString("entryId", value.EntryId);
        }

        if (value.DetectedVersion is not null)
        {
            writer.WriteString("detectedVersion", value.DetectedVersion);
        }

        if (value.Intent is not null)
        {
            writer.WriteString("intent", value.Intent);
        }

        writer.WriteString("message", value.Message);
        writer.WriteEndObject();
    }
}

/// <summary>Match attribution and warnings from one resolution pass.</summary>
public sealed record ResolutionDiagnostics(
    string CatalogRevision,
    IReadOnlyList<ToolDiagnostics> Tools,
    IReadOnlyList<ResolvedDependency> ResolvedDependencies,
    IReadOnlyList<ResolutionWarning> Warnings);

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

    /// <summary>The names, for <c>invocation-name</c>.</summary>
    public IReadOnlyList<string>? Names { get; init; }
}

/// <summary>An intent an entry or overlay defines or extends.</summary>
public sealed record CatalogIntentMetadata(string Name, IReadOnlyList<string> DependencyEntryIds)
{
    /// <summary>Example subcommands, when the catalog lists them.</summary>
    public IReadOnlyList<string>? ExampleSubcommands { get; init; }
}

/// <summary>The entry's unversioned default.</summary>
public sealed record CatalogDefaultMetadata(
    IReadOnlyList<string> DependencyEntryIds,
    string SandboxPolicyVersion,
    IReadOnlyList<CatalogIntentMetadata> Intents);

/// <summary>An additive platform overlay.</summary>
/// <param name="Platform">A <see cref="CatalogPlatforms"/> value.</param>
/// <param name="Architecture">A <see cref="CatalogArchitectures"/> value, or <see langword="null"/> for every architecture.</param>
/// <param name="DependencyEntryIds">Added dependencies.</param>
/// <param name="IntentAdditions">Extensions of intents the default declares.</param>
/// <param name="NewIntents">Intents the overlay introduces.</param>
public sealed record CatalogPlatformVariantMetadata(
    string Platform,
    string? Architecture,
    IReadOnlyList<string> DependencyEntryIds,
    IReadOnlyList<CatalogIntentMetadata> IntentAdditions,
    IReadOnlyList<CatalogIntentMetadata> NewIntents);

/// <summary>An additive version overlay, selected by a purl <c>vers</c> range.</summary>
/// <param name="VersionRange">The <c>vers</c> range.</param>
/// <param name="DependencyEntryIds">Added dependencies.</param>
/// <param name="IntentAdditions">Extensions of intents the default declares.</param>
/// <param name="NewIntents">Intents the overlay introduces.</param>
public sealed record CatalogVersionVariantMetadata(
    string VersionRange,
    IReadOnlyList<string> DependencyEntryIds,
    IReadOnlyList<CatalogIntentMetadata> IntentAdditions,
    IReadOnlyList<CatalogIntentMetadata> NewIntents);

/// <summary>Entry provenance.</summary>
public sealed record CatalogProvenance(string Method, string SourceRevision);

/// <summary>Inspection metadata for one catalog entry. It never exposes a policy body.</summary>
public sealed record CatalogEntryMetadata(
    string CatalogRevision,
    string EntryId,
    int EntryRevision,
    string DisplayName,
    string VersionScheme,
    IReadOnlyList<CatalogIdentityMetadata> Identity,
    CatalogDefaultMetadata Default,
    IReadOnlyList<CatalogPlatformVariantMetadata> PlatformVariants,
    IReadOnlyList<CatalogVersionVariantMetadata> VersionVariants,
    CatalogProvenance Provenance);
