//! Syntax-highlighted code editor component.
//!
//! Provides a textarea with real-time syntax highlighting overlay. Uses a
//! transparent textarea layered over a highlighted `pre` element for editing.
//!
//! # Components
//!
//! - [`CodeEditor`] - Editable code editor with syntax highlighting
//! - [`CodeView`] - Read-only syntax highlighted display
//!
//! # Languages
//!
//! - **Logos**: Imperative `.logos` files (zones, parallel, etc.)
//! - **Vernacular**: Math/theorem mode (Definition, Check, Eval)
//! - **Rust**: Generated Rust output
//!
//! # Props (CodeEditor)
//!
//! - `value` - Current code content
//! - `on_change` - Callback when content changes
//! - `language` - Syntax highlighting mode
//! - `placeholder` - Optional placeholder text
//! - `readonly` - Whether editing is disabled

use dioxus::prelude::*;
use logicaffeine_lsp::document::DocumentState;

#[cfg(target_arch = "wasm32")]
use super::hover_hint::{hover_markdown, markdown_to_html};
#[cfg(target_arch = "wasm32")]
use super::indent::Indentation;
#[cfg(target_arch = "wasm32")]
use super::problems::problems_html;
use super::problems::{problems, rebase, squiggle_class, Problem};
use super::status_bar::{Caret, EditorStatus, ProblemCount};
#[cfg(target_arch = "wasm32")]
use super::vernacular::Snap;

/// Language mode for syntax highlighting.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    #[default]
    Logos, // Imperative .logos files
    Vernacular, // Math/theorem mode (Definition, Check, Eval)
    Rust,       // Generated Rust output
    Hardware,   // Hardware mode: English specs and Verilog
    Logic,      // Logic mode: English sentences, theorems and proofs
}

impl Language {
    /// The language's name as a status bar shows it; Hardware names Verilog apart from the
    /// English specs by what `source` is.
    pub fn name(self, source: &str) -> &'static str {
        use crate::ui::pages::hardware_report::is_verilog;
        match self {
            Language::Logos => "LOGOS",
            Language::Logic => "Logic",
            Language::Vernacular => "Vernacular",
            Language::Rust => "Rust",
            Language::Hardware if is_verilog(source) => "Verilog",
            Language::Hardware => "Hardware",
        }
    }

    /// The class the editor's root carries for this language (`lang-vernacular`, …).
    pub fn css_class(self) -> &'static str {
        match self {
            Language::Logos => "logos",
            Language::Logic => "logic",
            Language::Vernacular => "vernacular",
            Language::Rust => "rust",
            Language::Hardware => "hardware",
        }
    }
}

/// Token type for syntax highlighting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TokenKind {
    Keyword,
    Type,
    String,
    Number,
    Comment,
    Operator,
    Punctuation,
    Identifier,
    Builtin,
}

/// A highlighted token with its text and kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Token {
    pub(crate) text: String,
    pub(crate) kind: TokenKind,
}

/// The editor's stylesheet: inlined ahead of each editor, unless its host places it once ahead of
/// its own markup (see [`CodeEditor`]'s `stylesheet`).
pub const CODE_EDITOR_STYLE: &str = r#"
.code-editor {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--studio-surface, #0f1419);
    font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
}

/* Math sets its text in Logos Math Mono, whose ligatures draw the kernel's ASCII spellings as the
   symbols they stand for (forall as ∀, fun as λ, -> as →). Both layers shape the same glyphs, so
   the caret, selection and clicks stay on the text; the file stays ASCII. */
@font-face {
    font-family: 'Logos Math Mono';
    src: url('/assets/fonts/LogosMathMono.woff2') format('woff2');
    font-display: swap;
}
.code-editor.lang-vernacular .code-editor-textarea,
.code-editor.lang-vernacular .code-editor-highlight {
    font-family: 'Logos Math Mono', 'SF Mono', 'Fira Code', 'Consolas', monospace;
    font-variant-ligatures: contextual;
}

.code-editor-input {
    position: relative;
    flex: 1;
    min-height: 200px;
}

.code-editor-textarea {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    /* The host's inset, and room to scroll the last line up the editor — which the Studio
       declines, ending every column's scroll at its last line. */
    padding: var(--studio-inset, 16px);
    padding-bottom: var(--editor-overscroll, 50%);
    margin: 0;
    font-size: 14px;
    font-family: inherit;
    line-height: 1.6;
    tab-size: 4;
    background: transparent;
    color: transparent;
    caret-color: #667eea;
    border: none;
    outline: none;
    resize: none;
    white-space: pre-wrap;
    word-wrap: break-word;
    overflow-wrap: break-word;
    overflow: auto;
    z-index: 2;
    box-sizing: border-box;
}

.code-editor-highlight {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    /* The host's inset, and room to scroll the last line up the editor — which the Studio
       declines, ending every column's scroll at its last line. */
    padding: var(--studio-inset, 16px);
    padding-bottom: var(--editor-overscroll, 50%);
    margin: 0;
    font-size: 14px;
    font-family: inherit;
    line-height: 1.6;
    tab-size: 4;
    color: #e8eaed;
    white-space: pre-wrap;
    word-wrap: break-word;
    overflow-wrap: break-word;
    overflow: auto;
    pointer-events: none;
    z-index: 1;
    box-sizing: border-box;
    /* Hide scrollbar but keep scroll functionality for sync */
    scrollbar-width: none;
    -ms-overflow-style: none;
}

.code-editor-highlight::-webkit-scrollbar {
    display: none;
}

