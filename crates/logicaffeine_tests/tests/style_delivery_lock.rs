//! ★ LOCK — a `Style:` block REACHES THE ELEMENTS IT NAMES, on both tiers.
//!
//! Reported by the user as "sometimes we can't seem to properly apply the coloring CSS", which is
//! the worst shape a styling bug takes: the page renders, the rule is absent, and nothing anywhere
//! says why. Three independent causes were found, each of which produced exactly that symptom, and
//! each of which every existing style test missed:
//!
//! 1. **A layout's stylesheet was never emitted.** Emission walks the components a page reaches,
//!    and `placed_components` — the ONE walk all three tiers share — dropped `{extends Site}` into
//!    a catch-all. `Site`'s elements were stamped `lgx-site`, its rule would have matched, and the
//!    rule was not in the document. (It dropped a `{block}`'s default body too, so a component
//!    placed there went unreached for the same reason.)
//! 2. **The stylesheet was HTML-escaped.** `<style>` is a RAW TEXT element — HTML does not decode
//!    character references inside it — so `.list > li` was serialized `.list &gt; li`, which is not
//!    a selector, and a browser discards the rule and every declaration in it. Any `Style:` using a
//!    child combinator silently did nothing.
//! 3. **A layout's element lost its scope class on the browser tier.** Layout markup goes through
//!    `lower_layout_element`, a SECOND element emitter that stamped no scope class, so the sheet
//!    said `.page.lgx-site` while the element said `class="page"`. The reference tier got it right,
//!    so the page looked correct in every in-process test and was unstyled in production.
//!
//! ★ THE COMMON SHAPE: all three are a rule and its subject arriving by DIFFERENT paths, and only
//! one path knowing about scoping. That is why these assert the SELECTOR AND THE ELEMENT TOGETHER —
//! either half alone passes while the styling does not apply.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

/// Both tiers, byte for byte — a styling bug that reaches only production is the one that matters.
fn agree(label: &str, src: &str) -> String {
    dom::reset();
    assert_eq!(
        tw_outcome(src).error,
        None,
        "`{label}` must run on the tree-walker"
    );
    let tw = dom::render();
    dom::reset();
    assert_eq!(vm_outcome(src).error, None, "`{label}` must run on the VM");
    let vm = dom::render();
    assert_eq!(
        tw, vm,
        "`{label}` styles differently on the two tiers:\n{src}"
    );
    tw
}

const LAYOUT_PAGE: &str = "\
## Component Site
View:
    <main class=\"page\">{block content}</main>
Style:
    .page { padding: 1rem; }

## Component Chip
View:
    <span class=\"c\">c</span>
Style:
    .c { color: red; }

## Component App
View:
    {extends Site}
    {block content:}<Chip/>{end}

## Main
Mount App.
";

/// A layout's rule is in the document AND its element carries the class that rule requires.
#[test]
fn a_layouts_style_reaches_the_layouts_own_element() {
    let out = agree("layout style", LAYOUT_PAGE);
    assert!(
        out.contains(".page.lgx-site { padding: 1rem; }"),
        "the layout's rule is emitted: {out}"
    );
    assert!(
        out.contains("<main class=\"page lgx-site\">"),
        "and its element carries the scope the rule asks for — a rule whose subject is unstamped \
         matches nothing, which is a page that renders unstyled with no error: {out}"
    );
}

/// A component reached only THROUGH a layout's block still gets its own stylesheet.
#[test]
fn a_component_inside_a_block_still_gets_its_stylesheet() {
    let out = agree("component in a block", LAYOUT_PAGE);
    assert!(
        out.contains(".c.lgx-chip { color: red; }"),
        "the child's rule is emitted too: {out}"
    );
    assert!(
        out.contains("<span class=\"c lgx-chip\">"),
        "and its element is scoped: {out}"
    );
}

