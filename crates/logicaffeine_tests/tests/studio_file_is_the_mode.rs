//! The Studio has no mode switch: the file you open decides the mode.
//!
//! A mode toggle under the header duplicated what the file browser already said — opening
//! `fibonacci.logos` IS choosing Code — and let the two disagree (Logic mode showing a
//! `.logos` buffer). The mode now comes only from `StudioMode::for_path`, locked in
//! `ui::state`'s tests; every mode stays reachable because some shipped example opens in
//! it, locked in `ui::examples`'s tests. This file pins the rendered page.

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

/// The classes of the removed switch: its container, its buttons, and the mobile "Mode:" label.
const MODE_SWITCH_CLASSES: [&str; 3] = ["mode-toggle", "mode-toggle-btn", "mode-label"];

/// Every class name on every rendered element. Matching whole class names, not substrings,
/// keeps a different control that merely contains the words — the Output/Rust
/// `output-mode-toggle` — from reading as the removed switch.
fn class_names(html: &str) -> Vec<&str> {
    html.split("class=\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .flat_map(str::split_whitespace)
        .collect()
}

/// The mode-switch classes present in `html`, if any.
fn mode_switch_classes(html: &str) -> Vec<&str> {
    class_names(html).into_iter().filter(|c| MODE_SWITCH_CLASSES.contains(c)).collect()
}

#[test]
fn the_matcher_finds_the_switch_and_nothing_else() {
    let switch = r#"<div class="mode-toggle"><button class="mode-toggle-btn active">Logic</button></div><span class="mode-label">Mode:</span>"#;
    assert_eq!(mode_switch_classes(switch), ["mode-toggle", "mode-toggle-btn", "mode-label"]);
    let neighbors = r#"<div class="output-mode-toggle"><button class="output-mode-btn active">Output</button></div><div class="studio-mode-toggle-x"></div>"#;
    assert!(mode_switch_classes(neighbors).is_empty(), "{:?}", mode_switch_classes(neighbors));
}

#[test]
fn the_studio_toolbar_renders() {
    let html = render_route("/studio");
    assert!(html.contains("studio-toolbar"), "the Studio body did not render, so the checks below prove nothing");
}

#[test]
fn the_studio_renders_no_mode_switch() {
    let html = render_route("/studio");
    let found = mode_switch_classes(&html);
    assert!(found.is_empty(), "the Studio renders a mode switch again ({found:?})");
    assert!(!html.contains("Mode:"), "the Studio renders the mode switch's \"Mode:\" label again");
}

#[test]
fn a_deep_linked_studio_renders_no_mode_switch() {
    for file in [
        "examples/code/algorithms/fibonacci.logos",
        "examples/math/natural-numbers.logos",
        "examples/hardware/handshake.hw",
        "examples/code/web/counter.logos",
    ] {
        let html = render_route(&format!("/studio?file={file}"));
        assert!(html.contains("studio-toolbar"), "/studio?file={file} did not render the Studio");
        let found = mode_switch_classes(&html);
        assert!(found.is_empty(), "/studio?file={file} renders a mode switch ({found:?})");
    }
}
