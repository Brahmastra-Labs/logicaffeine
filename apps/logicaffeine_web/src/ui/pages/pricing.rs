//! Contact and licensing page.
//!
//! Displays product features, licensing information, and contact details.
//! No pricing is shown — interested parties are directed to reach out.
//!
//! # Route
//!
//! Accessed via [`Route::Pricing`].

use dioxus::prelude::*;
use crate::ui::router::Route;
use crate::ui::components::main_nav::{MainNav, ActivePage};
use crate::ui::components::footer::Footer;
use crate::ui::components::icon::{Icon, IconVariant, IconSize};
use crate::ui::seo::{JsonLdMultiple, PageHead, organization_schema, contact_schema, breadcrumb_schema, BreadcrumbItem, pages as seo_pages};

const PRICING_STYLE: &str = r#"
* { box-sizing: border-box; }
a { color: inherit; }

.pricing {
  height: 100vh;
  color: var(--text-primary);
  background:
    radial-gradient(1200px 600px at 50% -120px, rgba(var(--accent-secondary-rgb),0.18), transparent 60%),
    radial-gradient(900px 500px at 15% 30%, rgba(var(--accent-primary-rgb),0.18), transparent 60%),
    radial-gradient(800px 450px at 90% 45%, rgba(34,197,94,0.10), transparent 62%),
    linear-gradient(180deg, var(--bg-gradient-start), var(--bg-gradient-mid) 55%, var(--bg-gradient-end));
  overflow-x: hidden;
  overflow-y: auto;
  font-family: var(--font-sans);
  position: relative;
}

.bg-orb {
  position: absolute;
  inset: auto;
  width: 520px;
  height: 520px;
  border-radius: var(--radius-full);
  filter: blur(42px);
  opacity: 0.22;
  pointer-events: none;
  animation: float 14s ease-in-out infinite, pulse-glow 10s ease-in-out infinite;
}
.orb1 { top: -220px; left: -160px; background: radial-gradient(circle at 30% 30%, var(--accent-primary), transparent 60%); animation-delay: 0s; }
.orb2 { top: 120px; right: -200px; background: radial-gradient(circle at 40% 35%, var(--accent-secondary), transparent 60%); animation-delay: -5s; }
.orb3 { bottom: -260px; left: 20%; background: radial-gradient(circle at 40% 35%, rgba(34,197,94,0.9), transparent 60%); animation-delay: -10s; }

@keyframes float {
  0%, 100% { transform: translate3d(0, 0, 0); }
  50% { transform: translate3d(0, -20px, 0); }
}

@keyframes pulse-glow {
  0%, 100% { opacity: 0.22; }
  50% { opacity: 0.32; }
}

@keyframes fadeInUp {
  from { opacity: 0; transform: translateY(24px); }
  to { opacity: 1; transform: translateY(0); }
}

.pricing-container {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 60px var(--spacing-xl);
  max-width: 1000px;
  margin: 0 auto;
}

.pricing-header {
  text-align: center;
  margin-bottom: 50px;
  animation: fadeInUp 0.6s ease both;
}

.pricing-header h1 {
  font-size: var(--font-display-lg);
  font-weight: 900;
  letter-spacing: -2px;
  background: linear-gradient(180deg, #ffffff 0%, rgba(229,231,235,0.78) 65%, rgba(229,231,235,0.62) 100%);
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
  margin-bottom: var(--spacing-lg);
}

.pricing-header p {
  color: var(--text-secondary);
  font-size: var(--font-body-lg);
  line-height: 1.65;
}

.features-showcase {
  position: relative;
  background: rgba(255,255,255,0.04);
  border: 1px solid rgba(255,255,255,0.10);
  border-radius: var(--radius-xl);
  padding: 40px;
  margin-bottom: 40px;
  width: 100%;
  backdrop-filter: blur(18px);
  animation: fadeInUp 0.6s ease 0.08s both;
}

.features-showcase-header {
  text-align: center;
  margin-bottom: 32px;
}

.features-showcase-header h2 {
  color: var(--text-primary);
  font-size: var(--font-heading-lg);
  font-weight: 700;
  margin-bottom: var(--spacing-sm);
}

.features-showcase-header p {
  color: var(--text-secondary);
  font-size: var(--font-body-md);
  line-height: 1.65;
}

.features-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 18px;
}

