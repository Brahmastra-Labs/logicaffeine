//! Design token system for consistent styling.
//!
//! Centralizes all visual design constants—colors, typography, spacing, and
//! border radii—as Rust constants. These tokens ensure visual consistency
//! across all components.
//!
//! # Philosophy
//!
//! Design tokens are the single source of truth for styling:
//! - **Colors**: Brand palette, semantic colors, syntax highlighting
//! - **Typography**: Font sizes optimized for accessibility (+2px base)
//! - **Spacing**: Consistent rhythm for margins and padding
//! - **Radii**: Border radius scale from sharp to fully rounded
//!
//! # Usage
//!
//! Import tokens directly or use CSS custom properties:
//!
//! ```ignore
//! use crate::ui::theme::{colors, font_size, spacing};
//!
//! // Direct constant usage
//! let style = format!("color: {}; font-size: {};", colors::PRIMARY_BLUE, font_size::BODY_LG);
//!
//! // Or inject CSS variables and use var(--name)
//! let vars = theme::css_variables();
//! rsx! {
//!     style { "{vars}" }
//!     div { style: "color: var(--color-primary-blue);", "Hello" }
//! }
//! ```
//!
//! # Accessibility
//!
//! All font sizes have been increased by 2px from typical values to improve
//! readability. Text colors use sufficient contrast ratios against dark
//! backgrounds.

// =============================================================================
// COLORS
// =============================================================================

/// Color palette for the application.
///
/// Organized into categories:
/// - Primary/accent: Brand identity colors
/// - Semantic: Success, warning, error, info
/// - Gamification: XP, combos, achievements
/// - Syntax: Code highlighting colors
/// - Text: Accessible text color scale
/// - Background/Border: Surface colors
pub mod colors {
    // Primary palette - Modern tech aesthetic
    pub const PRIMARY_BLUE: &str = "#00d4ff";
    pub const PRIMARY_PURPLE: &str = "#818cf8";
    pub const ACCENT_BLUE: &str = "#22d3ee";
    pub const ACCENT_PURPLE: &str = "#a78bfa";

    // Semantic colors
    pub const SUCCESS: &str = "#22c55e";
    pub const WARNING: &str = "#f59e0b";
    pub const ERROR: &str = "#ef4444";
    pub const INFO: &str = "#00d4ff";

    // Achievement/gamification colors
    pub const XP_GREEN: &str = "#4ade80";
    pub const COMBO_ORANGE: &str = "#f97316";
    pub const COMBO_LIGHT: &str = "#fb923c";
    pub const ACHIEVEMENT_GOLD: &str = "#fbbf24";

    // Syntax highlighting colors
    pub const SYNTAX_QUANTIFIER: &str = "#c678dd";
    pub const SYNTAX_VARIABLE: &str = "#61afef";
    pub const SYNTAX_PREDICATE: &str = "#98c379";
    pub const SYNTAX_CONSTANT: &str = "#e5c07b";
    pub const SYNTAX_DETERMINER: &str = "#56b6c2";
    pub const SYNTAX_CONNECTIVE: &str = "#c678dd";

    // Text colors - MUCH lighter for readability
    pub const TEXT_PRIMARY: &str = "#f5f5f5";
    pub const TEXT_SECONDARY: &str = "#d0d0d0";
    pub const TEXT_TERTIARY: &str = "#b8b8b8";
    pub const TEXT_MUTED: &str = "#c0c0c0";
    pub const TEXT_PLACEHOLDER: &str = "#a0a0a0";

    // Text with opacity (for overlays/backgrounds) - higher opacity for readability
    pub const TEXT_HIGH_CONTRAST: &str = "rgba(245,245,245,0.98)";
    pub const TEXT_MEDIUM: &str = "rgba(245,245,245,0.88)";
    pub const TEXT_LOW: &str = "rgba(245,245,245,0.78)";
    pub const TEXT_SUBTLE: &str = "rgba(245,245,245,0.68)";
    pub const TEXT_VERY_SUBTLE: &str = "rgba(245,245,245,0.58)";

    // Background colors
    pub const BG_DARK: &str = "#060814";
    pub const BG_OVERLAY_DARK: &str = "rgba(0,0,0,0.8)";
    pub const BG_OVERLAY_LIGHT: &str = "rgba(0,0,0,0.25)";
    pub const BG_SUBTLE: &str = "rgba(255,255,255,0.08)";
    pub const BG_VERY_SUBTLE: &str = "rgba(255,255,255,0.04)";
    pub const BG_HOVER: &str = "rgba(255,255,255,0.1)";

    // Border colors
    pub const BORDER_SUBTLE: &str = "rgba(255,255,255,0.1)";
    pub const BORDER_MEDIUM: &str = "rgba(255,255,255,0.2)";
}

// =============================================================================
// TYPOGRAPHY
// =============================================================================

/// Font size scale.
///
/// Minimum 14px for all text to ensure readability.
/// Organized from display (largest) to caption (smallest).
pub mod font_size {
    // Display sizes
    pub const DISPLAY_XL: &str = "66px";
    pub const DISPLAY_LG: &str = "50px";
    pub const DISPLAY_MD: &str = "34px";

