// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using System.Text.Json.Nodes;
using Microsoft.Mxc.PolicyCatalog.Internal;
using Xunit;

namespace Microsoft.Mxc.PolicyCatalog.Tests;

/// <summary>Port of tests/unit/validation.test.ts (contribution/CI rules).</summary>
public sealed class ValidationTests
{
    private static readonly CatalogContract Contract = Catalog.ValidateContract(JsonText.Parse(TestData.Contract));

    private static string? Validate(params JsonNode[] entries)
    {
        try
        {
            Catalog.ValidateRevision(JsonText.Parse(TestData.Revision(entries).ToJsonString()), Contract);
            return null;
        }
        catch (PolicyCatalogException error)
        {
            Assert.Equal("invalid_catalog", error.Reason);
            return error.Message;
        }
    }

    private static void Rejects(string expected, params JsonNode[] entries)
    {
        var message = Validate(entries);
        Assert.NotNull(message);
        Assert.Contains(expected, message, StringComparison.Ordinal);
    }

    private static string Linux(string policy, string extra = "") =>
        $$"""{ "when": { "platform": "linux" }{{extra}}, "sandboxPolicy": {{policy}} }""";

    private static string Variants(params string[] variants) => $$"""{ "platformVariants": [{{string.Join(",", variants)}}] }""";

    private static string Dep(string id) => Variants(Linux("""{ "version": "0.9.0-alpha" }""", $$""", "dependencies": [{ "entryId": "{{id}}" }]"""));

    [Fact]
    public void AcceptsMinimalRevision() => Assert.Null(Validate(TestData.Entry("tool:a")));

    [Fact]
    public void RejectsDependencyCycles()
    {
        Rejects("cycle (tool:a -> tool:b -> tool:c -> tool:a)", TestData.Entry("tool:a", Dep("tool:b")), TestData.Entry("tool:b", Dep("tool:c")), TestData.Entry("tool:c", Dep("tool:a")));
        Rejects("depends on itself", TestData.Entry("tool:a", Dep("tool:a")));
        Rejects("[policy_validation] 'tool:a' on linux/x64: cycle", TestData.Entry("tool:a", Dep("tool:b")), TestData.Entry("tool:b", Dep("tool:a")));
    }

    [Fact]
    public void RejectsUnknownDependencies() => Rejects("unknown entry 'tool:missing'", TestData.Entry("tool:a", Dep("tool:missing")));

    [Fact]
    public void RejectsInvalidDependencyRanges() =>
        Rejects("versionRange", TestData.Entry("tool:a", Variants(Linux("""{ "version": "0.9.0-alpha" }""", """, "dependencies": [{ "entryId": "tool:b", "versionRange": "^1.x" }]"""))), TestData.Entry("tool:b"));

    [Fact]
    public void RejectsDuplicateSelectors()
    {
        static string V(string? arch) => arch is null
            ? """{ "when": { "platform": "linux" }, "sandboxPolicy": { "version": "0.9.0-alpha" } }"""
            : $$"""{ "when": { "platform": "linux", "architecture": "{{arch}}" }, "sandboxPolicy": { "version": "0.9.0-alpha" } }""";
        Rejects("duplicates selector 'linux/x64'", TestData.Entry("tool:a", Variants(V("x64"), V("x64"))));
        Rejects("second architecture-neutral", TestData.Entry("tool:a", Variants(V(null), V(null))));
        Assert.Null(Validate(TestData.Entry("tool:a", Variants(V(null), V("x64"), V("arm64")))));
        Rejects("'entries[0].platformVariants[0].when.architecture' must be one of x64, arm64", TestData.Entry("tool:a", Variants(V("riscv"))));
    }

    [Fact]
    public void RejectsBackendKeysAndUnregisteredVersions()
    {
        Rejects("names a containment backend", TestData.Entry("tool:a", Variants(Linux("""{ "version": "0.9.0-alpha", "processContainer": {} }"""))));
        Rejects("containment backend", TestData.Entry("tool:a", Variants(Linux("""{ "version": "0.9.0-alpha", "containment": "lxc" }"""))));
        Rejects("not a SandboxPolicy version registered", TestData.Entry("tool:a", Variants(Linux("""{ "version": "0.9.0" }"""))));
    }

