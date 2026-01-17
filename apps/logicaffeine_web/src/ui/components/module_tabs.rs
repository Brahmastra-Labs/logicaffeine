//! Module Tab Bar component for the integrated Learn page.
//!
//! Displays tabs: LESSON | EXAMPLES | PRACTICE ∞ | TEST 📝
//! allowing users to switch between different learning modes within a module.

use dioxus::prelude::*;
use crate::learn_state::TabMode;

const MODULE_TABS_STYLE: &str = r#"
.module-tabs {
    display: flex;
    gap: 4px;
    padding: 4px;
    background: rgba(255, 255, 255, 0.03);
    border-radius: 12px;
    border: 1px solid rgba(255, 255, 255, 0.08);
    width: fit-content;
}

.module-tab {
    padding: 10px 18px;
    border-radius: 8px;
    border: none;
    background: transparent;
    color: rgba(229, 231, 235, 0.56);
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    cursor: pointer;
    transition: all 0.2s ease;
    white-space: nowrap;
}

.module-tab:hover:not(.locked):not(.active) {
    color: rgba(229, 231, 235, 0.85);
    background: rgba(255, 255, 255, 0.05);
}

.module-tab.active {
    background: linear-gradient(135deg, rgba(96, 165, 250, 0.2), rgba(167, 139, 250, 0.2));
    color: #e5e7eb;
    box-shadow: 0 2px 8px rgba(96, 165, 250, 0.15);
}

.module-tab.locked {
    opacity: 0.4;
    cursor: not-allowed;
}

.module-tab.lesson.active {
    background: linear-gradient(135deg, rgba(96, 165, 250, 0.25), rgba(96, 165, 250, 0.15));
}

.module-tab.examples.active {
    background: linear-gradient(135deg, rgba(167, 139, 250, 0.25), rgba(167, 139, 250, 0.15));
}

.module-tab.practice.active {
    background: linear-gradient(135deg, rgba(74, 222, 128, 0.25), rgba(74, 222, 128, 0.15));
}

.module-tab.test.active {
    background: linear-gradient(135deg, rgba(251, 146, 60, 0.25), rgba(251, 146, 60, 0.15));
}

/* Compact variant for inline use */
.module-tabs.compact {
    padding: 2px;
    gap: 2px;
}

.module-tabs.compact .module-tab {
    padding: 6px 12px;
    font-size: 11px;
}
"#;

/// Props for the ModuleTabs component
#[derive(Props, Clone, PartialEq)]
pub struct ModuleTabsProps {
    /// Currently active tab
    current: TabMode,
    /// Handler called when user clicks a tab
    on_change: EventHandler<TabMode>,
    /// Tabs that should be locked (disabled)
    #[props(default)]
    locked_tabs: Vec<TabMode>,
    /// Use compact variant
    #[props(default = false)]
    compact: bool,
}

/// Tab bar for switching between module learning modes
#[component]
pub fn ModuleTabs(props: ModuleTabsProps) -> Element {
    let container_class = if props.compact {
        "module-tabs compact"
    } else {
        "module-tabs"
    };

    rsx! {
        style { "{MODULE_TABS_STYLE}" }
        div { class: "{container_class}",
            for tab in TabMode::all() {
                {
                    let is_active = tab == props.current;
                    let is_locked = props.locked_tabs.contains(&tab);
                    let tab_class_name = match tab {
                        TabMode::Lesson => "lesson",
                        TabMode::Examples => "examples",
                        TabMode::Practice => "practice",
                        TabMode::Test => "test",
                    };
                    let class = format!(
                        "module-tab {}{}{}",
                        tab_class_name,
                        if is_active { " active" } else { "" },
                        if is_locked { " locked" } else { "" }
                    );

                    rsx! {
                        button {
                            key: "{tab_class_name}",
                            class: "{class}",
                            disabled: is_locked,
                            onclick: {
                                let on_change = props.on_change.clone();
                                move |_| {
                                    if !is_locked {
                                        on_change.call(tab);
                                    }
                                }
                            },
                            "{tab.label()}"
                        }
                    }
                }
            }
        }
    }
}

/// Individual tab button component for custom layouts
#[component]
pub fn TabButton(
    tab: TabMode,
    is_active: bool,
    #[props(default = false)] is_locked: bool,
    on_click: EventHandler<TabMode>,
) -> Element {
    let tab_class_name = match tab {
        TabMode::Lesson => "lesson",
        TabMode::Examples => "examples",
        TabMode::Practice => "practice",
        TabMode::Test => "test",
    };
    let class = format!(
        "module-tab {}{}{}",
        tab_class_name,
        if is_active { " active" } else { "" },
        if is_locked { " locked" } else { "" }
    );

    rsx! {
        style { "{MODULE_TABS_STYLE}" }
        button {
            class: "{class}",
            disabled: is_locked,
            onclick: move |_| {
                if !is_locked {
                    on_click.call(tab);
                }
            },
            "{tab.label()}"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tab_class_names() {
        // Verify class name generation logic
        assert!(TabMode::Lesson.label().contains("LESSON"));
        assert!(TabMode::Practice.label().contains("PRACTICE"));
    }
}