/// ★ CSS IS NOT MARKUP. A child combinator must survive serialization intact.
#[test]
fn a_stylesheet_is_not_html_escaped() {
    let out = agree(
        "child combinator",
        "## Component App\nView:\n    <ul class=\"l\"><li>a</li></ul>\n\
         Style:\n    .l > li { color: red; }\n\n## Main\nMount App.\n",
    );
    assert!(
        out.contains(".l > li.lgx-app"),
        "the combinator must be a combinator, not `&gt;` — HTML does not decode entities inside \
         `<style>`, so an escaped selector is one a browser throws away whole: {out}"
    );
    assert!(
        !out.contains("&gt;"),
        "nothing in the sheet may be entity-encoded: {out}"
    );
}

/// The scope class goes on the SUBJECT compound, and in the right position for each selector
/// shape. Each row is a shape that would silently stop matching if it were placed wrongly.
#[test]
fn the_scope_class_lands_on_the_subject_of_every_selector_shape() {
    for (label, css, want) in [
        ("plain class", ".x { color: red; }", ".x.lgx-app"),
        ("element", "p { color: red; }", "p.lgx-app"),
        ("descendant", ".card p { color: red; }", ".card p.lgx-app"),
        // Before the pseudo-class, never after: `.b:hover.lgx-app` is not the same selector.
        (
            "pseudo-class",
            ".b:hover { color: red; }",
            ".b.lgx-app:hover",
        ),
        (
            "pseudo-element",
            ".q::before { content: \"x\"; }",
            ".q.lgx-app::before",
        ),
        (
            "selector list",
            ".a, .b { color: red; }",
            ".a.lgx-app, .b.lgx-app",
        ),
        ("compound", ".a.b { color: red; }", ".a.b.lgx-app"),
        (
            "attribute",
            "input[type=\"text\"] { color: red; }",
            "input[type=\"text\"].lgx-app",
        ),
        // The rule inside the query is scoped; the query itself is not a selector.
        (
            "media query",
            "@media (min-width: 40rem) { .m { color: red; } }",
            ".m.lgx-app",
        ),
    ] {
        let src = format!(
            "## Component App\nView:\n    <p class=\"x\">hi</p>\nStyle:\n    {css}\n\n\
             ## Main\nMount App.\n"
        );
        let out = agree(label, &src);
        assert!(
            out.contains(want),
            "`{label}` must scope to `{want}`: {out}"
        );
    }
}

/// ★ AND A KEYFRAME IS NOT A SELECTOR. `0%` and `100%` are keyframe selectors inside an
/// `@keyframes` block; stamping a class on them would make the animation stop existing.
#[test]
fn keyframe_stops_are_not_scoped() {
    let out = agree(
        "keyframes",
        "## Component App\nView:\n    <p class=\"s\">hi</p>\n\
         Style:\n    @keyframes spin { 0% { opacity: 0; } 100% { opacity: 1; } }\n    \
         .s { animation: spin 1s; }\n\n## Main\nMount App.\n",
    );
    assert!(
        out.contains("0% { opacity: 0; }"),
        "a keyframe stop is untouched: {out}"
    );
    assert!(!out.contains("0%.lgx-app"), "and never scoped: {out}");
    assert!(
        out.contains(".s.lgx-app { animation: spin 1s; }"),
        "the real rule still is: {out}"
    );
}

/// Two components using the SAME class name stay separate — the point of scoping.
#[test]
fn the_same_class_in_two_components_does_not_collide() {
    let out = agree(
        "same class twice",
        "## Component One\nView:\n    <p class=\"t\">1</p>\nStyle:\n    .t { color: red; }\n\n\
         ## Component Two\nView:\n    <p class=\"t\">2</p>\nStyle:\n    .t { color: blue; }\n\n\
         ## Component App\nView:\n    <main><One/><Two/></main>\n\n## Main\nMount App.\n",
    );
    assert!(out.contains(".t.lgx-one { color: red; }"), "one: {out}");
    assert!(out.contains(".t.lgx-two { color: blue; }"), "two: {out}");
    assert!(
        out.contains("<p class=\"t lgx-one\">1</p>"),
        "and each element takes its own: {out}"
    );
    assert!(
        out.contains("<p class=\"t lgx-two\">2</p>"),
        "and each element takes its own: {out}"
    );
}
