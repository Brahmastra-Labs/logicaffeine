//! ★ LOCK — a "did you mean" is offered only for a word the parser did not recognise. ★
//!
//! The suggestion is a spelling: the word under the error's span, measured against the words the
//! language knows. It means something only when the error IS about that word — a quantifier, a
//! copula, a verb the parser expected and found misspelled. A refusal composed for a construct
//! (`ParseErrorKind::Custom`: a `View:` that does not open with markup, a component placed wrong)
//! points at a word the author spelled correctly, and a spelling offered there is noise that
//! sends the author hunting for a typo they never made — `View:` answered "did you mean 'few'?".
//!
//! Every surface reads the one suggester (`ParseError::suggestion`): the terminal's caret block
//! (`display_with_source`) and the `logos-diagnostic/1` object (`diagnostic_for`).

#[path = "diagnostic_location/rows.rs"]
mod rows;

use logicaffeine_compile::compile::compile_to_wasm;
use logicaffeine_language::diagnostic::diagnostic_for;
use logicaffeine_language::error::{ParseError, ParseErrorKind};
use logicaffeine_language::token::Span;
use logicaffeine_language::Interner;

/// `error` as each surface shows it: the suggester's own answer, the caret block's help line, and
/// the diagnostic object's suggestion.
fn suggestions(error: &ParseError, source: &str) -> (Option<&'static str>, bool, Option<String>) {
    let caret = error.display_with_source(source);
    let object = diagnostic_for(error, source, Some("app.lg"), &Interner::new());
    (
        error.suggestion(source),
        caret.contains("did you mean"),
        object.suggestion,
    )
}

/// The span of the first occurrence of `word` in `source`.
fn span_of(source: &str, word: &str) -> Span {
    let at = source
        .find(word)
        .unwrap_or_else(|| panic!("{word:?} is in the fixture"));
    Span::new(at, at + word.len())
}

#[test]
fn a_refusal_composed_for_a_construct_offers_no_spelling() {
    let source = "## Component App\nView:\n    Hello there\n";
    let error = ParseError {
        kind: ParseErrorKind::Custom("A `View:` section holds HTML.".to_string()),
        span: span_of(source, "View"),
    };
    assert_eq!(
        suggestions(&error, source),
        (None, false, None),
        "`View` is spelled right; the refusal is about the section, not the word"
    );
}

#[test]
fn a_page_refused_at_a_correctly_spelled_word_is_shown_no_spelling() {
    // The view's first line is the one word `Some`, which the suggester knows: the refusal's span
    // covers exactly that word, so a blanket suggester would offer `some` for it.
    let source = rows::component("Some\n<p>{n}</p>");
    let error = compile_to_wasm(&source).expect_err("a `View:` that opens with a word is refused");
    assert_eq!(
        &source[error.span.start..error.span.end],
        "Some",
        "the refusal points at the view's first line"
    );
    assert_eq!(suggestions(&error, &source), (None, false, None));
}

#[test]
fn a_misspelled_word_the_parser_expected_is_still_offered_its_spelling() {
    let source = "Every man iz mortal.";
    let error = ParseError {
        kind: ParseErrorKind::ExpectedCopula,
        span: span_of(source, "iz"),
    };
    let (word, caret, object) = suggestions(&error, source);
    assert_eq!(
        word,
        Some("is"),
        "the copula the parser expected, one edit away"
    );
    assert!(caret, "the caret block carries it");
    assert_eq!(
        object.as_deref(),
        Some("did you mean 'is'?"),
        "and so does the diagnostic object"
    );
}
