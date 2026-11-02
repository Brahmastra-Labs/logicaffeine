//! Where the screen's pixels fall under an element: how large one device pixel is, and how far the
//! element's box starts past the device-pixel grid, kept current as the page zooms, moves to a
//! screen of another density, or lays the element out anew.

use dioxus::prelude::*;

/// One device pixel in CSS pixels, and the nudge that puts an element's content on the
/// device-pixel grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PixelGrid {
    /// One device pixel, in CSS pixels (`1 / devicePixelRatio`).
    pub device_px: f64,
    /// How far right, in CSS pixels, the element's content must move for its left edge to land on
    /// a device pixel: at least 0 and under one device pixel.
    pub nudge_x: f64,
    /// How far down, likewise, for its top edge.
    pub nudge_y: f64,
}

impl PixelGrid {
    /// The grid assumed where there is no window to measure (the server render): CSS pixels, no
    /// nudge.
    pub const UNMEASURED: PixelGrid = PixelGrid { device_px: 1.0, nudge_x: 0.0, nudge_y: 0.0 };

    /// A corner closer than this to a device pixel's edge, in device pixels, is on it. Layout keeps
    /// positions in its own units, so a corner meant for a grid line can land a hair past it, and a
    /// nudge of almost a whole pixel would only shift the content.
    pub const ON_GRID: f64 = 1e-3;

    /// The grid at device pixel ratio `ratio` under a box whose top-left corner sits at (`left`,
    /// `top`) CSS pixels in the viewport.
    pub fn at(ratio: f64, left: f64, top: f64) -> PixelGrid {
        if !(ratio.is_finite() && ratio > 0.0) {
            return Self::UNMEASURED;
        }
        let nudge = |at: f64| {
            let past = (at * ratio).rem_euclid(1.0);
            if !past.is_finite() || past < Self::ON_GRID || past > 1.0 - Self::ON_GRID {
                0.0
            } else {
                (1.0 - past) / ratio
            }
        };
        PixelGrid { device_px: 1.0 / ratio, nudge_x: nudge(left), nudge_y: nudge(top) }
    }
}

