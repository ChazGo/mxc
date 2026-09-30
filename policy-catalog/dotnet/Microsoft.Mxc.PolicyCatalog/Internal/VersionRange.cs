// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using System.Globalization;
using System.Text.RegularExpressions;

namespace Microsoft.Mxc.PolicyCatalog.Internal;

/// <summary>
/// Minimal version-range support for catalog v1 (TypeScript <c>version-range.ts</c>).
/// <c>range := set ("||" set)*</c>, <c>set := comparator (" " comparator)*</c>,
/// <c>comparator := op? N(.N(.N)?)?</c>. No operator is a prefix match.
/// Prerelease/build metadata on evidence versions is ignored.
/// </summary>
internal static class VersionRange
{
    private enum Op
    {
        GreaterOrEqual,
        LessOrEqual,
        Greater,
        Less,
        Equal,
        Prefix,
    }

    private sealed record Comparator(Op Op, double[] Version, int Parts);

    private static readonly Regex ComparatorPattern = new("^(>=|<=|>|<|=)?([0-9]+)(?:\\.([0-9]+)(?:\\.([0-9]+))?)?\\z", RegexOptions.CultureInvariant);

    private static readonly Regex EvidencePattern = new(
        "^v?([0-9]+)(?:\\.([0-9]+))?(?:\\.([0-9]+))?(?:[-+]" + JsString.AnyButLineTerminator + "*)?\\z",
        RegexOptions.CultureInvariant);

    private static readonly Regex Whitespace = new(JsString.WhitespaceClass + "+", RegexOptions.CultureInvariant);

    private static double Num(Group group) => group.Success ? double.Parse(group.Value, NumberStyles.None, CultureInfo.InvariantCulture) : 0;

    private static Comparator? ParseComparator(string token)
    {
        var match = ComparatorPattern.Match(token);
        if (!match.Success)
        {
            return null;
        }

        var parts = match.Groups[4].Success ? 3 : match.Groups[3].Success ? 2 : 1;
        var op = match.Groups[1].Success
            ? match.Groups[1].Value switch
            {
                ">=" => Op.GreaterOrEqual,
                "<=" => Op.LessOrEqual,
                ">" => Op.Greater,
                "<" => Op.Less,
                _ => Op.Equal,
            }
            : Op.Prefix;
        return new Comparator(op, new[] { Num(match.Groups[2]), Num(match.Groups[3]), Num(match.Groups[4]) }, parts);
    }

    private static List<List<Comparator>>? ParseRange(string range)
    {
        if (JsString.Trim(range).Length == 0)
        {
            return null;
        }

        var sets = new List<List<Comparator>>();
        foreach (var alternative in range.Split("||"))
        {
            var tokens = Whitespace.Split(JsString.Trim(alternative)).Where(token => token.Length > 0).ToList();
            if (tokens.Count == 0)
            {
                return null;
            }

            var comparators = new List<Comparator>();
            foreach (var token in tokens)
            {
                var comparator = ParseComparator(token);
                if (comparator is null)
                {
                    return null;
                }

                comparators.Add(comparator);
            }

            sets.Add(comparators);
        }

        return sets;
    }

    /// <summary>True when <paramref name="range"/> uses the supported v1 range grammar.</summary>
    public static bool IsValid(string range) => ParseRange(range) is not null;

    /// <summary>Parses tool version evidence such as <c>v22.3.1</c> or <c>10.9.0-rc.1</c>.</summary>
    public static double[]? ParseEvidence(string value)
    {
        var match = EvidencePattern.Match(JsString.Trim(value));
        if (!match.Success)
        {
            return null;
        }

        return new[] { Num(match.Groups[1]), Num(match.Groups[2]), Num(match.Groups[3]) };
    }

    private static int Compare(double[] left, double[] right)
    {
        for (var i = 0; i < 3; i++)
        {
            if (left[i] != right[i])
            {
                return left[i] < right[i] ? -1 : 1;
            }
        }

        return 0;
    }

    private static bool Satisfies(double[] version, Comparator comparator)
    {
        var order = Compare(version, comparator.Version);
        return comparator.Op switch
        {
            Op.GreaterOrEqual => order >= 0,
            Op.LessOrEqual => order <= 0,
            Op.Greater => order > 0,
            Op.Less => order < 0,
            Op.Equal => order == 0,
            _ => Enumerable.Range(0, comparator.Parts).All(i => version[i] == comparator.Version[i]),
        };
    }

    /// <summary>Evaluates <paramref name="version"/> against <paramref name="range"/>; <c>null</c> when either cannot be evaluated.</summary>
    public static bool? Satisfies(string version, string range)
    {
        var parsedVersion = ParseEvidence(version);
        var parsedRange = ParseRange(range);
        if (parsedVersion is null || parsedRange is null)
        {
            return null;
        }

        return parsedRange.Any(set => set.All(comparator => Satisfies(parsedVersion, comparator)));
    }
}
