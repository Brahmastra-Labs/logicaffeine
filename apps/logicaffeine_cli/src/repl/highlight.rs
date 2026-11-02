//! Live syntax highlighting for the REPL prompt line — driven by the SAME
//! classifier the LSP uses ([`logicaffeine_language::token_class`]), so the
//! terminal and the editor can never disagree about what a word is.
//!
//! Invariant (locked below): highlighting only inserts ANSI escapes —
//! stripping them yields the input byte-for-byte.

use anstyle::{AnsiColor, Style};
use logicaffeine_base::Interner;
use logicaffeine_language::token_class::{classify, TokenClass};
use logicaffeine_language::Lexer;

/// ANSI style per token class — 8-color-safe, readable on dark and light.
fn style_for(class: TokenClass) -> Style {
    match class {
        TokenClass::Keyword => AnsiColor::Blue.on_default().bold(),
        TokenClass::Type => AnsiColor::Cyan.on_default(),
        TokenClass::Function => AnsiColor::Yellow.on_default(),
        TokenClass::Variable => Style::new(),
        TokenClass::String => AnsiColor::Green.on_default(),
        TokenClass::Number => AnsiColor::Magenta.on_default(),
        TokenClass::Operator => Style::new(),
        TokenClass::Namespace => Style::new().bold(),
        TokenClass::Modifier => AnsiColor::Cyan.on_default().italic(),
        TokenClass::Property => AnsiColor::Cyan.on_default(),
        TokenClass::Comment => AnsiColor::BrightBlack.on_default(),
        TokenClass::Parameter => Style::new().italic(),
        TokenClass::EnumMember => AnsiColor::Magenta.on_default(),
    }
}

/// Paint one prompt line. Meta-commands (`:help`) render dim as a whole;
/// everything else lexes and paints per token. Unlexable text passes
/// through unpainted — highlighting must never get in the way of typing.
pub fn highlight_line(line: &str) -> String {
    if line.trim_start().starts_with(':') {
        let style = AnsiColor::Cyan.on_default();
        return format!("{style}{line}{reset}", reset = style.render_reset());
    }

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(line, &mut interner);
    let tokens = lexer.tokenize();

    let mut out = String::with_capacity(line.len() * 2);
    let mut cursor = 0usize;
    for token in &tokens {
        let start = token.span.start.min(line.len());
        let end = token.span.end.min(line.len());
        if start < cursor || end <= start {
            continue; // structural/zero-width tokens
        }
        out.push_str(&line[cursor..start]);
        let text = &line[start..end];
        match classify(&token.kind) {
            Some(class) => {
                let style = style_for(class);
                if style == Style::new() {
                    out.push_str(text);
                } else {
                    out.push_str(&format!(
                        "{style}{text}{reset}",
                        reset = style.render_reset()
                    ));
                }
            }
            None => out.push_str(text),
        }
        cursor = end;
    }
    out.push_str(&line[cursor.min(line.len())..]);
    out
}

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_cli/repl/highlight/tests.rs"]
mod tests;
