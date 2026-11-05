//! TRaSH's regular expressions are written for .NET (Radarr and Sonarr). Most work as they are with
//! `fancy-regex` (look-arounds, back-references); two .NET habits need help:
//!
//! - a `[` inside a character class (`[([]`) is a plain character in .NET; here it is escaped;
//! - a look-behind of varying length (`(?<!\bweb[ ._-]?(dl|rip)?\b.*)`). TRaSH puts these only at the start of a
//!   top-level alternative, so such an alternative is split: the rest is matched, and at each place it matches
//!   the look-behind is checked by matching it, anchored at the end, against the text before that place.
//!
//! Matching ignores case, like Radarr and Sonarr.

/// A .NET-style expression, ready to match.
#[derive(Clone, Debug)]
pub struct Regex {
    /// the alternatives: whole when `fancy-regex` reads the expression as it is, else split as above
    branches: Vec<Branch>,
}

/// One top-level alternative.
#[derive(Clone, Debug)]
struct Branch {
    /// look-behinds before the rest: (the look-behind anchored at the end, true for `(?<=`, false for `(?<!`)
    behind: Vec<(fancy_regex::Regex, bool)>,
    /// the rest of the alternative
    rest: fancy_regex::Regex,
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
                // `]` or `^]` right after the opening is a plain `]`
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

/// The leading `(?<=…)`/`(?<!…)` groups of an alternative and the rest: ([(inside, positive)], rest).
fn leading_lookbehinds(branch: &str) -> (Vec<(String, bool)>, String) {
    let mut rest = branch;
    let mut found = Vec::new();
    while let Some(positive) = if rest.starts_with("(?<=") {
        Some(true)
    } else if rest.starts_with("(?<!") {
        Some(false)
    } else {
        None
    } {
        let body = &rest[4..];
        let (mut depth, mut in_class, mut end) = (1i32, false, None);
        let mut it = body.char_indices();
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
                        end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else { break };
        found.push((body[..end].to_string(), positive));
        rest = &body[end + 1..];
    }
    (found, rest.to_string())
}

/// `p` compiled ignoring case.
fn compile(p: &str) -> Option<fancy_regex::Regex> {
    fancy_regex::Regex::new(&format!("(?i){p}")).ok()
}

impl Regex {
    /// Reads a .NET-style expression; `None` when it can't be read even split.
    pub fn new(pattern: &str) -> Option<Regex> {
        let p = escape_class_brackets(pattern);
        if let Some(whole) = compile(&p) {
            return Some(Regex { branches: vec![Branch { behind: vec![], rest: whole }] });
        }
        let mut branches = Vec::new();
        for alt in alternatives(&p) {
            let (behind, rest) = leading_lookbehinds(&alt);
            let behind = behind.iter().map(|(b, pos)| compile(&format!("(?:{b})$")).map(|r| (r, *pos))).collect::<Option<Vec<_>>>()?;
            branches.push(Branch { behind, rest: compile(&rest)? });
        }
        Some(Regex { branches })
    }

    /// Whether the expression matches somewhere in `text`.
    pub fn is_match(&self, text: &str) -> bool {
        self.branches.iter().any(|b| b.is_match(text))
    }
}

impl Branch {
    /// Whether this alternative matches somewhere in `text`.
    fn is_match(&self, text: &str) -> bool {
        if self.behind.is_empty() {
            return self.rest.is_match(text).unwrap_or(false);
        }
        let mut pos = 0;
        while pos <= text.len() {
            let Ok(Some(m)) = self.rest.find_from_pos(text, pos) else { return false };
            let before = &text[..m.start()];
            if self.behind.iter().all(|(r, positive)| r.is_match(before).unwrap_or(false) == *positive) {
                return true;
            }
            // try the next place the rest starts
            pos = m.start() + text[m.start()..].chars().next().map_or(1, char::len_utf8);
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::Regex;

    #[test]
    fn brackets_in_classes_are_plain() {
        let r = Regex::new(r"[([]dual[])]").unwrap();
        assert!(r.is_match("Show (DUAL) 1080p"));
        assert!(r.is_match("Show [Dual] 1080p"));
        assert!(!r.is_match("Show dual 1080p"));
    }

    #[test]
    fn varying_look_behinds_work_like_dotnet() {
        let lq = Regex::new(r"(?<!\bweb[ ._-]?(dl|rip)?\b.*)-(EVO)\b").unwrap();
        assert!(lq.is_match("Movie.2020.1080p.BluRay.x264-EVO"));
        assert!(!lq.is_match("Movie.2020.1080p.WEB-DL.x264-EVO"), "EVO's WEB releases are fine");
        let stereo = Regex::new(r"(?<=\b[12]\d{3}\b.*?)(?<!repac)[^0-9]2[ .]0\b|\bStereo\b").unwrap();
        assert!(stereo.is_match("Movie.2020.1080p.WEB-DL.AAC2.0.x264-GRP"));
        assert!(!stereo.is_match("Movie 2.0 Returns"), "no year before it");
        assert!(stereo.is_match("Old Movie Stereo"));
        let dl = Regex::new(r"(?<!WEB[-_. ]?)\b(DL)\b").unwrap();
        assert!(dl.is_match("Film.2020.German.DL.1080p.BluRay"));
        assert!(!dl.is_match("Film.2020.German.1080p.WEB-DL"));
        let ai = Regex::new(r"(?<=\b\d{3,4}p\b)(?=.*\b(HEVC)\b)(?=.*\b(AI)\b)").unwrap();
        assert!(ai.is_match("Show.S01.2160p.AI.HEVC-GRP"));
        assert!(!ai.is_match("Show.S01.AI.HEVC-GRP"));
    }

    #[test]
    fn case_is_ignored() {
        assert!(Regex::new(r"\bremux\b").unwrap().is_match("Movie.REMUX.2160p"));
    }
}
