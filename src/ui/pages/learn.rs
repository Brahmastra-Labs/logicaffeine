use dioxus::prelude::*;
use crate::ui::router::Route;
use crate::ui::components::mode_selector::{ModeSelector, ModeInfo};
use crate::ui::components::app_navbar::AppNavbar;

const LEARN_STYLE: &str = r#"
.learn-container {
    min-height: 100vh;
    background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f3460 100%);
    color: #e8e8e8;
    padding: 40px 20px;
}

.learn-header {
    max-width: 1000px;
    margin: 0 auto 40px;
}

.learn-header h1 {
    font-size: 36px;
    font-weight: 700;
    background: linear-gradient(90deg, #00d4ff, #7b2cbf);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    margin-bottom: 8px;
}

.learn-header p {
    color: #888;
    font-size: 16px;
}

.era-list {
    max-width: 1000px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    gap: 24px;
}

.era-card {
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 16px;
    overflow: hidden;
}

.era-header {
    padding: 24px;
    cursor: pointer;
    display: flex;
    justify-content: space-between;
    align-items: center;
    transition: background 0.2s ease;
}

.era-header:hover {
    background: rgba(255, 255, 255, 0.03);
}

.era-title {
    font-size: 24px;
    font-weight: 600;
    color: #fff;
    margin-bottom: 4px;
}

.era-description {
    color: #888;
    font-size: 14px;
}

.era-toggle {
    font-size: 24px;
    color: #667eea;
    transition: transform 0.3s ease;
}

.era-toggle.open {
    transform: rotate(180deg);
}

.module-list {
    padding: 0 24px 24px;
    display: flex;
    flex-direction: column;
    gap: 12px;
}

.module-card {
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 12px;
    padding: 16px 20px;
    display: flex;
    justify-content: space-between;
    align-items: center;
    cursor: pointer;
    transition: all 0.2s ease;
    text-decoration: none;
}

.module-card:hover {
    background: rgba(255, 255, 255, 0.06);
    border-color: #667eea;
    transform: translateX(4px);
}

.module-info h3 {
    color: #fff;
    font-size: 16px;
    margin-bottom: 4px;
}

.module-info p {
    color: #666;
    font-size: 13px;
}

.module-stats {
    display: flex;
    align-items: center;
    gap: 16px;
}

.exercise-count {
    color: #888;
    font-size: 13px;
}

.difficulty-stars {
    color: #667eea;
    font-size: 14px;
}

.start-btn {
    background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
    color: white;
    border: none;
    padding: 8px 16px;
    border-radius: 8px;
    font-size: 14px;
    cursor: pointer;
    transition: transform 0.2s ease;
}

.start-btn:hover {
    transform: scale(1.05);
}

.back-link {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    color: #667eea;
    text-decoration: none;
    margin-bottom: 24px;
    font-size: 14px;
}

.back-link:hover {
    text-decoration: underline;
}
"#;

#[component]
pub fn Learn() -> Element {
    let mut expanded_era = use_signal(|| Some("logicaffeine".to_string()));
    let mut pending_module = use_signal(|| None::<(String, String, String)>);
    let navigator = use_navigator();

    let eras = vec![
        ("logicaffeine", "Practice", "Classic logic exercises from Gensler's Introduction to Logic.", vec![
            ("syllogistic", "The Syllogism", "Translate English into syllogistic notation", 99, 1),
            ("propositional", "Propositional Logic", "AND, OR, NOT, and IF-THEN connectives", 114, 2),
            ("modal", "Modal Logic", "Possibility and necessity operators", 34, 3),
            ("deontic", "Deontic Logic", "Obligation, permission, and prohibition", 38, 3),
            ("belief", "Belief Logic", "Beliefs, knowledge, and attitudes", 15, 3),
            ("informal", "Definitions & Meanings", "Identify problems with definitions", 48, 2),
        ]),
        ("trivium", "Basics", "Learn to name objects, describe properties, and express relationships.", vec![
            ("atomic", "The Atomic World", "Properties as functions mapping individuals to Truth", 3, 1),
            ("relations", "Connections", "Relations bind multiple individuals", 2, 2),
            ("negation", "Negation", "Negation flips truth values", 2, 2),
        ]),
        ("quadrivium", "Quantifiers", "Master universal and existential quantifiers that give logic its power.", vec![
            ("universal", "Universal Quantification", "The ∀ expresses claims about ALL", 3, 3),
            ("existential", "Existential Quantification", "The ∃ asserts that SOME exists", 2, 3),
            ("scope", "Scope Ambiguity", "Multiple quantifiers create ambiguity", 1, 4),
        ]),
        ("metaphysics", "Modality & Time", "Express possibility, necessity, and temporal relationships.", vec![
            ("modality", "Modal Logic", "Possibility and necessity", 2, 4),
            ("time", "Temporal Logic", "Past and future operators", 2, 4),
        ]),
    ];

    rsx! {
        style { "{LEARN_STYLE}" }

        AppNavbar { title: "Curriculum".to_string() }

        div { class: "learn-container",
            div { class: "learn-header",
                h1 { "Curriculum" }
                p { "Master first-order logic through progressive challenges" }
            }

            div { class: "era-list",
                for (era_id, title, description, modules) in eras.iter() {
                    div { class: "era-card",
                        div {
                            class: "era-header",
                            onclick: {
                                let era_id_str = era_id.to_string();
                                move |_| {
                                    if expanded_era().as_ref() == Some(&era_id_str) {
                                        expanded_era.set(None);
                                    } else {
                                        expanded_era.set(Some(era_id_str.clone()));
                                    }
                                }
                            },
                            div {
                                div { class: "era-title", "{title}" }
                                div { class: "era-description", "{description}" }
                            }
                            span {
                                class: if expanded_era().as_ref() == Some(&era_id.to_string()) { "era-toggle open" } else { "era-toggle" },
                                "▼"
                            }
                        }

                        if expanded_era().as_ref() == Some(&era_id.to_string()) {
                            div { class: "module-list",
                                for (mod_id, mod_title, mod_desc, ex_count, difficulty) in modules.iter() {
                                    {
                                        let era_for_click = era_id.to_string();
                                        let mod_for_click = mod_id.to_string();
                                        let title_for_click = mod_title.to_string();
                                        rsx! {
                                            div {
                                                class: "module-card",
                                                onclick: move |_| {
                                                    pending_module.set(Some((
                                                        era_for_click.clone(),
                                                        mod_for_click.clone(),
                                                        title_for_click.clone(),
                                                    )));
                                                },
                                                div { class: "module-info",
                                                    h3 { "{mod_title}" }
                                                    p { "{mod_desc}" }
                                                }
                                                div { class: "module-stats",
                                                    span { class: "exercise-count", "{ex_count} exercises" }
                                                    span { class: "difficulty-stars",
                                                        for _ in 0..*difficulty {
                                                            "★"
                                                        }
                                                        for _ in *difficulty..5 {
                                                            "☆"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if let Some((era, module, title)) = pending_module() {
                ModeSelector {
                    info: ModeInfo {
                        era: era.clone(),
                        module: module.clone(),
                        title: title.clone(),
                    },
                    on_select: move |mode: String| {
                        let nav = navigator.clone();
                        let e = era.clone();
                        let m = module.clone();
                        pending_module.set(None);
                        nav.push(Route::Lesson {
                            era: e,
                            module: m,
                            mode,
                        });
                    },
                    on_cancel: move |_| {
                        pending_module.set(None);
                    },
                }
            }
        }
    }
}
