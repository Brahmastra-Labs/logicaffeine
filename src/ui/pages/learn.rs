use dioxus::prelude::*;
use crate::ui::router::Route;
use crate::ui::components::mode_selector::{ModeSelector, ModeInfo};
use crate::ui::components::main_nav::{MainNav, ActivePage};
use crate::ui::components::learn_sidebar::{LearnSidebar, ModuleInfo};
use crate::ui::components::guide_code_block::GuideCodeBlock;
use crate::ui::pages::guide::content::ExampleMode;

const LEARN_STYLE: &str = r#"
:root {
    --bg0: #070a12;
    --bg1: #0b1022;
    --card: rgba(255,255,255,0.06);
    --card2: rgba(255,255,255,0.04);
    --border: rgba(255,255,255,0.10);
    --border2: rgba(255,255,255,0.14);
    --text: #e5e7eb;
    --muted: rgba(229,231,235,0.72);
    --muted2: rgba(229,231,235,0.56);
    --brand: #a78bfa;
    --brand2: #60a5fa;
    --ok: #22c55e;
}

* { box-sizing: border-box; }
a { color: inherit; }

.learn-page {
    min-height: 100vh;
    color: var(--text);
    background:
        radial-gradient(1200px 600px at 50% -120px, rgba(167,139,250,0.14), transparent 60%),
        radial-gradient(900px 500px at 15% 30%, rgba(96,165,250,0.14), transparent 60%),
        radial-gradient(800px 450px at 90% 45%, rgba(34,197,94,0.08), transparent 62%),
        linear-gradient(180deg, var(--bg0), var(--bg1) 55%, #070a12);
    font-family: ui-sans-serif, system-ui, -apple-system, 'Segoe UI', Roboto, 'Inter', 'Helvetica Neue', Arial, sans-serif;
}

/* Hero */
.learn-hero {
    max-width: 1280px;
    margin: 0 auto;
    padding: 60px 24px 40px;
}

.learn-hero h1 {
    font-size: 48px;
    font-weight: 900;
    letter-spacing: -1.5px;
    line-height: 1.1;
    background: linear-gradient(180deg, #ffffff 0%, rgba(229,231,235,0.78) 65%, rgba(229,231,235,0.62) 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    margin: 0 0 16px;
}

.learn-hero p {
    font-size: 18px;
    color: var(--muted);
    max-width: 600px;
    line-height: 1.6;
    margin: 0;
}

.learn-hero-badge {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    border-radius: 999px;
    background: rgba(255,255,255,0.06);
    border: 1px solid rgba(255,255,255,0.10);
    font-size: 13px;
    font-weight: 600;
    color: rgba(255,255,255,0.85);
    margin-bottom: 20px;
}

.learn-hero-badge .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--ok);
    box-shadow: 0 0 0 4px rgba(34,197,94,0.15);
}

/* Layout */
.learn-layout {
    max-width: 1280px;
    margin: 0 auto;
    display: flex;
    gap: 48px;
    padding: 0 24px 80px;
}

/* Main content */
.learn-content {
    flex: 1;
    min-width: 0;
    max-width: 800px;
}

/* Era sections */
.learn-era {
    margin-bottom: 64px;
    scroll-margin-top: 100px;
}

.learn-era-divider {
    margin: 80px 0 48px;
    padding: 24px 0;
    border-top: 1px solid rgba(255,255,255,0.08);
}

.learn-era-divider h2 {
    font-size: 14px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 1.5px;
    color: var(--muted2);
    margin: 0;
}

.learn-era-header {
    margin-bottom: 24px;
    padding-bottom: 16px;
    border-bottom: 1px solid rgba(255,255,255,0.08);
}

.learn-era-header h2 {
    font-size: 32px;
    font-weight: 800;
    letter-spacing: -0.8px;
    line-height: 1.2;
    background: linear-gradient(180deg, #ffffff 0%, rgba(229,231,235,0.85) 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    margin: 0 0 8px;
}

.learn-era-header p {
    color: var(--muted);
    font-size: 15px;
    line-height: 1.6;
    margin: 0;
}

/* Module cards */
.learn-modules {
    display: flex;
    flex-direction: column;
    gap: 20px;
}

.learn-module-card {
    background: rgba(255,255,255,0.04);
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: 16px;
    padding: 24px;
    transition: all 0.2s ease;
    scroll-margin-top: 100px;
}

.learn-module-card:hover {
    background: rgba(255,255,255,0.06);
    border-color: rgba(255,255,255,0.12);
}

.learn-module-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 16px;
    margin-bottom: 16px;
}

.learn-module-info {
    flex: 1;
}

.learn-module-title {
    font-size: 20px;
    font-weight: 700;
    color: var(--text);
    margin: 0 0 6px;
    display: flex;
    align-items: center;
    gap: 10px;
}

