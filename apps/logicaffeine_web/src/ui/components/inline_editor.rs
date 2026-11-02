//! The Studio's editor set into a page of prose: the Syntax Guide's examples, exercises and
//! quoted snippets.
//!
//! It is [`CodeEditor`] with the language server on — the same highlighting, hovers and
//! squiggles — sized to the code it holds instead of filling a panel: the highlight layer sits in
//! the page's flow and gives the editor its height, and the text area lies exactly over it and
//! never scrolls, so the editor grows line by line as the reader types. A page holds hundreds of
//! them, so each is analysed once it first comes on screen, as VS Code analyses the files it has
//! open rather than every file in the folder.
//!
//! The editor carries no stylesheet of its own: its host places [`InlineEditorStyle`] once, ahead
//! of the host's markup, so a block's CSS always comes before what it styles.

use dioxus::prelude::*;

use super::code_editor::{CodeEditor, Language, CODE_EDITOR_STYLE};

const INLINE_EDITOR_STYLE: &str = r#"
.inline-editor .code-editor {
    height: auto;
    background: transparent;
}
.inline-editor .code-editor-input {
    flex: none;
    min-height: 0;
}
.inline-editor .code-editor-highlight {
    position: relative;
    height: auto;
    overflow: hidden;
    padding-bottom: 16px;
}
/* A new line at the very end is a line the reader sees at once: the space holds it open. */
.inline-editor .code-editor-highlight::after {
    content: " ";
}
.inline-editor .code-editor-textarea {
    overflow: hidden;
    padding-bottom: 16px;
}
@media (max-width: 768px) {
    .inline-editor .code-editor-highlight,
    .inline-editor .code-editor-textarea {
        padding-bottom: 12px;
    }
}
"#;

/// The stylesheets an inline editor is drawn with: the editor's own, and its fit to its code.
#[component]
pub fn InlineEditorStyle() -> Element {
    rsx! {
        style { "{CODE_EDITOR_STYLE}" }
        style { "{INLINE_EDITOR_STYLE}" }
    }
}

#[component]
pub fn InlineEditor(
    value: String,
    on_change: EventHandler<String>,
    language: Language,
    #[props(default = false)]
    readonly: bool,
) -> Element {
    let mut seen = use_signal(|| false);

    rsx! {
        div {
            class: "inline-editor",
            onvisible: move |evt| {
                if !*seen.peek() && evt.data().is_intersecting().unwrap_or(false) {
                    seen.set(true);
                }
            },
            CodeEditor {
                value,
                on_change,
                language,
                readonly,
                language_server: seen(),
                placeholder: String::new(),
                stylesheet: false,
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/components/inline_editor/tests.rs"]
mod tests;
