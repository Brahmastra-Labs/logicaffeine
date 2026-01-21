use dioxus::prelude::*;
use crate::ui::components::icon::{Icon, IconVariant, IconSize};

const MODE_SELECTOR_STYLE: &str = r#"
.mode-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: rgba(0, 0, 0, 0.8);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    animation: fade-in 0.2s ease-out;
}

@keyframes fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
}

.mode-dialog {
    background: linear-gradient(135deg, #1a1a2e 0%, #16213e 100%);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 20px;
    padding: 32px;
    max-width: 600px;
    width: 90%;
    animation: slide-up 0.3s ease-out;
}

@keyframes slide-up {
    from { transform: translateY(20px); opacity: 0; }
    to { transform: translateY(0); opacity: 1; }
}

.mode-dialog h2 {
    color: #e8e8e8;
    font-size: 24px;
    margin-bottom: 8px;
    text-align: center;
}

.mode-dialog p {
    color: #888;
    font-size: 14px;
    text-align: center;
    margin-bottom: 24px;
}

.mode-options {
    display: flex;
    flex-direction: column;
    gap: 12px;
}

.mode-option {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 20px;
    background: rgba(255, 255, 255, 0.03);
    border: 2px solid rgba(255, 255, 255, 0.08);
    border-radius: 12px;
    cursor: pointer;
    transition: all 0.2s ease;
}

.mode-option:hover {
    background: rgba(255, 255, 255, 0.06);
    border-color: rgba(255, 255, 255, 0.15);
    transform: translateX(4px);
}

.mode-option.textbook:hover {
    border-color: rgba(96, 165, 250, 0.5);
}

.mode-option.learning:hover {
    border-color: rgba(74, 222, 128, 0.5);
}

.mode-option.testing:hover {
    border-color: rgba(251, 146, 60, 0.5);
}

.mode-icon {
    width: 48px;
    height: 48px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 12px;
    font-size: 24px;
}

.mode-icon.textbook {
    background: rgba(96, 165, 250, 0.2);
}

.mode-icon.learning {
    background: rgba(74, 222, 128, 0.2);
}

.mode-icon.testing {
    background: rgba(251, 146, 60, 0.2);
}

.mode-info {
    flex: 1;
}

.mode-info h3 {
    color: #e8e8e8;
    font-size: 18px;
    margin-bottom: 4px;
}

.mode-info p {
    color: #888;
    font-size: 13px;
    text-align: left;
    margin: 0;
}

.mode-arrow {
    color: #666;
    font-size: 20px;
}

.mode-cancel {
    display: block;
    width: 100%;
    margin-top: 16px;
    padding: 12px;
    background: transparent;
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 8px;
    color: #888;
    font-size: 14px;
    cursor: pointer;
    transition: all 0.2s ease;
}

.mode-cancel:hover {
    border-color: rgba(255, 255, 255, 0.2);
    color: #aaa;
}

.recommended-badge {
    background: rgba(74, 222, 128, 0.2);
    color: #4ade80;
    padding: 2px 8px;
    border-radius: 4px;
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    margin-left: 8px;
}
"#;

#[derive(Clone, PartialEq)]
pub struct ModeInfo {
    pub era: String,
    pub module: String,
    pub title: String,
}

#[component]
pub fn ModeSelector(
    info: ModeInfo,
    on_select: EventHandler<String>,
    on_cancel: EventHandler<()>,
) -> Element {
    rsx! {
        style { "{MODE_SELECTOR_STYLE}" }
        div {
            class: "mode-overlay",
            onclick: move |_| on_cancel.call(()),
            div {
                class: "mode-dialog",
                onclick: move |e| e.stop_propagation(),
                h2 { "{info.title}" }
                p { "Choose how you want to approach this module" }

                div { class: "mode-options",
                    button {
                        class: "mode-option textbook",
                        onclick: move |_| on_select.call("textbook".to_string()),
                        div { class: "mode-icon textbook",
                            Icon { variant: IconVariant::Book, size: IconSize::Large }
                        }
                        div { class: "mode-info",
                            h3 { "Read" }
                            p { "Study the concepts and examples before practicing" }
                        }
                        span { class: "mode-arrow",
                            Icon { variant: IconVariant::ChevronRight, size: IconSize::Medium }
                        }
                    }

                    button {
                        class: "mode-option learning",
                        onclick: move |_| on_select.call("learning".to_string()),
                        div { class: "mode-icon learning",
                            Icon { variant: IconVariant::GraduationCap, size: IconSize::Large }
                        }
                        div { class: "mode-info",
                            h3 {
                                "Practice"
                                span { class: "recommended-badge", "Recommended" }
                            }
                            p { "Learn with hints and immediate feedback on your answers" }
                        }
                        span { class: "mode-arrow",
                            Icon { variant: IconVariant::ChevronRight, size: IconSize::Medium }
                        }
                    }

                    button {
                        class: "mode-option testing",
                        onclick: move |_| on_select.call("testing".to_string()),
                        div { class: "mode-icon testing",
                            Icon { variant: IconVariant::Document, size: IconSize::Large }
                        }
                        div { class: "mode-info",
                            h3 { "Test" }
                            p { "Prove your knowledge with no hints - see results at the end" }
                        }
                        span { class: "mode-arrow",
                            Icon { variant: IconVariant::ChevronRight, size: IconSize::Medium }
                        }
                    }
                }

                button {
                    class: "mode-cancel",
                    onclick: move |_| on_cancel.call(()),
                    "Cancel"
                }
            }
        }
    }
}
