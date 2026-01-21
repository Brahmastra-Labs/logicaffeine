//! Theme state management with localStorage persistence.
//!
//! Provides a reactive theme system with 7 nature-inspired themes:
//! - Sunrise: Warm dawn colors (coral, gold, amber)
//! - Violet: Twilight mystical (lavender, magenta, soft pink)
//! - Ocean: Deep sea calm (turquoise, aqua, seafoam)
//! - Mountain: Clean tech (cyan, indigo) - default
//! - Rose: Warm pink/rose tones
//! - Forest: Deep green earth tones
//! - Ember: Red/orange fire tones
//!
//! # Usage
//!
//! ```ignore
//! // Provide at app root
//! use_context_provider(ThemeState::new);
//!
//! // Use in components
//! let theme_state = use_context::<ThemeState>();
//! let current = theme_state.current();
//! theme_state.set_theme(Theme::Ocean);
//! ```

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use gloo_storage::{LocalStorage, Storage};

const THEME_STORAGE_KEY: &str = "logicaffeine-theme";

/// Available theme variants.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Theme {
    /// Warm dawn - coral, gold, amber
    Sunrise,
    /// Soft dusk - muted lavender, rose, warm gray
    Violet,
    /// Deep sea - cyan, teal, seafoam
    Ocean,
    /// Midnight tech - cyan accent, minimal, cutting-edge (default)
    #[default]
    Mountain,
    /// Warm pink - rose, blush, coral pink
    Rose,
    /// Deep green - forest, emerald, moss
    Forest,
    /// Fire tones - red, orange, amber
    Ember,
}

