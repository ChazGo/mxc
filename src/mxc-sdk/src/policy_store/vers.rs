// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Version schemes and purl `vers` ranges.
//!
//! Every catalog entry declares one `versionScheme`; its version variants and
//! the caller's `detectedVersion` are parsed and compared in that scheme.
//! Ranges use the purl `vers` syntax (`vers:<scheme>/<constraints>`), with
//! the validation and containment rules of the VERS specification (ECMA-427,
//! Clause 5).
//!
//! | Scheme | Parsing and ordering |
//! |---|---|
//! | `semver` | Strict SemVer 2.0.0; build metadata is ignored for precedence. |
//! | `npm` | node-semver strict parsing (surrounding whitespace and one leading `v` are accepted; at most 256 characters; major, minor, and patch at most 2^53 - 1), SemVer precedence. |
//! | `pypi` | PEP 440 (case-insensitive, normalized), ordered like `packaging.version`. |
//! | `nuget` | NuGet: one to four numeric parts (missing parts are zero), optional `-` release labels and ignored `+` metadata; labels compare numerically or ordinal-ignore-case. |
//! | `intdot` | Dot-separated non-negative integers, compared numerically with missing components as zero. |
//!
//! A caller-supplied `intdot` version is read up to its first character that
//! is not a digit or `.` (so `2.45.1.windows.1` is `2.45.1`); range literals
//! must be plain `intdot` versions. Integer components of any size compare
//! exactly.

use std::cmp::Ordering;
use std::fmt;

/// The version schemes an entry may declare.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VersionScheme {
    Npm,
    Semver,
    Pypi,
    Nuget,
    Intdot,
}

impl VersionScheme {
    pub const ALL: [VersionScheme; 5] = [
        VersionScheme::Npm,
        VersionScheme::Semver,
        VersionScheme::Pypi,
        VersionScheme::Nuget,
        VersionScheme::Intdot,
    ];

    /// Parses an exact, lower-case scheme name.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == text)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            VersionScheme::Npm => "npm",
            VersionScheme::Semver => "semver",
            VersionScheme::Pypi => "pypi",
            VersionScheme::Nuget => "nuget",
            VersionScheme::Intdot => "intdot",
        }
    }

    /// Parses a caller-supplied version (for example `detectedVersion`).
    pub fn parse_version(self, text: &str) -> Option<Version> {
        let key = match self {
            VersionScheme::Semver => VersionKey::Semver(parse_semver(text)?),
            VersionScheme::Npm => VersionKey::Semver(parse_npm(text)?),
            VersionScheme::Pypi => VersionKey::Pypi(parse_pep440(text)?),
            VersionScheme::Nuget => VersionKey::Nuget(parse_nuget(text)?),
            VersionScheme::Intdot => VersionKey::Intdot(parse_intdot_lenient(text)?),
        };
        Some(Version { scheme: self, key })
    }

    /// Parses a version that appears literally in a `vers` range. `intdot`
    /// literals must be plain dot-separated integers.
    fn parse_literal(self, text: &str) -> Option<Version> {
        match self {
            VersionScheme::Intdot => Some(Version {
                scheme: self,
                key: VersionKey::Intdot(parse_intdot_strict(text)?),
            }),
            _ => self.parse_version(text),
        }
    }
}

impl fmt::Display for VersionScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A parsed version in one scheme. Versions of different schemes are never
/// compared by the resolver; [`Ord`] orders them by scheme first.
#[derive(Clone, Debug)]
pub struct Version {
    scheme: VersionScheme,
    key: VersionKey,
}

impl Version {
    pub fn scheme(&self) -> VersionScheme {
        self.scheme
    }
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        match (&self.key, &other.key) {
            (VersionKey::Semver(a), VersionKey::Semver(b)) => a.cmp(b),
            (VersionKey::Pypi(a), VersionKey::Pypi(b)) => a.cmp(b),
            (VersionKey::Nuget(a), VersionKey::Nuget(b)) => a.cmp(b),
            (VersionKey::Intdot(a), VersionKey::Intdot(b)) => cmp_intdot(a, b),
            _ => (self.scheme as u8).cmp(&(other.scheme as u8)),
        }
    }
}

