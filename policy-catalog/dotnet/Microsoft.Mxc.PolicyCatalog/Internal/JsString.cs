// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using System.Globalization;
using System.Text;

namespace Microsoft.Mxc.PolicyCatalog.Internal;

/// <summary>JavaScript string semantics the TypeScript reference relies on.</summary>
internal static class JsString
{
    /// <summary>JavaScript <c>\s</c> / <c>String.prototype.trim</c> whitespace (WhiteSpace + LineTerminator).</summary>
    public const string WhitespaceClass = "[\\t\\n\\v\\f\\r \\u00a0\\u1680\\u2000-\\u200a\\u2028\\u2029\\u202f\\u205f\\u3000\\ufeff]";

    /// <summary>JavaScript <c>.</c> without the <c>s</c> flag: anything except a line terminator.</summary>
    public const string AnyButLineTerminator = "[^\\n\\r\\u2028\\u2029]";

    public static bool IsWhitespace(char c) => c switch
    {
        '\t' or '\n' or '\v' or '\f' or '\r' or ' ' or '\u00a0' or '\u1680' or '\u2028' or '\u2029' or '\u202f' or '\u205f' or '\u3000' or '\ufeff' => true,
        _ => c >= '\u2000' && c <= '\u200a',
    };

    /// <summary><c>String.prototype.trim</c>.</summary>
    public static string Trim(string value)
    {
        var start = 0;
        var end = value.Length;
        while (start < end && IsWhitespace(value[start]))
        {
            start++;
        }

        while (end > start && IsWhitespace(value[end - 1]))
        {
            end--;
        }

        return value.Substring(start, end - start);
    }

    // Code points whose lower-case mapping is newer than the Unicode data in
    // .NET 8's invariant casing (Unicode 16 additions), taken from V8/ICU 78.
    private static readonly Dictionary<int, int> NewerLowerCase = BuildNewerLowerCase();

    private static Dictionary<int, int> BuildNewerLowerCase()
    {
        var map = new Dictionary<int, int>
        {
            [0x1C89] = 0x1C8A,
            [0xA7CB] = 0x0264,
            [0xA7CC] = 0xA7CD,
            [0xA7CE] = 0xA7CF,
            [0xA7D2] = 0xA7D3,
            [0xA7D4] = 0xA7D5,
            [0xA7DA] = 0xA7DB,
            [0xA7DC] = 0x019B,
        };
        for (var cp = 0x10D50; cp <= 0x10D65; cp++)
        {
            map[cp] = cp + 0x20;
        }

        for (var cp = 0x16EA0; cp <= 0x16EB8; cp++)
        {
            map[cp] = cp + 0x1B;
        }

        return map;
    }

    /// <summary>
    /// <c>String.prototype.toLowerCase</c>: Unicode default (locale-independent)
    /// lower-casing, including the unconditional <c>U+0130</c> expansion and the
    /// context-sensitive Greek final sigma, which invariant lower-casing omits.
    /// </summary>
    public static string ToLower(string value)
    {
        var simple = value.ToLowerInvariant();
        var needsFixup = false;
        foreach (var c in value)
        {
            if (c == '\u0130' || c == '\u03a3' || char.IsSurrogate(c) || c == '\u1c89' || (c >= '\ua7cb' && c <= '\ua7dc'))
            {
                needsFixup = true;
                break;
            }
        }

        if (!needsFixup)
        {
            return simple;
        }

        var builder = new StringBuilder(value.Length + 4);
        for (var i = 0; i < value.Length; i++)
        {
            var c = value[i];
            if (c == '\u0130')
            {
                builder.Append("i\u0307");
                continue;
            }

            if (c == '\u03a3')
            {
                builder.Append(IsFinalSigma(value, i) ? '\u03c2' : '\u03c3');
                continue;
            }

            int cp = c;
            var width = 1;
            if (char.IsHighSurrogate(c) && i + 1 < value.Length && char.IsLowSurrogate(value[i + 1]))
            {
                cp = char.ConvertToUtf32(c, value[i + 1]);
                width = 2;
            }

            if (NewerLowerCase.TryGetValue(cp, out var lower))
            {
                builder.Append(char.ConvertFromUtf32(lower));
            }
            else
            {
                builder.Append(value.Substring(i, width).ToLowerInvariant());
            }

            i += width - 1;
        }

        return builder.ToString();
    }

    // Unicode Final_Sigma: preceded by a cased letter (skipping case-ignorables)
    // and not followed by one.
    private static bool IsFinalSigma(string value, int index)
    {
        var before = false;
        for (var i = index - 1; i >= 0; i--)
        {
            var (cp, start) = CodePointBefore(value, i);
            i = start;
            if (IsCaseIgnorable(cp))
            {
                continue;
            }

            before = IsCased(cp);
            break;
        }

        if (!before)
        {
            return false;
        }

        for (var i = index + 1; i < value.Length; i++)
        {
            var cp = CodePointAt(value, i, out var width);
            i += width - 1;
            if (IsCaseIgnorable(cp))
            {
                continue;
            }

            return !IsCased(cp);
        }

        return true;
    }

    private static (int CodePoint, int Start) CodePointBefore(string value, int index)
    {
        if (char.IsLowSurrogate(value[index]) && index > 0 && char.IsHighSurrogate(value[index - 1]))
        {
            return (char.ConvertToUtf32(value[index - 1], value[index]), index - 1);
        }

        return (value[index], index);
    }

    private static int CodePointAt(string value, int index, out int width)
    {
        if (char.IsHighSurrogate(value[index]) && index + 1 < value.Length && char.IsLowSurrogate(value[index + 1]))
        {
            width = 2;
            return char.ConvertToUtf32(value[index], value[index + 1]);
        }

        width = 1;
        return value[index];
    }

    private static bool IsCased(int cp)
    {
        var category = CharUnicodeInfo.GetUnicodeCategory(cp);
        return category is UnicodeCategory.UppercaseLetter or UnicodeCategory.LowercaseLetter or UnicodeCategory.TitlecaseLetter
            || (cp >= 0x02B0 && cp <= 0x02B8) || (cp >= 0x02C0 && cp <= 0x02C1) || (cp >= 0x02E0 && cp <= 0x02E4)
            || cp == 0x0345 || cp == 0x037A || (cp >= 0x2160 && cp <= 0x217F) || (cp >= 0x24B6 && cp <= 0x24E9);
    }

    private static bool IsCaseIgnorable(int cp)
    {
        var category = CharUnicodeInfo.GetUnicodeCategory(cp);
        return category is UnicodeCategory.NonSpacingMark or UnicodeCategory.EnclosingMark or UnicodeCategory.Format
                or UnicodeCategory.ModifierLetter or UnicodeCategory.ModifierSymbol
            || cp is 0x0027 or 0x002E or 0x003A or 0x005E or 0x0060 or 0x00A8 or 0x00AD or 0x00AF or 0x00B4 or 0x00B7 or 0x00B8
                or 0x2018 or 0x2019 or 0x2024 or 0x2027 or 0xFE13 or 0xFE52 or 0xFE55 or 0xFF07 or 0xFF0E or 0xFF1A;
    }

    /// <summary>JavaScript <c>&lt;</c> on strings: UTF-16 code unit order.</summary>
    public static int Compare(string left, string right) => Math.Sign(string.CompareOrdinal(left, right));
}
