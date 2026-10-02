// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! A small JSON value model with ECMAScript semantics.
//!
//! The TypeScript implementation parses catalog data with `JSON.parse`, so
//! every number is an IEEE double, object keys follow ECMAScript property
//! order (array-index keys ascending first, then insertion order; a repeated
//! key keeps its first position and takes the last value), and there is no
//! nesting limit. [`Json::parse`] reproduces those rules exactly so validation
//! messages and output match byte for byte.
//!
//! Known difference: a lone UTF-16 surrogate escape (`"\ud800"`) cannot be
//! represented in a Rust `String` and is rejected as a parse error.

use std::cmp::Ordering;
use std::fmt;

/// A parsed JSON value. Numbers are always `f64`, as in ECMAScript.
#[derive(Clone, Debug)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(JsonObject),
}

/// A JSON object whose key order follows ECMAScript `OrdinaryOwnPropertyKeys`.
#[derive(Clone, Debug, Default)]
pub struct JsonObject {
    entries: Vec<(String, Json)>,
}

/// Returns the array index a property key denotes, if any (ECMAScript
/// canonical numeric string in `0..2^32-1`).
fn array_index(key: &str) -> Option<u32> {
    let bytes = key.as_bytes();
    if bytes.is_empty() || bytes.len() > 10 || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    if bytes.len() > 1 && bytes[0] == b'0' {
        return None;
    }
    let value: u64 = key.parse().ok()?;
    if value < u64::from(u32::MAX) {
        Some(value as u32)
    } else {
        None
    }
}

impl JsonObject {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets `key`. An existing key keeps its position; a new key is placed by
    /// ECMAScript property-order rules.
    pub fn insert(&mut self, key: impl Into<String>, value: Json) {
        let key = key.into();
        if let Some(existing) = self.entries.iter_mut().find(|(k, _)| *k == key) {
            existing.1 = value;
            return;
        }
        match array_index(&key) {
            Some(index) => {
                let position = self
                    .entries
                    .iter()
                    .position(|(k, _)| array_index(k).is_none_or(|other| other > index))
                    .unwrap_or(self.entries.len());
                self.entries.insert(position, (key, value));
            }
            None => self.entries.push((key, value)),
        }
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.entries.iter().any(|(k, _)| k == key)
    }

    pub fn remove(&mut self, key: &str) -> Option<Json> {
        let index = self.entries.iter().position(|(k, _)| k == key)?;
        Some(self.entries.remove(index).1)
    }

    /// Keys in ECMAScript `Object.keys` order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(k, _)| k.as_str())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Json)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl PartialEq for JsonObject {
    /// Deep equality that ignores key order (like `assert.deepEqual`).
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().all(|(k, v)| other.get(k) == Some(v))
    }
}

impl PartialEq for Json {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Json::Null, Json::Null) => true,
            (Json::Bool(a), Json::Bool(b)) => a == b,
            (Json::Number(a), Json::Number(b)) => a == b,
            (Json::String(a), Json::String(b)) => a == b,
            (Json::Array(a), Json::Array(b)) => a == b,
            (Json::Object(a), Json::Object(b)) => a == b,
            _ => false,
        }
    }
}

impl From<&str> for Json {
    fn from(value: &str) -> Self {
        Json::String(value.to_string())
    }
}

impl From<String> for Json {
    fn from(value: String) -> Self {
        Json::String(value)
    }
}

impl From<f64> for Json {
    fn from(value: f64) -> Self {
        Json::Number(value)
    }
}

impl From<bool> for Json {
    fn from(value: bool) -> Self {
        Json::Bool(value)
    }
}

impl From<JsonObject> for Json {
    fn from(value: JsonObject) -> Self {
        Json::Object(value)
    }
}

impl From<Vec<Json>> for Json {
    fn from(value: Vec<Json>) -> Self {
        Json::Array(value)
    }
}

