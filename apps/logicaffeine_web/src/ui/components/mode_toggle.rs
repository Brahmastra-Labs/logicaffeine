//! Studio mode toggle component.
//!
//! Provides a segmented control for switching between Logic, Code, and Math modes
//! in the Studio playground. Icons indicate each mode; labels are hidden on mobile.
//!
//! # Props
//!
//! - `mode` - Currently active mode
//! - `on_change` - Callback when a new mode is selected
//!
//! # Modes
//!
//! - **Logic** (∀): English-to-FOL translation
//! - **Code** (λ): Vernacular programming REPL
//! - **Math** (π): LaTeX formula builder

use dioxus::prelude::*;
use crate::ui::state::StudioMode;

const MODE_TOGGLE_STYLE: &str = r#"
.mode-toggle {
    display: flex;
    gap: 2px;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 8px;
    padding: 3px;
}

.mode-toggle-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 14px;
    border: none;
    background: transparent;
    color: rgba(255, 255, 255, 0.5);
    font-size: 13px;
    font-weight: 500;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.15s ease;
    white-space: nowrap;
}

.mode-toggle-btn:hover {
    color: rgba(255, 255, 255, 0.8);
    background: rgba(255, 255, 255, 0.04);
}

.mode-toggle-btn.active {
    background: rgba(102, 126, 234, 0.15);
    color: #667eea;
}

.mode-toggle-btn .icon {
    font-size: 14px;
}

/* Mobile: compact with labels */
@media (max-width: 768px) {
    .mode-toggle-btn {
        padding: 6px 10px;
        font-size: 12px;
        gap: 4px;
    }

    .mode-toggle-btn .icon {
        font-size: 12px;
    }

    .mode-toggle-btn .label {
        font-size: 12px;
    }
}
"#;

#[component]
pub fn ModeToggle(
    mode: StudioMode,
    on_change: EventHandler<StudioMode>,
) -> Element {
    rsx! {
        style { "{MODE_TOGGLE_STYLE}" }

        div { class: "mode-toggle",
            // Logic mode button
            button {
                class: if mode == StudioMode::Logic { "mode-toggle-btn active" } else { "mode-toggle-btn" },
                onclick: move |_| on_change.call(StudioMode::Logic),
                title: "Logic Mode - English to FOL translation",
                span { class: "icon", "\u{2200}" } // ∀ forall
                span { class: "label", "Logic" }
            }

            // Code mode button
            button {
                class: if mode == StudioMode::Code { "mode-toggle-btn active" } else { "mode-toggle-btn" },
                onclick: move |_| on_change.call(StudioMode::Code),
                title: "Code Mode - Vernacular REPL",
                span { class: "icon", "\u{03BB}" } // λ lambda
                span { class: "label", "Code" }
            }

            // Math mode button
            button {
                class: if mode == StudioMode::Math { "mode-toggle-btn active" } else { "mode-toggle-btn" },
                onclick: move |_| on_change.call(StudioMode::Math),
                title: "Math Mode - Formula builder",
                span { class: "icon", "\u{03C0}" } // π pi
                span { class: "label", "Math" }
            }
        }
    }
}
