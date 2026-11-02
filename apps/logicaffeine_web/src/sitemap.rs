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
const CURRENT_DATE: &str = "2026-11-02";

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
        ("/contact", "monthly", 0.8),
        ("/roadmap", "monthly", 0.7),
        ("/news", "weekly", 0.7),
        ("/registry", "weekly", 0.6),
        ("/privacy", "yearly", 0.3),
        ("/terms", "yearly", 0.3),
    ]
    .into_iter()
    .map(|(path, changefreq, priority)| SitemapEntry {
        path: path.to_string(),
        changefreq,
        priority,
    })
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

/// Every route the SSG build prerenders — the sitemap in path form. `/workspace/:subject` and
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
    writeln!(
        xml,
        r#"<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">"#
    )
    .unwrap();

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
#[path = "../../../crates/logicaffeine_tests/unit/logicaffeine_web/sitemap/tests.rs"]
mod tests;
