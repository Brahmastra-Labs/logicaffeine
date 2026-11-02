//! ★ LOCK — `is visible` answers whether the visitor can SEE it. ★
//!
//! It meant `query_selector(sel) >= 0` — present in the document — and `is hidden` meant absent
//! from it. So for an element that exists and is not shown, both answered the wrong thing:
//!
//! ```text
//! <p id="a" hidden>x</p>          "#a" is visible  →  true
//! <p id="a" style="display:none"> "#a" is visible  →  true
//! <p id="a" hidden>x</p>          "#a" is hidden   →  false
//! ```
//!
//! ★ WHICH MAKES THE ONE TEST ANYBODY WRITES UNABLE TO FAIL. A page shows and hides things — an
//! error line, a spinner, a dropdown — and LGX's own idiom for it is `hidden={flag}` or
//! `class:hidden={flag}`. Neither changes whether the element is in the document, so
//! `Expect "#error" to be visible.` passed in BOTH states, and a toggle that stopped working
//! reported nothing. The question exists precisely to tell those two states apart.
//!
//! ★ WHAT IS DECIDED, AND WHAT IS NOT, said plainly rather than implied:
//!   * the `hidden` attribute — HTML's own "not rendered";
//!   * an inline `style` declaring `display:none` or `visibility:hidden`;
//!   * a `Style:` rule declaring either, whose selector matches — this is what
//!     `class:hidden={flag}` toggles, so leaving it out would leave the idiom uncovered;
//!   * and ALL of those on any ANCESTOR, because hiding a container hides what is inside it.
//!
//! What is not decided is anything needing a full cascade — specificity between two rules that
//! both match, a `display` set from a stylesheet the page did not declare, or a computed height of
//! zero. `is visible` is a page test's question, not a rendering engine, and the honest boundary is
//! the one written here.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

/// Ask the tier a page question runs on.
///
/// ★ ONE TIER, AND THAT IS THE DESIGN RATHER THAN A GAP. A page query needs a document, and the VM
/// refuses one by name — *"`the visible of` asks about the page, which this tier does not have"* —
/// so requiring the two to agree here would be asserting that a refusal equals an answer. The
/// cross-tier question this file could ask is already asked by `page_query_lock`.
fn ask(subject: &str, view: &str, question: &str) -> String {
    let src = format!(
        "## Component App\nState:\n\x20   Let n be 1.\nView:\n\x20   {view}\n\n\
         ## Main\nMount App.\nShow {question}.\n"
    );
    dom::reset();
    let tw = tw_outcome(&src);
    assert_eq!(tw.error, None, "`{subject}` must run:\n{src}");
    tw.output.trim().to_string()
}

#[test]
fn the_vm_still_refuses_a_page_question_by_name() {
    // The reason `ask` drives one tier, pinned so it cannot quietly become a silent answer.
    let src = "## Component App\nState:\n\x20   Let n be 1.\nView:\n\x20   <p id=\"a\">x</p>\n\n\
               ## Main\nMount App.\nShow \"#a\" is visible.\n";
    let err = vm_outcome(src)
        .error
        .unwrap_or_else(|| panic!("the VM must refuse a page query"));
    assert!(err.contains("page"), "and say why: {err}");
}

/// `is visible` and `is hidden` must be exact complements — an element is one or the other.
fn complementary(subject: &str, view: &str, sel: &str, visible: bool) {
    assert_eq!(
        ask(subject, view, &format!("\"{sel}\" is visible")),
        visible.to_string(),
        "`{subject}`: is visible"
    );
    assert_eq!(
        ask(subject, view, &format!("\"{sel}\" is hidden")),
        (!visible).to_string(),
        "`{subject}`: is hidden — it is the exact complement, so a page is never both and never \
         neither"
    );
}

#[test]
fn an_element_on_the_page_is_visible() {
    complementary("a plain element", "<p id=\"a\">x</p>", "#a", true);
}

#[test]
fn an_element_that_is_not_there_is_not_visible() {
    complementary("an absent element", "<p id=\"b\">x</p>", "#a", false);
}

#[test]
fn the_hidden_attribute_hides_it() {
    // HTML's own way of saying "not rendered", and the one a boolean attribute binding writes.
    complementary(
        "the hidden attribute",
        "<p id=\"a\" hidden>x</p>",
        "#a",
        false,
    );
}

#[test]
fn an_inline_display_none_hides_it() {
    complementary(
        "inline display:none",
        "<p id=\"a\" style=\"display:none\">x</p>",
        "#a",
        false,
    );
    complementary(
        "inline display: none with a space",
        "<p id=\"a\" style=\"color: red; display: none;\">x</p>",
        "#a",
        false,
    );
}

#[test]
fn an_inline_visibility_hidden_hides_it() {
    complementary(
        "inline visibility:hidden",
        "<p id=\"a\" style=\"visibility:hidden\">x</p>",
        "#a",
        false,
    );
}

#[test]
fn a_hidden_ancestor_hides_what_is_inside_it() {
    // ★ THE CASE A PER-ELEMENT CHECK MISSES ENTIRELY, and the commonest real one: a panel is
    // hidden, and the question is about a button inside it.
    complementary(
        "a hidden container",
        "<div id=\"box\" hidden><p id=\"a\">x</p></div>",
        "#a",
        false,
    );
    complementary(
        "a display:none container",
        "<div id=\"box\" style=\"display:none\"><span><p id=\"a\">x</p></span></div>",
        "#a",
        false,
    );
}

