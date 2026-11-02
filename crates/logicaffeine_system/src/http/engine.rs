//! Engine selection + the pre-serialized static-response table — the
//! cross-platform half of the HTTP fast path.
//!
//! LOGOS serves HTTP everywhere; the ENGINE is a knob. Two engines exist:
//!
//! - **tokio** — the portable baseline in [`super::server`]. Always compiled,
//!   works on every native platform, and is the only engine for the
//!   interpreter tier (whose `!Send` handler drives `Listener`/`Conn` inline).
//! - **uring** — the Linux thread-per-core io_uring engine in
//!   `engine_uring` (behind the `io-uring` feature). The compiled
//!   (AOT) tier's fast path: its handlers are plain `Fn`s, exactly the
//!   engine's contract.
//!
//! [`choose_engine`] is the ONE selection law, applied by the serve entries in
//! `server.rs`: `LOGOS_HTTP_ENGINE=uring|tokio` overrides; the default is
//! uring wherever a probe ring actually initializes (Linux, `io-uring` feature
//! compiled in, kernel support present), else tokio. Selection is ALWAYS
//! announced with one stderr line — loud, never silent — so a deploy can never
//! quietly run the slow path.

use std::collections::HashMap;

use super::wire::{serialize_response_conn, HttpRequest, HttpResponse};

/// Which engine a serve entry runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineKind {
    /// The Linux thread-per-core io_uring engine.
    Uring,
    /// The portable tokio accept loop.
    Tokio,
}

/// The outcome of [`choose_engine`]: the engine plus the human-readable reason
/// that goes on stderr.
#[derive(Debug, Clone)]
pub struct EngineChoice {
    pub kind: EngineKind,
    pub reason: String,
}

/// Read the selection knobs from the environment and apply
/// [`choose_engine_from`]. `LOGOS_HTTP_FORCE_FALLBACK=1` makes the uring probe
/// fail artificially (the fallback-honesty test hook — it exercises the same
/// path a ring-init failure takes on a real kernel without io_uring).
pub fn choose_engine() -> EngineChoice {
    let requested = std::env::var("LOGOS_HTTP_ENGINE").ok();
    let force_fallback = std::env::var("LOGOS_HTTP_FORCE_FALLBACK")
        .map(|v| v.trim() == "1")
        .unwrap_or(false);
    let requested_lifecycle =
        lifecycle_knob_requested(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()));
    match requested_lifecycle {
        // A lifecycle guarantee was ASKED for by name. The uring engine calls its
        // handler inline on the shard thread, so it cannot enforce a wall-clock
        // request budget, answer a probe path, or shed by admission — and silently
        // serving without the guarantee someone configured is the one outcome worse
        // than being slower. Fall back, and say which name did it.
        Some(knob) => EngineChoice {
            kind: EngineKind::Tokio,
            reason: format!(
                "{knob} configures a lifecycle guarantee the io_uring engine cannot enforce \
                 (its handler runs inline on the shard thread); serving on tokio so the \
                 guarantee holds"
            ),
        },
        None => choose_engine_from(requested.as_deref(), force_fallback, probe_uring),
    }
}

/// The name of the first lifecycle knob the environment sets, if any — the pure
/// half of the rule above, so it can be tested without the process environment.
///
/// Only knobs the uring engine cannot honor are listed. Body and read-deadline
/// caps ARE enforced there (they are properties of parsing, not of scheduling),
/// so configuring those does not cost you the fast engine.
pub fn lifecycle_knob_requested(get: impl Fn(&str) -> Option<String>) -> Option<&'static str> {
    const UNSUPPORTED: [&str; 5] = [
        "LOGOS_REQUEST_TIMEOUT_MS",
        "LOGOS_MAX_INFLIGHT",
        "LOGOS_READY_PATH",
        "LOGOS_HEALTH_PATH",
        "LOGOS_CORRELATION_ID",
    ];
    UNSUPPORTED.into_iter().find(|k| get(k).is_some())
}

/// The pure selection law. `probe` initializes a throwaway ring (or explains
/// why it cannot); it runs only when uring is actually a candidate.
pub fn choose_engine_from(
    requested: Option<&str>,
    force_fallback: bool,
    probe: impl FnOnce() -> Result<(), String>,
) -> EngineChoice {
    let requested = requested.map(|r| r.trim().to_ascii_lowercase());
    match requested.as_deref() {
        Some("tokio") => EngineChoice {
            kind: EngineKind::Tokio,
            reason: "LOGOS_HTTP_ENGINE=tokio".to_string(),
        },
        Some("uring") => try_uring("LOGOS_HTTP_ENGINE=uring", force_fallback, probe),
        Some(other) => EngineChoice {
            kind: EngineKind::Tokio,
            reason: format!(
                "LOGOS_HTTP_ENGINE={other:?} is not recognized (expected `uring` or `tokio`)"
            ),
        },
        None => try_uring("default", force_fallback, probe),
    }
}

