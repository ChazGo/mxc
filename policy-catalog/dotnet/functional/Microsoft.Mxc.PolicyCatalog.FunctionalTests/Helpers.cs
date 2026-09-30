// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Shared helpers for the functional suite. The library is the packed
// Microsoft.Mxc.PolicyCatalog package restored from a local feed, and the CLI
// is a consumer build of policy-catalog against that same package; the driver
// (scripts/dotnet-functional.mjs) sets POLICY_CATALOG_CLI_DLL and
// POLICY_CATALOG_REPO_ROOT. Nothing here reads the repository's sources.
using System.Diagnostics;
using System.Text;
using System.Text.Json.Nodes;
using Xunit;

namespace Microsoft.Mxc.PolicyCatalog.FunctionalTests;

internal sealed record CliResult(int Status, string Stdout, string Stderr)
{
    public JsonNode? Json => string.IsNullOrWhiteSpace(Stdout) ? null : JsonNode.Parse(Stdout);

    public string Warnings => string.Join("\n", Json?["diagnostics"]?["warnings"]?.AsArray().Select(w => w!.GetValue<string>()) ?? Array.Empty<string>());

    public string? ErrorReason => Json?["error"]?["details"]?["reason"]?.GetValue<string>();

    public string ErrorMessage => Json?["error"]?["message"]?.GetValue<string>() ?? string.Empty;
}

internal sealed class FixedHost : IHostEnvironment
{
    private readonly string _platform;
    private readonly string _architecture;

    public FixedHost(string platform, string architecture)
    {
        _platform = platform;
        _architecture = architecture;
    }

    public string Platform() => _platform;

    public string NativeArchitecture() => _architecture;

    public string? Symbol(string name) => null;
}

internal static class Fx
{
    /// <summary>JSON.stringify-like escaping for comparisons (System.Text.Json escapes quotes by default).</summary>
    public static readonly System.Text.Json.JsonSerializerOptions Relaxed = new() { Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping, TypeInfoResolver = new System.Text.Json.Serialization.Metadata.DefaultJsonTypeInfoResolver() };

    public static readonly string[] Platforms = { "windows", "linux", "macos" };
    public static readonly string[] Architectures = { "x64", "arm64" };

    public static string CliDll => Environment.GetEnvironmentVariable("POLICY_CATALOG_CLI_DLL")
        ?? throw new InvalidOperationException("POLICY_CATALOG_CLI_DLL is not set; run the functional suite through scripts/dotnet-functional.mjs");

    /// <summary>The catalog directory shipped in the package (contentFiles copied to output).</summary>
    public static string InstalledCatalogDir => Path.Combine(AppContext.BaseDirectory, "catalog");

    public static JsonNode InstalledManifest => JsonNode.Parse(File.ReadAllText(Path.Combine(InstalledCatalogDir, "manifest.json")))!;

    public static CliResult Cli(params string[] args)
    {
        var start = new ProcessStartInfo("dotnet")
        {
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
            StandardOutputEncoding = new UTF8Encoding(false),
            StandardErrorEncoding = new UTF8Encoding(false),
            WorkingDirectory = Path.GetTempPath(),
        };
        start.ArgumentList.Add(CliDll);
        foreach (var arg in args)
        {
            start.ArgumentList.Add(arg);
        }

        using var process = Process.Start(start)!;
        var stderr = process.StandardError.ReadToEndAsync();
        var stdout = process.StandardOutput.ReadToEnd();
        process.WaitForExit();
        return new CliResult(process.ExitCode, stdout, stderr.Result);
    }

    public static readonly Dictionary<string, (string Root, string Prefix, string Cache)> PlatformContext = new()
    {
        ["windows"] = ("C:\\work\\app", "C:\\tools", "C:\\cache\\npm"),
        ["linux"] = ("/work/app", "/opt/tools", "/var/cache/npm"),
        ["macos"] = ("/Users/dev/app", "/opt/homebrew/bin", "/Users/dev/.npm"),
    };

    public static string[] SymbolArgs(string platform)
    {
        var c = PlatformContext[platform];
        return new[]
        {
            "--project-root", c.Root,
            "--symbol", $"git_prefix={c.Prefix}",
            "--symbol", $"node_prefix={c.Prefix}",
            "--symbol", $"npm_prefix={c.Prefix}",
            "--symbol", $"npm_cache={c.Cache}",
        };
    }

    public static string[] FullContext(string platform, string architecture) =>
        new[] { "--platform", platform, "--architecture", architecture, "--allow-weak" }.Concat(SymbolArgs(platform)).ToArray();

    public static string[] Args(params object[] parts) =>
        parts.SelectMany(part => part is string s ? new[] { s } : (IEnumerable<string>)part).ToArray();

    public static JsonObject Entry(string entryId, string? overrides = null)
    {
        var name = entryId.Split(':')[1];
        var entry = new JsonObject
        {
            ["entryId"] = entryId,
            ["entryRevision"] = 1,
            ["displayName"] = name,
            ["identity"] = JsonNode.Parse($$"""[{ "kind": "invocation-name", "names": ["{{name}}"] }]"""),
            ["platformVariants"] = JsonNode.Parse($$$"""[{ "when": { "platform": "linux" }, "sandboxPolicy": { "version": "0.9.0-alpha", "filesystem": { "readonlyPaths": ["${git_prefix}/{{{name}}}"] } } }]"""),
            ["provenance"] = JsonNode.Parse("""{ "method": "functional-test", "sourceRevision": "functional-test" }"""),
        };
        if (overrides is not null)
        {
            foreach (var (key, value) in JsonNode.Parse(overrides)!.AsObject().ToList())
            {
                entry[key] = value?.DeepClone();
            }
        }

        return entry;
    }

