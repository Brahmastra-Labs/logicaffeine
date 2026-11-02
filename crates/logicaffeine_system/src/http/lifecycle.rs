//! The production server LIFECYCLE — the shared state machine that makes a
//! LOGOS `Serve` "serious and stays serving": it drains gracefully (never drops
//! an in-flight request), gates readiness separately from liveness, sheds load
//! with bounded backpressure (503, never an unbounded hang), bounds per-request
//! time, survives malformed floods, and stamps every response/log line with a
//! correlation id (secret-safe).
//!
//! Built ONCE here so BOTH serving tiers inherit the identical, proven
//! behavior: the compiled-AOT tier through [`super::server::serve`] /
//! [`super::server::spawn`] / [`super::server::spawn_with_lifecycle`], and the
//! interpreter tier (`largo serve`) by calling the SAME pure decision functions
//! ([`Lifecycle::admit`], [`Lifecycle::assign_correlation_id`],
//! [`stamp_correlation_id`]) from its own `!Send` accept loop.
//!
//! The decisions are factored as small pure/`&self` methods on [`Lifecycle`] so
//! the two loops — the tokio loop (Send, task-per-connection) and the
//! interpreter's `FuturesUnordered` loop (`!Send`, inline) — share the exact
//! same admission/backpressure/correlation-id law and can never drift.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::wire::{HttpResponse, MAX_BODY};

/// Tunables for the server lifecycle. Every field is public so an embedder can
/// dial them; [`Default`] is the deployed shape.
#[derive(Debug, Clone)]
pub struct LifecycleConfig {
    /// The most requests whose handlers may execute CONCURRENTLY. A request
    /// arriving while the slots are full is shed with `503` immediately
    /// (bounded backpressure — never queued into an unbounded hang).
    pub max_inflight: usize,
    /// The wall-clock budget for a single handler call. A handler that runs
    /// past it yields `503` and frees the slot, so a slow handler cannot wedge
    /// a connection (or a slot) forever.
    pub request_timeout: Duration,
    /// The wall-clock budget for READING one request off a connection (the
    /// slow-header / slow-body deadline). A client that connects and then stalls
    /// — sending a partial head or dribbling a body — is dropped when this
    /// elapses rather than held forever (the slowloris defense: a stalled peer
    /// can never pin a connection or a slot).
    pub read_deadline: Duration,
    /// How long a graceful [`Lifecycle::drain`] waits for in-flight requests to
    /// finish before returning and reporting the still-running count as
    /// force-closed.
    pub drain_timeout: Duration,
    /// The largest request body accepted (defends memory; passed through to the
    /// wire parser's cap).
    pub max_body: usize,
    /// The largest RESPONSE body the server will ship. A handler that produces a
    /// body past this cap is refused with a defined `500` (the giant body is
    /// never serialized onto the wire), defending server and peer memory against
    /// a handler that computes an unbounded response.
    pub max_response_body: usize,
    /// Whether every response carries an `X-Correlation-Id` (echoing the
    /// client's, else a server-assigned one). ON for the lifecycle-aware
    /// [`super::server::spawn_with_lifecycle`] entry; OFF for the plain
    /// [`super::server::serve`]/[`super::server::spawn`] entries so the
    /// cross-tier byte-for-byte differential is unperturbed (two independent
    /// processes would assign different ids — that is the ONE thing a
    /// byte-identical differential cannot tolerate).
    pub stamp_correlation_id: bool,
    /// The path that answers a READINESS probe, when one is configured.
    ///
    /// `None` — the default — reserves nothing, so a handler owns every path it
    /// declares and the cross-tier byte-for-byte differential is unperturbed. Name
    /// a path (`LOGOS_READY_PATH=/readyz`) and the server answers it BEFORE
    /// admission: `200` while ready, `503` once draining or not yet ready. Opting
    /// in is what makes a deployment's probe work; not opting in is what keeps the
    /// language from quietly owning two of your routes.
    pub ready_path: Option<String>,
    /// The path that answers a LIVENESS probe, when one is configured. `200` for as
    /// long as the process is answering at all — deliberately NOT gated on
    /// readiness, because a draining server is alive and must not be killed for
    /// shedding. `None` by default, for the same reason as [`Self::ready_path`].
    pub health_path: Option<String>,
}

