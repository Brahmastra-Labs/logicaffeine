//! Native SSR render safety for every prerendered route.
//!
//! The SSG server (`--features server`, driven by `dx build --ssg`) renders these
//! routes on the native target; any browser API reached from a render path — a
//! component body, a hook initializer, a context provider — aborts right there.
//! Rendering every route here exercises the same target and the same code paths,
//! so a regression can never reach the deploy build. Each route must also carry a
//! page-unique marker, so a route cannot silently render the wrong page (or an
//! empty shell) into its prerendered HTML.

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

#[test]
fn every_prerender_route_renders_on_native() {
    for path in logicaffeine_web::sitemap::prerender_routes() {
        let html = render_route(&path);
        assert!(!html.trim().is_empty(), "route {path} rendered empty HTML");
    }
}

/// The benchmarks page shows each benchmark's timings, sources and generated code, and no per-benchmark CPU-profile
/// panel: the flamegraphs ship as the downloadable PDF only.
#[test]
fn the_benchmarks_page_renders_no_under_the_hood_panel() {
    let html = render_route("/benchmarks");
    assert!(html.contains("English-level readability"), "/benchmarks did not render its page");
    assert!(
        !html.contains("Under the hood") && !html.contains("bench-hood"),
        "/benchmarks renders the Under the hood panel"
    );
    assert!(html.contains("flamegraphs.pdf"), "/benchmarks lost its flamegraph PDF download");
}

#[test]
fn prerendered_pages_carry_their_content() {
    for (path, marker) in [
        ("/", "Code we can all read."),
        ("/contact", "Contact the team"),
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
    ] {
        let html = render_route(path);
        assert!(
            html.contains(marker),
            "route {path} is missing its page marker {marker:?}"
        );
    }
}

/// The contact page states the license as LICENSE.md grants it, the MIT License, and gives the company's contact
/// address, which no page under `ui/` may spell out on its own.
#[test]
fn the_contact_page_states_the_license_and_the_company_address() {
    let html = render_route("/contact");
    assert!(html.contains("team@datacenter.dev"), "/contact lacks the company contact address");
    assert!(!html.contains(concat!("brahmastra", "-labs.com")), "/contact still gives the retired mail domain");
    assert!(html.contains("open source under the MIT License"), "/contact does not state the license LICENSE.md grants");

    let ui = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/logicaffeine_web/src/ui");
    let mut stack = vec![ui];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read apps/logicaffeine_web/src/ui") {
            let p = entry.expect("dir entry").path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e == "rs") {
                let src = std::fs::read_to_string(&p).expect("read a ui source file");
                assert!(!src.contains(concat!("brahmastra", "-labs.com")), "{} spells out the retired mail domain", p.display());
            }
        }
    }
}

/// Every page with a footer releases the site under the MIT License and reserves no rights.
#[test]
fn every_footer_releases_the_site_under_the_mit_license() {
    let mut footed = 0;
    for path in logicaffeine_web::sitemap::prerender_routes() {
        let html = render_route(&path);
        if !html.contains("footer-copyright") {
            continue;
        }
        footed += 1;
        assert!(html.contains("Released under the MIT License"), "route {path}'s footer does not release the site under MIT");
        assert!(!html.to_lowercase().contains("rights reserved"), "route {path}'s footer reserves rights MIT grants");
    }
    assert!(footed >= 8, "only {footed} prerendered routes render a footer; the check reads nothing");
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
