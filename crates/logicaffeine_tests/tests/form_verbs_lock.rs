//! ★ LOCK — the form-driving verbs work, and work on BOTH tiers. ★
//!
//! `Submit` and `Choose` are how a test works a form the way a visitor does. A verb that runs on
//! the reference tier and refuses on the bytecode tier is a page that passes its tests and fails
//! in a browser, which is the exact gap tier parity exists to close.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

/// Both tiers run it, and render the same page.
fn both(src: &str) -> String {
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

fn failure(src: &str) -> String {
    dom::reset();
    tw_outcome(src)
        .error
        .unwrap_or_else(|| panic!("expected a failure:\n{src}"))
}

const FORM: &str = "\
## Component App
State:
    Let n be 0.
    Let choice be \"a\".
Actions:
    To go:
        Set n to n + 1.
View:
    <form id=\"f\" on-submit={go}><button>x</button></form>
    <select id=\"s\" bind={choice}><option>a</option><option>b</option></select>
    <p id=\"n\">{n}</p>
    <p id=\"c\">{choice}</p>

## Main
Mount App.
";

#[test]
fn submitting_a_form_runs_its_action_on_both_tiers() {
    // ★ The VM had the AST, the parser and the reference-tier arms for `Submit` and NO bytecode
    // lowering, so it fell through to the generic "not executable on this tier" refusal — a verb
    // that looked landed and only worked on one tier.
    let out = both(&format!("{FORM}Submit \"#f\".\n"));
    assert!(
        out.contains("<p id=\"n\">1</p>"),
        "the form's action ran: {out}"
    );
}

#[test]
fn submitting_twice_runs_it_twice() {
    let out = both(&format!("{FORM}Submit \"#f\".\nSubmit \"#f\".\n"));
    assert!(out.contains("<p id=\"n\">2</p>"), "{out}");
}

#[test]
fn a_submit_nothing_answers_fails_loudly_on_both_tiers() {
    // A submit that went nowhere would leave every later assertion describing a page nobody
    // touched. Both tiers must refuse, so a test cannot pass on one and mislead on the other.
    let src = format!("{FORM}Submit \"#c\".\n");
    let tw = failure(&src);
    dom::reset();
    let vm = vm_outcome(&src)
        .error
        .unwrap_or_else(|| panic!("the VM must refuse too:\n{src}"));
    assert!(
        tw.contains("#c"),
        "the tree-walker names the selector: {tw}"
    );
    assert!(vm.contains("#c"), "and so does the VM: {vm}");
}

#[test]
fn a_submit_on_a_selector_that_matches_nothing_fails_on_both_tiers() {
    let src = format!("{FORM}Submit \"#nope\".\n");
    let tw = failure(&src);
    dom::reset();
    let vm = vm_outcome(&src)
        .error
        .unwrap_or_else(|| panic!("the VM must refuse too:\n{src}"));
    assert!(tw.contains("#nope"), "{tw}");
    assert!(vm.contains("#nope"), "{vm}");
}

// ---- `Choose V in SEL.` — working a <select> the way a visitor does. ----

#[test]
fn choosing_an_option_writes_the_bound_field() {
    // A `<select>` is not typed into and not clicked: choosing is its own act, and without a verb
    // for it a test can only reach a dropdown by faking the event some other control raises.
    let out = both(&format!("{FORM}Choose \"b\" in \"#s\".\n"));
    assert!(
        out.contains("<p id=\"c\">b</p>"),
        "the bound field moved: {out}"
    );
}

#[test]
fn choosing_leaves_the_select_showing_the_choice() {
    let out = both(&format!("{FORM}Choose \"b\" in \"#s\".\n"));
    assert!(
        out.contains("value=\"b\""),
        "the control shows it too: {out}"
    );
}

#[test]
fn choosing_answers_a_change_handler() {
    // A browser raises `change` when a selection is made, so a page listening for it hears one.
    let src = "## Component App\nState:\n    Let s be \"a\".\n    Let seen be 0.\n\
        Actions:\n    When \"#s\" changes:\n        Set seen to seen + 1.\n\
        View:\n    <select id=\"s\" bind={s}><option>a</option><option>b</option></select>\n    \
        <p id=\"seen\">{seen}</p>\n\n## Main\nMount App.\nChoose \"b\" in \"#s\".\n";
    let out = both(src);
    assert!(
        out.contains("<p id=\"seen\">1</p>"),
        "the change handler ran: {out}"
    );
}

#[test]
fn choosing_in_a_selector_that_matches_nothing_fails_loudly() {
    let src = format!("{FORM}Choose \"b\" in \"#nope\".\n");
    let err = failure(&src);
    assert!(
        err.contains("#nope"),
        "the refusal names the selector: {err}"
    );
}

#[test]
fn choosing_where_nothing_listens_fails_loudly() {
    // The same honesty rule every driving verb follows: a choice nothing hears means the rest of
    // the test describes a page nobody touched.
    let src = "## Component App\nView:\n    <select id=\"s\"><option>a</option></select>\n\n\
        ## Main\nMount App.\nChoose \"b\" in \"#s\".\n";
    let err = failure(src);
    assert!(err.contains("#s"), "the refusal names the selector: {err}");
}