    [Fact]
    public void RejectsUnsupportedFields()
    {
        Rejects("unsupported field 'entries[0].extra'", TestData.Entry("tool:a", """{ "extra": 1 }"""));
        Rejects("unsupported field", TestData.Entry("tool:a", Variants(Linux("""{ "version": "0.9.0-alpha", "filesystem": { "clearPolicyOnExit": true } }"""))));
        Rejects("unsupported field", TestData.Entry("tool:a", Variants(Linux("""{ "version": "0.9.0-alpha", "network": { "allowOutbound": true } }"""))));
        Rejects("unsupported field", TestData.Entry("tool:a", Variants(Linux("""{ "version": "0.9.0-alpha", "telemetry": {} }"""))));
    }

    [Theory]
    [InlineData("/home/alice/.npm", "literal paths are not allowed")]
    [InlineData("C:\\\\Users\\\\alice", "literal paths are not allowed")]
    [InlineData("${project_root}/*", "wildcard")]
    [InlineData("${project_root}/../x", "'..'")]
    [InlineData("${unknown_thing}", "unknown symbol 'unknown_thing'")]
    [InlineData("${project_root", "malformed symbol")]
    [InlineData("${project_root}/node_modules", null)]
    public void PathRules(string path, string? expected)
    {
        var entry = TestData.Entry("tool:a", Variants(Linux($$"""{ "version": "0.9.0-alpha", "filesystem": { "readonlyPaths": ["{{path}}"] } }""")));
        if (expected is null)
        {
            Assert.Null(Validate(entry));
        }
        else
        {
            Rejects(expected, entry);
        }
    }

    [Theory]
    [InlineData("""{ "egress": { "default": "allow" } }""", "default-allow")]
    [InlineData("""{ "ingress": { "default": "allow" } }""", "default-allow")]
    [InlineData("""{ "egress": { "allow": [{ "ports": [{ "port": 443 }] }] } }""", "wildcard network grant")]
    [InlineData("""{ "egress": { "allow": [{ "to": [{ "cidr": "0.0.0.0/0" }] }] } }""", "wildcard network grant")]
    [InlineData("""{ "egress": { "allow": [{ "to": [{ "cidr": "10.0.0.0/8" }], "ports": [{ "port": 70000 }] }] } }""", "must be an integer in 1..65535")]
    [InlineData("""{ "egress": { "allow": [{ "to": [{ "cidr": "10.0.0.0/8" }], "ports": [{ "port": 5, "endPort": 4 }] }] } }""", "requires a lower or equal 'port'")]
    [InlineData("""{ "egress": { "deny": [{ "ports": [{ "protocol": "tcp" }] }] } }""", null)]
    [InlineData("""{ "egress": { "default": "deny", "allow": [{ "to": [{ "cidr": "192.0.2.0/24" }], "ports": [{ "protocol": "tcp", "port": 443 }] }] } }""", null)]
    public void NetworkRules(string network, string? expected)
    {
        var entry = TestData.Entry("tool:a", Variants(Linux($$"""{ "version": "0.9.0-alpha", "network": {{network}} }""")));
        if (expected is null)
        {
            Assert.Null(Validate(entry));
        }
        else
        {
            Rejects(expected, entry);
        }
    }

