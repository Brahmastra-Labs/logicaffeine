//! How tightly an emitted Rust expression binds, read from its tokens, and the parentheses each place needs.
//!
//! The emitters write an operator's operands, a condition, an initializer or an argument as text they have already
//! produced. Parenthesizing all of it is always correct, and it is what rustc's `unused_parens` lint and every reader
//! object to; parenthesizing none of it is wrong wherever an operand binds looser than the operator over it.
//! [`binding_of`] reads an expression's outermost operator from its tokens at nesting depth zero — strings,
//! characters, brackets, braces, closures and turbofish generics accounted for — and the writers below put
//! parentheses exactly where Rust's grammar needs them:
//! - [`operand`]: an operand of a binary operator, a cast, a prefix operator or a method call;
//! - [`cast`]: a cast, whole;
//! - [`standalone`]: a whole expression in a place that delimits it (an initializer, an assignment's value, an
//!   argument, a returned value, an index);
//! - [`condition`]: the condition of an `if` or a `while`, where a brace at depth zero would open the body;
//! - [`tail`]: a returned value as the expression a function's body ends with.

use std::borrow::Cow;

/// How tightly an expression's outermost operator binds, loosest first (the Rust Reference's precedence table).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Binding {
    /// `return`, `break`, a closure, an assignment, a range, or a keyword-led `if`/`match`/`loop`/`while`/`for`:
    /// parenthesized under any operator.
    Loosest,
    /// `a || b`
    Or,
    /// `a && b`
    And,
    /// `==`, `!=`, `<`, `>`, `<=`, `>=` — non-associative: a comparison under a comparison is parenthesized.
    Compare,
    /// `a | b`
    BitOr,
    /// `a ^ b`
    BitXor,
    /// `a & b`
    BitAnd,
    /// `<<`, `>>`
    Shift,
    /// `+`, `-`
    Sum,
    /// `*`, `/`, `%`
    Product,
    /// `a as T`
    Cast,
    /// Prefix `-`, `!`, `*`, `&`, `&mut`.
    Prefix,
    /// A literal, a path, a call, a method call, a field, an index, a macro, a block, or a parenthesized expression.
    Atom,
}

/// One token of an expression, as far as binding is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Token<'s> {
    /// An identifier, a keyword or a literal.
    Word(&'s str),
    /// An operator or punctuation, longest match.
    Punct(&'static str),
    /// A bracketed group `( … )`, `[ … ]` or `{ … }`, whole; the opening character.
    Group(char),
}

const PUNCTS: &[&str] = &[
    "..=", "<<=", ">>=", "...", "::", "->", "=>", "==", "!=", "<=", ">=", "&&", "||", "<<", ">>", "+=", "-=", "*=",
    "/=", "%=", "^=", "&=", "|=", "..", "=", "<", ">", "+", "-", "*", "/", "%", "^", "&", "|", "!", ".", ",", ";",
    ":", "#", "?", "@", "$", "~",
];

/// The tokens of `code` at nesting depth zero: a bracketed group is one token. `None` when `code` is not balanced.
fn depth_zero_tokens(code: &str) -> Option<Vec<Token<'_>>> {
    let bytes = code.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && bytes.get(i + 1) == Some(&b'*') {
            let end = code[i + 2..].find("*/")?;
            i += 2 + end + 2;
            continue;
        }
        if matches!(c, b'(' | b'[' | b'{') {
            let close = matching(code, i)?;
            out.push(Token::Group(c as char));
            i = close + 1;
            continue;
        }
        if matches!(c, b')' | b']' | b'}') {
            return None;
        }
        if c == b'"' || (c == b'b' && bytes.get(i + 1) == Some(&b'"')) {
            let start = i;
            i = string_end(code, if c == b'b' { i + 1 } else { i })?;
            out.push(Token::Word(&code[start..i]));
            continue;
        }
        if c == b'r' && (bytes.get(i + 1) == Some(&b'"') || bytes.get(i + 1) == Some(&b'#')) {
            if let Some(end) = raw_string_end(code, i + 1) {
                out.push(Token::Word(&code[i..end]));
                i = end;
                continue;
            }
            // `r#loop`: a raw identifier, one word.
            if bytes.get(i + 1) == Some(&b'#') && bytes.get(i + 2).is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_') {
                let start = i;
                i += 2;
                while i < bytes.len() && (is_word_byte(bytes[i]) || bytes[i] >= 0x80) {
                    i += 1;
                }
                out.push(Token::Word(&code[start..i]));
                continue;
            }
        }
        if c == b'\'' {
            let start = i;
            i = char_or_lifetime_end(code, i)?;
            out.push(Token::Word(&code[start..i]));
            continue;
        }
        if c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80 {
            let start = i;
            while i < bytes.len() {
                let d = bytes[i];
                let float_point = d == b'.'
                    && bytes[start].is_ascii_digit()
                    && bytes.get(i + 1).is_some_and(|n| n.is_ascii_digit());
                let exponent_sign = matches!(d, b'+' | b'-')
                    && bytes[start].is_ascii_digit()
                    && matches!(bytes[i - 1], b'e' | b'E')
                    && !code[start..i].starts_with("0x");
                if d.is_ascii_alphanumeric() || d == b'_' || d >= 0x80 || float_point || exponent_sign {
                    i += 1;
                } else {
                    break;
                }
            }
            out.push(Token::Word(&code[start..i]));
            continue;
        }
        let p = PUNCTS.iter().find(|p| code[i..].starts_with(**p))?;
        out.push(Token::Punct(p));
        i += p.len();
    }
    Some(out)
}

