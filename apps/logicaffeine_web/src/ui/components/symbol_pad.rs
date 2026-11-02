//! The symbol pad: a button for every symbol a formula needs, beside a typed answer.
//!
//! A click types its symbol at the caret of the field it names (`target`), replacing a selection,
//! and leaves the caret after it with the field still focused, so the learner goes on typing. A
//! pointer press never takes the focus from the field; a keyboard press (Enter or Space on a focused
//! button) types the symbol and hands the focus back. The buttons and their meanings come from
//! [`crate::composer`].

use dioxus::prelude::*;
use crate::composer::symbol_buttons;

const SYMBOL_PAD_STYLE: &str = r#"
.symbol-pad {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(76px, 1fr));
    gap: var(--spacing-sm);
    margin-top: var(--spacing-md);
}

.symbol-key {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
    min-height: 48px;
    padding: 6px 5px 5px;
    border-radius: var(--radius-md);
    border: 1px solid rgba(var(--accent-primary-rgb), 0.22);
    background: rgba(var(--accent-primary-rgb), 0.05);
    color: var(--text-primary);
    font: inherit;
    cursor: pointer;
    touch-action: manipulation;
    transition: background 0.15s ease, border-color 0.15s ease;
}

.symbol-key:hover {
    background: rgba(var(--accent-primary-rgb), 0.12);
    border-color: rgba(var(--accent-primary-rgb), 0.5);
}

.symbol-key:active {
    background: rgba(var(--accent-primary-rgb), 0.2);
}

.symbol-key:focus-visible {
    outline: 2px solid var(--accent-primary);
    outline-offset: 2px;
}

.symbol-key-glyph {
    font-family: var(--font-mono);
    font-size: 20px;
    line-height: 1;
    color: var(--accent-primary);
}

.symbol-key-name {
    font-size: 11px;
    line-height: 1.2;
    color: var(--text-tertiary);
    text-align: center;
}

@media (max-width: 640px) {
    .symbol-pad {
        grid-template-columns: repeat(auto-fill, minmax(64px, 1fr));
    }
    .symbol-key {
        min-height: 52px;
    }
}
"#;

/// Type `symbol` into the field `target` at its caret, and return the field's new text.
#[cfg(target_arch = "wasm32")]
fn type_into(target: &str, symbol: &str) -> Option<String> {
    use wasm_bindgen::JsCast;
    let field = web_sys::window()?.document()?.get_element_by_id(target)?.dyn_into::<web_sys::HtmlInputElement>().ok()?;
    let text = field.value();
    let end_of_text = text.encode_utf16().count() as u32;
    let start = field.selection_start().ok().flatten().unwrap_or(end_of_text);
    let end = field.selection_end().ok().flatten().unwrap_or(start);
    let (typed, caret) = crate::composer::insert_at(&text, start as usize, end as usize, symbol);
    field.set_value(&typed);
    let _ = field.focus();
    // The page then re-renders the field with this same text, which leaves the caret where it is
    // set here. Setting it again later would undo any keys pressed in between.
    let _ = field.set_selection_range(caret as u32, caret as u32);
    Some(typed)
}

#[cfg(not(target_arch = "wasm32"))]
fn type_into(target: &str, symbol: &str) -> Option<String> {
    let _ = (target, symbol);
    None
}

/// A button for every symbol, typing into the field whose id is `target`. `on_input` hears the
/// field's new text, the same way the field's own `oninput` does.
#[component]
pub fn SymbolPad(target: String, on_input: EventHandler<String>) -> Element {
    rsx! {
        style { "{SYMBOL_PAD_STYLE}" }
        div { class: "symbol-pad", role: "group", aria_label: "Symbols",
            for button in symbol_buttons() {
                {
                    let field = target.clone();
                    rsx! {
                        button {
                            key: "{button.symbol}",
                            class: "symbol-key",
                            r#type: "button",
                            "data-symbol": "{button.symbol}",
                            aria_label: "{button.label()}",
                            aria_controls: "{target}",
                            title: "{button.meaning}",
                            onmousedown: move |e: Event<MouseData>| e.prevent_default(),
                            onclick: move |_| {
                                if let Some(text) = type_into(&field, button.symbol) {
                                    on_input.call(text);
                                }
                            },
                            span { class: "symbol-key-glyph", aria_hidden: "true", "{button.symbol}" }
                            span { class: "symbol-key-name", aria_hidden: "true", "{button.name}" }
                        }
                    }
                }
            }
        }
    }
}
