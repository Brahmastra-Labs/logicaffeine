//! ★ LOCK — `Every N seconds:` and `After N seconds:` are page timers on the SEALED CLOCK. ★
//!
//! A timer is an event like any other: it opens a turn, so its state changes roll back on a
//! trap and land in the action journal. Because it runs on the sealed world's virtual clock,
//! `Advance time by` drives it exactly — a test of a 60-second refresh costs no real seconds
//! and can never flake.

use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};

/// Run one `## Test` and return its failures (empty when it passed).
fn failures(suite: &str) -> Vec<String> {
    let report = run_tests(suite, &TestConfig::default()).expect("the suite parses");
    let outcome = &report.outcomes[0];
    match outcome.verdict {
        Verdict::Passed => Vec::new(),
        _ => {
            let mut f = outcome.failures.clone();
            if f.is_empty() {
                f.push(format!("{:?}", outcome.verdict));
            }
            f
        }
    }
}

fn passes(suite: &str) {
    let f = failures(suite);
    assert!(
        f.is_empty(),
        "expected a green test:\n{suite}\nfailures: {f:#?}"
    );
}

#[test]
fn a_repeating_timer_fires_once_per_period() {
    passes(
        "\
## Component App
State:
    Let ticks be 0.
Actions:
    Every 10 seconds:
        Set ticks to ticks + 1.
View:
    <p id=\"n\">{ticks}</p>

## Test: A timer ticks on the virtual clock
    Mount App.
    Expect that the text of \"#n\" is equal to \"0\".
    Advance time by 10 seconds.
    Expect that the text of \"#n\" is equal to \"1\".
    Advance time by 30 seconds.
    Expect that the text of \"#n\" is equal to \"4\".
",
    );
}

#[test]
fn a_timer_does_not_fire_before_its_period_elapses() {
    passes(
        "\
## Component App
State:
    Let ticks be 0.
Actions:
    Every 10 seconds:
        Set ticks to ticks + 1.
View:
    <p id=\"n\">{ticks}</p>

## Test: A timer waits for its whole period
    Mount App.
    Advance time by 9 seconds.
    Expect that the text of \"#n\" is equal to \"0\".
    Advance time by 1 seconds.
    Expect that the text of \"#n\" is equal to \"1\".
",
    );
}

#[test]
fn a_one_shot_timer_fires_exactly_once() {
    passes(
        "\
## Component App
State:
    Let ticks be 0.
Actions:
    After 5 seconds:
        Set ticks to ticks + 1.
View:
    <p id=\"n\">{ticks}</p>

## Test: An After timer is one-shot
    Mount App.
    Advance time by 100 seconds.
    Expect that the text of \"#n\" is equal to \"1\".
",
    );
}

#[test]
fn a_named_timer_can_be_stopped() {
    passes(
        "\
## Component App
State:
    Let ticks be 0.
Actions:
    Every 10 seconds as refresh:
        Set ticks to ticks + 1.
    When \"#go\" is clicked:
        Stop the timer refresh.
View:
    <p id=\"n\">{ticks}</p>
    <button id=\"go\">stop</button>

## Test: A stopped timer never fires again
    Mount App.
    Advance time by 10 seconds.
    Expect that the text of \"#n\" is equal to \"1\".
    Click \"#go\".
    Advance time by 100 seconds.
    Expect that the text of \"#n\" is equal to \"1\".
",
    );
}

#[test]
fn a_timer_runs_as_a_turn_so_a_trap_rolls_it_back() {
    passes(
        "\
## Component App
State:
    Let ticks be 0.
    Let error be \"\".
Actions:
    Every 10 seconds:
        Set ticks to ticks + 1.
        Set ticks to item 9 of [1].
    When an action fails (e: Text):
        Set error to \"caught\".
View:
    <p id=\"n\">{ticks}</p>
    <p id=\"e\">{error}</p>

## Test: A trapping timer leaves the state it started from
    Mount App.
    Advance time by 10 seconds.
    Expect that the text of \"#n\" is equal to \"0\".
    Expect that the text of \"#e\" is equal to \"caught\".
",
    );
}

#[test]
fn a_timer_updates_the_page() {
    passes(
        "\
## Component App
State:
    Let items be a new Seq of Text.
Actions:
    Every 10 seconds:
        Push \"tick\" to items.
View:
    <ul>{for each x in items:}<li class=\"row\">{x}</li>{end}</ul>

## Test: The page repaints after a timer turn
    Mount App.
    Advance time by 20 seconds.
    Expect that the count of \".row\" is 2.
",
    );
}

