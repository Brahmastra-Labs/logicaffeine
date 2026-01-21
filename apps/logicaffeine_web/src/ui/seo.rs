//! SEO utilities and JSON-LD structured data generators.
//!
//! Provides structured data schemas for rich results in search engines:
//! - Organization schema (sitewide)
//! - WebSite schema with search action
//! - SoftwareApplication schema (Studio page)
//! - Course schema (Learn page)
//! - Article schema (News articles)
//! - FAQPage schema (Guide page)
//! - BreadcrumbList schema (all pages)
//! - TechArticle schema (Crates page)
//!
//! # Usage
//!
//! ```ignore
//! use crate::ui::seo::{organization_schema, breadcrumb_schema};
//!
//! let org_json = organization_schema();
//! let breadcrumbs = breadcrumb_schema(&[("Home", "/"), ("Learn", "/learn")]);
//! ```

use dioxus::prelude::*;

const BASE_URL: &str = "https://logicaffeine.com";
const LOGO_URL: &str = "https://logicaffeine.com/logo.png";
const ORG_NAME: &str = "LOGICAFFEINE";
const GITHUB_URL: &str = "https://github.com/Brahmastra-Labs/logicaffeine";

/// Generate Organization schema (used on all pages)
pub fn organization_schema() -> String {
    format!(
        r#"{{
  "@context": "https://schema.org",
  "@type": "Organization",
  "name": "{ORG_NAME}",
  "url": "{BASE_URL}",
  "logo": "{LOGO_URL}",
  "description": "Turn everyday English into rigorous First-Order Logic. Debug your thoughts with precision.",
  "sameAs": ["{GITHUB_URL}"]
}}"#
    )
}

/// Generate WebSite schema with search action
pub fn website_schema() -> String {
    format!(
        r#"{{
  "@context": "https://schema.org",
  "@type": "WebSite",
  "name": "{ORG_NAME}",
  "url": "{BASE_URL}",
  "description": "Turn everyday English into rigorous First-Order Logic",
  "potentialAction": {{
    "@type": "SearchAction",
    "target": "{BASE_URL}/registry?q={{search_term}}",
    "query-input": "required name=search_term"
  }}
}}"#
    )
}

/// Generate SoftwareApplication schema (for Studio page)
pub fn software_application_schema() -> String {
    format!(
        r#"{{
  "@context": "https://schema.org",
  "@type": "SoftwareApplication",
  "name": "LOGICAFFEINE Studio",
  "applicationCategory": "DeveloperApplication",
  "operatingSystem": "Web Browser",
  "description": "Interactive playground for experimenting with First-Order Logic translations",
  "url": "{BASE_URL}/studio",
  "offers": {{
    "@type": "Offer",
    "price": "0",
    "priceCurrency": "USD"
  }},
  "provider": {{
    "@type": "Organization",
    "name": "{ORG_NAME}",
    "url": "{BASE_URL}"
  }}
}}"#
    )
}

/// Generate Course schema (for Learn page)
pub fn course_schema() -> String {
    format!(
        r#"{{
  "@context": "https://schema.org",
  "@type": "Course",
  "name": "First-Order Logic Fundamentals",
  "description": "Learn to translate everyday English into rigorous First-Order Logic through interactive exercises and real-world examples.",
  "url": "{BASE_URL}/learn",
  "provider": {{
    "@type": "Organization",
    "name": "{ORG_NAME}",
    "url": "{BASE_URL}"
  }},
  "educationalLevel": "Beginner to Advanced",
  "isAccessibleForFree": true,
  "inLanguage": "en",
  "teaches": ["First-Order Logic", "Formal Logic", "Logical Reasoning", "Symbolic Logic"]
}}"#
    )
}

/// Generate Article schema for news articles
pub fn article_schema(
    headline: &str,
    description: &str,
    date_published: &str,
    slug: &str,
) -> String {
    format!(
        r#"{{
  "@context": "https://schema.org",
  "@type": "Article",
  "headline": "{headline}",
  "description": "{description}",
  "datePublished": "{date_published}",
  "dateModified": "{date_published}",
  "url": "{BASE_URL}/news/{slug}",
  "author": {{
    "@type": "Organization",
    "name": "{ORG_NAME}",
    "url": "{BASE_URL}"
  }},
  "publisher": {{
    "@type": "Organization",
    "name": "{ORG_NAME}",
    "logo": {{
      "@type": "ImageObject",
      "url": "{LOGO_URL}"
    }}
  }}
}}"#
    )
}

/// Generate FAQPage schema
pub fn faq_schema(questions: &[(&str, &str)]) -> String {
    let qa_items: Vec<String> = questions
        .iter()
        .map(|(q, a)| {
            format!(
                r#"{{
      "@type": "Question",
      "name": "{}",
      "acceptedAnswer": {{
        "@type": "Answer",
        "text": "{}"
      }}
    }}"#,
                q, a
            )
        })
        .collect();

    format!(
        r#"{{
  "@context": "https://schema.org",
  "@type": "FAQPage",
  "mainEntity": [
    {}
  ]
}}"#,
        qa_items.join(",\n    ")
    )
}

/// Generate TechArticle schema (for Crates documentation)
pub fn tech_article_schema(title: &str, description: &str, path: &str) -> String {
    format!(
        r#"{{
  "@context": "https://schema.org",
  "@type": "TechArticle",
  "headline": "{title}",
  "description": "{description}",
  "url": "{BASE_URL}{path}",
  "author": {{
    "@type": "Organization",
    "name": "{ORG_NAME}"
  }},
  "publisher": {{
    "@type": "Organization",
    "name": "{ORG_NAME}",
    "logo": {{
      "@type": "ImageObject",
      "url": "{LOGO_URL}"
    }}
  }},
  "proficiencyLevel": "Expert"
}}"#
    )
}

