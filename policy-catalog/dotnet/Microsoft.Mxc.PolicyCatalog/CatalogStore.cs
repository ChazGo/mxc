// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using System.Collections.Concurrent;
using System.Reflection;
using System.Text.RegularExpressions;
using Microsoft.Mxc.PolicyCatalog.Internal;

namespace Microsoft.Mxc.PolicyCatalog;

/// <summary>One published revision as listed in <c>manifest.json</c>.</summary>
/// <param name="CatalogRevision">The revision ID (<c>YYYY-MM-DD.N</c>).</param>
/// <param name="File">The revision file, <c>revisions/&lt;id&gt;.json</c>.</param>
/// <param name="Sha256">Lower-case hex SHA-256 of the revision's canonical JSON.</param>
public sealed record ManifestRevision(string CatalogRevision, string File, string Sha256);

/// <summary>
/// Read-only access to locally installed, integrity-validated catalog revisions. Revisions are loaded
/// lazily, verified against the manifest digest, validated against the contract, and cached.
/// </summary>
public sealed class CatalogStore
{
    private static readonly Regex Sha256Pattern = new("^[0-9a-f]{64}\\z", RegexOptions.CultureInvariant);
    private static readonly Lazy<CatalogStore> BundledStore = new(LoadBundled, LazyThreadSafetyMode.ExecutionAndPublication);

    private readonly Func<string, JsonValue> _readRevision;
    private readonly ConcurrentDictionary<string, CatalogRevisionData> _loaded = new(StringComparer.Ordinal);

    internal CatalogStore(JsonValue contract, JsonValue manifest, Func<string, JsonValue> readRevision)
    {
        _readRevision = readRevision;
        Contract = Catalog.ValidateContract(contract);
        var (defaultRevision, revisions) = ValidateManifest(manifest);
        DefaultRevision = defaultRevision;
        Revisions = revisions;
    }

    internal CatalogContract Contract { get; }

    /// <summary>The installed default revision.</summary>
    public string DefaultRevision { get; }

    /// <summary>The published revisions in manifest order.</summary>
    public IReadOnlyList<ManifestRevision> Revisions { get; }

    /// <summary>Every installed revision ID, in manifest order.</summary>
    public IReadOnlyList<string> AvailableRevisions => Revisions.Select(revision => revision.CatalogRevision).ToList();

    /// <summary>The catalog embedded in this assembly (the repository's <c>catalog/</c> directory).</summary>
    /// <returns>The shared bundled store.</returns>
    /// <exception cref="PolicyCatalogException">The bundled data is invalid.</exception>
    public static CatalogStore Bundled() => BundledStore.Value;

    /// <summary>Loads a store from a catalog directory (<c>contract.v1.json</c>, <c>manifest.json</c>, <c>revisions/</c>).</summary>
    /// <param name="directory">The catalog directory.</param>
    /// <returns>The store.</returns>
    /// <exception cref="PolicyCatalogException">The data cannot be read (<c>integrity</c>) or is invalid (<c>invalid_catalog</c>).</exception>
    public static CatalogStore FromDirectory(string directory)
    {
        var baseDir = Path.GetFullPath(directory);
        JsonValue contract;
        JsonValue manifest;
        try
        {
            contract = ReadJsonFile(Path.Combine(baseDir, "contract.v1.json"));
            manifest = ReadJsonFile(Path.Combine(baseDir, "manifest.json"));
        }
        catch (Exception error) when (error is not PolicyCatalogException)
        {
            throw new PolicyCatalogException(PolicyCatalogErrorReason.Integrity, $"catalog data could not be read: {error.Message}");
        }

        return new CatalogStore(contract, manifest, file => ReadJsonFile(Path.Combine(baseDir, file)));
    }

    /// <summary>Creates a store from in-memory JSON text (used by tests and fixtures).</summary>
    /// <param name="contractJson">Contract JSON text.</param>
    /// <param name="manifestJson">Manifest JSON text.</param>
    /// <param name="readRevisionJson">Returns the JSON text of a manifest <c>file</c> entry.</param>
    /// <returns>The store.</returns>
    /// <exception cref="PolicyCatalogException">The contract or manifest is invalid.</exception>
    public static CatalogStore FromJson(string contractJson, string manifestJson, Func<string, string> readRevisionJson)
    {
        ArgumentNullException.ThrowIfNull(readRevisionJson);
        return new CatalogStore(JsonText.Parse(contractJson), JsonText.Parse(manifestJson), file => JsonText.Parse(readRevisionJson(file)));
    }

    internal static JsonValue ReadJsonFile(string path) => JsonText.Parse(JsonText.DecodeUtf8(File.ReadAllBytes(path)));

    private static CatalogStore LoadBundled()
    {
        var assembly = typeof(CatalogStore).Assembly;
        JsonValue contract;
        JsonValue manifest;
        try
        {
            contract = ReadResource(assembly, "contract.v1.json");
            manifest = ReadResource(assembly, "manifest.json");
        }
        catch (Exception error) when (error is not PolicyCatalogException)
        {
            throw new PolicyCatalogException(PolicyCatalogErrorReason.Integrity, $"catalog data could not be read: {error.Message}");
        }

        return new CatalogStore(contract, manifest, file => ReadResource(assembly, file));
    }

