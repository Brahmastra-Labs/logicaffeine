//! ★ LOCK — a selector outside the subset is REFUSED, not silently matched against nothing. ★
//!
//! `lgx/selector.rs` opens by saying what it understands: *"a descendant chain of compounds, each
//! an optional tag plus any number of `#id`, `.class`, and `[attr]` qualifiers. **No sibling
//! combinators, no pseudo-classes** — those belong to stylesheets."* That is the right subset, and
//! the parser did not enforce a word of it.
//!
//! ★ IT READ THEM AS TAG NAMES. `compound()` takes the leading run up to `#`, `.` or `[` as the
//! tag, so:
//!
//! ```text
//! "ul > li"        →  three tokens; `>` becomes a TAG named ">"        →  0 matches
//! ".row, #x"       →  a class named "row,"                             →  0 matches
//! "li:first-child" →  a tag named "li:first-child"                     →  0 matches
//! ```
//!
//! Every one of them answers **0**, `false`, or "nothing found" — the same answer a correct
//! selector gives when the page genuinely has none. So a page test written with ordinary CSS
//! habits passes, forever, having asserted nothing:
//!
//! ```text
//! Expect the count of "ul > li" to be 0.     ← passes, and would pass with 500 items on the page
//! ```
//!
//! ★ AND BOTH READERS SWALLOWED IT. `parse` answers `None` for a malformed selector, and its two
//! callers each discarded that: the runtime engine returned an empty match list, and the static
//! check `continue`d — so the one component that could have said something skipped exactly the
//! selectors it could not understand.
//!
//! A selector that is IN the subset and matches nothing is a different thing and stays legal: `0`
//! is the honest answer to "how many `.rwo` are there". What is refused is syntax the engine cannot
//! evaluate at all, which is not an answer of 0 — it is no answer.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

const PAGE: &str = "## Component App\nState:\n\x20   Let n be 1.\n\
    View:\n\x20   <div class=\"card\"><ul><li class=\"row\">a</li><li class=\"row\">b</li></ul>\
    <input id=\"x\"></div>\n\n## Main\nMount App.\n";

/// The refusal both driving tiers give for `query`, asserted to be the SAME on each.
fn refusal(subject: &str, query: &str) -> String {
    let src = format!("{PAGE}Show {query}.\n");
    dom::reset();
    let tw = tw_outcome(&src)
        .error
        .unwrap_or_else(|| panic!("`{subject}` must be refused on the tree-walker:\n{src}"));
    dom::reset();
    let vm = vm_outcome(&src)
        .error
        .unwrap_or_else(|| panic!("`{subject}` must be refused on the VM too:\n{src}"));
    assert!(
        vm.contains(&tw) || tw.contains(&vm),
        "two tiers refusing one thing in different words send whoever reads one looking for the \
         other:\n  tw: {tw}\n  vm: {vm}"
    );
    tw
}

fn answers(subject: &str, query: &str, expected: &str) {
    dom::reset();
    let src = format!("{PAGE}Show {query}.\n");
    let out = tw_outcome(&src);
    assert_eq!(out.error, None, "`{subject}` must run:\n{src}");
    assert_eq!(out.output.trim(), expected, "`{subject}`");
}

#[test]
fn a_child_combinator_is_refused() {
    let err = refusal("a child combinator", "the count of \"ul > li\"");
    assert!(
        err.contains("ul > li"),
        "the refusal must quote the selector: {err}"
    );
    let lower = err.to_lowercase();
    assert!(
        lower.contains("combinator") || lower.contains('>'),
        "and name what it does not understand: {err}"
    );
    assert!(
        lower.contains("descendant") || lower.contains("space") || lower.contains("`ul li`"),
        "and say what to write instead — a descendant chain is the supported form: {err}"
    );
}

