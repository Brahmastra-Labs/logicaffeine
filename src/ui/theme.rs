/// Design tokens for consistent styling across the application.
/// All colors, font sizes, and spacing values should be defined here.

// =============================================================================
// COLORS
// =============================================================================

/// Primary brand colors
pub mod colors {
    // Primary palette
    pub const PRIMARY_BLUE: &str = "#667eea";
    pub const PRIMARY_PURPLE: &str = "#764ba2";
    pub const ACCENT_BLUE: &str = "#60a5fa";
    pub const ACCENT_PURPLE: &str = "#a78bfa";

    // Semantic colors
    pub const SUCCESS: &str = "#4ade80";
    pub const WARNING: &str = "#f59e0b";
    pub const ERROR: &str = "#e06c75";
    pub const INFO: &str = "#60a5fa";

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

    // Text colors - UPDATED for better accessibility
    pub const TEXT_PRIMARY: &str = "#f0f0f0";           // Was #e8e8e8, now lighter
    pub const TEXT_SECONDARY: &str = "#b0b0b0";         // Was #888, now lighter
    pub const TEXT_TERTIARY: &str = "#909090";          // Was #666, now lighter
    pub const TEXT_MUTED: &str = "#a0a0a0";             // Was #aaa, kept similar
    pub const TEXT_PLACEHOLDER: &str = "#808080";       // Was darker

    // Text with opacity (for overlays/backgrounds)
    pub const TEXT_HIGH_CONTRAST: &str = "rgba(240,242,245,0.95)";    // Was 0.9
    pub const TEXT_MEDIUM: &str = "rgba(240,242,245,0.80)";           // Was 0.72
    pub const TEXT_LOW: &str = "rgba(240,242,245,0.70)";              // Was 0.65
    pub const TEXT_SUBTLE: &str = "rgba(240,242,245,0.55)";           // Was 0.45
    pub const TEXT_VERY_SUBTLE: &str = "rgba(240,242,245,0.45)";      // Was 0.35

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

/// Font sizes - UPDATED: all increased by 2px for accessibility
pub mod font_size {
    // Display sizes
    pub const DISPLAY_XL: &str = "66px";    // Was 64px
    pub const DISPLAY_LG: &str = "50px";    // Was 48px
    pub const DISPLAY_MD: &str = "34px";    // Was 32px

    // Heading sizes
    pub const HEADING_LG: &str = "26px";    // Was 24px
    pub const HEADING_MD: &str = "22px";    // Was 20px
    pub const HEADING_SM: &str = "20px";    // Was 18px

    // Body sizes
    pub const BODY_LG: &str = "18px";       // Was 16px
    pub const BODY_MD: &str = "16px";       // Was 14px
    pub const BODY_SM: &str = "15px";       // Was 13px

    // Small/caption sizes
    pub const CAPTION_LG: &str = "14px";    // Was 12px
    pub const CAPTION_MD: &str = "13px";    // Was 11px
    pub const CAPTION_SM: &str = "12px";    // Was 10px
}

/// Font families
pub mod font_family {
    pub const MONO: &str = "'SF Mono', 'Fira Code', 'Consolas', monospace";
    pub const MONO_SYSTEM: &str = "ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Monaco, 'Cascadia Code', monospace";
    pub const SANS: &str = "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, sans-serif";
}

// =============================================================================
// SPACING
// =============================================================================

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

pub mod radius {
    pub const SM: &str = "4px";
    pub const MD: &str = "8px";
    pub const LG: &str = "12px";
    pub const XL: &str = "16px";
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
        /* Primary colors */
        --color-primary-blue: #667eea;
        --color-primary-purple: #764ba2;
        --color-accent-blue: #60a5fa;
        --color-accent-purple: #a78bfa;

        /* Semantic colors */
        --color-success: #4ade80;
        --color-warning: #f59e0b;
        --color-error: #e06c75;
        --color-info: #60a5fa;

        /* Text colors - accessible grays */
        --text-primary: #f0f0f0;
        --text-secondary: #b0b0b0;
        --text-tertiary: #909090;
        --text-muted: #a0a0a0;
        --text-placeholder: #808080;

        /* Font sizes - +2px for accessibility */
        --font-display-xl: 66px;
        --font-display-lg: 50px;
        --font-display-md: 34px;
        --font-heading-lg: 26px;
        --font-heading-md: 22px;
        --font-heading-sm: 20px;
        --font-body-lg: 18px;
        --font-body-md: 16px;
        --font-body-sm: 15px;
        --font-caption-lg: 14px;
        --font-caption-md: 13px;
        --font-caption-sm: 12px;

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
    }
    "#
}
