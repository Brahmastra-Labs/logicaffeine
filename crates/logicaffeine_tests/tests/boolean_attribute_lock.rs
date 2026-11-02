//! ★ LOCK — an HTML boolean attribute is a PRESENCE, not a string. ★
//!
//! `disabled="false"` DISABLES the control in every browser there is. So a page whose author
//! wrote `disabled={false}` — meaning enabled — shipped disabled, and the button nobody could
//! click looked exactly like a button that was supposed to be there.
//!
//! Both tiers agreed on the wrong answer, which is why `lgx_vm_parity_lock` never saw it: two
//! implementations agreeing is not the same as either being right. This is the sibling of the
//! checkbox defect in `checkbox_bind_lock` — the same mistake (encoding a presence as a value)
//! in the other position on the same element.

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

fn button(cond: &str) -> String {
    format!(
        "## Component App\nState:\n    Let busy be {cond}.\nView:\n    \
         <button id=\"b\" disabled={{busy}}>go</button>\n\n## Main\nMount App.\n"
    )
}

#[test]
fn a_false_boolean_attribute_is_absent_entirely() {
    let out = page(&button("false"));
    assert!(
        !out.contains("disabled"),
        "`disabled={{false}}` means ENABLED, and the only encoding of that is no attribute at \
         all — `disabled=\"false\"` disables it: {out}"
    );
}

#[test]
fn a_true_boolean_attribute_is_present_and_bare() {
    let out = page(&button("true"));
    assert!(
        out.contains("disabled"),
        "`disabled={{true}}` disables it: {out}"
    );
    assert!(
        !out.contains("disabled=\"true\""),
        "and does it as a presence, not a string: {out}"
    );
}

#[test]
fn a_boolean_attribute_flips_with_the_state_it_reads() {
    // The whole point of writing `disabled={cond}`: it has to follow the condition across a turn.
    let src = "## Component App\n\
        State:\n    Let busy be true.\n\
        Actions:\n    To finish:\n        Set busy to false.\n\
        View:\n    <button id=\"b\" disabled={busy}>go</button>\n    \
        <button id=\"go\" on-click={finish}>x</button>\n\n\
        ## Main\nMount App.\nClick \"#go\".\n";
    let out = page(src);
    assert!(!out.contains("disabled"), "the turn enabled it: {out}");
}

#[test]
fn an_ordinary_attribute_holding_a_bool_still_renders_its_value() {
    // The rule is keyed on the ATTRIBUTE NAME, because that is what HTML keys it on. `data-done`
    // is not a boolean attribute, so a Bool in it is data and must survive as text.
    let src = "## Component App\nState:\n    Let done be false.\nView:\n    \
        <p id=\"p\" data-done={done}>x</p>\n\n## Main\nMount App.\n";
    let out = page(src);
    assert!(
        out.contains("data-done=\"false\""),
        "an ordinary attribute keeps its value: {out}"
    );
}

#[test]
fn a_literal_empty_boolean_attribute_is_still_present() {
    // `disabled=""` IS disabled in HTML. A rule that went by TRUTHINESS would drop it, because
    // an empty Text is falsy — which is why the rule goes by the value's TYPE instead.
    let src = "## Component App\nView:\n    <button id=\"b\" disabled=\"\">go</button>\n\n\
        ## Main\nMount App.\n";
    let out = page(src);
    assert!(
        out.contains("disabled"),
        "an empty boolean attribute is present: {out}"
    );
}

#[test]
fn every_boolean_attribute_in_the_set_obeys_the_same_rule() {
    // One rule, not a special case for `disabled`. Each of these is a presence in the HTML
    // standard, and each would silently take effect if rendered as the string "false".
    for name in [
        "disabled", "readonly", "required", "hidden", "open", "multiple", "selected",
    ] {
        let src = format!(
            "## Component App\nState:\n    Let flag be false.\nView:\n    \
             <p id=\"p\" {name}={{flag}}>x</p>\n\n## Main\nMount App.\n"
        );
        let out = page(&src);
        assert!(
            !out.contains(name),
            "`{name}={{false}}` must be absent: {out}"
        );
    }
}

#[test]
fn the_op_log_shows_no_attribute_call_at_all_for_a_false_flag() {
    // Absence has to reach the BROWSER as absence. Recording `setAttribute(el, "disabled", "")`
    // and hoping a reconciler drops it later would disable the control on the very first paint.
    dom::reset();
    let src = button("false");
    assert_eq!(tw_outcome(&src).error, None);
    let tw_log = dom::op_log();
    dom::reset();
    assert_eq!(vm_outcome(&src).error, None);
    assert_eq!(
        dom::op_log(),
        tw_log,
        "the two tiers issue the same ops:\n{src}"
    );
    assert!(
        !tw_log
            .iter()
            .any(|op| matches!(op, DomOp::SetAttribute { name, .. } if name == "disabled")),
        "no `disabled` op is issued at all: {tw_log:?}"
    );
}

/// `checked` is the one boolean attribute that must NOT be written as an attribute.
///
/// It is boolean in the markup, but in the DOM the attribute is `defaultChecked` — the state the
/// box returns to on a form RESET — while what a visitor sees and a form submits is the
/// PROPERTY. On a freshly created element the two coincide, which is exactly why this is a trap:
/// it works on the first paint and stops working the moment a reconciler patches an existing
/// node instead of building a new one, which is the whole point of having a reconciler.
#[test]
fn a_checked_attribute_moves_the_live_box_and_not_default_checked() {
    dom::reset();
    let src = "## Component App\nState:\n    Let on be true.\nView:\n    \
        <input id=\"c\" type=\"checkbox\" checked={on}>\n\n## Main\nMount App.\n";
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
        "a `checked` attribute is the live property: {tw_log:?}"
    );
    assert!(
        !tw_log
            .iter()
            .any(|op| matches!(op, DomOp::SetAttribute { name, .. } if name == "checked")),
        "and never the attribute, which is defaultChecked: {tw_log:?}"
    );
}

#[test]
fn a_false_checked_attribute_leaves_the_box_alone() {
    let src = "## Component App\nState:\n    Let on be false.\nView:\n    \
        <input id=\"c\" type=\"checkbox\" checked={on}>\n\n## Main\nMount App.\n";
    let out = page(src);
    assert!(
        !out.contains("checked"),
        "`checked={{false}}` is unchecked: {out}"
    );
}
