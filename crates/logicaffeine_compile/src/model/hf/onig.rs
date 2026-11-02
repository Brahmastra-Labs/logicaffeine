//! Regular expressions with the semantics `tokenizers` compiles a `tokenizer.json`'s patterns with: Oniguruma, Ruby
//! syntax, over UTF-8 text. Alternatives are tried left first and quantifiers backtrack (greedy longest first, lazy
//! shortest first); a lookahead or atomic group is matched once and never re-entered. The classes are Unicode's:
//! `\s` the White_Space property, `\d` the decimal digits (Nd), `\w` the Alphabetic property, marks, decimal digits
//! and connector punctuation (and, from its Latin-1 table, the superscript digits and vulgar fractions), `\h` the
//! hexadecimal digits, `\p{..}` a general category, `.` every codepoint but a line feed, and
//! `(?i)` compares by simple case folding. `^` and `$` are line anchors, as Ruby has them.
//!
//! A pattern runs over codepoints; [`Regex::find_iter`] walks a text as the `onig` crate's `find_iter` does, which is
//! the walk `tokenizers` splits by. A construct no tokenizer pattern uses (a backreference, a lookbehind, a script
//! property, a possessive quantifier, a loop whose body can match nothing) is refused by name.
//!
//! [`Regex::tiktoken`] reads a tiktoken pattern, which `tiktoken` compiles with fancy-regex: the same constructs where
//! their meanings agree, and the Han script (`\p{Han}`), nested classes and `&&`, which intersects what precedes it
//! in a class with the rest of the class (Kimi K2's `[\p{Lu}\p{M}&&[^\p{Han}]]`). The constructs whose meaning
//! differs there (`^` and `$`, text anchors; `\b`, `\w` and their negations, Unicode's word characters; `\h`, `\Z`)
//! are refused by name.

use super::unicode_data::{CATEGORY_NAMES, CATEGORY_RANGES, HAN, SIMPLE_FOLDS, WHITE_SPACE, WORD};
use std::collections::HashMap;
use std::sync::OnceLock;

/// A codepoint's general category, as an index into [`CATEGORY_NAMES`].
pub(super) fn category(c: u32) -> u8 {
    let at = CATEGORY_RANGES.partition_point(|&e| (e >> 5) <= c);
    (CATEGORY_RANGES[at - 1] & 31) as u8
}

/// The general category's name: `Lu`, `Mn`, `Nd`, …
pub(super) fn category_name(c: u32) -> &'static str {
    CATEGORY_NAMES[category(c) as usize]
}

/// The White_Space property.
pub(super) fn is_white_space(c: u32) -> bool {
    WHITE_SPACE.binary_search(&c).is_ok()
}

/// `c`'s simple case fold.
pub(super) fn fold(c: u32) -> u32 {
    match SIMPLE_FOLDS.binary_search_by_key(&c, |&(from, _)| from) {
        Ok(i) => SIMPLE_FOLDS[i].1,
        Err(_) => c,
    }
}

/// Every codepoint whose fold is `f`, `f` among them.
fn fold_class(f: u32) -> &'static [u32] {
    static CLASSES: OnceLock<HashMap<u32, Vec<u32>>> = OnceLock::new();
    let classes = CLASSES.get_or_init(|| {
        let mut m: HashMap<u32, Vec<u32>> = HashMap::new();
        for &(from, to) in SIMPLE_FOLDS.iter() {
            m.entry(to).or_insert_with(|| vec![to]).push(from);
        }
        m
    });
    classes.get(&f).map(Vec::as_slice).unwrap_or(&[])
}

/// A set of codepoints one class item names.
#[derive(Clone, Debug, PartialEq)]
enum Set {
    Range(u32, u32),
    /// A union of general categories, by bit.
    Categories(u32),
    WhiteSpace,
    Word,
    Hex,
    /// The Han script.
    Han,
    /// A nested bracketed class.
    Class(Box<Class>),
}

