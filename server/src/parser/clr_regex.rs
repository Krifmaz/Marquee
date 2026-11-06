//! A small backtracking regular-expression engine that matches the way .NET's `System.Text.RegularExpressions`
//! does, including the one thing no Rust engine offers: every capture a group made (`Group.Captures`). Sonarr's
//! title parser reads those (all the episode numbers of `S01E01E02E03`, each title of `A / B / S01E01`), so its
//! port runs on this engine; the other ports use [`super::net_regex`].
//!
//! What it knows is what the upstream expressions use: literals, `.`, classes (`[^]a-z\d]`, `\d \w \s` and their
//! negations, `\uXXXX`), `^ $ \b \B \A \z \Z`, groups (numbered, named, `(?:)`, atomic `(?>)`), look-ahead and
//! look-behind of any length, back-references (`\k<name>`, `\1`), greedy and lazy quantifiers, alternation, and
//! the ignore-case option. Groups that share a name share their captures, as in .NET. Positions are counted in
//! characters while matching and handed out as byte positions.

use std::collections::HashMap;
use std::sync::LazyLock;

/// How many steps one search may take before it gives up and finds nothing, so a name that would make the
/// search run away cannot hang the server. No upstream case comes near it.
const STEP_LIMIT: u64 = 20_000_000;

/// One piece of a compiled expression.
#[derive(Debug)]
enum Node {
    /// matches nothing, always succeeds
    Empty,
    /// one character
    Char(char),
    /// one character from a class
    Set(Class),
    /// any character but a line feed (`.`)
    Any,
    /// the start of the text (`^`, `\A`)
    Start,
    /// the end of the text, or just before a line feed that ends it (`$`, `\Z`)
    End,
    /// the very end of the text (`\z`)
    EndAbs,
    /// a word boundary (`\b`), or with `true` a place that is not one (`\B`)
    Boundary(bool),
    /// pieces one after another
    Concat(Vec<Node>),
    /// alternatives, tried in order
    Alt(Vec<Node>),
    /// a capturing group: its slot and contents
    Group(usize, Box<Node>),
    /// a look-around
    Look(Look),
    /// an atomic group: once matched, never tried again another way
    Atomic(Box<Node>),
    /// a repeated piece
    Repeat(Repeat),
    /// a back-reference to a group's last capture
    Backref(usize),
}

/// A look-ahead or look-behind.
#[derive(Debug)]
struct Look {
    /// looks behind (`(?<=`, `(?<!`) rather than ahead
    behind: bool,
    /// succeeds when the contents do not match
    negative: bool,
    /// the shortest text the contents can match, in characters
    min: usize,
    /// the longest, or `None` without a limit
    max: Option<usize>,
    /// what is looked for
    node: Box<Node>,
}

/// A quantifier and what it repeats.
#[derive(Debug)]
struct Repeat {
    /// what is repeated
    node: Box<Node>,
    /// the fewest times
    min: usize,
    /// the most times (`usize::MAX` without a limit)
    max: usize,
    /// takes as many as it can first (`false` for a lazy `*?`, `+?`, `??`)
    greedy: bool,
}

/// A character class.
#[derive(Debug)]
struct Class {
    /// matches the characters not listed (`[^...]`)
    negated: bool,
    /// what is listed
    items: Vec<Item>,
}

/// One entry of a character class.
#[derive(Debug)]
enum Item {
    /// a range of characters, a single one when both ends are the same
    Range(char, char),
    /// `\d`, or `\D` with `true`
    Digit(bool),
    /// `\w`, or `\W` with `true`
    Word(bool),
    /// `\s`, or `\S` with `true`
    Space(bool),
}

/// A compiled expression.
#[derive(Debug)]
pub struct ClrRegex {
    /// the expression as written, for messages
    pattern: String,
    /// the compiled form
    root: Node,
    /// how many capture slots there are (slot 0 is the whole match)
    slots: usize,
    /// the slot of each named group
    names: HashMap<String, usize>,
    /// the slots of the numbered groups, in order (.NET numbers them before the named ones)
    numbered: Vec<usize>,
    /// compares letters without regard to case
    ignore_case: bool,
    /// the expression starts with `^`, so only the start of the text needs trying
    anchored: bool,
}

