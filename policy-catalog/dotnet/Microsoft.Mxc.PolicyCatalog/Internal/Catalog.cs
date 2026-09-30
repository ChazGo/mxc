// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using System.Globalization;
using System.Text.RegularExpressions;

namespace Microsoft.Mxc.PolicyCatalog.Internal;

internal sealed record SymbolDefinition(string Source, string Description);

internal sealed class CatalogContract
{
    public CatalogContract(string schemaVersion, IReadOnlyList<string> policyVersions, IReadOnlyDictionary<string, SymbolDefinition> symbols)
    {
        SchemaVersion = schemaVersion;
        PolicyVersions = policyVersions;
        Symbols = symbols;
    }

    public string SchemaVersion { get; }

    public IReadOnlyList<string> PolicyVersions { get; }

    /// <summary>Own-key symbol table; lookups never see inherited names.</summary>
    public IReadOnlyDictionary<string, SymbolDefinition> Symbols { get; }
}

internal abstract record IdentityPredicate(string Kind)
{
    public string Strength => Kind == "purl" ? "strong" : "weak";
}

internal sealed record PurlPredicate(string Value, string? VersionRange) : IdentityPredicate("purl");

internal sealed record NamePredicate(IReadOnlyList<string> Names) : IdentityPredicate("invocation-name");

internal sealed record Dependency(string EntryId, string? VersionRange);

internal sealed class Variant
{
    public Variant(string platform, string? architecture, IReadOnlyList<Dependency>? dependencies, JsonObject policy)
    {
        Platform = platform;
        Architecture = architecture;
        Dependencies = dependencies;
        Policy = policy;
    }

    public string Platform { get; }

    public string? Architecture { get; }

    public IReadOnlyList<Dependency>? Dependencies { get; }

    /// <summary>The validated raw policy object (only contract fields).</summary>
    public JsonObject Policy { get; }

    public string PolicyVersion => ((JsonString)Policy.Get("version")!).Value;

    public IReadOnlyList<Dependency> DependencyList => Dependencies ?? Array.Empty<Dependency>();

    public List<string> Paths(string field)
    {
        if (Policy.Get("filesystem") is JsonObject filesystem && filesystem.Get(field) is JsonArray array)
        {
            return array.Items.Select(item => ((JsonString)item).Value).ToList();
        }

        return new List<string>();
    }
}

internal sealed class Entry
{
    public Entry(string entryId, double entryRevision, string displayName, IReadOnlyList<IdentityPredicate> identity, IReadOnlyList<Variant> variants, string method, string sourceRevision, JsonObject raw)
    {
        EntryId = entryId;
        EntryRevision = entryRevision;
        DisplayName = displayName;
        Identity = identity;
        Variants = variants;
        Method = method;
        SourceRevision = sourceRevision;
        Raw = raw;
    }

    public string EntryId { get; }

    public double EntryRevision { get; }

    public string DisplayName { get; }

    public IReadOnlyList<IdentityPredicate> Identity { get; }

    public IReadOnlyList<Variant> Variants { get; }

    public string Method { get; }

    public string SourceRevision { get; }

    public JsonObject Raw { get; }
}

internal sealed class CatalogRevisionData
{
    public CatalogRevisionData(string schemaVersion, string catalogRevision, IReadOnlyList<Entry> entries)
    {
        SchemaVersion = schemaVersion;
        CatalogRevision = catalogRevision;
        Entries = entries;
    }

    public string SchemaVersion { get; }

    public string CatalogRevision { get; }

    public IReadOnlyList<Entry> Entries { get; }
}

internal sealed record VariantSelection(Variant Variant, bool Exact);

internal sealed record ClosureNode(Entry Entry, Variant Variant, bool Exact);

internal sealed record ClosureResult(List<ClosureNode>? Nodes, string? Reason, string? Detail)
{
    public bool Ok => Nodes is not null;
}

/// <summary>Port of TypeScript <c>catalog.ts</c>.</summary>
internal static class Catalog
{
    public const string SchemaVersion = "1";

    public static readonly string[] ComposableFields = { "deniedPaths", "readonlyPaths", "readwritePaths" };

    private static readonly string[] BackendKeys = { "containment", "processContainer", "appContainer", "lxc", "seatbelt", "wslc", "hyperlight", "bwrap" };

