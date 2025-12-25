use dioxus::prelude::*;
use crate::ui::router::Route;

// (label, english, simple_fol, unicode)
const MILESTONE_EXAMPLES: &[&[(&str, &str, &str, &str)]] = &[
    // Phase 1: Core Transpiler
    &[
        ("Universal", "Every user who has a key enters the room.",
            "For all x: User(x) and HasKey(x) implies Enter(x, Room)",
            "∀x((User(x) ∧ HasKey(x)) → Enter(x, Room))"),
        ("Conditional", "If a user enters the room, the alarm triggers.",
            "Enter(User, Room) implies Trigger(Alarm)",
            "(Enter(User, Room) → Trigger(Alarm))"),
        ("Negation", "No user who lacks a key can enter.",
            "For all x: User(x) and LacksKey(x) implies not Enter(x)",
            "∀x((User(x) ∧ LacksKey(x)) → ¬Enter(x))"),
    ],
    // Phase 2: Web Platform
    &[
        ("Interactive", "Check that the answer equals expected.",
            "Assert: answer equals expected",
            "Assert(Eq(answer, expected))"),
        ("Feedback", "Show the hint to the learner.",
            "Display hint to learner",
            "Display(hint, learner)"),
    ],
    // Phase 3: Codegen Pipeline
    &[
        ("Hello World", "To run:\n    Show \"Hello, World!\" to the console.",
            "fn main() {\n    println!(\"Hello, World!\");\n}",
            "fn main() -> Result<(), Error> {\n    println!(\"Hello, World!\");\n    Ok(())\n}"),
        ("Binding", "Let result be the factorial of 10.",
            "let result = factorial(10);",
            "let result: u64 = factorial(10);"),
    ],
    // Phase 4: Type System
    &[
        ("Refinement", "Let age be an Integer where age > 0.",
            "Age = Int where value > 0",
            "type Age = { n: Int | n > 0 }"),
        ("Dependent", "A Vector of n elements.",
            "Vec<T, n: Nat>",
            "struct Vec<T, const N: usize>"),
    ],
    // Phase 5: Proof System
    &[
        ("Theorem", "The factorial terminates for all naturals.",
            "For all n in Nat: terminates(factorial(n))",
            "∀n:ℕ. terminates(factorial(n))"),
        ("Proof", "By structural induction on n. Auto.",
            "Proof: induction on n. QED",
            "induction(n); auto. QED"),
    ],
    // Phase 6: Concurrency
    &[
        ("Parallel", "Attempt all of the following:\n    Process A.\n    Process B.",
            "parallel {\n    process_a()\n    process_b()\n}",
            "join!(process_a(), process_b())"),
        ("Channel", "Send the message through the channel.",
            "channel.send(message)",
            "tx.send(message).await"),
    ],
    // Phase 7: Standard Library
    &[
        ("I/O", "Read a line from the console.",
            "read_line(console)",
            "io::stdin().read_line(&mut buf)"),
        ("FFI", "Call the external C function.",
            "external from C",
            "extern \"C\" { fn external(); }"),
    ],
];

