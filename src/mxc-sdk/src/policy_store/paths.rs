// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Node.js `path.win32` / `path.posix` `normalize`, `isAbsolute`, and
//! `parse().root`, ported from Node v24.20.0 `lib/path.js`, plus the catalog
//! helpers built on them (ported from the TypeScript prototype's `normalizePath`, `isAbsolutePath`,
//! `pathKeySegments`).
//!
//! The algorithms run over UTF-16 code units, exactly like JavaScript, so
//! index arithmetic and slicing match Node for every input.

use crate::policy_store::model::Platform;
use crate::policy_store::text::js_to_lower;

const SLASH: u16 = b'/' as u16;
const BACKSLASH: u16 = b'\\' as u16;
const DOT: u16 = b'.' as u16;
const COLON: u16 = b':' as u16;
const QUESTION: u16 = b'?' as u16;

fn is_sep(code: u16) -> bool {
    code == SLASH || code == BACKSLASH
}

fn is_posix_sep(code: u16) -> bool {
    code == SLASH
}

fn is_device_root(code: u16) -> bool {
    (u16::from(b'A')..=u16::from(b'Z')).contains(&code)
        || (u16::from(b'a')..=u16::from(b'z')).contains(&code)
}

fn u(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}

fn s(value: &[u16]) -> String {
    String::from_utf16_lossy(value)
}

fn index_of(haystack: &[u16], needle: u16, from: usize) -> Option<usize> {
    haystack
        .iter()
        .skip(from)
        .position(|c| *c == needle)
        .map(|i| i + from)
}

/// JavaScript `String.prototype.slice(start, end)` with negative-index rules.
fn js_slice(value: &[u16], start: isize, end: isize) -> &[u16] {
    let len = value.len() as isize;
    let clamp = |i: isize| if i < 0 { (len + i).max(0) } else { i.min(len) };
    let (start, end) = (clamp(start), clamp(end));
    if start >= end {
        &[]
    } else {
        &value[start as usize..end as usize]
    }
}

const WINDOWS_RESERVED_NAMES: [&str; 28] = [
    "CON",
    "PRN",
    "AUX",
    "NUL",
    "COM1",
    "COM2",
    "COM3",
    "COM4",
    "COM5",
    "COM6",
    "COM7",
    "COM8",
    "COM9",
    "LPT1",
    "LPT2",
    "LPT3",
    "LPT4",
    "LPT5",
    "LPT6",
    "LPT7",
    "LPT8",
    "LPT9",
    "COM\u{b9}",
    "COM\u{b2}",
    "COM\u{b3}",
    "LPT\u{b9}",
    "LPT\u{b2}",
    "LPT\u{b3}",
];

fn is_windows_reserved_name(path: &[u16], colon_index: isize) -> bool {
    let part = js_slice(path, 0, colon_index);
    // A slice that splits a surrogate pair cannot equal a reserved name.
    let Ok(part) = String::from_utf16(part) else {
        return false;
    };
    let upper = part.to_uppercase();
    WINDOWS_RESERVED_NAMES.contains(&upper.as_str())
}

