// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using System.Globalization;
using Microsoft.Mxc.PolicyCatalog.Internal;
using Xunit;

namespace Microsoft.Mxc.PolicyCatalog.Tests;

/// <summary>Version ranges, package URLs, and the JavaScript semantics the port reproduces.</summary>
public sealed class PrimitivesTests
{
    [Theory]
    [InlineData(">=10 <12", true)]
    [InlineData("22", true)]
    [InlineData(">=1.2.3", true)]
    [InlineData("=1.0.0", true)]
    [InlineData("<2 || >=4", true)]
    [InlineData("", false)]
    [InlineData("^1.2.3", false)]
    [InlineData("~1", false)]
    [InlineData("1.x", false)]
    [InlineData(">= 1", false)]
    [InlineData("1 ||", false)]
    [InlineData("latest", false)]
    public void VersionRangeGrammar(string range, bool valid) => Assert.Equal(valid, VersionRange.IsValid(range));

    [Theory]
    [InlineData("10.9.0", ">=10 <12", true)]
    [InlineData("v12.0.0", ">=10 <12", false)]
    [InlineData("22.3.1", "22", true)]
    [InlineData("23.0.0", "22", false)]
    [InlineData("3.0.0", "<2 || >=3", true)]
    [InlineData("10.0.0-rc.1", ">=10", true)]
    [InlineData(" 10.1 ", ">=10.1", true)]
    [InlineData("nightly", ">=10", null)]
    [InlineData("10.0.0", "bogus", null)]
    public void VersionRangeEvaluation(string version, string range, bool? expected) => Assert.Equal(expected, VersionRange.Satisfies(version, range));

    [Fact]
    public void PackageUrls()
    {
        Assert.Equal(new ParsedPurl("npm/npm", "10.9.0"), Purl.Parse("pkg:npm/npm@10.9.0"));
        Assert.Equal(new ParsedPurl("npm/%40scope/pkg", "1.0.0"), Purl.Parse("pkg:NPM/%40scope/pkg@1.0.0?x=y#sub"));
        Assert.Equal(new ParsedPurl("npm/npm", null), Purl.Parse("pkg:npm/npm"));
        Assert.Equal(new ParsedPurl("npm/npm", null), Purl.Parse("pkg:npm/npm@"));
        Assert.Equal(new ParsedPurl("npm/npm", "1 2\u00e9"), Purl.Parse("pkg:npm/npm@1%202%C3%A9"));
        Assert.Null(Purl.Parse("npm/npm"));
        Assert.Null(Purl.Parse("pkg:npm"));
        Assert.Null(Purl.Parse("pkg:1npm/x"));
        Assert.Null(Purl.Parse("pkg:npm//x"));
        Assert.Null(Purl.Parse("pkg:npm/npm@%E0%A4%A"));
        Assert.Null(Purl.Parse("pkg:npm/npm@%ED%A0%80"));
        Assert.Null(Purl.Parse("pkg:npm/npm@%zz"));
    }

    [Theory]
    [InlineData(0.0, "0")]
    [InlineData(1e21, "1e+21")]
    [InlineData(1e20, "100000000000000000000")]
    [InlineData(1e-7, "1e-7")]
    [InlineData(0.000001, "0.000001")]
    [InlineData(12345678901234567890.0, "12345678901234567000")]
    [InlineData(5e-324, "5e-324")]
    [InlineData(1.7976931348623157e308, "1.7976931348623157e+308")]
    [InlineData(-1234.5678, "-1234.5678")]
    [InlineData(0.30000000000000004, "0.30000000000000004")]
    [InlineData(123e-20, "1.23e-18")]
    [InlineData(1.5, "1.5")]
    public void EcmaScriptNumberToString(double value, string expected)
    {
        Assert.Equal(expected, JsNumber.ToString(value));
        Assert.Equal("0", JsNumber.ToString(-0.0));
    }

    [Fact]
    public void NumberFormattingMatchesRoundTripForManyValues()
    {
        var random = new Random(1234);
        for (var i = 0; i < 20000; i++)
        {
            var value = BitConverter.Int64BitsToDouble(random.NextInt64());
            if (!double.IsFinite(value))
            {
                continue;
            }

            var text = JsNumber.ToString(value);
            Assert.Equal(value == 0 ? 0 : value, double.Parse(text, CultureInfo.InvariantCulture));
        }
    }

