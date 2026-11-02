//! ★ LOCK — a `keyed by` loop whose siblings share a key is a LOUD failure. ★
//!
//! Tristen's decision (`LANES.md`, 2026-09-25): never aliasing, never a silent positional fallback
//! — a `keyed by` loop that hands out the SAME key to two siblings names the loop, the key and both
//! (1-based) positions. `logicaffeine_language::ast::view::duplicate_keyed_loop_message` is the ONE
//! constructor the lowered loop calls, so a duplicate key fails identically wherever a page runs —
//! the loop checks every key before any item renders and fails the moment a repeat turns up.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::dom;
use logicaffeine_language::ast::view::duplicate_keyed_loop_message;

const PAGE: &str = "## An Item has:\n    an id, which is Int.\n    a label, which is Text.\n\n\
## Component App\n\
State:\n    Let items be [a new Item with id 1 and label \"a\", a new Item with id 2 and \
label \"b\", a new Item with id 1 and label \"c\"].\n\
View:\n    <ul>{for each t in items, keyed by t's id:}<li>{t's label}</li>{end}</ul>\n\n\
## Main\nMount App.\n";

/// Positions 1 and 3 (1-based, this campaign's own convention, matching `at index`) both key `"1"` —
/// the exact wording `duplicate_keyed_loop_message` produces, so a future edit to either the
/// constructor or `interpreter.rs`'s call site is caught by an exact-string mismatch, not just a
/// vague "still fails" pass.
#[test]
fn a_keyed_loop_with_two_siblings_sharing_a_key_fails_loudly() {
    let out = tw_outcome(PAGE);
    let want = duplicate_keyed_loop_message("items", "1", 1, 3);
    assert_eq!(
        out.error.as_deref(),
        Some(want.as_str()),
        "must fail with the ONE shared constructor's own wording, byte-identical: output={:?}",
        out.output
    );
}

/// The check runs BEFORE any item renders (`duplicate_keyed_loop_message`'s own doc comment: the
/// tree-walker "evaluates every key up front and checks for a repeat before rendering") — a page
/// that would otherwise mount the FIRST two (distinct-keyed) `<li>`s and only then hit the duplicate
/// third must still commit NOTHING, not a partial `<ul>` with two children. Asserted on the real DOM
/// tree (`dom::render()`), not just the tree-walker's own text output buffer, since a `<li>` with no
/// `Show` inside it would leave that buffer empty either way.
#[test]
fn the_check_runs_before_any_item_renders() {
    dom::reset();
    let out = tw_outcome(PAGE);
    assert!(out.error.is_some(), "must fail");
    assert!(
        !dom::render().contains("<li>"),
        "a duplicate key must be caught before any `<li>` commits to the document: {}",
        dom::render()
    );
}
