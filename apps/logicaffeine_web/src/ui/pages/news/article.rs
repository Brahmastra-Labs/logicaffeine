//! Individual news article page.

use dioxus::prelude::*;
use crate::ui::components::main_nav::{MainNav, ActivePage};
use crate::ui::components::footer::Footer;
use crate::ui::seo::{JsonLdMultiple, organization_schema, breadcrumb_schema, article_schema, BreadcrumbItem};
use crate::ui::router::Route;
use super::data::get_article_by_slug;

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
    gap: 8px;
}

.article-tag {
    font-size: 11px;
    padding: 4px 10px;
    border-radius: var(--radius-full, 9999px);
    background: rgba(102, 126, 234, 0.15);
    color: var(--color-accent-blue, #60a5fa);
    text-transform: uppercase;
    letter-spacing: 0.5px;
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
}
"#;

#[component]
pub fn NewsArticle(slug: String) -> Element {
    let article = get_article_by_slug(&slug);

    rsx! {
        style { "{ARTICLE_STYLES}" }

        div { class: "article-page",
            MainNav { active: ActivePage::News }

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

                    rsx! {
                        JsonLdMultiple { schemas }

                        main { class: "article-content",
                            Link { to: Route::News {}, class: "article-back",
                                "← Back to News"
                            }

                            article {
                                header { class: "article-header",
                                    div { class: "article-meta",
                                        span { class: "article-date", "{article.date}" }
                                        div { class: "article-tags",
                                            for tag in article.tags.iter() {
                                                span { class: "article-tag", "{tag}" }
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
                            }
                        }
                    }
                }
            } else {
                main { class: "article-content",
                    div { class: "article-not-found",
                        h1 { "Article Not Found" }
                        p { "The article you're looking for doesn't exist." }
                        Link { to: Route::News {}, "Back to News" }
                    }
                }
            }

            Footer {}
        }
    }
}

/// Simple markdown to HTML converter for article content
fn markdown_to_html(markdown: &str) -> String {
    let mut html = String::new();
    let mut in_code_block = false;
    let mut in_list = false;
    let mut list_type = "ul";

    for line in markdown.lines() {
        let trimmed = line.trim();

        // Code blocks
        if trimmed.starts_with("```") {
            if in_code_block {
                html.push_str("</code></pre>\n");
                in_code_block = false;
            } else {
                html.push_str("<pre><code>");
                in_code_block = true;
            }
            continue;
        }

        if in_code_block {
            html.push_str(&html_escape(trimmed));
            html.push('\n');
            continue;
        }

        // Close list if line doesn't start with list marker
        if in_list && !trimmed.starts_with("- ") && !trimmed.starts_with("* ") && !trimmed.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            html.push_str(&format!("</{}>", list_type));
            in_list = false;
        }

        // Headers
        if trimmed.starts_with("### ") {
            html.push_str(&format!("<h3>{}</h3>\n", &trimmed[4..]));
        } else if trimmed.starts_with("## ") {
            html.push_str(&format!("<h2>{}</h2>\n", &trimmed[3..]));
        } else if trimmed.starts_with("# ") {
            html.push_str(&format!("<h1>{}</h1>\n", &trimmed[2..]));
        }
        // Blockquotes
        else if trimmed.starts_with("> ") {
            html.push_str(&format!("<blockquote>{}</blockquote>\n", &trimmed[2..]));
        }
        // Unordered lists
        else if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            if !in_list {
                html.push_str("<ul>\n");
                in_list = true;
                list_type = "ul";
            }
            html.push_str(&format!("<li>{}</li>\n", inline_markdown(&trimmed[2..])));
        }
        // Ordered lists
        else if let Some(rest) = trimmed.strip_prefix(|c: char| c.is_ascii_digit()) {
            if rest.starts_with(". ") {
                if !in_list {
                    html.push_str("<ol>\n");
                    in_list = true;
                    list_type = "ol";
                }
                html.push_str(&format!("<li>{}</li>\n", inline_markdown(&rest[2..])));
            }
        }
        // Paragraphs (non-empty lines)
        else if !trimmed.is_empty() {
            html.push_str(&format!("<p>{}</p>\n", inline_markdown(trimmed)));
        }
    }

    // Close any remaining list
    if in_list {
        html.push_str(&format!("</{}>", list_type));
    }

    html
}

/// Process inline markdown (bold, italic, code, links)
fn inline_markdown(text: &str) -> String {
    let mut result = html_escape(text);

    // Code (backticks) - must be done first
    let mut i = 0;
    while let Some(start) = result[i..].find('`') {
        let start = i + start;
        if let Some(end) = result[start + 1..].find('`') {
            let end = start + 1 + end;
            let code = result[start + 1..end].to_string();
            let code_len = code.len();
            result = format!("{}<code>{}</code>{}", &result[..start], code, &result[end + 1..]);
            i = start + 13 + code_len; // Skip past the inserted code tag
        } else {
            break;
        }
    }

    // Bold (**text**)
    while let Some(start) = result.find("**") {
        if let Some(end) = result[start + 2..].find("**") {
            let end = start + 2 + end;
            let bold = &result[start + 2..end];
            result = format!("{}<strong>{}</strong>{}", &result[..start], bold, &result[end + 2..]);
        } else {
            break;
        }
    }

    // Links [text](url)
    while let Some(start) = result.find('[') {
        if let Some(mid) = result[start..].find("](") {
            let mid = start + mid;
            if let Some(end) = result[mid + 2..].find(')') {
                let end = mid + 2 + end;
                let text = &result[start + 1..mid];
                let url = &result[mid + 2..end];
                result = format!("{}<a href=\"{}\">{}</a>{}", &result[..start], url, text, &result[end + 1..]);
                continue;
            }
        }
        break;
    }

    result
}

/// Escape HTML special characters
fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