/// One place a group captured, as byte positions in the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span<'t> {
    /// the captured text
    pub text: &'t str,
    /// where it starts
    pub start: usize,
    /// where it ends
    pub end: usize,
}

/// One match: the captures of every group.
#[derive(Debug, Clone)]
pub struct ClrMatch<'t> {
    /// the text searched
    text: &'t str,
    /// each group's captures in the order they were made, as byte ranges
    caps: Vec<Vec<(usize, usize)>>,
    /// the slot of each named group
    names: &'t HashMap<String, usize>,
    /// the slots of the numbered groups, in order
    numbered: &'t [usize],
}

impl<'t> ClrMatch<'t> {
    /// The span of a range.
    fn span(&self, (start, end): (usize, usize)) -> Span<'t> {
        Span { text: &self.text[start..end], start, end }
    }

    /// The whole match.
    pub fn whole(&self) -> Span<'t> {
        self.span(self.caps[0][0])
    }

    /// The group's last capture (`Groups[name]` when it succeeded); `None` when it captured nothing.
    pub fn group(&self, name: &str) -> Option<Span<'t>> {
        let slot = *self.names.get(name)?;
        self.caps[slot].last().map(|&r| self.span(r))
    }

    /// The group's text (`Groups[name].Value`), empty when it captured nothing.
    pub fn str(&self, name: &str) -> &'t str {
        self.group(name).map_or("", |s| s.text)
    }

    /// Whether the group captured anything (`Groups[name].Success`).
    pub fn ok(&self, name: &str) -> bool {
        self.group(name).is_some()
    }

    /// Every capture the group made, in order (`Groups[name].Captures`).
    pub fn captures(&self, name: &str) -> Vec<Span<'t>> {
        match self.names.get(name) {
            Some(&slot) => self.caps[slot].iter().map(|&r| self.span(r)).collect(),
            None => Vec::new(),
        }
    }

    /// The text of numbered group `i`'s last capture, for `$1` in a replacement.
    fn numbered(&self, i: usize) -> Option<&'t str> {
        let slot = if i == 0 { 0 } else { *self.numbered.get(i - 1)? };
        self.caps[slot].last().map(|&r| self.span(r).text)
    }
}

/// The work of one search.
struct State {
    /// each slot's captures, as character ranges
    caps: Vec<Vec<(usize, usize)>>,
    /// steps taken so far
    steps: u64,
}

/// Lengths of every capture list, to undo captures made by a part that is given up.
type Snapshot = Vec<usize>;

impl State {
    /// Remembers how many captures each slot holds.
    fn snapshot(&self) -> Snapshot {
        self.caps.iter().map(Vec::len).collect()
    }

    /// Drops the captures made after `snap`.
    fn restore(&mut self, snap: &Snapshot) {
        for (c, &n) in self.caps.iter_mut().zip(snap) {
            c.truncate(n);
        }
    }
}

/// The continuation: what must match after a piece, given where the piece ended.
type Next<'a> = &'a mut dyn FnMut(usize, &mut State) -> bool;

/// Matches a compiled expression against one text.
struct Matcher<'a> {
    /// the expression
    re: &'a ClrRegex,
    /// the text, one entry per character
    t: &'a [char],
}

/// Whether a character is a .NET word character (`\w`).
fn is_word(c: char) -> bool {
    static WORD: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^\w$").expect("valid"));
    if c.is_ascii() {
        c.is_ascii_alphanumeric() || c == '_'
    } else {
        WORD.is_match(c.encode_utf8(&mut [0; 4]))
    }
}

/// Whether a character is a decimal digit (`\d`, any script).
fn is_digit(c: char) -> bool {
    static DIGIT: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^\d$").expect("valid"));
    if c.is_ascii() {
        c.is_ascii_digit()
    } else {
        DIGIT.is_match(c.encode_utf8(&mut [0; 4]))
    }
}

/// Whether a character is white space (`\s`).
fn is_space(c: char) -> bool {
    c.is_whitespace()
}

/// The character in lower case, for comparing without regard to case.
fn fold(c: char) -> char {
    let mut l = c.to_lowercase();
    match (l.next(), l.next()) {
        (Some(x), None) => x,
        _ => c,
    }
}

