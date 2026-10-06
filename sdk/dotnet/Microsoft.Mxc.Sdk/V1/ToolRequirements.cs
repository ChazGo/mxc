// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using System.Text.Json;
using System.Text.Json.Serialization;

namespace Microsoft.Mxc.Sdk.V1;

/// <summary>
/// <b>PROTOTYPE, pending API review.</b> Command-free container requirements:
/// the filesystem, network, UI, and timeout fields of a
/// <see cref="ContainerRequest"/>, using the same v1 section types.
/// </summary>
/// <remarks>
/// A caller reviews and constrains the requirements, then adds its own command
/// with <see cref="ContainerRequest.FromRequirements"/>. The requirements are a
/// best-effort floor, not a guarantee.
/// </remarks>
[JsonUnmappedMemberHandling(JsonUnmappedMemberHandling.Disallow)]
public sealed class ContainerRequirements
{
    /// <summary>Cross-backend filesystem access requirements.</summary>
    [JsonPropertyName("filesystem")]
    public FilesystemPolicy? Filesystem { get; set; }

    /// <summary>Cross-backend directional network requirements.</summary>
    [JsonPropertyName("network")]
    public NetworkPolicy? Network { get; set; }

    /// <summary>Cross-backend UI requirements.</summary>
    [JsonPropertyName("ui")]
    public UiPolicy? Ui { get; set; }

    /// <summary>Execution timeout in milliseconds; null means none required.</summary>
    [JsonPropertyName("timeoutMs")]
    public uint? TimeoutMs { get; set; }
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
/// converts implicitly to a candidate with only <see cref="InvocationName"/>.
/// </summary>
/// <param name="InvocationName">
/// The bare invocation name, for example <c>git</c>. A weak identity, used only
/// when <see cref="ResolveContext.AllowWeakIdentityFallback"/> is set.
/// </param>
public sealed record ToolCandidate(string InvocationName)
{
    /// <summary>
    /// A Package URL, for example <c>pkg:npm/npm</c>. A strong identity matched
    /// on type, namespace, and name; a version, qualifiers, or subpath are
    /// ignored with a <c>purl_components_ignored</c> warning, and an invalid
    /// PURL makes the pair <c>tool_unmatched</c>.
    /// </summary>
    public string? PackageUrl { get; init; }

    /// <summary>
    /// The tool version the caller detected. It selects at most one reviewed
    /// version range; no other version evidence is inferred.
    /// </summary>
    public string? DetectedVersion { get; init; }

    /// <summary>
    /// The intended operation, for example <c>fetch</c> or <c>push</c>. When
    /// omitted, the base plus every intent of the effective entry applies.
    /// </summary>
    public string? Intent { get; init; }

    /// <summary>String shorthand for <c>new ToolCandidate(name)</c>.</summary>
    public static implicit operator ToolCandidate(string invocationName) => new(invocationName);
}

/// <summary><b>PROTOTYPE, pending API review.</b> Lookup context shared by every input.</summary>
public sealed record ResolveContext
{
    /// <summary>Substituted for the <c>project_root</c> catalog symbol.</summary>
    public string? ProjectRoot { get; init; }

    /// <summary>
    /// Catalog symbol values, for example <c>git_prefix</c>. They take
    /// precedence over host values, discovery, and contract defaults.
    /// </summary>
    public IReadOnlyDictionary<string, string>? Symbols { get; init; }

    /// <summary>A <see cref="CatalogPlatforms"/> value; defaults to the host platform.</summary>
    public string? Platform { get; init; }

    /// <summary>A <see cref="CatalogArchitectures"/> value; defaults to the host architecture.</summary>
    public string? Architecture { get; init; }

    /// <summary>Defaults to the bundled catalog's default revision.</summary>
    public string? CatalogRevision { get; init; }

