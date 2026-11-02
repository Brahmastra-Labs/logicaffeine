//! A native HTTP/1.1 server — the runtime a LOGOS program's `Serve` statement
//! executes on.
//!
//! Two layers, split so the protocol is swappable:
//! - [`wire`] is a pure, dependency-free HTTP/1.1 parser + writer over plain
//!   byte buffers (hand-rolled, matching the repo's self-contained crypto
//!   ethos — no hyper): the server half (`parse_request`/`serialize_response`)
//!   and its exact client-side mirror (`serialize_request`/`parse_response`).
//! - `server` (added alongside) is a tokio accept loop generic over a handler,
//!   sitting behind an internal transport boundary so a hyper-backed transport
//!   can drop in later without touching the LOGOS handler bridge or the wire types.
//!
//! Above them, [`engine`] holds the ENGINE knob: the serve entries pick
//! between the portable tokio loop and the Linux thread-per-core io_uring
//! engine (`engine_uring`, behind the `io-uring` feature) via
//! `LOGOS_HTTP_ENGINE`, defaulting to the fast path wherever a ring
//! initializes — always announced on stderr, and byte-identical on the wire
//! (both engines speak through the same [`wire`] core).
//!
//! Native only, behind the `http` feature.

pub mod engine;
#[cfg(all(target_os = "linux", feature = "io-uring"))]
pub mod engine_uring;
pub mod lifecycle;
pub mod server;
pub mod wire;

pub use engine::{EngineChoice, EngineKind, StaticResponses};
pub use lifecycle::{
    service_unavailable, shutdown_requested, Admission, DrainOutcome, Lifecycle, LifecycleConfig,
    CORRELATION_HEADER,
};
pub use server::{
    serve, serve_with_statics, spawn, spawn_with_lifecycle, Conn, HttpServer, Listener, ReadError,
};
pub use wire::{
    parse_request, parse_response, serialize_request, serialize_response, HttpRequest,
    HttpResponse, ParseError, MAX_BODY,
};
