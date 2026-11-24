//! Building a Web page's preview beside the page instead of on its main thread.
//!
//! A preview is the LOGOS compiler plus a first render, seconds of work on a phone. Run where the
//! Studio runs, every edit held the whole page still until it finished. In the browser the build
//! runs in a worker that loads the site's own bundle ([`worker`], `assets/preview-worker.js`), and
//! the frame keeps its last document until the new one arrives. A browser that cannot start the
//! worker builds on the main thread as before.

use std::collections::HashMap;

/// How many built documents a session keeps. A document carries its page's wasm and runs to about
/// half a megabyte, so keeping one per edit grew without end on a phone.
pub const KEPT_DOCUMENTS: usize = 16;

/// Keep `doc` as the build of `source`, within [`KEPT_DOCUMENTS`].
pub fn remember(built: &mut HashMap<String, String>, source: String, doc: String) {
    if built.len() >= KEPT_DOCUMENTS && !built.contains_key(&source) {
        if let Some(oldest) = built.keys().next().cloned() {
            built.remove(&oldest);
        }
    }
    built.insert(source, doc);
}

/// The builds asked of the worker: the one running, and the latest one waiting behind it. A source
/// typed over before its turn came is never built.
#[derive(Debug)]
pub struct BuildQueue<T> {
    running: Option<(String, T)>,
    waiting: Option<(String, T)>,
}

impl<T> Default for BuildQueue<T> {
    fn default() -> Self {
        Self { running: None, waiting: None }
    }
}

impl<T> BuildQueue<T> {
    /// Ask for `source`. Returns the source to send to the worker now, if it is free.
    pub fn ask(&mut self, source: String, done: T) -> Option<String> {
        let asked = |slot: &Option<(String, T)>| slot.as_ref().is_some_and(|(s, _)| *s == source);
        if asked(&self.running) || asked(&self.waiting) {
            return None;
        }
        if self.running.is_some() {
            self.waiting = Some((source, done));
            return None;
        }
        self.running = Some((source.clone(), done));
        Some(source)
    }

    /// The running build finished: what to tell its asker, and the next source to send.
    pub fn finish(&mut self) -> (Option<T>, Option<String>) {
        let done = self.running.take().map(|(_, done)| done);
        self.running = self.waiting.take();
        (done, self.running.as_ref().map(|(source, _)| source.clone()))
    }

    /// The worker is gone: every asker still waiting.
    pub fn abandon(&mut self) -> Vec<T> {
        [self.running.take(), self.waiting.take()].into_iter().flatten().map(|(_, done)| done).collect()
    }
}

/// The worker that runs the builds.
#[cfg(target_arch = "wasm32")]
pub mod worker {
    use super::BuildQueue;
    use std::cell::RefCell;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::{JsCast, JsValue};
    use web_sys::{ErrorEvent, MessageEvent, Worker, WorkerOptions, WorkerType};

    /// What a build's asker is told: the document, or `None` when it must build for itself.
    type Done = Box<dyn FnOnce(Option<String>)>;

    struct Builder {
        worker: Worker,
        entry: String,
        queue: BuildQueue<Done>,
        _on_message: Closure<dyn FnMut(MessageEvent)>,
        _on_error: Closure<dyn FnMut(ErrorEvent)>,
    }

    enum Slot {
        Unstarted,
        Ready(Builder),
        Unavailable,
    }

    thread_local! {
        static BUILDER: RefCell<Slot> = const { RefCell::new(Slot::Unstarted) };
    }

    /// The address of the site's own bundle, which the worker loads to reach the compiler.
    fn bundle_url() -> Option<String> {
        let document = web_sys::window()?.document()?;
        let script = document.query_selector("script[src*=\"logicaffeine-web-\"]").ok().flatten()?;
        let src = script.get_attribute("src")?;
        let base = document.base_uri().ok().flatten()?;
        web_sys::Url::new_with_base(&src, &base).ok().map(|url| url.href())
    }

    fn send(worker: &Worker, entry: &str, source: &str) -> bool {
        let message = js_sys::Object::new();
        let put = |key: &str, value: &str| js_sys::Reflect::set(&message, &JsValue::from_str(key), &JsValue::from_str(value)).is_ok();
        put("entry", entry) && put("source", source) && worker.post_message(&message).is_ok()
    }

    /// The worker failed for good: every asker builds for itself from here on.
    fn give_up() {
        let waiting = BUILDER.with(|slot| match std::mem::replace(&mut *slot.borrow_mut(), Slot::Unavailable) {
            Slot::Ready(mut builder) => {
                builder.worker.terminate();
                // One of these two may be the closure running this very call.
                std::mem::forget(builder._on_message);
                std::mem::forget(builder._on_error);
                builder.queue.abandon()
            }
            _ => Vec::new(),
        });
        for done in waiting {
            done(None);
        }
    }

    fn answered(event: MessageEvent) {
        let data = event.data();
        let doc = js_sys::Reflect::get(&data, &JsValue::from_str("doc")).ok().and_then(|d| d.as_string());
        let Some(doc) = doc else {
            give_up();
            return;
        };
        let (done, failed) = BUILDER.with(|slot| match &mut *slot.borrow_mut() {
            Slot::Ready(builder) => {
                let (done, next) = builder.queue.finish();
                let failed = next.is_some_and(|source| !send(&builder.worker, &builder.entry, &source));
                (done, failed)
            }
            _ => (None, false),
        });
        if let Some(done) = done {
            done(Some(doc));
        }
        if failed {
            give_up();
        }
    }

    fn start() -> Option<Builder> {
        let entry = bundle_url()?;
        let options = WorkerOptions::new();
        options.set_type(WorkerType::Module);
        let worker = Worker::new_with_options("/assets/preview-worker.js", &options).ok()?;
        let on_message = Closure::<dyn FnMut(MessageEvent)>::new(answered);
        let on_error = Closure::<dyn FnMut(ErrorEvent)>::new(|_: ErrorEvent| give_up());
        worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        worker.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        Some(Builder { worker, entry, queue: BuildQueue::default(), _on_message: on_message, _on_error: on_error })
    }

    /// Ask the worker to build `source`, and tell `done` when it has. `false` when this browser has
    /// no worker to ask, and the caller builds it itself.
    pub fn build(source: String, done: impl FnOnce(Option<String>) + 'static) -> bool {
        let sent = BUILDER.with(|slot| {
            let mut slot = slot.borrow_mut();
            if matches!(*slot, Slot::Unstarted) {
                *slot = start().map_or(Slot::Unavailable, Slot::Ready);
            }
            let Slot::Ready(builder) = &mut *slot else { return None };
            let now = builder.queue.ask(source, Box::new(done));
            Some(now.is_none_or(|source| send(&builder.worker, &builder.entry, &source)))
        });
        match sent {
            None => false,
            Some(true) => true,
            Some(false) => {
                give_up();
                true
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/preview_builds/tests.rs"]
mod tests;
