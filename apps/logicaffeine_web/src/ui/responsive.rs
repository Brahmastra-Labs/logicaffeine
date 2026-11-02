//! Responsive design system and mobile styling utilities.
//!
//! Centralizes the mobile/responsive CSS shared across pages so behavior stays
//! consistent across device sizes: the base utilities and the mobile tab bar.
//!
//! # Breakpoint Strategy
//!
//! Mobile-first approach with these breakpoints:
//! - 768px: Tablets (primary mobile breakpoint, `--breakpoint-mobile`)
//! - 1024px: Small laptops (`--breakpoint-tablet`)
//!
//! # Touch Target Standards
//!
//! Follows WCAG 2.5 guidelines:
//! - Minimum: 44x44px
//! - Comfortable: 48x48px
//! - Large (primary actions): 56x56px
//!
//! # Usage
//!
//! Include the styles in the page component:
//!
//! ```no_run
//! # use dioxus::prelude::*;
//! use logicaffeine_web::ui::responsive::{MOBILE_BASE_STYLES, MOBILE_TAB_BAR_STYLES};
//!
//! # fn Example() -> Element {
//! rsx! {
//!     style { "{MOBILE_BASE_STYLES}" }
//!     style { "{MOBILE_TAB_BAR_STYLES}" }
//! }
//! # }
//! ```
//!
//! # CSS Classes
//!
//! | Class | Description |
//! |-------|-------------|
//! | `.desktop-only` | Hidden on mobile (≤768px) |
//! | `.mobile-only` | Hidden on desktop (>768px) |
//! | `.touch-target` | Minimum 44x44px touch area |
//! | `.safe-top` | Respects notch/status bar |
//! | `.safe-bottom` | Respects home indicator |
//! | `.scroll-smooth` | iOS momentum scrolling |

// =============================================================================
// BREAKPOINTS, MEDIA QUERIES AND TOUCH
// =============================================================================

/// The widths and heights the layout changes at, in CSS pixels.
pub mod breakpoints {
    /// The widest phone held upright: iPhones are 390 to 430, Android phones 360 to 412.
    pub const PHONE_MAX: u32 = 600;
    /// The widest window that stacks the Studio's panels: an upright iPad mini (744) and iPad (768).
    pub const MOBILE_MAX: u32 = 768;
    /// The widest window the site's nav folds into its menu, and the widest upright tablet.
    pub const TABLET_MAX: u32 = 1024;
    /// The tallest window that is a phone turned on its side.
    pub const SHORT_MAX: u32 = 500;
    /// The narrowest phones, where the buttons tighten further.
    pub const SMALL_PHONE_MAX: u32 = 480;
    /// The width from which a frame on another page leaves its wider margins.
    pub const FRAME_WIDE_MIN: u32 = 720;
}

/// The media queries the Studio's stylesheets and these shared ones use, by name. A stylesheet cannot
/// read a custom property inside `@media`, so each query is written out in the CSS, and the one-Studio
/// lock holds every `@media` the Studio draws from to one of [`media::ALL`]. The app's own `matchMedia`
/// checks use the same strings.
pub mod media {
    /// A phone held upright.
    pub const PHONE: &str = "(max-width: 600px)";
    /// A phone or an upright tablet: the Studio's panels stack.
    pub const MOBILE: &str = "(max-width: 768px)";
    /// Wider than an upright tablet: the desktop's columns.
    pub const DESKTOP: &str = "(min-width: 769px)";
    /// An upright tablet the width of a small phone's landscape: iPad mini and iPad portrait.
    pub const TABLET_PORTRAIT: &str = "(min-width: 601px) and (max-width: 768px)";
    /// An upright tablet wider than 768px, an iPad Air or iPad Pro held upright, known by its finger.
    pub const UPRIGHT_TABLET: &str = "(min-width: 769px) and (max-width: 1024px) and (orientation: portrait) and (pointer: coarse)";
    /// A tablet turned on its side.
    pub const TABLET_LANDSCAPE: &str = "(min-width: 769px) and (min-height: 501px) and (pointer: coarse)";
    /// A phone turned on its side.
    pub const PHONE_LANDSCAPE: &str = "(max-height: 500px) and (orientation: landscape)";
    /// A finger, on any screen.
    pub const COARSE: &str = "(pointer: coarse)";
    /// A screen nothing hovers over.
    pub const NO_HOVER: &str = "(hover: none)";
    /// The narrowest phones.
    pub const SMALL_PHONE: &str = "(max-width: 480px)";
    /// Up to an upright tablet's width: the site's nav folds into its menu.
    pub const TABLET: &str = "(max-width: 1024px)";
    /// A small laptop or a tablet on its side: the menu's roomier layout.
    pub const SMALL_LAPTOP: &str = "(min-width: 769px) and (max-width: 1024px)";
    /// A frame on another page with room for its wider margins.
    pub const FRAME_WIDE: &str = "(min-width: 720px)";
    /// A reader who asked for less motion.
    pub const REDUCED_MOTION: &str = "(prefers-reduced-motion: reduce)";

