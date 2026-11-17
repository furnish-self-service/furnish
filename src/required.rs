//! Required versions: a `Furnish/Package` or `Furnish/Installer` resource may list the versions
//! that are acceptable (`properties.versions`). The installed version is acceptable when it matches
//! at least one entry:
//!
//! - a fixed version (`115.15.0esr`): exactly this version;
//! - a range (`>=128.0.3, <129`): comparisons joined by commas, all of which must hold;
//! - a regular expression (`{ regex: '^115\.' }`), matched against the version string.
//!
//! Versions compare by the rules of whoever installs them (see [`Scheme`]). furnish takes
//! `versions` out of the document before DSC sees it and tells the resource what it needs instead:
//! `reportVersions` (`latest` or `all`) makes it report the versions it can install, and
//! `targetVersion` makes it install exactly one (see [`decide`]).

use std::cmp::Ordering;

use anyhow::{Result, bail};
use serde::Serialize;
use serde_json::Value;

/// The property apps write the acceptable versions in.
pub const PROPERTY: &str = "versions";

/// A comparison operator of a range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Op {
    #[serde(rename = ">=")]
    Ge,
    #[serde(rename = ">")]
    Gt,
    #[serde(rename = "<=")]
    Le,
    #[serde(rename = "<")]
    Lt,
    #[serde(rename = "=")]
    Eq,
    #[serde(rename = "!=")]
    Ne,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Comparator {
    pub op: Op,
    pub version: String,
}

/// One entry of `versions`. A fixed version is a range with one `=`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Entry {
    Range { all: Vec<Comparator> },
    Regex { pattern: String },
}

/// How versions compare, as the resource reports it (`versionScheme`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    /// pacman's `vercmp` (libalpm).
    Pacman,
    /// `dpkg --compare-versions`: apt and .deb files.
    Dpkg,
    /// rpm's `rpmvercmp`: dnf, zypper and .rpm files.
    Rpm,
    /// Homebrew's version strings.
    Brew,
    /// Nix's `builtins.compareVersions`.
    Nix,
    /// winget's version strings.
    Winget,
    /// Numbers compare as numbers, pre-release words before the release (see `generic`).
    Generic,
}

/// What an app's resource needs and what this computer has, for the app's status: set when the
/// installed version isn't acceptable, or when the package manager offers no acceptable version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Required {
    /// The resource's name.
    pub resource: String,
    /// The installed version; `None`: not installed.
    pub installed: Option<String>,
    /// The installed version is acceptable (also `true` when nothing is installed).
    pub acceptable: bool,
    /// The newest version the package manager offers; for an installer, the version in the
    /// collection's file. `None` if unknown.
    pub offered: Option<String>,
    /// What installs it, e.g. `winget`, `apt`, `Homebrew`.
    pub via: String,
    /// A `Furnish/Installer`: what is offered is the collection's file.
    pub installer: bool,
    /// The version furnish installs; `None`: no acceptable version is offered.
    pub target: Option<String>,
    /// The acceptable versions, as the app lists them.
    pub needs: Vec<Entry>,
    /// In English, e.g. "128.0.3 or later; winget offers 128.0.1".
    pub summary: String,
}

// --- Parsing

const OPS: [(&str, Op); 7] = [(">=", Op::Ge), ("<=", Op::Le), ("!=", Op::Ne), ("==", Op::Eq), (">", Op::Gt), ("<", Op::Lt), ("=", Op::Eq)];

/// The entries of a `versions` property: a non-empty list of strings and `{ regex: … }`.
pub fn parse(value: &Value) -> Result<Vec<Entry>> {
    let Some(items) = value.as_array().filter(|a| !a.is_empty()) else {
        bail!("versions: a list of acceptable versions, such as [\">=128.0.3\"]");
    };
    items.iter().map(parse_entry).collect()
}

fn parse_entry(item: &Value) -> Result<Entry> {
    match item {
        Value::String(text) => parse_text(text),
        Value::Object(map) if map.len() == 1 && map.contains_key("regex") => {
            let Some(pattern) = map["regex"].as_str() else { bail!("versions: regex must be a string") };
            if let Err(err) = regex::Regex::new(pattern) {
                bail!("versions: regex '{pattern}' doesn't compile: {err}");
            }
            Ok(Entry::Regex { pattern: pattern.to_string() })
        }
        other => bail!("versions: {other} is neither a version, a range such as \">=1.2, <2\" nor {{ regex: … }}"),
    }
}

fn parse_text(text: &str) -> Result<Entry> {
    let text = text.trim();
    if text.is_empty() {
        bail!("versions: an entry is empty");
    }
    if !text.starts_with(['<', '>', '=', '!']) {
        if text.contains(',') || text.contains(char::is_whitespace) {
            bail!("versions: '{text}': a range starts with an operator (>=, >, <=, <, =, !=), a fixed version has no spaces or commas");
        }
        return Ok(Entry::Range { all: vec![Comparator { op: Op::Eq, version: text.to_string() }] });
    }
    let all = text
        .split(',')
        .map(|part| {
            let part = part.trim();
            let Some((sign, op)) = OPS.iter().find(|(sign, _)| part.starts_with(sign)) else {
                bail!("versions: '{text}': each part of a range starts with an operator (>=, >, <=, <, =, !=), '{part}' doesn't");
            };
            let version = part[sign.len()..].trim();
            if version.is_empty() || version.contains(char::is_whitespace) || version.contains(['<', '>', '=', '!']) {
                bail!("versions: '{text}': '{part}' needs one version after its operator");
            }
            Ok(Comparator { op: *op, version: version.to_string() })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Entry::Range { all })
}

impl Scheme {
    /// The resource's `versionScheme`; versions of unknown schemes compare generically.
    pub fn named(name: &str) -> Scheme {
        match name {
            "pacman" => Scheme::Pacman,
            "dpkg" => Scheme::Dpkg,
            "rpm" => Scheme::Rpm,
            "brew" => Scheme::Brew,
            "nix" => Scheme::Nix,
            "winget" => Scheme::Winget,
            _ => Scheme::Generic,
        }
    }

    /// The scheme of an installer file, by its extension.
    pub fn of_file(file: &str) -> Scheme {
        match file.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
            Some("deb") => Scheme::Dpkg,
            Some("rpm") => Scheme::Rpm,
            _ => Scheme::Generic,
        }
    }
}

// --- Matching

/// Whether `version` is acceptable: it matches at least one entry.
pub fn matches(entries: &[Entry], scheme: Scheme, version: &str) -> bool {
    entries.iter().any(|entry| match entry {
        Entry::Range { all } => all.iter().all(|c| {
            let order = compare(scheme, version, &c.version);
            match c.op {
                Op::Ge => order != Ordering::Less,
                Op::Gt => order == Ordering::Greater,
                Op::Le => order != Ordering::Greater,
                Op::Lt => order == Ordering::Less,
                Op::Eq => order == Ordering::Equal,
                Op::Ne => order != Ordering::Equal,
            }
        }),
        Entry::Regex { pattern } => regex::Regex::new(pattern).is_ok_and(|r| r.is_match(version)),
    })
}