impl Default for LifecycleConfig {
    fn default() -> Self {
        LifecycleConfig {
            max_inflight: 1024,
            request_timeout: Duration::from_secs(30),
            read_deadline: Duration::from_secs(30),
            drain_timeout: Duration::from_secs(25),
            max_body: MAX_BODY,
            max_response_body: MAX_BODY,
            stamp_correlation_id: true,
            ready_path: None,
            health_path: None,
        }
    }
}

impl LifecycleConfig {
    /// The config the plain [`super::server::serve`]/[`super::server::spawn`]
    /// entries use: the full lifecycle (readiness, backpressure, per-request
    /// timeout, drain) EXCEPT the correlation-id header, which is gated off so
    /// the cross-tier byte-for-byte differential stays byte-identical.
    ///
    /// The plain entries start READY (no orchestrator flips readiness), and use
    /// the historical connection bound [`super::server::MAX_INFLIGHT`] so their
    /// behavior is unchanged from before the lifecycle existed.
    pub fn serve_default() -> Self {
        LifecycleConfig {
            max_inflight: super::server::MAX_INFLIGHT,
            stamp_correlation_id: false,
            ..LifecycleConfig::default()
        }
    }

    /// [`Self::serve_default`] with any knob the environment names layered over it.
    ///
    /// This is the seam that gives EVERY tier the same lifecycle without any tier
    /// knowing about it. The compiled `Serve` statement lowers to a call that routes
    /// through the plain server entry, so sourcing the config here means a deployed
    /// binary honors these names with no change to code generation at all — exactly
    /// the trick the port override already plays. The interpreted and bytecode
    /// servers read the same function, so the three tiers cannot drift.
    ///
    /// Every name is optional and an unparseable value is ignored rather than
    /// fatal: a typo in a deployment's environment must not stop a server booting.
    pub fn from_env() -> Self {
        Self::from_env_with(|k| std::env::var(k).ok())
    }

    /// [`Self::from_env`] against an arbitrary lookup — the pure law, so it can be
    /// tested without touching the process environment.
    pub fn from_env_with(get: impl Fn(&str) -> Option<String>) -> Self {
        let mut c = Self::serve_default();
        let num = |k: &str| get(k).and_then(|v| v.trim().parse::<u64>().ok());
        if let Some(v) = num("LOGOS_MAX_INFLIGHT").filter(|v| *v > 0) {
            c.max_inflight = v as usize;
        }
        if let Some(v) = num("LOGOS_REQUEST_TIMEOUT_MS") {
            c.request_timeout = Duration::from_millis(v);
        }
        if let Some(v) = num("LOGOS_READ_DEADLINE_MS") {
            c.read_deadline = Duration::from_millis(v);
        }
        if let Some(v) = num("LOGOS_DRAIN_MS") {
            c.drain_timeout = Duration::from_millis(v);
        }
        if let Some(v) = num("LOGOS_MAX_BODY").filter(|v| *v > 0) {
            c.max_body = v as usize;
        }
        if let Some(v) = get("LOGOS_CORRELATION_ID") {
            c.stamp_correlation_id = truthy(&v);
        }
        c.ready_path = get("LOGOS_READY_PATH")
            .map(|p| p.trim().to_string())
            .filter(|p| p.starts_with('/'));
        c.health_path = get("LOGOS_HEALTH_PATH")
            .map(|p| p.trim().to_string())
            .filter(|p| p.starts_with('/'));
        c
    }
}

/// The environment's idea of yes. Anything else — including an empty value — is no,
/// so `LOGOS_CORRELATION_ID=` cannot accidentally turn stamping on.
fn truthy(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// The verdict of [`Lifecycle::admit`]: whether a request may reach the handler,
/// and if not, WHY — so both tiers answer the identical status.
pub enum Admission {
    /// The request may run; hold this slot permit until the handler returns
    /// (dropping it frees the slot and decrements the in-flight count).
    Admit(SlotGuard),
    /// The server is draining or not yet ready — shed with `503`.
    NotReady,
    /// All [`LifecycleConfig::max_inflight`] slots are busy — shed with `503`
    /// (bounded backpressure), promptly, never a queue.
    Overloaded,
}

/// A held in-flight slot: while it lives, one of the `max_inflight` slots is
/// occupied and the in-flight gauge counts it. Dropping it (handler returned,
/// panicked, or timed out) frees the slot — RAII, so no path can leak one.
pub struct SlotGuard {
    _permit: OwnedSemaphorePermit,
}

/// The outcome of a graceful [`Lifecycle::drain`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrainOutcome {
    /// In-flight requests still running when the drain deadline elapsed —
    /// force-closed. `0` means every in-flight request finished gracefully.
    pub dropped: usize,
}

