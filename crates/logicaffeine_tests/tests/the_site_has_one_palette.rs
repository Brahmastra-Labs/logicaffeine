//! The site has one palette: there is no theme picker.
//!
//! A footer dropdown once re-skinned the site in seven colourways, but only the global chrome
//! followed it — the Studio kept its own palette, so any choice but the default made the
//! header and the IDE disagree. The picker, its stored choice and the `data-theme` hook are
//! gone; the palette tokens live once, in the global stylesheet. This file pins the rendered
//! pages.

#![cfg(feature = "web-tests")]

use dioxus::prelude::*;
use dioxus_history::{History, MemoryHistory};
use logicaffeine_web::App;
use std::rc::Rc;

fn render_route(path: &str) -> String {
    let mut vdom = VirtualDom::new(App);
    vdom.provide_root_context(Rc::new(MemoryHistory::with_initial_path(path)) as Rc<dyn History>);
    vdom.rebuild_in_place();
    dioxus_ssr::render(&vdom)
}

/// Every prerendered route with its rendered HTML.
fn every_page() -> Vec<(String, String)> {
    logicaffeine_web::sitemap::prerender_routes()
        .into_iter()
        .map(|path| {
            let html = render_route(&path);
            (path, html)
        })
        .collect()
}

/// The classes of the removed picker: the dropdown, its button and menu, each option, the
/// cycle button, and the footer's "Theme:" slot.
const PICKER_CLASSES: [&str; 7] = [
    "theme-dropdown",
    "theme-dropdown-btn",
    "theme-dropdown-menu",
    "theme-option",
    "theme-btn",
    "footer-theme-section",
    "footer-theme-label",
];

/// The palette every page reads: the page background gradient and the two accents, each in
/// its colour, `-rgb` triple and border/glow forms.
const PALETTE_TOKENS: [&str; 10] = [
    "--bg-gradient-start",
    "--bg-gradient-mid",
    "--bg-gradient-end",
    "--accent-primary",
    "--accent-secondary",
    "--accent-tertiary",
    "--accent-primary-rgb",
    "--accent-secondary-rgb",
    "--accent-border",
    "--accent-glow",
];

/// Every class name on every rendered element. Matching whole class names, not substrings,
/// keeps an unrelated class that merely contains the word — a "themed" overlay — from reading
/// as the removed picker.
fn class_names(html: &str) -> Vec<&str> {
    html.split("class=\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .flat_map(str::split_whitespace)
        .collect()
}

/// The picker classes present in `html`, if any.
fn picker_classes(html: &str) -> Vec<&str> {
    class_names(html).into_iter().filter(|c| PICKER_CLASSES.contains(c)).collect()
}

/// How many times `html` defines the custom property `token`. A definition is the name
/// followed by a colon; `var(--accent-primary)` reads it and `--accent-primary-rgb:` is a
/// different token, so neither counts.
fn definitions(html: &str, token: &str) -> usize {
    html.match_indices(token)
        .filter(|(at, _)| html[at + token.len()..].trim_start().starts_with(':'))
        .count()
}

#[test]
fn the_matcher_finds_the_picker_and_nothing_else() {
    let picker = r#"<div class="theme-dropdown open"><button class="theme-dropdown-btn"></button><div class="theme-dropdown-menu"><button class="theme-option sunrise active"></button></div></div><button class="theme-btn"></button><div class="footer-theme-section"><span class="footer-theme-label">Theme:</span></div>"#;
    assert_eq!(picker_classes(picker), PICKER_CLASSES);
    let neighbors = r#"<div class="themed-overlay"></div><div class="studio-theme-x"></div><meta name="theme-color">"#;
    assert!(picker_classes(neighbors).is_empty(), "{:?}", picker_classes(neighbors));
}

#[test]
fn the_definition_counter_tells_a_definition_from_a_read() {
    let css = ":root { --accent-primary: #00d4ff; --accent-primary-rgb: 0, 212, 255; } a { color: var(--accent-primary); }";
    assert_eq!(definitions(css, "--accent-primary"), 1);
    assert_eq!(definitions(css, "--accent-primary-rgb"), 1);
    assert_eq!(definitions(css, "--accent-secondary"), 0);
    assert_eq!(definitions(":root{--accent-glow :rgba(0,0,0,.2)}", "--accent-glow"), 1);
}

#[test]
fn the_footer_renders() {
    let html = render_route("/");
    assert!(
        class_names(&html).contains(&"footer-bottom-links"),
        "the landing page rendered no footer, so the checks below prove nothing"
    );
}

#[test]
fn no_page_offers_a_theme_picker() {
    for (path, html) in every_page() {
        let found = picker_classes(&html);
        assert!(found.is_empty(), "{path} renders the theme picker again ({found:?})");
        assert!(!html.contains(">Theme:<"), "{path} renders the picker's \"Theme:\" label again");
    }
}

#[test]
fn no_page_carries_a_theme_attribute() {
    for (path, html) in every_page() {
        assert!(!html.contains("data-theme"), "{path} carries a data-theme attribute again");
    }
}

#[test]
fn every_page_defines_each_palette_token_exactly_once() {
    for (path, html) in every_page() {
        for token in PALETTE_TOKENS {
            let n = definitions(&html, token);
            assert_eq!(n, 1, "{path} defines {token} {n} times; the palette has one source");
        }
    }
}