    public static JsonObject DependsOn(string entryId, string target)
    {
        var name = target.Split(':')[1];
        return Entry(entryId, $$$"""{ "platformVariants": [{ "when": { "platform": "linux" }, "dependencies": [{ "entryId": "{{{target}}}" }], "sandboxPolicy": { "version": "0.9.0-alpha", "filesystem": { "readonlyPaths": ["${git_prefix}/{{{name}}}-dep"] } } }] }""");
    }

    public static JsonObject Revision(IEnumerable<JsonNode> entries, string catalogRevision = "2000-01-01.1") => new()
    {
        ["catalogSchemaVersion"] = "1",
        ["catalogRevision"] = catalogRevision,
        ["entries"] = new JsonArray(entries.ToArray()),
    };

    /// <summary>A complete catalog directory with correct digests, so the only defect is the one a test adds.</summary>
    public static string WriteCatalog(string dir, IReadOnlyList<JsonObject> revisions, string? defaultRevision = null)
    {
        Directory.CreateDirectory(Path.Combine(dir, "revisions"));
        File.Copy(Path.Combine(InstalledCatalogDir, "contract.v1.json"), Path.Combine(dir, "contract.v1.json"), true);
        var list = new JsonArray();
        foreach (var revision in revisions)
        {
            var id = revision["catalogRevision"]!.GetValue<string>();
            var file = $"revisions/{id}.json";
            var text = revision.ToJsonString(new System.Text.Json.JsonSerializerOptions { WriteIndented = true });
            File.WriteAllText(Path.Combine(dir, file), text + "\n");
            list.Add(new JsonObject { ["catalogRevision"] = id, ["file"] = file, ["sha256"] = PolicyCatalogJson.CanonicalSha256(text) });
        }

        var manifest = new JsonObject
        {
            ["catalogSchemaVersion"] = "1",
            ["defaultRevision"] = defaultRevision ?? revisions[^1]["catalogRevision"]!.GetValue<string>(),
            ["revisions"] = list,
        };
        File.WriteAllText(Path.Combine(dir, "manifest.json"), manifest.ToJsonString() + "\n");
        return dir;
    }

    public static void CopyDirectory(string from, string to)
    {
        Directory.CreateDirectory(to);
        foreach (var file in Directory.GetFiles(from, "*", SearchOption.AllDirectories))
        {
            var target = Path.Combine(to, Path.GetRelativePath(from, file));
            Directory.CreateDirectory(Path.GetDirectoryName(target)!);
            File.Copy(file, target, true);
        }
    }
}

/// <summary>A per-test temporary directory outside the repository.</summary>
public abstract class WorkDirTest : IDisposable
{
    protected WorkDirTest()
    {
        Work = Path.Combine(Path.GetTempPath(), "policy-catalog-dotnet-functional-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(Work);
    }

    protected string Work { get; }

    protected string NewDir(string prefix)
    {
        var dir = Path.Combine(Work, prefix + Guid.NewGuid().ToString("N").Substring(0, 8));
        Directory.CreateDirectory(dir);
        return dir;
    }

    public void Dispose()
    {
        try
        {
            foreach (var file in Directory.GetFiles(Work, "*", SearchOption.AllDirectories))
            {
                File.SetAttributes(file, FileAttributes.Normal);
            }

            Directory.Delete(Work, true);
        }
        catch (IOException)
        {
        }
        catch (UnauthorizedAccessException)
        {
        }

        GC.SuppressFinalize(this);
    }
}

public sealed class PackageTests
{
    [Fact]
    public void LibraryComesFromTheRestoredPackageNotTheSourceTree()
    {
        var location = typeof(PolicyCatalog).Assembly.Location;
        var repo = Environment.GetEnvironmentVariable("POLICY_CATALOG_REPO_ROOT");
        Assert.False(string.IsNullOrEmpty(repo), "POLICY_CATALOG_REPO_ROOT is not set");
        Assert.False(location.StartsWith(Path.GetFullPath(repo!), StringComparison.OrdinalIgnoreCase), $"{location} is inside the repository {repo}");
        Assert.False(Fx.CliDll.StartsWith(Path.GetFullPath(repo!), StringComparison.OrdinalIgnoreCase), $"{Fx.CliDll} is inside the repository");
        var packages = Environment.GetEnvironmentVariable("NUGET_PACKAGES");
        Assert.False(string.IsNullOrEmpty(packages));
        Assert.True(File.Exists(Path.Combine(packages!, "microsoft.mxc.policycatalog", "0.0.0-prototype", "lib", "net8.0", "Microsoft.Mxc.PolicyCatalog.dll")));
        Assert.True(File.Exists(Path.Combine(Fx.InstalledCatalogDir, "manifest.json")));
    }
}
