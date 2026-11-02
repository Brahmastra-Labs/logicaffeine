//! ★ LOCK — a built site carries the three files every site on the internet has. ★
//!
//! A favicon somebody chose, a `robots.txt`, and a `sitemap.xml`. None of them was
//! generated, and the favicon was worse than absent: a `static/favicon.ico` was SERVED
//! but nothing linked it, so every page still pointed at the built-in clay-λ data URI
//! and the file the author added did nothing. A thing that is there and inert is harder
//! to debug than a thing that is missing.
//!
//! ★ HAND-WRITTEN BEATS GENERATED, ALWAYS. A project that ships its own `robots.txt`
//! keeps it — generation fills a gap, it does not overrule a decision. Every assertion
//! below is paired with the one that proves the project's own file survives.
//!
//! ★ AND A SITEMAP NEEDS TO KNOW ITS OWN URL. `<loc>` is absolute by specification, so a
//! sitemap without a site URL would be invalid XML pointing nowhere. It is generated
//! only when `Largo.toml` declares `site`, and its absence is silent — a site with no
//! declared address has not asked for one.

#[path = "cli_common/mod.rs"]
mod common;
use common::{largo_in, stderr};

use std::path::{Path, PathBuf};

/// A `--web` project with two pages, so a sitemap has something to list.
fn site(dir: &Path, name: &str) -> PathBuf {
    let out = largo_in(dir, &["new", name, "--web"]);
    assert!(out.status.success(), "scaffold: {}", stderr(&out));
    let root = dir.join(name);
    std::fs::write(
        root.join("pages/about.lg"),
        "## Page /about\n\nView:\n    <main><h1>About</h1></main>\n",
    )
    .unwrap();
    root
}

/// The site's planned assets as `(path, bytes)` — the REAL table `largo build` bakes in.
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

/// Declare a site URL in the manifest.
fn set_site_url(root: &Path, url: &str) {
    let manifest = root.join("Largo.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    let with = text.replace("[package]", &format!("[package]\nsite = \"{url}\""));
    assert_ne!(
        with, text,
        "the manifest must have a [package] table:\n{text}"
    );
    std::fs::write(&manifest, with).unwrap();
}

// ── the favicon ────────────────────────────────────────────────────────────────

/// ★ THE LOAD-BEARING ONE — a favicon the author added must be the one the pages link.
#[test]
fn a_projects_own_favicon_is_the_one_every_page_links() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "sitea");
    std::fs::write(root.join("static/favicon.svg"), b"<svg xmlns='x'></svg>").unwrap();

    let table = assets(&root);
    for page in ["/", "/about"] {
        let html = text_at(&table, page).unwrap_or_else(|| panic!("{page} is built"));
        assert!(
            html.contains(r#"href="/favicon.svg""#),
            "every page must link the project's own favicon, not the built-in data URI — a \
             `static/favicon.svg` that is served and linked by nothing is a file the author \
             added that does nothing:\n{html}"
        );
        assert_eq!(
            html.matches("rel=\"icon\"").count(),
            1,
            "and exactly one icon link — two is a browser picking one at random:\n{html}"
        );
    }
}

/// With no favicon of its own, a fresh site is still branded — the built-in stays.
#[test]
fn with_no_favicon_of_its_own_a_site_keeps_the_built_in_one() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "siteb");
    let html = text_at(&assets(&root), "/").expect("the front page is built");
    assert!(
        html.contains("data:image/svg+xml"),
        "the clay-λ default stands when the project names none:\n{html}"
    );
}

// ── robots.txt ─────────────────────────────────────────────────────────────────

#[test]
fn a_built_site_ships_a_robots_txt() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "sitec");
    let robots = text_at(&assets(&root), "/robots.txt").expect("a built site ships /robots.txt");
    assert!(
        robots.contains("User-agent: *"),
        "it must be a robots file a crawler understands:\n{robots}"
    );
}

#[test]
fn a_hand_written_robots_txt_is_kept_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "sited");
    std::fs::write(
        root.join("static/robots.txt"),
        b"User-agent: *\nDisallow: /\n",
    )
    .unwrap();
    let robots = text_at(&assets(&root), "/robots.txt").expect("served");
    assert_eq!(
        robots, "User-agent: *\nDisallow: /\n",
        "a project that wrote its own robots.txt meant it — generation fills a gap, it never \
         overrules a decision, and quietly re-opening a site somebody closed would be the \
         worst version of this feature"
    );
}

// ── sitemap.xml ────────────────────────────────────────────────────────────────

#[test]
fn a_site_that_declares_its_url_gets_a_sitemap_of_every_page() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "sitee");
    set_site_url(&root, "https://acme.example");

    let map = text_at(&assets(&root), "/sitemap.xml").expect("a sitemap is generated");
    assert!(map.contains("<urlset"), "it is a sitemap:\n{map}");
    assert!(
        map.contains("<loc>https://acme.example/</loc>"),
        "the front page is listed at the site's own URL:\n{map}"
    );
    assert!(
        map.contains("<loc>https://acme.example/about</loc>"),
        "and so is every other page — a page missing from the sitemap is a page a crawler may \
         never reach:\n{map}"
    );
    // The `.html` twin is the SAME page at a second URL. Listing both asks a crawler to
    // index one page twice and split its own ranking between them.
    assert!(
        !map.contains("/about.html"),
        "a page's `.html` twin must not be listed beside its canonical URL:\n{map}"
    );
}