    /// Where the Files tree is a drawer over the workbench, folded until the reader opens it: phones,
    /// upright tablets, and a phone on its side.
    pub const FILES_DRAWER: &[&str] = &[MOBILE, UPRIGHT_TABLET, PHONE_LANDSCAPE];
    /// Where the panes show one at a time, or side by side in halves, so a pane's header folds nothing.
    pub const PANES_FIXED: &[&str] = &[PHONE, PHONE_LANDSCAPE];

    pub const ALL: &[&str] = &[
        PHONE,
        MOBILE,
        DESKTOP,
        TABLET_PORTRAIT,
        UPRIGHT_TABLET,
        TABLET_LANDSCAPE,
        PHONE_LANDSCAPE,
        COARSE,
        NO_HOVER,
        SMALL_PHONE,
        TABLET,
        SMALL_LAPTOP,
        FRAME_WIDE,
        REDUCED_MOTION,
    ];
}

/// Touch targets, in CSS pixels (WCAG 2.5.5).
pub mod touch {
    /// The least a finger can press.
    pub const MIN: u32 = 44;
    /// A comfortable press.
    pub const COMFORTABLE: u32 = 48;
}

// =============================================================================
// BASE MOBILE STYLES
// =============================================================================

/// Include this in your root component (app.rs) for global mobile utilities
pub const MOBILE_BASE_STYLES: &str = r#"
/* ============================================ */
/* MOBILE CSS VARIABLES                         */
/* ============================================ */
:root {
    /* Touch targets */
    --touch-min: 44px;
    --touch-comfortable: 48px;
    --touch-large: 56px;

    /* Mobile spacing */
    --mobile-padding: 12px;
    --mobile-gap: 8px;

    /* Safe area insets for notched devices */
    --safe-top: env(safe-area-inset-top, 0px);
    --safe-bottom: env(safe-area-inset-bottom, 0px);
    --safe-left: env(safe-area-inset-left, 0px);
    --safe-right: env(safe-area-inset-right, 0px);

    /* Layout heights - standardized across app */
    --header-height: 72px;
    --footer-height-desktop: 280px;
    --footer-height-tablet: 360px;
    --footer-height-mobile: auto;

    /* Standardized breakpoints (as CSS custom properties for reference) */
    --breakpoint-mobile: 768px;
    --breakpoint-tablet: 1024px;
}

/* ============================================ */
/* MOBILE UTILITY CLASSES                       */
/* ============================================ */

/* Hide on mobile, show on desktop */
.desktop-only {
    display: block;
}

/* Show on mobile, hide on desktop */
.mobile-only {
    display: none;
}

@media (max-width: 768px) {
    .desktop-only {
        display: none !important;
    }
    .mobile-only {
        display: block !important;
    }
    .mobile-only-flex {
        display: flex !important;
    }
}

/* Touch-friendly button base */
.touch-target {
    min-width: var(--touch-min);
    min-height: var(--touch-min);
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
    touch-action: manipulation;
}

