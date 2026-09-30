// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using System.Globalization;
using System.Text;
using System.Text.Json;

namespace Microsoft.Mxc.PolicyCatalog.Internal;

/// <summary>
/// A JSON value with JavaScript semantics: every number is an IEEE double, and
/// object keys follow ECMAScript own-property order (array-index keys ascending,
/// then the remaining keys in insertion order; a repeated key keeps its first
/// position and takes the last value, as <c>JSON.parse</c> does).
/// </summary>
internal abstract class JsonValue
{
}

internal sealed class JsonNull : JsonValue
{
    public static readonly JsonNull Instance = new();

    private JsonNull()
    {
    }
}

internal sealed class JsonBool : JsonValue
{
    public static readonly JsonBool True = new(true);
    public static readonly JsonBool False = new(false);

    private JsonBool(bool value) => Value = value;

    public bool Value { get; }

    public static JsonBool Of(bool value) => value ? True : False;
}

internal sealed class JsonNumber : JsonValue
{
    public JsonNumber(double value) => Value = value;

    public double Value { get; }
}

internal sealed class JsonString : JsonValue
{
    public JsonString(string value) => Value = value;

    public string Value { get; }
}

internal sealed class JsonArray : JsonValue
{
    public JsonArray()
    {
        Items = new List<JsonValue>();
    }

    public JsonArray(IEnumerable<JsonValue> items)
    {
        Items = new List<JsonValue>(items);
    }

    public List<JsonValue> Items { get; }

    public int Count => Items.Count;

    public JsonValue this[int index] => Items[index];
}

internal sealed class JsonObject : JsonValue
{
    private readonly List<string> _insertion = new();
    private readonly Dictionary<string, JsonValue> _values = new(StringComparer.Ordinal);

    /// <summary>Own keys in ECMAScript <c>OrdinaryOwnPropertyKeys</c> order.</summary>
    public IReadOnlyList<string> Keys
    {
        get
        {
            var indices = new List<(uint Index, string Key)>();
            var others = new List<string>();
            foreach (var key in _insertion)
            {
                if (TryArrayIndex(key, out var index))
                {
                    indices.Add((index, key));
                }
                else
                {
                    others.Add(key);
                }
            }

            if (indices.Count == 0)
            {
                return others;
            }

            indices.Sort((a, b) => a.Index.CompareTo(b.Index));
            var ordered = new List<string>(_insertion.Count);
            ordered.AddRange(indices.Select(i => i.Key));
            ordered.AddRange(others);
            return ordered;
        }
    }

    public int Count => _insertion.Count;

    public bool Has(string key) => _values.ContainsKey(key);

    /// <summary>The own property value, or <c>null</c> when absent (JavaScript <c>undefined</c>).</summary>
    public JsonValue? Get(string key) => _values.TryGetValue(key, out var value) ? value : null;

    public void Set(string key, JsonValue value)
    {
        if (!_values.ContainsKey(key))
        {
            _insertion.Add(key);
        }

        _values[key] = value;
    }

    public JsonObject With(string key, JsonValue? value)
    {
        if (value is not null)
        {
            Set(key, value);
        }

        return this;
    }

    internal static bool TryArrayIndex(string key, out uint index)
    {
        index = 0;
        if (key.Length == 0 || key.Length > 10)
        {
            return false;
        }

        if (key.Length > 1 && key[0] == '0')
        {
            return false;
        }

        foreach (var c in key)
        {
            if (c < '0' || c > '9')
            {
                return false;
            }
        }

        if (!ulong.TryParse(key, NumberStyles.None, CultureInfo.InvariantCulture, out var value) || value > 4294967294UL)
        {
            return false;
        }

        index = (uint)value;
        return true;
    }
}

/// <summary>Parsing and <c>JSON.stringify</c>-compatible serialization.</summary>
internal static class JsonText
{
    private static readonly JsonReaderOptions ReaderOptions = new()
    {
        AllowTrailingCommas = false,
        CommentHandling = JsonCommentHandling.Disallow,
        MaxDepth = 4096,
    };

