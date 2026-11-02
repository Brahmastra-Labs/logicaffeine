//! ★★ LOCK — a page is served as HTML at the URL it declares. ★★
//!
//! `Asset::new` derived every content type from the path's EXTENSION, special-casing `/`
//! and `/index.html` to `text/html`. A page's canonical URL has no extension — `/about`,
//! `/blog`, `/contact` — so it fell through to `application/octet-stream`.
//!
//! ★ WHAT THAT DOES IS NOT SUBTLE. A browser handed `application/octet-stream` **offers
//! the file as a download** instead of rendering it. So on any LOGOS site with more than
//! one page, the front page worked and every link off it prompted a save dialog. The
//! front page is the one everybody tests.
//!
//! ★ AND NOTHING SAW IT, because every existing assertion reads the BODY. `largo build`
//! put the right bytes at the right URL, the deploy locks found their content in the
//! response, and the static-export locks read files off disk where a content type does
//! not exist. The header was the one part nobody asked about. Found while compressing
//! assets — the compressible-type allowlist skipped these pages, which is what made a
//! wrong content type finally visible.
//!
//! The rule now: a page document is HTML because it IS one, not because its URL happens
//! to end in `.html`. Guessing from the path is right for `static/**`, where the file
//! name is all there is; it is never right for something the build itself produced and
//! therefore knows the type of.

#[path = "cli_common/mod.rs"]
mod common;
use common::{largo_in, stderr};

use std::path::{Path, PathBuf};

fn site(dir: &Path, name: &str) -> PathBuf {
    let out = largo_in(dir, &["new", name, "--web"]);
    assert!(out.status.success(), "scaffold: {}", stderr(&out));
    let root = dir.join(name);
    std::fs::write(
        root.join("pages/app.lg"),
        "## Page /\n\nView:\n    <main><h1>Home</h1></main>\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages/about.lg"),
        "## Page /about\n\nView:\n    <main><h1>About</h1></main>\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages/deep.lg"),
        "## Page /docs/guide\n\nView:\n    <main><h1>Guide</h1></main>\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages/oops.lg"),
        "## Error page 404\n\nView:\n    <main><h1>Gone</h1></main>\n",
    )
    .unwrap();
    root
}

fn typed(root: &Path) -> Vec<(String, String)> {
    logicaffeine_cli::plan_site_assets(&root.join("pages"), &root.join("static"))
        .expect("the site plans")
        .into_iter()
        .map(|(p, ct, _)| (p, ct))
        .collect()
}

fn content_type(table: &[(String, String)], path: &str) -> String {
    table
        .iter()
        .find(|(p, _)| p == path)
        .map(|(_, ct)| ct.clone())
        .unwrap_or_else(|| panic!("{path} is not in the table:\n{table:#?}"))
}

/// ★ THE LOAD-BEARING TEST — every page, at every URL it answers, is HTML.
#[test]
fn every_page_url_is_served_as_html() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "sitea");
    let table = typed(&root);

    for path in [
        "/",
        "/index.html",
        "/about",
        "/about.html",
        "/docs/guide",
        "/docs/guide.html",
    ] {
        let ct = content_type(&table, path);
        assert!(
            ct.starts_with("text/html"),
            "{path} is served as `{ct}`. A browser handed `application/octet-stream` offers the \
             page as a DOWNLOAD instead of rendering it — so every link off the front page \
             prompted a save dialog."
        );
    }
}

/// The declared error page is HTML too — it is the page somebody sees when something has
/// already gone wrong, which is the worst moment to hand them a download prompt.
#[test]
fn the_error_page_is_served_as_html() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "siteb");
    assert!(content_type(&typed(&root), "/404.html").starts_with("text/html"));
}

/// ★ AND THE SITEMAP IS XML. A crawler is entitled to refuse a sitemap that does not
/// arrive as XML, which would make the file pointless — it exists only to be read by one.
#[test]
fn the_sitemap_is_served_as_xml() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "sitec");
    let manifest = root.join("Largo.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text.replace("[package]", "[package]\nsite = \"https://a.example\""),
    )
    .unwrap();
    let ct = content_type(&typed(&root), "/sitemap.xml");
    assert!(
        ct.contains("xml"),
        "a sitemap must arrive as XML — it exists only to be read by a crawler, and one is \
         entitled to refuse it otherwise. Got `{ct}`."
    );
}

/// ★ STATIC FILES KEEP THE PATH RULE. Guessing from the extension is right for
/// `static/**`, where the file name is genuinely all there is to go on — this fix narrows
/// the guess to where it belongs, it does not remove it.
#[test]
fn a_static_file_is_still_typed_from_its_name() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "sited");
    std::fs::write(root.join("static/logo.svg"), b"<svg xmlns='x'></svg>").unwrap();
    let table = typed(&root);
    assert!(content_type(&table, "/style.css").starts_with("text/css"));
    assert!(content_type(&table, "/logo.svg").contains("svg"));
}
