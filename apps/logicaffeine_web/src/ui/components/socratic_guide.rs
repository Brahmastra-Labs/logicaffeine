//! Socratic guide component for hints and feedback.
//!
//! Displays contextual hints and error messages using a Socratic teaching approach.
//! Guides users toward understanding rather than providing direct answers.
//!
//! # Props
//!
//! - `mode` - Current guide state (Idle, Success, Error, Hint, Info)
//! - `on_hint_request` - Optional callback for "Show me a hint" button
//!
//! # Modes
//!
//! - `Idle` - Placeholder when no input
//! - `Success` - Green message for successful parsing
//! - `Error` - Red message with Socratic guidance
//! - `Hint` - Green hint text
//! - `Info` - Blue informational message

use dioxus::prelude::*;

const GUIDE_STYLE: &str = r#"
.socratic-guide {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 20px;
    background: transparent;
    min-height: 44px;
}

.guide-content {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 10px;
}

.guide-label {
    font-size: 13px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: #667eea;
    white-space: nowrap;
}

.guide-message {
    font-size: 16px;
    line-height: 1.5;
    color: #e0e0e0;
}

.guide-message.error {
    color: #e06c75;
}

.guide-message.hint {
    color: #98c379;
}

.guide-message.info {
    color: #61afef;
}

.guide-message code {
    font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
    background: rgba(255, 255, 255, 0.08);
    padding: 2px 6px;
    border-radius: 4px;
    font-size: 15px;
}

.guide-actions {
    display: flex;
    gap: 8px;
    margin-top: 8px;
}

.guide-btn {
    padding: 6px 12px;
    border-radius: 6px;
    border: 1px solid rgba(255, 255, 255, 0.15);
    background: rgba(255, 255, 255, 0.05);
    color: #888;
    font-size: 12px;
    cursor: pointer;
    transition: all 0.2s ease;
}

.guide-btn:hover {
    background: rgba(255, 255, 255, 0.1);
    color: #e8e8e8;
}

.guide-btn.primary {
    background: linear-gradient(135deg, #667eea, #764ba2);
    border-color: transparent;
    color: white;
}

.guide-btn.primary:hover {
    opacity: 0.9;
}

.guide-empty {
    color: #666;
    font-style: italic;
}
"#;

#[derive(Clone, PartialEq)]
pub enum GuideMode {
    Idle,
    Success(String),
    Error(String),
    Hint(String),
    Info(String),
}

impl Default for GuideMode {
    fn default() -> Self {
        GuideMode::Idle
    }
}

#[component]
pub fn SocraticGuide(mode: GuideMode, on_hint_request: Option<EventHandler<()>>) -> Element {
    let (message_class, label, message) = match &mode {
        GuideMode::Idle => {
            return rsx! {
                style { "{GUIDE_STYLE}" }
                div { class: "socratic-guide",
                    div { class: "guide-content",
                        div { class: "guide-message guide-empty",
                            "Type an English sentence to translate it to First-Order Logic"
                        }
                    }
                }
            }
        }
        GuideMode::Success(msg) => ("guide-message", "Analysis", msg.clone()),
        GuideMode::Error(msg) => ("guide-message error", "Something to consider", msg.clone()),
        GuideMode::Hint(msg) => ("guide-message hint", "Hint", msg.clone()),
        GuideMode::Info(msg) => ("guide-message info", "Observation", msg.clone()),
    };

    rsx! {
        style { "{GUIDE_STYLE}" }

        div { class: "socratic-guide",
            div { class: "guide-content",
                div { class: "guide-label", "{label}" }
                div { class: "{message_class}",
                    dangerous_inner_html: format_guide_message(&message)
                }
                if on_hint_request.is_some() && matches!(mode, GuideMode::Error(_)) {
                    div { class: "guide-actions",
                        button {
                            class: "guide-btn",
                            onclick: move |_| {
                                if let Some(handler) = &on_hint_request {
                                    handler.call(());
                                }
                            },
                            "Show me a hint"
                        }
                    }
                }
            }
        }
    }
}

fn format_guide_message(message: &str) -> String {
    let mut result = message.to_string();

    let code_patterns = [
        ("\u{2200}", "<code>\u{2200}</code>"),
        ("\u{2203}", "<code>\u{2203}</code>"),
        ("\u{2227}", "<code>\u{2227}</code>"),
        ("\u{2228}", "<code>\u{2228}</code>"),
        ("\u{2192}", "<code>\u{2192}</code>"),
        ("\u{00AC}", "<code>\u{00AC}</code>"),
    ];

    for (pattern, replacement) in &code_patterns {
        result = result.replace(pattern, replacement);
    }

    result
}

pub fn get_success_message(readings_count: usize) -> String {
    match readings_count {
        0 => "This sentence could not be resolved into a logical form.".to_string(),
        1 => "This sentence has a single, unambiguous logical form.".to_string(),
        2 => format!(
            "This sentence is ambiguous: {} different readings found. \
            Click the reading buttons above to explore each interpretation.",
            readings_count
        ),
        n => format!(
            "{} distinct logical readings found. \
            Each represents a valid interpretation of your input.",
            n
        ),
    }
}

pub fn get_context_hint(input: &str) -> Option<String> {
    let lower = input.to_lowercase();

    if lower.starts_with("every") || lower.starts_with("all") {
        Some(
            "Universal quantification (\u{2200}) asserts something about ALL members of a set."
                .to_string(),
        )
    } else if lower.starts_with("some") || lower.starts_with("a ") {
        Some(
            "Existential quantification (\u{2203}) asserts the EXISTENCE of at least one entity."
                .to_string(),
        )
    } else if lower.contains(" loves ") || lower.contains(" sees ") {
        Some(
            "Transitive verbs create two-place predicates relating a subject to an object."
                .to_string(),
        )
    } else if lower.contains(" is ") && lower.contains(" not ") {
        Some(
            "Negation (\u{00AC}) inverts the truth value of the proposition it scopes over."
                .to_string(),
        )
    } else {
        None
    }
}

/// What the Analysis row says about Logic input: for sentences, how many readings they have;
/// for a theorem, whether its goal was proved. Never "could not be resolved" beside a
/// formula the Logic Output is showing.
pub fn logic_guide(
    error: Option<&str>,
    logic: Option<&str>,
    readings: usize,
    input: &str,
    proof: &crate::ui::components::proof_panel::ProofStatus,
) -> GuideMode {
    use crate::ui::components::proof_panel::ProofStatus;
    if let Some(err) = error {
        return GuideMode::Error(err.to_string());
    }
    if logic.is_none() {
        return GuideMode::Idle;
    }
    match proof {
        ProofStatus::Success => {
            return GuideMode::Success("Proved: the goal follows from the premises.".to_string())
        }
        ProofStatus::Failed(_) => {
            return GuideMode::Info("The goal does not follow from the premises as they stand.".to_string())
        }
        ProofStatus::Uncertified(_) => {
            return GuideMode::Hint(
                "A derivation was found but is not certified. The kernel did not accept it as a proof.".to_string(),
            )
        }
        ProofStatus::Proving => return GuideMode::Info("Searching for a proof\u{2026}".to_string()),
        ProofStatus::Idle => {}
    }
    // A formula on screen is at least one reading.
    let message = get_success_message(readings.max(1));
    match get_context_hint(input) {
        Some(hint) => GuideMode::Info(format!("{message} {hint}")),
        None => GuideMode::Success(message),
    }
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/components/socratic_guide/logic_guide_tests.rs"]
mod logic_guide_tests;