    private static readonly Regex RevisionPattern = new("^([0-9]{4}-[0-9]{2}-[0-9]{2})\\.([1-9][0-9]*)\\z", RegexOptions.CultureInvariant);
    private static readonly Regex EntryIdPattern = new("^[a-z][a-z0-9-]*:[a-z0-9][a-z0-9._-]*\\z", RegexOptions.CultureInvariant);
    private static readonly Regex SymbolNamePattern = new("^[a-z][a-z0-9_]*\\z", RegexOptions.CultureInvariant);
    public static readonly Regex SymbolPattern = new("\\$\\{([a-z][a-z0-9_]*)\\}", RegexOptions.CultureInvariant);
    private static readonly Regex AnchoredSymbol = new("^\\$\\{([a-z][a-z0-9_]*)\\}(?:[\\\\/]|\\z)", RegexOptions.CultureInvariant);
    private static readonly Regex PathSplit = new("[\\\\/]", RegexOptions.CultureInvariant);
    private static readonly Regex CidrWildcard = new("/0\\z", RegexOptions.CultureInvariant);

    public static PolicyCatalogException Fail(string message) => new(PolicyCatalogErrorReason.InvalidCatalog, message);

    private static bool IsRecord(JsonValue? value) => value is JsonObject;

    private static void OnlyFields(JsonObject value, IReadOnlyCollection<string> allowed, string at)
    {
        foreach (var key in value.Keys)
        {
            if (!allowed.Contains(key))
            {
                throw Fail($"unsupported field '{at}.{key}'");
            }
        }
    }

    private static string NonEmptyString(JsonValue? value, string at)
    {
        if (value is not JsonString s || s.Value.Length == 0)
        {
            throw Fail($"'{at}' must be a non-empty string");
        }

        return s.Value;
    }

    private static List<string> StringArray(JsonValue? value, string at, int minItems = 0)
    {
        if (value is not JsonArray array || array.Count < minItems)
        {
            throw Fail($"'{at}' must be an array with at least {minItems} item(s)");
        }

        return array.Items.Select((item, index) => NonEmptyString(item, $"{at}[{index}]")).ToList();
    }

    private static bool IsString(JsonValue? value, string expected) => value is JsonString s && s.Value == expected;

    private static bool IsInteger(JsonValue? value, out double number)
    {
        number = value is JsonNumber n ? n.Value : double.NaN;
        return value is JsonNumber && double.IsFinite(number) && Math.Floor(number) == number;
    }

    /// <summary>Formats a raw value the way a JavaScript template literal would for the values that reach messages.</summary>
    internal static string Js(JsonValue? value) => value switch
    {
        null => "undefined",
        JsonString s => s.Value,
        JsonNumber n => JsNumber.ToString(n.Value),
        JsonBool b => b.Value ? "true" : "false",
        JsonNull => "null",
        JsonArray a => string.Join(",", a.Items.Select(item => item is JsonNull ? string.Empty : Js(item))),
        _ => "[object Object]",
    };

    public static int CompareRevisions(string left, string right)
    {
        var a = RevisionPattern.Match(left);
        var b = RevisionPattern.Match(right);
        if (!a.Success || !b.Success)
        {
            throw Fail($"cannot compare malformed catalog revisions '{left}' and '{right}'");
        }

        if (a.Groups[1].Value != b.Groups[1].Value)
        {
            return string.CompareOrdinal(a.Groups[1].Value, b.Groups[1].Value) < 0 ? -1 : 1;
        }

        var x = double.Parse(a.Groups[2].Value, NumberStyles.None, CultureInfo.InvariantCulture);
        var y = double.Parse(b.Groups[2].Value, NumberStyles.None, CultureInfo.InvariantCulture);
        return Math.Sign(x - y);
    }

    public static bool IsRevisionId(string value) => RevisionPattern.IsMatch(value);

    // -----------------------------------------------------------------------
    // Contract
    // -----------------------------------------------------------------------

