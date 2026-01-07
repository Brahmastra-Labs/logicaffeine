/// Unified responsive and mobile styling system for Logicaffeine.
///
/// This module centralizes all mobile/responsive concerns:
/// - Breakpoint definitions (XS: 480px, SM: 640px, MD: 768px, LG: 1024px, XL: 1280px)
/// - Touch target standards (44px WCAG minimum, 48px comfortable, 56px large)
/// - Mobile-specific CSS utilities (.desktop-only, .mobile-only, .touch-target)
/// - Safe area insets for notched devices (iPhone X+)
/// - Reduced motion support (WCAG 2.1 Level AAA)
/// - Reusable mobile component styles (tabs, accordions, panels, buttons)
///
/// # Usage
///
/// Import this module and include `MOBILE_BASE_STYLES` in your root component (app.rs),
/// then use the provided class names and CSS variables throughout.
///
/// ```rust,ignore
/// // In app.rs:
/// use crate::ui::responsive::MOBILE_BASE_STYLES;
///
/// rsx! {
///     style { "{MOBILE_BASE_STYLES}" }
///     // ... rest of app
/// }
/// ```
///
/// # Mobile Patterns
///
/// ## Breakpoints
/// Use the constants from `breakpoints::` module:
/// - `breakpoints::XS` (480px) - Small phones
/// - `breakpoints::SM` (640px) - Phones landscape, hamburger menu threshold
/// - `breakpoints::MD` (768px) - Primary mobile/desktop breakpoint
/// - `breakpoints::LG` (1024px) - Small laptops
/// - `breakpoints::XL` (1280px) - Desktops
///
/// ## Touch Targets (WCAG 2.5.5)
/// All interactive elements on mobile must meet minimum 44x44px size:
/// ```css
/// @media (max-width: 768px) {
///     .my-button {
///         min-height: var(--touch-min, 44px);
///         -webkit-tap-highlight-color: transparent;
///         touch-action: manipulation;
///     }
/// }
/// ```
///
/// ## Safe Area Insets
/// For fixed/sticky elements on notched devices:
/// ```css
/// @supports (padding: env(safe-area-inset-top)) {
///     .fixed-header {
///         padding-top: env(safe-area-inset-top);
///     }
///     .fixed-bottom {
///         bottom: max(24px, env(safe-area-inset-bottom));
///     }
/// }
/// ```
///
/// ## Reduced Motion
/// Always respect user preference for reduced motion:
/// ```css
/// @media (prefers-reduced-motion: reduce) {
///     .animated-element {
///         animation: none !important;
///         transition: none !important;
///     }
/// }
/// ```
///
/// # Testing
///
/// Run mobile-specific tests:
/// ```bash
/// cargo test --test mobile_tabs_tests
/// ```
///
/// See `.zenflow/tasks/implement-a-mobile-responsivenes-0bca/MOBILE_TESTING.md`
/// for comprehensive testing guidelines.

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