impl Matcher<'_> {
    /// Whether two characters are the same, ignoring case when asked to.
    fn same(&self, a: char, b: char) -> bool {
        a == b || (self.re.ignore_case && fold(a) == fold(b))
    }

    /// Whether a class matches a character, before case is considered.
    fn in_class_raw(class: &Class, c: char) -> bool {
        let hit = class.items.iter().any(|i| match *i {
            Item::Range(a, b) => a <= c && c <= b,
            Item::Digit(neg) => is_digit(c) != neg,
            Item::Word(neg) => is_word(c) != neg,
            Item::Space(neg) => is_space(c) != neg,
        });
        hit != class.negated
    }

    /// Whether a class matches a character.
    fn in_class(&self, class: &Class, c: char) -> bool {
        if Self::in_class_raw(class, c) {
            return true;
        }
        if !self.re.ignore_case {
            return false;
        }
        let lower = fold(c);
        let mut upper = c.to_uppercase();
        let upper = match (upper.next(), upper.next()) {
            (Some(x), None) => x,
            _ => c,
        };
        (lower != c && Self::in_class_raw(class, lower)) || (upper != c && Self::in_class_raw(class, upper))
    }

    /// Whether the single-character piece `n` matches the character at `pos`.
    fn one(&self, n: &Node, pos: usize) -> bool {
        let Some(&c) = self.t.get(pos) else { return false };
        match n {
            Node::Char(p) => self.same(c, *p),
            Node::Set(class) => self.in_class(class, c),
            Node::Any => c != '\n',
            _ => false,
        }
    }

    /// Whether `pos` is a word boundary.
    fn boundary(&self, pos: usize) -> bool {
        let before = pos > 0 && is_word(self.t[pos - 1]);
        let after = pos < self.t.len() && is_word(self.t[pos]);
        before != after
    }

    /// Matches `n` at `pos`, then `k` from where it ended; tries every way `n` can match until `k` succeeds.
    fn m(&self, n: &Node, pos: usize, st: &mut State, k: Next<'_>) -> bool {
        st.steps += 1;
        if st.steps > STEP_LIMIT {
            return false;
        }
        let len = self.t.len();
        match n {
            Node::Empty => k(pos, st),
            Node::Char(_) | Node::Set(_) | Node::Any => self.one(n, pos) && k(pos + 1, st),
            Node::Start => pos == 0 && k(pos, st),
            Node::End => (pos == len || (pos + 1 == len && self.t[pos] == '\n')) && k(pos, st),
            Node::EndAbs => pos == len && k(pos, st),
            Node::Boundary(negated) => self.boundary(pos) != *negated && k(pos, st),
            Node::Concat(nodes) => self.seq(nodes, pos, st, k),
            Node::Alt(alts) => {
                for a in alts {
                    if self.m(a, pos, st, k) {
                        return true;
                    }
                }
                false
            }
            Node::Group(slot, inner) => {
                let slot = *slot;
                self.m(inner, pos, st, &mut |p, st: &mut State| {
                    st.caps[slot].push((pos, p));
                    if k(p, st) {
                        true
                    } else {
                        st.caps[slot].pop();
                        false
                    }
                })
            }
            Node::Look(look) => {
                let snap = st.snapshot();
                let found = if look.behind { self.behind(look, pos, st) } else { self.m(&look.node, pos, st, &mut |_, _| true) };
                if look.negative {
                    st.restore(&snap);
                    !found && k(pos, st)
                } else if found && k(pos, st) {
                    true
                } else {
                    st.restore(&snap);
                    false
                }
            }
            Node::Atomic(inner) => {
                let snap = st.snapshot();
                let mut end = None;
                self.m(inner, pos, st, &mut |p, _| {
                    end = Some(p);
                    true
                });
                match end {
                    Some(p) if k(p, st) => true,
                    _ => {
                        st.restore(&snap);
                        false
                    }
                }
            }
            Node::Repeat(r) => {
                if matches!(*r.node, Node::Char(_) | Node::Set(_) | Node::Any) {
                    self.rep_simple(r, pos, st, k)
                } else {
                    self.rep(r, 0, pos, st, k)
                }
            }
            Node::Backref(slot) => {
                let Some(&(a, b)) = st.caps[*slot].last() else { return false };
                let l = b - a;
                pos + l <= len && (0..l).all(|i| self.same(self.t[pos + i], self.t[a + i])) && k(pos + l, st)
            }
        }
    }

    /// Matches pieces one after another.
    fn seq(&self, nodes: &[Node], pos: usize, st: &mut State, k: Next<'_>) -> bool {
        match nodes.split_first() {
            None => k(pos, st),
            Some((first, rest)) => self.m(first, pos, st, &mut |p, st: &mut State| self.seq(rest, p, st, k)),
        }
    }

    /// A repeated single character: counts how far it reaches, then hands `k` each length in turn.
    fn rep_simple(&self, r: &Repeat, pos: usize, st: &mut State, k: Next<'_>) -> bool {
        let mut n = 0;
        while n < r.max && self.one(&r.node, pos + n) {
            n += 1;
        }
        if n < r.min {
            return false;
        }
        if r.greedy {
            (r.min..=n).rev().any(|i| k(pos + i, st))
        } else {
            (r.min..=n).any(|i| k(pos + i, st))
        }
    }

    /// A repeated piece after `count` rounds.
    fn rep(&self, r: &Repeat, count: usize, pos: usize, st: &mut State, k: Next<'_>) -> bool {
        let enough = count >= r.min;
        // a lazy quantifier tries to stop first, a greedy one tries one more round first
        if enough && !r.greedy && k(pos, st) {
            return true;
        }
        if self.rep_more(r, count, pos, st, k) {
            return true;
        }
        enough && r.greedy && k(pos, st)
    }

    /// One more round of a repeated piece; a round past the minimum that matches nothing does not count.
    fn rep_more(&self, r: &Repeat, count: usize, pos: usize, st: &mut State, k: Next<'_>) -> bool {
        count < r.max && self.m(&r.node, pos, st, &mut |p, st: &mut State| (count < r.min || p != pos) && self.rep(r, count + 1, p, st, k))
    }

    /// Whether the look-behind's contents match text that ends exactly at `pos`.
    fn behind(&self, look: &Look, pos: usize, st: &mut State) -> bool {
        if pos < look.min {
            return false;
        }
        let first = look.max.map_or(0, |m| pos.saturating_sub(m));
        (first..=pos - look.min).rev().any(|s| self.m(&look.node, s, st, &mut |p, _| p == pos))
    }
}