#[test]
fn the_other_combinators_are_refused_too() {
    // ★ NOT A `>` SPECIAL CASE. A fix that knew only about the child combinator would leave the
    // adjacent and general-sibling ones reading as tag names, which is the same silence.
    refusal("an adjacent sibling", "the count of \"li + li\"");
    refusal("a general sibling", "the count of \"li ~ li\"");
}

#[test]
fn a_selector_group_is_refused() {
    let err = refusal("a group", "the count of \".row, #x\"");
    assert!(err.contains(','), "the refusal must name the comma: {err}");
}

#[test]
fn a_pseudo_class_is_refused() {
    let err = refusal("a pseudo-class", "the count of \"li:first-child\"");
    let lower = err.to_lowercase();
    assert!(
        lower.contains("pseudo") || lower.contains(':'),
        "the refusal must name it: {err}"
    );
    assert!(
        lower.contains("style"),
        "and point at `Style:`, where real CSS is read verbatim: {err}"
    );
}

#[test]
fn a_universal_selector_is_refused() {
    refusal("a universal selector", "the count of \"*\"");
}

#[test]
fn every_page_question_refuses_it_not_just_the_count() {
    // ★ THE POSITION MUST NOT MATTER. `the count of`, `the text of`, `the value of` and the
    // `is …` questions all take a selector, and a check that landed on one of them would be the
    // arbitrary-list failure this codebase keeps finding.
    refusal("the text of", "the text of \"ul > li\"");
    refusal("the value of", "the value of \"input > x\"");
    refusal("is visible", "\"ul > li\" is visible");
    refusal("is hidden", "\"ul > li\" is hidden");
}

#[test]
fn a_handler_trigger_refuses_it_too() {
    // The other reader of the same parse — `check_selectors_exist` used to `continue` past exactly
    // the selectors it could not understand, so a handler on `ul > li` bound to nothing at all and
    // said nothing.
    // A selector trigger lives in `Actions:`, beside the actions it drives.
    let src = "## Component App\nState:\n\x20   Let n be 0.\n\
               Actions:\n\x20   When \"ul > li\" is clicked:\n\x20       Set n to n + 1.\n\
               View:\n\x20   <ul><li>a</li></ul>\n\n## Main\nMount App.\n";
    dom::reset();
    let err = tw_outcome(src)
        .error
        .unwrap_or_else(|| panic!("must be refused:\n{src}"));
    assert!(err.contains("ul > li"), "{err}");
}

// ---------------------------------------------------------------------------------------------
// ★ THE SUBSET ITSELF — everything that IS supported keeps working, and a valid selector that
// matches nothing keeps answering 0. The refusal has to stay narrow or it breaks every page test.
// ---------------------------------------------------------------------------------------------

#[test]
fn the_supported_subset_is_unchanged() {
    answers("a class", "the count of \".row\"", "2");
    answers("a tag", "the count of \"li\"", "2");
    answers("an id", "the count of \"#x\"", "1");
    answers("a descendant chain", "the count of \".card li\"", "2");
    answers("a deeper chain", "the count of \"div ul li\"", "2");
    answers("an attribute presence", "the count of \"[id]\"", "1");
    answers("an attribute value", "the count of \"[id=x]\"", "1");
    answers("a compound", "the count of \"li.row\"", "2");
    answers("a tag with an id", "the count of \"input#x\"", "1");
}

#[test]
fn a_valid_selector_that_matches_nothing_still_answers_zero() {
    // ★ THE NARROWNESS GUARD, and the distinction the whole file rests on. `.rwo` is a typo, but it
    // is a WELL-FORMED selector, and "how many `.rwo` are on this page" has an honest answer: none.
    // Refusing it would be refusing the ordinary case of asserting something is absent.
    answers("a class nothing has", "the count of \".rwo\"", "0");
    answers("a tag nothing has", "the count of \"section\"", "0");
    answers("an id nothing has", "the count of \"#nope\"", "0");
    answers("absence as a question", "\"#nope\" is visible", "false");
}