/* ============================================ */
/* REDUCED MOTION SUPPORT                       */
/* ============================================ */
/* Respects user preference for reduced motion. */
/* This is a WCAG 2.1 Level AAA requirement.    */
/* Animations are reduced to near-instant while */
/* still showing final states for functionality.*/
@media (prefers-reduced-motion: reduce) {
    *,
    *::before,
    *::after {
        /* Reduce animation to near-instant (0.01ms allows final state to render) */
        animation-duration: 0.01ms !important;
        animation-iteration-count: 1 !important;
        animation-delay: 0ms !important;
        /* Reduce transitions to near-instant */
        transition-duration: 0.01ms !important;
        transition-delay: 0ms !important;
        /* Disable scroll behavior animations */
        scroll-behavior: auto !important;
    }

    /* Ensure decorative animations are completely disabled */
    .particles,
    .flame,
    .combo-flames,
    [class*="pulse"],
    [class*="bounce"],
    [class*="shake"],
    [class*="dance"] {
        animation: none !important;
    }

    /* Ensure transform-based hover effects are static */
    *:hover {
        transform: none !important;
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
// MOBILE ACCORDION TABS
// =============================================================================

/// Mobile accordion tab styles for stacked, expandable tab navigation.
/// Use this pattern when horizontal tabs overflow on mobile viewports.
/// Each tab header is a full-width touch target that expands to reveal content.
pub const MOBILE_ACCORDION_STYLES: &str = r#"
/* Accordion container - hidden on desktop, shown on mobile */
.accordion-tabs {
    display: none;
    flex-direction: column;
    width: 100%;
    gap: 8px;
    padding: 0 var(--mobile-padding, 12px);
}

@media (max-width: 768px) {
    .accordion-tabs {
        display: flex;
    }

    /* Hide standard horizontal tabs on mobile when accordion is used */
    .desktop-tabs {
        display: none !important;
    }
}

/* Individual accordion tab item */
.accordion-tab-item {
    border-radius: var(--radius-md, 8px);
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.08);
    overflow: hidden;
    transition: border-color 0.2s ease;
}

.accordion-tab-item.expanded {
    border-color: rgba(102, 126, 234, 0.4);
}

/* Accordion tab header - the clickable touch target */
.accordion-tab-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    min-height: var(--touch-comfortable, 48px);
    padding: 12px 16px;
    border: none;
    background: transparent;
    color: var(--text-secondary, #888);
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
    touch-action: manipulation;
    transition: background 0.15s ease, color 0.15s ease;
}

.accordion-tab-header:active {
    background: rgba(255, 255, 255, 0.08);
}

.accordion-tab-header.expanded {
    color: var(--text-primary, #e8e8e8);
    background: rgba(255, 255, 255, 0.06);
}

/* Header left section with icon and label */
.accordion-tab-header-left {
    display: flex;
    align-items: center;
    gap: 12px;
}

.accordion-tab-icon {
    font-size: 18px;
    line-height: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
}

.accordion-tab-label {
    font-size: 14px;
    font-weight: 600;
}

/* Expand/collapse chevron indicator */
.accordion-tab-chevron {
    font-size: 12px;
    transition: transform 0.2s ease;
    color: var(--text-tertiary, #666);
}

.accordion-tab-header.expanded .accordion-tab-chevron {
    transform: rotate(180deg);
}

/* Lock icon for locked tabs */
.accordion-tab-lock {
    font-size: 14px;
    color: var(--text-tertiary, #666);
    margin-left: 8px;
}

.accordion-tab-header.locked {
    opacity: 0.5;
    cursor: not-allowed;
}

/* Accordion tab content - animated expand/collapse */
.accordion-tab-content {
    max-height: 0;
    overflow: hidden;
    transition: max-height 0.25s ease-out;
}

.accordion-tab-content.expanded {
    max-height: 2000px;
    transition: max-height 0.35s ease-in;
}

.accordion-tab-content-inner {
    padding: 0 16px 16px;
}

/* Color variants for different tab types */

/* Lesson tab - blue accent (default) */
.accordion-tab-header.lesson.expanded {
    background: rgba(96, 165, 250, 0.12);
    color: #60a5fa;
}

.accordion-tab-item.lesson.expanded {
    border-color: rgba(96, 165, 250, 0.3);
}

.accordion-tab-header.lesson .accordion-tab-icon {
    color: #60a5fa;
}

/* Examples tab - purple accent */
.accordion-tab-header.examples.expanded {
    background: rgba(167, 139, 250, 0.12);
    color: #a78bfa;
}

.accordion-tab-item.examples.expanded {
    border-color: rgba(167, 139, 250, 0.3);
}

.accordion-tab-header.examples .accordion-tab-icon {
    color: #a78bfa;
}

/* Practice tab - green accent */
.accordion-tab-header.practice.expanded {
    background: rgba(74, 222, 128, 0.12);
    color: #4ade80;
}

.accordion-tab-item.practice.expanded {
    border-color: rgba(74, 222, 128, 0.3);
}

.accordion-tab-header.practice .accordion-tab-icon {
    color: #4ade80;
}

/* Test tab - yellow/amber accent */
.accordion-tab-header.test.expanded {
    background: rgba(251, 191, 36, 0.12);
    color: #fbbf24;
}

.accordion-tab-item.test.expanded {
    border-color: rgba(251, 191, 36, 0.3);
}

.accordion-tab-header.test .accordion-tab-icon {
    color: #fbbf24;
}

/* Reduced motion support */
@media (prefers-reduced-motion: reduce) {
    .accordion-tab-content {
        transition: none;
    }

    .accordion-tab-chevron {
        transition: none;
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
        "{}\n{}\n{}\n{}\n{}\n{}\n{}",
        MOBILE_BASE_STYLES,
        MOBILE_TAB_BAR_STYLES,
        MOBILE_PANEL_STYLES,
        MOBILE_BUTTON_STYLES,
        MOBILE_INPUT_STYLES,
        MOBILE_ACCORDION_STYLES,
        MOBILE_RESIZER_STYLES,
    )
}

/// Generate a complete mobile-ready style block for a page
/// This combines the base mobile utilities with any page-specific styles
pub fn with_mobile_styles(page_styles: &str) -> String {
    format!("{}\n{}", MOBILE_BASE_STYLES, page_styles)
}
