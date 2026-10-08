// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Package URL parsing for identity matching (design §4.3). The whole PURL is
//! validated using the purl-spec component rules; matching then compares only
//! type, namespace, and name.

/// A parsed, percent-decoded package URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedPurl {
    /// Lower-cased type.
    pub package_type: String,
    /// Decoded namespace segments joined with `/`, if any.
    pub namespace: Option<String>,
    /// Decoded name.
    pub name: String,
    pub version: Option<String>,
    pub has_qualifiers: bool,
    pub has_subpath: bool,
    /// Catalog comparison key: type, case-folded namespace, and the name
    /// normalized by its type's rules.
    pub key: String,
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

fn is_qualifier_key(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '.' || c == '-' || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
}

/// Name normalization for package types whose definitions declare
/// case-insensitive names. Every other type compares names exactly.
fn normalize_name(package_type: &str, name: &str) -> String {
    match package_type {
        "pypi" => name.to_lowercase().replace('_', "-"),
        "bitbucket" | "github" | "composer" | "hex" | "nuget" => name.to_lowercase(),
        _ => name.to_string(),
    }
}

/// Parses `pkg:type/namespace/name@version?qualifiers#subpath` following the
/// purl-spec parsing steps. `None` for an invalid package URL.
pub fn parse_purl(value: &str) -> Option<ParsedPurl> {
    if value.chars().any(char::is_whitespace) {
        return None;
    }
    let (rest, subpath) = match value.rsplit_once('#') {
        Some((rest, subpath)) => (rest, Some(subpath)),
        None => (value, None),
    };
    let mut has_subpath = false;
    if let Some(subpath) = subpath {
        for segment in subpath.split('/') {
            if segment.is_empty() || segment == "." || segment == ".." {
                continue;
            }
            let decoded = decode_uri_component(segment)?;
            if decoded.contains('/') {
                return None;
            }
            has_subpath = true;
        }
    }
    let (rest, qualifiers) = match rest.rsplit_once('?') {
        Some((rest, qualifiers)) => (rest, Some(qualifiers)),
        None => (rest, None),
    };
    let mut has_qualifiers = false;
    let mut keys: Vec<String> = Vec::new();
    if let Some(qualifiers) = qualifiers {
        for pair in qualifiers.split('&').filter(|p| !p.is_empty()) {
            let (key, raw) = pair.split_once('=')?;
            let key = key.to_ascii_lowercase();
            if !is_qualifier_key(&key) || keys.contains(&key) {
                return None;
            }
            let decoded = decode_uri_component(raw)?;
            keys.push(key);
            if !decoded.is_empty() {
                has_qualifiers = true;
            }
        }
    }
    let (scheme, rest) = rest.split_once(':')?;
    if !scheme.eq_ignore_ascii_case("pkg") {
        return None;
    }
    let rest = rest.trim_matches('/');
    let (package_type, rest) = rest.split_once('/')?;
    if !is_purl_type(package_type) {
        return None;
    }
    let package_type = package_type.to_ascii_lowercase();
    // An '@' before the last '/' is an unencoded npm scope, not a version.
    let (rest, version) = match rest.rsplit_once('@') {
        Some((head, version)) if !version.contains('/') => {
            (head, Some(decode_uri_component(version)?))
        }
        _ => (rest, None),
    };
    let rest = rest.trim_end_matches('/');
    let (namespace, name) = match rest.rsplit_once('/') {
        Some((namespace, name)) => (Some(namespace), name),
        None => (None, rest),
    };
    let name = decode_uri_component(name)?;
    if name.is_empty() {
        return None;
    }
    let namespace = match namespace {
        None => None,
        Some(namespace) => {
            let mut segments = Vec::new();
            for segment in namespace.split('/').filter(|s| !s.is_empty()) {
                let decoded = decode_uri_component(segment)?;
                if decoded.is_empty() || decoded.contains('/') {
                    return None;
                }
                segments.push(decoded);
            }
            (!segments.is_empty()).then(|| segments.join("/"))
        }
    };
    let key = format!(
        "{package_type}\u{0}{}\u{0}{}",
        namespace.as_deref().unwrap_or("").to_lowercase(),
        normalize_name(&package_type, &name)
    );
    Some(ParsedPurl {
        package_type,
        namespace,
        name,
        version: version.filter(|v| !v.is_empty()),
        has_qualifiers,
        has_subpath,
        key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(value: &str) -> Option<String> {
        parse_purl(value).map(|p| p.key)
    }

    #[test]
    fn compares_type_namespace_and_name_only() {
        assert_eq!(key("pkg:npm/npm@10.9.0"), key("pkg:npm/npm"));
        assert_eq!(key("pkg:NPM/%40Scope/pkg"), key("pkg:npm/%40scope/pkg"));
        assert_eq!(key("pkg:npm/@scope/pkg"), key("pkg:npm/%40scope/pkg"));
        assert_ne!(key("pkg:npm/Pkg"), key("pkg:npm/pkg"));
        assert_eq!(key("pkg:pypi/Foo_Bar"), key("pkg:pypi/foo-bar"));
        assert_eq!(
            key("pkg:nuget/Newtonsoft.Json"),
            key("pkg:nuget/newtonsoft.json")
        );
        assert_eq!(key("pkg://npm/npm"), key("pkg:npm/npm"));
        assert_eq!(key("PKG:npm/npm"), key("pkg:npm/npm"));
    }

    #[test]
    fn records_ignored_components() {
        let p = parse_purl("pkg:npm/%40scope/pkg@1.0.0?arch=x64#lib/x").unwrap();
        assert_eq!(p.namespace.as_deref(), Some("@scope"));
        assert_eq!(p.name, "pkg");
        assert_eq!(p.version.as_deref(), Some("1.0.0"));
        assert!(p.has_qualifiers && p.has_subpath);
        let p = parse_purl("pkg:npm/npm@").unwrap();
        assert_eq!(p.version, None);
        assert!(!p.has_qualifiers && !p.has_subpath);
        assert_eq!(
            parse_purl("pkg:npm/a@%E2%82%AC")
                .unwrap()
                .version
                .as_deref(),
            Some("\u{20ac}")
        );
    }

    #[test]
    fn rejects_invalid_package_urls() {
        for invalid in [
            "npm/npm",
            "pkg:npm",
            "pkg:npm/",
            "pkg:1pm/x",
            "pkg:npm/npm@%E0%A4%A",
            "pkg:npm/npm@%C0%AF",
            "pkg:npm/npm@%ED%A0%80",
            "pkg:npm/npm@%zz",
            "pkg:npm/npm?1bad=x",
            "pkg:npm/npm?novalue",
            "pkg:npm/npm?a=1&a=2",
            "pkg:npm/has space",
            "http:npm/npm",
        ] {
            assert_eq!(parse_purl(invalid), None, "{invalid}");
        }
    }
}