    public static CatalogContract ValidateContract(JsonValue raw)
    {
        if (raw is not JsonObject obj)
        {
            throw Fail("contract root must be an object");
        }

        OnlyFields(obj, new[] { "$comment", "catalogSchemaVersion", "sandboxPolicyVersions", "symbols" }, "contract");
        if (!IsString(obj.Get("catalogSchemaVersion"), SchemaVersion))
        {
            throw Fail($"contract.catalogSchemaVersion must be '{SchemaVersion}'");
        }

        var versions = StringArray(obj.Get("sandboxPolicyVersions"), "contract.sandboxPolicyVersions", 1);
        if (obj.Get("symbols") is not JsonObject symbolsRaw)
        {
            throw Fail("contract.symbols must be an object");
        }

        var symbols = new Dictionary<string, SymbolDefinition>(StringComparer.Ordinal);
        foreach (var name in symbolsRaw.Keys)
        {
            var definition = symbolsRaw.Get(name);
            if (!SymbolNamePattern.IsMatch(name) || definition is not JsonObject def)
            {
                throw Fail($"contract.symbols.{name} is malformed");
            }

            OnlyFields(def, new[] { "source", "description" }, $"contract.symbols.{name}");
            var source = def.Get("source");
            if (!IsString(source, "context") && !IsString(source, "caller") && !IsString(source, "host"))
            {
                throw Fail($"contract.symbols.{name}.source is unsupported");
            }

            symbols[name] = new SymbolDefinition(((JsonString)source!).Value, NonEmptyString(def.Get("description"), $"contract.symbols.{name}.description"));
        }

        return new CatalogContract(SchemaVersion, versions, symbols);
    }

    // -----------------------------------------------------------------------
    // Paths and symbols
    // -----------------------------------------------------------------------

    private static void ValidateTemplatePath(string value, string at, CatalogContract contract)
    {
        if (value.IndexOfAny(new[] { '*', '?' }) >= 0)
        {
            throw Fail($"'{at}' contains a wildcard");
        }

        if (SymbolPattern.Replace(value, string.Empty).Contains("${", StringComparison.Ordinal))
        {
            throw Fail($"'{at}' contains malformed symbol syntax");
        }

        foreach (Match match in SymbolPattern.Matches(value))
        {
            if (!contract.Symbols.ContainsKey(match.Groups[1].Value))
            {
                throw Fail($"'{at}' references unknown symbol '{match.Groups[1].Value}'");
            }
        }

        if (!AnchoredSymbol.IsMatch(value))
        {
            throw Fail($"'{at}' must start with a declared symbol; literal paths are not allowed");
        }

        if (PathSplit.Split(value).Any(segment => segment == ".."))
        {
            throw Fail($"'{at}' must not contain '..' segments");
        }
    }

    /// <summary>Symbols a policy references, in first-seen order.</summary>
    public static List<string> PolicySymbols(Variant variant)
    {
        var seen = new List<string>();
        foreach (var field in ComposableFields)
        {
            foreach (var value in variant.Paths(field))
            {
                foreach (Match match in SymbolPattern.Matches(value))
                {
                    if (!seen.Contains(match.Groups[1].Value))
                    {
                        seen.Add(match.Groups[1].Value);
                    }
                }
            }
        }

        return seen;
    }

    private static bool IsSameOrNested(List<string> left, List<string> right)
    {
        var shorter = left.Count <= right.Count ? left : right;
        var longer = ReferenceEquals(shorter, left) ? right : left;
        for (var i = 0; i < shorter.Count; i++)
        {
            if (longer[i] != shorter[i])
            {
                return false;
            }
        }

        return true;
    }

    /// <summary>First equal or ancestor/descendant pair across access classes, or <c>null</c>.</summary>
    public static string? FindCrossClassOverlap(IReadOnlyDictionary<string, List<string>> classes, string platform)
    {
        var fields = ComposableFields.Where(field => classes.TryGetValue(field, out var list) && list.Count > 0).ToList();
        for (var i = 0; i < fields.Count; i++)
        {
            for (var j = i + 1; j < fields.Count; j++)
            {
                foreach (var left in classes[fields[i]])
                {
                    foreach (var right in classes[fields[j]])
                    {
                        if (IsSameOrNested(Paths.PathKeySegments(left, platform), Paths.PathKeySegments(right, platform)))
                        {
                            return $"'{left}' ({fields[i]}) overlaps '{right}' ({fields[j]})";
                        }
                    }
                }
            }
        }

        return null;
    }

    // -----------------------------------------------------------------------
    // Embedded SandboxPolicy
    // -----------------------------------------------------------------------

