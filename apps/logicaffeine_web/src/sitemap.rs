//! Sitemap + prerender route enumeration for SEO.
//!
//! [`prerender_routes`] is the canonical list of crawlable pages: the SSG build
//! prerenders exactly these routes (served via `/api/static_routes`), the sitemap
//! is generated from them, and the tests below hold `public/sitemap.xml` byte-equal
//! to [`generate_sitemap`] so the shipped file can never go stale.
//!
//! # Usage
//!
//! ```
//! use logicaffeine_web::sitemap::generate_sitemap;
//! let xml = generate_sitemap();
//! assert!(!xml.is_empty());
//! ```

use std::fmt::Write;

const BASE_URL: &str = "https://logicaffeine.com";
const CURRENT_DATE: &str = "2026-07-08";

/// Sitemap entry with URL and metadata
pub struct SitemapEntry {
    pub path: String,
    pub changefreq: &'static str,
    pub priority: f32,
}

/// The curated static pages (path, changefreq, priority).
pub fn get_static_routes() -> Vec<SitemapEntry> {
    [
        ("/", "weekly", 1.0),
        ("/guide", "monthly", 0.9),
        ("/learn", "monthly", 0.9),
        ("/crates", "monthly", 0.8),
        ("/studio", "weekly", 0.8),
        ("/benchmarks", "weekly", 0.8),
        ("/pricing", "monthly", 0.8),
        ("/roadmap", "monthly", 0.7),
        ("/news", "weekly", 0.7),
        ("/registry", "weekly", 0.6),
        ("/profile", "monthly", 0.5),
        ("/privacy", "yearly", 0.3),
        ("/terms", "yearly", 0.3),
    ]
    .into_iter()
    .map(|(path, changefreq, priority)| SitemapEntry { path: path.to_string(), changefreq, priority })
    .collect()
}

/// One entry per published news article, derived from the article registry so a
/// new post can never be forgotten here.
pub fn get_news_routes() -> Vec<SitemapEntry> {
    crate::ui::pages::news::get_articles()
        .into_iter()
        .map(|article| SitemapEntry {
            path: format!("/news/{}", article.slug),
            changefreq: "monthly",
            priority: 0.6,
        })
        .collect()
}

/// Every route the SSG build prerenders — the sitemap in path form. `/success`
/// (session-specific, noindex), `/workspace/:subject`, and
/// `/registry/package/:name` (not enumerable at build time) stay client-side.
pub fn prerender_routes() -> Vec<String> {
    get_static_routes()
        .into_iter()
        .chain(get_news_routes())
        .map(|entry| entry.path)
        .collect()
}

