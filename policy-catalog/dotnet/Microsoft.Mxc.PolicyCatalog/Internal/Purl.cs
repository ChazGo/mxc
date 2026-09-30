// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using System.Text;
using System.Text.RegularExpressions;

namespace Microsoft.Mxc.PolicyCatalog.Internal;

/// <summary>Parsed package URL, reduced to the parts identity matching uses (TypeScript <c>purl.ts</c>).</summary>
internal sealed record ParsedPurl(string Key, string? Version);

internal static class Purl
{
    private static readonly Regex TypePattern = new("^[a-zA-Z][a-zA-Z0-9.+-]*\\z", RegexOptions.CultureInvariant);

    /// <summary>Parses <c>pkg:type/namespace/name@version?qualifiers#subpath</c>; <c>null</c> when invalid.</summary>
    public static ParsedPurl? Parse(string value)
    {
        if (!value.StartsWith("pkg:", StringComparison.Ordinal))
        {
            return null;
        }

        var rest = value.Substring("pkg:".Length);
        var hash = rest.IndexOf('#');
        if (hash >= 0)
        {
            rest = rest.Substring(0, hash);
        }

        var query = rest.IndexOf('?');
        if (query >= 0)
        {
            rest = rest.Substring(0, query);
        }

        var lastSlash = rest.LastIndexOf('/');
        var at = rest.LastIndexOf('@');
        string? version = null;
        if (at > lastSlash)
        {
            version = DecodeUriComponent(rest.Substring(at + 1));
            if (version is null)
            {
                return null;
            }

            rest = rest.Substring(0, at);
        }

        var segments = rest.Split('/');
        if (segments.Length < 2 || segments.Any(segment => segment.Length == 0))
        {
            return null;
        }

        var type = segments[0];
        if (!TypePattern.IsMatch(type))
        {
            return null;
        }

        return new ParsedPurl(
            $"{type.ToLowerInvariant()}/{string.Join("/", segments.Skip(1))}",
            version is not null && version.Length > 0 ? version : null);
    }

    /// <summary>ECMAScript <c>decodeURIComponent</c>; <c>null</c> where JavaScript throws <c>URIError</c>.</summary>
    public static string? DecodeUriComponent(string value)
    {
        if (value.IndexOf('%') < 0)
        {
            return value;
        }

        var builder = new StringBuilder(value.Length);
        var i = 0;
        while (i < value.Length)
        {
            var c = value[i];
            if (c != '%')
            {
                builder.Append(c);
                i++;
                continue;
            }

            if (!TryHexByte(value, i, out var first))
            {
                return null;
            }

            i += 3;
            if (first < 0x80)
            {
                builder.Append((char)first);
                continue;
            }

            int count;
            int codePoint;
            if ((first & 0xE0) == 0xC0)
            {
                count = 1;
                codePoint = first & 0x1F;
            }
            else if ((first & 0xF0) == 0xE0)
            {
                count = 2;
                codePoint = first & 0x0F;
            }
            else if ((first & 0xF8) == 0xF0)
            {
                count = 3;
                codePoint = first & 0x07;
            }
            else
            {
                return null;
            }

            for (var k = 0; k < count; k++)
            {
                if (i >= value.Length || value[i] != '%' || !TryHexByte(value, i, out var next) || (next & 0xC0) != 0x80)
                {
                    return null;
                }

                codePoint = (codePoint << 6) | (next & 0x3F);
                i += 3;
            }

            var minimum = count switch { 1 => 0x80, 2 => 0x800, _ => 0x10000 };
            if (codePoint < minimum || codePoint > 0x10FFFF || (codePoint >= 0xD800 && codePoint <= 0xDFFF))
            {
                return null;
            }

            builder.Append(char.ConvertFromUtf32(codePoint));
        }

        return builder.ToString();
    }

    private static bool TryHexByte(string value, int percent, out int result)
    {
        result = 0;
        if (percent + 2 >= value.Length)
        {
            return false;
        }

        var high = HexValue(value[percent + 1]);
        var low = HexValue(value[percent + 2]);
        if (high < 0 || low < 0)
        {
            return false;
        }

        result = (high << 4) | low;
        return true;
    }

    private static int HexValue(char c) => c switch
    {
        >= '0' and <= '9' => c - '0',
        >= 'a' and <= 'f' => c - 'a' + 10,
        >= 'A' and <= 'F' => c - 'A' + 10,
        _ => -1,
    };
}