impl Json {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Json>> {
        match self {
            Json::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&JsonObject> {
        match self {
            Json::Object(o) => Some(o),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }

    /// Property lookup; `None` for a missing key or a non-object value.
    pub fn get(&self, key: &str) -> Option<&Json> {
        self.as_object().and_then(|o| o.get(key))
    }

    /// Parses JSON text with `JSON.parse` semantics.
    pub fn parse(text: &str) -> Result<Json, JsonError> {
        let mut parser = Parser {
            text,
            bytes: text.as_bytes(),
            pos: 0,
        };
        parser.skip_ws();
        let value = parser.value()?;
        parser.skip_ws();
        if parser.pos != parser.bytes.len() {
            return Err(parser.unexpected());
        }
        Ok(value)
    }

    /// `JSON.stringify(value)` (compact).
    pub fn to_compact_string(&self) -> String {
        let mut out = String::new();
        write_value(self, &mut out, None, 0);
        out
    }

    /// `JSON.stringify(value, null, 2)`.
    pub fn to_pretty_string(&self) -> String {
        let mut out = String::new();
        write_value(self, &mut out, Some(2), 0);
        out
    }
}

impl fmt::Display for Json {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_compact_string())
    }
}

/// JSON syntax error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonError {
    pub message: String,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for JsonError {}

struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn unexpected(&self) -> JsonError {
        match self.text[self.pos..].chars().next() {
            None => JsonError {
                message: "Unexpected end of JSON input".to_string(),
            },
            Some(c) => JsonError {
                message: format!("Unexpected token '{c}' in JSON at position {}", self.pos),
            },
        }
    }

    fn error(&self, message: &str) -> JsonError {
        JsonError {
            message: format!("{message} in JSON at position {}", self.pos),
        }
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.bytes.get(self.pos) {
            if matches!(b, b' ' | b'\t' | b'\n' | b'\r') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, JsonError> {
        if self.text[self.pos..].starts_with(word) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.unexpected())
        }
    }

    fn value(&mut self) -> Result<Json, JsonError> {
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Json::String(self.string()?)),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'n') => self.literal("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.unexpected()),
        }
    }

    fn object(&mut self) -> Result<Json, JsonError> {
        self.pos += 1;
        let mut object = JsonObject::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Object(object));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(self.unexpected());
            }
            let key = self.string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(self.unexpected());
            }
            self.pos += 1;
            self.skip_ws();
            let value = self.value()?;
            object.insert(key, value);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Object(object));
                }
                _ => return Err(self.unexpected()),
            }
        }
    }

    fn array(&mut self) -> Result<Json, JsonError> {
        self.pos += 1;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(self.unexpected()),
            }
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.pos;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        self.pos - start
    }

    fn number(&mut self) -> Result<Json, JsonError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return Err(self.unexpected()),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if self.digits() == 0 {
                return Err(self.unexpected());
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if self.digits() == 0 {
                return Err(self.unexpected());
            }
        }
        // Rust's float parser is correctly rounded (like ECMAScript
        // StringToNumber) and overflows to infinity like JSON.parse.
        let lexeme = &self.text[start..self.pos];
        lexeme
            .parse::<f64>()
            .map(Json::Number)
            .map_err(|_| self.error("Invalid number"))
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let slice = self
            .bytes
            .get(self.pos..self.pos + 4)
            .ok_or_else(|| self.error("Bad Unicode escape"))?;
        let mut value = 0u32;
        for b in slice {
            let digit = (*b as char)
                .to_digit(16)
                .ok_or_else(|| self.error("Bad Unicode escape"))?;
            value = value * 16 + digit;
        }
        self.pos += 4;
        Ok(value)
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            let start = self.pos;
            while let Some(b) = self.peek() {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.pos += 1;
            }
            out.push_str(&self.text[start..self.pos]);
            match self.peek() {
                None => return Err(self.error("Unterminated string")),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let escape = self
                        .peek()
                        .ok_or_else(|| self.error("Bad escaped character"))?;
                    self.pos += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let unit = self.hex4()?;
                            if (0xD800..0xDC00).contains(&unit) {
                                if self.text[self.pos..].starts_with("\\u") {
                                    let save = self.pos;
                                    self.pos += 2;
                                    let low = self.hex4()?;
                                    if (0xDC00..0xE000).contains(&low) {
                                        let code =
                                            0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00);
                                        out.push(
                                            char::from_u32(code).expect("valid surrogate pair"),
                                        );
                                        continue;
                                    }
                                    self.pos = save;
                                }
                                return Err(self.error("Lone surrogate escapes are not supported"));
                            }
                            if (0xDC00..0xE000).contains(&unit) {
                                return Err(self.error("Lone surrogate escapes are not supported"));
                            }
                            out.push(char::from_u32(unit).expect("BMP scalar"));
                        }
                        _ => {
                            self.pos -= 1;
                            return Err(self.error("Bad escaped character"));
                        }
                    }
                }
                Some(_) => return Err(self.error("Bad control character in string literal")),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Serialization
