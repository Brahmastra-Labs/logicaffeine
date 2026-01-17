//! First-Order Logic output display component.
//!
//! Renders compiled logic expressions with syntax highlighting. Supports multiple
//! output formats (Unicode, SimpleFOL, LaTeX, Kripke) and multiple readings for
//! ambiguous sentences.
//!
//! # Props
//!
//! - `logic` - Primary Unicode output
//! - `simple_logic` - SimpleFOL format output
//! - `kripke_logic` - Kripke semantics output
//! - `readings` - Multiple readings for ambiguous sentences
//! - `error` - Optional error message
//! - `format` - Output format selection
//!
//! # Syntax Highlighting
//!
//! Different logical elements are color-coded:
//! - Quantifiers (∀, ∃): purple
//! - Connectives (∧, ∨, →, ↔, ¬): purple
//! - Variables (lowercase): blue
//! - Predicates (uppercase words): green
//! - Constants (capitalized names): yellow

use dioxus::prelude::*;

const OUTPUT_STYLE: &str = r#"
.logic-output-container {
    display: flex;
    flex-direction: column;
    padding: 16px;
}

.reading-selector {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 12px;
    flex-wrap: wrap;
}

.reading-selector span {
    color: #888;
    font-size: 14px;
}

.reading-btn {
    width: 28px;
    height: 28px;
    border-radius: 6px;
    border: 1px solid rgba(255, 255, 255, 0.2);
    background: rgba(255, 255, 255, 0.08);
    color: #888;
    font-size: 12px;
    cursor: pointer;
    transition: all 0.2s ease;
}

.reading-btn:hover {
    background: rgba(255, 255, 255, 0.15);
    color: #e8e8e8;
}

.reading-btn.active {
    background: #667eea;
    border-color: transparent;
    color: white;
}

.logic-display {
    background: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.2);
    border-radius: 12px;
    padding: 16px;
    font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
    font-size: 16px;
    line-height: 1.6;
    color: #e8e8e8;
    overflow: auto;
    -webkit-overflow-scrolling: touch;
    word-break: break-word;
}

.logic-display.empty {
    color: #666;
    font-style: italic;
    padding: 16px;
}

.logic-display.error {
    border-color: rgba(224, 108, 117, 0.3);
    color: #e06c75;
}

