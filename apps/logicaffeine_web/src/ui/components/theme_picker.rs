//! Compact theme selector component.
//!
//! Provides a dropdown selector for switching between the nature-inspired themes.
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
.theme-dropdown {
    position: relative;
    display: inline-block;
}

.theme-dropdown-btn {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 10px;
    cursor: pointer;
    transition: all 0.2s ease;
    color: var(--text-secondary);
    font-size: 14px;
    font-weight: 500;
    min-width: 140px;
}

.theme-dropdown-btn:hover {
    background: rgba(255, 255, 255, 0.08);
    border-color: rgba(255, 255, 255, 0.12);
    color: var(--text-primary);
}

.theme-dropdown-btn .theme-icon {
    color: var(--accent-primary);
}

.theme-dropdown-btn .theme-name {
    flex: 1;
    text-align: left;
}

.theme-dropdown-btn .chevron {
    transition: transform 0.2s ease;
}

.theme-dropdown.open .chevron {
    transform: rotate(180deg);
}

.theme-dropdown-menu {
    position: absolute;
    bottom: 100%;
    left: 0;
    right: 0;
    margin-bottom: 4px;
    background: rgba(15, 15, 20, 0.98);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 10px;
    padding: 6px;
    opacity: 0;
    visibility: hidden;
    transform: translateY(8px);
    transition: all 0.2s ease;
    z-index: 1000;
    box-shadow: 0 -8px 32px rgba(0, 0, 0, 0.4);
}

.theme-dropdown.open .theme-dropdown-menu {
    opacity: 1;
    visibility: visible;
    transform: translateY(0);
}

.theme-option {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.15s ease;
    color: var(--text-secondary);
    border: none;
    background: transparent;
    width: 100%;
    font-size: 14px;
    text-align: left;
}

.theme-option:hover {
    background: rgba(255, 255, 255, 0.08);
    color: var(--text-primary);
}

.theme-option.active {
    background: rgba(var(--accent-primary-rgb), 0.15);
    color: var(--accent-primary);
}

.theme-option .option-icon {
    width: 20px;
    display: flex;
    justify-content: center;
}

.theme-option.sunrise .option-icon { color: #f97316; }
.theme-option.violet .option-icon { color: #a78bfa; }
.theme-option.ocean .option-icon { color: #22d3ee; }
.theme-option.mountain .option-icon { color: #00d4ff; }
.theme-option.rose .option-icon { color: #f472b6; }
.theme-option.forest .option-icon { color: #4ade80; }
.theme-option.ember .option-icon { color: #ef4444; }

/* Mobile adjustments */
@media (max-width: 768px) {
    .theme-dropdown-btn {
        padding: 10px 14px;
        min-width: 150px;
    }

    .theme-option {
        padding: 12px 14px;
    }
}
"#;

fn theme_icon(theme: Theme) -> IconVariant {
    match theme {
        Theme::Sunrise => IconVariant::Sunrise,
        Theme::Violet => IconVariant::Moon,
        Theme::Ocean => IconVariant::Wave,
        Theme::Mountain => IconVariant::Mountain,
        Theme::Rose => IconVariant::Flower,
        Theme::Forest => IconVariant::Leaf,
        Theme::Ember => IconVariant::Fire,
    }
}

fn theme_class(theme: Theme) -> &'static str {
    match theme {
        Theme::Sunrise => "sunrise",
        Theme::Violet => "violet",
        Theme::Ocean => "ocean",
        Theme::Mountain => "mountain",
        Theme::Rose => "rose",
        Theme::Forest => "forest",
        Theme::Ember => "ember",
    }
}

/// Compact theme picker with dropdown selector.
#[component]
pub fn ThemePicker() -> Element {
    let mut theme_state = use_context::<ThemeState>();
    let current_theme = theme_state.current();
    let mut is_open = use_signal(|| false);

    let dropdown_class = if *is_open.read() {
        "theme-dropdown open"
    } else {
        "theme-dropdown"
    };

    rsx! {
        style { "{THEME_PICKER_STYLE}" }

        div {
            class: "{dropdown_class}",

            button {
                class: "theme-dropdown-btn",
                onclick: move |_| {
                    let current = *is_open.read();
                    is_open.set(!current);
                },

                span { class: "theme-icon",
                    Icon { variant: theme_icon(current_theme), size: IconSize::Medium }
                }
                span { class: "theme-name", "{current_theme.name()}" }
                span { class: "chevron",
                    Icon { variant: IconVariant::ChevronDown, size: IconSize::Small }
                }
            }

            div { class: "theme-dropdown-menu",
                for theme in Theme::all() {
                    {
                        let is_active = theme == current_theme;
                        let option_class = if is_active {
                            format!("theme-option {} active", theme_class(theme))
                        } else {
                            format!("theme-option {}", theme_class(theme))
                        };
                        rsx! {
                            button {
                                class: "{option_class}",
                                onclick: move |_| {
                                    theme_state.set_theme(theme);
                                    is_open.set(false);
                                },
                                span { class: "option-icon",
                                    Icon { variant: theme_icon(theme), size: IconSize::Medium }
                                }
                                span { "{theme.name()}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Simple theme cycle button for compact spaces.
#[component]
pub fn ThemeCycleButton() -> Element {
    let mut theme_state = use_context::<ThemeState>();
    let current_theme = theme_state.current();

    let icon = theme_icon(current_theme);

    rsx! {
        button {
            class: "theme-btn",
            onclick: move |_| theme_state.cycle_theme(),
            title: "Change theme ({current_theme.name()})",
            Icon { variant: icon, size: IconSize::Medium }
        }
    }
}
