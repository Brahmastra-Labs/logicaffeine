use dioxus::prelude::*;
use crate::ui::router::Route;
use crate::ui::components::main_nav::{MainNav, ActivePage};

const TERMS_HTML: &str = include_str!("../../../terms.html");

const LEGAL_STYLE: &str = r#"
.legal-container {
    min-height: 100vh;
    display: flex;
    flex-direction: column;
    background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f3460 100%);
}

.legal-content {
    flex: 1;
    max-width: 900px;
    margin: 0 auto;
    padding: 40px 20px 60px;
    width: 100%;
}

.legal-content-inner {
    background: rgba(255, 255, 255, 0.98);
    border-radius: 16px;
    padding: 40px;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.3);
}

.legal-footer {
    border-top: 1px solid rgba(255,255,255,0.06);
    padding: 24px 20px;
    text-align: center;
    color: rgba(229,231,235,0.56);
    font-size: 13px;
}

.legal-footer a {
    color: rgba(229,231,235,0.72);
    text-decoration: none;
    margin: 0 8px;
}

.legal-footer a:hover {
    color: #a78bfa;
}

.github-link {
    display: inline-flex;
    align-items: center;
    gap: 6px;
}
"#;

#[component]
pub fn Terms() -> Element {
    rsx! {
        style { "{LEGAL_STYLE}" }

        div { class: "legal-container",
            MainNav { active: ActivePage::Other, subtitle: Some("Terms of Use"), show_nav_links: false }

            main { class: "legal-content",
                div {
                    class: "legal-content-inner",
                    dangerous_inner_html: "{TERMS_HTML}"
                }
            }

            footer { class: "legal-footer",
                span { "© 2025 Brahmastra Labs LLC" }
                span { " • " }
                a {
                    href: "https://github.com/Brahmastra-Labs/logicaffeine",
                    target: "_blank",
                    class: "github-link",
                    svg {
                        xmlns: "http://www.w3.org/2000/svg",
                        width: "14",
                        height: "14",
                        view_box: "0 0 24 24",
                        fill: "currentColor",
                        path {
                            d: "M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0024 12c0-6.63-5.37-12-12-12z"
                        }
                    }
                    "GitHub"
                }
                span { " • " }
                Link { to: Route::Privacy {}, "Privacy Policy" }
                span { " • " }
                Link { to: Route::Terms {}, "Terms of Use" }
            }
        }
    }
}
