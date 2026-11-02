//! LOCK — a callback prop (`(onGo: fn(…) -> Unit)`, `docs/frontend.md`'s "Passing actions down")
//! called from the child's own element runs the action it was handed on every tier.
//!
//! A browser calls the closure the child's render attached; a driving verb (`Click`, `Press`) goes
//! through the lowered dispatcher, which cannot call a closure it found at run time. The dispatcher
//! resolves the listener site to `ListenerSite::CallbackProp` and reads the (tag, owner key) pair
//! the PLACEMENT recorded against the child's instance key. Each row below is one way a prop can
//! carry an action to an element: named directly, forwarded through a middle component, with an
//! argument, from a keyed loop row, into a stateful child, behind a key modifier, and the same child
//! placed twice with two different actions. Every row runs through `run_tests`, which checks the
//! lowered VM against the reference tier statement by statement, and must build for the browser.

use logicaffeine_compile::compile::compile_to_wasm;
use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};

/// Every `## Test` in `source` passes on the reference tier AND agrees on the lowered VM, and the
/// page builds for the browser.
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
fn a_callback_named_directly_runs_the_owners_action() {
    passes_everywhere(
        r##"
## Component Child (onGo: fn() -> Unit)

View:
    <button id="go" on-click={onGo}>go</button>

## Page /

State:
    Let count be 0.

Actions:
    To bump:
        Set count to count + 1.

View:
    <Child onGo={bump}/>
    <p id="count">{count}</p>

## Test: two clicks run the owner's action twice

Click "#go".
Click "#go".
Expect that the text of "#count" is equal to "2".
"##,
    );
}

#[test]
fn a_callback_forwarded_through_a_middle_component_reaches_the_owner() {
    passes_everywhere(
        r##"
## Component Inner (onGo: fn() -> Unit)

View:
    <button id="go" on-click={onGo}>go</button>

## Component Middle (onGo: fn() -> Unit)

View:
    <section><Inner onGo={onGo}/></section>

## Page /

State:
    Let count be 0.

Actions:
    To bump:
        Set count to count + 1.

View:
    <Middle onGo={bump}/>
    <p id="count">{count}</p>

## Test: the click two components down runs the page's action

Click "#go".
Expect that the text of "#count" is equal to "1".
"##,
    );
}

#[test]
fn the_documented_row_example_removes_the_clicked_row_by_its_argument() {
    passes_everywhere(
        r##"
## Component Row (label: Text) and (remove: fn(Text) -> Unit)

View:
    <li class="row">{label} <button id="remove-{label}" on-click={remove(label)}>remove</button></li>

## Page /

State:
    Let names be ["ada", "grace", "linus"].

Actions:
    To drop (who: Text):
        Let mutable kept be a new Seq of Text.
        Repeat for n in names:
            If n is not equal to who:
                Push n to kept.
        Set names to kept.

View:
    <ul>{for each n in names, keyed by n:}<Row label={n} remove={drop}/>{end}</ul>

## Test: each row removes itself and no other

Click "#remove-grace".
Expect that the count of ".row" is equal to 2.
Click "#remove-ada".
Expect that the count of ".row" is equal to 1.
Expect that the text of ".row" is equal to "linus remove".
"##,
    );
}

#[test]
fn a_callback_with_an_int_argument_hands_the_action_that_value() {
    passes_everywhere(
        r##"
## Component Stepper (onStep: fn(Int) -> Unit)

View:
    <button id="by-one" on-click={onStep(1)}>+1</button>
    <button id="by-ten" on-click={onStep(10)}>+10</button>

## Page /

State:
    Let total be 0.

Actions:
    To add (amount: Int):
        Set total to total + amount.

View:
    <Stepper onStep={add}/>
    <p id="total">{total}</p>

## Test: each button hands its own amount

Click "#by-ten".
Click "#by-one".
Click "#by-ten".
Expect that the text of "#total" is equal to "21".
"##,
    );
}