.feature-group {
  position: relative;
  border-radius: var(--radius-xl);
  border: 1px solid rgba(255,255,255,0.08);
  background: rgba(255,255,255,0.03);
  padding: 18px;
  overflow: hidden;
  animation: fadeInUp 0.6s ease both;
}

.feature-group:nth-child(1) { animation-delay: 0.10s; box-shadow: 0 0 16px rgba(251,191,36,0.35), inset 0 0 12px rgba(251,191,36,0.08); border-color: rgba(251,191,36,0.45); }
.feature-group:nth-child(2) { animation-delay: 0.14s; box-shadow: 0 0 16px rgba(251,191,36,0.35), inset 0 0 12px rgba(251,191,36,0.08); border-color: rgba(251,191,36,0.45); }
.feature-group:nth-child(3) { animation-delay: 0.18s; box-shadow: 0 0 16px rgba(251,191,36,0.35), inset 0 0 12px rgba(251,191,36,0.08); border-color: rgba(251,191,36,0.45); }
.feature-group:nth-child(4) { animation-delay: 0.22s; box-shadow: 0 0 16px rgba(251,191,36,0.35), inset 0 0 12px rgba(251,191,36,0.08); border-color: rgba(251,191,36,0.45); }
.feature-group:nth-child(5) { animation-delay: 0.26s; box-shadow: 0 0 16px rgba(251,191,36,0.35), inset 0 0 12px rgba(251,191,36,0.08); border-color: rgba(251,191,36,0.45); }
.feature-group:nth-child(6) { animation-delay: 0.30s; box-shadow: 0 0 16px rgba(251,191,36,0.35), inset 0 0 12px rgba(251,191,36,0.08); border-color: rgba(251,191,36,0.45); }

.feature-group-header {
  display: flex;
  align-items: center;
  gap: var(--spacing-md);
  margin-bottom: var(--spacing-md);
}

.feature-group-icon {
  width: 40px;
  height: 40px;
  border-radius: var(--radius-lg);
  display: flex;
  align-items: center;
  justify-content: center;
  border: 1px solid rgba(255,255,255,0.10);
  flex-shrink: 0;
}

.feature-group-header h3 {
  color: var(--text-primary);
  font-size: var(--font-body-md);
  font-weight: 650;
}

.feature-group-list {
  list-style: none;
  padding: 0;
  margin: 0;
}

.feature-group-list li {
  color: var(--text-secondary);
  font-size: var(--font-caption-lg);
  padding: var(--spacing-xs) 0;
  padding-left: var(--spacing-xl);
  position: relative;
  line-height: 1.5;
}

.feature-group-list li::before {
  content: "\2713";
  position: absolute;
  left: 0;
  color: var(--accent-secondary);
}

.license-section {
  background: rgba(255,255,255,0.04);
  border: 1px solid rgba(255,255,255,0.10);
  border-radius: var(--radius-xl);
  padding: 40px;
  margin-bottom: 40px;
  width: 100%;
  backdrop-filter: blur(18px);
  animation: fadeInUp 0.6s ease 0.15s both;
}

.license-section h2 {
  color: var(--text-primary);
  font-size: var(--font-heading-lg);
  font-weight: 700;
  margin-bottom: var(--spacing-xl);
}

.license-section h3 {
  color: var(--accent-secondary);
  font-size: var(--font-body-lg);
  font-weight: 600;
  margin: var(--spacing-xl) 0 var(--spacing-md) 0;
}

.license-section p {
  color: var(--text-secondary);
  line-height: 1.8;
  margin-bottom: var(--spacing-lg);
}

.license-section ul {
  color: var(--text-secondary);
  line-height: 1.8;
  margin-left: var(--spacing-xl);
  margin-bottom: var(--spacing-lg);
}

.license-section li {
  margin-bottom: var(--spacing-sm);
}

