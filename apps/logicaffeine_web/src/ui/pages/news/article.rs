//! Individual news article page.

use super::data::{format_tag, get_all_tags, get_article_by_slug};
use crate::ui::components::footer::Footer;
use crate::ui::components::main_nav::{ActivePage, MainNav};
use crate::ui::inline_markup::inline_markup_html;
use crate::ui::markdown_blocks::{blocks, Block};
use crate::ui::router::Route;
use crate::ui::seo::{
    article_schema, breadcrumb_schema, organization_schema, BreadcrumbItem, JsonLdMultiple,
    PageHead,
};
use dioxus::prelude::*;
#[cfg(all(feature = "split", target_arch = "wasm32"))]
use dioxus::wasm_split;

const ARTICLE_STYLES: &str = r#"
.article-page {
    min-height: 100vh;
    background: var(--bg-dark, #060814);
    display: flex;
    flex-direction: column;
}

.article-content {
    flex: 1;
    max-width: 720px;
    margin: 0 auto;
    padding: 48px 24px;
    width: 100%;
}

.article-back {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    color: var(--text-secondary, #b0b0b0);
    text-decoration: none;
    font-size: var(--font-body-sm, 14px);
    margin-bottom: 32px;
    transition: color 0.2s ease;
}

.article-back:hover {
    color: var(--text-primary, #f0f0f0);
}

.article-header {
    margin-bottom: 40px;
}

.article-meta {
    display: flex;
    align-items: center;
    gap: 16px;
    margin-bottom: 16px;
    flex-wrap: wrap;
}

.article-date {
    font-size: var(--font-caption-md, 12px);
    color: var(--text-tertiary, #909090);
    text-transform: uppercase;
    letter-spacing: 0.5px;
}

.article-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
}

.article-tag {
    white-space: nowrap;
    font-size: 11px;
    padding: 4px 10px;
    border-radius: var(--radius-full, 9999px);
    background: rgba(102, 126, 234, 0.15);
    color: var(--color-accent-blue, #60a5fa);
    letter-spacing: 0.3px;
    font-weight: 600;
}

.article-title {
    font-size: var(--font-display-md, 36px);
    font-weight: 800;
    background: linear-gradient(180deg, #ffffff 0%, rgba(229,231,235,0.85) 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
    margin: 0 0 16px;
    line-height: 1.2;
}

.article-author {
    font-size: var(--font-body-md, 16px);
    color: var(--text-secondary, #b0b0b0);
}

.article-body {
    color: var(--text-primary, #f0f0f0);
    font-size: var(--font-body-lg, 18px);
    line-height: 1.8;
}

.article-body h2 {
    font-size: var(--font-heading-lg, 26px);
    font-weight: 700;
    margin: 48px 0 24px;
    color: var(--text-primary, #f0f0f0);
}

.article-body h3 {
    font-size: var(--font-heading-md, 22px);
    font-weight: 600;
    margin: 32px 0 16px;
    color: var(--text-primary, #f0f0f0);
}

.article-body p {
    margin: 0 0 24px;
}

.article-body ul, .article-body ol {
    margin: 0 0 24px;
    padding-left: 24px;
}

.article-body li {
    margin-bottom: 8px;
}

.article-body blockquote {
    margin: 24px 0;
    padding: 16px 24px;
    background: rgba(255, 255, 255, 0.03);
    border-left: 3px solid var(--color-accent-blue, #60a5fa);
    border-radius: 0 var(--radius-md, 8px) var(--radius-md, 8px) 0;
    font-style: italic;
    color: var(--text-secondary, #b0b0b0);
}

.article-body code {
    font-family: var(--font-mono, 'SF Mono', monospace);
    font-size: 0.9em;
    padding: 2px 6px;
    background: rgba(255, 255, 255, 0.08);
    border-radius: 4px;
    color: var(--color-accent-purple, #a78bfa);
}

.article-body pre {
    margin: 24px 0;
    padding: 20px;
    background: rgba(0, 0, 0, 0.3);
    border-radius: var(--radius-lg, 12px);
    overflow-x: auto;
}

.article-body pre code {
    padding: 0;
    background: none;
    font-size: var(--font-body-sm, 14px);
    color: var(--text-primary, #f0f0f0);
}

.article-body a {
    color: var(--color-accent-blue, #60a5fa);
    text-decoration: none;
    border-bottom: 1px solid transparent;
    transition: border-color 0.2s ease;
}

.article-body a:hover {
    border-color: var(--color-accent-blue, #60a5fa);
}

.article-body strong {
    font-weight: 600;
    color: var(--text-primary, #f0f0f0);
}

.article-body hr {
    border: 0;
    border-top: 1px solid rgba(255, 255, 255, 0.08);
    margin: 40px 0;
}

.article-body .article-table {
    margin: 0 0 24px;
    overflow-x: auto;
}

.article-body table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--font-body-md, 16px);
    line-height: 1.5;
}

.article-body th,
.article-body td {
    padding: 10px 14px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    text-align: left;
    vertical-align: top;
}

.article-body th {
    font-weight: 600;
    color: var(--text-primary, #f0f0f0);
    border-bottom-color: rgba(255, 255, 255, 0.16);
}

.article-not-found {
    text-align: center;
    padding: 80px 24px;
}

.article-not-found h1 {
    font-size: var(--font-display-md, 36px);
    margin-bottom: 16px;
    color: var(--text-primary, #f0f0f0);
}

.article-not-found p {
    color: var(--text-secondary, #b0b0b0);
    margin-bottom: 32px;
}

.article-not-found a {
    display: inline-block;
    padding: 12px 24px;
    background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
    color: white;
    text-decoration: none;
    border-radius: var(--radius-lg, 12px);
    font-weight: 600;
}

/* Article footer with tags */
.article-footer {
    margin-top: 64px;
    padding-top: 32px;
    border-top: 1px solid rgba(255, 255, 255, 0.08);
}

.article-footer-section {
    margin-bottom: 24px;
}

.article-footer-title {
    font-size: var(--font-caption-md, 12px);
    font-weight: 600;
    color: var(--text-tertiary, #909090);
    text-transform: uppercase;
    letter-spacing: 1px;
    margin: 0 0 16px;
}

.article-browse-tags {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
}

.article-browse-tag {
    padding: 8px 16px;
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: var(--radius-full, 9999px);
    color: var(--text-secondary, #b0b0b0);
    font-size: var(--font-body-sm, 14px);
    text-decoration: none;
    transition: all 0.2s ease;
    letter-spacing: 0.3px;
    font-weight: 600;
}

.article-browse-tag:hover {
    background: rgba(102, 126, 234, 0.15);
    border-color: rgba(102, 126, 234, 0.3);
    color: var(--color-accent-blue, #60a5fa);
}

.article-browse-tag.current {
    background: rgba(102, 126, 234, 0.2);
    border-color: rgba(102, 126, 234, 0.4);
    color: var(--color-accent-blue, #60a5fa);
}

/* Mobile */
@media (max-width: 768px) {
    .article-content {
        padding: 32px 16px;
    }

    .article-title {
        font-size: var(--font-heading-lg, 28px);
    }

    .article-body {
        font-size: var(--font-body-md, 16px);
    }

    .article-body h2 {
        font-size: var(--font-heading-md, 22px);
    }

    .article-body h3 {
        font-size: var(--font-heading-sm, 18px);
    }

    .article-footer {
        margin-top: 48px;
    }
}
"#;

#[component(lazy)]
pub fn NewsArticle(slug: String) -> Element {
    let article = get_article_by_slug(&slug);
    let (page_title, page_desc) = match article {
        Some(a) => (format!("{} - LOGICAFFEINE", a.title), a.summary.to_string()),
        None => (
            "Article Not Found - LOGICAFFEINE".to_string(),
            "Latest news and updates from LOGICAFFEINE.".to_string(),
        ),
    };
    let page_path = format!("/news/{}", slug);

    rsx! {
        PageHead {
            title: page_title,
            description: page_desc,
            canonical_path: page_path,
        }
        style { "{ARTICLE_STYLES}" }

        div { class: "article-page",
            MainNav { active: ActivePage::News, subtitle: Some("Latest updates") }

            if let Some(article) = article {
                {
                    let breadcrumbs = vec![
                        BreadcrumbItem { name: "Home", path: "/" },
                        BreadcrumbItem { name: "News", path: "/news" },
                    ];

                    let schemas = vec![
                        organization_schema(),
                        breadcrumb_schema(&breadcrumbs),
                        article_schema(article.title, article.summary, article.date, article.slug),
                    ];

                    {
                        let all_tags = get_all_tags();
                        let current_tags: Vec<&str> = article.tags.iter().copied().collect();
                        rsx! {
                            JsonLdMultiple { schemas }

                            main { class: "article-content",
                                Link { to: Route::News { tag: None }, class: "article-back",
                                    "← Back to News"
                                }

                                article {
                                    header { class: "article-header",
                                        div { class: "article-meta",
                                            span { class: "article-date", "{article.date}" }
                                            div { class: "article-tags",
                                                for tag in article.tags.iter() {
                                                    {
                                                        let tag_display = format_tag(tag);
                                                        rsx! { span { class: "article-tag", "{tag_display}" } }
                                                    }
                                                }
                                            }
                                        }
                                        h1 { class: "article-title", "{article.title}" }
                                        p { class: "article-author", "By {article.author}" }
                                    }

                                    div {
                                        class: "article-body",
                                        dangerous_inner_html: markdown_to_html(article.content)
                                    }

                                    // Article footer with browse tags
                                    footer { class: "article-footer",
                                        div { class: "article-footer-section",
                                            h3 { class: "article-footer-title", "Browse by Topic" }
                                            div { class: "article-browse-tags",
                                                for tag in all_tags.iter() {
                                                    {
                                                        let tag_str = *tag;
                                                        let tag_display = format_tag(tag_str);
                                                        let is_current = current_tags.contains(&tag_str);
                                                        rsx! {
                                                            Link {
                                                                to: Route::News { tag: Some(tag_str.to_string()) },
                                                                class: if is_current { "article-browse-tag current" } else { "article-browse-tag" },
                                                                "{tag_display}"
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                main { class: "article-content",
                    div { class: "article-not-found",
                        h1 { "Article Not Found" }
                        p { "The article you're looking for doesn't exist." }
                        Link { to: Route::News { tag: None }, "Back to News" }
                    }
                }
            }

            Footer {}
        }
    }
}

/// An article's Markdown as HTML, block by block; each block's text with inline code, bold and
/// links.
fn markdown_to_html(markdown: &str) -> String {
    blocks(markdown).iter().map(block_html).collect()
}

fn block_html(block: &Block) -> String {
    match block {
        Block::Heading { level, text } => {
            format!("<h{level}>{}</h{level}>\n", inline_markup_html(text))
        }
        Block::Paragraph(text) => format!("<p>{}</p>\n", inline_markup_html(text)),
        Block::List { ordered, items } => {
            let tag = if *ordered { "ol" } else { "ul" };
            let items: String = items
                .iter()
                .map(|item| format!("<li>{}</li>\n", inline_markup_html(item)))
                .collect();
            format!("<{tag}>\n{items}</{tag}>\n")
        }
        Block::Quote(paragraphs) => {
            let body: Vec<String> = paragraphs.iter().map(|p| inline_markup_html(p)).collect();
            format!("<blockquote>{}</blockquote>\n", body.join("<br/><br/>"))
        }
        Block::Table { header, rows } => {
            let row = |cells: &[String], tag: &str| -> String {
                let cells: String = cells
                    .iter()
                    .map(|cell| format!("<{tag}>{}</{tag}>", inline_markup_html(cell)))
                    .collect();
                format!("<tr>{cells}</tr>")
            };
            let head = header.as_ref().map_or(String::new(), |h| {
                format!("<thead>{}</thead>", row(h, "th"))
            });
            let body: String = rows.iter().map(|r| row(r, "td")).collect();
            format!(
                "<div class=\"article-table\"><table>{head}<tbody>{body}</tbody></table></div>\n"
            )
        }
        Block::Fence { code, .. } => format!("<pre><code>{}</code></pre>\n", html_escape(code)),
        Block::Rule => "<hr/>\n".to_string(),
    }
}

/// Escape HTML special characters
fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
#[path = "../../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/news/article/tests.rs"]
mod tests;
