// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using System.Text.RegularExpressions;
using Microsoft.Mxc.PolicyCatalog.Internal;

namespace Microsoft.Mxc.PolicyCatalog;

/// <summary>
/// Read-only view over one catalog store. Runtime resolution and inspection are separate operations;
/// neither mutates catalog data or any consumer state.
/// </summary>
public sealed class PolicyCatalog
{
    private readonly CatalogStore _store;
    private readonly IHostEnvironment _host;

    /// <summary>Creates a catalog over <paramref name="store"/>.</summary>
    /// <param name="store">The catalog store.</param>
    /// <param name="host">Host facts; defaults to <see cref="SystemHostEnvironment.Instance"/>.</param>
    public PolicyCatalog(CatalogStore store, IHostEnvironment? host = null)
    {
        _store = store ?? throw new ArgumentNullException(nameof(store));
        _host = host ?? SystemHostEnvironment.Instance;
    }

    /// <summary>The store this catalog reads.</summary>
    public CatalogStore Store => _store;

    /// <summary>The installed default revision's schema version and ID (design §5.2).</summary>
    /// <returns>The catalog info.</returns>
    /// <exception cref="PolicyCatalogException">The revision fails integrity or validation.</exception>
    public CatalogInfo GetCatalogInfo()
    {
        var revision = _store.Revision();
        return new CatalogInfo(revision.SchemaVersion, revision.CatalogRevision);
    }

    /// <summary>Metadata for every entry in the installed revision, ordered by entry ID. Never includes a policy body.</summary>
    /// <returns>Entry metadata.</returns>
    /// <exception cref="PolicyCatalogException">The revision fails integrity or validation.</exception>
    public IReadOnlyList<CatalogEntryMetadata> ListCatalogEntries()
    {
        var revision = _store.Revision();
        return revision.Entries
            .OrderBy(entry => entry.EntryId, StringComparer.Ordinal)
            .Select(entry => new CatalogEntryMetadata(
                revision.CatalogRevision,
                entry.EntryId,
                entry.EntryRevision,
                entry.DisplayName,
                entry.Identity.Select(predicate => predicate switch
                {
                    PurlPredicate purl => new CatalogIdentityMetadata("purl") { Value = purl.Value, VersionRange = purl.VersionRange },
                    NamePredicate names => new CatalogIdentityMetadata("invocation-name") { Names = names.Names.ToList() },
                    _ => throw new InvalidOperationException(),
                }).ToList(),
                entry.Variants.Select(variant => new CatalogVariantMetadata(
                    variant.Platform,
                    variant.Architecture,
                    variant.DependencyList.Select(dependency => dependency.EntryId).ToList(),
                    variant.PolicyVersion)).ToList(),
                new CatalogProvenance(entry.Method, entry.SourceRevision)))
            .ToList();
    }

    /// <summary>The composed candidate policy for several tools, or <c>null</c> when no policy can be resolved.</summary>
    /// <param name="tools">Tool inputs, resolved in one pass.</param>
    /// <param name="context">Lookup context; <c>null</c> means all defaults.</param>
    /// <returns>The policy or <c>null</c>.</returns>
    /// <exception cref="PolicyCatalogException">A library failure; never reported as absence.</exception>
    public CatalogSandboxPolicy? ResolveSandboxPolicy(IReadOnlyList<ToolInput> tools, ResolveContext? context = null) =>
        Resolve(tools, context).Policy;

    /// <summary>The composed candidate policy for one tool; exactly a one-element list.</summary>
    /// <param name="tool">The tool input (a string converts implicitly).</param>
    /// <param name="context">Lookup context.</param>
    /// <returns>The policy or <c>null</c>.</returns>
    /// <exception cref="PolicyCatalogException">A library failure.</exception>
    public CatalogSandboxPolicy? ResolveSandboxPolicy(ToolInput tool, ResolveContext? context = null) =>
        Resolve(new[] { tool }, context).Policy;

    /// <summary>
    /// Resolves several tools in one pass and returns the composed candidate policy with attribution and warnings.
    /// <see cref="SandboxConfigResolution.Policy"/> is <c>null</c> for an empty input, when nothing matched, or
    /// when a required symbol is unresolved. The result is a candidate lower bound, not authorization.
    /// </summary>
    /// <param name="tools">Tool inputs.</param>
    /// <param name="context">Lookup context.</param>
    /// <returns>The resolution.</returns>
    /// <exception cref="PolicyCatalogException">A library failure.</exception>
    public SandboxConfigResolution ResolveSandboxPolicyWithDiagnostics(IReadOnlyList<ToolInput> tools, ResolveContext? context = null) =>
        Resolve(tools, context);