.learn-module-number {
    font-size: 14px;
    font-weight: 700;
    color: var(--brand);
    opacity: 0.8;
}

.learn-module-desc {
    color: var(--muted);
    font-size: 14px;
    line-height: 1.5;
    margin: 0;
}

.learn-module-meta {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 8px;
}

.learn-exercise-count {
    font-size: 13px;
    color: var(--muted);
    background: rgba(255,255,255,0.05);
    padding: 4px 10px;
    border-radius: 999px;
}

.learn-difficulty {
    display: flex;
    gap: 3px;
}

.learn-difficulty-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: rgba(255,255,255,0.15);
}

.learn-difficulty-dot.filled {
    background: linear-gradient(135deg, #60a5fa, #a78bfa);
}

/* Preview section */
.learn-module-preview {
    margin-top: 16px;
    padding-top: 16px;
    border-top: 1px solid rgba(255,255,255,0.06);
}

.learn-preview-label {
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--muted2);
    margin-bottom: 12px;
}

/* Action buttons */
.learn-module-actions {
    display: flex;
    gap: 10px;
    margin-top: 20px;
}

.learn-action-btn {
    padding: 10px 18px;
    border-radius: 10px;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.18s ease;
    text-decoration: none;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border: 1px solid transparent;
}

.learn-action-btn.primary {
    background: linear-gradient(135deg, rgba(96,165,250,0.9), rgba(167,139,250,0.9));
    color: #060814;
    border-color: rgba(255,255,255,0.1);
}

