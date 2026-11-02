//! `source_format` — the canonical LOGOS source formatter.
//!
//! One rule set, shared by the LSP's formatting provider and `largo fmt`:
//! leading tabs become 4 spaces, trailing whitespace goes, content is
//! otherwise untouched.

use logicaffeine_language::source_format::{format_line, format_source};

#[test]
fn properly_indented_source_is_unchanged() {
    let src = "## Main\n    Let x be 5.\n";
    assert_eq!(format_source(src), src);
}

#[test]
fn leading_tab_becomes_four_spaces() {
    assert_eq!(
        format_source("## Main\n\tLet x be 5.\n"),
        "## Main\n    Let x be 5.\n"
    );
}

#[test]
fn double_tab_reindents_to_lexed_depth() {
    // The lexer reads a double-tab FIRST indent as one nesting level; the
    // canonical form is depth × 4 spaces (structure, not width).
    assert_eq!(
        format_source("## Main\n\t\tLet x be 5.\n"),
        "## Main\n    Let x be 5.\n"
    );
}

#[test]
fn mixed_leading_spaces_and_tabs_normalize() {
    assert_eq!(format_line("  \tLet x be 5."), "      Let x be 5.");
}

#[test]
fn trailing_whitespace_is_removed() {
    assert_eq!(
        format_source("## Main   \n    Let x be 5.   \n"),
        "## Main\n    Let x be 5.\n"
    );
}

#[test]
fn content_tabs_are_preserved() {
    let src = "    Show \"a\tb\".\n";
    assert_eq!(format_source(src), src);
}

#[test]
fn empty_source_stays_empty() {
    assert_eq!(format_source(""), "");
}

#[test]
fn missing_final_newline_is_preserved() {
    assert_eq!(format_source("Show 1."), "Show 1.");
}

#[test]
fn final_newline_is_preserved() {
    assert_eq!(format_source("Show 1.\n"), "Show 1.\n");
}

#[test]
fn crlf_normalizes_to_lf() {
    assert_eq!(
        format_source("## Main\r\n\tLet x be 5.\r\n"),
        "## Main\n    Let x be 5.\n"
    );
}

#[test]
fn formatting_is_idempotent() {
    let gnarly = "## Main  \r\n\t\tLet x be 5.\t \n  \tShow x.   \nno_indent\t\n";
    let once = format_source(gnarly);
    assert_eq!(format_source(&once), once, "format must be a fixed point");
}
