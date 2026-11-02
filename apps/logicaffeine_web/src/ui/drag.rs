//! Pointer drags that follow the pointer at frame rate without re-rendering the page.
//!
//! A drag captures the pointer on its handle, so it keeps tracking over iframes and outside
//! the window, and reports movement at most once per animation frame. The caller paints each
//! frame straight onto the DOM (a CSS custom property or an inline size) and commits the final
//! value to its signal once, when the pointer is released. Re-rendering a component as large
//! as the Studio on every pointer event is what made its resizers lag.

/// The direction a drag resizes, which sets the cursor shown everywhere while it lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Resizing widths: a column boundary.
    Columns,
    /// Resizing heights: a row boundary.
    Rows,
}

impl Axis {
    /// The class the document root carries while a drag on this axis is live.
    pub fn root_class(self) -> &'static str {
        match self {
            Axis::Columns => "dragging-columns",
            Axis::Rows => "dragging-rows",
        }
    }
}

/// While a drag is live: no text selection anywhere, the resize cursor everywhere, and no
/// iframe (the Web preview) swallowing pointer events.
pub const DRAG_STYLE: &str = r#"
html.dragging, html.dragging * { user-select: none !important; -webkit-user-select: none !important; }
html.dragging iframe { pointer-events: none !important; }
html.dragging-columns, html.dragging-columns * { cursor: col-resize !important; }
html.dragging-rows, html.dragging-rows * { cursor: row-resize !important; }
"#;

#[cfg(target_arch = "wasm32")]
pub use live::begin;

#[cfg(target_arch = "wasm32")]
mod live {
    use super::Axis;
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;

    const ENDS: [&str; 3] = ["pointerup", "pointercancel", "lostpointercapture"];

    struct Live {
        handle: web_sys::Element,
        axis: Axis,
        origin: (f64, f64),
        latest: (f64, f64),
        frame: Option<i32>,
        on_frame: Box<dyn FnMut(f64, f64)>,
        on_end: Option<Box<dyn FnOnce(f64, f64)>>,
        on_move: Option<Closure<dyn FnMut(web_sys::PointerEvent)>>,
        on_stop: Option<Closure<dyn FnMut(web_sys::PointerEvent)>>,
        paint: Option<Closure<dyn FnMut()>>,
    }

    fn root() -> Option<web_sys::Element> {
        web_sys::window()?.document()?.document_element()
    }

    /// Start a drag on `handle` for the pointer that just went down at `origin` (client
    /// coordinates). `on_frame` receives the offset from `origin` at most once per animation
    /// frame; `on_end` receives the final offset once, when the pointer is released.
    pub fn begin(
        handle: web_sys::Element,
        pointer_id: i32,
        origin: (f64, f64),
        axis: Axis,
        on_frame: impl FnMut(f64, f64) + 'static,
        on_end: impl FnOnce(f64, f64) + 'static,
    ) {
        let _ = handle.set_pointer_capture(pointer_id);
        if let Some(root) = root() {
            let _ = root.class_list().add_2("dragging", axis.root_class());
        }

        let live = Rc::new(RefCell::new(Live {
            handle: handle.clone(),
            axis,
            origin,
            latest: origin,
            frame: None,
            on_frame: Box::new(on_frame),
            on_end: Some(Box::new(on_end)),
            on_move: None,
            on_stop: None,
            paint: None,
        }));

        let weak = Rc::downgrade(&live);
        let paint = Closure::<dyn FnMut()>::new(move || {
            let Some(live) = weak.upgrade() else { return };
            let mut live = live.borrow_mut();
            live.frame = None;
            let (x, y) = live.latest;
            let (ox, oy) = live.origin;
            (live.on_frame)(x - ox, y - oy);
        });

        let weak = Rc::downgrade(&live);
        let on_move = Closure::<dyn FnMut(web_sys::PointerEvent)>::new(move |e: web_sys::PointerEvent| {
            let Some(live) = weak.upgrade() else { return };
            let mut live = live.borrow_mut();
            live.latest = (e.client_x() as f64, e.client_y() as f64);
            if live.frame.is_none() {
                let request = live
                    .paint
                    .as_ref()
                    .and_then(|paint| web_sys::window()?.request_animation_frame(paint.as_ref().unchecked_ref()).ok());
                live.frame = request;
            }
        });

        let weak = Rc::downgrade(&live);
        let on_stop = Closure::<dyn FnMut(web_sys::PointerEvent)>::new(move |e: web_sys::PointerEvent| {
            let Some(live) = weak.upgrade() else { return };
            if e.type_() != "lostpointercapture" {
                live.borrow_mut().latest = (e.client_x() as f64, e.client_y() as f64);
            }
            finish(live);
        });

        let _ = handle.add_event_listener_with_callback("pointermove", on_move.as_ref().unchecked_ref());
        for end in ENDS {
            let _ = handle.add_event_listener_with_callback(end, on_stop.as_ref().unchecked_ref());
        }

        let mut state = live.borrow_mut();
        state.paint = Some(paint);
        state.on_move = Some(on_move);
        state.on_stop = Some(on_stop);
        drop(state);
        // The listeners hold only weak references; this strong one keeps the drag alive until
        // `finish` releases it.
        LIVE.with(|slot| *slot.borrow_mut() = Some(live));
    }

    thread_local! {
        static LIVE: RefCell<Option<Rc<RefCell<Live>>>> = const { RefCell::new(None) };
    }

    fn finish(live: Rc<RefCell<Live>>) {
        let (on_end, offset) = {
            let mut state = live.borrow_mut();
            let Some(on_end) = state.on_end.take() else { return };
            if let (Some(id), Some(window)) = (state.frame.take(), web_sys::window()) {
                let _ = window.cancel_animation_frame(id);
            }
            if let Some(on_move) = &state.on_move {
                let _ = state.handle.remove_event_listener_with_callback("pointermove", on_move.as_ref().unchecked_ref());
            }
            if let Some(on_stop) = &state.on_stop {
                for end in ENDS {
                    let _ = state.handle.remove_event_listener_with_callback(end, on_stop.as_ref().unchecked_ref());
                }
            }
            if let Some(root) = root() {
                let _ = root.class_list().remove_2("dragging", state.axis.root_class());
            }
            let (x, y) = state.latest;
            let (ox, oy) = state.origin;
            (on_end, (x - ox, y - oy))
        };
        on_end(offset.0, offset.1);
        // A listener cannot free the closure that is running it: drop the drag once this
        // event has finished dispatching.
        LIVE.with(|slot| {
            let done = slot.borrow_mut().take();
            wasm_bindgen_futures::spawn_local(async move { drop(done) });
        });
        drop(live);
    }
}

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/drag/tests.rs"]
mod tests;