    [Fact]
    public void StringEscapingMatchesJsonStringify()
    {
        Assert.Equal("\"\\u0000\\b\\t\\n\\f\\r\\u001f \\\" \\\\ / \u007f \u2028 \u00e9\"", JsonText.Quote("\0\b\t\n\f\r\u001f \" \\ / \u007f \u2028 \u00e9"));
        Assert.Equal("\"\\ud800x\\udc00\ud83d\ude00\"", JsonText.Quote("\ud800x\udc00\ud83d\ude00"));
    }

    [Fact]
    public void ParsingKeepsJavaScriptKeyOrderAndDuplicateSemantics()
    {
        var parsed = (JsonObject)JsonText.Parse("""{"b":1,"2":0,"a":2,"1":0,"b":3}""");
        Assert.Equal(new[] { "1", "2", "b", "a" }, parsed.Keys);
        Assert.Equal(3.0, ((JsonNumber)parsed.Get("b")!).Value);
        Assert.Equal("""{"1":0,"2":0,"b":3,"a":2}""", JsonText.Stringify(parsed));
        Assert.ThrowsAny<System.Text.Json.JsonException>(() => JsonText.Parse("{\"a\":1,}"));
        Assert.ThrowsAny<System.Text.Json.JsonException>(() => JsonText.Parse("{} x"));
    }

    [Theory]
    [InlineData("GIT.EXE", "git.exe")]
    [InlineData("\u00c9T\u00c9", "\u00e9t\u00e9")]
    [InlineData("\u0130", "i\u0307")]
    [InlineData("\u0391\u03a3", "\u03b1\u03c2")]
    [InlineData("\u0391\u03a3\u0391", "\u03b1\u03c3\u03b1")]
    [InlineData("\u03a3", "\u03c3")]
    [InlineData("\ua7cb", "\u0264")]
    [InlineData("\u212a", "k")]
    public void LowerCasingMatchesJavaScript(string input, string expected) => Assert.Equal(expected, JsString.ToLower(input));

    [Theory]
    [InlineData("AMD64", "x64")]
    [InlineData("x86_64", "x64")]
    [InlineData(" x64\n", "x64")]
    [InlineData("ARM64", "arm64")]
    [InlineData("aarch64", "arm64")]
    [InlineData("riscv64", null)]
    [InlineData("x86", null)]
    public void MachineNamesMapToSelectors(string machine, string? expected) => Assert.Equal(expected, SystemHostEnvironment.ArchitectureFromMachine(machine));

    [Fact]
    public void SystemHostDetectsANativeArchitectureOrFailsWithUnsupportedHost()
    {
        var host = SystemHostEnvironment.Instance;
        Assert.Contains(host.Platform(), CatalogPlatforms.All);
        try
        {
            Assert.Contains(host.NativeArchitecture(), CatalogArchitectures.All);
        }
        catch (PolicyCatalogException error)
        {
            Assert.Equal("unsupported_host", error.Reason);
        }

        Assert.Null(host.Symbol("git_prefix"));
        Assert.NotNull(host.Symbol("temp_dir"));
    }

    [Fact]
    public void ErrorCodesMapLikeTypeScript()
    {
        var expected = new Dictionary<PolicyCatalogErrorReason, (string Reason, string Code)>
        {
            [PolicyCatalogErrorReason.InvalidCatalog] = ("invalid_catalog", "policy_validation"),
            [PolicyCatalogErrorReason.CompositionConflict] = ("composition_conflict", "policy_validation"),
            [PolicyCatalogErrorReason.InvalidContext] = ("invalid_context", "malformed_request"),
            [PolicyCatalogErrorReason.UnsupportedHost] = ("unsupported_host", "unsupported_containment"),
            [PolicyCatalogErrorReason.Integrity] = ("integrity", "backend_error"),
            [PolicyCatalogErrorReason.RevisionUnavailable] = ("revision_unavailable", "backend_error"),
        };
        foreach (var (reason, (wire, code)) in expected)
        {
            var error = new PolicyCatalogException(reason, "m");
            Assert.Equal((wire, code, $"[{code}] m"), (error.Reason, error.Code, error.Message));
        }

        Assert.Equal(
            "{\"error\":{\"code\":\"backend_error\",\"message\":\"[backend_error] m\",\"details\":{\"reason\":\"integrity\"}}}",
            PolicyCatalogJson.Serialize(new PolicyCatalogException(PolicyCatalogErrorReason.Integrity, "m")));
    }
}
