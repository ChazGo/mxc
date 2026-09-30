// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using System.Text.Json;
using System.Text.Json.Nodes;
using Xunit;

namespace Microsoft.Mxc.PolicyCatalog.Tests;

/// <summary>Runs every case in conformance/fixtures/*.json (the TypeScript conformance runner's rules).</summary>
public sealed class ConformanceTests
{
    public static TheoryData<string, int, string> Cases()
    {
        var data = new TheoryData<string, int, string>();
        foreach (var resource in TestData.Resources("conformance/fixtures/"))
        {
            using var doc = JsonDocument.Parse(TestData.Resource(resource));
            var index = 0;
            foreach (var testCase in doc.RootElement.GetProperty("cases").EnumerateArray())
            {
                data.Add(resource, index++, testCase.GetProperty("name").GetString()!);
            }
        }

        return data;
    }

    [Fact]
    public void EveryFixtureFileHasCases()
    {
        var files = TestData.Resources("conformance/fixtures/").ToList();
        Assert.Contains("conformance/fixtures/bundled-catalog.json", files);
        Assert.Contains("conformance/fixtures/synthetic-catalog.json", files);
        Assert.True(Cases().Count >= 20);
    }

    [Theory]
    [MemberData(nameof(Cases))]
    public void Case(string file, int index, string name)
    {
        Assert.False(string.IsNullOrEmpty(name));
        using var doc = JsonDocument.Parse(TestData.Resource(file));
        var root = doc.RootElement;
        var testCase = root.GetProperty("cases")[index];
        var hostPlatform = "linux";
        var hostArchitecture = "x64";
        if (testCase.TryGetProperty("host", out var host))
        {
            if (host.TryGetProperty("platform", out var p))
            {
                hostPlatform = p.GetString()!;
            }

            if (host.TryGetProperty("nativeArchitecture", out var a))
            {
                hostArchitecture = a.GetString()!;
            }
        }

        var fixedHost = new FixedHost(hostPlatform, hostArchitecture);
        var catalogElement = root.GetProperty("catalog");
        var catalog = catalogElement.ValueKind == JsonValueKind.String && catalogElement.GetString() == "bundled"
            ? TestData.Bundled(fixedHost)
            : TestData.CatalogFor(JsonNode.Parse(catalogElement.GetRawText())!, fixedHost);
        var toolsElement = testCase.GetProperty("tools");
        var tools = TestData.Tools(toolsElement);
        JsonElement? contextElement = testCase.TryGetProperty("context", out var c) ? c : null;
        var context = TestData.Context(contextElement);

        if (testCase.TryGetProperty("expectError", out var expectError))
        {
            var code = expectError.GetProperty("code").GetString();
            var reason = expectError.GetProperty("reason").GetString();
            var withDiagnostics = TestData.Failure(() => catalog.GetSandboxConfigWithDiagnostics(tools, context));
            Assert.Equal((code, reason), (withDiagnostics.Code, withDiagnostics.Reason));
            var policyOnly = TestData.Failure(() => catalog.GetSandboxConfig(tools, context));
            Assert.Equal((code, reason), (policyOnly.Code, policyOnly.Reason));
            return;
        }

        var expect = JsonNode.Parse(testCase.GetProperty("expect").GetRawText())!.AsObject();
        var expectedPolicy = expect["policy"];
        if (expectedPolicy is null)
        {
            // JSON.stringify omits an undefined policy.
            expect.Remove("policy");
        }

        var expected = TestData.Canonical(expect.ToJsonString());
        var result = catalog.GetSandboxConfigWithDiagnostics(tools, context);
        Assert.Equal(expected, TestData.Canonical(PolicyCatalogJson.Serialize(result)));

        var policy = catalog.GetSandboxConfig(tools, context);
        Assert.Equal(TestData.Canonical(expectedPolicy?.ToJsonString() ?? "null"), TestData.Canonical(PolicyCatalogJson.Serialize(policy)));
        Assert.Equal(result.Policy is null, policy is null);

        if (TestData.IsSingle(toolsElement))
        {
            var single = catalog.GetSandboxConfigWithDiagnostics(tools[0], context);
            Assert.Equal(expected, TestData.Canonical(PolicyCatalogJson.Serialize(single)));
            Assert.Equal(
                TestData.Canonical(PolicyCatalogJson.Serialize(catalog.GetSandboxConfig(tools[0], context))),
                TestData.Canonical(PolicyCatalogJson.Serialize(policy)));
        }
    }
}
