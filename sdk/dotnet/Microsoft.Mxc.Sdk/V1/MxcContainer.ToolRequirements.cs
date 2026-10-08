// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using System.Text;
using System.Text.Json;
using Microsoft.Mxc.Sdk.Native;

namespace Microsoft.Mxc.Sdk.V1;

/// <summary>
/// <b>PROTOTYPE, pending API review.</b> The policy store's tool-requirements
/// lookup: resolve known tools to command-free
/// <see cref="ContainerRequirements"/> from the catalog bundled statically in
/// the native <c>mxc_ffi</c> library.
/// </summary>
/// <remarks>
/// <para>
/// The result is a best-effort floor, not a guarantee, and complementary to
/// Learning Mode. Lookup never grants access, creates a container, contacts a
/// network service, or writes state; nothing is downloaded. Names and shapes
/// may change before sign-off, and this API is not part of MXC 1.0.
/// </para>
/// <para>
/// Failures throw <see cref="MxcException"/>: <see cref="MxcException.Code"/>
/// is the MXC error code and <see cref="MxcException.Reason"/> the optional
/// stable reason (for example <c>invalid_context</c>).
/// </para>
/// </remarks>
public static partial class MxcContainer
{
    /// <summary>
    /// <b>PROTOTYPE, pending API review.</b> Resolve one tool to composed
    /// container requirements.
    /// </summary>
    /// <returns>The requirements, or <see langword="null"/> when none resolved.</returns>
    /// <exception cref="MxcException">The input, context, or catalog is invalid.</exception>
    public static ContainerRequirements? ResolveToolRequirements(
        ToolCandidate tool,
        ResolveContext? context = null)
    {
        ArgumentNullException.ThrowIfNull(tool);
        return ResolveToolRequirements(new[] { tool }, context);
    }

    /// <summary>
    /// <b>PROTOTYPE, pending API review.</b> Resolve several tools to one set of
    /// composed container requirements. The result may cover only some of the
    /// tools; use <see cref="ResolveToolRequirementsWithDiagnostics(IReadOnlyList{ToolCandidate}, ResolveContext?)"/>
    /// to see which contributed.
    /// </summary>
    /// <returns>The requirements, or <see langword="null"/> when none resolved.</returns>
    /// <exception cref="MxcException">The input, context, or catalog is invalid.</exception>
    public static ContainerRequirements? ResolveToolRequirements(
        IReadOnlyList<ToolCandidate> tools,
        ResolveContext? context = null)
    {
        var json = CallResolve(ToolRequirementsRequest(tools, context), withDiagnostics: false);
        return ParseToolRequirementsResolution(json, requireDiagnostics: false).Requirements;
    }

    /// <summary>
    /// <b>PROTOTYPE, pending API review.</b> Like
    /// <see cref="ResolveToolRequirements(ToolCandidate, ResolveContext?)"/>, plus
    /// per-input statuses, contributing entries and dependencies, and
    /// structured warnings from the same pass.
    /// </summary>
    /// <exception cref="MxcException">The input, context, or catalog is invalid.</exception>
    public static ToolRequirementsResolution ResolveToolRequirementsWithDiagnostics(
        ToolCandidate tool,
        ResolveContext? context = null)
    {
        ArgumentNullException.ThrowIfNull(tool);
        return ResolveToolRequirementsWithDiagnostics(new[] { tool }, context);
    }

    /// <summary>
    /// <b>PROTOTYPE, pending API review.</b> Like
    /// <see cref="ResolveToolRequirements(IReadOnlyList{ToolCandidate}, ResolveContext?)"/>,
    /// plus per-input statuses, contributing entries and dependencies, and
    /// structured warnings from the same pass.
    /// </summary>
    /// <exception cref="MxcException">The input, context, or catalog is invalid.</exception>
    public static ToolRequirementsResolution ResolveToolRequirementsWithDiagnostics(
        IReadOnlyList<ToolCandidate> tools,
        ResolveContext? context = null)
    {
        var json = CallResolve(ToolRequirementsRequest(tools, context), withDiagnostics: true);
        return ParseToolRequirementsResolution(json, requireDiagnostics: true);
    }

    /// <summary>
    /// <b>PROTOTYPE, pending API review.</b> Asynchronous form of
    /// <see cref="ResolveToolRequirements(ToolCandidate, ResolveContext?)"/>;
    /// lookup examines host filesystem objects off the calling thread.
    /// </summary>
    public static Task<ContainerRequirements?> ResolveToolRequirementsAsync(
        ToolCandidate tool,
        ResolveContext? context = null,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(tool);
        return ResolveToolRequirementsAsync(new[] { tool }, context, cancellationToken);
    }