.logic-quantifier { color: #c678dd; font-weight: 500; }
.logic-variable { color: #61afef; }
.logic-predicate { color: #98c379; }
.logic-connective { color: #c678dd; }
.logic-constant { color: #e5c07b; }
.logic-paren { color: #abb2bf; }

/* Mobile optimizations */
@media (max-width: 768px) {
    .logic-output-container {
        padding: 12px;
    }

    .reading-selector {
        gap: 6px;
        margin-bottom: 10px;
    }

    .reading-selector span {
        font-size: 13px;
    }

    /* Touch-friendly reading buttons */
    .reading-btn {
        width: 44px;
        height: 44px;
        font-size: 14px;
        border-radius: 8px;
        -webkit-tap-highlight-color: transparent;
    }

    .reading-btn:active {
        transform: scale(0.95);
    }

    .logic-display {
        padding: 16px;
        font-size: 16px;
        border-radius: 10px;
    }

    .logic-display.empty {
        font-size: 14px;
        padding: 16px;
    }
}

/* Extra small screens */
@media (max-width: 480px) {
    .reading-btn {
        width: 40px;
        height: 40px;
        font-size: 13px;
    }

    .logic-display {
        font-size: 15px;
        padding: 14px;
    }
}
"#;

#[derive(Clone, Copy, PartialEq, Default)]
pub enum OutputFormat {
    #[default]
    Unicode,
    SimpleFOL,
    LaTeX,
    Kripke,  // Deep/Kripke semantics
}

#[component]
pub fn LogicOutput(
    logic: Option<String>,
    simple_logic: Option<String>,
    kripke_logic: Option<String>,  // Deep/Kripke semantics output (single)
    readings: Vec<String>,
    simple_readings: Vec<String>,   // SimpleFOL readings (deduplicated)
    kripke_readings: Vec<String>,   // Deep/Kripke readings for ambiguous sentences
    error: Option<String>,
    format: OutputFormat,
) -> Element {
    let mut current_reading = use_signal(|| 0usize);

    // Use format-appropriate readings
    let active_readings = match format {
        OutputFormat::Kripke => &kripke_readings,
        OutputFormat::SimpleFOL => &simple_readings,
        _ => &readings,
    };

    let total_readings = active_readings.len().max(1);
    let display_logic = if !active_readings.is_empty() {
        let idx = (*current_reading.read()).min(active_readings.len().saturating_sub(1));
        Some(active_readings.get(idx).cloned().unwrap_or_default())
    } else if format == OutputFormat::Kripke {
        kripke_logic.clone()
    } else {
        logic.clone()
    };

    let formatted_output = match format {
        OutputFormat::LaTeX => display_logic.as_ref().map(|l| convert_to_latex(l)).unwrap_or_default(),
        _ => display_logic.clone().unwrap_or_default(),
    };

    rsx! {
        style { "{OUTPUT_STYLE}" }

        div { class: "logic-output-container",
            if total_readings > 1 {
                div { class: "reading-selector",
                    span { "Reading" }
                    for i in 0..total_readings {
                        button {
                            class: if *current_reading.read() == i { "reading-btn active" } else { "reading-btn" },
                            onclick: move |_| current_reading.set(i),
                            "{i + 1}"
                        }
                    }
                    span { "of {total_readings}" }
                }
            }

            if let Some(err) = &error {
                div { class: "logic-display error",
                    "{err}"
                }
            } else if formatted_output.is_empty() {
                div { class: "logic-display empty",
                    "Type a sentence to see its logical form..."
                }
            } else {
                div { class: "logic-display",
                    dangerous_inner_html: highlight_logic(&formatted_output)
                }
            }
        }
    }
}

fn convert_to_latex(unicode: &str) -> String {
    unicode
        .replace('\u{2200}', "\\forall ")
        .replace('\u{2203}', "\\exists ")
        .replace('\u{00AC}', "\\neg ")
        .replace('\u{2227}', "\\land ")
        .replace('\u{2228}', "\\lor ")
        .replace('\u{2192}', "\\rightarrow ")
        .replace('\u{2194}', "\\leftrightarrow ")
        .replace('\u{22A5}', "\\bot ")
        .replace('\u{22A4}', "\\top ")
}

pub fn highlight_logic(logic: &str) -> String {
    let mut result = String::new();
    let mut chars = logic.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\u{2200}' | '\u{2203}' => {
                result.push_str(&format!(r#"<span class="logic-quantifier">{}</span>"#, c));
            }
            '\u{00AC}' | '\u{2227}' | '\u{2228}' | '\u{2192}' | '\u{2194}' => {
                result.push_str(&format!(r#"<span class="logic-connective">{}</span>"#, c));
            }
            '(' | ')' | '[' | ']' => {
                result.push_str(&format!(r#"<span class="logic-paren">{}</span>"#, c));
            }
            'a'..='z' if chars.peek().map(|n| !n.is_alphabetic()).unwrap_or(true) => {
                result.push_str(&format!(r#"<span class="logic-variable">{}</span>"#, c));
            }
            'A'..='Z' => {
                let mut word = String::from(c);
                while let Some(&next) = chars.peek() {
                    if next.is_alphanumeric() {
                        word.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
                if word.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
                    && word.len() > 1
                    && word.chars().skip(1).all(|c| c.is_lowercase() || c.is_numeric())
                {
                    result.push_str(&format!(r#"<span class="logic-constant">{}</span>"#, word));
                } else {
                    result.push_str(&format!(r#"<span class="logic-predicate">{}</span>"#, word));
                }
            }
            '\n' => result.push_str("<br>"),
            _ => result.push(c),
        }
    }

    result
}
