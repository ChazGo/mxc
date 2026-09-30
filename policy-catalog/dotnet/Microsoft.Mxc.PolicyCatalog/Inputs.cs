// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

namespace Microsoft.Mxc.PolicyCatalog;

/// <summary>Catalog platform selector values (design §4.4).</summary>
public static class CatalogPlatforms
{
    /// <summary>Windows.</summary>
    public const string Windows = "windows";

    /// <summary>Linux.</summary>
    public const string Linux = "linux";

    /// <summary>macOS.</summary>
    public const string MacOS = "macos";

    /// <summary>Every selector, in catalog order.</summary>
    public static IReadOnlyList<string> All { get; } = new[] { Windows, Linux, MacOS };
}

/// <summary>Catalog architecture selector values (design §4.4).</summary>
public static class CatalogArchitectures
{
    /// <summary>x64 (AMD64 / x86_64).</summary>
    public const string X64 = "x64";

    /// <summary>ARM64 (aarch64).</summary>
    public const string Arm64 = "arm64";

    /// <summary>Every selector, in catalog order.</summary>
    public static IReadOnlyList<string> All { get; } = new[] { X64, Arm64 };
}

/// <summary>
/// Runtime lookup input (design §5.1). A plain string converts implicitly to an input with only
/// <see cref="InvocationName"/>.
/// </summary>
/// <param name="InvocationName">The bare invocation name (never a path).</param>
public sealed record ToolInput(string InvocationName)
{
    /// <summary>Package URL evidence (strong identity). The caller must have verified it.</summary>
    public string? PackageUrl { get; init; }

    /// <summary>Detected tool version evidence, compared with reviewed version ranges.</summary>
    public string? DetectedVersion { get; init; }

    /// <summary>String shorthand for <c>new ToolInput(name)</c>.</summary>
    /// <param name="invocationName">The invocation name.</param>
    public static implicit operator ToolInput(string invocationName) => new(invocationName);
}

/// <summary>Runtime lookup context, shared by every input in one lookup (design §5.1).</summary>
public sealed record ResolveContext
{
    /// <summary>The consumer's project root; the only source of the <c>project_root</c> symbol.</summary>
    public string? ProjectRoot { get; init; }

    /// <summary>Caller-supplied symbol values (caller and host symbols; host values override detection).</summary>
    public IReadOnlyDictionary<string, string?>? Symbols { get; init; }

    /// <summary>Target platform (<see cref="CatalogPlatforms"/>); defaults to the host platform.</summary>
    public string? Platform { get; init; }

    /// <summary>Target architecture (<see cref="CatalogArchitectures"/>); defaults to the native system architecture.</summary>
    public string? Architecture { get; init; }

    /// <summary>Explicit catalog revision; defaults to the installed default revision. Never substituted.</summary>
    public string? CatalogRevision { get; init; }

    /// <summary>Allows invocation-name-only (weak) identity matches. Off by default.</summary>
    public bool AllowWeakIdentityFallback { get; init; }
}

/// <summary>Host facts the resolver uses only when the caller omits them.</summary>
public interface IHostEnvironment
{
    /// <summary>The host platform selector. Throw <see cref="PolicyCatalogException"/> (<c>unsupported_host</c>) when it has none.</summary>
    /// <returns>A value of <see cref="CatalogPlatforms"/>.</returns>
    string Platform();

    /// <summary>
    /// The device's native system architecture (not the process architecture). Throw when it cannot be
    /// determined; never guess.
    /// </summary>
    /// <returns>A value of <see cref="CatalogArchitectures"/>.</returns>
    string NativeArchitecture();

    /// <summary>An approved host-known symbol (<c>source: "host"</c>) for the current host.</summary>
    /// <param name="name">The symbol name, e.g. <c>user_home</c>.</param>
    /// <returns>The value, or <c>null</c> when unknown.</returns>
    string? Symbol(string name);
}