.learn-action-btn.primary:hover {
    background: linear-gradient(135deg, #60a5fa, #a78bfa);
}

.learn-action-btn.secondary {
    background: rgba(255,255,255,0.06);
    color: var(--muted);
    border-color: rgba(255,255,255,0.12);
}

.learn-action-btn.secondary:hover {
    background: rgba(255,255,255,0.10);
    color: var(--text);
}

/* Responsive */
@media (max-width: 1024px) {
    .learn-layout {
        flex-direction: column;
    }

    .learn-hero h1 {
        font-size: 36px;
    }

    .learn-hero {
        padding: 40px 24px 32px;
    }
}

@media (max-width: 640px) {
    .learn-hero h1 {
        font-size: 28px;
    }

    .learn-hero p {
        font-size: 16px;
    }

    .learn-era-header h2 {
        font-size: 24px;
    }

    .learn-module-header {
        flex-direction: column;
    }

    .learn-module-meta {
        flex-direction: row;
        align-items: center;
    }

    .learn-module-actions {
        flex-direction: column;
    }

    .learn-action-btn {
        justify-content: center;
    }
}
"#;

/// Module data with preview example
struct ModuleData {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    exercise_count: u32,
    difficulty: u8,
    preview_code: Option<&'static str>,
}

/// Era data structure
struct EraData {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    modules: Vec<ModuleData>,
}

fn get_curriculum_data() -> Vec<EraData> {
    vec![
        EraData {
            id: "logicaffeine",
            title: "Practice",
            description: "Classic logic exercises from Gensler's Introduction to Logic. Build your skills through structured problem sets.",
            modules: vec![
                ModuleData {
                    id: "syllogistic",
                    title: "The Syllogism",
                    description: "Translate English into syllogistic notation. Learn the classical form of logical reasoning used since Aristotle.",
                    exercise_count: 99,
                    difficulty: 1,
                    preview_code: Some("All humans are mortal."),
                },
                ModuleData {
                    id: "propositional",
                    title: "Propositional Logic",
                    description: "Master AND, OR, NOT, and IF-THEN connectives. The foundation of all logical reasoning.",
                    exercise_count: 114,
                    difficulty: 2,
                    preview_code: Some("If John runs, then Mary walks."),
                },
                ModuleData {
                    id: "modal",
                    title: "Modal Logic",
                    description: "Explore possibility and necessity operators. Express what could be, must be, or might be true.",
                    exercise_count: 34,
                    difficulty: 3,
                    preview_code: Some("John can swim."),
                },
                ModuleData {
                    id: "deontic",
                    title: "Deontic Logic",
                    description: "Reason about obligation, permission, and prohibition. The logic of ethics and law.",
                    exercise_count: 38,
                    difficulty: 3,
                    preview_code: Some("John must leave."),
                },
                ModuleData {
                    id: "belief",
                    title: "Belief Logic",
                    description: "Express beliefs, knowledge, and attitudes. Model what agents know and believe.",
                    exercise_count: 15,
                    difficulty: 3,
                    preview_code: Some("John believes that Mary runs."),
                },
                ModuleData {
                    id: "informal",
                    title: "Definitions & Meanings",
                    description: "Identify problems with definitions. Critical thinking about language and meaning.",
                    exercise_count: 48,
                    difficulty: 2,
                    preview_code: None,
                },
            ],
        },
        EraData {
            id: "trivium",
            title: "Basics",
            description: "Learn to name objects, describe properties, and express relationships. The building blocks of logical language.",
            modules: vec![
                ModuleData {
                    id: "atomic",
                    title: "The Atomic World",
                    description: "Properties as functions mapping individuals to Truth. Start with the simplest logical statements.",
                    exercise_count: 3,
                    difficulty: 1,
                    preview_code: Some("John runs."),
                },
                ModuleData {
                    id: "relations",
                    title: "Connections",
                    description: "Relations bind multiple individuals. Express how things relate to each other.",
                    exercise_count: 2,
                    difficulty: 2,
                    preview_code: Some("John loves Mary."),
                },
                ModuleData {
                    id: "negation",
                    title: "Negation",
                    description: "Negation flips truth values. Learn to express what is NOT the case.",
                    exercise_count: 2,
                    difficulty: 2,
                    preview_code: Some("John does not run."),
                },
            ],
        },
        EraData {
            id: "quadrivium",
            title: "Quantifiers",
            description: "Master universal and existential quantifiers that give logic its power to express general claims.",
            modules: vec![
                ModuleData {
                    id: "universal",
                    title: "Universal Quantification",
                    description: "The universal quantifier expresses claims about ALL individuals in a domain.",
                    exercise_count: 3,
                    difficulty: 3,
                    preview_code: Some("All birds fly."),
                },
                ModuleData {
                    id: "existential",
                    title: "Existential Quantification",
                    description: "The existential quantifier asserts that SOME individual exists with a property.",
                    exercise_count: 2,
                    difficulty: 3,
                    preview_code: Some("Some cats sleep."),
                },
                ModuleData {
                    id: "scope",
                    title: "Scope Ambiguity",
                    description: "Multiple quantifiers create ambiguity. Learn to disambiguate complex logical statements.",
                    exercise_count: 1,
                    difficulty: 4,
                    preview_code: Some("Every man loves a woman."),
                },
            ],
        },
        EraData {
            id: "metaphysics",
            title: "Modality & Time",
            description: "Express possibility, necessity, and temporal relationships. Advanced logical operators for rich reasoning.",
            modules: vec![
                ModuleData {
                    id: "modality",
                    title: "Modal Logic",
                    description: "Explore possibility and necessity in depth. What could be vs what must be.",
                    exercise_count: 2,
                    difficulty: 4,
                    preview_code: Some("John might run."),
                },
                ModuleData {
                    id: "time",
                    title: "Temporal Logic",
                    description: "Past and future operators. Reason about what was, is, and will be.",
                    exercise_count: 2,
                    difficulty: 4,
                    preview_code: Some("John will run."),
                },
            ],
        },
    ]
}

#[component]
pub fn Learn() -> Element {
    let mut active_module = use_signal(|| None::<String>);
    let mut pending_module = use_signal(|| None::<(String, String, String)>);
    let navigator = use_navigator();

    let eras = get_curriculum_data();

    // Build module info for sidebar
    let sidebar_modules: Vec<ModuleInfo> = eras.iter().flat_map(|era| {
        era.modules.iter().map(|m| ModuleInfo {
            era_id: era.id.to_string(),
            era_title: era.title.to_string(),
            module_id: m.id.to_string(),
            module_title: m.title.to_string(),
            exercise_count: m.exercise_count,
            difficulty: m.difficulty,
        })
    }).collect();

    // Collect all module IDs for intersection observer (used in wasm32 target)
    #[allow(unused_variables)]
    let module_ids: Vec<String> = eras.iter()
        .flat_map(|era| era.modules.iter().map(|m| m.id.to_string()))
        .collect();

    // Set up scroll tracking with IntersectionObserver
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::prelude::*;
        use wasm_bindgen::JsCast;

        let module_ids_for_effect = module_ids.clone();

        use_effect(move || {
            let window = match web_sys::window() {
                Some(w) => w,
                None => return,
            };
            let document = match window.document() {
                Some(d) => d,
                None => return,
            };

            // Create a closure that will be called when elements intersect
            // Use RefCell to allow mutation from within Fn closure
            use std::cell::RefCell;
            use std::rc::Rc;

            let active_module_clone = Rc::new(RefCell::new(active_module.clone()));
            let active_module_for_closure = active_module_clone.clone();

            let callback = Closure::<dyn Fn(js_sys::Array, web_sys::IntersectionObserver)>::new(
                move |entries: js_sys::Array, _observer: web_sys::IntersectionObserver| {
                    // Simple approach: when a module crosses the threshold line (enters from below),
                    // it becomes active. The threshold is set so modules activate when their top
                    // reaches ~100px from the top of the viewport.
                    for i in 0..entries.length() {
                        if let Ok(entry) = entries.get(i).dyn_into::<web_sys::IntersectionObserverEntry>() {
                            // Only activate when element is entering (crossing the threshold)
                            if entry.is_intersecting() {
                                let target = entry.target();
                                let id = target.id();
                                if !id.is_empty() {
                                    active_module_for_closure.borrow_mut().set(Some(id));
                                }
                            }
                        }
                    }
                },
            );

            // Create IntersectionObserver options
            let mut options = web_sys::IntersectionObserverInit::new();
            // Root margin: top offset of -100px means the "viewport" starts 100px below the actual top
            // Bottom margin of -90% means only the top 10% of viewport triggers intersection
            // This creates a thin "tripwire" near the top of the screen
            options.root_margin("-100px 0px -90% 0px");
            // Single threshold at 0 - fires once when element crosses the line
            let thresholds = js_sys::Array::new();
            thresholds.push(&JsValue::from(0.0));
            options.threshold(&thresholds);

            // Create the observer
            let observer = match web_sys::IntersectionObserver::new_with_options(
                callback.as_ref().unchecked_ref(),
                &options,
            ) {
                Ok(obs) => obs,
                Err(_) => return,
            };

            // Observe all module cards
            for module_id in &module_ids_for_effect {
                if let Some(element) = document.get_element_by_id(module_id) {
                    observer.observe(&element);
                }
            }

            // Keep callback alive
            callback.forget();
        });
    }

    // Track first era for divider logic
    let mut is_first_era = true;

    rsx! {
        style { "{LEARN_STYLE}" }

        div { class: "learn-page",
            MainNav { active: ActivePage::Learn }

            // Hero
            header { class: "learn-hero",
                div { class: "learn-hero-badge",
                    div { class: "dot" }
                    span { "Interactive Curriculum" }
                }
                h1 { "Learn Logic" }
                p {
                    "Master first-order logic through progressive challenges. Start with the basics and work your way up to advanced reasoning."
                }
            }

            // Main layout
            div { class: "learn-layout",
                // Sidebar
                LearnSidebar {
                    modules: sidebar_modules,
                    active_module: active_module.read().clone(),
                    on_module_click: move |(_era_id, module_id): (String, String)| {
                        active_module.set(Some(module_id));
                    },
                }

                // Content
                main { class: "learn-content",
                    for era in eras.iter() {
                        {
                            // Show divider for all eras except the first
                            let show_divider = !is_first_era;
                            is_first_era = false;

                            rsx! {
                                // Era divider
                                if show_divider {
                                    div { class: "learn-era-divider",
                                        h2 { "{era.title}" }
                                    }
                                }

                                // Era section
                                section { class: "learn-era",
                                    div { class: "learn-era-header",
                                        h2 { "{era.title}" }
                                        p { "{era.description}" }
                                    }

                                    div { class: "learn-modules",
                                        for (idx, module) in era.modules.iter().enumerate() {
                                            {
                                                let era_id = era.id.to_string();
                                                let module_id = module.id.to_string();
                                                let module_title = module.title.to_string();
                                                let module_number = idx + 1;

                                                rsx! {
                                                    div {
                                                        class: "learn-module-card",
                                                        id: "{module.id}",

                                                        div { class: "learn-module-header",
                                                            div { class: "learn-module-info",
                                                                h3 { class: "learn-module-title",
                                                                    span { class: "learn-module-number", "{module_number}." }
                                                                    "{module.title}"
                                                                }
                                                                p { class: "learn-module-desc", "{module.description}" }
                                                            }

                                                            div { class: "learn-module-meta",
                                                                span { class: "learn-exercise-count", "{module.exercise_count} exercises" }
                                                                div { class: "learn-difficulty",
                                                                    for i in 1..=5u8 {
                                                                        div {
                                                                            class: if i <= module.difficulty { "learn-difficulty-dot filled" } else { "learn-difficulty-dot" }
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }

                                                        // Preview code block
                                                        if let Some(preview) = module.preview_code {
                                                            div { class: "learn-module-preview",
                                                                div { class: "learn-preview-label", "Try an Example" }
                                                                GuideCodeBlock {
                                                                    id: format!("preview-{}", module.id),
                                                                    label: "Example".to_string(),
                                                                    mode: ExampleMode::Logic,
                                                                    initial_code: preview.to_string(),
                                                                }
                                                            }
                                                        }

                                                        // Action buttons
                                                        div { class: "learn-module-actions",
                                                            button {
                                                                class: "learn-action-btn primary",
                                                                onclick: {
                                                                    let era = era_id.clone();
                                                                    let module = module_id.clone();
                                                                    let title = module_title.clone();
                                                                    move |_| {
                                                                        pending_module.set(Some((
                                                                            era.clone(),
                                                                            module.clone(),
                                                                            title.clone(),
                                                                        )));
                                                                    }
                                                                },
                                                                "Start Learning"
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
            }

            // Mode selector modal
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
