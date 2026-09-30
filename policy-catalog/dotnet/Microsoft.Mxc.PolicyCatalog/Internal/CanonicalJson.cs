// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using System.Security.Cryptography;
using System.Text;

namespace Microsoft.Mxc.PolicyCatalog.Internal;

/// <summary>
/// Canonical JSON used for revision integrity digests (TypeScript <c>canonicalJson</c>):
/// object keys sorted by UTF-16 code units at every level, arrays in order, no
/// whitespace, <c>JSON.stringify</c> string escaping, ECMAScript number formatting.
/// </summary>
internal static class CanonicalJson
{
    public static string Serialize(JsonValue value)
    {
        var builder = new StringBuilder();
        Write(builder, value);
        return builder.ToString();
    }

    private static void Write(StringBuilder builder, JsonValue value)
    {
        switch (value)
        {
            case JsonArray array:
                builder.Append('[');
                for (var i = 0; i < array.Count; i++)
                {
                    if (i > 0)
                    {
                        builder.Append(',');
                    }

                    Write(builder, array[i]);
                }

                builder.Append(']');
                break;
            case JsonObject obj:
                var keys = obj.Keys.ToList();
                keys.Sort(string.CompareOrdinal);
                builder.Append('{');
                for (var i = 0; i < keys.Count; i++)
                {
                    if (i > 0)
                    {
                        builder.Append(',');
                    }

                    JsonText.Quote(builder, keys[i]);
                    builder.Append(':');
                    Write(builder, obj.Get(keys[i])!);
                }

                builder.Append('}');
                break;
            default:
                builder.Append(JsonText.Stringify(value));
                break;
        }
    }

    /// <summary>Lower-case hex SHA-256 of the UTF-8 canonical form.</summary>
    public static string Sha256(JsonValue value)
    {
        // Lone surrogates become U+FFFD in UTF-8, exactly as Node's hash update does.
        var bytes = Encoding.UTF8.GetBytes(Serialize(value));
        return Convert.ToHexString(SHA256.HashData(bytes)).ToLowerInvariant();
    }
}