/// The shortest and longest text a piece can match, in characters (`None` without a limit).
fn width(n: &Node) -> (usize, Option<usize>) {
    match n {
        Node::Char(_) | Node::Set(_) | Node::Any => (1, Some(1)),
        Node::Empty | Node::Start | Node::End | Node::EndAbs | Node::Boundary(_) | Node::Look(_) => (0, Some(0)),
        Node::Concat(v) => v.iter().map(width).fold((0, Some(0)), |(a, b), (c, d)| (a + c, b.zip(d).map(|(x, y)| x + y))),
        Node::Alt(v) => {
            let ws: Vec<_> = v.iter().map(width).collect();
            let min = ws.iter().map(|w| w.0).min().unwrap_or(0);
            let max = ws.iter().try_fold(0, |m, w| w.1.map(|x| m.max(x)));
            (min, max)
        }
        Node::Group(_, inner) | Node::Atomic(inner) => width(inner),
        Node::Repeat(r) => {
            let (a, b) = width(&r.node);
            let max = if r.max == usize::MAX {
                if b == Some(0) {
                    Some(0)
                } else {
                    None
                }
            } else {
                b.map(|x| x * r.max)
            };
            (a * r.min, max)
        }
        Node::Backref(_) => (0, None),
    }
}

/// Reads an expression.
struct Parser<'a> {
    /// the expression, one entry per character
    s: &'a [char],
    /// where reading is
    i: usize,
    /// slots taken so far (slot 0 is the whole match)
    slots: usize,
    /// slots of the numbered groups, in order
    numbered: Vec<usize>,
    /// slots of the named groups
    names: HashMap<String, usize>,
}