/// An entry that no version can match, e.g. `>=2, <1`, judged by `scheme`. For `furnish check`.
pub fn never_matches(entry: &Entry, scheme: Scheme) -> bool {
    let Entry::Range { all } = entry else { return false };
    // The highest lower bound and the lowest upper bound, each with whether it is included.
    type Bound<'a> = Option<(&'a str, bool)>;
    let (mut low, mut high): (Bound, Bound) = (None, None);
    for c in all {
        let (bound, included, is_low) = match c.op {
            Op::Ge => (&c.version, true, true),
            Op::Gt => (&c.version, false, true),
            Op::Le => (&c.version, true, false),
            Op::Lt => (&c.version, false, false),
            Op::Eq => {
                if all.iter().any(|o| o.op == Op::Eq && compare(scheme, &o.version, &c.version) != Ordering::Equal)
                    || !matches(std::slice::from_ref(entry), scheme, &c.version)
                {
                    return true;
                }
                continue;
            }
            Op::Ne => continue,
        };
        let slot = if is_low { &mut low } else { &mut high };
        let tighter = match *slot {
            None => true,
            Some((v, inc)) => match compare(scheme, bound, v) {
                Ordering::Equal => inc && !included,
                order => (order == Ordering::Greater) == is_low,
            },
        };
        if tighter {
            *slot = Some((bound, included));
        }
    }
    match (low, high) {
        (Some((l, li)), Some((h, hi))) => match compare(scheme, l, h) {
            Ordering::Greater => true,
            Ordering::Equal => !(li && hi) || all.iter().any(|c| c.op == Op::Ne && compare(scheme, &c.version, l) == Ordering::Equal),
            Ordering::Less => false,
        },
        _ => false,
    }
}

/// The entries in English, e.g. "128.0.3 or later and earlier than 129, or 115.15.0esr".
pub fn describe(entries: &[Entry]) -> String {
    let parts: Vec<String> = entries
        .iter()
        .map(|entry| match entry {
            Entry::Range { all } => all
                .iter()
                .map(|c| match c.op {
                    Op::Ge => format!("{} or later", c.version),
                    Op::Gt => format!("later than {}", c.version),
                    Op::Le => format!("{} or earlier", c.version),
                    Op::Lt => format!("earlier than {}", c.version),
                    Op::Eq => c.version.clone(),
                    Op::Ne => format!("not {}", c.version),
                })
                .collect::<Vec<_>>()
                .join(" and "),
            Entry::Regex { pattern } => format!("a version matching {pattern}"),
        })
        .collect();
    match parts.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{}, or {last}", rest.join(", ")),
    }
}

// --- Comparing

/// Compare two versions by the rules of `scheme`.
pub fn compare(scheme: Scheme, a: &str, b: &str) -> Ordering {
    match scheme {
        Scheme::Pacman => evr(a, b, alpm_vercmp),
        Scheme::Dpkg => evr(a, b, dpkg_vercmp),
        Scheme::Rpm => evr(a, b, rpm_vercmp),
        Scheme::Nix => nix_compare(a, b),
        Scheme::Winget => winget_compare(a, b),
        Scheme::Brew | Scheme::Generic => generic(a, b),
    }
}

/// `[epoch:]version[-release]`: epochs (default 0), then versions, then releases, which only
/// count if both have one. So `>=128.0.3` takes `128.0.3-1`, as dnf, pacman and apt users expect.
/// pacman and rpm compare this way; dpkg would rank a missing revision lowest.
fn evr(a: &str, b: &str, cmp: fn(&str, &str) -> Ordering) -> Ordering {
    let split = |v: &str| {
        let (epoch, rest) = match v.split_once(':') {
            Some((e, rest)) if !e.is_empty() && e.bytes().all(|c| c.is_ascii_digit()) => (e.trim_start_matches('0').to_string(), rest),
            _ => (String::new(), v),
        };
        let (version, release) = match rest.rsplit_once('-') {
            Some((version, release)) => (version.to_string(), Some(release.to_string())),
            None => (rest.to_string(), None),
        };
        (epoch, version, release)
    };
    let (ea, va, ra) = split(a);
    let (eb, vb, rb) = split(b);
    // Epochs are numbers: compare by length, then digits (leading zeros removed).
    ea.len()
        .cmp(&eb.len())
        .then_with(|| ea.cmp(&eb))
        .then_with(|| cmp(&va, &vb))
        .then_with(|| match (ra, rb) {
            (Some(ra), Some(rb)) => cmp(&ra, &rb),
            _ => Ordering::Equal,
        })
}

/// Numeric strings compared as numbers, without leading zeros.
fn numeric(a: &[u8], b: &[u8]) -> Ordering {
    let strip = |s: &[u8]| {
        let start = s.iter().position(|&c| c != b'0').unwrap_or(s.len());
        s[start..].to_vec()
    };
    let (a, b) = (strip(a), strip(b));
    a.len().cmp(&b.len()).then_with(|| a.cmp(&b))
}

/// dpkg's `verrevcmp`: `~` sorts before everything, even the end; letters before other characters.
fn dpkg_vercmp(a: &str, b: &str) -> Ordering {
    fn order(c: Option<u8>) -> i32 {
        match c {
            None => 0,
            Some(c) if c.is_ascii_digit() => 0,
            Some(c) if c.is_ascii_alphabetic() => i32::from(c),
            Some(b'~') => -1,
            Some(c) => i32::from(c) + 256,
        }
    }
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        while (i < a.len() && !a[i].is_ascii_digit()) || (j < b.len() && !b[j].is_ascii_digit()) {
            let (ac, bc) = (order(a.get(i).copied()), order(b.get(j).copied()));
            if ac != bc {
                return ac.cmp(&bc);
            }
            i += 1;
            j += 1;
        }
        let (si, sj) = (i, j);
        while i < a.len() && a[i].is_ascii_digit() {
            i += 1;
        }
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        let order = numeric(&a[si.min(a.len())..i.min(a.len())], &b[sj.min(b.len())..j.min(b.len())]);
        if order != Ordering::Equal {
            return order;
        }
    }
    Ordering::Equal
}

