//! A single `#` opens a comment to the end of the line, wherever it stands, and `##` opens a
//! block header — exactly as the LOGOS lexer reads them, in Code and Logic alike.
use super::{tokenize, Language, TokenKind};

fn comment_texts(source: &str, language: Language) -> Vec<String> {
    tokenize(source, language).into_iter().filter(|t| t.kind == TokenKind::Comment).map(|t| t.text).collect()
}

#[test]
fn a_hash_line_is_a_comment_in_code() {
    let source = "# It runs live in the preview: click the button and the count updates.\n## Main\nLet x be 1.";
    assert_eq!(
        comment_texts(source, Language::Logos),
        ["# It runs live in the preview: click the button and the count updates."]
    );
    let in_word = tokenize(source, Language::Logos).into_iter().find(|t| t.text == "in").map(|t| t.kind);
    assert!(in_word != Some(TokenKind::Keyword), "a word inside a comment is highlighted as a keyword");
}

#[test]
fn a_block_header_keeps_its_words_in_code() {
    let tokens = tokenize("## Main\nLet x be 1.", Language::Logos);
    assert!(!tokens.iter().any(|t| t.kind == TokenKind::Comment), "## Main is a header, not a comment");
}

#[test]
fn a_hash_after_text_is_a_comment() {
    assert_eq!(comment_texts("Every cat sleeps. # a note", Language::Logic), ["# a note"]);
    assert_eq!(comment_texts("Let x be 1. # a note", Language::Logos), ["# a note"]);
}

#[test]
fn a_dash_comment_still_reads_as_a_comment_in_code() {
    assert_eq!(comment_texts("-- Each number is the sum.\nLet x be 1.", Language::Logos), ["-- Each number is the sum."]);
}

/// In Code, the literate doc before the first header and a `## Note` body are prose: coloured as
/// comments, and the hover leaves them alone, while the code beside them keeps its words (the
/// user, 2026-11-01: comments in Web mode showed hints, in the Studio and the front page's frame).
#[test]
fn code_prose_is_a_comment_and_has_no_hover() {
    let source = "# A Website in a Website\n\nThis card is a whole website, and the program beside it is all of it.\n\n## Note\n\nCard is a piece the page places.\n\n## Page /\n\nState:\n    Let hellos be 0.\n";
    let comments = comment_texts(source, super::Language::Logos);
    assert_eq!(
        comments,
        ["# A Website in a Website", "This card is a whole website, and the program beside it is all of it.", "Card is a piece the page places."],
        "the prose is not read as comments"
    );
    for word in ["This card", "Card is a piece", "# A Website"] {
        assert!(super::comment_at(source, super::Language::Logos, source.find(word).unwrap()), "the hover is not hidden over {word:?}");
    }
    for word in ["## Note", "## Page", "State:", "Let hellos"] {
        assert!(!super::comment_at(source, super::Language::Logos, source.find(word).unwrap()), "the hover is hidden over {word:?}");
    }
    let tokens = tokenize(source, super::Language::Logos);
    assert!(tokens.iter().any(|t| t.text == "Let" && t.kind == TokenKind::Keyword), "Let lost its colour");
}

/// A file with no header is all code: nothing before a header that is not there is prose.
#[test]
fn a_headerless_file_is_all_code() {
    let source = "Let x be 1.\nShow x.\n";
    assert!(comment_texts(source, super::Language::Logos).is_empty());
    assert!(!super::comment_at(source, super::Language::Logos, 0));
}

/// Logic keeps every sentence: prose is the content there.
#[test]
fn logic_prose_stays_content() {
    let source = "Every cat sleeps.\n## Theorem: cats\nGiven: Every cat sleeps.\n";
    assert!(comment_texts(source, super::Language::Logic).is_empty());
}
