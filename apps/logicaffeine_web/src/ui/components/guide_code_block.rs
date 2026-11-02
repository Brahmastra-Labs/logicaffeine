//! Interactive code block component for the Programmer's Guide.
//!
//! Features:
//! - The Studio's editor with the language server: highlighting, hovers, squiggles
//! - Run button (Logic mode: FOL output, Imperative mode: interpreter, Web mode: the page
//!   compiled to WebAssembly and shown live in a sandboxed frame)
//! - Copy button
//! - Reset button
//! - Output panel

use super::inline_editor::{InlineEditor, InlineEditorStyle};
use crate::ui::pages::guide::content::{ExampleMode, Runnability};
use crate::ui::pages::guide::theorem_verdict;
use dioxus::prelude::*;
use logicaffeine_compile::{compile_for_ui, compile_theorem_for_ui, interpret_for_ui};

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
    flex-wrap: wrap;
    justify-content: space-between;
    align-items: center;
    gap: 10px;
    padding: 12px 16px;
    background: rgba(255,255,255,0.04);
    border-bottom: 1px solid rgba(255,255,255,0.08);
}

.guide-code-label {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 10px;
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
    flex-wrap: wrap;
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

.guide-code-mode.web {
    background: rgba(52,211,153,0.15);
    color: #34d399;
    border: 1px solid rgba(52,211,153,0.3);
}

.guide-code-preview {
    display: block;
    width: 100%;
    height: 320px;
    border: 0;
    background: #faf9f5;
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

.guide-code-compiled-pill {
    font-size: 11px;
    padding: 4px 10px;
    border-radius: 999px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    background: rgba(251,191,36,0.15);
    color: #fbbf24;
    border: 1px solid rgba(251,191,36,0.3);
    cursor: help;
    white-space: nowrap;
}
"#;

#[derive(Props, Clone, PartialEq)]
pub struct GuideCodeBlockProps {
    pub id: String,
    pub label: String,
    pub mode: ExampleMode,
    pub runnability: Runnability,
    pub expected_output: Option<String>,
    /// Predict-first (Muller/POE): start with the locked expectation HIDDEN
    /// and ask the reader to commit to a prediction before revealing.
    pub predict: bool,
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
    let mut predict_hidden = use_signal(|| props.predict);
    let mut preview = use_signal(String::new);

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
            ExampleMode::Theorem => {
                // The proof engine + CoC kernel run in-process — the verdict
                // is the output.
                let result = compile_theorem_for_ui(&current_code);
                match theorem_verdict(&result) {
                    Ok(verdict) => {
                        output.set(verdict);
                        output_type.set("success".to_string());
                    }
                    Err(err) => {
                        output.set(err);
                        output_type.set("error".to_string());
                    }
                }
            }
            ExampleMode::Web => {
                // The Studio's Web preview: the page compiled to WebAssembly here, in the browser,
                // and handed to a sandboxed frame as its whole document. A build error becomes the
                // themed overlay with the offending line, in the same frame.
                let doc = match logicaffeine_compile::web_host::assemble_preview(&current_code) {
                    Ok(srcdoc) => {
                        output_type.set("success".to_string());
                        srcdoc
                    }
                    Err(pe) => {
                        output_type.set("error".to_string());
                        logicaffeine_compile::web_host::error_overlay_html(&pe, &current_code, "")
                    }
                };
                preview.set(doc);
            }
            ExampleMode::Imperative => {
                // Phase 55: interpret_for_ui is now async for VFS support
                spawn(async move {
                    let result = interpret_for_ui(&current_code).await;
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
                    is_running.set(false);
                });
                return;
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
            preview.set(String::new());
            has_run.set(false);
        }
    };

    let mode_class = match mode {
        ExampleMode::Logic | ExampleMode::Theorem => "logic",
        ExampleMode::Imperative => "imperative",
        ExampleMode::Web => "web",
    };

    let mode_label = match mode {
        ExampleMode::Logic => "Logic",
        ExampleMode::Theorem => "Prove",
        ExampleMode::Imperative => "Run",
        ExampleMode::Web => "Open page",
    };

    // The runnability declaration drives the affordance: a CompiledOnly example
    // gets a pill carrying its reason instead of a Run button that would fail.
    let compiled_reason: Option<&'static str> = match props.runnability {
        Runnability::CompiledOnly { reason } => Some(reason),
        _ => None,
    };

    // Before the first run, an example with locked content shows it up front:
    // the expected output (Playground) or the expected diagnostic (ExpectedError).
    let prepopulated: Option<(&'static str, String, &'static str)> = if *has_run.read() {
        None
    } else {
        match props.runnability {
            Runnability::Playground => props
                .expected_output
                .clone()
                .map(|text| ("Expected output", text, "info")),
            Runnability::ExpectedError { message } => {
                Some(("Expected error", message.to_string(), "error"))
            }
            Runnability::CompiledOnly { .. } => None,
        }
    };

    rsx! {
        style { "{CODE_BLOCK_STYLE}" }
        InlineEditorStyle {}

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
                            ExampleMode::Theorem => "Theorem",
                            ExampleMode::Imperative => "Imperative",
                            ExampleMode::Web => "Web page",
                        }
                    }
                }

                div { class: "guide-code-actions",
                    if let Some(reason) = compiled_reason {
                        a {
                            class: "guide-code-compiled-pill",
                            title: "{reason}",
                            href: "/guide#appendix-runnability",
                            "Compiled — run with largo"
                        }
                    } else {
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
                InlineEditor {
                    value: code.read().clone(),
                    on_change: move |v| code.set(v),
                    language: mode.language(),
                }

                if *show_copied.read() {
                    div { class: "guide-code-copied", "Copied!" }
                }
            }

            // Output: the live page for a web example; the actual output after a run; the locked
            // expectation before it.
            if *has_run.read() && mode == ExampleMode::Web {
                div { class: "guide-code-output",
                    div { class: "guide-code-output-header",
                        span { "Page" }
                    }
                    iframe {
                        class: "guide-code-preview",
                        srcdoc: "{preview}",
                        "sandbox": "allow-scripts",
                    }
                }
            } else if *has_run.read() {
                div { class: "guide-code-output",
                    div { class: "guide-code-output-header",
                        span { "Output" }
                    }
                    div {
                        class: "guide-code-output-content {output_type}",
                        "{output}"
                    }
                }
            } else if let Some((header, text, kind)) = prepopulated {
                div { class: "guide-code-output",
                    div { class: "guide-code-output-header",
                        // While hidden, the header asks for a committed
                        // prediction instead of naming the answer's category.
                        if *predict_hidden.read() {
                            span {
                                match (props.runnability, mode) {
                                    (Runnability::ExpectedError { .. }, _) => "What error do you expect?",
                                    (_, ExampleMode::Logic) => "What FOL does this produce?",
                                    (_, ExampleMode::Theorem) => "What verdict do you expect?",
                                    (_, ExampleMode::Imperative) => "What will this print?",
                                    (_, ExampleMode::Web) => "What will this page show?",
                                }
                            }
                        } else {
                            span { "{header}" }
                        }
                        // Predict-the-output: hide the answer, guess, reveal.
                        button {
                            class: "guide-predict-toggle",
                            style: "background: none; border: 1px solid rgba(255,255,255,0.15); border-radius: 6px; color: rgba(229,231,235,0.6); font-size: 10px; padding: 2px 8px; cursor: pointer; text-transform: none; letter-spacing: 0;",
                            onclick: move |_| { let v = *predict_hidden.read(); predict_hidden.set(!v); },
                            if *predict_hidden.read() { "Reveal" } else { "Hide & predict" }
                        }
                    }
                    if !*predict_hidden.read() {
                        div {
                            class: "guide-code-output-content {kind}",
                            "{text}"
                        }
                    }
                }
            }
        }
    }
}