/// rpm's `rpmvercmp`, with `~` (sorts before everything) and `^` (after the version, before more).
fn rpm_vercmp(a: &str, b: &str) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    let sep = |c: u8| !c.is_ascii_alphanumeric() && c != b'~' && c != b'^';
    loop {
        while i < a.len() && sep(a[i]) {
            i += 1;
        }
        while j < b.len() && sep(b[j]) {
            j += 1;
        }
        let (ca, cb) = (a.get(i).copied(), b.get(j).copied());
        if ca == Some(b'~') || cb == Some(b'~') {
            if ca != Some(b'~') {
                return Ordering::Greater;
            }
            if cb != Some(b'~') {
                return Ordering::Less;
            }
            i += 1;
            j += 1;
            continue;
        }
        if ca == Some(b'^') || cb == Some(b'^') {
            if ca.is_none() {
                return Ordering::Less;
            }
            if cb.is_none() {
                return Ordering::Greater;
            }
            if ca != Some(b'^') {
                return Ordering::Greater;
            }
            if cb != Some(b'^') {
                return Ordering::Less;
            }
            i += 1;
            j += 1;
            continue;
        }
        if ca.is_none() || cb.is_none() {
            break;
        }
        let digits = a[i].is_ascii_digit();
        let take = |s: &[u8], k: &mut usize| {
            let start = *k;
            while *k < s.len() && if digits { s[*k].is_ascii_digit() } else { s[*k].is_ascii_alphabetic() } {
                *k += 1;
            }
            start
        };
        let (si, sj) = (take(a, &mut i), take(b, &mut j));
        if sj == j {
            // The other has a segment of the other kind here: numbers are newer than letters.
            return if digits { Ordering::Greater } else { Ordering::Less };
        }
        let order = if digits { numeric(&a[si..i], &b[sj..j]) } else { a[si..i].cmp(&b[sj..j]) };
        if order != Ordering::Equal {
            return order;
        }
    }
    match (i >= a.len(), j >= b.len()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Less,
        _ => Ordering::Greater,
    }
}

/// libalpm's `rpmvercmp`: like rpm's, but separators of different lengths decide, and a letter
/// suffix sorts before the release (`1.0a` < `1.0` < `1.0.a` < `1.0.1`).
fn alpm_vercmp(a: &str, b: &str) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    let (mut pi, mut pj) = (0, 0);
    while i < a.len() && j < b.len() {
        while i < a.len() && !a[i].is_ascii_alphanumeric() {
            i += 1;
        }
        while j < b.len() && !b[j].is_ascii_alphanumeric() {
            j += 1;
        }
        if i >= a.len() || j >= b.len() {
            break;
        }
        if i - pi != j - pj {
            return (i - pi).cmp(&(j - pj));
        }
        let digits = a[i].is_ascii_digit();
        let kind = |c: u8| if digits { c.is_ascii_digit() } else { c.is_ascii_alphabetic() };
        let (si, sj) = (i, j);
        while i < a.len() && kind(a[i]) {
            i += 1;
        }
        while j < b.len() && kind(b[j]) {
            j += 1;
        }
        if sj == j {
            return if digits { Ordering::Greater } else { Ordering::Less };
        }
        let order = if digits { numeric(&a[si..i], &b[sj..j]) } else { a[si..i].cmp(&b[sj..j]) };
        if order != Ordering::Equal {
            return order;
        }
        pi = i;
        pj = j;
    }
    let (ra, rb) = (a.get(i).copied(), b.get(j).copied());
    match (ra, rb) {
        (None, None) => Ordering::Equal,
        // A remaining letter never beats the end; anything else does.
        (None, Some(c)) if !c.is_ascii_alphabetic() => Ordering::Less,
        (Some(c), _) if c.is_ascii_alphabetic() => Ordering::Less,
        _ => Ordering::Greater,
    }
}

/// Nix's `builtins.compareVersions`: parts split at `.` and `-` and where digits start or end;
/// numbers compare as numbers, `pre` before anything, other words before numbers.
fn nix_compare(a: &str, b: &str) -> Ordering {
    fn parts(v: &str) -> Vec<&str> {
        let mut out = Vec::new();
        let bytes = v.as_bytes();
        let mut k = 0;
        while k < bytes.len() {
            if bytes[k] == b'.' || bytes[k] == b'-' {
                k += 1;
                continue;
            }
            let start = k;
            let digit = bytes[k].is_ascii_digit();
            while k < bytes.len() && bytes[k] != b'.' && bytes[k] != b'-' && bytes[k].is_ascii_digit() == digit {
                k += 1;
            }
            out.push(&v[start..k]);
        }
        out
    }
    fn less(c1: &str, c2: &str) -> bool {
        let num = |c: &str| !c.is_empty() && c.bytes().all(|b| b.is_ascii_digit());
        match (num(c1), num(c2)) {
            (true, true) => numeric(c1.as_bytes(), c2.as_bytes()) == Ordering::Less,
            _ if c1.is_empty() && num(c2) => true,
            _ if c1 == "pre" && c2 != "pre" => true,
            _ if c2 == "pre" => false,
            (_, true) => true,
            (true, _) => false,
            _ => c1 < c2,
        }
    }
    let (pa, pb) = (parts(a), parts(b));
    for k in 0..pa.len().max(pb.len()) {
        let (c1, c2) = (pa.get(k).copied().unwrap_or(""), pb.get(k).copied().unwrap_or(""));
        if less(c1, c2) {
            return Ordering::Less;
        }
        if less(c2, c1) {
            return Ordering::Greater;
        }
    }
    Ordering::Equal
}

/// winget's versions: parts split at `.`, each a number and an optional rest; trailing zero parts
/// don't count (`1.0` = `1.0.0`), and a part with a rest sorts before the bare number
/// (`1.0-beta` < `1.0`). Rests compare ignoring case.
fn winget_compare(a: &str, b: &str) -> Ordering {
    fn parts(v: &str) -> Vec<(u64, String)> {
        let mut parts: Vec<(u64, String)> = v
            .trim()
            .split('.')
            .map(|p| {
                let p = p.trim();
                let end = p.find(|c: char| !c.is_ascii_digit()).unwrap_or(p.len());
                (p[..end].parse().unwrap_or(0), p[end..].to_ascii_lowercase())
            })
            .collect();
        while parts.last().is_some_and(|(n, rest)| *n == 0 && rest.is_empty()) {
            parts.pop();
        }
        parts
    }
    let (pa, pb) = (parts(a), parts(b));
    let zero = (0, String::new());
    for k in 0..pa.len().max(pb.len()) {
        let (x, y) = (pa.get(k).unwrap_or(&zero), pb.get(k).unwrap_or(&zero));
        let order = x.0.cmp(&y.0).then_with(|| match (x.1.is_empty(), y.1.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => x.1.cmp(&y.1),
        });
        if order != Ordering::Equal {
            return order;
        }
    }
    Ordering::Equal
}