.contact-section {
  background: linear-gradient(135deg, rgba(var(--accent-primary-rgb),0.08) 0%, rgba(var(--accent-secondary-rgb),0.08) 100%);
  border: 1px solid rgba(var(--accent-secondary-rgb),0.25);
  border-radius: var(--radius-xl);
  padding: 40px;
  text-align: center;
  width: 100%;
  backdrop-filter: blur(18px);
  animation: fadeInUp 0.6s ease 0.2s both;
}

.contact-section h2 {
  color: var(--text-primary);
  font-size: var(--font-heading-lg);
  font-weight: 700;
  margin-bottom: var(--spacing-lg);
}

.contact-section p {
  color: var(--text-secondary);
  margin-bottom: var(--spacing-xl);
  line-height: 1.65;
}

.contact-links {
  display: flex;
  gap: var(--spacing-lg);
  justify-content: center;
  flex-wrap: wrap;
}

.contact-email {
  display: inline-block;
  background: linear-gradient(135deg, var(--accent-primary), var(--accent-secondary));
  color: #060814;
  padding: var(--spacing-md) var(--spacing-xxl);
  border-radius: var(--radius-lg);
  font-size: var(--font-body-md);
  font-weight: 650;
  text-decoration: none;
  transition: all 0.2s ease;
  box-shadow: 0 18px 40px rgba(var(--accent-primary-rgb),0.18);
}

.contact-email:hover {
  transform: translateY(-2px);
  box-shadow: 0 6px 20px rgba(var(--accent-primary-rgb),0.4);
}

.email-display {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--spacing-md);
  margin-top: var(--spacing-lg);
}

.email-text {
  color: var(--text-secondary);
  font-size: var(--font-body-md);
  font-family: var(--font-mono, monospace);
  user-select: all;
}

.copy-btn {
  display: inline-flex;
  align-items: center;
  gap: var(--spacing-sm);
  background: rgba(255,255,255,0.06);
  border: 1px solid rgba(255,255,255,0.12);
  border-radius: var(--radius-md);
  padding: var(--spacing-sm) var(--spacing-md);
  color: var(--text-secondary);
  font-size: var(--font-caption-lg);
  font-weight: 600;
  cursor: pointer;
  transition: all 0.2s ease;
}

.copy-btn:hover {
  background: rgba(255,255,255,0.10);
  color: var(--text-primary);
  border-color: rgba(255,255,255,0.18);
}

.copy-btn.copied {
  background: rgba(34,197,94,0.15);
  border-color: rgba(34,197,94,0.4);
  color: var(--color-success);
}

.back-link {
  background: rgba(255,255,255,0.05);
  border: 1px solid rgba(255,255,255,0.10);
  border-radius: var(--radius-lg);
  padding: var(--spacing-md) var(--spacing-xl);
  color: var(--text-secondary);
  font-size: var(--font-body-sm);
  font-weight: 600;
  cursor: pointer;
  transition: all 0.2s ease;
}

.back-link:hover {
  background: rgba(255,255,255,0.08);
  color: var(--text-primary);
  border-color: rgba(255,255,255,0.14);
}

.pricing-footer-links {
  display: flex;
  gap: var(--spacing-md);
  align-items: center;
  margin-top: 40px;
}

.github-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--spacing-sm);
  background: rgba(255,255,255,0.05);
  border: 1px solid rgba(255,255,255,0.10);
  border-radius: var(--radius-lg);
  padding: var(--spacing-md) var(--spacing-xl);
  color: var(--text-secondary);
  font-size: var(--font-body-sm);
  font-weight: 600;
  text-decoration: none;
  transition: all 0.2s ease;
}

.github-btn:hover {
  background: rgba(255,255,255,0.08);
  color: var(--text-primary);
  border-color: rgba(255,255,255,0.14);
}

.github-btn svg {
  width: 18px;
  height: 18px;
  fill: currentColor;
}

@media (max-width: 700px) {
  .pricing-header h1 {
    font-size: var(--font-display-md);
  }
  .features-grid {
    grid-template-columns: 1fr;
  }
}

@media (prefers-reduced-motion: reduce) {
  * { transition: none !important; animation: none !important; }
}
"#;

