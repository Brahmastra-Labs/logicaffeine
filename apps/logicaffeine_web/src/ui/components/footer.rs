//! Site-wide footer component.
//!
//! A responsive footer with four sections: Brand, Product, Resources, and Legal.
//! Responsive behavior:
//! - Desktop (1025px+): 4-column layout
//! - Tablet (769-1024px): 2-column layout
//! - Mobile (≤768px): Single column layout

use dioxus::prelude::*;
use crate::ui::components::icon::{Icon, IconVariant, IconSize};
use crate::ui::components::theme_picker::ThemePicker;

/// Embedded logo SVG
const LOGO_SVG: &str = include_str!("../../../assets/logo.svg");

const FOOTER_STYLES: &str = r#"
.site-footer {
    background: linear-gradient(180deg, rgba(6, 8, 20, 0) 0%, rgba(6, 8, 20, 1) 100%);
    border-top: 1px solid rgba(255, 255, 255, 0.06);
    padding: 64px 24px 32px;
    margin-top: auto;
}

.footer-container {
    max-width: 1200px;
    margin: 0 auto;
}

.footer-grid {
    display: grid;
    grid-template-columns: 2fr repeat(3, 1fr);
    gap: 48px;
    margin-bottom: 48px;
}

/* Brand section (first column, wider) */
.footer-brand {
    display: flex;
    flex-direction: column;
    gap: 16px;
}

.footer-brand-content {
    display: flex;
    flex-direction: column;
    gap: 12px;
}

.footer-brand-logo {
    display: flex;
    align-items: center;
    gap: 12px;
    text-decoration: none;
}

.footer-logo-img {
    width: 40px;
    height: 40px;
    border-radius: 8px;
    overflow: hidden;
    flex-shrink: 0;
}

.footer-logo-img svg {
    width: 100%;
    height: 100%;
    filter: invert(1);
}

