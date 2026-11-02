//! Math mode's colouring, from the kernel's own vocabulary: its commands (`Definition`,
//! `Inductive`, `Check`, …), the literate forms (`## To …:`, `Consider`, `When`, `Yield`,
//! `## Theorem:` with `Statement:` and `Proof:`), the term grammar's binders and sorts
//! (`forall`, `fun`, `Prop`), and the proof tactics. A test holds these lists to the kernel's
//! parsers: every keyword they read is coloured, or named here as plain prose.

use super::code_editor::{Token, TokenKind};

/// The kernel's commands, the literate forms' structure words, and the term grammar's binders and
/// annotations (`@[binder …]`, `@[source …]`, `@[mdata …]`).
const KEYWORDS: &[&str] = &[
    "Definition", "Inductive", "Check", "Eval", "Theorem", "Lemma", "Axiom", "Hypothesis",
    "Variable", "Parameter", "Proof", "Qed", "Statement", "To", "Consider", "When", "Yield", "Let",
    "let", "Theory", "Main", "Note", "Attribute", "Apply", "Name", "hint", "given", "prove",
    "cites", "be", "yields", "fix", "forall", "fun", "exists", "match", "return", "with", "end",
    "in", "if", "then", "else", "as", "by", "binder", "source", "mdata",
];

/// The literate forms' connectives.
const CONNECTIVES: &[&str] = &["and", "or", "implies", "equals", "is", "either", "not"];

/// The proof tactics, coloured in a proof.
const TACTICS: &[&str] = &[
    "intro", "intros", "assumption", "auto", "decide", "split", "left", "right", "ring", "refl",
    "lia", "omega", "cc", "simp", "induction", "inversion", "rewrite", "apply", "exact",
    "destruct", "cases", "reflexivity", "symmetry", "unfold", "trivial", "contradiction",
    "constructor", "diagonalization",
];

/// The sorts.
const SORTS: &[&str] = &["Prop", "Type", "Set"];

/// Commands and headers whose next name is the one they declare.
const DECLARERS: &[&str] = &["Definition", "Inductive", "Theorem", "Lemma", "Axiom", "Hypothesis", "Variable", "Parameter", "To", "Theory"];

/// Operators, longest first.
const OPERATORS: &[&str] = &[
    "<->", "->", "=>", ":=", "<=", ">=", "<>", "/\\", "\\/", "::", "=", "<", ">", "+", "-", "*", "/", "|",
    ":", "~", "^", "%",
];

const PUNCTUATION: &[char] = &['(', ')', '[', ']', '{', '}', '.', ',', ';'];

/// `source` as the editor colours it, token by token; the tokens spell `source` back exactly.
pub(crate) fn tokens(source: &str) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::new();
    let mut at = 0;
    // Tactics are words of a proof: from `Proof` to the next blank line, header or command.
    let mut in_proof = false;
    // The name after `Definition`, `## To`, `## Theorem:` … is the one it declares: a type
    // after `Inductive`, a name to call or cite after the rest.
    let mut declares: Option<TokenKind> = None;
    let mut line_start = true;
    while at < source.len() {
        let rest = &source[at..];
        let first = rest.chars().next().expect("a token starts somewhere");
        let (len, kind) = if first.is_whitespace() {
            let len = rest.find(|c: char| !c.is_whitespace()).unwrap_or(rest.len());
            if rest[..len].matches('\n').count() > 1 {
                in_proof = false;
            }
            (len, TokenKind::Identifier)
        } else if rest.starts_with("--") {
            (rest.find('\n').unwrap_or(rest.len()), TokenKind::Comment)
        } else if rest.starts_with("##") {
            in_proof = false;
            (rest.find(|c: char| c != '#').unwrap_or(rest.len()), TokenKind::Punctuation)
        } else if first == '"' {
            (string_len(rest), TokenKind::String)
        } else if first.is_ascii_digit() {
            (rest.find(|c: char| !(c.is_ascii_digit() || c == '_')).unwrap_or(rest.len()), TokenKind::Number)
        } else if first.is_alphabetic() || first == '_' {
            let len = rest.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '\'')).unwrap_or(rest.len());
            let word = &rest[..len];
            if line_start && KEYWORDS.contains(&word) && word != "Proof" {
                in_proof = false;
            }
            let kind = if let Some(declared) = declares.filter(|_| !KEYWORDS.contains(&word)) {
                declares = None;
                declared
            } else if in_proof && TACTICS.contains(&word) {
                TokenKind::Builtin
            } else if KEYWORDS.contains(&word) {
                declares = DECLARERS.contains(&word).then_some(if word == "Inductive" { TokenKind::Type } else { TokenKind::Builtin });
                in_proof |= word == "Proof";
                TokenKind::Keyword
            } else if CONNECTIVES.contains(&word) {
                TokenKind::Operator
            } else if SORTS.contains(&word) || word.starts_with(char::is_uppercase) {
                TokenKind::Type
            } else {
                TokenKind::Identifier
            };
            (len, kind)
        } else if let Some(op) = OPERATORS.iter().find(|op| rest.starts_with(**op)) {
            (op.len(), TokenKind::Operator)
        } else if is_symbol(first) {
            (first.len_utf8(), TokenKind::Operator)
        } else if PUNCTUATION.contains(&first) {
            (first.len_utf8(), TokenKind::Punctuation)
        } else {
            (first.len_utf8(), TokenKind::Identifier)
        };
        let text = &rest[..len];
        line_start = if text.trim().is_empty() { line_start || text.contains('\n') } else { false };
        out.push(Token { text: rest[..len].to_string(), kind });
        at += len;
    }
    out
}

