//! LOCK — a timer's period is whatever its sentence says, on every tier; a named timer can be
//! started again (S15).
//!
//! `Every <period>:` / `After <period>:` take a period in seconds or any duration the language
//! writes (`500 milliseconds`), and the period may be computed — read from state or a prop when
//! the instance is drawn, as the reference evaluates it when it schedules the timer. A period the
//! lowered page could only read as a literal used to drop the timer without a word: the page ticked
//! on the reference and never in a browser. `Start the timer <name>.` restarts a named timer from a
//! whole period, whether it was stopped or still running.

use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};

/// Every `## Test` in `source` passes on the reference and traces identically on the lowered VM,
/// and the page builds for the browser.
fn passes_everywhere(source: &str) {
    let report = run_tests(source, &TestConfig::default())
        .unwrap_or_else(|e| panic!("the fixture must parse and run: {e}\n{source}"));
    assert!(
        !report.outcomes.is_empty(),
        "the fixture declares at least one ## Test"
    );
    for outcome in &report.outcomes {
        assert_eq!(
            outcome.verdict,
            Verdict::Passed,
            "`{}`: {:?}",
            outcome.name,
            outcome.failures
        );
    }
    let built = logicaffeine_compile::compile::compile_to_wasm(source);
    assert!(
        built.is_ok(),
        "the page builds for the browser: {:?}",
        built.err()
    );
}

#[test]
fn a_period_in_milliseconds_ticks_on_every_tier() {
    passes_everywhere(
        r##"## Page /

State:
    Let ticks be 0.

Actions:
    Every 500 milliseconds:
        Set ticks to ticks + 1.

View:
    <p id="n">{ticks}</p>

## Test: two ticks a second
    Advance time by 400 milliseconds.
    Expect that the text of "#n" is equal to "0".
    Advance time by 100 milliseconds.
    Expect that the text of "#n" is equal to "1".
    Advance time by 1 second.
    Expect that the text of "#n" is equal to "3".
"##,
    );
}

#[test]
fn a_period_read_from_state_ticks_on_every_tier() {
    passes_everywhere(
        r##"## Page /

State:
    Let every be 2.
    Let ticks be 0.

Actions:
    Every every seconds:
        Set ticks to ticks + 1.

View:
    <p id="n">{ticks}</p>

## Test: the period is the field's value when the page is drawn
    Advance time by 1 second.
    Expect that the text of "#n" is equal to "0".
    Advance time by 5 seconds.
    Expect that the text of "#n" is equal to "3".
"##,
    );
}

#[test]
fn a_period_read_from_a_prop_gives_each_instance_its_own_clock() {
    passes_everywhere(
        r##"## Component Ticker (step: Int)

State:
    Let ticks be 0.

Actions:
    Every step seconds:
        Set ticks to ticks + 1.

View:
    <p id="t{step}">{ticks}</p>

## Page /

View:
    <main><Ticker step={1}/><Ticker step={3}/></main>

## Test: each instance ticks on its own period
    Advance time by 6 seconds.
    Expect that the text of "#t1" is equal to "6".
    Expect that the text of "#t3" is equal to "2".
"##,
    );
}

#[test]
fn a_one_shot_after_a_computed_delay_fires_once_on_every_tier() {
    passes_everywhere(
        r##"## Page /

State:
    Let wait be 3.
    Let fired be 0.

Actions:
    After wait seconds:
        Set fired to fired + 1.

View:
    <p id="n">{fired}</p>

## Test: once, when its delay has passed
    Advance time by 2 seconds.
    Expect that the text of "#n" is equal to "0".
    Advance time by 5 seconds.
    Expect that the text of "#n" is equal to "1".
"##,
    );
}

#[test]
fn start_the_timer_restarts_a_stopped_named_timer_from_a_whole_period() {
    passes_everywhere(
        r##"## Page /

State:
    Let ticks be 0.

Actions:
    Every 10 seconds as clock:
        Set ticks to ticks + 1.
    To pause:
        Stop the timer clock.
    To resume:
        Start the timer clock.

View:
    <p id="n">{ticks}</p>
    <button id="pause" on-click={pause}>pause</button>
    <button id="resume" on-click={resume}>resume</button>

## Test: stopped, then started again from a whole period
    Advance time by 10 seconds.
    Expect that the text of "#n" is equal to "1".
    Click "#pause".
    Advance time by 30 seconds.
    Expect that the text of "#n" is equal to "1".
    Click "#resume".
    Advance time by 9 seconds.
    Expect that the text of "#n" is equal to "1".
    Advance time by 1 second.
    Expect that the text of "#n" is equal to "2".
"##,
    );
}

#[test]
fn start_the_timer_on_a_running_timer_restarts_its_period() {
    passes_everywhere(
        r##"## Page /

State:
    Let ticks be 0.

Actions:
    Every 10 seconds as clock:
        Set ticks to ticks + 1.
    To restart:
        Start the timer clock.

View:
    <p id="n">{ticks}</p>
    <button id="restart" on-click={restart}>restart</button>

## Test: a restart pushes the next tick a whole period out
    Advance time by 6 seconds.
    Click "#restart".
    Advance time by 6 seconds.
    Expect that the text of "#n" is equal to "0".
    Advance time by 4 seconds.
    Expect that the text of "#n" is equal to "1".
"##,
    );
}

#[test]
fn starting_a_timer_that_was_never_declared_is_refused_by_name() {
    let src = "## Page /\n\nState:\n    Let n be 0.\n\nActions:\n    To go:\n        Start the timer clock.\n\n\
               View:\n    <button id=\"go\" on-click={go}>go</button>\n";
    let err = format!(
        "{:?}",
        logicaffeine_compile::compile::compile_to_program(src).err()
    );
    assert!(
        err.contains("clock"),
        "the refusal names the timer nothing declares: {err}"
    );
}

/// Two timers due at the same moment fire in the order they were armed — the order their
/// instances were drawn — each as its own turn.
#[test]
fn timers_due_together_fire_in_the_order_they_were_armed() {
    passes_everywhere(
        r##"## Store Log
State:
    Let text be "".
Actions:
    To note (x: Text):
        Set text to text + x.

## Component Beat (tag: Text)
Use Log.
Actions:
    After 2 seconds:
        Call Log's note with tag.
View:
    <i>{tag}</i>

## Page /
Use Log.
View:
    <main><Beat tag="a"/><Beat tag="b"/><Beat tag="c"/><p id="log">{Log's text}</p></main>

## Test: one moment, arming order
    Advance time by 5 seconds.
    Expect that the text of "#log" is equal to "abc".
"##,
    );
}