    private static JsonValue ReadResource(Assembly assembly, string file)
    {
        var name = "catalog/" + file.Replace('\\', '/');
        // MSBuild's RecursiveDir uses the build host's separator, so compare
        // resource names with separators normalized.
        var actual = assembly.GetManifestResourceNames().FirstOrDefault(resource => resource.Replace('\\', '/') == name);
        using var stream = (actual is null ? null : assembly.GetManifestResourceStream(actual))
            ?? throw new FileNotFoundException($"embedded catalog resource '{name}' does not exist");
        using var memory = new MemoryStream();
        stream.CopyTo(memory);
        return JsonText.Parse(JsonText.DecodeUtf8(memory.ToArray()));
    }

    private static PolicyCatalogException Invalid(string message) => new(PolicyCatalogErrorReason.InvalidCatalog, message);

    private static (string DefaultRevision, List<ManifestRevision> Revisions) ValidateManifest(JsonValue raw)
    {
        if (raw is not JsonObject obj)
        {
            throw Invalid("manifest root must be an object");
        }

        foreach (var key in obj.Keys)
        {
            if (key is not ("catalogSchemaVersion" or "defaultRevision" or "revisions"))
            {
                throw Invalid($"unsupported field 'manifest.{key}'");
            }
        }

        if (obj.Get("catalogSchemaVersion") is not JsonString { Value: Catalog.SchemaVersion })
        {
            throw Invalid($"manifest.catalogSchemaVersion must be '{Catalog.SchemaVersion}'");
        }

        if (obj.Get("revisions") is not JsonArray items || items.Count == 0)
        {
            throw Invalid("manifest.revisions must be a non-empty array");
        }

        var revisions = new List<ManifestRevision>();
        for (var index = 0; index < items.Count; index++)
        {
            var at = $"manifest.revisions[{index}]";
            if (items[index] is not JsonObject item)
            {
                throw Invalid($"'{at}' must be an object");
            }

            foreach (var key in item.Keys)
            {
                if (key is not ("catalogRevision" or "file" or "sha256"))
                {
                    throw Invalid($"unsupported field '{at}.{key}'");
                }
            }

            if (item.Get("catalogRevision") is not JsonString revisionValue || !Catalog.IsRevisionId(revisionValue.Value))
            {
                throw Invalid($"'{at}.catalogRevision' must match YYYY-MM-DD.N");
            }

            var catalogRevision = revisionValue.Value;
            var expectedFile = $"revisions/{catalogRevision}.json";
            if (item.Get("file") is not JsonString file || file.Value != expectedFile)
            {
                throw Invalid($"'{at}.file' must be '{expectedFile}'");
            }

            if (item.Get("sha256") is not JsonString sha256 || !Sha256Pattern.IsMatch(sha256.Value))
            {
                throw Invalid($"'{at}.sha256' must be a lower-case hex SHA-256 digest");
            }

            revisions.Add(new ManifestRevision(catalogRevision, file.Value, sha256.Value));
        }

        for (var index = 1; index < revisions.Count; index++)
        {
            if (Catalog.CompareRevisions(revisions[index - 1].CatalogRevision, revisions[index].CatalogRevision) >= 0)
            {
                throw Invalid($"manifest.revisions must be strictly increasing ('{revisions[index].CatalogRevision}')");
            }
        }

        if (obj.Get("defaultRevision") is not JsonString defaultRevision || !revisions.Any(r => r.CatalogRevision == defaultRevision.Value))
        {
            throw Invalid("manifest.defaultRevision must name a listed revision");
        }

        return (defaultRevision.Value, revisions);
    }

    /// <summary>
    /// Returns the requested revision, or the default when <paramref name="catalogRevision"/> is <c>null</c>.
    /// An explicitly requested revision that is not installed is an error; it is never substituted.
    /// </summary>
    internal CatalogRevisionData Revision(string? catalogRevision = null)
    {
        var id = catalogRevision ?? DefaultRevision;
        if (_loaded.TryGetValue(id, out var cached))
        {
            return cached;
        }

        var listed = Revisions.FirstOrDefault(revision => revision.CatalogRevision == id)
            ?? throw new PolicyCatalogException(PolicyCatalogErrorReason.RevisionUnavailable, $"catalog revision '{id}' is not installed");
        JsonValue raw;
        try
        {
            raw = _readRevision(listed.File);
        }
        catch (Exception error) when (error is not PolicyCatalogException)
        {
            throw new PolicyCatalogException(PolicyCatalogErrorReason.Integrity, $"catalog revision '{id}' could not be read: {error.Message}");
        }

        var digest = CanonicalJson.Sha256(raw);
        if (digest != listed.Sha256)
        {
            throw new PolicyCatalogException(
                PolicyCatalogErrorReason.Integrity,
                $"catalog revision '{id}' digest {digest} does not match the published digest {listed.Sha256}");
        }

        var revision = Catalog.ValidateRevision(raw, Contract);
        if (revision.CatalogRevision != id)
        {
            throw new PolicyCatalogException(PolicyCatalogErrorReason.Integrity, $"file '{listed.File}' declares revision '{revision.CatalogRevision}', expected '{id}'");
        }

        return _loaded.GetOrAdd(id, revision);
    }

    /// <summary>Loads and verifies a revision (integrity, contract, revision ID) without resolving anything.</summary>
    /// <param name="catalogRevision">The revision, or <c>null</c> for the default.</param>
    /// <exception cref="PolicyCatalogException">The revision is unavailable, tampered, or invalid.</exception>
    public void Verify(string? catalogRevision = null) => Revision(catalogRevision);
}
