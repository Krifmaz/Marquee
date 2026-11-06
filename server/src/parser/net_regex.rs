//! Sonarr's and Radarr's parsers are built on .NET regular expressions. This module reads those expressions as
//! they are written upstream, so the ported parsers keep the upstream patterns character for character, and runs
//! them on `fancy-regex` (look-arounds, back-references).
//!
//! What .NET does that `fancy-regex` doesn't, and how it is bridged:
//!
//! - **The same group name more than once** (`\[v(?<version>\d)\]|repack(?<version>\d)`). Each repeat is renamed
//!   (`version__1`, …) and [`Caps::name`] answers the one that took part in the match, like .NET's `Groups[name]`.
//! - **`RegexOptions.IgnorePatternWhitespace`**: whitespace and `#` comments outside character classes are dropped
//!   before compiling.
//! - **A `[` inside a character class** (`[([]`) is a plain character in .NET; here it is escaped.
//! - **A look-behind whose alternatives differ in length** (`(?<!HD[._ -]|HD)`): `(?<!a|b)` becomes
//!   `(?<!a)(?<!b)` and `(?<=a|b)` becomes `(?:(?<=a)|(?<=b))`, which mean the same.
//! - **A look-behind of varying length** (`(?<!WEB[-_. ]?)`, `(?<!\d+)`): a last piece that may be left out
//!   (`x?`) gives two alternatives, with and without it, which are then split as above; a first piece that repeats
//!   (`x+`, `x*`) is read once or left out, since text that ends in `x+…` also ends in `x…`. What is still of
//!   varying length after that (`(?<!(19|20)\d{2}.*?)`) `fancy-regex` reads itself; it answers wrongly when such
//!   a look-behind has a `\b` or look-around after its start, so those are refused (a test compiles every
//!   upstream expression).
//!
//! .NET also keeps every repetition of a group (`Group.Captures`); `fancy-regex` keeps the last. The parsers that
//! need the repetitions read them again from the text the group covered.

use std::collections::HashMap;
use std::sync::Arc;

/// Options, as .NET's `RegexOptions` names them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Opts {
    /// `RegexOptions.IgnoreCase`
    pub ignore_case: bool,
    /// `RegexOptions.IgnorePatternWhitespace`
    pub ignore_whitespace: bool,
}

/// Ignore case (the usual upstream choice).
pub const I: Opts = Opts { ignore_case: true, ignore_whitespace: false };
/// Ignore case and pattern whitespace.
pub const IX: Opts = Opts { ignore_case: true, ignore_whitespace: true };
/// Neither: case matters.
pub const NONE: Opts = Opts { ignore_case: false, ignore_whitespace: false };

/// A .NET expression, compiled.
#[derive(Clone, Debug)]
pub struct NetRegex {
    /// the compiled expression
    re: fancy_regex::Regex,
    /// each upstream group name and the names its repeats were given here, in pattern order
    aliases: Arc<HashMap<String, Vec<String>>>,
}

/// One match of a [`NetRegex`] and its groups.
#[derive(Debug)]
pub struct Caps<'t> {
    /// the groups as `fancy-regex` answers them
    caps: fancy_regex::Captures<'t, str>,
    /// the upstream group names (see [`NetRegex`])
    aliases: Arc<HashMap<String, Vec<String>>>,
}

/// A matched piece of text and where it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece<'t> {
    /// the text matched
    pub text: &'t str,
    /// its first byte in the text searched
    pub start: usize,
    /// one past its last byte
    pub end: usize,
}

impl<'t> Piece<'t> {
    /// The text matched.
    pub fn as_str(&self) -> &'t str {
        self.text
    }
}

impl NetRegex {
    /// Compiles an upstream expression; panics when it can't be read, because the patterns are fixed in the source
    /// and a test compiles every one of them.
    pub fn new(pattern: &str, opts: Opts) -> NetRegex {
        NetRegex::try_new(pattern, opts).unwrap_or_else(|e| panic!("The upstream expression {pattern:?} can't be read: {e}"))
    }