/// The index of the bracket closing the one at `open`.
fn matching(code: &str, open: usize) -> Option<usize> {
    let bytes = code.as_bytes();
    let mut stack = vec![bytes[open]];
    let mut i = open + 1;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'(' | b'[' | b'{' => stack.push(c),
            b')' | b']' | b'}' => {
                let want = match stack.pop()? {
                    b'(' => b')',
                    b'[' => b']',
                    _ => b'}',
                };
                if c != want {
                    return None;
                }
                if stack.is_empty() {
                    return Some(i);
                }
            }
            b'"' => {
                i = string_end(code, i)?;
                continue;
            }
            b'r' if matches!(bytes.get(i + 1), Some(b'"') | Some(b'#'))
                && (i == 0 || !is_word_byte(bytes[i - 1])) =>
            {
                if let Some(end) = raw_string_end(code, i + 1) {
                    i = end;
                    continue;
                }
            }
            b'\'' => {
                i = char_or_lifetime_end(code, i)?;
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2 + code[i + 2..].find("*/")? + 2;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Past the closing quote of the string whose opening quote is at `open`.
fn string_end(code: &str, open: usize) -> Option<usize> {
    let bytes = code.as_bytes();
    let mut i = open + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

/// Past the raw string whose hashes or quote start at `at` (just after the `r`).
fn raw_string_end(code: &str, at: usize) -> Option<usize> {
    let hashes = code[at..].bytes().take_while(|b| *b == b'#').count();
    if code.as_bytes().get(at + hashes) != Some(&b'"') {
        return None;
    }
    let close = format!("\"{}", "#".repeat(hashes));
    let body = at + hashes + 1;
    Some(body + code[body..].find(&close)? + close.len())
}

/// Past a character literal (`'a'`, `'\''`, `'\n'`, `'\x7f'`, `'\u{1F600}'`) or a lifetime (`'a`) starting at
/// `at`.
fn char_or_lifetime_end(code: &str, at: usize) -> Option<usize> {
    let rest = &code[at + 1..];
    if let Some(escaped) = rest.strip_prefix('\\') {
        let body = if escaped.starts_with("u{") {
            escaped.find('}')? + 1
        } else if escaped.starts_with('x') {
            3
        } else {
            escaped.chars().next()?.len_utf8()
        };
        let close = at + 2 + body;
        return (code.as_bytes().get(close) == Some(&b'\'')).then_some(close + 1);
    }
    let first = rest.chars().next()?;
    let after = at + 1 + first.len_utf8();
    if code[after..].starts_with('\'') {
        return Some(after + 1);
    }
    let lifetime = rest
        .char_indices()
        .find(|(_, c)| !(c.is_alphanumeric() || *c == '_'))
        .map_or(rest.len(), |(i, _)| i);
    Some(at + 1 + lifetime)
}

const BLOCK_KEYWORDS: &[&str] = &["if", "match", "loop", "while", "for", "unsafe", "async", "const"];
const JUMP_KEYWORDS: &[&str] = &["return", "break", "continue", "yield", "move", "static"];

fn binary(p: &str) -> Option<Binding> {
    Some(match p {
        "||" => Binding::Or,
        "&&" => Binding::And,
        "==" | "!=" | "<" | ">" | "<=" | ">=" => Binding::Compare,
        "|" => Binding::BitOr,
        "^" => Binding::BitXor,
        "&" => Binding::BitAnd,
        "<<" | ">>" => Binding::Shift,
        "+" | "-" => Binding::Sum,
        "*" | "/" | "%" => Binding::Product,
        _ => return None,
    })
}

/// How tightly `code`, a whole Rust expression, binds: its loosest operator at depth zero.
pub(crate) fn binding_of(code: &str) -> Binding {
    let Some(tokens) = depth_zero_tokens(code) else {
        return Binding::Loosest;
    };
    let Some(first) = tokens.first() else {
        return Binding::Loosest;
    };
    match first {
        Token::Word(w) if BLOCK_KEYWORDS.contains(w) || JUMP_KEYWORDS.contains(w) => return Binding::Loosest,
        Token::Punct("|") | Token::Punct("||") => return Binding::Loosest,
        _ => {}
    }
    let mut loosest = Binding::Atom;
    // Whether the next token stands where an operand starts (so `-`, `*`, `&`, `!` are prefix there).
    let mut operand_expected = true;
    let mut generic_depth = 0usize;
    let mut i = 0;
    while i < tokens.len() {
        let t = tokens[i];
        if generic_depth > 0 {
            match t {
                Token::Punct("<") => generic_depth += 1,
                Token::Punct(">") => generic_depth -= 1,
                Token::Punct(">>") => generic_depth = generic_depth.saturating_sub(2),
                _ => {}
            }
            i += 1;
            continue;
        }
        match t {
            Token::Punct("::") => {
                if tokens.get(i + 1) == Some(&Token::Punct("<")) {
                    generic_depth = 1;
                    i += 2;
                    continue;
                }
                operand_expected = true;
            }
            Token::Punct("=" | "+=" | "-=" | "*=" | "/=" | "%=" | "^=" | "&=" | "|=" | "<<=" | ">>=" | ".." | "..=") => {
                return Binding::Loosest;
            }
            Token::Punct("?") | Token::Punct(".") => {
                operand_expected = matches!(t, Token::Punct("."));
            }
            // `name!( … )`: a macro call, whole.
            Token::Punct("!") if !operand_expected => {}
            Token::Punct(p) if operand_expected => {
                if matches!(p, "-" | "!" | "*" | "&" | "&&") {
                    loosest = loosest.min(Binding::Prefix);
                } else if p == "|" || p == "||" {
                    return Binding::Loosest;
                }
            }
            Token::Punct(p) => match binary(p) {
                Some(b) => {
                    loosest = loosest.min(b);
                    operand_expected = true;
                }
                None => return Binding::Loosest,
            },
            Token::Word("as") => {
                loosest = loosest.min(Binding::Cast);
                // The type after `as`: a path, maybe with generics.
                i += 1;
                while let Some(Token::Word(w)) = tokens.get(i) {
                    if *w == "as" {
                        break;
                    }
                    i += 1;
                    match tokens.get(i) {
                        Some(Token::Punct("::")) => i += 1,
                        Some(Token::Punct("<")) => {
                            generic_depth = 1;
                            i += 1;
                            break;
                        }
                        _ => break,
                    }
                }
                operand_expected = false;
                continue;
            }
            Token::Word("mut") if operand_expected => {}
            Token::Word(w) if !operand_expected && (BLOCK_KEYWORDS.contains(&w) || JUMP_KEYWORDS.contains(&w)) => {
                return Binding::Loosest;
            }
            Token::Word(_) => operand_expected = false,
            Token::Group(_) => operand_expected = false,
        }
        i += 1;
    }
    loosest
}

/// What an operand stands under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Under {
    /// A binary operator, by its Rust spelling (`"+"`, `"<<"`, `"&&"`, `"<="`, …), on its left or right.
    Binary(&'static str, Side),
    /// The left of `as`.
    Cast,
    /// A prefix operator (`-`, `!`, `*`, `&`).
    Prefix,
    /// The receiver of a method call, a field, an index, a call, or `?`.
    Postfix,
}

/// Which operand of a binary operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Left,
    Right,
}

/// Whether `code` ends with a cast's type: its last depth-zero `as` is followed by nothing but the type's path and
/// generic arguments.
fn ends_with_cast(code: &str) -> bool {
    let Some(tokens) = depth_zero_tokens(code) else {
        return false;
    };
    let Some(at) = tokens.iter().rposition(|t| *t == Token::Word("as")) else {
        return false;
    };
    tokens[at + 1..]
        .iter()
        .all(|t| matches!(t, Token::Word(_) | Token::Punct("::" | "<" | ">" | ">>" | ",")))
}

/// `code` as an operand: parenthesized exactly when Rust would read it otherwise — it binds looser than what it
/// stands under, or as tightly on the side the operator does not associate (the right of a left-associative
/// operator; either side of a comparison), or it ends with a cast's type and stands left of an operator beginning
/// `<`, which Rust would read as that type's generic arguments. Parentheses already around all of `code` are its
/// own only where it needs them.
pub(crate) fn operand(code: &str, under: Under) -> Cow<'_, str> {
    let code = unparenthesized(code);
    let own = binding_of(code);
    let needs = match under {
        Under::Binary(op, side) => {
            let parent = binary(op).unwrap_or(Binding::Loosest);
            let not_associating = parent == Binding::Compare || side == Side::Right;
            own < parent
                || (own == parent && not_associating)
                || (side == Side::Left && op.starts_with('<') && ends_with_cast(code))
        }
        Under::Cast => own < Binding::Cast,
        Under::Prefix => own < Binding::Prefix,
        Under::Postfix => own < Binding::Atom,
    };
    if needs {
        Cow::Owned(format!("({code})"))
    } else {
        Cow::Borrowed(code)
    }
}

/// `code` cast to `ty`, parenthesized whole as every operator the emitters write is: `(x as i64)`, `((a + b) as f64)`.
/// A numeric literal the cast leaves as it is is written as the literal of `ty` it is ([`literal_of`]).
pub(crate) fn cast(code: &str, ty: &str) -> String {
    if let Some(literal) = literal_of(unparenthesized(code), ty) {
        return literal;
    }
    format!("({} as {ty})", operand(code, Under::Cast))
}

/// `code`, a numeric literal, as the literal of the numeric type `ty` that `code as ty` is: one already of `ty` (`0f64`
/// for `f64`), or an unsuffixed integer `ty` holds exactly (`7` as `7u32`, `3` as `3f64`). `None` for anything a cast
/// changes or rounds (a negative or out-of-range integer, a float to an integer type, a float of another width) and
/// for anything that is not a literal.
fn literal_of(code: &str, ty: &str) -> Option<String> {
    const INTEGERS: [(&str, i128, i128); 12] = [
        ("i8", i8::MIN as i128, i8::MAX as i128),
        ("i16", i16::MIN as i128, i16::MAX as i128),
        ("i32", i32::MIN as i128, i32::MAX as i128),
        ("i64", i64::MIN as i128, i64::MAX as i128),
        ("i128", i128::MIN, i128::MAX),
        ("isize", i64::MIN as i128, i64::MAX as i128),
        ("u8", 0, u8::MAX as i128),
        ("u16", 0, u16::MAX as i128),
        ("u32", 0, u32::MAX as i128),
        ("u64", 0, u64::MAX as i128),
        ("u128", 0, i128::MAX),
        ("usize", 0, u64::MAX as i128),
    ];
    let digits = |d: &str| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit() || b == b'_') && d.as_bytes()[0].is_ascii_digit();
    if let Some(body) = code.strip_suffix(ty) {
        let body = body.trim_end_matches('_');
        let float_body = |d: &str| d.split_once('.').map_or(digits(d), |(i, f)| digits(i) && (f.is_empty() || digits(f)));
        let fits = if ty.starts_with('f') { float_body(body) } else { digits(body) };
        if fits {
            return Some(code.to_string());
        }
    }
    if !digits(code) {
        return None;
    }
    let value: i128 = code.replace('_', "").parse().ok()?;
    if let Some((name, lo, hi)) = INTEGERS.iter().find(|(name, _, _)| *name == ty) {
        return (value >= *lo && value <= *hi).then(|| format!("{code}{name}"));
    }
    // An integer an `f32`/`f64` holds exactly is written as that float.
    let exact = match ty {
        "f64" => value.abs() <= 1 << 53,
        "f32" => value.abs() <= 1 << 24,
        _ => false,
    };
    exact.then(|| format!("{code}{ty}"))
}

