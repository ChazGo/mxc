// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using Microsoft.Mxc.PolicyCatalog.Internal;

namespace Microsoft.Mxc.PolicyCatalog.Internal
{
    /// <summary>Converts validated raw policy JSON into the typed policy records.</summary>
    internal static class PolicyConversion
    {
        private static string? Str(JsonObject obj, string key) => obj.Get(key) is JsonString s ? s.Value : null;

        private static double? Num(JsonObject obj, string key) => obj.Get(key) is JsonNumber n ? n.Value : null;

        private static bool? Bool(JsonObject obj, string key) => obj.Get(key) is JsonBool b ? b.Value : null;

        private static List<string>? Strings(JsonObject obj, string key) =>
            obj.Get(key) is JsonArray a ? a.Items.Select(item => ((JsonString)item).Value).ToList() : null;

        public static CatalogSandboxPolicy ToPolicy(JsonObject raw)
        {
            var policy = new CatalogSandboxPolicy(Str(raw, "version")!) { TimeoutMs = Num(raw, "timeoutMs") };
            if (raw.Get("filesystem") is JsonObject fs)
            {
                policy = policy with
                {
                    Filesystem = new CatalogFilesystemPolicy
                    {
                        DeniedPaths = Strings(fs, "deniedPaths"),
                        ReadonlyPaths = Strings(fs, "readonlyPaths"),
                        ReadwritePaths = Strings(fs, "readwritePaths"),
                    },
                };
            }

            if (raw.Get("network") is JsonObject network)
            {
                CatalogEgressPolicy? egress = null;
                if (network.Get("egress") is JsonObject e)
                {
                    egress = new CatalogEgressPolicy { Default = Str(e, "default"), Allow = Rules(e.Get("allow")), Deny = Rules(e.Get("deny")) };
                }

                CatalogIngressPolicy? ingress = null;
                if (network.Get("ingress") is JsonObject i)
                {
                    ingress = new CatalogIngressPolicy { Default = Str(i, "default"), HostLoopback = Str(i, "hostLoopback") };
                }

                policy = policy with { Network = new CatalogNetworkPolicy { Egress = egress, Ingress = ingress } };
            }

            if (raw.Get("ui") is JsonObject ui)
            {
                policy = policy with
                {
                    Ui = new CatalogUiPolicy { AllowWindows = Bool(ui, "allowWindows"), Clipboard = Str(ui, "clipboard"), AllowInputInjection = Bool(ui, "allowInputInjection") },
                };
            }

            return policy;
        }

        private static List<CatalogNetworkRule>? Rules(JsonValue? value)
        {
            if (value is not JsonArray rules)
            {
                return null;
            }

            return rules.Items.Cast<JsonObject>().Select(rule => new CatalogNetworkRule
            {
                To = rule.Get("to") is JsonArray to
                    ? to.Items.Cast<JsonObject>().Select(peer => new CatalogNetworkPeer(Str(peer, "cidr")!) { Except = Strings(peer, "except") }).ToList()
                    : null,
                Ports = rule.Get("ports") is JsonArray ports
                    ? ports.Items.Cast<JsonObject>().Select(port => new CatalogNetworkPort { Protocol = Str(port, "protocol"), Port = Num(port, "port"), EndPort = Num(port, "endPort") }).ToList()
                    : null,
            }).ToList();
        }
    }
}

namespace Microsoft.Mxc.PolicyCatalog
{
    /// <summary>
    /// Serializes results to the exact JSON shape the TypeScript reference emits (<c>JSON.stringify</c>):
    /// absent optional fields are omitted, never <c>null</c>; numbers use ECMAScript formatting.
    /// </summary>
    public static class PolicyCatalogJson
    {
        /// <summary>Serializes a policy; <c>null</c> becomes <c>null</c>.</summary>
        /// <param name="policy">The policy or <c>null</c>.</param>
        /// <param name="indent">Indentation (0 = compact, 2 = the CLI form).</param>
        /// <returns>JSON text without a trailing newline.</returns>
        public static string Serialize(CatalogSandboxPolicy? policy, int indent = 0) => JsonText.Stringify(ToJson(policy), indent);

        /// <summary>Serializes a resolution; <c>policy</c> is omitted when absent.</summary>
        /// <param name="resolution">The resolution.</param>
        /// <param name="indent">Indentation.</param>
        /// <returns>JSON text.</returns>
        public static string Serialize(SandboxConfigResolution resolution, int indent = 0) => JsonText.Stringify(ToJson(resolution), indent);

