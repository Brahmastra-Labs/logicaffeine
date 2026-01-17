//! Crate documentation landing page.
//!
//! Displays a grid of crate cards with descriptions, linking to rustdoc HTML.
//! Each card links directly to `/crates/{crate_name}/index.html`.

use dioxus::prelude::*;
use crate::ui::components::main_nav::{MainNav, ActivePage};

const CRATES_STYLE: &str = r#"
.crates-page {
    min-height: 100vh;
    color: var(--text-primary);
    background:
        radial-gradient(1200px 600px at 50% -120px, rgba(167,139,250,0.14), transparent 60%),
        radial-gradient(900px 500px at 15% 30%, rgba(96,165,250,0.14), transparent 60%),
        radial-gradient(800px 450px at 90% 45%, rgba(34,197,94,0.08), transparent 62%),
        linear-gradient(180deg, #070a12, #0b1022 55%, #070a12);
    font-family: var(--font-sans);
}

.crates-hero {
    max-width: 1280px;
    margin: 0 auto;
    padding: 60px var(--spacing-xl) 40px;
}

.crates-hero h1 {
    font-size: var(--font-display-lg);
    font-weight: 900;
    letter-spacing: -1.5px;
    line-height: 1.1;
    background: linear-gradient(180deg, #ffffff 0%, rgba(229,231,235,0.78) 65%, rgba(229,231,235,0.62) 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    margin: 0 0 var(--spacing-lg);
}

.crates-hero p {
    font-size: var(--font-body-lg);
    color: var(--text-secondary);
    max-width: 600px;
    line-height: 1.6;
    margin: 0;
}

.crates-hero-badge {
    display: inline-flex;
    align-items: center;
    gap: var(--spacing-sm);
    padding: var(--spacing-sm) 14px;
    border-radius: var(--radius-full);
    background: rgba(255,255,255,0.06);
    border: 1px solid rgba(255,255,255,0.10);
    font-size: var(--font-caption-md);
    font-weight: 600;
    color: var(--text-primary);
    margin-bottom: var(--spacing-xl);
}

.crates-hero-badge .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--color-accent-purple);
    box-shadow: 0 0 0 4px rgba(167,139,250,0.15);
}

.crates-content {
    max-width: 1280px;
    margin: 0 auto;
    padding: 0 var(--spacing-xl) 80px;
}

.crates-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(340px, 1fr));
    gap: 24px;
}

.crate-card {
    display: block;
    background: rgba(255,255,255,0.03);
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: var(--radius-xl);
    padding: 24px;
    text-decoration: none;
    color: inherit;
    transition: transform 0.2s, border-color 0.2s, background 0.2s, box-shadow 0.2s;
}

.crate-card:hover {
    transform: translateY(-2px);
    border-color: rgba(167,139,250,0.4);
    background: rgba(255,255,255,0.05);
    box-shadow: 0 12px 30px rgba(167,139,250,0.08);
}

.crate-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    margin-bottom: 12px;
}

.crate-name {
    font-size: 18px;
    font-weight: 700;
    color: #fff;
    margin: 0;
    font-family: var(--font-mono);
    display: flex;
    align-items: center;
    gap: 8px;
}

.crate-badge {
    display: inline-flex;
    align-items: center;
    font-size: 11px;
    font-weight: 700;
    padding: 3px 8px;
    border-radius: 999px;
}

.crate-badge.core {
    background: linear-gradient(135deg, rgba(96,165,250,0.9), rgba(167,139,250,0.9));
    color: #060814;
}

.crate-badge.app {
    background: rgba(34,197,94,0.2);
    color: #22c55e;
    border: 1px solid rgba(34,197,94,0.3);
}

.crate-description {
    color: var(--text-secondary);
    font-size: 14px;
    line-height: 1.6;
    margin: 0;
}

.crate-arrow {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    margin-top: 16px;
    color: var(--text-tertiary);
    font-size: 13px;
    gap: 6px;
    transition: color 0.2s;
}

.crate-card:hover .crate-arrow {
    color: var(--color-accent-purple);
}

.crate-arrow svg {
    width: 16px;
    height: 16px;
    transition: transform 0.2s;
}

.crate-card:hover .crate-arrow svg {
    transform: translateX(3px);
}

.crates-section {
    margin-bottom: 48px;
}

