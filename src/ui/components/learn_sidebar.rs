//! Sidebar navigation component for the Learn/Curriculum page.
//!
//! Features:
//! - Sticky positioning
//! - Eras grouped with modules
//! - Active module highlighting
//! - Click navigation with scroll behavior
//! - Difficulty indicators

use dioxus::prelude::*;

const SIDEBAR_STYLE: &str = r#"
.learn-sidebar {
    position: sticky;
    top: 90px;
    width: 280px;
    max-height: calc(100vh - 120px);
    overflow-y: auto;
    flex-shrink: 0;
    padding: var(--spacing-sm) 0;

    /* Custom scrollbar */
    scrollbar-width: thin;
    scrollbar-color: rgba(255,255,255,0.1) transparent;
}

.learn-sidebar::-webkit-scrollbar {
    width: 6px;
}

.learn-sidebar::-webkit-scrollbar-track {
    background: transparent;
}

.learn-sidebar::-webkit-scrollbar-thumb {
    background: rgba(255,255,255,0.1);
    border-radius: 3px;
}

.learn-sidebar::-webkit-scrollbar-thumb:hover {
    background: rgba(255,255,255,0.2);
}

.learn-sidebar-era {
    margin-bottom: var(--spacing-xl);
}

.learn-sidebar-era:last-child {
    margin-bottom: 0;
}

.learn-sidebar-era-title {
    font-size: var(--font-caption-md);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.8px;
    color: var(--text-tertiary);
    padding: 0 var(--spacing-lg);
    margin-bottom: var(--spacing-sm);
    display: flex;
    align-items: center;
    gap: var(--spacing-sm);
}

.learn-sidebar-era-title::before {
    content: "";
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: linear-gradient(135deg, var(--color-accent-blue), var(--color-accent-purple));
    opacity: 0.6;
}

.learn-sidebar-module {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px var(--spacing-lg);
    margin: 2px var(--spacing-sm);
    border-radius: var(--radius-md);
    color: var(--text-secondary);
    font-size: var(--font-body-md);
    font-weight: 500;
    text-decoration: none;
    transition: all 0.18s ease;
    cursor: pointer;
    border-left: 3px solid transparent;
}

.learn-sidebar-module:hover {
    background: rgba(255,255,255,0.06);
    color: var(--text-primary);
}

.learn-sidebar-module.active {
    background: rgba(96,165,250,0.15);
    color: var(--color-accent-blue);
    border-left-color: var(--color-accent-blue);
    font-weight: 600;
}

.learn-sidebar-module-name {
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

.learn-sidebar-difficulty {
    display: flex;
    gap: 2px;
    margin-left: var(--spacing-sm);
    flex-shrink: 0;
}

.learn-sidebar-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: rgba(255,255,255,0.15);
}

.learn-sidebar-dot.filled {
    background: linear-gradient(135deg, var(--color-accent-blue), var(--color-accent-purple));
}

.learn-sidebar-module.active .learn-sidebar-dot {
    background: rgba(96,165,250,0.3);
}

.learn-sidebar-module.active .learn-sidebar-dot.filled {
    background: var(--color-accent-blue);
}

.learn-sidebar-count {
    font-size: var(--font-caption-md);
    color: var(--text-muted);
    margin-left: 6px;
    flex-shrink: 0;
}

.learn-sidebar-module.active .learn-sidebar-count {
    color: rgba(96,165,250,0.7);
}

@media (max-width: 1024px) {
    .learn-sidebar {
        display: none;
    }
}

/* Mobile sidebar module touch targets (for mobile sheet view) */
@media (max-width: 768px) {
    .learn-sidebar-module {
        min-height: 44px;
        padding: 12px var(--spacing-lg);
        -webkit-tap-highlight-color: transparent;
        touch-action: manipulation;
    }
}
"#;

/// Information about a module for the sidebar
#[derive(Clone, PartialEq, Debug)]
pub struct ModuleInfo {
    pub era_id: String,
    pub era_title: String,
    pub module_id: String,
    pub module_title: String,
    pub exercise_count: u32,
    pub difficulty: u8,
}

#[derive(Props, Clone, PartialEq)]
pub struct LearnSidebarProps {
    pub modules: Vec<ModuleInfo>,
    pub active_module: Option<String>,
    pub on_module_click: EventHandler<(String, String)>, // (era_id, module_id)
}

#[component]
pub fn LearnSidebar(props: LearnSidebarProps) -> Element {
    // Group modules by era
    let grouped = group_modules_by_era(&props.modules);

    rsx! {
        style { "{SIDEBAR_STYLE}" }

        nav { class: "learn-sidebar",
            for (era_id, era_title, modules) in grouped {
                div { class: "learn-sidebar-era",
                    h4 { class: "learn-sidebar-era-title", "{era_title}" }

                    for module in modules {
                        {
                            let is_active = props.active_module.as_ref() == Some(&module.module_id);
                            let class_name = if is_active {
                                "learn-sidebar-module active"
                            } else {
                                "learn-sidebar-module"
                            };

                            let era_for_click = era_id.clone();
                            let mod_for_click = module.module_id.clone();

                            rsx! {
                                a {
                                    class: "{class_name}",
                                    href: "#{module.module_id}",
                                    onclick: {
                                        let era = era_for_click.clone();
                                        let module_id = mod_for_click.clone();
                                        let handler = props.on_module_click.clone();
                                        move |evt: Event<MouseData>| {
                                            evt.prevent_default();
                                            handler.call((era.clone(), module_id.clone()));

                                            // Smooth scroll to section
                                            #[cfg(target_arch = "wasm32")]
                                            {
                                                if let Some(window) = web_sys::window() {
                                                    if let Some(document) = window.document() {
                                                        if let Some(element) = document.get_element_by_id(&module_id) {
                                                            let options = web_sys::ScrollIntoViewOptions::new();
                                                            options.set_behavior(web_sys::ScrollBehavior::Smooth);
                                                            options.set_block(web_sys::ScrollLogicalPosition::Start);
                                                            let _ = element.scroll_into_view_with_scroll_into_view_options(&options);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    },
                                    span { class: "learn-sidebar-module-name", "{module.module_title}" }

                                    // Difficulty dots
                                    div { class: "learn-sidebar-difficulty",
                                        for i in 1..=5u8 {
                                            div {
                                                class: if i <= module.difficulty { "learn-sidebar-dot filled" } else { "learn-sidebar-dot" }
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

/// Groups modules by their era, preserving order
fn group_modules_by_era(modules: &[ModuleInfo]) -> Vec<(String, String, Vec<ModuleInfo>)> {
    let mut result: Vec<(String, String, Vec<ModuleInfo>)> = Vec::new();

    for module in modules {
        if let Some((_, _, group)) = result.iter_mut().find(|(era_id, _, _)| era_id == &module.era_id) {
            group.push(module.clone());
        } else {
            result.push((module.era_id.clone(), module.era_title.clone(), vec![module.clone()]));
        }
    }

    result
}
