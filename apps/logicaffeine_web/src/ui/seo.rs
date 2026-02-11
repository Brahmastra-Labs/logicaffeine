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
//! ```no_run
//! use logicaffeine_web::ui::seo::{organization_schema, breadcrumb_schema, BreadcrumbItem};
//!
//! let org_json = organization_schema();
//! let breadcrumbs = breadcrumb_schema(&[
//!     BreadcrumbItem { name: "Home", path: "/" },
//!     BreadcrumbItem { name: "Learn", path: "/learn" },
//! ]);
//! ```

use dioxus::prelude::*;

const BASE_URL: &str = "https://logicaffeine.com";
const LOGO_URL: &str = "https://logicaffeine.com/assets/logo.jpeg";
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
  "sameAs": ["{GITHUB_URL}", "https://x.com/logicaffeine"]
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

/// Generate ItemList schema for Roadmap page
pub fn roadmap_schema() -> String {
    format!(
        r#"{{
  "@context": "https://schema.org",
  "@type": "ItemList",
  "name": "LOGICAFFEINE Development Roadmap",
  "description": "Development milestones from English-to-Logic transpiler to universal compilation",
  "url": "{BASE_URL}/roadmap",
  "numberOfItems": 9,
  "itemListElement": [
    {{"@type": "ListItem", "position": 1, "name": "Core Transpiler", "description": "Parse English, produce First-Order Logic. 53+ linguistic phenomena."}},
    {{"@type": "ListItem", "position": 2, "name": "Web Platform", "description": "Interactive learning, studio playground, gamification."}},
    {{"@type": "ListItem", "position": 3, "name": "Imperative Language", "description": "Functions, structs, enums, pattern matching, standard library, I/O."}},
    {{"@type": "ListItem", "position": 4, "name": "Type System", "description": "Refinement types, generics, type inference, sum types."}},
    {{"@type": "ListItem", "position": 5, "name": "Concurrency & Actors", "description": "Channels, agents, structured parallelism, async/await."}},
    {{"@type": "ListItem", "position": 6, "name": "Distributed Systems", "description": "CRDTs, P2P networking, persistent storage, conflict resolution."}},
    {{"@type": "ListItem", "position": 7, "name": "Security & Policies", "description": "Capability-based security with policy blocks in plain English."}},
    {{"@type": "ListItem", "position": 8, "name": "Proof Assistant", "description": "Curry-Howard in English. Trust statements, termination proofs, Z3 verification."}},
    {{"@type": "ListItem", "position": 9, "name": "Universal Compilation", "description": "Compile to WASM. Live Codex IDE for real-time proof visualization."}}
  ]
}}"#
    )
}

/// Generate WebPage schema for generic pages
pub fn webpage_schema(name: &str, description: &str, path: &str) -> String {
    format!(
        r#"{{
  "@context": "https://schema.org",
  "@type": "WebPage",
  "name": "{name}",
  "description": "{description}",
  "url": "{BASE_URL}{path}",
  "isPartOf": {{
    "@type": "WebSite",
    "name": "{ORG_NAME}",
    "url": "{BASE_URL}"
  }},
  "publisher": {{
    "@type": "Organization",
    "name": "{ORG_NAME}",
    "url": "{BASE_URL}"
  }}
}}"#
    )
}

/// Generate ProfilePage schema for user profile
pub fn profile_page_schema() -> String {
    format!(
        r#"{{
  "@context": "https://schema.org",
  "@type": "ProfilePage",
  "name": "User Profile - {ORG_NAME}",
  "description": "Track your logic learning progress, achievements, and streaks.",
  "url": "{BASE_URL}/profile",
  "isPartOf": {{
    "@type": "WebSite",
    "name": "{ORG_NAME}",
    "url": "{BASE_URL}"
  }}
}}"#
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
            title: "LOGICAFFEINE | Debug Your Thoughts",
            description: "Humanity's last programming language. Transform plain English into compiled Rust code with Z3-powered verification. Debug your thoughts with mathematical certainty.",
            canonical_path: "/",
            og_image: Some("/assets/OG-photo.png"),
        }
    }
}

/// Page-specific metadata definitions
pub mod pages {
    use super::PageMeta;

    pub const LANDING: PageMeta = PageMeta {
        title: "LOGICAFFEINE | Debug Your Thoughts",
        description: "Humanity's last programming language. Transform plain English into compiled Rust code with Z3-powered verification. Debug your thoughts with mathematical certainty.",
        canonical_path: "/",
        og_image: Some("/assets/OG-photo.png"),
    };

    pub const LEARN: PageMeta = PageMeta {
        title: "Learn First-Order Logic | LOGICAFFEINE",
        description: "Master First-Order Logic through interactive exercises. From syllogisms to modal logic, learn to reason precisely.",
        canonical_path: "/learn",
        og_image: Some("/assets/OG-photo.png"),
    };

