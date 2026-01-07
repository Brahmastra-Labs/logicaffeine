//! Unified navigation component for consistent header across all pages.
//!
//! Features:
//! - Logo and brand name
//! - Navigation links with active underline indicator
//! - GitHub icon and CTA buttons
//! - Responsive design with hamburger menu and slide-out drawer on mobile

use dioxus::prelude::*;
use crate::ui::router::Route;
use super::hamburger_menu::{HamburgerMenu, HAMBURGER_MENU_STYLES};
use super::nav_drawer::NavDrawer;

/// Embedded logo SVG
const LOGO_SVG: &str = include_str!("../../../assets/logo.svg");

/// Navigation item definition
#[derive(Clone, PartialEq)]
pub struct NavItem {
    pub label: &'static str,
    pub route: Route,
}

/// Which page is currently active
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub enum ActivePage {
    #[default]
    Guide,
    Learn,
    Studio,
    Roadmap,
    Pricing,
    Registry,
    Profile,
    Other,
}

impl ActivePage {
    /// Determine the active page from a Route
    pub fn from_route(route: &Route) -> Self {
        match route {
            Route::Landing {} => ActivePage::Other,
            Route::Guide {} => ActivePage::Guide,
            Route::Learn {} => ActivePage::Learn,
            Route::Studio {} => ActivePage::Studio,
            Route::Workspace { .. } => ActivePage::Studio,
            Route::Roadmap {} => ActivePage::Roadmap,
            Route::Pricing {} => ActivePage::Pricing,
            Route::Success {} => ActivePage::Pricing,
            Route::Registry {} => ActivePage::Registry,
            Route::PackageDetail { .. } => ActivePage::Registry,
            Route::Profile {} => ActivePage::Profile,
            _ => ActivePage::Other,
        }
    }
}

const MAIN_NAV_STYLE: &str = r#"
.main-nav {
    position: sticky;
    top: 0;
    z-index: 50;
    backdrop-filter: blur(18px);
    background: linear-gradient(180deg, rgba(7,10,18,0.72), rgba(7,10,18,0.44));
    border-bottom: 1px solid rgba(255,255,255,0.06);
}

.main-nav-inner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--spacing-lg) var(--spacing-xl);
    max-width: 1280px;
    margin: 0 auto;
    gap: var(--spacing-lg);
}

.main-nav-brand {
    display: flex;
    align-items: center;
    gap: var(--spacing-md);
    text-decoration: none;
    color: var(--text-primary);
}

.main-nav-logo {
    width: 64px;
    height: 64px;
    border-radius: var(--radius-lg);
    overflow: hidden;
}

.main-nav-logo svg {
    width: 100%;
    height: 100%;
    filter: invert(1);
}

.main-nav-brand-text {
    display: flex;
    flex-direction: column;
    line-height: 1.05;
}

.main-nav-brand-name {
    font-weight: 800;
    letter-spacing: -0.5px;
    font-size: var(--font-body-md);
}

.main-nav-brand-subtitle {
    font-size: var(--font-caption-sm);
    color: var(--text-tertiary);
}

.main-nav-links {
    display: flex;
    gap: var(--spacing-xs);
    align-items: center;
}

.main-nav-link {
    position: relative;
    text-decoration: none;
    padding: 10px 14px;
    font-size: var(--font-body-md);
    font-weight: 500;
    color: var(--text-secondary);
    transition: color 0.18s ease;
    border-radius: var(--radius-md);
}

.main-nav-link:hover {
    color: var(--text-primary);
    background: rgba(255,255,255,0.04);
}

/* Active underline indicator */
.main-nav-link::after {
    content: "";
    position: absolute;
    bottom: 2px;
    left: 14px;
    right: 14px;
    height: 2px;
    background: linear-gradient(90deg, var(--color-accent-blue), var(--color-accent-purple));
    border-radius: 2px;
    opacity: 0;
    transform: scaleX(0);
    transition: opacity 0.18s ease, transform 0.18s ease;
}

.main-nav-link.active {
    color: var(--text-primary);
}

.main-nav-link.active::after {
    opacity: 1;
    transform: scaleX(1);
}

