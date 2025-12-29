//! Interactive code block component for the Programmer's Guide.
//!
//! Features:
//! - Editable code area
//! - Run button (Logic mode: FOL output, Imperative mode: interpreter)
//! - Copy button
//! - Reset button
//! - Output panel

use dioxus::prelude::*;
use crate::ui::pages::guide::content::ExampleMode;
use crate::compile_for_ui;
use crate::interpret_for_ui;

const CODE_BLOCK_STYLE: &str = r#"
.guide-code-block {
    border-radius: 16px;
    border: 1px solid rgba(255,255,255,0.10);
    background: rgba(0,0,0,0.35);
    overflow: hidden;
    margin: 20px 0;
    box-shadow: 0 8px 32px rgba(0,0,0,0.3);
}

.guide-code-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 12px 16px;
    background: rgba(255,255,255,0.04);
    border-bottom: 1px solid rgba(255,255,255,0.08);
}

.guide-code-label {
    display: flex;
    align-items: center;
    gap: 10px;
}

.guide-code-title {
    font-size: 13px;
    font-weight: 600;
    color: rgba(229,231,235,0.9);
}

.guide-code-mode {
    font-size: 11px;
    padding: 4px 10px;
    border-radius: 999px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
}

.guide-code-mode.logic {
    background: rgba(167,139,250,0.2);
    color: #a78bfa;
    border: 1px solid rgba(167,139,250,0.3);
}

.guide-code-mode.imperative {
    background: rgba(34,197,94,0.2);
    color: #22c55e;
    border: 1px solid rgba(34,197,94,0.3);
}

.guide-code-actions {
    display: flex;
    gap: 8px;
}

.guide-code-btn {
    padding: 8px 14px;
    border-radius: 8px;
    border: 1px solid rgba(255,255,255,0.12);
    background: rgba(255,255,255,0.06);
    color: rgba(229,231,235,0.8);
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.2s ease;
    display: flex;
    align-items: center;
    gap: 6px;
}

.guide-code-btn:hover {
    background: rgba(255,255,255,0.10);
    border-color: rgba(255,255,255,0.20);
    color: #fff;
}

.guide-code-btn:active {
    transform: scale(0.97);
}

.guide-code-btn.primary {
    background: linear-gradient(135deg, rgba(96,165,250,0.9), rgba(167,139,250,0.9));
    border-color: rgba(255,255,255,0.2);
    color: #060814;
}

.guide-code-btn.primary:hover {
    background: linear-gradient(135deg, #60a5fa, #a78bfa);
}

.guide-code-btn.running {
    opacity: 0.7;
    cursor: wait;
}

.guide-code-editor {
    position: relative;
}

.guide-code-textarea {
    width: 100%;
    min-height: 120px;
    padding: 16px;
    background: transparent;
    border: none;
    color: rgba(229,231,235,0.95);
    font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Monaco, 'Cascadia Code', monospace;
    font-size: 14px;
    line-height: 1.7;
    resize: vertical;
    outline: none;
    tab-size: 4;
}

.guide-code-textarea::placeholder {
    color: rgba(229,231,235,0.3);
}

.guide-code-output {
    border-top: 1px solid rgba(255,255,255,0.08);
    background: rgba(0,0,0,0.25);
}

.guide-code-output-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 16px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: rgba(229,231,235,0.5);
    border-bottom: 1px solid rgba(255,255,255,0.06);
}

.guide-code-output-content {
    padding: 16px;
    font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Monaco, monospace;
    font-size: 14px;
    line-height: 1.6;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 300px;
    overflow-y: auto;
}

.guide-code-output-content.success {
    color: #a78bfa;
}

.guide-code-output-content.error {
    color: #f87171;
}

.guide-code-output-content.info {
    color: rgba(229,231,235,0.7);
}

.guide-code-copied {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    padding: 8px 16px;
    background: rgba(34,197,94,0.9);
    color: #fff;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 600;
    animation: fadeInOut 1.5s ease forwards;
    pointer-events: none;
}

@keyframes fadeInOut {
    0% { opacity: 0; transform: translate(-50%, -50%) scale(0.9); }
    15% { opacity: 1; transform: translate(-50%, -50%) scale(1); }
    85% { opacity: 1; }
    100% { opacity: 0; }
}

.guide-code-placeholder {
    padding: 24px 16px;
    text-align: center;
    color: rgba(229,231,235,0.4);
    font-size: 13px;
}
"#;

