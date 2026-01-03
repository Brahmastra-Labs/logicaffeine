use dioxus::prelude::*;
use crate::{compile_for_ui, CompileResult};
use crate::ui::components::editor::LiveEditor;
use crate::ui::components::logic_output::{LogicOutput, OutputFormat};
use crate::ui::components::ast_tree::AstTree;
use crate::ui::components::socratic_guide::{SocraticGuide, GuideMode, get_success_message, get_context_hint};
use crate::ui::components::main_nav::{MainNav, ActivePage};

const STUDIO_STYLE: &str = r#"
.studio-container {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f3460 100%);
    color: #e8e8e8;
}

.studio-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 16px 24px;
    background: rgba(0, 0, 0, 0.2);
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
}

.studio-logo {
    display: flex;
    align-items: center;
    gap: 12px;
}

.studio-logo-icon {
    font-size: 24px;
}

.studio-logo-text {
    font-size: 20px;
    font-weight: 700;
    background: linear-gradient(135deg, #667eea, #764ba2);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
}

.studio-nav {
    display: flex;
    gap: 8px;
}

.studio-nav-btn {
    padding: 8px 16px;
    border-radius: 8px;
    border: 1px solid rgba(255, 255, 255, 0.15);
    background: rgba(255, 255, 255, 0.05);
    color: #888;
    font-size: 14px;
    cursor: pointer;
    transition: all 0.2s ease;
    text-decoration: none;
}

.studio-nav-btn:hover {
    background: rgba(255, 255, 255, 0.1);
    color: #e8e8e8;
}

.studio-main {
    flex: 1;
    display: flex;
    overflow: hidden;
}

.studio-panel {
    background: rgba(0, 0, 0, 0.3);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-width: 200px;
}

.panel-header {
    padding: 12px 16px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: #888;
    display: flex;
    justify-content: space-between;
    align-items: center;
    flex-shrink: 0;
}

.panel-content {
    flex: 1;
    overflow: auto;
}

.panel-resizer {
    width: 6px;
    background: rgba(255, 255, 255, 0.05);
    cursor: col-resize;
    transition: background 0.2s ease;
    flex-shrink: 0;
}

.panel-resizer:hover,
.panel-resizer.active {
    background: rgba(102, 126, 234, 0.5);
}

.format-toggle {
    display: flex;
    gap: 4px;
    background: rgba(255, 255, 255, 0.05);
    border-radius: 6px;
    padding: 2px;
}

.format-btn {
    padding: 4px 10px;
    border: none;
    background: transparent;
    color: #888;
    font-size: 11px;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.2s ease;
}

.format-btn:hover {
    color: #e8e8e8;
}

.format-btn.active {
    background: rgba(255, 255, 255, 0.1);
    color: #e8e8e8;
}

.studio-footer {
    background: rgba(0, 0, 0, 0.3);
    border-top: 1px solid rgba(255, 255, 255, 0.08);
}

@media (max-width: 768px) {
    .studio-main {
        flex-direction: column;
    }
    .panel-resizer {
        width: 100%;
        height: 6px;
        cursor: row-resize;
    }
    .studio-panel {
        min-width: unset;
        min-height: 150px;
    }
}
"#;

#[component]
pub fn Studio() -> Element {
    let mut input = use_signal(String::new);
    let mut result = use_signal(|| CompileResult {
        logic: None,
        simple_logic: None,
        kripke_logic: None,
        ast: None,
        readings: Vec::new(),
        simple_readings: Vec::new(),
        kripke_readings: Vec::new(),
        tokens: Vec::new(),
        error: None,
    });
    let mut format = use_signal(|| OutputFormat::SimpleFOL);

    let mut left_width = use_signal(|| 35.0f64);
    let mut right_width = use_signal(|| 25.0f64);
    let mut resizing = use_signal(|| None::<&'static str>);

    let handle_input = move |new_value: String| {
        input.set(new_value.clone());
        if !new_value.trim().is_empty() {
            let compiled = compile_for_ui(&new_value);
            result.set(compiled);
        } else {
            result.set(CompileResult {
                logic: None,
                simple_logic: None,
                kripke_logic: None,
                ast: None,
                readings: Vec::new(),
                simple_readings: Vec::new(),
                kripke_readings: Vec::new(),
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

    let current_format = *format.read();

    rsx! {
        style { "{STUDIO_STYLE}" }

        div {
            class: "studio-container",
            onmousemove: handle_mouse_move,
            onmouseup: handle_mouse_up,
            onmouseleave: handle_mouse_up,

            MainNav { active: ActivePage::Studio }

            main { class: "studio-main",
                section {
                    class: "studio-panel",
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

                div {
                    class: if resizing.read().is_some() { "panel-resizer active" } else { "panel-resizer" },
                    onmousedown: move |_| resizing.set(Some("left")),
                }

                section {
                    class: "studio-panel",
                    style: "width: {center_w}%;",
                    div { class: "panel-header",
                        span { "Logic Output" }
                        div { class: "format-toggle",
                            button {
                                class: if current_format == OutputFormat::SimpleFOL { "format-btn active" } else { "format-btn" },
                                onclick: move |_| format.set(OutputFormat::SimpleFOL),
                                "Simple"
                            }
                            button {
                                class: if current_format == OutputFormat::Unicode { "format-btn active" } else { "format-btn" },
                                onclick: move |_| format.set(OutputFormat::Unicode),
                                "Full"
                            }
                            button {
                                class: if current_format == OutputFormat::LaTeX { "format-btn active" } else { "format-btn" },
                                onclick: move |_| format.set(OutputFormat::LaTeX),
                                "LaTeX"
                            }
                            button {
                                class: if current_format == OutputFormat::Kripke { "format-btn active" } else { "format-btn" },
                                onclick: move |_| format.set(OutputFormat::Kripke),
                                "Deep"
                            }
                        }
                    }
                    div { class: "panel-content",
                        LogicOutput {
                            logic: current_result.logic.clone(),
                            simple_logic: current_result.simple_logic.clone(),
                            kripke_logic: current_result.kripke_logic.clone(),
                            readings: current_result.readings.clone(),
                            simple_readings: current_result.simple_readings.clone(),
                            kripke_readings: current_result.kripke_readings.clone(),
                            error: current_result.error.clone(),
                            format: current_format,
                        }
                    }
                }

                div {
                    class: if resizing.read().is_some() { "panel-resizer active" } else { "panel-resizer" },
                    onmousedown: move |_| resizing.set(Some("right")),
                }

                aside {
                    class: "studio-panel",
                    style: "width: {right_w}%;",
                    div { class: "panel-header",
                        span { "AST Inspector" }
                    }
                    div { class: "panel-content",
                        AstTree {
                            ast: current_result.ast.clone(),
                        }
                    }
                }
            }

            footer { class: "studio-footer",
                SocraticGuide {
                    mode: guide_mode,
                    on_hint_request: None,
                }
            }
        }
    }
}
