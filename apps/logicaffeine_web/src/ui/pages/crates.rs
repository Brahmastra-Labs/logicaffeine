//! Crate documentation landing page.
//!
//! Displays a grid of crate cards with descriptions, linking to rustdoc HTML.
//! Each card links directly to `/crates/{crate_name}/index.html`.

use crate::ui::components::footer::Footer;
use crate::ui::components::main_nav::{ActivePage, MainNav};
use crate::ui::seo::{
    breadcrumb_schema, organization_schema, pages as seo_pages, tech_article_schema,
    BreadcrumbItem, JsonLdMultiple, PageHead,
};
use dioxus::prelude::*;
#[cfg(all(feature = "split", target_arch = "wasm32"))]
use dioxus::wasm_split;

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

.crates-hero .crates-hero-version {
    margin-top: var(--spacing-md);
    font-size: 0.95rem;
    opacity: 0.75;
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

.crate-links {
    display: flex;
    gap: 12px;
    margin-top: 16px;
    padding-top: 16px;
    border-top: 1px solid rgba(255,255,255,0.06);
}

.crate-link {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 14px;
    border-radius: var(--radius-md);
    font-size: var(--font-caption-md);
    font-weight: 600;
    text-decoration: none;
    transition: all 0.2s ease;
}

.crate-link.docs {
    background: rgba(167,139,250,0.15);
    color: #a78bfa;
    border: 1px solid rgba(167,139,250,0.3);
}

.crate-link.docs:hover {
    background: rgba(167,139,250,0.25);
    border-color: rgba(167,139,250,0.5);
}

.crate-link.crates-io {
    background: rgba(249,115,22,0.15);
    color: #fb923c;
    border: 1px solid rgba(249,115,22,0.3);
}

.crate-link.crates-io:hover {
    background: rgba(249,115,22,0.25);
    border-color: rgba(249,115,22,0.5);
}

.crate-link svg {
    width: 16px;
    height: 16px;
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

/// The workspace version every crate on the page is published at.
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, PartialEq)]
struct CrateInfo {
    name: &'static str,
    description: &'static str,
    badge: &'static str,
    crates_io: Option<&'static str>,
}

/// The language, the compiler and its execution tiers, the proof kernel and the platform, in the order the
/// docs site's landing page groups them.
const CORE_CRATES: &[CrateInfo] = &[
    CrateInfo {
        name: "logicaffeine_language",
        description: "The LOGOS front end: English sentences to first-order logic, and the parser for LOGOS programs and LGX pages",
        badge: "Core",
        crates_io: Some("https://crates.io/crates/logicaffeine-language"),
    },
    CrateInfo {
        name: "logicaffeine_lexicon",
        description: "The English vocabulary behind LOGOS: verb, noun and adjective metadata for build-time lookup tables, plus an optional runtime JSON index",
        badge: "Core",
        crates_io: Some("https://crates.io/crates/logicaffeine-lexicon"),
    },
    CrateInfo {
        name: "logicaffeine_compile",
        description: "The LOGOS compiler: type checking, the optimizer, the interpreter and bytecode VM, and the Rust, C, WASM, RISC-V, CUDA and hardware backends",
        badge: "Core",
        crates_io: Some("https://crates.io/crates/logicaffeine-compile"),
    },
    CrateInfo {
        name: "logicaffeine_runtime",
        description: "Deterministic, replayable concurrency runtime for the LOGOS interpreter and VM: task scheduler, channels, Select, timers, seed and trace",
        badge: "Core",
        crates_io: Some("https://crates.io/crates/logicaffeine-runtime"),
    },
    CrateInfo {
        name: "logicaffeine_forge",
        description: "The machine-code layer of the LOGOS copy-and-patch JIT: W^X executable memory, build-time stencils, a register-allocating x86-64 backend",
        badge: "Core",
        crates_io: Some("https://crates.io/crates/logicaffeine-forge"),
    },
    CrateInfo {
        name: "logicaffeine_jit",
        description: "Native execution tier for the LOGOS bytecode VM: hot functions and loops tier up to x86-64 machine code",
        badge: "Core",
        crates_io: Some("https://crates.io/crates/logicaffeine-jit"),
    },
    CrateInfo {
        name: "logicaffeine_kernel",
        description: "Calculus of Constructions proof kernel with inductive types, universes, quotients, a second checker and Lean 4 import",
        badge: "Core",
        crates_io: Some("https://crates.io/crates/logicaffeine-kernel"),
    },
    CrateInfo {
        name: "logicaffeine_proof",
        description: "Kernel-checked proof search and tactics, certified SAT/CDCL with DRAT/LRAT output, and a number-theory and cryptanalysis library",
        badge: "Core",
        crates_io: Some("https://crates.io/crates/logicaffeine-proof"),
    },
    CrateInfo {
        name: "logicaffeine_base",
        description: "Foundation types for Logicaffeine: arenas, string interning, spans, and exact integers, decimals, quantities, money, dates and UUIDs",
        badge: "Core",
        crates_io: Some("https://crates.io/crates/logicaffeine-base"),
    },
    CrateInfo {
        name: "logicaffeine_data",
        description: "LOGOS runtime values, collections, wire format and CRDTs, shared by native and WASM builds",
        badge: "Core",
        crates_io: Some("https://crates.io/crates/logicaffeine-data"),
    },
    CrateInfo {
        name: "logicaffeine_system",
        description: "Platform IO, networking, HTTP serving, persistence, and post-quantum crypto (Keccak/SHA-3, ChaCha20-Poly1305, ML-KEM, ML-DSA) for LOGOS",
        badge: "Core",
        crates_io: Some("https://crates.io/crates/logicaffeine-system"),
    },
];

