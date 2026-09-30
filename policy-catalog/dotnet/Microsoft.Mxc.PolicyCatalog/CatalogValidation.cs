// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Mxc.PolicyCatalog.Internal;

namespace Microsoft.Mxc.PolicyCatalog;

/// <summary>
/// Contribution and CI tooling (TypeScript <c>history.ts</c> and <c>validate.ts</c>): catalog-directory validation,
/// entry-revision history, and the <c>--base-ref</c> published-revision immutability check. The runtime lookup
/// path never uses this class or runs git.
/// </summary>
public static class CatalogValidation
{
    /// <summary>
    /// The on-disk copy of the bundled catalog shipped beside the consuming application
    /// (<c>&lt;app&gt;/catalog/</c>, package content copied to output). The embedded copy is what
    /// <see cref="CatalogStore.Bundled"/> reads; both come from the same source directory.
    /// </summary>
    public static string BundledCatalogDirectory => Path.Combine(AppContext.BaseDirectory, "catalog");

    /// <summary>
    /// Validates a catalog directory: manifest and contract validity, per-revision integrity, the full catalog
    /// contract, entry-revision history, and, when <paramref name="baseRef"/> is given, immutability of every revision
    /// published at that git ref. Never throws for catalog problems; they are reported in <see cref="CatalogValidationReport.Errors"/>.
    /// </summary>
    /// <param name="catalogDir">The catalog directory (relative paths resolve against the current directory).</param>
    /// <param name="baseRef">An optional git ref.</param>
    /// <returns>The report.</returns>
    public static CatalogValidationReport ValidateCatalogDirectory(string catalogDir, string? baseRef = null)
    {
        var dir = Path.GetFullPath(catalogDir);
        var errors = new List<string>();
        CatalogStore store;
        try
        {
            store = CatalogStore.FromDirectory(dir);
        }
        catch (Exception error)
        {
            errors.Add(DescribeError(error));
            return new CatalogValidationReport(false, dir, null, null, null, errors);
        }

        errors.AddRange(CheckStoreHistory(store));
        BaseRefReport? baseRefReport = null;
        if (baseRef is not null)
        {
            var (compared, baseErrors) = CheckAgainstBaseRef(dir, store.Revisions, baseRef);
            baseRefReport = new BaseRefReport(baseRef, compared);
            errors.AddRange(baseErrors);
        }

        return new CatalogValidationReport(errors.Count == 0, dir, store.DefaultRevision, store.AvailableRevisions, baseRefReport, errors);
    }

    /// <summary>Validates every installed revision (integrity + contract) and the entry-revision chain between consecutive revisions.</summary>
    /// <param name="store">The store.</param>
    /// <returns>Error strings; empty when valid.</returns>
    public static IReadOnlyList<string> CheckStoreHistory(CatalogStore store)
    {
        ArgumentNullException.ThrowIfNull(store);
        var errors = new List<string>();
        CatalogRevisionData? previous = null;
        foreach (var id in store.AvailableRevisions)
        {
            CatalogRevisionData current;
            try
            {
                current = store.Revision(id);
            }
            catch (PolicyCatalogException error)
            {
                errors.Add(error.Message);
                previous = null;
                continue;
            }

            if (previous is not null)
            {
                errors.AddRange(CheckEntryRevisions(previous, current));
            }

            previous = current;
        }

        return errors;
    }

    /// <summary>Entry-revision monotonicity between two consecutive published revisions (design §4.1, §10).</summary>
    internal static List<string> CheckEntryRevisions(CatalogRevisionData previous, CatalogRevisionData next)
    {
        var errors = new List<string>();
        if (Catalog.CompareRevisions(previous.CatalogRevision, next.CatalogRevision) >= 0)
        {
            errors.Add($"catalog revision '{next.CatalogRevision}' must be newer than '{previous.CatalogRevision}'");
        }

        var before = new Dictionary<string, Entry>(StringComparer.Ordinal);
        foreach (var entry in previous.Entries)
        {
            before[entry.EntryId] = entry;
        }

        foreach (var entry in next.Entries)
        {
            if (!before.TryGetValue(entry.EntryId, out var old))
            {
                continue;
            }

            var changed = SemanticKey(old) != SemanticKey(entry);
            var oldRevision = JsNumber.ToString(old.EntryRevision);
            var newRevision = JsNumber.ToString(entry.EntryRevision);
            if (changed && entry.EntryRevision <= old.EntryRevision)
            {
                errors.Add($"{next.CatalogRevision}: '{entry.EntryId}' changed but entryRevision did not increase ({oldRevision} -> {newRevision})");
            }
            else if (!changed && entry.EntryRevision != old.EntryRevision)
            {
                errors.Add($"{next.CatalogRevision}: '{entry.EntryId}' is unchanged but entryRevision moved ({oldRevision} -> {newRevision})");
            }
        }

        return errors;
    }

