//! Programmer's Guide page.
//!
//! A beautiful, interactive guide to the LOGOS programming language with:
//! - 22 sections from PROGRAMMERS_LANGUAGE_STARTER.md
//! - Sticky sidebar navigation
//! - Interactive code examples with Run/Copy/Reset
//! - Dual mode: Logic (FOL output) and Imperative (WASM execution)

pub mod content;

use dioxus::prelude::*;
use crate::ui::router::Route;
use crate::ui::components::guide_code_block::GuideCodeBlock;
use crate::ui::components::guide_sidebar::{GuideSidebar, SectionInfo};
use content::SECTIONS;

const GUIDE_STYLE: &str = r#"
:root {
    --bg0: #070a12;
    --bg1: #0b1022;
    --card: rgba(255,255,255,0.06);
    --card2: rgba(255,255,255,0.04);
    --border: rgba(255,255,255,0.10);
    --border2: rgba(255,255,255,0.14);
    --text: #e5e7eb;
    --muted: rgba(229,231,235,0.72);
    --muted2: rgba(229,231,235,0.56);
    --brand: #a78bfa;
    --brand2: #60a5fa;
    --ok: #22c55e;
}

* { box-sizing: border-box; }
a { color: inherit; }

.guide-page {
    min-height: 100vh;
    color: var(--text);
    background:
        radial-gradient(1200px 600px at 50% -120px, rgba(167,139,250,0.14), transparent 60%),
        radial-gradient(900px 500px at 15% 30%, rgba(96,165,250,0.14), transparent 60%),
        radial-gradient(800px 450px at 90% 45%, rgba(34,197,94,0.08), transparent 62%),
        linear-gradient(180deg, var(--bg0), var(--bg1) 55%, #070a12);
    font-family: ui-sans-serif, system-ui, -apple-system, 'Segoe UI', Roboto, 'Inter', 'Helvetica Neue', Arial, sans-serif;
}

/* Navigation */
.guide-nav {
    position: sticky;
    top: 0;
    z-index: 50;
    backdrop-filter: blur(18px);
    background: linear-gradient(180deg, rgba(7,10,18,0.85), rgba(7,10,18,0.65));
    border-bottom: 1px solid rgba(255,255,255,0.06);
}

.guide-nav-inner {
    max-width: 1280px;
    margin: 0 auto;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 24px;
    gap: 16px;
}

.guide-brand {
    display: flex;
    align-items: center;
    gap: 12px;
    text-decoration: none;
}

.guide-logo {
    width: 36px;
    height: 36px;
    border-radius: 12px;
    background:
        radial-gradient(circle at 30% 30%, rgba(96,165,250,0.85), transparent 55%),
        radial-gradient(circle at 65% 60%, rgba(167,139,250,0.85), transparent 55%),
        rgba(255,255,255,0.06);
    border: 1px solid rgba(255,255,255,0.10);
    box-shadow: 0 14px 35px rgba(0,0,0,0.35);
}

.guide-brand-text {
    display: flex;
    flex-direction: column;
    line-height: 1.1;
}

.guide-brand-name {
    font-weight: 800;
    font-size: 14px;
    letter-spacing: -0.3px;
}

.guide-brand-subtitle {
    font-size: 12px;
    color: var(--muted2);
}

.guide-nav-links {
    display: flex;
    gap: 8px;
    align-items: center;
}

.guide-nav-link {
    padding: 10px 16px;
    border-radius: 10px;
    font-size: 14px;
    font-weight: 500;
    color: var(--muted);
    text-decoration: none;
    transition: all 0.18s ease;
    border: 1px solid transparent;
}

.guide-nav-link:hover {
    background: rgba(255,255,255,0.05);
    color: var(--text);
}

.guide-nav-link.primary {
    background: linear-gradient(135deg, rgba(96,165,250,0.9), rgba(167,139,250,0.9));
    color: #060814;
    font-weight: 600;
    border-color: rgba(255,255,255,0.1);
}

.guide-nav-link.primary:hover {
    background: linear-gradient(135deg, #60a5fa, #a78bfa);
}

/* Hero */
.guide-hero {
    max-width: 1280px;
    margin: 0 auto;
    padding: 60px 24px 40px;
}

.guide-hero h1 {
    font-size: 48px;
    font-weight: 900;
    letter-spacing: -1.5px;
    line-height: 1.1;
    background: linear-gradient(180deg, #ffffff 0%, rgba(229,231,235,0.78) 65%, rgba(229,231,235,0.62) 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    margin: 0 0 16px;
}

.guide-hero p {
    font-size: 18px;
    color: var(--muted);
    max-width: 600px;
    line-height: 1.6;
    margin: 0;
}

.guide-hero-badge {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    border-radius: 999px;
    background: rgba(255,255,255,0.06);
    border: 1px solid rgba(255,255,255,0.10);
    font-size: 13px;
    font-weight: 600;
    color: rgba(255,255,255,0.85);
    margin-bottom: 20px;
}

.guide-hero-badge .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--ok);
    box-shadow: 0 0 0 4px rgba(34,197,94,0.15);
}

/* Layout */
.guide-layout {
    max-width: 1280px;
    margin: 0 auto;
    display: flex;
    gap: 48px;
    padding: 0 24px 80px;
}

/* Main content */
.guide-content {
    flex: 1;
    min-width: 0;
    max-width: 800px;
}

/* Section styling */
.guide-section {
    margin-bottom: 64px;
    scroll-margin-top: 100px;
}

.guide-section h2 {
    font-size: 32px;
    font-weight: 800;
    letter-spacing: -0.8px;
    line-height: 1.2;
    background: linear-gradient(180deg, #ffffff 0%, rgba(229,231,235,0.85) 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    margin: 0 0 24px;
    padding-bottom: 16px;
    border-bottom: 1px solid rgba(255,255,255,0.08);
}

.guide-section h3 {
    font-size: 20px;
    font-weight: 700;
    color: var(--text);
    margin: 32px 0 16px;
}

.guide-section p {
    color: var(--muted);
    font-size: 15px;
    line-height: 1.75;
    margin: 0 0 16px;
}

.guide-section ul,
.guide-section ol {
    color: var(--muted);
    font-size: 15px;
    line-height: 1.75;
    padding-left: 24px;
    margin: 0 0 16px;
}

.guide-section li {
    margin-bottom: 8px;
}

.guide-section code {
    font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Monaco, monospace;
    background: rgba(255,255,255,0.08);
    padding: 3px 7px;
    border-radius: 5px;
    font-size: 0.9em;
    color: #a78bfa;
}

.guide-section strong {
    color: var(--text);
    font-weight: 600;
}

/* Tables */
.guide-section table {
    width: 100%;
    border-collapse: collapse;
    margin: 20px 0;
    font-size: 14px;
    border-radius: 12px;
    overflow: hidden;
    border: 1px solid rgba(255,255,255,0.08);
}

.guide-section th {
    text-align: left;
    padding: 14px 16px;
    background: rgba(255,255,255,0.05);
    color: var(--text);
    font-weight: 600;
    border-bottom: 1px solid rgba(255,255,255,0.08);
}

.guide-section td {
    padding: 12px 16px;
    color: var(--muted);
    border-bottom: 1px solid rgba(255,255,255,0.05);
}

.guide-section tr:last-child td {
    border-bottom: none;
}

.guide-section tr:hover td {
    background: rgba(255,255,255,0.02);
}

/* Part dividers */
.guide-part-divider {
    margin: 80px 0 48px;
    padding: 24px 0;
    border-top: 1px solid rgba(255,255,255,0.08);
}

.guide-part-divider h2 {
    font-size: 14px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 1.5px;
    color: var(--muted2);
    margin: 0;
    background: none;
    -webkit-text-fill-color: currentColor;
    border-bottom: none;
    padding-bottom: 0;
}

/* Section number */
.section-number {
    display: inline-block;
    font-size: 14px;
    font-weight: 700;
    color: var(--brand);
    margin-right: 8px;
    opacity: 0.8;
}

/* Examples container */
.guide-examples {
    margin-top: 24px;
}

/* Responsive */
@media (max-width: 1024px) {
    .guide-layout {
        flex-direction: column;
    }

    .guide-hero h1 {
        font-size: 36px;
    }

    .guide-hero {
        padding: 40px 24px 32px;
    }
}

@media (max-width: 640px) {
    .guide-nav-inner {
        padding: 12px 16px;
    }

    .guide-nav-links {
        gap: 4px;
    }

    .guide-nav-link {
        padding: 8px 12px;
        font-size: 13px;
    }

    .guide-brand-text {
        display: none;
    }

    .guide-hero h1 {
        font-size: 28px;
    }

    .guide-hero p {
        font-size: 16px;
    }

    .guide-section h2 {
        font-size: 24px;
    }
}
"#;

#[component]
pub fn Guide() -> Element {
    let mut active_section = use_signal(|| "introduction".to_string());

    // Build section info for sidebar
    let sections_info: Vec<SectionInfo> = SECTIONS.iter().map(|s| SectionInfo {
        id: s.id.to_string(),
        number: s.number,
        title: s.title.to_string(),
        part: s.part.to_string(),
    }).collect();

    // Track current part for dividers
    let mut current_part = String::new();

    rsx! {
        style { "{GUIDE_STYLE}" }

        div { class: "guide-page",
            // Navigation
            nav { class: "guide-nav",
                div { class: "guide-nav-inner",
                    Link {
                        to: Route::Landing {},
                        class: "guide-brand",
                        div { class: "guide-logo" }
                        div { class: "guide-brand-text",
                            span { class: "guide-brand-name", "LOGICAFFEINE" }
                            span { class: "guide-brand-subtitle", "Programmer's Guide" }
                        }
                    }

                    div { class: "guide-nav-links",
                        Link { to: Route::Studio {}, class: "guide-nav-link", "Studio" }
                        Link { to: Route::Learn {}, class: "guide-nav-link", "Learn" }
                        Link { to: Route::Roadmap {}, class: "guide-nav-link", "Roadmap" }
                        Link { to: Route::Landing {}, class: "guide-nav-link primary", "Home" }
                    }
                }
            }

            // Hero
            header { class: "guide-hero",
                div { class: "guide-hero-badge",
                    div { class: "dot" }
                    span { "Interactive Guide" }
                }
                h1 { "LOGOS Language Guide" }
                p {
                    "Write English. Get Logic. Run Code. A comprehensive guide to programming in LOGOS, from basics to advanced features."
                }
            }

            // Main layout
            div { class: "guide-layout",
                // Sidebar
                GuideSidebar {
                    sections: sections_info,
                    active_section: active_section.read().clone(),
                    on_section_click: move |id: String| {
                        active_section.set(id);
                    },
                }

                // Content
                main { class: "guide-content",
                    for section in SECTIONS.iter() {
                        {
                            // Check if we need a part divider
                            let show_divider = section.part != current_part && section.number > 1;
                            current_part = section.part.to_string();

                            rsx! {
                                // Part divider
                                if show_divider {
                                    div { class: "guide-part-divider",
                                        h2 { "{section.part}" }
                                    }
                                }

                                // Section
                                section {
                                    id: "{section.id}",
                                    class: "guide-section",

                                    h2 {
                                        span { class: "section-number", "{section.number}." }
                                        "{section.title}"
                                    }

                                    // Render content as HTML
                                    div {
                                        dangerous_inner_html: render_markdown(section.content)
                                    }

                                    // Render code examples
                                    if !section.examples.is_empty() {
                                        div { class: "guide-examples",
                                            for example in section.examples.iter() {
                                                GuideCodeBlock {
                                                    id: example.id.to_string(),
                                                    label: example.label.to_string(),
                                                    mode: example.mode,
                                                    initial_code: example.code.to_string(),
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
}

/// Simple markdown to HTML converter
/// Handles: headers, paragraphs, lists, tables, inline code, bold
fn render_markdown(content: &str) -> String {
    let mut html = String::new();
    let mut in_list = false;
    let mut in_table = false;
    let mut in_table_header = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip empty lines
        if trimmed.is_empty() {
            if in_list {
                html.push_str("</ul>");
                in_list = false;
            }
            if in_table {
                html.push_str("</tbody></table>");
                in_table = false;
            }
            continue;
        }

        // Headers
        if trimmed.starts_with("### ") {
            if in_list { html.push_str("</ul>"); in_list = false; }
            if in_table { html.push_str("</tbody></table>"); in_table = false; }
            html.push_str(&format!("<h3>{}</h3>", inline_markdown(&trimmed[4..])));
            continue;
        }

        // Table row
        if trimmed.starts_with('|') && trimmed.ends_with('|') {
            // Check if this is a separator row (|---|---|)
            if trimmed.contains("---") {
                in_table_header = false;
                continue;
            }

            if !in_table {
                html.push_str("<table><thead>");
                in_table = true;
                in_table_header = true;
            }

            let cells: Vec<&str> = trimmed[1..trimmed.len()-1]
                .split('|')
                .map(|s| s.trim())
                .collect();

            if in_table_header {
                html.push_str("<tr>");
                for cell in &cells {
                    html.push_str(&format!("<th>{}</th>", inline_markdown(cell)));
                }
                html.push_str("</tr></thead><tbody>");
            } else {
                html.push_str("<tr>");
                for cell in &cells {
                    html.push_str(&format!("<td>{}</td>", inline_markdown(cell)));
                }
                html.push_str("</tr>");
            }
            continue;
        }

        // Close table if not a table row
        if in_table && !trimmed.starts_with('|') {
            html.push_str("</tbody></table>");
            in_table = false;
        }

        // List items
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            if !in_list {
                html.push_str("<ul>");
                in_list = true;
            }
            html.push_str(&format!("<li>{}</li>", inline_markdown(&trimmed[2..])));
            continue;
        }

        // Numbered list
        if trimmed.chars().next().map_or(false, |c| c.is_ascii_digit()) {
            if let Some(dot_pos) = trimmed.find(". ") {
                if !in_list {
                    html.push_str("<ul>");
                    in_list = true;
                }
                html.push_str(&format!("<li>{}</li>", inline_markdown(&trimmed[dot_pos + 2..])));
                continue;
            }
        }

        // Close list if not a list item
        if in_list {
            html.push_str("</ul>");
            in_list = false;
        }

        // Paragraph
        html.push_str(&format!("<p>{}</p>", inline_markdown(trimmed)));
    }

    // Close any open tags
    if in_list {
        html.push_str("</ul>");
    }
    if in_table {
        html.push_str("</tbody></table>");
    }

    html
}

/// Process inline markdown: **bold**, `code`, [links]
fn inline_markdown(text: &str) -> String {
    let mut result = text.to_string();

    // Escape HTML entities
    result = result.replace('&', "&amp;");
    result = result.replace('<', "&lt;");
    result = result.replace('>', "&gt;");

    // Bold: **text**
    while let Some(start) = result.find("**") {
        if let Some(end) = result[start + 2..].find("**") {
            let before = &result[..start];
            let inner = &result[start + 2..start + 2 + end];
            let after = &result[start + 2 + end + 2..];
            result = format!("{}<strong>{}</strong>{}", before, inner, after);
        } else {
            break;
        }
    }

    // Inline code: `code`
    while let Some(start) = result.find('`') {
        if let Some(end) = result[start + 1..].find('`') {
            let before = &result[..start];
            let inner = &result[start + 1..start + 1 + end];
            let after = &result[start + 1 + end + 1..];
            result = format!("{}<code>{}</code>{}", before, inner, after);
        } else {
            break;
        }
    }

    result
}
