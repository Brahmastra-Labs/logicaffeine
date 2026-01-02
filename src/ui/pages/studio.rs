use dioxus::prelude::*;
use crate::{compile_for_ui, CompileResult};
use crate::ui::components::editor::LiveEditor;
use crate::ui::components::logic_output::{LogicOutput, OutputFormat};
use crate::ui::components::ast_tree::AstTree;
use crate::ui::components::socratic_guide::{SocraticGuide, GuideMode, get_success_message, get_context_hint};
use crate::ui::components::main_nav::{MainNav, ActivePage};
use crate::ui::components::symbol_dictionary::SymbolDictionary;
use crate::ui::components::vocab_reference::VocabReference;
use crate::ui::responsive::{MOBILE_BASE_STYLES, MOBILE_TAB_BAR_STYLES};

/// Studio-specific styles that extend the shared responsive styles
const STUDIO_STYLE: &str = r#"
/* ============================================ */
/* STUDIO PAGE - Design Tokens                  */
/* ============================================ */
:root {
    --studio-bg: #0f1419;
    --studio-panel-bg: #12161c;
    --studio-elevated: #1a1f27;
    --studio-border: rgba(255, 255, 255, 0.08);
    --studio-border-hover: rgba(255, 255, 255, 0.15);
    --studio-text: #e8eaed;
    --studio-text-secondary: #9ca3af;
    --studio-text-muted: #6b7280;
    --studio-accent: #667eea;
}

/* ============================================ */
/* STUDIO PAGE - Desktop Layout                 */
/* ============================================ */
.studio-container {
    display: flex;
    flex-direction: column;
    height: 100vh;
    height: 100dvh;
    background: var(--studio-bg);
    color: var(--studio-text);
    overflow: hidden;
}

/* Desktop: 3-column panel layout */
.studio-main {
    flex: 1;
    display: flex;
    overflow: hidden;
    gap: 1px;
    background: var(--studio-border);
}

.studio-panel {
    background: var(--studio-panel-bg);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-width: 200px;
}

.studio-panel .panel-header {
    padding: 0 20px;
    height: 52px;
    background: rgba(255, 255, 255, 0.02);
    border-bottom: 1px solid var(--studio-border);
    font-size: 16px;
    font-weight: 600;
    letter-spacing: 0.3px;
    color: var(--studio-text);
    display: flex;
    justify-content: space-between;
    align-items: center;
    flex-shrink: 0;
}

.studio-panel .panel-content {
    flex: 1;
    overflow: auto;
    -webkit-overflow-scrolling: touch;
}

/* Panel Resizers (desktop only) */
.panel-resizer {
    width: 4px;
    background: var(--studio-border);
    cursor: col-resize;
    transition: background 0.2s ease;
    flex-shrink: 0;
}

.panel-resizer:hover,
.panel-resizer.active {
    background: var(--studio-accent);
}

/* Format Toggle (Unicode/LaTeX) */
.format-toggle {
    display: flex;
    gap: 4px;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid var(--studio-border);
    border-radius: 6px;
    padding: 2px;
}

.format-btn {
    padding: 4px 10px;
    border: none;
    background: transparent;
    color: var(--studio-text-muted);
    font-size: 12px;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.15s ease;
    line-height: 1;
}

.format-btn:hover {
    color: var(--studio-text);
    background: rgba(255, 255, 255, 0.04);
}

.format-btn.active {
    background: rgba(255, 255, 255, 0.08);
    color: var(--studio-text);
}

/* Guide Bar - above panels */
.studio-guide {
    background: var(--studio-panel-bg);
    border-bottom: 1px solid var(--studio-border);
    flex-shrink: 0;
}

/* ============================================ */
/* STUDIO PAGE - Mobile Overrides               */
/* ============================================ */
@media (max-width: 768px) {
    /* Hide desktop resizers */
    .panel-resizer {
        display: none;
    }

    /* Mobile main switches to column with stacked panels */
    .studio-main {
        flex-direction: column;
        position: relative;
        gap: 0;
        background: var(--studio-bg);
    }

    /* Panels are absolute positioned and hidden by default */
    .studio-panel {
        min-width: unset;
        min-height: unset;
        position: absolute;
        top: 0;
        left: 0;
        right: 0;
        bottom: 0;
        opacity: 0;
        pointer-events: none;
        transition: opacity 0.15s ease;
        width: 100% !important;
    }

    /* Active panel becomes visible */
    .studio-panel.mobile-active {
        position: relative;
        flex: 1;
        opacity: 1;
        pointer-events: auto;
    }

    /* Hide panel headers on mobile (tabs replace them) */
    .studio-panel .panel-header {
        display: none;
    }

    /* Show header only for Logic panel when it has format toggle */
    .studio-panel.mobile-active.has-controls .panel-header {
        display: flex;
        padding: 10px 14px;
        background: var(--studio-elevated);
        border-bottom: 1px solid var(--studio-border);
    }

    /* Mobile-sized format toggle */
    .format-toggle {
        gap: 6px;
        padding: 4px;
        border-radius: 8px;
    }

    .format-btn {
        padding: 10px 16px;
        font-size: 14px;
        border-radius: 6px;
        min-height: var(--touch-min, 44px);
        min-width: var(--touch-min, 44px);
        display: flex;
        align-items: center;
        justify-content: center;
    }

    /* Footer constraints */
    .studio-footer {
        max-height: 30vh;
        overflow: auto;
    }
}

