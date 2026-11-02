//! ★ LOCK — the `## Error page` a project declares is the one a visitor gets. ★
//!
//! `## Error page 404` is lexed, is a `ComponentKind`, renders on the interpreter and on
//! the VM, and is covered by `server_surface_lock`. And a grep for `ErrorPage` across the
//! whole of `apps/logicaffeine_cli` returned **nothing** — so neither `largo build` nor
//! `largo build --static` had ever heard of it. A project declared a 404 page, tested it,
//! and shipped a site that served the handler's bare `text/plain` instead.
//!
//! ★ THE FAMILY THIS BELONGS TO. `Stmt::Fetch` was in the AST and refused on every tier;
//! `DomQuery::Visible` was parsed by nothing; `lgx/check.rs` was called by nothing;
//! `ComponentKind::Page { path }` was carried and never read. Each LOOKED supported. This
//! is the same shape one layer further out: supported by the language, and absent from
//! the thing that ships.
//!
//! ★ A STATIC HOST NEEDS THE FILE, NOT THE ROUTE. There is no handler on a CDN, so the
//! 404 page has to exist as `404.html` — the name GitHub Pages, Netlify, Cloudflare Pages
//! and S3 all look for. Same page, two delivery paths, one declaration.

#[path = "cli_common/mod.rs"]
mod common;
use common::{largo_in, stderr};

use std::path::{Path, PathBuf};

/// A site that declares its own 404.
const NOT_FOUND: &str = r#"## Error page 404

View:
    <main>
      <h1>Nothing here</h1>
      <p>That page has moved or never existed.</p>
      <a href="/">Back to Acme</a>
    </main>
"#;

const FRONT: &str = "## Page /\n\nView:\n    <main><h1>Acme</h1></main>\n";

fn site(dir: &Path, name: &str, with_404: bool) -> PathBuf {
    let out = largo_in(dir, &["new", name, "--web"]);
    assert!(out.status.success(), "scaffold: {}", stderr(&out));
    let root = dir.join(name);
    std::fs::write(root.join("pages/app.lg"), FRONT).unwrap();
    if with_404 {
        std::fs::write(root.join("pages/not_found.lg"), NOT_FOUND).unwrap();
    }
    root
}

fn assets(root: &Path) -> Vec<(String, Vec<u8>)> {
    logicaffeine_cli::plan_site_assets(&root.join("pages"), &root.join("static"))
        .expect("the site plans")
        .into_iter()
        .map(|(p, _, b)| (p, b))
        .collect()
}

fn text_at(assets: &[(String, Vec<u8>)], path: &str) -> Option<String> {
    assets
        .iter()
        .find(|(p, _)| p == path)
        .map(|(_, b)| String::from_utf8_lossy(b).to_string())
}

/// ★ THE LOAD-BEARING TEST — a declared 404 page is published as the file every static
/// host serves for an unmatched URL.
#[test]
fn a_declared_error_page_is_published_as_404_html() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "sitea", true);
    let html = text_at(&assets(&root), "/404.html").unwrap_or_else(|| {
        panic!(
            "a project that declares `## Error page 404` must publish it as /404.html — that is \
             the name GitHub Pages, Netlify, Cloudflare Pages and S3 all serve for an unmatched \
             URL, and a static host has no handler to ask."
        )
    });
    assert!(
        html.contains("Nothing here"),
        "with the project's own content:\n{html}"
    );
    assert!(html.contains("That page has moved"), "all of it:\n{html}");
}

/// It is a whole document, not a fragment — a browser showing a bare `<main>` renders
/// unstyled text with no charset.
#[test]
fn the_error_page_is_a_whole_document() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "siteb", true);
    let html = text_at(&assets(&root), "/404.html").expect("published");
    assert!(
        html.starts_with("<!doctype html>"),
        "it is a document:\n{html}"
    );
    assert!(
        html.contains("charset=\"utf-8\""),
        "with the base meta:\n{html}"
    );
    assert!(
        html.contains("<style>"),
        "and the site's theme, so it does not look broken:\n{html}"
    );
}

/// ★ AND A PROJECT THAT DECLARES NONE GETS NONE. Publishing an invented 404 page would
/// put words nobody wrote on somebody's site.
#[test]
fn a_site_that_declares_no_error_page_publishes_no_404() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "sitec", false);
    assert!(
        text_at(&assets(&root), "/404.html").is_none(),
        "a 404 page nobody wrote is words nobody wrote, on their site"
    );
}

/// The static export carries it to disk, where a host can find it.
#[test]
fn the_static_export_writes_404_html() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "sited", true);
    let out = largo_in(&root, &["build", "--static"]);
    assert!(
        out.status.success(),
        "`largo build --static`: {}",
        stderr(&out)
    );
    let html = std::fs::read_to_string(root.join("dist/404.html"))
        .expect("dist/404.html must exist — a static host serves it by name");
    assert!(html.contains("Nothing here"), "{html}");
}

/// ★ AND IT IS NOT A PAGE. `/not_found` must not become a URL of its own: an error page is
/// what a visitor gets when they asked for something else, never a destination somebody
/// links — and publishing it as one would put it in the sitemap.
#[test]
fn an_error_page_is_not_also_an_ordinary_page() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "sitee", true);
    let table = assets(&root);
    assert!(
        text_at(&table, "/not_found").is_none() && text_at(&table, "/not_found.html").is_none(),
        "an error page must not also answer at its file name:\n{:#?}",
        table.iter().map(|(p, _)| p).collect::<Vec<_>>()
    );
}
