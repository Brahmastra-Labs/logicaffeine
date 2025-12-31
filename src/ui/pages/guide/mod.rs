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
use crate::ui::components::main_nav::{MainNav, ActivePage};
use content::SECTIONS;

const GUIDE_STYLE: &str = r#"
.guide-page {
    min-height: 100vh;
    color: var(--text-primary);
    background:
        radial-gradient(1200px 600px at 50% -120px, rgba(167,139,250,0.14), transparent 60%),
        radial-gradient(900px 500px at 15% 30%, rgba(96,165,250,0.14), transparent 60%),
        radial-gradient(800px 450px at 90% 45%, rgba(34,197,94,0.08), transparent 62%),
        linear-gradient(180deg, #070a12, #0b1022 55%, #070a12);
    font-family: var(--font-sans);
}

/* Navigation - now handled by MainNav component */

/* Hero */
.guide-hero {
    max-width: 1280px;
    margin: 0 auto;
    padding: 60px var(--spacing-xl) 40px;
}

.guide-hero h1 {
    font-size: var(--font-display-lg);
    font-weight: 900;
    letter-spacing: -1.5px;
    line-height: 1.1;
    background: linear-gradient(180deg, #ffffff 0%, rgba(229,231,235,0.78) 65%, rgba(229,231,235,0.62) 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    margin: 0 0 var(--spacing-lg);
}

.guide-hero p {
    font-size: var(--font-body-lg);
    color: var(--text-secondary);
    max-width: 600px;
    line-height: 1.6;
    margin: 0;
}

.guide-hero-badge {
    display: inline-flex;
    align-items: center;
    gap: var(--spacing-sm);
    padding: var(--spacing-sm) 14px;
    border-radius: var(--radius-full);
    background: rgba(255,255,255,0.06);
    border: 1px solid rgba(255,255,255,0.10);
    font-size: var(--font-caption-md);
    font-weight: 600;
    color: var(--text-primary);
    margin-bottom: var(--spacing-xl);
}

.guide-hero-badge .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--color-success);
    box-shadow: 0 0 0 4px rgba(34,197,94,0.15);
}

