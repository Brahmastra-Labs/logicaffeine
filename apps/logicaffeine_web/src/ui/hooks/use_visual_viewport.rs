//! How much of the window an on-screen keyboard covers.

use dioxus::prelude::*;

/// The height, in CSS pixels, of the window's bottom that an on-screen keyboard covers: what the
/// visual viewport leaves of the layout viewport. Android with `interactive-widget=resizes-content`
/// shrinks the layout viewport itself, so this stays 0 there; iOS keeps the layout viewport and
/// shrinks only the visual one. Updated as the keyboard opens and closes, 0 off the browser.
pub fn use_keyboard_inset() -> Signal<f64> {
    #[cfg(target_arch = "wasm32")]
    {
        use std::rc::Rc;
        use wasm_bindgen::prelude::*;
        use wasm_bindgen::JsCast;

        fn measure() -> Option<f64> {
            let window = web_sys::window()?;
            let layout = window.inner_height().ok()?.as_f64()?;
            let visual = window.visual_viewport()?;
            Some((layout - visual.height() - visual.offset_top()).max(0.0))
        }

        let mut inset = use_signal(|| measure().unwrap_or(0.0));
        let on_resize = use_hook(move || {
            let on_resize = Closure::<dyn FnMut()>::new(move || {
                if let Some(now) = measure() {
                    if (*inset.peek() - now).abs() >= 1.0 {
                        inset.set(now);
                    }
                }
            });
            if let Some(visual) = web_sys::window().and_then(|w| w.visual_viewport()) {
                let _ = visual.add_event_listener_with_callback("resize", on_resize.as_ref().unchecked_ref());
                let _ = visual.add_event_listener_with_callback("scroll", on_resize.as_ref().unchecked_ref());
            }
            Rc::new(on_resize)
        });
        use_drop(move || {
            if let Some(visual) = web_sys::window().and_then(|w| w.visual_viewport()) {
                let _ = visual.remove_event_listener_with_callback("resize", (*on_resize).as_ref().unchecked_ref());
                let _ = visual.remove_event_listener_with_callback("scroll", (*on_resize).as_ref().unchecked_ref());
            }
        });
        inset
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use_signal(|| 0.0)
    }
}