    // Heading sizes
    pub const HEADING_LG: &str = "26px";
    pub const HEADING_MD: &str = "22px";
    pub const HEADING_SM: &str = "20px";

    // Body sizes
    pub const BODY_LG: &str = "18px";
    pub const BODY_MD: &str = "16px";
    pub const BODY_SM: &str = "15px";

    // Caption sizes - minimum 14px for readability
    pub const CAPTION_LG: &str = "15px";
    pub const CAPTION_MD: &str = "14px";
    pub const CAPTION_SM: &str = "14px";
}

/// Font family stacks with fallbacks.
///
/// - `MONO`: Code and logic expressions
/// - `MONO_SYSTEM`: System monospace (better kerning)
/// - `SANS`: UI text and prose
pub mod font_family {
    /// Primary monospace for code editing.
    pub const MONO: &str = "'SF Mono', 'Fira Code', 'Consolas', monospace";
    /// System monospace with better native rendering.
    pub const MONO_SYSTEM: &str = "ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Monaco, 'Cascadia Code', monospace";
    /// System sans-serif for UI text.
    pub const SANS: &str = "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, sans-serif";
}

// =============================================================================
// SPACING
// =============================================================================

/// Spacing scale for consistent rhythm.
///
/// Use these for padding, margin, and gap values:
/// - `XS` (4px): Tight spacing within components
/// - `SM` (8px): Default component padding
/// - `MD` (12px): Medium gaps
/// - `LG` (16px): Section spacing
/// - `XL` (24px): Large section gaps
/// - `XXL` (32px): Page-level spacing
pub mod spacing {
    pub const XS: &str = "4px";
    pub const SM: &str = "8px";
    pub const MD: &str = "12px";
    pub const LG: &str = "16px";
    pub const XL: &str = "24px";
    pub const XXL: &str = "32px";
}

// =============================================================================
// BORDER RADIUS
// =============================================================================

/// Border radius scale.
///
/// - `SM`: Subtle rounding for inputs
/// - `MD`: Default button/card rounding
/// - `LG`: Prominent card rounding
/// - `XL`: Modal/dialog rounding
/// - `FULL`: Pill shapes and circles
pub mod radius {
    /// Subtle rounding (4px).
    pub const SM: &str = "4px";
    /// Default rounding (8px).
    pub const MD: &str = "8px";
    /// Prominent rounding (12px).
    pub const LG: &str = "12px";
    /// Large rounding (16px).
    pub const XL: &str = "16px";
    /// Fully rounded (pill shape).
    pub const FULL: &str = "9999px";
}

// =============================================================================
// CSS VARIABLE INJECTION
// =============================================================================

/// Returns a CSS block that defines all theme variables as CSS custom properties.
/// Include this in your root component to make variables available everywhere.
pub fn css_variables() -> &'static str {
    r#"
    :root {
        /* Primary colors - Modern tech aesthetic */
        --color-primary-blue: #00d4ff;
        --color-primary-purple: #818cf8;
        --color-accent-blue: #22d3ee;
        --color-accent-purple: #a78bfa;

        /* Semantic colors */
        --color-success: #22c55e;
        --color-warning: #f59e0b;
        --color-error: #ef4444;
        --color-info: #00d4ff;

        /* Text colors - MUCH lighter for readability */
        --text-primary: #f5f5f5;
        --text-secondary: #d0d0d0;
        --text-tertiary: #b8b8b8;
        --text-muted: #c0c0c0;
        --text-placeholder: #a0a0a0;

        /* Font sizes - minimum 14px for readability */
        --font-display-xl: 66px;
        --font-display-lg: 50px;
        --font-display-md: 34px;
        --font-heading-lg: 26px;
        --font-heading-md: 22px;
        --font-heading-sm: 20px;
        --font-body-lg: 18px;
        --font-body-md: 16px;
        --font-body-sm: 15px;
        --font-caption-lg: 15px;
        --font-caption-md: 14px;
        --font-caption-sm: 14px;

        /* Font families */
        --font-mono: 'SF Mono', 'Fira Code', 'Consolas', monospace;
        --font-sans: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, sans-serif;

        /* Spacing */
        --spacing-xs: 4px;
        --spacing-sm: 8px;
        --spacing-md: 12px;
        --spacing-lg: 16px;
        --spacing-xl: 24px;
        --spacing-xxl: 32px;

        /* Border radius */
        --radius-sm: 4px;
        --radius-md: 8px;
        --radius-lg: 12px;
        --radius-xl: 16px;
        --radius-full: 9999px;

        /* Theme defaults (Mountain - Cutting-edge tech) */
        --bg-gradient-start: #09090b;
        --bg-gradient-mid: #0c0c10;
        --bg-gradient-end: #09090b;
        --accent-primary: #00d4ff;
        --accent-secondary: #818cf8;
        --accent-tertiary: #f0f0f0;
        --accent-primary-rgb: 0, 212, 255;
        --accent-secondary-rgb: 129, 140, 248;
    }
    "#
}