    /// <summary>Allow entries matched only by invocation name. Off by default.</summary>
    public bool AllowWeakIdentityFallback { get; init; }
}

/// <summary>A satisfied identity predicate.</summary>
/// <param name="Kind"><c>purl</c> or <c>invocation-name</c>.</param>
/// <param name="Strength"><c>strong</c> or <c>weak</c>.</param>
public sealed record MatchedIdentity(string Kind, string Strength);

/// <summary>How a detected version selected the entry's version data.</summary>
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
/// Which intents were selected. A dependency reports <c>none</c> (base only)
/// unless its reference names intents, which report <c>named</c>.
/// </summary>
/// <param name="Mode"><c>named</c>, <c>all</c>, <c>none</c>, or <c>unsupported</c>.</param>
/// <param name="Selected">Selected intent names, sorted.</param>
public sealed record IntentSelection(string Mode, IReadOnlyList<string> Selected)
{
    /// <summary>The requested intent, when one was supplied.</summary>
    public string? Requested { get; init; }
}

/// <summary>The entry matched by one input.</summary>
public sealed record ToolMatch(
    string EntryId,
    int EntryRevision,
    IReadOnlyList<MatchedIdentity> MatchedIdentities,
    VersionSelection VersionSelection,
    IntentSelection? IntentSelection = null);

/// <summary>Per-input attribution, in input order.</summary>
/// <param name="InputIndex">The input's position.</param>
/// <param name="Status">
/// The pair's version status, or <c>intent_unsupported</c>,
/// <c>tool_unmatched</c>, or <c>filesystem_identity_unresolved</c>.
/// </param>
/// <param name="Matches">At most one entry; empty for <c>tool_unmatched</c>.</param>
public sealed record ToolDiagnostics(int InputIndex, string Status, IReadOnlyList<ToolMatch> Matches);

/// <summary>A dependency pulled in by one or more inputs.</summary>
/// <param name="EntryId">The dependency entry.</param>
/// <param name="EntryRevision">Its entry revision.</param>
/// <param name="InputIndexes">Sorted, distinct indexes of every requiring input.</param>
/// <param name="VersionSelection">Always <c>matched_default</c>.</param>
/// <param name="IntentSelection">The selected dependency intents.</param>
public sealed record ResolvedDependency(
    string EntryId,
    int EntryRevision,
    IReadOnlyList<int> InputIndexes,
    VersionSelection VersionSelection,
    IntentSelection IntentSelection)
{
    /// <summary>The dependency's declared <c>vers</c> range, when one is declared.</summary>
    public string? RequiredVersionRange { get; init; }
}

/// <summary>A resolved path, its access class, and its source entries.</summary>
/// <param name="Path">The resolved path.</param>
/// <param name="Access"><c>denied</c>, <c>readonly</c>, or <c>readwrite</c>.</param>
/// <param name="EntryIds">Contributing entries.</param>
public sealed record PathRequirement(string Path, string Access, IReadOnlyList<string> EntryIds);

/// <summary>A complete egress rule and its source entries.</summary>
public sealed record NetworkRequirement(NetworkRulePolicy Rule, IReadOnlyList<string> EntryIds);

/// <summary>
/// A structured diagnostics warning. <see cref="Code"/> selects the category;
/// <see cref="Message"/> is human-readable, not a parsing contract.
/// </summary>
[JsonConverter(typeof(ResolutionWarningJsonConverter))]
public abstract record ResolutionWarning(string Code, string Message);

/// <summary>
/// A warning about one input: <c>version_out_of_range</c>,
/// <c>version_unparseable</c>, <c>intent_unsupported</c>,
/// <c>tool_unmatched</c>, <c>purl_invalid</c>,
/// <c>purl_components_ignored</c>, or <c>weak_identity</c>.
/// </summary>
public sealed record ToolResolutionWarning(string Code, int InputIndex, string Message)
    : ResolutionWarning(Code, Message)
{
    /// <summary>The matched entry, for codes that have one.</summary>
    public string? EntryId { get; init; }

    /// <summary>The detected version, for the version codes.</summary>
    public string? DetectedVersion { get; init; }

    /// <summary>The requested intent, for <c>intent_unsupported</c>.</summary>
    public string? Intent { get; init; }

    /// <summary>The invocation name, for <c>tool_unmatched</c> and <c>weak_identity</c>.</summary>
    public string? InvocationName { get; init; }

    /// <summary>The package URL, for the PURL codes.</summary>
    public string? PackageUrl { get; init; }

    /// <summary>
    /// <c>version</c>, <c>qualifiers</c>, and/or <c>subpath</c>, for
    /// <c>purl_components_ignored</c>.
    /// </summary>
    public IReadOnlyList<string>? IgnoredComponents { get; init; }
}

/// <summary>
/// A warning about shared resolution: <c>architecture_default</c>,
/// <c>architecture_fallback</c>, <c>symbol_resolved</c>,
/// <c>symbol_unresolved</c>, <c>filesystem_case_assumed</c>,
/// <c>filesystem_identity_unresolved</c>, <c>readonly_superseded</c>,
/// <c>filesystem_deny_removed</c>, or <c>network_deny_removed</c>.
/// </summary>
public record ResolutionDetailWarning(
    string Code,
    IReadOnlyList<int> InputIndexes,
    IReadOnlyList<string> EntryIds,
    string Message)
    : ResolutionWarning(Code, Message)
{
    /// <summary>The target platform, for the architecture and identity codes.</summary>
    public string? Platform { get; init; }

    /// <summary>The target architecture, for the architecture codes.</summary>
    public string? Architecture { get; init; }

    /// <summary><c>platform</c> or <c>default</c>, for <c>architecture_fallback</c>.</summary>
    public string? Selected { get; init; }

    /// <summary>The catalog symbol, for the symbol codes.</summary>
    public string? Symbol { get; init; }

    /// <summary>The resolved value, for <c>symbol_resolved</c>.</summary>
    public string? Value { get; init; }

    /// <summary>
    /// <c>caller</c>, <c>discovery</c>, <c>host</c>, or <c>default</c>, for
    /// <c>symbol_resolved</c>.
    /// </summary>
    public string? Source { get; init; }

    /// <summary>The affected paths, for the filesystem case and identity codes.</summary>
    public IReadOnlyList<string>? Paths { get; init; }

    /// <summary><c>case_sensitive</c>, for <c>filesystem_case_assumed</c>.</summary>
    public string? Comparison { get; init; }
}

/// <summary>
/// <c>readonly_superseded</c> or <c>filesystem_deny_removed</c>: the complete
/// original path requirement removed and the requirements that needed it gone.
/// </summary>
public sealed record PathAdjustmentWarning(
    string Code,
    IReadOnlyList<int> InputIndexes,
    IReadOnlyList<string> EntryIds,
    string Message,
    PathRequirement Removed,
    IReadOnlyList<PathRequirement> RequiredBy)
    : ResolutionDetailWarning(Code, InputIndexes, EntryIds, Message);

/// <summary>
/// <c>network_deny_removed</c>: the complete catalog egress deny removed and
/// the requested rules that overlapped it. Other grants may now apply
/// throughout its whole scope.
/// </summary>
public sealed record NetworkDenyRemovedWarning(
    IReadOnlyList<int> InputIndexes,
    IReadOnlyList<string> EntryIds,
    string Message,
    NetworkRequirement Removed,
    IReadOnlyList<NetworkRequirement> RequiredBy)
    : ResolutionDetailWarning("network_deny_removed", InputIndexes, EntryIds, Message);

/// <summary>Match attribution and warnings from one resolution pass.</summary>
public sealed record ToolRequirementsDiagnostics(
    string CatalogRevision,
    IReadOnlyList<ToolDiagnostics> Tools,
    IReadOnlyList<ResolvedDependency> ResolvedDependencies,
    IReadOnlyList<ResolutionWarning> Warnings);

/// <summary>
/// <b>PROTOTYPE, pending API review.</b> Result of
/// <see cref="MxcContainer.ResolveToolRequirementsWithDiagnostics(IReadOnlyList{ToolCandidate}, ResolveContext?)"/>.
/// </summary>
/// <param name="Requirements">The composed requirements, or <see langword="null"/> when none resolved.</param>
/// <param name="Diagnostics">Attribution and warnings from the same pass.</param>
public sealed record ToolRequirementsResolution(
    ContainerRequirements? Requirements,
    ToolRequirementsDiagnostics Diagnostics);

/// <summary>The bundled catalog's schema version, default revision, and SDK contract version.</summary>
public sealed record CatalogInfo(
    string CatalogSchemaVersion,
    string CatalogRevision,
    string SdkContractVersion);

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

/// <summary>Inspection metadata for one catalog entry. It never exposes a requirements body.</summary>
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

/// <summary>
/// Reads the structured warning union by <c>code</c>. Unknown codes or fields
/// are rejected rather than dropped.
/// </summary>
internal sealed class ResolutionWarningJsonConverter : JsonConverter<ResolutionWarning>
{
    private static readonly HashSet<string> ToolCodes = new(StringComparer.Ordinal)
    {
        "version_out_of_range", "version_unparseable", "intent_unsupported", "tool_unmatched",
        "purl_invalid", "purl_components_ignored", "weak_identity",
    };

