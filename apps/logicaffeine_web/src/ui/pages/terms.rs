//! Terms of service page.
//!
//! Renders the terms of service from an embedded HTML file with consistent
//! legal page styling and navigation.
//!
//! # Route
//!
//! Accessed via [`Route::Terms`].

use dioxus::prelude::*;
use crate::ui::components::main_nav::{MainNav, ActivePage};
use crate::ui::components::footer::Footer;
use crate::ui::seo::{JsonLdMultiple, PageHead, organization_schema, webpage_schema, breadcrumb_schema, BreadcrumbItem, pages as seo_pages};

const TERMS_HTML: &str = include_str!("../../../terms.html");

const LEGAL_STYLE: &str = r#"
.legal-container {
    min-height: 100vh;
    display: flex;
    flex-direction: column;
    background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f3460 100%);
}

.legal-content {
    flex: 1;
    max-width: 900px;
    margin: 0 auto;
    padding: 40px 20px 60px;
    width: 100%;
}

.legal-content-inner {
    background: rgba(255, 255, 255, 0.98);
    border-radius: 16px;
    padding: 40px;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.3);
}

.legal-footer {
    border-top: 1px solid rgba(255,255,255,0.06);
    padding: 24px 20px;
    text-align: center;
    color: rgba(229,231,235,0.56);
    font-size: 13px;
}

.legal-footer a {
    color: rgba(229,231,235,0.72);
    text-decoration: none;
    margin: 0 8px;
}

.legal-footer a:hover {
    color: #a78bfa;
}

.github-link {
    display: inline-flex;
    align-items: center;
    gap: 6px;
}
"#;

#[component]
pub fn Terms() -> Element {
    let breadcrumbs = vec![
        BreadcrumbItem { name: "Home", path: "/" },
        BreadcrumbItem { name: "Terms of Service", path: "/terms" },
    ];
    let schemas = vec![
        organization_schema(),
        webpage_schema("Terms of Service", "Terms and conditions for using LOGICAFFEINE. Business Source License details and usage policies.", "/terms"),
        breadcrumb_schema(&breadcrumbs),
    ];

    rsx! {
        PageHead {
            title: seo_pages::TERMS.title,
            description: seo_pages::TERMS.description,
            canonical_path: seo_pages::TERMS.canonical_path,
        }
        style { "{LEGAL_STYLE}" }
        JsonLdMultiple { schemas }

        div { class: "legal-container",
            MainNav { active: ActivePage::Other, subtitle: Some("Terms of Use") }

            main { class: "legal-content",
                div {
                    class: "legal-content-inner",
                    dangerous_inner_html: "{TERMS_HTML}"
                }
            }

            Footer {}
        }
    }
}
