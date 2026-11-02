//! Pages whose route models a `#fragment` open at the element it names.
//!
//! The route keeps the fragment through boot normalization (`#:section` in its `#[route]`), but
//! the browser's own jump to it cannot be relied on: the prerendered copy it jumped in is swapped
//! for the live app, a link inside the app changes the route without loading a page, and a page
//! whose body is fetched renders the element only once the fetch lands. So the page scrolls
//! itself, once the element exists, honouring `prefers-reduced-motion`.

/// How long to wait for the element to appear: a fetched page body (the legal pages) can take a
/// few seconds on a slow connection.
#[cfg(target_arch = "wasm32")]
const WAIT_FRAMES: u32 = 600;

/// Bring the element with `id` to the top of the page, under the sticky nav (the target's
/// `scroll-margin-top` sets the gap), waiting up to ten seconds for it to be rendered.
pub fn scroll_to_section(id: String) {
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(async move {
        for _ in 0..WAIT_FRAMES {
            gloo_timers::future::TimeoutFuture::new(16).await;
            let Some(window) = web_sys::window() else { return };
            let Some(element) = window.document().and_then(|d| d.get_element_by_id(&id)) else { continue };
            let reduced = window
                .match_media("(prefers-reduced-motion: reduce)")
                .ok()
                .flatten()
                .is_some_and(|m| m.matches());
            let options = web_sys::ScrollIntoViewOptions::new();
            options.set_behavior(if reduced { web_sys::ScrollBehavior::Instant } else { web_sys::ScrollBehavior::Smooth });
            options.set_block(web_sys::ScrollLogicalPosition::Start);
            element.scroll_into_view_with_scroll_into_view_options(&options);
            return;
        }
    });
    #[cfg(not(target_arch = "wasm32"))]
    let _ = id;
}
