//! Unified navigation component for consistent header across all pages.
//!
//! Features:
//! - Logo and brand name
//! - Navigation links with active underline indicator (desktop)
//! - Mobile drawer with expandable trees for Syntax Guide and Learn
//! - GitHub icon and CTA buttons
//! - Responsive design with 980px breakpoint

use dioxus::prelude::*;
use crate::ui::router::Route;
use crate::ui::components::icon::{Icon, IconVariant, IconSize};
use crate::content::ContentEngine;
use crate::ui::pages::guide::content::SECTIONS;

/// Embedded logo SVG
const LOGO_SVG: &str = include_str!("../../../assets/logo.svg");

/// Which page is currently active
#[derive(Clone, Copy, PartialEq, Default)]
pub enum ActivePage {
    #[default]
    Guide,
    Crates,
    Learn,
    Studio,
    Roadmap,
    Pricing,
    Registry,
    Profile,
    News,
    Other,
}

impl ActivePage {
    /// Determine the active page from a Route
    pub fn from_route(route: &Route) -> Self {
        match route {
            Route::Landing {} => ActivePage::Other,
            Route::Guide {} => ActivePage::Guide,
            Route::Crates {} => ActivePage::Crates,
            Route::Learn {} => ActivePage::Learn,
            Route::Studio {} => ActivePage::Studio,
            Route::Workspace { .. } => ActivePage::Studio,
            Route::Roadmap {} => ActivePage::Roadmap,
            Route::Pricing {} => ActivePage::Pricing,
            Route::Success {} => ActivePage::Pricing,
            Route::Registry {} => ActivePage::Registry,
            Route::PackageDetail { .. } => ActivePage::Registry,
            Route::Profile {} => ActivePage::Profile,
            Route::News {} => ActivePage::News,
            Route::NewsArticle { .. } => ActivePage::News,
            _ => ActivePage::Other,
        }
    }
}

const MAIN_NAV_STYLE: &str = r#"
.main-nav {
    position: sticky;
    top: 0;
    z-index: 150;
    backdrop-filter: blur(18px);
    -webkit-backdrop-filter: blur(18px);
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
    z-index: 1001;
}

.main-nav-logo {
    width: 64px;
    height: 64px;
    border-radius: var(--radius-lg);
    overflow: hidden;
    flex-shrink: 0;
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
    background: linear-gradient(135deg, rgba(0,212,255,0.95), rgba(129,140,248,0.95));
    border-color: rgba(255,255,255,0.20);
    color: #09090b;
    box-shadow: 0 12px 30px rgba(0,212,255,0.18);
}

.main-nav-btn.primary:hover {
    background: linear-gradient(135deg, rgba(0,212,255,1.0), rgba(129,140,248,1.0));
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

/* Mobile hamburger button */
.mobile-menu-btn {
    display: none;
    width: 44px;
    height: 44px;
    border-radius: 5px;
    border: 2px solid rgba(255,255,255,0.2);
    background: rgba(255,255,255,0.05);
    color: var(--text-primary);
    cursor: pointer;
    align-items: center;
    justify-content: center;
    z-index: 1001;
}

.mobile-menu-btn:hover {
    background: rgba(255,255,255,0.1);
    border-color: rgba(255,255,255,0.3);
}

/* Mobile drawer overlay */
.mobile-drawer-overlay {
    display: none;
    position: fixed;
    inset: 0;
    background: rgba(0,0,0,0.6);
    z-index: 999;
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.3s ease;
}

.mobile-drawer-overlay.open {
    opacity: 1;
    pointer-events: auto;
}

/* Mobile drawer */
.mobile-drawer {
    position: fixed;
    top: 0;
    right: 0;
    width: 320px;
    max-width: 85vw;
    height: 100vh;
    background: rgba(12, 16, 24, 0.98);
    backdrop-filter: blur(20px);
    -webkit-backdrop-filter: blur(20px);
    border-left: 1px solid rgba(255,255,255,0.08);
    z-index: 1000;
    transform: translateX(100%);
    transition: transform 0.3s ease;
    overflow-y: auto;
    display: none;
}

.mobile-drawer.open {
    transform: translateX(0);
}

.mobile-drawer-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px var(--spacing-lg) var(--spacing-lg);
    border-bottom: 1px solid rgba(255,255,255,0.06);
}

