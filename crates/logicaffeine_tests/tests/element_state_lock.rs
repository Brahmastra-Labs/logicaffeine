//! ★ LOCK — a test asks whether an element is visible, hidden, disabled, checked or focused. ★
//!
//! These complete the page-question vocabulary the plan specifies. They are ORDINARY EXPRESSIONS
//! (`Expr::Dom`), like `the text of SEL`, so `Expect that "#b" is disabled` works through the
//! assertion machinery LOGOS already has rather than through a statement invented for testing.
//!
//! `DomQuery::Visible` had been in the AST since the page queries landed and NOTHING PARSED IT —
//! `"#x" is visible` was not a sentence any program could write. A variant no syntax reaches is
//! the same class of gap as a checker nobody calls: it looks supported and is not.
//!
//! Each predicate reads the state the page ACTUALLY holds. `disabled` and `checked` are the two
//! this file exists to get right: after `boolean_attribute_lock` and `checkbox_bind_lock` they
//! are PRESENCES, so asking about them must be asking about presence too — a test that passed by
//! comparing against the string `"false"` would be green on a control that is really enabled.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::dom;

fn output(src: &str) -> Vec<String> {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the program must run cleanly:\n{src}");
    out.output.lines().map(str::to_string).collect()
}

/// A page with one of everything the predicates ask about.
const PAGE: &str = "\
## Component App
State:
    Let busy be true.
    Let on be true.
View:
    <p id=\"there\">here</p>
    <button id=\"b\" disabled={busy}>go</button>
    <button id=\"ok\">fine</button>
    <input id=\"c\" type=\"checkbox\" bind={on}>
    <input id=\"d\" type=\"checkbox\">
    <input id=\"t\" type=\"text\">

## Main
Mount App.
";

fn ask(question: &str) -> Vec<String> {
    output(&format!("{PAGE}Show {question}.\n"))
}

#[test]
fn visible_answers_whether_anything_matches() {
    assert_eq!(ask("\"#there\" is visible"), ["true"]);
    assert_eq!(ask("\"#nope\" is visible"), ["false"]);
}

#[test]
fn hidden_is_the_exact_opposite_of_visible() {
    // Two words for one question is a liability unless they are held to being complementary:
    // an element that is neither visible nor hidden would make every test written with one of
    // them quietly incomplete.
    assert_eq!(ask("\"#there\" is hidden"), ["false"]);
    assert_eq!(ask("\"#nope\" is hidden"), ["true"]);
}

#[test]
fn disabled_asks_about_presence_and_not_about_a_string() {
    assert_eq!(
        ask("\"#b\" is disabled"),
        ["true"],
        "the button bound to a true flag"
    );
    assert_eq!(
        ask("\"#ok\" is disabled"),
        ["false"],
        "one with no `disabled` at all"
    );
}

#[test]
fn a_control_whose_flag_is_false_is_not_disabled() {
    // The case the old `disabled="false"` rendering got backwards: the page says ENABLED, and
    // both the render and the question about it have to agree that it is.
    let src = PAGE.replace("Let busy be true.", "Let busy be false.");
    let out = output(&format!("{src}Show \"#b\" is disabled.\n"));
    assert_eq!(out, ["false"]);
}

#[test]
fn checked_asks_the_live_state_of_a_box() {
    assert_eq!(ask("\"#c\" is checked"), ["true"], "bound to a true field");
    assert_eq!(ask("\"#d\" is checked"), ["false"], "bound to nothing");
}

#[test]
fn a_click_changes_the_answer_to_checked() {
    // The predicate has to follow the page, not the markup it started as.
    let out = output(&format!("{PAGE}Click \"#d\".\nShow \"#d\" is checked.\n"));
    assert_eq!(out, ["true"]);
}

#[test]
fn focused_names_the_one_element_that_has_focus() {
    // Focus is moved by the PAGE, from an action — `Focus` is deliberately not a top-level
    // driving verb, because it is ordinary English and a program beginning a sentence with it
    // must keep its meaning. So this is also the realistic shape: an action focuses a field and
    // the test asks whether it landed.
    let src = "## Component App\n\
        Actions:\n    To go:\n        Focus \"#t\".\n\
        View:\n    <input id=\"t\" type=\"text\">\n    \
        <button id=\"b\" on-click={go}>go</button>\n\n\
        ## Main\nMount App.\nClick \"#b\".\nShow \"#t\" is focused.\nShow \"#b\" is focused.\n";
    assert_eq!(output(src), ["true", "false"]);
}

#[test]
fn nothing_is_focused_before_anything_focuses_it() {
    assert_eq!(ask("\"#t\" is focused"), ["false"]);
}

#[test]
fn a_selector_matching_nothing_answers_false_rather_than_failing() {
    // A page question about an absent element is a legitimate thing to ask — `is visible` exists
    // precisely to ask it — so every one of these must ANSWER rather than trap. `Click` is the
    // verb that fails loudly on a missing selector, because a click that went nowhere means the
    // rest of the test describes a page nobody touched.
    for q in ["is disabled", "is checked", "is focused"] {
        assert_eq!(ask(&format!("\"#nope\" {q}")), ["false"], "{q} on nothing");
    }
}

#[test]
fn an_ordinary_comparison_against_a_variable_still_means_comparison() {
    // The collision this syntax has to avoid. `visible`, `hidden`, `checked` and `focused` are
    // ordinary English adjectives and a program may well hold a Bool named one of them, so the
    // predicates fire ONLY on a literal selector. `mode is visible` is, and stays, an equality.
    let src = "## Main\nLet visible be true.\nLet mode be true.\nShow mode is visible.\n";
    assert_eq!(output(src), ["true"]);
}

