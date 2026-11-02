//! Misconception-first callout (Derek Muller): state the belief the reader
//! arrives with — in their voice — then what LOGOS does and why, and deep-link
//! the runnable evidence example that settles it.

use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct MisconceptionBoxProps {
    pub id: String,
    pub expectation: String,
    pub reality: String,
    /// Example id in the same section — the anchor the "See it run" link jumps to.
    pub evidence: String,
}

#[component]
pub fn MisconceptionBox(props: MisconceptionBoxProps) -> Element {
    rsx! {
        div {
            class: "guide-misconception",
            id: "{props.id}",
            style: "border: 1px solid rgba(251,191,36,0.25); border-left: 3px solid #fbbf24; border-radius: 0 4px 4px 0; padding: 14px 16px; margin: 16px 0; background: rgba(251,191,36,0.05);",

            div {
                style: "font-size: 11px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.5px; color: #fbbf24; margin-bottom: 8px;",
                "You might expect"
            }
            div {
                style: "font-size: 14px; color: rgba(229,231,235,0.9); line-height: 1.6; font-style: italic;",
                "{props.expectation}"
            }
            div {
                style: "font-size: 11px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.5px; color: #22c55e; margin: 12px 0 8px;",
                "LOGOS instead"
            }
            div {
                style: "font-size: 14px; color: rgba(229,231,235,0.9); line-height: 1.6;",
                "{props.reality}"
            }
            a {
                style: "display: inline-block; margin-top: 10px; font-size: 12px; color: #60a5fa; text-decoration: none; font-weight: 600;",
                href: "#{props.evidence}",
                "See it run ↓"
            }
        }
    }
}