.main-nav-cta {
    display: flex;
    gap: 10px;
    align-items: center;
}

.main-nav-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--spacing-sm);
    padding: 10px var(--spacing-lg);
    border-radius: var(--radius-lg);
    border: 1px solid rgba(255,255,255,0.10);
    background: rgba(255,255,255,0.05);
    text-decoration: none;
    font-weight: 600;
    font-size: var(--font-body-md);
    color: var(--text-primary);
    transition: transform 0.18s ease, background 0.18s ease, border-color 0.18s ease;
}

.main-nav-btn:hover {
    transform: translateY(-1px);
    background: rgba(255,255,255,0.08);
    border-color: rgba(255,255,255,0.18);
}

.main-nav-btn.primary {
    background: linear-gradient(135deg, rgba(96,165,250,0.95), rgba(167,139,250,0.95));
    border-color: rgba(255,255,255,0.20);
    color: #060814;
    box-shadow: 0 12px 30px rgba(96,165,250,0.18);
}

.main-nav-btn.primary:hover {
    background: linear-gradient(135deg, rgba(96,165,250,1.0), rgba(167,139,250,1.0));
}

.main-nav-btn.ghost {
    background: rgba(255,255,255,0.03);
}

.main-nav-btn-icon {
    padding: 10px;
    background: rgba(255,255,255,0.03);
}

.main-nav-btn-icon svg {
    width: 20px;
    height: 20px;
    fill: currentColor;
}

/* Safe area insets for notched devices */
@supports (padding: env(safe-area-inset-top)) {
    .main-nav {
        padding-top: env(safe-area-inset-top);
    }
    .main-nav-inner {
        padding-left: max(var(--spacing-xl), env(safe-area-inset-left));
        padding-right: max(var(--spacing-xl), env(safe-area-inset-right));
    }
}

/* Responsive - tablet breakpoint */
@media (max-width: 980px) {
    .main-nav-brand-text {
        display: none;
    }
}

/* Responsive - mobile breakpoint: hide nav links, show hamburger */
@media (max-width: 640px) {
    .main-nav-inner {
        padding: var(--spacing-md) var(--spacing-lg);
    }
    .main-nav-links {
        display: none;
    }
    .main-nav-cta {
        display: none;
    }
    .main-nav-btn {
        padding: var(--spacing-sm) var(--spacing-md);
        font-size: var(--font-caption-md);
    }
}
"#;