/// Node `normalizeString`: resolves `.` and `..` segments.
fn normalize_string(
    path: &[u16],
    allow_above_root: bool,
    separator: u16,
    is_separator: fn(u16) -> bool,
) -> Vec<u16> {
    let mut res: Vec<u16> = Vec::new();
    let mut last_segment_length: isize = 0;
    let mut last_slash: isize = -1;
    let mut dots: i32 = 0;
    let mut code: u16 = 0;
    let len = path.len();
    let mut i = 0usize;
    while i <= len {
        if i < len {
            code = path[i];
        } else if is_separator(code) {
            break;
        } else {
            code = SLASH;
        }
        let ii = i as isize;
        if is_separator(code) {
            if last_slash == ii - 1 || dots == 1 {
                // NOOP
            } else if dots == 2 {
                let rl = res.len();
                if rl < 2 || last_segment_length != 2 || res[rl - 1] != DOT || res[rl - 2] != DOT {
                    if rl > 2 {
                        let last_slash_index = rl as isize - last_segment_length - 1;
                        if last_slash_index == -1 {
                            res.clear();
                            last_segment_length = 0;
                        } else {
                            res.truncate(last_slash_index as usize);
                            let last_sep = res
                                .iter()
                                .rposition(|c| *c == separator)
                                .map_or(-1, |p| p as isize);
                            last_segment_length = res.len() as isize - 1 - last_sep;
                        }
                        last_slash = ii;
                        dots = 0;
                        i += 1;
                        continue;
                    } else if rl != 0 {
                        res.clear();
                        last_segment_length = 0;
                        last_slash = ii;
                        dots = 0;
                        i += 1;
                        continue;
                    }
                }
                if allow_above_root {
                    if !res.is_empty() {
                        res.push(separator);
                    }
                    res.extend_from_slice(&[DOT, DOT]);
                    last_segment_length = 2;
                }
            } else {
                let segment = &path[(last_slash + 1) as usize..i];
                if !res.is_empty() {
                    res.push(separator);
                }
                res.extend_from_slice(segment);
                last_segment_length = ii - last_slash - 1;
            }
            last_slash = ii;
            dots = 0;
        } else if code == DOT && dots != -1 {
            dots += 1;
        } else {
            dots = -1;
        }
        i += 1;
    }
    res
}

fn cat(parts: &[&[u16]]) -> Vec<u16> {
    parts.concat()
}

/// Node `path.win32.normalize`.
pub fn win32_normalize(input: &str) -> String {
    s(&win32_normalize_units(&u(input)))
}

fn win32_normalize_units(path: &[u16]) -> Vec<u16> {
    let len = path.len();
    if len == 0 {
        return vec![DOT];
    }
    let mut root_end = 0usize;
    let mut device: Option<Vec<u16>> = None;
    let mut is_absolute = false;
    let code = path[0];
    let bs = [BACKSLASH];
    if len == 1 {
        return if is_posix_sep(code) {
            vec![BACKSLASH]
        } else {
            path.to_vec()
        };
    }
    if is_sep(code) {
        is_absolute = true;
        if is_sep(path[1]) {
            let mut j = 2;
            let mut last = j;
            while j < len && !is_sep(path[j]) {
                j += 1;
            }
            if j < len && j != last {
                let first_part = &path[last..j];
                last = j;
                while j < len && is_sep(path[j]) {
                    j += 1;
                }
                if j < len && j != last {
                    last = j;
                    while j < len && !is_sep(path[j]) {
                        j += 1;
                    }
                    if j == len || j != last {
                        if first_part == [DOT] || first_part == [QUESTION] {
                            device = Some(cat(&[&[BACKSLASH, BACKSLASH], first_part]));
                            root_end = 4;
                            let colon_index = index_of(path, COLON, 0).map_or(-1, |i| i as isize);
                            let possible = js_slice(path, 4, colon_index + 1).to_vec();
                            if is_windows_reserved_name(&possible, possible.len() as isize - 1) {
                                device = Some(cat(&[
                                    &[BACKSLASH, BACKSLASH, QUESTION, BACKSLASH],
                                    &possible,
                                ]));
                                root_end = 4 + possible.len();
                            }
                        } else if j == len {
                            return cat(&[
                                &[BACKSLASH, BACKSLASH],
                                first_part,
                                &bs,
                                &path[last..],
                                &bs,
                            ]);
                        } else {
                            device = Some(cat(&[
                                &[BACKSLASH, BACKSLASH],
                                first_part,
                                &bs,
                                &path[last..j],
                            ]));
                            root_end = j;
                        }
                    }
                }
            }
        } else {
            root_end = 1;
        }
    } else if let Some(colon_index) = index_of(path, COLON, 0).filter(|i| *i > 0) {
        if is_device_root(code) && colon_index == 1 {
            device = Some(path[0..2].to_vec());
            root_end = 2;
            if len > 2 && is_sep(path[2]) {
                is_absolute = true;
                root_end = 3;
            }
        } else if is_windows_reserved_name(path, colon_index as isize) {
            device = Some(path[..colon_index + 1].to_vec());
            root_end = colon_index + 1;
        }
    }

    let mut tail = if root_end < len {
        normalize_string(&path[root_end..], !is_absolute, BACKSLASH, is_sep)
    } else {
        Vec::new()
    };
    if tail.is_empty() && !is_absolute {
        tail = vec![DOT];
    }
    if !tail.is_empty() && is_sep(path[len - 1]) {
        tail.push(BACKSLASH);
    }
    let dot_bs = [DOT, BACKSLASH];
    if !is_absolute && device.is_none() && path.contains(&COLON) {
        if tail.len() >= 2 && is_device_root(tail[0]) && tail[1] == COLON {
            return cat(&[&dot_bs, &tail]);
        }
        let mut index = index_of(path, COLON, 0);
        while let Some(at) = index {
            if at == len - 1 || is_sep(path[at + 1]) {
                return cat(&[&dot_bs, &tail]);
            }
            index = index_of(path, COLON, at + 1);
        }
    }
    let colon_index = index_of(path, COLON, 0).map_or(-1, |i| i as isize);
    if is_windows_reserved_name(path, colon_index) {
        return cat(&[&dot_bs, device.as_deref().unwrap_or(&[]), &tail]);
    }
    match device {
        None => {
            if is_absolute {
                cat(&[&bs, &tail])
            } else {
                tail
            }
        }
        Some(device) => {
            if is_absolute {
                cat(&[&device, &bs, &tail])
            } else {
                cat(&[&device, &tail])
            }
        }
    }
}