    // The validated entry holds exactly the raw entry's fields (unsupported fields
    // are rejected), so the raw object minus entryRevision is its semantic key.
    private static string SemanticKey(Entry entry)
    {
        var copy = new JsonObject();
        foreach (var key in entry.Raw.Keys)
        {
            if (key != "entryRevision")
            {
                copy.Set(key, entry.Raw.Get(key)!);
            }
        }

        return CanonicalJson.Serialize(copy);
    }

    // -----------------------------------------------------------------------
    // Published-revision immutability
    // -----------------------------------------------------------------------

    /// <summary>A raw manifest revision record from a published state (fields may be absent or of any JSON type).</summary>
    internal sealed record PublishedRevision(JsonValue? CatalogRevision, JsonValue? File, JsonValue? Sha256);

    /// <summary>Raw published state used to compare a proposed change against its base.</summary>
    internal sealed record PublishedState(IReadOnlyList<PublishedRevision>? Revisions, IReadOnlyDictionary<string, string> Files);

    private static bool StrictEquals(JsonValue? left, JsonValue? right) => (left, right) switch
    {
        (null, null) => true,
        (JsonString a, JsonString b) => a.Value == b.Value,
        (JsonNumber a, JsonNumber b) => a.Value == b.Value,
        (JsonBool a, JsonBool b) => a.Value == b.Value,
        (JsonNull, JsonNull) => true,
        _ => ReferenceEquals(left, right),
    };

    /// <summary>Published revisions keep their manifest entry, digest, and content; new revisions are only appended.</summary>
    internal static List<string> CheckPublishedImmutability(PublishedState @base, PublishedState proposed)
    {
        if (@base.Revisions is null)
        {
            throw new InvalidOperationException("Cannot read properties of undefined (reading 'forEach')");
        }

        var errors = new List<string>();
        for (var index = 0; index < @base.Revisions.Count; index++)
        {
            var published = @base.Revisions[index];
            var name = Catalog.Js(published.CatalogRevision);
            var now = proposed.Revisions is not null && index < proposed.Revisions.Count ? proposed.Revisions[index] : null;
            if (now is null || !StrictEquals(now.CatalogRevision, published.CatalogRevision))
            {
                errors.Add($"published revision '{name}' was removed or reordered in the manifest");
                continue;
            }

            if (!StrictEquals(now.File, published.File) || !StrictEquals(now.Sha256, published.Sha256))
            {
                errors.Add($"published revision '{name}' manifest entry was modified");
            }

            var file = Catalog.Js(published.File);
            var baseContent = Canonical(@base.Files.TryGetValue(file, out var baseText) ? baseText : null);
            var proposedContent = Canonical(proposed.Files.TryGetValue(file, out var proposedText) ? proposedText : null);
            if (baseContent is not null && proposedContent != baseContent)
            {
                errors.Add($"published revision file '{file}' was modified; publish a new revision instead");
            }
        }

        return errors;
    }

    // Canonical content, not raw bytes, so line-ending conversion cannot produce a false positive.
    private static string? Canonical(string? text)
    {
        if (text is null)
        {
            return null;
        }

        try
        {
            return CanonicalJson.Serialize(JsonText.Parse(text));
        }
        catch (System.Text.Json.JsonException)
        {
            return $"invalid:{text}";
        }
    }

    private static string DescribeError(Exception error) =>
        error is PolicyCatalogException ? error.Message : $"[policy_validation] {error.Message}";