    private static void ValidateNetworkRules(JsonValue value, string at, bool denyList)
    {
        if (value is not JsonArray rules)
        {
            throw Fail($"'{at}' must be an array");
        }

        for (var index = 0; index < rules.Count; index++)
        {
            var ruleAt = $"{at}[{index}]";
            if (rules[index] is not JsonObject rule)
            {
                throw Fail($"'{ruleAt}' must be an object");
            }

            OnlyFields(rule, new[] { "to", "ports" }, ruleAt);
            var to = rule.Get("to");
            if (to is null && !denyList)
            {
                throw Fail($"'{ruleAt}' has no 'to'; wildcard network grants are not allowed");
            }

            if (to is not null)
            {
                if (to is not JsonArray peers || peers.Count == 0)
                {
                    throw Fail($"'{ruleAt}.to' must be a non-empty array");
                }

                for (var peerIndex = 0; peerIndex < peers.Count; peerIndex++)
                {
                    var peerAt = $"{ruleAt}.to[{peerIndex}]";
                    if (peers[peerIndex] is not JsonObject peer)
                    {
                        throw Fail($"'{peerAt}' must be an object");
                    }

                    OnlyFields(peer, new[] { "cidr", "except" }, peerAt);
                    var cidr = NonEmptyString(peer.Get("cidr"), $"{peerAt}.cidr");
                    if (!denyList && CidrWildcard.IsMatch(cidr))
                    {
                        throw Fail($"'{peerAt}.cidr' is a wildcard network grant");
                    }

                    if (peer.Get("except") is { } except)
                    {
                        StringArray(except, $"{peerAt}.except");
                    }
                }
            }

            var portsValue = rule.Get("ports");
            if (portsValue is not null)
            {
                if (portsValue is not JsonArray ports || ports.Count == 0)
                {
                    throw Fail($"'{ruleAt}.ports' must be a non-empty array");
                }

                for (var portIndex = 0; portIndex < ports.Count; portIndex++)
                {
                    var portAt = $"{ruleAt}.ports[{portIndex}]";
                    if (ports[portIndex] is not JsonObject port)
                    {
                        throw Fail($"'{portAt}' must be an object");
                    }

                    OnlyFields(port, new[] { "protocol", "port", "endPort" }, portAt);
                    var protocol = port.Get("protocol");
                    if (protocol is not null && !(protocol is JsonString p && (p.Value is "tcp" or "udp" or "icmp" or "any")))
                    {
                        throw Fail($"'{portAt}.protocol' is unsupported");
                    }

                    foreach (var key in new[] { "port", "endPort" })
                    {
                        var n = port.Get(key);
                        if (n is not null && (!IsInteger(n, out var number) || number < 1 || number > 65535))
                        {
                            throw Fail($"'{portAt}.{key}' must be an integer in 1..65535");
                        }
                    }

                    if (port.Get("endPort") is JsonNumber endPort
                        && (port.Get("port") is not JsonNumber start || endPort.Value < start.Value))
                    {
                        throw Fail($"'{portAt}.endPort' requires a lower or equal 'port'");
                    }
                }
            }
        }
    }