impl Set {
    fn contains(&self, c: u32) -> bool {
        match *self {
            Set::Range(lo, hi) => (lo..=hi).contains(&c),
            Set::Categories(bits) => bits & (1 << category(c)) != 0,
            Set::WhiteSpace => is_white_space(c),
            Set::Word => {
                let at = WORD.partition_point(|&(first, _)| first <= c);
                at > 0 && c <= WORD[at - 1].1
            }
            Set::Hex => matches!(c, 0x30..=0x39 | 0x41..=0x46 | 0x61..=0x66),
            Set::Han => {
                let at = HAN.partition_point(|&(first, _)| first <= c);
                at > 0 && c <= HAN[at - 1].1
            }
            Set::Class(ref class) => class.matches(c),
        }
    }
}

/// A bracketed or escaped class: items, each possibly negated, intersected with the class after a `&&`, and the class
/// negated as a whole.
#[derive(Clone, Debug, PartialEq)]
struct Class {
    items: Vec<(Set, bool)>,
    and: Option<Box<Class>>,
    negated: bool,
    fold: bool,
}

impl Class {
    fn has(&self, c: u32) -> bool {
        self.items.iter().any(|(s, neg)| s.contains(c) != *neg) && self.and.as_ref().is_none_or(|rest| rest.matches(c))
    }
    fn matches(&self, c: u32) -> bool {
        let hit = if self.fold {
            let f = fold(c);
            self.has(c) || self.has(f) || fold_class(f).iter().any(|&x| self.has(x))
        } else {
            self.has(c)
        };
        hit != self.negated
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Inst {
    Char(u32),
    /// A codepoint compared by simple case fold (the fold is stored).
    FoldChar(u32),
    Class(usize),
    /// Every codepoint but a line feed.
    Any,
    /// Try `.0` first, `.1` on failure.
    Split(usize, usize),
    Jmp(usize),
    /// A sub-program matched once at this position: `negated` for a negative lookahead, `consume` for an atomic
    /// group (the position moves to its end).
    Look { prog: usize, negated: bool, consume: bool },
    LineStart,
    LineEnd,
    TextStart,
    TextEnd,
    /// `\Z`: the end, or before a final line feed.
    TextEndNewline,
    WordBoundary(bool),
    Match,
}

/// A compiled pattern.
#[derive(Clone, Debug)]
pub struct Regex {
    progs: Vec<Vec<Inst>>,
    classes: Vec<Class>,
}

/// A parsed pattern node.
#[derive(Clone, Debug)]
enum Node {
    Empty,
    Char(u32, bool),
    Class(Class),
    Any,
    Concat(Vec<Node>),
    Alt(Vec<Node>),
    Repeat { node: Box<Node>, min: u32, max: Option<u32>, greedy: bool },
    Look { node: Box<Node>, negated: bool },
    Atomic(Box<Node>),
    LineStart,
    LineEnd,
    TextStart,
    TextEnd,
    TextEndNewline,
    WordBoundary(bool),
}

impl Node {
    /// Whether the node can match without consuming a codepoint.
    fn nullable(&self) -> bool {
        match self {
            Node::Empty | Node::Look { .. } => true,
            Node::LineStart | Node::LineEnd | Node::TextStart | Node::TextEnd | Node::TextEndNewline | Node::WordBoundary(_) => true,
            Node::Char(..) | Node::Class(_) | Node::Any => false,
            Node::Concat(v) => v.iter().all(Node::nullable),
            Node::Alt(v) => v.iter().any(Node::nullable),
            Node::Repeat { node, min, .. } => *min == 0 || node.nullable(),
            Node::Atomic(n) => n.nullable(),
        }
    }
}

/// The syntax a pattern is read in: `tokenizers`' (Oniguruma's Ruby syntax) or `tiktoken`'s (fancy-regex's).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Syntax {
    Oniguruma,
    Tiktoken,
}

struct Parser<'a> {
    pattern: &'a str,
    chars: Vec<char>,
    at: usize,
    syntax: Syntax,
}

