//! The roadmap page as it is prerendered.
//!
//! Each card's dot draws its status's glyph; the structured data search engines read lists the
//! timeline the page shows; the Next release block lists what `[Unreleased]` has added so far and
//! links the news written since the last release; and the release history titles each release
//! the way the CHANGELOG parser reads it. Where things are painted is the browser suite's job
//! (`scripts/web/test-site-e2e.sh`); this is what the page says.

#![cfg(feature = "web-tests")]

use dioxus::prelude::*;
use dioxus_history::{History, MemoryHistory};
use logicaffeine_web::App;
use logicaffeine_web::ui::components::icon::IconVariant;
use logicaffeine_web::ui::pages::changelog;
use logicaffeine_web::ui::pages::news::get_articles;
use logicaffeine_web::ui::pages::roadmap::next_release_summary;
use logicaffeine_web::ui::pages::roadmap_data::{get_milestones, Status};
use logicaffeine_web::ui::pages::roadmap_history::get_history;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::OnceLock;

fn roadmap() -> &'static str {
    static HTML: OnceLock<String> = OnceLock::new();
    HTML.get_or_init(|| {
        let mut vdom = VirtualDom::new(App);
        vdom.provide_root_context(Rc::new(MemoryHistory::with_initial_path("/roadmap")) as Rc<dyn History>);
        vdom.rebuild_in_place();
        dioxus_ssr::render(&vdom)
    })
}

fn repo_changelog() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../CHANGELOG.md");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Text as the prerender writes it: the five characters HTML escapes, as numeric references.
fn escaped(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '"' => "&#34;".to_string(),
            '&' => "&#38;".to_string(),
            '\'' => "&#39;".to_string(),
            '<' => "&#60;".to_string(),
            '>' => "&#62;".to_string(),
            c => c.to_string(),
        })
        .collect()
}

/// The markup between `from` and the next `to`, both of which the page must render.
fn between<'a>(html: &'a str, from: &str, to: &str) -> &'a str {
    let start = html.find(from).unwrap_or_else(|| panic!("the roadmap renders no {from}"));
    let end = html[start..].find(to).map(|i| start + i).unwrap_or_else(|| panic!("nothing follows {from} up to {to}"));
    &html[start..end]
}

fn timeline() -> &'static str {
    between(roadmap(), "class=\"timeline\"", "class=\"release-history\"")
}

fn next_release() -> &'static str {
    between(roadmap(), "class=\"next-release\"", "class=\"release-list\"")
}

#[test]
fn each_card_draws_its_status_glyph() {
    let html = timeline();
    for (status, glyph, name) in [
        (Status::Complete, IconVariant::Check, "check"),
        (Status::InProgress, IconVariant::Hammer, "hammer"),
        (Status::Planned, IconVariant::DraftingCompass, "drafting compass"),
    ] {
        let cards = get_milestones().iter().filter(|m| m.status == status).count();
        let drawn = html.matches(glyph.svg_content()).count();
        assert_eq!(drawn, cards, "{cards} {} card(s), {drawn} {name} glyph(s) in the timeline", status.label());
    }
    assert_eq!(
        html.matches(IconVariant::Sparkles.svg_content()).count(),
        0,
        "the timeline still draws the sparkles glyph"
    );
}

#[test]
fn the_structured_data_lists_the_timeline() {
    let lists: Vec<serde_json::Value> = roadmap()
        .split("<script type=\"application/ld+json\">")
        .skip(1)
        .map(|tail| {
            let json = &tail[..tail.find("</script>").expect("a closed JSON-LD script")];
            serde_json::from_str(json).unwrap_or_else(|e| panic!("the roadmap's JSON-LD does not parse ({e}):\n{json}"))
        })
        .filter(|schema: &serde_json::Value| schema["@type"] == "ItemList")
        .collect();
    assert_eq!(lists.len(), 1, "the roadmap should carry exactly one ItemList");
    let list = &lists[0];
    let milestones = get_milestones();
    assert_eq!(list["numberOfItems"], milestones.len(), "numberOfItems disagrees with the timeline");
    let items = list["itemListElement"].as_array().expect("an itemListElement array");
    let named: Vec<(u64, &str)> = items
        .iter()
        .map(|item| (item["position"].as_u64().expect("a position"), item["name"].as_str().expect("a name")))
        .collect();
    let shown: Vec<(u64, &str)> = milestones.iter().enumerate().map(|(i, m)| (i as u64 + 1, m.title)).collect();
    assert_eq!(named, shown, "the structured data names a different timeline than the page shows");
    for (item, milestone) in items.iter().zip(milestones) {
        let description = item["description"].as_str().expect("a description");
        assert!(
            description.starts_with(milestone.status.label()),
            "the structured data for {:?} does not open with its status: {description:?}",
            milestone.title
        );
    }
}

#[test]
fn the_next_release_lists_every_addition() {
    let next = changelog::unreleased(&repo_changelog());
    assert!(!next.added.is_empty(), "[Unreleased] adds nothing, so this test checks nothing");
    let block = next_release();
    for headline in &next.added {
        assert!(block.contains(&escaped(headline)), "the Next release block leaves out {headline:?}");
    }
    let since = &get_history()[0].version;
    if let Some(summary) = next_release_summary(next.changed, next.fixed, since) {
        assert!(block.contains(&escaped(&summary)), "the Next release block does not say {summary:?}");
    }
}

#[test]
fn the_next_release_links_the_news_written_since_the_last_release() {
    let last = &get_history()[0];
    let newest = get_articles().into_iter().filter(|a| a.date > last.date.as_str()).max_by_key(|a| a.date);
    let block = next_release();
    match newest {
        Some(article) => assert!(
            block.contains(&format!("href=\"/news/{}\"", article.slug)),
            "the Next release block does not link {:?}, written {} after v{} ({})",
            article.title,
            article.date,
            last.version,
            last.date
        ),
        None => assert!(!block.contains("href=\"/news/"), "the Next release block links news older than v{}", last.version),
    }
}

#[test]
fn every_release_title_is_read_the_way_the_history_reads_it() {
    let log = repo_changelog();
    for release in get_history() {
        let section = changelog::section(&log, &release.version)
            .unwrap_or_else(|| panic!("CHANGELOG.md has no section for v{}", release.version));
        assert_eq!(
            changelog::release_title(section),
            release.title,
            "v{}: the CHANGELOG parser and scripts/generate-roadmap.sh title it differently",
            release.version
        );
    }
}
