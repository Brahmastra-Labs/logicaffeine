//! ★ LOCK — the built-in theme comes first, so anything a site writes beats it. ★
//!
//! The web guide says of the theme that "anything you write beats it", and for a top-level
//! `static/*.css` that was true: the build links those after the theme's `<style>`. A stylesheet a
//! page links in its own `Head:` was written BEFORE the theme, so every rule of equal weight in it
//! lost: the body kept the theme's width and background, links stayed the theme's colour, and the
//! site's `--ink` was the theme's. The same rules behaved differently depending on where the file
//! was linked from.
//!
//! The order is the built-in theme, then the site's own stylesheets, then the page's, in the order
//! the page wrote them — in the built document and in the one the dev server sends.

#[path = "cli_common/mod.rs"]
mod common;
use common::http::{body, Server};
use common::{largo_in, stderr};

use std::path::{Path, PathBuf};

const PAGE: &str = "## Page /\n\nHead:\n    <title>Ordered</title>\n    <link rel=\"stylesheet\" href=\"/a.css\">\n    <link rel=\"stylesheet\" href=\"/c.css\">\n\nView:\n    <main><a href=\"/\">home</a></main>\n";

fn site(dir: &Path) -> PathBuf {
    let root = dir.join("ordered");
    for (rel, text) in [
        ("Largo.toml", "[package]\nname = \"ordered\"\nversion = \"0.1.0\"\n"),
        ("pages/app.lg", PAGE),
        ("static/b.css", "a { color: inherit; }\n"),
    ] {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    root
}

/// The byte offset of `needle` in the document's head, or a failure naming it.
fn at(doc: &str, needle: &str) -> usize {
    let head = &doc[..doc.find("</head>").unwrap_or(doc.len())];
    head.find(needle)
        .unwrap_or_else(|| panic!("the head holds `{needle}`:\n{head}"))
}

fn assert_ordered(doc: &str) {
    let theme = at(doc, "<style>");
    let site_sheet = at(doc, "href=\"/b.css\"");
    let first = at(doc, "href=\"/a.css\"");
    let second = at(doc, "href=\"/c.css\"");
    assert!(
        theme < site_sheet && site_sheet < first && first < second,
        "the theme, then the site's stylesheet, then the page's in the order written \
         (theme {theme}, b.css {site_sheet}, a.css {first}, c.css {second}):\n{doc}"
    );
    assert_eq!(doc.matches("href=\"/a.css\"").count(), 1, "each link once:\n{doc}");
}

#[test]
fn a_built_page_links_its_own_stylesheets_after_the_theme_and_the_site() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path());
    let out = largo_in(&root, &["build", "--static"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let doc = std::fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert_ordered(&doc);
}

#[test]
fn the_dev_server_sends_the_same_order() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path());
    let mut server = Server::start(&root);
    assert_ordered(&body(&server.get("/")));
}

/// Moving the stylesheets moves nothing else: the page's title still replaces the default, and the
/// head's other tags keep their place before the theme.
#[test]
fn the_rest_of_the_head_is_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path());
    let out = largo_in(&root, &["build", "--static"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let doc = std::fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert_eq!(doc.matches("<title>").count(), 1, "{doc}");
    assert!(at(&doc, "<title>Ordered</title>") < at(&doc, "<style>"), "{doc}");
    assert!(at(&doc, "<meta charset=\"utf-8\">") < at(&doc, "<style>"), "{doc}");
}
