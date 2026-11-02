//! ★ LOCK — a page whose path has `:name` segments is REACHABLE. ★
//!
//! `page_params_lock` (compile crate) proved the binding half: `## Page /posts/:id (id: Int)`
//! reads `42` out of `/posts/42`. That is only worth having if a visitor can GET there.
//!
//! `page_path` returned the declared path verbatim, so a parameterized page published its
//! document, module and shim at the PATTERN: `/posts/:id`, `/posts/:id.html`, and
//! `/posts/:id.<hash>.wasm`. A colon is an illegal filename character on Windows, is
//! percent-encoded inconsistently by CDNs, and is a URL nobody can type. The page compiled,
//! bound its parameter correctly, and shipped assets nothing could load.
//!
//! ★ THE TWO HALVES SHIP TOGETHER. A framework where `## Page /posts/:id` compiles, binds and
//! then 404s is worse than one that refuses the header: the author is told twice that it worked.
//!
//! ★ ONE MATCHING RULE. The page reads its own parameters with `routeParam(pattern, name,
//! routePath())` from `web.lg`; the router picks the page with `page_path_matches`. If those two
//! rules ever differed, a visitor could reach a page that then bound nothing — so this file pins
//! them against each other rather than trusting that they agree.

#[path = "cli_common/mod.rs"]
mod common;

use common::{largo_in, stderr};
use logicaffeine_cli::{page_asset_base, page_path_matches};

fn site_with(pages: &[(&str, &str)]) -> Result<Vec<String>, String> {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = largo_in(dir.path(), &["new", "site", "--web"]);
    assert!(out.status.success(), "largo new --web: {}", stderr(&out));
    let root = dir.path().join("site");
    std::fs::remove_file(root.join("pages/app.lg")).expect("replace the scaffolded page");
    for (name, src) in pages {
        std::fs::write(root.join("pages").join(name), src).expect("write page");
    }
    logicaffeine_cli::plan_site_assets(&root.join("pages"), &root.join("static"))
        .map(|assets| assets.into_iter().map(|(p, _, _)| p).collect())
}

const DETAIL: &str = "## Page /posts/:id (id: Text)\nView:\n    <p id=\"out\">post {id}</p>\n";

#[test]
fn no_published_path_carries_a_literal_colon() {
    let paths = site_with(&[("post.lg", DETAIL)]).expect("plans");
    assert!(
        !paths.iter().any(|p| p.contains(':')),
        "`:id` is a PATTERN, not a URL: nothing a site publishes may contain one, or the build \
         writes a file Windows cannot name and a CDN cannot serve: {paths:?}"
    );
}

#[test]
fn the_document_module_and_shim_all_publish_under_the_safe_base() {
    let paths = site_with(&[("post.lg", DETAIL)]).expect("plans");
    assert!(
        paths.iter().any(|p| p == "/posts/id"),
        "the document: {paths:?}"
    );
    assert!(
        paths.iter().any(|p| p == "/posts/id.html"),
        "and its .html twin: {paths:?}"
    );
    assert!(
        paths
            .iter()
            .any(|p| p.starts_with("/posts/id.") && p.ends_with(".wasm")),
        "the module hangs off the SAME safe base, so it cannot collide with another page's: \
         {paths:?}"
    );
    assert!(
        paths
            .iter()
            .any(|p| p.starts_with("/posts/id.") && p.ends_with(".js")),
        "and so does the shim: {paths:?}"
    );
}

#[test]
fn a_concrete_url_matches_the_pattern_that_declared_it() {
    assert!(
        page_path_matches("/posts/:id", "/posts/42"),
        "the visitor's actual URL"
    );
    assert!(
        page_path_matches("/posts/:id", "/posts/hello-world"),
        "any one segment"
    );
    assert!(
        page_path_matches("/blog/:year/:slug", "/blog/2026/hi"),
        "several slots"
    );
}

