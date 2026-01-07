/// Tests for Mobile Tabs implementation
///
/// Verifies that the mobile tab styles and components meet the requirements:
/// - All 4 tabs visible and tappable on mobile viewports
/// - Touch targets meet 44px minimum height (WCAG 2.5)
/// - Active tab content displays correctly
/// - Color coding preserved (blue Lesson, purple Examples, green Practice, yellow Test)
/// - Desktop layout unchanged (horizontal tabs hidden on mobile)
/// - Responsive breakpoint at 768px
/// - Reduced motion support

use logos::ui::responsive::{
    MOBILE_ACCORDION_STYLES, MOBILE_BASE_STYLES, all_mobile_styles, breakpoints, media, touch,
};

// =============================================================================
// CSS CONSTANTS TESTS
// =============================================================================

#[test]
fn test_breakpoints_defined() {
    assert_eq!(breakpoints::XS, "480px");
    assert_eq!(breakpoints::SM, "640px");
    assert_eq!(breakpoints::MD, "768px");
    assert_eq!(breakpoints::LG, "1024px");
    assert_eq!(breakpoints::XL, "1280px");
}

#[test]
fn test_media_queries_defined() {
    assert!(media::MOBILE.contains("768px"));
    assert!(media::TABLET.contains("769px"));
    assert!(media::DESKTOP.contains("1025px"));
    assert!(media::REDUCED_MOTION.contains("prefers-reduced-motion"));
}

#[test]
fn test_touch_targets_meet_wcag_standards() {
    assert_eq!(touch::MIN_TARGET, "44px", "WCAG 2.5 requires minimum 44px touch targets");
    assert_eq!(touch::COMFORTABLE_TARGET, "48px");
    assert_eq!(touch::LARGE_TARGET, "56px");
}

// =============================================================================
// MOBILE ACCORDION STYLES TESTS
// =============================================================================

#[test]
fn test_accordion_styles_contain_required_classes() {
    assert!(
        MOBILE_ACCORDION_STYLES.contains(".accordion-tabs"),
        "Should define accordion container class"
    );
    assert!(
        MOBILE_ACCORDION_STYLES.contains(".accordion-tab-item"),
        "Should define accordion item class"
    );
    assert!(
        MOBILE_ACCORDION_STYLES.contains(".accordion-tab-header"),
        "Should define accordion header class"
    );
    assert!(
        MOBILE_ACCORDION_STYLES.contains(".accordion-tab-content"),
        "Should define accordion content class"
    );
}

#[test]
fn test_accordion_hidden_on_desktop_shown_on_mobile() {
    assert!(
        MOBILE_ACCORDION_STYLES.contains(".accordion-tabs {")
            && MOBILE_ACCORDION_STYLES.contains("display: none"),
        "Accordion should be hidden by default (desktop)"
    );
    assert!(
        MOBILE_ACCORDION_STYLES.contains("@media (max-width: 768px)")
            && MOBILE_ACCORDION_STYLES.contains("display: flex"),
        "Accordion should be displayed on mobile"
    );
}

#[test]
fn test_accordion_touch_target_minimum_height() {
    assert!(
        MOBILE_ACCORDION_STYLES.contains("min-height: var(--touch-comfortable, 48px)"),
        "Tab headers should meet touch target requirements (48px >= 44px WCAG minimum)"
    );
}

#[test]
fn test_accordion_content_animation() {
    assert!(
        MOBILE_ACCORDION_STYLES.contains("max-height: 0"),
        "Content should collapse to 0 height when closed"
    );
    assert!(
        MOBILE_ACCORDION_STYLES.contains("max-height: 2000px"),
        "Content should expand to large max-height when open"
    );
    assert!(
        MOBILE_ACCORDION_STYLES.contains("transition: max-height"),
        "Content should animate with transition"
    );
}

#[test]
fn test_accordion_reduced_motion_support() {
    assert!(
        MOBILE_ACCORDION_STYLES.contains("@media (prefers-reduced-motion: reduce)"),
        "Should respect reduced motion preference"
    );
    assert!(
        MOBILE_ACCORDION_STYLES.contains("transition: none"),
        "Should disable transitions for reduced motion"
    );
}