    /// <summary>Decodes UTF-8 bytes the way Node's <c>readFileSync(..., 'utf8')</c> does (a BOM is kept; invalid bytes become U+FFFD).</summary>
    public static string DecodeUtf8(byte[] bytes) => Encoding.UTF8.GetString(bytes);

    /// <summary>Parses JSON text like <c>JSON.parse</c>. Throws <see cref="JsonException"/> on invalid input.</summary>
    public static JsonValue Parse(string text)
    {
        // Re-encoding a .NET string can only produce well-formed UTF-8; lone
        // surrogates in the text itself are replaced, which is also invalid JSON
        // outside a string and rare inside one.
        var bytes = Encoding.UTF8.GetBytes(text);
        var reader = new Utf8JsonReader(bytes, ReaderOptions);
        if (!reader.Read())
        {
            throw new JsonException("Unexpected end of JSON input");
        }

        var value = ReadValue(ref reader);
        if (reader.Read())
        {
            throw new JsonException($"Unexpected non-whitespace character after JSON at byte {reader.TokenStartIndex}");
        }

        return value;
    }

    private static JsonValue ReadValue(ref Utf8JsonReader reader)
    {
        switch (reader.TokenType)
        {
            case JsonTokenType.Null:
                return JsonNull.Instance;
            case JsonTokenType.True:
                return JsonBool.True;
            case JsonTokenType.False:
                return JsonBool.False;
            case JsonTokenType.Number:
                return new JsonNumber(double.Parse(Encoding.UTF8.GetString(reader.ValueSpan), NumberStyles.Float, CultureInfo.InvariantCulture));
            case JsonTokenType.String:
                return new JsonString(ReadString(ref reader));
            case JsonTokenType.StartArray:
            {
                var array = new JsonArray();
                while (reader.Read() && reader.TokenType != JsonTokenType.EndArray)
                {
                    array.Items.Add(ReadValue(ref reader));
                }

                return array;
            }

            case JsonTokenType.StartObject:
            {
                var obj = new JsonObject();
                while (reader.Read() && reader.TokenType != JsonTokenType.EndObject)
                {
                    var key = ReadString(ref reader);
                    reader.Read();
                    obj.Set(key, ReadValue(ref reader));
                }

                return obj;
            }

            default:
                throw new JsonException($"Unexpected token {reader.TokenType}");
        }
    }

    /// <summary>Unescapes a string token without rejecting lone surrogate escapes (JavaScript strings may hold them).</summary>
    private static string ReadString(ref Utf8JsonReader reader)
    {
        var span = reader.ValueSpan;
        if (!reader.ValueIsEscaped)
        {
            return Encoding.UTF8.GetString(span);
        }

        var builder = new StringBuilder(span.Length);
        var runStart = 0;
        var i = 0;
        while (i < span.Length)
        {
            if (span[i] != (byte)'\\')
            {
                i++;
                continue;
            }

            builder.Append(Encoding.UTF8.GetString(span.Slice(runStart, i - runStart)));
            var escape = (char)span[i + 1];
            switch (escape)
            {
                case '"': builder.Append('"'); i += 2; break;
                case '\\': builder.Append('\\'); i += 2; break;
                case '/': builder.Append('/'); i += 2; break;
                case 'b': builder.Append('\b'); i += 2; break;
                case 'f': builder.Append('\f'); i += 2; break;
                case 'n': builder.Append('\n'); i += 2; break;
                case 'r': builder.Append('\r'); i += 2; break;
                case 't': builder.Append('\t'); i += 2; break;
                case 'u':
                    builder.Append((char)int.Parse(Encoding.ASCII.GetString(span.Slice(i + 2, 4)), NumberStyles.HexNumber, CultureInfo.InvariantCulture));
                    i += 6;
                    break;
                default:
                    throw new JsonException($"Bad escaped character '{escape}'");
            }

            runStart = i;
        }

        builder.Append(Encoding.UTF8.GetString(span.Slice(runStart)));
        return builder.ToString();
    }