/// ★ AND A TRAILING SLASH ON THE DECLARED URL MUST NOT DOUBLE. `https://a.com/` + `/`
/// is `https://a.com//`, a different URL to every crawler and cache.
#[test]
fn a_trailing_slash_on_the_site_url_does_not_double() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "sitef");
    set_site_url(&root, "https://acme.example/");
    let map = text_at(&assets(&root), "/sitemap.xml").expect("a sitemap is generated");
    assert!(!map.contains("//about"), "no doubled slash:\n{map}");
    assert!(
        map.contains("<loc>https://acme.example/about</loc>"),
        "{map}"
    );
}

/// With no declared URL there is no honest sitemap to write, so none is written.
#[test]
fn a_site_with_no_declared_url_gets_no_sitemap() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "siteg");
    assert!(
        text_at(&assets(&root), "/sitemap.xml").is_none(),
        "`<loc>` is absolute by specification: a sitemap with no site URL would point nowhere, \
         and a broken sitemap is worse for a site than no sitemap"
    );
}

/// When the URL IS declared, `robots.txt` points at the sitemap — that is how a crawler
/// finds it without being told.
#[test]
fn robots_points_at_the_sitemap_when_there_is_one() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "siteh");
    set_site_url(&root, "https://acme.example");
    let robots = text_at(&assets(&root), "/robots.txt").expect("served");
    assert!(
        robots.contains("Sitemap: https://acme.example/sitemap.xml"),
        "a generated robots.txt must name the sitemap it was generated beside:\n{robots}"
    );
}

// ── the canonical URL ──────────────────────────────────────────────────────────────

/// The `href` of every `<link rel="canonical">` in a document, whatever order its attributes are
/// written in.
fn canonicals(doc: &str) -> Vec<String> {
    doc.match_indices("rel=\"canonical\"")
        .filter_map(|(i, _)| {
            let tag_start = doc[..i].rfind('<')?;
            let tag_end = doc[i..].find('>').map(|e| i + e + 1)?;
            let tag = &doc[tag_start..tag_end];
            let href = tag.split("href=\"").nth(1)?;
            Some(href[..href.find('"')?].to_string())
        })
        .collect()
}

/// ★ THE SITE'S ADDRESS IS WRITTEN ONCE. `Largo.toml`'s `site` already makes the sitemap's `<loc>`s;
/// the canonical URL each page's document carries comes from the same place, so a site that moves
/// changes one line — and a page never has to hard-code the address it is served at.
#[test]
fn a_declared_site_gives_every_page_its_canonical_url() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "sitei");
    set_site_url(&root, "https://acme.example/");
    let assets = assets(&root);
    let front = text_at(&assets, "/").expect("the front page");
    assert_eq!(
        canonicals(&front),
        vec!["https://acme.example/".to_string()],
        "the front page's canonical URL is the site's own address:\n{front}"
    );
    let about = text_at(&assets, "/about").expect("the about page");
    assert_eq!(
        canonicals(&about),
        vec!["https://acme.example/about".to_string()],
        "every page's canonical URL is its declared path on the site:\n{about}"
    );
}

/// A page that states its own canonical URL — an article syndicated from elsewhere — keeps it, and
/// only it: two canonical links on one document is a contradiction a crawler resolves by ignoring
/// both.
#[test]
fn a_page_that_declares_its_own_canonical_keeps_only_that_one() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "sitej");
    set_site_url(&root, "https://acme.example");
    std::fs::write(
        root.join("pages/about.lg"),
        "## Page /about\n\nHead:\n    <link rel=\"canonical\" href=\"https://elsewhere.example/about\">\n\n\
         View:\n    <main><h1>About</h1></main>\n",
    )
    .unwrap();
    let about = text_at(&assets(&root), "/about").expect("the about page");
    assert_eq!(
        canonicals(&about),
        vec!["https://elsewhere.example/about".to_string()],
        "{about}"
    );
}

/// With no declared address there is no honest canonical URL to write, so none is written.
#[test]
fn a_site_with_no_declared_url_writes_no_canonical_link() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "sitek");
    let front = text_at(&assets(&root), "/").expect("the front page");
    assert!(canonicals(&front).is_empty(), "{front}");
}

/// A parameterized page is one document for a whole family of URLs; its canonical address depends
/// on the id it is showing, which the build does not know — so it is not guessed.
#[test]
fn a_parameterized_page_gets_no_guessed_canonical() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "sitel");
    set_site_url(&root, "https://acme.example");
    std::fs::write(
        root.join("pages/post.lg"),
        "## Page /posts/:id (id: Int)\n\nView:\n    <main><h1>Post</h1></main>\n",
    )
    .unwrap();
    let post =
        text_at(&assets(&root), "/posts/id").expect("the pattern page publishes at /posts/id");
    assert!(
        canonicals(&post).is_empty(),
        "no canonical is invented for `/posts/:id`:\n{post}"
    );
}

/// ★ DEVELOPMENT SHOWS THE DOCUMENT THAT SHIPS — the dev server writes the same canonical link.
#[test]
fn the_dev_server_serves_the_same_canonical_link() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "sitem");
    set_site_url(&root, "https://acme.example");
    let mut server = common::http::Server::start(&root);
    let port = server.port();
    let about = common::http::body(&server.get("/about"));
    assert_eq!(
        canonicals(&about),
        vec!["https://acme.example/about".to_string()],
        "{about}\n{}",
        server.stderr()
    );
}