/// `code` without the parentheses that enclose all of it, when what they enclose is one balanced expression.
pub(crate) fn unparenthesized(code: &str) -> &str {
    let mut inner = code.trim();
    while inner.starts_with('(') && matching(inner, 0) == Some(inner.len() - 1) {
        let candidate = inner[1..inner.len() - 1].trim();
        // `(x,)` and `()` are a tuple and the unit value, not a parenthesized expression.
        if candidate.is_empty() || depth_zero_tokens(candidate).is_some_and(|t| t.contains(&Token::Punct(","))) {
            break;
        }
        inner = candidate;
    }
    inner
}

/// `code`, Rust written line by line, with each line that starts outside a string literal moved in by `by`: a block of
/// emitted code placed one level deeper. A line that begins inside a string (a multi-line literal an `Escape to Rust:`
/// block wrote) is the literal's own text and keeps it, and an empty line stays empty.
pub(crate) fn indented(code: &str, by: &str) -> String {
    let bytes = code.as_bytes();
    let mut out = String::with_capacity(code.len() + code.len() / 8);
    let (mut copied, mut i, mut line_start, mut in_comment) = (0, 0, true, false);
    while i < bytes.len() {
        if line_start {
            line_start = false;
            if bytes[i] != b'\n' {
                out.push_str(&code[copied..i]);
                out.push_str(by);
                copied = i;
            }
        }
        let c = bytes[i];
        let boundary = i == 0 || !is_word_byte(bytes[i - 1]);
        i = if c == b'\n' {
            line_start = true;
            i + 1
        } else if in_comment {
            if code[i..].starts_with("*/") {
                in_comment = false;
                i + 2
            } else {
                i + 1
            }
        } else if code[i..].starts_with("/*") {
            in_comment = true;
            i + 2
        } else if code[i..].starts_with("//") {
            code[i..].find('\n').map_or(bytes.len(), |n| i + n)
        } else if c == b'"' {
            string_end(code, i).unwrap_or(bytes.len())
        } else if c == b'r' && boundary && matches!(bytes.get(i + 1), Some(b'"') | Some(b'#')) {
            raw_string_end(code, i + 1).unwrap_or(i + 1)
        } else if c == b'b' && boundary && bytes.get(i + 1) == Some(&b'r') {
            raw_string_end(code, i + 2).unwrap_or(i + 1)
        } else if c == b'\'' {
            char_or_lifetime_end(code, i).unwrap_or(i + 1)
        } else {
            i + 1
        };
    }
    out.push_str(&code[copied..]);
    out
}