    pub const STUDIO: PageMeta = PageMeta {
        title: "Studio | LOGICAFFEINE",
        description: "Interactive playground for experimenting with First-Order Logic translations. Try examples and see results in real-time.",
        canonical_path: "/studio",
        og_image: Some("/assets/OG-photo.png"),
    };

    pub const GUIDE: PageMeta = PageMeta {
        title: "Documentation | LOGICAFFEINE",
        description: "Comprehensive guide to LOGICAFFEINE syntax, features, and First-Order Logic concepts.",
        canonical_path: "/guide",
        og_image: Some("/assets/OG-photo.png"),
    };

    pub const PRICING: PageMeta = PageMeta {
        title: "Pricing | LOGICAFFEINE",
        description: "Choose the right plan for your logic needs. Free tier available with premium features for professionals.",
        canonical_path: "/pricing",
        og_image: Some("/assets/OG-photo.png"),
    };

    pub const CRATES: PageMeta = PageMeta {
        title: "Crates Documentation | LOGICAFFEINE",
        description: "Technical documentation for LOGICAFFEINE Rust crates. Integrate First-Order Logic parsing into your applications.",
        canonical_path: "/crates",
        og_image: Some("/assets/OG-photo.png"),
    };

    pub const ROADMAP: PageMeta = PageMeta {
        title: "Roadmap | LOGICAFFEINE",
        description: "See what's coming next for LOGICAFFEINE. Track our progress and upcoming features.",
        canonical_path: "/roadmap",
        og_image: Some("/assets/OG-photo.png"),
    };

    pub const NEWS: PageMeta = PageMeta {
        title: "News | LOGICAFFEINE",
        description: "Latest updates, release notes, and announcements from LOGICAFFEINE.",
        canonical_path: "/news",
        og_image: Some("/assets/OG-photo.png"),
    };

    pub const PRIVACY: PageMeta = PageMeta {
        title: "Privacy Policy | LOGICAFFEINE",
        description: "How LOGICAFFEINE collects, uses, and protects your personal information. Read our full privacy policy.",
        canonical_path: "/privacy",
        og_image: Some("/assets/OG-photo.png"),
    };

    pub const TERMS: PageMeta = PageMeta {
        title: "Terms of Service | LOGICAFFEINE",
        description: "Terms and conditions for using LOGICAFFEINE. Business Source License details and usage policies.",
        canonical_path: "/terms",
        og_image: Some("/assets/OG-photo.png"),
    };

    pub const PROFILE: PageMeta = PageMeta {
        title: "Your Profile | LOGICAFFEINE",
        description: "Track your logic learning progress, achievements, XP, and streaks on LOGICAFFEINE.",
        canonical_path: "/profile",
        og_image: Some("/assets/OG-photo.png"),
    };

    pub const REGISTRY: PageMeta = PageMeta {
        title: "Package Registry | LOGICAFFEINE",
        description: "Browse and discover community-contributed logic modules and packages for LOGICAFFEINE.",
        canonical_path: "/registry",
        og_image: Some("/assets/OG-photo.png"),
    };
}

/// Update document head meta tags for SEO on every page render.
///
/// Uses `document::Title` for the page title and direct web-sys DOM
/// manipulation for description, Open Graph, Twitter Card, and canonical URL.
#[component]
pub fn PageHead(
    title: String,
    description: String,
    canonical_path: String,
    #[props(default = String::from("/assets/OG-photo.png"))]
    og_image: String,
) -> Element {
    let canonical_url = format!("{}{}", BASE_URL, canonical_path);
    let image_url = format!("{}{}", BASE_URL, og_image);

    #[cfg(target_arch = "wasm32")]
    {
        update_head_meta(&title, &description, &canonical_url, &image_url);
    }

    rsx! {
        document::Title { "{title}" }
    }
}

/// Synchronously patch existing `<meta>` and `<link>` tags in `<head>`.
///
/// Only touches elements already present in `index.html`, so the static
/// fallback remains intact for crawlers that don't execute JavaScript.
#[cfg(target_arch = "wasm32")]
fn update_head_meta(title: &str, description: &str, canonical_url: &str, image_url: &str) {
    let Some(window) = web_sys::window() else { return };
    let Some(doc) = window.document() else { return };

    let set = |selector: &str, attr: &str, value: &str| {
        if let Ok(Some(el)) = doc.query_selector(selector) {
            let _ = el.set_attribute(attr, value);
        }
    };

    // Primary
    set("meta[name='description']", "content", description);
    set("meta[name='title']", "content", title);
    // Open Graph
    set("meta[property='og:url']", "content", canonical_url);
    set("meta[property='og:title']", "content", title);
    set("meta[property='og:description']", "content", description);
    set("meta[property='og:image']", "content", image_url);
    // Twitter
    set("meta[name='twitter:url']", "content", canonical_url);
    set("meta[name='twitter:title']", "content", title);
    set("meta[name='twitter:description']", "content", description);
    set("meta[name='twitter:image']", "content", image_url);
    // Canonical
    set("link[rel='canonical']", "href", canonical_url);
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
