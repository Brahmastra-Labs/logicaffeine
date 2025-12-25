use dioxus::prelude::*;
use crate::ui::state::{ChatMessage, Role};

#[component]
pub fn ChatDisplay(messages: Vec<ChatMessage>) -> Element {
    rsx! {
        div { class: "chat-area",
            for (i, msg) in messages.iter().enumerate() {
                div {
                    key: "{i}",
                    class: match msg.role {
                        Role::User => "message user",
                        Role::System => "message system",
                        Role::Error => "message error",
                    },
                    "{msg.content}"
                }
            }
        }
    }
}