const VERIFY_CRATES: &[CrateInfo] = &[
    CrateInfo {
        name: "logicaffeine_verify",
        description: "Z3-based static verification and model checking for Logicaffeine",
        badge: "Z3",
        crates_io: Some("https://crates.io/crates/logicaffeine-verify"),
    },
    CrateInfo {
        name: "logicaffeine_tv",
        description: "Translation validation for LOGOS: a symbolic encoder of the source, checked with Z3 against the tree-walking interpreter per program",
        badge: "Z3",
        crates_io: Some("https://crates.io/crates/logicaffeine-tv"),
    },
];

const APP_CRATES: &[CrateInfo] = &[
    CrateInfo {
        name: "logicaffeine_cli",
        description: "largo — the LOGOS build tool: projects, builds, tests, serving, proofs, and the package registry",
        badge: "Tool",
        crates_io: Some("https://crates.io/crates/logicaffeine-cli"),
    },
    CrateInfo {
        name: "logicaffeine_lsp",
        description: "The LOGOS language server (LSP): diagnostics, completion, hover, navigation and refactoring for .lg files, literate .md files and LGX pages",
        badge: "Tool",
        crates_io: Some("https://crates.io/crates/logicaffeine-lsp"),
    },
    CrateInfo {
        name: "logicaffeine_web",
        description: "The logicaffeine.com site (Dioxus/WASM): LOGOS Studio with live LGX previews, Learn, Syntax Guide, benchmarks, news and the registry browser",
        badge: "Tool",
        crates_io: None,
    },
];

#[component(lazy)]
pub fn Crates() -> Element {
    let breadcrumbs = vec![
        BreadcrumbItem {
            name: "Home",
            path: "/",
        },
        BreadcrumbItem {
            name: "Crates",
            path: "/crates",
        },
    ];
    let schemas = vec![
        organization_schema(),
        tech_article_schema("LOGICAFFEINE Crate Documentation", "The Rust crates behind LOGOS: the English-to-logic front end, the compiler and its execution tiers, the proof kernel, and the tools, with API docs and crates.io links.", "/crates"),
        breadcrumb_schema(&breadcrumbs),
    ];

    rsx! {
        PageHead {
            title: seo_pages::CRATES.title,
            description: seo_pages::CRATES.description,
            canonical_path: seo_pages::CRATES.canonical_path,
        }
        style { "{CRATES_STYLE}" }
        JsonLdMultiple { schemas }

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
                h1 { "LOGOS Crates" }
                p {
                    "LOGOS is a workspace of Rust crates you can use on their own: parse English into logic, compile and run LOGOS programs, or check a proof with the kernel. Each card links to the crate's API documentation and its crates.io page."
                }
                p { class: "crates-hero-version", "Version {VERSION}" }
            }

            main { class: "crates-content",
                section { class: "crates-section",
                    h2 { class: "crates-section-title", "Language, compiler and kernel" }
                    div { class: "crates-grid",
                        for crate_info in CORE_CRATES.iter() {
                            CrateCard { info: crate_info }
                        }
                    }
                }

                section { class: "crates-section",
                    h2 { class: "crates-section-title", "Z3 verification" }
                    div { class: "crates-grid",
                        for crate_info in VERIFY_CRATES.iter() {
                            CrateCard { info: crate_info }
                        }
                    }
                }

                section { class: "crates-section",
                    h2 { class: "crates-section-title", "Tools" }
                    div { class: "crates-grid",
                        for crate_info in APP_CRATES.iter() {
                            CrateCard { info: crate_info }
                        }
                    }
                }
            }

            Footer {}
        }
    }
}

#[component]
fn CrateCard(info: &'static CrateInfo) -> Element {
    let doc_url = format!("https://docs.logicaffeine.com/{}/index.html", info.name);

    rsx! {
        div { class: "crate-card",
            div { class: "crate-header",
                h3 { class: "crate-name",
                    "{info.name}"
                }
                span {
                    class: if info.badge == "Core" { "crate-badge core" } else { "crate-badge app" },
                    "{info.badge}"
                }
            }
            p { class: "crate-description", "{info.description}" }
            div { class: "crate-links",
                a {
                    class: "crate-link docs",
                    href: "{doc_url}",
                    target: "_blank",
                    rel: "noopener",
                    svg {
                        xmlns: "http://www.w3.org/2000/svg",
                        fill: "none",
                        view_box: "0 0 24 24",
                        stroke: "currentColor",
                        stroke_width: "2",
                        path {
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                            d: "M12 6.253v13m0-13C10.832 5.477 9.246 5 7.5 5S4.168 5.477 3 6.253v13C4.168 18.477 5.754 18 7.5 18s3.332.477 4.5 1.253m0-13C13.168 5.477 14.754 5 16.5 5c1.747 0 3.332.477 4.5 1.253v13C19.832 18.477 18.247 18 16.5 18c-1.746 0-3.332.477-4.5 1.253"
                        }
                    }
                    "Docs"
                }
                if let Some(crates_url) = info.crates_io {
                    a {
                        class: "crate-link crates-io",
                        href: "{crates_url}",
                        target: "_blank",
                        rel: "noopener",
                        svg {
                            xmlns: "http://www.w3.org/2000/svg",
                            fill: "none",
                            view_box: "0 0 24 24",
                            stroke: "currentColor",
                            stroke_width: "2",
                            path {
                                stroke_linecap: "round",
                                stroke_linejoin: "round",
                                d: "M20 7l-8-4-8 4m16 0l-8 4m8-4v10l-8 4m0-10L4 7m8 4v10M4 7v10l8 4"
                            }
                        }
                        "crates.io"
                    }
                }
            }
        }
    }
}
