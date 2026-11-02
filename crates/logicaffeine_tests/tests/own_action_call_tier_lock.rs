//! LOCK — `Call <action>.` naming one of the SAME component's actions by its bare name runs that
//! action NESTED in the caller's turn, identically on the reference tier and the lowered VM, and
//! builds for the browser.
//!
//! The lowering inlines the sibling's body into the caller's turn
//! (`Parser::inline_own_action_call`), against the caller's own bindings. Each row is one way that
//! inlining could come apart from a nested call: repeated
//! calls, arguments, a parameter shadowing a state field, an early `Return` inside the sibling, a
//! chain of siblings, a call under control flow, a trap after the call, and a sibling that talks to
//! a store. `run_tests` compares every statement's document, journal and failure on both tiers.

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
fn two_calls_run_the_sibling_twice_against_the_callers_state() {
    passes_everywhere(
        r##"
## Page /

State:
    Let n be 0.

Actions:
    To bumpTwice:
        Call bump.
        Call bump.

    To bump:
        Set n to n + 1.

View:
    <button id="go" on-click={bumpTwice}>go</button>
    <p id="n">{n}</p>

## Test: two nested calls and the caller's own write-back agree

Click "#go".
Click "#go".
Expect that the text of "#n" is equal to "4".
"##,
    );
}

#[test]
fn a_sibling_takes_its_arguments_evaluated_in_the_callers_scope() {
    passes_everywhere(
        r##"
## Page /

State:
    Let n be 0.
    Let k be 5.

Actions:
    To go:
        Let step be 2.
        Call accumulate with step and k.
        Call accumulate with k and step.

    To accumulate (k: Int) and (step: Int):
        Set n to n + (k * 10) + step.

View:
    <button id="go" on-click={go}>go</button>
    <p id="n">{n}</p>
    <p id="k">{k}</p>

## Test: each argument is the caller's value, bound to the sibling's parameter by position

Click "#go".
Expect that the text of "#n" is equal to "77".
Expect that the text of "#k" is equal to "5".
"##,
    );
}

#[test]
fn a_parameter_named_like_a_state_field_shadows_it_inside_the_sibling_only() {
    passes_everywhere(
        r##"
## Page /

State:
    Let n be 1.
    Let seen be 0.

Actions:
    To go:
        Call peek with 40.
        Set n to n + 1.

    To peek (n: Int):
        Set seen to n + 2.

View:
    <button id="go" on-click={go}>go</button>
    <p id="n">{n}</p>
    <p id="seen">{seen}</p>

## Test: the sibling reads its parameter, and the field keeps its own value

Click "#go".
Expect that the text of "#seen" is equal to "42".
Expect that the text of "#n" is equal to "2".
"##,
    );
}

#[test]
fn a_return_inside_the_sibling_ends_the_sibling_and_not_the_callers_turn() {
    passes_everywhere(
        r##"
## Page /

State:
    Let n be 0.
    Let after be 0.

Actions:
    To go:
        Call guarded.
        Set after to after + 1.

    To guarded:
        If n is greater than 0:
            Return.
        Set n to n + 10.

View:
    <button id="go" on-click={go}>go</button>
    <p id="n">{n}</p>
    <p id="after">{after}</p>

## Test: the second call returns early and the caller carries on

Click "#go".
Click "#go".
Expect that the text of "#n" is equal to "10".
Expect that the text of "#after" is equal to "2".
"##,
    );
}

#[test]
fn a_return_inside_a_loop_inside_the_sibling_leaves_only_the_sibling() {
    passes_everywhere(
        r##"
## Page /

State:
    Let found be 0.
    Let calls be 0.

Actions:
    To go:
        Call scan.
        Set calls to calls + 1.

    To scan:
        Repeat for x in [1, 2, 3, 4]:
            If x is equal to 3:
                Set found to x.
                Return.
        Set found to 99.

View:
    <button id="go" on-click={go}>go</button>
    <p id="found">{found}</p>
    <p id="calls">{calls}</p>

## Test: the loop's early return stops the scan and nothing after it in the sibling

Click "#go".
Expect that the text of "#found" is equal to "3".
Expect that the text of "#calls" is equal to "1".
"##,
    );
}

#[test]
fn a_chain_of_siblings_runs_in_order_inside_one_turn() {
    passes_everywhere(
        r##"
## Page /

State:
    Let trail be "".

Actions:
    To first:
        Set trail to trail + "a".
        Call second.
        Set trail to trail + "d".

    To second:
        Set trail to trail + "b".
        Call third.

    To third:
        Set trail to trail + "c".

View:
    <button id="go" on-click={first}>go</button>
    <p id="trail">{trail}</p>

## Test: first, second, third, then back in first

Click "#go".
Expect that the text of "#trail" is equal to "abcd".
"##,
    );
}

#[test]
fn a_sibling_called_under_control_flow_runs_only_where_it_is_reached() {
    passes_everywhere(
        r##"
## Page /

State:
    Let n be 0.
    Let items be [1, 2, 3].

Actions:
    To go:
        Repeat for x in items:
            If x is not equal to 2:
                Call accumulate with x.

    To accumulate (x: Int):
        Set n to n + x.

View:
    <button id="go" on-click={go}>go</button>
    <p id="n">{n}</p>

## Test: the call inside the loop runs for the items that reach it

Click "#go".
Expect that the text of "#n" is equal to "4".
"##,
    );
}

#[test]
fn a_sibling_that_calls_a_store_action_moves_the_store_and_its_readers() {
    passes_everywhere(
        r##"
## Store Basket

State:
    Let count be 0.

Actions:
    To add:
        Set count to count + 1.

## Page /

Use Basket.

State:
    Let own be 0.

Actions:
    To go:
        Call countHit.
        Set own to own + 1.

    To countHit:
        Call Basket's add.

View:
    <button id="go" on-click={go}>go</button>
    <p id="count">{Basket's count}</p>
    <p id="own">{own}</p>

## Test: the store moves through the sibling

Click "#go".
Click "#go".
Expect that the text of "#count" is equal to "2".
Expect that the text of "#own" is equal to "2".
"##,
    );
}

#[test]
fn a_component_placed_twice_runs_each_instances_sibling_against_that_instance() {
    passes_everywhere(
        r##"
## Component Counter (name: Text)

State:
    Let n be 0.

Actions:
    To double:
        Call inc.
        Call inc.

    To inc:
        Set n to n + 1.

View:
    <div><button id="{name}" on-click={double}>+2</button><p id="{name}-n">{n}</p></div>

## Page /

View:
    <Counter name="a"/>
    <Counter name="b"/>

## Test: each instance counts its own clicks

Click "#a".
Click "#b".
Click "#a".
Expect that the text of "#a-n" is equal to "4".
Expect that the text of "#b-n" is equal to "2".
"##,
    );
}

/// A circle of siblings has no finite inlining; the page is refused for the browser by name.
#[test]
fn siblings_that_call_each_other_in_a_circle_are_refused_for_the_browser_by_name() {
    let source = r##"
## Page /

State:
    Let n be 0.

Actions:
    To ping:
        If n is less than 3:
            Set n to n + 1.
            Call pong.

    To pong:
        Call ping.

View:
    <button id="go" on-click={ping}>go</button>
"##;
    let refused = compile_to_wasm(source).expect_err("a circle of sibling calls cannot be inlined");
    let message = format!("{refused:?}");
    assert!(
        message.contains("reaches itself")
            && (message.contains("`ping`") || message.contains("`pong`")),
        "the refusal names the action on the circle: {message}"
    );
}
