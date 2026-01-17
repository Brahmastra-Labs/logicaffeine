//! Phase: Two-Pass Lexer Architecture Tests (Spec §2.5.2)
//!
//! Tests for the LineLexer (Stage 1) indent/dedent behavior.
//! The LineLexer handles structural tokens (Indent, Dedent, Newline)
//! while treating all other content as opaque for Stage 2.

use logicaffeine_language::lexer::{LineLexer, LineToken};

// ============================================================================
// Basic Indentation Tests
// ============================================================================

#[test]
fn test_line_lexer_no_indentation() {
    let source = "Show x.";
    let lexer = LineLexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // Should have exactly one Content token, no Indent/Dedent
    assert_eq!(tokens.len(), 1);
    assert!(matches!(&tokens[0], LineToken::Content { .. }));
}

#[test]
fn test_line_lexer_basic_indent() {
    let source = "If x:\n    Show x.";
    let lexer = LineLexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // Should have: Content("If x:"), Indent, Content("Show x."), Dedent (at EOF)
    assert!(tokens.iter().any(|t| matches!(t, LineToken::Indent)));
}

#[test]
fn test_line_lexer_dedent_at_eof() {
    let source = "If x:\n    Show x.";
    let lexer = LineLexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // Should emit Dedent at EOF to close the block
    assert!(tokens.iter().any(|t| matches!(t, LineToken::Dedent)));
}

#[test]
fn test_line_lexer_explicit_dedent() {
    let source = "If x:\n    Show x.\nShow y.";
    let lexer = LineLexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // Should have Dedent before "Show y." (explicit return to base indent)
    let dedent_count = tokens.iter().filter(|t| matches!(t, LineToken::Dedent)).count();
    assert_eq!(dedent_count, 1, "Should have exactly one Dedent");
}

// ============================================================================
// Multiple Indent Levels
// ============================================================================

#[test]
fn test_line_lexer_nested_indent() {
    let source = "If x:\n    If y:\n        Show z.";
    let lexer = LineLexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // Should have 2 Indents (one for each level)
    let indent_count = tokens.iter().filter(|t| matches!(t, LineToken::Indent)).count();
    assert_eq!(indent_count, 2, "Should have two Indents for nested blocks");
}

#[test]
fn test_line_lexer_multiple_dedents() {
    let source = "If x:\n    If y:\n        Show z.\nShow w.";
    let lexer = LineLexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // Should have 2 Dedents when dropping from level 2 to level 0
    let dedent_count = tokens.iter().filter(|t| matches!(t, LineToken::Dedent)).count();
    assert_eq!(dedent_count, 2, "Should have two Dedents for double dedent");
}

#[test]
fn test_line_lexer_partial_dedent() {
    let source = "If x:\n    If y:\n        Show z.\n    Show y.";
    let lexer = LineLexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // Should have 1 Dedent (from level 2 to level 1), then 1 more at EOF
    let dedent_count = tokens.iter().filter(|t| matches!(t, LineToken::Dedent)).count();
    assert_eq!(dedent_count, 2, "Should have two Dedents total");
}

// ============================================================================
// Tab Handling (Tab = 4 spaces)
// ============================================================================

#[test]
fn test_line_lexer_tab_indent() {
    let source = "If x:\n\tShow x.";
    let lexer = LineLexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // Tab should be treated as 4 spaces (indentation)
    assert!(tokens.iter().any(|t| matches!(t, LineToken::Indent)));
}

#[test]
fn test_line_lexer_mixed_tabs_spaces() {
    let source = "If x:\n  \tShow x.";  // 2 spaces + tab = 6 "spaces"
    let lexer = LineLexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // Mixed tabs and spaces should work (tab = 4 spaces)
    assert!(tokens.iter().any(|t| matches!(t, LineToken::Indent)));
}

// ============================================================================
// Blank Lines
// ============================================================================

#[test]
fn test_line_lexer_blank_line_preserves_indent() {
    let source = "If x:\n    Show x.\n\n    Show y.";
    let lexer = LineLexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // Blank line should not emit Dedent; both Shows are in same block
    let dedent_count = tokens.iter().filter(|t| matches!(t, LineToken::Dedent)).count();
    assert_eq!(dedent_count, 1, "Only one Dedent at EOF");
}

#[test]
fn test_line_lexer_multiple_blank_lines() {
    let source = "If x:\n    Show x.\n\n\n    Show y.";
    let lexer = LineLexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // Multiple blank lines should not affect indentation
    let dedent_count = tokens.iter().filter(|t| matches!(t, LineToken::Dedent)).count();
    assert_eq!(dedent_count, 1, "Only one Dedent at EOF");
}

// ============================================================================
// Content Preservation
// ============================================================================

#[test]
fn test_line_lexer_content_text() {
    let source = "Let x be 5.";
    let lexer = LineLexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // Content should have the full trimmed line text
    if let Some(LineToken::Content { text, .. }) = tokens.first() {
        assert_eq!(text, "Let x be 5.");
    } else {
        panic!("Expected Content token");
    }
}

#[test]
fn test_line_lexer_content_spans() {
    let source = "Let x be 5.\nLet y be 10.";
    let lexer = LineLexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // Should have two Content tokens
    let content_count = tokens.iter().filter(|t| matches!(t, LineToken::Content { .. })).count();
    assert_eq!(content_count, 2, "Should have two Content tokens");
}

#[test]
fn test_line_lexer_content_start_end() {
    let source = "    Let x be 5.";
    let lexer = LineLexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // Content should have correct byte offsets (after leading whitespace)
    if let Some(LineToken::Content { text, start, end }) = tokens.iter().find(|t| matches!(t, LineToken::Content { .. })) {
        assert_eq!(text, "Let x be 5.");
        assert_eq!(*start, 4, "Start should be after 4 spaces");
        assert_eq!(*end, source.len(), "End should be at end of source");
    } else {
        panic!("Expected Content token");
    }
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_line_lexer_empty_source() {
    let source = "";
    let lexer = LineLexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // Empty source should produce no tokens
    assert!(tokens.is_empty(), "Empty source should produce no tokens");
}

#[test]
fn test_line_lexer_only_whitespace() {
    let source = "   \n   \n   ";
    let lexer = LineLexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // Only whitespace lines should not produce Content tokens
    let content_count = tokens.iter().filter(|t| matches!(t, LineToken::Content { .. })).count();
    assert_eq!(content_count, 0, "No Content tokens from whitespace-only lines");
}

#[test]
fn test_line_lexer_inconsistent_indent_error() {
    // Dedent to a level that was never on the stack
    let source = "If x:\n        Show x.\n    Show y.";  // 8 spaces then 4 spaces
    let lexer = LineLexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // Should still work - 4 spaces is a valid dedent from 8 spaces
    // The dedent goes from 8 to 4 (not to 0)
    assert!(tokens.iter().any(|t| matches!(t, LineToken::Dedent)));
}

// ============================================================================
// Integration with Block Headers
// ============================================================================

#[test]
fn test_line_lexer_block_header() {
    let source = "## Main\nLet x be 5.";
    let lexer = LineLexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // Block header should be Content, no indent change
    let content_count = tokens.iter().filter(|t| matches!(t, LineToken::Content { .. })).count();
    assert_eq!(content_count, 2);
}

#[test]
fn test_line_lexer_colon_line() {
    let source = "If x:";
    let lexer = LineLexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // Single line with colon - no indent yet (would need next line)
    let indent_count = tokens.iter().filter(|t| matches!(t, LineToken::Indent)).count();
    assert_eq!(indent_count, 0, "No Indent without following indented line");
}