.mobile-drawer-title {
    font-weight: 700;
    font-size: var(--font-body-lg);
    color: var(--text-primary);
}

.mobile-drawer-close {
    width: 36px;
    height: 36px;
    border-radius: var(--radius-md);
    border: 1px solid rgba(255,255,255,0.1);
    background: rgba(255,255,255,0.03);
    color: var(--text-secondary);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
}

.mobile-drawer-close:hover {
    background: rgba(255,255,255,0.08);
    color: var(--text-primary);
}

.mobile-drawer-content {
    padding: var(--spacing-md);
}

/* Mobile nav section */
.mobile-nav-section {
    margin-bottom: var(--spacing-md);
}

.mobile-nav-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--spacing-md);
    border-radius: var(--radius-md);
    cursor: pointer;
    transition: background 0.15s ease;
}

.mobile-nav-header:hover {
    background: rgba(255,255,255,0.04);
}

.mobile-nav-header-left {
    display: flex;
    align-items: center;
    gap: var(--spacing-sm);
}

.mobile-nav-header-icon {
    color: var(--color-accent-blue);
}

.mobile-nav-header-title {
    font-weight: 600;
    font-size: var(--font-body-md);
    color: var(--text-primary);
}

.mobile-nav-header-chevron {
    color: var(--text-tertiary);
    transition: transform 0.2s ease;
}

.mobile-nav-header-chevron.expanded {
    transform: rotate(90deg);
}

.mobile-nav-tree {
    max-height: 0;
    overflow: hidden;
    transition: max-height 0.3s ease;
}

.mobile-nav-tree.expanded {
    max-height: 2000px;
}

.mobile-nav-tree-inner {
    padding: var(--spacing-xs) 0 var(--spacing-sm) var(--spacing-xl);
    border-left: 2px solid rgba(255,255,255,0.06);
    margin-left: var(--spacing-lg);
}

.mobile-nav-item {
    display: block;
    padding: var(--spacing-sm) var(--spacing-md);
    border-radius: var(--radius-sm);
    color: var(--text-secondary);
    font-size: var(--font-body-sm);
    text-decoration: none;
    transition: all 0.15s ease;
}

.mobile-nav-item:hover {
    background: rgba(255,255,255,0.04);
    color: var(--text-primary);
}

.mobile-nav-item.active {
    background: rgba(0,212,255,0.12);
    color: #00d4ff;
}

/* Simple mobile link (no tree) */
.mobile-nav-link {
    display: flex;
    align-items: center;
    gap: var(--spacing-sm);
    padding: var(--spacing-md);
    border-radius: var(--radius-md);
    color: var(--text-secondary);
    text-decoration: none;
    transition: all 0.15s ease;
}

.mobile-nav-link:hover {
    background: rgba(255,255,255,0.04);
    color: var(--text-primary);
}

.mobile-nav-link.active {
    background: rgba(0,212,255,0.12);
    color: #00d4ff;
}

.mobile-nav-link-icon {
    color: var(--color-accent-blue);
}

.mobile-nav-divider {
    height: 1px;
    background: rgba(255,255,255,0.06);
    margin: var(--spacing-md) 0;
}

/* Responsive */
@media (max-width: 980px) {
    .main-nav-links {
        display: none;
    }
    .main-nav-brand-text {
        display: none;
    }
    .mobile-menu-btn {
        display: flex;
    }
    .mobile-drawer {
        display: block;
    }
    .mobile-drawer-overlay {
        display: block;
    }
}

