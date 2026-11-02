//! ★ LOCK — a component's `Style:` is REAL CSS that reaches only that component's elements. ★
//!
//! No wrapper element, no naming convention to remember, no leakage into a child. Each compound
//! selector is stamped with the component's scope class, and that class is stamped on the
//! elements the component itself renders — so `.card` in `Todos` styles `Todos`'s cards and
//! nothing else, and a child component's `.card` is untouched. `:global(…)` is the deliberate
//! way out.

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

#[test]
fn a_styled_component_emits_its_css_once() {
    let src = "\
## Component App
View:
    <p class=\"card\">hi</p>
Style:
    .card { color: red; }

## Main
Mount App.
";
    let out = rendered(src);
    assert_eq!(out.matches("<style>").count(), 1, "one stylesheet: {out}");
    assert!(
        out.contains(".card.lgx-app { color: red; }"),
        "the selector is scoped: {out}"
    );
}

#[test]
fn the_scope_class_lands_on_the_components_own_elements() {
    let src = "\
## Component App
View:
    <p class=\"card\">hi</p>
Style:
    .card { color: red; }

## Main
Mount App.
";
    assert!(
        rendered(src).contains("class=\"card lgx-app\""),
        "rendered: {}",
        rendered(src)
    );
}

#[test]
fn an_unstyled_component_gets_no_scope_class_and_no_stylesheet() {
    let src = "## Component App\nView:\n    <p class=\"card\">hi</p>\n\n## Main\nMount App.\n";
    let out = rendered(src);
    assert_eq!(
        out, "<p class=\"card\">hi</p>",
        "nothing added when nothing is styled: {out}"
    );
}

#[test]
fn a_childs_elements_do_not_take_the_parents_scope() {
    let src = "\
## Component Row
View:
    <li class=\"card\">row</li>

## Component App
View:
    <div class=\"card\"><Row/></div>
Style:
    .card { color: red; }

## Main
Mount App.
";
    let out = rendered(src);
    assert!(
        out.contains("<div class=\"card lgx-app\">"),
        "the parent's element is scoped: {out}"
    );
    assert!(
        out.contains("<li class=\"card\">"),
        "the child's is NOT: {out}"
    );
}

#[test]
fn each_styled_component_gets_its_own_stylesheet_and_class() {
    let src = "\
## Component Row
View:
    <li class=\"card\">row</li>
Style:
    .card { color: blue; }

## Component App
View:
    <div class=\"card\"><Row/></div>
Style:
    .card { color: red; }

## Main
Mount App.
";
    let out = rendered(src);
    assert!(
        out.contains(".card.lgx-app { color: red; }"),
        "the parent's rule: {out}"
    );
    assert!(
        out.contains(".card.lgx-row { color: blue; }"),
        "the child's rule: {out}"
    );
    assert!(
        out.contains("<li class=\"card lgx-row\">"),
        "the child carries its own scope: {out}"
    );
}

#[test]
fn three_instances_of_one_component_still_emit_one_stylesheet() {
    let src = "\
## Component Row
View:
    <li class=\"card\">row</li>
Style:
    .card { color: blue; }

## Component App
View:
    <ul><Row/><Row/><Row/></ul>

## Main
Mount App.
";
    let out = rendered(src);
    assert_eq!(
        out.matches("<style>").count(),
        1,
        "one per TYPE, not per instance: {out}"
    );
    assert_eq!(
        out.matches("class=\"card lgx-row\"").count(),
        3,
        "every instance is scoped: {out}"
    );
}

#[test]
fn a_global_selector_opts_out_of_scoping() {
    let src = "\
## Component App
View:
    <p class=\"card\">hi</p>
Style:
    :global(body) { margin: 0; }
    .card { color: red; }

## Main
Mount App.
";
    let out = rendered(src);
    assert!(
        out.contains("body { margin: 0; }"),
        "the global rule is left alone: {out}"
    );
    assert!(
        out.contains(".card.lgx-app { color: red; }"),
        "the scoped one is not: {out}"
    );
}

#[test]
fn a_descendant_selector_is_stamped_on_its_subject() {
    let src = "\
## Component App
View:
    <div class=\"card\"><h1>t</h1></div>
Style:
    .card h1 { font-size: 2rem; }

## Main
Mount App.
";
    assert!(
        rendered(src).contains(".card h1.lgx-app { font-size: 2rem; }"),
        "the LAST compound carries the scope: {}",
        rendered(src)
    );
}

#[test]
fn a_keyframes_block_survives_verbatim() {
    let src = "\
## Component App
View:
    <p class=\"card\">hi</p>
Style:
    @keyframes fade { from { opacity: 0; } to { opacity: 1; } }
    .card { animation: fade 1s; }

## Main
Mount App.
";
    let out = rendered(src);
    assert!(
        out.contains("@keyframes fade"),
        "the animation is declared: {out}"
    );
    assert!(
        !out.contains("fade.lgx-app"),
        "an animation name is not a selector: {out}"
    );
}

#[test]
fn a_media_query_scopes_the_rules_inside_it() {
    let src = "\
## Component App
View:
    <p class=\"card\">hi</p>
Style:
    @media (min-width: 40rem) { .card { color: red; } }

## Main
Mount App.
";
    assert!(
        rendered(src).contains(".card.lgx-app { color: red; }"),
        "a group at-rule recurses: {}",
        rendered(src)
    );
}

#[test]
fn the_scope_class_composes_with_a_class_toggle() {
    let src = "\
## Component App
State:
    Let done be true.
View:
    <p class=\"card\" class:done={done}>hi</p>
Style:
    .card { color: red; }

## Main
Mount App.
";
    assert!(
        rendered(src).contains("class=\"card done lgx-app\""),
        "one class list, every source composed: {}",
        rendered(src)
    );
}