impl Parser<'_> {
    /// The next character, if any.
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    /// Whether the text at the reading place starts with `p`; moves past it when it does.
    fn eat(&mut self, p: &str) -> bool {
        let p: Vec<char> = p.chars().collect();
        if self.s[self.i..].starts_with(&p) {
            self.i += p.len();
            true
        } else {
            false
        }
    }

    /// The next character; an error at the end.
    fn next(&mut self) -> Result<char, String> {
        let c = self.peek().ok_or("the expression ends too early")?;
        self.i += 1;
        Ok(c)
    }

    /// Alternatives up to a `)` or the end.
    fn alt(&mut self) -> Result<Node, String> {
        let mut alts = vec![self.concat()?];
        while self.eat("|") {
            alts.push(self.concat()?);
        }
        Ok(if alts.len() == 1 { alts.pop().expect("one") } else { Node::Alt(alts) })
    }

    /// Pieces up to a `|`, a `)` or the end.
    fn concat(&mut self) -> Result<Node, String> {
        let mut nodes = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom()?;
            nodes.push(self.quantifier(atom)?);
        }
        Ok(match nodes.len() {
            0 => Node::Empty,
            1 => nodes.pop().expect("one"),
            _ => Node::Concat(nodes),
        })
    }

    /// The number at the reading place, if there is one.
    fn number(&mut self) -> Option<usize> {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        self.s[start..self.i].iter().collect::<String>().parse().ok()
    }

    /// A quantifier after `atom`, if one follows.
    fn quantifier(&mut self, atom: Node) -> Result<Node, String> {
        let (min, max) = match self.peek() {
            Some('*') => {
                self.i += 1;
                (0, usize::MAX)
            }
            Some('+') => {
                self.i += 1;
                (1, usize::MAX)
            }
            Some('?') => {
                self.i += 1;
                (0, 1)
            }
            Some('{') => {
                let save = self.i;
                self.i += 1;
                let parsed = self.number().and_then(|min| {
                    if self.eat("}") {
                        Some((min, min))
                    } else if self.eat(",") {
                        if self.eat("}") {
                            Some((min, usize::MAX))
                        } else {
                            let max = self.number()?;
                            self.eat("}").then_some((min, max))
                        }
                    } else {
                        None
                    }
                });
                match parsed {
                    Some(q) => q,
                    None => {
                        // not a quantifier: a plain `{`, read as the next literal
                        self.i = save;
                        return Ok(atom);
                    }
                }
            }
            _ => return Ok(atom),
        };
        let greedy = !self.eat("?");
        Ok(Node::Repeat(Repeat { node: Box::new(atom), min, max, greedy }))
    }

    /// A group's name up to `>`.
    fn name(&mut self) -> Result<String, String> {
        let mut name = String::new();
        loop {
            match self.next()? {
                '>' => return Ok(name),
                c => name.push(c),
            }
        }
    }

    /// The slot of a named group, taken on first use.
    fn named_slot(&mut self, name: &str) -> usize {
        if let Some(&s) = self.names.get(name) {
            return s;
        }
        let s = self.slots;
        self.slots += 1;
        self.names.insert(name.to_string(), s);
        s
    }

    /// One piece: a group, a class, an escape or a character.
    fn atom(&mut self) -> Result<Node, String> {
        let c = self.next()?;
        Ok(match c {
            '(' => {
                let node = if self.eat("?:") {
                    self.alt()?
                } else if self.eat("?>") {
                    Node::Atomic(Box::new(self.alt()?))
                } else if self.eat("?=") {
                    self.look(false, false)?
                } else if self.eat("?!") {
                    self.look(false, true)?
                } else if self.eat("?<=") {
                    self.look(true, false)?
                } else if self.eat("?<!") {
                    self.look(true, true)?
                } else if self.eat("?<") || self.eat("?P<") {
                    let name = self.name()?;
                    let slot = self.named_slot(&name);
                    Node::Group(slot, Box::new(self.alt()?))
                } else if self.peek() == Some('?') {
                    return Err(format!("unknown group at {}", self.i));
                } else {
                    let slot = self.slots;
                    self.slots += 1;
                    self.numbered.push(slot);
                    Node::Group(slot, Box::new(self.alt()?))
                };
                if !self.eat(")") {
                    return Err("a group is not closed".into());
                }
                node
            }
            '[' => Node::Set(self.class()?),
            '.' => Node::Any,
            '^' => Node::Start,
            '$' => Node::End,
            '\\' => self.escape()?,
            c => Node::Char(c),
        })
    }

    /// A look-around's contents, after its opening.
    fn look(&mut self, behind: bool, negative: bool) -> Result<Node, String> {
        let node = self.alt()?;
        let (min, max) = width(&node);
        Ok(Node::Look(Look { behind, negative, min, max, node: Box::new(node) }))
    }

    /// Four or two hex digits of a `\u` or `\x` escape.
    fn hex(&mut self, n: usize) -> Result<char, String> {
        let digits: String = (0..n).map(|_| self.next()).collect::<Result<_, _>>()?;
        u32::from_str_radix(&digits, 16).ok().and_then(char::from_u32).ok_or_else(|| format!("bad escape \\{digits}"))
    }

    /// The character a simple escape stands for (`\t`, `\.`, `一`).
    fn escaped_char(&mut self, c: char) -> Result<char, String> {
        Ok(match c {
            't' => '\t',
            'n' => '\n',
            'r' => '\r',
            'f' => '\x0C',
            'v' => '\x0B',
            'e' => '\x1B',
            'a' => '\x07',
            '0' => '\0',
            'u' => self.hex(4)?,
            'x' => self.hex(2)?,
            c => c,
        })
    }

    /// An escape outside a class.
    fn escape(&mut self) -> Result<Node, String> {
        let c = self.next()?;
        Ok(match c {
            'd' => Node::Set(Class { negated: false, items: vec![Item::Digit(false)] }),
            'D' => Node::Set(Class { negated: false, items: vec![Item::Digit(true)] }),
            'w' => Node::Set(Class { negated: false, items: vec![Item::Word(false)] }),
            'W' => Node::Set(Class { negated: false, items: vec![Item::Word(true)] }),
            's' => Node::Set(Class { negated: false, items: vec![Item::Space(false)] }),
            'S' => Node::Set(Class { negated: false, items: vec![Item::Space(true)] }),
            'b' => Node::Boundary(false),
            'B' => Node::Boundary(true),
            'A' | 'G' => Node::Start,
            'Z' => Node::End,
            'z' => Node::EndAbs,
            'k' => {
                if !self.eat("<") {
                    return Err("\\k needs a <name>".into());
                }
                let name = self.name()?;
                Node::Backref(self.named_slot(&name))
            }
            '1'..='9' => {
                self.i -= 1;
                let n = self.number().expect("a digit");
                let slot = *self.numbered.get(n - 1).ok_or_else(|| format!("no group {n}"))?;
                Node::Backref(slot)
            }
            c => Node::Char(self.escaped_char(c)?),
        })
    }

    /// A class, after its `[`.
    fn class(&mut self) -> Result<Class, String> {
        let negated = self.eat("^");
        let mut items = Vec::new();
        let mut first = true;
        loop {
            let c = self.next()?;
            if c == ']' && !first {
                break;
            }
            first = false;
            let lo = if c == '\\' {
                let e = self.next()?;
                match e {
                    'd' => {
                        items.push(Item::Digit(false));
                        continue;
                    }
                    'D' => {
                        items.push(Item::Digit(true));
                        continue;
                    }
                    'w' => {
                        items.push(Item::Word(false));
                        continue;
                    }
                    'W' => {
                        items.push(Item::Word(true));
                        continue;
                    }
                    's' => {
                        items.push(Item::Space(false));
                        continue;
                    }
                    'S' => {
                        items.push(Item::Space(true));
                        continue;
                    }
                    'b' => '\x08',
                    e => self.escaped_char(e)?,
                }
            } else {
                c
            };
            if self.peek() == Some('-') && self.s.get(self.i + 1).is_some_and(|&n| n != ']') {
                self.i += 1;
                let hi = match self.next()? {
                    '\\' => {
                        let e = self.next()?;
                        self.escaped_char(e)?
                    }
                    h => h,
                };
                items.push(Item::Range(lo, hi));
            } else {
                items.push(Item::Range(lo, lo));
            }
        }
        Ok(Class { negated, items })
    }
}

