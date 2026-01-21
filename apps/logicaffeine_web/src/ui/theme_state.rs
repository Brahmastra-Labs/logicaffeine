//! Theme state management with localStorage persistence.
//!
//! Provides a reactive theme system with 4 nature-inspired themes:
//! - Sunrise: Warm dawn colors (coral, gold, amber)
//! - Violet: Twilight mystical (lavender, magenta, soft pink)
//! - Ocean: Deep sea calm (turquoise, aqua, seafoam)
//! - Mountain: Earth grounded (forest green, stone, muted gold) - default
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
    /// Warm dawn colors - coral, gold, amber
    Sunrise,
    /// Twilight mystical - lavender, magenta, soft pink
    Violet,
    /// Deep sea calm - turquoise, aqua, seafoam
    Ocean,
    /// Earth grounded - forest green, stone, muted gold (default)
    #[default]
    Mountain,
}

impl Theme {
    /// Returns the theme name for display.
    pub fn name(&self) -> &'static str {
        match self {
            Theme::Sunrise => "Sunrise",
            Theme::Violet => "Violet",
            Theme::Ocean => "Ocean",
            Theme::Mountain => "Mountain",
        }
    }

    /// Returns the data-theme attribute value.
    pub fn data_attr(&self) -> &'static str {
        match self {
            Theme::Sunrise => "sunrise",
            Theme::Violet => "violet",
            Theme::Ocean => "ocean",
            Theme::Mountain => "mountain",
        }
    }

    /// Returns an iterator over all themes.
    pub fn all() -> impl Iterator<Item = Theme> {
        [Theme::Sunrise, Theme::Violet, Theme::Ocean, Theme::Mountain].into_iter()
    }

    /// Returns the CSS variables for this theme.
    pub fn css_variables(&self) -> &'static str {
        match self {
            Theme::Sunrise => r#"
                --bg-gradient-start: #0d1b2a;
                --bg-gradient-mid: #3d1c56;
                --bg-gradient-end: #e07b53;
                --accent-primary: #ff7f50;
                --accent-secondary: #ffd700;
                --accent-tertiary: #ffbf00;
                --accent-primary-rgb: 255, 127, 80;
                --accent-secondary-rgb: 255, 215, 0;
            "#,
            Theme::Violet => r#"
                --bg-gradient-start: #1a0a2e;
                --bg-gradient-mid: #2d1b69;
                --bg-gradient-end: #16213e;
                --accent-primary: #e6b3ff;
                --accent-secondary: #ff1493;
                --accent-tertiary: #ffb6c1;
                --accent-primary-rgb: 230, 179, 255;
                --accent-secondary-rgb: 255, 20, 147;
            "#,
            Theme::Ocean => r#"
                --bg-gradient-start: #0a1628;
                --bg-gradient-mid: #064e3b;
                --bg-gradient-end: #0c4a6e;
                --accent-primary: #40e0d0;
                --accent-secondary: #00ffff;
                --accent-tertiary: #98d8c8;
                --accent-primary-rgb: 64, 224, 208;
                --accent-secondary-rgb: 0, 255, 255;
            "#,
            Theme::Mountain => r#"
                --bg-gradient-start: #070a12;
                --bg-gradient-mid: #0b1022;
                --bg-gradient-end: #070a12;
                --accent-primary: #22c55e;
                --accent-secondary: #94a3b8;
                --accent-tertiary: #d4a574;
                --accent-primary-rgb: 34, 197, 94;
                --accent-secondary-rgb: 148, 163, 184;
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
            Theme::Ocean => Theme::Mountain,
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
        .progress-accent {{
            background: linear-gradient(90deg, var(--accent-primary), var(--accent-secondary));
        }}

        /* Focus rings */
        .focus-accent:focus {{
            box-shadow: 0 0 0 3px rgba(var(--accent-primary-rgb), 0.3);
            border-color: var(--accent-primary);
        }}
        "#
    )
}
