//! `public/_redirects`: Cloudflare Pages' redirect table for the deployed site.
//!
//! Pages reads its rules top to bottom and the first match wins, so the SPA fallback
//! (`/* /index.html 200`) must come last: a rule below it never fires. No local server reads the
//! file, so only this lock can hold it. `/pricing` is the old address of the contact page; crawlers
//! and clients without scripts get a real 301 to `/contact` instead of the landing page's HTML
//! served as a soft 404 (the app's router redirects the same URL for everyone else). A prerendered
//! page never appears as a source: Pages applies `_redirects` before it looks for the file, so a
//! rule for `/contact` would hide the contact page itself.

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
fn the_spa_fallback_comes_last() {
    let rules = rules();
    let last = rules.last().expect("_redirects has rules");
    assert_eq!(last, &["/*", "/index.html", "200"], "the SPA fallback is not the last rule");
    assert_eq!(rules.iter().filter(|r| r[0] == "/*").count(), 1, "more than one catch-all rule");
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