    /// <summary>
    /// <b>PROTOTYPE, pending API review.</b> Asynchronous form of
    /// <see cref="ResolveToolRequirements(IReadOnlyList{ToolCandidate}, ResolveContext?)"/>.
    /// </summary>
    public static Task<ContainerRequirements?> ResolveToolRequirementsAsync(
        IReadOnlyList<ToolCandidate> tools,
        ResolveContext? context = null,
        CancellationToken cancellationToken = default)
    {
        var request = ToolRequirementsRequest(tools, context);
        return MxcLifecycle.RunBlockingOperationAsync(
            () => ParseToolRequirementsResolution(
                CallResolve(request, withDiagnostics: false),
                requireDiagnostics: false).Requirements,
            _ => { },
            cancellationToken);
    }

    /// <summary>
    /// <b>PROTOTYPE, pending API review.</b> Asynchronous form of
    /// <see cref="ResolveToolRequirementsWithDiagnostics(ToolCandidate, ResolveContext?)"/>.
    /// </summary>
    public static Task<ToolRequirementsResolution> ResolveToolRequirementsWithDiagnosticsAsync(
        ToolCandidate tool,
        ResolveContext? context = null,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(tool);
        return ResolveToolRequirementsWithDiagnosticsAsync(new[] { tool }, context, cancellationToken);
    }

    /// <summary>
    /// <b>PROTOTYPE, pending API review.</b> Asynchronous form of
    /// <see cref="ResolveToolRequirementsWithDiagnostics(IReadOnlyList{ToolCandidate}, ResolveContext?)"/>.
    /// </summary>
    public static Task<ToolRequirementsResolution> ResolveToolRequirementsWithDiagnosticsAsync(
        IReadOnlyList<ToolCandidate> tools,
        ResolveContext? context = null,
        CancellationToken cancellationToken = default)
    {
        var request = ToolRequirementsRequest(tools, context);
        return MxcLifecycle.RunBlockingOperationAsync(
            () => ParseToolRequirementsResolution(
                CallResolve(request, withDiagnostics: true),
                requireDiagnostics: true),
            _ => { },
            cancellationToken);
    }

    /// <summary>
    /// <b>PROTOTYPE, pending API review.</b> The bundled catalog's schema
    /// version, default revision, and SDK contract version.
    /// </summary>
    public static CatalogInfo GetCatalogInfo() =>
        DeserializeStoreResult<CatalogInfo>(CallInspect(listEntries: false));

    /// <summary>
    /// <b>PROTOTYPE, pending API review.</b> Metadata for every entry in the
    /// bundled catalog's default revision. It never exposes a requirements body.
    /// </summary>
    public static IReadOnlyList<CatalogEntryMetadata> ListCatalogEntries() =>
        DeserializeStoreResult<CatalogEntryMetadata[]>(CallInspect(listEntries: true));

    /// <summary>
    /// Writes the <c>{"tools", "context"?}</c> lookup request. Validation is the
    /// native resolver's; this only rejects null references.
    /// </summary>
    internal static byte[] ToolRequirementsRequest(
        IReadOnlyList<ToolCandidate> tools,
        ResolveContext? context)
    {
        ArgumentNullException.ThrowIfNull(tools);
        using var buffer = new MemoryStream();
        using (var writer = new Utf8JsonWriter(buffer))
        {
            writer.WriteStartObject();
            writer.WriteStartArray("tools");
            foreach (var tool in tools)
            {
                if (tool is null)
                {
                    throw new ArgumentException("A tool candidate is null.", nameof(tools));
                }

                writer.WriteStartObject();
                writer.WriteString("invocationName", tool.InvocationName);
                WriteOptional(writer, "packageUrl", tool.PackageUrl);
                WriteOptional(writer, "detectedVersion", tool.DetectedVersion);
                WriteOptional(writer, "intent", tool.Intent);
                writer.WriteEndObject();
            }

            writer.WriteEndArray();
            if (context is not null)
            {
                writer.WriteStartObject("context");
                WriteOptional(writer, "projectRoot", context.ProjectRoot);
                if (context.Symbols is not null)
                {
                    writer.WriteStartObject("symbols");
                    foreach (var (name, value) in context.Symbols)
                    {
                        writer.WriteString(name, value);
                    }

                    writer.WriteEndObject();
                }

                WriteOptional(writer, "platform", context.Platform);
                WriteOptional(writer, "architecture", context.Architecture);
                WriteOptional(writer, "catalogRevision", context.CatalogRevision);
                writer.WriteBoolean("allowWeakIdentityFallback", context.AllowWeakIdentityFallback);
                writer.WriteEndObject();
            }

            writer.WriteEndObject();
        }

        buffer.WriteByte(0);
        return buffer.ToArray();
    }

