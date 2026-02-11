use tower_lsp::lsp_types::{FoldingRange, FoldingRangeKind};

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
mod tests {
    use super::*;
    use crate::document::DocumentState;

    fn make_doc(source: &str) -> DocumentState {
        DocumentState::new(source.to_string(), 1)
    }

    #[test]
    fn folding_ranges_for_block() {
        let doc = make_doc("## Main\n    Let x be 5.\n    Let y be 10.\n");
        let ranges = folding_ranges(&doc);
        // Should have at least one folding range for the Main block
        assert!(!ranges.is_empty(), "Expected folding ranges for Main block");
    }

    #[test]
    fn folding_ranges_empty_for_empty_doc() {
        let doc = make_doc("");
        let ranges = folding_ranges(&doc);
        assert!(ranges.is_empty());
    }

    #[test]
    fn folding_ranges_have_valid_lines() {
        let doc = make_doc("## Main\n    Let x be 5.\n    Let y be 10.\n");
        let ranges = folding_ranges(&doc);
        for range in &ranges {
            assert!(
                range.start_line <= range.end_line,
                "Folding range has start > end: {} > {}",
                range.start_line, range.end_line
            );
        }
    }

    #[test]
    fn folding_ranges_indent_dedent_folding() {
        let doc = make_doc("## Main\n    If 1 > 0:\n        Let x be 5.\n");
        let ranges = folding_ranges(&doc);
        // Should have at least one indent/dedent folding range
        assert!(!ranges.is_empty(), "Expected folding ranges for indented block");
    }

    #[test]
    fn folding_ranges_nested_blocks_multiple_ranges() {
        let doc = make_doc("## Main\n    Let x be 5.\n    If x > 0:\n        Show x.\n");
        let ranges = folding_ranges(&doc);
        assert!(ranges.len() >= 2,
            "Expected at least 2 folding ranges (block + indent), got {}", ranges.len());
    }

    #[test]
    fn folding_ranges_unmatched_indent_no_panic() {
        // Malformed document — more indents than dedents
        let doc = make_doc("## Main\n");
        let ranges = folding_ranges(&doc);
        // Should not panic on malformed doc — ranges may or may not be produced
        assert!(ranges.len() < 100, "Unreasonable number of ranges for a tiny doc");
    }

    #[test]
    fn folding_ranges_are_region_kind() {
        let doc = make_doc("## Main\n    Let x be 5.\n    Let y be 10.\n");
        let ranges = folding_ranges(&doc);
        for range in &ranges {
            assert_eq!(
                range.kind,
                Some(FoldingRangeKind::Region),
                "Folding ranges should be Region kind"
            );
        }
    }
}