        /// <summary>Serializes catalog info.</summary>
        /// <param name="info">The info.</param>
        /// <param name="indent">Indentation.</param>
        /// <returns>JSON text.</returns>
        public static string Serialize(CatalogInfo info, int indent = 0) => JsonText.Stringify(ToJson(info), indent);

        /// <summary>Serializes entry metadata as a JSON array.</summary>
        /// <param name="entries">The entries.</param>
        /// <param name="indent">Indentation.</param>
        /// <returns>JSON text.</returns>
        public static string Serialize(IReadOnlyList<CatalogEntryMetadata> entries, int indent = 0) => JsonText.Stringify(ToJson(entries), indent);

        /// <summary>Serializes the CLI's inspect output, <c>{"info": ..., "entries": [...]}</c>.</summary>
        /// <param name="info">The catalog info.</param>
        /// <param name="entries">The entries.</param>
        /// <param name="indent">Indentation.</param>
        /// <returns>JSON text.</returns>
        public static string SerializeInspect(CatalogInfo info, IReadOnlyList<CatalogEntryMetadata> entries, int indent = 0) =>
            JsonText.Stringify(new JsonObject().With("info", ToJson(info)).With("entries", ToJson(entries)), indent);

        /// <summary>Serializes a validation report.</summary>
        /// <param name="report">The report.</param>
        /// <param name="indent">Indentation.</param>
        /// <returns>JSON text.</returns>
        public static string Serialize(CatalogValidationReport report, int indent = 0) => JsonText.Stringify(ToJson(report), indent);

        /// <summary>Serializes a failure as the CLI's <c>{"error": {"code", "message", "details": {"reason"}}}</c>.</summary>
        /// <param name="error">The failure.</param>
        /// <param name="indent">Indentation.</param>
        /// <returns>JSON text.</returns>
        public static string Serialize(PolicyCatalogException error, int indent = 0)
        {
            ArgumentNullException.ThrowIfNull(error);
            var body = new JsonObject()
                .With("code", new JsonString(error.Code))
                .With("message", new JsonString(error.Message))
                .With("details", new JsonObject().With("reason", new JsonString(error.Reason)));
            return JsonText.Stringify(new JsonObject().With("error", body), indent);
        }

        /// <summary>The canonical JSON form (sorted keys, no whitespace) of JSON text, as used for revision digests.</summary>
        /// <param name="json">JSON text.</param>
        /// <returns>Canonical JSON text.</returns>
        public static string Canonicalize(string json) => CanonicalJson.Serialize(JsonText.Parse(json));

        /// <summary>Lower-case hex SHA-256 over the canonical JSON form of JSON text.</summary>
        /// <param name="json">JSON text.</param>
        /// <returns>The digest.</returns>
        public static string CanonicalSha256(string json) => CanonicalJson.Sha256(JsonText.Parse(json));

        private static JsonValue S(string? value) => value is null ? null! : new JsonString(value);

        private static JsonValue? OptS(string? value) => value is null ? null : new JsonString(value);

        private static JsonValue? OptN(double? value) => value is null ? null : new JsonNumber(value.Value);

        private static JsonValue? OptB(bool? value) => value is null ? null : JsonBool.Of(value.Value);

        private static JsonValue? OptList(IReadOnlyList<string>? values) => values is null ? null : Strings(values);

        private static JsonArray Strings(IEnumerable<string> values) => new(values.Select(value => (JsonValue)new JsonString(value)));

        internal static JsonValue ToJson(CatalogSandboxPolicy? policy)
        {
            if (policy is null)
            {
                return JsonNull.Instance;
            }

            var obj = new JsonObject().With("version", S(policy.Version));
            if (policy.Filesystem is { } fs)
            {
                obj.Set("filesystem", new JsonObject()
                    .With("deniedPaths", OptList(fs.DeniedPaths))
                    .With("readonlyPaths", OptList(fs.ReadonlyPaths))
                    .With("readwritePaths", OptList(fs.ReadwritePaths)));
            }

            if (policy.Network is { } network)
            {
                var n = new JsonObject();
                if (network.Egress is { } egress)
                {
                    n.Set("egress", new JsonObject()
                        .With("default", OptS(egress.Default))
                        .With("allow", Rules(egress.Allow))
                        .With("deny", Rules(egress.Deny)));
                }

                if (network.Ingress is { } ingress)
                {
                    n.Set("ingress", new JsonObject()
                        .With("default", OptS(ingress.Default))
                        .With("hostLoopback", OptS(ingress.HostLoopback)));
                }

                obj.Set("network", n);
            }

            if (policy.Ui is { } ui)
            {
                obj.Set("ui", new JsonObject()
                    .With("allowWindows", OptB(ui.AllowWindows))
                    .With("clipboard", OptS(ui.Clipboard))
                    .With("allowInputInjection", OptB(ui.AllowInputInjection)));
            }

            return obj.With("timeoutMs", OptN(policy.TimeoutMs));
        }

