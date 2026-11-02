//! Root application component and global styles.
//!
//! This module defines the top-level Dioxus component that bootstraps the
//! application, sets up global context providers, and injects CSS styles.
//!
//! # Context Providers
//!
//! The [`App`] component provides these global contexts:
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

use crate::ui::router::Route;
use crate::ui::state::RegistryAuthState;
use dioxus::prelude::*;

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

    /* Palette: the page background and the two accents */
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
}

* {
    margin: 0;
    padding: 0;
    box-sizing: border-box;
}

html, body {
    height: 100%;
    background: linear-gradient(
        135deg,
        var(--bg-gradient-start) 0%,
        var(--bg-gradient-mid) 50%,
        var(--bg-gradient-end) 100%
    );
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
    color: var(--accent-primary);
    text-shadow: 0 0 20px var(--accent-glow);
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

/* Accents. These components still hardcode colours of their own; the rules below paint
   them from the palette, which is why they carry !important. */
.main-nav-link.active::after {
    background: linear-gradient(90deg, var(--accent-primary), var(--accent-secondary)) !important;
}

.btn-primary,
.main-nav-btn.primary {
    background: linear-gradient(135deg, var(--accent-primary), var(--accent-secondary)) !important;
    box-shadow: 0 12px 30px rgba(var(--accent-primary-rgb), 0.18) !important;
}

.btn-primary:hover,
.main-nav-btn.primary:hover {
    box-shadow: 0 16px 40px rgba(var(--accent-primary-rgb), 0.25) !important;
}

.mobile-nav-item.active,
.mobile-nav-link.active {
    background: rgba(var(--accent-primary-rgb), 0.12) !important;
    color: var(--accent-primary) !important;
}

input:focus,
textarea:focus {
    box-shadow: 0 0 0 3px rgba(var(--accent-primary-rgb), 0.3);
    border-color: var(--accent-primary);
}

.badge .dot {
    background: var(--accent-primary) !important;
    box-shadow: 0 0 0 6px rgba(var(--accent-primary-rgb), 0.12) !important;
}

.card:hover {
    border-color: rgba(var(--accent-secondary-rgb), 0.28) !important;
}

.card:hover::before {
    background: linear-gradient(135deg, rgba(var(--accent-primary-rgb), 0.12), rgba(var(--accent-secondary-rgb), 0.12)) !important;
}

.icon-box {
    background: rgba(var(--accent-primary-rgb), 0.15) !important;
}

.code.logic {
    color: var(--accent-secondary) !important;
}
"#;

/// Root application component.
///
/// Sets up the registry authentication context and renders the router.
///
/// # Context Providers
///
/// - [`RegistryAuthState`] - Manages GitHub OAuth for package registry
pub fn App() -> Element {
    let _registry_auth = use_context_provider(RegistryAuthState::new);

    rsx! {
        style { "{GLOBAL_STYLE}" }
        div {
            id: "app-root",
            // No explicit SuspenseBoundary: lazy route chunks suspend against
            // the implicit root boundary (the wasm-split harness shape). The
            // CSR takeover that swaps out the prerendered copy watches this
            // element's content from outside — see the poll in index.html.
            Router::<Route> {}
        }
    }
}