    /// Compiles an upstream expression, or says why it can't.
    pub fn try_new(pattern: &str, opts: Opts) -> Result<NetRegex, String> {
        let mut p = if opts.ignore_whitespace { strip_whitespace(pattern) } else { pattern.to_string() };
        p = escape_class_brackets(&p);
        p = split_lookbehinds(&p);
        check_lookbehinds(&p)?;
        let (p, aliases) = rename_groups(&p);
        let full = if opts.ignore_case { format!("(?i){p}") } else { p };
        let re = fancy_regex::RegexBuilder::new(&full).backtrack_limit(10_000_000).build().map_err(|e| e.to_string())?;
        Ok(NetRegex { re, aliases: Arc::new(aliases) })
    }

    /// Whether the expression matches somewhere in `text`.
    pub fn is_match(&self, text: &str) -> bool {
        self.re.is_match(text).unwrap_or(false)
    }

    /// The first match in `text` and its groups.
    pub fn captures<'t>(&self, text: &'t str) -> Option<Caps<'t>> {
        self.re.captures(text).ok().flatten().map(|caps| Caps { caps, aliases: self.aliases.clone() })
    }

    /// Every match in `text`, left to right, like .NET's `Matches`.
    pub fn captures_iter<'t>(&self, text: &'t str) -> Vec<Caps<'t>> {
        self.re.captures_iter(text).filter_map(Result::ok).map(|caps| Caps { caps, aliases: self.aliases.clone() }).collect()
    }

    /// How many times the expression matches in `text`.
    pub fn count(&self, text: &str) -> usize {
        self.re.find_iter(text).filter(Result::is_ok).count()
    }

    /// `text` with every match replaced; `rep` uses .NET's `$1`/`${name}` syntax, which is also Rust's.
    pub fn replace_all(&self, text: &str, rep: &str) -> String {
        self.re.replace_all(text, rep).into_owned()
    }

    /// `text` with every match replaced by what `f` makes of it (.NET's `Replace` with a `MatchEvaluator`).
    pub fn replace_all_with(&self, text: &str, f: impl Fn(&Caps<'_>) -> String) -> String {
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for c in self.captures_iter(text) {
            let w = c.whole();
            out.push_str(&text[last..w.start]);
            out.push_str(&f(&c));
            last = w.end;
        }
        out.push_str(&text[last..]);
        out
    }

    /// The pieces of `text` between matches (.NET's `Regex.Split`, without captured groups).
    pub fn split<'t>(&self, text: &'t str) -> Vec<&'t str> {
        let mut out = Vec::new();
        let mut last = 0;
        for c in self.captures_iter(text) {
            let w = c.whole();
            out.push(&text[last..w.start]);
            last = w.end;
        }
        out.push(&text[last..]);
        out
    }

    /// `text` with the first match replaced.
    pub fn replace(&self, text: &str, rep: &str) -> String {
        self.re.replace(text, rep).into_owned()
    }
}

impl<'t> Caps<'t> {
    /// The whole match.
    pub fn whole(&self) -> Piece<'t> {
        let m = self.caps.get(0).expect("group 0 is the whole match");
        Piece { text: m.as_str(), start: m.start(), end: m.end() }
    }

    /// The upstream group `name`: the one of its repeats that took part, the last when several did.
    pub fn name(&self, name: &str) -> Option<Piece<'t>> {
        let names = self.aliases.get(name)?;
        names.iter().rev().find_map(|n| self.caps.name(n)).map(|m| Piece { text: m.as_str(), start: m.start(), end: m.end() })
    }

    /// Whether the upstream group `name` took part (.NET's `Groups[name].Success`).
    pub fn ok(&self, name: &str) -> bool {
        self.name(name).is_some()
    }

    /// The upstream group `name`'s text, "" when it didn't take part (.NET's `Groups[name].Value`).
    pub fn str(&self, name: &str) -> &'t str {
        self.name(name).map_or("", |p| p.text)
    }

    /// The numbered group `i`.
    pub fn index(&self, i: usize) -> Option<Piece<'t>> {
        self.caps.get(i).map(|m| Piece { text: m.as_str(), start: m.start(), end: m.end() })
    }
}