    private static JsonObject ValidateSandboxPolicy(JsonValue? raw, string at, CatalogContract contract)
    {
        if (raw is not JsonObject policy)
        {
            throw Fail($"'{at}' must be an object");
        }

        foreach (var key in BackendKeys)
        {
            if (policy.Has(key))
            {
                throw Fail($"'{at}.{key}' names a containment backend; platform variants must stay backend-neutral");
            }
        }

        OnlyFields(policy, new[] { "version", "filesystem", "network", "ui", "timeoutMs" }, at);
        var version = NonEmptyString(policy.Get("version"), $"{at}.version");
        if (!contract.PolicyVersions.Contains(version))
        {
            throw Fail($"'{at}.version' '{version}' is not a SandboxPolicy version registered in the catalog contract");
        }

        var filesystemValue = policy.Get("filesystem");
        if (filesystemValue is not null)
        {
            if (filesystemValue is not JsonObject filesystem)
            {
                throw Fail($"'{at}.filesystem' must be an object");
            }

            OnlyFields(filesystem, ComposableFields, $"{at}.filesystem");
            foreach (var field in ComposableFields)
            {
                var list = filesystem.Get(field);
                if (list is not null)
                {
                    var values = StringArray(list, $"{at}.filesystem.{field}");
                    for (var index = 0; index < values.Count; index++)
                    {
                        ValidateTemplatePath(values[index], $"{at}.filesystem.{field}[{index}]", contract);
                    }
                }
            }
        }

        var networkValue = policy.Get("network");
        if (networkValue is not null)
        {
            if (networkValue is not JsonObject network)
            {
                throw Fail($"'{at}.network' must be an object");
            }

            OnlyFields(network, new[] { "egress", "ingress" }, $"{at}.network");
            var egressValue = network.Get("egress");
            if (egressValue is not null)
            {
                if (egressValue is not JsonObject egress)
                {
                    throw Fail($"'{at}.network.egress' must be an object");
                }

                OnlyFields(egress, new[] { "default", "allow", "deny" }, $"{at}.network.egress");
                if (egress.Get("default") is { } egressDefault && !IsString(egressDefault, "deny"))
                {
                    throw Fail($"'{at}.network.egress.default' must be 'deny'; a default-allow grant is a wildcard");
                }

                if (egress.Get("allow") is { } allow)
                {
                    ValidateNetworkRules(allow, $"{at}.network.egress.allow", false);
                }

                if (egress.Get("deny") is { } deny)
                {
                    ValidateNetworkRules(deny, $"{at}.network.egress.deny", true);
                }
            }

            var ingressValue = network.Get("ingress");
            if (ingressValue is not null)
            {
                if (ingressValue is not JsonObject ingress)
                {
                    throw Fail($"'{at}.network.ingress' must be an object");
                }

                OnlyFields(ingress, new[] { "default", "hostLoopback" }, $"{at}.network.ingress");
                if (ingress.Get("default") is { } ingressDefault && !IsString(ingressDefault, "deny"))
                {
                    throw Fail($"'{at}.network.ingress.default' must be 'deny'; a default-allow grant is a wildcard");
                }

                if (ingress.Get("hostLoopback") is { } loopback && !IsString(loopback, "allow") && !IsString(loopback, "deny"))
                {
                    throw Fail($"'{at}.network.ingress.hostLoopback' is unsupported");
                }
            }
        }

        var uiValue = policy.Get("ui");
        if (uiValue is not null)
        {
            if (uiValue is not JsonObject ui)
            {
                throw Fail($"'{at}.ui' must be an object");
            }

            OnlyFields(ui, new[] { "allowWindows", "clipboard", "allowInputInjection" }, $"{at}.ui");
            foreach (var key in new[] { "allowWindows", "allowInputInjection" })
            {
                if (ui.Get(key) is { } flag && flag is not JsonBool)
                {
                    throw Fail($"'{at}.ui.{key}' must be a boolean");
                }
            }

            if (ui.Get("clipboard") is { } clipboard && !(clipboard is JsonString c && (c.Value is "none" or "read" or "write" or "all")))
            {
                throw Fail($"'{at}.ui.clipboard' is unsupported");
            }
        }

        var timeout = policy.Get("timeoutMs");
        if (timeout is not null && (!IsInteger(timeout, out var ms) || ms < 1))
        {
            throw Fail($"'{at}.timeoutMs' must be a positive integer");
        }

        return policy;
    }

    // -----------------------------------------------------------------------
    // Entries
    // -----------------------------------------------------------------------

    private static List<IdentityPredicate> ValidateIdentity(JsonValue? raw, string at)
    {
        if (raw is not JsonArray items || items.Count == 0)
        {
            throw Fail($"'{at}' must be a non-empty array");
        }

        var predicates = new List<IdentityPredicate>();
        for (var index = 0; index < items.Count; index++)
        {
            var itemAt = $"{at}[{index}]";
            if (items[index] is not JsonObject item)
            {
                throw Fail($"'{itemAt}' must be an object");
            }

            var kind = item.Get("kind");
            if (IsString(kind, "purl"))
            {
                OnlyFields(item, new[] { "kind", "value", "versionRange" }, itemAt);
                var value = NonEmptyString(item.Get("value"), $"{itemAt}.value");
                var parsed = Purl.Parse(value) ?? throw Fail($"'{itemAt}.value' is not a valid package URL");
                if (parsed.Version is not null)
                {
                    throw Fail($"'{itemAt}.value' must not pin a version; use 'versionRange'");
                }

                var rangeValue = item.Get("versionRange");
                string? range = null;
                if (rangeValue is not null)
                {
                    range = NonEmptyString(rangeValue, $"{itemAt}.versionRange");
                    if (!VersionRange.IsValid(range))
                    {
                        throw Fail($"'{itemAt}.versionRange' is not a valid version range");
                    }
                }

                predicates.Add(new PurlPredicate(value, range));
                continue;
            }

            if (IsString(kind, "invocation-name"))
            {
                OnlyFields(item, new[] { "kind", "names" }, itemAt);
                var names = StringArray(item.Get("names"), $"{itemAt}.names", 1);
                foreach (var name in names)
                {
                    if (name.IndexOfAny(new[] { '\\', '/' }) >= 0)
                    {
                        throw Fail($"'{itemAt}.names' entry '{name}' must be a bare invocation name, not a path");
                    }
                }

                predicates.Add(new NamePredicate(names));
                continue;
            }

            throw Fail($"'{itemAt}.kind' is not a supported identity kind");
        }

        var seen = new HashSet<string>(StringComparer.Ordinal);
        foreach (var key in predicates.SelectMany(IdentityKeys))
        {
            if (!seen.Add(key))
            {
                throw Fail($"'{at}' repeats identity '{key}'");
            }
        }

        return predicates;
    }

