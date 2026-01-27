//! News index page - displays list of all articles.

use dioxus::prelude::*;
use crate::ui::components::main_nav::{MainNav, ActivePage};
use crate::ui::components::footer::Footer;
use crate::ui::seo::{JsonLdMultiple, PageHead, organization_schema, breadcrumb_schema, BreadcrumbItem, pages as seo_pages};
use crate::ui::router::Route;
use super::data::get_articles;

const NEWS_STYLES: &str = r#"
.news-page {
    min-height: 100vh;
    background: var(--bg-dark, #060814);
    display: flex;
    flex-direction: column;
}

.news-content {
    flex: 1;
    max-width: 800px;
    margin: 0 auto;
    padding: 48px 24px;
    width: 100%;
}

.news-header {
    margin-bottom: 48px;
    text-align: center;
}

.news-header h1 {
    font-size: var(--font-display-lg, 42px);
    font-weight: 800;
    background: linear-gradient(180deg, #ffffff 0%, rgba(229,231,235,0.85) 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
    margin: 0 0 16px;
}

.news-header p {
    color: var(--text-secondary, #b0b0b0);
    font-size: var(--font-body-lg, 18px);
    margin: 0;
}

.news-list {
    display: flex;
    flex-direction: column;
    gap: 24px;
}

.news-card {
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: var(--radius-xl, 16px);
    padding: 24px;
    transition: all 0.2s ease;
    text-decoration: none;
    display: block;
}

.news-card:hover {
    background: rgba(255, 255, 255, 0.06);
    border-color: rgba(255, 255, 255, 0.12);
    transform: translateY(-2px);
}

.news-card-date {
    font-size: var(--font-caption-md, 12px);
    color: var(--text-tertiary, #909090);
    margin-bottom: 8px;
    text-transform: uppercase;
    letter-spacing: 0.5px;
}

.news-card-title {
    font-size: var(--font-heading-md, 22px);
    font-weight: 700;
    color: var(--text-primary, #f0f0f0);
    margin: 0 0 12px;
    line-height: 1.3;
}

.news-card-summary {
    font-size: var(--font-body-md, 16px);
    color: var(--text-secondary, #b0b0b0);
    line-height: 1.6;
    margin: 0 0 16px;
}

.news-card-tags {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
}

.news-tag {
    font-size: 11px;
    padding: 4px 10px;
    border-radius: var(--radius-full, 9999px);
    background: rgba(102, 126, 234, 0.15);
    color: var(--color-accent-blue, #60a5fa);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    font-weight: 600;
}

/* Mobile */
@media (max-width: 768px) {
    .news-content {
        padding: 32px 16px;
    }

    .news-header h1 {
        font-size: var(--font-display-md, 32px);
    }

    .news-card {
        padding: 20px;
    }

    .news-card-title {
        font-size: var(--font-heading-sm, 18px);
    }
}
"#;

#[component]
pub fn News() -> Element {
    let articles = get_articles();

    let breadcrumbs = vec![
        BreadcrumbItem { name: "Home", path: "/" },
        BreadcrumbItem { name: "News", path: "/news" },
    ];

    let schemas = vec![
        organization_schema(),
        breadcrumb_schema(&breadcrumbs),
    ];

    rsx! {
        PageHead {
            title: seo_pages::NEWS.title,
            description: seo_pages::NEWS.description,
            canonical_path: seo_pages::NEWS.canonical_path,
        }
        style { "{NEWS_STYLES}" }
        JsonLdMultiple { schemas }

        div { class: "news-page",
            MainNav { active: ActivePage::News, subtitle: Some("Latest updates") }

            main { class: "news-content",
                header { class: "news-header",
                    h1 { "News" }
                    p { "Latest updates, release notes, and announcements" }
                }

                div { class: "news-list",
                    for article in articles {
                        Link {
                            to: Route::NewsArticle { slug: article.slug.to_string() },
                            class: "news-card",
                            div { class: "news-card-date", "{article.date}" }
                            h2 { class: "news-card-title", "{article.title}" }
                            p { class: "news-card-summary", "{article.summary}" }
                            div { class: "news-card-tags",
                                for tag in article.tags.iter() {
                                    span { class: "news-tag", "{tag}" }
                                }
                            }
                        }
                    }
                }
            }

            Footer {}
        }
    }
}
