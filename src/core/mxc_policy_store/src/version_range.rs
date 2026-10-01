// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Minimal version-range support for catalog v1, identical to TypeScript
//! `src/version-range.ts`.
//!
//! ```text
//! range         := comparatorSet ( "||" comparatorSet )*
//! comparatorSet := comparator ( " " comparator )*
//! comparator    := op? version
//! op            := ">=" | "<=" | ">" | "<" | "="
//! version       := N ( "." N ( "." N )? )?
//! ```
//!
//! Missing components are zero-filled. A comparator with no operator is a
//! prefix match. Prerelease and build metadata on evidence are ignored.

use crate::text::{js_is_line_terminator, js_is_space, js_trim};

type Triple = [f64; 3];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Ge,
    Le,
    Gt,
    Lt,
    Eq,
    Prefix,
}

struct Comparator {
    op: Op,
    version: Triple,
    parts: usize,
}

/// Consumes `\d+` (ASCII) from `s`, returning the digits and the rest.
fn digits(s: &str) -> Option<(&str, &str)> {
    let end = s
        .bytes()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(s.len());
    if end == 0 {
        None
    } else {
        Some((&s[..end], &s[end..]))
    }
}

fn number(digits: &str) -> f64 {
    digits.parse::<f64>().expect("ASCII digits parse as f64")
}

/// `/^(>=|<=|>|<|=)?(\d+)(?:\.(\d+)(?:\.(\d+))?)?$/`
fn parse_comparator(token: &str) -> Option<Comparator> {
    let (op, rest) = if let Some(rest) = token.strip_prefix(">=") {
        (Op::Ge, rest)
    } else if let Some(rest) = token.strip_prefix("<=") {
        (Op::Le, rest)
    } else if let Some(rest) = token.strip_prefix('>') {
        (Op::Gt, rest)
    } else if let Some(rest) = token.strip_prefix('<') {
        (Op::Lt, rest)
    } else if let Some(rest) = token.strip_prefix('=') {
        (Op::Eq, rest)
    } else {
        (Op::Prefix, token)
    };
    let (major, mut rest) = digits(rest)?;
    let mut version = [number(major), 0.0, 0.0];
    let mut parts = 1;
    if let Some(after_dot) = rest.strip_prefix('.') {
        let (minor, after_minor) = digits(after_dot)?;
        version[1] = number(minor);
        parts = 2;
        rest = after_minor;
        if let Some(after_dot) = rest.strip_prefix('.') {
            let (patch, after_patch) = digits(after_dot)?;
            version[2] = number(patch);
            parts = 3;
            rest = after_patch;
        }
    }
    if !rest.is_empty() {
        return None;
    }
    Some(Comparator { op, version, parts })
}

fn parse_range(range: &str) -> Option<Vec<Vec<Comparator>>> {
    if js_trim(range).is_empty() {
        return None;
    }
    let mut sets = Vec::new();
    for alternative in range.split("||") {
        let tokens: Vec<&str> = js_trim(alternative)
            .split(js_is_space)
            .filter(|t| !t.is_empty())
            .collect();
        if tokens.is_empty() {
            return None;
        }
        let mut comparators = Vec::new();
        for token in tokens {
            comparators.push(parse_comparator(token)?);
        }
        sets.push(comparators);
    }
    Some(sets)
}

/// True when `range` uses the supported v1 range grammar.
pub fn is_valid_version_range(range: &str) -> bool {
    parse_range(range).is_some()
}

/// `/^v?(\d+)(?:\.(\d+))?(?:\.(\d+))?(?:[-+].*)?$/` over the trimmed value.
fn parse_version_evidence(value: &str) -> Option<Triple> {
    let trimmed = js_trim(value);
    let rest = trimmed.strip_prefix('v').unwrap_or(trimmed);
    let (major, mut rest) = digits(rest)?;
    let mut version = [number(major), 0.0, 0.0];
    for slot in version.iter_mut().skip(1) {
        if let Some(after_dot) = rest.strip_prefix('.') {
            if let Some((value, after)) = digits(after_dot) {
                *slot = number(value);
                rest = after;
            }
        }
    }
    if rest.is_empty()
        || ((rest.starts_with('-') || rest.starts_with('+'))
            && !rest[1..].chars().any(js_is_line_terminator))
    {
        Some(version)
    } else {
        None
    }
}

fn compare(left: &Triple, right: &Triple) -> i32 {
    for index in 0..3 {
        if left[index] != right[index] {
            return if left[index] < right[index] { -1 } else { 1 };
        }
    }
    0
}

fn satisfies_comparator(version: &Triple, comparator: &Comparator) -> bool {
    let order = compare(version, &comparator.version);
    match comparator.op {
        Op::Ge => order >= 0,
        Op::Le => order <= 0,
        Op::Gt => order > 0,
        Op::Lt => order < 0,
        Op::Eq => order == 0,
        Op::Prefix => (0..comparator.parts).all(|i| version[i] == comparator.version[i]),
    }
}

/// Evaluates `version` against `range`; `None` when either cannot be evaluated.
pub fn satisfies_version_range(version: &str, range: &str) -> Option<bool> {
    let parsed_version = parse_version_evidence(version)?;
    let parsed_range = parse_range(range)?;
    Some(
        parsed_range
            .iter()
            .any(|set| set.iter().all(|c| satisfies_comparator(&parsed_version, c))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grammar() {
        for range in [">=10 <12", "22", ">=1.2.3", "=1.0.0", "<2 || >=4"] {
            assert!(is_valid_version_range(range), "{range}");
        }
        for range in [
            "", "^1.2.3", "~1", "1.x", ">= 1", "1 ||", "latest", "1.2.3.4", "１",
        ] {
            assert!(!is_valid_version_range(range), "{range}");
        }
    }

    #[test]
    fn evaluation() {
        assert_eq!(satisfies_version_range("10.9.0", ">=10 <12"), Some(true));
        assert_eq!(satisfies_version_range("v12.0.0", ">=10 <12"), Some(false));
        assert_eq!(satisfies_version_range("22.3.1", "22"), Some(true));
        assert_eq!(satisfies_version_range("23.0.0", "22"), Some(false));
        assert_eq!(satisfies_version_range("3.0.0", "<2 || >=3"), Some(true));
        assert_eq!(satisfies_version_range("10.0.0-rc.1", ">=10"), Some(true));
        assert_eq!(satisfies_version_range("nightly", ">=10"), None);
        assert_eq!(satisfies_version_range(" 1.2 ", "1.2"), Some(true));
        assert_eq!(satisfies_version_range("1.0.0-a\nb", ">=1"), None);
        assert_eq!(satisfies_version_range("1..2", ">=1"), None);
        assert_eq!(satisfies_version_range("1.2.3.4", ">=1"), None);
    }
}
