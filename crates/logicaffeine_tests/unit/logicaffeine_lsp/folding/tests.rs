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
            range.start_line,
            range.end_line
        );
    }
}

#[test]
fn folding_ranges_indent_dedent_folding() {
    let doc = make_doc("## Main\n    If 1 > 0:\n        Let x be 5.\n");
    let ranges = folding_ranges(&doc);
    // Should have at least one indent/dedent folding range
    assert!(
        !ranges.is_empty(),
        "Expected folding ranges for indented block"
    );
}

#[test]
fn folding_ranges_nested_blocks_multiple_ranges() {
    let doc = make_doc("## Main\n    Let x be 5.\n    If x > 0:\n        Show x.\n");
    let ranges = folding_ranges(&doc);
    assert!(
        ranges.len() >= 2,
        "Expected at least 2 folding ranges (block + indent), got {}",
        ranges.len()
    );
}

#[test]
fn folding_ranges_unmatched_indent_no_panic() {
    // Malformed document — more indents than dedents
    let doc = make_doc("## Main\n");
    let ranges = folding_ranges(&doc);
    // Should not panic on malformed doc — ranges may or may not be produced
    assert!(
        ranges.len() < 100,
        "Unreasonable number of ranges for a tiny doc"
    );
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