// ---------------------------------------------------------------------------

/// `JSON.stringify` of a string: quotes, `\"`, `\\`, short escapes, and
/// `\u00XX` (lower-case hex) for other control characters. Everything else is
/// emitted literally.
pub fn quote_json_string(value: &str, out: &mut String) {
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// ECMAScript `Number::toString(x)` (radix 10).
pub fn js_number_to_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value == 0.0 {
        return "0".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    let mut out = String::new();
    if value < 0.0 {
        out.push('-');
    }
    // Rust's `{:e}` yields the shortest round-tripping digits (closest to the
    // exact value), which is the digit string ECMAScript specifies.
    let formatted = format!("{:e}", value.abs());
    let (mantissa, exponent) = formatted.split_once('e').expect("exponent form");
    let exponent: i32 = exponent.parse().expect("integer exponent");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let n = exponent + 1;
    if k <= n && n <= 21 {
        out.push_str(&digits);
        out.extend(std::iter::repeat_n('0', (n - k) as usize));
    } else if 0 < n && n <= 21 {
        out.push_str(&digits[..n as usize]);
        out.push('.');
        out.push_str(&digits[n as usize..]);
    } else if -6 < n && n <= 0 {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', (-n) as usize));
        out.push_str(&digits);
    } else {
        let e = n - 1;
        out.push_str(&digits[..1]);
        if k > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if e < 0 { '-' } else { '+' });
        out.push_str(&e.abs().to_string());
    }
    out
}

/// A number as `JSON.stringify` writes it (`null` for non-finite values).
pub fn json_number(value: f64) -> String {
    if value.is_finite() {
        js_number_to_string(value)
    } else {
        "null".to_string()
    }
}

fn newline(out: &mut String, indent: Option<usize>, depth: usize) {
    if let Some(width) = indent {
        out.push('\n');
        out.extend(std::iter::repeat_n(' ', width * depth));
    }
}

fn write_value(value: &Json, out: &mut String, indent: Option<usize>, depth: usize) {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Number(n) => out.push_str(&json_number(*n)),
        Json::String(s) => quote_json_string(s, out),
        Json::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                newline(out, indent, depth + 1);
                write_value(item, out, indent, depth + 1);
            }
            newline(out, indent, depth);
            out.push(']');
        }
        Json::Object(object) => {
            if object.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (index, (key, item)) in object.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                newline(out, indent, depth + 1);
                quote_json_string(key, out);
                out.push(':');
                if indent.is_some() {
                    out.push(' ');
                }
                write_value(item, out, indent, depth + 1);
            }
            newline(out, indent, depth);
            out.push('}');
        }
    }
}

