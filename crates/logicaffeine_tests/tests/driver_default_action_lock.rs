//! ★ LOCK — the browser's own default actions happen on every page tier. ★
//!
//! A click on a checkbox moves it whether or not the page listens: the box flips (a radio is
//! selected) before the page hears the `click`, `input` and `change` follow carrying the new state,
//! and a listener that cancels the click puts the box back. The browser answers that click itself,
//! so a checkbox nothing listens to is not the loud "nothing answers a click" failure. One driver
//! carries this out for every page tier (`page_host`'s `Driver::toggle`), the reference among them.

use logicaffeine_compile::page_host::{compare_tests, PageTierKind, TierRun, TraceStep};

const BUDGET: u64 = 50_000_000;

/// Every tier's run of `src`'s one test, by label, each required to run to its end.
fn runs(src: &str) -> Vec<(&'static str, Vec<TraceStep>)> {
    let (reference, mut out) = lowered_runs(src);
    out.insert(0, ("reference", reference));
    out
}

/// The reference's steps, and each lowered tier's run of `src`'s one test by label — every one
/// required to run to its end.
fn lowered_runs(src: &str) -> (Vec<TraceStep>, Vec<(&'static str, Vec<TraceStep>)>) {
    let all = compare_tests(
        src,
        &[PageTierKind::LoweredHere, PageTierKind::LoweredVm],
        0,
        BUDGET,
        None,
    )
    .unwrap_or_else(|e| panic!("the page parses:\n{src}\n=> {e}"));
    assert_eq!(all.len(), 1, "one test");
    let c = &all[0];
    assert_eq!(
        c.reference.stopped, None,
        "the reference runs to the end:\n{src}"
    );
    let mut out = Vec::new();
    for (kind, run) in &c.candidates {
        match run {
            TierRun::Traced(t) => {
                assert_eq!(t.stopped, None, "{} runs to the end:\n{src}", kind.label());
                out.push((kind.label(), t.steps.clone()));
            }
            TierRun::Refused { statement, reason } => {
                panic!("{} refused at {statement}: {reason}\n{src}", kind.label())
            }
        }
    }
    assert_eq!(out.len(), 2, "both lowered tiers ran");
    (c.reference.steps.clone(), out)
}

#[test]
fn a_checkbox_nothing_listens_to_is_ticked_by_a_click_on_every_page_tier() {
    let src = "\
## Component App
View:
    <input id=\"d\" type=\"checkbox\">

## Test: ticked
Mount App.
Click \"#d\".
Expect that \"#d\" is checked.
Click \"#d\".
Expect that \"#d\" is not checked.
";
    for (tier, steps) in runs(src) {
        assert!(
            steps[2].snapshot.document.contains("checked"),
            "{tier}: the first click ticks it"
        );
        assert!(
            !steps[4].snapshot.document.contains("checked"),
            "{tier}: the second unticks it"
        );
    }
}

#[test]
fn a_bound_checkbox_click_writes_its_field_on_every_page_tier() {
    let src = "\
## Component App
State:
    Let on be true.
View:
    <input id=\"c\" type=\"checkbox\" bind={on}>
    <p id=\"v\">{on}</p>

## Test: unticked
Mount App.
Click \"#c\".
Expect that the text of \"#v\" is equal to \"false\".
";
    for (tier, steps) in runs(src) {
        let last = &steps.last().unwrap().snapshot.document;
        assert!(
            last.contains("<p id=\"v\">false</p>"),
            "{tier}: the field follows the box: {last}"
        );
    }
}

#[test]
fn a_cancelled_click_leaves_the_box_as_it_was_on_every_page_tier() {
    let src = "\
## Component App
State:
    Let heard be 0.
Actions:
    To hear:
        Set heard to heard + 1.
View:
    <input id=\"d\" type=\"checkbox\" on-click.prevent={hear}>
    <p id=\"n\">{heard}</p>

## Test: cancelled
Mount App.
Click \"#d\".
Expect that \"#d\" is not checked.
";
    for (tier, steps) in runs(src) {
        let last = &steps.last().unwrap().snapshot.document;
        assert!(
            !last.contains("checked"),
            "{tier}: the cancelled click put the box back: {last}"
        );
        assert!(
            last.contains("<p id=\"n\">1</p>"),
            "{tier}: the listener heard it: {last}"
        );
    }
}

#[test]
fn a_radio_click_selects_it_on_every_page_tier() {
    let src = "\
## Component App
View:
    <input id=\"r\" type=\"radio\" name=\"size\">

## Test: selected
Mount App.
Click \"#r\".
Click \"#r\".
Expect that \"#r\" is checked.
";
    for (tier, steps) in runs(src) {
        let last = &steps.last().unwrap().snapshot.document;
        assert!(
            last.contains("checked"),
            "{tier}: a radio stays selected: {last}"
        );
    }
}
