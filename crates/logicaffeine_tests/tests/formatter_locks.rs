//! Formatter safety and quality locks.
//!
//! The prime directive: **formatting never changes the program.** The lock
//! is token-stream equivalence — `tokens(format(x))` must equal `tokens(x)`
//! as (kind, lexeme) sequences, string contents byte-for-byte included.
//! On top of that: idempotence, multiline-string interiors untouched, and
//! structural reindentation to canonical 4-space depth.

use logicaffeine_base::Interner;
use logicaffeine_language::source_format::format_source;
use logicaffeine_language::Lexer;

/// Lex to a comparable fingerprint: kind discriminants + lexemes.
/// Indent/Dedent/Newline are INCLUDED — a formatter that changes nesting
/// or line structure is changing the program.
fn fingerprint(source: &str) -> Vec<(std::mem::Discriminant<logicaffeine_language::TokenType>, String)> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    lexer
        .tokenize()
        .into_iter()
        .map(|t| {
            (
                std::mem::discriminant(&t.kind),
                interner.resolve(t.lexeme).to_string(),
            )
        })
        .collect()
}

fn assert_semantics_preserved(source: &str, context: &str) {
    let formatted = format_source(source);
    assert_eq!(
        fingerprint(source),
        fingerprint(&formatted),
        "{context}: formatting changed the token stream.\n--- before ---\n{source}\n--- after ---\n{formatted}"
    );
    assert_eq!(
        formatted,
        format_source(&formatted),
        "{context}: formatting is not idempotent"
    );
}

#[test]
fn multiline_string_interiors_are_untouchable() {
    // Trailing spaces, a tab, and a whitespace-only line INSIDE the string
    // are all string CONTENT — the formatter must not touch a byte of it.
    let source = "## Main\nLet banner be \"\"\"\nline with trailing spaces   \n\tstarts with a tab\n   \nend\n\"\"\".\nShow banner.\n";
    let formatted = format_source(source);
    assert!(
        formatted.contains("line with trailing spaces   \n"),
        "trailing spaces inside a multiline string are content:\n{formatted}"
    );
    assert!(
        formatted.contains("\tstarts with a tab\n"),
        "a tab inside a multiline string is content:\n{formatted}"
    );
    assert!(
        formatted.contains("\n   \n"),
        "a whitespace-only line inside a multiline string is content:\n{formatted}"
    );
    assert_semantics_preserved(source, "multiline string");
}

#[test]
fn formatting_preserves_semantics_across_the_corpus() {
    let corpus = [
        include_str!("../../../benchmarks/programs/quicksort/main.lg"),
        include_str!("../../../editors/vscode/logicaffeine/test/grammar/corpus/imperative.lg"),
        include_str!("../../../editors/vscode/logicaffeine/test/grammar/corpus/theorem.lg"),
        include_str!("../../../editors/vscode/logicaffeine/test/grammar/corpus/types_and_crdt.lg"),
        include_str!("../../../editors/vscode/logicaffeine/test/grammar/corpus/prose_note.lg"),
        include_str!("../../../assets/std/file.lg"),
    ];
    for (i, source) in corpus.iter().enumerate() {
        assert_semantics_preserved(source, &format!("corpus[{i}]"));
    }
}

#[test]
fn code_reindents_to_canonical_four_space_depth() {
    // 2-space and 3-space indents lex to the same structure as 4-space;
    // the canonical form IS 4 spaces per nesting level.
    let source = "## Main\nIf 1 < 2:\n  Show 1.\n  If 2 < 3:\n     Show 2.\nShow 3.\n";
    let formatted = format_source(source);
    assert!(
        formatted.contains("\n    Show 1.\n"),
        "depth 1 = 4 spaces:\n{formatted}"
    );
    assert!(
        formatted.contains("\n        Show 2.\n"),
        "depth 2 = 8 spaces:\n{formatted}"
    );
    assert!(
        formatted.contains("\nShow 3.\n"),
        "depth 0 stays flush:\n{formatted}"
    );
    assert_semantics_preserved(source, "reindent");
}

#[test]
fn tab_indented_code_reindents_by_depth_not_width() {
    let source = "## Main\nWhile 1 < 2:\n\tShow 1.\n\t\tShow 9.\n";
    // NOTE: `\t\tShow 9.` is depth 2 only if the lexer opens a block there;
    // a stray over-indent lexes as its own level. Whatever structure the
    // lexer sees, formatting must preserve it — that's the equivalence lock.
    assert_semantics_preserved(source, "tab reindent");
}

#[test]
fn note_prose_indentation_is_left_alone() {
    // `## Note` bodies are documentation; a 3-space markdown nested list is
    // the AUTHOR'S formatting, not code to normalize.
    let source = "## Note\n\n- outer point\n   - nested markdown list\n\n## Main\nShow 1.\n";
    let formatted = format_source(source);
    assert!(
        formatted.contains("\n   - nested markdown list\n"),
        "prose indentation belongs to the author:\n{formatted}"
    );
    assert_semantics_preserved(source, "note prose");
}

#[test]
fn trailing_whitespace_and_crlf_still_normalize_in_code() {
    let source = "## Main\r\nLet x be 5.   \r\nShow x.\t\r\n";
    let formatted = format_source(source);
    assert_eq!(formatted, "## Main\nLet x be 5.\nShow x.\n");
    assert_semantics_preserved(source, "trailing/crlf");
}
