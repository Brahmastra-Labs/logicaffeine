//! ★ LOCK — a checkbox bound to a Bool is CHECKED, not `value="true"`. ★
//!
//! A checkbox's state is a PRESENCE (`checked`), not a string. Rendering `value="true"` produces
//! a box every browser draws UNCHECKED while the page believes it is checked — and the author has
//! been told their `bind` was understood, which is the worst kind of wrong.
//!
//! Both tiers agreed on the wrong answer, so no parity lock could see it. That is the same shape
//! as `disabled="false"`: agreeing is not the same as being right, and a test that only compares
//! two implementations to each other cannot tell you when both are wrong.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom::{self, DomOp};

/// The page both tiers render, held to being identical.
fn page(src: &str) -> String {
    dom::reset();
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker must run it:\n{src}");
    let rendered = dom::render();
    dom::reset();
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM must run it:\n{src}");
    assert_eq!(
        dom::render(),
        rendered,
        "the two tiers rendered differently:\n{src}"
    );
    rendered
}

fn checkbox(initial: &str) -> String {
    format!(
        "## Component App\nState:\n    Let on be {initial}.\nView:\n    \
         <input id=\"c\" type=\"checkbox\" bind={{on}}>\n\n## Main\nMount App.\n"
    )
}

#[test]
fn a_checked_box_renders_the_checked_attribute() {
    let out = page(&checkbox("true"));
    assert!(
        out.contains("checked"),
        "a bound true must render as CHECKED: {out}"
    );
    assert!(
        !out.contains("value=\"true\""),
        "and never as a value — a browser draws that box unchecked: {out}"
    );
}

#[test]
fn an_unchecked_box_renders_no_checked_attribute_at_all() {
    // Absence is the encoding. `checked="false"` is CHECKED in every browser there is, so a
    // false that renders the attribute is worse than one that renders nothing.
    let out = page(&checkbox("false"));
    assert!(
        !out.contains("checked"),
        "a bound false must render NO checked attribute — `checked=\"false\"` is still checked: {out}"
    );
}

#[test]
fn a_text_input_still_binds_by_value() {
    // The fix must not spread: a text input's state IS its value, and turning that into a
    // presence would empty every text field on the page.
    let src = "## Component App\nState:\n    Let t be \"hi\".\nView:\n    \
        <input id=\"t\" type=\"text\" bind={t}>\n\n## Main\nMount App.\n";
    let out = page(src);
    assert!(
        out.contains("value=\"hi\""),
        "a text input binds by value: {out}"
    );
}

#[test]
fn a_checkbox_toggled_by_a_turn_repaints_as_checked() {
    // The state has to survive a turn, which is what a checkbox is for.
    let src = "## Component App\n\
        State:\n    Let on be false.\n\
        Actions:\n    To flip:\n        Set on to true.\n\
        View:\n    <input id=\"c\" type=\"checkbox\" bind={on}>\n    \
        <button id=\"b\" on-click={flip}>go</button>\n\n\
        ## Main\nMount App.\nClick \"#b\".\n";
    let out = page(src);
    assert!(
        out.contains("checked"),
        "after the turn the box is checked: {out}"
    );
}

/// A `Click` on a checkbox is what a visitor actually does, and in a browser it FLIPS the box
/// and fires `input`/`change` — so `bind` has to hear it and land a real Bool. Without this the
/// checkbox renders correctly and is inert: the page shows what the state says and the state
/// never hears what the visitor did.
fn toggler() -> String {
    "## Component App\n\
     State:\n    Let on be false.\n\
     View:\n    <input id=\"c\" type=\"checkbox\" bind={on}>\n    <p id=\"s\">{on}</p>\n\n\
     ## Main\nMount App.\n"
        .to_string()
}

#[test]
fn clicking_a_checkbox_checks_it() {
    let mut src = toggler();
    src.push_str("Click \"#c\".\n");
    let out = page(&src);
    assert!(out.contains("checked"), "a click checks the box: {out}");
}

#[test]
fn clicking_a_checkbox_writes_a_real_bool_into_state() {
    // The write-back must be a Bool, not the text `"on"` a browser puts in a checkbox's `value`
    // and not the string `"true"` — a Text in a Bool field is truthy whatever it says, so
    // unchecking would never take effect.
    let mut src = toggler();
    src.push_str("Click \"#c\".\n");
    let out = page(&src);
    assert!(
        out.contains("<p id=\"s\">true</p>"),
        "state reads the Bool true: {out}"
    );
}

#[test]
fn clicking_a_checked_box_unchecks_it() {
    // The half that a truthy-Text write-back would silently break.
    let mut src = toggler();
    src.push_str("Click \"#c\".\nClick \"#c\".\n");
    let out = page(&src);
    assert!(
        !out.contains("checked"),
        "a second click unchecks it: {out}"
    );
    assert!(
        out.contains("<p id=\"s\">false</p>"),
        "and the state says so: {out}"
    );
}

