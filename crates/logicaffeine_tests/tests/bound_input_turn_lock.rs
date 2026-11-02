//! LOCK — a keystroke into a `bind={field}` input is a TURN on every tier: the field is written
//! back, a `When <field> changes:` sentence watching it runs once, derived fields and every site
//! reading them repaint, and the journal records one `bind` entry.
//!
//! The lowering builds `__bind_<C>_<field>` through `turn_body_with`, the one place a turn's shape
//! — and its field watchers — lives (`events.rs::lower_bind_writeback`). `run_tests` compares
//! document, journal and failure after every statement on the reference tier and the lowered VM;
//! each page must also build for the browser.

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

/// `docs/frontend.md`'s Triggers example, reduced to the bound field and its watcher.
#[test]
fn typing_into_a_bound_input_runs_the_sentence_watching_its_field() {
    passes_everywhere(
        r##"
## Page /

State:
    Let query be "".
    Let searches be 0.

Actions:
    When query changes:
        Set searches to searches + 1.

View:
    <input id="q" type="search" aria-label="q" bind={query}>
    <p id="searches">{searches}</p>

## Test: each keystroke is one change

Type "tea" into "#q".
Expect that the text of "#searches" is equal to "1".
Type "teapot" into "#q".
Expect that the text of "#searches" is equal to "2".
"##,
    );
}

#[test]
fn a_watcher_that_writes_another_field_repaints_that_field_in_the_same_turn() {
    passes_everywhere(
        r##"
## Page /

State:
    Let name be "".
    Let greeting be "nobody".

Actions:
    When name changes:
        Set greeting to "hello " + name.

View:
    <input id="n" type="text" aria-label="n" bind={name}>
    <p id="g">{greeting}</p>
    <p id="echo">{name}</p>

## Test: the watcher's write and the bound field both show

Type "ada" into "#n".
Expect that the text of "#g" is equal to "hello ada".
Expect that the text of "#echo" is equal to "ada".
"##,
    );
}

#[test]
fn a_derived_field_reading_the_bound_field_follows_every_keystroke() {
    passes_everywhere(
        r##"
## Page /

State:
    Let draft be "".
    Let size always be length of draft.

View:
    <input id="d" type="text" aria-label="d" bind={draft}>
    <p id="size">{size}</p>

## Test: the derived length follows the field

Type "four" into "#d".
Expect that the text of "#size" is equal to "4".
Type "sixsix" into "#d".
Expect that the text of "#size" is equal to "6".
"##,
    );
}

#[test]
fn a_bound_checkbox_wakes_the_sentence_watching_it() {
    passes_everywhere(
        r##"
## Page /

State:
    Let agreed be false.
    Let flips be 0.

Actions:
    When agreed changes:
        Set flips to flips + 1.

View:
    <input id="a" type="checkbox" aria-label="a" bind={agreed}>
    <p id="flips">{flips}</p>

## Test: two clicks, two changes

Click "#a".
Click "#a".
Expect that the text of "#flips" is equal to "2".
"##,
    );
}

#[test]
fn typing_the_value_already_held_is_no_change() {
    passes_everywhere(
        r##"
## Page /

State:
    Let query be "same".
    Let searches be 0.

Actions:
    When query changes:
        Set searches to searches + 1.

View:
    <input id="q" type="search" aria-label="q" bind={query}>
    <p id="searches">{searches}</p>

## Test: an unchanged value wakes nothing

Type "same" into "#q".
Expect that the text of "#searches" is equal to "0".
"##,
    );
}