    /// <summary>Resolves one tool; exactly a one-element list.</summary>
    /// <param name="tool">The tool input.</param>
    /// <param name="context">Lookup context.</param>
    /// <returns>The resolution.</returns>
    /// <exception cref="PolicyCatalogException">A library failure.</exception>
    public SandboxConfigResolution ResolveSandboxPolicyWithDiagnostics(ToolInput tool, ResolveContext? context = null) =>
        Resolve(new[] { tool }, context);

    private sealed record EntryMatch(Entry Entry, VariantSelection Selection, List<IdentityPredicate> Satisfied);

    private static PolicyCatalogException InvalidContext(string message) => new(PolicyCatalogErrorReason.InvalidContext, message);

    private static ToolInput ToCandidate(ToolInput? input, int index)
    {
        if (input is null)
        {
            throw InvalidContext($"tool input {index} must be a string or a ToolCandidate");
        }

        if (string.IsNullOrEmpty(input.InvocationName))
        {
            throw InvalidContext($"tool input {index}: invocationName must be a non-empty string");
        }

        if (input.InvocationName.IndexOfAny(new[] { '\\', '/' }) >= 0)
        {
            throw InvalidContext($"tool input {index}: invocationName must be a bare name, not a path");
        }

        if (input.PackageUrl is { Length: 0 })
        {
            throw InvalidContext($"tool input {index}: packageUrl must be a non-empty string when present");
        }

        if (input.DetectedVersion is { Length: 0 })
        {
            throw InvalidContext($"tool input {index}: detectedVersion must be a non-empty string when present");
        }

        return input;
    }

    private static string DescribeInput(int index, ToolInput tool) => $"input {index} ('{tool.InvocationName}')";

    /// <summary>Own keys in ECMAScript property order (array-index keys first, ascending).</summary>
    internal static List<string> JsKeyOrder(IEnumerable<string> keys)
    {
        var obj = new JsonObject();
        foreach (var key in keys)
        {
            obj.Set(key, JsonNull.Instance);
        }

        return obj.Keys.ToList();
    }

    private SandboxConfigResolution Resolve(IReadOnlyList<ToolInput> tools, ResolveContext? context)
    {
        ArgumentNullException.ThrowIfNull(tools);
        var ctx = context ?? new ResolveContext();
        var candidates = tools.Select((tool, index) => ToCandidate(tool, index)).ToList();
        ValidateContext(ctx);
        var revision = _store.Revision(ctx.CatalogRevision);
        var platform = ctx.Platform ?? _host.Platform();
        var allowWeak = ctx.AllowWeakIdentityFallback;

        // The native architecture is resolved lazily: a lookup that never needs
        // variant selection must not fail on host detection (design §4.4).
        var architecture = ctx.Architecture;
        string EffectiveArchitecture()
        {
            architecture ??= _host.NativeArchitecture();
            return architecture;
        }

        var warnings = new List<string>();
        var toolRecords = new List<ToolDiagnostics>();
        var selected = new List<ClosureNode>();
        var selectedIds = new HashSet<string>(StringComparer.Ordinal);
        var byId = new Dictionary<string, Entry>(StringComparer.Ordinal);
        foreach (var entry in revision.Entries)
        {
            byId[entry.EntryId] = entry;
        }

        var ordered = revision.Entries.OrderBy(entry => entry.EntryId, StringComparer.Ordinal).ToList();
        for (var inputIndex = 0; inputIndex < candidates.Count; inputIndex++)
        {
            var tool = candidates[inputIndex];
            var matches = MatchTool(ordered, tool, inputIndex, platform, allowWeak, EffectiveArchitecture, warnings);
            toolRecords.Add(new ToolDiagnostics(
                inputIndex,
                matches.Select(match => new ToolMatch(
                    match.Entry.EntryId,
                    match.Entry.EntryRevision,
                    match.Satisfied.Select(predicate => new MatchedIdentity(predicate.Kind, predicate.Strength)).ToList())).ToList()));
            if (matches.Count > 1)
            {
                warnings.Add($"{DescribeInput(inputIndex, tool)} matched {matches.Count} entries ({string.Join(", ", matches.Select(m => m.Entry.EntryId))}); all contribute");
            }

            foreach (var match in matches)
            {
                var closure = Catalog.DependencyClosure(match.Entry, match.Selection, byId, platform, EffectiveArchitecture());
                if (!closure.Ok)
                {
                    throw new PolicyCatalogException(PolicyCatalogErrorReason.InvalidCatalog, $"dependency resolution failed: {closure.Reason} ({closure.Detail})");
                }

                foreach (var node in closure.Nodes!)
                {
                    if (selectedIds.Add(node.Entry.EntryId))
                    {
                        selected.Add(node);
                    }
                }
            }
        }

        var diagnostics = new ResolutionDiagnostics(revision.CatalogRevision, toolRecords, DependencyRecords(selected, byId), warnings);
        if (selected.Count == 0)
        {
            return new SandboxConfigResolution(null, diagnostics);
        }

        if (ctx.Architecture is null)
        {
            warnings.Add($"architecture was not specified; variants were selected for the native system architecture '{EffectiveArchitecture()}'; the tool's architecture was not verified");
        }

        foreach (var node in selected)
        {
            if (!node.Exact)
            {
                warnings.Add($"{node.Entry.EntryId} uses its architecture-neutral {platform} variant; no {EffectiveArchitecture()}-specific variant exists");
            }
        }

        var violation = Catalog.CompositionViolation(selected);
        if (violation is not null)
        {
            throw new PolicyCatalogException(PolicyCatalogErrorReason.CompositionConflict, $"selected entries cannot be composed: {violation}");
        }

        var symbols = ResolveSymbols(selected, ctx, platform, warnings);
        if (symbols is null)
        {
            return new SandboxConfigResolution(null, diagnostics);
        }

        return new SandboxConfigResolution(PolicyConversion.ToPolicy(ComposePolicy(selected, symbols, platform)), diagnostics);
    }