/// Homebrew's rules, also used for installers' versions: runs of digits compare as numbers, words
/// as words. alpha, beta, pre and rc (and a, b) come before the release; p, patch and post after
/// it; other words (`esr`) after the release too. Numbers rank above words in the same place.
fn generic(a: &str, b: &str) -> Ordering {
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    enum Token {
        Pre(u8),
        Null,
        Word(String),
        Post,
        Number(Vec<u8>),
    }
    fn tokens(v: &str) -> Vec<Token> {
        let mut out = Vec::new();
        let bytes = v.as_bytes();
        let mut k = 0;
        while k < bytes.len() {
            if !bytes[k].is_ascii_alphanumeric() {
                k += 1;
                continue;
            }
            let start = k;
            let digit = bytes[k].is_ascii_digit();
            while k < bytes.len() && bytes[k].is_ascii_alphanumeric() && bytes[k].is_ascii_digit() == digit {
                k += 1;
            }
            let run = &v[start..k];
            out.push(if digit {
                let trimmed = run.trim_start_matches('0');
                Token::Number(trimmed.as_bytes().to_vec())
            } else {
                match run.to_ascii_lowercase().as_str() {
                    "alpha" | "a" => Token::Pre(0),
                    "beta" | "b" => Token::Pre(1),
                    "pre" => Token::Pre(2),
                    "rc" => Token::Pre(3),
                    "p" | "patch" | "post" | "pl" => Token::Post,
                    word => Token::Word(word.to_string()),
                }
            });
        }
        out
    }
    let cmp = |x: &Token, y: &Token| match (x, y) {
        (Token::Number(m), Token::Number(n)) => m.len().cmp(&n.len()).then_with(|| m.cmp(n)),
        // A missing part is a zero.
        (Token::Null, Token::Number(n)) if n.is_empty() => Ordering::Equal,
        (Token::Number(n), Token::Null) if n.is_empty() => Ordering::Equal,
        _ => x.cmp(y),
    };
    let (ta, tb) = (tokens(a), tokens(b));
    for k in 0..ta.len().max(tb.len()) {
        let order = cmp(ta.get(k).unwrap_or(&Token::Null), tb.get(k).unwrap_or(&Token::Null));
        if order != Ordering::Equal {
            return order;
        }
    }
    Ordering::Equal
}

// --- Deciding what to install

/// What a resource with required versions should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// What the package manager does on its own gives an acceptable version (or furnish can't tell).
    Free,
    /// Install or keep exactly this version (`targetVersion`).
    Exactly(String),
    /// No acceptable version is offered.
    NoneOffered,
}

/// What a resource reports about its versions (with `reportVersions`).
#[derive(Debug, Clone, Default)]
pub struct Facts {
    /// The installed version; `Some("?")`: installed, version unknown.
    pub installed: Option<String>,
    pub latest: Option<String>,
    /// Every version it can install (`reportVersions: all`), if it can install a given one.
    pub available: Option<Vec<String>>,
    pub scheme: Option<Scheme>,
    pub via: String,
    /// It can install a given version (apt, dnf, zypper, winget).
    pub pinnable: bool,
    /// A `Furnish/Installer`: what is offered is the collection's file.
    pub installer: bool,
}

/// Package managers that install a given version: apt `name=version`, dnf and zypper, winget
/// `--version`.
pub fn pinnable(kind: &str, via: &str) -> bool {
    kind == crate::catalog::PACKAGE_TYPE && matches!(via, "apt" | "dnf" | "zypper" | "winget")
}

impl Facts {
    /// From a resource's state.
    pub fn from_state(kind: &str, state: &Value) -> Facts {
        let text = |key: &str| state[key].as_str().filter(|s| !s.is_empty()).map(String::from);
        let exists = !(state["_exist"] == false || state["_exist"] == "false");
        let via = text("via").unwrap_or_default();
        Facts {
            installed: if exists { Some(text("version").unwrap_or_else(|| "?".into())) } else { None },
            latest: text("latestVersion"),
            available: state["availableVersions"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).filter(|v| !v.is_empty()).collect()),
            scheme: text("versionScheme").map(|s| Scheme::named(&s)),
            pinnable: pinnable(kind, &via),
            installer: kind == crate::catalog::INSTALLER_TYPE,
            via,
        }
    }

    fn known_installed(&self) -> Option<&str> {
        self.installed.as_deref().filter(|v| *v != "?")
    }

    /// Whether `decide` needs every version the resource can install (`reportVersions: all`).
    pub fn need_list(&self, entries: &[Entry]) -> bool {
        let scheme = self.scheme.unwrap_or(Scheme::Generic);
        let ok = |v: &str| matches(entries, scheme, v);
        self.pinnable
            && self.available.is_none()
            && match (&self.latest, self.known_installed()) {
                (Some(latest), installed) => !ok(latest) || installed.is_some_and(|i| !ok(i) && compare(scheme, latest, i) != Ordering::Greater),
                (None, Some(installed)) => !ok(installed),
                (None, None) => false,
            }
    }
}

/// What to install, by these rules: the newest version offered if it is acceptable; else, where
/// the package manager installs a given version, the newest acceptable one it offers; else the
/// installed version if it is acceptable; else nothing acceptable is offered.
pub fn decide(entries: &[Entry], facts: &Facts) -> Decision {
    let scheme = facts.scheme.unwrap_or(Scheme::Generic);
    let ok = |v: &str| matches(entries, scheme, v);
    let installed = facts.known_installed();
    let installed_ok = installed.map(ok);
    match facts.latest.as_deref() {
        // The package manager goes there on its own, unless it would have to go back.
        Some(latest) if ok(latest) => match installed {
            Some(i) if installed_ok == Some(false) && compare(scheme, latest, i) != Ordering::Greater => best(entries, facts).map_or(Decision::NoneOffered, Decision::Exactly),
            _ => Decision::Free,
        },
        // Nothing known about what is offered: nothing to judge unless the installed one fails.
        None if installed_ok != Some(false) && facts.available.is_none() => Decision::Free,
        _ => match best(entries, facts) {
            Some(version) => Decision::Exactly(version),
            // Installed, version unknown: leave it be.
            None if facts.installed.as_deref() == Some("?") => Decision::Free,
            None => Decision::NoneOffered,
        },
    }
}

/// The newest acceptable version among those offered (where a given one can be installed) and
/// the installed one.
fn best(entries: &[Entry], facts: &Facts) -> Option<String> {
    let scheme = facts.scheme.unwrap_or(Scheme::Generic);
    let offered = if facts.pinnable { facts.available.clone().unwrap_or_default() } else { vec![] };
    let latest = facts.latest.iter().filter(|_| facts.pinnable).cloned();
    offered
        .into_iter()
        .chain(latest)
        .chain(facts.known_installed().map(String::from))
        .filter(|v| matches(entries, scheme, v))
        .max_by(|a, b| compare(scheme, a, b))
}

/// The decision, and what the app shows if the installed version isn't acceptable or no
/// acceptable version is offered.
pub fn assess(resource: &str, entries: &[Entry], facts: &Facts) -> (Decision, Option<Required>) {
    let decision = decide(entries, facts);
    let scheme = facts.scheme.unwrap_or(Scheme::Generic);
    let installed = facts.known_installed();
    let acceptable = installed.is_none_or(|i| matches(entries, scheme, i));
    let target = match &decision {
        Decision::Exactly(v) => Some(v.clone()),
        Decision::Free => facts.latest.clone().or_else(|| installed.map(String::from)),
        Decision::NoneOffered => None,
    };
    if acceptable && decision != Decision::NoneOffered {
        return (decision, None);
    }
    let offered = match (&facts.latest, facts.installer) {
        (Some(v), true) => format!("the collection's file has {v}"),
        (None, true) => "furnish can't read the version of the collection's file".to_string(),
        (Some(v), false) => format!("{} offers {v}", facts.via),
        (None, false) => format!("{} offers no version", facts.via),
    };
    let required = Required {
        resource: resource.to_string(),
        installed: installed.map(String::from),
        acceptable,
        offered: facts.latest.clone(),
        via: facts.via.clone(),
        installer: facts.installer,
        target,
        needs: entries.to_vec(),
        summary: format!("{}; {offered}", describe(entries)),
    };
    (decision, Some(required))
}

