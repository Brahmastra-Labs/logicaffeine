use dioxus::prelude::*;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = initCodeMirror)]
    fn init_codemirror(element_id: &str, on_change: &Closure<dyn FnMut(String)>) -> JsValue;

    #[wasm_bindgen(js_namespace = window, js_name = setCodeMirrorValue)]
    fn set_codemirror_value(editor: &JsValue, value: &str);

    #[wasm_bindgen(js_namespace = window, js_name = getCodeMirrorValue)]
    fn get_codemirror_value(editor: &JsValue) -> String;
}

const EDITOR_STYLE: &str = r#"
.editor-container {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 200px;
    padding: 16px;
}

.editor-wrapper {
    flex: 1;
    background: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.2);
    border-radius: 12px;
    overflow: hidden;
}

.editor-fallback {
    width: 100%;
    height: 100%;
    min-height: 150px;
    background: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.2);
    border-radius: 12px;
    padding: 16px;
    font-size: 16px;
    font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
    color: #e8e8e8;
    resize: none;
    outline: none;
}

.editor-fallback:focus {
    border-color: #667eea;
    box-shadow: 0 0 0 3px rgba(102, 126, 234, 0.2);
}

.editor-fallback::placeholder {
    color: #666;
}
"#;

#[component]
pub fn Editor(
    value: String,
    on_change: EventHandler<String>,
    placeholder: Option<String>,
) -> Element {
    let placeholder_text = placeholder.unwrap_or_else(|| "Type an English sentence...".to_string());

    rsx! {
        style { "{EDITOR_STYLE}" }

        div { class: "editor-container",
            textarea {
                class: "editor-fallback",
                placeholder: "{placeholder_text}",
                value: "{value}",
                oninput: move |evt| on_change.call(evt.value()),
            }
        }
    }
}

#[component]
pub fn LiveEditor(
    on_change: EventHandler<String>,
    placeholder: Option<String>,
) -> Element {
    let mut text = use_signal(String::new);
    let placeholder_text = placeholder.unwrap_or_else(|| "Type an English sentence...".to_string());

    let handle_input = move |evt: Event<FormData>| {
        let new_value = evt.value();
        text.set(new_value.clone());
        on_change.call(new_value);
    };

    rsx! {
        style { "{EDITOR_STYLE}" }

        div { class: "editor-container",
            textarea {
                class: "editor-fallback",
                placeholder: "{placeholder_text}",
                value: "{text}",
                oninput: handle_input,
                spellcheck: "false",
                autocomplete: "off",
                autocapitalize: "off",
            }
        }
    }
}
