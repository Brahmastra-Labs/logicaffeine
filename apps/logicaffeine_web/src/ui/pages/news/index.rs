//! News index page - displays list of all articles.

use dioxus::prelude::*;
use crate::ui::components::main_nav::{MainNav, ActivePage};
use crate::ui::components::footer::Footer;
use crate::ui::seo::{JsonLdMultiple, PageHead, organization_schema, breadcrumb_schema, BreadcrumbItem, pages as seo_pages};
use crate::ui::router::Route;
use super::data::{get_articles, get_all_tags, get_articles_by_tag, format_tag};

const NEWS_STYLES: &str = r#"
.news-page {
    min-height: 100vh;
    background: var(--bg-dark, #060814);
    display: flex;
    flex-direction: column;
}

.news-layout {
    flex: 1;
    display: flex;
    max-width: 1200px;
    margin: 0 auto;
    padding: 48px 24px;
    width: 100%;
    gap: 48px;
}

/* Sidebar - Desktop only */
.news-sidebar {
    width: 240px;
    flex-shrink: 0;
    position: sticky;
    top: 100px;
    height: fit-content;
}

.news-sidebar-section {
    margin-bottom: 32px;
}

.news-sidebar-title {
    font-size: var(--font-caption-md, 12px);
    font-weight: 600;
    color: var(--text-tertiary, #909090);
    text-transform: uppercase;
    letter-spacing: 1px;
    margin: 0 0 16px;
    padding-bottom: 8px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
}

.news-search {
    position: relative;
    margin-bottom: 24px;
}

.news-search-input {
    width: 100%;
    padding: 10px 14px 10px 38px;
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: var(--radius-lg, 12px);
    color: var(--text-primary, #f0f0f0);
    font-size: var(--font-body-sm, 14px);
    outline: none;
    transition: all 0.2s ease;
}

.news-search-input::placeholder {
    color: var(--text-tertiary, #909090);
}

.news-search-input:focus {
    border-color: rgba(102, 126, 234, 0.5);
    background: rgba(255, 255, 255, 0.08);
}

.news-search-icon {
    position: absolute;
    left: 12px;
    top: 50%;
    transform: translateY(-50%);
    color: var(--text-tertiary, #909090);
    pointer-events: none;
}

.news-tag-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
}

.news-filter-tag {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px;
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--radius-md, 8px);
    color: var(--text-secondary, #b0b0b0);
    font-size: var(--font-body-sm, 14px);
    cursor: pointer;
    transition: all 0.2s ease;
    text-align: left;
    width: 100%;
}

.news-filter-tag:hover {
    background: rgba(255, 255, 255, 0.05);
    color: var(--text-primary, #f0f0f0);
}

.news-filter-tag.active {
    background: rgba(102, 126, 234, 0.15);
    border-color: rgba(102, 126, 234, 0.3);
    color: var(--color-accent-blue, #60a5fa);
}

.news-filter-tag-count {
    font-size: 11px;
    padding: 2px 8px;
    background: rgba(255, 255, 255, 0.1);
    border-radius: var(--radius-full, 9999px);
    color: var(--text-tertiary, #909090);
}

.news-filter-tag.active .news-filter-tag-count {
    background: rgba(102, 126, 234, 0.3);
    color: var(--color-accent-blue, #60a5fa);
}

.news-clear-filter {
    margin-top: 12px;
    padding: 8px 12px;
    background: transparent;
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: var(--radius-md, 8px);
    color: var(--text-secondary, #b0b0b0);
    font-size: var(--font-body-sm, 14px);
    cursor: pointer;
    transition: all 0.2s ease;
    width: 100%;
}

.news-clear-filter:hover {
    background: rgba(255, 255, 255, 0.05);
    border-color: rgba(255, 255, 255, 0.2);
    color: var(--text-primary, #f0f0f0);
}

/* Main content area */
.news-main {
    flex: 1;
    min-width: 0;
}

.news-header {
    margin-bottom: 48px;
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

.news-active-filter {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 16px;
    padding: 12px 16px;
    background: rgba(102, 126, 234, 0.1);
    border: 1px solid rgba(102, 126, 234, 0.2);
    border-radius: var(--radius-lg, 12px);
}

.news-active-filter-label {
    color: var(--text-secondary, #b0b0b0);
    font-size: var(--font-body-sm, 14px);
}

.news-active-filter-tag {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 4px 12px;
    background: rgba(102, 126, 234, 0.2);
    border-radius: var(--radius-full, 9999px);
    color: var(--color-accent-blue, #60a5fa);
    font-size: var(--font-body-sm, 14px);
    font-weight: 600;
    letter-spacing: 0.3px;
}

.news-active-filter-clear {
    background: transparent;
    border: none;
    color: var(--text-tertiary, #909090);
    cursor: pointer;
    padding: 2px;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: color 0.2s ease;
}

.news-active-filter-clear:hover {
    color: var(--text-primary, #f0f0f0);
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
    letter-spacing: 0.3px;
    font-weight: 600;
}

.news-no-results {
    text-align: center;
    padding: 48px 24px;
    color: var(--text-secondary, #b0b0b0);
}

.news-no-results h3 {
    font-size: var(--font-heading-md, 22px);
    color: var(--text-primary, #f0f0f0);
    margin: 0 0 12px;
}

.news-no-results p {
    margin: 0;
}

/* Mobile filter (bottom) */
.news-mobile-filters {
    display: none;
    padding: 16px;
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: var(--radius-xl, 16px);
    margin-bottom: 24px;
}

.news-mobile-filters-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 12px;
}

.news-mobile-filters-title {
    font-size: var(--font-body-sm, 14px);
    font-weight: 600;
    color: var(--text-secondary, #b0b0b0);
}

.news-mobile-tags {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
}

.news-mobile-tag {
    padding: 6px 12px;
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: var(--radius-full, 9999px);
    color: var(--text-secondary, #b0b0b0);
    font-size: 12px;
    cursor: pointer;
    transition: all 0.2s ease;
    letter-spacing: 0.3px;
    font-weight: 600;
}

.news-mobile-tag:hover {
    background: rgba(255, 255, 255, 0.1);
}

.news-mobile-tag.active {
    background: rgba(102, 126, 234, 0.2);
    border-color: rgba(102, 126, 234, 0.4);
    color: var(--color-accent-blue, #60a5fa);
}

/* Mobile */
@media (max-width: 900px) {
    .news-sidebar {
        display: none;
    }

    .news-mobile-filters {
        display: block;
    }

    .news-layout {
        padding: 32px 16px;
        flex-direction: column;
        gap: 0;
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
    // Read tag from URL query parameter
    let initial_tag = {
        #[cfg(target_arch = "wasm32")]
        {
            web_sys::window()
                .and_then(|w| w.location().search().ok())
                .and_then(|s| {
                    s.strip_prefix("?tag=")
                        .or_else(|| s.strip_prefix("?").and_then(|q| q.split('&').find_map(|p| p.strip_prefix("tag="))))
                        .map(|t| t.to_string())
                })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            None::<String>
        }
    };

    let mut active_tag = use_signal(move || initial_tag.clone());
    let mut search_query = use_signal(|| String::new());

    let all_tags = get_all_tags();
    let all_articles = get_articles();

    // Count articles per tag
    let tag_counts: std::collections::HashMap<&str, usize> = all_tags
        .iter()
        .map(|tag| (*tag, get_articles_by_tag(tag).len()))
        .collect();

    // Filter articles based on active tag and search query
    let filtered_articles: Vec<_> = {
        let query = search_query.read().to_lowercase();
        let tag_filter = active_tag.read().clone();

        all_articles
            .into_iter()
            .filter(|article| {
                // Tag filter
                let tag_match = match &tag_filter {
                    Some(tag) => article.tags.contains(&tag.as_str()),
                    None => true,
                };

                // Search filter
                let search_match = if query.is_empty() {
                    true
                } else {
                    article.title.to_lowercase().contains(&query)
                        || article.summary.to_lowercase().contains(&query)
                        || article.tags.iter().any(|t| t.to_lowercase().contains(&query))
                };

                tag_match && search_match
            })
            .collect()
    };

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

            div { class: "news-layout",
                // Desktop sidebar
                aside { class: "news-sidebar",
                    // Search
                    div { class: "news-search",
                        span { class: "news-search-icon", "🔍" }
                        input {
                            class: "news-search-input",
                            r#type: "text",
                            placeholder: "Search articles...",
                            value: "{search_query}",
                            oninput: move |e| search_query.set(e.value())
                        }
                    }

                    // Tags
                    div { class: "news-sidebar-section",
                        h3 { class: "news-sidebar-title", "Filter by Topic" }
                        div { class: "news-tag-list",
                            for tag in all_tags.iter() {
                                {
                                    let tag_str = *tag;
                                    let tag_display = format_tag(tag_str);
                                    let is_active = active_tag.read().as_deref() == Some(tag_str);
                                    let count = tag_counts.get(tag_str).copied().unwrap_or(0);
                                    rsx! {
                                        button {
                                            class: if is_active { "news-filter-tag active" } else { "news-filter-tag" },
                                            onclick: move |_| {
                                                if active_tag.read().as_deref() == Some(tag_str) {
                                                    active_tag.set(None);
                                                } else {
                                                    active_tag.set(Some(tag_str.to_string()));
                                                }
                                            },
                                            span { "{tag_display}" }
                                            span { class: "news-filter-tag-count", "{count}" }
                                        }
                                    }
                                }
                            }
                        }

                        if active_tag.read().is_some() {
                            button {
                                class: "news-clear-filter",
                                onclick: move |_| active_tag.set(None),
                                "Clear filter"
                            }
                        }
                    }
                }

                // Main content
                main { class: "news-main",
                    header { class: "news-header",
                        h1 { "News" }
                        p { "Latest updates, release notes, and announcements" }

                        // Active filter indicator
                        if let Some(tag) = active_tag.read().as_ref() {
                            {
                                let tag_display = format_tag(tag);
                                rsx! {
                                    div { class: "news-active-filter",
                                        span { class: "news-active-filter-label", "Showing:" }
                                        span { class: "news-active-filter-tag",
                                            "{tag_display}"
                                            button {
                                                class: "news-active-filter-clear",
                                                onclick: move |_| active_tag.set(None),
                                                "✕"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Mobile filters
                    div { class: "news-mobile-filters",
                        div { class: "news-mobile-filters-header",
                            span { class: "news-mobile-filters-title", "Filter by topic" }
                            if active_tag.read().is_some() {
                                button {
                                    class: "news-clear-filter",
                                    onclick: move |_| active_tag.set(None),
                                    "Clear"
                                }
                            }
                        }
                        div { class: "news-mobile-tags",
                            for tag in all_tags.iter() {
                                {
                                    let tag_str = *tag;
                                    let tag_display = format_tag(tag_str);
                                    let is_active = active_tag.read().as_deref() == Some(tag_str);
                                    rsx! {
                                        button {
                                            class: if is_active { "news-mobile-tag active" } else { "news-mobile-tag" },
                                            onclick: move |_| {
                                                if active_tag.read().as_deref() == Some(tag_str) {
                                                    active_tag.set(None);
                                                } else {
                                                    active_tag.set(Some(tag_str.to_string()));
                                                }
                                            },
                                            "{tag_display}"
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if filtered_articles.is_empty() {
                        div { class: "news-no-results",
                            h3 { "No articles found" }
                            p { "Try adjusting your search or filter." }
                        }
                    } else {
                        div { class: "news-list",
                            for article in filtered_articles {
                                Link {
                                    to: Route::NewsArticle { slug: article.slug.to_string() },
                                    class: "news-card",
                                    div { class: "news-card-date", "{article.date}" }
                                    h2 { class: "news-card-title", "{article.title}" }
                                    p { class: "news-card-summary", "{article.summary}" }
                                    div { class: "news-card-tags",
                                        for tag in article.tags.iter() {
                                            {
                                                let tag_display = format_tag(tag);
                                                rsx! { span { class: "news-tag", "{tag_display}" } }
                                            }
                                        }
                                    }
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