        private static JsonValue? Rules(IReadOnlyList<CatalogNetworkRule>? rules)
        {
            if (rules is null)
            {
                return null;
            }

            return new JsonArray(rules.Select(rule => (JsonValue)new JsonObject()
                .With("to", rule.To is null ? null : new JsonArray(rule.To.Select(peer => (JsonValue)new JsonObject()
                    .With("cidr", S(peer.Cidr))
                    .With("except", OptList(peer.Except)))))
                .With("ports", rule.Ports is null ? null : new JsonArray(rule.Ports.Select(port => (JsonValue)new JsonObject()
                    .With("protocol", OptS(port.Protocol))
                    .With("port", OptN(port.Port))
                    .With("endPort", OptN(port.EndPort)))))));
        }

        internal static JsonValue ToJson(SandboxConfigResolution resolution)
        {
            ArgumentNullException.ThrowIfNull(resolution);
            var d = resolution.Diagnostics;
            var diagnostics = new JsonObject()
                .With("catalogRevision", S(d.CatalogRevision))
                .With("tools", new JsonArray(d.Tools.Select(tool => (JsonValue)new JsonObject()
                    .With("inputIndex", new JsonNumber(tool.InputIndex))
                    .With("matches", new JsonArray(tool.Matches.Select(match => (JsonValue)new JsonObject()
                        .With("entryId", S(match.EntryId))
                        .With("entryRevision", new JsonNumber(match.EntryRevision))
                        .With("matchedIdentities", new JsonArray(match.MatchedIdentities.Select(identity => (JsonValue)new JsonObject()
                            .With("kind", S(identity.Kind))
                            .With("strength", S(identity.Strength)))))))))))
                .With("resolvedDependencies", new JsonArray(d.ResolvedDependencies.Select(dependency => (JsonValue)new JsonObject()
                    .With("entryId", S(dependency.EntryId))
                    .With("entryRevision", new JsonNumber(dependency.EntryRevision))
                    .With("requiredVersionRange", OptS(dependency.RequiredVersionRange)))))
                .With("warnings", Strings(d.Warnings));
            return new JsonObject()
                .With("policy", resolution.Policy is null ? null : ToJson(resolution.Policy))
                .With("diagnostics", diagnostics);
        }

        internal static JsonValue ToJson(CatalogInfo info) => new JsonObject()
            .With("catalogSchemaVersion", S(info.CatalogSchemaVersion))
            .With("catalogRevision", S(info.CatalogRevision));

        internal static JsonValue ToJson(IReadOnlyList<CatalogEntryMetadata> entries) => new JsonArray(entries.Select(entry => (JsonValue)new JsonObject()
            .With("catalogRevision", S(entry.CatalogRevision))
            .With("entryId", S(entry.EntryId))
            .With("entryRevision", new JsonNumber(entry.EntryRevision))
            .With("displayName", S(entry.DisplayName))
            .With("identity", new JsonArray(entry.Identity.Select(identity => (JsonValue)(identity.Kind == "purl"
                ? new JsonObject().With("kind", S(identity.Kind)).With("value", OptS(identity.Value)).With("versionRange", OptS(identity.VersionRange))
                : new JsonObject().With("kind", S(identity.Kind)).With("names", OptList(identity.Names))))))
            .With("platformVariants", new JsonArray(entry.PlatformVariants.Select(variant => (JsonValue)new JsonObject()
                .With("platform", S(variant.Platform))
                .With("architecture", OptS(variant.Architecture))
                .With("dependencyEntryIds", Strings(variant.DependencyEntryIds))
                .With("sandboxPolicyVersion", S(variant.SandboxPolicyVersion)))))
            .With("provenance", new JsonObject()
                .With("method", S(entry.Provenance.Method))
                .With("sourceRevision", S(entry.Provenance.SourceRevision)))));

        internal static JsonValue ToJson(CatalogValidationReport report) => new JsonObject()
            .With("ok", JsonBool.Of(report.Ok))
            .With("catalogDir", S(report.CatalogDir))
            .With("defaultRevision", OptS(report.DefaultRevision))
            .With("revisions", OptList(report.Revisions))
            .With("baseRef", report.BaseRef is null ? null : new JsonObject()
                .With("ref", S(report.BaseRef.Ref))
                .With("comparedRevisions", new JsonNumber(report.BaseRef.ComparedRevisions)))
            .With("errors", Strings(report.Errors));
    }
}