#[test]
fn one_child_placed_twice_calls_the_action_each_placement_handed_it() {
    passes_everywhere(
        r##"
## Component Button (name: Text) and (onPress: fn() -> Unit)

View:
    <button id="{name}" on-click={onPress}>{name}</button>

## Page /

State:
    Let n be 0.

Actions:
    To up:
        Set n to n + 1.

    To down:
        Set n to n - 1.

View:
    <Button name="up" onPress={up}/>
    <Button name="down" onPress={down}/>
    <p id="n">{n}</p>

## Test: each placement runs its own action

Click "#up".
Click "#up".
Click "#down".
Expect that the text of "#n" is equal to "1".
"##,
    );
}

#[test]
fn a_callback_handed_by_a_child_component_runs_that_components_action_not_the_pages() {
    passes_everywhere(
        r##"
## Component Child (onGo: fn() -> Unit)

View:
    <button id="inner" on-click={onGo}>go</button>

## Component Panel

State:
    Let hits be 0.

Actions:
    To bump:
        Set hits to hits + 1.

View:
    <div><Child onGo={bump}/><p id="panel-hits">{hits}</p></div>

## Page /

State:
    Let hits be 0.

Actions:
    To bump:
        Set hits to hits + 100.

View:
    <Panel/>
    <p id="page-hits">{hits}</p>

## Test: the panel's own bump runs, and the page's same-named bump does not

Click "#inner".
Expect that the text of "#panel-hits" is equal to "1".
Expect that the text of "#page-hits" is equal to "0".
"##,
    );
}

#[test]
fn a_stateful_child_runs_its_own_action_and_its_callback_prop_side_by_side() {
    passes_everywhere(
        r##"
## Component Counter (onDone: fn() -> Unit)

State:
    Let own be 0.

Actions:
    To tick:
        Set own to own + 1.

View:
    <div>
        <button id="tick" on-click={tick}>tick</button>
        <p id="own">{own}</p>
        <button id="done" on-click={onDone}>done</button>
    </div>

## Page /

State:
    Let finished be 0.

Actions:
    To finish:
        Set finished to finished + 1.

View:
    <Counter onDone={finish}/>
    <p id="finished">{finished}</p>

## Test: the child's own turn and the handed action each land where they belong

Click "#tick".
Click "#done".
Click "#tick".
Click "#done".
Expect that the text of "#own" is equal to "2".
Expect that the text of "#finished" is equal to "2".
"##,
    );
}

const KEYED_CALLBACK: &str = r##"
## Component Field (onSubmit: fn() -> Unit)

View:
    <input id="d" aria-label="d" on-keydown.enter={onSubmit}>

## Page /

State:
    Let sent be 0.

Actions:
    To send:
        Set sent to sent + 1.

View:
    <Field onSubmit={send}/>
    <p id="sent">{sent}</p>

## Test: Enter submits

Press "Enter" in "#d".
Press "Enter" in "#d".
Expect that the text of "#sent" is equal to "2".
"##;

#[test]
fn a_keyed_callback_prop_answers_its_key() {
    passes_everywhere(KEYED_CALLBACK);
}

#[test]
fn a_keyed_callback_prop_refuses_another_key_in_the_same_words_on_both_tiers() {
    let source = KEYED_CALLBACK.replace(
        "Press \"Enter\" in \"#d\".\nPress \"Enter\" in \"#d\".\nExpect that the text of \"#sent\" is equal to \"2\".",
        "Press \"a\" in \"#d\".",
    );
    assert_ne!(source, KEYED_CALLBACK, "the fixture's drive was replaced");
    let report = run_tests(&source, &TestConfig::default()).expect("the fixture parses and runs");
    let outcome = &report.outcomes[0];
    assert_eq!(
        outcome.verdict,
        Verdict::Failed,
        "a press of another key answers nothing: {:?}",
        outcome.failures
    );
    assert!(
        outcome
            .failures
            .iter()
            .all(|f| f.contains("Nothing answers") && !f.contains("TIER DIVERGENCE")),
        "the reference refuses the press, and the lowered VM refuses it identically: {:?}",
        outcome.failures
    );
}

