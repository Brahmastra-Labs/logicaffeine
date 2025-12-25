use dioxus::prelude::*;

const OUTPUT_STYLE: &str = r#"
.logic-output-container {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: 16px;
}

.reading-selector {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 12px;
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
    background: linear-gradient(135deg, #667eea, #764ba2);
    border-color: transparent;
    color: white;
}

.logic-display {
    flex: 1;
    background: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.2);
    border-radius: 12px;
    padding: 20px;
    font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
    font-size: 18px;
    line-height: 1.6;
    color: #e8e8e8;
    overflow: auto;
}

.logic-display.empty {
    color: #666;
    font-style: italic;
    display: flex;
    align-items: center;
    justify-content: center;
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
"#;

#[derive(Clone, Copy, PartialEq, Default)]
pub enum OutputFormat {
    #[default]
    Unicode,
    LaTeX,
}

#[component]
pub fn LogicOutput(
    logic: Option<String>,
    readings: Vec<String>,
    error: Option<String>,
    format: OutputFormat,
) -> Element {
    let mut current_reading = use_signal(|| 0usize);

    let total_readings = readings.len().max(1);
    let display_logic = if !readings.is_empty() {
        let idx = (*current_reading.read()).min(readings.len().saturating_sub(1));
        Some(readings.get(idx).cloned().unwrap_or_default())
    } else {
        logic.clone()
    };

    let formatted_output = match (&display_logic, format) {
        (Some(logic), OutputFormat::LaTeX) => convert_to_latex(logic),
        (Some(logic), OutputFormat::Unicode) => logic.clone(),
        (None, _) => String::new(),
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
            _ => result.push(c),
        }
    }

    result
}