/// Node `path.win32.isAbsolute`.
pub fn win32_is_absolute(input: &str) -> bool {
    let path = u(input);
    let len = path.len();
    if len == 0 {
        return false;
    }
    let code = path[0];
    is_sep(code) || (len > 2 && is_device_root(code) && path[1] == COLON && is_sep(path[2]))
}

/// Node `path.win32.parse(path).root`.
fn win32_root(path: &[u16]) -> usize {
    let len = path.len();
    if len == 0 {
        return 0;
    }
    let code = path[0];
    if len == 1 {
        return if is_sep(code) { 1 } else { 0 };
    }
    let mut root_end = 0;
    if is_sep(code) {
        root_end = 1;
        if is_sep(path[1]) {
            let mut j = 2;
            let mut last = j;
            while j < len && !is_sep(path[j]) {
                j += 1;
            }
            if j < len && j != last {
                last = j;
                while j < len && is_sep(path[j]) {
                    j += 1;
                }
                if j < len && j != last {
                    last = j;
                    while j < len && !is_sep(path[j]) {
                        j += 1;
                    }
                    if j == len {
                        root_end = j;
                    } else if j != last {
                        root_end = j + 1;
                    }
                }
            }
        }
    } else if is_device_root(code) && path[1] == COLON {
        if len <= 2 {
            return len;
        }
        root_end = 2;
        if is_sep(path[2]) {
            if len == 3 {
                return len;
            }
            root_end = 3;
        }
    }
    root_end
}

/// Node `path.posix.normalize`.
pub fn posix_normalize(input: &str) -> String {
    let path = u(input);
    if path.is_empty() {
        return ".".to_string();
    }
    let is_absolute = path[0] == SLASH;
    let trailing = path[path.len() - 1] == SLASH;
    let mut normalized = normalize_string(&path, !is_absolute, SLASH, is_posix_sep);
    if normalized.is_empty() {
        if is_absolute {
            return "/".to_string();
        }
        return if trailing { "./" } else { "." }.to_string();
    }
    if trailing {
        normalized.push(SLASH);
    }
    if is_absolute {
        format!("/{}", s(&normalized))
    } else {
        s(&normalized)
    }
}

/// Node `path.posix.isAbsolute`.
pub fn posix_is_absolute(input: &str) -> bool {
    input.starts_with('/')
}

/// The one casing rule for a target platform: Windows and macOS fold case,
/// Linux does not. Used for invocation-name matching and path comparison.
pub fn folds_case(platform: Platform) -> bool {
    platform != Platform::Linux
}