#[test]
fn the_pattern_does_not_swallow_paths_it_should_not() {
    // A `:name` matches ONE segment, never a suffix — otherwise `/posts/:id` would claim
    // `/posts/42/edit` and shadow a real page.
    assert!(
        !page_path_matches("/posts/:id", "/posts/42/edit"),
        "one segment, not a prefix"
    );
    assert!(
        !page_path_matches("/posts/:id", "/posts"),
        "counts must agree"
    );
    assert!(
        !page_path_matches("/posts/:id", "/other/42"),
        "literal segments still must match"
    );
    assert!(
        !page_path_matches("/about", "/posts/42"),
        "an unparameterized path is exact"
    );
    assert!(
        page_path_matches("/about", "/about"),
        "and still matches itself"
    );
}

#[test]
fn the_safe_base_keeps_the_parameters_name() {
    // Dropping the colon rather than the word: `/posts/id` says what the slot IS, so the
    // published name is still readable in a directory listing and in a network tab.
    assert_eq!(page_asset_base("/posts/:id"), "/posts/id");
    assert_eq!(page_asset_base("/blog/:year/:slug"), "/blog/year/slug");
    assert_eq!(
        page_asset_base("/about"),
        "/about",
        "an ordinary path is untouched"
    );
    assert_eq!(page_asset_base("/"), "/", "and so is the root");
}

#[test]
fn a_literal_page_at_the_published_path_is_a_loud_collision() {
    // `/posts/:id` publishes at `/posts/id`, so a literal `## Page /posts/id` lands on the same
    // bytes. Claiming the raw pattern would let both through and ship whichever was read last.
    let err = site_with(&[
        ("post.lg", DETAIL),
        (
            "literal.lg",
            "## Page /posts/id\nView:\n    <p>literal</p>\n",
        ),
    ])
    .expect_err("two pages at one published URL must be refused");
    assert!(
        err.contains("post") && err.contains("literal"),
        "the refusal must name BOTH files, since either one could be the mistake: {err}"
    );
}

// ─── The static half. A `dist/` has no server, so the pattern has to travel with it. ───

/// Scaffold a site with one parameterized page and export it, returning `dist/`.
fn exported_site(dir: &std::path::Path) -> std::path::PathBuf {
    let out = largo_in(dir, &["new", "site", "--web"]);
    assert!(out.status.success(), "scaffold: {}", stderr(&out));
    let root = dir.join("site");
    std::fs::write(root.join("pages/post.lg"), DETAIL).unwrap();
    let out = largo_in(&root, &["build", "--static"]);
    assert!(out.status.success(), "static export: {}", stderr(&out));
    root.join("dist")
}

#[test]
fn a_static_export_carries_a_rewrite_for_every_parameterized_page() {
    // ★ THE DEV SERVER AND THE NATIVE BINARY BOTH MATCH THE PATTERN THEMSELVES. A static host
    // cannot: it maps a URL to a file and nothing else. So the pattern has to be written down
    // in the one file those hosts read, or `/posts/42` is a 404 on the only delivery path that
    // has no code of ours running in it — the same page working in development and 404ing in
    // production, which is the divergence this project keeps hunting.
    let dir = tempfile::tempdir().unwrap();
    let dist = exported_site(dir.path());
    let redirects = std::fs::read_to_string(dist.join("_redirects"))
        .expect("a site with a parameterized page must publish `_redirects`");
    assert!(
        redirects.contains("/posts/:id"),
        "the rule has to carry the PATTERN, which is what the host matches on:\n{redirects}"
    );
    assert!(
        redirects.contains("/posts/id"),
        "and point at the document actually published:\n{redirects}"
    );
    assert!(
        redirects.contains("200"),
        "status 200 — a REWRITE, not a redirect: the visitor's URL must stay `/posts/42`, \
         because the page reads its own id back out of the location:\n{redirects}"
    );
}

#[test]
fn a_site_with_no_parameterized_page_publishes_no_rewrites() {
    // A file of noise in every `dist/` teaches people to ignore it.
    let dir = tempfile::tempdir().unwrap();
    let out = largo_in(dir.path(), &["new", "plain", "--web"]);
    assert!(out.status.success(), "scaffold: {}", stderr(&out));
    let root = dir.path().join("plain");
    let out = largo_in(&root, &["build", "--static"]);
    assert!(out.status.success(), "static export: {}", stderr(&out));
    assert!(
        !root.join("dist/_redirects").exists(),
        "no patterns, no `_redirects` — an empty rules file is noise"
    );
}
