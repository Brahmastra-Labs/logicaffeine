//! Responsive design system and mobile styling utilities.
//!
//! Centralizes all mobile/responsive concerns to ensure consistent behavior
//! across device sizes. Provides breakpoints, touch target standards, and
//! reusable CSS for mobile-first components.
//!
//! # Breakpoint Strategy
//!
//! Mobile-first approach with these breakpoints:
//! - `XS` (480px): Small phones
//! - `SM` (640px): Large phones, small tablets
//! - `MD` (768px): Tablets (primary mobile breakpoint)
//! - `LG` (1024px): Small laptops
//! - `XL` (1280px): Desktops
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
//! Include base styles in your root component:
//!
//! ```no_run
//! # use dioxus::prelude::*;
//! use logicaffeine_web::ui::responsive::{MOBILE_BASE_STYLES, all_mobile_styles};
//!
//! # fn Example() -> Element {
//! rsx! {
//!     // Option 1: Just the base utilities
//!     style { "{MOBILE_BASE_STYLES}" }
//!
//!     // Option 2: All mobile styles including tabs, panels, buttons
//!     style { "{all_mobile_styles()}" }
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
// BREAKPOINTS
// =============================================================================

/// Standard breakpoint values used across the application
pub mod breakpoints {
    /// Extra small devices (phones in portrait)
    pub const XS: &str = "480px";
    /// Small devices (phones in landscape, small tablets)
    pub const SM: &str = "640px";
    /// Medium devices (tablets)
    pub const MD: &str = "768px";
    /// Large devices (small laptops)
    pub const LG: &str = "1024px";
    /// Extra large devices (desktops)
    pub const XL: &str = "1280px";
}

/// Media query helpers - use these in your CSS strings
pub mod media {
    pub const MOBILE: &str = "@media (max-width: 768px)";
    pub const TABLET: &str = "@media (min-width: 769px) and (max-width: 1024px)";
    pub const DESKTOP: &str = "@media (min-width: 1025px)";
    pub const MOBILE_LANDSCAPE: &str = "@media (max-height: 500px) and (orientation: landscape)";
    pub const TOUCH_DEVICE: &str = "@media (hover: none) and (pointer: coarse)";
    pub const REDUCED_MOTION: &str = "@media (prefers-reduced-motion: reduce)";
}

// =============================================================================
// TOUCH TARGETS
// =============================================================================

/// WCAG 2.5 compliant touch target sizes
pub mod touch {
    /// Minimum touch target size (44x44px per WCAG 2.5)
    pub const MIN_TARGET: &str = "44px";
    /// Comfortable touch target size
    pub const COMFORTABLE_TARGET: &str = "48px";
    /// Large touch target for primary actions
    pub const LARGE_TARGET: &str = "56px";
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

// =============================================================================
// MOBILE PANEL STYLES
// =============================================================================

/// Styles for switchable panel content (used with mobile tabs)
pub const MOBILE_PANEL_STYLES: &str = r#"
/* Desktop: show all panels side by side */
.panel-container {
    display: flex;
    flex: 1;
    overflow: hidden;
}

.panel {
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: rgba(0, 0, 0, 0.3);
}

.panel-header {
    padding: 12px 16px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: #888;
    display: flex;
    justify-content: space-between;
    align-items: center;
    flex-shrink: 0;
}

.panel-content {
    flex: 1;
    overflow: auto;
    -webkit-overflow-scrolling: touch;
}

@media (max-width: 768px) {
    .panel-container {
        flex-direction: column;
        position: relative;
    }

    /* On mobile, panels stack and only active one shows */
    .panel {
        position: absolute;
        top: 0;
        left: 0;
        right: 0;
        bottom: 0;
        opacity: 0;
        pointer-events: none;
        transition: opacity 0.15s ease;
        min-width: unset;
    }

    .panel.panel-active {
        position: relative;
        flex: 1;
        opacity: 1;
        pointer-events: auto;
    }

    /* Panel headers hidden on mobile (tabs replace them) */
    .panel .panel-header {
        display: none;
    }

    /* But show header for active panel if it has controls */
    .panel.panel-active .panel-header.has-controls {
        display: flex;
        padding: 8px 12px;
        background: rgba(0, 0, 0, 0.2);
    }
}
"#;

// =============================================================================
// MOBILE BUTTON STYLES
// =============================================================================

/// Mobile-optimized button styles with proper touch targets
pub const MOBILE_BUTTON_STYLES: &str = r#"
@media (max-width: 768px) {
    /* Ensure all buttons meet touch target requirements */
    button,
    .btn,
    [role="button"] {
        min-height: var(--touch-min, 44px);
        min-width: var(--touch-min, 44px);
        padding: 10px 16px;
        font-size: 14px;
    }

    /* Toggle button groups */
    .toggle-group {
        gap: 6px;
        padding: 4px;
        border-radius: 8px;
    }

    .toggle-btn {
        padding: 10px 16px;
        font-size: 14px;
        border-radius: 6px;
        min-height: var(--touch-min, 44px);
        min-width: var(--touch-min, 44px);
        display: flex;
        align-items: center;
        justify-content: center;
    }

    /* Small icon buttons */
    .icon-btn {
        width: var(--touch-min, 44px);
        height: var(--touch-min, 44px);
        padding: 0;
        display: flex;
        align-items: center;
        justify-content: center;
    }

    .icon-btn svg,
    .icon-btn .icon {
        width: 20px;
        height: 20px;
    }
}
"#;

// =============================================================================
// MOBILE INPUT STYLES
// =============================================================================

/// Mobile-optimized form input styles
pub const MOBILE_INPUT_STYLES: &str = r#"
@media (max-width: 768px) {
    /* Text inputs and textareas */
    input[type="text"],
    input[type="email"],
    input[type="password"],
    input[type="search"],
    textarea {
        font-size: 16px; /* Prevents iOS zoom on focus */
        min-height: var(--touch-min, 44px);
        padding: 12px 16px;
        border-radius: 10px;
    }

    textarea {
        min-height: 120px;
        resize: vertical;
    }

    /* Labels above inputs */
    label {
        font-size: 14px;
        margin-bottom: 6px;
    }

    /* Form groups */
    .form-group {
        margin-bottom: 16px;
    }
}
"#;

// =============================================================================
// MOBILE RESIZER ALTERNATIVE
// =============================================================================

/// On mobile, hide desktop resizers entirely
pub const MOBILE_RESIZER_STYLES: &str = r#"
.panel-resizer {
    width: 6px;
    background: rgba(255, 255, 255, 0.05);
    cursor: col-resize;
    transition: background 0.2s ease;
    flex-shrink: 0;
}

.panel-resizer:hover,
.panel-resizer.active {
    background: rgba(102, 126, 234, 0.5);
}

@media (max-width: 768px) {
    .panel-resizer {
        display: none;
    }
}
"#;

// =============================================================================
// COMBINED MOBILE STYLES
// =============================================================================

/// All mobile styles combined - include this for a complete mobile solution
pub fn all_mobile_styles() -> String {
    format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        MOBILE_BASE_STYLES,
        MOBILE_TAB_BAR_STYLES,
        MOBILE_PANEL_STYLES,
        MOBILE_BUTTON_STYLES,
        MOBILE_INPUT_STYLES,
        MOBILE_RESIZER_STYLES,
    )
}

/// Generate a complete mobile-ready style block for a page
/// This combines the base mobile utilities with any page-specific styles
pub fn with_mobile_styles(page_styles: &str) -> String {
    format!("{}\n{}", MOBILE_BASE_STYLES, page_styles)
}