/// Compares two strings by UTF-16 code units (ECMAScript `<` and default `sort`).
pub fn cmp_utf16(left: &str, right: &str) -> Ordering {
    left.encode_utf16().cmp(right.encode_utf16())
}

/// Canonical JSON: keys sorted by UTF-16 code units at every level, arrays in
/// order, no insignificant whitespace, `JSON.stringify` scalars.
pub fn canonical_json(value: &Json) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Json, out: &mut String) {
    match value {
        Json::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        Json::Object(object) => {
            let mut keys: Vec<(&str, &Json)> = object.iter().collect();
            keys.sort_by(|a, b| cmp_utf16(a.0, b.0));
            out.push('{');
            for (index, (key, item)) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                quote_json_string(key, out);
                out.push(':');
                write_canonical(item, out);
            }
            out.push('}');
        }
        scalar => write_value(scalar, out, None, 0),
    }
}

/// ECMAScript `String(value)` for a JSON value (used where TypeScript
/// interpolates unvalidated data into a message).
pub fn js_to_string(value: &Json) -> String {
    match value {
        Json::Null => "null".to_string(),
        Json::Bool(b) => b.to_string(),
        Json::Number(n) => js_number_to_string(*n),
        Json::String(s) => s.clone(),
        Json::Array(items) => items
            .iter()
            .map(|item| {
                if item.is_null() {
                    String::new()
                } else {
                    js_to_string(item)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Json::Object(_) => "[object Object]".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_to_string_follows_ecmascript() {
        let cases: &[(f64, &str)] = &[
            (0.0, "0"),
            (-0.0, "0"),
            (1.0, "1"),
            (1.5, "1.5"),
            (0.1, "0.1"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (1e-7, "1e-7"),
            (0.000001, "0.000001"),
            (2.5e-7, "2.5e-7"),
            (123456789012345680000.0, "123456789012345680000"),
            (5e-324, "5e-324"),
            (1.7976931348623157e308, "1.7976931348623157e+308"),
            (-1234.5678, "-1234.5678"),
            (0.30000000000000004, "0.30000000000000004"),
            (1.2345e-5, "0.000012345"),
            (123e-20, "1.23e-18"),
        ];
        for (value, expected) in cases {
            assert_eq!(js_number_to_string(*value), *expected, "{value:e}");
        }
    }

    #[test]
    fn parse_follows_json_parse() {
        let value = Json::parse(r#"{"b":1,"2":2,"a":3,"1":4,"b":5}"#).unwrap();
        let keys: Vec<&str> = value.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["1", "2", "b", "a"]);
        assert_eq!(value.get("b"), Some(&Json::Number(5.0)));
        assert_eq!(
            Json::parse("12345678901234567890").unwrap(),
            Json::Number(12345678901234567000.0)
        );
        assert_eq!(Json::parse("1e400").unwrap(), Json::Number(f64::INFINITY));
        assert_eq!(canonical_json(&Json::parse("1e400").unwrap()), "null");
        for bad in [
            "",
            "01",
            "1.",
            "[1,]",
            "{\"a\":1,}",
            "\u{feff}{}",
            "\"\\x\"",
            "tru",
            "\"\u{1}\"",
            "\"\\ud800\"",
        ] {
            assert!(Json::parse(bad).is_err(), "{bad:?}");
        }
        assert!(Json::parse(" \t\r\n[ ] ").is_ok());
        assert_eq!(
            Json::parse(r#""\ud83d\ude00""#).unwrap(),
            Json::String("\u{1F600}".into())
        );
    }

    #[test]
    fn pretty_matches_json_stringify_indent_2() {
        let value = Json::parse(r#"{"a":[],"b":{},"c":[1,{"d":null}]}"#).unwrap();
        assert_eq!(
            value.to_pretty_string(),
            "{\n  \"a\": [],\n  \"b\": {},\n  \"c\": [\n    1,\n    {\n      \"d\": null\n    }\n  ]\n}"
        );
    }
}