    /// <summary>Stable comparison keys for one predicate; invocation names always fold case.</summary>
    private static IEnumerable<string> IdentityKeys(IdentityPredicate predicate) => predicate switch
    {
        PurlPredicate purl => new[] { $"purl:{Purl.Parse(purl.Value)!.Key}" },
        NamePredicate names => names.Names.Select(name => $"invocation-name:{JsString.ToLower(name)}"),
        _ => Array.Empty<string>(),
    };

    private static List<Variant> ValidateVariants(JsonValue? raw, string at, CatalogContract contract)
    {
        if (raw is not JsonArray items || items.Count == 0)
        {
            throw Fail($"'{at}' must be a non-empty array");
        }

        var selectors = new HashSet<string>(StringComparer.Ordinal);
        var variants = new List<Variant>();
        for (var index = 0; index < items.Count; index++)
        {
            var itemAt = $"{at}[{index}]";
            if (items[index] is not JsonObject item)
            {
                throw Fail($"'{itemAt}' must be an object");
            }

            OnlyFields(item, new[] { "when", "dependencies", "sandboxPolicy" }, itemAt);
            if (item.Get("when") is not JsonObject when)
            {
                throw Fail($"'{itemAt}.when' must be an object");
            }

            OnlyFields(when, new[] { "platform", "architecture" }, $"{itemAt}.when");
            if (when.Get("platform") is not JsonString platformValue || !CatalogPlatforms.All.Contains(platformValue.Value))
            {
                throw Fail($"'{itemAt}.when.platform' must be one of {string.Join(", ", CatalogPlatforms.All)}");
            }

            var platform = platformValue.Value;
            var architectureValue = when.Get("architecture");
            string? architecture = null;
            if (architectureValue is not null)
            {
                if (architectureValue is not JsonString a || !CatalogArchitectures.All.Contains(a.Value))
                {
                    throw Fail($"'{itemAt}.when.architecture' must be one of {string.Join(", ", CatalogArchitectures.All)}");
                }

                architecture = a.Value;
            }

            var selector = $"{platform}/{architecture ?? "*"}";
            if (!selectors.Add(selector))
            {
                throw Fail(architecture is null
                    ? $"'{itemAt}' is a second architecture-neutral variant for '{platform}'"
                    : $"'{itemAt}' duplicates selector '{selector}'");
            }

            List<Dependency>? dependencies = null;
            var dependenciesValue = item.Get("dependencies");
            if (dependenciesValue is not null)
            {
                if (dependenciesValue is not JsonArray list)
                {
                    throw Fail($"'{itemAt}.dependencies' must be an array");
                }

                var seen = new HashSet<string>(StringComparer.Ordinal);
                dependencies = new List<Dependency>();
                for (var depIndex = 0; depIndex < list.Count; depIndex++)
                {
                    var depAt = $"{itemAt}.dependencies[{depIndex}]";
                    if (list[depIndex] is not JsonObject dependency)
                    {
                        throw Fail($"'{depAt}' must be an object");
                    }

                    OnlyFields(dependency, new[] { "entryId", "versionRange" }, depAt);
                    var entryId = NonEmptyString(dependency.Get("entryId"), $"{depAt}.entryId");
                    if (!seen.Add(entryId))
                    {
                        throw Fail($"'{depAt}.entryId' '{entryId}' is listed twice");
                    }

                    string? range = null;
                    if (dependency.Get("versionRange") is { } rangeValue)
                    {
                        range = NonEmptyString(rangeValue, $"{depAt}.versionRange");
                        if (!VersionRange.IsValid(range))
                        {
                            throw Fail($"'{depAt}.versionRange' is not a valid version range");
                        }
                    }

                    dependencies.Add(new Dependency(entryId, range));
                }
            }

            var policy = ValidateSandboxPolicy(item.Get("sandboxPolicy"), $"{itemAt}.sandboxPolicy", contract);
            variants.Add(new Variant(platform, architecture, dependencies, policy));
        }

        return variants;
    }

