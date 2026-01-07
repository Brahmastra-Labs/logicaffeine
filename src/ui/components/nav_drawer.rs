//! Mobile navigation drawer component.
//!
//! A slide-out navigation panel for mobile devices that provides:
//! - Full-height overlay from the left side
//! - All main navigation links with active state
//! - Close button and click-outside-to-close functionality
//! - Smooth slide animation with reduced motion support

use dioxus::prelude::*;
use crate::ui::router::Route;
use super::main_nav::ActivePage;

/// Navigation drawer styles for mobile slide-out menu
const NAV_DRAWER_STYLES: &str = r#"
/* Drawer overlay backdrop */
.nav-drawer-backdrop {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 100;
    background: rgba(0, 0, 0, 0.6);
    opacity: 0;
    visibility: hidden;
    transition: opacity 0.25s ease, visibility 0.25s ease;
    -webkit-tap-highlight-color: transparent;
}

.nav-drawer-backdrop.open {
    opacity: 1;
    visibility: visible;
}

/* Drawer panel container */
.nav-drawer {
    position: fixed;
    top: 0;
    left: 0;
    bottom: 0;
    z-index: 101;
    width: 280px;
    max-width: 85vw;
    background: linear-gradient(180deg, rgba(12, 16, 28, 0.98), rgba(7, 10, 18, 0.98));
    border-right: 1px solid rgba(255, 255, 255, 0.08);
    transform: translateX(-100%);
    transition: transform 0.3s cubic-bezier(0.4, 0, 0.2, 1);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    box-shadow: 4px 0 24px rgba(0, 0, 0, 0.4);
}

.nav-drawer.open {
    transform: translateX(0);
}

/* Drawer header with brand and close button */
.nav-drawer-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 20px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    background: rgba(0, 0, 0, 0.2);
    flex-shrink: 0;
}