    private static readonly HashSet<string> DetailCodes = new(StringComparer.Ordinal)
    {
        "architecture_default", "architecture_fallback", "symbol_resolved", "symbol_unresolved",
        "filesystem_case_assumed", "filesystem_identity_unresolved",
    };

    public override ResolutionWarning Read(
        ref Utf8JsonReader reader,
        Type typeToConvert,
        JsonSerializerOptions options)
    {
        using var document = JsonDocument.ParseValue(ref reader);
        var root = document.RootElement;
        if (root.ValueKind != JsonValueKind.Object)
        {
            throw new JsonException("A warning must be an object.");
        }

        var code = RequiredString(root, "code");
        var message = RequiredString(root, "message");
        if (ToolCodes.Contains(code))
        {
            Only(root, "code", "inputIndex", "message", "entryId", "detectedVersion", "intent",
                "invocationName", "packageUrl", "ignoredComponents");
            return new ToolResolutionWarning(code, Required(root, "inputIndex").GetInt32(), message)
            {
                EntryId = OptionalString(root, "entryId"),
                DetectedVersion = OptionalString(root, "detectedVersion"),
                Intent = OptionalString(root, "intent"),
                InvocationName = OptionalString(root, "invocationName"),
                PackageUrl = OptionalString(root, "packageUrl"),
                IgnoredComponents = OptionalStrings(root, "ignoredComponents"),
            };
        }

        var inputIndexes = Required(root, "inputIndexes").EnumerateArray().Select(i => i.GetInt32()).ToArray();
        var entryIds = RequiredStrings(root, "entryIds");
        switch (code)
        {
            case "readonly_superseded":
            case "filesystem_deny_removed":
                Only(root, "code", "inputIndexes", "entryIds", "message", "removed", "requiredBy");
                return new PathAdjustmentWarning(
                    code,
                    inputIndexes,
                    entryIds,
                    message,
                    ReadPath(Required(root, "removed")),
                    Required(root, "requiredBy").EnumerateArray().Select(ReadPath).ToArray());
            case "network_deny_removed":
                Only(root, "code", "inputIndexes", "entryIds", "message", "removed", "requiredBy");
                return new NetworkDenyRemovedWarning(
                    inputIndexes,
                    entryIds,
                    message,
                    ReadRule(Required(root, "removed"), options),
                    Required(root, "requiredBy").EnumerateArray().Select(r => ReadRule(r, options)).ToArray());
        }

        if (!DetailCodes.Contains(code))
        {
            throw new JsonException($"Unknown warning code '{code}'.");
        }

        Only(root, "code", "inputIndexes", "entryIds", "message", "platform", "architecture",
            "selected", "symbol", "value", "source", "paths", "comparison");
        return new ResolutionDetailWarning(code, inputIndexes, entryIds, message)
        {
            Platform = OptionalString(root, "platform"),
            Architecture = OptionalString(root, "architecture"),
            Selected = OptionalString(root, "selected"),
            Symbol = OptionalString(root, "symbol"),
            Value = OptionalString(root, "value"),
            Source = OptionalString(root, "source"),
            Paths = OptionalStrings(root, "paths"),
            Comparison = OptionalString(root, "comparison"),
        };
    }

