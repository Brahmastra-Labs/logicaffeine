//! The observation channel's emission buffer. When an observation statement (`Note`/`Warn`/`Record`) executes, it
//! emits a structured [`ObsEvent`] to a per-run thread-local buffer, drained by
//! sinks (the in-memory test sink today; console and wire in Wave 16). This is
//! the SEPARATE observation channel (D0.1) — it never touches program output.
//!
//! Emission is wait-free from the program's perspective (a buffer push) and
//! never draws from the scheduler's entropy, so a run's `SchedTrace` is
//! identical with observation on or off.

use serde::{Deserialize, Serialize};
use std::cell::RefCell;

/// The speech-act category of a message event (mirrors the language AST's
/// `ObsCategory`, kept local so sinks need no AST dependency).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObsCat {
    Note,
    Warn,
    Record,
}

/// What an emitted event IS: a speech-act message, or a trace-span boundary.
/// Spans are lexical (`During`),
/// so every `SpanEnter` is matched by exactly one `SpanExit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Message(ObsCat),
    SpanEnter,
    SpanExit,
}

/// One emitted observation event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObsEvent {
    pub kind: EventKind,
    /// For a message, the that-clause; for a span, the span label.
    pub message: String,
    /// Captured payloads: (name, display value). Empty for spans.
    pub payloads: Vec<(String, String)>,
    /// Per-run monotone sequence number (the emission order).
    pub seq: u64,
}

thread_local! {
    /// The active run's event buffer. `None` = no capture installed (emission
    /// is a cheap no-op — the zero-cost-when-nobody-listens path).
    static BUFFER: RefCell<Option<Vec<ObsEvent>>> = const { RefCell::new(None) };
    /// Per-statement sampling counters, keyed by a statement id, so
    /// `once in every N times` is deterministic and per-site.
    static SAMPLE_COUNTERS: RefCell<std::collections::HashMap<u64, u64>> =
        RefCell::new(std::collections::HashMap::new());
}

/// Begin capturing events for a run (the `largo test` runner, per test).
pub fn install() {
    BUFFER.with(|b| *b.borrow_mut() = Some(Vec::new()));
    SAMPLE_COUNTERS.with(|c| c.borrow_mut().clear());
}

/// Take the captured events and stop capturing.
pub fn drain() -> Vec<ObsEvent> {
    let events = BUFFER.with(|b| b.borrow_mut().take()).unwrap_or_default();
    SAMPLE_COUNTERS.with(|c| c.borrow_mut().clear());
    events
}

/// Emit an event. Honors per-site sampling: with `sample = Some(n)` and a
/// stable `site` id, only 1 in every `n` reaching this site is buffered (the
/// first, then every nth). Sampling is a rendering decision — replay with
/// capture-everything regenerates the full stream.
pub fn emit(
    category: ObsCat,
    message: String,
    payloads: Vec<(String, String)>,
    sample: Option<u64>,
    site: u64,
) {
    if let Some(n) = sample {
        let keep = SAMPLE_COUNTERS.with(|c| {
            let mut m = c.borrow_mut();
            let count = m.entry(site).or_insert(0);
            let keep = *count % n.max(1) == 0;
            *count += 1;
            keep
        });
        if !keep {
            return;
        }
    }
    push(EventKind::Message(category), message, payloads);
}

/// Emit a `SpanEnter` for a `During` block (Wave 8).
pub fn enter_span(label: String) {
    push(EventKind::SpanEnter, label, Vec::new());
}

/// Emit a `SpanExit` — always paired with its `enter_span` (the `During` arm
/// emits it on every exit path, so spans stay balanced).
pub fn exit_span(label: String) {
    push(EventKind::SpanExit, label, Vec::new());
}

fn push(kind: EventKind, message: String, payloads: Vec<(String, String)>) {
    BUFFER.with(|b| {
        if let Some(buf) = b.borrow_mut().as_mut() {
            let seq = buf.len() as u64;
            buf.push(ObsEvent {
                kind,
                message,
                payloads,
                seq,
            });
        }
    });
}

/// True when a capture buffer is installed (lets emitters skip work otherwise).
pub fn capturing() -> bool {
    BUFFER.with(|b| b.borrow().is_some())
}

/// Serialize an event stream to the `.logosobs` wire form — one JSON object
/// per line, so it appends cheaply and greps line-by-line
/// (columnar batching deferred).
pub fn to_logosobs(events: &[ObsEvent]) -> String {
    events
        .iter()
        .map(|e| serde_json::to_string(e).expect("an ObsEvent always serializes"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Parse a `.logosobs` stream back to events (the `largo trace` reader).
pub fn from_logosobs(text: &str) -> Result<Vec<ObsEvent>, String> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).map_err(|e| format!("bad .logosobs line: {e}")))
        .collect()
}
