//! `public/_redirects`: Cloudflare Pages' redirect table for the deployed site.
//!
//! Pages reads its rules top to bottom and the first match wins. `/pricing` is the old address of
//! the contact page; crawlers and clients without scripts get a real 301 to `/contact` instead of
//! the landing page's HTML served as a soft 404 (the app's router redirects the same URL for
//! everyone else). A prerendered page never appears as a source: Pages applies `_redirects` before
//! it looks for the file, so a rule for `/contact` would hide the contact page itself.
//!
//! No rule catches every path (DAT-171). The build writes a root `404.html`, which Pages serves
//! with a 404 status for any path no file and no rule answers, so an unknown address is a real
//! 404 rather than the landing page with a 200. The routes the build cannot enumerate
//! (`sitemap::CLIENT_SIDE_ROUTES`) each have one rewrite to the app shell, and those are the only
//! rewrites. `scripts/web/pages-server.py` serves the build by these rules for the local checks.

use std::path::PathBuf;

fn redirects() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/logicaffeine_web/public/_redirects");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Each rule as its fields, comments and blank lines skipped.
fn rules() -> Vec<Vec<String>> {
    redirects()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split_whitespace().map(str::to_string).collect())
        .collect()
}

#[test]
fn every_rule_has_a_source_a_destination_and_a_status() {
    for rule in rules() {
        assert_eq!(rule.len(), 3, "a rule is `source destination status`: {rule:?}");
        assert!(rule[0].starts_with('/'), "a rule's source is a path: {rule:?}");
        assert!(matches!(rule[2].as_str(), "200" | "301" | "302" | "307" | "308"), "a rule's status: {rule:?}");
    }
}

#[test]
fn no_rule_catches_every_path() {
    // A `/*` rule answers every unknown address with the landing page and a 200, a soft 404 that
    // crawlers index under the landing page's canonical address.
    let rules = rules();
    let catch_all: Vec<&Vec<String>> = rules.iter().filter(|r| r[0] == "/*" || r[0] == "/").collect();
    assert!(catch_all.is_empty(), "_redirects catches every path: {catch_all:?}");
}

#[test]
fn every_rewrite_serves_the_app_shell_below_a_route() {
    for rule in rules().iter().filter(|r| r[2] == "200") {
        assert_eq!(rule[1], "/index.html", "a rewrite serves something other than the app shell: {rule:?}");
        assert!(rule[0].ends_with("/*") && rule[0].len() > 2, "a rewrite is not a route's subtree: {rule:?}");
    }
}

#[cfg(feature = "web-tests")]
#[test]
fn the_rewrites_are_the_client_side_routes() {
    // Each route the build cannot prerender loads the app at any value of its parameter, and no
    // other path is rewritten, so every other unknown address reaches the 404 page.
    let rewrites: std::collections::BTreeSet<String> =
        rules().into_iter().filter(|r| r[2] == "200").map(|r| r[0].clone()).collect();
    let client_side: std::collections::BTreeSet<String> =
        logicaffeine_web::sitemap::CLIENT_SIDE_ROUTES.iter().map(|r| r.to_string()).collect();
    assert_eq!(rewrites, client_side, "the rewrites in _redirects are not the client-side routes");
}

#[test]
fn pricing_is_a_permanent_redirect_to_contact() {
    let rules = rules();
    for source in ["/pricing", "/pricing/"] {
        let rule = rules.iter().find(|r| r[0] == source).unwrap_or_else(|| panic!("_redirects has no rule for {source}"));
        assert_eq!(rule[1], "/contact", "{source} redirects to {}", rule[1]);
        assert_eq!(rule[2], "301", "{source} is not a permanent redirect");
    }
}

#[cfg(feature = "web-tests")]
#[test]
fn no_prerendered_page_is_redirected_away() {
    // Pages consults `_redirects` before the files it serves, so a rule whose source is a
    // prerendered route would replace that page with the redirect (this is how `/contact` once
    // pointed at the pricing form it had replaced).
    let rules = rules();
    for route in logicaffeine_web::sitemap::prerender_routes() {
        let with_slash = format!("{route}/");
        let hit = rules.iter().find(|r| r[0] == route || r[0] == with_slash);
        assert!(hit.is_none(), "{route} is prerendered but _redirects sends it elsewhere: {hit:?}");
    }
}

#[test]
fn profile_is_a_permanent_redirect_to_learn() {
    // The profile page is gone; Learn's roadmap shows the progress it used to. A bookmark or a
    // crawler that still asks for /profile gets a real 301 to /learn.
    let rules = rules();
    for source in ["/profile", "/profile/"] {
        let rule = rules.iter().find(|r| r[0] == source).unwrap_or_else(|| panic!("_redirects has no rule for {source}"));
        assert_eq!(rule[1], "/learn", "{source} redirects to {}", rule[1]);
        assert_eq!(rule[2], "301", "{source} is not a permanent redirect");
    }
}
