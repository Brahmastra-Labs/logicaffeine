//! News articles render their prose whole. An article is written wrapped at a comfortable
//! source width; a line break in the source is a space on the page, so no paragraph breaks
//! mid-sentence on a narrow screen, and no line — however it begins — goes missing.

#![cfg(feature = "web-tests")]

use dioxus::prelude::*;
use dioxus_history::{History, MemoryHistory};
use logicaffeine_web::App;
use std::rc::Rc;

#[path = "web_common/prose_words.rs"]
mod prose_words;

fn render_route(path: &str) -> String {
    let mut vdom = VirtualDom::new(App);
    vdom.provide_root_context(Rc::new(MemoryHistory::with_initial_path(path)) as Rc<dyn History>);
    vdom.rebuild_in_place();
    dioxus_ssr::render(&vdom)
}

/// The paragraphs of `html`, as their visible text.
fn paragraphs(html: &str) -> Vec<String> {
    html.split("<p>")
        .skip(1)
        .filter_map(|rest| rest.split("</p>").next())
        .map(prose_words::visible_text)
        .collect()
}

#[test]
fn every_line_of_every_article_reaches_the_page() {
    let mut lost = Vec::new();
    for article in logicaffeine_web::ui::pages::news::get_articles() {
        let page = prose_words::words(&prose_words::visible_text(&render_route(&format!("/news/{}", article.slug))));
        for line in prose_words::prose_lines(article.content) {
            if !prose_words::contains_run(&page, &prose_words::shown_words(line)) {
                lost.push(format!("[{}] {}", article.slug, line.trim()));
            }
        }
    }
    assert!(lost.is_empty(), "\n{} article line(s) missing from the page:\n  {}\n", lost.len(), lost.join("\n  "));
}

/// The EXODIA article wraps its install paragraph across two source lines; it is one
/// paragraph on the page.
#[test]
fn the_exodia_article_keeps_its_paragraphs_whole() {
    let html = render_route("/news/release-0-10-0-copy-and-patch-jit");
    let install = paragraphs(&html).into_iter().find(|p| p.contains("Prebuilt"));
    let install = install.expect("the EXODIA article has its install paragraph");
    assert!(
        install.contains("no toolchain, no sudo.") && install.contains("shell completions"),
        "the install paragraph is split across several paragraphs; the first is: {install:?}"
    );
}

/// The declarations of `selector`'s rule in the page's stylesheet.
fn css_rule<'a>(html: &'a str, selector: &str) -> &'a str {
    let open = format!("{selector} {{");
    let start = html.find(&open).unwrap_or_else(|| panic!("the page has no `{selector}` rule")) + open.len();
    let end = html[start..].find('}').map_or(html.len(), |len| start + len);
    &html[start..end]
}

/// The EXODIA article carries nine tags; on a phone they take as many rows as they need, and
/// a tag never breaks inside its own name ("Translation / Validation").
#[test]
fn article_tags_wrap_onto_rows_and_never_break_inside() {
    let html = render_route("/news/release-0-10-0-copy-and-patch-jit");
    let row = css_rule(&html, ".article-tags");
    assert!(row.contains("flex-wrap: wrap"), "the tag row runs off the screen instead of wrapping: {row}");
    let tag = css_rule(&html, ".article-tag");
    assert!(tag.contains("white-space: nowrap"), "a tag can break inside its name: {tag}");
}
