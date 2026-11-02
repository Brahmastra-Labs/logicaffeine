//! ★ LOCK — `Replay these actions: a, b.` re-runs a sequence against a page. ★
//!
//! The journal's reason for existing. Every turn is recorded (`turn_transaction_lock`) and
//! readable (`journal_query_lock`); replay is what makes that record USEFUL — a bug report
//! becomes a test by naming the actions that produced it, and the dev panel's time travel is the
//! same mechanism with a slider on it.
//!
//! Each replayed action is a REAL TURN: it rolls back if it traps, it is journalled, and the page
//! repaints. A replay that bypassed the turn machinery would reproduce a different bug from the
//! one being investigated.

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

fn failure(src: &str) -> String {
    dom::reset();
    tw_outcome(src)
        .error
        .unwrap_or_else(|| panic!("expected a failure:\n{src}"))
}

const PAGE: &str = "\
## Component App
State:
    Let n be 0.
    Let log be \"\".
Actions:
    To bump:
        Set n to n + 1.
        Set log to log + \"b\".
    To reset:
        Set n to 0.
        Set log to log + \"r\".
    To boom:
        Set n to 99.
        Set log to item 9 of [\"x\"].
View:
    <p id=\"n\">{n}</p>
    <p id=\"log\">{log}</p>

## Main
Mount App.
";

#[test]
fn a_replay_runs_each_action_in_order() {
    let out = output(&format!(
        "{PAGE}Replay these actions: bump, bump, reset, bump.\nShow the text of \"#n\".\n\
         Show the text of \"#log\".\n"
    ));
    assert_eq!(out, ["1", "bbrb"], "the order is the whole point");
}

#[test]
fn a_single_action_replays() {
    let out = output(&format!(
        "{PAGE}Replay these actions: bump.\nShow the text of \"#n\".\n"
    ));
    assert_eq!(out, ["1"]);
}

#[test]
fn each_replayed_action_is_a_real_turn_in_the_journal() {
    // Not a shortcut around the turn machinery: a replay that wrote state directly would
    // reproduce a different bug from the one being investigated.
    let out = output(&format!(
        "{PAGE}Replay these actions: bump, reset.\nShow the journal.\n"
    ));
    assert_eq!(out, ["[bump, reset]"]);
}

#[test]
fn a_trapping_action_rolls_back_and_the_replay_carries_on() {
    // The transaction rule holds under replay: `boom` sets n to 99 and then traps, so n goes back
    // to what the turn started from and the actions after it still run. A replay that stopped at
    // the first trap could not reproduce a sequence whose whole point is what happens AFTER one.
    let out = output_after_failure(&format!(
        "{PAGE}Replay these actions: bump, boom, bump.\nShow the text of \"#n\".\n"
    ));
    assert_eq!(
        out,
        ["2"],
        "boom's 99 was rolled back and the second bump still ran"
    );
}

#[test]
fn the_page_repaints_after_a_replay() {
    // What the reader SEES has to match what the state says.
    let out = output(&format!(
        "{PAGE}Replay these actions: bump, bump, bump.\nShow the text of \"#n\".\n"
    ));
    assert_eq!(out, ["3"]);
}

#[test]
fn an_action_the_component_does_not_have_is_refused_by_name() {
    // A typo in a replay would otherwise leave every later assertion describing a page nobody
    // touched — the same rule `Click` on a missing selector follows.
    let err = failure(&format!("{PAGE}Replay these actions: bump, nosuch.\n"));
    assert!(
        err.contains("nosuch"),
        "the refusal names the action: {err}"
    );
}

#[test]
fn replaying_before_anything_is_mounted_is_refused() {
    let src = "## Component App\nState:\n    Let n be 0.\nActions:\n    To bump:\n        \
        Set n to n + 1.\nView:\n    <p>{n}</p>\n\n\
        ## Main\nReplay these actions: bump.\n";
    let err = failure(src);
    assert!(
        err.to_lowercase().contains("mount"),
        "the refusal says the page has to exist first: {err}"
    );
}

#[test]
fn a_replay_composes_with_the_journal_assertion() {
    // The pair the plan describes: replay a sequence, then assert what it did.
    let src = format!(
        "{PAGE}## Test: the sequence from the bug report\n    Mount App.\n    \
         Replay these actions: bump, reset.\n    \
         Expect that the journal shows bump then reset.\n    \
         Expect that the text of \"#n\" is equal to \"0\".\n"
    );
    dom::reset();
    assert_eq!(
        tw_outcome(&src).error,
        None,
        "the test block must run:\n{src}"
    );
}
