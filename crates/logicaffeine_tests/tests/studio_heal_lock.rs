//! LLM Heal in the Studio (work/LLM_HEAL.md, DAT-108), as the page is prerendered: the Heal
//! button stands in Code and Logic modes only, after Run or Execute and before Compile, in the
//! site's blue with the fairy as its icon; at rest there is no fairy and no card on the page; and
//! the fairy the button shows is a file the site serves.
#![cfg(feature = "web-tests")]

#[path = "web_common/page_html.rs"]
mod page_html;

use logicaffeine_web::ui::examples::seeded_examples;
use logicaffeine_web::ui::pages::heal::FAIRY;
use logicaffeine_web::ui::pages::studio::STUDIO_STYLE;
use logicaffeine_web::ui::router::studio_file_url;
use page_html::{elements, has_class, inner, own_attributes, render_route, tags};
use std::path::PathBuf;

fn public_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/logicaffeine_web/public")
}

/// The toolbar's buttons in page order: (class list, inner html).
fn toolbar_buttons(html: &str) -> Vec<(String, String)> {
    let start = html.find("class=\"studio-toolbar\"").expect("the Studio renders its toolbar");
    let end = html[start..].find("class=\"studio-content\"").map_or(html.len(), |i| start + i);
    let bar = &html[start..end];
    elements(bar, "button").into_iter().map(|b| (own_attributes(b).get("class").cloned().unwrap_or_default(), inner(b).to_string())).collect()
}

fn first(mode_path: &str) -> String {
    let spec = seeded_examples().find(|s| s.vfs_path.starts_with(mode_path)).unwrap_or_else(|| panic!("no example under {mode_path}"));
    render_route(&studio_file_url(spec.vfs_path))
}

#[test]
fn code_and_logic_show_one_heal_button_after_run_and_before_compile() {
    for (path, run) in [("/examples/code/basics/", "Run"), ("/examples/logic/", "Execute")] {
        let html = first(path);
        let buttons = toolbar_buttons(&html);
        let heal: Vec<usize> = buttons.iter().enumerate().filter(|(_, (class, _))| class.split(' ').any(|c| c == "heal-btn")).map(|(i, _)| i).collect();
        assert_eq!(heal.len(), 1, "{path}: {} Heal buttons in the toolbar: {buttons:?}", heal.len());
        let (class, inner) = &buttons[heal[0]];
        assert!(class.split(' ').any(|c| c == "execute-btn"), "{path}: Heal is not drawn as a toolbar action: {class}");
        assert!(inner.contains(">Heal<"), "{path}: the button does not read Heal: {inner}");
        assert!(inner.contains(&format!("src=\"{FAIRY}\"")), "{path}: the fairy is not the button's icon: {inner}");
        let run_at = buttons.iter().position(|(_, inner)| inner.contains(run)).unwrap_or_else(|| panic!("{path}: no {run} button"));
        let compile_at = buttons.iter().position(|(_, inner)| inner.contains("Compile")).unwrap_or_else(|| panic!("{path}: no Compile button"));
        assert!(run_at < heal[0] && heal[0] < compile_at, "{path}: Heal is not between {run} and Compile: {buttons:?}");
    }
}

/// A web page is code the worker mends as code; it has no Run, so Heal is its toolbar's one action
/// (Collin, 2026-11-02: the front page's frame opens on a web page and must offer Heal).
#[test]
fn web_shows_one_heal_button_as_its_only_action() {
    let html = first("/examples/code/web/");
    let buttons = toolbar_buttons(&html);
    let actions: Vec<&(String, String)> = buttons.iter().filter(|(class, _)| class.split(' ').any(|c| c == "execute-btn")).collect();
    assert_eq!(actions.len(), 1, "a web page's toolbar actions: {buttons:?}");
    assert!(actions[0].0.split(' ').any(|c| c == "heal-btn") && actions[0].1.contains(">Heal<"), "the one action is not Heal: {actions:?}");
}

#[test]
fn the_other_modes_have_no_heal_button() {
    for path in ["/examples/math/", "/examples/hardware/"] {
        let html = first(path);
        let heal = toolbar_buttons(&html).into_iter().filter(|(class, _)| class.split(' ').any(|c| c == "heal-btn")).count();
        assert_eq!(heal, 0, "{path}: a Heal button in a mode that does not heal");
    }
}

#[test]
fn at_rest_there_is_no_fairy_and_no_card() {
    let html = first("/examples/code/basics/");
    // The stylesheet names the classes; the markup must not.
    let overlays = tags(&html, "div").into_iter().filter(|t| has_class(t, "heal-overlay") || has_class(t, "heal-card")).count();
    let fairies = tags(&html, "img").into_iter().filter(|t| has_class(t, "heal-fairy")).count();
    assert_eq!((overlays, fairies), (0, 0), "the fairy or her card is on the page before Heal is pressed");
    let fairy_icons = tags(&html, "img").into_iter().filter(|t| has_class(t, "heal-btn-icon")).count();
    assert_eq!(fairy_icons, 1, "the fairy appears {fairy_icons} times at rest; once, on her button");
}

#[test]
fn the_fairy_is_served_and_the_button_is_blue() {
    assert!(public_dir().join(FAIRY.trim_start_matches('/')).is_file(), "the site does not serve {FAIRY}");
    assert!(STUDIO_STYLE.contains(".heal-btn {") && STUDIO_STYLE.contains("var(--accent-primary"), "the Heal button is not in the site's blue");
    assert!(STUDIO_STYLE.contains("prefers-reduced-motion"), "the flight does not honour reduced motion");
}