/* Extra small screens */
@media (max-width: 480px) {
    .format-btn {
        padding: 8px 12px;
        font-size: 13px;
    }
}

/* Landscape mobile */
@media (max-height: 500px) and (orientation: landscape) {
    .studio-footer {
        max-height: 25vh;
    }
}
"#;

/// Mobile tab options
#[derive(Clone, Copy, PartialEq, Default)]
enum MobileTab {
    #[default]
    Input,
    Logic,
    Tree,
}

#[component]
pub fn Studio() -> Element {
    let mut input = use_signal(String::new);
    let mut result = use_signal(|| CompileResult {
        logic: None,
        ast: None,
        readings: Vec::new(),
        tokens: Vec::new(),
        error: None,
    });
    let mut format = use_signal(|| OutputFormat::Unicode);

    // Desktop panel resizing state
    let mut left_width = use_signal(|| 35.0f64);
    let mut right_width = use_signal(|| 25.0f64);
    let mut resizing = use_signal(|| None::<&'static str>);

    // Mobile tab state
    let mut active_tab = use_signal(|| MobileTab::Input);

    // Touch gesture state for swipe detection
    let mut touch_start_x = use_signal(|| 0.0f64);
    let mut touch_start_y = use_signal(|| 0.0f64);

    let handle_input = move |new_value: String| {
        input.set(new_value.clone());
        if !new_value.trim().is_empty() {
            let compiled = compile_for_ui(&new_value);
            result.set(compiled);
        } else {
            result.set(CompileResult {
                logic: None,
                ast: None,
                readings: Vec::new(),
                tokens: Vec::new(),
                error: None,
            });
        }
    };

    let current_result = result.read();
    let guide_mode = if let Some(err) = &current_result.error {
        GuideMode::Error(err.clone())
    } else if current_result.logic.is_some() {
        let msg = get_success_message(current_result.readings.len());
        if let Some(hint) = get_context_hint(&input.read()) {
            GuideMode::Info(format!("{} {}", msg, hint))
        } else {
            GuideMode::Success(msg)
        }
    } else {
        GuideMode::Idle
    };

    let left_w = *left_width.read();
    let right_w = *right_width.read();
    let center_w = 100.0 - left_w - right_w;

    // Desktop mouse handlers for panel resizing
    let handle_mouse_move = move |evt: MouseEvent| {
        if let Some(which) = *resizing.read() {
            let window = web_sys::window().unwrap();
            let width = window.inner_width().unwrap().as_f64().unwrap();
            let coords = evt.data().client_coordinates();
            let x: f64 = coords.x;
            let pct: f64 = (x / width) * 100.0;

            match which {
                "left" => {
                    let new_left: f64 = pct.clamp(15.0, 60.0);
                    left_width.set(new_left);
                }
                "right" => {
                    let new_right: f64 = (100.0 - pct).clamp(15.0, 40.0);
                    right_width.set(new_right);
                }
                _ => {}
            }
        }
    };

    let handle_mouse_up = move |_: MouseEvent| {
        resizing.set(None);
    };

    // Mobile touch handlers for swipe gestures
    let handle_touch_start = move |evt: TouchEvent| {
        let touches = evt.data().touches();
        if let Some(touch) = touches.first() {
            let coords = touch.client_coordinates();
            touch_start_x.set(coords.x);
            touch_start_y.set(coords.y);
        }
    };

    let handle_touch_end = move |evt: TouchEvent| {
        let changed = evt.data().touches_changed();
        if let Some(touch) = changed.first() {
            let coords = touch.client_coordinates();
            let end_x = coords.x;
            let end_y = coords.y;
            let dx = end_x - *touch_start_x.read();
            let dy = end_y - *touch_start_y.read();

            // Only trigger swipe if horizontal movement > vertical and > 50px threshold
            if dx.abs() > dy.abs() && dx.abs() > 50.0 {
                let current = *active_tab.read();
                if dx < 0.0 {
                    // Swipe left - go to next tab
                    match current {
                        MobileTab::Input => active_tab.set(MobileTab::Logic),
                        MobileTab::Logic => active_tab.set(MobileTab::Tree),
                        MobileTab::Tree => {} // Already at last tab
                    }
                } else {
                    // Swipe right - go to previous tab
                    match current {
                        MobileTab::Input => {} // Already at first tab
                        MobileTab::Logic => active_tab.set(MobileTab::Input),
                        MobileTab::Tree => active_tab.set(MobileTab::Logic),
                    }
                }
            }
        }
    };

    let current_format = *format.read();
    let current_tab = *active_tab.read();

    // Helper classes for panels based on active tab
    let input_panel_class = if current_tab == MobileTab::Input {
        "studio-panel mobile-active"
    } else {
        "studio-panel"
    };

    let logic_panel_class = if current_tab == MobileTab::Logic {
        "studio-panel mobile-active has-controls"
    } else {
        "studio-panel"
    };

    let tree_panel_class = if current_tab == MobileTab::Tree {
        "studio-panel mobile-active"
    } else {
        "studio-panel"
    };

    // Clone logic for VocabReference (needs owned String)
    let _vocab_logic = current_result.logic.clone();

    rsx! {
        // Include shared mobile styles from responsive module
        style { "{MOBILE_BASE_STYLES}" }
        style { "{MOBILE_TAB_BAR_STYLES}" }
        style { "{STUDIO_STYLE}" }

        div {
            class: "studio-container",
            onmousemove: handle_mouse_move,
            onmouseup: handle_mouse_up,
            onmouseleave: handle_mouse_up,
            ontouchstart: handle_touch_start,
            ontouchend: handle_touch_end,

            MainNav { active: ActivePage::Studio }

            // Mobile Tab Bar - shown only on mobile via CSS
            nav { class: "mobile-tabs",
                button {
                    class: if current_tab == MobileTab::Input { "mobile-tab active" } else { "mobile-tab" },
                    onclick: move |_| active_tab.set(MobileTab::Input),
                    span { class: "mobile-tab-icon", "\u{270F}" } // Pencil icon
                    span { class: "mobile-tab-label", "Input" }
                }
                button {
                    class: if current_tab == MobileTab::Logic { "mobile-tab active" } else { "mobile-tab" },
                    onclick: move |_| active_tab.set(MobileTab::Logic),
                    span { class: "mobile-tab-icon", "\u{2200}" } // Forall symbol
                    span { class: "mobile-tab-label", "Logic" }
                }
                button {
                    class: if current_tab == MobileTab::Tree { "mobile-tab active" } else { "mobile-tab" },
                    onclick: move |_| active_tab.set(MobileTab::Tree),
                    span { class: "mobile-tab-icon", "\u{1F333}" } // Tree emoji
                    span { class: "mobile-tab-label", "Tree" }
                }
            }

            // Socratic Guide - prominent position above panels
            div { class: "studio-guide",
                SocraticGuide {
                    mode: guide_mode.clone(),
                    on_hint_request: None,
                }
            }

            main { class: "studio-main",
                // Input Panel
                section {
                    class: "{input_panel_class}",
                    style: "width: {left_w}%;",
                    div { class: "panel-header",
                        span { "English Input" }
                    }
                    div { class: "panel-content",
                        LiveEditor {
                            on_change: handle_input,
                            placeholder: Some("Type an English sentence...".to_string()),
                        }
                    }
                }

                // Left resizer (desktop only)
                div {
                    class: if resizing.read().is_some() { "panel-resizer active" } else { "panel-resizer" },
                    onmousedown: move |_| resizing.set(Some("left")),
                }

                // Logic Output Panel
                section {
                    class: "{logic_panel_class}",
                    style: "width: {center_w}%;",
                    div { class: "panel-header",
                        span { "Logic Output" }
                        div { class: "format-toggle",
                            button {
                                class: if current_format == OutputFormat::Unicode { "format-btn active" } else { "format-btn" },
                                onclick: move |_| format.set(OutputFormat::Unicode),
                                "\u{2200}x"
                            }
                            button {
                                class: if current_format == OutputFormat::LaTeX { "format-btn active" } else { "format-btn" },
                                onclick: move |_| format.set(OutputFormat::LaTeX),
                                "LaTeX"
                            }
                        }
                    }
                    div { class: "panel-content",
                        LogicOutput {
                            logic: current_result.logic.clone(),
                            readings: current_result.readings.clone(),
                            error: current_result.error.clone(),
                            format: current_format,
                        }
                        // Symbol Dictionary - auto-generated from FOL output
                        if let Some(ref logic) = current_result.logic {
                            SymbolDictionary {
                                logic: logic.clone(),
                                collapsed: false,
                                inline: false,
                            }
                        }
                    }
                }

                // Right resizer (desktop only)
                div {
                    class: if resizing.read().is_some() { "panel-resizer active" } else { "panel-resizer" },
                    onmousedown: move |_| resizing.set(Some("right")),
                }

                // Syntax Tree Panel
                aside {
                    class: "{tree_panel_class}",
                    style: "width: {right_w}%;",
                    div { class: "panel-header",
                        span { "Syntax Tree" }
                    }
                    div { class: "panel-content",
                        AstTree {
                            ast: current_result.ast.clone(),
                        }
                    }
                }
            }

            // Floating vocab reference button
            VocabReference {}
        }
    }
}