// =============================================================================
// COLOR VARIANT TESTS
// =============================================================================

#[test]
fn test_lesson_tab_uses_blue_accent() {
    assert!(
        MOBILE_ACCORDION_STYLES.contains(".accordion-tab-header.lesson"),
        "Should have lesson-specific styling"
    );
    assert!(
        MOBILE_ACCORDION_STYLES.contains("#60a5fa") || MOBILE_ACCORDION_STYLES.contains("96, 165, 250"),
        "Lesson tab should use blue accent color"
    );
}

#[test]
fn test_examples_tab_uses_purple_accent() {
    assert!(
        MOBILE_ACCORDION_STYLES.contains(".accordion-tab-header.examples"),
        "Should have examples-specific styling"
    );
    assert!(
        MOBILE_ACCORDION_STYLES.contains("#a78bfa") || MOBILE_ACCORDION_STYLES.contains("167, 139, 250"),
        "Examples tab should use purple accent color"
    );
}

#[test]
fn test_practice_tab_uses_green_accent() {
    assert!(
        MOBILE_ACCORDION_STYLES.contains(".accordion-tab-header.practice"),
        "Should have practice-specific styling"
    );
    assert!(
        MOBILE_ACCORDION_STYLES.contains("#4ade80") || MOBILE_ACCORDION_STYLES.contains("74, 222, 128"),
        "Practice tab should use green accent color"
    );
}

#[test]
fn test_test_tab_uses_yellow_accent() {
    assert!(
        MOBILE_ACCORDION_STYLES.contains(".accordion-tab-header.test"),
        "Should have test-specific styling"
    );
    assert!(
        MOBILE_ACCORDION_STYLES.contains("#fbbf24") || MOBILE_ACCORDION_STYLES.contains("251, 191, 36"),
        "Test tab should use yellow/amber accent color"
    );
}

// =============================================================================
// MOBILE BASE STYLES TESTS
// =============================================================================

#[test]
fn test_base_styles_define_css_variables() {
    assert!(
        MOBILE_BASE_STYLES.contains("--touch-min: 44px"),
        "Should define minimum touch target variable"
    );
    assert!(
        MOBILE_BASE_STYLES.contains("--touch-comfortable: 48px"),
        "Should define comfortable touch target variable"
    );
    assert!(
        MOBILE_BASE_STYLES.contains("--safe-top: env(safe-area-inset-top"),
        "Should define safe area insets for notched devices"
    );
}

#[test]
fn test_base_styles_utility_classes() {
    assert!(
        MOBILE_BASE_STYLES.contains(".desktop-only"),
        "Should have desktop-only utility class"
    );
    assert!(
        MOBILE_BASE_STYLES.contains(".mobile-only"),
        "Should have mobile-only utility class"
    );
    assert!(
        MOBILE_BASE_STYLES.contains(".touch-target"),
        "Should have touch-target utility class"
    );
}

#[test]
fn test_base_styles_responsive_behavior() {
    assert!(
        MOBILE_BASE_STYLES.contains("@media (max-width: 768px)")
            && MOBILE_BASE_STYLES.contains(".desktop-only")
            && MOBILE_BASE_STYLES.contains("display: none !important"),
        "Desktop-only elements should be hidden on mobile"
    );
}

// =============================================================================
// ALL MOBILE STYLES COMBINATION TESTS
// =============================================================================

#[test]
fn test_all_mobile_styles_includes_accordion() {
    let all_styles = all_mobile_styles();
    assert!(
        all_styles.contains(".accordion-tabs"),
        "Combined styles should include accordion styles"
    );
}

#[test]
fn test_all_mobile_styles_includes_base() {
    let all_styles = all_mobile_styles();
    assert!(
        all_styles.contains("--touch-min"),
        "Combined styles should include base styles"
    );
}

#[test]
fn test_all_mobile_styles_not_empty() {
    let all_styles = all_mobile_styles();
    assert!(
        all_styles.len() > 1000,
        "Combined styles should be substantial"
    );
}

// =============================================================================
// ACCESSIBILITY TESTS
// =============================================================================

