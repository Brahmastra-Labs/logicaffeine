//! ★ LOCK — `html of <Page>` renders a page to HTML with no document. ★
//!
//! Server-side rendering: the same view the browser builds as NODES, built as a STRING, because a
//! server has no document. It is what makes a LOGOS page work before — or without — any script
//! running, which is the whole point of the constraint that the framework not need JavaScript.
//!
//! The markup a reader receives is the security boundary that matters most: it reaches people who
//! never load the app, so the escaping tests here are not a nicety.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// Both tiers, byte-identical — a page served by one and hydrated against the other would
/// mismatch on the first node.
fn both(src: &str) -> String {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker must run it:\n{src}");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM must run it:\n{src}");
    // Compared LINE BY LINE, as every other cross-tier lock does: the two tiers differ in
    // whether the final `Show` leaves a trailing newline, which is a harness detail and not a
    // difference in what they rendered.
    assert_eq!(
        tw.output.lines().collect::<Vec<_>>(),
        vm.output.lines().collect::<Vec<_>>(),
        "the two tiers rendered different HTML:\n{src}"
    );
    tw.output.trim().to_string()
}

#[test]
fn a_page_renders_its_markup() {
    let src = "## Component App\nView:\n    <h1>Hello</h1>\n\n## Main\nShow html of App.\n";
    assert_eq!(both(src), "<h1>Hello</h1>");
}

#[test]
fn the_pages_initial_state_is_already_in_the_markup() {
    // The reason SSR exists: a reader with no script running still sees the page's content, so
    // the state has to be RENDERED, not left for a later update to fill in.
    let src = "## Component App\nState:\n    Let name be \"ada\".\nView:\n    \
        <p>Hi {name}</p>\n\n## Main\nShow html of App.\n";
    assert_eq!(both(src), "<p>Hi ada</p>");
}

#[test]
fn a_derived_field_is_computed_for_the_served_markup() {
    let src = "## Component App\nState:\n    Let n be 4.\n    Let doubled always be n * 2.\n\
        View:\n    <p>{doubled}</p>\n\n## Main\nShow html of App.\n";
    assert_eq!(both(src), "<p>8</p>");
}

#[test]
fn attributes_and_nesting_survive() {
    let src = "## Component App\nView:\n    <div class=\"card\"><a href=\"/x\">go</a></div>\n\n\
        ## Main\nShow html of App.\n";
    assert_eq!(both(src), r#"<div class="card"><a href="/x">go</a></div>"#);
}

#[test]
fn a_hole_is_escaped_on_the_way_out() {
    // The security floor, on the tier whose output reaches people who never loaded the app.
    let src = "## Component App\nState:\n    Let bad be \"<img src=x onerror=alert(1)>\".\n\
        View:\n    <p>{bad}</p>\n\n## Main\nShow html of App.\n";
    let out = both(src);
    assert!(!out.contains("<img"), "the payload is not markup: {out}");
    assert!(out.contains("&lt;img"), "it is escaped text: {out}");
}

#[test]
fn a_hole_in_an_attribute_value_is_escaped_too() {
    // A bare quote there ends the attribute and everything after it becomes markup nobody wrote.
    let src = "## Component App\nState:\n    Let t be \"a\\\"b\".\nView:\n    \
        <p title={t}>x</p>\n\n## Main\nShow html of App.\n";
    let out = both(src);
    assert!(
        out.contains("&quot;") || out.contains("&#34;"),
        "the quote is escaped: {out}"
    );
}

#[test]
fn a_conditional_renders_only_the_branch_that_holds() {
    let src = "## Component App\nState:\n    Let ready be true.\nView:\n    \
        {if ready:}<p>yes</p>{else:}<p>no</p>{end}\n\n## Main\nShow html of App.\n";
    assert_eq!(both(src), "<p>yes</p>");
}

#[test]
fn a_loop_renders_one_element_per_item() {
    let src = "## Component App\nState:\n    Let xs be [\"a\", \"b\"].\nView:\n    \
        <ul>{for each x in xs:}<li>{x}</li>{end}</ul>\n\n## Main\nShow html of App.\n";
    assert_eq!(both(src), "<ul><li>a</li><li>b</li></ul>");
}

#[test]
fn a_boolean_attribute_is_a_presence_in_the_served_markup_too() {
    // The same rule the browser tier follows: `disabled="false"` disables the control, so a page
    // served with it is a form nobody can submit.
    let src = "## Component App\nState:\n    Let busy be false.\nView:\n    \
        <button disabled={busy}>go</button>\n\n## Main\nShow html of App.\n";
    assert_eq!(both(src), "<button>go</button>");
}

#[test]
fn a_page_the_renderer_cannot_draw_is_refused_by_name() {
    // A page rendered SHORT is worse than one refused: the reader sees a plausible document and
    // no error anywhere says a piece is missing.
    let src =
        "## Component App\nView:\n    <p>{raw \"<b>x</b>\"}</p>\n\n## Main\nShow html of App.\n";
    let err = tw_outcome(src)
        .error
        .unwrap_or_else(|| panic!("expected a refusal:\n{src}"));
    assert!(
        err.to_lowercase().contains("render") || err.to_lowercase().contains("html"),
        "the refusal says the renderer cannot draw it: {err}"
    );
}

#[test]
fn asking_for_html_of_something_that_is_not_a_component_is_refused() {
    let src = "## Main\nShow html of Nope.\n";
    let err = tw_outcome(src)
        .error
        .unwrap_or_else(|| panic!("expected a refusal:\n{src}"));
    assert!(err.contains("Nope"), "the refusal names it: {err}");
}