// --- Documents

/// Whether furnish handles `versions` for this resource type.
pub fn applies_to(kind: &str) -> bool {
    kind == crate::catalog::PACKAGE_TYPE || kind == crate::catalog::INSTALLER_TYPE
}

/// The resource's acceptable versions, if it lists them.
pub fn of(resource: &Value) -> Result<Option<Vec<Entry>>> {
    let versions = &resource["properties"][PROPERTY];
    if versions.is_null() || !applies_to(resource["type"].as_str().unwrap_or_default()) {
        return Ok(None);
    }
    parse(versions).map(Some)
}

/// Check the `versions` of a document's resources (and of those nested in groups) when an app
/// loads: only furnish's package and installer resources take them, and they must parse.
pub fn validate(resources: &Value) -> Result<()> {
    for resource in resources.as_array().into_iter().flatten() {
        let name = resource["name"].as_str().unwrap_or("?");
        let kind = resource["type"].as_str().unwrap_or_default();
        if resource["properties"].get(PROPERTY).is_some() && kind.starts_with("Furnish/") {
            if !applies_to(kind) {
                bail!("resource '{name}': {kind} has no versions (the system's nixpkgs decides the version)");
            }
            parse(&resource["properties"][PROPERTY]).map_err(|e| anyhow::anyhow!("resource '{name}': {e}"))?;
        }
        if resource["properties"]["resources"].is_array() {
            if resource["properties"]["resources"].as_array().into_iter().flatten().any(|r| applies_to(r["type"].as_str().unwrap_or_default()) && r["properties"].get(PROPERTY).is_some()) {
                bail!("resource '{name}': versions only work on resources at the top level of an app, not inside a group");
            }
            validate(&resource["properties"]["resources"])?;
        }
    }
    Ok(())
}

/// Take `versions` out of furnish's resources (also nested ones): DSC doesn't know it.
pub fn strip(resources: &mut Value) {
    for resource in resources.as_array_mut().into_iter().flatten() {
        if applies_to(resource["type"].as_str().unwrap_or_default())
            && let Some(properties) = resource["properties"].as_object_mut()
        {
            properties.remove(PROPERTY);
        }
        if resource["properties"]["resources"].is_array() {
            strip(&mut resource["properties"]["resources"]);
        }
    }
}

/// Give an earlier version of an app (to fall back to, or held at) the versions the collection
/// requires now: each of its resources gets those of the current resource with the same name and
/// type. An earlier version must not bring back a version that isn't acceptable anymore.
pub fn carry(current: &Value, earlier: &mut Value) {
    let now = current["resources"].as_array().cloned().unwrap_or_default();
    for resource in earlier["resources"].as_array_mut().into_iter().flatten() {
        let found = now.iter().find(|r| r["name"] == resource["name"] && r["type"] == resource["type"]);
        if let Some(versions) = found.map(|r| &r["properties"][PROPERTY]).filter(|v| !v.is_null())
            && resource["properties"].is_object()
        {
            resource["properties"][PROPERTY] = versions.clone();
        }
    }
}

/// What `furnish check` says about a resource's required versions, as (is an error, message):
/// entries no version can match, and an installer file that isn't acceptable itself. Entries that
/// don't parse already keep the app from loading.
pub fn check(name: &str, kind: &str, properties: &Value) -> Vec<(bool, String)> {
    let Some(Ok(entries)) = properties.get(PROPERTY).filter(|_| applies_to(kind)).map(parse) else { return vec![] };
    let file = properties["file"].as_str().filter(|_| kind == crate::catalog::INSTALLER_TYPE);
    let scheme = file.map_or(Scheme::Generic, Scheme::of_file);
    let mut findings: Vec<(bool, String)> = entries
        .iter()
        .filter(|e| never_matches(e, scheme))
        .map(|e| (false, format!("{name}: no version can be {}", describe(std::slice::from_ref(e)))))
        .collect();
    if let Some(file) = file.map(std::path::Path::new).filter(|f| f.is_file()) {
        let shown = file.file_name().unwrap_or_default().to_string_lossy();
        match file_version(file) {
            Some(v) if !matches(&entries, scheme, &v) => {
                findings.push((true, format!("{name}: {shown} has version {v}, which its versions don't accept (needs {})", describe(&entries))));
            }
            Some(_) => {}
            None => findings.push((false, format!("{name}: furnish can't read the version of {shown} here, so it can't tell whether its versions accept it"))),
        }
    }
    findings
}

// --- Installer files

/// The version in an installer file, where furnish can read it: .deb with `dpkg-deb`, .rpm from
/// its header. For `furnish check`, which runs anywhere.
pub fn file_version(path: &std::path::Path) -> Option<String> {
    let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();
    match extension.as_str() {
        "deb" => {
            let out = std::process::Command::new("dpkg-deb").arg("-f").arg("--").arg(path).arg("Version").output().ok()?;
            let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
            (out.status.success() && !version.is_empty()).then_some(version)
        }
        "rpm" => {
            use std::io::Read;
            let mut head = Vec::new();
            std::fs::File::open(path).ok()?.take(64 << 20).read_to_end(&mut head).ok()?;
            rpm_header_version(&head)
        }
        _ => None,
    }
}