/// Whether a compiled expression can only match at the start of the text.
fn starts_anchored(n: &Node) -> bool {
    match n {
        Node::Start => true,
        Node::Concat(v) => v.first().is_some_and(starts_anchored),
        Node::Group(_, inner) | Node::Atomic(inner) => starts_anchored(inner),
        Node::Alt(v) => v.iter().all(starts_anchored),
        _ => false,
    }
}

impl ClrRegex {
    /// Compiles an upstream expression; panics on one it can't read, since they are all fixed in the code.
    pub fn new(pattern: &str, ignore_case: bool) -> ClrRegex {
        Self::try_new(pattern, ignore_case).unwrap_or_else(|e| panic!("{pattern}: {e}"))
    }

    /// Compiles an expression.
    pub fn try_new(pattern: &str, ignore_case: bool) -> Result<ClrRegex, String> {
        let chars: Vec<char> = pattern.chars().collect();
        let mut p = Parser { s: &chars, i: 0, slots: 1, numbered: Vec::new(), names: HashMap::new() };
        let root = p.alt()?;
        if p.i != chars.len() {
            return Err(format!("unexpected ')' at {}", p.i));
        }
        let anchored = starts_anchored(&root);
        Ok(ClrRegex { pattern: pattern.to_string(), root, slots: p.slots, names: p.names, numbered: p.numbered, ignore_case, anchored })
    }

