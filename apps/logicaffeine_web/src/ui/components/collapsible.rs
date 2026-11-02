//! A collapsible block: a titled section of a Studio panel that the reader opens and closes.
//!
//! Its heading is a button — the drawn chevron, then the title in the panel-title type — that
//! says whether the block is expanded and names the region it controls. A closed block is its
//! heading alone and opens wherever it is clicked; an open block closes from its heading, never
//! from its content. Its content stays in the page while it is closed, hidden, so whatever state
//! the content keeps survives a close.
//!
//! The owner keeps the open state, so the reader's choice outlives the block's content coming and
//! going:
//!
//! ```no_run
//! # use dioxus::prelude::*;
//! use logicaffeine_web::ui::components::collapsible::Collapsible;
//!
//! # fn Example() -> Element {
//! let open = use_signal(|| false);
//! rsx! {
//!     Collapsible { title: "Syntax Tree", open, p { "…" } }
//! }
//! # }
//! ```

use dioxus::prelude::*;
use crate::ui::components::chevron::{Chevron, CHEVRON_STYLE};

const COLLAPSIBLE_STYLE: &str = r#"
/* A section of its panel set off by a hairline, as the panel's other parts are — not a card. */
.collapsible {
    border-top: 1px solid var(--studio-border, rgba(255, 255, 255, 0.08));
}

.collapsible:not(.open) {
    cursor: pointer;
}

/* The heading is the block's whole row, the chevron at the panel's inset and the title beside it;
   a closed block is this row alone. */
.collapsible-heading {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-height: 44px;
    margin: 0;
    padding: 0 var(--studio-inset, 20px);
    border: none;
    background: transparent;
    font: inherit;
    text-align: left;
    cursor: pointer;
}

.collapsible-heading:hover {
    background: rgba(255, 255, 255, 0.04);
}

.collapsible-heading:focus-visible {
    outline: 2px solid var(--studio-accent, #667eea);
    outline-offset: -2px;
}

/* The content holds the panel's inset on either side and below. */
.collapsible-body {
    padding: 0 var(--studio-inset, 20px) var(--studio-inset, 20px);
}
"#;

/// The words of `title`, lowercased and joined by hyphens: "Symbol Dictionary" names its parts
/// `symbol-dictionary-heading` and `symbol-dictionary-body`.
fn slug(title: &str) -> String {
    title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join("-")
}

/// A titled block of a panel that the reader opens and closes.
#[component]
pub fn Collapsible(
    /// What the block holds, named in the panel-title type.
    #[props(into)]
    title: String,
    /// Whether the block is open. The owner keeps it.
    open: Signal<bool>,
    /// Names the heading (`{id}-heading`) and the region it controls (`{id}-body`); the title's
    /// slug unless given, so two blocks with one title on a page need one each.
    #[props(default, into)]
    id: Option<String>,
    children: Element,
) -> Element {
    let mut open = open;
    let is_open = open();
    let id = id.unwrap_or_else(|| slug(&title));

    rsx! {
        style { "{CHEVRON_STYLE}" }
        style { "{COLLAPSIBLE_STYLE}" }
        section {
            class: if is_open { "collapsible open" } else { "collapsible" },
            // Closed, the block opens wherever it is pressed — its hairline too.
            onclick: move |_| {
                if !open() {
                    open.set(true);
                }
            },
            button {
                r#type: "button",
                id: "{id}-heading",
                class: "collapsible-heading",
                aria_expanded: "{is_open}",
                aria_controls: "{id}-body",
                onclick: move |evt| {
                    evt.stop_propagation();
                    let now = open();
                    open.set(!now);
                },
                Chevron { open: is_open }
                span { class: "collapsible-title panel-title", "{title}" }
            }
            div {
                id: "{id}-body",
                class: "collapsible-body",
                role: "region",
                aria_labelledby: "{id}-heading",
                hidden: !is_open,
                {children}
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/components/collapsible/tests.rs"]
mod tests;