    private static List<EntryMatch> MatchTool(
        List<Entry> ordered,
        ToolInput tool,
        int inputIndex,
        string platform,
        bool allowWeak,
        Func<string> architecture,
        List<string> warnings)
    {
        ParsedPurl? purl = null;
        if (tool.PackageUrl is not null)
        {
            purl = Purl.Parse(tool.PackageUrl)
                ?? throw InvalidContext($"{DescribeInput(inputIndex, tool)}: '{tool.PackageUrl}' is not a valid package URL");
        }

        var invocation = Paths.CaseKey(tool.InvocationName, platform);
        var matches = new List<EntryMatch>();
        var skipped = new List<string>();
        foreach (var entry in ordered)
        {
            var satisfied = entry.Identity.Where(predicate => predicate switch
            {
                PurlPredicate p => purl is not null && Purl.Parse(p.Value)?.Key == purl.Key,
                NamePredicate n => n.Names.Any(name => Paths.CaseKey(name, platform) == invocation),
                _ => false,
            }).ToList();
            if (satisfied.Count == 0)
            {
                continue;
            }

            var strong = satisfied.Any(predicate => predicate.Strength == "strong");
            if (!strong && !allowWeak)
            {
                skipped.Add($"{entry.EntryId} matched only by invocation name and allowWeakIdentityFallback is not enabled");
                continue;
            }

            var selection = Catalog.SelectVariant(entry, platform, architecture());
            if (selection is null)
            {
                skipped.Add($"{entry.EntryId} has no variant for {platform}/{architecture()}");
                continue;
            }

            foreach (var predicate in satisfied)
            {
                if (predicate is not PurlPredicate { VersionRange: { } range })
                {
                    continue;
                }

                var evidence = tool.DetectedVersion ?? purl?.Version;
                if (evidence is null)
                {
                    continue;
                }

                var inRange = VersionRange.Satisfies(evidence, range);
                if (inRange != true)
                {
                    warnings.Add($"{DescribeInput(inputIndex, tool)}: detected version '{evidence}' {(inRange == false ? "is outside" : "could not be compared with")} the reviewed range '{range}' for {entry.EntryId}");
                }
            }

            if (!strong)
            {
                warnings.Add($"{DescribeInput(inputIndex, tool)} matched {entry.EntryId} only by invocation name (weak identity)");
            }

            matches.Add(new EntryMatch(entry, selection, satisfied));
        }

        if (matches.Count == 0)
        {
            warnings.Add($"{DescribeInput(inputIndex, tool)} matched no eligible catalog entry{(skipped.Count > 0 ? $": {string.Join("; ", skipped)}" : string.Empty)}");
        }

        return matches;
    }