impl Theme {
    /// Returns the theme name for display.
    pub fn name(&self) -> &'static str {
        match self {
            Theme::Sunrise => "Sunrise",
            Theme::Violet => "Violet",
            Theme::Ocean => "Ocean",
            Theme::Mountain => "Mountain",
            Theme::Rose => "Rose",
            Theme::Forest => "Forest",
            Theme::Ember => "Ember",
        }
    }

    /// Returns the data-theme attribute value.
    pub fn data_attr(&self) -> &'static str {
        match self {
            Theme::Sunrise => "sunrise",
            Theme::Violet => "violet",
            Theme::Ocean => "ocean",
            Theme::Mountain => "mountain",
            Theme::Rose => "rose",
            Theme::Forest => "forest",
            Theme::Ember => "ember",
        }
    }

    /// Returns an iterator over all themes.
    pub fn all() -> impl Iterator<Item = Theme> {
        [
            Theme::Mountain,
            Theme::Sunrise,
            Theme::Violet,
            Theme::Ocean,
            Theme::Rose,
            Theme::Forest,
            Theme::Ember,
        ].into_iter()
    }

    /// Returns the CSS variables for this theme.
    pub fn css_variables(&self) -> &'static str {
        match self {
            // Sunrise: Warm, energetic dawn
            Theme::Sunrise => r#"
                --bg-gradient-start: #0c0a09;
                --bg-gradient-mid: #1c1410;
                --bg-gradient-end: #0c0a09;
                --accent-primary: #f97316;
                --accent-secondary: #fbbf24;
                --accent-tertiary: #fef3c7;
                --accent-primary-rgb: 249, 115, 22;
                --accent-secondary-rgb: 251, 191, 36;
                --accent-border: rgba(249, 115, 22, 0.3);
                --accent-glow: rgba(249, 115, 22, 0.2);
            "#,
            // Violet: Soft, sophisticated dusk (less intense purple)
            Theme::Violet => r#"
                --bg-gradient-start: #0f0d13;
                --bg-gradient-mid: #1a1520;
                --bg-gradient-end: #0f0d13;
                --accent-primary: #a78bfa;
                --accent-secondary: #f0abfc;
                --accent-tertiary: #e9d5ff;
                --accent-primary-rgb: 167, 139, 250;
                --accent-secondary-rgb: 240, 171, 252;
                --accent-border: rgba(167, 139, 250, 0.3);
                --accent-glow: rgba(167, 139, 250, 0.2);
            "#,
            // Ocean: Cool, calm depths
            Theme::Ocean => r#"
                --bg-gradient-start: #0a0f14;
                --bg-gradient-mid: #0c1820;
                --bg-gradient-end: #0a0f14;
                --accent-primary: #22d3ee;
                --accent-secondary: #2dd4bf;
                --accent-tertiary: #a5f3fc;
                --accent-primary-rgb: 34, 211, 238;
                --accent-secondary-rgb: 45, 212, 191;
                --accent-border: rgba(34, 211, 238, 0.3);
                --accent-glow: rgba(34, 211, 238, 0.2);
            "#,
            // Mountain (default): Clean, cutting-edge tech
            Theme::Mountain => r#"
                --bg-gradient-start: #09090b;
                --bg-gradient-mid: #0c0c10;
                --bg-gradient-end: #09090b;
                --accent-primary: #00d4ff;
                --accent-secondary: #818cf8;
                --accent-tertiary: #f0f0f0;
                --accent-primary-rgb: 0, 212, 255;
                --accent-secondary-rgb: 129, 140, 248;
                --accent-border: rgba(0, 212, 255, 0.3);
                --accent-glow: rgba(0, 212, 255, 0.2);
            "#,
            // Rose: Warm pink/rose tones
            Theme::Rose => r#"
                --bg-gradient-start: #0d0909;
                --bg-gradient-mid: #1a1015;
                --bg-gradient-end: #0d0909;
                --accent-primary: #f472b6;
                --accent-secondary: #fb7185;
                --accent-tertiary: #fce7f3;
                --accent-primary-rgb: 244, 114, 182;
                --accent-secondary-rgb: 251, 113, 133;
                --accent-border: rgba(244, 114, 182, 0.3);
                --accent-glow: rgba(244, 114, 182, 0.2);
            "#,
            // Forest: Deep green earth tones
            Theme::Forest => r#"
                --bg-gradient-start: #090d09;
                --bg-gradient-mid: #0f1a12;
                --bg-gradient-end: #090d09;
                --accent-primary: #4ade80;
                --accent-secondary: #86efac;
                --accent-tertiary: #dcfce7;
                --accent-primary-rgb: 74, 222, 128;
                --accent-secondary-rgb: 134, 239, 172;
                --accent-border: rgba(74, 222, 128, 0.3);
                --accent-glow: rgba(74, 222, 128, 0.2);
            "#,
            // Ember: Red/orange fire tones
            Theme::Ember => r#"
                --bg-gradient-start: #0d0808;
                --bg-gradient-mid: #1a0f0c;
                --bg-gradient-end: #0d0808;
                --accent-primary: #ef4444;
                --accent-secondary: #f97316;
                --accent-tertiary: #fef2f2;
                --accent-primary-rgb: 239, 68, 68;
                --accent-secondary-rgb: 249, 115, 22;
                --accent-border: rgba(239, 68, 68, 0.3);
                --accent-glow: rgba(239, 68, 68, 0.2);
            "#,
        }
    }
}

/// Global theme state with localStorage persistence.
#[derive(Clone, Copy)]
pub struct ThemeState {
    current: Signal<Theme>,
}

impl ThemeState {
    /// Creates a new ThemeState, loading from localStorage if available.
    pub fn new() -> Self {
        let stored_theme = LocalStorage::get::<Theme>(THEME_STORAGE_KEY).ok();
        let initial = stored_theme.unwrap_or_default();

        Self {
            current: Signal::new(initial),
        }
    }

    /// Returns the current theme.
    pub fn current(&self) -> Theme {
        *self.current.read()
    }

    /// Sets the theme and persists to localStorage.
    pub fn set_theme(&mut self, theme: Theme) {
        self.current.set(theme);
        let _ = LocalStorage::set(THEME_STORAGE_KEY, theme);
    }

    /// Cycles to the next theme.
    pub fn cycle_theme(&mut self) {
        let next = match self.current() {
            Theme::Mountain => Theme::Sunrise,
            Theme::Sunrise => Theme::Violet,
            Theme::Violet => Theme::Ocean,
            Theme::Ocean => Theme::Rose,
            Theme::Rose => Theme::Forest,
            Theme::Forest => Theme::Ember,
            Theme::Ember => Theme::Mountain,
        };
        self.set_theme(next);
    }
}