/// Main navigation component with consistent styling and active page underline
#[component]
pub fn MainNav(
    /// The currently active page
    #[props(default)]
    active: ActivePage,
    /// Optional subtitle for the brand (e.g., "Programmer's Guide")
    #[props(default)]
    subtitle: Option<&'static str>,
    /// Whether to show the full nav links (default true)
    #[props(default = true)]
    show_nav_links: bool,
) -> Element {
    // State for mobile nav drawer
    let mut nav_drawer_open = use_signal(|| false);

    // Toggle handler for hamburger menu
    let toggle_drawer = move |_| {
        nav_drawer_open.toggle();
    };

    // Close handler for nav drawer
    let close_drawer = move |_| {
        nav_drawer_open.set(false);
    };

    rsx! {
        style { "{MAIN_NAV_STYLE}" }
        style { "{HAMBURGER_MENU_STYLES}" }

        header { class: "main-nav",
            div { class: "main-nav-inner",
                // Brand
                Link {
                    to: Route::Landing {},
                    class: "main-nav-brand",
                    div {
                        class: "main-nav-logo",
                        dangerous_inner_html: "{LOGO_SVG}"
                    }
                    div { class: "main-nav-brand-text",
                        span { class: "main-nav-brand-name", "LOGICAFFEINE" }
                        if let Some(sub) = subtitle {
                            span { class: "main-nav-brand-subtitle", "{sub}" }
                        } else {
                            span { class: "main-nav-brand-subtitle", "Debug your thoughts." }
                        }
                    }
                }

                // Navigation links with active underline (desktop only)
                if show_nav_links {
                    nav { class: "main-nav-links",
                        Link {
                            to: Route::Guide {},
                            class: if active == ActivePage::Guide { "main-nav-link active" } else { "main-nav-link" },
                            "Guide"
                        }
                        Link {
                            to: Route::Learn {},
                            class: if active == ActivePage::Learn { "main-nav-link active" } else { "main-nav-link" },
                            "Learn"
                        }
                        Link {
                            to: Route::Studio {},
                            class: if active == ActivePage::Studio { "main-nav-link active" } else { "main-nav-link" },
                            "Studio"
                        }
                        Link {
                            to: Route::Roadmap {},
                            class: if active == ActivePage::Roadmap { "main-nav-link active" } else { "main-nav-link" },
                            "Roadmap"
                        }
                        Link {
                            to: Route::Pricing {},
                            class: if active == ActivePage::Pricing { "main-nav-link active" } else { "main-nav-link" },
                            "Pricing"
                        }
                    }
                }

                // CTA buttons (desktop only)
                div { class: "main-nav-cta",
                    // GitHub button
                    a {
                        href: "https://github.com/Brahmastra-Labs/logicaffeine",
                        target: "_blank",
                        class: "main-nav-btn main-nav-btn-icon",
                        title: "View on GitHub",
                        svg {
                            xmlns: "http://www.w3.org/2000/svg",
                            view_box: "0 0 24 24",
                            path {
                                d: "M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0024 12c0-6.63-5.37-12-12-12z"
                            }
                        }
                    }
                    // Profile button
                    Link {
                        to: Route::Profile {},
                        class: if active == ActivePage::Profile { "main-nav-btn main-nav-btn-icon active" } else { "main-nav-btn main-nav-btn-icon" },
                        title: "Your Profile",
                        svg {
                            xmlns: "http://www.w3.org/2000/svg",
                            view_box: "0 0 24 24",
                            path {
                                d: "M12 12c2.21 0 4-1.79 4-4s-1.79-4-4-4-4 1.79-4 4 1.79 4 4 4zm0 2c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z"
                            }
                        }
                    }
                }

                // Hamburger menu (mobile only, shown at ≤640px)
                HamburgerMenu {
                    is_open: nav_drawer_open,
                    on_toggle: toggle_drawer,
                }
            }
        }

        // Mobile navigation drawer
        NavDrawer {
            is_open: nav_drawer_open,
            on_close: close_drawer,
            active: active,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_main_nav_style_has_mobile_breakpoint() {
        assert!(MAIN_NAV_STYLE.contains("@media (max-width: 640px)"));
    }

    #[test]
    fn test_main_nav_hides_nav_links_on_mobile() {
        assert!(MAIN_NAV_STYLE.contains(".main-nav-links {\n        display: none;\n    }"));
    }

    #[test]
    fn test_main_nav_hides_cta_on_mobile() {
        assert!(MAIN_NAV_STYLE.contains(".main-nav-cta {\n        display: none;\n    }"));
    }

    #[test]
    fn test_main_nav_brand_text_hidden_on_tablet() {
        assert!(MAIN_NAV_STYLE.contains("@media (max-width: 980px)"));
        assert!(MAIN_NAV_STYLE.contains(".main-nav-brand-text {\n        display: none;\n    }"));
    }

    #[test]
    fn test_active_page_from_route_landing() {
        assert_eq!(ActivePage::from_route(&Route::Landing {}), ActivePage::Other);
    }

    #[test]
    fn test_active_page_from_route_guide() {
        assert_eq!(ActivePage::from_route(&Route::Guide {}), ActivePage::Guide);
    }

    #[test]
    fn test_active_page_from_route_learn() {
        assert_eq!(ActivePage::from_route(&Route::Learn {}), ActivePage::Learn);
    }

    #[test]
    fn test_active_page_from_route_studio() {
        assert_eq!(ActivePage::from_route(&Route::Studio {}), ActivePage::Studio);
    }

    #[test]
    fn test_active_page_from_route_pricing() {
        assert_eq!(ActivePage::from_route(&Route::Pricing {}), ActivePage::Pricing);
    }

    #[test]
    fn test_active_page_from_route_profile() {
        assert_eq!(ActivePage::from_route(&Route::Profile {}), ActivePage::Profile);
    }
}
