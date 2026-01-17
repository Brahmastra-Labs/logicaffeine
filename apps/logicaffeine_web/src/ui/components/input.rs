//! Text input component for the REPL interface.
//!
//! Provides a styled text input with submit button for entering English sentences.
//! Handles Enter key submission.
//!
//! # Props
//!
//! - `on_send` - Callback invoked with the input text when submitted

use dioxus::prelude::*;

/// Text input area with submit button.
#[component]
pub fn InputArea(on_send: EventHandler<String>) -> Element {
    let mut text = use_signal(String::new);

    let mut submit = move || {
        let current_text = text.read().clone();
        if !current_text.trim().is_empty() {
            on_send.call(current_text);
            text.set(String::new());
        }
    };

    rsx! {
        div { class: "input-area",
            div { class: "input-row",
                input {
                    placeholder: "Type an English sentence...",
                    value: "{text}",
                    oninput: move |evt| text.set(evt.value()),
                    onkeydown: move |evt| {
                        if evt.key() == Key::Enter {
                            submit();
                        }
                    }
                }
                button {
                    onclick: move |_| submit(),
                    "Transpile →"
                }
            }
        }
    }
}