/// Generate the complete sitemap XML
pub fn generate_sitemap() -> String {
    let mut xml = String::new();

    writeln!(xml, r#"<?xml version="1.0" encoding="UTF-8"?>"#).unwrap();
    writeln!(xml, r#"<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">"#).unwrap();

    writeln!(xml, "  <!-- Main Pages -->").unwrap();
    for entry in get_static_routes() {
        write_url_entry(&mut xml, entry);
    }

    writeln!(xml, "\n  <!-- News Articles -->").unwrap();
    for entry in get_news_routes() {
        write_url_entry(&mut xml, entry);
    }

    writeln!(xml, "</urlset>").unwrap();

    xml
}

fn write_url_entry(xml: &mut String, entry: SitemapEntry) {
    writeln!(xml, "  <url>").unwrap();
    writeln!(xml, "    <loc>{}{}</loc>", BASE_URL, entry.path).unwrap();
    writeln!(xml, "    <lastmod>{}</lastmod>", CURRENT_DATE).unwrap();
    writeln!(xml, "    <changefreq>{}</changefreq>", entry.changefreq).unwrap();
    writeln!(xml, "    <priority>{:.1}</priority>", entry.priority).unwrap();
    writeln!(xml, "  </url>").unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::router::Route;
    use std::str::FromStr;

    #[test]
    fn every_prerender_route_is_a_real_page() {
        // Each enumerated path must parse to a real Route variant — the catch-all
        // NotFound counts as a broken entry, not a match.
        for path in prerender_routes() {
            let route = Route::from_str(&path)
                .unwrap_or_else(|e| panic!("sitemap path '{path}' does not parse: {e}"));
            assert!(
                !matches!(route, Route::NotFound { .. }),
                "sitemap path '{path}' falls through to the 404 catch-all"
            );
        }
    }

    /// The completeness ratchet: every fully-static route in the [`Route`]
    /// enum must appear in the sitemap or on the explicit exclusion list — a
    /// new page can never be silently forgotten.
    #[test]
    fn every_static_route_is_sitemapped_or_excluded() {
        use dioxus::prelude::Routable;
        use dioxus::router::routable::SegmentType;

        // Session-specific pages that must NOT be crawled or prerendered.
        const EXCLUDED: &[&str] = &["/success"];

        let sitemapped: std::collections::BTreeSet<String> =
            prerender_routes().into_iter().collect();

        let mut static_paths = std::collections::BTreeSet::new();
        for segment in Route::SITE_MAP {
            for chain in segment.flatten() {
                let parts: Option<Vec<&str>> =
                    chain.iter().map(SegmentType::to_static).collect();
                if let Some(parts) = parts {
                    let joined = parts
                        .iter()
                        .filter(|p| !p.is_empty())
                        .copied()
                        .collect::<Vec<_>>()
                        .join("/");
                    static_paths.insert(format!("/{joined}"));
                }
            }
        }
        assert!(
            static_paths.contains("/") && static_paths.contains("/studio"),
            "site-map walk looks broken, found only: {static_paths:?}"
        );

        for path in &static_paths {
            assert!(
                sitemapped.contains(path.as_str()) || EXCLUDED.contains(&path.as_str()),
                "route '{path}' exists but is neither in the sitemap nor \
                 explicitly excluded — add it to get_static_routes() or EXCLUDED"
            );
        }
        // Keep the exclusion list honest: entries must be real routes that we
        // deliberately hide, not leftovers.
        for path in EXCLUDED {
            assert!(
                static_paths.contains(*path),
                "EXCLUDED entry '{path}' is not a static route anymore — drop it"
            );
            assert!(
                !sitemapped.contains(*path),
                "'{path}' is both excluded and sitemapped — pick one"
            );
        }
    }

    /// Crawlers must land exactly where the sitemap points: each entry parses
    /// to a real page and survives the router's boot normalization unchanged
    /// in meaning (parse → serialize → parse is identity).
    #[test]
    fn sitemap_urls_survive_boot_normalization() {
        for path in prerender_routes() {
            let first = Route::from_str(&path)
                .unwrap_or_else(|e| panic!("sitemap path '{path}' does not parse: {e}"));
            let second = Route::from_str(&first.to_string()).unwrap_or_else(|e| {
                panic!("'{path}' re-serialized to something unparseable: {e}")
            });
            assert_eq!(first, second, "'{path}' does not round-trip to the same page");
        }
    }

    /// `lastmod` must never lag the newest published article.
    #[test]
    fn sitemap_lastmod_is_current() {
        let date_ok = |d: &str| {
            d.len() == 10
                && d.bytes().enumerate().all(|(i, b)| match i {
                    4 | 7 => b == b'-',
                    _ => b.is_ascii_digit(),
                })
        };
        assert!(date_ok(CURRENT_DATE), "CURRENT_DATE '{CURRENT_DATE}' is not YYYY-MM-DD");

        for article in crate::ui::pages::news::get_articles() {
            assert!(
                date_ok(article.date),
                "article '{}' has malformed date '{}'",
                article.slug,
                article.date
            );
            // ISO dates compare correctly as strings.
            assert!(
                CURRENT_DATE >= article.date,
                "sitemap lastmod {CURRENT_DATE} lags article '{}' ({}) — bump \
                 CURRENT_DATE and regenerate the shipped sitemap",
                article.slug,
                article.date
            );
        }
    }

    /// Every URL we publish is XML-safe, URL-safe, canonical (lowercase, no
    /// trailing slash), unique, and carries valid metadata.
    #[test]
    fn sitemap_entries_are_hygienic() {
        const CHANGEFREQS: &[&str] =
            &["always", "hourly", "daily", "weekly", "monthly", "yearly", "never"];

        let entries: Vec<SitemapEntry> =
            get_static_routes().into_iter().chain(get_news_routes()).collect();

        let mut seen = std::collections::BTreeSet::new();
        for entry in &entries {
            let path = &entry.path;
            assert!(path.starts_with('/'), "'{path}' must be absolute");
            assert!(
                path == "/" || !path.ends_with('/'),
                "'{path}' must not have a trailing slash"
            );
            assert!(
                path.bytes().all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/')),
                "'{path}' contains characters that are not XML/URL-safe or not lowercase"
            );
            assert!(seen.insert(path.clone()), "'{path}' is listed twice");
            assert!(
                CHANGEFREQS.contains(&entry.changefreq),
                "'{path}' has invalid changefreq '{}'",
                entry.changefreq
            );
            assert!(
                entry.priority > 0.0 && entry.priority <= 1.0,
                "'{path}' has out-of-range priority {}",
                entry.priority
            );
        }
    }

    /// Cloudflare serves robots.txt; it must advertise the sitemap we ship.
    #[test]
    fn robots_txt_points_at_the_sitemap() {
        let robots = std::fs::read_to_string(
            concat!(env!("CARGO_MANIFEST_DIR"), "/public/robots.txt"),
        )
        .expect("public/robots.txt exists");
        assert!(
            robots.contains("Sitemap: https://logicaffeine.com/sitemap.xml"),
            "robots.txt must advertise the sitemap URL"
        );
    }

    #[test]
    fn every_article_is_prerendered() {
        let routes = prerender_routes();
        for article in crate::ui::pages::news::get_articles() {
            let path = format!("/news/{}", article.slug);
            assert!(routes.contains(&path), "article '{}' missing from prerender/sitemap", article.slug);
        }
    }

    #[test]
    fn prerender_list_and_sitemap_agree() {
        let xml = generate_sitemap();
        for path in prerender_routes() {
            let loc = format!("<loc>{BASE_URL}{path}</loc>");
            assert!(xml.contains(&loc), "sitemap XML missing {path}");
        }
        // And nothing session-specific leaks in.
        assert!(!xml.contains("/success"), "sitemap must not list the checkout success page");
        assert!(!xml.contains("/workspace"), "sitemap must not list workspace deep-links");
    }

    #[test]
    fn shipped_sitemap_is_current() {
        // public/sitemap.xml is what Cloudflare serves; hold it byte-equal to the
        // generator so route or article changes can never ship a stale file.
        let shipped = std::fs::read_to_string(
            concat!(env!("CARGO_MANIFEST_DIR"), "/public/sitemap.xml"),
        )
        .expect("public/sitemap.xml exists");
        assert_eq!(
            shipped.replace("\r\n", "\n"),
            generate_sitemap(),
            "public/sitemap.xml is stale — run the regenerate_shipped_sitemap test to refresh it"
        );
    }

    /// Not a check — the regenerator for the shipped file. Run explicitly:
    /// `cargo nextest run -p logicaffeine-web -E 'test(regenerate_shipped_sitemap)' --run-ignored all`
    #[test]
    #[ignore = "writes public/sitemap.xml; run explicitly to regenerate"]
    fn regenerate_shipped_sitemap() {
        std::fs::write(
            concat!(env!("CARGO_MANIFEST_DIR"), "/public/sitemap.xml"),
            generate_sitemap(),
        )
        .expect("write public/sitemap.xml");
    }
}