fn try_uring(
    origin: &str,
    force_fallback: bool,
    probe: impl FnOnce() -> Result<(), String>,
) -> EngineChoice {
    if force_fallback {
        return EngineChoice {
            kind: EngineKind::Tokio,
            reason: format!("{origin}, but LOGOS_HTTP_FORCE_FALLBACK=1 forced the tokio fallback"),
        };
    }
    match probe() {
        Ok(()) => EngineChoice {
            kind: EngineKind::Uring,
            reason: format!("{origin}, io_uring probe ring initialized"),
        },
        Err(why) => EngineChoice {
            kind: EngineKind::Tokio,
            reason: format!("{origin}, but {why}"),
        },
    }
}

/// The real probe: can a ring be set up on this box, with this build?
#[cfg(all(target_os = "linux", feature = "io-uring"))]
fn probe_uring() -> Result<(), String> {
    super::engine_uring::probe().map_err(|e| format!("io_uring probe failed: {e}"))
}

/// The engine is not compiled in (non-Linux target, or the `io-uring` feature
/// is off) — tokio serves, everywhere, with the reason on stderr.
#[cfg(not(all(target_os = "linux", feature = "io-uring")))]
fn probe_uring() -> Result<(), String> {
    Err(
        "the io_uring engine is not compiled in (requires Linux and the `io-uring` feature)"
            .to_string(),
    )
}

/// The one loud line: which engine this serve entry runs on, and why.
pub fn announce(entry: &str, choice: &EngineChoice) {
    let kind = match choice.kind {
        EngineKind::Uring => "uring",
        EngineKind::Tokio => "tokio",
    };
    eprintln!("logos-http: {entry}: engine={kind} ({})", choice.reason);
}

/// Pre-serialized static responses — the memcpy fast path.
///
/// A table from request path to the FULLY rendered wire bytes of a response,
/// in both connection dispositions. A serve entry carrying a table answers a
/// matching request by copying bytes straight into the socket, before (and
/// instead of) dispatching the handler.
///
/// Both variants are rendered by the same [`serialize_response_conn`] the
/// handler path uses, so table bytes are byte-identical to what the handler
/// path would produce for the same [`HttpResponse`] — the invariant the
/// engine differential asserts.
///
/// Matching is deliberately conservative: only `GET` requests with an empty
/// query string hit the table (anything else — `POST`, a `?query`, an unknown
/// path — falls through to the handler).
#[derive(Debug, Default, Clone)]
pub struct StaticResponses {
    entries: HashMap<String, StaticEntry>,
}

#[derive(Debug, Clone)]
struct StaticEntry {
    keep_alive: Vec<u8>,
    close: Vec<u8>,
}

impl StaticResponses {
    pub fn new() -> Self {
        Self::default()
    }

    /// Render `resp` into both wire variants and file them under `path`.
    pub fn insert(&mut self, path: &str, resp: &HttpResponse) {
        self.entries.insert(
            path.to_string(),
            StaticEntry {
                keep_alive: serialize_response_conn(resp, true),
                close: serialize_response_conn(resp, false),
            },
        );
    }

    /// The pre-rendered wire frame for `req` under the given disposition, if
    /// the table answers it.
    pub fn wire_frame(&self, req: &HttpRequest, keep_alive: bool) -> Option<&[u8]> {
        if req.method != "GET" || !req.query.is_empty() {
            return None;
        }
        self.entries.get(&req.path).map(|e| {
            if keep_alive {
                e.keep_alive.as_slice()
            } else {
                e.close.as_slice()
            }
        })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// The shard-count law for the uring engine: `LOGOS_HTTP_SHARDS` when it
/// parses to a positive count (clamped to 1024), else the box's available
/// parallelism, else 1. Pure so it is testable without touching the process
/// environment.
pub fn shard_count_from(env_shards: Option<&str>, available_parallelism: usize) -> usize {
    env_shards
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|&n| n >= 1)
        .map(|n| n.min(1024))
        .unwrap_or_else(|| available_parallelism.max(1))
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_system/http/engine/tests.rs"]
mod tests;
