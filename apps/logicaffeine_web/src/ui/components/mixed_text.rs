//! Mixed text and LaTeX rendering component.
//!
//! Renders text that may contain embedded LaTeX expressions.
//! Automatically parses `$...$` (inline) and `$$...$$` (display) markers.
//!
//! # Props
//!
//! - `content` - Text containing optional LaTeX markers
//!
//! # Example
//!
//! ```no_run
//! # use dioxus::prelude::*;
//! # use logicaffeine_web::ui::components::mixed_text::MixedText;
//! # fn Example() -> Element {
//! rsx! {
//!     MixedText { content: "The formula $x + y$ equals...".to_string() }
//! }
//! # }
//! ```

use dioxus::prelude::*;
use super::katex::{KatexSpan, TextPart, parse_latex_in_text};

#[component]
pub fn MixedText(content: String) -> Element {
    let parts = parse_latex_in_text(&content);

    rsx! {
        span { class: "mixed-text",
            for (i, part) in parts.iter().enumerate() {
                match part {
                    TextPart::Plain(text) => rsx! {
                        span { key: "{i}", "{text}" }
                    },
                    TextPart::Latex { content, display } => rsx! {
                        KatexSpan { key: "{i}", latex: content.clone(), display: *display }
                    },
                }
            }
        }
    }
}
