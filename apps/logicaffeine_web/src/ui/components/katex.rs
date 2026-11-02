//! KaTeX LaTeX rendering component.
//!
//! Renders mathematical expressions using the KaTeX library via JavaScript interop.
//! Supports both inline and display mode rendering.
//!
//! # Components
//!
//! - [`KatexSpan`] - Renders a single LaTeX expression
//!
//! # Functions
//!
//! - [`parse_latex_in_text`] - Parses text containing `$...$` and `$$...$$` markers
//!
//! # Usage
//!
//! ```no_run
//! # use dioxus::prelude::*;
//! # use logicaffeine_web::ui::components::katex::KatexSpan;
//! # fn Example() -> Element {
//! rsx! {
//!     // Inline math
//!     KatexSpan { latex: "x + y".to_string() }
//!     // Display mode (centered, larger)
//!     KatexSpan { latex: r"\forall x".to_string(), display: true }
//! }
//! # }
//! ```

use dioxus::prelude::*;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = renderKaTeX)]
    fn render_katex(element_id: &str, latex: &str, display_mode: bool);
}

static KATEX_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn next_katex_id() -> String {
    let id = KATEX_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("katex-{}", id)
}

#[component]
pub fn KatexSpan(latex: String, #[props(default = false)] display: bool) -> Element {
    let element_id = use_signal(|| next_katex_id());

    use_effect(move || {
        let id = element_id.read().clone();
        let latex = latex.clone();
        render_katex(&id, &latex, display);
    });

    rsx! {
        span {
            id: "{element_id}",
            class: if display { "katex-display" } else { "katex-inline" }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TextPart {
    Plain(String),
    Latex { content: String, display: bool },
}

pub fn parse_latex_in_text(text: &str) -> Vec<TextPart> {
    let mut parts = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        let display_pos = remaining.find("$$");
        let inline_pos = remaining.find('$');

        match (display_pos, inline_pos) {
            (Some(d), Some(i)) if d <= i => {
                if d > 0 {
                    parts.push(TextPart::Plain(remaining[..d].to_string()));
                }
                remaining = &remaining[d + 2..];
                if let Some(end) = remaining.find("$$") {
                    parts.push(TextPart::Latex {
                        content: remaining[..end].to_string(),
                        display: true,
                    });
                    remaining = &remaining[end + 2..];
                } else {
                    parts.push(TextPart::Plain(format!("$${}", remaining)));
                    break;
                }
            }
            (_, Some(i)) => {
                if i > 0 {
                    parts.push(TextPart::Plain(remaining[..i].to_string()));
                }
                remaining = &remaining[i + 1..];
                if let Some(end) = remaining.find('$') {
                    parts.push(TextPart::Latex {
                        content: remaining[..end].to_string(),
                        display: false,
                    });
                    remaining = &remaining[end + 1..];
                } else {
                    parts.push(TextPart::Plain(format!("${}", remaining)));
                    break;
                }
            }
            (Some(d), None) => {
                if d > 0 {
                    parts.push(TextPart::Plain(remaining[..d].to_string()));
                }
                remaining = &remaining[d + 2..];
                if let Some(end) = remaining.find("$$") {
                    parts.push(TextPart::Latex {
                        content: remaining[..end].to_string(),
                        display: true,
                    });
                    remaining = &remaining[end + 2..];
                } else {
                    parts.push(TextPart::Plain(format!("$${}", remaining)));
                    break;
                }
            }
            (None, None) => {
                parts.push(TextPart::Plain(remaining.to_string()));
                break;
            }
        }
    }

    parts
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/components/katex/tests.rs"]
mod tests;
