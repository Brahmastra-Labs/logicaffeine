use lsp_types::{FoldingRange, FoldingRangeKind};

use logicaffeine_language::token::TokenType;

use crate::document::DocumentState;

/// Handle folding range request.
///
/// Provides folding for:
/// - Block headers (## Main, ## To, etc.)
/// - Indent/Dedent pairs (indented blocks)
pub fn folding_ranges(doc: &DocumentState) -> Vec<FoldingRange> {
    let mut ranges = Vec::new();

    // Block header folding
    for (_i, (_, _, span)) in doc.symbol_index.block_spans.iter().enumerate() {
        let start_pos = doc.line_index.position(span.start);
        let end_pos = doc.line_index.position(span.end);

        // Don't create zero-line folds
        if end_pos.line > start_pos.line {
            ranges.push(FoldingRange {
                start_line: start_pos.line,
                start_character: Some(start_pos.character),
                end_line: end_pos.line.saturating_sub(1),
                end_character: None,
                kind: Some(FoldingRangeKind::Region),
                collapsed_text: None,
            });
        }
    }

    // Indent/Dedent block folding
    let mut indent_stack: Vec<u32> = Vec::new();
    for token in &doc.tokens {
        match &token.kind {
            TokenType::Indent => {
                let pos = doc.line_index.position(token.span.start);
                indent_stack.push(pos.line);
            }
            TokenType::Dedent => {
                if let Some(start_line) = indent_stack.pop() {
                    let end_pos = doc.line_index.position(token.span.start);
                    if end_pos.line > start_line {
                        ranges.push(FoldingRange {
                            start_line,
                            start_character: None,
                            end_line: end_pos.line.saturating_sub(1),
                            end_character: None,
                            kind: Some(FoldingRangeKind::Region),
                            collapsed_text: None,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    ranges
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/folding/tests.rs"]
mod tests;