    [Fact]
    public void IdentityRules()
    {
        Assert.Null(Validate(TestData.Entry("tool:a"), TestData.Entry("tool:b", """{ "identity": [{ "kind": "invocation-name", "names": ["A"] }] }""")));
        Assert.Null(Validate(
            TestData.Entry("tool:a", """{ "identity": [{ "kind": "purl", "value": "pkg:npm/x" }] }"""),
            TestData.Entry("tool:b", """{ "identity": [{ "kind": "purl", "value": "pkg:NPM/x" }] }""")));
        Rejects("repeats identity 'invocation-name:a'", TestData.Entry("tool:a", """{ "identity": [{ "kind": "invocation-name", "names": ["a", "A"] }] }"""));
        // Folding matches JavaScript toLowerCase beyond ASCII (Latin-1 and final sigma).
        Rejects("repeats identity", TestData.Entry("tool:a", """{ "identity": [{ "kind": "invocation-name", "names": ["\u00c9t\u00e9", "\u00e9T\u00c9"] }] }"""));
        Rejects("repeats identity 'purl:npm/a'", TestData.Entry("tool:a", """{ "identity": [{ "kind": "purl", "value": "pkg:npm/a" }, { "kind": "purl", "value": "pkg:NPM/a" }] }"""));
        Rejects("must not pin a version", TestData.Entry("tool:a", """{ "identity": [{ "kind": "purl", "value": "pkg:npm/a@1.0.0" }] }"""));
        Rejects("bare invocation name", TestData.Entry("tool:a", """{ "identity": [{ "kind": "invocation-name", "names": ["bin/a"] }] }"""));
        Rejects("not a supported identity kind", TestData.Entry("tool:a", """{ "identity": [{ "kind": "sha256", "value": "x" }] }"""));
        Rejects("duplicate entryId 'tool:a'", TestData.Entry("tool:a"), TestData.Entry("tool:a", """{ "identity": [{ "kind": "invocation-name", "names": ["z"] }] }"""));
        Rejects("must be namespaced", TestData.Entry("tool:a", """{ "entryId": "noNamespace" }"""));
        Rejects("'entries[0].entryRevision' must be a positive integer", TestData.Entry("tool:a", """{ "entryRevision": 1.5 }"""));
    }

    private static JsonNode[] WithDep(string policy, string depPolicy) => new[]
    {
        TestData.Entry("tool:a", Variants(Linux(policy, """, "dependencies": [{ "entryId": "tool:b" }]"""))),
        TestData.Entry("tool:b", Variants(Linux(depPolicy))),
    };

    [Fact]
    public void CompositionVocabulary()
    {
        Assert.Null(Validate(WithDep(
            """{ "version": "0.9.0-alpha", "filesystem": { "readwritePaths": ["${project_root}"], "readonlyPaths": ["${node_prefix}"] } }""",
            """{ "version": "0.9.0-alpha", "filesystem": { "readonlyPaths": ["${node_prefix}", "${git_prefix}"], "deniedPaths": ["${user_home}/.ssh"] } }""")));
        Rejects("overlaps", WithDep(
            """{ "version": "0.9.0-alpha", "filesystem": { "readwritePaths": ["${project_root}"] } }""",
            """{ "version": "0.9.0-alpha", "filesystem": { "deniedPaths": ["${project_root}/secrets"] } }"""));
        Rejects("mixed sandboxPolicy.version values (0.8.0-alpha, 0.9.0-alpha)", WithDep("""{ "version": "0.9.0-alpha" }""", """{ "version": "0.8.0-alpha" }"""));
        Rejects("'tool:a' uses 'network'", WithDep("""{ "version": "0.9.0-alpha", "network": { "egress": { "default": "deny" } } }""", """{ "version": "0.9.0-alpha" }"""));
        Rejects("'timeoutMs'", WithDep("""{ "version": "0.9.0-alpha" }""", """{ "version": "0.9.0-alpha", "timeoutMs": 5 }"""));
        Assert.Null(Validate(TestData.Entry("tool:a", Variants(Linux("""{ "version": "0.9.0-alpha", "network": { "egress": { "default": "deny" } }, "ui": { "clipboard": "read" }, "timeoutMs": 5 }""")))));
    }

    [Fact]
    public void ContractValidation()
    {
        Assert.Equal(7, Contract.Symbols.Count);
        var bad = TestData.Parse(TestData.Contract);
        bad["symbols"]!["x"] = TestData.Parse("""{ "source": "elsewhere", "description": "d" }""");
        var error = Assert.Throws<PolicyCatalogException>(() => Catalog.ValidateContract(JsonText.Parse(bad.ToJsonString())));
        Assert.Equal("[policy_validation] contract.symbols.x.source is unsupported", error.Message);
    }
}
