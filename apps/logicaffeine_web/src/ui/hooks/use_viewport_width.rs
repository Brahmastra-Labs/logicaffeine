//! The page's width, kept current as the window resizes.

use dioxus::prelude::*;

/// The width assumed where there is no window to measure (the server render).
pub const UNMEASURED_WIDTH: f64 = 1440.0;

/// The document's client width in CSS pixels (the viewport without its scrollbar), updated
/// on every window resize for as long as the calling component is mounted.
pub fn use_viewport_width() -> Signal<f64> {
    #[cfg(target_arch = "wasm32")]
    {
        use std::rc::Rc;
        use wasm_bindgen::prelude::*;
        use wasm_bindgen::JsCast;

        fn measure() -> Option<f64> {
            Some(web_sys::window()?.document()?.document_element()?.client_width() as f64)
        }

        let mut width = use_signal(|| measure().unwrap_or(UNMEASURED_WIDTH));
        let on_resize = use_hook(move || {
            let on_resize = Closure::<dyn FnMut()>::new(move || {
                if let Some(w) = measure() {
                    width.set(w);
                }
            });
            if let Some(window) = web_sys::window() {
                let _ = window.add_event_listener_with_callback("resize", on_resize.as_ref().unchecked_ref());
            }
            Rc::new(on_resize)
        });
        use_drop(move || {
            if let Some(window) = web_sys::window() {
                let _ = window.remove_event_listener_with_callback("resize", (*on_resize).as_ref().unchecked_ref());
            }
        });
        width
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use_signal(|| UNMEASURED_WIDTH)
    }
}