#[test]
fn a_callback_prop_in_a_conditionally_placed_stateful_child_runs_every_time_it_is_placed() {
    passes_everywhere(
        r##"
## Component Modal (close: fn() -> Unit)

Actions:
    To noop:
        Return.

View:
    <div id="modal">
        <h2 id="title" on-click={noop}>Sure?</h2>
        <button id="cancel" on-click={close}>Cancel</button>
    </div>

## Page /

State:
    Let open be false.
    Let closes be 0.

Actions:
    To show:
        Set open to true.

    To hide:
        Set open to false.
        Set closes to closes + 1.

View:
    <button id="open" on-click={show}>open</button>
    {if open:}<Modal close={hide}/>{end}
    <p id="closes">{closes}</p>

## Test: open, close, open, close

Click "#open".
Click "#cancel".
Expect that "#modal" is hidden.
Click "#open".
Click "#title".
Expect that "#modal" is visible.
Click "#cancel".
Expect that "#modal" is hidden.
Expect that the text of "#closes" is equal to "2".
"##,
    );
}

#[test]
fn an_action_typed_callback_prop_takes_its_signature_from_the_action_it_is_handed() {
    passes_everywhere(
        r##"
## Component Row (label: Text) and (remove: Action)

View:
    <li class="row" id="row-{label}" on-click={remove(label)}>{label}</li>

## Page /

State:
    Let items be ["a", "b"].
    Let gone be "".

Actions:
    To drop (what: Text):
        Set gone to what.

View:
    <p id="gone">{gone}</p>
    <ul>{for each x in items, keyed by x:}<Row label={x} remove={drop}/>{end}</ul>

## Test: the clicked row names itself

Click "#row-b".
Expect that the text of "#gone" is equal to "b".
Click "#row-a".
Expect that the text of "#gone" is equal to "a".
"##,
    );
}

#[test]
fn a_child_action_that_calls_its_callback_prop_runs_the_handed_action() {
    passes_everywhere(
        r##"
## Component Confirm (onYes: fn() -> Unit)

State:
    Let asked be 0.

Actions:
    To yes:
        Set asked to asked + 1.
        Call onYes.

View:
    <div><button id="yes" on-click={yes}>yes</button><p id="asked">{asked}</p></div>

## Page /

State:
    Let confirmed be 0.

Actions:
    To confirm:
        Set confirmed to confirmed + 1.

View:
    <Confirm onYes={confirm}/>
    <p id="confirmed">{confirmed}</p>

## Test: the child's own turn calls the page's action

Click "#yes".
Click "#yes".
Expect that the text of "#asked" is equal to "2".
Expect that the text of "#confirmed" is equal to "2".
"##,
    );
}

/// The lowered tier records which action a callback means at the placement; a value that is not
/// one of the placer's own actions or callback props carries no such identity, so the page is
/// refused for the browser by name rather than built with a handler that answers nothing.
#[test]
fn a_callback_prop_handed_something_other_than_an_action_is_refused_for_the_browser() {
    let source = r##"
## Component Child (onGo: fn() -> Unit)

View:
    <button id="go" on-click={onGo}>go</button>

## Page /

State:
    Let count be 0.

Actions:
    To bump:
        Set count to count + 1.

View:
    <Child onGo={count}/>
"##;
    let refused =
        compile_to_wasm(source).expect_err("a callback bound to a number cannot be built");
    let message = format!("{refused:?}");
    assert!(
        message.contains("callback") && message.contains("onGo"),
        "the refusal names the callback prop it cannot resolve: {message}"
    );
}

#[test]
fn an_action_typed_callback_handed_actions_of_two_shapes_is_refused_for_the_browser() {
    let source = r##"
## Component Row (label: Text) and (act: Action)

View:
    <li id="row-{label}" on-click={act(label)}>{label}</li>

## Page /

State:
    Let gone be "".
    Let n be 0.

Actions:
    To drop (what: Text):
        Set gone to what.

    To bump:
        Set n to n + 1.

View:
    <ul><Row label="a" act={drop}/><Row label="b" act={bump}/></ul>
"##;
    let refused = compile_to_wasm(source).expect_err("one callback cannot take two shapes");
    let message = format!("{refused:?}");
    assert!(
        message.contains("same parameters") && message.contains("bump"),
        "the refusal names the action whose shape disagrees: {message}"
    );
}