    /// <summary>Compares a directory's manifest and revision files with the revisions published at <paramref name="baseRef"/>. Returns errors, never throws.</summary>
    internal static (int ComparedRevisions, List<string> Errors) CheckAgainstBaseRef(string catalogDir, IReadOnlyList<ManifestRevision> manifest, string baseRef)
    {
        var dir = Path.GetFullPath(catalogDir);
        try
        {
            var @base = ReadPublishedStateAtRef(dir, baseRef);
            if (@base is null)
            {
                return (0, new List<string>());
            }

            var files = new Dictionary<string, string>(StringComparer.Ordinal);
            foreach (var revision in manifest)
            {
                string text;
                try
                {
                    text = JsonText.DecodeUtf8(File.ReadAllBytes(Path.GetFullPath(Path.Combine(dir, revision.File))));
                }
                catch (Exception)
                {
                    // A published file may not disappear; reported as a modification.
                    text = string.Empty;
                }

                files[revision.File] = text;
            }

            var proposed = new PublishedState(
                manifest.Select(r => new PublishedRevision(new JsonString(r.CatalogRevision), new JsonString(r.File), new JsonString(r.Sha256))).ToList(),
                files);
            return (@base.Revisions!.Count, CheckPublishedImmutability(@base, proposed).Select(message => $"[immutability] {message}").ToList());
        }
        catch (Exception error)
        {
            return (0, new List<string> { DescribeError(error) });
        }
    }

    /// <summary>
    /// Reads the published catalog state at <paramref name="baseRef"/> for the git repository containing
    /// <paramref name="catalogDir"/>; <c>null</c> when the ref has no catalog at that path. Throws when the check
    /// cannot run, so a typo never passes silently.
    /// </summary>
    internal static PublishedState? ReadPublishedStateAtRef(string catalogDir, string baseRef)
    {
        // A ref starting with '-' would be parsed by git as an option.
        if (baseRef.Length == 0 || baseRef.StartsWith('-') || baseRef.Any(c => JsString.IsWhitespace(c) || c == '\0'))
        {
            throw new PolicyCatalogException(PolicyCatalogErrorReason.InvalidCatalog, $"base-ref check: '{baseRef}' is not a valid git ref");
        }

        var dir = RealPath(catalogDir);
        string top;
        try
        {
            top = RealPath(Git(new[] { "rev-parse", "--show-toplevel" }, dir).Trim());
        }
        catch (Exception)
        {
            throw new PolicyCatalogException(PolicyCatalogErrorReason.InvalidCatalog, $"base-ref check: '{catalogDir}' is not inside a git work tree");
        }

        if (!GitSucceeds(new[] { "rev-parse", "--verify", "--quiet", $"{baseRef}^{{commit}}" }, top))
        {
            throw new PolicyCatalogException(PolicyCatalogErrorReason.InvalidCatalog, $"base-ref check: '{baseRef}' does not name a commit");
        }

        var prefix = Relative(top, dir);
        if (prefix is null)
        {
            throw new PolicyCatalogException(PolicyCatalogErrorReason.InvalidCatalog, $"base-ref check: '{catalogDir}' is outside its git work tree");
        }

        string At(string file) => $"{baseRef}:{(prefix.Length == 0 ? string.Empty : prefix + "/")}{file}";
        if (!GitSucceeds(new[] { "cat-file", "-e", At("manifest.json") }, top))
        {
            return null;
        }

        var manifest = JsonText.Parse(Git(new[] { "show", At("manifest.json") }, top));
        List<PublishedRevision>? revisions = null;
        var files = new Dictionary<string, string>(StringComparer.Ordinal);
        var revisionsValue = manifest switch
        {
            JsonObject obj => obj.Get("revisions"),
            JsonNull => throw new InvalidOperationException("Cannot read properties of null (reading 'revisions')"),
            _ => null,
        };
        if (revisionsValue is not null and not JsonNull)
        {
            if (revisionsValue is not JsonArray list)
            {
                throw new InvalidOperationException("manifest.revisions is not iterable");
            }

            revisions = new List<PublishedRevision>();
            foreach (var item in list.Items)
            {
                if (item is not JsonObject record)
                {
                    throw new InvalidOperationException("Cannot read properties of a non-object manifest revision");
                }

                var revision = new PublishedRevision(record.Get("catalogRevision"), record.Get("file"), record.Get("sha256"));
                revisions.Add(revision);
                var file = Catalog.Js(revision.File);
                if (GitSucceeds(new[] { "cat-file", "-e", At(file) }, top))
                {
                    files[file] = Git(new[] { "show", At(file) }, top);
                }
            }
        }

        return new PublishedState(revisions, files);
    }

