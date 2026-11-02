//! ★ LOCK — `on-keydown.enter={action}` works in a browser, and fires only on that key. ★
//!
//! "Press Enter to submit" is the second thing anybody writes after a click, and it did not
//! reach the browser tier at all. `nodes_lowerable` refused every event modifier except
//! `.prevent`, so a page using one compiled on the tier its author tests on and could not be
//! built for the tier a visitor runs.
//!
//! ★ THE REFUSAL WAS THE SECOND ATTEMPT, AND THE FIRST WAS WORSE. The gate began as a DENYLIST
//! of `stop|once|capture|passive`, so a KEY modifier — `.enter`, `.escape` — passed simply by not
//! being on it, and was then never honoured: the browser registered a listener with NO key
//! comparison while the reference tier threaded the key through and fired only on a match. A
//! page behaved one way where its tests ran and another where its visitors were. Turning it into
//! an allowlist made that loud; this file makes it work.
//!
//! ★ THE RULE: a listener's key matches case-insensitively, and a KEYED listener does not answer
//! an unkeyed press. A browser reports `KeyboardEvent.key` as `"Enter"` while the author writes
//! `.enter`, so case-insensitivity is not a nicety here — it is the whole reason the two ends
//! meet.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

/// A page whose draft field submits on Enter — the canonical shape.
fn page(driver: &str) -> String {
    format!(
        "## Component App\n\
         State:\n    Let saved be \"no\".\n\
         Actions:\n    To save:\n        Set saved to \"yes\".\n\
         View:\n    <input id=\"draft\" on-keydown.enter={{save}}>\n    <p id=\"out\">{{saved}}</p>\n\
         \n## Main\nMount App.\n{driver}"
    )
}

fn tw_doc(src: &str) -> (String, Option<String>) {
    dom::reset();
    let out = tw_outcome(src);
    (dom::render(), out.error)
}

fn vm_doc(src: &str) -> (String, Option<String>) {
    dom::reset();
    let out = vm_outcome(src);
    (dom::render(), out.error)
}

#[test]
fn the_declared_key_runs_the_action_on_both_tiers() {
    let src = page("Press \"Enter\" in \"#draft\".\n");
    let (tw, tw_err) = tw_doc(&src);
    assert_eq!(tw_err, None, "the reference tier must run it: {tw_err:?}");
    assert!(tw.contains("yes"), "the reference tier's contract:\n{tw}");

    let (vm, vm_err) = vm_doc(&src);
    assert_eq!(
        vm_err, None,
        "a key modifier must reach the browser tier: {vm_err:?}"
    );
    assert_eq!(vm, tw, "and paint what the reference tier paints");
}

#[test]
fn a_different_key_does_not_run_the_action_on_either_tier() {
    // The half that matters. A listener registered with NO key comparison fires on every
    // keystroke, so a page "worked" — until you typed a letter and it submitted.
    let src = page("Press \"a\" in \"#draft\".\n");
    let (_, tw_err) = tw_doc(&src);
    let (_, vm_err) = vm_doc(&src);
    assert!(
        tw_err.is_some(),
        "the reference tier refuses a press nothing answers — that is how it says the key \
         did not match"
    );
    assert!(
        vm_err.is_some(),
        "and the browser tier must agree rather than running the action for any key"
    );
}

#[test]
fn the_key_comparison_ignores_case_because_a_browser_reports_enter() {
    // The author writes `.enter`; `KeyboardEvent.key` is `"Enter"`. If these had to match
    // exactly, every key modifier would be dead in a real browser and alive in every test.
    for spelling in ["Enter", "enter", "ENTER"] {
        let src = page(&format!("Press \"{spelling}\" in \"#draft\".\n"));
        let (tw, tw_err) = tw_doc(&src);
        assert_eq!(tw_err, None, "{spelling} on the reference tier: {tw_err:?}");
        assert!(tw.contains("yes"), "{spelling} must run it:\n{tw}");
        let (vm, vm_err) = vm_doc(&src);
        assert_eq!(vm_err, None, "{spelling} on the browser tier: {vm_err:?}");
        assert_eq!(vm, tw, "{spelling} must agree across tiers");
    }
}