.footer-logo-text {
    font-size: 18px;
    font-weight: 700;
    color: var(--text-primary, #f0f0f0);
    letter-spacing: -0.02em;
}

.footer-brand-tagline {
    color: var(--text-tertiary, #909090);
    font-size: 14px;
    line-height: 1.6;
    max-width: 280px;
}

.footer-social-links {
    display: flex;
    gap: 12px;
    margin-top: 8px;
}

.footer-social-link {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    border-radius: 8px;
    background: rgba(255, 255, 255, 0.05);
    color: var(--text-secondary, #b0b0b0);
    text-decoration: none;
    transition: all 0.2s ease;
    font-size: 18px;
}

.footer-social-link:hover {
    background: rgba(0, 212, 255, 0.15);
    color: var(--text-primary, #f0f0f0);
}

/* Footer sections (Product, Resources, Legal) */
.footer-section {
    display: flex;
    flex-direction: column;
    gap: 16px;
}

.footer-section-title {
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.1em;
    color: var(--text-secondary, #b0b0b0);
    margin-bottom: 4px;
}

.footer-section-links {
    display: flex;
    flex-direction: column;
    gap: 12px;
}

.footer-link {
    color: var(--text-tertiary, #909090);
    text-decoration: none;
    font-size: 14px;
    transition: color 0.2s ease;
    display: flex;
    align-items: center;
    gap: 6px;
}

.footer-link:hover {
    color: var(--text-primary, #f0f0f0);
}

.footer-link-badge {
    font-size: 10px;
    padding: 2px 6px;
    border-radius: 4px;
    background: rgba(0, 212, 255, 0.15);
    color: var(--color-accent-blue, #60a5fa);
    text-transform: uppercase;
    font-weight: 600;
}

/* Footer bottom bar */
.footer-bottom {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding-top: 24px;
    border-top: 1px solid rgba(255, 255, 255, 0.06);
}

.footer-copyright {
    color: var(--text-tertiary, #909090);
    font-size: 13px;
}

.footer-bottom-links {
    display: flex;
    gap: 24px;
}

.footer-bottom-link {
    color: var(--text-tertiary, #909090);
    text-decoration: none;
    font-size: 13px;
    transition: color 0.2s ease;
}

.footer-bottom-link:hover {
    color: var(--text-primary, #f0f0f0);
}

.footer-bottom-right {
    display: flex;
    align-items: center;
    gap: 24px;
}

.footer-theme-section {
    display: flex;
    align-items: center;
    gap: 8px;
}

.footer-theme-label {
    font-size: 12px;
    color: var(--text-tertiary, #909090);
    font-weight: 500;
}

/* Tablet: 2-column layout */
@media (min-width: 769px) and (max-width: 1024px) {
    .footer-grid {
        grid-template-columns: repeat(2, 1fr);
        gap: 40px;
    }

    .footer-brand {
        grid-column: span 2;
        flex-direction: row;
        justify-content: space-between;
        align-items: flex-start;
        padding-bottom: 24px;
        border-bottom: 1px solid rgba(255, 255, 255, 0.06);
        margin-bottom: 8px;
    }

    .footer-brand-content {
        display: flex;
        flex-direction: column;
        gap: 12px;
    }

    .footer-brand-tagline {
        max-width: 320px;
    }
}

/* Mobile: single column */
@media (max-width: 768px) {
    .site-footer {
        padding: 48px 16px 24px;
    }

    .footer-grid {
        grid-template-columns: 1fr;
        gap: 32px;
    }

    .footer-brand {
        text-align: center;
        align-items: center;
    }

    .footer-brand-tagline {
        max-width: 100%;
    }

    .footer-social-links {
        justify-content: center;
    }

    .footer-section {
        align-items: center;
        text-align: center;
    }

    .footer-section-links {
        align-items: center;
    }

    .footer-bottom {
        flex-direction: column;
        gap: 16px;
        text-align: center;
    }

    .footer-bottom-links {
        flex-wrap: wrap;
        justify-content: center;
        gap: 16px;
    }

    .footer-bottom-right {
        flex-direction: column;
        gap: 16px;
    }

    .footer-theme-section {
        justify-content: center;
    }
}
"#;

/// Footer variant for minimal pages (Privacy, Terms)
#[derive(Clone, Copy, PartialEq, Default)]
pub enum FooterVariant {
    #[default]
    Full,
    Minimal,
}

#[component]
pub fn Footer(
    #[props(default)]
    variant: FooterVariant,
) -> Element {
    let current_year = 2026; // Static for now, could use chrono

    if variant == FooterVariant::Minimal {
        return rsx! {
            style { "{FOOTER_STYLES}" }
            footer { class: "site-footer",
                div { class: "footer-container",
                    div { class: "footer-bottom",
                        span { class: "footer-copyright",
                            "© {current_year} Brahmastra Labs. All rights reserved."
                        }
                        div { class: "footer-bottom-links",
                            Link { to: "/privacy", class: "footer-bottom-link", "Privacy Policy" }
                            Link { to: "/terms", class: "footer-bottom-link", "Terms of Service" }
                        }
                    }
                }
            }
        };
    }

    rsx! {
        style { "{FOOTER_STYLES}" }
        footer { class: "site-footer",
            div { class: "footer-container",
                div { class: "footer-grid",
                    // Brand section
                    div { class: "footer-brand",
                        div { class: "footer-brand-content",
                            Link { to: "/", class: "footer-brand-logo",
                                div {
                                    class: "footer-logo-img",
                                    dangerous_inner_html: "{LOGO_SVG}"
                                }
                                span { class: "footer-logo-text", "LOGICAFFEINE" }
                            }
                            p { class: "footer-brand-tagline",
                                "Debug your thoughts."
                            }
                        }
                        div { class: "footer-social-links",
                            a {
                                href: "https://github.com/Brahmastra-Labs/logicaffeine",
                                target: "_blank",
                                rel: "noopener noreferrer",
                                class: "footer-social-link",
                                title: "GitHub",
                                Icon { variant: IconVariant::Github, size: IconSize::Medium }
                            }
                            a {
                                href: "https://x.com/logicaffeine",
                                target: "_blank",
                                rel: "noopener noreferrer",
                                class: "footer-social-link",
                                title: "X",
                                "𝕏"
                            }
                        }
                    }

                    // Product section
                    div { class: "footer-section",
                        h4 { class: "footer-section-title", "Product" }
                        nav { class: "footer-section-links",
                            Link { to: "/studio", class: "footer-link", "Studio" }
                            Link { to: "/learn", class: "footer-link", "Learn" }
                            Link { to: "/guide", class: "footer-link", "Documentation" }
                            Link { to: "/crates", class: "footer-link", "Crates" }
                        }
                    }

                    // Resources section
                    div { class: "footer-section",
                        h4 { class: "footer-section-title", "Resources" }
                        nav { class: "footer-section-links",
                            Link { to: "/roadmap", class: "footer-link", "Roadmap" }
                            Link { to: "/pricing", class: "footer-link", "Pricing" }
                            a {
                                href: "https://github.com/Brahmastra-Labs/logicaffeine",
                                target: "_blank",
                                rel: "noopener noreferrer",
                                class: "footer-link",
                                "GitHub"
                            }
                        }
                    }

                    // Community section
                    div { class: "footer-section",
                        h4 { class: "footer-section-title", "Community" }
                        nav { class: "footer-section-links",
                            a {
                                href: "https://discord.gg/pwnjnXvUHH",
                                target: "_blank",
                                rel: "noopener noreferrer",
                                class: "footer-link",
                                "Discord"
                            }
                        }
                    }
                }

                // Bottom bar
                div { class: "footer-bottom",
                    span { class: "footer-copyright",
                        "© {current_year} Brahmastra Labs. All rights reserved."
                    }
                    div { class: "footer-bottom-right",
                        div { class: "footer-theme-section",
                            span { class: "footer-theme-label", "Theme:" }
                            ThemePicker {}
                        }
                        div { class: "footer-bottom-links",
                            Link { to: "/privacy", class: "footer-bottom-link", "Privacy" }
                            Link { to: "/terms", class: "footer-bottom-link", "Terms" }
                        }
                    }
                }
            }
        }
    }
}