/// `code` in a place that delimits a whole expression: an initializer, an assignment's value, an argument, a
/// returned value or an index. No parentheses are needed there, nor the braces of a block that holds an expression
/// alone (`{ if c { a } else { b } }`).
pub(crate) fn standalone(code: &str) -> &str {
    let bare = unparenthesized(code);
    lone_expression(bare).map_or(bare, standalone)
}

/// The expression a block `{ … }` holds alone: no statement, item, attribute or label before it, and no block-like
/// expression (`if c { … }`, a `match`, a loop) that a word follows — that one is a statement of its own and the word
/// begins the next (`{ if b == 0 { return -1; } a / b }` is two). `None` for any other code.
fn lone_expression(code: &str) -> Option<&str> {
    let code = code.trim();
    if !code.starts_with('{') || matching(code, 0)? != code.len() - 1 {
        return None;
    }
    let inner = code[1..code.len() - 1].trim();
    let tokens = depth_zero_tokens(inner)?;
    let statement = |t: &Token| {
        matches!(t, Token::Punct(";" | "," | "#" | "'"))
            || matches!(t, Token::Word(w) if ITEM_KEYWORDS.contains(w) || w.starts_with('\''))
    };
    let ends_a_statement =
        tokens.windows(2).any(|w| matches!(w, [Token::Group('{'), Token::Word(next)] if !["else", "as"].contains(next)));
    (!tokens.is_empty() && !tokens.iter().any(statement) && !ends_a_statement).then_some(inner)
}