    /// The expression as written.
    pub fn as_str(&self) -> &str {
        &self.pattern
    }

    /// The first match from character `from` on, as character ranges.
    fn search(&self, chars: &[char], from: usize) -> Option<Vec<Vec<(usize, usize)>>> {
        let m = Matcher { re: self, t: chars };
        let mut st = State { caps: vec![Vec::new(); self.slots], steps: 0 };
        for s in from..=chars.len() {
            if self.anchored && s > 0 {
                break;
            }
            for c in &mut st.caps {
                c.clear();
            }
            if m.m(&self.root, s, &mut st, &mut |p, st| {
                st.caps[0].push((s, p));
                true
            }) {
                return Some(st.caps);
            }
            if st.steps > STEP_LIMIT {
                return None;
            }
        }
        None
    }

    /// Every match, left to right, not overlapping (`Matches`).
    pub fn find_all<'t>(&'t self, text: &'t str) -> Vec<ClrMatch<'t>> {
        let chars: Vec<char> = text.chars().collect();
        let mut bytes: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
        bytes.push(text.len());
        let mut out = Vec::new();
        let mut from = 0;
        while from <= chars.len() {
            let Some(caps) = self.search(&chars, from) else { break };
            let (s, e) = caps[0][0];
            from = if e > s { e } else { e + 1 };
            let caps = caps.into_iter().map(|c| c.into_iter().map(|(a, b)| (bytes[a], bytes[b])).collect()).collect();
            out.push(ClrMatch { text, caps, names: &self.names, numbered: &self.numbered });
        }
        out
    }

    /// The first match (`Match`).
    pub fn find<'t>(&'t self, text: &'t str) -> Option<ClrMatch<'t>> {
        let chars: Vec<char> = text.chars().collect();
        let caps = self.search(&chars, 0)?;
        let mut bytes: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
        bytes.push(text.len());
        let caps = caps.into_iter().map(|c| c.into_iter().map(|(a, b)| (bytes[a], bytes[b])).collect()).collect();
        Some(ClrMatch { text, caps, names: &self.names, numbered: &self.numbered })
    }

    /// Whether the expression matches anywhere (`IsMatch`).
    pub fn is_match(&self, text: &str) -> bool {
        let chars: Vec<char> = text.chars().collect();
        self.search(&chars, 0).is_some()
    }