#[derive(Props, Clone, PartialEq)]
pub struct GuideCodeBlockProps {
    pub id: String,
    pub label: String,
    pub mode: ExampleMode,
    pub initial_code: String,
}

#[component]
pub fn GuideCodeBlock(props: GuideCodeBlockProps) -> Element {
    let mut code = use_signal(|| props.initial_code.clone());
    let mut output = use_signal(String::new);
    let mut output_type = use_signal(|| "info".to_string());
    let mut is_running = use_signal(|| false);
    let mut show_copied = use_signal(|| false);
    let mut has_run = use_signal(|| false);

    let initial_code = props.initial_code.clone();
    let mode = props.mode;
    let id = props.id.clone();

    // Run handler
    let handle_run = move |_| {
        is_running.set(true);
        has_run.set(true);

        let current_code = code.read().clone();

        match mode {
            ExampleMode::Logic => {
                // Use compile_for_ui for Logic mode
                let result = compile_for_ui(&current_code);
                if let Some(logic) = result.logic {
                    output.set(logic);
                    output_type.set("success".to_string());
                } else if let Some(err) = result.error {
                    output.set(err);
                    output_type.set("error".to_string());
                } else {
                    output.set("No output".to_string());
                    output_type.set("info".to_string());
                }
            }
            ExampleMode::Imperative => {
                // Use the real LOGOS parser + tree-walking interpreter
                let result = interpret_for_ui(&current_code);
                if let Some(err) = result.error {
                    output.set(err);
                    output_type.set("error".to_string());
                } else if result.lines.is_empty() {
                    output.set("(no output)".to_string());
                    output_type.set("info".to_string());
                } else {
                    output.set(result.lines.join("\n"));
                    output_type.set("success".to_string());
                }
            }
        }

        is_running.set(false);
    };

    // Copy handler
    let handle_copy = move |_| {
        let code_to_copy = code.read().clone();

        #[cfg(target_arch = "wasm32")]
        {
            if let Some(window) = web_sys::window() {
                let clipboard = window.navigator().clipboard();
                let _ = clipboard.write_text(&code_to_copy);
                show_copied.set(true);

                // Reset after animation
                spawn(async move {
                    gloo_timers::future::TimeoutFuture::new(1500).await;
                    show_copied.set(false);
                });
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = code_to_copy;
            show_copied.set(true);
        }
    };

    // Reset handler
    let handle_reset = {
        let initial = initial_code.clone();
        move |_| {
            code.set(initial.clone());
            output.set(String::new());
            has_run.set(false);
        }
    };

    let mode_class = match mode {
        ExampleMode::Logic => "logic",
        ExampleMode::Imperative => "imperative",
    };

    let mode_label = match mode {
        ExampleMode::Logic => "Logic",
        ExampleMode::Imperative => "Run",
    };

    rsx! {
        style { "{CODE_BLOCK_STYLE}" }

        div {
            class: "guide-code-block",
            id: "{id}",

            // Header
            div { class: "guide-code-header",
                div { class: "guide-code-label",
                    span { class: "guide-code-title", "{props.label}" }
                    span { class: "guide-code-mode {mode_class}",
                        match mode {
                            ExampleMode::Logic => "Logic Mode",
                            ExampleMode::Imperative => "Imperative",
                        }
                    }
                }

                div { class: "guide-code-actions",
                    button {
                        class: if *is_running.read() { "guide-code-btn primary running" } else { "guide-code-btn primary" },
                        onclick: handle_run,
                        disabled: *is_running.read(),
                        if *is_running.read() {
                            "Running..."
                        } else {
                            "{mode_label}"
                        }
                    }
                    button {
                        class: "guide-code-btn",
                        onclick: handle_copy,
                        "Copy"
                    }
                    button {
                        class: "guide-code-btn",
                        onclick: handle_reset,
                        "Reset"
                    }
                }
            }

            // Editor
            div { class: "guide-code-editor",
                textarea {
                    class: "guide-code-textarea",
                    value: "{code}",
                    oninput: move |evt| code.set(evt.value()),
                    spellcheck: "false",
                    autocomplete: "off",
                    autocapitalize: "off",
                }

                if *show_copied.read() {
                    div { class: "guide-code-copied", "Copied!" }
                }
            }

            // Output (only show if has run)
            if *has_run.read() {
                div { class: "guide-code-output",
                    div { class: "guide-code-output-header",
                        span { "Output" }
                    }
                    div {
                        class: "guide-code-output-content {output_type}",
                        "{output}"
                    }
                }
            }
        }
    }
}