#[test]
fn clicking_a_button_is_still_a_click_and_not_a_toggle() {
    // The flip is a CHECKBOX behaviour. A click on anything else must stay what it was.
    let src = "## Component App\n\
        State:\n    Let n be 0.\n\
        Actions:\n    To bump:\n        Set n to n + 1.\n\
        View:\n    <button id=\"b\" on-click={bump}>go</button>\n    <p id=\"s\">{n}</p>\n\n\
        ## Main\nMount App.\nClick \"#b\".\n";
    let out = page(src);
    assert!(
        out.contains("<p id=\"s\">1</p>"),
        "an ordinary click still runs its action: {out}"
    );
    assert!(!out.contains("checked"), "and checks nothing: {out}");
}

#[test]
fn a_radio_is_selected_by_a_click_and_stays_selected() {
    // A radio's default action SELECTS rather than flips: clicking a chosen radio in a browser
    // leaves it chosen and raises no change. Flipping it would let a second click deselect it,
    // which no radio group has ever done.
    let src = "## Component App\nState:\n    Let on be false.\nView:\n    \
        <input id=\"r\" type=\"radio\" bind={on}>\n\n## Main\nMount App.\nClick \"#r\".\nClick \"#r\".\n";
    let out = page(src);
    assert!(
        out.contains("checked"),
        "still selected after a second click: {out}"
    );
}

#[test]
fn a_checkbox_keeps_its_own_value_attribute() {
    // `value` and `checked` are different things on the same element: a checkbox's `value` is
    // what a form SUBMITS when it is ticked. Binding the checked state must not eat it.
    let src = "## Component App\nState:\n    Let on be true.\nView:\n    \
        <input id=\"c\" type=\"checkbox\" value=\"milk\" bind={on}>\n\n## Main\nMount App.\n";
    let out = page(src);
    assert!(
        out.contains("value=\"milk\""),
        "the author's value survives: {out}"
    );
    assert!(
        out.contains("checked"),
        "and the binding still checks it: {out}"
    );
}

#[test]
fn two_checkboxes_move_independently() {
    let src = "## Component App\n\
        State:\n    Let a be false.\n    Let b be false.\n\
        View:\n    <input id=\"x\" type=\"checkbox\" bind={a}>\n    \
        <input id=\"y\" type=\"checkbox\" bind={b}>\n    <p id=\"s\">{a} {b}</p>\n\n\
        ## Main\nMount App.\nClick \"#x\".\n";
    let out = page(src);
    assert!(
        out.contains("<p id=\"s\">true false</p>"),
        "only the clicked one moved: {out}"
    );
}

#[test]
fn a_computed_type_is_refused_in_the_same_words_on_both_tiers() {
    // Whether a `bind` writes `value` or `checked` is decided by the element's TYPE. A computed
    // type leaves that undecidable, and guessing `value` is the exact silent wrong answer this
    // whole lock exists to close. Both tiers must refuse, and in the SAME words — whoever reads
    // one message and searches for it has to find the other.
    let src = "## Component App\n\
        State:\n    Let on be false.\n    Let kind be \"checkbox\".\n\
        View:\n    <input id=\"c\" type={kind} bind={on}>\n\n## Main\nMount App.\n";
    dom::reset();
    let tw = tw_outcome(src);
    dom::reset();
    let vm = vm_outcome(src);
    let tw_err = tw.error.unwrap_or_default();
    let vm_err = vm.error.unwrap_or_default();
    for (tier, err) in [("tree-walker", &tw_err), ("VM", &vm_err)] {
        assert!(
            err.contains("computed"),
            "the {tier} must refuse a computed type by name, said: {err:?}"
        );
        assert!(
            err.contains("type=\\\"checkbox\\\"") || err.contains("type=\"checkbox\""),
            "and must say what to write instead, said: {err:?}"
        );
    }
}

#[test]
fn the_op_log_carries_the_checked_state_and_never_a_checked_attribute() {
    // The op-log is what a browser is actually TOLD to do, and it is where the two encodings are
    // distinguishable: a rendered tree cannot tell `setChecked(el, true)` from an author writing
    // `checked="true"` by hand, but the browser can — one moves the box, the other does not.
    dom::reset();
    let src = "## Component App\nState:\n    Let on be true.\nView:\n    \
        <input id=\"c\" type=\"checkbox\" bind={on}>\n\n## Main\nMount App.\n";
    assert_eq!(tw_outcome(src).error, None);
    let tw_log = dom::op_log();
    dom::reset();
    assert_eq!(vm_outcome(src).error, None);
    assert_eq!(
        dom::op_log(),
        tw_log,
        "the two tiers issue the same ops:\n{src}"
    );
    assert!(
        tw_log
            .iter()
            .any(|op| matches!(op, DomOp::SetChecked { checked: true, .. })),
        "a bound true is a SetChecked op: {tw_log:?}"
    );
    assert!(
        !tw_log
            .iter()
            .any(|op| matches!(op, DomOp::SetAttribute { name, .. } if name == "checked")),
        "and never a `checked` ATTRIBUTE, which sets defaultChecked: {tw_log:?}"
    );
}