/// The shared, cloneable lifecycle handle. One is created per server; the
/// accept loop consults it per connection/request and an orchestrator flips
/// readiness / requests drain through it.
#[derive(Clone)]
pub struct Lifecycle {
    inner: Arc<LifecycleState>,
}

struct LifecycleState {
    config: LifecycleConfig,
    /// Readiness gate (liveness = the loop is up; readiness is separate).
    ready: AtomicBool,
    /// Set once drain begins: the accept loop stops taking new connections and
    /// new requests are shed with `503`.
    draining: AtomicBool,
    /// The `max_inflight` concurrency budget. A permit == one in-flight handler.
    slots: Arc<Semaphore>,
    /// Pinged exactly once when drain begins — the accept loop `select!`s on it
    /// to stop. Kept SEPARATE from `slot_signal` so a slot-free wake can never
    /// be mistaken for a drain (nor a drain wake lost to a slot-free re-loop).
    drain_signal: Arc<tokio::sync::Notify>,
    /// Pinged when a slot frees, so the drain waiter re-checks the in-flight
    /// count promptly rather than polling.
    slot_signal: Arc<tokio::sync::Notify>,
    /// Monotonic id source for server-assigned correlation ids; the per-process
    /// nonce disambiguates ids across restarts/processes without a random draw
    /// on the hot path.
    corr_counter: AtomicU64,
    corr_nonce: u64,
}

/// The response header that carries the correlation id (canonical casing).
pub const CORRELATION_HEADER: &str = "X-Correlation-Id";

impl Lifecycle {
    /// Create a lifecycle from `config`. The server starts NOT ready by default
    /// — an orchestrator calls [`Lifecycle::set_ready`] once dependencies are
    /// wired — EXCEPT the plain-serve config, whose readiness the entry sets to
    /// `true` up front (there is no orchestrator on that path).
    pub fn new(config: LifecycleConfig) -> Lifecycle {
        let slots = Arc::new(Semaphore::new(config.max_inflight));
        Lifecycle {
            inner: Arc::new(LifecycleState {
                config,
                ready: AtomicBool::new(false),
                draining: AtomicBool::new(false),
                slots,
                drain_signal: Arc::new(tokio::sync::Notify::new()),
                slot_signal: Arc::new(tokio::sync::Notify::new()),
                corr_counter: AtomicU64::new(0),
                corr_nonce: process_nonce(),
            }),
        }
    }

    /// The config this lifecycle runs under (for the accept loop's timeout /
    /// body cap / correlation-id policy).
    pub fn config(&self) -> &LifecycleConfig {
        &self.inner.config
    }

    /// Flip the readiness gate. `false` → new requests are shed with `503`
    /// (load balancers stop routing); `true` → traffic is served. Independent
    /// of liveness (the loop stays up either way).
    pub fn set_ready(&self, ready: bool) {
        self.inner.ready.store(ready, Ordering::SeqCst);
    }

    /// Whether the server is currently serving traffic (ready and not draining).
    pub fn is_ready(&self) -> bool {
        self.inner.ready.load(Ordering::SeqCst) && !self.is_draining()
    }

    /// Whether a graceful drain is in progress.
    pub fn is_draining(&self) -> bool {
        self.inner.draining.load(Ordering::SeqCst)
    }

    /// THE admission decision — the one law both tiers apply per request:
    /// - draining or not ready → [`Admission::NotReady`] (`503`, no dispatch);
    /// - all slots busy → [`Admission::Overloaded`] (`503`, promptly, no queue);
    /// - otherwise → [`Admission::Admit`] holding a slot for the handler's run.
    ///
    /// `try_acquire` (never `.await`) is the backpressure keystone: admission
    /// never blocks the accept loop, so an over-capacity flood is shed, not
    /// buffered into a hang.
    pub fn admit(&self) -> Admission {
        if !self.is_ready() {
            return Admission::NotReady;
        }
        match Arc::clone(&self.inner.slots).try_acquire_owned() {
            Ok(permit) => Admission::Admit(SlotGuard { _permit: permit }),
            Err(_) => Admission::Overloaded,
        }
    }

