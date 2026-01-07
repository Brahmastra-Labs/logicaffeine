//! Hamburger Menu component for mobile navigation.
//!
//! A three-line menu icon that animates to an X when open.
//! Mobile-only visibility (hidden on desktop).

use dioxus::prelude::*;
use crate::ui::responsive::breakpoints;

/// CSS styles for the hamburger menu button.
///
/// Features:
/// - Three-line icon that animates to X when open
/// - 48px touch target for WCAG compliance
/// - Hidden on desktop (shows at breakpoints::SM and below)
/// - Smooth CSS transitions for open/close animation
pub const HAMBURGER_MENU_STYLES: &str = r#"
/* Hamburger menu button - hidden on desktop */
.hamburger-menu {
    display: none;
    align-items: center;
    justify-content: center;
    width: 48px;
    height: 48px;
    padding: 0;
    border: none;
    background: rgba(255, 255, 255, 0.05);
    border-radius: var(--radius-md, 8px);
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
    touch-action: manipulation;
    transition: background 0.15s ease;
    position: relative;
    z-index: 60;
}

.hamburger-menu:hover {
    background: rgba(255, 255, 255, 0.08);
}

.hamburger-menu:active {
    background: rgba(255, 255, 255, 0.12);
}

/* Show hamburger on mobile */
@media (max-width: 640px) {
    .hamburger-menu {
        display: flex;
    }
}

/* The hamburger icon container */
.hamburger-icon {
    display: flex;
    flex-direction: column;
    justify-content: center;
    align-items: center;
    width: 24px;
    height: 24px;
    position: relative;
}

/* The three lines */
.hamburger-line {
    position: absolute;
    width: 20px;
    height: 2px;
    background: var(--text-primary, #e8e8e8);
    border-radius: 2px;
    transition: transform 0.25s ease, opacity 0.25s ease;
}

.hamburger-line:nth-child(1) {
    transform: translateY(-6px);
}

.hamburger-line:nth-child(2) {
    transform: translateY(0);
}

.hamburger-line:nth-child(3) {
    transform: translateY(6px);
}

/* Open state - transforms to X */
.hamburger-menu.open .hamburger-line:nth-child(1) {
    transform: translateY(0) rotate(45deg);
}

.hamburger-menu.open .hamburger-line:nth-child(2) {
    opacity: 0;
    transform: scaleX(0);
}

.hamburger-menu.open .hamburger-line:nth-child(3) {
    transform: translateY(0) rotate(-45deg);
}

/* Reduced motion support */
@media (prefers-reduced-motion: reduce) {
    .hamburger-line {
        transition: none;
    }
}

/* Accessibility - focus outline */
.hamburger-menu:focus-visible {
    outline: 2px solid var(--color-accent-blue, #60a5fa);
    outline-offset: 2px;
}
"#;

/// Props for the HamburgerMenu component
#[derive(Props, Clone, PartialEq)]
pub struct HamburgerMenuProps {
    /// Whether the menu is currently open
    is_open: Signal<bool>,
    /// Handler called when the button is clicked
    on_toggle: EventHandler<()>,
}

/// Hamburger menu button for mobile navigation.
///
/// This component displays a three-line menu icon that animates to an X when open.
/// It's hidden on desktop and only visible at mobile breakpoints (≤640px).
///
/// # Usage
/// ```ignore
/// let mut is_nav_open = use_signal(|| false);
///
/// HamburgerMenu {
///     is_open: is_nav_open,
///     on_toggle: move |_| is_nav_open.toggle(),
/// }
/// ```
///
/// # Accessibility
/// - Includes aria-label for screen readers
/// - Uses aria-expanded to indicate state
/// - Meets WCAG 2.5 touch target size (48x48px)
/// - Focus-visible outline for keyboard navigation
#[component]
pub fn HamburgerMenu(props: HamburgerMenuProps) -> Element {
    let is_open = props.is_open.read();
    let class = if *is_open {
        "hamburger-menu open"
    } else {
        "hamburger-menu"
    };
    let aria_label = if *is_open {
        "Close navigation menu"
    } else {
        "Open navigation menu"
    };

    rsx! {
        style { "{HAMBURGER_MENU_STYLES}" }
        button {
            class: "{class}",
            r#type: "button",
            aria_label: "{aria_label}",
            aria_expanded: "{is_open}",
            onclick: move |_| props.on_toggle.call(()),

            div { class: "hamburger-icon",
                span { class: "hamburger-line" }
                span { class: "hamburger-line" }
                span { class: "hamburger-line" }
            }
        }
    }
}

/// Get the breakpoint value at which the hamburger menu becomes visible
pub const fn hamburger_breakpoint() -> &'static str {
    breakpoints::SM
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hamburger_styles_contain_required_classes() {
        assert!(HAMBURGER_MENU_STYLES.contains(".hamburger-menu"));
        assert!(HAMBURGER_MENU_STYLES.contains(".hamburger-icon"));
        assert!(HAMBURGER_MENU_STYLES.contains(".hamburger-line"));
    }

    #[test]
    fn test_hamburger_has_open_state_styles() {
        assert!(HAMBURGER_MENU_STYLES.contains(".hamburger-menu.open"));
        assert!(HAMBURGER_MENU_STYLES.contains("rotate(45deg)"));
        assert!(HAMBURGER_MENU_STYLES.contains("rotate(-45deg)"));
    }

    #[test]
    fn test_hamburger_hidden_on_desktop_shown_on_mobile() {
        // Should be hidden by default (desktop)
        assert!(HAMBURGER_MENU_STYLES.contains(".hamburger-menu {\n    display: none;"));
        // Should be shown on mobile via media query
        assert!(HAMBURGER_MENU_STYLES.contains("@media (max-width: 640px)"));
        assert!(HAMBURGER_MENU_STYLES.contains("display: flex;"));
    }

    #[test]
    fn test_hamburger_touch_target_meets_wcag() {
        // WCAG 2.5 requires 44px minimum, we use 48px
        assert!(HAMBURGER_MENU_STYLES.contains("width: 48px;"));
        assert!(HAMBURGER_MENU_STYLES.contains("height: 48px;"));
    }

    #[test]
    fn test_hamburger_has_tap_highlight_disabled() {
        assert!(HAMBURGER_MENU_STYLES.contains("-webkit-tap-highlight-color: transparent;"));
    }

    #[test]
    fn test_hamburger_has_touch_action_manipulation() {
        assert!(HAMBURGER_MENU_STYLES.contains("touch-action: manipulation;"));
    }

    #[test]
    fn test_hamburger_supports_reduced_motion() {
        assert!(HAMBURGER_MENU_STYLES.contains("@media (prefers-reduced-motion: reduce)"));
    }

    #[test]
    fn test_hamburger_has_focus_visible_styles() {
        assert!(HAMBURGER_MENU_STYLES.contains(":focus-visible"));
        assert!(HAMBURGER_MENU_STYLES.contains("outline:"));
    }

    #[test]
    fn test_hamburger_breakpoint_is_sm() {
        assert_eq!(hamburger_breakpoint(), "640px");
    }

    #[test]
    fn test_hamburger_has_animation_transitions() {
        assert!(HAMBURGER_MENU_STYLES.contains("transition: transform"));
        assert!(HAMBURGER_MENU_STYLES.contains("transition: opacity"));
    }
}
