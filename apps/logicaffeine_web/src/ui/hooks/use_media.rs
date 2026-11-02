//! Whether the page matches a media query, kept current as the window changes.

use dioxus::prelude::*;

/// Whether the window matches any of `queries` (named in [`crate::ui::responsive::media`]), updated
/// whenever that changes for as long as the calling component is mounted. False where there is no
/// window (the server render), which draws the desktop.
pub fn use_media(queries: &'static [&'static str]) -> Signal<bool> {
    #[cfg(target_arch = "wasm32")]
    {
        use std::rc::Rc;
        use wasm_bindgen::prelude::*;
        use wasm_bindgen::JsCast;

        fn list(queries: &[&str]) -> Option<web_sys::MediaQueryList> {
            web_sys::window()?.match_media(&queries.join(", ")).ok()?
        }

        let mut matches = use_signal(|| list(queries).is_some_and(|l| l.matches()));
        let watched = use_hook(move || {
            let query = list(queries);
            let on_change = Closure::<dyn FnMut()>::new(move || {
                if let Some(now) = list(queries).map(|l| l.matches()) {
                    if *matches.peek() != now {
                        matches.set(now);
                    }
                }
            });
            if let Some(query) = &query {
                let _ = query.add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref());
            }
            Rc::new((query, on_change))
        });
        use_drop(move || {
            if let (Some(query), on_change) = (&watched.0, &watched.1) {
                let _ = query.remove_event_listener_with_callback("change", on_change.as_ref().unchecked_ref());
            }
        });
        matches
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = queries;
        use_signal(|| false)
    }
}