#[derive(Clone, Debug)]
enum VersionKey {
    Semver(Semver),
    Pypi(Pep440),
    Nuget(NugetVersion),
    Intdot(Vec<String>),
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// Strips leading zeros, keeping at least one digit.
fn normalize_digits(s: &str) -> String {
    let trimmed = s.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Compares two normalized digit strings numerically.
fn cmp_num(a: &str, b: &str) -> Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-'
}

// ---------------------------------------------------------------------------
// SemVer 2.0.0 and npm
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
enum Ident {
    Num(String),
    Alnum(String),
}

impl Ord for Ident {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Ident::Num(a), Ident::Num(b)) => cmp_num(a, b),
            (Ident::Num(_), Ident::Alnum(_)) => Ordering::Less,
            (Ident::Alnum(_), Ident::Num(_)) => Ordering::Greater,
            (Ident::Alnum(a), Ident::Alnum(b)) => a.as_bytes().cmp(b.as_bytes()),
        }
    }
}

impl PartialOrd for Ident {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Semver {
    core: [String; 3],
    pre: Vec<Ident>,
}

impl Ord for Semver {
    fn cmp(&self, other: &Self) -> Ordering {
        for i in 0..3 {
            match cmp_num(&self.core[i], &other.core[i]) {
                Ordering::Equal => {}
                o => return o,
            }
        }
        match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => self.pre.cmp(&other.pre),
        }
    }
}

