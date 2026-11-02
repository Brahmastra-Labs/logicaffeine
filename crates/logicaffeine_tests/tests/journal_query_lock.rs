//! ★ LOCK — a test reads the ACTION JOURNAL in ordinary LOGOS. ★
//!
//! Every turn is journalled (`turn_transaction_lock` proves the record is complete: the instance,
//! the action, per-field before→after, and the failure that rolled one back). Until now that
//! record was readable only from Rust — so the framework's own best evidence about what happened
//! was unavailable to the tests written in the language the pages are written in.
//!
//! `the journal` is a `Seq of Text` of action names in order, so every sequence operation LOGOS
//! already has works on it; `the journal shows A then B` is the ORDERING question, which a Seq
//! operation cannot ask without an index-of.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::dom;

fn output(src: &str) -> Vec<String> {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the program must run cleanly:\n{src}");
    out.output.lines().map(str::to_string).collect()
}

/// The output of a program whose clicked action DELIBERATELY fails.
///
/// ★ [`output`] asserts the run was clean, and these programs are not — an unhandled action
/// failure is now reported rather than dropped, because a page whose button silently does nothing
/// is the worst thing a framework can ship. The failure is reported at the END of the run, so the
/// program still finishes: the rollback happens, the journal records it, and a `Replay` carries on
/// past the trap. This asserts the failure WAS reported, so going quiet again would fail here too.
fn output_after_failure(src: &str) -> Vec<String> {
    dom::reset();
    let out = tw_outcome(src);
    assert!(
        out.error.is_some(),
        "this program's action fails on purpose — the failure must be reported, not dropped:\n{src}"
    );
    out.output.lines().map(str::to_string).collect()
}

const PAGE: &str = "\
## Component App
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
    To reset:
        Set n to 0.
    To boom:
        Set n to item 9 of [1].
View:
    <button id=\"up\" on-click={bump}>+</button>
    <button id=\"zero\" on-click={reset}>0</button>
    <button id=\"bad\" on-click={boom}>x</button>

## Main
Mount App.
";

#[test]
fn the_journal_is_a_sequence_of_the_actions_that_ran() {
    let out = output(&format!(
        "{PAGE}Click \"#up\".\nClick \"#zero\".\nShow the journal.\n"
    ));
    assert_eq!(out, ["[bump, reset]"]);
}

#[test]
fn an_empty_journal_is_an_empty_sequence_and_not_a_failure() {
    // A page nobody has driven yet is a legitimate state to ask about.
    assert_eq!(
        output(&format!("{PAGE}Show the count of the journal.\n")),
        ["0"]
    );
}

#[test]
fn the_journal_composes_with_the_sequence_operations_logos_already_has() {
    // The whole reason it is a value rather than a bespoke assertion: `count of`, `contains`,
    // `item N of` all work, so no journal-specific vocabulary is needed for the ordinary cases.
    let src = format!(
        "{PAGE}Click \"#up\".\nClick \"#up\".\nClick \"#zero\".\n\
         Show the count of the journal.\nShow item 3 of the journal.\n\
         Show the journal contains \"reset\".\n"
    );
    assert_eq!(output(&src), ["3", "reset", "true"]);
}

#[test]
fn a_failed_turn_is_in_the_journal_too() {
    // A turn that rolled back still HAPPENED, and a test asking what the page did must see it —
    // the rollback is about state, not about history.
    let src = format!("{PAGE}Click \"#bad\".\nShow the journal.\n");
    assert_eq!(output_after_failure(&src), ["[boom]"]);
}

#[test]
fn shows_then_answers_the_ordering_question() {
    let src = format!(
        "{PAGE}Click \"#up\".\nClick \"#zero\".\n\
         Show the journal shows bump then reset.\n\
         Show the journal shows reset then bump.\n"
    );
    assert_eq!(
        output(&src),
        ["true", "false"],
        "order is the whole question"
    );
}

#[test]
fn shows_then_is_false_when_either_action_never_ran() {
    let src = format!(
        "{PAGE}Click \"#up\".\n\
         Show the journal shows bump then reset.\n\
         Show the journal shows reset then bump.\n"
    );
    assert_eq!(output(&src), ["false", "false"]);
}

#[test]
fn shows_then_wants_them_in_that_order_not_merely_both_present() {
    // The failure mode a `contains and contains` assertion would have: both ran, wrong order.
    let src = format!(
        "{PAGE}Click \"#zero\".\nClick \"#up\".\n\
         Show the journal shows bump then reset.\n"
    );
    assert_eq!(output(&src), ["false"]);
}

#[test]
fn shows_then_allows_anything_in_between() {
    // "then" means AFTER, not IMMEDIATELY AFTER — a test that pinned adjacency would break every
    // time an unrelated turn was added between the two the test cares about.
    let src = format!(
        "{PAGE}Click \"#up\".\nClick \"#bad\".\nClick \"#zero\".\n\
         Show the journal shows bump then reset.\n"
    );
    assert_eq!(output_after_failure(&src), ["true"]);
}

#[test]
fn shows_then_accepts_the_same_action_twice() {
    // `bump then bump` needs a SECOND occurrence, not the same one counted twice.
    let twice =
        format!("{PAGE}Click \"#up\".\nClick \"#up\".\nShow the journal shows bump then bump.\n");
    assert_eq!(output(&twice), ["true"]);
    let once = format!("{PAGE}Click \"#up\".\nShow the journal shows bump then bump.\n");
    assert_eq!(output(&once), ["false"]);
}

#[test]
fn the_journal_reads_inside_a_test_block() {
    let src = format!(
        "{PAGE}## Test: the buttons run their actions in order\n    Mount App.\n    \
         Click \"#up\".\n    Click \"#zero\".\n    \
         Expect that the journal shows bump then reset.\n"
    );
    dom::reset();
    let out = tw_outcome(&src);
    assert_eq!(out.error, None, "the test block must run:\n{src}");
}
