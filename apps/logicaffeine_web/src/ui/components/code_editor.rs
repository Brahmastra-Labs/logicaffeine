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

/// Language mode for syntax highlighting.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    #[default]
    Logos,      // Imperative .logos files
    Vernacular, // Math/theorem mode (Definition, Check, Eval)
    Rust,       // Generated Rust output
}

/// Token type for syntax highlighting.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TokenKind {
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
struct Token {
    text: String,
    kind: TokenKind,
}

const CODE_EDITOR_STYLE: &str = r#"
.code-editor {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: #0f1419;
    font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
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
    padding: 16px;
    padding-bottom: 50%;  /* Extra space to scroll past end */
    margin: 0;
    font-size: 14px;
    font-family: inherit;
    line-height: 1.6;
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
    padding: 16px;
    padding-bottom: 50%;  /* Extra space to scroll past end */
    margin: 0;
    font-size: 14px;
    font-family: inherit;
    line-height: 1.6;
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
.tok-comment { color: #5c6370; }
.tok-operator { color: #56b6c2; }
.tok-punctuation { color: #abb2bf; }
.tok-identifier { color: #e8eaed; }
.tok-builtin { color: #61afef; }

/* Mobile optimizations */
@media (max-width: 768px) {
    .code-editor-textarea,
    .code-editor-highlight {
        font-size: 16px; /* Prevent iOS zoom */
        padding: 12px;
    }
}
"#;

/// Tokenize source code for syntax highlighting.
fn tokenize(source: &str, language: Language) -> Vec<Token> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = source.chars().collect();
    let len = chars.len();
    let mut i = 0;

    let keywords = match language {
        Language::Logos => &[
            "let", "set", "if", "then", "else", "while", "repeat", "for", "in",
            "return", "match", "with", "end", "function", "struct", "enum",
            "show", "read", "write", "push", "pop", "add", "remove",
            "and", "or", "not", "true", "false", "nothing",
            "zone", "concurrent", "parallel", "launch", "send", "receive", "select",
            "mount", "sync", "merge", "increase", "decrease",
        ][..],
        Language::Vernacular => &[
            "Definition", "Check", "Eval", "Inductive", "Theorem", "Lemma",
            "Proof", "Qed", "Axiom", "Hypothesis", "Variable", "Parameter",
            "fun", "forall", "exists", "match", "with", "end", "let", "in",
            "Prop", "Type", "Set", "Nat", "Bool", "True", "False",
            "Not", "And", "Or", "Eq", "Ex", "All",
        ][..],
        Language::Rust => &[
            "fn", "let", "mut", "const", "static", "if", "else", "match", "loop",
            "while", "for", "in", "return", "break", "continue", "struct", "enum",
            "impl", "trait", "type", "pub", "mod", "use", "crate", "self", "super",
            "async", "await", "move", "ref", "where", "true", "false",
        ][..],
    };

    let types = match language {
        Language::Logos => &[
            "Int", "Text", "Bool", "Real", "Char", "Byte", "Nothing",
            "Seq", "Map", "Set", "Option", "Result", "Tuple",
            "Persistent", "Distributed",
        ][..],
        Language::Vernacular => &[
            "Prop", "Type", "Set", "Nat", "Bool", "Int", "List", "Option",
            "Syntax", "Derivation", "Term",
        ][..],
        Language::Rust => &[
            "i8", "i16", "i32", "i64", "i128", "isize",
            "u8", "u16", "u32", "u64", "u128", "usize",
            "f32", "f64", "bool", "char", "str", "String",
            "Vec", "HashMap", "Option", "Result", "Box", "Rc", "Arc",
        ][..],
    };

    let builtins = match language {
        Language::Logos => &[
            "show", "length", "first", "last", "rest", "append",
            "contains", "keys", "values", "range",
        ][..],
        Language::Vernacular => &[
            "Zero", "Succ", "Nil", "Cons", "Some", "None",
            "refl", "eq_ind", "nat_ind", "syn_diag", "syn_quote", "syn_size",
            "concludes", "Provable", "Consistent",
        ][..],
        Language::Rust => &[
            "println", "print", "format", "vec", "panic", "assert",
            "Some", "None", "Ok", "Err",
        ][..],
    };

    while i < len {
        let c = chars[i];

        // Skip whitespace (preserve it)
        if c.is_whitespace() {
            let start = i;
            while i < len && chars[i].is_whitespace() {
                i += 1;
            }
            tokens.push(Token {
                text: chars[start..i].iter().collect(),
                kind: TokenKind::Identifier, // Whitespace uses default color
            });
            continue;
        }

        // Comments
        let comment_start = match language {
            Language::Logos | Language::Vernacular => "--",
            Language::Rust => "//",
        };
        if i + comment_start.len() <= len {
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
            let kind = if keywords.contains(&word.as_str()) {
                TokenKind::Keyword
            } else if types.contains(&word.as_str()) {
                TokenKind::Type
            } else if builtins.contains(&word.as_str()) {
                TokenKind::Builtin
            } else {
                TokenKind::Identifier
            };
            tokens.push(Token { text: word, kind });
            continue;
        }

        // Operators
        let operators = [
            "->", "=>", ":=", "<=", ">=", "==", "!=", "&&", "||",
            "+", "-", "*", "/", "%", "=", "<", ">", "!", "&", "|", "^",
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

    tokens
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
    #[props(default = "Enter code...".to_string())]
    placeholder: String,
    #[props(default = false)]
    readonly: bool,
) -> Element {
    let tokens = tokenize(&value, language);

    rsx! {
        style { "{CODE_EDITOR_STYLE}" }

        div { class: "code-editor",
            div { class: "code-editor-input",
                // Highlighted overlay
                div { class: "code-editor-highlight",
                    for token in tokens {
                        if token.text.chars().all(|c| c.is_whitespace()) {
                            // Output whitespace as raw text to match textarea rendering
                            "{token.text}"
                        } else {
                            span {
                                class: "{token_class(token.kind)}",
                                "{token.text}"
                            }
                        }
                    }
                }

                // Actual textarea for input
                textarea {
                    class: "code-editor-textarea",
                    placeholder: "{placeholder}",
                    value: "{value}",
                    readonly: readonly,
                    spellcheck: "false",
                    autocomplete: "off",
                    autocapitalize: "off",
                    oninput: move |evt| {
                        on_change.call(evt.value());
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
        }
    }
}

/// Read-only syntax-highlighted code view.
#[component]
pub fn CodeView(
    code: String,
    language: Language,
) -> Element {
    let tokens = tokenize(&code, language);

    rsx! {
        style { "{CODE_EDITOR_STYLE}" }

        div { class: "code-editor",
            div { class: "code-editor-highlight",
                style: "position: relative; height: 100%; overflow: auto; pointer-events: auto;",
                for token in tokens {
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