    private void ValidateContext(ResolveContext ctx)
    {
        if (ctx.Platform is not null && !CatalogPlatforms.All.Contains(ctx.Platform))
        {
            throw InvalidContext($"ResolveContext.platform '{ctx.Platform}' is unsupported");
        }

        if (ctx.Architecture is not null && !CatalogArchitectures.All.Contains(ctx.Architecture))
        {
            throw InvalidContext($"ResolveContext.architecture '{ctx.Architecture}' is unsupported");
        }

        if (ctx.ProjectRoot is { Length: 0 })
        {
            throw InvalidContext("ResolveContext.projectRoot must be a non-empty string when present");
        }

        if (ctx.Symbols is null)
        {
            return;
        }

        foreach (var name in JsKeyOrder(ctx.Symbols.Keys))
        {
            if (!_store.Contract.Symbols.TryGetValue(name, out var definition))
            {
                throw InvalidContext($"ResolveContext.symbols.{name} is not a catalog symbol");
            }

            if (definition.Source == "context")
            {
                throw InvalidContext($"symbol '{name}' is supplied through ResolveContext.projectRoot, not symbols");
            }

            if (ctx.Symbols[name] is null)
            {
                throw InvalidContext($"ResolveContext.symbols.{name} must be a string");
            }
        }
    }

    private Dictionary<string, string>? ResolveSymbols(List<ClosureNode> nodes, ResolveContext ctx, string platform, List<string> warnings)
    {
        var contract = _store.Contract;
        var values = new Dictionary<string, string>(StringComparer.Ordinal);
        var missing = new Dictionary<string, List<string>>(StringComparer.Ordinal);
        var missingOrder = new List<string>();
        foreach (var node in nodes)
        {
            foreach (var name in Catalog.PolicySymbols(node.Variant))
            {
                if (values.ContainsKey(name))
                {
                    continue;
                }

                var definition = contract.Symbols[name];
                string? value;
                if (definition.Source == "context")
                {
                    value = ctx.ProjectRoot;
                }
                else
                {
                    value = ctx.Symbols is not null && ctx.Symbols.TryGetValue(name, out var supplied) ? supplied : null;
                    // Host-derived symbols describe the current host only.
                    if (value is null && definition.Source == "host" && platform == _host.Platform())
                    {
                        value = _host.Symbol(name);
                    }
                }

                if (value is null)
                {
                    if (!missing.TryGetValue(name, out var ids))
                    {
                        ids = new List<string>();
                        missing[name] = ids;
                        missingOrder.Add(name);
                    }

                    ids.Add(node.Entry.EntryId);
                    continue;
                }

                if (value.Contains("${", StringComparison.Ordinal) || !Paths.IsAbsolutePath(value, platform))
                {
                    throw InvalidContext($"symbol '{name}' must resolve to an absolute {platform} path");
                }

                values[name] = value;
            }
        }

        if (missing.Count > 0)
        {
            foreach (var name in missingOrder.OrderBy(n => n, StringComparer.Ordinal))
            {
                var hint = contract.Symbols[name].Source == "context" ? "ResolveContext.projectRoot" : $"ResolveContext.symbols.{name}";
                warnings.Add($"required symbol '{name}' (needed by {string.Join(", ", missing[name])}) is unresolved; supply {hint}; no policy was returned");
            }

            return null;
        }

        return values;
    }

    private static List<ResolvedDependency> DependencyRecords(List<ClosureNode> nodes, Dictionary<string, Entry> byId)
    {
        var records = new List<ResolvedDependency>();
        foreach (var node in nodes)
        {
            foreach (var dependency in node.Variant.DependencyList)
            {
                var target = byId[dependency.EntryId];
                var record = new ResolvedDependency(target.EntryId, target.EntryRevision, dependency.VersionRange);
                if (!records.Contains(record))
                {
                    records.Add(record);
                }
            }
        }

        records.Sort((a, b) =>
        {
            var byEntry = string.CompareOrdinal(a.EntryId, b.EntryId);
            if (byEntry != 0)
            {
                return Math.Sign(byEntry);
            }

            if (a.EntryRevision != b.EntryRevision)
            {
                return a.EntryRevision < b.EntryRevision ? -1 : 1;
            }

            if (a.RequiredVersionRange is null)
            {
                return b.RequiredVersionRange is null ? 0 : -1;
            }

            return b.RequiredVersionRange is null ? 1 : Math.Sign(string.CompareOrdinal(a.RequiredVersionRange, b.RequiredVersionRange));
        });
        return records;
    }

