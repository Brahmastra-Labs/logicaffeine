//! ★ LOCK — a View is a REAL VALUE: returned, passed, stored, and placed. ★
//!
//! `## To badge (…) -> View:` is a macro in the Jinja sense and a function in the LOGOS sense.
//! What it returns is an ordinary value that flows through props, collections and `Show` — and
//! placing one in a template moves its nodes into the page. Filters are the same idea from the
//! other side: `{x | upper}` is a function call written left to right.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::dom;

fn rendered(src: &str) -> String {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the program must run cleanly:\n{src}");
    let html = dom::render();
    html.strip_prefix("<body>")
        .and_then(|h| h.strip_suffix("</body>"))
        .unwrap_or(&html)
        .to_string()
}

fn output(src: &str) -> Vec<String> {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the program must run cleanly:\n{src}");
    out.output.lines().map(str::to_string).collect()
}

#[test]
fn a_view_returning_function_places_its_nodes() {
    let src = "\
## To badge (label: Text) -> View:
    <span class=\"badge\">{label}</span>

## Component App
View:
    <p>{badge(\"done\")}</p>

## Main
Mount App.
";
    assert_eq!(rendered(src), "<p><span class=\"badge\">done</span></p>");
}

#[test]
fn a_macro_reads_its_own_parameters_not_the_callers_state() {
    let src = "\
## To badge (label: Text) -> View:
    <span>{label}</span>

## Component App
State:
    Let label be \"page\".
View:
    <p>{badge(\"macro\")}</p>

## Main
Mount App.
";
    assert_eq!(rendered(src), "<p><span>macro</span></p>");
}

#[test]
fn a_macro_may_be_called_twice_with_different_arguments() {
    let src = "\
## To badge (label: Text) -> View:
    <span>{label}</span>

## Component App
View:
    <p>{badge(\"a\")}{badge(\"b\")}</p>

## Main
Mount App.
";
    assert_eq!(rendered(src), "<p><span>a</span><span>b</span></p>");
}

#[test]
fn a_macro_may_be_called_inside_a_loop() {
    let src = "\
## To row (label: Text) -> View:
    <li>{label}</li>

## Component App
State:
    Let items be [\"a\", \"b\"].
View:
    <ul>{for each x in items:}{row(x)}{end}</ul>

## Main
Mount App.
";
    assert_eq!(rendered(src), "<ul><li>a</li><li>b</li></ul>");
}

#[test]
fn a_view_travels_as_a_prop() {
    let src = "\
## To badge (label: Text) -> View:
    <span>{label}</span>

## Component Slot (content: View)
View:
    <div>{content}</div>

## Component App
View:
    <Slot content={badge(\"in a prop\")}/>

## Main
Mount App.
";
    assert_eq!(rendered(src), "<div><span>in a prop</span></div>");
}

#[test]
fn showing_a_view_renders_its_html() {
    let src = "\
## To badge (label: Text) -> View:
    <span class=\"badge\">{label}</span>

## Main
Show badge(\"done\").
";
    assert_eq!(output(src), ["<span class=\"badge\">done</span>"]);
}

#[test]
fn a_view_binds_to_a_view_annotated_name() {
    let src = "\
## To badge (label: Text) -> View:
    <span>{label}</span>

## Main
Let v: View be badge(\"x\").
Show v.
";
    assert_eq!(output(src), ["<span>x</span>"]);
}

#[test]
fn views_live_in_a_collection_and_place_in_order() {
    let src = "\
## To badge (label: Text) -> View:
    <span>{label}</span>

## Component App
State:
    Let badges be [badge(\"a\"), badge(\"b\")].
View:
    <p>{for each b in badges:}{b}{end}</p>

## Main
Mount App.
";
    assert_eq!(rendered(src), "<p><span>a</span><span>b</span></p>");
}

#[test]
fn a_filter_is_a_function_written_left_to_right() {
    let src = "\
## Component App
State:
    Let name be \"ada\".
View:
    <p>{name | upper}</p>

## Main
Mount App.
";
    assert_eq!(rendered(src), "<p>ADA</p>");
}

#[test]
fn filters_chain_in_the_order_they_are_written() {
    let src = "\
## Component App
State:
    Let name be \"  ada  \".
View:
    <p>[{name | trim | upper}]</p>

## Main
Mount App.
";
    assert_eq!(rendered(src), "<p>[ADA]</p>");
}

#[test]
fn a_filter_takes_arguments_after_its_name() {
    let src = "\
## Component App
State:
    Let name be \"ada,bob\".
View:
    <p>{name | replace \",\" \" and \"}</p>

## Main
Mount App.
";
    assert_eq!(rendered(src), "<p>ada and bob</p>");
}

#[test]
fn a_macros_nodes_carry_the_placing_components_scope() {
    // A macro is not a component: what it returns belongs to whoever placed it, so a styled
    // page's rules reach the markup its macros produce.
    let src = "\
## To badge (label: Text) -> View:
    <span class=\"badge\">{label}</span>

## Component App
View:
    <p>{badge(\"done\")}</p>
Style:
    .badge { color: red; }

## Main
Mount App.
";
    let out = rendered(src);
    assert!(
        out.contains("<span class=\"badge lgx-app\">"),
        "the macro's node is scoped: {out}"
    );
    assert!(
        out.contains(".badge.lgx-app { color: red; }"),
        "and the rule matches it: {out}"
    );
}