impl Default for ThemeState {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns the full theme CSS including variables and background.
pub fn theme_css(theme: Theme) -> String {
    let vars = theme.css_variables();
    format!(
        r#":root {{
            {vars}
        }}

        html, body {{
            background: linear-gradient(
                135deg,
                var(--bg-gradient-start) 0%,
                var(--bg-gradient-mid) 50%,
                var(--bg-gradient-end) 100%
            );
        }}

        /* Theme-aware accent colors */
        .accent-primary {{
            color: var(--accent-primary);
        }}

        .accent-secondary {{
            color: var(--accent-secondary);
        }}

        .bg-accent-primary {{
            background-color: var(--accent-primary);
        }}

        .bg-accent-secondary {{
            background-color: var(--accent-secondary);
        }}

        .border-accent {{
            border-color: var(--accent-primary);
        }}

        /* Override color-accent-blue/purple with theme colors */
        .main-nav-link.active::after {{
            background: linear-gradient(90deg, var(--accent-primary), var(--accent-secondary)) !important;
        }}

        .btn-primary,
        .main-nav-btn.primary {{
            background: linear-gradient(135deg, var(--accent-primary), var(--accent-secondary)) !important;
            box-shadow: 0 12px 30px rgba(var(--accent-primary-rgb), 0.18) !important;
        }}

        .btn-primary:hover,
        .main-nav-btn.primary:hover {{
            box-shadow: 0 16px 40px rgba(var(--accent-primary-rgb), 0.25) !important;
        }}

        /* Gradient buttons with theme colors */
        .btn-gradient {{
            background: linear-gradient(135deg, var(--accent-primary), var(--accent-secondary));
        }}

        .btn-gradient:hover {{
            box-shadow: 0 8px 24px rgba(var(--accent-primary-rgb), 0.3);
        }}

        /* Theme-aware link colors */
        a.accent-link {{
            color: var(--accent-primary);
            transition: color 0.2s ease;
        }}

        a.accent-link:hover {{
            color: var(--accent-secondary);
        }}

        /* Progress bars */
        .progress-accent,
        .progress-fill {{
            background: linear-gradient(90deg, var(--accent-primary), var(--accent-secondary));
        }}

        /* Active nav items */
        .mobile-nav-item.active,
        .mobile-nav-link.active {{
            background: rgba(var(--accent-primary-rgb), 0.12) !important;
            color: var(--accent-primary) !important;
        }}

        .learn-sidebar-module.active {{
            background: rgba(var(--accent-primary-rgb), 0.15) !important;
            color: var(--accent-primary) !important;
            border-left-color: var(--accent-primary) !important;
        }}

        /* Focus rings */
        .focus-accent:focus,
        input:focus,
        textarea:focus {{
            box-shadow: 0 0 0 3px rgba(var(--accent-primary-rgb), 0.3);
            border-color: var(--accent-primary);
        }}

        /* Badge/pill accents */
        .badge .dot {{
            background: var(--accent-primary) !important;
            box-shadow: 0 0 0 6px rgba(var(--accent-primary-rgb), 0.12) !important;
        }}

        /* Card hover accents */
        .card:hover {{
            border-color: rgba(var(--accent-secondary-rgb), 0.28) !important;
        }}

        .card:hover::before {{
            background: linear-gradient(135deg, rgba(var(--accent-primary-rgb), 0.12), rgba(var(--accent-secondary-rgb), 0.12)) !important;
        }}

        /* Icon box accents */
        .icon-box {{
            background: rgba(var(--accent-primary-rgb), 0.15) !important;
        }}

        /* Step numbers */
        .step-num {{
            background: linear-gradient(135deg, var(--accent-primary), var(--accent-secondary)) !important;
        }}

        /* Code/logic accents */
        .code.logic {{
            color: var(--accent-secondary) !important;
        }}
        "#
    )
}