impl<'a> Parser<'a> {
    fn err(&self, why: &str) -> String {
        format!("the tokenizer pattern {:?} is refused at codepoint {}: {why}", self.pattern, self.at)
    }
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }
    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn next(&mut self) -> Result<char, String> {
        let c = self.peek().ok_or_else(|| self.err("the pattern ends early"))?;
        self.at += 1;
        Ok(c)
    }

    fn alt(&mut self, fold: bool) -> Result<Node, String> {
        let mut fold = fold;
        let mut branches = vec![self.concat(&mut fold)?];
        while self.eat('|') {
            branches.push(self.concat(&mut fold)?);
        }
        Ok(if branches.len() == 1 { branches.pop().unwrap() } else { Node::Alt(branches) })
    }

    /// A sequence; an inline `(?i)` turns case folding on for the rest of the enclosing group.
    fn concat(&mut self, fold: &mut bool) -> Result<Node, String> {
        let mut items = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            if c == '(' && self.chars.get(self.at + 1) == Some(&'?') {
                let rest: String = self.chars[self.at + 2..].iter().take(3).collect();
                if rest.starts_with("i)") {
                    self.at += 4;
                    *fold = true;
                    continue;
                }
                if rest.starts_with("-i)") {
                    self.at += 5;
                    *fold = false;
                    continue;
                }
            }
            let atom = self.atom(*fold)?;
            items.push(self.quantified(atom)?);
        }
        Ok(match items.len() {
            0 => Node::Empty,
            1 => items.pop().unwrap(),
            _ => Node::Concat(items),
        })
    }

    fn quantified(&mut self, atom: Node) -> Result<Node, String> {
        let mut node = atom;
        loop {
            let (min, max, interval) = match self.peek() {
                Some('*') => (0, None, false),
                Some('+') => (1, None, false),
                Some('?') => (0, Some(1), false),
                Some('{') => match self.braces()? {
                    Some((min, max)) => (min, max, true),
                    None => return Ok(node),
                },
                _ => return Ok(node),
            };
            if !interval {
                self.at += 1;
            }
            let greedy = !self.eat('?');
            if greedy && !interval && self.peek() == Some('+') {
                return Err(self.err("a possessive quantifier"));
            }
            if max.is_none() && node.nullable() {
                return Err(self.err("an unbounded loop whose body can match nothing"));
            }
            node = Node::Repeat { node: Box::new(node), min, max, greedy };
        }
    }

    /// `{n}`, `{n,}`, `{,m}`, `{n,m}`, consumed when it is one (a brace that is not a quantifier is a literal).
    fn braces(&mut self) -> Result<Option<(u32, Option<u32>)>, String> {
        let start = self.at;
        let close = match self.chars[start..].iter().position(|&c| c == '}') {
            Some(p) => start + p,
            None => return Ok(None),
        };
        let body: String = self.chars[start + 1..close].iter().collect();
        let num = |s: &str| s.parse::<u32>().ok();
        let parsed = match body.split_once(',') {
            None => num(&body).map(|n| (n, Some(n))),
            Some(("", hi)) => num(hi).map(|m| (0, Some(m))),
            Some((lo, "")) => num(lo).map(|n| (n, None)),
            Some((lo, hi)) => num(lo).zip(num(hi)).map(|(n, m)| (n, Some(m))),
        };
        if parsed.is_some() {
            self.at = close + 1;
        }
        Ok(parsed)
    }

    fn atom(&mut self, fold: bool) -> Result<Node, String> {
        let c = self.next()?;
        Ok(match c {
            '.' => Node::Any,
            '^' | '$' if self.syntax == Syntax::Tiktoken => return Err(self.err("an anchor fancy-regex reads at the text's ends")),
            '^' => Node::LineStart,
            '$' => Node::LineEnd,
            '[' => Node::Class(self.class(fold)?),
            '(' => {
                let node = if self.eat('?') {
                    match self.next()? {
                        ':' => self.alt(fold)?,
                        '=' => Node::Look { node: Box::new(self.alt(fold)?), negated: false },
                        '!' => Node::Look { node: Box::new(self.alt(fold)?), negated: true },
                        '>' => Node::Atomic(Box::new(self.alt(fold)?)),
                        'i' if self.eat(':') => self.alt(true)?,
                        '-' if self.eat('i') && self.eat(':') => self.alt(false)?,
                        '<' => return Err(self.err("a lookbehind or a named group")),
                        _ => return Err(self.err("a group option other than i")),
                    }
                } else {
                    self.alt(fold)?
                };
                if !self.eat(')') {
                    return Err(self.err("a group is not closed"));
                }
                node
            }
            ')' => return Err(self.err("an unopened group closes")),
            '*' | '+' | '?' => return Err(self.err("a quantifier with nothing to repeat")),
            '\\' => self.escape(fold)?,
            c => Node::Char(c as u32, fold),
        })
    }

    fn escape(&mut self, fold: bool) -> Result<Node, String> {
        let c = self.next()?;
        if self.syntax == Syntax::Tiktoken && matches!(c, 'b' | 'B' | 'w' | 'W' | 'h' | 'H' | 'Z') {
            return Err(self.err(&format!("the escape \\{c}, which fancy-regex reads otherwise")));
        }
        Ok(match c {
            'A' => Node::TextStart,
            'z' => Node::TextEnd,
            'Z' => Node::TextEndNewline,
            'b' => Node::WordBoundary(true),
            'B' => Node::WordBoundary(false),
            'G' | 'K' | 'k' | 'g' | 'R' | 'X' | 'y' | 'Y' => return Err(self.err("an escape no tokenizer pattern uses")),
            '1'..='9' => return Err(self.err("a backreference")),
            _ => match self.class_escape(c, fold)? {
                Ok(set) => Node::Class(Class { items: vec![set], and: None, negated: false, fold }),
                Err(ch) => Node::Char(ch, fold),
            },
        })
    }

    /// An escape inside or outside a class: a set (`\s`, `\p{L}`, …, possibly negated) or one codepoint.
    fn class_escape(&mut self, c: char, _fold: bool) -> Result<Result<(Set, bool), u32>, String> {
        Ok(match c {
            's' => Ok((Set::WhiteSpace, false)),
            'S' => Ok((Set::WhiteSpace, true)),
            'd' => Ok((Set::Categories(1 << 8), false)),
            'D' => Ok((Set::Categories(1 << 8), true)),
            'w' => Ok((Set::Word, false)),
            'W' => Ok((Set::Word, true)),
            'h' => Ok((Set::Hex, false)),
            'H' => Ok((Set::Hex, true)),
            'p' | 'P' => {
                if !self.eat('{') {
                    return Err(self.err("a property escape without braces"));
                }
                let mut negated = c == 'P';
                if self.eat('^') {
                    negated = !negated;
                }
                let mut name = String::new();
                loop {
                    match self.next()? {
                        '}' => break,
                        ch => name.push(ch),
                    }
                }
                if self.syntax == Syntax::Tiktoken && name.replace(['_', ' ', '-'], "").eq_ignore_ascii_case("han") {
                    return Ok(Ok((Set::Han, negated)));
                }
                let bits = property(&name).ok_or_else(|| self.err(&format!("the property `{name}`")))?;
                Ok((Set::Categories(bits), negated))
            }
            'n' => Err(0x0A),
            'r' => Err(0x0D),
            't' => Err(0x09),
            'f' => Err(0x0C),
            'v' => Err(0x0B),
            'a' => Err(0x07),
            'e' => Err(0x1B),
            '0' => Err(0),
            'x' => {
                let digits: String = if self.eat('{') {
                    let mut s = String::new();
                    loop {
                        match self.next()? {
                            '}' => break,
                            ch => s.push(ch),
                        }
                    }
                    s
                } else {
                    (0..2).filter_map(|_| self.peek().filter(char::is_ascii_hexdigit).inspect(|_| self.at += 1)).collect()
                };
                Err(u32::from_str_radix(&digits, 16).map_err(|_| self.err("a hexadecimal escape"))?)
            }
            'u' => {
                let digits: String = (0..4).map(|_| self.next()).collect::<Result<_, _>>()?;
                Err(u32::from_str_radix(&digits, 16).map_err(|_| self.err("a \\u escape"))?)
            }
            c if c.is_ascii_alphanumeric() => return Err(self.err(&format!("the escape \\{c}"))),
            c => Err(c as u32),
        })
    }

    /// A bracketed class after its `[`.
    fn class(&mut self, fold: bool) -> Result<Class, String> {
        let negated = self.eat('^');
        let mut class = self.class_body(fold, true)?;
        class.negated = negated;
        Ok(class)
    }

    /// A class's items up to its closing `]`, consumed; at a `&&`, the items so far intersected with the rest. A `]` at
    /// the very start (`first`) is a literal.
    fn class_body(&mut self, fold: bool, mut first: bool) -> Result<Class, String> {
        let mut items: Vec<(Set, bool)> = Vec::new();
        loop {
            let c = self.next()?;
            if c == ']' && !first {
                break;
            }
            first = false;
            if c == '[' {
                if self.syntax != Syntax::Tiktoken {
                    return Err(self.err("a nested class"));
                }
                items.push((Set::Class(Box::new(self.class(fold)?)), false));
                continue;
            }
            if c == '&' && self.peek() == Some('&') {
                if self.syntax != Syntax::Tiktoken {
                    return Err(self.err("a class intersection"));
                }
                self.at += 1;
                let rest = self.class_body(fold, false)?;
                return Ok(Class { items, and: Some(Box::new(rest)), negated: false, fold });
            }
            let lo = if c == '\\' {
                let e = self.next()?;
                match self.class_escape(e, fold)? {
                    Ok(set) => {
                        items.push(set);
                        continue;
                    }
                    Err(ch) => ch,
                }
            } else {
                c as u32
            };
            if self.peek() == Some('-') && self.chars.get(self.at + 1).is_some_and(|&n| n != ']') {
                self.at += 1;
                let h = self.next()?;
                let hi = if h == '\\' {
                    let e = self.next()?;
                    match self.class_escape(e, fold)? {
                        Ok(_) => return Err(self.err("a range ending in a class")),
                        Err(ch) => ch,
                    }
                } else {
                    h as u32
                };
                if hi < lo {
                    return Err(self.err("a range out of order"));
                }
                items.push((Set::Range(lo, hi), false));
            } else {
                items.push((Set::Range(lo, lo), false));
            }
        }
        Ok(Class { items, and: None, negated: false, fold })
    }
}

