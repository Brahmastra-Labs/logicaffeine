//! SEO utilities and JSON-LD structured data generators.
//!
//! Provides structured data schemas for rich results in search engines:
//! - Organization schema (sitewide), and with a sales contact point (Pricing page)
//! - WebSite schema with search action
//! - SoftwareApplication schema (Studio page)
//! - Course schema (Learn page)
//! - Article schema (News articles)
//! - FAQPage schema (Landing and Pricing pages)
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

fn organization() -> serde_json::Value {
    serde_json::json!({
        "@context": "https://schema.org",
        "@type": "Organization",
        "name": ORG_NAME,
        "url": BASE_URL,
        "logo": LOGO_URL,
        "description": "Turn everyday English into rigorous First-Order Logic. Debug your thoughts with precision.",
        "sameAs": [GITHUB_URL, "https://x.com/logicaffeine"],
    })
}

/// Generate Organization schema (used on all pages)
pub fn organization_schema() -> String {
    organization().to_string()
}

/// Generate Organization schema that names where to ask for a price: the sales `ContactPoint`
/// reached at `email` and at the page `contact_path`.
pub fn organization_contact_schema(email: &str, contact_path: &str) -> String {
    let mut org = organization();
    org["contactPoint"] = serde_json::json!({
        "@type": "ContactPoint",
        "contactType": "sales",
        "email": email,
        "url": format!("{BASE_URL}{contact_path}"),
        "availableLanguage": "English",
    });
    org.to_string()
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

/// Generate FAQPage schema from (question, answer) pairs. Built as JSON, so a quote in an
/// answer is escaped rather than ending the string.
pub fn faq_schema(questions: &[(&str, &str)]) -> String {
    let items: Vec<serde_json::Value> = questions
        .iter()
        .map(|(question, answer)| {
            serde_json::json!({
                "@type": "Question",
                "name": question,
                "acceptedAnswer": { "@type": "Answer", "text": answer },
            })
        })
        .collect();
    serde_json::json!({
        "@context": "https://schema.org",
        "@type": "FAQPage",
        "mainEntity": items,
    })
    .to_string()
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

/// Generate ItemList schema for Roadmap page: one `ListItem` per timeline card, in page order,
/// each described by its status and the first sentence of its card, so search engines read the
/// roadmap the page shows.
pub fn roadmap_schema() -> String {
    let milestones = crate::ui::pages::roadmap_data::get_milestones();
    let items: Vec<serde_json::Value> = milestones
        .iter()
        .enumerate()
        .map(|(i, m)| {
            serde_json::json!({
                "@type": "ListItem",
                "position": i + 1,
                "name": m.title,
                "description": format!("{} — {}", m.status.label(), first_sentence(m.description)),
            })
        })
        .collect();
    serde_json::json!({
        "@context": "https://schema.org",
        "@type": "ItemList",
        "name": "LOGICAFFEINE Development Roadmap",
        "description": "Where LOGOS stands: finished milestones, work in progress, and what is planned next.",
        "url": format!("{BASE_URL}/roadmap"),
        "numberOfItems": milestones.len(),
        "itemListElement": items,
    })
    .to_string()
}

/// A description's first sentence: up to and including the first period that ends a word.
fn first_sentence(text: &str) -> &str {
    match text.find(". ") {
        Some(end) => &text[..=end],
        None => text,
    }
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
        title: "LOGICAFFEINE | Code we can all read",
        description: "LOGOS is an English programming language with logic and mathematics built in. Bring your own coding agent: give it the instructions, then read every line it writes.",
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
        description:
            "Comprehensive guide to LOGICAFFEINE syntax, features, and First-Order Logic concepts.",
        canonical_path: "/guide",
        og_image: Some("/assets/OG-photo.png"),
    };

    pub const CONTACT: PageMeta = PageMeta {
        title: "Contact | LOGICAFFEINE",
        description: "Reach the team behind LOGOS, the open source English programming language under the MIT License: questions, bugs, working together.",
        canonical_path: "/contact",
        og_image: Some("/assets/OG-photo.png"),
    };

    pub const CRATES: PageMeta = PageMeta {
        title: "Crates Documentation | LOGICAFFEINE",
        description: "The Rust crates behind LOGOS: the English-to-logic front end, the compiler and its execution tiers, the proof kernel, and the tools.",
        canonical_path: "/crates",
        og_image: Some("/assets/OG-photo.png"),
    };

    pub const ROADMAP: PageMeta = PageMeta {
        title: "Roadmap | LOGICAFFEINE",
        description:
            "See what's coming next for LOGICAFFEINE. Track our progress and upcoming features.",
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
        description: "Terms and conditions for using LOGICAFFEINE and its MIT-licensed software.",
        canonical_path: "/terms",
        og_image: Some("/assets/OG-photo.png"),
    };

    pub const REGISTRY: PageMeta = PageMeta {
        title: "Package Registry | LOGICAFFEINE",
        description:
            "Browse and discover community-contributed logic modules and packages for LOGICAFFEINE.",
        canonical_path: "/registry",
        og_image: Some("/assets/OG-photo.png"),
    };

    pub const BENCHMARKS: PageMeta = PageMeta {
        title: "Benchmarks | LOGICAFFEINE",
        description: "LOGOS performance benchmarks across 13 languages. Near-native speed from English source code.",
        canonical_path: "/benchmarks",
        og_image: Some("/assets/OG-photo.png"),
    };
}

/// Per-page head tags: title, description, Open Graph, Twitter Card, canonical.
///
/// Server renders (the SSG prerender) emit real `<meta>`/`<link>` elements via
/// `document::*`, so every prerendered page carries its own share-card and
/// canonical tags for crawlers and link unfurlers. The wasm client instead
/// creates-or-patches the live head (the tags may come from a prerendered page
/// OR be absent on an SPA-fallback route), keeping the DOM tag-set identical
/// without ever duplicating an element.
#[component]
pub fn PageHead(
    title: String,
    description: String,
    canonical_path: String,
    #[props(default = String::from("/assets/OG-photo.png"))] og_image: String,
) -> Element {
    let canonical_url = format!("{}{}", BASE_URL, canonical_path);
    let image_url = format!("{}{}", BASE_URL, og_image);
    // The default card is the resized 1200x630 OG photo; a page supplying its own
    // image also owns that image's dimensions, so the size tags are skipped.
    let default_card = og_image == "/assets/OG-photo.png";

    #[cfg(target_arch = "wasm32")]
    {
        update_head_meta(&title, &description, &canonical_url, &image_url);
    }

    rsx! {
        document::Title { "{title}" }
        if cfg!(not(target_arch = "wasm32")) {
            document::Meta { name: "title", content: title.clone() }
            document::Meta { name: "description", content: description.clone() }
            document::Meta { property: "og:type", content: "website" }
            document::Meta { property: "og:url", content: canonical_url.clone() }
            document::Meta { property: "og:title", content: title.clone() }
            document::Meta { property: "og:description", content: description.clone() }
            document::Meta { property: "og:image", content: image_url.clone() }
            if default_card {
                document::Meta { property: "og:image:width", content: "1200" }
                document::Meta { property: "og:image:height", content: "630" }
                document::Meta { property: "og:image:alt", content: "LOGICAFFEINE | Debug Your Thoughts" }
            }
            document::Meta { name: "twitter:url", content: canonical_url.clone() }
            document::Meta { name: "twitter:title", content: title.clone() }
            document::Meta { name: "twitter:description", content: description.clone() }
            document::Meta { name: "twitter:image", content: image_url.clone() }
            document::Link { rel: "canonical", href: canonical_url.clone() }
        }
    }
}

/// Create-or-patch `<meta>` and `<link>` tags in the live `<head>`.
///
/// Prerendered pages already carry these elements (patched in place); a route
/// served by the SPA fallback shell has none, so missing elements are created.
#[cfg(target_arch = "wasm32")]
fn update_head_meta(title: &str, description: &str, canonical_url: &str, image_url: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(doc) = window.document() else { return };
    let Some(head) = doc.head() else { return };

    let set = |selector: &str, key_attr: &str, key: &str, attr: &str, value: &str| {
        if let Ok(Some(el)) = doc.query_selector(selector) {
            let _ = el.set_attribute(attr, value);
        } else if let Ok(el) = doc.create_element(if key_attr == "rel" { "link" } else { "meta" }) {
            let _ = el.set_attribute(key_attr, key);
            let _ = el.set_attribute(attr, value);
            let _ = head.append_child(&el);
        }
    };
    let set_name = |name: &str, value: &str| {
        set(
            &format!("meta[name='{name}']"),
            "name",
            name,
            "content",
            value,
        );
    };
    let set_property = |property: &str, value: &str| {
        set(
            &format!("meta[property='{property}']"),
            "property",
            property,
            "content",
            value,
        );
    };

    // Primary
    set_name("description", description);
    set_name("title", title);
    // Open Graph
    set_property("og:type", "website");
    set_property("og:url", canonical_url);
    set_property("og:title", title);
    set_property("og:description", description);
    set_property("og:image", image_url);
    // Twitter
    set_name("twitter:url", canonical_url);
    set_name("twitter:title", title);
    set_name("twitter:description", description);
    set_name("twitter:image", image_url);
    // Canonical
    set(
        "link[rel='canonical']",
        "rel",
        "canonical",
        "href",
        canonical_url,
    );
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