    /// Every match replaced by what `f` makes of it.
    pub fn replace_all_with(&self, text: &str, f: impl Fn(&ClrMatch<'_>) -> String) -> String {
        let mut out = String::new();
        let mut last = 0;
        for m in self.find_all(text) {
            let w = m.whole();
            out.push_str(&text[last..w.start]);
            out.push_str(&f(&m));
            last = w.end;
        }
        out.push_str(&text[last..]);
        out
    }

    /// Every match replaced by a .NET replacement pattern (`$1`, `${name}`, `$&`, `$$`).
    pub fn replace_all(&self, text: &str, replacement: &str) -> String {
        self.replace_all_with(text, |m| expand(m, replacement))
    }
}

/// A .NET replacement pattern filled in from a match.
fn expand(m: &ClrMatch<'_>, replacement: &str) -> String {
    let mut out = String::new();
    let mut rest = replacement;
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        if let Some(r) = after.strip_prefix('$') {
            out.push('$');
            rest = r;
        } else if let Some(r) = after.strip_prefix('&') {
            out.push_str(m.whole().text);
            rest = r;
        } else if let Some((name, r)) = after.strip_prefix('{').and_then(|a| a.split_once('}')) {
            match name.parse::<usize>() {
                Ok(i) => out.push_str(m.numbered(i).unwrap_or("")),
                Err(_) => out.push_str(m.str(name)),
            }
            rest = r;
        } else {
            let digits = after.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 {
                out.push('$');
                rest = after;
            } else {
                let i: usize = after[..digits].parse().expect("digits");
                out.push_str(m.numbered(i).unwrap_or(""));
                rest = &after[digits..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_groups_keep_every_capture() {
        let re = ClrRegex::new(r"^(?<title>.+?)\.S(?<season>\d{2})(?:E(?<episode>\d{2}))+", true);
        let m = re.find("Show.Name.S01E01E02E03.720p").expect("match");
        assert_eq!(m.str("title"), "Show.Name");
        let eps: Vec<_> = m.captures("episode").iter().map(|c| c.text).collect();
        assert_eq!(eps, ["01", "02", "03"]);
        assert_eq!(m.str("episode"), "03");
    }

    #[test]
    fn groups_with_one_name_share_captures() {
        let re = ClrRegex::new(r"E(?<episode>\d+)-(?<episode>\d+)", false);
        let m = re.find("xE01-05").expect("match");
        assert_eq!(m.captures("episode").len(), 2);
        assert_eq!(m.str("episode"), "05");
    }

    #[test]
    fn captures_of_a_failed_round_are_dropped() {
        let re = ClrRegex::new(r"^(?:(?<d>\d)x)*\dy", false);
        let m = re.find("1x2x3y").expect("match");
        let ds: Vec<_> = m.captures("d").iter().map(|c| c.text).collect();
        assert_eq!(ds, ["1", "2"]);
    }

    #[test]
    fn look_behind_of_any_length() {
        let re = ClrRegex::new(r"(?<!\d+)\d{2}(?!\d+)", false);
        assert_eq!(re.find("a123 45").map(|m| m.whole().text), Some("45"));
        let re = ClrRegex::new(r"(?<=\b\d+)%", false);
        assert!(re.is_match("3%"));
        assert!(!re.is_match("a%"));
    }

    #[test]
    fn classes_ignore_case_and_escapes() {
        let re = ClrRegex::new(r"^\[(?<g>[^]]+)\][一-鿌]+", true);
        assert_eq!(re.find("[Grp]中文").map(|m| m.str("g").to_string()), Some("Grp".into()));
        assert!(ClrRegex::new("ABC", true).is_match("xabcx"));
        assert!(ClrRegex::new(r"[a-c]{2}", true).is_match("BC"));
        assert!(ClrRegex::new(r"(?<s>[-_]?)a\k<s>b", false).is_match("_a_b"));
        assert!(!ClrRegex::new(r"^(?<s>[-_]?)a\k<s>b$", false).is_match("_a-b"));
    }

    #[test]
    fn atomic_groups_do_not_give_back() {
        assert!(!ClrRegex::new(r"^(?>a+)a", false).is_match("aaa"));
        assert!(ClrRegex::new(r"^Part[-._ ](?<e>One|Two)(?>[-._ ])", true).is_match("Part.Two.x"));
    }

    #[test]
    fn replacements_like_dotnet() {
        let re = ClrRegex::new(r"\.E(\d{2,4})\.\d{6}\.(.*-NEXT)$", false);
        assert_eq!(re.replace_all("Show.E01.180101.720p-NEXT", ".S01E$1.$2"), "Show.S01E01.720p-NEXT");
        let re = ClrRegex::new(r"^(?<n>x)?\[(?<g>[^\]]+)\](?<t>.+)", false);
        assert_eq!(re.replace_all("[A]B", "${t} by ${g} $$"), "B by A $");
        // numbered groups come before named ones, as in .NET
        assert_eq!(ClrRegex::new(r"(?<n>a)(b)", false).replace_all("ab", "$1"), "b");
    }

    #[test]
    fn lazy_and_empty_matches() {
        let re = ClrRegex::new(r"a*?", false);
        assert_eq!(re.find_all("aa").len(), 3);
        assert_eq!(ClrRegex::new(r"^(.+?)x", false).find("abxcx").map(|m| m.whole().text), Some("abx"));
    }
}
