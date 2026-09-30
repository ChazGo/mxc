// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using System.Text.Json;
using Microsoft.Mxc.PolicyCatalog.Internal;
using Xunit;

namespace Microsoft.Mxc.PolicyCatalog.Tests;

/// <summary>conformance/vectors/*.json: canonical JSON and path rules shared by every binding.</summary>
public sealed class VectorTests
{
    public static TheoryData<int> CanonicalCases()
    {
        using var doc = JsonDocument.Parse(TestData.Resource("conformance/vectors/canonical-json.json"));
        var data = new TheoryData<int>();
        for (var i = 0; i < doc.RootElement.GetProperty("cases").GetArrayLength(); i++)
        {
            data.Add(i);
        }

        return data;
    }

    [Theory]
    [MemberData(nameof(CanonicalCases))]
    public void CanonicalJsonVector(int index)
    {
        using var doc = JsonDocument.Parse(TestData.Resource("conformance/vectors/canonical-json.json"));
        var c = doc.RootElement.GetProperty("cases")[index];
        var json = c.GetProperty("json").GetString()!;
        Assert.Equal(c.GetProperty("canonical").GetString(), PolicyCatalogJson.Canonicalize(json));
        Assert.Equal(c.GetProperty("sha256").GetString(), PolicyCatalogJson.CanonicalSha256(json));
    }

    [Fact]
    public void EveryVectorFileIsCovered()
    {
        Assert.Equal(
            new[] { "conformance/vectors/canonical-json.json", "conformance/vectors/paths.json" },
            TestData.Resources("conformance/vectors/").ToArray());
    }

    [Theory]
    [InlineData("windows")]
    [InlineData("linux")]
    [InlineData("macos")]
    public void PathVectors(string platform)
    {
        using var doc = JsonDocument.Parse(TestData.Resource("conformance/vectors/paths.json"));
        var rows = doc.RootElement.GetProperty(platform).EnumerateArray().ToList();
        Assert.True(rows.Count > 5);
        foreach (var row in rows)
        {
            var input = row.GetProperty("input").GetString()!;
            Assert.True(row.GetProperty("absolute").GetBoolean() == Paths.IsAbsolutePath(input, platform), input);
            var normalized = row.GetProperty("normalized");
            if (normalized.ValueKind != JsonValueKind.Null)
            {
                Assert.Equal(normalized.GetString(), Paths.NormalizePath(input, platform));
            }

            Assert.Equal(row.GetProperty("keySegments").EnumerateArray().Select(s => s.GetString()!).ToList(), Paths.PathKeySegments(input, platform));
        }
    }

    [Fact]
    public void MalformedPercentEncodingIsAnInvalidPurl()
    {
        Assert.Null(Purl.Parse("pkg:npm/npm@%E0%A4%A"));
        Assert.Equal(new ParsedPurl("npm/npm", "10.9.0"), Purl.Parse("pkg:npm/npm@10.9.0"));
    }
}