/// The general categories a property name selects, as bits.
fn property(name: &str) -> Option<u32> {
    let bit = |n: &str| CATEGORY_NAMES.iter().position(|c| *c == n).map(|i| 1u32 << i);
    let all = |prefix: char| CATEGORY_NAMES.iter().enumerate().filter(|(_, c)| c.starts_with(prefix)).fold(0u32, |b, (i, _)| b | 1 << i);
    let n = name.replace(['_', ' ', '-'], "").to_ascii_lowercase();
    Some(match n.as_str() {
        "l" | "letter" => all('L'),
        "m" | "mark" | "combiningmark" => all('M'),
        "n" | "number" => all('N'),
        "p" | "punctuation" | "punct" => all('P'),
        "s" | "symbol" => all('S'),
        "z" | "separator" => all('Z'),
        "c" | "other" => all('C'),
        "lu" | "uppercaseletter" => bit("Lu")?,
        "ll" | "lowercaseletter" => bit("Ll")?,
        "lt" | "titlecaseletter" => bit("Lt")?,
        "lm" | "modifierletter" => bit("Lm")?,
        "lo" | "otherletter" => bit("Lo")?,
        "mn" | "nonspacingmark" => bit("Mn")?,
        "mc" | "spacingmark" => bit("Mc")?,
        "me" | "enclosingmark" => bit("Me")?,
        "nd" | "decimalnumber" | "digit" => bit("Nd")?,
        "nl" | "letternumber" => bit("Nl")?,
        "no" | "othernumber" => bit("No")?,
        "pc" | "connectorpunctuation" => bit("Pc")?,
        "pd" | "dashpunctuation" => bit("Pd")?,
        "ps" | "openpunctuation" => bit("Ps")?,
        "pe" | "closepunctuation" => bit("Pe")?,
        "pi" | "initialpunctuation" => bit("Pi")?,
        "pf" | "finalpunctuation" => bit("Pf")?,
        "po" | "otherpunctuation" => bit("Po")?,
        "sm" | "mathsymbol" => bit("Sm")?,
        "sc" | "currencysymbol" => bit("Sc")?,
        "sk" | "modifiersymbol" => bit("Sk")?,
        "so" | "othersymbol" => bit("So")?,
        "zs" | "spaceseparator" => bit("Zs")?,
        "zl" | "lineseparator" => bit("Zl")?,
        "zp" | "paragraphseparator" => bit("Zp")?,
        "cc" | "control" | "cntrl" => bit("Cc")?,
        "cf" | "format" => bit("Cf")?,
        "cs" | "surrogate" => bit("Cs")?,
        "co" | "privateuse" => bit("Co")?,
        "cn" | "unassigned" => bit("Cn")?,
        _ => return None,
    })
}

