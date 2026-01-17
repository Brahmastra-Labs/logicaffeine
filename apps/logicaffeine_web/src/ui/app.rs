//! Root application component and global styles.
//!
//! This module defines the top-level Dioxus component that bootstraps the
//! application, sets up global context providers, and injects CSS styles.
//!
//! # Context Providers
//!
//! The [`App`] component provides these global contexts:
//! - [`LicenseState`] - License key validation and plan tiers
//! - [`RegistryAuthState`] - GitHub authentication for package registry
//!
//! # Global Styles
//!
//! CSS custom properties (design tokens) are injected at the root level,
//! making them available throughout the component tree via `var(--name)`.
//!
//! # Routing
//!
//! The app uses Dioxus Router with routes defined in [`crate::ui::router::Route`].

use dioxus::prelude::*;
use crate::ui::router::Route;
use crate::ui::state::{LicenseState, RegistryAuthState};
use crate::ui::theme;

/// Global CSS including design tokens, reset styles, and common component styles.
const GLOBAL_STYLE: &str = r#"
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

    /* Text colors - accessible grays (lighter for visibility) */
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

* {
    margin: 0;
    padding: 0;
    box-sizing: border-box;
}

html, body {
    height: 100%;
    background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f3460 100%);
    color: var(--text-primary);
    font-family: var(--font-sans);
    font-size: var(--font-body-lg);
    overflow-x: hidden;
}

#main {
    min-height: 100vh;
}

a {
    color: inherit;
    text-decoration: none;
}

.chat-area {
    flex: 1;
    overflow-y: auto;
    padding: 30px;
    display: flex;
    flex-direction: column;
    gap: var(--spacing-lg);
}

.message {
    max-width: 75%;
    padding: 14px 20px;
    border-radius: var(--radius-xl);
    line-height: 1.6;
    animation: fadeIn 0.3s ease;
}

@keyframes fadeIn {
    from { opacity: 0; transform: translateY(10px); }
    to { opacity: 1; transform: translateY(0); }
}

.message.user {
    align-self: flex-end;
    background: linear-gradient(135deg, var(--color-primary-blue) 0%, var(--color-primary-purple) 100%);
    color: white;
    border-bottom-right-radius: var(--radius-sm);
    box-shadow: 0 4px 15px rgba(102, 126, 234, 0.3);
}

.message.system {
    align-self: flex-start;
    background: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.15);
    border-bottom-left-radius: var(--radius-sm);
    font-family: var(--font-mono);
    font-size: var(--font-heading-sm);
    color: #00d4ff;
    text-shadow: 0 0 20px rgba(0, 212, 255, 0.3);
}

.message.error {
    align-self: flex-start;
    background: linear-gradient(135deg, #ff6b6b 0%, #c92a2a 100%);
    color: white;
    border-bottom-left-radius: var(--radius-sm);
    font-style: italic;
    box-shadow: 0 4px 15px rgba(255, 107, 107, 0.3);
}

.input-area {
    background: rgba(0, 0, 0, 0.4);
    backdrop-filter: blur(10px);
    padding: 20px 30px;
    border-top: 1px solid rgba(255, 255, 255, 0.1);
}

.input-row {
    display: flex;
    gap: var(--spacing-md);
    align-items: center;
}

.input-row input {
    flex: 1;
    background: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.2);
    border-radius: var(--radius-lg);
    padding: 14px 20px;
    font-size: var(--font-body-lg);
    color: white;
    outline: none;
    transition: all 0.2s ease;
}

.input-row input::placeholder {
    color: var(--text-placeholder);
}

.input-row input:focus {
    border-color: var(--color-primary-blue);
    box-shadow: 0 0 0 3px rgba(102, 126, 234, 0.2);
}

.input-row button {
    background: linear-gradient(135deg, var(--color-primary-blue) 0%, var(--color-primary-purple) 100%);
    border: none;
    border-radius: var(--radius-lg);
    padding: 14px 28px;
    font-size: var(--font-body-lg);
    font-weight: 600;
    color: white;
    cursor: pointer;
    transition: all 0.2s ease;
    box-shadow: 0 4px 15px rgba(102, 126, 234, 0.3);
}

.input-row button:hover {
    transform: translateY(-2px);
    box-shadow: 0 6px 20px rgba(102, 126, 234, 0.4);
}

.input-row button:active {
    transform: translateY(0);
}

/* Interactive reveal section */
.reveal-section {
    margin-top: var(--spacing-lg);
    padding-top: var(--spacing-lg);
    border-top: 1px solid rgba(255,255,255,0.06);
}

.reveal-buttons {
    display: flex;
    gap: var(--spacing-md);
    flex-wrap: wrap;
    margin-bottom: var(--spacing-lg);
}