    private static void WriteOptional(Utf8JsonWriter writer, string name, string? value)
    {
        if (value is not null)
        {
            writer.WriteString(name, value);
        }
    }

    private static unsafe string CallResolve(byte[] request, bool withDiagnostics)
    {
        fixed (byte* requestPtr = request)
        {
            MxcPolicyStoreResult result = default;
            var status = withDiagnostics
                ? NativeMethods.mxc_resolve_tool_requirements_with_diagnostics_json(requestPtr, &result)
                : NativeMethods.mxc_resolve_tool_requirements_json(requestPtr, &result);
            return TakeStoreResult(status, &result);
        }
    }

    private static unsafe string CallInspect(bool listEntries)
    {
        MxcPolicyStoreResult result = default;
        var status = listEntries
            ? NativeMethods.mxc_list_policy_catalog_entries_json(&result)
            : NativeMethods.mxc_policy_catalog_info_json(&result);
        return TakeStoreResult(status, &result);
    }

    private static unsafe string TakeStoreResult(int status, MxcPolicyStoreResult* result)
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

    /// <summary>
    /// Maps the native result onto the v1 types. A field the SDK model cannot
    /// hold is rejected rather than silently dropped from the requirements.
    /// </summary>
    internal static ToolRequirementsResolution ParseToolRequirementsResolution(
        string json,
        bool requireDiagnostics)
    {
        var resolution = DeserializeStoreResult<NativeToolRequirementsResolution>(json);
        if (requireDiagnostics && resolution.Diagnostics is null)
        {
            throw new MxcException(ErrorCode.BackendError, "The policy store returned no diagnostics.");
        }

        if (resolution.Diagnostics is { } diagnostics)
        {
            ValidateDiagnostics(resolution.Requirements, diagnostics);
        }

        return new ToolRequirementsResolution(
            resolution.Requirements,
            resolution.Diagnostics ?? EmptyToolRequirementsDiagnostics);
    }

    /// <summary>
    /// Rejects a diagnostics result whose shape the SDK cannot vouch for: input
    /// records out of order, or output present unless at least one input
    /// contributes. Unfamiliar status strings stay descriptive.
    /// </summary>
    private static void ValidateDiagnostics(
        ContainerRequirements? requirements,
        ToolRequirementsDiagnostics diagnostics)
    {
        string? problem = null;
        if (diagnostics.CatalogRevision is null
            || diagnostics.Tools is null
            || diagnostics.ResolvedDependencies is null
            || diagnostics.Warnings is null)
        {
            problem = "a required diagnostics field is missing";
        }
        else
        {
            for (var index = 0; problem is null && index < diagnostics.Tools.Count; index++)
            {
                var tool = diagnostics.Tools[index];
                if (tool is null || tool.InputIndex != index || tool.Status is null)
                {
                    problem = $"tools[{index}] is malformed or out of input order";
                }
            }

            if (problem is null
                && (requirements is not null) != diagnostics.Tools.Any(tool => tool.Contributes))
            {
                problem = "requirements must be present exactly when at least one input contributes";
            }
        }

        if (problem is not null)
        {
            throw new MxcException(
                ErrorCode.BackendError,
                $"The policy store result does not match the SDK v1 types: {problem}.");
        }
    }

    private static T DeserializeStoreResult<T>(string json)
    {
        try
        {
            return MxcJson.Deserialize<T>(json, MxcJson.PolicyStoreOptions)
                ?? throw new JsonException("The policy store returned null JSON.");
        }
        catch (JsonException error)
        {
            throw new MxcException(
                ErrorCode.BackendError,
                $"The policy store result does not match the SDK v1 types: {error.Message}",
                error);
        }
    }

    private static readonly ToolRequirementsDiagnostics EmptyToolRequirementsDiagnostics =
        new(string.Empty, Array.Empty<ToolDiagnostics>(), Array.Empty<ResolvedDependency>(), Array.Empty<ResolutionWarning>());
}

/// <summary>The native <c>{"requirements"?, "diagnostics"?}</c> document.</summary>
internal sealed record NativeToolRequirementsResolution(
    ContainerRequirements? Requirements,
    ToolRequirementsDiagnostics? Diagnostics);
