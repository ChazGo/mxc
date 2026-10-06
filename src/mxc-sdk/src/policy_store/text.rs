// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! ECMAScript string helpers the TypeScript implementation relies on.

/// ECMAScript `WhiteSpace` or `LineTerminator` (the set `String.prototype.trim`
/// removes and the regular-expression class `\s` matches).
pub fn js_is_space(c: char) -> bool {
    matches!(
        c,
        '\u{9}' | '\u{a}' | '\u{b}' | '\u{c}' | '\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// `String.prototype.trim`.
pub fn js_trim(value: &str) -> &str {
    value.trim_matches(js_is_space)
}

/// `String.prototype.toLowerCase` (locale-independent Unicode lowercasing).
pub fn js_to_lower(value: &str) -> String {
    value.to_lowercase()
}

/// Every match of `/\$\{([a-z][A-Za-z0-9_]*)\}/g`: `(start, end, name)` byte ranges.
pub fn symbol_matches(value: &str) -> Vec<(usize, usize, &str)> {
    let bytes = value.as_bytes();
    let mut matches = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$'
            && bytes.get(i + 1) == Some(&b'{')
            && bytes.get(i + 2).is_some_and(u8::is_ascii_lowercase)
        {
            let mut j = i + 3;
            while bytes
                .get(j)
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
            {
                j += 1;
            }
            if bytes.get(j) == Some(&b'}') {
                matches.push((i, j + 1, &value[i + 2..j]));
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    matches
}

/// `value.replace(/\$\{([a-z][A-Za-z0-9_]*)\}/g, name => replacement(name))`.
pub fn replace_symbols(value: &str, mut replacement: impl FnMut(&str) -> String) -> String {
    let mut out = String::new();
    let mut last = 0;
    for (start, end, name) in symbol_matches(value) {
        out.push_str(&value[last..start]);
        out.push_str(&replacement(name));
        last = end;
    }
    out.push_str(&value[last..]);
    out
}

/// `/^[a-z][A-Za-z0-9_]*$/`
pub fn is_symbol_name(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_matches_ecmascript() {
        assert_eq!(js_trim("\u{feff} a \u{3000}"), "a");
        assert_eq!(js_trim("\u{85}a"), "\u{85}a");
    }

    #[test]
    fn symbols() {
        let found: Vec<&str> = symbol_matches("${a}/${b_1}${C}${x${programData}")
            .iter()
            .map(|m| m.2)
            .collect();
        assert_eq!(found, ["a", "b_1", "programData"]);
        assert_eq!(replace_symbols("${a}/x/${a}", |_| "$&".into()), "$&/x/$&");
        assert!(is_symbol_name("git_prefix"));
        assert!(is_symbol_name("programData"));
        assert!(!is_symbol_name("Git"));
        assert!(!is_symbol_name(""));
    }
}
