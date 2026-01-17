//! Application navigation bar component.
//!
//! Renders a top navigation bar with logo, title, and navigation links.
//! Used on internal app pages (Learn, Studio, Profile).
//!
//! # Props
//!
//! - `title` - Optional page title (defaults to "LOGOS")

use dioxus::prelude::*;
use crate::ui::router::Route;

const APP_NAVBAR_STYLE: &str = r#"
.app-navbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 12px 24px;
    background: rgba(0, 0, 0, 0.25);
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    backdrop-filter: blur(12px);
}

.app-navbar-brand {
    display: flex;
    align-items: center;
    gap: 10px;
    text-decoration: none;
    color: inherit;
}

.app-navbar-logo {
    width: 28px;
    height: 28px;
    border-radius: 8px;
    background:
        radial-gradient(circle at 30% 30%, rgba(96,165,250,0.85), transparent 55%),
        radial-gradient(circle at 65% 60%, rgba(167,139,250,0.85), transparent 55%),
        rgba(255,255,255,0.06);
    border: 1px solid rgba(255,255,255,0.10);
}

.app-navbar-title {
    font-size: 16px;
    font-weight: 700;
    background: linear-gradient(135deg, #667eea, #764ba2);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
}

.app-navbar-nav {
    display: flex;
    gap: 8px;
    align-items: center;
}

.app-navbar-link {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 14px;
    border-radius: 8px;
    border: 1px solid rgba(255, 255, 255, 0.10);
    background: rgba(255, 255, 255, 0.04);
    color: #888;
    font-size: 13px;
    text-decoration: none;
    transition: all 0.2s ease;
}

.app-navbar-link:hover {
    background: rgba(255, 255, 255, 0.08);
    border-color: rgba(255, 255, 255, 0.15);
    color: #e8e8e8;
}

.app-navbar-link.site-link {
    color: #667eea;
}

.app-navbar-link.site-link:hover {
    color: #8b9cf7;
}
"#;

#[derive(Props, Clone, PartialEq)]
pub struct AppNavbarProps {
    #[props(default)]
    pub title: Option<String>,
}

#[component]
pub fn AppNavbar(props: AppNavbarProps) -> Element {
    let title = props.title.unwrap_or_else(|| "LOGOS".to_string());

    rsx! {
        style { "{APP_NAVBAR_STYLE}" }

        nav { class: "app-navbar",
            Link {
                class: "app-navbar-brand",
                to: Route::Landing {},
                div { class: "app-navbar-logo" }
                span { class: "app-navbar-title", "{title}" }
            }

            div { class: "app-navbar-nav",
                Link {
                    class: "app-navbar-link site-link",
                    to: Route::Landing {},
                    "← Home"
                }
            }
        }
    }
}
