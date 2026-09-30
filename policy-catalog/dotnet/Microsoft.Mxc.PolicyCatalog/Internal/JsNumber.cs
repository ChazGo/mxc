// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using System.Globalization;
using System.Text;

namespace Microsoft.Mxc.PolicyCatalog.Internal;

/// <summary>ECMAScript <c>Number::toString</c> for IEEE doubles.</summary>
internal static class JsNumber
{
    /// <summary><c>String(value)</c> in JavaScript.</summary>
    public static string ToString(double value)
    {
        if (double.IsNaN(value))
        {
            return "NaN";
        }

        if (double.IsPositiveInfinity(value))
        {
            return "Infinity";
        }

        if (double.IsNegativeInfinity(value))
        {
            return "-Infinity";
        }

        if (value == 0)
        {
            return "0";
        }

        var negative = value < 0;
        // "R" yields the shortest round-trip digits on .NET Core 3.0 and later.
        var text = Math.Abs(value).ToString("R", CultureInfo.InvariantCulture);
        var (digits, n) = Decompose(text);
        var k = digits.Length;
        var builder = new StringBuilder();
        if (negative)
        {
            builder.Append('-');
        }

        if (k <= n && n <= 21)
        {
            builder.Append(digits).Append('0', n - k);
        }
        else if (0 < n && n <= 21)
        {
            builder.Append(digits, 0, n).Append('.').Append(digits, n, k - n);
        }
        else if (-6 < n && n <= 0)
        {
            builder.Append("0.").Append('0', -n).Append(digits);
        }
        else
        {
            var exponent = n - 1;
            builder.Append(digits[0]);
            if (k > 1)
            {
                builder.Append('.').Append(digits, 1, k - 1);
            }

            builder.Append('e').Append(exponent >= 0 ? '+' : '-').Append(Math.Abs(exponent).ToString(CultureInfo.InvariantCulture));
        }

        return builder.ToString();
    }

    /// <summary><c>JSON.stringify(value)</c> for a number: non-finite values are <c>null</c>.</summary>
    public static string ToJson(double value) => double.IsFinite(value) ? ToString(value) : "null";

    /// <summary>
    /// Splits a positive .NET round-trip string into significant digits (no
    /// leading/trailing zeros) and the decimal exponent <c>n</c> such that the
    /// value is <c>0.digits × 10^n</c>.
    /// </summary>
    private static (string Digits, int N) Decompose(string text)
    {
        var exponent = 0;
        var e = text.IndexOfAny(new[] { 'E', 'e' });
        var mantissa = text;
        if (e >= 0)
        {
            exponent = int.Parse(text.AsSpan(e + 1), NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture);
            mantissa = text.Substring(0, e);
        }

        var dot = mantissa.IndexOf('.');
        var integerPart = dot >= 0 ? mantissa.Substring(0, dot) : mantissa;
        var fractionPart = dot >= 0 ? mantissa.Substring(dot + 1) : string.Empty;
        var all = integerPart + fractionPart;
        var pointPosition = integerPart.Length + exponent;
        var leading = 0;
        while (leading < all.Length - 1 && all[leading] == '0')
        {
            leading++;
        }

        all = all.Substring(leading);
        pointPosition -= leading;
        all = all.TrimEnd('0');
        if (all.Length == 0)
        {
            all = "0";
        }

        return (all, pointPosition);
    }
}