/// Generate Product schema (for Pricing page)
pub fn product_schema() -> String {
    format!(
        r#"{{
  "@context": "https://schema.org",
  "@type": "Product",
  "name": "LOGICAFFEINE Pro",
  "description": "Professional subscription for advanced First-Order Logic tools with Z3 verification",
  "url": "{BASE_URL}/pricing",
  "brand": {{
    "@type": "Organization",
    "name": "{ORG_NAME}"
  }},
  "offers": [
    {{
      "@type": "Offer",
      "name": "Pro Monthly",
      "price": "9.99",
      "priceCurrency": "USD",
      "availability": "https://schema.org/InStock"
    }},
    {{
      "@type": "Offer",
      "name": "Pro Yearly",
      "price": "99.99",
      "priceCurrency": "USD",
      "availability": "https://schema.org/InStock"
    }}
  ]
}}"#
    )
}

/// Breadcrumb item for schema generation
pub struct BreadcrumbItem {
    pub name: &'static str,
    pub path: &'static str,
}

/// Generate BreadcrumbList schema
pub fn breadcrumb_schema(items: &[BreadcrumbItem]) -> String {
    let list_items: Vec<String> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            format!(
                r#"{{
      "@type": "ListItem",
      "position": {},
      "name": "{}",
      "item": "{}{}"
    }}"#,
                i + 1,
                item.name,
                BASE_URL,
                item.path
            )
        })
        .collect();

    format!(
        r#"{{
  "@context": "https://schema.org",
  "@type": "BreadcrumbList",
  "itemListElement": [
    {}
  ]
}}"#,
        list_items.join(",\n    ")
    )
}

/// Page metadata for SEO
pub struct PageMeta {
    pub title: &'static str,
    pub description: &'static str,
    pub canonical_path: &'static str,
    pub og_image: Option<&'static str>,
}

/// Default page metadata
impl Default for PageMeta {
    fn default() -> Self {
        Self {
            title: "LOGICAFFEINE - Debug Your Thoughts",
            description: "Turn everyday English into rigorous First-Order Logic. Debug your thoughts with precision.",
            canonical_path: "/",
            og_image: Some("/og-image.png"),
        }
    }
}

/// Page-specific metadata definitions
pub mod pages {
    use super::PageMeta;

    pub const LANDING: PageMeta = PageMeta {
        title: "LOGICAFFEINE - Debug Your Thoughts",
        description: "Turn everyday English into rigorous First-Order Logic. Debug your thoughts with precision.",
        canonical_path: "/",
        og_image: Some("/og-image.png"),
    };

    pub const LEARN: PageMeta = PageMeta {
        title: "Learn First-Order Logic - LOGICAFFEINE",
        description: "Master First-Order Logic through interactive exercises. From syllogisms to modal logic, learn to reason precisely.",
        canonical_path: "/learn",
        og_image: Some("/og-learn.png"),
    };

    pub const STUDIO: PageMeta = PageMeta {
        title: "Studio - LOGICAFFEINE",
        description: "Interactive playground for experimenting with First-Order Logic translations. Try examples and see results in real-time.",
        canonical_path: "/studio",
        og_image: Some("/og-studio.png"),
    };

    pub const GUIDE: PageMeta = PageMeta {
        title: "Documentation - LOGICAFFEINE",
        description: "Comprehensive guide to LOGICAFFEINE syntax, features, and First-Order Logic concepts.",
        canonical_path: "/guide",
        og_image: Some("/og-guide.png"),
    };

    pub const PRICING: PageMeta = PageMeta {
        title: "Pricing - LOGICAFFEINE",
        description: "Choose the right plan for your logic needs. Free tier available with premium features for professionals.",
        canonical_path: "/pricing",
        og_image: Some("/og-pricing.png"),
    };

    pub const CRATES: PageMeta = PageMeta {
        title: "Crates Documentation - LOGICAFFEINE",
        description: "Technical documentation for LOGICAFFEINE Rust crates. Integrate First-Order Logic parsing into your applications.",
        canonical_path: "/crates",
        og_image: Some("/og-crates.png"),
    };

    pub const ROADMAP: PageMeta = PageMeta {
        title: "Roadmap - LOGICAFFEINE",
        description: "See what's coming next for LOGICAFFEINE. Track our progress and upcoming features.",
        canonical_path: "/roadmap",
        og_image: Some("/og-roadmap.png"),
    };

    pub const NEWS: PageMeta = PageMeta {
        title: "News - LOGICAFFEINE",
        description: "Latest updates, release notes, and announcements from LOGICAFFEINE.",
        canonical_path: "/news",
        og_image: Some("/og-news.png"),
    };
}

/// Render JSON-LD script tag for a schema
#[component]
pub fn JsonLd(schema: String) -> Element {
    rsx! {
        script {
            r#type: "application/ld+json",
            dangerous_inner_html: "{schema}"
        }
    }
}

/// Render multiple JSON-LD schemas
#[component]
pub fn JsonLdMultiple(schemas: Vec<String>) -> Element {
    rsx! {
        for schema in schemas.iter() {
            script {
                r#type: "application/ld+json",
                dangerous_inner_html: "{schema}"
            }
        }
    }
}