/// Keywords that begin a statement or an item inside a block, never an expression.
const ITEM_KEYWORDS: &[&str] = &[
    "let", "fn", "struct", "enum", "impl", "use", "mod", "const", "static", "type", "trait", "macro_rules", "extern",
];

/// `target = value` as a Rust statement's assignment: `target op= rest` where `value` is `target op rest` at its
/// outermost operator (`rest` binds tighter than `op`, so `a - b + c` stays an assignment of `a - b + c`).
pub(crate) fn assignment(target: &str, value: &str, operators: bool) -> String {
    let value = standalone(value);
    if !operators {
        return format!("{target} = {value}");
    }
    for op in ["+", "-", "*", "/", "%", "&", "|", "^", "<<", ">>"] {
        let rest = value
            .strip_prefix(target)
            .and_then(|r| r.strip_prefix(' '))
            .and_then(|r| r.strip_prefix(op))
            .and_then(|r| r.strip_prefix(' '));
        if let (Some(rest), Some(b)) = (rest, binary(op)) {
            if binding_of(rest) > b {
                return format!("{target} {op}= {}", standalone(rest));
            }
        }
    }
    format!("{target} = {value}")
}

/// `code` as the condition of an `if` or a `while`: bare, unless a brace at depth zero (a block, a struct literal, a
/// block-led expression) would be read as the body. An `unsafe { … }` block is read whole there.
pub(crate) fn condition(code: &str) -> Cow<'_, str> {
    let bare = unparenthesized(code);
    let braces = depth_zero_tokens(bare).is_none_or(|tokens| {
        tokens.iter().enumerate().any(|(i, t)| match t {
            Token::Word("unsafe") => false,
            Token::Group('{') => i == 0 || tokens[i - 1] != Token::Word("unsafe"),
            Token::Word(w) => BLOCK_KEYWORDS.contains(w),
            _ => false,
        })
    });
    if braces {
        Cow::Owned(format!("({bare})"))
    } else {
        Cow::Borrowed(bare)
    }
}

