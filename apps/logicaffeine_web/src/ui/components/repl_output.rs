//! REPL output for Math mode.
//!
//! Displays command history and execution results in a terminal-like interface.
//! Shows success output in green and errors in red. The panel around it carries the
//! title and the Clear control.
//!
//! # Props
//!
//! - `lines` - List of executed commands and their results

use crate::ui::state::ReplLine;
use dioxus::prelude::*;

const REPL_OUTPUT_STYLE: &str = r#"
.repl-output {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--studio-surface, #0f1419);
    font-family: 'SF Mono', 'Fira Code', monospace;
}


.repl-history {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: var(--studio-inset, 12px);
}

.repl-line {
    margin-bottom: 16px;
}

/* The history ends at its last line; the panel's own inset is the only space after it. */
.repl-line:last-child {
    margin-bottom: 0;
}

.repl-input-line {
    display: flex;
    gap: 8px;
    color: rgba(255, 255, 255, 0.6);
    font-size: 13px;
    line-height: 1.5;
}

.repl-prompt {
    color: #667eea;
    user-select: none;
    flex-shrink: 0;
}

.repl-input-text {
    color: rgba(255, 255, 255, 0.9);
    white-space: pre-wrap;
    word-break: break-word;
}

.repl-output-line {
    padding-left: 24px;
    margin-top: 4px;
    font-size: 13px;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
}

.repl-output-line.success {
    color: #4ade80;
}

.repl-output-line.error {
    color: #e06c75;
}

.repl-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: rgba(255, 255, 255, 0.4);
    text-align: center;
    padding: 20px;
}

.repl-empty .hint {
    font-size: 13px;
    margin-top: 8px;
}

.repl-empty .commands {
    margin-top: 16px;
    font-size: 12px;
    color: rgba(255, 255, 255, 0.3);
}

.repl-empty .commands code {
    background: rgba(255, 255, 255, 0.05);
    padding: 2px 6px;
    border-radius: 4px;
    margin: 0 2px;
}
"#;

#[component]
pub fn ReplOutput(lines: Vec<ReplLine>) -> Element {
    rsx! {
        style { "{REPL_OUTPUT_STYLE}" }

        div { class: "repl-output",
            // History
            div { class: "repl-history",
                if lines.is_empty() {
                    div { class: "repl-empty",
                        div { "No output yet" }
                        div { class: "hint", "Execute code to see results here" }
                        div { class: "commands",
                            "Commands: "
                            code { "Definition" }
                            code { "Check" }
                            code { "Eval" }
                            code { "Inductive" }
                        }
                    }
                } else {
                    for (i, line) in lines.iter().enumerate() {
                        div { key: "{i}", class: "repl-line",
                            // Input line
                            div { class: "repl-input-line",
                                span { class: "repl-prompt", ">" }
                                span { class: "repl-input-text", "{line.input}" }
                            }

                            // Output line
                            match &line.output {
                                Ok(output) if !output.is_empty() => rsx! {
                                    div { class: "repl-output-line success", "{output}" }
                                },
                                Err(error) => rsx! {
                                    div { class: "repl-output-line error", "{error}" }
                                },
                                _ => rsx! {}
                            }
                        }
                    }
                }
            }
        }
    }
}
