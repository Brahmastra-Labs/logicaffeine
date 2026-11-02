//! ★ LOCK — one event is one TURN, and a turn is a TRANSACTION. ★
//!
//! Everything a page does happens inside a turn: the actions run to completion, derived fields
//! recompute, change handlers run once, and the DOM is repainted once. If any action traps, the
//! instance's record is restored to exactly what it held when the turn began — no half-applied
//! state, no dead page. The failure is recorded and `When an action fails` gets to say what the
//! user sees. This is the property that makes a LOGOS page safe to write without defensive code.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::{dom, world};

fn rendered(src: &str) -> String {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the program must run cleanly:\n{src}");
    let html = dom::render();
    html.strip_prefix("<body>")
        .and_then(|h| h.strip_suffix("</body>"))
        .unwrap_or(&html)
        .to_string()
}

/// Run `src` and return the action journal it produced.
fn journal(src: &str) -> Vec<world::JournalEntry> {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the program must run cleanly:\n{src}");
    world::journal()
}

const TRAPPING: &str = "\
## Component App
State:
    Let a be 0.
    Let b be 0.
    Let error be \"\".
Actions:
    To half:
        Set a to 1.
        Set b to item 9 of [1].
    When an action fails (e: Text):
        Set error to \"caught\".
View:
    <p id=\"n\">{a}-{b}-{error}</p>
    <button id=\"go\" on-click={half}>x</button>

## Main
Mount App.
Click \"#go\".
";

#[test]
fn a_trap_restores_every_field_the_turn_had_touched() {
    // `a` WAS set to 1 before the trap. A turn is all-or-nothing, so it goes back to 0.
    assert!(
        rendered(TRAPPING).contains("<p id=\"n\">0-0-caught</p>"),
        "rendered: {}",
        rendered(TRAPPING)
    );
}

#[test]
fn the_page_stays_alive_after_a_trap() {
    let src = TRAPPING.replace(
        "    To half:",
        "    To fine:\n        Set b to 7.\n    To half:",
    );
    let src = src.replace(
        "<button id=\"go\" on-click={half}>x</button>",
        "<button id=\"go\" on-click={half}>x</button>\n    <button id=\"ok\" on-click={fine}>ok</button>",
    );
    let src = format!("{src}Click \"#ok\".\n");
    assert!(
        rendered(&src).contains("0-7-caught"),
        "the next turn works normally: {}",
        rendered(&src)
    );
}

#[test]
fn a_clean_turn_commits_everything_it_did() {
    let src = "\
## Component App
State:
    Let a be 0.
    Let b be 0.
Actions:
    To both:
        Set a to 1.
        Set b to 2.
View:
    <p id=\"n\">{a}-{b}</p>
    <button id=\"go\" on-click={both}>x</button>

## Main
Mount App.
Click \"#go\".
";
    assert!(
        rendered(src).contains("<p id=\"n\">1-2</p>"),
        "rendered: {}",
        rendered(src)
    );
}

#[test]
fn a_trap_in_a_change_handler_does_not_undo_the_action_that_caused_it() {
    // The action committed; the handler is a separate step of the same turn and its own failure
    // does not reach back and rewrite history.
    let src = "\
## Component App
State:
    Let n be 0.
    Let seen be 0.
Actions:
    To bump:
        Set n to n + 1.
    When n changes:
        Set seen to item 9 of [1].
View:
    <p id=\"n\">{n}-{seen}</p>
    <button id=\"go\" on-click={bump}>x</button>

## Main
Mount App.
Click \"#go\".
";
    assert!(
        rendered(src).contains("<p id=\"n\">1-0</p>"),
        "rendered: {}",
        rendered(src)
    );
}

#[test]
fn the_journal_records_every_turn_in_order() {
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
    To reset:
        Set n to 0.
View:
    <p>{n}</p>
    <button id=\"up\" on-click={bump}>+</button>
    <button id=\"z\" on-click={reset}>0</button>

## Main
Mount App.
Click \"#up\".
Click \"#up\".
Click \"#z\".
";
    let entries = journal(src);
    let names: Vec<&str> = entries.iter().map(|e| e.action.as_str()).collect();
    assert_eq!(names, ["bump", "bump", "reset"], "in the order they ran");
}