    private static Entry ValidateEntry(JsonValue raw, string at, CatalogContract contract)
    {
        if (raw is not JsonObject entry)
        {
            throw Fail($"'{at}' must be an object");
        }

        OnlyFields(entry, new[] { "entryId", "entryRevision", "displayName", "identity", "platformVariants", "provenance" }, at);
        var entryId = NonEmptyString(entry.Get("entryId"), $"{at}.entryId");
        if (!EntryIdPattern.IsMatch(entryId))
        {
            throw Fail($"'{at}.entryId' '{entryId}' must be namespaced, e.g. 'tool:name'");
        }

        if (!IsInteger(entry.Get("entryRevision"), out var entryRevision) || entryRevision < 1)
        {
            throw Fail($"'{at}.entryRevision' must be a positive integer");
        }

        if (entry.Get("provenance") is not JsonObject provenance)
        {
            throw Fail($"'{at}.provenance' must be an object");
        }

        OnlyFields(provenance, new[] { "method", "sourceRevision" }, $"{at}.provenance");
        var displayName = NonEmptyString(entry.Get("displayName"), $"{at}.displayName");
        var identity = ValidateIdentity(entry.Get("identity"), $"{at}.identity");
        var variants = ValidateVariants(entry.Get("platformVariants"), $"{at}.platformVariants", contract);
        var method = NonEmptyString(provenance.Get("method"), $"{at}.provenance.method");
        var sourceRevision = NonEmptyString(provenance.Get("sourceRevision"), $"{at}.provenance.sourceRevision");
        return new Entry(entryId, entryRevision, displayName, identity, variants, method, sourceRevision, entry);
    }

    // -----------------------------------------------------------------------
    // Variant selection and dependency closure
    // -----------------------------------------------------------------------

    public static VariantSelection? SelectVariant(Entry entry, string platform, string architecture)
    {
        var forPlatform = entry.Variants.Where(variant => variant.Platform == platform).ToList();
        var exact = forPlatform.FirstOrDefault(variant => variant.Architecture == architecture);
        if (exact is not null)
        {
            return new VariantSelection(exact, true);
        }

        var neutral = forPlatform.FirstOrDefault(variant => variant.Architecture is null);
        return neutral is null ? null : new VariantSelection(neutral, false);
    }

    /// <summary>Depth-first closure: root first, dependencies in declaration order, each once; cycles reported.</summary>
    public static ClosureResult DependencyClosure(Entry root, VariantSelection rootSelection, IReadOnlyDictionary<string, Entry> byId, string platform, string architecture)
    {
        var nodes = new List<ClosureNode>();
        var done = new HashSet<string>(StringComparer.Ordinal);
        var stack = new List<string>();
        ClosureResult? failure = null;

        void Visit(Entry entry, VariantSelection selection)
        {
            if (failure is not null)
            {
                return;
            }

            if (stack.Contains(entry.EntryId))
            {
                failure = new ClosureResult(null, "cycle", string.Join(" -> ", stack.Append(entry.EntryId)));
                return;
            }

            if (done.Contains(entry.EntryId))
            {
                return;
            }

            stack.Add(entry.EntryId);
            nodes.Add(new ClosureNode(entry, selection.Variant, selection.Exact));
            foreach (var dependency in selection.Variant.DependencyList)
            {
                if (!byId.TryGetValue(dependency.EntryId, out var target))
                {
                    failure = new ClosureResult(null, "missing-entry", $"{entry.EntryId} -> {dependency.EntryId}");
                    return;
                }

                var selected = SelectVariant(target, platform, architecture);
                if (selected is null)
                {
                    failure = new ClosureResult(null, "unsupported-dependency", $"{entry.EntryId} -> {dependency.EntryId} has no {platform}/{architecture} variant");
                    return;
                }

                Visit(target, selected);
                if (failure is not null)
                {
                    return;
                }
            }

            stack.RemoveAt(stack.Count - 1);
            done.Add(entry.EntryId);
        }

        Visit(root, rootSelection);
        return failure ?? new ClosureResult(nodes, null, null);
    }