    /// How many in-flight slots are currently occupied (a handler is running).
    pub fn inflight(&self) -> usize {
        self.inner.config.max_inflight - self.inner.slots.available_permits()
    }

    /// The correlation id for a response: the client's `X-Correlation-Id`
    /// (verbatim, when present and non-empty) else a freshly assigned one. Two
    /// concurrent no-id requests get DISTINCT ids (the counter is atomic).
    pub fn correlation_id(&self, client_supplied: Option<&str>) -> String {
        match client_supplied {
            Some(v) if !v.trim().is_empty() => v.to_string(),
            _ => self.assign_correlation_id(),
        }
    }

    /// Assign a fresh, unique correlation id (monotonic counter + per-process
    /// nonce) — cheap and non-blocking, no RNG on the hot path.
    pub fn assign_correlation_id(&self) -> String {
        let n = self.inner.corr_counter.fetch_add(1, Ordering::Relaxed);
        format!("{:016x}-{:x}", self.inner.corr_nonce, n)
    }

    /// The signal the accept loop `select!`s on so drain can stop it. Pinged
    /// exactly once by [`Lifecycle::drain`]; combined with the `is_draining`
    /// flag check in the loop, a stop can never be missed.
    pub fn drain_signal(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.inner.drain_signal)
    }

    /// Call when a slot frees (a `SlotGuard` dropped) so a pending drain waiter
    /// re-checks the in-flight count promptly rather than polling.
    pub fn notify_slot_freed(&self) {
        self.inner.slot_signal.notify_waiters();
    }

    /// Begin a GRACEFUL drain and wait up to `deadline` for in-flight requests
    /// to finish. Flips the draining flag (the accept loop stops taking new
    /// connections; new requests are shed with `503`), then awaits in-flight →
    /// zero OR the deadline, returning the still-running count at the deadline
    /// as [`DrainOutcome::dropped`]. Never hangs past `deadline`.
    pub async fn drain(&self, deadline: Duration) -> DrainOutcome {
        self.inner.draining.store(true, Ordering::SeqCst);
        // Wake the accept loop out of its `select!` so it stops accepting.
        // `notify_one` (not `notify_waiters`) STORES a permit when no waiter is
        // currently registered, so a drain edge that races the loop's brief
        // between-iterations window is caught by its next `notified()` poll — a
        // drain stop can never be lost.
        self.inner.drain_signal.notify_one();

        let start = std::time::Instant::now();
        loop {
            if self.inflight() == 0 {
                return DrainOutcome { dropped: 0 };
            }
            let remaining = match deadline.checked_sub(start.elapsed()) {
                Some(r) if !r.is_zero() => r,
                _ => {
                    return DrainOutcome {
                        dropped: self.inflight(),
                    }
                }
            };
            // Wake either when a slot frees (progress) or when the deadline
            // slice elapses (re-check). A missed notification is bounded by the
            // slice, so we never wait past the deadline.
            let slice = remaining.min(Duration::from_millis(20));
            let _ = tokio::time::timeout(slice, self.inner.slot_signal.notified()).await;
        }
    }
}

/// Stamp `resp` with the correlation-id header (canonical casing), IN PLACE.
/// Shared by both tiers so the header they emit is identical. A handler-set
/// `X-Correlation-Id` (any casing) is replaced — the lifecycle owns this header
/// exactly as the wire writer owns the framing headers.
///
/// The value is sanitized (CR/LF/NUL stripped) as defense in depth: the wire
/// parser already rejects those bytes in a parsed request header, but a
/// correlation id built by any other caller can never split the response here.
pub fn stamp_correlation_id(resp: &mut HttpResponse, id: &str) {
    let clean: String = id
        .chars()
        .filter(|&c| !matches!(c, '\r' | '\n' | '\0'))
        .collect();
    resp.headers
        .retain(|k, _| !k.eq_ignore_ascii_case(CORRELATION_HEADER));
    resp.headers.insert(CORRELATION_HEADER.to_string(), clean);
}

