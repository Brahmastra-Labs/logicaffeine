//! LaTeX formula editor with live preview.
//!
//! Provides a split-pane interface with a LaTeX input area and real-time
//! KaTeX rendering. Includes quick-insert buttons for common symbols.
//!
//! # Components
//!
//! - [`FormulaEditor`] - Full editor with input, quick-insert, and preview
//! - [`LatexPreview`] - Standalone preview component
//!
//! # Props (FormulaEditor)
//!
//! - `latex` - Current LaTeX source
//! - `on_change` - Callback when source changes
//! - `on_insert` - Optional callback for quick-insert buttons

use dioxus::prelude::*;
use super::katex::KatexSpan;

const FORMULA_EDITOR_STYLE: &str = r#"
.formula-editor {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: #0f1419;
}

.formula-editor-input {
    flex: 1;
    display: flex;
    flex-direction: column;
    padding: 16px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
}

.formula-editor-label {
    font-size: 12px;
    font-weight: 600;
    color: rgba(255, 255, 255, 0.5);
    margin-bottom: 8px;
    text-transform: uppercase;
    letter-spacing: 0.5px;
}

.formula-textarea {
    flex: 1;
    width: 100%;
    min-height: 120px;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 8px;
    padding: 14px;
    font-size: 15px;
    font-family: 'SF Mono', 'Fira Code', monospace;
    color: rgba(255, 255, 255, 0.9);
    resize: none;
    outline: none;
    line-height: 1.6;
}

.formula-textarea:focus {
    border-color: rgba(102, 126, 234, 0.5);
    box-shadow: 0 0 0 3px rgba(102, 126, 234, 0.15);
}

.formula-textarea::placeholder {
    color: rgba(255, 255, 255, 0.3);
}

.formula-preview {
    padding: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 150px;
    background: rgba(255, 255, 255, 0.02);
}

.formula-preview-empty {
    color: rgba(255, 255, 255, 0.3);
    font-size: 14px;
    text-align: center;
}

.formula-preview .katex-display {
    font-size: 1.8em;
}

.formula-error {
    color: #e06c75;
    padding: 12px 16px;
    background: rgba(224, 108, 117, 0.1);
    border-radius: 6px;
    font-size: 13px;
    margin: 16px;
}

/* Quick insert bar */
.formula-quick-insert {
    display: flex;
    gap: 4px;
    padding: 8px 16px;
    border-top: 1px solid rgba(255, 255, 255, 0.06);
    background: rgba(255, 255, 255, 0.02);
    flex-wrap: wrap;
}

.quick-insert-btn {
    padding: 6px 10px;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 4px;
    color: rgba(255, 255, 255, 0.7);
    font-size: 14px;
    cursor: pointer;
    transition: all 0.1s ease;
}

.quick-insert-btn:hover {
    background: rgba(102, 126, 234, 0.15);
    border-color: rgba(102, 126, 234, 0.3);
    color: #667eea;
}

/* Mobile */
@media (max-width: 768px) {
    .formula-textarea {
        font-size: 16px; /* Prevent iOS zoom */
        min-height: 100px;
    }

    .formula-preview .katex-display {
        font-size: 1.5em;
    }
}
"#;

/// Common LaTeX snippets for quick insertion.
const QUICK_INSERTS: &[(&str, &str)] = &[
    ("\u{2200}", "\\forall "),
    ("\u{2203}", "\\exists "),
    ("\u{2192}", "\\to "),
    ("\u{2227}", "\\land "),
    ("\u{2228}", "\\lor "),
    ("\u{00AC}", "\\neg "),
    ("frac", "\\frac{}{}"),
    ("sum", "\\sum_{i=0}^{n}"),
    ("sqrt", "\\sqrt{}"),
];

#[component]
pub fn FormulaEditor(
    latex: String,
    on_change: EventHandler<String>,
    on_insert: Option<EventHandler<String>>,
) -> Element {
    // Track if we have a render error
    let has_content = !latex.trim().is_empty();

    rsx! {
        style { "{FORMULA_EDITOR_STYLE}" }

        div { class: "formula-editor",
            // Input area
            div { class: "formula-editor-input",
                label { class: "formula-editor-label", "LaTeX Input" }
                textarea {
                    class: "formula-textarea",
                    placeholder: "Enter LaTeX formula... (e.g., \\forall x \\in A: P(x))",
                    value: "{latex}",
                    oninput: move |e| on_change.call(e.value()),
                    spellcheck: "false",
                }
            }

            // Quick insert bar
            div { class: "formula-quick-insert",
                for (label, insert) in QUICK_INSERTS.iter() {
                    button {
                        class: "quick-insert-btn",
                        key: "{insert}",
                        onclick: {
                            let insert = insert.to_string();
                            let handler = on_insert.clone();
                            move |_| {
                                if let Some(ref h) = handler {
                                    h.call(insert.clone());
                                }
                            }
                        },
                        "{label}"
                    }
                }
            }

            // Live preview using existing KatexSpan
            div { class: "formula-preview",
                if has_content {
                    KatexSpan { latex: latex.clone(), display: true }
                } else {
                    div { class: "formula-preview-empty",
                        "Preview will appear here"
                    }
                }
            }
        }
    }
}

/// Standalone preview component that just renders LaTeX.
#[component]
pub fn LatexPreview(latex: String) -> Element {
    let has_content = !latex.trim().is_empty();

    rsx! {
        div { class: "formula-preview",
            style: "height: 100%; background: #12161c;",
            if has_content {
                KatexSpan { latex: latex, display: true }
            } else {
                div { class: "formula-preview-empty",
                    "Enter a formula to see the preview"
                }
            }
        }
    }
}