const ROADMAP_STYLE: &str = r#"
.roadmap-container {
    min-height: 100vh;
    background: linear-gradient(135deg, #070a12 0%, #0b1022 50%, #070a12 100%);
    color: #e5e7eb;
    font-family: ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, sans-serif;
}

.roadmap-nav {
    position: sticky;
    top: 0;
    z-index: 50;
    backdrop-filter: blur(18px);
    background: linear-gradient(180deg, rgba(7,10,18,0.72), rgba(7,10,18,0.44));
    border-bottom: 1px solid rgba(255,255,255,0.06);
    padding: 16px 20px;
}

.roadmap-nav-inner {
    max-width: 1000px;
    margin: 0 auto;
    display: flex;
    align-items: center;
    justify-content: space-between;
}

.roadmap-brand {
    display: flex;
    align-items: center;
    gap: 12px;
    text-decoration: none;
    color: #e5e7eb;
}

.roadmap-logo {
    width: 36px;
    height: 36px;
    border-radius: 12px;
    background:
        radial-gradient(circle at 30% 30%, rgba(96,165,250,0.85), transparent 55%),
        radial-gradient(circle at 65% 60%, rgba(167,139,250,0.85), transparent 55%),
        rgba(255,255,255,0.06);
    border: 1px solid rgba(255,255,255,0.10);
}

.roadmap-brand-name {
    font-weight: 800;
    font-size: 14px;
    letter-spacing: -0.5px;
}

.roadmap-back {
    color: #a78bfa;
    text-decoration: none;
    font-size: 14px;
    padding: 8px 16px;
    border-radius: 8px;
    border: 1px solid rgba(167,139,250,0.3);
    transition: all 0.2s ease;
}

.roadmap-back:hover {
    background: rgba(167,139,250,0.1);
    border-color: rgba(167,139,250,0.5);
}

.roadmap-hero {
    text-align: center;
    padding: 60px 20px 40px;
    max-width: 800px;
    margin: 0 auto;
}

.roadmap-hero h1 {
    font-size: 42px;
    font-weight: 800;
    letter-spacing: -1px;
    background: linear-gradient(180deg, #ffffff 0%, rgba(229,231,235,0.78) 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    margin-bottom: 12px;
}

.roadmap-hero .version {
    display: inline-block;
    font-size: 14px;
    padding: 6px 14px;
    border-radius: 20px;
    background: rgba(167,139,250,0.15);
    border: 1px solid rgba(167,139,250,0.3);
    color: #a78bfa;
    margin-bottom: 16px;
}

.roadmap-hero p {
    color: rgba(229,231,235,0.72);
    font-size: 18px;
    line-height: 1.6;
}

.timeline {
    max-width: 700px;
    margin: 0 auto;
    padding: 0 20px 80px;
    position: relative;
}

.timeline::before {
    content: "";
    position: absolute;
    left: 28px;
    top: 0;
    bottom: 80px;
    width: 3px;
    background: linear-gradient(
        180deg,
        #22c55e 0%,
        #22c55e 28%,
        #a78bfa 35%,
        #a78bfa 42%,
        rgba(255,255,255,0.15) 50%,
        rgba(255,255,255,0.08) 100%
    );
    border-radius: 2px;
}

.milestone {
    position: relative;
    padding-left: 70px;
    margin-bottom: 40px;
}

.milestone-dot {
    position: absolute;
    left: 16px;
    top: 4px;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 12px;
    font-weight: 600;
}

.milestone-dot.done {
    background: linear-gradient(135deg, #22c55e, #16a34a);
    box-shadow: 0 0 20px rgba(34,197,94,0.4);
}

.milestone-dot.progress {
    background: linear-gradient(135deg, #a78bfa, #8b5cf6);
    box-shadow: 0 0 20px rgba(167,139,250,0.4);
    animation: pulse 2s ease-in-out infinite;
}

.milestone-dot.planned {
    background: rgba(255,255,255,0.1);
    border: 2px solid rgba(255,255,255,0.2);
}

@keyframes pulse {
    0%, 100% { transform: scale(1); }
    50% { transform: scale(1.1); }
}

.milestone-content {
    background: rgba(255,255,255,0.03);
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: 16px;
    padding: 24px;
    transition: all 0.2s ease;
}

.milestone-content:hover {
    background: rgba(255,255,255,0.05);
    border-color: rgba(255,255,255,0.12);
}

.milestone-header {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 12px;
}

.milestone-title {
    font-size: 20px;
    font-weight: 700;
    color: #fff;
}

.milestone-badge {
    font-size: 11px;
    font-weight: 600;
    padding: 4px 10px;
    border-radius: 12px;
    text-transform: uppercase;
    letter-spacing: 0.5px;
}

.milestone-badge.done {
    background: rgba(34,197,94,0.15);
    color: #22c55e;
    border: 1px solid rgba(34,197,94,0.3);
}

.milestone-badge.progress {
    background: rgba(167,139,250,0.15);
    color: #a78bfa;
    border: 1px solid rgba(167,139,250,0.3);
}

.milestone-badge.planned {
    background: rgba(255,255,255,0.05);
    color: rgba(255,255,255,0.5);
    border: 1px solid rgba(255,255,255,0.1);
}

.milestone-desc {
    color: rgba(229,231,235,0.72);
    font-size: 14px;
    line-height: 1.6;
    margin-bottom: 16px;
}

.milestone-features {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
}

.feature-tag {
    font-size: 12px;
    padding: 6px 12px;
    border-radius: 8px;
    background: rgba(255,255,255,0.04);
    border: 1px solid rgba(255,255,255,0.08);
    color: rgba(229,231,235,0.8);
}

.feature-tag.done {
    background: rgba(34,197,94,0.08);
    border-color: rgba(34,197,94,0.2);
    color: #86efac;
}

.roadmap-footer {
    border-top: 1px solid rgba(255,255,255,0.06);
    padding: 24px 20px;
    text-align: center;
    color: rgba(229,231,235,0.56);
    font-size: 13px;
}

.roadmap-footer a {
    color: rgba(229,231,235,0.72);
    text-decoration: none;
    margin: 0 8px;
}

.roadmap-footer a:hover {
    color: #a78bfa;
}

.roadmap-nav-links {
    display: flex;
    align-items: center;
    gap: 12px;
}

.roadmap-github {
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

.roadmap-github:hover {
    background: rgba(255,255,255,0.08);
    color: #e5e7eb;
    border-color: rgba(255,255,255,0.2);
}

.roadmap-github svg {
    width: 18px;
    height: 18px;
    fill: currentColor;
}

.github-link {
    display: inline-flex;
    align-items: center;
    gap: 6px;
}

@media (max-width: 600px) {
    .timeline::before {
        left: 18px;
    }
    .milestone {
        padding-left: 50px;
    }
    .milestone-dot {
        left: 6px;
        width: 20px;
        height: 20px;
    }
    .milestone-title {
        font-size: 18px;
    }
}

.milestone-examples {
    margin-top: 16px;
    border-top: 1px solid rgba(255,255,255,0.06);
    padding-top: 16px;
}

.milestone-tabs {
    display: flex;
    gap: 6px;
    margin-bottom: 12px;
    flex-wrap: wrap;
}

.milestone-tab {
    padding: 6px 12px;
    border-radius: 6px;
    border: 1px solid rgba(255,255,255,0.1);
    background: rgba(255,255,255,0.03);
    color: #94a3b8;
    cursor: pointer;
    font-size: 12px;
    font-weight: 500;
    transition: all 0.2s ease;
}

.milestone-tab:hover {
    background: rgba(255,255,255,0.08);
    color: #e8e8e8;
}

.milestone-tab.active {
    background: linear-gradient(135deg, #667eea, #764ba2);
    color: white;
    border-color: transparent;
}

.milestone-code {
    background: rgba(0,0,0,0.25);
    border-radius: 8px;
    padding: 16px;
    font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
    font-size: 13px;
}

.milestone-english {
    color: #e8e8e8;
    font-style: italic;
    margin-bottom: 8px;
    white-space: pre-wrap;
    line-height: 1.5;
}

.milestone-arrow {
    color: #667eea;
    margin: 8px 0;
    font-size: 16px;
}

.milestone-output {
    color: #98c379;
    white-space: pre-wrap;
    line-height: 1.4;
}

.format-toggle {
    display: flex;
    gap: 4px;
    margin-bottom: 8px;
}

.format-btn {
    padding: 3px 8px;
    border-radius: 4px;
    border: 1px solid rgba(255,255,255,0.1);
    background: rgba(255,255,255,0.03);
    color: #64748b;
    cursor: pointer;
    font-size: 10px;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    transition: all 0.15s ease;
}

.format-btn:hover {
    background: rgba(255,255,255,0.06);
    color: #94a3b8;
}

.format-btn.active {
    background: rgba(102,126,234,0.2);
    border-color: rgba(102,126,234,0.4);
    color: #a5b4fc;
}
"#;

#[component]
fn MilestoneExamples(index: usize) -> Element {
    let mut active = use_signal(|| 0usize);
    let mut use_unicode = use_signal(|| false);
    let examples = MILESTONE_EXAMPLES[index];

    rsx! {
        div { class: "milestone-examples",
            div { class: "milestone-tabs",
                for (i, (label, _, _, _)) in examples.iter().enumerate() {
                    button {
                        key: "{i}",
                        class: if active() == i { "milestone-tab active" } else { "milestone-tab" },
                        onclick: move |_| active.set(i),
                        "{label}"
                    }
                }
            }
            div { class: "milestone-code",
                div { class: "milestone-english", "\"{examples[active()].1}\"" }
                div { class: "milestone-arrow", "↓" }
                div { class: "format-toggle",
                    button {
                        class: if !use_unicode() { "format-btn active" } else { "format-btn" },
                        onclick: move |_| use_unicode.set(false),
                        "Simple"
                    }
                    button {
                        class: if use_unicode() { "format-btn active" } else { "format-btn" },
                        onclick: move |_| use_unicode.set(true),
                        "Unicode"
                    }
                }
                div { class: "milestone-output",
                    if use_unicode() {
                        "{examples[active()].3}"
                    } else {
                        "{examples[active()].2}"
                    }
                }
            }
        }
    }
}

#[component]
pub fn Roadmap() -> Element {
    rsx! {
        style { "{ROADMAP_STYLE}" }

        div { class: "roadmap-container",
            nav { class: "roadmap-nav",
                div { class: "roadmap-nav-inner",
                    Link {
                        to: Route::Landing {},
                        class: "roadmap-brand",
                        div { class: "roadmap-logo" }
                        span { class: "roadmap-brand-name", "LOGICAFFEINE" }
                    }
                    div { class: "roadmap-nav-links",
                        a {
                            href: "https://github.com/Brahmastra-Labs/logicaffeine",
                            target: "_blank",
                            class: "roadmap-github",
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
                            class: "roadmap-back",
                            "← Back to Home"
                        }
                    }
                }
            }

            section { class: "roadmap-hero",
                h1 { "LOGOS Roadmap" }
                p { "From English to executable logic. Track our journey from transpiler to full programming language." }
            }

            div { class: "timeline",
                // Phase 1: Core Transpiler - DONE
                div { class: "milestone",
                    div { class: "milestone-dot done", "✓" }
                    div { class: "milestone-content",
                        div { class: "milestone-header",
                            span { class: "milestone-title", "Core Transpiler" }
                            span { class: "milestone-badge done", "Complete" }
                        }
                        p { class: "milestone-desc",
                            "The foundation: parse English, produce First-Order Logic. 802 tests validate 20+ linguistic phenomena."
                        }
                        div { class: "milestone-features",
                            span { class: "feature-tag done", "Lexer" }
                            span { class: "feature-tag done", "Parser" }
                            span { class: "feature-tag done", "AST" }
                            span { class: "feature-tag done", "Transpiler" }
                            span { class: "feature-tag done", "Quantifiers" }
                            span { class: "feature-tag done", "Modals" }
                            span { class: "feature-tag done", "Aspect/Tense" }
                        }
                        MilestoneExamples { index: 0 }
                    }
                }

                // Phase 2: Web Platform - DONE
                div { class: "milestone",
                    div { class: "milestone-dot done", "✓" }
                    div { class: "milestone-content",
                        div { class: "milestone-header",
                            span { class: "milestone-title", "Web Platform" }
                            span { class: "milestone-badge done", "Complete" }
                        }
                        p { class: "milestone-desc",
                            "Learn logic interactively. Structured curriculum, free-form studio, and gamification to keep you engaged."
                        }
                        div { class: "milestone-features",
                            span { class: "feature-tag done", "Dioxus WASM" }
                            span { class: "feature-tag done", "Learn Mode" }
                            span { class: "feature-tag done", "Studio" }
                            span { class: "feature-tag done", "Achievements" }
                            span { class: "feature-tag done", "Streaks" }
                        }
                        MilestoneExamples { index: 1 }
                    }
                }

                // Phase 3: Codegen Pipeline - IN PROGRESS
                div { class: "milestone",
                    div { class: "milestone-dot progress", "◐" }
                    div { class: "milestone-content",
                        div { class: "milestone-header",
                            span { class: "milestone-title", "Codegen Pipeline" }
                            span { class: "milestone-badge progress", "In Progress" }
                        }
                        p { class: "milestone-desc",
                            "From English to native binary. Generate Rust code, compile to executables, target WASM for the web."
                        }
                        div { class: "milestone-features",
                            span { class: "feature-tag done", "Rust Codegen" }
                            span { class: "feature-tag", "Native Compilation" }
                            span { class: "feature-tag", "WASM Target" }
                            span { class: "feature-tag", "Module System" }
                        }
                        MilestoneExamples { index: 2 }
                    }
                }

                // Phase 4: Type System - PLANNED
                div { class: "milestone",
                    div { class: "milestone-dot planned" }
                    div { class: "milestone-content",
                        div { class: "milestone-header",
                            span { class: "milestone-title", "Type System" }
                            span { class: "milestone-badge planned", "Planned" }
                        }
                        p { class: "milestone-desc",
                            "Full dependent types with refinements. Types that depend on values. Constraints that catch bugs at compile time."
                        }
                        div { class: "milestone-features",
                            span { class: "feature-tag", "Dependent Types" }
                            span { class: "feature-tag", "Refinements" }
                            span { class: "feature-tag", "Universe Hierarchy" }
                            span { class: "feature-tag", "Type Inference" }
                        }
                        MilestoneExamples { index: 3 }
                    }
                }

                // Phase 5: Proof System - PLANNED
                div { class: "milestone",
                    div { class: "milestone-dot planned" }
                    div { class: "milestone-content",
                        div { class: "milestone-header",
                            span { class: "milestone-title", "Proof System" }
                            span { class: "milestone-badge planned", "Planned" }
                        }
                        p { class: "milestone-desc",
                            "Curry-Howard in English. Write proofs as prose. The compiler verifies your reasoning."
                        }
                        div { class: "milestone-features",
                            span { class: "feature-tag", "Proof Obligations" }
                            span { class: "feature-tag", "Auto Tactic" }
                            span { class: "feature-tag", "Induction" }
                            span { class: "feature-tag", "Totality Checking" }
                        }
                        MilestoneExamples { index: 4 }
                    }
                }

                // Phase 6: Concurrency - PLANNED
                div { class: "milestone",
                    div { class: "milestone-dot planned" }
                    div { class: "milestone-content",
                        div { class: "milestone-header",
                            span { class: "milestone-title", "Concurrency Model" }
                            span { class: "milestone-badge planned", "Planned" }
                        }
                        p { class: "milestone-desc",
                            "Structured concurrency with proof obligations. Channels, pipelines, and distributed agents — all verified."
                        }
                        div { class: "milestone-features",
                            span { class: "feature-tag", "Structured Concurrency" }
                            span { class: "feature-tag", "Channels" }
                            span { class: "feature-tag", "Agent Model" }
                            span { class: "feature-tag", "CSP Processes" }
                        }
                        MilestoneExamples { index: 5 }
                    }
                }

                // Phase 7: Standard Library - PLANNED
                div { class: "milestone",
                    div { class: "milestone-dot planned" }
                    div { class: "milestone-content",
                        div { class: "milestone-header",
                            span { class: "milestone-title", "Standard Library & Beyond" }
                            span { class: "milestone-badge planned", "Planned" }
                        }
                        p { class: "milestone-desc",
                            "A complete standard library. FFI for Rust and C. The Live Codex IDE for real-time proof visualization."
                        }
                        div { class: "milestone-features",
                            span { class: "feature-tag", "Core Types" }
                            span { class: "feature-tag", "I/O Operations" }
                            span { class: "feature-tag", "FFI" }
                            span { class: "feature-tag", "Live Codex IDE" }
                        }
                        MilestoneExamples { index: 6 }
                    }
                }
            }

            footer { class: "roadmap-footer",
                span { "© 2025 Brahmastra Labs LLC  •  Written in Rust 🦀" }
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
                Link { to: Route::Privacy {}, "Privacy" }
                span { " • " }
                Link { to: Route::Terms {}, "Terms" }
                span { " • " }
                Link { to: Route::Pricing {}, "Pricing" }
            }
        }
    }
}