/// Applies [`folds_case`] to one value.
pub fn case_key(value: &str, platform: Platform) -> String {
    if folds_case(platform) {
        js_to_lower(value)
    } else {
        value.to_string()
    }
}

/// Normalizes a path with the platform's rules, then removes trailing
/// separators unless only the root remains (the prototype's `normalizePath`).
pub fn normalize_path(value: &str, platform: Platform) -> String {
    let (normalized, root_len) = if platform == Platform::Windows {
        let n = win32_normalize_units(&u(value));
        let root = win32_root(&n);
        (n, root)
    } else {
        let n = u(&posix_normalize(value));
        let root = usize::from(n.first() == Some(&SLASH));
        (n, root)
    };
    if normalized.len() > root_len {
        let mut end = normalized.len();
        while end > 0 && is_sep(normalized[end - 1]) {
            end -= 1;
        }
        s(&normalized[..end])
    } else {
        s(&normalized)
    }
}

/// Whether `value` is absolute under the platform's rules (Node `isAbsolute`).
pub fn is_absolute_path(value: &str, platform: Platform) -> bool {
    if platform == Platform::Windows {
        win32_is_absolute(value)
    } else {
        posix_is_absolute(value)
    }
}

/// Splits a path into comparable segments with the platform's separator and casing rules.
pub fn path_key_segments(value: &str, platform: Platform) -> Vec<String> {
    split_segments(value, platform, true)
}

/// Splits a path into segments with the platform's separators, preserving
/// case. Used where filesystem case sensitivity is unknown (design §4.5).
pub fn path_exact_segments(value: &str, platform: Platform) -> Vec<String> {
    split_segments(value, platform, false)
}

fn split_segments(value: &str, platform: Platform, fold: bool) -> Vec<String> {
    let is_separator: fn(char) -> bool = if platform == Platform::Windows {
        |c| c == '/' || c == '\\'
    } else {
        |c| c == '/'
    };
    // `value.split(/[\\/]+/)`: split on runs of separators.
    let mut raw: Vec<&str> = Vec::new();
    let mut start = 0;
    let mut in_run = false;
    for (index, c) in value.char_indices() {
        if is_separator(c) {
            if !in_run {
                raw.push(&value[start..index]);
                in_run = true;
            }
            start = index + c.len_utf8();
        } else {
            in_run = false;
        }
    }
    raw.push(&value[start..]);
    let segments: Vec<&str> = raw
        .into_iter()
        .enumerate()
        .filter(|(index, segment)| *segment != "." && (!segment.is_empty() || *index == 0))
        .map(|(_, segment)| segment)
        .collect();
    let segments = if segments.len() > 1 && segments.last() == Some(&"") {
        &segments[..segments.len() - 1]
    } else {
        &segments[..]
    };
    segments
        .iter()
        .map(|segment| {
            if fold {
                case_key(segment, platform)
            } else {
                segment.to_string()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win32_normalize_matches_node() {
        let cases = [
            ("", "."),
            ("/", "\\"),
            ("a", "a"),
            ("C:", "C:."),
            ("C:\\", "C:\\"),
            ("C:foo\\..\\..", "C:.."),
            ("\\\\server\\share", "\\\\server\\share\\"),
            ("\\\\.\\PHYSICALDRIVE0\\x", "\\\\.\\PHYSICALDRIVE0\\x"),
            ("\\\\?\\COM1:\\x", "\\\\?\\COM1:\\x"),
            ("COM1:foo", ".\\COM1:foo"),
            ("foo:bar", "foo:bar"),
            ("foo:", ".\\foo:"),
            ("a\\..\\c:x", ".\\c:x"),
            ("./..", ".."),
            ("a/b/../../..", ".."),
            ("\\\\x", "\\x"),
        ];
        for (input, expected) in cases {
            assert_eq!(win32_normalize(input), expected, "{input:?}");
        }
    }

    #[test]
    fn posix_normalize_matches_node() {
        for (input, expected) in [
            ("", "."),
            ("./", "./"),
            ("a/..", "."),
            ("../a/", "../a/"),
            ("//a//b/../", "/a/"),
        ] {
            assert_eq!(posix_normalize(input), expected, "{input:?}");
        }
    }
}
