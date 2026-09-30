// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using System.Text.Json;
using System.Text.Json.Nodes;
using Xunit;

namespace Microsoft.Mxc.PolicyCatalog.Tests;

/// <summary>Host with fixed facts, so tests never depend on the machine running them.</summary>
internal sealed class FixedHost : IHostEnvironment
{
    private readonly string _platform;
    private readonly Func<string> _architecture;
    private readonly IReadOnlyDictionary<string, string> _symbols;

    public FixedHost(string platform = "linux", string architecture = "x64", IReadOnlyDictionary<string, string>? symbols = null)
        : this(platform, () => architecture, symbols)
    {
    }

    public FixedHost(string platform, Func<string> architecture, IReadOnlyDictionary<string, string>? symbols = null)
    {
        _platform = platform;
        _architecture = architecture;
        _symbols = symbols ?? new Dictionary<string, string>();
    }

    public int ArchitectureCalls { get; private set; }

    public string Platform() => _platform;

    public string NativeArchitecture()
    {
        ArchitectureCalls++;
        return _architecture();
    }

    public string? Symbol(string name) => _symbols.TryGetValue(name, out var value) ? value : null;
}

internal static class TestData
{
    public const string V = "0.9.0-alpha";

    public static string Resource(string name)
    {
        var assembly = typeof(TestData).Assembly;
        var actual = assembly.GetManifestResourceNames().Single(resource => resource.Replace('\\', '/') == name);
        using var stream = assembly.GetManifestResourceStream(actual)!;
        using var reader = new StreamReader(stream);
        return reader.ReadToEnd();
    }

    public static IEnumerable<string> Resources(string prefix) =>
        typeof(TestData).Assembly.GetManifestResourceNames()
            .Select(resource => resource.Replace('\\', '/'))
            .Where(resource => resource.StartsWith(prefix, StringComparison.Ordinal))
            .OrderBy(resource => resource, StringComparer.Ordinal);

    public static string Contract => Resource("catalog/contract.v1.json");

    /// <summary>An in-memory store whose manifest publishes the given revisions with correct (or overridden) digests.</summary>
    public static CatalogStore StoreFor(IReadOnlyList<JsonNode> revisions, string? defaultRevision = null, IReadOnlyDictionary<string, string>? digests = null)
    {
        var files = new Dictionary<string, string>();
        var manifestRevisions = new JsonArray();
        foreach (var revision in revisions)
        {
            var id = revision["catalogRevision"]!.GetValue<string>();
            var file = $"revisions/{id}.json";
            var text = revision.ToJsonString();
            files[file] = text;
            manifestRevisions.Add(new JsonObject
            {
                ["catalogRevision"] = id,
                ["file"] = file,
                ["sha256"] = digests is not null && digests.TryGetValue(id, out var digest) ? digest : PolicyCatalogJson.CanonicalSha256(text),
            });
        }

        var manifest = new JsonObject
        {
            ["catalogSchemaVersion"] = "1",
            ["defaultRevision"] = defaultRevision ?? revisions[^1]["catalogRevision"]!.GetValue<string>(),
            ["revisions"] = manifestRevisions,
        };
        return CatalogStore.FromJson(Contract, manifest.ToJsonString(), file => files[file]);
    }

    public static PolicyCatalog CatalogFor(JsonNode revision, IHostEnvironment? host = null) =>
        new(StoreFor(new[] { revision }), host ?? new FixedHost());

    public static PolicyCatalog Bundled(IHostEnvironment? host = null) => new(CatalogStore.Bundled(), host ?? new FixedHost());

    public static JsonNode Revision(IEnumerable<JsonNode> entries, string catalogRevision = "2000-01-01.1") => new JsonObject
    {
        ["catalogSchemaVersion"] = "1",
        ["catalogRevision"] = catalogRevision,
        ["entries"] = new JsonArray(entries.ToArray()),
    };

    public static JsonNode Parse(string json) => JsonNode.Parse(json)!;

    /// <summary>Minimal valid entry; <paramref name="overrides"/> is a JSON object merged over it.</summary>
    public static JsonNode Entry(string entryId, string? overrides = null)
    {
        var name = entryId.Split(':')[1];
        var entry = new JsonObject
        {
            ["entryId"] = entryId,
            ["entryRevision"] = 1,
            ["displayName"] = name,
            ["identity"] = Parse($$"""[{ "kind": "invocation-name", "names": ["{{name}}"] }]"""),
            ["platformVariants"] = Parse("""[{ "when": { "platform": "linux" }, "sandboxPolicy": { "version": "0.9.0-alpha", "filesystem": { "readwritePaths": ["${project_root}"] } } }]"""),
            ["provenance"] = Parse("""{ "method": "test", "sourceRevision": "test" }"""),
        };
        if (overrides is not null)
        {
            foreach (var (key, value) in Parse(overrides).AsObject().ToList())
            {
                entry[key] = value?.DeepClone();
            }
        }

        return entry;
    }

    /// <summary>The failure's reason, after checking that its code is the reason's code.</summary>
    public static string? ErrorReason(Action action)
    {
        try
        {
            action();
        }
        catch (PolicyCatalogException error)
        {
            Assert.Equal(PolicyCatalogException.CodeFor(error.ErrorReason), error.ErrorCode);
            Assert.StartsWith($"[{error.Code}] ", error.Message, StringComparison.Ordinal);
            return error.Reason;
        }

        return null;
    }

    public static PolicyCatalogException Failure(Action action) => Assert.Throws<PolicyCatalogException>(action);

    // --- fixture conversion --------------------------------------------------

    public static List<ToolInput> Tools(JsonElement tools)
    {
        if (tools.ValueKind == JsonValueKind.Array)
        {
            return tools.EnumerateArray().Select(Tool).ToList();
        }

        return new List<ToolInput> { Tool(tools) };
    }

    public static bool IsSingle(JsonElement tools) => tools.ValueKind != JsonValueKind.Array;

    public static ToolInput Tool(JsonElement tool)
    {
        if (tool.ValueKind == JsonValueKind.String)
        {
            return tool.GetString()!;
        }

        return new ToolInput(tool.GetProperty("invocationName").GetString()!)
        {
            PackageUrl = tool.TryGetProperty("packageUrl", out var purl) ? purl.GetString() : null,
            DetectedVersion = tool.TryGetProperty("detectedVersion", out var version) ? version.GetString() : null,
        };
    }

    public static ResolveContext Context(JsonElement? context)
    {
        if (context is not { } ctx || ctx.ValueKind != JsonValueKind.Object)
        {
            return new ResolveContext();
        }

        string? Str(string name) => ctx.TryGetProperty(name, out var value) ? value.GetString() : null;
        Dictionary<string, string?>? symbols = null;
        if (ctx.TryGetProperty("symbols", out var s))
        {
            symbols = s.EnumerateObject().ToDictionary(p => p.Name, p => p.Value.GetString());
        }

        return new ResolveContext
        {
            Platform = Str("platform"),
            Architecture = Str("architecture"),
            ProjectRoot = Str("projectRoot"),
            CatalogRevision = Str("catalogRevision"),
            AllowWeakIdentityFallback = ctx.TryGetProperty("allowWeakIdentityFallback", out var weak) && weak.GetBoolean(),
            Symbols = symbols,
        };
    }

    public static string Canonical(string json) => PolicyCatalogJson.Canonicalize(json);
}