#[test]
fn an_unkeyed_handler_still_answers_any_key() {
    // The rule cuts both ways: adding key support must not make a plain `on-keydown` picky.
    let src = "## Component App\n\
               State:\n    Let saved be \"no\".\n\
               Actions:\n    To save:\n        Set saved to \"yes\".\n\
               View:\n    <input id=\"draft\" on-keydown={save}>\n    <p id=\"out\">{saved}</p>\n\
               \n## Main\nMount App.\nPress \"q\" in \"#draft\".\n";
    let (tw, tw_err) = tw_doc(src);
    assert_eq!(tw_err, None, "reference tier: {tw_err:?}");
    assert!(
        tw.contains("yes"),
        "an unkeyed handler answers any key:\n{tw}"
    );
    let (vm, vm_err) = vm_doc(src);
    assert_eq!(vm_err, None, "browser tier: {vm_err:?}");
    assert_eq!(vm, tw);
}

#[test]
fn a_key_modifier_compiles_to_wasm() {
    // The whole point: this has to reach the tier a visitor runs.
    let src = page("");
    logicaffeine_compile::compile::compile_to_wasm(&src)
        .expect("a page with a key modifier compiles to wasm");
}

// ─── The other half: `When the user presses <Key> in "<sel>":` ───

/// ★ A KEYED SELECTOR TRIGGER WAS SKIPPED OUTRIGHT, AND SILENTLY. The walk that resolves
/// `When "<sel>" is <event>:` sentences against the rendered view destructured
/// `Trigger::Selector { key: None }` and `continue`d — while `lowerable` admitted a selector
/// trigger whatever its key. So `When the user presses Escape in "#draft":` compiled, mounted,
/// registered NO listener, and answered nothing. Not refused, not lowered: absent, on the one
/// tier a visitor uses.
///
/// This is the same shape as the modifier bug in `event_modifier_lock` and it sat one function
/// away from it — a fix landing where the bug was OBSERVED while the identical flaw waits in a
/// parallel spelling.
fn trigger_page(driver: &str) -> String {
    format!(
        "## Component App\n\
         State:\n    Let saved be \"no\".\n\
         Actions:\n\
         \x20   When the user presses Escape in \"#draft\":\n\
         \x20       Set saved to \"yes\".\n\
         View:\n    <input id=\"draft\">\n    <p id=\"out\">{{saved}}</p>\n\
         \n## Main\nMount App.\n{driver}"
    )
}

#[test]
fn a_keyed_selector_trigger_answers_its_key_on_both_tiers() {
    let src = trigger_page("Press \"Escape\" in \"#draft\".\n");
    let (tw, tw_err) = tw_doc(&src);
    assert_eq!(tw_err, None, "the reference tier: {tw_err:?}");
    assert!(
        tw.contains("yes"),
        "the sentence must run the action:\n{tw}"
    );

    let (vm, vm_err) = vm_doc(&src);
    assert_eq!(
        vm_err, None,
        "a keyed trigger must REGISTER on the browser tier — it used to be skipped in silence: \
         {vm_err:?}"
    );
    assert_eq!(vm, tw, "and paint what the reference tier paints");
}

#[test]
fn a_keyed_selector_trigger_ignores_another_key_on_both_tiers() {
    let src = trigger_page("Press \"a\" in \"#draft\".\n");
    let (_, tw_err) = tw_doc(&src);
    let (_, vm_err) = vm_doc(&src);
    assert!(
        tw_err.is_some(),
        "the reference tier refuses a press nothing answers"
    );
    assert!(
        vm_err.is_some(),
        "and so must the browser tier — registering is not enough, it gates"
    );
}

#[test]
fn a_keyed_selector_trigger_compiles_to_wasm() {
    logicaffeine_compile::compile::compile_to_wasm(&trigger_page(""))
        .expect("a keyed selector trigger compiles to wasm");
}