/// Compiles nodes into programs.
struct Compiler {
    progs: Vec<Vec<Inst>>,
    classes: Vec<Class>,
}

impl Compiler {
    fn emit(&mut self, prog: usize, node: &Node) {
        match node {
            Node::Empty => {}
            Node::Char(c, fold_on) => {
                let inst = if *fold_on && fold_class(fold(*c)).len() > 1 { Inst::FoldChar(fold(*c)) } else { Inst::Char(*c) };
                self.progs[prog].push(inst);
            }
            Node::Class(cl) => {
                self.classes.push(cl.clone());
                let id = self.classes.len() - 1;
                self.progs[prog].push(Inst::Class(id));
            }
            Node::Any => self.progs[prog].push(Inst::Any),
            Node::LineStart => self.progs[prog].push(Inst::LineStart),
            Node::LineEnd => self.progs[prog].push(Inst::LineEnd),
            Node::TextStart => self.progs[prog].push(Inst::TextStart),
            Node::TextEnd => self.progs[prog].push(Inst::TextEnd),
            Node::TextEndNewline => self.progs[prog].push(Inst::TextEndNewline),
            Node::WordBoundary(b) => self.progs[prog].push(Inst::WordBoundary(*b)),
            Node::Concat(v) => {
                for n in v {
                    self.emit(prog, n);
                }
            }
            Node::Alt(v) => {
                let mut jumps = Vec::new();
                for (i, n) in v.iter().enumerate() {
                    if i + 1 < v.len() {
                        let split = self.progs[prog].len();
                        self.progs[prog].push(Inst::Split(split + 1, 0));
                        self.emit(prog, n);
                        jumps.push(self.progs[prog].len());
                        self.progs[prog].push(Inst::Jmp(0));
                        let next = self.progs[prog].len();
                        self.progs[prog][split] = Inst::Split(split + 1, next);
                    } else {
                        self.emit(prog, n);
                    }
                }
                let end = self.progs[prog].len();
                for j in jumps {
                    self.progs[prog][j] = Inst::Jmp(end);
                }
            }
            Node::Repeat { node, min, max, greedy } => {
                for _ in 0..*min {
                    self.emit(prog, node);
                }
                match max {
                    None => {
                        let top = self.progs[prog].len();
                        self.progs[prog].push(Inst::Split(0, 0));
                        self.emit(prog, node);
                        self.progs[prog].push(Inst::Jmp(top));
                        let out = self.progs[prog].len();
                        self.progs[prog][top] = if *greedy { Inst::Split(top + 1, out) } else { Inst::Split(out, top + 1) };
                    }
                    Some(max) => {
                        let mut splits = Vec::new();
                        for _ in *min..*max {
                            splits.push(self.progs[prog].len());
                            self.progs[prog].push(Inst::Split(0, 0));
                            self.emit(prog, node);
                        }
                        let out = self.progs[prog].len();
                        for s in splits {
                            self.progs[prog][s] = if *greedy { Inst::Split(s + 1, out) } else { Inst::Split(out, s + 1) };
                        }
                    }
                }
            }
            Node::Look { node, negated } => {
                let sub = self.sub(node);
                self.progs[prog].push(Inst::Look { prog: sub, negated: *negated, consume: false });
            }
            Node::Atomic(node) => {
                let sub = self.sub(node);
                self.progs[prog].push(Inst::Look { prog: sub, negated: false, consume: true });
            }
        }
    }

