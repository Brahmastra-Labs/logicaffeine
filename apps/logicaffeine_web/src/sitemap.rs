//! Sitemap generation for SEO.
//!
//! Provides utilities to generate sitemap.xml from route definitions.
//! This module defines the structure and can be used to regenerate
//! the sitemap when routes change.
//!
//! # Usage
//!
//! Run the sitemap generator to output the XML:
//! ```
//! use logicaffeine_web::sitemap::generate_sitemap;
//! let xml = generate_sitemap();
//! assert!(!xml.is_empty());
//! ```

use std::fmt::Write;

const BASE_URL: &str = "https://logicaffeine.com";
const CURRENT_DATE: &str = "2026-02-02";

/// Sitemap entry with URL and metadata
pub struct SitemapEntry {
    pub path: &'static str,
    pub changefreq: &'static str,
    pub priority: f32,
}

/// All static routes in the application
pub fn get_static_routes() -> Vec<SitemapEntry> {
    vec![
        SitemapEntry { path: "/", changefreq: "weekly", priority: 1.0 },
        SitemapEntry { path: "/guide", changefreq: "monthly", priority: 0.9 },
        SitemapEntry { path: "/learn", changefreq: "monthly", priority: 0.9 },
        SitemapEntry { path: "/crates", changefreq: "monthly", priority: 0.8 },
        SitemapEntry { path: "/studio", changefreq: "weekly", priority: 0.8 },
        SitemapEntry { path: "/pricing", changefreq: "monthly", priority: 0.8 },
        SitemapEntry { path: "/roadmap", changefreq: "monthly", priority: 0.7 },
        SitemapEntry { path: "/news", changefreq: "weekly", priority: 0.7 },
        SitemapEntry { path: "/registry", changefreq: "weekly", priority: 0.6 },
        SitemapEntry { path: "/profile", changefreq: "monthly", priority: 0.5 },
        SitemapEntry { path: "/privacy", changefreq: "yearly", priority: 0.3 },
        SitemapEntry { path: "/terms", changefreq: "yearly", priority: 0.3 },
    ]
}

/// News article entries (add new articles here)
pub fn get_news_routes() -> Vec<SitemapEntry> {
    vec![
        SitemapEntry { path: "/news/introducing-logicaffeine", changefreq: "monthly", priority: 0.6 },
        SitemapEntry { path: "/news/getting-started-with-fol", changefreq: "monthly", priority: 0.6 },
        SitemapEntry { path: "/news/studio-mode-playground", changefreq: "monthly", priority: 0.6 },
    ]
}

/// Generate the complete sitemap XML
pub fn generate_sitemap() -> String {
    let mut xml = String::new();

    writeln!(xml, r#"<?xml version="1.0" encoding="UTF-8"?>"#).unwrap();
    writeln!(xml, r#"<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">"#).unwrap();

    // Static routes
    writeln!(xml, "  <!-- Main Pages -->").unwrap();
    for entry in get_static_routes() {
        write_url_entry(&mut xml, entry);
    }

    // News articles
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

/// Validate that all sitemap URLs are valid routes
/// Returns a list of invalid URLs if any are found
pub fn validate_sitemap_routes() -> Vec<&'static str> {
    let valid_paths = [
        "/", "/guide", "/learn", "/crates", "/studio", "/pricing",
        "/roadmap", "/news", "/registry", "/profile", "/privacy", "/terms",
        // News articles
        "/news/introducing-logicaffeine",
        "/news/getting-started-with-fol",
        "/news/studio-mode-playground",
    ];

    let mut invalid = Vec::new();

    for entry in get_static_routes().iter().chain(get_news_routes().iter()) {
        if !valid_paths.contains(&entry.path) {
            invalid.push(entry.path);
        }
    }

    invalid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sitemap_routes_valid() {
        let invalid = validate_sitemap_routes();
        assert!(
            invalid.is_empty(),
            "Found invalid routes in sitemap: {:?}",
            invalid
        );
    }

    #[test]
    fn test_sitemap_generation() {
        let xml = generate_sitemap();
        assert!(xml.contains("<?xml"));
        assert!(xml.contains("<urlset"));
        assert!(xml.contains("<loc>https://logicaffeine.com/</loc>"));
        assert!(xml.contains("<priority>1.0</priority>"));
    }
}