impl PartialOrd for Semver {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A SemVer numeric identifier: `0` or no leading zero.
fn semver_numeric(s: &str) -> bool {
    all_digits(s) && (s == "0" || !s.starts_with('0'))
}

fn parse_semver(text: &str) -> Option<Semver> {
    let (rest, build) = match text.split_once('+') {
        Some((r, b)) => (r, Some(b)),
        None => (text, None),
    };
    if let Some(build) = build {
        if build
            .split('.')
            .any(|p| p.is_empty() || !p.bytes().all(is_ident_char))
        {
            return None;
        }
    }
    let (core, pre) = match rest.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (rest, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3 || !parts.iter().all(|p| semver_numeric(p)) {
        return None;
    }
    let mut idents = Vec::new();
    if let Some(pre) = pre {
        for p in pre.split('.') {
            if p.is_empty() || !p.bytes().all(is_ident_char) {
                return None;
            }
            if all_digits(p) {
                if !semver_numeric(p) {
                    return None;
                }
                idents.push(Ident::Num(p.to_string()));
            } else {
                idents.push(Ident::Alnum(p.to_string()));
            }
        }
    }
    Some(Semver {
        core: [
            parts[0].to_string(),
            parts[1].to_string(),
            parts[2].to_string(),
        ],
        pre: idents,
    })
}

/// node-semver `new SemVer(text)` without `loose`.
fn parse_npm(text: &str) -> Option<Semver> {
    const MAX_LENGTH: usize = 256;
    const MAX_SAFE_INTEGER: &str = "9007199254740991";
    if text.len() > MAX_LENGTH {
        return None;
    }
    let trimmed = text.trim();
    let body = trimmed.strip_prefix('v').unwrap_or(trimmed);
    let parsed = parse_semver(body)?;
    if parsed
        .core
        .iter()
        .any(|n| cmp_num(n, MAX_SAFE_INTEGER) == Ordering::Greater)
    {
        return None;
    }
    Some(parsed)
}

// ---------------------------------------------------------------------------
// PEP 440 (pypi)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum PreKey {
    /// A dev release with no pre or post segment sorts before any pre-release.
    NegInf,
    Pre(u8, String),
    PosInf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum LocalPart {
    Str(String),
    Num(String),
}

impl Ord for LocalPart {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (LocalPart::Num(a), LocalPart::Num(b)) => cmp_num(a, b),
            (LocalPart::Str(_), LocalPart::Num(_)) => Ordering::Less,
            (LocalPart::Num(_), LocalPart::Str(_)) => Ordering::Greater,
            (LocalPart::Str(a), LocalPart::Str(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for LocalPart {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Pep440 {
    epoch: String,
    /// Release with trailing zeros removed.
    release: Vec<String>,
    pre: PreKey,
    /// `None` sorts before any post release.
    post: Option<String>,
    /// `None` sorts after any dev release.
    dev: Option<String>,
    /// `None` sorts before any local version.
    local: Option<Vec<LocalPart>>,
}

fn cmp_num_list(a: &[String], b: &[String]) -> Ordering {
    for (x, y) in a.iter().zip(b) {
        match cmp_num(x, y) {
            Ordering::Equal => {}
            o => return o,
        }
    }
    a.len().cmp(&b.len())
}

impl Ord for Pep440 {
    fn cmp(&self, other: &Self) -> Ordering {
        cmp_num(&self.epoch, &other.epoch)
            .then_with(|| cmp_num_list(&self.release, &other.release))
            .then_with(|| match (&self.pre, &other.pre) {
                (PreKey::Pre(la, na), PreKey::Pre(lb, nb)) => {
                    la.cmp(lb).then_with(|| cmp_num(na, nb))
                }
                (a, b) => a.cmp(b),
            })
            .then_with(|| match (&self.post, &other.post) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Less,
                (Some(_), None) => Ordering::Greater,
                (Some(a), Some(b)) => cmp_num(a, b),
            })
            .then_with(|| match (&self.dev, &other.dev) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => cmp_num(a, b),
            })
            .then_with(|| match (&self.local, &other.local) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Less,
                (Some(_), None) => Ordering::Greater,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

impl PartialOrd for Pep440 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

struct Cursor<'a> {
    s: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.s.get(self.pos + offset).copied()
    }

    fn digits(&mut self) -> Option<String> {
        let start = self.pos;
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.pos += 1;
        }
        (self.pos > start).then(|| String::from_utf8_lossy(&self.s[start..self.pos]).into_owned())
    }

    fn eat_word(&mut self, words: &[&'static str]) -> Option<&'static str> {
        // Longest match, so `preview` wins over `pre`.
        let mut best: Option<&'static str> = None;
        for word in words {
            if self.s[self.pos..].starts_with(word.as_bytes())
                && best.is_none_or(|b| word.len() > b.len())
            {
                best = Some(word);
            }
        }
        let word = best?;
        self.pos += word.len();
        Some(word)
    }

    fn is_sep(b: Option<u8>) -> bool {
        matches!(b, Some(b'-' | b'_' | b'.'))
    }

    /// An optional separator followed by digits; consumes nothing if no
    /// digits follow.
    fn sep_digits(&mut self) -> Option<String> {
        if Self::is_sep(self.peek()) && self.peek_at(1).is_some_and(|b| b.is_ascii_digit()) {
            self.pos += 1;
        }
        self.digits()
    }

    /// An optional separator followed by one of `words`; consumes nothing on
    /// failure.
    fn sep_word(&mut self, words: &[&'static str]) -> Option<&'static str> {
        let start = self.pos;
        if Self::is_sep(self.peek()) {
            self.pos += 1;
        }
        let found = self.eat_word(words);
        if found.is_none() {
            self.pos = start;
        }
        found
    }
}

fn parse_pep440(text: &str) -> Option<Pep440> {
    let lower = text.trim().to_ascii_lowercase();
    let mut c = Cursor {
        s: lower.as_bytes(),
        pos: 0,
    };
    if c.peek() == Some(b'v') {
        c.pos += 1;
    }
    let mut epoch = "0".to_string();
    let mark = c.pos;
    if let Some(d) = c.digits() {
        if c.peek() == Some(b'!') {
            c.pos += 1;
            epoch = normalize_digits(&d);
        } else {
            c.pos = mark;
        }
    }
    let mut release = vec![normalize_digits(&c.digits()?)];
    while c.peek() == Some(b'.') && c.peek_at(1).is_some_and(|b| b.is_ascii_digit()) {
        c.pos += 1;
        release.push(normalize_digits(&c.digits()?));
    }
    let mut pre = None;
    if let Some(word) = c.sep_word(&["a", "b", "c", "rc", "alpha", "beta", "pre", "preview"]) {
        let rank = match word {
            "a" | "alpha" => 0,
            "b" | "beta" => 1,
            _ => 2,
        };
        let n = c.sep_digits().map(|d| normalize_digits(&d));
        pre = Some((rank, n.unwrap_or_else(|| "0".to_string())));
    }
    let mut post = None;
    if c.peek() == Some(b'-') && c.peek_at(1).is_some_and(|b| b.is_ascii_digit()) {
        c.pos += 1;
        post = Some(normalize_digits(&c.digits()?));
    } else if c.sep_word(&["post", "rev", "r"]).is_some() {
        post = Some(
            c.sep_digits()
                .map(|d| normalize_digits(&d))
                .unwrap_or_else(|| "0".to_string()),
        );
    }
    let mut dev = None;
    if c.sep_word(&["dev"]).is_some() {
        dev = Some(
            c.sep_digits()
                .map(|d| normalize_digits(&d))
                .unwrap_or_else(|| "0".to_string()),
        );
    }
    let mut local = None;
    if c.peek() == Some(b'+') {
        c.pos += 1;
        let rest = std::str::from_utf8(&c.s[c.pos..]).ok()?;
        let mut parts = Vec::new();
        for part in rest.split(['-', '_', '.']) {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_alphanumeric()) {
                return None;
            }
            parts.push(if all_digits(part) {
                LocalPart::Num(normalize_digits(part))
            } else {
                LocalPart::Str(part.to_string())
            });
        }
        c.pos = c.s.len();
        local = Some(parts);
    }
    if c.pos != c.s.len() {
        return None;
    }
    while release.len() > 1 && release.last().is_some_and(|r| r == "0") {
        release.pop();
    }
    if release.len() == 1 && release[0] == "0" {
        release.clear();
    }
    let pre_key = match (&pre, &post, &dev) {
        (None, None, Some(_)) => PreKey::NegInf,
        (None, _, _) => PreKey::PosInf,
        (Some((rank, n)), _, _) => PreKey::Pre(*rank, n.clone()),
    };
    Some(Pep440 {
        epoch,
        release,
        pre: pre_key,
        post,
        dev,
        local,
    })
}

// ---------------------------------------------------------------------------
// NuGet
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
struct NugetVersion {
    parts: [u32; 4],
    labels: Vec<String>,
}

fn nuget_label_number(label: &str) -> Option<i32> {
    if all_digits(label) {
        label.parse::<i32>().ok()
    } else {
        None
    }
}

fn cmp_nuget_label(a: &str, b: &str) -> Ordering {
    match (nuget_label_number(a), nuget_label_number(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a
            .bytes()
            .map(|c| c.to_ascii_uppercase())
            .cmp(b.bytes().map(|c| c.to_ascii_uppercase())),
    }
}

impl Ord for NugetVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.parts.cmp(&other.parts).then_with(|| {
            match (self.labels.is_empty(), other.labels.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => {
                    for (x, y) in self.labels.iter().zip(&other.labels) {
                        match cmp_nuget_label(x, y) {
                            Ordering::Equal => {}
                            o => return o,
                        }
                    }
                    self.labels.len().cmp(&other.labels.len())
                }
            }
        })
    }
}

impl PartialOrd for NugetVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn parse_nuget(text: &str) -> Option<NugetVersion> {
    let text = text.trim();
    let (rest, metadata) = match text.split_once('+') {
        Some((r, m)) => (r, Some(m)),
        None => (text, None),
    };
    if let Some(metadata) = metadata {
        if metadata
            .split('.')
            .any(|p| p.is_empty() || !p.bytes().all(is_ident_char))
        {
            return None;
        }
    }
    let (core, release) = match rest.split_once('-') {
        Some((c, r)) => (c, Some(r)),
        None => (rest, None),
    };
    let numbers: Vec<&str> = core.split('.').collect();
    if numbers.is_empty() || numbers.len() > 4 {
        return None;
    }
    let mut parts = [0u32; 4];
    for (i, n) in numbers.iter().enumerate() {
        if !all_digits(n) {
            return None;
        }
        let value = n.parse::<i32>().ok()?;
        parts[i] = value as u32;
    }
    let mut labels = Vec::new();
    if let Some(release) = release {
        for label in release.split('.') {
            if label.is_empty() || !label.bytes().all(is_ident_char) {
                return None;
            }
            if all_digits(label) && label.len() > 1 && label.starts_with('0') {
                return None;
            }
            labels.push(label.to_string());
        }
    }
    Some(NugetVersion { parts, labels })
}

// ---------------------------------------------------------------------------
// intdot
// ---------------------------------------------------------------------------

fn parse_intdot_strict(text: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for part in text.split('.') {
        if !all_digits(part) {
            return None;
        }
        out.push(normalize_digits(part));
    }
    Some(out)
}

fn parse_intdot_lenient(text: &str) -> Option<Vec<String>> {
    let text = text.trim();
    let end = text
        .bytes()
        .position(|b| !(b.is_ascii_digit() || b == b'.'))
        .unwrap_or(text.len());
    let prefix = text[..end].trim_end_matches('.');
    if !prefix.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    parse_intdot_strict(prefix)
}

fn cmp_intdot(a: &[String], b: &[String]) -> Ordering {
    let zero = "0".to_string();
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).unwrap_or(&zero);
        let y = b.get(i).unwrap_or(&zero);
        match cmp_num(x, y) {
            Ordering::Equal => {}
            o => return o,
        }
    }
    Ordering::Equal
}

// ---------------------------------------------------------------------------
// vers ranges
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Comparator {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Comparator {
    fn is_lesser(self) -> bool {
        matches!(self, Comparator::Lt | Comparator::Le)
    }

    fn is_greater(self) -> bool {
        matches!(self, Comparator::Gt | Comparator::Ge)
    }

    fn holds(self, ordering: Ordering) -> bool {
        match self {
            Comparator::Eq => ordering == Ordering::Equal,
            Comparator::Ne => ordering != Ordering::Equal,
            Comparator::Lt => ordering == Ordering::Less,
            Comparator::Le => ordering != Ordering::Greater,
            Comparator::Gt => ordering == Ordering::Greater,
            Comparator::Ge => ordering != Ordering::Less,
        }
    }
}

#[derive(Clone, Debug)]
struct Constraint {
    comparator: Comparator,
    version: Version,
}

/// A validated purl `vers` range.
#[derive(Clone, Debug)]
pub struct VersRange {
    text: String,
    scheme: VersionScheme,
    /// Empty means `*` (every version).
    constraints: Vec<Constraint>,
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

impl VersRange {
    /// Parses and validates a `vers` range. The error is a human-readable
    /// reason.
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.chars().any(char::is_whitespace) {
            return Err("must not contain whitespace".into());
        }
        let rest = text
            .strip_prefix("vers:")
            .ok_or("must start with 'vers:'")?;
        let (scheme_text, constraints_text) = rest
            .split_once('/')
            .ok_or("must have the form 'vers:<scheme>/<constraints>'")?;
        let scheme = VersionScheme::parse(scheme_text)
            .ok_or_else(|| format!("unsupported versioning scheme '{scheme_text}'"))?;
        if constraints_text.is_empty() {
            return Err("must have at least one constraint".into());
        }
        let raw: Vec<&str> = constraints_text.split('|').collect();
        if raw.iter().any(|c| c.is_empty()) {
            return Err("must not have an empty constraint".into());
        }
        if raw.contains(&"*") {
            if raw.len() != 1 {
                return Err("'*' must be the only constraint".into());
            }
            return Ok(Self {
                text: text.to_string(),
                scheme,
                constraints: Vec::new(),
            });
        }
        let mut constraints = Vec::with_capacity(raw.len());
        for item in raw {
            let (comparator, encoded) = if let Some(v) = item.strip_prefix(">=") {
                (Comparator::Ge, v)
            } else if let Some(v) = item.strip_prefix("<=") {
                (Comparator::Le, v)
            } else if let Some(v) = item.strip_prefix("!=") {
                (Comparator::Ne, v)
            } else if let Some(v) = item.strip_prefix('>') {
                (Comparator::Gt, v)
            } else if let Some(v) = item.strip_prefix('<') {
                (Comparator::Lt, v)
            } else {
                (Comparator::Eq, item)
            };
            if encoded.is_empty() {
                return Err(format!("constraint '{item}' has no version"));
            }
            if encoded.contains(['<', '>', '=', '!', '*', '|']) {
                return Err(format!(
                    "constraint '{item}' has an invalid comparator or an unencoded reserved character"
                ));
            }
            let decoded = percent_decode(encoded)
                .ok_or_else(|| format!("constraint '{item}' has an invalid percent-encoding"))?;
            let version = scheme
                .parse_literal(&decoded)
                .ok_or_else(|| format!("'{decoded}' is not a valid {scheme} version"))?;
            constraints.push(Constraint {
                comparator,
                version,
            });
        }
        for pair in constraints.windows(2) {
            match pair[0].version.cmp(&pair[1].version) {
                Ordering::Less => {}
                Ordering::Equal => return Err("constraint versions must be unique".into()),
                Ordering::Greater => {
                    return Err("constraints must be sorted by ascending version".into())
                }
            }
        }
        let no_ne: Vec<Comparator> = constraints
            .iter()
            .map(|c| c.comparator)
            .filter(|c| *c != Comparator::Ne)
            .collect();
        for pair in no_ne.windows(2) {
            if pair[0] == Comparator::Eq
                && !matches!(pair[1], Comparator::Eq | Comparator::Gt | Comparator::Ge)
            {
                return Err("an '=' constraint may only be followed by '=', '>', or '>='".into());
            }
        }
        let bounds: Vec<Comparator> = no_ne.into_iter().filter(|c| *c != Comparator::Eq).collect();
        for pair in bounds.windows(2) {
            if pair[0].is_lesser() == pair[1].is_lesser() {
                return Err("'<'/'<=' and '>'/'>=' constraints must alternate".into());
            }
        }
        Ok(Self {
            text: text.to_string(),
            scheme,
            constraints,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn scheme(&self) -> VersionScheme {
        self.scheme
    }

    /// Whether `version` is in the range (VERS containment algorithm). A
    /// range made only of `!=` constraints contains every other version.
    pub fn contains(&self, version: &Version) -> bool {
        if self.constraints.is_empty() {
            return true;
        }
        for c in &self.constraints {
            let equal = version.cmp(&c.version) == Ordering::Equal;
            if equal && c.comparator == Comparator::Eq {
                return true;
            }
            if equal && c.comparator == Comparator::Ne {
                return false;
            }
        }
        let bounds: Vec<&Constraint> = self
            .constraints
            .iter()
            .filter(|c| !matches!(c.comparator, Comparator::Eq | Comparator::Ne))
            .collect();
        if bounds.is_empty() {
            return self
                .constraints
                .iter()
                .any(|c| c.comparator == Comparator::Ne);
        }
        let holds = |c: &Constraint| c.comparator.holds(version.cmp(&c.version));
        if bounds[0].comparator.is_lesser() && holds(bounds[0]) {
            return true;
        }
        let last = bounds[bounds.len() - 1];
        if last.comparator.is_greater() && holds(last) {
            return true;
        }
        bounds.windows(2).any(|pair| {
            pair[0].comparator.is_greater()
                && pair[1].comparator.is_lesser()
                && holds(pair[0])
                && holds(pair[1])
        })
    }

    /// The range as intervals plus excluded points.
    fn intervals(&self) -> (Vec<Interval<'_>>, Vec<&Version>) {
        let excluded: Vec<&Version> = self
            .constraints
            .iter()
            .filter(|c| c.comparator == Comparator::Ne)
            .map(|c| &c.version)
            .collect();
        if self.constraints.is_empty() {
            return (vec![Interval::default()], excluded);
        }
        let mut out = Vec::new();
        for c in &self.constraints {
            if c.comparator == Comparator::Eq {
                out.push(Interval {
                    lo: Some((&c.version, true)),
                    hi: Some((&c.version, true)),
                });
            }
        }
        let bounds: Vec<&Constraint> = self
            .constraints
            .iter()
            .filter(|c| !matches!(c.comparator, Comparator::Eq | Comparator::Ne))
            .collect();
        if bounds.is_empty() {
            if out.is_empty() {
                out.push(Interval::default());
            }
            return (out, excluded);
        }
        fn bound(c: &Constraint) -> (&Version, bool) {
            (
                &c.version,
                matches!(c.comparator, Comparator::Le | Comparator::Ge),
            )
        }
        let mut i = 0;
        if bounds[0].comparator.is_lesser() {
            out.push(Interval {
                lo: None,
                hi: Some(bound(bounds[0])),
            });
            i = 1;
        }
        while i < bounds.len() {
            let lo = Some(bound(bounds[i]));
            let hi = bounds.get(i + 1).map(|c| bound(c));
            out.push(Interval { lo, hi });
            i += 2;
        }
        (out, excluded)
    }

    /// Whether some version is in both ranges. Intervals are treated as
    /// dense: a non-degenerate intersection always contains a version that
    /// no finite set of `!=` constraints excludes.
    pub fn overlaps(&self, other: &VersRange) -> bool {
        let (a, a_excluded) = self.intervals();
        let (b, b_excluded) = other.intervals();
        for x in &a {
            for y in &b {
                let lo = max_lo(x.lo, y.lo);
                let hi = min_hi(x.hi, y.hi);
                match (lo, hi) {
                    (Some((l, li)), Some((h, hi_inc))) => match l.cmp(h) {
                        Ordering::Less => return true,
                        Ordering::Equal if li && hi_inc => {
                            if !a_excluded.contains(&l) && !b_excluded.contains(&l) {
                                return true;
                            }
                        }
                        _ => {}
                    },
                    _ => return true,
                }
            }
        }
        false
    }
}

impl fmt::Display for VersRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

type Bound<'a> = Option<(&'a Version, bool)>;

#[derive(Clone, Copy, Default)]
struct Interval<'a> {
    lo: Bound<'a>,
    hi: Bound<'a>,
}

fn max_lo<'a>(a: Bound<'a>, b: Bound<'a>) -> Bound<'a> {
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some((va, ia)), Some((vb, ib))) => match va.cmp(vb) {
            Ordering::Greater => Some((va, ia)),
            Ordering::Less => Some((vb, ib)),
            Ordering::Equal => Some((va, ia && ib)),
        },
    }
}

fn min_hi<'a>(a: Bound<'a>, b: Bound<'a>) -> Bound<'a> {
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some((va, ia)), Some((vb, ib))) => match va.cmp(vb) {
            Ordering::Less => Some((va, ia)),
            Ordering::Greater => Some((vb, ib)),
            Ordering::Equal => Some((va, ia && ib)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(scheme: VersionScheme, text: &str) -> Version {
        scheme
            .parse_version(text)
            .unwrap_or_else(|| panic!("{scheme} {text}"))
    }

    fn assert_ascending(scheme: VersionScheme, versions: &[&str]) {
        for pair in versions.windows(2) {
            assert!(
                v(scheme, pair[0]) < v(scheme, pair[1]),
                "{scheme}: {} < {}",
                pair[0],
                pair[1]
            );
        }
    }

    fn range(text: &str) -> VersRange {
        VersRange::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    #[test]
    fn semver_parsing_and_precedence() {
        let s = VersionScheme::Semver;
        for bad in [
            "1",
            "1.2",
            "01.2.3",
            "1.2.3-01",
            "1.2.3-",
            "1.2.3+",
            "v1.2.3",
            " 1.2.3",
            "1.2.3-a..b",
        ] {
            assert!(s.parse_version(bad).is_none(), "{bad}");
        }
        assert_ascending(
            s,
            &[
                "1.0.0-alpha",
                "1.0.0-alpha.1",
                "1.0.0-alpha.beta",
                "1.0.0-beta",
                "1.0.0-beta.2",
                "1.0.0-beta.11",
                "1.0.0-rc.1",
                "1.0.0",
                "1.0.1",
                "99999999999999999999.0.0",
            ],
        );
        assert_eq!(v(s, "1.0.0+a"), v(s, "1.0.0+b"));
    }

    #[test]
    fn npm_accepts_v_and_whitespace_but_not_loose_forms() {
        let n = VersionScheme::Npm;
        assert_eq!(v(n, " v1.2.3 "), v(n, "1.2.3"));
        for bad in [
            "=1.2.3",
            "1.2",
            "1.2.3.4",
            "9007199254740992.0.0",
            "vv1.2.3",
        ] {
            assert!(n.parse_version(bad).is_none(), "{bad}");
        }
        assert!(n.parse_version("9007199254740991.0.0").is_some());
        assert!(n
            .parse_version(&format!("1.2.3-{}", "a".repeat(260)))
            .is_none());
    }

    #[test]
    fn pep440_parsing_and_ordering() {
        let p = VersionScheme::Pypi;
        assert_eq!(v(p, "1.0"), v(p, "1.0.0"));
        assert_eq!(v(p, "1.0ALPHA1"), v(p, "1.0a1"));
        assert_eq!(v(p, "1.0-1"), v(p, "1.0.post1"));
        assert_eq!(v(p, "1.0.rev"), v(p, "1.0.post0"));
        assert_eq!(v(p, "v1.0c1"), v(p, "1.0rc1"));
        assert_eq!(v(p, "1.0-preview_2"), v(p, "1.0rc2"));
        for bad in ["", "1.0+", "1.0+a..b", "1.0x", "a1", "1.0.", "1!"] {
            assert!(p.parse_version(bad).is_none(), "{bad}");
        }
        assert_ascending(
            p,
            &[
                "1.0.dev0",
                "1.0a1.dev1",
                "1.0a1",
                "1.0b1",
                "1.0rc1",
                "1.0",
                "1.0+abc",
                "1.0+1",
                "1.0.post1.dev0",
                "1.0.post1",
                "1.1",
                "1!0.1",
            ],
        );
    }

    #[test]
    fn nuget_parsing_and_ordering() {
        let n = VersionScheme::Nuget;
        assert_eq!(v(n, "1"), v(n, "1.0.0.0"));
        assert_eq!(v(n, "1.0.0+meta"), v(n, "1.0.0"));
        assert_eq!(v(n, "1.0.0-BETA"), v(n, "1.0.0-beta"));
        for bad in ["1.2.3.4.5", "1.0.0-01", "1.0.0-", "a", "2147483648", "1..2"] {
            assert!(n.parse_version(bad).is_none(), "{bad}");
        }
        assert_ascending(
            n,
            &[
                "1.0.0-1",
                "1.0.0-alpha",
                "1.0.0-alpha.1",
                "1.0.0-beta",
                "1.0.0",
                "1.0.0.1",
                "1.0.1",
            ],
        );
    }

    #[test]
    fn intdot_parsing_and_ordering() {
        let i = VersionScheme::Intdot;
        assert_eq!(v(i, "2.40"), v(i, "2.40.0"));
        assert_eq!(v(i, "2.45.1.windows.1"), v(i, "2.45.1"));
        assert_eq!(v(i, "2.45.1-rc0"), v(i, "2.45.1"));
        for bad in ["", "windows", ".1", "1..2", "x2"] {
            assert!(i.parse_version(bad).is_none(), "{bad}");
        }
        assert_ascending(
            i,
            &["2.9", "2.10", "2.40", "2.40.0.1", "100000000000000000000"],
        );
        assert!(VersRange::parse("vers:intdot/>=2.45.1.windows.1").is_err());
    }

    #[test]
    fn vers_syntax_validation() {
        for bad in [
            "",
            "2.0",
            "vers:git/1.0",
            "vers:NPM/1.0.0",
            "vers:npm/",
            "vers:npm/ 1.0.0",
            "vers:npm/=1.0.0",
            "vers:npm/>=1.0.0||<2.0.0",
            "vers:npm/*|>=1.0.0",
            "vers:npm/>=2.0.0|<1.0.0",
            "vers:npm/1.0.0|1.0.0",
            "vers:npm/>=1.0.0|>=2.0.0",
            "vers:npm/<1.0.0|<2.0.0",
            "vers:npm/1.0.0|<2.0.0",
            "vers:npm/>=1.0.0|<2.0.0|<3.0.0",
            "vers:npm/>=1.0.0-a%2",
            "vers:npm/>=1.0.0-a%zz",
            "vers:npm/>=1.0.0-a!b",
            "vers:intdot/>=1.x",
        ] {
            assert!(VersRange::parse(bad).is_err(), "{bad}");
        }
        for good in [
            "vers:npm/*",
            "vers:npm/1.0.0",
            "vers:npm/>=1.0.0|<2.0.0",
            "vers:npm/<1.0.0|>=2.0.0",
            "vers:npm/1.0.0|>=2.0.0",
            "vers:npm/!=1.5.0",
            "vers:npm/>=1.0.0|!=1.5.0|<2.0.0",
            "vers:pypi/>=1.0%2Blocal",
            "vers:intdot/>=2.40|<2.50",
            "vers:nuget/>=1.0|<2",
            "vers:semver/>=1.0.0-rc.1",
        ] {
            assert!(VersRange::parse(good).is_ok(), "{good}");
        }
    }

    #[test]
    fn vers_containment() {
        let i = VersionScheme::Intdot;
        let r = range("vers:intdot/>=2.40|<2.50");
        assert!(r.contains(&v(i, "2.40")));
        assert!(r.contains(&v(i, "2.49.9")));
        assert!(!r.contains(&v(i, "2.50")));
        assert!(!r.contains(&v(i, "2.39.9")));
        let r = range("vers:intdot/<1|>=2|<3|5|>7");
        for (ver, inside) in [
            ("0.9", true),
            ("1", false),
            ("2.5", true),
            ("3", false),
            ("5", true),
            ("6", false),
            ("8", true),
        ] {
            assert_eq!(r.contains(&v(i, ver)), inside, "{ver}");
        }
        let r = range("vers:intdot/!=2");
        assert!(r.contains(&v(i, "1")) && !r.contains(&v(i, "2")));
        let r = range("vers:intdot/>=1|!=1.5|<2");
        assert!(r.contains(&v(i, "1.4")) && !r.contains(&v(i, "1.5")));
        assert!(range("vers:intdot/*").contains(&v(i, "0")));
        assert!(!range("vers:intdot/1|2").contains(&v(i, "1.5")));
    }

    #[test]
    fn vers_overlap() {
        let overlaps = |a: &str, b: &str| range(a).overlaps(&range(b));
        assert!(!overlaps(
            "vers:intdot/>=2.40|<2.50",
            "vers:intdot/>=2.50|<3"
        ));
        assert!(overlaps(
            "vers:intdot/>=2.40|<=2.50",
            "vers:intdot/>=2.50|<3"
        ));
        assert!(overlaps("vers:intdot/>=2.40|<2.50", "vers:intdot/>=2.45"));
        assert!(overlaps("vers:intdot/*", "vers:intdot/5"));
        assert!(!overlaps("vers:intdot/5", "vers:intdot/!=5"));
        assert!(overlaps("vers:intdot/>=4|<6", "vers:intdot/!=5"));
        assert!(!overlaps("vers:intdot/<1", "vers:intdot/>1"));
        assert!(!overlaps("vers:intdot/1|3", "vers:intdot/>=1.5|<2.5"));
        assert!(overlaps("vers:intdot/1|3", "vers:intdot/>=2.5"));
        assert!(!overlaps("vers:intdot/>=1|!=2|<3", "vers:intdot/2"));
    }
}
