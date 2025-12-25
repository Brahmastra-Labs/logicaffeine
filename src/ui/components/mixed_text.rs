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