.reveal-btn {
    padding: 10px 16px;
    border-radius: var(--radius-md);
    border: 1px solid rgba(255,255,255,0.15);
    background: rgba(255,255,255,0.05);
    color: var(--text-secondary);
    font-size: var(--font-body-sm);
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s ease;
    display: inline-flex;
    align-items: center;
    gap: 6px;
}

.reveal-btn:hover {
    background: rgba(255,255,255,0.10);
    color: var(--text-primary);
}

.reveal-btn.active {
    background: linear-gradient(135deg, rgba(96,165,250,0.2), rgba(167,139,250,0.2));
    border-color: rgba(167,139,250,0.4);
    color: var(--text-primary);
}

.revealed-content {
    padding: var(--spacing-lg);
    background: rgba(255,255,255,0.03);
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: var(--radius-lg);
    margin-top: var(--spacing-md);
    animation: fadeIn 0.2s ease;
}

.revealed-label {
    font-size: var(--font-caption-sm);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-tertiary);
    margin-bottom: var(--spacing-sm);
}

.revealed-logic {
    font-family: var(--font-mono);
    font-size: var(--font-heading-sm);
    color: var(--color-accent-blue);
    padding: var(--spacing-md);
    background: rgba(96, 165, 250, 0.08);
    border-radius: var(--radius-md);
    margin: var(--spacing-md) 0;
}

/* Socratic hint box */
.socratic-hint-box {
    margin-top: var(--spacing-lg);
    padding: var(--spacing-lg);
    background: linear-gradient(135deg, rgba(167,139,250,0.08), rgba(96,165,250,0.08));
    border: 1px solid rgba(167,139,250,0.2);
    border-radius: var(--radius-lg);
    border-left: 4px solid var(--color-accent-purple);
}

.hint-header {
    display: flex;
    align-items: center;
    gap: var(--spacing-sm);
    margin-bottom: var(--spacing-sm);
    font-size: var(--font-caption-md);
    font-weight: 600;
    color: var(--color-accent-purple);
}

.hint-text {
    color: var(--text-secondary);
    line-height: 1.6;
}

/* Multiple choice options */
.multiple-choice-options {
    display: flex;
    flex-direction: column;
    gap: var(--spacing-sm);
    margin: var(--spacing-lg) 0;
}

.multiple-choice-options .reveal-btn {
    width: 100%;
    text-align: left;
    padding: var(--spacing-md) var(--spacing-lg);
    font-family: var(--font-mono);
}

.multiple-choice-options .reveal-btn.correct {
    background: rgba(74, 222, 128, 0.15);
    border-color: var(--color-success);
}

.multiple-choice-options .reveal-btn.incorrect {
    background: rgba(248, 113, 113, 0.15);
    border-color: var(--color-error);
}

/* Progress indicator - shows exercise completion within a module */
.exercise-progress {
    display: flex;
    align-items: center;
    gap: var(--spacing-sm);
    font-size: var(--font-caption-md);
    color: var(--text-tertiary);
    margin-bottom: var(--spacing-md);
}

.progress-bar {
    flex: 1;
    height: 4px;
    background: rgba(255,255,255,0.1);
    border-radius: 2px;
    overflow: hidden;
}

.progress-fill {
    height: 100%;
    background: linear-gradient(90deg, var(--color-accent-blue), var(--color-accent-purple));
    border-radius: 2px;
    transition: width 0.3s ease;
}

.practice-score {
    font-weight: 600;
    color: var(--color-success);
}

/* Exercise mode badges */
.exercise-mode-badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 4px 10px;
    border-radius: var(--radius-full);
    font-size: var(--font-caption-sm);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-right: var(--spacing-md);
}

.exercise-mode-badge.test {
    background: rgba(251, 191, 36, 0.15);
    color: #fbbf24;
}

.exercise-mode-badge.practice {
    background: rgba(74, 222, 128, 0.15);
    color: var(--color-success);
}
"#;

/// Root application component.
///
/// Sets up global context providers for license and registry authentication,
/// triggers license revalidation on mount, and renders the router.
///
/// # Context Providers
///
/// - [`LicenseState`] - Manages subscription validation
/// - [`RegistryAuthState`] - Manages GitHub OAuth for package registry
pub fn App() -> Element {
    let license_state = use_context_provider(LicenseState::new);
    let _registry_auth = use_context_provider(RegistryAuthState::new);

    use_effect(move || {
        let mut license_state = license_state.clone();
        spawn(async move {
            if license_state.has_license() && license_state.needs_revalidation() {
                license_state.validate().await;
            }
        });
    });

    rsx! {
        style { "{GLOBAL_STYLE}" }
        Router::<Route> {}
    }
}
