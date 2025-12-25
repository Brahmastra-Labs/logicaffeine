use dioxus::prelude::*;

const GUIDE_STYLE: &str = r#"
.socratic-guide {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 16px 20px;
    background: rgba(255, 255, 255, 0.03);
    border-top: 1px solid rgba(255, 255, 255, 0.08);
    min-height: 60px;
}

.guide-avatar {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: linear-gradient(135deg, #667eea, #764ba2);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 18px;
    flex-shrink: 0;
}

.guide-content {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 4px;
}

.guide-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: #667eea;
}

.guide-message {
    font-size: 14px;
    line-height: 1.5;
    color: #c8c8c8;
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
    font-size: 13px;
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
pub fn SocraticGuide(
    mode: GuideMode,
    on_hint_request: Option<EventHandler<()>>,
) -> Element {
    let (message_class, label, message) = match &mode {
        GuideMode::Idle => {
            return rsx! {
                style { "{GUIDE_STYLE}" }
                div { class: "socratic-guide",
                    div { class: "guide-avatar", "\u{1F989}" }
                    div { class: "guide-content",
                        div { class: "guide-label", "Socrates" }
                        div { class: "guide-message guide-empty",
                            "Type a sentence to begin your journey into logic..."
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
            div { class: "guide-avatar", "\u{1F989}" }
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
        0 => "Hmm, I couldn't parse that sentence. Let me think about why...".to_string(),
        1 => "This sentence has a single, unambiguous logical form.".to_string(),
        2 => format!(
            "Interesting! This sentence is ambiguous. I found {} different readings. \
            Click the reading buttons above to explore each interpretation.",
            readings_count
        ),
        n => format!(
            "Fascinating complexity! I discovered {} different ways to interpret this sentence. \
            Each represents a valid logical reading of your input.",
            n
        ),
    }
}

pub fn get_context_hint(input: &str) -> Option<String> {
    let lower = input.to_lowercase();

    if lower.starts_with("every") || lower.starts_with("all") {
        Some("Universal quantification (\u{2200}) asserts something about ALL members of a set.".to_string())
    } else if lower.starts_with("some") || lower.starts_with("a ") {
        Some("Existential quantification (\u{2203}) asserts the EXISTENCE of at least one entity.".to_string())
    } else if lower.contains(" loves ") || lower.contains(" sees ") {
        Some("Transitive verbs create two-place predicates relating a subject to an object.".to_string())
    } else if lower.contains(" is ") && lower.contains(" not ") {
        Some("Negation (\u{00AC}) inverts the truth value of the proposition it scopes over.".to_string())
    } else {
        None
    }
}