    /// <summary><c>JSON.stringify(value)</c> or, with <paramref name="indent"/>, <c>JSON.stringify(value, null, indent)</c>.</summary>
    public static string Stringify(JsonValue value, int indent = 0)
    {
        var builder = new StringBuilder();
        Write(builder, value, indent, 0);
        return builder.ToString();
    }

    private static void Write(StringBuilder builder, JsonValue value, int indent, int depth)
    {
        switch (value)
        {
            case JsonNull:
                builder.Append("null");
                break;
            case JsonBool b:
                builder.Append(b.Value ? "true" : "false");
                break;
            case JsonNumber n:
                builder.Append(JsNumber.ToJson(n.Value));
                break;
            case JsonString s:
                Quote(builder, s.Value);
                break;
            case JsonArray a:
                if (a.Count == 0)
                {
                    builder.Append("[]");
                    break;
                }

                builder.Append('[');
                for (var i = 0; i < a.Count; i++)
                {
                    if (i > 0)
                    {
                        builder.Append(',');
                    }

                    NewLine(builder, indent, depth + 1);
                    Write(builder, a[i], indent, depth + 1);
                }

                NewLine(builder, indent, depth);
                builder.Append(']');
                break;
            case JsonObject o:
                var keys = o.Keys;
                if (keys.Count == 0)
                {
                    builder.Append("{}");
                    break;
                }

                builder.Append('{');
                for (var i = 0; i < keys.Count; i++)
                {
                    if (i > 0)
                    {
                        builder.Append(',');
                    }

                    NewLine(builder, indent, depth + 1);
                    Quote(builder, keys[i]);
                    builder.Append(indent > 0 ? ": " : ":");
                    Write(builder, o.Get(keys[i])!, indent, depth + 1);
                }

                NewLine(builder, indent, depth);
                builder.Append('}');
                break;
            default:
                throw new InvalidOperationException("unknown JSON value");
        }
    }

    private static void NewLine(StringBuilder builder, int indent, int depth)
    {
        if (indent > 0)
        {
            builder.Append('\n');
            builder.Append(' ', indent * depth);
        }
    }

    /// <summary><c>JSON.stringify(string)</c>: ECMAScript <c>QuoteJSONString</c>, including well-formed lone-surrogate escapes.</summary>
    public static string Quote(string value)
    {
        var builder = new StringBuilder(value.Length + 2);
        Quote(builder, value);
        return builder.ToString();
    }

    public static void Quote(StringBuilder builder, string value)
    {
        builder.Append('"');
        for (var i = 0; i < value.Length; i++)
        {
            var c = value[i];
            switch (c)
            {
                case '"': builder.Append("\\\""); continue;
                case '\\': builder.Append("\\\\"); continue;
                case '\b': builder.Append("\\b"); continue;
                case '\f': builder.Append("\\f"); continue;
                case '\n': builder.Append("\\n"); continue;
                case '\r': builder.Append("\\r"); continue;
                case '\t': builder.Append("\\t"); continue;
            }

            if (c < 0x20)
            {
                AppendUnicodeEscape(builder, c);
            }
            else if (char.IsHighSurrogate(c))
            {
                if (i + 1 < value.Length && char.IsLowSurrogate(value[i + 1]))
                {
                    builder.Append(c).Append(value[i + 1]);
                    i++;
                }
                else
                {
                    AppendUnicodeEscape(builder, c);
                }
            }
            else if (char.IsLowSurrogate(c))
            {
                AppendUnicodeEscape(builder, c);
            }
            else
            {
                builder.Append(c);
            }
        }

        builder.Append('"');
    }

    private static void AppendUnicodeEscape(StringBuilder builder, char c)
    {
        builder.Append("\\u").Append(((int)c).ToString("x4", CultureInfo.InvariantCulture));
    }
}