/// A logical or mathematical symbol: the arrows, the mathematical operators, and ¬ λ.
fn is_symbol(c: char) -> bool {
    matches!(c, '\u{2190}'..='\u{21FF}' | '\u{2200}'..='\u{22FF}' | '\u{27F0}'..='\u{27FF}' | '¬' | 'λ' | 'Π' | 'Σ')
}

/// The length of the string literal `rest` opens: to its closing quote, past escapes.
fn string_len(rest: &str) -> usize {
    let mut escaped = false;
    for (i, c) in rest.char_indices().skip(1) {
        match c {
            '"' if !escaped => return i + 1,
            '\\' if !escaped => escaped = true,
            _ => escaped = false,
        }
    }
    rest.len()
}

/// The spellings Logos Math Mono draws as symbols, and the characters that keep one from being
/// drawn — as `scripts/web/build-math-font.py` builds them (a test holds the two together). Only the
/// browser's caret uses them.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) const LIGATURE_WORDS: &[&str] = &["forall", "exists", "fun"];
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) const LIGATURE_OPERATORS: &[&str] = &["->", "=>", "<=", ">="];
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) const WORD_CHARS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_'";
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) const OPERATOR_CHARS: &str = "-<>=|:!~*+/\\";

/// The byte ranges of `text` the Math font draws as one symbol: a spelling with no word
/// character beside a word, and no operator character beside an operator.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn ligatures(text: &str) -> Vec<std::ops::Range<usize>> {
    let bytes = text.as_bytes();
    // Each spelling with the characters that would join it to its neighbours.
    let spellings = LIGATURE_WORDS
        .iter()
        .map(|s| (*s, WORD_CHARS))
        .chain(LIGATURE_OPERATORS.iter().map(|s| (*s, OPERATOR_CHARS)));
    let mut out = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let before = at.checked_sub(1).map(|i| bytes[i]);
        let found = spellings.clone().find(|(spelling, joiners)| {
            let joins = |b: u8| joiners.as_bytes().contains(&b);
            text[at..].starts_with(spelling)
                && !before.is_some_and(joins)
                && !bytes.get(at + spelling.len()).is_some_and(|b| joins(*b))
        });
        match found {
            Some((spelling, _)) => {
                out.push(at..at + spelling.len());
                at += spelling.len();
            }
            None => at += text[at..].chars().next().map_or(1, char::len_utf8),
        }
    }
    out
}

/// Which edge of a ligature a caret inside it moves to.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Snap {
    Nearest,
    Start,
    End,
}

/// A caret at byte `at` of `text`, moved out of any ligature it sits inside, toward `snap`: the
/// symbol is one character to the reader, so the caret never stands within its spelling.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn snap(text: &str, at: usize, snap: Snap) -> usize {
    match ligatures(text).into_iter().find(|r| r.start < at && at < r.end) {
        Some(r) => match snap {
            Snap::Start => r.start,
            Snap::End => r.end,
            Snap::Nearest if at - r.start < r.end - at => r.start,
            Snap::Nearest => r.end,
        },
        None => at,
    }
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/components/vernacular/tests.rs"]
mod tests;