    public override void Write(
        Utf8JsonWriter writer,
        ResolutionWarning value,
        JsonSerializerOptions options) =>
        throw new NotSupportedException("Resolution warnings are read-only results.");

    private static PathRequirement ReadPath(JsonElement element)
    {
        Only(element, "path", "access", "entryIds");
        return new PathRequirement(
            RequiredString(element, "path"),
            RequiredString(element, "access"),
            RequiredStrings(element, "entryIds"));
    }

    private static NetworkRequirement ReadRule(JsonElement element, JsonSerializerOptions options)
    {
        Only(element, "rule", "entryIds");
        var rule = Required(element, "rule").Deserialize(MxcJson.TypeInfo<NetworkRulePolicy>(options))
            ?? throw new JsonException("A network requirement needs a rule.");
        return new NetworkRequirement(rule, RequiredStrings(element, "entryIds"));
    }

    private static void Only(JsonElement element, params string[] allowed)
    {
        foreach (var property in element.EnumerateObject())
        {
            if (Array.IndexOf(allowed, property.Name) < 0)
            {
                throw new JsonException($"Unexpected warning field '{property.Name}'.");
            }
        }
    }

    private static JsonElement Required(JsonElement element, string name) =>
        element.TryGetProperty(name, out var value)
            ? value
            : throw new JsonException($"A warning is missing '{name}'.");

    private static string RequiredString(JsonElement element, string name) =>
        Required(element, name).GetString() ?? throw new JsonException($"'{name}' must be a string.");

    private static string[] RequiredStrings(JsonElement element, string name) =>
        Required(element, name).EnumerateArray()
            .Select(item => item.GetString() ?? throw new JsonException($"'{name}' must hold strings."))
            .ToArray();

    private static string? OptionalString(JsonElement element, string name) =>
        element.TryGetProperty(name, out var value) ? value.GetString() : null;

    private static string[]? OptionalStrings(JsonElement element, string name) =>
        element.TryGetProperty(name, out _) ? RequiredStrings(element, name) : null;
}
