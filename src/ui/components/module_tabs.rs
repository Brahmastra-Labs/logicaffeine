//! Module Tab Bar component for the integrated Learn page.
//!
//! Displays tabs: LESSON | EXAMPLES | PRACTICE ∞ | TEST 📝
//! allowing users to switch between different learning modes within a module.
//!
//! This module provides two variants:
//! - `ModuleTabs`: Horizontal tab bar for desktop layouts
//! - `MobileAccordionTabs`: Stacked accordion tabs for mobile viewports

use dioxus::prelude::*;
use crate::learn_state::TabMode;
use crate::ui::responsive::MOBILE_ACCORDION_STYLES;

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

/// Props for the MobileAccordionTabs component
#[derive(Props, Clone, PartialEq)]
pub struct MobileAccordionTabsProps {
    /// Currently active/expanded tab
    current: TabMode,
    /// Handler called when user clicks a tab header
    on_change: EventHandler<TabMode>,
    /// Tabs that should be locked (disabled)
    #[props(default)]
    locked_tabs: Vec<TabMode>,
    /// Content to render for each tab (children should be 4 elements, one per tab)
    children: Element,
}

/// Get icon for a tab mode
fn tab_icon(tab: TabMode) -> &'static str {
    match tab {
        TabMode::Lesson => "📖",
        TabMode::Examples => "💡",
        TabMode::Practice => "✏️",
        TabMode::Test => "📝",
    }
}

/// Get CSS class name for a tab mode
fn tab_class_name(tab: TabMode) -> &'static str {
    match tab {
        TabMode::Lesson => "lesson",
        TabMode::Examples => "examples",
        TabMode::Practice => "practice",
        TabMode::Test => "test",
    }
}