.nav-drawer-brand {
    display: flex;
    align-items: center;
    gap: 12px;
    text-decoration: none;
    color: var(--text-primary, #e8e8e8);
}

.nav-drawer-logo {
    width: 40px;
    height: 40px;
    border-radius: var(--radius-md, 8px);
    overflow: hidden;
}

.nav-drawer-logo svg {
    width: 100%;
    height: 100%;
    filter: invert(1);
}

.nav-drawer-brand-text {
    display: flex;
    flex-direction: column;
    line-height: 1.1;
}

.nav-drawer-brand-name {
    font-weight: 700;
    font-size: 14px;
    letter-spacing: -0.3px;
}

.nav-drawer-brand-subtitle {
    font-size: 11px;
    color: var(--text-tertiary, #666);
}

/* Close button */
.nav-drawer-close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 44px;
    height: 44px;
    border: none;
    border-radius: var(--radius-md, 8px);
    background: rgba(255, 255, 255, 0.05);
    color: var(--text-secondary, #888);
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
    touch-action: manipulation;
    transition: background 0.15s ease, color 0.15s ease;
}

.nav-drawer-close:hover,
.nav-drawer-close:active {
    background: rgba(255, 255, 255, 0.1);
    color: var(--text-primary, #e8e8e8);
}

.nav-drawer-close svg {
    width: 20px;
    height: 20px;
}

/* Navigation links container */
.nav-drawer-links {
    flex: 1;
    overflow-y: auto;
    -webkit-overflow-scrolling: touch;
    padding: 16px 12px;
    display: flex;
    flex-direction: column;
    gap: 4px;
}

/* Individual navigation link */
.nav-drawer-link {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 14px 16px;
    border-radius: var(--radius-md, 8px);
    text-decoration: none;
    color: var(--text-secondary, #888);
    font-size: 15px;
    font-weight: 500;
    min-height: 48px;
    transition: background 0.15s ease, color 0.15s ease;
    -webkit-tap-highlight-color: transparent;
    touch-action: manipulation;
}

.nav-drawer-link:active {
    background: rgba(255, 255, 255, 0.08);
}

.nav-drawer-link:hover {
    background: rgba(255, 255, 255, 0.06);
    color: var(--text-primary, #e8e8e8);
}

.nav-drawer-link.active {
    background: linear-gradient(135deg, rgba(96, 165, 250, 0.15), rgba(167, 139, 250, 0.15));
    color: var(--text-primary, #e8e8e8);
    border-left: 3px solid;
    border-image: linear-gradient(180deg, #60a5fa, #a78bfa) 1;
}

.nav-drawer-link-icon {
    width: 20px;
    height: 20px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
}

.nav-drawer-link-icon svg {
    width: 20px;
    height: 20px;
    stroke: currentColor;
}

/* Drawer footer with secondary actions */
.nav-drawer-footer {
    padding: 16px 12px;
    border-top: 1px solid rgba(255, 255, 255, 0.08);
    background: rgba(0, 0, 0, 0.15);
    display: flex;
    flex-direction: column;
    gap: 8px;
    flex-shrink: 0;
}

.nav-drawer-footer-link {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 16px;
    border-radius: var(--radius-md, 8px);
    text-decoration: none;
    color: var(--text-tertiary, #666);
    font-size: 13px;
    font-weight: 500;
    min-height: 44px;
    transition: background 0.15s ease, color 0.15s ease;
    -webkit-tap-highlight-color: transparent;
}

.nav-drawer-footer-link:hover,
.nav-drawer-footer-link:active {
    background: rgba(255, 255, 255, 0.05);
    color: var(--text-secondary, #888);
}

.nav-drawer-footer-link svg {
    width: 18px;
    height: 18px;
    fill: currentColor;
}

/* Separator */
.nav-drawer-separator {
    height: 1px;
    background: rgba(255, 255, 255, 0.06);
    margin: 8px 0;
}

/* Reduced motion support */
@media (prefers-reduced-motion: reduce) {
    .nav-drawer-backdrop {
        transition: none;
    }

    .nav-drawer {
        transition: none;
    }
}

/* Safe area support for notched devices */
@supports (padding: env(safe-area-inset-left)) {
    .nav-drawer {
        padding-left: env(safe-area-inset-left);
    }

    .nav-drawer-header {
        padding-top: max(16px, env(safe-area-inset-top));
    }

    .nav-drawer-footer {
        padding-bottom: max(16px, env(safe-area-inset-bottom));
    }
}
"#;

/// Embedded logo SVG (same as main_nav)
const LOGO_SVG: &str = include_str!("../../../assets/logo.svg");

/// Navigation drawer item for rendering
#[derive(Clone)]
struct NavDrawerItem {
    label: &'static str,
    route: Route,
    page: ActivePage,
}

/// Get all navigation items for the drawer
fn get_nav_items() -> Vec<NavDrawerItem> {
    vec![
        NavDrawerItem {
            label: "Guide",
            route: Route::Guide {},
            page: ActivePage::Guide,
        },
        NavDrawerItem {
            label: "Learn",
            route: Route::Learn {},
            page: ActivePage::Learn,
        },
        NavDrawerItem {
            label: "Studio",
            route: Route::Studio {},
            page: ActivePage::Studio,
        },
        NavDrawerItem {
            label: "Roadmap",
            route: Route::Roadmap {},
            page: ActivePage::Roadmap,
        },
        NavDrawerItem {
            label: "Pricing",
            route: Route::Pricing {},
            page: ActivePage::Pricing,
        },
        NavDrawerItem {
            label: "Profile",
            route: Route::Profile {},
            page: ActivePage::Profile,
        },
    ]
}

/// Render SVG icon for a navigation page
fn nav_icon(page: ActivePage) -> Element {
    match page {
        ActivePage::Guide => rsx! {
            svg {
                xmlns: "http://www.w3.org/2000/svg",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "2",
                stroke_linecap: "round",
                stroke_linejoin: "round",
                // Book icon
                path { d: "M4 19.5A2.5 2.5 0 0 1 6.5 17H20" }
                path { d: "M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z" }
            }
        },
        ActivePage::Learn => rsx! {
            svg {
                xmlns: "http://www.w3.org/2000/svg",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "2",
                stroke_linecap: "round",
                stroke_linejoin: "round",
                // Graduation cap icon
                path { d: "M22 10v6M2 10l10-5 10 5-10 5z" }
                path { d: "M6 12v5c3 3 9 3 12 0v-5" }
            }
        },
        ActivePage::Studio => rsx! {
            svg {
                xmlns: "http://www.w3.org/2000/svg",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "2",
                stroke_linecap: "round",
                stroke_linejoin: "round",
                // Terminal/code icon
                polyline { points: "4 17 10 11 4 5" }
                line { x1: "12", y1: "19", x2: "20", y2: "19" }
            }
        },
        ActivePage::Roadmap => rsx! {
            svg {
                xmlns: "http://www.w3.org/2000/svg",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "2",
                stroke_linecap: "round",
                stroke_linejoin: "round",
                // Map icon
                polygon { points: "1 6 1 22 8 18 16 22 23 18 23 2 16 6 8 2 1 6" }
                line { x1: "8", y1: "2", x2: "8", y2: "18" }
                line { x1: "16", y1: "6", x2: "16", y2: "22" }
            }
        },
        ActivePage::Pricing => rsx! {
            svg {
                xmlns: "http://www.w3.org/2000/svg",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "2",
                stroke_linecap: "round",
                stroke_linejoin: "round",
                // Tag/price icon
                path { d: "M20.59 13.41l-7.17 7.17a2 2 0 0 1-2.83 0L2 12V2h10l8.59 8.59a2 2 0 0 1 0 2.82z" }
                line { x1: "7", y1: "7", x2: "7.01", y2: "7" }
            }
        },
        ActivePage::Profile => rsx! {
            svg {
                xmlns: "http://www.w3.org/2000/svg",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "2",
                stroke_linecap: "round",
                stroke_linejoin: "round",
                // User icon
                path { d: "M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2" }
                circle { cx: "12", cy: "7", r: "4" }
            }
        },
        _ => rsx! {
            svg {
                xmlns: "http://www.w3.org/2000/svg",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "2",
                // Circle placeholder
                circle { cx: "12", cy: "12", r: "10" }
            }
        },
    }
}

/// Mobile navigation drawer component
///
/// A slide-out navigation panel from the left side of the screen.
/// Shows all main navigation links with active state indication.
///
/// # Props
/// - `is_open`: Signal controlling the open/closed state
/// - `on_close`: Event handler called when the drawer should close
/// - `active`: The currently active page for highlighting
#[component]
pub fn NavDrawer(
    /// Signal controlling the open state
    is_open: Signal<bool>,
    /// Called when drawer should close (backdrop click, close button, or link click)
    on_close: EventHandler<()>,
    /// Currently active page
    #[props(default)]
    active: ActivePage,
) -> Element {
    let nav_items = get_nav_items();

    // Handle backdrop click
    let handle_backdrop_click = move |_| {
        on_close.call(());
    };

    // Handle close button click
    let handle_close = move |_| {
        on_close.call(());
    };

    // Handle link click - close drawer after navigation
    let handle_link_click = move |_| {
        on_close.call(());
    };

    let is_open_val = is_open();
    let backdrop_class = if is_open_val { "nav-drawer-backdrop open" } else { "nav-drawer-backdrop" };
    let drawer_class = if is_open_val { "nav-drawer open" } else { "nav-drawer" };

    rsx! {
        style { "{NAV_DRAWER_STYLES}" }

        // Backdrop overlay
        div {
            class: "{backdrop_class}",
            onclick: handle_backdrop_click,
            // Prevent scroll on body when drawer is open
            onmounted: move |_| {
                // Body scroll lock would be handled via JS interop if needed
            }
        }

        // Drawer panel
        nav {
            class: "{drawer_class}",
            role: "dialog",
            aria_modal: "true",
            aria_label: "Navigation menu",

            // Header with brand and close button
            div { class: "nav-drawer-header",
                Link {
                    to: Route::Landing {},
                    class: "nav-drawer-brand",
                    onclick: handle_link_click,
                    div {
                        class: "nav-drawer-logo",
                        dangerous_inner_html: "{LOGO_SVG}"
                    }
                    div { class: "nav-drawer-brand-text",
                        span { class: "nav-drawer-brand-name", "LOGICAFFEINE" }
                        span { class: "nav-drawer-brand-subtitle", "Debug your thoughts." }
                    }
                }

                // Close button
                button {
                    class: "nav-drawer-close",
                    onclick: handle_close,
                    "aria-label": "Close navigation",
                    svg {
                        xmlns: "http://www.w3.org/2000/svg",
                        view_box: "0 0 24 24",
                        fill: "none",
                        stroke: "currentColor",
                        stroke_width: "2",
                        stroke_linecap: "round",
                        stroke_linejoin: "round",
                        line { x1: "18", y1: "6", x2: "6", y2: "18" }
                        line { x1: "6", y1: "6", x2: "18", y2: "18" }
                    }
                }
            }

            // Navigation links
            div { class: "nav-drawer-links",
                for item in nav_items.iter() {
                    Link {
                        to: item.route.clone(),
                        class: if active == item.page { "nav-drawer-link active" } else { "nav-drawer-link" },
                        onclick: handle_link_click,
                        span { class: "nav-drawer-link-icon", {nav_icon(item.page)} }
                        "{item.label}"
                    }
                }
            }

            // Footer with secondary links
            div { class: "nav-drawer-footer",
                div { class: "nav-drawer-separator" }

                // GitHub link
                a {
                    href: "https://github.com/Brahmastra-Labs/logicaffeine",
                    target: "_blank",
                    class: "nav-drawer-footer-link",
                    svg {
                        xmlns: "http://www.w3.org/2000/svg",
                        view_box: "0 0 24 24",
                        path {
                            d: "M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0024 12c0-6.63-5.37-12-12-12z"
                        }
                    }
                    "View on GitHub"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nav_drawer_styles_contain_required_classes() {
        assert!(NAV_DRAWER_STYLES.contains(".nav-drawer-backdrop"));
        assert!(NAV_DRAWER_STYLES.contains(".nav-drawer"));
        assert!(NAV_DRAWER_STYLES.contains(".nav-drawer-header"));
        assert!(NAV_DRAWER_STYLES.contains(".nav-drawer-links"));
        assert!(NAV_DRAWER_STYLES.contains(".nav-drawer-link"));
        assert!(NAV_DRAWER_STYLES.contains(".nav-drawer-close"));
        assert!(NAV_DRAWER_STYLES.contains(".nav-drawer-footer"));
    }

    #[test]
    fn test_nav_drawer_has_backdrop_overlay() {
        assert!(NAV_DRAWER_STYLES.contains("position: fixed"));
        assert!(NAV_DRAWER_STYLES.contains("z-index: 100"));
        assert!(NAV_DRAWER_STYLES.contains("background: rgba(0, 0, 0, 0.6)"));
    }

    #[test]
    fn test_nav_drawer_slides_from_left() {
        assert!(NAV_DRAWER_STYLES.contains("left: 0"));
        assert!(NAV_DRAWER_STYLES.contains("transform: translateX(-100%)"));
        assert!(NAV_DRAWER_STYLES.contains("transform: translateX(0)"));
    }

    #[test]
    fn test_nav_drawer_width_constraints() {
        assert!(NAV_DRAWER_STYLES.contains("width: 280px"));
        assert!(NAV_DRAWER_STYLES.contains("max-width: 85vw"));
    }

    #[test]
    fn test_nav_drawer_touch_targets_meet_wcag() {
        // Close button should be at least 44x44
        assert!(NAV_DRAWER_STYLES.contains("width: 44px"));
        assert!(NAV_DRAWER_STYLES.contains("height: 44px"));
        // Links should have minimum height
        assert!(NAV_DRAWER_STYLES.contains("min-height: 48px"));
        assert!(NAV_DRAWER_STYLES.contains("min-height: 44px"));
    }

    #[test]
    fn test_nav_drawer_has_reduced_motion_support() {
        assert!(NAV_DRAWER_STYLES.contains("prefers-reduced-motion: reduce"));
        assert!(NAV_DRAWER_STYLES.contains("transition: none"));
    }

    #[test]
    fn test_nav_drawer_has_safe_area_support() {
        assert!(NAV_DRAWER_STYLES.contains("env(safe-area-inset-left)"));
        assert!(NAV_DRAWER_STYLES.contains("env(safe-area-inset-top)"));
        assert!(NAV_DRAWER_STYLES.contains("env(safe-area-inset-bottom)"));
    }

    #[test]
    fn test_nav_drawer_active_link_styling() {
        assert!(NAV_DRAWER_STYLES.contains(".nav-drawer-link.active"));
        assert!(NAV_DRAWER_STYLES.contains("border-left: 3px solid"));
    }

    #[test]
    fn test_nav_drawer_has_smooth_animation() {
        assert!(NAV_DRAWER_STYLES.contains("transition: transform 0.3s cubic-bezier"));
        assert!(NAV_DRAWER_STYLES.contains("transition: opacity 0.25s ease"));
    }

    #[test]
    fn test_nav_drawer_tap_highlight_disabled() {
        assert!(NAV_DRAWER_STYLES.contains("-webkit-tap-highlight-color: transparent"));
    }

    #[test]
    fn test_get_nav_items_returns_all_pages() {
        let items = get_nav_items();

        assert_eq!(items.len(), 6);

        let labels: Vec<&str> = items.iter().map(|i| i.label).collect();
        assert!(labels.contains(&"Guide"));
        assert!(labels.contains(&"Learn"));
        assert!(labels.contains(&"Studio"));
        assert!(labels.contains(&"Roadmap"));
        assert!(labels.contains(&"Pricing"));
        assert!(labels.contains(&"Profile"));
    }

    #[test]
    fn test_nav_drawer_icon_svg_styles() {
        // Verify SVG icon styling in the nav drawer
        assert!(NAV_DRAWER_STYLES.contains(".nav-drawer-link-icon svg"));
        assert!(NAV_DRAWER_STYLES.contains("stroke: currentColor"));
    }

    #[test]
    fn test_nav_drawer_backdrop_visibility_states() {
        // Backdrop should have visibility states for open/closed
        assert!(NAV_DRAWER_STYLES.contains("visibility: hidden"));
        assert!(NAV_DRAWER_STYLES.contains("visibility: visible"));
    }
}
