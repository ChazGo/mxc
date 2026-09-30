// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Package URL parsing reduced to identity matching (TypeScript `src/purl.ts`).

/// A parsed package URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedPurl {
    /// `type/[namespace/]name` with the type lower-cased. Version-free.
    pub key: String,
    pub version: Option<String>,
}

/// ECMAScript `decodeURIComponent`; `None` where it throws `URIError`.
pub fn decode_uri_component(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            let high = (hex[0] as char).to_digit(16)?;
            let low = (hex[1] as char).to_digit(16)?;
            out.push((high * 16 + low) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    // Literal characters are complete UTF-8 sequences, so the combined bytes
    // are valid exactly when every escaped sequence is (no overlong forms,
    // surrogates, or out-of-range code points), matching the URIError rules.
    String::from_utf8(out).ok()
}

fn is_purl_type(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '+' || c == '-')
}

/// Parses `pkg:type/namespace/name@version?qualifiers#subpath`.
pub fn parse_purl(value: &str) -> Option<ParsedPurl> {
    let rest = value.strip_prefix("pkg:")?;
    let rest = rest.split('#').next().unwrap_or("");
    let mut rest = rest.split('?').next().unwrap_or("");
    let last_slash = rest.rfind('/');
    let at = rest.rfind('@');
    let mut version = None;
    if let Some(at) = at {
        if last_slash.is_none_or(|slash| at > slash) {
            version = Some(decode_uri_component(&rest[at + 1..])?);
            rest = &rest[..at];
        }
    }
    let segments: Vec<&str> = rest.split('/').collect();
    if segments.len() < 2 || segments.iter().any(|s| s.is_empty()) {
        return None;
    }
    if !is_purl_type(segments[0]) {
        return None;
    }
    Some(ParsedPurl {
        key: format!("{}/{}", segments[0].to_ascii_lowercase(), segments[1..].join("/")),
        version: version.filter(|v| !v.is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn purl(key: &str, version: Option<&str>) -> Option<ParsedPurl> {
        Some(ParsedPurl {
            key: key.into(),
            version: version.map(Into::into),
        })
    }

    #[test]
    fn parses_like_typescript() {
        assert_eq!(parse_purl("pkg:npm/npm@10.9.0"), purl("npm/npm", Some("10.9.0")));
        assert_eq!(
            parse_purl("pkg:NPM/%40scope/pkg@1.0.0?x=y#sub"),
            purl("npm/%40scope/pkg", Some("1.0.0"))
        );
        assert_eq!(parse_purl("pkg:npm/npm"), purl("npm/npm", None));
        assert_eq!(parse_purl("pkg:npm/npm@"), purl("npm/npm", None));
        assert_eq!(parse_purl("pkg:npm/a@%E2%82%AC"), purl("npm/a", Some("\u{20ac}")));
        assert_eq!(parse_purl("npm/npm"), None);
        assert_eq!(parse_purl("pkg:npm"), None);
        assert_eq!(parse_purl("pkg:npm//x"), None);
        assert_eq!(parse_purl("pkg:1pm/x"), None);
        assert_eq!(parse_purl("pkg:npm/npm@%E0%A4%A"), None);
        assert_eq!(parse_purl("pkg:npm/npm@%C0%AF"), None);
        assert_eq!(parse_purl("pkg:npm/npm@%ED%A0%80"), None);
        assert_eq!(parse_purl("pkg:npm/npm@%zz"), None);
    }
}
