use dioxus::prelude::*;
use crate::ui::router::Route;

const PRIVACY_HTML: &str = include_str!("../../../privacy.html");

const LEGAL_STYLE: &str = r#"
.legal-container {
    min-height: 100vh;
    display: flex;
    flex-direction: column;
    background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f3460 100%);
}

.legal-nav {
    position: sticky;
    top: 0;
    z-index: 50;
    backdrop-filter: blur(18px);
    background: linear-gradient(180deg, rgba(7,10,18,0.72), rgba(7,10,18,0.44));
    border-bottom: 1px solid rgba(255,255,255,0.06);
    padding: 16px 20px;
}

.legal-nav-inner {
    max-width: 1120px;
    margin: 0 auto;
    display: flex;
    align-items: center;
    justify-content: space-between;
}

.legal-brand {
    display: flex;
    align-items: center;
    gap: 12px;
    text-decoration: none;
    color: #e5e7eb;
}

.legal-logo {
    width: 36px;
    height: 36px;
    border-radius: 12px;
    background:
        radial-gradient(circle at 30% 30%, rgba(96,165,250,0.85), transparent 55%),
        radial-gradient(circle at 65% 60%, rgba(167,139,250,0.85), transparent 55%),
        rgba(255,255,255,0.06);
    border: 1px solid rgba(255,255,255,0.10);
}

.legal-brand-name {
    font-weight: 800;
    font-size: 14px;
    letter-spacing: -0.5px;
}

.legal-back {
    color: #a78bfa;
    text-decoration: none;
    font-size: 14px;
    padding: 8px 16px;
    border-radius: 8px;
    border: 1px solid rgba(167,139,250,0.3);
    transition: all 0.2s ease;
}

.legal-back:hover {
    background: rgba(167,139,250,0.1);
    border-color: rgba(167,139,250,0.5);
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

.legal-nav-links {
    display: flex;
    align-items: center;
    gap: 12px;
}

.legal-github {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    border-radius: 8px;
    border: 1px solid rgba(255,255,255,0.1);
    background: rgba(255,255,255,0.03);
    color: rgba(229,231,235,0.72);
    transition: all 0.2s ease;
}

.legal-github:hover {
    background: rgba(255,255,255,0.08);
    color: #e5e7eb;
    border-color: rgba(255,255,255,0.2);
}

.legal-github svg {
    width: 18px;
    height: 18px;
    fill: currentColor;
}

.github-link {
    display: inline-flex;
    align-items: center;
    gap: 6px;
}
"#;

#[component]
pub fn Privacy() -> Element {
    rsx! {
        style { "{LEGAL_STYLE}" }

        div { class: "legal-container",
            nav { class: "legal-nav",
                div { class: "legal-nav-inner",
                    Link {
                        to: Route::Landing {},
                        class: "legal-brand",
                        div { class: "legal-logo" }
                        span { class: "legal-brand-name", "LOGICAFFEINE" }
                    }
                    div { class: "legal-nav-links",
                        a {
                            href: "https://github.com/Brahmastra-Labs/logicaffeine",
                            target: "_blank",
                            class: "legal-github",
                            title: "View on GitHub",
                            svg {
                                xmlns: "http://www.w3.org/2000/svg",
                                view_box: "0 0 24 24",
                                path {
                                    d: "M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0024 12c0-6.63-5.37-12-12-12z"
                                }
                            }
                        }
                        Link {
                            to: Route::Landing {},
                            class: "legal-back",
                            "← Back to Home"
                        }
                    }
                }
            }

            main { class: "legal-content",
                div {
                    class: "legal-content-inner",
                    dangerous_inner_html: "{PRIVACY_HTML}"
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
