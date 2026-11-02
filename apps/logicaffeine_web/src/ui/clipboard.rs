//! Handing text to the reader's clipboard.
//!
//! The Clipboard API exists only on pages served over HTTPS (or from localhost), and a browser may
//! refuse a write even there. Safari grants the clipboard only inside the click that asked, so the
//! write starts at once, in the handler, never in a task that runs a turn later (Collin,
//! 2026-11-02: the hero's Copy did nothing in Safari). Where the API is missing, the text is
//! copied the old way, by the copy command, still inside the click, from a textarea the reader
//! never sees: selecting the visible box painted its selection across every line's whitespace
//! (Collin, 2026-11-02). A page that offers a copy says so when the copy does not happen, and
//! selects the text instead, so the reader's own copy key finishes the job.

use std::future::Future;
use std::pin::Pin;

/// Put `text` on the clipboard. The write starts now, in the caller's turn; the future says whether
/// the browser took it. When the Clipboard API is not there, the copy command does it at once.
pub fn copy(text: &str) -> Pin<Box<dyn Future<Output = bool>>> {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::{JsCast, JsValue};
        let Some(window) = web_sys::window() else { return Box::pin(std::future::ready(false)) };
        let navigator = window.navigator();
        let api = match js_sys::Reflect::get(&navigator, &JsValue::from_str("clipboard")) {
            Ok(value) if !value.is_undefined() && !value.is_null() => Some(value.unchecked_into::<web_sys::Clipboard>()),
            _ => None,
        };
        match api {
            Some(clipboard) => {
                // Started here, inside the gesture; awaited by the caller.
                let promise = clipboard.write_text(text);
                Box::pin(async move { wasm_bindgen_futures::JsFuture::from(promise).await.is_ok() })
            }
            None => Box::pin(std::future::ready(copy_by_command(text))),
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = text;
        Box::pin(std::future::ready(false))
    }
}

/// Copy `text` with the browser's copy command, from a textarea placed off screen for the moment
/// it takes: the way to the clipboard on a page with no Clipboard API, inside a click. Nothing the
/// reader sees is selected. True when the browser says it copied.
pub fn copy_by_command(text: &str) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::{JsCast, JsValue};
        let Some(window) = web_sys::window() else { return false };
        let Some(document) = window.document() else { return false };
        let Some(body) = document.body() else { return false };
        let Ok(element) = document.create_element("textarea") else { return false };
        let Ok(area) = element.dyn_into::<web_sys::HtmlTextAreaElement>() else { return false };
        area.set_value(text);
        area.set_read_only(true);
        let _ = area.set_attribute("aria-hidden", "true");
        let _ = area.set_attribute("style", "position:fixed;top:0;left:0;width:1px;height:1px;opacity:0;border:0;padding:0;margin:0");
        if body.append_child(&area).is_err() {
            return false;
        }
        let _ = area.focus();
        area.select();
        // `document.execCommand("copy")`, reached by name: web_sys does not bind the old command.
        let copied = js_sys::Reflect::get(&document, &JsValue::from_str("execCommand"))
            .ok()
            .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
            .and_then(|f| f.call1(&document, &JsValue::from_str("copy")).ok())
            .is_some_and(|v| v.is_truthy());
        area.remove();
        if let Ok(Some(selection)) = window.get_selection() {
            let _ = selection.remove_all_ranges();
        }
        copied
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = text;
        false
    }
}

/// Focus the element `id` and select the text inside it, from its first character to its last,
/// so the reader can copy it by hand. The selection runs over the text, not the box around it.
pub fn select_contents(id: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        let Some(window) = web_sys::window() else { return };
        let Some(document) = window.document() else { return };
        let Some(element) = document.get_element_by_id(id) else { return };
        if let Some(focusable) = element.dyn_ref::<web_sys::HtmlElement>() {
            let _ = focusable.focus();
        }
        let (Ok(Some(selection)), Ok(range)) = (window.get_selection(), document.create_range()) else { return };
        let Ok(walker) = document.create_tree_walker_with_what_to_show(&element, 4 /* NodeFilter.SHOW_TEXT */) else { return };
        let mut first: Option<web_sys::Node> = None;
        let mut last: Option<web_sys::Node> = None;
        while let Ok(Some(node)) = walker.next_node() {
            if first.is_none() {
                first = Some(node.clone());
            }
            last = Some(node);
        }
        let (Some(first), Some(last)) = (first, last) else { return };
        let end = last.text_content().map_or(0, |t| t.encode_utf16().count() as u32);
        if range.set_start(&first, 0).is_err() || range.set_end(&last, end).is_err() {
            return;
        }
        let _ = selection.remove_all_ranges();
        let _ = selection.add_range(&range);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = id;
}