#[test]
fn test_locked_tab_styling() {
    assert!(
        MOBILE_ACCORDION_STYLES.contains(".accordion-tab-header.locked"),
        "Should have styling for locked tabs"
    );
    assert!(
        MOBILE_ACCORDION_STYLES.contains("cursor: not-allowed"),
        "Locked tabs should show not-allowed cursor"
    );
    assert!(
        MOBILE_ACCORDION_STYLES.contains("opacity: 0.5"),
        "Locked tabs should be visually dimmed"
    );
}

#[test]
fn test_tap_highlight_disabled() {
    assert!(
        MOBILE_ACCORDION_STYLES.contains("-webkit-tap-highlight-color: transparent"),
        "Should disable default tap highlight for custom styling"
    );
}

#[test]
fn test_touch_action_manipulation() {
    assert!(
        MOBILE_ACCORDION_STYLES.contains("touch-action: manipulation"),
        "Should use touch-action: manipulation to remove 300ms delay"
    );
}

// =============================================================================
// CHEVRON INDICATOR TESTS
// =============================================================================

#[test]
fn test_chevron_rotation_animation() {
    assert!(
        MOBILE_ACCORDION_STYLES.contains(".accordion-tab-chevron"),
        "Should have chevron indicator"
    );
    assert!(
        MOBILE_ACCORDION_STYLES.contains("transform: rotate(180deg)"),
        "Expanded chevron should rotate 180 degrees"
    );
}

// =============================================================================
// LEARN PAGE INTEGRATION TESTS (CSS assertions)
// =============================================================================

#[test]
fn test_mobile_768px_breakpoint_standard() {
    // The standard breakpoint for mobile/desktop split
    let breakpoint = breakpoints::MD;
    assert_eq!(breakpoint, "768px", "Standard mobile breakpoint should be 768px");
}

#[test]
fn test_four_tab_modes_all_represented_in_styles() {
    // Each tab type should have its own color styling
    let all_styles = all_mobile_styles();
    let tab_types = ["lesson", "examples", "practice", "test"];

    for tab_type in tab_types {
        assert!(
            all_styles.contains(&format!(".accordion-tab-header.{}", tab_type)),
            "Should have styling for {} tab type",
            tab_type
        );
    }
}

// =============================================================================
// SAFE AREA INSET TESTS
// =============================================================================

#[test]
fn test_base_styles_safe_area_variables() {
    assert!(
        MOBILE_BASE_STYLES.contains("--safe-top: env(safe-area-inset-top, 0px)"),
        "Should define --safe-top CSS variable with fallback"
    );
    assert!(
        MOBILE_BASE_STYLES.contains("--safe-bottom: env(safe-area-inset-bottom, 0px)"),
        "Should define --safe-bottom CSS variable with fallback"
    );
    assert!(
        MOBILE_BASE_STYLES.contains("--safe-left: env(safe-area-inset-left, 0px)"),
        "Should define --safe-left CSS variable with fallback"
    );
    assert!(
        MOBILE_BASE_STYLES.contains("--safe-right: env(safe-area-inset-right, 0px)"),
        "Should define --safe-right CSS variable with fallback"
    );
}

#[test]
fn test_base_styles_safe_area_utility_classes() {
    assert!(
        MOBILE_BASE_STYLES.contains(".safe-top"),
        "Should have .safe-top utility class"
    );
    assert!(
        MOBILE_BASE_STYLES.contains(".safe-bottom"),
        "Should have .safe-bottom utility class"
    );
    assert!(
        MOBILE_BASE_STYLES.contains(".safe-horizontal"),
        "Should have .safe-horizontal utility class"
    );
}

#[test]
fn test_safe_area_uses_max_for_fallback() {
    // Ensure safe area utilities use max() to provide minimum padding
    assert!(
        MOBILE_BASE_STYLES.contains("padding-top: max(var(--mobile-padding), var(--safe-top))"),
        ".safe-top should use max() to ensure minimum padding"
    );
    assert!(
        MOBILE_BASE_STYLES.contains("padding-bottom: max(var(--mobile-padding), var(--safe-bottom))"),
        ".safe-bottom should use max() to ensure minimum padding"
    );
}