#[test]
fn stopping_a_timer_that_was_never_declared_is_a_clean_error() {
    // A timer name is checked where the component is written, not when a click happens to
    // reach it — so a typo is a program error found before the page ever runs.
    let msg = run_tests(
        "\
## Component App
State:
    Let n be 0.
Actions:
    Every 10 seconds as refresh:
        Set n to n + 1.
    When \"#go\" is clicked:
        Stop the timer ghost.
View:
    <button id=\"go\">x</button>

## Test: Stopping an unknown timer says so
    Mount App.
    Expect that 1 is 1.
",
        &TestConfig::default(),
    )
    .expect_err("an unknown timer name is a program error");
    assert!(msg.contains("ghost"), "the message names the timer: {msg}");
    assert!(
        msg.contains("refresh"),
        "and says which timer the component does have: {msg}"
    );
    assert!(
        msg.contains("as ghost"),
        "and shows how to declare one: {msg}"
    );
}

#[test]
fn a_timer_may_be_stopped_before_it_is_declared_in_the_file() {
    passes(
        "\
## Component App
State:
    Let ticks be 0.
Actions:
    When \"#go\" is clicked:
        Stop the timer refresh.
    Every 10 seconds as refresh:
        Set ticks to ticks + 1.
View:
    <p id=\"n\">{ticks}</p>
    <button id=\"go\">stop</button>

## Test: Order inside a component does not matter
    Mount App.
    Click \"#go\".
    Advance time by 100 seconds.
    Expect that the text of \"#n\" is equal to \"0\".
",
    );
}

/// A component that appears at `t` — shown by a click — with a timer of its own.
fn appearing(timer: &str, body: &str, script: &str) -> String {
    format!(
        "## Store Log\nState:\n    Let text be \"\".\nActions:\n    To note (x: Text):\n        Set text to text + x.\n\n\
         ## Component Late (tag: Text)\nUse Log.\nActions:\n    {timer}\n        {body}\n\
         View:\n    <i class=\"late\">{{tag}}</i>\n\n\
         ## Page /\nUse Log.\nState:\n    Let shown be a new Seq of Text.\n\
         Actions:\n    To show (tag: Text):\n        Push tag to shown.\n\
         View:\n    <main><button id=\"a\" on-click={{show(\"a\")}}>a</button><button id=\"b\" on-click={{show(\"b\")}}>b</button>\
         {{for each s in shown, keyed by s:}}<Late tag={{s}}/>{{end}}<p id=\"log\">{{Log's text}}</p></main>\n\n\
         ## Test: t\n{script}"
    )
}

/// ★ A TIMER COUNTS FROM WHEN IT WAS ARMED. A component that appears two seconds into the page
/// with `After 3 seconds:` fires at five seconds, as a browser's `setTimeout` does — not at the
/// next beat of a clock started with the page.
#[test]
fn a_timer_armed_after_mount_counts_its_whole_period_from_when_it_was_armed() {
    passes(&appearing(
        "After 3 seconds:",
        "Call Log's note with tag.",
        "Advance time by 2 seconds.\nClick \"#a\".\nAdvance time by 2 seconds.\n\
         Expect that the text of \"#log\" is equal to \"\".\nAdvance time by 1 second.\n\
         Expect that the text of \"#log\" is equal to \"a\".\n",
    ));
}

/// Two instances armed two seconds apart each keep their own clock.
#[test]
fn two_instances_armed_apart_keep_their_own_clocks() {
    passes(&appearing(
        "After 3 seconds:",
        "Call Log's note with tag.",
        "Click \"#a\".\nAdvance time by 2 seconds.\nClick \"#b\".\nAdvance time by 1 second.\n\
         Expect that the text of \"#log\" is equal to \"a\".\nAdvance time by 1 second.\n\
         Expect that the text of \"#log\" is equal to \"a\".\nAdvance time by 1 second.\n\
         Expect that the text of \"#log\" is equal to \"ab\".\n",
    ));
}

/// A repeating timer armed mid-way keeps its own phase: armed at 1s every 2s, it beats at 3s and 5s.
#[test]
fn a_repeating_timer_armed_mid_period_beats_on_its_own_phase() {
    passes(&appearing(
        "Every 2 seconds:",
        "Call Log's note with tag.",
        "Advance time by 1 second.\nClick \"#a\".\nAdvance time by 1 second.\n\
         Expect that the text of \"#log\" is equal to \"\".\nAdvance time by 1 second.\n\
         Expect that the text of \"#log\" is equal to \"a\".\nAdvance time by 2 seconds.\n\
         Expect that the text of \"#log\" is equal to \"aa\".\n",
    ));
}

/// Within one step of the clock, timers fire in the order they come due — whichever component or
/// instance they belong to — each as its own turn.
#[test]
fn timers_fire_in_due_order_within_one_advance() {
    passes(&appearing(
        "After 3 seconds:",
        "Call Log's note with tag.",
        "Click \"#a\".\nAdvance time by 1 second.\nClick \"#b\".\nAdvance time by 10 seconds.\n\
         Expect that the text of \"#log\" is equal to \"ab\".\n",
    ));
}