#[test]
fn a_style_rule_that_hides_it_counts() {
    // ★ THE IDIOM. `class:hidden={flag}` toggles a CLASS, and the rule that hides it lives in
    // `Style:` — so a check that only read attributes would leave the framework's own way of
    // showing and hiding things completely uncovered.
    let src = "## Component App\nState:\n\x20   Let n be 1.\n\
               View:\n\x20   <p id=\"a\" class=\"gone\">x</p>\n\
               Style:\n\x20   .gone { display: none; }\n\n\
               ## Main\nMount App.\nShow \"#a\" is visible.\nShow \"#a\" is hidden.\n";
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "{src}");
    assert_eq!(
        out.output.trim(),
        "false\ntrue",
        "a `Style:` rule declaring `display: none` hides what it selects:\n{src}"
    );
}

#[test]
fn a_sheet_is_its_text_however_the_text_is_split() {
    // A `<style>` element's sheet is its TEXT CONTENT, as a browser reads it: here the rule arrives
    // as two text nodes, and the one it spells still hides what it selects.
    let src = "## Main\n\
               Let s be createElement(\"style\").\n\
               Call appendChild with s and createTextNode(\".gone {{ display:\").\n\
               Call appendChild with s and createTextNode(\" none; }\").\n\
               Call appendChild with body() and s.\n\
               Let p be createElement(\"p\").\n\
               Call setAttribute with p and \"id\" and \"a\".\n\
               Call setAttribute with p and \"class\" and \"gone\".\n\
               Call appendChild with body() and p.\n\
               Show \"#a\" is visible.\n";
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "{src}");
    assert_eq!(out.output.trim(), "false", "{src}");
}

#[test]
fn a_style_rule_that_does_not_hide_leaves_it_visible() {
    // The narrowness guard on the stylesheet half: ordinary styling must not read as hiding.
    let src = "## Component App\nState:\n\x20   Let n be 1.\n\
               View:\n\x20   <p id=\"a\" class=\"loud\">x</p>\n\
               Style:\n\x20   .loud { color: red; font-weight: bold; }\n\n\
               ## Main\nMount App.\nShow \"#a\" is visible.\n";
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "{src}");
    assert_eq!(out.output.trim(), "true", "{src}");
}

#[test]
fn a_style_rule_that_selects_something_else_leaves_it_visible() {
    let src = "## Component App\nState:\n\x20   Let n be 1.\n\
               View:\n\x20   <p id=\"a\">x</p><p id=\"b\" class=\"gone\">y</p>\n\
               Style:\n\x20   .gone { display: none; }\n\n\
               ## Main\nMount App.\nShow \"#a\" is visible.\nShow \"#b\" is visible.\n";
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "{src}");
    assert_eq!(out.output.trim(), "true\nfalse", "{src}");
}

#[test]
fn the_question_can_tell_a_toggle_apart() {
    // ★★ THE ANTI-VACUITY, AND THE WHOLE POINT. Before this, both states answered `true`, so a
    // test driving a show/hide toggle passed whichever way the toggle went — including when it
    // stopped working. The two answers must differ.
    let shown = ask("a toggle, shown", "<p id=\"a\">x</p>", "\"#a\" is visible");
    let gone = ask(
        "a toggle, hidden",
        "<p id=\"a\" hidden>x</p>",
        "\"#a\" is visible",
    );
    assert_ne!(
        shown, gone,
        "showing and hiding one element must give different answers, or every show/hide test in \
         the suite asserts nothing"
    );
    assert_eq!(shown, "true");
    assert_eq!(gone, "false");
}

#[test]
fn a_bound_boolean_attribute_drives_it() {
    // The form a page actually writes: `hidden={flag}`, the framework's own boolean-attribute
    // binding, which `server_surface_lock` already pins renders bare or not at all.
    for (flag, visible) in [("true", "false"), ("false", "true")] {
        let src = format!(
            "## Component App\nState:\n\x20   Let gone be {flag}.\n\
             View:\n\x20   <p id=\"a\" hidden={{gone}}>x</p>\n\n\
             ## Main\nMount App.\nShow \"#a\" is visible.\n"
        );
        dom::reset();
        let out = tw_outcome(&src);
        assert_eq!(out.error, None, "{src}");
        assert_eq!(out.output.trim(), visible, "`hidden={{{flag}}}`:\n{src}");
    }
}

#[test]
fn the_other_element_questions_are_unchanged() {
    // `is disabled` and `is checked` ask about state, not about being seen — a fix to visibility
    // must not wander into them.
    assert_eq!(
        ask(
            "disabled",
            "<button id=\"a\" disabled>x</button>",
            "\"#a\" is disabled"
        ),
        "true"
    );
    assert_eq!(
        ask(
            "a hidden but enabled button",
            "<button id=\"a\" hidden>x</button>",
            "\"#a\" is disabled"
        ),
        "false",
        "hiding a control does not disable it"
    );
}