/* Layout */
.guide-layout {
    max-width: 1280px;
    margin: 0 auto;
    display: flex;
    gap: 48px;
    padding: 0 var(--spacing-xl) 80px;
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
    font-size: var(--font-display-md);
    font-weight: 800;
    letter-spacing: -0.8px;
    line-height: 1.2;
    background: linear-gradient(180deg, #ffffff 0%, rgba(229,231,235,0.85) 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    margin: 0 0 var(--spacing-xl);
    padding-bottom: var(--spacing-lg);
    border-bottom: 1px solid rgba(255,255,255,0.08);
}

.guide-section h3 {
    font-size: var(--font-heading-sm);
    font-weight: 700;
    color: var(--text-primary);
    margin: var(--spacing-xxl) 0 var(--spacing-lg);
}

.guide-section p {
    color: var(--text-secondary);
    font-size: var(--font-body-sm);
    line-height: 1.75;
    margin: 0 0 var(--spacing-lg);
}

.guide-section ul,
.guide-section ol {
    color: var(--text-secondary);
    font-size: var(--font-body-sm);
    line-height: 1.75;
    padding-left: var(--spacing-xl);
    margin: 0 0 var(--spacing-lg);
}

.guide-section li {
    margin-bottom: var(--spacing-sm);
}

.guide-section code {
    font-family: var(--font-mono);
    background: rgba(255,255,255,0.08);
    padding: 3px 7px;
    border-radius: var(--radius-sm);
    font-size: 0.9em;
    color: var(--color-accent-purple);
}

.guide-section strong {
    color: var(--text-primary);
    font-weight: 600;
}

/* Tables */
.guide-section table {
    width: 100%;
    border-collapse: collapse;
    margin: var(--spacing-xl) 0;
    font-size: var(--font-body-md);
    border-radius: var(--radius-lg);
    overflow: hidden;
    border: 1px solid rgba(255,255,255,0.08);
}

.guide-section th {
    text-align: left;
    padding: 14px var(--spacing-lg);
    background: rgba(255,255,255,0.05);
    color: var(--text-primary);
    font-weight: 600;
    border-bottom: 1px solid rgba(255,255,255,0.08);
}

.guide-section td {
    padding: var(--spacing-md) var(--spacing-lg);
    color: var(--text-secondary);
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
    padding: var(--spacing-xl) 0;
    border-top: 1px solid rgba(255,255,255,0.08);
}

.guide-part-divider h2 {
    font-size: var(--font-body-md);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 1.5px;
    color: var(--text-tertiary);
    margin: 0;
    background: none;
    -webkit-text-fill-color: currentColor;
    border-bottom: none;
    padding-bottom: 0;
}

/* Section number */
.section-number {
    display: inline-block;
    font-size: var(--font-body-md);
    font-weight: 700;
    color: var(--color-accent-purple);
    margin-right: var(--spacing-sm);
    opacity: 0.8;
}

/* Examples container */
.guide-examples {
    margin-top: var(--spacing-xl);
}

/* Responsive */
@media (max-width: 1024px) {
    .guide-layout {
        flex-direction: column;
    }

    .guide-hero h1 {
        font-size: var(--font-display-md);
    }

    .guide-hero {
        padding: 40px var(--spacing-xl) var(--spacing-xxl);
    }
}

@media (max-width: 640px) {
    .guide-hero h1 {
        font-size: var(--font-heading-lg);
    }

    .guide-hero p {
        font-size: var(--font-body-md);
    }

    .guide-section h2 {
        font-size: var(--font-heading-lg);
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

    // Collect all section IDs for intersection observer
    #[allow(unused_variables)]
    let section_ids: Vec<String> = SECTIONS.iter().map(|s| s.id.to_string()).collect();

    // Set up scroll tracking with IntersectionObserver
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::prelude::*;
        use wasm_bindgen::JsCast;

        let section_ids_for_effect = section_ids.clone();

        use_effect(move || {
            let window = match web_sys::window() {
                Some(w) => w,
                None => return,
            };
            let document = match window.document() {
                Some(d) => d,
                None => return,
            };

            // Use RefCell to allow mutation from within Fn closure
            use std::cell::RefCell;
            use std::rc::Rc;

            let active_section_clone = Rc::new(RefCell::new(active_section.clone()));
            let active_section_for_closure = active_section_clone.clone();

            let callback = Closure::<dyn Fn(js_sys::Array, web_sys::IntersectionObserver)>::new(
                move |entries: js_sys::Array, _observer: web_sys::IntersectionObserver| {
                    // Simple approach: when a section crosses the threshold line,
                    // it becomes active
                    for i in 0..entries.length() {
                        if let Ok(entry) = entries.get(i).dyn_into::<web_sys::IntersectionObserverEntry>() {
                            if entry.is_intersecting() {
                                let target = entry.target();
                                let id = target.id();
                                if !id.is_empty() {
                                    active_section_for_closure.borrow_mut().set(id);
                                }
                            }
                        }
                    }
                },
            );

            // Create IntersectionObserver options
            let mut options = web_sys::IntersectionObserverInit::new();
            // Root margin creates a thin "tripwire" near the top of the screen
            options.root_margin("-100px 0px -90% 0px");
            let thresholds = js_sys::Array::new();
            thresholds.push(&JsValue::from(0.0));
            options.threshold(&thresholds);

            // Create the observer
            let observer = match web_sys::IntersectionObserver::new_with_options(
                callback.as_ref().unchecked_ref(),
                &options,
            ) {
                Ok(obs) => obs,
                Err(_) => return,
            };

            // Observe all sections
            for section_id in &section_ids_for_effect {
                if let Some(element) = document.get_element_by_id(section_id) {
                    observer.observe(&element);
                }
            }

            // Keep callback alive
            callback.forget();
        });
    }

    // Track current part for dividers
    let mut current_part = String::new();

    rsx! {
        style { "{GUIDE_STYLE}" }

        div { class: "guide-page",
            // Navigation
            MainNav {
                active: ActivePage::Guide,
                subtitle: Some("Programmer's Guide"),
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