    /// <summary>Node <c>path.relative(top, dir)</c> with forward slashes, or <c>null</c> when <paramref name="dir"/> is outside <paramref name="top"/>.</summary>
    private static string? Relative(string top, string dir)
    {
        var comparison = OperatingSystem.IsWindows() ? StringComparison.OrdinalIgnoreCase : StringComparison.Ordinal;
        var t = Path.TrimEndingDirectorySeparator(top);
        var d = Path.TrimEndingDirectorySeparator(dir);
        if (string.Equals(t, d, comparison))
        {
            return string.Empty;
        }

        var withSeparator = t.EndsWith(Path.DirectorySeparatorChar) ? t : t + Path.DirectorySeparatorChar;
        if (!d.StartsWith(withSeparator, comparison))
        {
            return null;
        }

        return d.Substring(withSeparator.Length).Replace('\\', '/');
    }

    /// <summary>Node <c>realpathSync.native</c>: absolute, symlinks resolved, Windows 8.3 names expanded. Throws when missing.</summary>
    private static string RealPath(string path)
    {
        var full = Path.GetFullPath(path);
        if (!Directory.Exists(full) && !File.Exists(full))
        {
            throw new DirectoryNotFoundException($"ENOENT: no such file or directory, realpath '{path}'");
        }

        var info = new DirectoryInfo(full);
        var resolved = info.LinkTarget is not null ? info.ResolveLinkTarget(true)?.FullName ?? full : full;
        // Resolve links in ancestors as well.
        var parts = new List<string>();
        var current = new DirectoryInfo(resolved);
        while (current.Parent is not null)
        {
            if (current.LinkTarget is not null && current.ResolveLinkTarget(true) is { } target)
            {
                current = new DirectoryInfo(target.FullName);
                continue;
            }

            parts.Insert(0, current.Name);
            current = current.Parent;
        }

        var result = parts.Count == 0 ? current.FullName : Path.Combine(new[] { current.FullName }.Concat(parts).ToArray());
        return OperatingSystem.IsWindows() ? LongPath(result) : result;
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true, EntryPoint = "GetLongPathNameW")]
    private static extern uint GetLongPathName(string shortPath, StringBuilder longPath, uint bufferLength);

    private static string LongPath(string path)
    {
        var buffer = new StringBuilder(1024);
        var length = GetLongPathName(path, buffer, (uint)buffer.Capacity);
        if (length > buffer.Capacity)
        {
            buffer = new StringBuilder((int)length);
            length = GetLongPathName(path, buffer, (uint)buffer.Capacity);
        }

        return length == 0 ? path : buffer.ToString();
    }

    private static string Git(IReadOnlyList<string> args, string cwd)
    {
        var start = new ProcessStartInfo("git")
        {
            WorkingDirectory = cwd,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            RedirectStandardInput = true,
            UseShellExecute = false,
            CreateNoWindow = true,
            StandardOutputEncoding = new UTF8Encoding(false),
        };
        foreach (var arg in args)
        {
            start.ArgumentList.Add(arg);
        }

        using var process = Process.Start(start) ?? throw new InvalidOperationException("git could not be started");
        process.StandardInput.Close();
        var stderr = process.StandardError.ReadToEndAsync();
        var output = process.StandardOutput.ReadToEnd();
        process.WaitForExit();
        var errorText = stderr.Result;
        if (process.ExitCode != 0)
        {
            throw new InvalidOperationException($"Command failed: git {string.Join(" ", args)}\n{errorText}");
        }

        return output;
    }

    private static bool GitSucceeds(IReadOnlyList<string> args, string cwd)
    {
        try
        {
            Git(args, cwd);
            return true;
        }
        catch (Exception)
        {
            return false;
        }
    }
}