    /// <summary>v1 composition limits (design §4.5); a violation description or <c>null</c>.</summary>
    public static string? CompositionViolation(IReadOnlyList<ClosureNode> nodes)
    {
        var versions = new List<string>();
        foreach (var node in nodes)
        {
            if (!versions.Contains(node.Variant.PolicyVersion))
            {
                versions.Add(node.Variant.PolicyVersion);
            }
        }

        if (versions.Count > 1)
        {
            versions.Sort(string.CompareOrdinal);
            return $"mixed sandboxPolicy.version values ({string.Join(", ", versions)})";
        }

        if (nodes.Count < 2)
        {
            return null;
        }

        foreach (var node in nodes)
        {
            foreach (var key in node.Variant.Policy.Keys)
            {
                if (key != "version" && key != "filesystem")
                {
                    return $"'{node.Entry.EntryId}' uses '{key}', which has no v1 cross-entry composition rule";
                }
            }
        }

        return null;
    }

    // -----------------------------------------------------------------------
    // Revision-level validation
    // -----------------------------------------------------------------------

    public static CatalogRevisionData ValidateRevision(JsonValue raw, CatalogContract contract)
    {
        if (raw is not JsonObject obj)
        {
            throw Fail("catalog root must be an object");
        }

        OnlyFields(obj, new[] { "catalogSchemaVersion", "catalogRevision", "entries" }, "catalog");
        if (!IsString(obj.Get("catalogSchemaVersion"), contract.SchemaVersion))
        {
            throw Fail($"catalog.catalogSchemaVersion must be '{contract.SchemaVersion}'");
        }

        var catalogRevision = NonEmptyString(obj.Get("catalogRevision"), "catalog.catalogRevision");
        if (!IsRevisionId(catalogRevision))
        {
            throw Fail($"catalog.catalogRevision '{catalogRevision}' must match YYYY-MM-DD.N");
        }

        if (obj.Get("entries") is not JsonArray rawEntries)
        {
            throw Fail("catalog.entries must be an array");
        }

        var entries = rawEntries.Items.Select((entry, index) => ValidateEntry(entry, $"entries[{index}]", contract)).ToList();
        var byId = new Dictionary<string, Entry>(StringComparer.Ordinal);
        foreach (var entry in entries)
        {
            if (byId.ContainsKey(entry.EntryId))
            {
                throw Fail($"duplicate entryId '{entry.EntryId}'");
            }

            byId[entry.EntryId] = entry;
        }

        foreach (var entry in entries)
        {
            foreach (var variant in entry.Variants)
            {
                foreach (var dependency in variant.DependencyList)
                {
                    if (!byId.ContainsKey(dependency.EntryId))
                    {
                        throw Fail($"'{entry.EntryId}' depends on unknown entry '{dependency.EntryId}' in this revision");
                    }

                    if (dependency.EntryId == entry.EntryId)
                    {
                        throw Fail($"'{entry.EntryId}' depends on itself");
                    }
                }
            }

            foreach (var platform in CatalogPlatforms.All)
            {
                foreach (var architecture in CatalogArchitectures.All)
                {
                    var selected = SelectVariant(entry, platform, architecture);
                    if (selected is null)
                    {
                        continue;
                    }

                    var closure = DependencyClosure(entry, selected, byId, platform, architecture);
                    if (!closure.Ok)
                    {
                        throw Fail($"'{entry.EntryId}' on {platform}/{architecture}: {closure.Reason} ({closure.Detail})");
                    }

                    var violation = CompositionViolation(closure.Nodes!);
                    if (violation is not null)
                    {
                        throw Fail($"'{entry.EntryId}' on {platform}/{architecture}: {violation}");
                    }

                    var classes = new Dictionary<string, List<string>>(StringComparer.Ordinal);
                    foreach (var field in ComposableFields)
                    {
                        classes[field] = closure.Nodes!.SelectMany(node => node.Variant.Paths(field)).ToList();
                    }

                    var overlap = FindCrossClassOverlap(classes, platform);
                    if (overlap is not null)
                    {
                        throw Fail($"'{entry.EntryId}' on {platform}/{architecture}: {overlap}");
                    }
                }
            }
        }

        return new CatalogRevisionData(contract.SchemaVersion, catalogRevision, entries);
    }
}
