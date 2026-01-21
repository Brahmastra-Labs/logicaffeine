//! Compact theme selector component.
//!
//! Provides icon buttons for switching between the four nature-inspired themes:
//! Sunrise, Violet, Ocean, and Mountain.
//!
//! # Usage
//!
//! ```ignore
//! use crate::ui::components::theme_picker::ThemePicker;
//!
//! rsx! {
//!     ThemePicker {}
//! }
//! ```

use dioxus::prelude::*;
use crate::ui::theme_state::{Theme, ThemeState};
use crate::ui::components::icon::{Icon, IconVariant, IconSize};

const THEME_PICKER_STYLE: &str = r#"
.theme-picker {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px;
    background: rgba(255, 255, 255, 0.04);
    border-radius: 12px;
    border: 1px solid rgba(255, 255, 255, 0.08);
}

.theme-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    border: none;
    border-radius: 8px;
    background: transparent;
    cursor: pointer;
    transition: all 0.2s ease;
    color: var(--text-tertiary);
    position: relative;
}

.theme-btn:hover {
    background: rgba(255, 255, 255, 0.08);
    color: var(--text-primary);
}

.theme-btn.active {
    background: rgba(var(--accent-primary-rgb), 0.2);
    color: var(--accent-primary);
}

.theme-btn.sunrise { --btn-color: #ff7f50; }
.theme-btn.violet { --btn-color: #e6b3ff; }
.theme-btn.ocean { --btn-color: #40e0d0; }
.theme-btn.mountain { --btn-color: #22c55e; }

.theme-btn.active.sunrise { background: rgba(255, 127, 80, 0.2); color: #ff7f50; }
.theme-btn.active.violet { background: rgba(230, 179, 255, 0.2); color: #e6b3ff; }
.theme-btn.active.ocean { background: rgba(64, 224, 208, 0.2); color: #40e0d0; }
.theme-btn.active.mountain { background: rgba(34, 197, 94, 0.2); color: #22c55e; }

.theme-tooltip {
    position: absolute;
    bottom: -32px;
    left: 50%;
    transform: translateX(-50%);
    padding: 4px 8px;
    background: rgba(0, 0, 0, 0.9);
    border-radius: 4px;
    font-size: 11px;
    font-weight: 500;
    white-space: nowrap;
    opacity: 0;
    visibility: hidden;
    transition: opacity 0.15s ease, visibility 0.15s ease;
    pointer-events: none;
    z-index: 100;
}

.theme-btn:hover .theme-tooltip {
    opacity: 1;
    visibility: visible;
}

/* Mobile: Vertical layout */
@media (max-width: 768px) {
    .theme-picker.mobile {
        flex-direction: column;
        padding: 8px;
    }

    .theme-picker.mobile .theme-btn {
        width: 44px;
        height: 44px;
    }

    .theme-picker.mobile .theme-tooltip {
        bottom: auto;
        left: calc(100% + 8px);
        transform: none;
    }
}
"#;

/// Compact theme picker with icon buttons.
#[component]
pub fn ThemePicker(
    /// Use vertical mobile layout
    #[props(default = false)]
    mobile: bool,
) -> Element {
    let mut theme_state = use_context::<ThemeState>();
    let current_theme = theme_state.current();

    let picker_class = if mobile {
        "theme-picker mobile"
    } else {
        "theme-picker"
    };

    rsx! {
        style { "{THEME_PICKER_STYLE}" }

        div { class: "{picker_class}",
            // Sunrise theme button
            button {
                class: if current_theme == Theme::Sunrise { "theme-btn sunrise active" } else { "theme-btn sunrise" },
                onclick: move |_| theme_state.set_theme(Theme::Sunrise),
                title: "Sunrise theme",
                Icon { variant: IconVariant::Sunrise, size: IconSize::Medium }
                span { class: "theme-tooltip", "Sunrise" }
            }

            // Violet theme button
            button {
                class: if current_theme == Theme::Violet { "theme-btn violet active" } else { "theme-btn violet" },
                onclick: move |_| theme_state.set_theme(Theme::Violet),
                title: "Violet theme",
                Icon { variant: IconVariant::Moon, size: IconSize::Medium }
                span { class: "theme-tooltip", "Violet" }
            }

            // Ocean theme button
            button {
                class: if current_theme == Theme::Ocean { "theme-btn ocean active" } else { "theme-btn ocean" },
                onclick: move |_| theme_state.set_theme(Theme::Ocean),
                title: "Ocean theme",
                Icon { variant: IconVariant::Wave, size: IconSize::Medium }
                span { class: "theme-tooltip", "Ocean" }
            }

            // Mountain theme button (default)
            button {
                class: if current_theme == Theme::Mountain { "theme-btn mountain active" } else { "theme-btn mountain" },
                onclick: move |_| theme_state.set_theme(Theme::Mountain),
                title: "Mountain theme",
                Icon { variant: IconVariant::Mountain, size: IconSize::Medium }
                span { class: "theme-tooltip", "Mountain" }
            }
        }
    }
}

/// Simple theme cycle button for compact spaces.
#[component]
pub fn ThemeCycleButton() -> Element {
    let mut theme_state = use_context::<ThemeState>();
    let current_theme = theme_state.current();

    let icon = match current_theme {
        Theme::Sunrise => IconVariant::Sunrise,
        Theme::Violet => IconVariant::Moon,
        Theme::Ocean => IconVariant::Wave,
        Theme::Mountain => IconVariant::Mountain,
    };

    rsx! {
        button {
            class: "theme-btn",
            onclick: move |_| theme_state.cycle_theme(),
            title: "Change theme ({current_theme.name()})",
            Icon { variant: icon, size: IconSize::Medium }
        }
    }
}