#[test]
fn the_predicates_compose_with_the_assertion_forms_logos_already_has() {
    // The whole reason these are expressions: no new assertion vocabulary.
    let src = format!(
        "{PAGE}## Test: the page is in the state it says\n    Mount App.\n    \
         Expect that \"#b\" is disabled.\n    Expect that \"#c\" is checked.\n"
    );
    dom::reset();
    let out = tw_outcome(&src);
    assert_eq!(out.error, None, "the test block must run:\n{src}");
}

// ---- Focus across a repaint: the G17 bug, in the reference tier's own terms. ----

/// A page that focuses a field and then changes state must still be focused afterwards.
///
/// This tier paints by clearing and rebuilding, which DESTROYS the focused node — so without
/// something carrying focus across, `To add: … Focus "#draft".` (the plan's own example) loses
/// the focus it just set, on the very tier that defines what correct means. A browser's
/// reconciler keeps the node alive; here the element's `id` is the identity that follows it.
#[test]
fn focus_survives_the_repaint_of_the_turn_that_set_it() {
    let src = "## Component App\n\
        State:\n    Let n be 0.\n\
        Actions:\n    To go:\n        Set n to n + 1.\n        Focus \"#t\".\n\
        View:\n    <input id=\"t\" type=\"text\">\n    <p id=\"n\">{n}</p>\n    \
        <button id=\"b\" on-click={go}>go</button>\n\n\
        ## Main\nMount App.\nClick \"#b\".\nShow \"#t\" is focused.\nShow the text of \"#n\".\n";
    assert_eq!(
        output(src),
        ["true", "1"],
        "focus survived, and the turn really did repaint"
    );
}

#[test]
fn focus_survives_a_structural_turn_on_an_element_with_no_id() {
    // ★ FOCUS STAYS WITH THE NODE, AND THE NODE STAYS. The turn pushes a row, so the `{for each}`
    // is structural — and the reconciler keeps every element it can match, the unkeyed `<input>`
    // included, exactly as a browser's does. Focus is never handed to a node that merely landed in
    // the same position: it stays on the one node that had it, because that node was never
    // replaced.
    let src = "## Component App\n\
        State:\n    Let n be 0.\n    Let rows be [\"a\"].\n\
        Actions:\n    To go:\n        Focus \"input\".\n        Push \"b\" to rows.\n\
        View:\n    <input type=\"text\">\n    <p id=\"n\">{n}</p>\n    \
        <ul>{for each r in rows:}<li>{r}</li>{end}</ul>\n    \
        <button id=\"b\" on-click={go}>go</button>\n\n\
        ## Main\nMount App.\nClick \"#b\".\nShow \"input\" is focused.\n";
    assert_eq!(output(src), ["true"]);
}

#[test]
fn focus_survives_a_value_only_turn_even_with_no_id() {
    // ★ THE OTHER HALF, and the reason the limit above had to be re-aimed rather than flipped:
    // a turn that only moves a VALUE patches the node it changed and leaves every other node
    // alone — so the element being focused is the same element afterwards, id or no id.
    //
    // This is the behaviour a visitor actually feels. Under the clear-and-rebuild model every
    // keystroke replaced the input being typed into, and focus survived only because a host
    // reconciler put it back by id; an element with no id simply lost it. Now nothing is
    // destroyed, so there is nothing to put back.
    let src = "## Component App\n\
        State:\n    Let n be 0.\n\
        Actions:\n    To go:\n        Focus \"input\".\n        Set n to n + 1.\n\
        View:\n    <input type=\"text\">\n    <p id=\"n\">{n}</p>\n    \
        <button id=\"b\" on-click={go}>go</button>\n\n\
        ## Main\nMount App.\nClick \"#b\".\nShow \"input\" is focused.\nShow the text of \"#n\".\n";
    assert_eq!(
        output(src),
        ["true", "1"],
        "focus kept, and the turn really did take effect"
    );
}

#[test]
fn focus_lands_on_the_element_that_declares_the_id_and_not_a_neighbour() {
    // The rule is IDENTITY, not position: after a repaint that reorders things, focus has to
    // follow the id it was on.
    let src = "## Component App\n\
        State:\n    Let flip be false.\n\
        Actions:\n    To go:\n        Focus \"#second\".\n        Set flip to true.\n\
        View:\n    {if flip:}<input id=\"extra\" type=\"text\">{end}\n    \
        <input id=\"first\" type=\"text\">\n    <input id=\"second\" type=\"text\">\n    \
        <button id=\"b\" on-click={go}>go</button>\n\n\
        ## Main\nMount App.\nClick \"#b\".\n\
        Show \"#second\" is focused.\nShow \"#first\" is focused.\nShow \"#extra\" is focused.\n";
    assert_eq!(output(src), ["true", "false", "false"]);
}

#[test]
fn an_element_whose_id_follows_state_keeps_its_focus_when_the_id_changes() {
    // The other side of identity by id: an id COMPUTED from state is data about the element, not
    // its name. A turn that changes it patches the attribute on the element the visitor is in —
    // it must not replace the element, which would take the focus with it.
    let src = "## Component App\n\
        State:\n    Let n be 1.\n\
        Actions:\n    To go:\n        Focus \"#f-1\".\n        Set n to 2.\n\
        View:\n    <input id=\"f-{n}\" type=\"text\">\n    \
        <button id=\"b\" on-click={go}>go</button>\n\n\
        ## Main\nMount App.\nClick \"#b\".\n\
        Show \"#f-2\" is focused.\nShow \"#f-1\" is visible.\n";
    assert_eq!(output(src), ["true", "false"]);
}