@media (max-width: 640px) {
    .main-nav-inner {
        padding: var(--spacing-md) var(--spacing-lg);
    }
    .main-nav-btn {
        padding: var(--spacing-sm) var(--spacing-md);
        font-size: var(--font-caption-md);
    }
    .main-nav-logo {
        width: 48px;
        height: 48px;
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
    let mut drawer_open = use_signal(|| false);
    let mut guide_expanded = use_signal(|| false);
    let mut learn_expanded = use_signal(|| false);

    // Get content for trees
    let content_engine = ContentEngine::new();
    let eras = content_engine.eras();

    // Get guide sections grouped by part
    let guide_parts: Vec<(&str, Vec<&crate::ui::pages::guide::content::Section>)> = {
        let mut parts: Vec<(&str, Vec<&crate::ui::pages::guide::content::Section>)> = Vec::new();
        for section in SECTIONS {
            if let Some(part) = parts.iter_mut().find(|(p, _)| *p == section.part) {
                part.1.push(section);
            } else {
                parts.push((section.part, vec![section]));
            }
        }
        parts
    };

    rsx! {
        style { "{MAIN_NAV_STYLE}" }

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

                // Desktop navigation links with active underline
                if show_nav_links {
                    nav { class: "main-nav-links",
                        Link {
                            to: Route::Guide {},
                            class: if active == ActivePage::Guide { "main-nav-link active" } else { "main-nav-link" },
                            "Syntax Guide"
                        }
                        Link {
                            to: Route::Crates {},
                            class: if active == ActivePage::Crates { "main-nav-link active" } else { "main-nav-link" },
                            "Crates"
                        }
                        Link {
                            to: Route::Learn {},
                            class: if active == ActivePage::Learn { "main-nav-link active" } else { "main-nav-link" },
                            "Learn Logic"
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

                // CTA buttons
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
                    // Mobile menu button
                    button {
                        class: "mobile-menu-btn",
                        onclick: move |_| drawer_open.set(true),
                        Icon { variant: IconVariant::Menu, size: IconSize::Medium }
                    }
                }
            }
        }

        // Mobile drawer overlay
        div {
            class: if *drawer_open.read() { "mobile-drawer-overlay open" } else { "mobile-drawer-overlay" },
            onclick: move |_| drawer_open.set(false),
        }

        // Mobile drawer
        div {
            class: if *drawer_open.read() { "mobile-drawer open" } else { "mobile-drawer" },

            // Header
            div { class: "mobile-drawer-header",
                span { class: "mobile-drawer-title", "Navigation" }
                button {
                    class: "mobile-drawer-close",
                    onclick: move |_| drawer_open.set(false),
                    Icon { variant: IconVariant::Close, size: IconSize::Medium }
                }
            }

            // Content
            div { class: "mobile-drawer-content",
                // Syntax Guide section with tree
                div { class: "mobile-nav-section",
                    div {
                        class: "mobile-nav-header",
                        onclick: move |_| {
                            let current = *guide_expanded.read();
                            guide_expanded.set(!current);
                        },
                        div { class: "mobile-nav-header-left",
                            span { class: "mobile-nav-header-icon",
                                Icon { variant: IconVariant::Book, size: IconSize::Medium }
                            }
                            span { class: "mobile-nav-header-title", "Syntax Guide" }
                        }
                        span {
                            class: if *guide_expanded.read() { "mobile-nav-header-chevron expanded" } else { "mobile-nav-header-chevron" },
                            Icon { variant: IconVariant::ChevronRight, size: IconSize::Small }
                        }
                    }
                    div {
                        class: if *guide_expanded.read() { "mobile-nav-tree expanded" } else { "mobile-nav-tree" },
                        div { class: "mobile-nav-tree-inner",
                            for (part_name, sections) in guide_parts.iter() {
                                div { style: "margin-bottom: var(--spacing-sm);",
                                    div {
                                        style: "font-size: 10px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.5px; color: var(--text-tertiary); padding: var(--spacing-xs) var(--spacing-md); margin-bottom: var(--spacing-xs);",
                                        "{part_name}"
                                    }
                                    for section in sections.iter() {
                                        a {
                                            href: "/guide#{section.id}",
                                            class: "mobile-nav-item",
                                            onclick: move |_| drawer_open.set(false),
                                            "{section.number}. {section.title}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Learn section with tree
                div { class: "mobile-nav-section",
                    div {
                        class: "mobile-nav-header",
                        onclick: move |_| {
                            let current = *learn_expanded.read();
                            learn_expanded.set(!current);
                        },
                        div { class: "mobile-nav-header-left",
                            span { class: "mobile-nav-header-icon",
                                Icon { variant: IconVariant::GraduationCap, size: IconSize::Medium }
                            }
                            span { class: "mobile-nav-header-title", "Learn Logic" }
                        }
                        span {
                            class: if *learn_expanded.read() { "mobile-nav-header-chevron expanded" } else { "mobile-nav-header-chevron" },
                            Icon { variant: IconVariant::ChevronRight, size: IconSize::Small }
                        }
                    }
                    div {
                        class: if *learn_expanded.read() { "mobile-nav-tree expanded" } else { "mobile-nav-tree" },
                        div { class: "mobile-nav-tree-inner",
                            for era in eras.iter() {
                                div { style: "margin-bottom: var(--spacing-sm);",
                                    div {
                                        style: "font-size: 10px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.5px; color: var(--text-tertiary); padding: var(--spacing-xs) var(--spacing-md); margin-bottom: var(--spacing-xs);",
                                        "{era.meta.title}"
                                    }
                                    for module in era.modules.iter() {
                                        a {
                                            href: "/learn#{module.meta.id}",
                                            class: "mobile-nav-item",
                                            onclick: move |_| drawer_open.set(false),
                                            "{module.meta.title}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                div { class: "mobile-nav-divider" }

                // Simple links for other pages
                Link {
                    to: Route::Studio {},
                    class: if active == ActivePage::Studio { "mobile-nav-link active" } else { "mobile-nav-link" },
                    onclick: move |_| drawer_open.set(false),
                    span { class: "mobile-nav-link-icon",
                        Icon { variant: IconVariant::Beaker, size: IconSize::Medium }
                    }
                    "Studio"
                }
                Link {
                    to: Route::Crates {},
                    class: if active == ActivePage::Crates { "mobile-nav-link active" } else { "mobile-nav-link" },
                    onclick: move |_| drawer_open.set(false),
                    span { class: "mobile-nav-link-icon",
                        Icon { variant: IconVariant::Package, size: IconSize::Medium }
                    }
                    "Crates"
                }
                Link {
                    to: Route::Roadmap {},
                    class: if active == ActivePage::Roadmap { "mobile-nav-link active" } else { "mobile-nav-link" },
                    onclick: move |_| drawer_open.set(false),
                    span { class: "mobile-nav-link-icon",
                        Icon { variant: IconVariant::Map, size: IconSize::Medium }
                    }
                    "Roadmap"
                }
                Link {
                    to: Route::Pricing {},
                    class: if active == ActivePage::Pricing { "mobile-nav-link active" } else { "mobile-nav-link" },
                    onclick: move |_| drawer_open.set(false),
                    span { class: "mobile-nav-link-icon",
                        Icon { variant: IconVariant::Diamond, size: IconSize::Medium }
                    }
                    "Pricing"
                }

                div { class: "mobile-nav-divider" }

                Link {
                    to: Route::Profile {},
                    class: if active == ActivePage::Profile { "mobile-nav-link active" } else { "mobile-nav-link" },
                    onclick: move |_| drawer_open.set(false),
                    span { class: "mobile-nav-link-icon",
                        Icon { variant: IconVariant::User, size: IconSize::Medium }
                    }
                    "Profile"
                }
                a {
                    href: "https://github.com/Brahmastra-Labs/logicaffeine",
                    target: "_blank",
                    class: "mobile-nav-link",
                    span { class: "mobile-nav-link-icon",
                        Icon { variant: IconVariant::Github, size: IconSize::Medium }
                    }
                    "GitHub"
                }
            }
        }
    }
}