/// The canned `503` a request gets when the server is not ready, draining, or
/// Resolve when the process is asked to stop: `SIGTERM` or `SIGINT` on unix,
/// Ctrl-C elsewhere.
///
/// Deliberately a FUTURE the accept loop selects on rather than a spawned task.
/// Two of the three servers that need it — the tree-walker's and the bytecode
/// VM's — drive `!Send` connection futures inline on a current-thread runtime and
/// cannot spawn anything; handing them a future is what lets all three share one
/// shutdown story instead of growing three.
///
/// If the signal handlers cannot be installed (a sandbox that forbids it), this
/// never resolves, which is the right failure: the server keeps serving rather
/// than shutting down because it could not listen for a shutdown.
pub async fn shutdown_requested() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let term = signal(SignalKind::terminate());
        let int = signal(SignalKind::interrupt());
        match (term, int) {
            (Ok(mut term), Ok(mut int)) => {
                tokio::select! {
                    _ = term.recv() => {}
                    _ = int.recv() => {}
                }
            }
            _ => std::future::pending::<()>().await,
        }
    }
    #[cfg(not(unix))]
    {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

impl Lifecycle {
    /// The answer a configured probe path owes, or `None` when the path is the
    /// program's own.
    ///
    /// Called BEFORE admission, so a probe is never shed by backpressure and never
    /// consumes a slot — a monitor asking "are you alive" during an overload must
    /// get an answer, not the overload. Readiness reports whether this server wants
    /// traffic right now (`503` once draining, so a load balancer stops sending
    /// while in-flight work finishes); liveness reports only that the process is
    /// answering, and deliberately stays `200` through a drain, because a draining
    /// server is healthy and killing it would abandon the requests it is finishing.
    ///
    /// Both paths are opt-in. With neither configured this returns `None` for every
    /// path, so no route is taken away from the program and the byte-for-byte
    /// cross-tier differential is unperturbed.
    pub fn probe_response(&self, path: &str) -> Option<HttpResponse> {
        let cfg = self.config();
        if cfg.health_path.as_deref() == Some(path) {
            return Some(plain_status(200, "alive"));
        }
        if cfg.ready_path.as_deref() == Some(path) {
            return Some(if self.is_ready() {
                plain_status(200, "ready")
            } else {
                plain_status(503, "not ready")
            });
        }
        None
    }
}

/// A tiny `text/plain` response — the shape both probes answer in.
fn plain_status(status: u16, body: &str) -> HttpResponse {
    let mut headers = BTreeMap::new();
    headers.insert(
        "content-type".to_string(),
        "text/plain; charset=utf-8".to_string(),
    );
    HttpResponse {
        status,
        headers,
        body: body.as_bytes().to_vec(),
    }
}

/// over capacity — shared by both tiers (the readiness/backpressure twin of the
/// wire core's `bad_request`/`internal_error`).
pub fn service_unavailable() -> HttpResponse {
    let mut headers = std::collections::BTreeMap::new();
    headers.insert("Content-Type".to_string(), "text/plain".to_string());
    HttpResponse {
        status: 503,
        headers,
        body: b"Service Unavailable".to_vec(),
    }
}

/// A best-effort per-process nonce for correlation ids: the PID mixed with the
/// process start instant, so ids from two processes (and two restarts) almost
/// never collide, WITHOUT an RNG draw on the hot path.
fn process_nonce() -> u64 {
    let pid = std::process::id() as u64;
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    // A cheap mix so the low bits of the PID and the time both matter.
    pid.rotate_left(21) ^ t.wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// A structured, SECRET-SAFE request log line: method, path, status,
/// correlation id, and duration — and never an arbitrary request header VALUE
/// (so a secret in `Authorization`/`Cookie` can never leak into logs). Returned
/// as a string so the caller decides the sink (both tiers format identically).
pub fn access_log_line(
    method: &str,
    path: &str,
    status: u16,
    correlation_id: &str,
    dur: Duration,
) -> String {
    format!(
        "logos-http method={method} path={path} status={status} \
         correlation_id={correlation_id} dur_ms={}",
        dur.as_millis()
    )
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_system/http/lifecycle/tests.rs"]
mod tests;