.crates-section-title {
    font-size: var(--font-body-md);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 1.5px;
    color: var(--text-tertiary);
    margin: 0 0 24px;
    padding-bottom: var(--spacing-md);
    border-bottom: 1px solid rgba(255,255,255,0.08);
}

@media (max-width: 768px) {
    .crates-grid {
        grid-template-columns: 1fr;
    }

    .crates-hero h1 {
        font-size: var(--font-display-md);
    }

    .crates-hero {
        padding: 40px var(--spacing-xl) var(--spacing-xxl);
    }
}
"#;

#[derive(Clone, Copy, PartialEq)]
struct CrateInfo {
    name: &'static str,
    description: &'static str,
    is_core: bool,
}

const CORE_CRATES: &[CrateInfo] = &[
    CrateInfo {
        name: "logicaffeine_language",
        description: "Core LOGOS language implementation including AST, parser, and first-order logic transpiler.",
        is_core: true,
    },
    CrateInfo {
        name: "logicaffeine_compile",
        description: "Code generation and compilation from LOGOS to executable formats.",
        is_core: true,
    },
    CrateInfo {
        name: "logicaffeine_kernel",
        description: "Runtime kernel for executing LOGOS programs with built-in inference.",
        is_core: true,
    },
    CrateInfo {
        name: "logicaffeine_proof",
        description: "Proof assistant and formal verification engine for logical reasoning.",
        is_core: true,
    },
    CrateInfo {
        name: "logicaffeine_lexicon",
        description: "Vocabulary database and lexical analysis for natural language parsing.",
        is_core: true,
    },
    CrateInfo {
        name: "logicaffeine_base",
        description: "Shared base types, traits, and utilities used across all crates.",
        is_core: true,
    },
    CrateInfo {
        name: "logicaffeine_data",
        description: "Data structures and persistent storage for programs and proofs.",
        is_core: true,
    },
    CrateInfo {
        name: "logicaffeine_system",
        description: "System integration layer for platform-specific functionality.",
        is_core: true,
    },
];

const APP_CRATES: &[CrateInfo] = &[
    CrateInfo {
        name: "logicaffeine_cli",
        description: "Command-line interface for the LOGOS toolchain (largo).",
        is_core: false,
    },
    CrateInfo {
        name: "logicaffeine_web",
        description: "Web frontend components built with Dioxus for the learning platform.",
        is_core: false,
    },
];

#[component]
pub fn Crates() -> Element {
    rsx! {
        style { "{CRATES_STYLE}" }

        div { class: "crates-page",
            MainNav {
                active: ActivePage::Crates,
                subtitle: Some("Crate Documentation"),
            }

            header { class: "crates-hero",
                div { class: "crates-hero-badge",
                    div { class: "dot" }
                    span { "Rustdoc" }
                }
                h1 { "Crate Documentation" }
                p {
                    "Explore the API documentation for all LOGICAFFEINE crates. Each crate is documented with examples and detailed type information."
                }
            }

            main { class: "crates-content",
                section { class: "crates-section",
                    h2 { class: "crates-section-title", "Core Crates" }
                    div { class: "crates-grid",
                        for crate_info in CORE_CRATES.iter() {
                            CrateCard { info: crate_info }
                        }
                    }
                }

                section { class: "crates-section",
                    h2 { class: "crates-section-title", "Application Crates" }
                    div { class: "crates-grid",
                        for crate_info in APP_CRATES.iter() {
                            CrateCard { info: crate_info }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn CrateCard(info: &'static CrateInfo) -> Element {
    let doc_url = format!("https://docs.logicaffeine.com/{}/index.html", info.name);

    rsx! {
        a {
            class: "crate-card",
            href: "{doc_url}",
            target: "_blank",
            rel: "noopener",
            div { class: "crate-header",
                h3 { class: "crate-name",
                    "{info.name}"
                }
                span {
                    class: if info.is_core { "crate-badge core" } else { "crate-badge app" },
                    if info.is_core { "Core" } else { "App" }
                }
            }
            p { class: "crate-description", "{info.description}" }
            div { class: "crate-arrow",
                "View docs"
                svg {
                    xmlns: "http://www.w3.org/2000/svg",
                    fill: "none",
                    view_box: "0 0 24 24",
                    stroke: "currentColor",
                    stroke_width: "2",
                    path {
                        stroke_linecap: "round",
                        stroke_linejoin: "round",
                        d: "M13 7l5 5m0 0l-5 5m5-5H6"
                    }
                }
            }
        }
    }
}