/* Syntax highlighting colors - no font-weight/style changes to keep heights identical */
.tok-keyword { color: #c678dd; }
.tok-type { color: #e5c07b; }
.tok-string { color: #98c379; }
.tok-number { color: #d19a66; }
.tok-comment { color: var(--studio-text-secondary, #9ca3af); }
.tok-operator { color: #56b6c2; }
.tok-punctuation { color: #abb2bf; }
.tok-identifier { color: #e8eaed; }
.tok-builtin { color: #61afef; }

/* A hover hint in VS Code's manner: a quiet panel over the code, never in the pointer's way.
   Its parts (a problem and its source, code, prose, lists, rules) sit one even gap apart, and
   nothing inside adds space of its own. */
.hover-hint {
    position: fixed;
    z-index: 1000;
    max-width: 520px;
    max-height: 40vh;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 4px 8px;
    background: #252526;
    border: 1px solid #454545;
    border-radius: 4px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.45);
    color: #cccccc;
    font: 13px/1.4 -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
}
/* Problems, VS Code's way: a wavy underline in the severity's colour; unused code faded. */
.sq-error, .sq-warning, .sq-info {
    text-decoration-line: underline;
    text-decoration-style: wavy;
    text-decoration-skip-ink: none;
    text-underline-offset: 3px;
}
.sq-error { text-decoration-color: #f14c4c; }
.sq-warning { text-decoration-color: #cca700; }
.sq-info { text-decoration-color: #3794ff; }
.sq-hint {
    text-decoration: underline dotted rgba(238, 238, 238, 0.55);
    text-underline-offset: 3px;
}
.sq-unnecessary { opacity: 0.6; }

.hover-hint .hint-problem { display: flex; flex-direction: column; gap: 4px; }
.hover-hint > *, .hover-hint .hint-problem > * { flex: none; margin: 0; }
.hover-hint .hint-source { color: #9ca3af; font-size: 12px; }
.hover-hint a { color: #3794ff; text-decoration: none; }
.hover-hint a:hover { text-decoration: underline; }
.hover-hint hr { border: none; border-top: 1px solid #454545; margin: 0 -8px; }
.hover-hint strong { color: #e8eaed; }
.hover-hint code { font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace; font-size: 12px; background: rgba(255, 255, 255, 0.06); padding: 0 3px; border-radius: 3px; }
.hover-hint pre { padding: 4px 8px; background: #1e1e1e; border-radius: 3px; overflow-x: auto; }
.hover-hint pre code { background: none; padding: 0; }
.hover-hint ul { padding-left: 18px; }

/* Mobile optimizations */
@media (max-width: 768px) {
    .code-editor-textarea,
    .code-editor-highlight {
        font-size: 16px; /* Prevent iOS zoom */
        padding: var(--studio-inset, 12px);
    }
}
"#;

/// The byte offset of every word inside a comparison written in English (`is less than`,
/// `greater than or equal to`, `at least`, `is equal to`, `equals`), read by the language's own
/// phrase reader (the one its hover teaches from), so the editor colours the words as operators.
fn comparison_word_starts(source: &str, lexed: &[logicaffeine_language::token::Token]) -> std::collections::HashSet<usize> {
    let mut starts = std::collections::HashSet::new();
    for at in 0..lexed.len() {
        if let Some((_, span)) = logicaffeine_language::teach::doc_for_phrase(source, lexed, at) {
            let mut in_word = false;
            for (i, c) in source[span.start..span.end].char_indices() {
                let word_char = c.is_alphanumeric() || c == '_';
                if word_char && !in_word {
                    starts.insert(span.start + i);
                }
                in_word = word_char;
            }
        }
    }
    starts
}

/// The byte ranges of a Code file that are prose, not code: the literate doc before the first
/// `## ` header (what `teach::module_doc` reads) and the body of every `## Note` block. The editor
/// colours them as comments and the language server's hover leaves them alone (the user,
/// 2026-11-01: comments in Web mode showed hints). A file with no header at all is all code.
fn prose_spans(source: &str) -> Vec<std::ops::Range<usize>> {
    let mut spans = Vec::new();
    let headers: Vec<usize> = source
        .match_indices("## ")
        .map(|(i, _)| i)
        .filter(|&i| i == 0 || source.as_bytes()[i - 1] == b'\n')
        .collect();
    let Some(&first) = headers.first() else { return spans };
    if first > 0 {
        spans.push(0..first);
    }
    for (n, &at) in headers.iter().enumerate() {
        let line_end = source[at..].find('\n').map_or(source.len(), |i| at + i);
        let title = source[at + 3..line_end].trim();
        if title.eq_ignore_ascii_case("Note") || title.to_ascii_lowercase().starts_with("note:") {
            let body_end = headers.get(n + 1).copied().unwrap_or(source.len());
            if line_end < body_end {
                spans.push(line_end + 1..body_end);
            }
        }
    }
    spans
}

/// Whether the byte at `offset` sits in text the editor colours as a comment (a `#` or `--` note,
/// or Code prose): nothing there is a word the language server should explain.
pub(crate) fn comment_at(source: &str, language: Language, offset: usize) -> bool {
    let mut at = 0;
    for token in tokenize(source, language) {
        let end = at + token.text.len();
        if (at..end).contains(&offset) {
            return token.kind == TokenKind::Comment;
        }
        at = end;
    }
    false
}

/// A LOGOS token the editor colours as a keyword: a statement word of the language, or a word
/// it teaches as a keyword (`If` classes as a connective but is taught as a statement).
fn is_logos_keyword(kind: &logicaffeine_language::token::TokenType) -> bool {
    logicaffeine_language::token_class::is_imperative_keyword(kind)
        || logicaffeine_language::teach::doc_for(kind).is_some()
}

/// A Hardware spec's field (`pigeons`, `registers`), named as its parser reads it.
fn is_spec_field(word: &str) -> bool {
    use crate::ui::pages::hardware_spec::{is_field, FIELDS};
    FIELDS.iter().any(|field| is_field(word, field))
}

/// `source` cut into the tokens the editor colours; Verilog and Math through their own lexers.
fn tokenize(source: &str, language: Language) -> Vec<Token> {
    if language == Language::Hardware && crate::ui::pages::hardware_report::is_verilog(source) {
        return super::verilog::tokens(source);
    }
    if language == Language::Vernacular {
        return super::vernacular::tokens(source);
    }
    let mut tokens = Vec::new();
    let chars: Vec<char> = source.chars().collect();
    let len = chars.len();
    let mut i = 0;

    let keywords = match language {
        Language::Logos => &[
            "let",
            "set",
            "if",
            "then",
            "else",
            "while",
            "repeat",
            "for",
            "in",
            "return",
            "match",
            "with",
            "end",
            "function",
            "struct",
            "enum",
            "show",
            "read",
            "write",
            "push",
            "pop",
            "add",
            "remove",
            "and",
            "or",
            "not",
            "true",
            "false",
            "nothing",
            "zone",
            "concurrent",
            "parallel",
            "launch",
            "send",
            "receive",
            "select",
            "mount",
            "sync",
            "merge",
            "increase",
            "decrease",
        ][..],
        Language::Vernacular => &[
            "Definition",
            "Check",
            "Eval",
            "Inductive",
            "Theorem",
            "Lemma",
            "Proof",
            "Qed",
            "Axiom",
            "Hypothesis",
            "Variable",
            "Parameter",
            "fun",
            "forall",
            "exists",
            "match",
            "with",
            "end",
            "let",
            "in",
            "Prop",
            "Type",
            "Set",
            "Nat",
            "Bool",
            "True",
            "False",
            "Not",
            "And",
            "Or",
            "Eq",
            "Ex",
            "All",
        ][..],
        // English specs ("Always, if request is high, then …", signal plans) and Verilog.
        // Theorem structure. Tactics join them on a `Proof:` line (see `in_proof` below).
        Language::Logic => &[
            "Theorem",
            "Lemma",
            "Definition",
            "Axiom",
            "Given",
            "Prove",
            "Proof",
            "Auto",
        ][..],
        Language::Hardware => &[
            "Always",
            "always",
            "If",
            "if",
            "then",
            "else",
            "is",
            "are",
            "Not",
            "not",
            "both",
            "and",
            "or",
            "only",
            "Never",
            "never",
            "Eventually",
            "eventually",
            "until",
            "within",
            "next",
            "cycle",
            "cycles",
            "high",
            "low",
            "Movements",
            "conflicts",
            "with",
            "module",
            "endmodule",
            "input",
            "output",
            "inout",
            "reg",
            "wire",
            "logic",
            "assign",
            "posedge",
            "negedge",
            "begin",
            "end",
            "initial",
            "case",
            "endcase",
            "assert",
            "property",
            "parameter",
            "localparam",
        ][..],
        Language::Rust => &[
            "fn", "let", "mut", "const", "static", "if", "else", "match", "loop", "while", "for",
            "in", "return", "break", "continue", "struct", "enum", "impl", "trait", "type", "pub",
            "mod", "use", "crate", "self", "super", "async", "await", "move", "ref", "where",
            "true", "false",
        ][..],
    };

    let types = match language {
        Language::Logos => &[
            "Int",
            "Text",
            "Bool",
            "Real",
            "Char",
            "Byte",
            "Nothing",
            "Seq",
            "Map",
            "Set",
            "Option",
            "Result",
            "Tuple",
            "Persistent",
            "Distributed",
        ][..],
        Language::Vernacular => &[
            "Prop",
            "Type",
            "Set",
            "Nat",
            "Bool",
            "Int",
            "List",
            "Option",
            "Syntax",
            "Derivation",
            "Term",
        ][..],
        // Modal auxiliaries: necessity and possibility.
        Language::Logic => &[
            "must", "Must", "can", "Can", "cannot", "Cannot", "might", "Might", "may", "May",
            "should", "Should", "would", "Would", "could", "Could",
        ][..],
        Language::Hardware => &[][..],
        Language::Rust => &[
            "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
            "f32", "f64", "bool", "char", "str", "String", "Vec", "HashMap", "Option", "Result",
            "Box", "Rc", "Arc",
        ][..],
    };

    let builtins = match language {
        Language::Logos => &[
            "show", "length", "first", "last", "rest", "append", "contains", "keys", "values",
            "range",
        ][..],
        Language::Vernacular => &[
            "Zero",
            "Succ",
            "Nil",
            "Cons",
            "Some",
            "None",
            "refl",
            "eq_ind",
            "nat_ind",
            "syn_diag",
            "syn_quote",
            "syn_size",
            "concludes",
            "Provable",
            "Consistent",
        ][..],
        // Quantifiers.
        Language::Logic => &[
            "Every", "every", "All", "all", "Some", "some", "No", "no", "Each", "each", "Exactly",
            "exactly", "Few", "few", "Most", "most", "Many", "many", "Any", "any",
        ][..],
        Language::Hardware => &[][..],
        Language::Rust => &[
            "println", "print", "format", "vec", "panic", "assert", "Some", "None", "Ok", "Err",
        ][..],
    };

    // Logic's connectives read as operators.
    let connectives: &[&str] = match language {
        Language::Logic => &[
            "If", "if", "then", "Then", "and", "or", "not", "iff", "Neither", "neither", "nor",
            "unless", "Unless", "only",
        ],
        _ => &[],
    };
    // On a Logic `Proof:` line, the tactic engine's vocabulary is keywords too.
    let mut in_proof = false;
    // LOGOS words take the lexer's own reading: a word it reads as a statement word
    // (`Let`, `Otherwise`, `Give`, …) or a keyword the language teaches (`If`) colours as one at
    // whatever case it is written, and `Set of Int` — which the lexer reads as a type — does not.
    let (lexer_keywords, lexer_operators): (std::collections::HashSet<usize>, std::collections::HashSet<usize>) =
        if language == Language::Logos {
            let mut interner = logicaffeine_base::Interner::new();
            let lexed = logicaffeine_language::Lexer::new(source, &mut interner).tokenize();
            let keywords = lexed.iter().filter(|t| is_logos_keyword(&t.kind)).map(|t| t.span.start).collect();
            (keywords, comparison_word_starts(source, &lexed))
        } else {
            Default::default()
        };
    // The byte offset where each character starts: the lexer speaks in bytes.
    let byte_at: Vec<usize> = chars
        .iter()
        .scan(0usize, |at, c| {
            let here = *at;
            *at += c.len_utf8();
            Some(here)
        })
        .collect();
    let prose: Vec<std::ops::Range<usize>> = if language == Language::Logos { prose_spans(source) } else { Vec::new() };

    while i < len {
        let c = chars[i];

        // Skip whitespace (preserve it)
        if c.is_whitespace() {
            let start = i;
            while i < len && chars[i].is_whitespace() {
                i += 1;
            }
            if chars[start..i].contains(&'\n') {
                in_proof = false;
            }
            tokens.push(Token {
                text: chars[start..i].iter().collect(),
                kind: TokenKind::Identifier, // Whitespace uses default color
            });
            continue;
        }

        // Code prose (the literate doc before the first header, a `## Note` body) reads as a
        // comment, a line at a time.
        if language == Language::Logos && c != '\n' && prose.iter().any(|span| span.contains(&byte_at[i])) {
            let start = i;
            let span_end = prose.iter().find(|span| span.contains(&byte_at[i])).map(|span| span.end).unwrap_or(usize::MAX);
            while i < len && chars[i] != '\n' && byte_at[i] < span_end {
                i += 1;
            }
            tokens.push(Token {
                text: chars[start..i].iter().collect(),
                kind: TokenKind::Comment,
            });
            continue;
        }

        // LOGOS (Code and Logic alike): one `#` opens a comment to the end of the line, wherever
        // it stands; `##` opens a block header (`## Main`, `## Theorem: …`), whose words keep
        // their highlighting — exactly as the lexer reads them.
        if matches!(language, Language::Logos | Language::Logic) && c == '#' {
            let run = chars[i..].iter().take_while(|&&ch| ch == '#').count();
            let start = i;
            if run == 1 {
                while i < len && chars[i] != '\n' {
                    i += 1;
                }
                tokens.push(Token {
                    text: chars[start..i].iter().collect(),
                    kind: TokenKind::Comment,
                });
            } else {
                i += run;
                tokens.push(Token {
                    text: chars[start..i].iter().collect(),
                    kind: TokenKind::Punctuation,
                });
            }
            continue;
        }

        // A Hardware spec comments with a `#` run and a space (`# note`, `## Title`), as its
        // parsers read it; Verilog's `#5` and `#(10)` delays stay code.
        if language == Language::Hardware && c == '#' {
            let run = chars[i..].iter().take_while(|&&ch| ch == '#').count();
            if crate::ui::pages::hardware_spec::opens_comment(chars.get(i + run).copied()) {
                let start = i;
                while i < len && chars[i] != '\n' {
                    i += 1;
                }
                tokens.push(Token {
                    text: chars[start..i].iter().collect(),
                    kind: TokenKind::Comment,
                });
                continue;
            }
        }

        // Comments
        let comment_start = match language {
            Language::Logos | Language::Vernacular => "--",
            Language::Rust | Language::Hardware => "//",
            Language::Logic => "",
        };
        if !comment_start.is_empty() && i + comment_start.len() <= len {
            let slice: String = chars[i..i + comment_start.len()].iter().collect();
            if slice == comment_start {
                let start = i;
                while i < len && chars[i] != '\n' {
                    i += 1;
                }
                tokens.push(Token {
                    text: chars[start..i].iter().collect(),
                    kind: TokenKind::Comment,
                });
                continue;
            }
        }

        // Strings
        if c == '"' {
            let start = i;
            i += 1;
            while i < len && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < len {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if i < len {
                i += 1; // closing quote
            }
            tokens.push(Token {
                text: chars[start..i].iter().collect(),
                kind: TokenKind::String,
            });
            continue;
        }

        // Numbers
        if c.is_ascii_digit() {
            let start = i;
            while i < len && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == '_') {
                i += 1;
            }
            tokens.push(Token {
                text: chars[start..i].iter().collect(),
                kind: TokenKind::Number,
            });
            continue;
        }

        // Identifiers and keywords
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < len && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let kind = if lexer_operators.contains(&byte_at[start]) {
                // `is less than`, `at least`, `is equal to`: the comparison the words spell, coloured
                // as `<`, `>=` and `==` are.
                TokenKind::Operator
            } else if keywords.contains(&word.as_str())
                || lexer_keywords.contains(&byte_at[start])
            {
                TokenKind::Keyword
            } else if language == Language::Hardware
                && chars.get(i) == Some(&':')
                && is_spec_field(&word)
            {
                TokenKind::Keyword
            } else if in_proof && logicaffeine_proof::tactic_script::canonical(&word).is_some() {
                TokenKind::Keyword
            } else if types.contains(&word.as_str()) {
                TokenKind::Type
            } else if builtins.contains(&word.as_str()) {
                TokenKind::Builtin
            } else if connectives.contains(&word.as_str()) {
                TokenKind::Operator
            } else {
                TokenKind::Identifier
            };
            if language == Language::Logic && word == "Proof" {
                in_proof = true;
            }
            tokens.push(Token { text: word, kind });
            continue;
        }

        // Operators
        let operators = [
            "->", "=>", ":=", "<=", ">=", "==", "!=", "&&", "||", "+", "-", "*", "/", "%", "=",
            "<", ">", "!", "&", "|", "^",
        ];
        let mut matched = false;
        for op in operators {
            if i + op.len() <= len {
                let slice: String = chars[i..i + op.len()].iter().collect();
                if slice == op {
                    tokens.push(Token {
                        text: slice,
                        kind: TokenKind::Operator,
                    });
                    i += op.len();
                    matched = true;
                    break;
                }
            }
        }
        if matched {
            continue;
        }

        // Punctuation
        if "(){}[].,;:".contains(c) {
            tokens.push(Token {
                text: c.to_string(),
                kind: TokenKind::Punctuation,
            });
            i += 1;
            continue;
        }

        // Default: single character
        tokens.push(Token {
            text: c.to_string(),
            kind: TokenKind::Identifier,
        });
        i += 1;
    }

    if language == Language::Logic {
        join_counting_quantifiers(&mut tokens);
    }
    tokens
}

/// Logic's counting quantifiers read as one phrase, the way the lexer reads them as one
/// quantifier: "at least three", "at most 2", "exactly one", "more than two", "fewer than 3".
fn join_counting_quantifiers(tokens: &mut [Token]) {
    let words: Vec<usize> = tokens
        .iter()
        .enumerate()
        .filter(|(_, t)| !t.text.trim().is_empty())
        .map(|(i, _)| i)
        .collect();
    let text = |k: usize| tokens[words[k]].text.to_lowercase();
    let mut phrases = Vec::new();
    for w in 0..words.len() {
        let next = |k: usize| (w + k < words.len()).then(|| text(w + k));
        let count_at = match text(w).as_str() {
            "at" if matches!(next(1).as_deref(), Some("least" | "most")) => w + 2,
            "more" | "fewer" | "less" if next(1).as_deref() == Some("than") => w + 2,
            "exactly" => w + 1,
            _ => continue,
        };
        if count_at < words.len() && is_count(&text(count_at)) {
            phrases.push(w..=count_at);
        }
    }
    for phrase in phrases {
        for k in phrase {
            tokens[words[k]].kind = TokenKind::Builtin;
        }
    }
}

/// A count: digits, or a number word the lexicon reads as one.
fn is_count(word: &str) -> bool {
    (!word.is_empty() && word.chars().all(|c| c.is_ascii_digit()))
        || logicaffeine_language::lexicon::word_to_number(word).is_some()
}

/// Get CSS class for a token kind.
fn token_class(kind: TokenKind) -> &'static str {
    match kind {
        TokenKind::Keyword => "tok-keyword",
        TokenKind::Type => "tok-type",
        TokenKind::String => "tok-string",
        TokenKind::Number => "tok-number",
        TokenKind::Comment => "tok-comment",
        TokenKind::Operator => "tok-operator",
        TokenKind::Punctuation => "tok-punctuation",
        TokenKind::Identifier => "tok-identifier",
        TokenKind::Builtin => "tok-builtin",
    }
}

/// Syntax-highlighted code editor component.
#[component]
pub fn CodeEditor(
    value: String,
    on_change: EventHandler<String>,
    language: Language,
    #[props(default = "Enter code...".to_string())] placeholder: String,
    #[props(default = false)] readonly: bool,
    /// Run the language's service over the source — the LOGOS language server, or the Hardware
    /// specs' own (`hardware_spec`): a VS Code-style hover for the word under a resting pointer,
    /// and squiggles under the problems it finds.
    #[props(default = false)]
    language_server: bool,
    /// Inline [`CODE_EDITOR_STYLE`] ahead of the editor. A host that sets editors into its own
    /// markup places the stylesheet once, ahead of that markup, and turns this off.
    #[props(default = true)]
    stylesheet: bool,
    /// Where to tell a status bar where the caret is, what is selected, and how many problems
    /// the language service found.
    #[props(default)]
    status: Option<Signal<EditorStatus>>,
) -> Element {
    // Each token with its byte offset in the source, so a hover can ask about exactly it.
    let mut at = 0usize;
    let tokens: Vec<(Token, usize)> = tokenize(&value, language)
        .into_iter()
        .map(|t| {
            let o = at;
            at += t.text.len();
            (t, o)
        })
        .collect();
    let mut hint = use_signal(|| None::<HoverHint>);
    // Bumped to void a hover waiting to appear, and a hide waiting to happen.
    let hint_gen = use_signal(|| 0u64);
    let hide_gen = use_signal(|| 0u64);
    let mut hide_pending = use_signal(|| false);
    let mut hint_hovered = use_signal(|| false);
    let analysis =
        use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(None::<std::rc::Rc<DocumentState>>)));
    let source = value.clone();
    #[cfg(target_arch = "wasm32")]
    let source_for_tap = value.clone();

    // The problems the language server found, and the text it found them in. Re-analysed once
    // typing pauses; until then the last problems move with the text they sit under.
    let mut found = use_signal(|| (String::new(), Vec::<Problem>::new()));
    let analysis_gen = use_signal(|| 0u64);
    {
        let analysis = analysis.clone();
        let value = value.clone();
        use_effect(use_reactive!(|(value, language_server, language)| {
            let generation = bump(analysis_gen);
            if !language_server {
                if !found.peek().1.is_empty() {
                    found.set((String::new(), Vec::new()));
                }
                return;
            }
            let analysis = analysis.clone();
            #[cfg(target_arch = "wasm32")]
            spawn(async move {
                gloo_timers::future::TimeoutFuture::new(ANALYSIS_DELAY_MS).await;
                if *analysis_gen.peek() == generation {
                    let now = service_problems(language, &analysis, &value);
                    found.set((value, now));
                }
            });
            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = generation;
                let now = service_problems(language, &analysis, &value);
                found.set((value, now));
            }
        }));
    }
    let shown: Vec<Problem> = {
        let found = found.read();
        if found.0 == value {
            found.1.clone()
        } else {
            rebase(&found.1, &found.0, &value)
        }
    };

    // The status bar: the problem counts as the squiggles show them, and the caret, read from
    // the textarea whenever it may have moved.
    let counted = ProblemCount::of(&shown);
    use_effect(use_reactive!(|counted| {
        if let Some(mut status) = status {
            if status.peek().problems != counted {
                status.with_mut(|s| s.problems = counted);
            }
        }
    }));
    let textarea_id = use_hook(|| CopyValue::new(next_textarea_id()));
    let dom_id = textarea_id.read().clone();
    let report_caret = move || {
        #[cfg(target_arch = "wasm32")]
        if let (Some(mut status), Some(caret)) = (status, caret_in(&textarea_id.read())) {
            if status.peek().caret != caret {
                status.with_mut(|s| s.caret = caret);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = (status, textarea_id);
    };
    // The last text this editor's own typing produced. Any other new value was loaded from
    // outside — a file opened, a template chosen — and opens with the caret at its start, the
    // way VS Code opens a file.
    let typed = use_hook(|| CopyValue::new(None::<String>));
    // Escape hands Tab back to the page, so the keyboard can leave the editor; any other key
    // takes it back.
    let tab_released = use_hook(|| CopyValue::new(false));
    use_effect(use_reactive!(|value| {
        if typed.peek().as_deref() != Some(value.as_str()) {
            #[cfg(target_arch = "wasm32")]
            caret_to_start(&textarea_id.read());
            if let Some(mut status) = status {
                if status.peek().caret != Caret::default() {
                    status.with_mut(|s| s.caret = Caret::default());
                }
            }
        }
    }));
    // The on-screen keyboard opening or closing moves the part of the editor the reader can see:
    // the caret's line stays in it.
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::prelude::*;
        use wasm_bindgen::JsCast;
        let on_resize = use_hook(move || {
            let on_resize = Closure::<dyn FnMut()>::new(move || keep_caret_in_view(&textarea_id.read()));
            if let Some(visual) = web_sys::window().and_then(|w| w.visual_viewport()) {
                let _ = visual.add_event_listener_with_callback("resize", on_resize.as_ref().unchecked_ref());
            }
            std::rc::Rc::new(on_resize)
        });
        use_drop(move || {
            if let Some(visual) = web_sys::window().and_then(|w| w.visual_viewport()) {
                let _ = visual.remove_event_listener_with_callback("resize", (*on_resize).as_ref().unchecked_ref());
            }
        });
    }
    let mut keep_hint = move || {
        bump(hide_gen);
        hide_pending.set(false);
    };
    // Hide now: typing, scrolling, or the pointer leaving the hint itself.
    let mut hide_hint = move || {
        if hint.peek().is_some() {
            hint.set(None);
        }
        hint_hovered.set(false);
        bump(hint_gen);
        keep_hint();
    };
    // Hide once the grace period runs out, unless the pointer comes back to the word or reaches
    // the hint first — the way VS Code lets you move into a hover to click its links.
    let mut release_hint = move || {
        bump(hint_gen);
        if hint.peek().is_none() || *hide_pending.peek() {
            return;
        }
        hide_pending.set(true);
        let generation = bump(hide_gen);
        #[cfg(target_arch = "wasm32")]
        spawn(async move {
            gloo_timers::future::TimeoutFuture::new(HIDE_GRACE_MS).await;
            if *hide_gen.peek() == generation {
                hide_pending.set(false);
                if !*hint_hovered.peek() {
                    hint.set(None);
                }
            }
        });
        #[cfg(not(target_arch = "wasm32"))]
        let _ = generation;
    };

    rsx! {
        if stylesheet {
            style { "{CODE_EDITOR_STYLE}" }
        }

        div { class: "code-editor lang-{language.css_class()}",
            div { class: "code-editor-input",
                // Highlighted overlay
                div { class: "code-editor-highlight",
                    for run in tokens.chunks(TOKEN_RUN) {
                        span {
                            for (token, offset) in run.iter() {
                                if token.text.chars().all(|c| c.is_whitespace()) {
                                    // Output whitespace as raw text to match textarea rendering
                                    "{token.text}"
                                } else {
                                    span {
                                        class: "{token_class(token.kind)} {squiggle_class(&shown, *offset, offset + token.text.len())}",
                                        "data-o": "{offset}",
                                        "{token.text}"
                                    }
                                }
                            }
                        }
                    }
                }

                // Actual textarea for input
                textarea {
                    class: "code-editor-textarea",
                    placeholder: "{placeholder}",
                    value: "{value}",
                    id: "{dom_id}",
                    readonly: readonly,
                    spellcheck: "false",
                    autocomplete: "off",
                    autocapitalize: "off",
                    onmousemove: move |evt| {
                        if !language_server {
                            return;
                        }
                        #[cfg(target_arch = "wasm32")]
                        {
                            let point = evt.data().client_coordinates();
                            let Some((offset, rect)) = token_under(point.x, point.y) else {
                                release_hint();
                                return;
                            };
                            if hint.peek().as_ref().is_some_and(|h| h.offset == offset) {
                                keep_hint();
                                return;
                            }
                            release_hint();
                            let generation = *hint_gen.peek();
                            let source = source.clone();
                            let analysis = analysis.clone();
                            spawn(async move {
                                gloo_timers::future::TimeoutFuture::new(HOVER_DELAY_MS).await;
                                if *hint_gen.peek() != generation {
                                    return;
                                }
                                // VS Code's order: the problems under the word, then its lesson.
                                let here = {
                                    let found = found.peek();
                                    if found.0 == source { found.1.clone() } else { rebase(&found.1, &found.0, &source) }
                                };
                                let problem = problems_html(&here, offset);
                                let lesson = service_hover(language, &analysis, &source, offset).map(|m| markdown_to_html(&m));
                                let html = match (problem, lesson) {
                                    (Some(p), Some(l)) => format!("{p}<hr>{l}"),
                                    (Some(only), None) | (None, Some(only)) => only,
                                    (None, None) => return,
                                };
                                if *hint_gen.peek() == generation {
                                    keep_hint();
                                    hint.set(Some(HoverHint { offset, html, style: hint_placement(&rect) }));
                                }
                            });
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        let _ = (evt, &source, &analysis);
                    },
                    onmouseleave: move |_| release_hint(),
                    onkeydown: move |evt: KeyboardEvent| {
                        hide_hint();
                        #[cfg(target_arch = "wasm32")]
                        {
                            let mut tab_released = tab_released;
                            let released = *tab_released.peek();
                            tab_released.set(evt.key() == Key::Escape);
                            let mods = evt.modifiers();
                            let plain = !mods.ctrl() && !mods.meta() && !mods.alt();
                            let key = match evt.key() {
                                Key::Tab if plain && !released => Some(if mods.shift() { EditKey::Outdent } else { EditKey::Indent }),
                                Key::Enter if plain && !mods.shift() && !evt.is_composing() => Some(EditKey::Enter),
                                _ => None,
                            };
                            if let Some(key) = key.filter(|_| !readonly) {
                                if edit_on_key(&textarea_id.read(), language, key) {
                                    evt.prevent_default();
                                }
                            }
                            // The key moves the caret after this handler; read it once it has — in
                            // Math, first stepping it over a symbol whose spelling it landed in.
                            let snap_to = match evt.key() {
                                _ if language != Language::Vernacular => None,
                                Key::ArrowLeft => Some(Snap::Start),
                                Key::ArrowRight => Some(Snap::End),
                                Key::ArrowUp | Key::ArrowDown | Key::Home | Key::End | Key::PageUp | Key::PageDown => Some(Snap::Nearest),
                                _ => None,
                            };
                            spawn(async move {
                                gloo_timers::future::TimeoutFuture::new(0).await;
                                if let Some(toward) = snap_to {
                                    snap_caret(&textarea_id.read(), toward);
                                }
                                report_caret();
                            });
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        let _ = (evt, tab_released);
                    },
                    onkeyup: move |_| report_caret(),
                    onmouseup: move |_| {
                        #[cfg(target_arch = "wasm32")]
                        if language == Language::Vernacular {
                            snap_caret(&textarea_id.read(), Snap::Nearest);
                        }
                        report_caret();
                    },
                    onselect: move |_| report_caret(),
                    onfocus: move |_| {
                        report_caret();
                        #[cfg(target_arch = "wasm32")]
                        keep_caret_in_view(&textarea_id.read());
                    },
                    // A finger cannot rest on a word: a tap that puts the caret inside a problem
                    // shows that problem, as a resting pointer would.
                    onpointerup: move |evt: PointerEvent| {
                        if !language_server || evt.data().pointer_type() != "touch" {
                            return;
                        }
                        #[cfg(target_arch = "wasm32")]
                        {
                            let source = source_for_tap.clone();
                            spawn(async move {
                                gloo_timers::future::TimeoutFuture::new(0).await;
                                let Some(area) = textarea(&textarea_id.read()) else { return };
                                let Some(units) = area.selection_start().ok().flatten() else { return };
                                let offset = byte_at(&area.value(), units as usize);
                                let here = {
                                    let found = found.peek();
                                    if found.0 == source { found.1.clone() } else { rebase(&found.1, &found.0, &source) }
                                };
                                match (problems_html(&here, offset), token_box(&area, offset)) {
                                    (Some(html), Some(rect)) => {
                                        keep_hint();
                                        hint.set(Some(HoverHint { offset, html, style: hint_placement(&rect) }));
                                    }
                                    _ => hide_hint(),
                                }
                            });
                        }
                    },
                    oninput: move |evt| {
                        hide_hint();
                        let text = evt.value();
                        let mut typed = typed;
                        typed.set(Some(text.clone()));
                        on_change.call(text);
                        report_caret();
                        #[cfg(target_arch = "wasm32")]
                        spawn(async move {
                            gloo_timers::future::TimeoutFuture::new(0).await;
                            keep_caret_in_view(&textarea_id.read());
                        });
                        // Sync scroll after input in case highlight re-renders
                        #[cfg(target_arch = "wasm32")]
                        {
                            let _ = js_sys::eval(r#"
                                requestAnimationFrame(function() {
                                    document.querySelectorAll('.code-editor-textarea').forEach(function(ta) {
                                        var highlight = ta.previousElementSibling;
                                        if (highlight && highlight.classList.contains('code-editor-highlight')) {
                                            var taMax = ta.scrollHeight - ta.clientHeight;
                                            var hlMax = highlight.scrollHeight - highlight.clientHeight;
                                            // Cap scroll to the shorter element's max
                                            var maxScroll = Math.min(taMax, hlMax);
                                            if (ta.scrollTop > maxScroll) {
                                                ta.scrollTop = maxScroll;
                                            }
                                            highlight.scrollTop = ta.scrollTop;
                                            highlight.scrollLeft = ta.scrollLeft;
                                        }
                                    });
                                });
                            "#);
                        }
                    },
                    onscroll: move |_| {
                        hide_hint();
                        #[cfg(target_arch = "wasm32")]
                        {
                            let _ = js_sys::eval(r#"
                                document.querySelectorAll('.code-editor-textarea').forEach(function(ta) {
                                    var highlight = ta.previousElementSibling;
                                    if (highlight && highlight.classList.contains('code-editor-highlight')) {
                                        var taMax = ta.scrollHeight - ta.clientHeight;
                                        var hlMax = highlight.scrollHeight - highlight.clientHeight;
                                        // Cap scroll to the shorter element's max
                                        var maxScroll = Math.min(taMax, hlMax);
                                        if (ta.scrollTop > maxScroll) {
                                            ta.scrollTop = maxScroll;
                                        }
                                        highlight.scrollTop = ta.scrollTop;
                                        highlight.scrollLeft = ta.scrollLeft;
                                    }
                                });
                            "#);
                        }
                    },
                }
            }
            if let Some(h) = hint.read().as_ref() {
                div {
                    class: "hover-hint",
                    style: "{h.style}",
                    onmouseenter: move |_| {
                        hint_hovered.set(true);
                        bump(hint_gen);
                        keep_hint();
                    },
                    onmouseleave: move |_| hide_hint(),
                    dangerous_inner_html: "{h.html}",
                }
            }
        }
    }
}

/// How long the pointer rests on a word before its hover appears — VS Code's default.
#[cfg(target_arch = "wasm32")]
const HOVER_DELAY_MS: u32 = 300;

/// A key the editor handles itself.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
enum EditKey {
    Enter,
    Indent,
    Outdent,
}

/// Enter, Tab or Shift-Tab in the textarea `id`, as its language indents (see [`Indentation`]):
/// the edit goes through the browser's own editing, so it can be undone like typing. False when
/// the browser should do what it does.
#[cfg(target_arch = "wasm32")]
fn edit_on_key(id: &str, language: Language, key: EditKey) -> bool {
    let Some(area) = textarea(id) else {
        return false;
    };
    let text = area.value();
    let (Ok(Some(start)), Ok(Some(end))) = (area.selection_start(), area.selection_end()) else {
        return false;
    };
    let (start, end) = (byte_at(&text, start as usize), byte_at(&text, end as usize));
    let indentation = Indentation::of(language, &text);
    let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[start..].find('\n').map_or(text.len(), |i| start + i);
    let select = |from: usize, to: usize| {
        let _ = area.set_selection_range(utf16_at(&text, from) as u32, utf16_at(&text, to) as u32);
    };
    match key {
        EditKey::Enter => {
            let line = &text[line_start..line_end];
            let enter = indentation.enter(line, start - line_start);
            let own = line.len() - line.trim_start().len();
            if let (Some(reindent), true) =
                (&enter.reindent, start == end && start >= line_start + own)
            {
                select(line_start, line_start + own);
                insert_text(&area, reindent);
                let caret = area.selection_start().ok().flatten().unwrap_or(0) as usize
                    + utf16_len(&text[line_start + own..start]);
                let _ = area.set_selection_range(caret as u32, caret as u32);
            }
            insert_text(&area, &enter.insert);
        }
        EditKey::Indent if !text[start..end].contains('\n') => {
            let column = text[line_start..start].chars().count();
            insert_text(&area, &indentation.tab_fill(column));
        }
        EditKey::Indent | EditKey::Outdent => {
            // Every line the selection touches — but not a last line it only reaches the start of.
            let last = if end > start && text[..end].ends_with('\n') {
                end - 1
            } else {
                end
            };
            let block_end = text[last..].find('\n').map_or(text.len(), |i| last + i);
            let block = &text[line_start..block_end];
            let moved = match key {
                EditKey::Indent => indentation.indent_lines(block),
                _ => indentation.outdent_lines(block),
            };
            if moved == block {
                return true;
            }
            let shift = utf16_len(block) as isize - utf16_len(&moved) as isize;
            select(line_start, block_end);
            insert_text(&area, &moved);
            if start == end {
                let caret = (utf16_at(&text, start) as isize - shift)
                    .max(utf16_at(&text, line_start) as isize) as u32;
                let _ = area.set_selection_range(caret, caret);
            } else {
                let from = utf16_at(&text, line_start) as u32;
                let _ = area.set_selection_range(from, from + utf16_len(&moved) as u32);
            }
        }
    }
    true
}

/// Replace the textarea's selection with `text` through the browser's editing command, which
/// keeps the undo history and raises `input` like typing; where the command is missing, set
/// the text directly and raise `input` by hand.
#[cfg(target_arch = "wasm32")]
fn insert_text(area: &web_sys::HtmlTextAreaElement, text: &str) {
    use wasm_bindgen::JsCast;
    let document = area
        .owner_document()
        .and_then(|d| d.dyn_into::<web_sys::HtmlDocument>().ok());
    let command = if text.is_empty() {
        "delete"
    } else {
        "insertText"
    };
    let done = document.is_some_and(|d| {
        d.exec_command_with_show_ui_and_value(command, false, text)
            .unwrap_or(false)
    });
    if !done {
        let start = area.selection_start().ok().flatten().unwrap_or(0);
        let end = area.selection_end().ok().flatten().unwrap_or(start);
        let _ = area.set_range_text_with_start_and_end(text, start, end);
        let caret = start + utf16_len(text) as u32;
        let _ = area.set_selection_range(caret, caret);
        if let Ok(event) = web_sys::InputEvent::new("input") {
            let _ = area.dispatch_event(&event);
        }
    }
}

/// The byte offset in `text` of the UTF-16 offset `units`, as the browser counts selections.
#[cfg(target_arch = "wasm32")]
fn byte_at(text: &str, units: usize) -> usize {
    let mut seen = 0;
    for (at, c) in text.char_indices() {
        if seen >= units {
            return at;
        }
        seen += c.len_utf16();
    }
    text.len()
}

/// The UTF-16 offset of the byte offset `at` in `text`.
#[cfg(target_arch = "wasm32")]
fn utf16_at(text: &str, at: usize) -> usize {
    utf16_len(&text[..at])
}

#[cfg(target_arch = "wasm32")]
fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// A fresh id for an editor's textarea, so its caret can be read back.
fn next_textarea_id() -> String {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    format!("code-editor-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// The editor's textarea `id`.
#[cfg(target_arch = "wasm32")]
fn textarea(id: &str) -> Option<web_sys::HtmlTextAreaElement> {
    use wasm_bindgen::JsCast;
    web_sys::window()?
        .document()?
        .get_element_by_id(id)?
        .dyn_into::<web_sys::HtmlTextAreaElement>()
        .ok()
}

/// Where the caret sits in the textarea `id`.
#[cfg(target_arch = "wasm32")]
fn caret_in(id: &str) -> Option<Caret> {
    let area = textarea(id)?;
    let start = area.selection_start().ok()?? as usize;
    let end = area.selection_end().ok()?? as usize;
    let backward = area.selection_direction().ok().flatten().as_deref() == Some("backward");
    Some(Caret::of(&area.value(), start, end, backward))
}

/// Put the textarea `id`'s caret at the start of its text.
#[cfg(target_arch = "wasm32")]
fn caret_to_start(id: &str) {
    if let Some(area) = textarea(id) {
        let _ = area.set_selection_range(0, 0);
    }
}

/// In Math, a symbol the font draws for a spelling (`forall` as ∀) is one character to the
/// reader: a caret that lands inside the spelling moves to an edge of it.
#[cfg(target_arch = "wasm32")]
fn snap_caret(id: &str, toward: Snap) {
    let Some(area) = textarea(id) else { return };
    let (Ok(Some(start)), Ok(Some(end))) = (area.selection_start(), area.selection_end()) else {
        return;
    };
    if start != end {
        return;
    }
    let text = area.value();
    let at = byte_at(&text, start as usize);
    let snapped = super::vernacular::snap(&text, at, toward);
    if snapped != at {
        let caret = utf16_at(&text, snapped) as u32;
        let _ = area.set_selection_range(caret, caret);
    }
}

/// How long a hover waits, once the pointer leaves its word, for the pointer to reach it.
#[cfg(target_arch = "wasm32")]
const HIDE_GRACE_MS: u32 = 300;

/// How long typing pauses before the language server looks again.
#[cfg(target_arch = "wasm32")]
const ANALYSIS_DELAY_MS: u32 = 350;

/// The problems `language`'s service finds in `source`.
fn service_problems(
    language: Language,
    cache: &std::rc::Rc<std::cell::RefCell<Option<std::rc::Rc<DocumentState>>>>,
    source: &str,
) -> Vec<Problem> {
    match language {
        Language::Logos | Language::Logic => problems(&analysed(cache, source)),
        Language::Hardware => crate::ui::pages::hardware_spec::problems(source),
        Language::Vernacular | Language::Rust => Vec::new(),
    }
}

/// What `language`'s service says about the word at byte `offset`, as markdown.
#[cfg(target_arch = "wasm32")]
fn service_hover(
    language: Language,
    cache: &std::rc::Rc<std::cell::RefCell<Option<std::rc::Rc<DocumentState>>>>,
    source: &str,
    offset: usize,
) -> Option<String> {
    if comment_at(source, language, offset) {
        return None;
    }
    match language {
        Language::Logos | Language::Logic => hover_markdown(&analysed(cache, source), offset),
        Language::Hardware => crate::ui::pages::hardware_spec::hover(source, offset),
        Language::Vernacular | Language::Rust => None,
    }
}

/// The language server's analysis of `source`, from the cache when it is of the same text.
fn analysed(
    cache: &std::rc::Rc<std::cell::RefCell<Option<std::rc::Rc<DocumentState>>>>,
    source: &str,
) -> std::rc::Rc<DocumentState> {
    let mut cached = cache.borrow_mut();
    match cached.as_ref() {
        Some(doc) if doc.source == source => doc.clone(),
        _ => {
            let doc = std::rc::Rc::new(DocumentState::new(source.to_string(), 0));
            *cached = Some(doc.clone());
            doc
        }
    }
}

/// Advance a generation counter, voiding whatever was waiting on its old value.
fn bump(mut generation: Signal<u64>) -> u64 {
    let next = *generation.peek() + 1;
    generation.set(next);
    next
}

/// A hover on screen: the token it describes, its HTML, and where it sits.
#[derive(Clone, PartialEq)]
struct HoverHint {
    offset: usize,
    html: String,
    style: String,
}

/// The highlighted token under a viewport point, as its source offset and on-screen box: the
/// editor's textarea sits over the highlight layer, so find the textarea there and test the
/// token spans beside it.
#[cfg(target_arch = "wasm32")]
fn token_under(x: f64, y: f64) -> Option<(usize, web_sys::DomRect)> {
    use wasm_bindgen::JsCast;
    let document = web_sys::window()?.document()?;
    let textarea = document
        .elements_from_point(x as f32, y as f32)
        .iter()
        .filter_map(|v| v.dyn_into::<web_sys::Element>().ok())
        .find(|e| e.class_list().contains("code-editor-textarea"))?;
    let highlight = textarea.previous_element_sibling()?;
    let spans = highlight.query_selector_all("[data-o]").ok()?;
    for i in 0..spans.length() {
        let Some(span) = spans
            .item(i)
            .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
        else {
            continue;
        };
        let boxes = span.get_client_rects();
        for j in 0..boxes.length() {
            let Some(r) = boxes.item(j) else { continue };
            if x >= r.left() && x <= r.right() && y >= r.top() && y <= r.bottom() {
                return Some((span.get_attribute("data-o")?.parse().ok()?, r));
            }
        }
    }
    None
}

/// The on-screen box of the highlighted token at byte `offset` of the textarea `area`'s source.
#[cfg(target_arch = "wasm32")]
fn token_box(area: &web_sys::HtmlTextAreaElement, offset: usize) -> Option<web_sys::DomRect> {
    use wasm_bindgen::JsCast;
    let spans = area.previous_element_sibling()?.query_selector_all("[data-o]").ok()?;
    (0..spans.length()).rev().filter_map(|i| spans.item(i)?.dyn_into::<web_sys::Element>().ok()).find_map(|span| {
        let start: usize = span.get_attribute("data-o")?.parse().ok()?;
        let end = start + span.text_content().map_or(0, |t| t.len());
        (start <= offset && offset <= end).then(|| span.get_bounding_client_rect())
    })
}

/// Scroll the focused textarea `id` so its caret's line shows in the part of it the reader can see,
/// above an on-screen keyboard ([`caret_in_view`]). Lines do not wrap, so the caret's line is its
/// count of line breaks down the text.
#[cfg(target_arch = "wasm32")]
fn keep_caret_in_view(id: &str) {
    use crate::ui::pages::studio_layout::{caret_in_view, Extent};
    let (Some(area), Some(window)) = (textarea(id), web_sys::window()) else { return };
    let focused = window.document().and_then(|d| d.active_element()).is_some_and(|active| active == **area);
    let Some(style) = window.get_computed_style(&area).ok().flatten() else { return };
    let length = |name: &str| style.get_property_value(name).ok().and_then(|v| v.trim_end_matches("px").parse::<f64>().ok());
    let (Some(line_height), Some(padding_top), Some(units)) = (length("line-height"), length("padding-top"), area.selection_start().ok().flatten()) else { return };
    if !focused || line_height <= 0.0 {
        return;
    }
    let line = area.value().encode_utf16().take(units as usize).filter(|&unit| unit == u16::from(b'\n')).count() as f64;
    let rect = area.get_bounding_client_rect();
    let top = rect.top() + padding_top + line * line_height - f64::from(area.scroll_top());
    let (seen_top, seen_bottom) = match window.visual_viewport() {
        Some(visual) => (visual.offset_top(), visual.offset_top() + visual.height()),
        None => (0.0, window.inner_height().ok().and_then(|h| h.as_f64()).unwrap_or(rect.bottom())),
    };
    let view = Extent { top: rect.top().max(seen_top), bottom: rect.bottom().min(seen_bottom) };
    let dy = caret_in_view(Extent { top, bottom: top + line_height }, view);
    if dy.abs() >= 1.0 {
        area.set_scroll_top(area.scroll_top() + dy.round() as i32);
    }
}

/// Above the word when there is room (VS Code's habit), otherwise below; never off-screen.
#[cfg(target_arch = "wasm32")]
fn hint_placement(word: &web_sys::DomRect) -> String {
    let window = web_sys::window();
    let size = |f: fn(&web_sys::Window) -> Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>| {
        window
            .as_ref()
            .and_then(|w| f(w).ok())
            .and_then(|v| v.as_f64())
            .unwrap_or(1080.0)
    };
    let (width, height) = (
        size(web_sys::Window::inner_width),
        size(web_sys::Window::inner_height),
    );
    let left = word.left().min(width - HINT_MAX_WIDTH - 8.0).max(8.0);
    if word.top() > height * 0.35 {
        format!(
            "left: {left:.0}px; bottom: {:.0}px;",
            height - word.top() + 6.0
        )
    } else {
        format!("left: {left:.0}px; top: {:.0}px;", word.bottom() + 6.0)
    }
}

/// The widest a hover grows, matching `.hover-hint`'s `max-width`.
#[cfg(target_arch = "wasm32")]
const HINT_MAX_WIDTH: f64 = 520.0;

/// The most tokens a highlight layer draws as siblings. dioxus-interpreter sends the number of
/// nodes an edit moves as a `u16`, so a longer row of siblings wraps the count and the page stops
/// answering; the highlighters group their tokens into inline runs no longer than this (locked by
/// tests/code_view_channel_lock.rs).
const TOKEN_RUN: usize = 4096;

/// Read-only syntax-highlighted code view.
#[component]
pub fn CodeView(code: String, language: Language) -> Element {
    let tokens = tokenize(&code, language);

    rsx! {
        style { "{CODE_EDITOR_STYLE}" }

        div { class: "code-editor lang-{language.css_class()}",
            div { class: "code-editor-highlight",
                style: "position: relative; height: 100%; overflow: auto; pointer-events: auto;",
                for run in tokens.chunks(TOKEN_RUN) {
                    span {
                        for token in run.iter() {
                            if token.text.chars().all(|c| c.is_whitespace()) {
                                "{token.text}"
                            } else {
                                span {
                                    class: "{token_class(token.kind)}",
                                    "{token.text}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/components/code_editor/hardware_highlighting_tests.rs"]
mod hardware_highlighting_tests;

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/components/code_editor/logic_highlighting_tests.rs"]
mod logic_highlighting_tests;

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/components/code_editor/hash_comment_tests.rs"]
mod hash_comment_tests;

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/components/code_editor/logos_keyword_tests.rs"]
mod logos_keyword_tests;