/* Safe area padding utilities */
.safe-top {
    padding-top: max(var(--mobile-padding), var(--safe-top));
}

.safe-bottom {
    padding-bottom: max(var(--mobile-padding), var(--safe-bottom));
}

.safe-horizontal {
    padding-left: max(var(--mobile-padding), var(--safe-left));
    padding-right: max(var(--mobile-padding), var(--safe-right));
}

/* Smooth scrolling with momentum on iOS */
.scroll-smooth {
    -webkit-overflow-scrolling: touch;
    scroll-behavior: smooth;
}

/* Prevent text selection on interactive elements */
.no-select {
    -webkit-user-select: none;
    user-select: none;
}

/* Reduced motion support */
@media (prefers-reduced-motion: reduce) {
    *,
    *::before,
    *::after {
        animation-duration: 0.01ms !important;
        animation-iteration-count: 1 !important;
        transition-duration: 0.01ms !important;
    }
}

/* ============================================ */
/* BODY SCROLL LOCK                            */
/* ============================================ */

/* Applied to body when mobile menu is open */
body.menu-open {
    overflow: hidden;
    position: fixed;
    width: 100%;
    height: 100%;
}

/* ============================================ */
/* HAMBURGER MENU & OVERLAY                    */
/* ============================================ */

/* Hamburger button */
.hamburger-btn {
    display: none;
    width: var(--touch-comfortable);
    height: var(--touch-comfortable);
    padding: 0;
    border: none;
    background: transparent;
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
    position: relative;
    z-index: 1001;
}

.hamburger-icon {
    width: 24px;
    height: 18px;
    position: relative;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
}

.hamburger-line {
    width: 100%;
    height: 2px;
    background: var(--text-primary, #f0f0f0);
    border-radius: 1px;
    transition: all 0.3s ease;
    transform-origin: center;
}

/* Hamburger to X animation */
.hamburger-btn.open .hamburger-line:nth-child(1) {
    transform: translateY(8px) rotate(45deg);
}

.hamburger-btn.open .hamburger-line:nth-child(2) {
    opacity: 0;
    transform: scaleX(0);
}

.hamburger-btn.open .hamburger-line:nth-child(3) {
    transform: translateY(-8px) rotate(-45deg);
}

/* Mobile menu overlay */
.mobile-menu-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: rgba(6, 8, 20, 0.98);
    backdrop-filter: blur(20px);
    -webkit-backdrop-filter: blur(20px);
    z-index: 1000;
    opacity: 0;
    visibility: hidden;
    transition: opacity 0.3s ease, visibility 0.3s ease;
    display: flex;
    flex-direction: column;
    padding: calc(var(--header-height) + 20px) var(--mobile-padding) var(--mobile-padding);
    overflow-y: auto;
}

.mobile-menu-overlay.open {
    opacity: 1;
    visibility: visible;
}

/* Mobile menu navigation links */
.mobile-menu-nav {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-bottom: 32px;
}

