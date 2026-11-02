//! The Verilog lexer the Studio's editor colours `.v` designs by: keywords and types, sized
//! literals (`4'd8`, `8'hFF`), `//` and `/* */` comments, strings, system tasks (`$display`),
//! compiler directives (`` `timescale ``), operators and punctuation — every character of the
//! source in exactly one token.

use super::code_editor::{Token, TokenKind};

/// Words that shape a design — its structure, control flow, ports and events — and the net and
/// variable types it declares with, all coloured as keywords.
const KEYWORDS: &[&str] = &[
    "always", "always_comb", "always_ff", "always_latch", "and", "assert", "assign", "assume",
    "automatic", "begin", "buf", "case", "casex", "casez", "cover", "deassign", "default",
    "defparam", "disable", "edge", "else", "end", "endcase", "endfunction", "endgenerate",
    "endmodule", "endprimitive", "endproperty", "endsequence", "endspecify", "endtable", "endtask",
    "enum", "event", "for", "force", "forever", "fork", "function", "generate", "genvar", "if",
    "initial", "inout", "input", "join", "localparam", "module", "nand", "negedge", "nor", "not",
    "or", "output", "parameter", "posedge", "primitive", "property", "release", "repeat",
    "sequence", "specify", "specparam", "struct", "table", "task", "typedef", "wait", "while",
    "xnor", "xor", "bit", "byte", "int", "integer", "logic", "longint", "packed", "real",
    "realtime", "reg", "shortint", "signed", "supply0", "supply1", "time", "tri", "tri0", "tri1",
    "triand", "trior", "unsigned", "wand", "wire", "wor",
];

/// Operators, longest first so each is read whole.
const OPERATORS: &[&str] = &[
    "<<<", ">>>", "===", "!==", "<=", ">=", "==", "!=", "&&", "||", "<<", ">>", "~&", "~|", "~^",
    "^~", "**", "->", "+:", "-:", "::", "+", "-", "*", "/", "%", "=", "<", ">", "!", "&", "|",
    "^", "~", "?", ":", "@", "#",
];

const PUNCTUATION: &[char] = &['(', ')', '[', ']', '{', '}', ';', ',', '.'];

/// `source` as the editor colours it, token by token; the tokens spell `source` back exactly.
pub(crate) fn tokens(source: &str) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::new();
    let mut at = 0;
    while at < source.len() {
        let rest = &source[at..];
        let (len, kind) = next_token(rest);
        out.push(Token { text: rest[..len].to_string(), kind });
        at += len;
    }
    out
}

/// The length in bytes and kind of the token `rest` starts with.
fn next_token(rest: &str) -> (usize, TokenKind) {
    let first = rest.chars().next().expect("a token starts somewhere");
    let word = |from: usize| from + rest[from..].find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$')).unwrap_or(rest.len() - from);
    if first.is_whitespace() {
        return (rest.find(|c: char| !c.is_whitespace()).unwrap_or(rest.len()), TokenKind::Identifier);
    }
    if rest.starts_with("//") {
        return (rest.find('\n').unwrap_or(rest.len()), TokenKind::Comment);
    }
    if rest.starts_with("/*") {
        return (rest[2..].find("*/").map_or(rest.len(), |i| i + 4), TokenKind::Comment);
    }
    if first == '"' {
        return (string_len(rest), TokenKind::String);
    }
    if (first == '$' || first == '`') && rest[1..].starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        return (word(1), TokenKind::Builtin);
    }
    if first == '\\' {
        return (rest.find(char::is_whitespace).unwrap_or(rest.len()), TokenKind::Identifier);
    }
    if let Some(len) = number_len(rest) {
        return (len, TokenKind::Number);
    }
    if first.is_ascii_alphabetic() || first == '_' {
        let len = word(0);
        let text = &rest[..len];
        let kind = if KEYWORDS.contains(&text) { TokenKind::Keyword } else { TokenKind::Identifier };
        return (len, kind);
    }
    if let Some(op) = OPERATORS.iter().find(|op| rest.starts_with(**op)) {
        return (op.len(), TokenKind::Operator);
    }
    let len = first.len_utf8();
    if PUNCTUATION.contains(&first) {
        (len, TokenKind::Punctuation)
    } else {
        (len, TokenKind::Identifier)
    }
}

/// The length of the string literal `rest` opens: to its closing quote, past escapes, or to the
/// end of its line if it has none.
fn string_len(rest: &str) -> usize {
    let mut escaped = false;
    for (i, c) in rest.char_indices().skip(1) {
        match c {
            '\n' => return i,
            '"' if !escaped => return i + 1,
            '\\' if !escaped => escaped = true,
            _ => escaped = false,
        }
    }
    rest.len()
}

/// The length of the number `rest` opens, if it opens one: a decimal (`12`, `1.5e3`), a sized
/// or unsized based literal (`4'd8`, `'hFF`, `32'sd5`), or an unbased one (`'0`).
fn number_len(rest: &str) -> Option<usize> {
    let bytes = rest.as_bytes();
    let run = |from: usize, ok: &dyn Fn(u8) -> bool| from + bytes[from..].iter().take_while(|b| ok(**b)).count();
    let decimal = |b: u8| b.is_ascii_digit() || b == b'_';
    let mut at = run(0, &decimal);
    if bytes.get(at) == Some(&b'\'') {
        let mut base = at + 1;
        if matches!(bytes.get(base), Some(b's' | b'S')) {
            base += 1;
        }
        if matches!(bytes.get(base), Some(b'b' | b'B' | b'o' | b'O' | b'd' | b'D' | b'h' | b'H')) {
            let value = |b: u8| b.is_ascii_hexdigit() || matches!(b, b'_' | b'x' | b'X' | b'z' | b'Z' | b'?');
            return Some(run(base + 1, &value));
        }
        if at == 0 && matches!(bytes.get(1), Some(b'0' | b'1' | b'x' | b'X' | b'z' | b'Z')) {
            return Some(2);
        }
    }
    if at == 0 {
        return None;
    }
    if bytes.get(at) == Some(&b'.') && bytes.get(at + 1).is_some_and(u8::is_ascii_digit) {
        at = run(at + 1, &decimal);
    }
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        let sign = at + 1 + usize::from(matches!(bytes.get(at + 1), Some(b'+' | b'-')));
        if bytes.get(sign).is_some_and(u8::is_ascii_digit) {
            at = run(sign, &decimal);
        }
    }
    Some(at)
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/components/verilog/tests.rs"]
mod tests;