    private static JsonObject ComposePolicy(List<ClosureNode> nodes, Dictionary<string, string> symbols, string platform)
    {
        var classes = new Dictionary<string, List<string>>(StringComparer.Ordinal);
        foreach (var field in Catalog.ComposableFields)
        {
            var seen = new HashSet<string>(StringComparer.Ordinal);
            var output = new List<string>();
            foreach (var node in nodes)
            {
                foreach (var template in node.Variant.Paths(field))
                {
                    var substituted = Catalog.SymbolPattern.Replace(template, (Match match) => symbols[match.Groups[1].Value]);
                    var resolved = Paths.NormalizePath(substituted, platform);
                    var key = Paths.JoinKey(Paths.PathKeySegments(resolved, platform));
                    if (seen.Add(key))
                    {
                        output.Add(resolved);
                    }
                }
            }

            if (output.Count > 0)
            {
                classes[field] = output;
            }
        }

        var overlap = Catalog.FindCrossClassOverlap(classes, platform);
        if (overlap is not null)
        {
            throw new PolicyCatalogException(PolicyCatalogErrorReason.CompositionConflict, $"resolved paths overlap across access classes: {overlap}");
        }

        var root = nodes[0].Variant.Policy;
        var result = new JsonObject();
        result.Set("version", root.Get("version")!);
        if (nodes.Any(node => node.Variant.Policy.Has("filesystem")))
        {
            var filesystem = new JsonObject();
            foreach (var field in Catalog.ComposableFields)
            {
                if (classes.TryGetValue(field, out var list))
                {
                    filesystem.Set(field, new JsonArray(list.Select(path => (JsonValue)new JsonString(path))));
                }
            }

            result.Set("filesystem", filesystem);
        }

        // compositionViolation guarantees these exist only when exactly one entry is selected.
        foreach (var key in new[] { "network", "ui", "timeoutMs" })
        {
            if (root.Get(key) is { } value)
            {
                result.Set(key, value);
            }
        }

        return result;
    }
}

/// <summary>Convenience functions over the catalog embedded in this package.</summary>
public static class BundledPolicyCatalog
{
    private static readonly Lazy<PolicyCatalog> Default = new(() => new PolicyCatalog(CatalogStore.Bundled()), LazyThreadSafetyMode.ExecutionAndPublication);

    /// <summary>The bundled catalog with the system host environment.</summary>
    public static PolicyCatalog Catalog => Default.Value;

    /// <summary>Composed candidate policy for several tools, from the bundled catalog.</summary>
    /// <param name="tools">Tool inputs.</param>
    /// <param name="context">Lookup context.</param>
    /// <returns>The policy or <c>null</c>.</returns>
    public static CatalogSandboxPolicy? ResolveSandboxPolicy(IReadOnlyList<ToolInput> tools, ResolveContext? context = null) => Catalog.ResolveSandboxPolicy(tools, context);

    /// <summary>Composed candidate policy for one tool, from the bundled catalog.</summary>
    /// <param name="tool">The tool input.</param>
    /// <param name="context">Lookup context.</param>
    /// <returns>The policy or <c>null</c>.</returns>
    public static CatalogSandboxPolicy? ResolveSandboxPolicy(ToolInput tool, ResolveContext? context = null) => Catalog.ResolveSandboxPolicy(tool, context);

    /// <summary>Composed candidate policy plus attribution for several tools.</summary>
    /// <param name="tools">Tool inputs.</param>
    /// <param name="context">Lookup context.</param>
    /// <returns>The resolution.</returns>
    public static SandboxConfigResolution ResolveSandboxPolicyWithDiagnostics(IReadOnlyList<ToolInput> tools, ResolveContext? context = null) => Catalog.ResolveSandboxPolicyWithDiagnostics(tools, context);

    /// <summary>Composed candidate policy plus attribution for one tool.</summary>
    /// <param name="tool">The tool input.</param>
    /// <param name="context">Lookup context.</param>
    /// <returns>The resolution.</returns>
    public static SandboxConfigResolution ResolveSandboxPolicyWithDiagnostics(ToolInput tool, ResolveContext? context = null) => Catalog.ResolveSandboxPolicyWithDiagnostics(tool, context);

    /// <summary>Bundled catalog entry metadata.</summary>
    /// <returns>Entry metadata ordered by entry ID.</returns>
    public static IReadOnlyList<CatalogEntryMetadata> ListCatalogEntries() => Catalog.ListCatalogEntries();

    /// <summary>The bundled catalog schema version and revision.</summary>
    /// <returns>The catalog info.</returns>
    public static CatalogInfo GetCatalogInfo() => Catalog.GetCatalogInfo();
}