.mobile-menu-link {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 16px 20px;
    border-radius: 12px;
    background: rgba(255, 255, 255, 0.03);
    color: var(--text-secondary, #b0b0b0);
    text-decoration: none;
    font-size: 18px;
    font-weight: 500;
    transition: all 0.2s ease;
    border: 1px solid transparent;
}

.mobile-menu-link:active {
    transform: scale(0.98);
    background: rgba(255, 255, 255, 0.08);
}

.mobile-menu-link.active {
    background: rgba(102, 126, 234, 0.15);
    color: var(--text-primary, #f0f0f0);
    border-color: rgba(102, 126, 234, 0.3);
}

.mobile-menu-link-icon {
    font-size: 20px;
    width: 24px;
    text-align: center;
}

/* Mobile menu CTAs */
.mobile-menu-ctas {
    display: flex;
    flex-direction: column;
    gap: 12px;
    margin-top: auto;
    padding-top: 24px;
    border-top: 1px solid rgba(255, 255, 255, 0.08);
}

.mobile-menu-cta {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 16px 24px;
    border-radius: 12px;
    font-size: 16px;
    font-weight: 600;
    text-decoration: none;
    transition: all 0.2s ease;
}

.mobile-menu-cta.primary {
    background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
    color: white;
}

.mobile-menu-cta.secondary {
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid rgba(255, 255, 255, 0.1);
    color: var(--text-primary, #f0f0f0);
}

/* Show hamburger on mobile/tablet */
@media (max-width: 1024px) {
    .hamburger-btn {
        display: flex;
        align-items: center;
        justify-content: center;
    }

    .nav-links-desktop {
        display: none !important;
    }

    .nav-ctas-desktop {
        display: none !important;
    }
}

/* ============================================ */
/* TABLET-SPECIFIC ADJUSTMENTS                 */
/* ============================================ */

@media (min-width: 769px) and (max-width: 1024px) {
    .mobile-menu-overlay {
        padding: calc(var(--header-height) + 40px) 40px 40px;
    }

    .mobile-menu-link {
        padding: 18px 24px;
        font-size: 20px;
    }

    .mobile-menu-nav {
        max-width: 400px;
        margin: 0 auto 40px;
    }

    .mobile-menu-ctas {
        max-width: 400px;
        margin: auto auto 0;
    }
}
"#;

// =============================================================================
// MOBILE TAB BAR COMPONENT STYLES
// =============================================================================

/// Reusable mobile tab bar styles - use for any tabbed interface on mobile
pub const MOBILE_TAB_BAR_STYLES: &str = r#"
/* Mobile Tab Bar Container */
.mobile-tabs {
    display: none;
}

@media (max-width: 768px) {
    .mobile-tabs {
        display: flex;
        gap: 4px;
        padding: 8px var(--mobile-padding, 12px);
        background: rgba(0, 0, 0, 0.4);
        border-bottom: 1px solid rgba(255, 255, 255, 0.08);
        overflow-x: auto;
        -webkit-overflow-scrolling: touch;
        flex-shrink: 0;
        /* Hide scrollbar but keep functionality */
        scrollbar-width: none;
        -ms-overflow-style: none;
    }

    .mobile-tabs::-webkit-scrollbar {
        display: none;
    }

    /* Individual Tab Button */
    .mobile-tab {
        flex: 1;
        min-width: 0;
        padding: 10px 8px;
        border: none;
        border-radius: 10px;
        background: rgba(255, 255, 255, 0.05);
        color: #888;
        font-size: 13px;
        font-weight: 500;
        cursor: pointer;
        transition: all 0.2s ease;
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 4px;
        min-height: var(--touch-min, 44px);
        -webkit-tap-highlight-color: transparent;
    }

    .mobile-tab-icon {
        font-size: 18px;
        line-height: 1;
    }

    .mobile-tab-label {
        font-size: 11px;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        max-width: 100%;
    }

    .mobile-tab:active {
        background: rgba(255, 255, 255, 0.15);
        transform: scale(0.97);
    }

    .mobile-tab.active {
        background: rgba(102, 126, 234, 0.25);
        color: #e8e8e8;
        border: 1px solid rgba(102, 126, 234, 0.4);
    }

    /* Tab indicator dots (optional, for swipe hint) */
    .mobile-tab-indicator {
        display: flex;
        justify-content: center;
        gap: 6px;
        padding: 6px;
        background: rgba(0, 0, 0, 0.2);
    }

    .mobile-tab-dot {
        width: 6px;
        height: 6px;
        border-radius: 50%;
        background: rgba(255, 255, 255, 0.2);
        transition: all 0.2s ease;
    }

    .mobile-tab-dot.active {
        background: #667eea;
        width: 18px;
        border-radius: 3px;
    }
}

/* Landscape mobile - horizontal tab layout */
@media (max-height: 500px) and (orientation: landscape) {
    .mobile-tabs {
        padding: 4px 8px;
    }

    .mobile-tab {
        padding: 6px 12px;
        flex-direction: row;
        gap: 6px;
        min-height: 36px;
    }

    .mobile-tab-icon {
        font-size: 16px;
    }
}
"#;

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/responsive/tests.rs"]
mod tests;
