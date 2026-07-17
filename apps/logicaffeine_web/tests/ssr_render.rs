//! Native SSR render safety for every prerendered route.
//!
//! The SSG server (`--features server`, driven by `dx build --ssg`) renders these
//! routes on the native target; any browser API reached from a render path — a
//! component body, a hook initializer, a context provider — aborts right there.
//! Rendering every route here exercises the same target and the same code paths,
//! so a regression can never reach the deploy build. Each route must also carry a
//! page-unique marker, so a route cannot silently render the wrong page (or an
//! empty shell) into its prerendered HTML.

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

#[test]
fn every_prerender_route_renders_on_native() {
    for path in logicaffeine_web::sitemap::prerender_routes() {
        let html = render_route(&path);
        assert!(!html.trim().is_empty(), "route {path} rendered empty HTML");
    }
}

#[test]
fn prerendered_pages_carry_their_content() {
    for (path, marker) in [
        ("/", "Debug Your Thoughts."),
        ("/pricing", "Contact Us"),
        ("/privacy", "Privacy Policy"),
        ("/terms", "Terms of Service"),
        ("/roadmap", "LOGOS Roadmap"),
        ("/guide", "LOGOS Syntax Guide"),
        ("/crates", "Crate Documentation"),
        ("/news", "Latest updates, release notes, and announcements"),
        ("/learn", "Learn Logic"),
        ("/benchmarks", "English-level readability"),
        ("/studio", "LOGICAFFEINE Studio"),
        ("/registry", "Package Registry"),
        ("/profile", "Logic Learner"),
    ] {
        let html = render_route(path);
        assert!(
            html.contains(marker),
            "route {path} is missing its page marker {marker:?}"
        );
    }
}

#[test]
fn every_article_page_carries_its_headline() {
    for article in logicaffeine_web::ui::pages::news::get_articles() {
        let html = render_route(&format!("/news/{}", article.slug));
        assert!(
            html.contains(article.title),
            "article '{}' page is missing its title {:?}",
            article.slug,
            article.title
        );
    }
}

#[test]
fn every_prerendered_page_carries_structured_data() {
    // JSON-LD renders in the body, so crawlers get schema.org data from the
    // prerendered HTML alone — every route in the manifest must carry at least one.
    for path in logicaffeine_web::sitemap::prerender_routes() {
        let html = render_route(&path);
        assert!(
            html.contains("application/ld+json"),
            "route {path} carries no JSON-LD structured data"
        );
    }
}

#[test]
fn landing_showcase_fallback_renders_a_loading_skeleton() {
    // The interactive hero showcase is a lazy component; while the app boots the
    // user must see a loading skeleton, never a blank panel. The native SSG render
    // commits the suspense fallback, so the skeleton markup must be present here —
    // this locks the fallback against regressing back to an empty `<div>`.
    let html = render_route("/");
    assert!(
        html.contains("mini-studio-skeleton") && html.contains("skeleton-line"),
        "landing / prerender is missing the showcase loading skeleton"
    );
}
