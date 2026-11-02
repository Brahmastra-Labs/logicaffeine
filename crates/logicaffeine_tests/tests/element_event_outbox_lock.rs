//! LOCK — an element event a turn asks for (`Publish`'s page-local delivery, `Focus`, `Blur`,
//! `Hover`) is delivered exactly when the asking statement RAN, with the value it had there, after
//! the turn has committed — on the reference tier and the lowered VM alike.
//!
//! The lowering appends to the page's outbox where the statement runs
//! (`elements.rs::queue_element_event`) and the turn's epilogue flushes it. Each row is one way a
//! compile-time queue would answer wrongly: a statement under a false `If` (G22 — a subscriber heard
//! a message nothing sent), a statement in a loop (one delivery per pass), a value naming a loop
//! variable, a store action that publishes for its caller, a sibling action inlined into the
//! caller's turn, and a subscriber that publishes in turn.

use logicaffeine_compile::compile::compile_to_wasm;
use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};

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
            "`{}` must pass on the reference tier and trace identically on the lowered VM: {:?}",
            outcome.name,
            outcome.failures
        );
    }
    let built = compile_to_wasm(source);
    assert!(
        built.is_ok(),
        "the page must build for the browser: {:?}",
        built.err()
    );
}

#[test]
fn a_publish_under_a_false_condition_delivers_nothing_and_a_true_one_delivers_once() {
    passes_everywhere(
        r##"
## Page /

State:
    Let draft be "".
    Let heard be 0.
    Let last be "none".

Actions:
    To send:
        If draft is not equal to "":
            Publish draft on "room".

    To fill:
        Set draft to "hi".

    When a message arrives on "room" (m: Text):
        Set heard to heard + 1.
        Set last to m.

View:
    <button id="send" on-click={send}>send</button>
    <button id="fill" on-click={fill}>fill</button>
    <p id="heard">{heard}</p>
    <p id="last">{last}</p>

## Test: nothing is sent while the draft is empty

Click "#send".
Expect that the text of "#heard" is equal to "0".

## Test: once the draft has text, one message arrives carrying it

Click "#fill".
Click "#send".
Expect that the text of "#heard" is equal to "1".
Expect that the text of "#last" is equal to "hi".
"##,
    );
}

#[test]
fn a_publish_in_a_loop_delivers_once_per_pass_with_that_passs_value() {
    passes_everywhere(
        r##"
## Page /

State:
    Let log be "".

Actions:
    To burst:
        Repeat for word in ["a", "b", "c"]:
            Publish word on "room".

    When a message arrives on "room" (m: Text):
        Set log to log + m.

View:
    <button id="go" on-click={burst}>go</button>
    <p id="log">{log}</p>

## Test: three passes, three messages, in order

Click "#go".
Expect that the text of "#log" is equal to "abc".
"##,
    );
}

#[test]
fn a_focus_under_a_false_condition_raises_no_focus_event() {
    passes_everywhere(
        r##"
## Page /

State:
    Let armed be false.
    Let focused be 0.

Actions:
    To maybeFocus:
        If armed:
            Focus "#field".

    To arm:
        Set armed to true.

    When "#field" is focused:
        Set focused to focused + 1.

View:
    <input id="field" type="text" aria-label="field">
    <button id="try" on-click={maybeFocus}>try</button>
    <button id="arm" on-click={arm}>arm</button>
    <p id="focused">{focused}</p>

## Test: an unarmed try focuses nothing, an armed one focuses once

Click "#try".
Expect that the text of "#focused" is equal to "0".
Click "#arm".
Click "#try".
Expect that the text of "#focused" is equal to "1".
"##,
    );
}

#[test]
fn a_store_action_that_publishes_is_heard_after_its_caller_commits() {
    passes_everywhere(
        r##"
## Store Basket

State:
    Let count be 0.

Actions:
    To add:
        Set count to count + 1.
        Publish "added" on "basket".

## Page /

Use Basket.

State:
    Let heard be 0.
    Let own be 0.

Actions:
    To buy:
        Call Basket's add.
        Set own to own + 1.

    When a message arrives on "basket" (m: Text):
        Set heard to heard + 1.

View:
    <button id="buy" on-click={buy}>buy</button>
    <p id="heard">{heard}</p>
    <p id="own">{own}</p>

## Test: the store's message arrives, and the caller's own write survives it

Click "#buy".
Click "#buy".
Expect that the text of "#heard" is equal to "2".
Expect that the text of "#own" is equal to "2".
"##,
    );
}

#[test]
fn a_sibling_action_that_publishes_is_heard_after_the_caller_commits() {
    passes_everywhere(
        r##"
## Page /

State:
    Let heard be 0.
    Let own be 0.

Actions:
    To go:
        Call sendNote.
        Set own to own + 1.

    To sendNote:
        Publish "x" on "room".

    When a message arrives on "room" (m: Text):
        Set heard to heard + 1.

View:
    <button id="go" on-click={go}>go</button>
    <p id="heard">{heard}</p>
    <p id="own">{own}</p>

## Test: the inlined sibling's message waits for the caller's write-back

Click "#go".
Expect that the text of "#heard" is equal to "1".
Expect that the text of "#own" is equal to "1".
"##,
    );
}

#[test]
fn a_subscriber_that_publishes_in_turn_is_heard_too() {
    passes_everywhere(
        r##"
## Page /

State:
    Let trail be "".

Actions:
    To start:
        Publish "1" on "first".

    When a message arrives on "first" (m: Text):
        Set trail to trail + "first:" + m + " ".
        Publish "2" on "second".

    When a message arrives on "second" (m: Text):
        Set trail to trail + "second:" + m.

View:
    <button id="go" on-click={start}>go</button>
    <p id="trail">{trail}</p>

## Test: the chain runs to its end, each turn after the one before committed

Click "#go".
Expect that the text of "#trail" is equal to "first:1 second:2".
"##,
    );
}
