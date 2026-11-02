//! LOGOS quoted in the Syntax Guide's prose.
//!
//! A snippet is read-only — the prose quotes it as written — but it is still LOGOS, so it is read
//! the way the guide's examples are: the shared editor with the language server on, its words
//! coloured and explained by the same hover. A squiggle there would mean the guide quotes broken
//! code, which `tests/guide_language_server.rs` forbids.

use dioxus::prelude::*;

use super::code_editor::Language;
use super::inline_editor::{InlineEditor, InlineEditorStyle};

#[component]
pub fn GuideSnippet(code: String, language: Language) -> Element {
    let mut copied = use_signal(|| false);
    let copy = {
        let code = code.clone();
        move |_| {
            #[cfg(target_arch = "wasm32")]
            if let Some(window) = web_sys::window() {
                let _ = window.navigator().clipboard().write_text(&code);
                copied.set(true);
                spawn(async move {
                    gloo_timers::future::TimeoutFuture::new(1200).await;
                    copied.set(false);
                });
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = &code;
                copied.set(true);
            }
        }
    };

    rsx! {
        InlineEditorStyle {}
        div { class: "guide-snippet",
            button { class: "guide-code-copy", onclick: copy, if copied() { "Copied!" } else { "Copy" } }
            InlineEditor { value: code, on_change: move |_| {}, language, readonly: true }
        }
    }
}