/// `code`, a returned value, as the expression a function's body ends with: `None` where it stays a `return`.
///
/// A body's final expression drops its temporaries after the body's locals (edition 2021), so a value that takes a
/// guard on a local (`f(&t.borrow())`: a `Ref` that borrows `t`) would hold its borrow past `t`'s end (E0597); it stays
/// `return f(&t.borrow());`. A value led by a block-like expression is parenthesized, since at the start of a
/// statement that expression ends at its last brace: `match v { … } + 1` is the statement `match v { … }` and then
/// `+ 1`.
pub(crate) fn tail(code: &str) -> Option<Cow<'_, str>> {
    let value = standalone(code);
    if GUARD_TAKING.iter().any(|g| value.contains(g)) {
        return None;
    }
    let tokens = depth_zero_tokens(value)?;
    match block_led_end(&tokens) {
        Some(end) if end < tokens.len() => Some(Cow::Owned(format!("({value})"))),
        _ => Some(Cow::Borrowed(value)),
    }
}

/// The methods emitted code calls whose result borrows the receiver and runs code when dropped.
const GUARD_TAKING: &[&str] = &[".borrow()", ".borrow_mut()", ".lock()", ".read()", ".write()", ".drain("];

/// Where the block-like expression that `tokens` start with ends (the index after its last brace group), when they
/// start with one: a block, a labeled expression, or one a [`BLOCK_KEYWORDS`] keyword leads. An `if` goes on through
/// each `else`.
fn block_led_end(tokens: &[Token<'_>]) -> Option<usize> {
    match tokens.first()? {
        Token::Group('{') => return Some(1),
        Token::Word(w) if BLOCK_KEYWORDS.contains(w) || w.starts_with('\'') => {}
        _ => return None,
    }
    let mut at = 0;
    loop {
        at += tokens[at..].iter().position(|t| *t == Token::Group('{'))? + 1;
        if tokens.get(at) != Some(&Token::Word("else")) {
            return Some(at);
        }
        at += 1;
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen/precedence/tests.rs"]
mod tests;
