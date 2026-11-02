//! The regular expressions of the pre-tokenizers that llama.cpp runs through `std::regex` / `std::wregex`
//! (`unicode_regex_split_stl`), matched with the semantics libstdc++ gives them: ECMAScript grammar, depth-first
//! backtracking, the first match in priority order (left alternative first, greedy quantifiers longest first), and
//! the `regex_iterator` walk over one span — an empty match is retried at its end as non-null and continuous, and
//! every later search runs with `match_prev_avail`.
//!
//! A pattern runs over code units of one [`Width`]: [`Width::Narrow`] is llama.cpp's *collapsed* text (one byte per
//! codepoint, a Unicode category folded to one byte, matched by `std::regex` over signed `char`), [`Width::Wide`]
//! is its codepoint text (`std::wregex`). The character classes are libstdc++'s in the classic locale, measured
//! against it for every code unit by `model_tokenizer_lock`: `\s` is `\t \n \v \f \r` and space, `\d` is ASCII
//! digits, `\w` is ASCII alphanumerics and `_`, and `.` is every unit but `\n` and `\r` (and, wide, U+2028/U+2029).
//!
//! The matcher keeps its own backtrack stack, so a long input cannot exhaust the thread's stack. A quantified group
//! that can match the empty string is refused at compile time: libstdc++ admits one empty iteration where
//! ECMAScript admits none, and no pre-tokenizer pattern depends on either.

use std::fmt;

/// The code-unit width a pattern runs over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    /// One byte per unit, compared as signed `char` the way `std::regex` compares them.
    Narrow,
    /// One codepoint per unit (`std::wregex`, 32-bit `wchar_t`).
    Wide,
}

/// A pattern the compiler refuses, by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegexError {
    pub pattern: String,
    pub at: usize,
    pub why: &'static str,
}

impl fmt::Display for RegexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the pre-tokenizer pattern {:?} is refused at unit {}: {}", self.pattern, self.at, self.why)
    }
}

impl std::error::Error for RegexError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Named {
    Space,
    Digit,
    Word,
}

impl Named {
    fn contains(self, u: u32) -> bool {
        match self {
            Named::Space => matches!(u, 0x09..=0x0D | 0x20),
            Named::Digit => (0x30..=0x39).contains(&u),
            Named::Word => {
                (0x30..=0x39).contains(&u) || (0x41..=0x5A).contains(&u) || (0x61..=0x7A).contains(&u) || u == 0x5F
            }
        }
    }
}