/// `[epoch:]version-release` from an .rpm file's header: the lead (96 bytes), the signature
/// header (padded to 8 bytes), then the header with the tags VERSION (1001), RELEASE (1002) and
/// EPOCH (1003).
fn rpm_header_version(bytes: &[u8]) -> Option<String> {
    let u32_at = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    if bytes.get(..4)? != [0xed, 0xab, 0xee, 0xdb] {
        return None;
    }
    // (index entries: tag, type, offset, count), the store, and where the header ends.
    type Header<'a> = (Vec<(u32, u32, usize)>, &'a [u8], usize);
    let header = |at: usize| -> Option<Header> {
        if bytes.get(at..at + 4)? != [0x8e, 0xad, 0xe8, 0x01] {
            return None;
        }
        let (count, size) = (u32_at(at + 8)? as usize, u32_at(at + 12)? as usize);
        let entries = (0..count)
            .map(|k| {
                let e = at + 16 + k * 16;
                Some((u32_at(e)?, u32_at(e + 4)?, u32_at(e + 8)? as usize))
            })
            .collect::<Option<Vec<_>>>()?;
        let store_at = at + 16 + count * 16;
        Some((entries, bytes.get(store_at..store_at + size)?, store_at + size))
    };
    let (_, _, end) = header(96)?;
    let (entries, store, _) = header(end.div_ceil(8) * 8)?;
    let string = |tag: u32| {
        let (_, _, offset) = entries.iter().find(|(t, kind, _)| *t == tag && *kind == 6)?;
        let text = store.get(*offset..)?.split(|&c| c == 0).next()?;
        Some(String::from_utf8_lossy(text).into_owned())
    };
    let epoch = entries.iter().find(|(t, kind, _)| *t == 1003 && *kind == 4).and_then(|(_, _, offset)| {
        let b = store.get(*offset..offset + 4)?;
        Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    });
    let epoch = epoch.filter(|e| *e != 0).map(|e| format!("{e}:")).unwrap_or_default();
    Some(format!("{epoch}{}-{}", string(1001)?, string(1002)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use Ordering::{Equal, Greater, Less};

    fn entries(value: Value) -> Vec<Entry> {
        parse(&value).unwrap()
    }

    fn check(scheme: Scheme, cases: &[(&str, &str, Ordering)]) {
        for (a, b, want) in cases {
            assert_eq!(compare(scheme, a, b), *want, "{scheme:?}: {a} vs {b}");
            assert_eq!(compare(scheme, b, a), want.reverse(), "{scheme:?}: {b} vs {a}");
        }
    }

    #[test]
    fn entries_parse_as_fixed_versions_ranges_and_regexes() {
        let parsed = entries(json!([">=128.0.3, <129", "115.15.0esr", {"regex": "^115\\.(1[6-9]|[2-9][0-9])\\."}, "== 2.0", "!=1.5"]));
        assert_eq!(
            parsed[0],
            Entry::Range { all: vec![Comparator { op: Op::Ge, version: "128.0.3".into() }, Comparator { op: Op::Lt, version: "129".into() }] }
        );
        assert_eq!(parsed[1], Entry::Range { all: vec![Comparator { op: Op::Eq, version: "115.15.0esr".into() }] });
        assert!(matches!(&parsed[2], Entry::Regex { pattern } if pattern.starts_with("^115")));
        assert_eq!(parsed[3], Entry::Range { all: vec![Comparator { op: Op::Eq, version: "2.0".into() }] });
        assert_eq!(parsed[4], Entry::Range { all: vec![Comparator { op: Op::Ne, version: "1.5".into() }] });
        for bad in [json!([]), json!("1.0"), json!([""]), json!([">=1, 2"]), json!(["1, 2"]), json!([">="]), json!([{"regex": "("}]), json!([{"regex": 1}]), json!([{"version": "1"}]), json!([3])] {
            assert!(parse(&bad).is_err(), "{bad} should not parse");
        }
        let json = serde_json::to_value(&parsed[0]).unwrap();
        assert_eq!(json, json!({"kind": "range", "all": [{"op": ">=", "version": "128.0.3"}, {"op": "<", "version": "129"}]}));
    }

    #[test]
    fn dpkg_compares_like_dpkg() {
        check(Scheme::Dpkg, &[
            ("1.10", "1.9", Greater),
            ("1.0~rc1", "1.0", Less),
            ("1.0~~", "1.0~", Less),
            ("1.0", "1.0+b1", Less),
            ("1.0a", "1.0", Greater),
            ("1:1.0", "2.0", Greater),
            ("0:2.0", "2.0", Equal),
            ("1.0-1", "1.0-2", Less),
            ("2.30-1ubuntu1", "2.30-1ubuntu2", Less),
            ("128.0.3+build1-0ubuntu1", "128.0.3", Greater),
            ("115.16.0esr-1~deb12u1", "115.15.0esr", Greater),
            ("115.15.0esr-1~deb12u1", "115.15.0esr-1", Less),
            ("1.001", "1.1", Equal),
        ]);
        // A requirement without a revision ignores the installed one's.
        assert_eq!(compare(Scheme::Dpkg, "128.0.3-0ubuntu1", "128.0.3"), Equal);
    }

    #[test]
    fn rpm_compares_like_rpm() {
        check(Scheme::Rpm, &[
            ("1.10", "1.9", Greater),
            ("1.0", "1.0.1", Less),
            ("1.0.0", "1.0", Greater),
            ("1.0~rc1", "1.0", Less),
            ("1.0^git1", "1.0", Greater),
            ("1.0^git1", "1.0.1", Less),
            ("1.0a", "1.0", Greater),
            ("1.a", "1.1", Less),
            ("2:1.0-1", "1:9.0-1", Greater),
            ("1.0-1.fc40", "1.0-2.fc40", Less),
            ("1.0-10", "1.0-9", Greater),
            ("115.15.0-1.el9_4", "115.15.0", Equal),
        ]);
    }

    #[test]
    fn pacman_compares_like_vercmp() {
        check(Scheme::Pacman, &[
            ("1.10", "1.9", Greater),
            ("1.0a", "1.0", Less),
            ("1.0alpha", "1.0b", Less),
            ("1.0rc", "1.0", Less),
            ("1.0", "1.0.a", Less),
            ("1.0.a", "1.0.1", Less),
            ("1:1.0-1", "2.0-1", Greater),
            ("1.0-1", "1.0-2", Less),
            ("1.0", "1.0-5", Equal),
            ("128.0.3-1", "128.0.3", Equal),
        ]);
    }

    #[test]
    fn winget_nix_and_homebrew_compare_by_their_rules() {
        check(Scheme::Winget, &[
            ("1.0", "1.0.0", Equal),
            ("1.10", "1.9", Greater),
            ("1.0-beta", "1.0", Less),
            ("128.0.3", "128.0.1", Greater),
            ("1.0.0.0", "1", Equal),
            ("1.0-Beta", "1.0-beta", Equal),
        ]);
        check(Scheme::Nix, &[
            ("1.10", "1.9", Greater),
            ("1.0pre1", "1.0", Less),
            ("2.3a", "2.3.1", Less),
            ("1.0", "1.0.0", Less),
        ]);
        check(Scheme::Brew, &[
            ("1.10", "1.9", Greater),
            ("115.15.0esr", "115.15.0", Greater),
            ("115.16.0esr", "115.15.0esr", Greater),
            ("1.0rc1", "1.0", Less),
            ("1.0beta2", "1.0rc1", Less),
            ("1.0", "1.0.0", Equal),
            ("1.0p1", "1.0", Greater),
            ("1.0.1", "1.0p1", Greater),
        ]);
    }

    #[test]
    fn a_version_is_acceptable_when_it_matches_an_entry() {
        let required = entries(json!([">=128.0.3, <129", "115.15.0esr", {"regex": "^115\\.(1[6-9]|[2-9][0-9])\\."}]));
        for (version, ok) in [
            ("128.0.3", true),
            ("128.0.10", true),
            ("128.0.2", false),
            ("129.0", false),
            ("129", false),
            ("115.15.0esr", true),
            ("115.16.1esr", true),
            ("115.14.0esr", false),
        ] {
            assert_eq!(matches(&required, Scheme::Winget, version), ok, "{version}");
        }
        // The package manager's own rules: an epoch makes a version newer, a release counts only
        // when the requirement has one.
        let range = entries(json!([">=2.0, <3"]));
        assert!(!matches(&range, Scheme::Rpm, "1:1.0-1"));
        assert!(matches(&range, Scheme::Rpm, "2.0-1.fc40"));
        assert!(!matches(&range, Scheme::Dpkg, "3.0-1"));
        assert!(matches(&entries(json!(["!=2.5"])), Scheme::Dpkg, "2.6"));
        assert!(!matches(&entries(json!(["!=2.5"])), Scheme::Dpkg, "2.5-1"));
    }

    #[test]
    fn contradictory_ranges_never_match() {
        for (range, never) in [(">=2, <1", true), (">=1, <2", false), (">=1, <1", true), (">=1, <=1", false), ("=1, !=1", true), (">1, <=1", true), (">=1, <=1, !=1", true), ("=1, =2", true), (">=1.10, <1.9", true)] {
            let entry = parse_text(range).unwrap();
            assert_eq!(never_matches(&entry, Scheme::Generic), never, "{range}");
        }
    }

    #[test]
    fn requirements_read_as_english() {
        let required = entries(json!([">=128.0.3, <129", "115.15.0esr", {"regex": "^115\\."}]));
        assert_eq!(describe(&required), "128.0.3 or later and earlier than 129, 115.15.0esr, or a version matching ^115\\.");
        assert_eq!(describe(&entries(json!([">=128.0.3"]))), "128.0.3 or later");
    }

    fn facts(installed: Option<&str>, latest: Option<&str>, available: Option<&[&str]>, via: &str) -> Facts {
        Facts {
            installed: installed.map(String::from),
            latest: latest.map(String::from),
            available: available.map(|a| a.iter().map(|v| v.to_string()).collect()),
            scheme: Some(Scheme::Winget),
            pinnable: matches!(via, "apt" | "dnf" | "zypper" | "winget"),
            installer: false,
            via: via.into(),
        }
    }

    #[test]
    fn the_newest_acceptable_version_offered_is_installed() {
        let required = entries(json!([">=128.0.3, <129"]));
        // The newest offered is acceptable: the package manager updates on its own.
        let (decision, shown) = assess("Firefox", &required, &facts(Some("127.0"), Some("128.0.5"), None, "pacman"));
        assert_eq!(decision, Decision::Free);
        let shown = shown.unwrap();
        assert!(!shown.acceptable);
        assert_eq!(shown.target.as_deref(), Some("128.0.5"));
        // Acceptable already, and nothing better: nothing to show.
        assert_eq!(assess("Firefox", &required, &facts(Some("128.0.4"), Some("128.0.5"), None, "pacman")), (Decision::Free, None));

        // The newest isn't acceptable: a package manager that installs a given version gets the
        // newest acceptable one it offers.
        let offered = ["129.0", "128.0.7", "128.0.6", "127.0"];
        let f = facts(Some("127.0"), Some("129.0"), None, "winget");
        assert!(f.need_list(&required));
        let f = facts(Some("127.0"), Some("129.0"), Some(&offered), "winget");
        assert!(!f.need_list(&required));
        let (decision, shown) = assess("Firefox", &required, &f);
        assert_eq!(decision, Decision::Exactly("128.0.7".into()));
        assert_eq!(shown.unwrap().target.as_deref(), Some("128.0.7"));
        // Installed and acceptable stays, also when only older acceptable versions are offered.
        let f = facts(Some("128.0.9"), Some("129.0"), Some(&offered), "winget");
        assert_eq!(assess("Firefox", &required, &f), (Decision::Exactly("128.0.9".into()), None));
        // One that can't (pacman) keeps an acceptable version where it is...
        let f = facts(Some("128.0.4"), Some("129.0"), None, "pacman");
        assert!(!f.need_list(&required));
        assert_eq!(assess("Firefox", &required, &f), (Decision::Exactly("128.0.4".into()), None));
        // ...and has nothing to offer for one that isn't acceptable.
        let (decision, shown) = assess("Firefox", &required, &facts(Some("127.0"), Some("128.0.1"), None, "pacman"));
        assert_eq!(decision, Decision::NoneOffered);
        let shown = shown.unwrap();
        assert_eq!((shown.target, shown.acceptable), (None, false));
        assert_eq!(shown.summary, "128.0.3 or later and earlier than 129; pacman offers 128.0.1");
        // Not installed and nothing acceptable offered, even by version.
        let (decision, shown) = assess("Firefox", &required, &facts(None, Some("129.0"), Some(&["129.0", "128.0.1"]), "apt"));
        assert_eq!(decision, Decision::NoneOffered);
        assert!(shown.unwrap().acceptable);
        // Too new: only a package manager that installs a given version can go back.
        let f = facts(Some("129.1"), Some("128.0.5"), None, "apt");
        assert!(f.need_list(&required));
        let f = facts(Some("129.1"), Some("128.0.5"), Some(&["128.0.5", "128.0.4"]), "apt");
        assert_eq!(assess("Firefox", &required, &f).0, Decision::Exactly("128.0.5".into()));
        assert_eq!(assess("Firefox", &required, &facts(Some("129.1"), Some("128.0.5"), None, "pacman")).0, Decision::NoneOffered);
    }

    #[test]
    fn unknown_versions_leave_things_as_they_are() {
        let required = entries(json!([">=2"]));
        // Installed, version unknown: can't judge it.
        assert_eq!(assess("x", &required, &facts(Some("?"), Some("1.0"), None, "pacman")), (Decision::Free, None));
        // Nothing known about what is offered (offline): leave the install to the package manager.
        assert_eq!(assess("x", &required, &facts(None, None, None, "pacman")), (Decision::Free, None));
        assert_eq!(assess("x", &required, &facts(Some("2.1"), None, None, "pacman")), (Decision::Free, None));
        let (decision, shown) = assess("x", &required, &facts(Some("1.0"), None, None, "pacman"));
        assert_eq!(decision, Decision::NoneOffered);
        assert_eq!(shown.unwrap().summary, "2 or later; pacman offers no version");
    }

    #[test]
    fn installers_offer_the_collections_file() {
        let required = entries(json!([">=2"]));
        let mut f = facts(Some("1.0"), Some("1.5"), None, "apt");
        (f.installer, f.pinnable, f.scheme) = (true, false, Some(Scheme::Dpkg));
        let (decision, shown) = assess("VPN", &required, &f);
        assert_eq!(decision, Decision::NoneOffered);
        assert_eq!(shown.unwrap().summary, "2 or later; the collection's file has 1.5");
        assert!(Facts::from_state(crate::catalog::INSTALLER_TYPE, &json!({"_exist": true, "version": "1.0", "via": "apt"})).installer);
        assert!(!pinnable(crate::catalog::INSTALLER_TYPE, "apt"));
    }

    #[test]
    fn facts_come_from_the_resources_state() {
        let state = json!({"name": "firefox", "_exist": true, "version": "127.0", "latestVersion": "128.0.5", "versionScheme": "dpkg", "via": "apt", "availableVersions": ["128.0.5", "127.0"]});
        let f = Facts::from_state(crate::catalog::PACKAGE_TYPE, &state);
        assert_eq!((f.installed.as_deref(), f.latest.as_deref(), f.scheme, f.pinnable), (Some("127.0"), Some("128.0.5"), Some(Scheme::Dpkg), true));
        assert_eq!(f.available.unwrap().len(), 2);
        let f = Facts::from_state(crate::catalog::PACKAGE_TYPE, &json!({"_exist": false}));
        assert_eq!(f.installed, None);
        let f = Facts::from_state(crate::catalog::PACKAGE_TYPE, &json!({"_exist": true}));
        assert_eq!(f.installed.as_deref(), Some("?"));
    }

    #[test]
    fn documents_lose_versions_before_dsc_and_earlier_versions_get_todays() {
        let mut resources = json!([
            {"name": "a", "type": "Furnish/Package", "properties": {"name": "a", "versions": [">=1"]}},
            {"name": "g", "type": "Microsoft.DSC/Group", "properties": {"resources": [{"name": "b", "type": "Furnish/Installer", "properties": {"file": "x", "versions": ["1"]}}]}},
            {"name": "o", "type": "Other/Thing", "properties": {"versions": "theirs"}}
        ]);
        assert!(validate(&resources).is_err(), "versions inside a group are refused");
        strip(&mut resources);
        assert_eq!(resources[0]["properties"], json!({"name": "a"}));
        assert_eq!(resources[1]["properties"]["resources"][0]["properties"], json!({"file": "x"}));
        assert_eq!(resources[2]["properties"]["versions"], "theirs");
        assert!(validate(&json!([{"name": "n", "type": "Furnish/NixOS", "properties": {"versions": ["1"]}}])).is_err());
        assert!(validate(&json!([{"name": "p", "type": "Furnish/Package", "properties": {"versions": ["(", ">="]}}])).is_err());
        assert!(validate(&json!([{"name": "p", "type": "Furnish/Package", "properties": {"versions": [">=1"]}}])).is_ok());

        let current = json!({"resources": [{"name": "a", "type": "Furnish/Package", "properties": {"name": "a", "versions": [">=2"]}}]});
        let mut earlier = json!({"resources": [{"name": "a", "type": "Furnish/Package", "properties": {"name": "a"}}, {"name": "b", "type": "Furnish/Package", "properties": {"name": "b", "versions": ["1"]}}]});
        carry(&current, &mut earlier);
        assert_eq!(earlier["resources"][0]["properties"]["versions"], json!([">=2"]));
        assert_eq!(earlier["resources"][1]["properties"]["versions"], json!(["1"]));
    }

    /// Compares with the real tools' results: lines "a b -1|0|1" in $FURNISH_VERCMP_DIR/{pacman,dpkg,rpm}.txt
    /// (from `vercmp`, `dpkg --compare-versions` and rpm's `rpm.vercmp`). Pairs where only one
    /// side has a release are left out: furnish ignores the release then (see `evr`).
    #[test]
    #[ignore]
    fn same_results_as_the_package_managers() {
        let dir = std::env::var("FURNISH_VERCMP_DIR").expect("FURNISH_VERCMP_DIR");
        let mut wrong = vec![];
        for (file, scheme) in [("pacman", Scheme::Pacman), ("dpkg", Scheme::Dpkg), ("rpm", Scheme::Rpm)] {
            for line in std::fs::read_to_string(format!("{dir}/{file}.txt")).unwrap().lines() {
                let [a, b, r] = line.split(' ').collect::<Vec<_>>()[..] else { continue };
                let release = |v: &str| v.split_once(':').map_or(v, |(_, rest)| rest).contains('-');
                if release(a) != release(b) {
                    continue;
                }
                let want = match r { "-1" => Less, "0" => Equal, _ => Greater };
                if compare(scheme, a, b) != want {
                    wrong.push(format!("{file}: {a} vs {b}: want {want:?}"));
                }
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// An .rpm with only what `rpm_header_version` reads.
    fn rpm_bytes(epoch: Option<u32>) -> Vec<u8> {
        let header = |tags: &[(u32, u32, Vec<u8>)]| {
            let mut store: Vec<u8> = Vec::new();
            let mut index = Vec::new();
            for (tag, kind, data) in tags {
                for v in [*tag, *kind, store.len() as u32, 1] {
                    index.extend(v.to_be_bytes());
                }
                store.extend(data);
            }
            let mut out = vec![0x8e, 0xad, 0xe8, 0x01, 0, 0, 0, 0];
            out.extend((tags.len() as u32).to_be_bytes());
            out.extend((store.len() as u32).to_be_bytes());
            out.extend(index);
            out.extend(store);
            out
        };
        let mut bytes = vec![0xed, 0xab, 0xee, 0xdb];
        bytes.resize(96, 0);
        bytes.extend(header(&[(62, 7, vec![1, 2, 3])])); // a signature of 3 bytes: padding follows
        while bytes.len() % 8 != 0 {
            bytes.push(0);
        }
        let mut tags = vec![(1000, 6, b"vpn\0".to_vec()), (1001, 6, b"2.4.1\0".to_vec()), (1002, 6, b"3.el9\0".to_vec())];
        if let Some(e) = epoch {
            tags.push((1003, 4, e.to_be_bytes().to_vec()));
        }
        bytes.extend(header(&tags));
        bytes
    }

    #[test]
    fn rpm_files_tell_their_version() {
        assert_eq!(rpm_header_version(&rpm_bytes(None)).as_deref(), Some("2.4.1-3.el9"));
        assert_eq!(rpm_header_version(&rpm_bytes(Some(2))).as_deref(), Some("2:2.4.1-3.el9"));
        assert_eq!(rpm_header_version(&rpm_bytes(Some(0))).as_deref(), Some("2.4.1-3.el9"));
        assert_eq!(rpm_header_version(b"not an rpm"), None);
        let truncated = rpm_bytes(None);
        assert_eq!(rpm_header_version(&truncated[..truncated.len() - 4]), None);
    }

    #[test]
    fn check_reports_installers_their_versions_refuse_and_ranges_that_never_match() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("vpn.rpm");
        std::fs::write(&file, rpm_bytes(None)).unwrap();
        let file = file.to_string_lossy().into_owned();
        let installer = crate::catalog::INSTALLER_TYPE;
        let findings = super::check("VPN", installer, &json!({"file": file, "versions": [">=2.5"]}));
        assert_eq!(findings, vec![(true, "VPN: vpn.rpm has version 2.4.1-3.el9, which its versions don't accept (needs 2.5 or later)".to_string())]);
        assert!(super::check("VPN", installer, &json!({"file": file, "versions": [">=2.4.1, <3"]})).is_empty());
        let findings = super::check("VPN", crate::catalog::PACKAGE_TYPE, &json!({"name": "vpn", "versions": [">=3, <2", "1.0"]}));
        assert_eq!(findings, vec![(false, "VPN: no version can be 3 or later and earlier than 2".to_string())]);
        let exe = dir.path().join("setup.exe");
        std::fs::write(&exe, b"MZ").unwrap();
        let findings = super::check("Setup", installer, &json!({"file": exe, "versions": ["1.0"]}));
        assert!(findings[0].1.contains("can't read the version of setup.exe"), "{findings:?}");
        assert!(super::check("Other", "Other/Type", &json!({"versions": ["(("]})).is_empty());
    }
}