/// The pattern without the whitespace and `#` comments that `IgnorePatternWhitespace` ignores.
fn strip_whitespace(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    let mut in_class = false;
    let mut chars = p.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                out.push(c);
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            '[' if !in_class => {
                in_class = true;
                out.push(c);
                if chars.peek() == Some(&'^') {
                    out.push(chars.next().unwrap_or('^'));
                }
                if chars.peek() == Some(&']') {
                    out.push(chars.next().unwrap_or(']'));
                }
            }
            ']' if in_class => {
                in_class = false;
                out.push(c);
            }
            '#' if !in_class => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        break;
                    }
                }
            }
            c if !in_class && c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    out
}

/// Escapes `[` inside character classes (a plain `[` in .NET, the start of a nested class here).
fn escape_class_brackets(p: &str) -> String {
    let mut out = String::with_capacity(p.len() + 4);
    let mut in_class = false;
    let mut chars = p.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                out.push(c);
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            '[' if in_class => out.push_str("\\["),
            '[' => {
                in_class = true;
                out.push(c);
                if chars.peek() == Some(&'^') {
                    out.push(chars.next().unwrap_or('^'));
                }
                if chars.peek() == Some(&']') {
                    chars.next();
                    out.push_str("\\]");
                }
            }
            ']' if in_class => {
                in_class = false;
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// The byte index of the `)` that closes the group whose body starts at `body_start`, if any.
fn closing_paren(p: &str, body_start: usize) -> Option<usize> {
    let (mut depth, mut in_class) = (1i32, false);
    let mut it = p[body_start..].char_indices();
    while let Some((i, c)) = it.next() {
        match c {
            '\\' => {
                it.next();
            }
            '[' if !in_class => in_class = true,
            ']' if in_class => in_class = false,
            '(' if !in_class => depth += 1,
            ')' if !in_class => {
                depth -= 1;
                if depth == 0 {
                    return Some(body_start + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Splits `p` at its top-level `|`s.
fn alternatives(p: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let (mut depth, mut in_class) = (0i32, false);
    let mut chars = p.chars();
    while let Some(c) = chars.next() {
        let cur = out.last_mut().expect("one at least");
        match c {
            '\\' => {
                cur.push(c);
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
                continue;
            }
            '[' if !in_class => in_class = true,
            ']' if in_class => in_class = false,
            '(' if !in_class => depth += 1,
            ')' if !in_class => depth -= 1,
            '|' if !in_class && depth == 0 => {
                out.push(String::new());
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    out
}

/// One piece of a pattern and its quantifier (`?`, `+`, `*`, `{n,m}`, maybe lazy), at the top level.
fn pieces(p: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut i = 0;
    while i < p.len() {
        let c = p[i..].chars().next().expect("in bounds");
        let end = match c {
            '\\' => i + 1 + p[i + 1..].chars().next().map_or(0, char::len_utf8),
            '[' => {
                // a class: up to its unescaped `]` (a `]` right after `[` or `[^` is a plain character)
                let mut j = i + 1;
                if p[j..].starts_with('^') {
                    j += 1;
                }
                if p[j..].starts_with(']') {
                    j += 1;
                }
                while j < p.len() && !p[j..].starts_with(']') {
                    j += if p[j..].starts_with('\\') {
                        1 + p[j + 1..].chars().next().map_or(0, char::len_utf8)
                    } else {
                        p[j..].chars().next().map_or(1, char::len_utf8)
                    };
                }
                (j + 1).min(p.len())
            }
            '(' => closing_paren(p, i + 1).map_or(p.len(), |c| c + 1),
            _ => i + c.len_utf8(),
        };
        let mut q = end;
        if let Some(n) = p[q..].chars().next() {
            if matches!(n, '?' | '+' | '*') {
                q += 1;
            } else if n == '{' {
                if let Some(close) = p[q..].find('}') {
                    q += close + 1;
                }
            }
            if q > end && p[q..].starts_with('?') {
                q += 1;
            }
        }
        out.push((p[i..end].to_string(), p[end..q].to_string()));
        i = q;
    }
    out
}

/// The fixed-length alternatives a look-behind alternative of varying length stands for (see the module notes).
fn fixed_lengths(alt: &str) -> Vec<String> {
    let mut ps = pieces(alt);
    match ps.first().map(|(_, q)| q.as_str()) {
        Some("+") => ps[0].1.clear(),
        Some("*") => {
            ps.remove(0);
        }
        _ => {}
    }
    let join = |ps: &[(String, String)]| ps.iter().map(|(a, q)| format!("{a}{q}")).collect::<String>();
    if ps.len() > 1 && ps.last().is_some_and(|(_, q)| q == "?") {
        let mut with = ps.clone();
        with.last_mut().expect("not empty").1.clear();
        ps.pop();
        return vec![join(&with), join(&ps)];
    }
    vec![join(&ps)]
}

/// Refuses a look-behind of varying length with an assertion after its start (see the module notes).
fn check_lookbehinds(p: &str) -> Result<(), String> {
    let mut i = 0;
    let mut in_class = false;
    while i < p.len() {
        let c = p[i..].chars().next().expect("in bounds");
        if c == '\\' {
            i += 1 + p[i + 1..].chars().next().map_or(0, char::len_utf8);
            continue;
        }
        if in_class {
            in_class = c != ']';
        } else if c == '[' {
            in_class = true;
        } else if p[i..].starts_with("(?<=") || p[i..].starts_with("(?<!") {
            if let Some(close) = closing_paren(p, i + 4) {
                let ps = pieces(&p[i + 4..close]);
                let varies = ps.iter().any(|(a, q)| {
                    (!q.is_empty() && !(q.starts_with('{') && !q.contains(',')))
                        || alternatives(a.trim_start_matches(['(', '?', ':'])).len() > 1
                });
                let assertion = |a: &str| ["\\b", "\\B", "(?=", "(?!", "(?<=", "(?<!", "^", "$"].iter().any(|x| a.starts_with(x));
                if varies && ps.iter().skip(1).any(|(a, _)| assertion(a)) {
                    return Err(format!("the look-behind {} changes length and has a check inside it", &p[i..=close]));
                }
            }
        }
        i += c.len_utf8();
    }
    Ok(())
}

/// Rewrites look-behinds with several alternatives into one look-behind per alternative (see the module notes).
fn split_lookbehinds(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    let mut i = 0;
    let mut in_class = false;
    let bytes = p.as_bytes();
    while i < p.len() {
        let c = p[i..].chars().next().expect("in bounds");
        if c == '\\' {
            let n = p[i + 1..].chars().next().map_or(0, char::len_utf8);
            out.push_str(&p[i..i + 1 + n]);
            i += 1 + n;
            continue;
        }
        if in_class {
            in_class = c != ']';
        } else if c == '[' {
            in_class = true;
        } else if c == '(' && (p[i..].starts_with("(?<=") || p[i..].starts_with("(?<!")) {
            if let Some(close) = closing_paren(p, i + 4) {
                let positive = bytes[i + 3] == b'=';
                let body = split_lookbehinds(&p[i + 4..close]);
                let alts: Vec<String> = alternatives(&body).iter().flat_map(|a| fixed_lengths(a)).collect();
                if alts.len() > 1 {
                    if positive {
                        out.push_str("(?:");
                        out.push_str(&alts.iter().map(|a| format!("(?<={a})")).collect::<Vec<_>>().join("|"));
                        out.push(')');
                    } else {
                        for a in &alts {
                            out.push_str(&format!("(?<!{a})"));
                        }
                    }
                } else {
                    out.push_str(&p[i..i + 4]);
                    out.push_str(&alts.concat());
                    out.push(')');
                }
                i = close + 1;
                continue;
            }
        }
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// Gives every repeat of a group name its own name, and lists them per upstream name.
fn rename_groups(p: &str) -> (String, HashMap<String, Vec<String>>) {
    let mut out = String::with_capacity(p.len() + 16);
    let mut names: HashMap<String, Vec<String>> = HashMap::new();
    let mut in_class = false;
    let mut i = 0;
    while i < p.len() {
        let c = p[i..].chars().next().expect("in bounds");
        if c == '\\' {
            let n = p[i + 1..].chars().next().map_or(0, char::len_utf8);
            out.push_str(&p[i..i + 1 + n]);
            i += 1 + n;
            continue;
        }
        if in_class {
            in_class = c != ']';
        } else if c == '[' {
            in_class = true;
        } else if c == '(' && p[i..].starts_with("(?<") && !p[i..].starts_with("(?<=") && !p[i..].starts_with("(?<!") {
            if let Some(end) = p[i + 3..].find('>') {
                let name = &p[i + 3..i + 3 + end];
                let list = names.entry(name.to_string()).or_default();
                let new = if list.is_empty() { name.to_string() } else { format!("{name}__{}", list.len()) };
                list.push(new.clone());
                out.push_str("(?P<");
                out.push_str(&new);
                out.push('>');
                i += 3 + end + 1;
                continue;
            }
        }
        out.push(c);
        i += c.len_utf8();
    }
    (out, names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_group_names_answer_the_one_that_matched() {
        let r = NetRegex::new(r"\d[-._ ]?v(?<version>\d)[-._ ]|\[v(?<version>\d)\]|repack(?<version>\d)", I);
        assert_eq!(r.captures("Show [v3] 1080p").unwrap().str("version"), "3");
        assert_eq!(r.captures("Show REPACK2 1080p").unwrap().str("version"), "2");
        assert!(!r.captures("Show REPACK2").unwrap().ok("missing"));
    }

    #[test]
    fn pattern_whitespace_is_ignored_outside_classes() {
        let r = NetRegex::new("\\b(?:\n   (?<a>foo)|  # a comment\n   (?<b>bar[ ]baz)\n)\\b", IX);
        assert!(r.captures("x bar baz y").unwrap().ok("b"));
        assert!(r.captures("FOO").unwrap().ok("a"));
    }

    #[test]
    fn look_behinds_of_different_lengths_work() {
        let r = NetRegex::new(r"(?<!HD[._ -]|HD)DVD", I);
        assert!(r.is_match("Movie DVD"));
        assert!(!r.is_match("Movie HD-DVD"));
        assert!(!r.is_match("Movie HDDVD"));
        let r = NetRegex::new(r"(?<=x|yy)z", NONE);
        assert!(r.is_match("xz") && r.is_match("yyz") && !r.is_match("yz"));
    }

    #[test]
    fn look_behinds_of_varying_length_work() {
        let r = NetRegex::new(r"(?<!WEB[-_. ]?)\bDL\b", I);
        assert!(r.is_match("German DL 1080p"));
        assert!(!r.is_match("German WEB-DL 1080p"));
        assert!(!r.is_match("German WEBDL") && !r.is_match("WEB DL"));
        let r = NetRegex::new(r"(?<!\d+)\d{1,3}(?!\d+)$", NONE);
        assert!(r.is_match("Title - 2") && r.is_match("a12"));
        assert!(!r.is_match("2012"));
        assert_eq!(fixed_lengths(r"[ab]c?"), vec!["[ab]c".to_string(), "[ab]".to_string()]);
        assert_eq!(fixed_lengths(r"\d*x"), vec!["x".to_string()]);
    }

    #[test]
    fn look_behinds_fancy_regex_gets_wrong_are_refused() {
        assert!(NetRegex::try_new(r"(?<!web\b.*)-EVO", I).is_err());
        assert!(NetRegex::try_new(r"(?<!\bweb.*)-EVO", I).is_ok());
        let r = NetRegex::new(r"(?<!(19|20)\d{2}.*?)German", I);
        assert!(!r.is_match("Movie.2020.German") && r.is_match("Movie.German.2020"));
    }

    #[test]
    fn case_and_inline_flags_follow_dotnet() {
        assert!(NetRegex::new(r"[. ](?-i:WEB)$", I).is_match("Show.WEB"));
        assert!(!NetRegex::new(r"[. ](?-i:WEB)$", I).is_match("Show.web"));
        assert!(!NetRegex::new(r"\bREAL\b", NONE).is_match("real"));
        assert!(NetRegex::new(r"[([]dual[])]", I).is_match("Show [DUAL]"));
    }
}
