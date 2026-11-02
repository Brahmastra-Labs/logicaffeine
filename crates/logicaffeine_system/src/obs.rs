//! The observation channel for COMPILED (AOT-Rust) programs — Wave 13.
//!
//! Emitted-Rust programs compiled in the OBSERVED profile call these functions
//! for `Note`/`Warn`/`Record`/`During`, so the compiled binary produces the same
//! observation event stream the interpreter and VM do (the compile-side buffer
//! lives in `logicaffeine_compile::observe`; this is its standalone-runtime twin,
//! reachable from generated code via `use logicaffeine_system::*`). Events land
//! in a thread-local buffer a host sink drains; when nothing is installed,
//! emission is a cheap no-op, so the release (elided) profile — which never emits
//! these calls at all — pays exactly nothing.

use std::cell::RefCell;

/// One observation event captured in a compiled program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObsEvent {
    /// `"note"` / `"warn"` / `"record"` for a message, or `"span-enter"` /
    /// `"span-exit"` for a `During` boundary.
    pub kind: String,
    /// The that-clause text (or the span label).
    pub message: String,
    /// Captured payloads (`with X and Y`), name → rendered value.
    pub payloads: Vec<(String, String)>,
}

thread_local! {
    static BUFFER: RefCell<Option<Vec<ObsEvent>>> = const { RefCell::new(None) };
}

/// Begin capturing observation events on this thread (a fresh, empty buffer).
pub fn install() {
    BUFFER.with(|b| *b.borrow_mut() = Some(Vec::new()));
}

/// Take and clear the captured events (empty if nothing was installed).
pub fn drain() -> Vec<ObsEvent> {
    BUFFER.with(|b| {
        b.borrow_mut()
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    })
}

/// Is a sink capturing on this thread? (Emission is a no-op otherwise.)
pub fn capturing() -> bool {
    BUFFER.with(|b| b.borrow().is_some())
}

fn push(kind: &str, message: &str, payloads: Vec<(String, String)>) {
    BUFFER.with(|b| {
        if let Some(buf) = b.borrow_mut().as_mut() {
            buf.push(ObsEvent {
                kind: kind.to_string(),
                message: message.to_string(),
                payloads,
            });
        }
    });
}

/// Emit a `Note`/`Warn`/`Record` message with its captured payloads.
pub fn record(kind: &str, message: &str, payloads: &[(&str, String)]) {
    if !capturing() {
        return;
    }
    let owned = payloads
        .iter()
        .map(|(n, v)| (n.to_string(), v.clone()))
        .collect();
    push(kind, message, owned);
}

/// Emit a `During` span entry.
pub fn span_enter(label: &str) {
    push("span-enter", label, Vec::new());
}

/// Emit a `During` span exit (always paired with its `span_enter`).
pub fn span_exit(label: &str) {
    push("span-exit", label, Vec::new());
}