/// A unit's position in range order: `std::regex` compares `char`s, which are signed.
fn key(width: Width, u: u32) -> i64 {
    match width {
        Width::Narrow => u as u8 as i8 as i64,
        Width::Wide => u as i64,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ClassItem {
    Unit(u32),
    Range(i64, i64),
    Named { class: Named, negate: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Class {
    negate: bool,
    items: Vec<ClassItem>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Look {
    LineStart,
    LineEnd,
    WordBoundary,
    NotWordBoundary,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Unit(u32),
    Class(Class),
    Any,
    Concat(Vec<Node>),
    Alt(Vec<Node>),
    Repeat { node: Box<Node>, min: u32, max: Option<u32>, greedy: bool },
    Look(Look),
    Ahead { node: Box<Node>, negate: bool },
}

impl Node {
    /// Whether the node can match without consuming a unit.
    fn nullable(&self) -> bool {
        match self {
            Node::Unit(_) | Node::Class(_) | Node::Any => false,
            Node::Concat(v) => v.iter().all(Node::nullable),
            Node::Alt(v) => v.iter().any(Node::nullable),
            Node::Repeat { node, min, .. } => *min == 0 || node.nullable(),
            Node::Look(_) | Node::Ahead { .. } => true,
        }
    }
}

struct Parser<'a> {
    p: &'a [u32],
    i: usize,
    pattern: &'a str,
    width: Width,
}

impl<'a> Parser<'a> {
    fn err(&self, why: &'static str) -> RegexError {
        RegexError { pattern: self.pattern.to_string(), at: self.i, why }
    }

    fn peek(&self) -> Option<u32> {
        self.p.get(self.i).copied()
    }

    fn peek_at(&self, k: usize) -> Option<u32> {
        self.p.get(self.i + k).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c as u32) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn disjunction(&mut self) -> Result<Node, RegexError> {
        let mut alts = vec![self.alternative()?];
        while self.eat('|') {
            alts.push(self.alternative()?);
        }
        Ok(if alts.len() == 1 { alts.pop().expect("one alternative") } else { Node::Alt(alts) })
    }

    fn alternative(&mut self) -> Result<Node, RegexError> {
        let mut terms = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' as u32 || c == ')' as u32 {
                break;
            }
            terms.push(self.term()?);
        }
        Ok(if terms.len() == 1 { terms.pop().expect("one term") } else { Node::Concat(terms) })
    }

    fn term(&mut self) -> Result<Node, RegexError> {
        let c = self.peek().expect("a term starts at a unit");
        if c == '^' as u32 {
            self.i += 1;
            return Ok(Node::Look(Look::LineStart));
        }
        if c == '$' as u32 {
            self.i += 1;
            return Ok(Node::Look(Look::LineEnd));
        }
        if c == '\\' as u32 {
            match self.peek_at(1) {
                Some(b) if b == 'b' as u32 => {
                    self.i += 2;
                    return Ok(Node::Look(Look::WordBoundary));
                }
                Some(b) if b == 'B' as u32 => {
                    self.i += 2;
                    return Ok(Node::Look(Look::NotWordBoundary));
                }
                _ => {}
            }
        }
        if c == '(' as u32 && self.peek_at(1) == Some('?' as u32) {
            match self.peek_at(2) {
                Some(k) if k == '=' as u32 || k == '!' as u32 => {
                    self.i += 3;
                    let node = self.disjunction()?;
                    if !self.eat(')') {
                        return Err(self.err("a lookahead is not closed"));
                    }
                    return Ok(Node::Ahead { node: Box::new(node), negate: k == '!' as u32 });
                }
                _ => {}
            }
        }
        let atom = self.atom()?;
        self.quantified(atom)
    }

    fn quantified(&mut self, atom: Node) -> Result<Node, RegexError> {
        let (min, max) = match self.peek() {
            Some(c) if c == '*' as u32 => {
                self.i += 1;
                (0, None)
            }
            Some(c) if c == '+' as u32 => {
                self.i += 1;
                (1, None)
            }
            Some(c) if c == '?' as u32 => {
                self.i += 1;
                (0, Some(1))
            }
            Some(c) if c == '{' as u32 => {
                self.i += 1;
                let min = self.decimal().ok_or_else(|| self.err("an interval has no lower bound"))?;
                let max = if self.eat(',') {
                    if self.peek() == Some('}' as u32) {
                        None
                    } else {
                        Some(self.decimal().ok_or_else(|| self.err("an interval's upper bound is not a number"))?)
                    }
                } else {
                    Some(min)
                };
                if !self.eat('}') {
                    return Err(self.err("an interval is not closed"));
                }
                if max.is_some_and(|m| m < min) {
                    return Err(self.err("an interval's upper bound is below its lower bound"));
                }
                (min, max)
            }
            _ => return Ok(atom),
        };
        let greedy = !self.eat('?');
        if atom.nullable() && max != Some(1) {
            return Err(self.err("a quantified group that can match the empty string"));
        }
        Ok(Node::Repeat { node: Box::new(atom), min, max, greedy })
    }

    fn decimal(&mut self) -> Option<u32> {
        let start = self.i;
        let mut v: u32 = 0;
        while let Some(c) = self.peek() {
            if !(0x30..=0x39).contains(&c) {
                break;
            }
            v = v.checked_mul(10)?.checked_add(c - 0x30)?;
            self.i += 1;
        }
        (self.i > start).then_some(v)
    }

    fn atom(&mut self) -> Result<Node, RegexError> {
        let c = self.peek().expect("an atom starts at a unit");
        self.i += 1;
        match char::from_u32(c) {
            Some('.') => Ok(Node::Any),
            Some('(') => {
                if self.peek() == Some('?' as u32) {
                    if self.peek_at(1) == Some(':' as u32) {
                        self.i += 2;
                    } else {
                        return Err(self.err("a group modifier other than (?: (?= (?!"));
                    }
                }
                let node = self.disjunction()?;
                if !self.eat(')') {
                    return Err(self.err("a group is not closed"));
                }
                Ok(node)
            }
            Some(')') => Err(self.err("an unmatched )")),
            Some('[') => self.class(),
            Some('*') | Some('+') | Some('?') | Some('{') => Err(self.err("a quantifier with nothing to repeat")),
            Some('\\') => self.atom_escape(),
            _ => Ok(Node::Unit(c)),
        }
    }

    fn hex(&mut self, n: usize) -> Result<u32, RegexError> {
        let mut v = 0u32;
        for _ in 0..n {
            let d = self
                .peek()
                .and_then(char::from_u32)
                .and_then(|c| c.to_digit(16))
                .ok_or_else(|| self.err("a hex escape is short"))?;
            v = v * 16 + d;
            self.i += 1;
        }
        Ok(v)
    }

    /// The unit a character escape denotes, or the named class it is.
    fn escape(&mut self, in_class: bool) -> Result<Result<u32, (Named, bool)>, RegexError> {
        let c = self.peek().ok_or_else(|| self.err("a pattern ends in a backslash"))?;
        self.i += 1;
        Ok(match char::from_u32(c) {
            Some('d') => Err((Named::Digit, false)),
            Some('D') => Err((Named::Digit, true)),
            Some('s') => Err((Named::Space, false)),
            Some('S') => Err((Named::Space, true)),
            Some('w') => Err((Named::Word, false)),
            Some('W') => Err((Named::Word, true)),
            Some('f') => Ok(0x0C),
            Some('n') => Ok(0x0A),
            Some('r') => Ok(0x0D),
            Some('t') => Ok(0x09),
            Some('v') => Ok(0x0B),
            Some('b') if in_class => Ok(0x08),
            Some('0') => Ok(0),
            Some('x') => Ok(self.hex(2)?),
            Some('u') => Ok(self.hex(4)?),
            Some('c') => {
                let l = self
                    .peek()
                    .filter(|&l| (0x41..=0x5A).contains(&l) || (0x61..=0x7A).contains(&l))
                    .ok_or_else(|| self.err("a control escape is not followed by a letter"))?;
                self.i += 1;
                Ok(l % 32)
            }
            Some(d) if d.is_ascii_digit() => return Err(self.err("a back-reference")),
            _ => Ok(c),
        })
    }

    fn atom_escape(&mut self) -> Result<Node, RegexError> {
        Ok(match self.escape(false)? {
            Ok(u) => Node::Unit(u),
            Err((class, negate)) => {
                Node::Class(Class { negate: false, items: vec![ClassItem::Named { class, negate }] })
            }
        })
    }

    fn class(&mut self) -> Result<Node, RegexError> {
        let negate = self.eat('^');
        let mut items = Vec::new();
        if self.peek() == Some(']' as u32) {
            return Err(self.err("an empty bracket expression"));
        }
        loop {
            let c = self.peek().ok_or_else(|| self.err("a bracket expression is not closed"))?;
            if c == ']' as u32 {
                self.i += 1;
                break;
            }
            let lo = self.class_atom()?;
            let dash = self.peek() == Some('-' as u32) && self.peek_at(1).is_some_and(|n| n != ']' as u32);
            match lo {
                Ok(lo) if dash => {
                    self.i += 1;
                    let hi = match self.class_atom()? {
                        Ok(hi) => hi,
                        Err(_) => return Err(self.err("a range ends in a class escape")),
                    };
                    let (lo, hi) = (key(self.width, lo), key(self.width, hi));
                    if lo > hi {
                        return Err(self.err("a range whose start is above its end"));
                    }
                    items.push(ClassItem::Range(lo, hi));
                }
                Ok(u) => items.push(ClassItem::Unit(u)),
                Err((class, n)) => items.push(ClassItem::Named { class, negate: n }),
            }
        }
        Ok(Node::Class(Class { negate, items }))
    }

    fn class_atom(&mut self) -> Result<Result<u32, (Named, bool)>, RegexError> {
        let c = self.peek().expect("a class atom starts at a unit");
        self.i += 1;
        if c == '\\' as u32 {
            self.escape(true)
        } else {
            Ok(Ok(c))
        }
    }
}

#[derive(Clone, Debug)]
enum Inst {
    Unit(u32),
    Class(usize),
    Any,
    Split(usize, usize),
    Jmp(usize),
    Look(Look),
    Ahead { body: usize, negate: bool },
    Match,
}

/// A compiled pre-tokenizer pattern.
#[derive(Clone, Debug)]
pub struct Regex {
    insts: Vec<Inst>,
    classes: Vec<Class>,
    width: Width,
}

struct Compiler {
    insts: Vec<Inst>,
    classes: Vec<Class>,
    pending: Vec<(usize, Node)>,
}

impl Compiler {
    fn emit(&mut self, i: Inst) -> usize {
        self.insts.push(i);
        self.insts.len() - 1
    }

    fn node(&mut self, n: &Node) {
        match n {
            Node::Unit(u) => {
                self.emit(Inst::Unit(*u));
            }
            Node::Class(c) => {
                self.classes.push(c.clone());
                let id = self.classes.len() - 1;
                self.emit(Inst::Class(id));
            }
            Node::Any => {
                self.emit(Inst::Any);
            }
            Node::Concat(v) => v.iter().for_each(|n| self.node(n)),
            Node::Alt(v) => {
                let mut exits = Vec::new();
                for (k, alt) in v.iter().enumerate() {
                    if k + 1 < v.len() {
                        let split = self.emit(Inst::Split(0, 0));
                        self.node(alt);
                        exits.push(self.emit(Inst::Jmp(0)));
                        let next = self.insts.len();
                        self.insts[split] = Inst::Split(split + 1, next);
                    } else {
                        self.node(alt);
                    }
                }
                let end = self.insts.len();
                for e in exits {
                    self.insts[e] = Inst::Jmp(end);
                }
            }
            Node::Repeat { node, min, max, greedy } => {
                for _ in 0..*min {
                    self.node(node);
                }
                match max {
                    None => {
                        let split = self.emit(Inst::Split(0, 0));
                        self.node(node);
                        self.emit(Inst::Jmp(split));
                        let out = self.insts.len();
                        self.insts[split] = if *greedy { Inst::Split(split + 1, out) } else { Inst::Split(out, split + 1) };
                    }
                    Some(max) => {
                        let mut splits = Vec::new();
                        for _ in *min..*max {
                            splits.push(self.emit(Inst::Split(0, 0)));
                            self.node(node);
                        }
                        let out = self.insts.len();
                        for s in splits {
                            self.insts[s] = if *greedy { Inst::Split(s + 1, out) } else { Inst::Split(out, s + 1) };
                        }
                    }
                }
            }
            Node::Look(l) => {
                self.emit(Inst::Look(*l));
            }
            Node::Ahead { node, negate } => {
                let at = self.emit(Inst::Ahead { body: 0, negate: *negate });
                self.pending.push((at, (**node).clone()));
            }
        }
    }
}

/// How one search is anchored: where its attempt began and the `regex_constants` flags it runs with.
#[derive(Clone, Copy)]
struct Ctx {
    begin: usize,
    prev_avail: bool,
    not_null: bool,
}

impl Regex {
    /// Compile `pattern` for code units of `width`.
    pub fn new(pattern: &str, width: Width) -> Result<Regex, RegexError> {
        let units: Vec<u32> = pattern.chars().map(|c| c as u32).collect();
        Regex::from_units(&units, pattern, width)
    }

    /// Compile a pattern given as code units — the collapsed narrow patterns carry raw bytes ≥ 0x80.
    pub fn from_units(units: &[u32], pattern: &str, width: Width) -> Result<Regex, RegexError> {
        let mut p = Parser { p: units, i: 0, pattern, width };
        let node = p.disjunction()?;
        if p.i != units.len() {
            return Err(p.err("an unmatched )"));
        }
        let mut c = Compiler { insts: Vec::new(), classes: Vec::new(), pending: Vec::new() };
        c.node(&node);
        c.emit(Inst::Match);
        while let Some((at, body)) = c.pending.pop() {
            let start = c.insts.len();
            c.node(&body);
            c.emit(Inst::Match);
            if let Inst::Ahead { negate, .. } = c.insts[at] {
                c.insts[at] = Inst::Ahead { body: start, negate };
            }
        }
        Ok(Regex { insts: c.insts, classes: c.classes, width })
    }

    fn class_has(&self, id: usize, u: u32) -> bool {
        let c = &self.classes[id];
        let k = key(self.width, u);
        let hit = c.items.iter().any(|item| match *item {
            ClassItem::Unit(v) => v == u,
            ClassItem::Range(lo, hi) => lo <= k && k <= hi,
            ClassItem::Named { class, negate } => class.contains(u) != negate,
        });
        hit != c.negate
    }

    fn any(&self, u: u32) -> bool {
        match self.width {
            Width::Narrow => u != 0x0A && u != 0x0D,
            Width::Wide => !matches!(u, 0x0A | 0x0D | 0x2028 | 0x2029),
        }
    }

    fn look(&self, l: Look, text: &[u32], pos: usize, ctx: Ctx) -> bool {
        let word = |u: u32| Named::Word.contains(u);
        match l {
            Look::LineStart => pos == ctx.begin && !ctx.prev_avail,
            Look::LineEnd => pos == text.len(),
            Look::WordBoundary | Look::NotWordBoundary => {
                let left = (pos != ctx.begin || ctx.prev_avail) && pos > 0 && word(text[pos - 1]);
                let right = pos < text.len() && word(text[pos]);
                (left != right) == (l == Look::WordBoundary)
            }
        }
    }

    /// The end of the first match starting exactly at `pos` from instruction `pc`, in priority order.
    fn run(&self, pc: usize, pos: usize, text: &[u32], ctx: Ctx) -> Option<usize> {
        let mut stack = vec![(pc, pos)];
        while let Some((mut pc, mut pos)) = stack.pop() {
            loop {
                match self.insts[pc] {
                    Inst::Unit(u) => {
                        if pos < text.len() && text[pos] == u {
                            pc += 1;
                            pos += 1;
                        } else {
                            break;
                        }
                    }
                    Inst::Class(id) => {
                        if pos < text.len() && self.class_has(id, text[pos]) {
                            pc += 1;
                            pos += 1;
                        } else {
                            break;
                        }
                    }
                    Inst::Any => {
                        if pos < text.len() && self.any(text[pos]) {
                            pc += 1;
                            pos += 1;
                        } else {
                            break;
                        }
                    }
                    Inst::Split(a, b) => {
                        stack.push((b, pos));
                        pc = a;
                    }
                    Inst::Jmp(a) => pc = a,
                    Inst::Look(l) => {
                        if self.look(l, text, pos, ctx) {
                            pc += 1;
                        } else {
                            break;
                        }
                    }
                    Inst::Ahead { body, negate } => {
                        let sub = Ctx { begin: pos, ..ctx };
                        if self.run(body, pos, text, sub).is_some() != negate {
                            pc += 1;
                        } else {
                            break;
                        }
                    }
                    Inst::Match => {
                        if ctx.not_null && pos == ctx.begin {
                            break;
                        }
                        return Some(pos);
                    }
                }
            }
        }
        None
    }

    /// `regex_search` over `text[start..]`: the first start position with a match, and the match.
    fn search(&self, text: &[u32], start: usize, prev_avail: bool, continuous: bool, not_null: bool) -> Option<(usize, usize)> {
        let mut s = start;
        loop {
            let ctx = Ctx { begin: s, prev_avail: prev_avail || s > start, not_null };
            if let Some(e) = self.run(0, s, text, ctx) {
                return Some((s, e));
            }
            if continuous || s == text.len() {
                return None;
            }
            s += 1;
        }
    }

    /// Every match `std::regex_iterator` yields over `text`, as `(position, length)`.
    pub fn matches(&self, text: &[u32]) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut m = self.search(text, 0, false, false, false);
        while let Some((ms, me)) = m {
            out.push((ms, me - ms));
            let mut start = me;
            if ms == me {
                if me == text.len() {
                    break;
                }
                if let Some(next) = self.search(text, me, true, true, true) {
                    m = Some(next);
                    continue;
                }
                start += 1;
            }
            m = self.search(text, start, true, false, false);
        }
        out
    }
}
