use dioxus::prelude::*;
use crate::ui::router::Route;
use crate::ui::components::logic_output::highlight_logic;
use crate::ui::components::main_nav::{MainNav, ActivePage};

const HOME_STYLE: &str = r#"
.home-wrapper {
    display: flex;
    flex-direction: column;
    align-items: center;
    min-height: 100vh;
    padding: 60px 20px;
    background: radial-gradient(circle at top center, #1e293b 0%, #0f172a 100%);
}

.brand-header {
    text-align: center;
    margin-bottom: 60px;
    animation: fadeIn 1s ease;
}

.brand-header h1 {
    font-size: 56px;
    font-weight: 800;
    margin-bottom: 12px;
    background: linear-gradient(135deg, #60a5fa 0%, #a78bfa 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    letter-spacing: -1px;
}

.brand-header p {
    font-size: 18px;
    color: #94a3b8;
}

.portal-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: 30px;
    max-width: 1000px;
    width: 100%;
}

.portal-card {
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 20px;
    padding: 40px 30px;
    text-align: left;
    text-decoration: none;
    transition: all 0.3s cubic-bezier(0.4, 0, 0.2, 1);
    position: relative;
    overflow: hidden;
}

.portal-card:hover {
    transform: translateY(-5px);
    background: rgba(255, 255, 255, 0.05);
    border-color: rgba(129, 140, 248, 0.3);
    box-shadow: 0 20px 40px rgba(0, 0, 0, 0.4);
}

.portal-card .icon {
    font-size: 42px;
    margin-bottom: 20px;
    background: rgba(255, 255, 255, 0.1);
    width: 80px;
    height: 80px;
    border-radius: 16px;
    display: flex;
    align-items: center;
    justify-content: center;
}

.portal-card h2 {
    color: #fff;
    font-size: 24px;
    margin-bottom: 10px;
}

.portal-card p {
    color: #94a3b8;
    line-height: 1.5;
    font-size: 15px;
}

.portal-card .arrow {
    position: absolute;
    bottom: 30px;
    right: 30px;
    opacity: 0;
    transition: all 0.3s ease;
    color: #818cf8;
    font-size: 24px;
}

.portal-card:hover .arrow {
    opacity: 1;
    transform: translateX(5px);
}

@keyframes fadeIn {
    from { opacity: 0; transform: translateY(20px); }
    to { opacity: 1; transform: translateY(0); }
}

@media (max-width: 600px) {
    .portal-grid {
        grid-template-columns: 1fr;
    }
    .brand-header h1 {
        font-size: 40px;
    }
    .example-showcase {
        padding: 20px;
    }
    .example-sentence {
        font-size: 16px !important;
    }
    .example-output {
        font-size: 14px !important;
    }
}

.example-showcase {
    max-width: 800px;
    width: 100%;
    margin-bottom: 50px;
    animation: fadeIn 1s ease 0.2s both;
}

.example-tabs {
    display: flex;
    gap: 8px;
    margin-bottom: 20px;
    flex-wrap: wrap;
}

.example-tab {
    padding: 10px 20px;
    border-radius: 8px;
    border: 1px solid rgba(255, 255, 255, 0.1);
    background: rgba(255, 255, 255, 0.03);
    color: #94a3b8;
    cursor: pointer;
    transition: all 0.2s ease;
    font-size: 14px;
    font-weight: 500;
}

.example-tab:hover {
    background: rgba(255, 255, 255, 0.08);
    color: #e8e8e8;
}

.example-tab.active {
    background: linear-gradient(135deg, #667eea, #764ba2);
    color: white;
    border-color: transparent;
}

.example-content {
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 16px;
    padding: 30px;
    text-align: center;
}

.example-sentence {
    font-size: 20px;
    color: #e8e8e8;
    font-style: italic;
    margin-bottom: 0;
    line-height: 1.5;
}

.example-arrow {
    color: #667eea;
    font-size: 28px;
    margin: 20px 0;
}

.example-output {
    font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
    font-size: 18px;
    line-height: 1.6;
    text-align: left;
    background: rgba(0, 0, 0, 0.2);
    padding: 20px;
    border-radius: 8px;
    overflow-x: auto;
}

.logic-quantifier { color: #c678dd; font-weight: 600; }
.logic-connective { color: #56b6c2; }
.logic-variable { color: #61afef; }
.logic-predicate { color: #98c379; }
.logic-constant { color: #e5c07b; }
.logic-paren { color: #abb2bf; }

@keyframes slideIn {
    from { opacity: 0; transform: translateX(20px); }
    to { opacity: 1; transform: translateX(0); }
}

.example-content {
    animation: slideIn 0.3s ease;
}

.example-nav {
    display: flex;
    justify-content: center;
    align-items: center;
    gap: 20px;
    margin-top: 20px;
}

.nav-arrow {
    width: 40px;
    height: 40px;
    border-radius: 50%;
    border: 1px solid rgba(255, 255, 255, 0.2);
    background: rgba(255, 255, 255, 0.05);
    color: #94a3b8;
    cursor: pointer;
    transition: all 0.2s;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 18px;
}

.nav-arrow:hover:not(:disabled) {
    background: rgba(102, 126, 234, 0.3);
    border-color: #667eea;
    color: white;
}

.nav-arrow:disabled {
    opacity: 0.3;
    cursor: not-allowed;
}

.nav-dots {
    display: flex;
    gap: 8px;
}

.nav-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.2);
    cursor: pointer;
    transition: all 0.2s;
}

.nav-dot:hover {
    background: rgba(255, 255, 255, 0.4);
}

.nav-dot.active {
    background: #667eea;
    transform: scale(1.2);
}
"#;

const EXAMPLES: &[(&str, &str, &str)] = &[
    (
        "Universal",
        "Every user who has a key enters the room.",
        "∃y(∀x(((User(x) ∧ (Key(y) ∧ Have(x, y))) → Enter(x, Room))))"
    ),
    (
        "Conditional",
        "If a user enters the room, the alarm triggers.",
        "(Enter(User, Room) → Trigger(Alarm))"
    ),
    (
        "Negation",
        "No user who lacks a key can enter the room.",
        "∀x(((User(x) ∧ (Key(y) ∧ Lack(x, y))) → ¬Enter(x)))"
    ),
    (
        "Donkey",
        "If a farmer owns a donkey, he beats it.",
        "∀x∀y((Farmer(x) ∧ Donkey(y) ∧ Own(x, y)) → Beat(x, y))"
    ),
    (
        "Focus",
        "Only John ate the cake.",
        "Only(John, ∃e(Eat(e) ∧ Agent(e, John) ∧ Theme(e, Cake)))"
    ),
];

#[component]
pub fn Home() -> Element {
    let mut active_tab = use_signal(|| 0usize);

    rsx! {
        style { "{HOME_STYLE}" }

        MainNav { active: ActivePage::Home, show_nav_links: true }

        div { class: "home-wrapper",
            div { class: "brand-header",
                h1 { "LOGICAFFEINE" }
                p { "Choose your path to logical mastery." }
            }

            div { class: "example-showcase",
                div { class: "example-tabs",
                    for (i, (label, _, _)) in EXAMPLES.iter().enumerate() {
                        button {
                            key: "{i}",
                            class: if active_tab() == i { "example-tab active" } else { "example-tab" },
                            onclick: move |_| active_tab.set(i),
                            "{label}"
                        }
                    }
                }
                div {
                    key: "{active_tab()}",
                    class: "example-content",
                    p { class: "example-sentence",
                        "\"{EXAMPLES[active_tab()].1}\""
                    }
                    div { class: "example-arrow", "↓" }
                    div {
                        class: "example-output",
                        dangerous_inner_html: highlight_logic(EXAMPLES[active_tab()].2)
                    }
                }
                div { class: "example-nav",
                    button {
                        class: "nav-arrow",
                        disabled: active_tab() == 0,
                        onclick: move |_| {
                            if active_tab() > 0 {
                                active_tab.set(active_tab() - 1);
                            }
                        },
                        "←"
                    }
                    div { class: "nav-dots",
                        for i in 0..EXAMPLES.len() {
                            div {
                                key: "{i}",
                                class: if active_tab() == i { "nav-dot active" } else { "nav-dot" },
                                onclick: move |_| active_tab.set(i),
                            }
                        }
                    }
                    button {
                        class: "nav-arrow",
                        disabled: active_tab() == EXAMPLES.len() - 1,
                        onclick: move |_| {
                            if active_tab() < EXAMPLES.len() - 1 {
                                active_tab.set(active_tab() + 1);
                            }
                        },
                        "→"
                    }
                }
            }

            div { class: "portal-grid",
                Link {
                    to: Route::Learn {},
                    class: "portal-card",
                    div { class: "icon", "🎓" }
                    h2 { "Curriculum" }
                    p { "Step-by-step interactive lessons from basics to advanced logic." }
                    div { class: "arrow", "→" }
                }

                Link {
                    to: Route::Studio {},
                    class: "portal-card",
                    div { class: "icon", "⚙️" }
                    h2 { "Studio" }
                    p { "Free-form sandbox. Type English, get Logic. Inspect the AST." }
                    div { class: "arrow", "→" }
                }

                Link {
                    to: Route::Review {},
                    class: "portal-card",
                    div { class: "icon", "🔄" }
                    h2 { "Daily Review" }
                    p { "Spaced repetition practice to keep your skills sharp." }
                    div { class: "arrow", "→" }
                }

                Link {
                    to: Route::Pricing {},
                    class: "portal-card",
                    div { class: "icon", "💼" }
                    h2 { "Enterprise" }
                    p { "Commercial licensing and team management features." }
                    div { class: "arrow", "→" }
                }
            }
        }
    }
}