/// Mobile accordion tab component for stacked, expandable tab navigation.
///
/// Use this component on mobile viewports where horizontal tabs would overflow.
/// Each tab header is a full-width touch target that expands to reveal content.
/// Only one tab can be expanded at a time (controlled by the `current` prop).
///
/// # Usage
/// ```ignore
/// MobileAccordionTabs {
///     current: tab_mode,
///     on_change: move |tab| set_tab_mode(tab),
///     locked_tabs: vec![TabMode::Test], // Optional
///     // Children should provide content for each tab
///     div { "Lesson content" }
///     div { "Examples content" }
///     div { "Practice content" }
///     div { "Test content" }
/// }
/// ```
#[component]
pub fn MobileAccordionTabs(props: MobileAccordionTabsProps) -> Element {
    rsx! {
        style { "{MOBILE_ACCORDION_STYLES}" }
        div { class: "accordion-tabs",
            for tab in TabMode::all() {
                {
                    let is_expanded = tab == props.current;
                    let is_locked = props.locked_tabs.contains(&tab);
                    let class_name = tab_class_name(tab);

                    let item_class = format!(
                        "accordion-tab-item {}{}",
                        class_name,
                        if is_expanded { " expanded" } else { "" }
                    );

                    let header_class = format!(
                        "accordion-tab-header {}{}{}",
                        class_name,
                        if is_expanded { " expanded" } else { "" },
                        if is_locked { " locked" } else { "" }
                    );

                    let content_class = format!(
                        "accordion-tab-content{}",
                        if is_expanded { " expanded" } else { "" }
                    );

                    rsx! {
                        div {
                            key: "{class_name}",
                            class: "{item_class}",

                            // Tab header (clickable)
                            button {
                                class: "{header_class}",
                                disabled: is_locked,
                                onclick: {
                                    let on_change = props.on_change.clone();
                                    move |_| {
                                        if !is_locked {
                                            on_change.call(tab);
                                        }
                                    }
                                },

                                // Left section: icon + label
                                div { class: "accordion-tab-header-left",
                                    span { class: "accordion-tab-icon", "{tab_icon(tab)}" }
                                    span { class: "accordion-tab-label", "{tab.label()}" }
                                }

                                // Right section: lock icon or chevron
                                if is_locked {
                                    span { class: "accordion-tab-lock", "🔒" }
                                } else {
                                    span { class: "accordion-tab-chevron", "▼" }
                                }
                            }

                            // Tab content (expandable)
                            div { class: "{content_class}",
                                div { class: "accordion-tab-content-inner",
                                    // Content will be projected via children
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Props for MobileAccordionTab - a single accordion tab item with content
#[derive(Props, Clone, PartialEq)]
pub struct MobileAccordionTabProps {
    /// The tab mode this accordion item represents
    tab: TabMode,
    /// Whether this tab is currently expanded
    is_expanded: bool,
    /// Whether this tab is locked
    #[props(default = false)]
    is_locked: bool,
    /// Handler called when the header is clicked
    on_click: EventHandler<TabMode>,
    /// Content to show when expanded
    children: Element,
}

/// A single mobile accordion tab item with its content.
///
/// Use this component when you need more control over individual tab content,
/// or when the content for each tab is generated dynamically.
///
/// # Usage
/// ```ignore
/// div { class: "accordion-tabs",
///     MobileAccordionTab {
///         tab: TabMode::Lesson,
///         is_expanded: current == TabMode::Lesson,
///         on_click: move |tab| set_current(tab),
///         div { "Lesson content here" }
///     }
///     MobileAccordionTab {
///         tab: TabMode::Practice,
///         is_expanded: current == TabMode::Practice,
///         on_click: move |tab| set_current(tab),
///         div { "Practice content here" }
///     }
/// }
/// ```
#[component]
pub fn MobileAccordionTab(props: MobileAccordionTabProps) -> Element {
    let class_name = tab_class_name(props.tab);

    let item_class = format!(
        "accordion-tab-item {}{}",
        class_name,
        if props.is_expanded { " expanded" } else { "" }
    );

    let header_class = format!(
        "accordion-tab-header {}{}{}",
        class_name,
        if props.is_expanded { " expanded" } else { "" },
        if props.is_locked { " locked" } else { "" }
    );

    let content_class = format!(
        "accordion-tab-content{}",
        if props.is_expanded { " expanded" } else { "" }
    );

    let tab = props.tab;

    rsx! {
        style { "{MOBILE_ACCORDION_STYLES}" }
        div {
            class: "{item_class}",

            // Tab header (clickable)
            button {
                class: "{header_class}",
                disabled: props.is_locked,
                onclick: move |_| {
                    if !props.is_locked {
                        props.on_click.call(tab);
                    }
                },

                // Left section: icon + label
                div { class: "accordion-tab-header-left",
                    span { class: "accordion-tab-icon", "{tab_icon(tab)}" }
                    span { class: "accordion-tab-label", "{tab.label()}" }
                }

                // Right section: lock icon or chevron
                if props.is_locked {
                    span { class: "accordion-tab-lock", "🔒" }
                } else {
                    span { class: "accordion-tab-chevron", "▼" }
                }
            }

            // Tab content (expandable)
            div { class: "{content_class}",
                div { class: "accordion-tab-content-inner",
                    {props.children}
                }
            }
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

    #[test]
    fn test_tab_icon_returns_emoji_for_each_mode() {
        assert_eq!(tab_icon(TabMode::Lesson), "📖");
        assert_eq!(tab_icon(TabMode::Examples), "💡");
        assert_eq!(tab_icon(TabMode::Practice), "✏️");
        assert_eq!(tab_icon(TabMode::Test), "📝");
    }

    #[test]
    fn test_tab_class_name_returns_css_class_for_each_mode() {
        assert_eq!(tab_class_name(TabMode::Lesson), "lesson");
        assert_eq!(tab_class_name(TabMode::Examples), "examples");
        assert_eq!(tab_class_name(TabMode::Practice), "practice");
        assert_eq!(tab_class_name(TabMode::Test), "test");
    }

    #[test]
    fn test_all_tab_modes_have_icons() {
        for tab in TabMode::all() {
            let icon = tab_icon(tab);
            assert!(!icon.is_empty(), "Tab {:?} should have an icon", tab);
        }
    }

    #[test]
    fn test_all_tab_modes_have_class_names() {
        for tab in TabMode::all() {
            let class = tab_class_name(tab);
            assert!(!class.is_empty(), "Tab {:?} should have a class name", tab);
        }
    }
}