    fn sub(&mut self, node: &Node) -> usize {
        self.progs.push(Vec::new());
        let id = self.progs.len() - 1;
        self.emit(id, node);
        self.progs[id].push(Inst::Match);
        id
    }
}

impl Regex {
    /// A pattern as `tokenizers` compiles it.
    pub fn new(pattern: &str) -> Result<Regex, String> {
        Regex::read(pattern, Syntax::Oniguruma)
    }

    /// A pattern as `tiktoken` compiles it.
    pub fn tiktoken(pattern: &str) -> Result<Regex, String> {
        Regex::read(pattern, Syntax::Tiktoken)
    }

    fn read(pattern: &str, syntax: Syntax) -> Result<Regex, String> {
        let mut p = Parser { pattern, chars: pattern.chars().collect(), at: 0, syntax };
        let node = p.alt(false)?;
        if p.at != p.chars.len() {
            return Err(p.err("an unopened group closes"));
        }
        let mut c = Compiler { progs: Vec::new(), classes: Vec::new() };
        c.sub(&node);
        Ok(Regex { progs: c.progs, classes: c.classes })
    }

    fn is_word(c: u32) -> bool {
        Set::Word.contains(c)
    }

    /// The end of the first match of program `prog` anchored at `start`, in priority order.
    fn run(&self, prog: usize, text: &[u32], start: usize) -> Option<usize> {
        let code = &self.progs[prog];
        let mut stack: Vec<(usize, usize)> = vec![(0, start)];
        'next: while let Some((mut pc, mut at)) = stack.pop() {
            loop {
                match code[pc] {
                    Inst::Match => return Some(at),
                    Inst::Char(c) => {
                        if text.get(at) != Some(&c) {
                            continue 'next;
                        }
                        at += 1;
                        pc += 1;
                    }
                    Inst::FoldChar(f) => {
                        if !text.get(at).is_some_and(|&c| fold(c) == f) {
                            continue 'next;
                        }
                        at += 1;
                        pc += 1;
                    }
                    Inst::Class(id) => {
                        if !text.get(at).is_some_and(|&c| self.classes[id].matches(c)) {
                            continue 'next;
                        }
                        at += 1;
                        pc += 1;
                    }
                    Inst::Any => {
                        if !text.get(at).is_some_and(|&c| c != 0x0A) {
                            continue 'next;
                        }
                        at += 1;
                        pc += 1;
                    }
                    Inst::Split(a, b) => {
                        stack.push((b, at));
                        pc = a;
                    }
                    Inst::Jmp(a) => pc = a,
                    Inst::Look { prog, negated, consume } => {
                        let found = self.run(prog, text, at);
                        if found.is_some() == negated {
                            continue 'next;
                        }
                        if consume {
                            at = found.unwrap();
                        }
                        pc += 1;
                    }
                    Inst::LineStart => {
                        if !(at == 0 || text[at - 1] == 0x0A) {
                            continue 'next;
                        }
                        pc += 1;
                    }
                    Inst::LineEnd => {
                        if !(at == text.len() || text[at] == 0x0A) {
                            continue 'next;
                        }
                        pc += 1;
                    }
                    Inst::TextStart => {
                        if at != 0 {
                            continue 'next;
                        }
                        pc += 1;
                    }
                    Inst::TextEnd => {
                        if at != text.len() {
                            continue 'next;
                        }
                        pc += 1;
                    }
                    Inst::TextEndNewline => {
                        if !(at == text.len() || (at + 1 == text.len() && text[at] == 0x0A)) {
                            continue 'next;
                        }
                        pc += 1;
                    }
                    Inst::WordBoundary(want) => {
                        let before = at > 0 && Self::is_word(text[at - 1]);
                        let after = at < text.len() && Self::is_word(text[at]);
                        if (before != after) != want {
                            continue 'next;
                        }
                        pc += 1;
                    }
                }
            }
        }
        None
    }

    /// The leftmost match starting at or after `from`.
    fn search(&self, text: &[u32], from: usize) -> Option<(usize, usize)> {
        (from..=text.len()).find_map(|s| self.run(0, text, s).map(|e| (s, e)))
    }

    /// The matches in `text` as the `onig` crate's `find_iter` yields them, in codepoint offsets: each search starts
    /// where the last match ended, and an empty match right at the last match's end is passed over by one codepoint.
    pub fn find_iter(&self, text: &[u32]) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let (mut last_end, mut last_match) = (0usize, None::<usize>);
        while last_end <= text.len() {
            let Some((s, e)) = self.search(text, last_end) else { break };
            if s == e && last_match == Some(e) {
                last_end += 1;
                continue;
            }
            last_end = e;
            last_match = Some(e);
            out.push((s, e));
        }
        out
    }
}