/// The device-pixel grid under the element given the returned `onmounted` handler, updated for as
/// long as the calling component is mounted.
///
/// The element's layout is watched by a `ResizeObserver`, which reports the box once it is laid out
/// and again whenever it changes size — the window resized, the page zoomed, or the layout around
/// it moved it. A move to a screen of another density changes the ratio alone, which only a
/// `resolution` media query reports; that query is re-armed for each new ratio. An element inside
/// a scrolling column also moves without changing size — the column scrolls, or what sits above it
/// in the column grows or shrinks — so the column's scroll and every change to its content are
/// watched too.
pub fn use_device_pixel_grid() -> (Signal<PixelGrid>, Callback<MountedEvent>) {
    #[cfg(target_arch = "wasm32")]
    {
        use std::cell::{Cell, RefCell};
        use std::rc::Rc;
        use wasm_bindgen::prelude::*;
        use wasm_bindgen::JsCast;

        /// What the grid is read from, and the watchers that say when to read it again.
        #[derive(Default)]
        struct Watch {
            element: RefCell<Option<web_sys::Element>>,
            observer: RefCell<Option<web_sys::ResizeObserver>>,
            /// The resolution query armed on the ratio last measured, which changes when the ratio
            /// leaves it.
            query: RefCell<Option<web_sys::MediaQueryList>>,
            armed: Cell<f64>,
            refresh: RefCell<Option<js_sys::Function>>,
            /// The scrolling column the element sits in, if any, and the watcher on its content.
            column: RefCell<Option<web_sys::Element>>,
            mutations: RefCell<Option<web_sys::MutationObserver>>,
        }

        impl Watch {
            /// The page's ratio, and the grid under the element (under the viewport's corner until
            /// the element is mounted).
            fn measure(&self) -> Option<(f64, PixelGrid)> {
                let ratio = web_sys::window()?.device_pixel_ratio();
                let (left, top) = self.element.borrow().as_ref().map_or((0.0, 0.0), |element| {
                    let rect = element.get_bounding_client_rect();
                    (rect.left(), rect.top())
                });
                Some((ratio, PixelGrid::at(ratio, left, top)))
            }

            fn arm(&self, ratio: f64) {
                if self.armed.get() == ratio && self.query.borrow().is_some() {
                    return;
                }
                self.disarm();
                let refresh = self.refresh.borrow();
                let Some(refresh) = refresh.as_ref() else { return };
                let query = web_sys::window().and_then(|w| w.match_media(&format!("(resolution: {ratio}dppx)")).ok().flatten());
                if let Some(query) = query {
                    let _ = query.add_event_listener_with_callback("change", refresh);
                    *self.query.borrow_mut() = Some(query);
                    self.armed.set(ratio);
                }
            }

            fn disarm(&self) {
                if let (Some(query), Some(refresh)) = (self.query.borrow_mut().take(), self.refresh.borrow().as_ref()) {
                    let _ = query.remove_event_listener_with_callback("change", refresh);
                }
            }

            fn observe(&self, element: web_sys::Element) {
                if let Some(observer) = self.observer.borrow().as_ref() {
                    if let Some(previous) = self.element.borrow().as_ref() {
                        observer.unobserve(previous);
                    }
                    let options = web_sys::ResizeObserverOptions::new();
                    options.set_box(web_sys::ResizeObserverBoxOptions::BorderBox);
                    observer.observe_with_options(&element, &options);
                }
                self.follow_column(&element);
                *self.element.borrow_mut() = Some(element);
            }

            /// Watch the scrolling column around `element`: its scroll, and any change to what it
            /// holds, move the element without resizing it.
            fn follow_column(&self, element: &web_sys::Element) {
                self.unfollow_column();
                let Some(window) = web_sys::window() else { return };
                let refresh = self.refresh.borrow();
                let Some(refresh) = refresh.as_ref() else { return };
                let scrolls = |el: &web_sys::Element| {
                    window.get_computed_style(el).ok().flatten().is_some_and(|style| {
                        let overflow = style.get_property_value("overflow-y").unwrap_or_default();
                        overflow == "auto" || overflow == "scroll"
                    })
                };
                let mut column = element.parent_element();
                while let Some(el) = column.as_ref() {
                    if scrolls(el) {
                        break;
                    }
                    column = el.parent_element();
                }
                let Some(column) = column else { return };
                let _ = column.add_event_listener_with_callback("scroll", refresh);
                if let Ok(mutations) = web_sys::MutationObserver::new(refresh) {
                    let options = web_sys::MutationObserverInit::new();
                    options.set_subtree(true);
                    options.set_child_list(true);
                    options.set_attributes(true);
                    options.set_character_data(true);
                    let _ = mutations.observe_with_options(&column, &options);
                    *self.mutations.borrow_mut() = Some(mutations);
                }
                *self.column.borrow_mut() = Some(column);
            }

            fn unfollow_column(&self) {
                if let Some(mutations) = self.mutations.borrow_mut().take() {
                    mutations.disconnect();
                }
                if let (Some(column), Some(refresh)) = (self.column.borrow_mut().take(), self.refresh.borrow().as_ref()) {
                    let _ = column.remove_event_listener_with_callback("scroll", refresh);
                }
            }

            fn stop(&self) {
                self.disarm();
                self.unfollow_column();
                if let Some(observer) = self.observer.borrow_mut().take() {
                    observer.disconnect();
                }
                self.refresh.borrow_mut().take();
                self.element.borrow_mut().take();
            }
        }

        let mut grid = use_signal(|| {
            web_sys::window().map_or(PixelGrid::UNMEASURED, |w| PixelGrid::at(w.device_pixel_ratio(), 0.0, 0.0))
        });
        let (watch, refresh) = use_hook(move || {
            let watch = Rc::new(Watch::default());
            let refresh = {
                let watch = Rc::clone(&watch);
                Closure::<dyn FnMut()>::new(move || {
                    let Some((ratio, now)) = watch.measure() else { return };
                    watch.arm(ratio);
                    if *grid.peek() != now {
                        grid.set(now);
                    }
                })
            };
            let function = refresh.as_ref().unchecked_ref::<js_sys::Function>().clone();
            *watch.observer.borrow_mut() = web_sys::ResizeObserver::new(&function).ok();
            *watch.refresh.borrow_mut() = Some(function);
            if let Some(window) = web_sys::window() {
                watch.arm(window.device_pixel_ratio());
            }
            (watch, Rc::new(refresh))
        });
        let on_mounted = use_callback({
            let watch = Rc::clone(&watch);
            move |event: MountedEvent| {
                if let Some(element) = event.data().downcast::<web_sys::Element>().cloned() {
                    watch.observe(element);
                }
            }
        });
        use_drop(move || {
            watch.stop();
            drop(refresh);
        });
        (grid, on_mounted)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        (use_signal(|| PixelGrid::UNMEASURED), use_callback(|_: MountedEvent| {}))
    }
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/hooks/use_device_pixel_grid/tests.rs"]
mod tests;
