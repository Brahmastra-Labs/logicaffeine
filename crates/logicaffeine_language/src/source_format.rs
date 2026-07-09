//! The canonical LOGOS source formatter.
//!
//! One rule set shared by every formatting surface (the LSP's
//! `textDocument/formatting` provider, its on-type formatter, and
//! `largo fmt`):
//!
//! - code lines reindent to **4 spaces per lexed nesting level** — the
//!   depth comes from the lexer's own Indent/Dedent reading, so the
//!   canonical form is the structure the program already has,
//! - trailing whitespace is removed from code lines; CRLF normalizes to LF,
//! - multiline-string interiors are **content** — not a byte is touched,
//! - `## Note`/`## Example` bodies are the author's prose — untouched
//!   (markdown hard-breaks and nested-list indents survive),
//! - comment-only lines keep their author indent (tabs still normalize),
//! - the presence or absence of a final newline is preserved.
//!
//! The prime directive, locked by `formatter_locks.rs` in the test suite:
//! **formatting never changes the token stream** — and it is a fixed point
//! (`format_source(format_source(s)) == format_source(s)`).

use crate::lexer::Lexer;
use crate::token::{BlockType, TokenType};
use logicaffeine_base::Interner;

/// Format a single line: normalize leading whitespace (tab → 4 spaces) and
/// strip trailing whitespace. Content between the indent and the trailing
/// whitespace is untouched. Line-local — the on-type formatting rule; the
/// whole-document [`format_source`] adds structure awareness on top.
pub fn format_line(line: &str) -> String {
    let trimmed_start = line.trim_start();
    let leading = &line[..line.len() - trimmed_start.len()];
    let mut out = String::with_capacity(line.len());
    for ch in leading.chars() {
        if ch == '\t' {
            out.push_str("    ");
        } else {
            out.push(ch);
        }
    }
    out.push_str(trimmed_start.trim_end());
    // A whitespace-only line reduces to empty.
    if out.chars().all(|c| c == ' ') {
        out.clear();
    }
    out
}

/// Per-line formatting plan, derived from one lexer pass.
#[derive(Clone, Copy, PartialEq)]
enum LinePlan {
    /// Code with a known nesting depth: reindent to `depth * 4` spaces.
    Code(usize),
    /// Code-adjacent but depthless (blank, comment-only): line-local rules.
    Plain,
    /// String interior or documentation prose: not a byte changes.
    Raw,
}

/// Format a whole source text: structural reindentation for code, raw
/// passthrough for string interiors and prose, LF endings, preserved final
/// newline.
pub fn format_source(source: &str) -> String {
    if source.is_empty() {
        return String::new();
    }

    let plans = line_plans(source);
    let mut out: String = source
        .lines()
        .enumerate()
        .map(|(i, line)| match plans.get(i).copied().unwrap_or(LinePlan::Plain) {
            LinePlan::Raw => line.to_string(),
            LinePlan::Plain => format_line(line),
            LinePlan::Code(depth) => {
                let content = line.trim_start().trim_end();
                if content.is_empty() {
                    String::new()
                } else {
                    let mut formatted = " ".repeat(depth * 4);
                    formatted.push_str(content);
                    formatted
                }
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    if source.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// One lexer pass over the source decides how every line formats.
fn line_plans(source: &str) -> Vec<LinePlan> {
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let line_of = |offset: usize| match line_starts.binary_search(&offset) {
        Ok(i) => i,
        Err(i) => i - 1,
    };
    let line_count = source.lines().count();
    let mut plans = vec![LinePlan::Plain; line_count];

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    // Pass 1: nesting depth per line — the depth in effect at each line's
    // first token, from the lexer's own Indent/Dedent reading.
    let mut depth: usize = 0;
    let mut current_line: Option<usize> = None;
    for token in &tokens {
        match &token.kind {
            TokenType::Indent => depth += 1,
            TokenType::Dedent => depth = depth.saturating_sub(1),
            TokenType::Newline | TokenType::EOF => {}
            _ => {
                let line = line_of(token.span.start);
                if current_line != Some(line) {
                    current_line = Some(line);
                    if let Some(plan) = plans.get_mut(line) {
                        *plan = LinePlan::Code(depth);
                    }
                }
            }
        }
    }

    // Pass 2: multiline tokens (string/escape-block interiors) are content.
    // Every line a token SPANS beyond its first is raw, and so is the first
    // line of any multi-line token — its trailing side is inside the token.
    for token in &tokens {
        let start_line = line_of(token.span.start);
        let end_line = line_of(token.span.end.saturating_sub(1).max(token.span.start));
        if end_line > start_line {
            for line in start_line..=end_line {
                if let Some(plan) = plans.get_mut(line) {
                    *plan = LinePlan::Raw;
                }
            }
        }
    }

    // Pass 3: documentation prose (`## Note` / `## Example` bodies) belongs
    // to the author — raw from the line after the header to the next block.
    let mut in_prose = false;
    let mut prose_from = 0usize;
    let mut mark_prose = |plans: &mut Vec<LinePlan>, from: usize, to: usize| {
        for plan in plans.iter_mut().take(to).skip(from) {
            *plan = LinePlan::Raw;
        }
    };
    for token in &tokens {
        if let TokenType::BlockHeader { block_type } = &token.kind {
            let header_line = line_of(token.span.start);
            if in_prose {
                mark_prose(&mut plans, prose_from, header_line);
            }
            in_prose = matches!(block_type, BlockType::Note | BlockType::Example);
            prose_from = header_line + 1;
        }
    }
    if in_prose {
        mark_prose(&mut plans, prose_from, line_count);
    }

    plans
}