#[test]
fn a_journal_entry_carries_the_before_and_after_of_each_changed_field() {
    let src = "\
## Component App
State:
    Let n be 0.
    Let other be 5.
Actions:
    To bump:
        Set n to n + 1.
View:
    <p>{n}{other}</p>
    <button id=\"go\" on-click={bump}>+</button>

## Main
Mount App.
Click \"#go\".
";
    let entries = journal(src);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].changes,
        vec![("n".to_string(), "0".to_string(), "1".to_string())],
        "only what actually changed, with both sides"
    );
}

#[test]
fn a_failed_turn_is_journalled_with_its_failure() {
    let entries = journal(TRAPPING);
    assert_eq!(entries.len(), 1, "one turn ran");
    assert_eq!(entries[0].action, "half");
    assert!(
        entries[0].failed.is_some(),
        "the failure is recorded: {:?}",
        entries[0]
    );
    assert!(
        entries[0].changes.is_empty(),
        "a rolled-back turn changed nothing: {:?}",
        entries[0]
    );
}

#[test]
fn the_journal_names_the_instance_a_turn_ran_against() {
    let src = "\
## Component Counter
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <li class=\"c\" on-click={bump}>{n}</li>

## Component App
View:
    <ul><Counter/><Counter/></ul>

## Main
Mount App.
Click \"li\".
";
    let entries = journal(src);
    assert_eq!(entries.len(), 1);
    assert!(
        entries[0].instance.contains("Counter"),
        "the path says which instance: {:?}",
        entries[0].instance
    );
}

#[test]
fn a_store_turn_is_journalled_like_any_other() {
    let src = "\
## Store Session
State:
    Let name be \"guest\".
Actions:
    To login (who: Text):
        Set name to who.

## Component App
Use Session.
Actions:
    To signIn:
        Call Session's login with \"ada\".
View:
    <p>{Session's name}</p>
    <button id=\"go\" on-click={signIn}>in</button>

## Main
Mount App.
Click \"#go\".
";
    let entries = journal(src);
    let login = entries
        .iter()
        .find(|e| e.action == "login")
        .unwrap_or_else(|| panic!("the store's turn is in the journal: {entries:?}"));
    assert!(
        login.instance.contains("Session"),
        "against the store: {}",
        login.instance
    );
    assert_eq!(
        login.changes,
        vec![("name".to_string(), "guest".to_string(), "ada".to_string())]
    );
}

#[test]
fn a_timer_turn_is_journalled_like_any_other() {
    // Every way state changes goes through a turn, so the journal is a COMPLETE record — that is
    // what makes replay and time travel possible rather than approximate.
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    Every 10 seconds:
        Set n to n + 1.
View:
    <p>{n}</p>

## Test: A timer turn is recorded
    Mount App.
    Advance time by 10 seconds.
    Expect that the text of \"p\" is equal to \"1\".
";
    // The test runner's session writes its journal on the session's own thread; read inside that
    // same session, the journal is the one the run left.
    let (report, entries) = logicaffeine_compile::interpreter::on_session_stack(|| {
        dom::reset();
        let report = logicaffeine_compile::testrun::run_tests(
            src,
            &logicaffeine_compile::testrun::TestConfig::default(),
        )
        .expect("the suite parses");
        (report, world::journal())
    });
    assert_eq!(
        report.outcomes[0].verdict,
        logicaffeine_compile::testrun::Verdict::Passed,
        "{:?}",
        report.outcomes[0].failures
    );
    assert_eq!(
        entries.len(),
        1,
        "the timer's turn is recorded: {entries:?}"
    );
    assert_eq!(
        entries[0].changes,
        vec![("n".to_string(), "0".to_string(), "1".to_string())]
    );
}
