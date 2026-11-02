//! ★ LOCK — component-scoped CSS by a real tokenizer, not string surgery. ★
//!
//! `lgx::css::scope(css, class)` stamps the component's scope class onto the SUBJECT
//! compound of every complex selector (`.card h1` → `.card h1.lgx-todos`), so a rule can
//! only match the component's own elements (they carry the class; a child component's
//! elements do not) — no wrapper element, no leakage. `:global(…)` opts a compound out.
//! Group at-rules (`@media`, `@supports`, `@container`, `@layer`, `@scope`,
//! `@starting-style`) recurse into their rule lists; every other at-rule (`@keyframes`,
//! `@font-face`, `@page`, `@property`, `@import …;`) passes through verbatim — the exact
//! rule `wasm_dom.rs`'s at-rule lock has always pinned. Declarations are never touched.

use logicaffeine_language::lgx::css::{scope, CssErrorKind};

fn s(css: &str) -> String {
    scope(css, "lgx-t").unwrap_or_else(|e| panic!("{css:?} must scope: {e:?}"))
}

#[test]
fn class_element_and_attribute_subjects_get_the_scope_class() {
    assert_eq!(s(".card { color: red }"), ".card.lgx-t { color: red }");
    assert_eq!(s("li { display: flex }"), "li.lgx-t { display: flex }");
    assert_eq!(
        s("input[type=text] { width: 100% }"),
        "input[type=text].lgx-t { width: 100% }"
    );
    assert_eq!(s("#root { margin: 0 }"), "#root.lgx-t { margin: 0 }");
    assert_eq!(
        s("* { box-sizing: border-box }"),
        ".lgx-t { box-sizing: border-box }"
    );
}

#[test]
fn only_the_subject_compound_of_a_complex_selector_is_stamped() {
    assert_eq!(s(".card h1 { margin: 0 }"), ".card h1.lgx-t { margin: 0 }");
    assert_eq!(
        s(".a > .b + .c ~ .d { x: y }"),
        ".a > .b + .c ~ .d.lgx-t { x: y }"
    );
    assert_eq!(
        s(".todos .del { color: red }"),
        ".todos .del.lgx-t { color: red }"
    );
}

#[test]
fn selector_lists_stamp_every_selector() {
    assert_eq!(
        s(".a, .b { margin: 0 }"),
        ".a.lgx-t, .b.lgx-t { margin: 0 }"
    );
    assert_eq!(
        s("h1,\nh2 { margin: 0 }"),
        "h1.lgx-t, h2.lgx-t { margin: 0 }"
    );
}

#[test]
fn pseudo_classes_and_elements_stay_after_the_stamp() {
    assert_eq!(
        s(".a:hover { color: red }"),
        ".a.lgx-t:hover { color: red }"
    );
    assert_eq!(
        s(".a::before { content: '' }"),
        ".a.lgx-t::before { content: '' }"
    );
    assert_eq!(
        s("li:nth-child(2n+1) { color: red }"),
        "li.lgx-t:nth-child(2n+1) { color: red }"
    );
    assert_eq!(
        s(".a:not(.b) { color: red }"),
        ".a.lgx-t:not(.b) { color: red }"
    );
}

#[test]
fn global_opts_a_compound_out_of_scoping() {
    assert_eq!(s(":global(body) { margin: 0 }"), "body { margin: 0 }");
    assert_eq!(
        s(":global(.theme-dark) .card { color: white }"),
        ".theme-dark .card.lgx-t { color: white }"
    );
    assert_eq!(
        s(".card :global(.icon) { fill: red }"),
        ".card.lgx-t .icon { fill: red }"
    );
}

#[test]
fn group_at_rules_recurse_and_other_at_rules_pass_through_verbatim() {
    assert_eq!(
        s("@media (max-width: 600px) { .card { padding: 0 } }"),
        "@media (max-width: 600px) { .card.lgx-t { padding: 0 } }"
    );
    assert_eq!(
        s("@supports (display: grid) { .g { display: grid } }"),
        "@supports (display: grid) { .g.lgx-t { display: grid } }"
    );
    assert_eq!(
        s("@layer base { .a { x: y } }"),
        "@layer base { .a.lgx-t { x: y } }"
    );
    assert_eq!(
        s("@keyframes pulse { 0% { opacity: 0 } to { opacity: 1 } }"),
        "@keyframes pulse { 0% { opacity: 0 } to { opacity: 1 } }"
    );
    assert_eq!(
        s("@font-face { font-family: X; src: url(x.woff2) }"),
        "@font-face { font-family: X; src: url(x.woff2) }"
    );
    assert_eq!(
        s("@import url(\"x.css\"); .a { color: blue }"),
        "@import url(\"x.css\"); .a.lgx-t { color: blue }"
    );
    assert_eq!(
        s("@property --x { syntax: '<length>'; inherits: false; initial-value: 0px }"),
        "@property --x { syntax: '<length>'; inherits: false; initial-value: 0px }"
    );
}

#[test]
fn declarations_strings_and_urls_are_never_touched() {
    assert_eq!(
        s(".a { content: \"{ not a block }\"; background: url(a{b}.png) }"),
        ".a.lgx-t { content: \"{ not a block }\"; background: url(a{b}.png) }"
    );
    assert_eq!(
        s(".a { grid-template-areas: \"h h\" \"s m\" }"),
        ".a.lgx-t { grid-template-areas: \"h h\" \"s m\" }"
    );
}

#[test]
fn comments_are_removed_and_rules_keep_their_order() {
    assert_eq!(
        s("/* top */ .a { x: y } /* mid */ .b { z: w }"),
        ".a.lgx-t { x: y } .b.lgx-t { z: w }"
    );
}

#[test]
fn an_empty_or_declaration_only_sheet_is_fine() {
    assert_eq!(s(""), "");
    assert_eq!(s("   \n  "), "");
}

#[test]
fn unbalanced_braces_are_a_precise_error() {
    let e = scope(".a { color: red", "lgx-t").expect_err("unclosed block");
    assert_eq!(e.kind, CssErrorKind::UnbalancedBraces);
    let e = scope(".a { color: red } }", "lgx-t").expect_err("stray close");
    assert_eq!(e.kind, CssErrorKind::UnbalancedBraces);
    assert!(!e.message().is_empty());
}

#[test]
fn hostile_css_never_panics() {
    for css in [
        "{",
        "}",
        "{}",
        "@",
        "@media",
        "@media {",
        ":global(",
        ":global()",
        "a,,b{}",
        "\"",
        "url(",
        "/*",
        "a{b:c",
        "a{{}}",
        "@keyframes {",
        ";;;",
        "\u{0}",
    ] {
        let _ = scope(css, "lgx-t");
    }
}