fn copy_to_clipboard(text: &str) {
    if let Some(window) = web_sys::window() {
        let clipboard = window.navigator().clipboard();
        let _ = clipboard.write_text(text);
    }
}

const EMAIL: &str = "tristen@brahmastra-labs.com";

#[component]
pub fn Pricing() -> Element {
    let mut copied = use_signal(|| false);

    let on_copy = move |_| {
        copy_to_clipboard(EMAIL);
        copied.set(true);
    };

    let breadcrumbs = vec![
        BreadcrumbItem { name: "Home", path: "/" },
        BreadcrumbItem { name: "Contact", path: "/pricing" },
    ];

    let schemas = vec![
        organization_schema(),
        contact_schema(),
        breadcrumb_schema(&breadcrumbs),
    ];

    rsx! {
        PageHead {
            title: seo_pages::PRICING.title,
            description: seo_pages::PRICING.description,
            canonical_path: seo_pages::PRICING.canonical_path,
        }
        style { "{PRICING_STYLE}" }
        JsonLdMultiple { schemas }

        div { class: "pricing",
            div { class: "bg-orb orb1" }
            div { class: "bg-orb orb2" }
            div { class: "bg-orb orb3" }

            MainNav { active: ActivePage::Pricing, subtitle: Some("Contact") }

            div { class: "pricing-container",
                div { class: "pricing-header",
                    h1 { "Contact Us" }
                    p { "Free for individuals, educational institutions, and teams under 25 people." }
                    p { "For commercial licensing, partnerships, or enterprise needs \u{2014} get in touch." }
                }

                div { class: "contact-section",
                    h2 { "Let\u{2019}s Talk" }
                    p { "Interested in LOGOS for your organization? Reach out and we\u{2019}ll get back to you." }
                    div { class: "contact-links",
                        a {
                            class: "contact-email",
                            href: "mailto:tristen@brahmastra-labs.com",
                            "Send Email"
                        }
                    }
                    div { class: "email-display",
                        span { class: "email-text", "{EMAIL}" }
                        button {
                            class: if *copied.read() { "copy-btn copied" } else { "copy-btn" },
                            onclick: on_copy,
                            if *copied.read() { "Copied!" } else { "Copy" }
                        }
                    }
                }

                div { class: "license-section",
                    h2 { "Business Source License" }

                    p {
                        "LOGOS is released under the Business Source License 1.1. The source code is "
                        "publicly available, and the software is free to use for individuals and small teams."
                    }

                    h3 { "Free Use" }
                    p { "You may use LOGOS at no cost if you are:" }
                    ul {
                        li { "An individual" }
                        li { "A university or educational institution" }
                        li { "An organization with fewer than 25 employees" }
                    }

                    h3 { "Commercial License Required" }
                    p {
                        "If your organization has 25 or more employees and you wish to use "
                        "LOGOS as a Logic Service, a commercial license is required. "
                        "Contact us to discuss licensing options for your organization."
                    }

                    h3 { "Open Source Transition" }
                    p {
                        "On December 24, 2029, LOGOS will transition to the MIT License, "
                        "making it fully open source."
                    }
                }

                div { class: "features-showcase",
                    div { class: "features-showcase-header",
                        h2 { "What You Get" }
                        p { "One language for code, logic, math, hardware verification, and distributed systems" }
                    }
                    div { class: "features-grid",

                        div { class: "feature-group",
                            div { class: "feature-group-header",
                                div {
                                    class: "feature-group-icon",
                                    style: "background: rgba(0,212,255,0.15);",
                                    Icon { variant: IconVariant::Lightning, size: IconSize::Large, color: "#00d4ff" }
                                }
                                h3 { "English-Native Programming" }
                            }
                            ul { class: "feature-group-list",
                                li { "Programs, proofs, and logic in plain English" }
                                li { "Compiles to native Rust via LLVM" }
                                li { "Ownership and borrowing in natural language" }
                            }
                        }

                        div { class: "feature-group",
                            div { class: "feature-group-header",
                                div {
                                    class: "feature-group-icon",
                                    style: "background: rgba(129,140,248,0.15);",
                                    Icon { variant: IconVariant::Brain, size: IconSize::Large, color: "#818cf8" }
                                }
                                h3 { "Formal Logic & Semantics" }
                            }
                            ul { class: "feature-group-list",
                                li { "English \u{2192} First-Order Logic transpilation" }
                                li { "Quantifier disambiguation with all scope readings" }
                                li { "Discourse tracking and anaphora resolution" }
                            }
                        }

                        div { class: "feature-group",
                            div { class: "feature-group-header",
                                div {
                                    class: "feature-group-icon",
                                    style: "background: rgba(34,197,94,0.15);",
                                    Icon { variant: IconVariant::Beaker, size: IconSize::Large, color: "#22c55e" }
                                }
                                h3 { "Type Theory & Proofs" }
                            }
                            ul { class: "feature-group-list",
                                li { "Calculus of Constructions with dependent types" }
                                li { "Automated tactics: induction, ring, simp, omega" }
                                li { "Kernel-verified proof certification" }
                            }
                        }

                        div { class: "feature-group",
                            div { class: "feature-group-header",
                                div {
                                    class: "feature-group-icon",
                                    style: "background: rgba(245,158,11,0.15);",
                                    Icon { variant: IconVariant::Beaker, size: IconSize::Large, color: "#f59e0b" }
                                }
                                h3 { "Hardware Verification & SVA" }
                            }
                            ul { class: "feature-group-list",
                                li { "English specs to SystemVerilog Assertions" }
                                li { "FOL-to-SVA synthesis via Futamura projections" }
                                li { "Bounded model checking and CEGAR refinement" }
                                li { "AXI4, APB, and handshake protocol templates" }
                            }
                        }

                        div { class: "feature-group",
                            div { class: "feature-group-header",
                                div {
                                    class: "feature-group-icon",
                                    style: "background: rgba(251,191,36,0.15);",
                                    Icon { variant: IconVariant::Sparkles, size: IconSize::Large, color: "#fbbf24" }
                                }
                                h3 { "Distributed Systems & CRDTs" }
                            }
                            ul { class: "feature-group-list",
                                li { "GCounter, PNCounter, ORSet, RGA, ORMap" }
                                li { "libp2p networking with GossipSub" }
                                li { "Automatic merge and eventual consistency" }
                            }
                        }

                        div { class: "feature-group",
                            div { class: "feature-group-header",
                                div {
                                    class: "feature-group-icon",
                                    style: "background: rgba(236,72,153,0.15);",
                                    Icon { variant: IconVariant::Shield, size: IconSize::Large, color: "#ec4899" }
                                }
                                h3 { "Security & Concurrency" }
                            }
                            ul { class: "feature-group-list",
                                li { "Capability-based access control" }
                                li { "CSP channels and structured concurrency" }
                                li { "Arena allocation \u{2014} no garbage collector" }
                            }
                        }

                        div { class: "feature-group",
                            div { class: "feature-group-header",
                                div {
                                    class: "feature-group-icon",
                                    style: "background: rgba(139,92,246,0.15);",
                                    Icon { variant: IconVariant::Lock, size: IconSize::Large, color: "#8b5cf6" }
                                }
                                h3 { "Verification & Tooling" }
                            }
                            ul { class: "feature-group-list",
                                li { "Z3 SMT solver for static verification" }
                                li { "Interactive Studio with live compilation" }
                                li { "CLI tool (largo) and WASM runtime" }
                            }
                        }
                    }
                }

                div { class: "pricing-footer-links",
                    a {
                        href: "https://github.com/Brahmastra-Labs/logicaffeine",
                        target: "_blank",
                        class: "github-btn",
                        svg {
                            xmlns: "http://www.w3.org/2000/svg",
                            view_box: "0 0 24 24",
                            path {
                                d: "M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0024 12c0-6.63-5.37-12-12-12z"
                            }
                        }
                        "GitHub"
                    }
                    Link {
                        class: "back-link",
                        to: Route::Landing {},
                        "\u{2190} Back"
                    }
                }
            }

            Footer {}
        }
    }
}
