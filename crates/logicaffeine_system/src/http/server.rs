//! The tokio accept loop — the runtime under a LOGOS `Serve`.
//!
//! This module **is** the swappable transport boundary. Its public surface —
//! [`Listener`] / [`Conn`] (low-level, for the interpreter's single-threaded
//! re-entry) and [`serve`] / [`spawn`] (a convenience for a plain
//! `Fn(HttpRequest) -> HttpResponse`) — is what a hyper-backed transport would
//! re-provide identically, so swapping to hyper never touches the LOGOS handler
//! bridge, `http.lg`, codegen, or the tests.
//!
//! Two entry shapes, because the two tiers re-enter LOGOS differently:
//! - The **interpreter** is `!Send`: it drives [`Listener::accept`] +
//!   [`Conn::read_request`] / [`Conn::write_response_conn`] as owned futures on
//!   its own thread (no thread hop), calling back into the LOGOS handler
//!   between `.await` points. It never uses [`serve`]/[`spawn`].
//! - **AOT** compiles the handler to a plain `Fn(HttpRequest) -> HttpResponse`
//!   (owned library types, `Send`), so it uses [`serve`] (block) or [`spawn`].
//!
//! Connection semantics (both tiers, so the cross-tier differential stays
//! byte-identical): HTTP/1.1 **persistent connections** — requests are read in
//! a loop per connection, the shared `request_wants_keep_alive` law decides the
//! disposition, a parse error answers `400` and closes, EOF ends the
//! connection, and at most [`MAX_INFLIGHT`] connections are in flight at once.
//! A handler failure (a panic here; a runtime error on the interpreter tier)
//! answers the shared [`internal_error`] `500` for THAT request and the server
//! keeps serving.

use std::collections::BTreeMap;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::engine::{self, EngineKind, StaticResponses};
use super::lifecycle::{service_unavailable, stamp_correlation_id, Admission, Lifecycle};
// Re-exported here so the lifecycle-aware entry and its types share one path
// (`http::server::{spawn_with_lifecycle, LifecycleConfig, DrainOutcome, …}`) —
// the surface an embedder and the lifecycle lock use.
pub use super::lifecycle::{DrainOutcome, LifecycleConfig};
use super::wire::{
    parse_request_consuming, request_wants_keep_alive, serialize_response_conn, HttpRequest,
    HttpResponse, ParseError,
};

/// The engine-facing handler shape: compiled LOGOS handlers are plain `fn`s,
/// so `Fn + Send + Sync` fits the AOT tier exactly. The interpreter tier's
/// `!Send` handler never goes through this — it drives [`Listener`]/[`Conn`]
/// inline on its own thread.
type SharedHandler = dyn Fn(HttpRequest) -> HttpResponse + Send + Sync;

/// A protocol-upgrade handler: takes over a connection's raw socket once the shared loop has
/// completed the HTTP-level handshake (a `GET <path>` request carrying `Upgrade: websocket`,
/// answered `101 Switching Protocols`) — the socket, plus any bytes already read into the
/// connection's buffer past that request (almost always empty; a bodyless `GET` ends its own
/// frame at the header block, but a pipelined first WS frame is possible and must not be
/// dropped). Spawned as its own task by the loop, which then moves on to the next connection —
/// the handler owns the socket for as long as it likes.
pub type UpgradeHandler =
    Arc<dyn Fn(TcpStream, Vec<u8>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Every path this server answers as an upgrade, by its exact request path (no query string —
/// same convention as every other route here). `Arc`-shared so the loop clones it once per
/// connection task rather than once per request.
pub(super) type Upgrades = Arc<BTreeMap<String, UpgradeHandler>>;

fn no_upgrades() -> Upgrades {
    Arc::new(BTreeMap::new())
}

/// Whether `req` asks to upgrade a path this server has registered — a `GET` naming one of
/// `upgrades`' paths, with an `Upgrade: websocket` header (checked by VALUE, since only the
/// header NAME is case-normalized on parse). Returns the matched handler, cloned.
fn matching_upgrade(
    upgrades: &BTreeMap<String, UpgradeHandler>,
    req: &HttpRequest,
) -> Option<UpgradeHandler> {
    if req.method != "GET" {
        return None;
    }
    if !req
        .headers
        .get("upgrade")
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
    {
        return None;
    }
    upgrades.get(&req.path).cloned()
}

/// `Sec-WebSocket-Accept`, derived from the client's `Sec-WebSocket-Key` by RFC 6455's own
/// algorithm — `tungstenite`'s implementation of it, already in the dependency tree behind the
/// `relay` feature (the only upgrade this server has ever needed one for). Without that feature
/// no request can ever complete an upgrade handshake, so [`matching_upgrade`]'s result is moot —
/// this is what actually gates the behaviour, not a second check at the call site.
#[cfg(feature = "relay")]
fn websocket_accept_key(req: &HttpRequest) -> Option<String> {
    let key = req.headers.get("sec-websocket-key")?;
    Some(tokio_tungstenite::tungstenite::handshake::derive_accept_key(key.as_bytes()))
}
#[cfg(not(feature = "relay"))]
fn websocket_accept_key(_req: &HttpRequest) -> Option<String> {
    None
}

/// The RFC 6455 handshake decision for `req` against `upgrades`, in ONE place so every tier that
/// drives its own connection loop (the tokio `serve_connection` below; the interpreter, which is
/// `!Send` and never reaches `serve_connection` at all) reaches the exact same answer rather than
/// hand-copying the accept-key derivation and the `101` response line a second time.
///
/// - [`None`]: `req` does not ask to upgrade a registered path at all (wrong method, no
///   `Upgrade: websocket` header, or a path nothing is registered for) — proceed to the ordinary
///   request path exactly as if `upgrades` were empty.
/// - `Some(Ok((response, handler)))`: the handshake is complete — write `response` verbatim, then
///   hand the raw connection to `handler`.
/// - `Some(Err(()))`: the path/method/header matched a registered upgrade, but the handshake
///   itself could not be completed (no `Sec-WebSocket-Key`, or the `relay` feature is not compiled
///   in). The connection must be DROPPED WITH NO RESPONSE — never a fallback to the ordinary
///   handler, and never a generic error page (see [`UpgradeHandler`]'s own doc comment: this is
///   what actually gates the behaviour, not a second check at the call site).
pub fn try_upgrade(
    req: &HttpRequest,
    upgrades: &BTreeMap<String, UpgradeHandler>,
) -> Option<Result<(String, UpgradeHandler), ()>> {
    let handler = matching_upgrade(upgrades, req)?;
    match websocket_accept_key(req) {
        Some(accept) => Some(Ok((
            format!(
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
            ),
            handler,
        ))),
        None => Some(Err(())),
    }
}

/// The most connections either tier holds open at once. Accepting pauses at the
/// cap (further connections wait in the OS backlog), so a flood of held-open
/// sockets cannot exhaust memory.
pub const MAX_INFLIGHT: usize = 64;

/// The port a `Serve` statement actually binds: the `LOGOS_PORT` environment
/// variable when set (falling back to `PORT`, the Fly/Heroku/Render
/// convention), else the program's own port expression. ONE seam serving both
/// the largo full-stack dev proxy (which runs the program as a child on a
/// port it chose) and PaaS deployment — implemented here so the interpreter
/// and AOT tiers cannot drift.
pub fn env_port_override(program_port: u16) -> u16 {
    port_override(
        std::env::var("LOGOS_PORT").ok().as_deref(),
        std::env::var("PORT").ok().as_deref(),
    )
    .unwrap_or(program_port)
}

/// Announce the address a `Serve` statement bound, on stdout and at once: `Serving on http://<bound>`.
/// Every tier prints it from here, after the listener is bound and before the first request is read,
/// so whoever started the server (the full-stack dev server, a test, an orchestrator) learns the real
/// address, including the port the system chose for a requested port `0`. It is flushed because a
/// server never returns: anything left in a buffer would never be seen.
pub fn announce_serving(bound: SocketAddr) {
    crate::io::println(format_args!("Serving on http://{bound}"));
    crate::io::flush_stdout();
}

/// The whole address a `Serve` statement binds — host AND port, composed HERE
/// so no tier can build one of its own.
///
/// ★ THE HOST HAD TO BECOME AN INPUT. Both tiers used to spell
/// `format!("127.0.0.1:{}", env_port_override(port))` at their own bind site,
/// which meant a `largo build --release` binary in a container, on Fly, Render
/// or a k8s pod bound loopback and was **unreachable from outside its own
/// network namespace** — while `docs/web.md` recommended exactly that
/// deployment. Every test dialed `127.0.0.1` from the same host, so nothing
/// caught it.
///
/// ★ AND WHY IT RETURNS THE WHOLE STRING rather than just the host: two callers
/// composing "the host" with "the port" is two chances to compose it wrongly,
/// and IPv6 is where that bites (see `bind_address`). One function answers
/// the question both sites are actually asking.
/// ★ IT TAKES THE PORT IT IS GIVEN. Reading `LOGOS_PORT` in here as well would apply
/// the port override TWICE — harmless today because the environment dominates either
/// way, and exactly the kind of hidden second application that stops being harmless the
/// moment the rule gains a case. One function reads the host, one reads the port, and a
/// call site shows both: `env_bind_address(env_port_override(8080))`.
pub fn env_bind_address(port: u16) -> String {
    bind_address(std::env::var("LOGOS_HOST").ok().as_deref(), port)
}

/// The pure law behind [`env_bind_address`]: `LOGOS_HOST` names the interface,
/// defaulting to `127.0.0.1`.
///
/// The default is loopback on purpose — running a program must not turn a
/// laptop into a public server. `LOGOS_HOST=0.0.0.0` is the one line that makes
/// a container reachable, and it is a decision somebody takes rather than one
/// they inherit.
///
/// An EMPTY (or blank) value means UNSET, because that is what an unset
/// variable expands to in the shell scripts that set these
/// (`LOGOS_HOST="$HOST"` with no `HOST`) — binding the empty host instead would
/// fail with a message about an address nobody wrote.
///
/// ★ An IPv6 host is BRACKETED: `::` and a port joined by a colon is `:::8080`,
/// which parses as no address at all, so a dual-stack deployment would fail to
/// bind and blame the address rather than the missing brackets. A host the
/// author already bracketed is taken as written.
fn bind_address(host: Option<&str>, port: u16) -> String {
    let host = host
        .map(str::trim)
        .filter(|h| !h.is_empty())
        .unwrap_or("127.0.0.1");
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// The pure law behind [`env_port_override`]: `LOGOS_PORT` wins over `PORT`;
/// a value that is not a valid port is ignored (never a panic mid-serve).
fn port_override(logos_port: Option<&str>, port: Option<&str>) -> Option<u16> {
    logos_port
        .and_then(|v| v.trim().parse::<u16>().ok())
        .or_else(|| port.and_then(|v| v.trim().parse::<u16>().ok()))
}

/// Reading a request off a connection failed.
#[derive(Debug)]
pub enum ReadError {
    /// The peer closed before (or between) requests — a normal end of stream.
    Closed,
    /// Socket I/O error.
    Io(std::io::Error),
    /// The bytes received were not a valid request; the server answers `400`.
    Parse(ParseError),
}

/// A bound TCP listener. Resolve `127.0.0.1:0` to its real port via
/// [`Listener::local_addr`] before dialing.
pub struct Listener {
    inner: TcpListener,
}

/// One accepted connection: the socket plus its receive buffer, which persists
/// across requests — keep-alive drains exactly one request's frame per parse
/// and keeps the remainder (a pipelined next request) buffered, and the
/// allocation is reused for every request on the connection.
pub struct Conn {
    stream: TcpStream,
    buf: Vec<u8>,
}

impl Listener {
    /// Bind a TCP listener (e.g. `"127.0.0.1:0"` for an OS-assigned port).
    pub async fn bind(addr: &str) -> std::io::Result<Listener> {
        let inner = TcpListener::bind(addr).await?;
        Ok(Listener { inner })
    }

    /// The actual bound address (resolves an ephemeral `:0` port).
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.inner.local_addr()
    }

    /// Accept the next connection.
    pub async fn accept(&self) -> std::io::Result<Conn> {
        let (stream, _peer) = self.inner.accept().await?;
        Ok(Conn {
            stream,
            buf: Vec::with_capacity(1024),
        })
    }
}

impl Conn {
    /// Read and parse the next complete request, growing the connection buffer
    /// until the header block and `Content-Length` body have arrived (bounded by
    /// the wire core's `MAX_HEADERS`/`MAX_BODY`). Consumes exactly the request's
    /// frame from the buffer; pipelined bytes stay for the next call.
    pub async fn read_request(&mut self) -> Result<HttpRequest, ReadError> {
        let mut chunk = [0u8; 8192];
        loop {
            match parse_request_consuming(&self.buf) {
                Ok((req, used)) => {
                    self.buf.drain(..used);
                    return Ok(req);
                }
                Err(ParseError::Incomplete) => {} // need more bytes
                Err(e) => return Err(ReadError::Parse(e)),
            }
            let n = self.stream.read(&mut chunk).await.map_err(ReadError::Io)?;
            if n == 0 {
                // EOF: a clean close if nothing buffered, else a truncated request.
                return if self.buf.is_empty() {
                    Err(ReadError::Closed)
                } else {
                    Err(ReadError::Parse(ParseError::Incomplete))
                };
            }
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }

    /// [`Conn::read_request`], but with a per-server `max_body` cap that is
    /// TIGHTER than the wire core's global [`super::wire::MAX_BODY`]: the instant
    /// the header block has arrived, the declared `Content-Length` is checked
    /// against `max_body` and an over-limit request is rejected with
    /// [`ParseError::BodyTooLarge`] BEFORE the promised body bytes are ever
    /// buffered — a client cannot make the server hold megabytes for a body it
    /// declared past the cap. Everything else is the identical parse.
    pub async fn read_request_capped(&mut self, max_body: usize) -> Result<HttpRequest, ReadError> {
        let mut chunk = [0u8; 8192];
        loop {
            // Once the header terminator is in the buffer, enforce the config
            // body cap on the DECLARED length before waiting for the body.
            if let Some(cl) = declared_content_length(&self.buf) {
                if cl > max_body {
                    return Err(ReadError::Parse(ParseError::BodyTooLarge));
                }
            }
            match parse_request_consuming(&self.buf) {
                Ok((req, used)) => {
                    self.buf.drain(..used);
                    return Ok(req);
                }
                Err(ParseError::Incomplete) => {} // need more bytes
                Err(e) => return Err(ReadError::Parse(e)),
            }
            let n = self.stream.read(&mut chunk).await.map_err(ReadError::Io)?;
            if n == 0 {
                return if self.buf.is_empty() {
                    Err(ReadError::Closed)
                } else {
                    Err(ReadError::Parse(ParseError::Incomplete))
                };
            }
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }

    /// Serialize and write a one-shot (`Connection: close`) response, then flush.
    pub async fn write_response(&mut self, resp: &HttpResponse) -> std::io::Result<()> {
        self.write_response_conn(resp, false).await
    }

    /// Serialize and write a response with an explicit connection disposition
    /// (`Connection: keep-alive` holds the connection for the next
    /// [`Conn::read_request`]; `close` ends it), then flush.
    pub async fn write_response_conn(
        &mut self,
        resp: &HttpResponse,
        keep_alive: bool,
    ) -> std::io::Result<()> {
        let bytes = serialize_response_conn(resp, keep_alive);
        self.write_bytes(&bytes).await
    }

    /// Write an already-rendered wire frame (the static-table fast path),
    /// then flush.
    async fn write_bytes(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.stream.write_all(bytes).await?;
        self.stream.flush().await
    }

    /// Hand off this connection's raw socket, with any bytes already read into the connection
    /// buffer past the request the loop just parsed — for a protocol upgrade that takes over
    /// after an HTTP request this server already answered (see [`UpgradeHandler`]). Consumes the
    /// connection; nothing else may read from or write to it afterward. Private: [`Conn::upgrade`]
    /// is its one caller.
    fn into_raw_parts(self) -> (TcpStream, Vec<u8>) {
        (self.stream, self.buf)
    }

    /// Complete an upgrade [`try_upgrade`] accepted: write its `101` response, then hand the socket
    /// — with whatever was read past the request — to its handler, spawned as its own task. The
    /// one hand-off every connection loop makes, the tokio engine's and the interpreter's alike.
    pub async fn upgrade(mut self, response: String, handler: UpgradeHandler) {
        if self.write_bytes(response.as_bytes()).await.is_ok() {
            let (stream, prefix) = self.into_raw_parts();
            tokio::spawn(async move { (handler)(stream, prefix).await });
        }
    }
}

/// Peek the declared `Content-Length` (in bytes) from a raw request buffer,
/// but only once the full header block (`\r\n\r\n`) has arrived — before that
/// the value is not yet trustworthy (a header could still be in flight).
/// Returns `None` when the header block is incomplete, absent, or the value
/// does not parse; the caller then falls through to the ordinary parse (which
/// answers `400` for a genuinely malformed length). Used by
/// [`Conn::read_request_capped`] to enforce a config body cap on the DECLARED
/// length before buffering the body.
fn declared_content_length(buf: &[u8]) -> Option<usize> {
    let end = buf.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&buf[..end]).ok()?;
    for line in head.split("\r\n") {
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                return value.trim().parse::<usize>().ok();
            }
        }
    }
    None
}

/// A page document composed for one visitor of a server that answers CSRF tokens: `page` with the
/// visitor's token in its head (`token` is the server's answer at its reserved token path), that
/// answer's `Set-Cookie` set on it, and the whole never stored by a cache — the document is this
/// visitor's alone. `None` when `token` is no token (a server with no `## Session` does not serve
/// the path), and the page goes out as it was.
pub fn page_with_visitor_token(page: &HttpResponse, token: &HttpResponse) -> Option<HttpResponse> {
    if token.status != 200 {
        return None;
    }
    let token_text = std::str::from_utf8(&token.body)
        .ok()
        .filter(|t| !t.is_empty())?;
    let document = std::str::from_utf8(&page.body).ok()?;
    let validator = |name: &str| {
        [
            "etag",
            "cache-control",
            "vary",
            "last-modified",
            "content-length",
        ]
        .iter()
        .any(|v| name.eq_ignore_ascii_case(v))
    };
    let mut headers: BTreeMap<String, String> = page
        .headers
        .iter()
        .filter(|(name, _)| !validator(name))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    headers.insert("Cache-Control".to_string(), "no-store".to_string());
    if let Some((_, cookie)) = token
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("set-cookie"))
    {
        headers.insert("Set-Cookie".to_string(), cookie.clone());
    }
    Some(HttpResponse {
        status: page.status,
        headers,
        body: crate::csrf::with_csrf_meta(document, token_text).into_bytes(),
    })
}

/// The canned `413` a request gets when its declared body or header block is
/// larger than the server's cap — a DEFINED "too large" refusal (distinct from
/// the malformed `400`) so a client can tell the two apart. Shared by both
/// tiers. `text/plain`, body `Payload Too Large`.
pub fn payload_too_large() -> HttpResponse {
    let mut headers = BTreeMap::new();
    headers.insert("Content-Type".to_string(), "text/plain".to_string());
    HttpResponse {
        status: 413,
        headers,
        body: b"Payload Too Large".to_vec(),
    }
}

/// Map a request read error to the response the server writes: an oversized
/// BODY (a declared `Content-Length` past the cap) is a defined `413` (never
/// buffered / never dispatched) so a client can tell "too big" from
/// "malformed". Everything else — including an oversized HEADER block, whose
/// head the server cannot even parse — is the malformed `400` (the shape the
/// cross-tier `http_differential` lock pins). THE shared read-error law both
/// the tokio and io_uring engines apply so their refusals are identical.
pub fn read_error_response(e: &ParseError) -> HttpResponse {
    match e {
        ParseError::BodyTooLarge => payload_too_large(),
        _ => bad_request(),
    }
}

/// The canned `400` a connection gets when its bytes don't parse — shared by
/// both tiers so the differential stays byte-identical.
pub fn bad_request() -> HttpResponse {
    HttpResponse {
        status: 400,
        headers: BTreeMap::new(),
        body: b"Bad Request".to_vec(),
    }
}

/// The canned `500` a request gets when its handler fails (a runtime error on
/// the interpreter tier, a panic on the AOT tier) — shared by both tiers. The
/// failure detail is deliberately NOT on the wire (it goes to stderr): status
/// 500, `text/plain`, body `internal error`.
pub fn internal_error() -> HttpResponse {
    let mut headers = BTreeMap::new();
    headers.insert("Content-Type".to_string(), "text/plain".to_string());
    HttpResponse {
        status: 500,
        headers,
        body: b"internal error".to_vec(),
    }
}

/// Serve on an already-bound [`Listener`], looping forever — the TOKIO engine.
/// Connections run CONCURRENTLY (one task each, capped at [`MAX_INFLIGHT`])
/// with HTTP/1.1 keep-alive, so a stalled or held-open connection never blocks
/// the rest; handler calls are serialized on a mutex (one at a time, matching
/// the single-threaded interpreter tier). A handler panic answers the shared
/// [`internal_error`] `500` for that request and the server keeps serving. A
/// parse error answers `400` and closes; a closed/errored socket just ends
/// that connection.
///
/// A pre-bound tokio listener always serves on the tokio engine (the io_uring
/// engine binds its own `SO_REUSEPORT` shard listeners); engine selection
/// lives in [`serve`]/[`serve_with_statics`]. If `LOGOS_HTTP_ENGINE=uring` is
/// set anyway, this says so on stderr rather than silently ignoring the knob.
pub async fn serve_on<F>(listener: Listener, handler: F) -> std::io::Result<()>
where
    F: FnMut(HttpRequest) -> HttpResponse + Send + 'static,
{
    serve_on_with(listener, handler, Vec::new()).await
}

/// [`serve_on`] with a table of protocol-upgrade paths (see [`UpgradeHandler`]) — a `GET` to one
/// of them, asking to upgrade, is answered `101` and handed off instead of reaching `handler`.
/// Every other request is unaffected, so `serve_on(l, h)` is exactly `serve_on_with(l, h, [])`.
pub async fn serve_on_with<F>(
    listener: Listener,
    handler: F,
    upgrades: Vec<(&str, UpgradeHandler)>,
) -> std::io::Result<()>
where
    F: FnMut(HttpRequest) -> HttpResponse + Send + 'static,
{
    let asked_for_uring = std::env::var("LOGOS_HTTP_ENGINE")
        .map(|v| v.trim().eq_ignore_ascii_case("uring"))
        .unwrap_or(false);
    if asked_for_uring {
        eprintln!(
            "logos-http: serve_on: engine=tokio (LOGOS_HTTP_ENGINE=uring, but a pre-bound \
             listener always serves on the tokio engine — use serve() for the io_uring path)"
        );
    }
    let upgrades: Upgrades = Arc::new(
        upgrades
            .into_iter()
            .map(|(p, h)| (p.to_string(), h))
            .collect(),
    );
    serve_loop(
        listener,
        serialize_handler(handler),
        None,
        ready_serve_lifecycle(),
        upgrades,
    )
    .await
}

/// [`serve_on`] with a SHARED handler (`Fn + Send + Sync`) — calls run CONCURRENTLY across
/// connection tasks instead of queueing on one mutex.
///
/// ★ WHY BOTH EXIST. [`serve_on`] serializes deliberately: a LOGOS handler on the interpreter tier
/// is `FnMut` and single-threaded, and running two turns at once there would not be the same
/// program. A handler that is already `Sync` has no such constraint, and serializing it anyway
/// makes every SLOW request a stop-the-world event.
///
/// That is not hypothetical — it is what the dev server did. `largo serve` answers its own static
/// assets, its pages and the `/__gen` reload poll from the same handler that PROXIES to the backend
/// with a multi-second timeout, so one hung route stalled the CSS, the wasm and the reload poll
/// behind it: the page could not even refresh to tell you something was wrong.
pub async fn serve_on_shared<F>(listener: Listener, handler: F) -> std::io::Result<()>
where
    F: Fn(HttpRequest) -> HttpResponse + Send + Sync + 'static,
{
    serve_on_shared_with(listener, handler, Vec::new()).await
}

/// [`serve_on_shared`] with a table of protocol-upgrade paths (see [`UpgradeHandler`]) — a `GET`
/// to one of them, asking to upgrade, is answered `101` and handed off instead of reaching
/// `handler`. Every other request goes through the ordinary request/response path unchanged, so
/// `serve_on_shared(l, h)` is exactly `serve_on_shared_with(l, h, [])`.
pub async fn serve_on_shared_with<F>(
    listener: Listener,
    handler: F,
    upgrades: Vec<(&str, UpgradeHandler)>,
) -> std::io::Result<()>
where
    F: Fn(HttpRequest) -> HttpResponse + Send + Sync + 'static,
{
    let asked_for_uring = std::env::var("LOGOS_HTTP_ENGINE")
        .map(|v| v.trim().eq_ignore_ascii_case("uring"))
        .unwrap_or(false);
    if asked_for_uring {
        eprintln!(
            "logos-http: serve_on_shared: engine=tokio (LOGOS_HTTP_ENGINE=uring, but a pre-bound \
             listener always serves on the tokio engine — use serve() for the io_uring path)"
        );
    }
    let upgrades: Upgrades = Arc::new(
        upgrades
            .into_iter()
            .map(|(p, h)| (p.to_string(), h))
            .collect(),
    );
    serve_loop(
        listener,
        Arc::new(handler),
        None,
        ready_serve_lifecycle(),
        upgrades,
    )
    .await
}

/// The lifecycle the plain (non-orchestrated) serve entries run under: the full
/// production lifecycle (readiness, backpressure, timeout, drain) but with the
/// correlation-id header OFF by default (so the cross-tier byte differential is
/// unmoved), started READY (there is no orchestrator on this path to flip it).
///
/// The config comes from [`LifecycleConfig::from_env`], which is the seam that
/// gives every tier the same lifecycle without any tier knowing about it: the
/// compiled `Serve` statement lowers to a call that reaches this function, so a
/// deployed binary honors the lifecycle environment names with no change to code
/// generation at all — exactly what `env_port_override` already does for the port.
fn ready_serve_lifecycle() -> Lifecycle {
    let lifecycle = Lifecycle::new(LifecycleConfig::from_env());
    lifecycle.set_ready(true);
    lifecycle
}

/// Wrap an exclusive (`FnMut`) handler into the shared engine contract: calls
/// are serialized on a mutex — one at a time, matching the single-threaded
/// interpreter tier. A panic inside a call unwinds through the guard (the
/// engines' [`crate::failure::caught`] turns it into the shared `500`); `into_inner` clears
/// the resulting poison on the next request, so one panic never wedges the
/// handler.
fn serialize_handler<F>(handler: F) -> Arc<SharedHandler>
where
    F: FnMut(HttpRequest) -> HttpResponse + Send + 'static,
{
    let handler = Mutex::new(handler);
    Arc::new(move |req| {
        let mut h = handler.lock().unwrap_or_else(|p| p.into_inner());
        (*h)(req)
    })
}

/// The tokio accept loop shared by every tokio-engine entry — now
/// LIFECYCLE-AWARE: per-connection tasks, keep-alive, the static-table fast
/// path, panic-to-500, 400-and-close, PLUS readiness gating (`503` when not
/// ready), bounded backpressure (`503` promptly when over `max_inflight` —
/// never a hang), a per-request time bound (`503` past `request_timeout`, slot
/// freed), a correlation-id stamp (when the config enables it), and graceful
/// drain (the loop `select!`s on the drain signal so drain actually stops it).
///
/// The lifecycle decisions come from the shared [`Lifecycle`] handle so the
/// interpreter tier applies the identical law.
async fn serve_loop(
    listener: Listener,
    handler: Arc<SharedHandler>,
    statics: Option<Arc<StaticResponses>>,
    lifecycle: Lifecycle,
    upgrades: Upgrades,
) -> std::io::Result<()> {
    let drain = lifecycle.drain_signal();
    loop {
        // Defense in depth: if drain already began, stop before blocking in
        // accept — closes the between-iterations window even if the edge below
        // is missed (belt to the `notify_one` permit's suspenders).
        if lifecycle.is_draining() {
            break Ok(());
        }
        // Stop accepting the instant drain begins — otherwise wait for the next
        // connection. `select!` races accept against the DEDICATED drain signal
        // (pinged only by `drain`, so no spurious wakes) so a graceful drain
        // actually halts the loop rather than being ignored.
        let conn = tokio::select! {
            biased;
            _ = drain.notified() => break Ok(()),
            // A stop request is a GRACEFUL one: stop advertising readiness so a
            // load balancer moves traffic away, then let the in-flight work finish
            // inside the drain deadline before the loop ends.
            _ = super::lifecycle::shutdown_requested() => {
                lifecycle.set_ready(false);
                let deadline = lifecycle.config().drain_timeout;
                lifecycle.drain(deadline).await;
                break Ok(());
            }
            accepted = listener.accept() => match accepted {
                Ok(c) => c,
                // A transient accept error (e.g. fd exhaustion under a flood)
                // ends THIS iteration, never the loop — the server stays up.
                Err(_) => continue,
            },
        };
        if lifecycle.is_draining() {
            // A connection that slipped in as drain flipped: refuse it cleanly.
            drop(conn);
            continue;
        }
        let handler = Arc::clone(&handler);
        let statics = statics.clone();
        let lifecycle = lifecycle.clone();
        let upgrades = Arc::clone(&upgrades);
        tokio::spawn(async move {
            serve_connection(conn, handler, statics, lifecycle, upgrades).await;
        });
    }
}

/// One connection's request loop: keep-alive, the static fast path, and the
/// per-request lifecycle (admission, timeout, correlation-id). Extracted so the
/// accept loop reads as pure orchestration.
async fn serve_connection(
    mut conn: Conn,
    handler: Arc<SharedHandler>,
    statics: Option<Arc<StaticResponses>>,
    lifecycle: Lifecycle,
    upgrades: Upgrades,
) {
    let max_body = lifecycle.config().max_body;
    let read_deadline = lifecycle.config().read_deadline;
    loop {
        // Read the next request under the slow-header / slow-body deadline: a
        // client that connects then stalls (a partial head, or a dribbled body)
        // is DROPPED when `read_deadline` elapses rather than pinning this
        // connection forever. A completed read proceeds as before.
        let read =
            match tokio::time::timeout(read_deadline, conn.read_request_capped(max_body)).await {
                Ok(r) => r,
                // Deadline: the peer stalled mid-request. Drop the connection (no
                // response — a slowloris gets nothing) so the slot/fd is reclaimed.
                Err(_) => break,
            };
        match read {
            Ok(req) => {
                // ★ AN UPGRADE ENDS THE HTTP LOOP FOR THIS CONNECTION, NOT THE ANSWER TO IT. Once
                // the handshake response is written, the socket belongs to `upgrade` alone — going
                // on to `handle_request`, or even reading another request off `conn`, would race
                // the upgrade handler for the same bytes. Checked before the static fast path and
                // readiness/backpressure: a registered path is answered on its own terms, not
                // shed under load the way an ordinary GET would be (a dropped realtime channel is
                // worse than a slow one). The decision itself lives in [`try_upgrade`], shared with
                // every other tier that drives its own connection loop.
                match try_upgrade(&req, &upgrades) {
                    Some(Ok((response, upgrade))) => {
                        conn.upgrade(response, upgrade).await;
                        return;
                    }
                    Some(Err(())) => return,
                    None => {}
                }
                let keep_alive = request_wants_keep_alive(&req);
                // The memcpy fast path: a pre-serialized frame answers before
                // the handler (or the lifecycle) is ever consulted. Static
                // assets are always-available content — not gated by readiness.
                if !lifecycle.config().stamp_correlation_id {
                    if let Some(frame) = statics
                        .as_ref()
                        .and_then(|s| s.wire_frame(&req, keep_alive))
                    {
                        if conn.write_bytes(frame).await.is_err() || !keep_alive {
                            break;
                        }
                        continue;
                    }
                }
                let (resp, keep) =
                    handle_request(&handler, &statics, &lifecycle, req, keep_alive).await;
                if conn.write_response_conn(&resp, keep).await.is_err() || !keep {
                    break;
                }
            }
            Err(ReadError::Parse(e)) => {
                // An oversized body/header block is a defined 413; any other
                // parse failure is the malformed 400. Either way the connection
                // closes (the peer sent something it cannot recover from).
                let _ = conn
                    .write_response_conn(&read_error_response(&e), false)
                    .await;
                break;
            }
            Err(ReadError::Closed) | Err(ReadError::Io(_)) => break,
        }
    }
    // `conn` drops here → socket closes.
}

/// Apply the per-request lifecycle to one parsed request and produce the
/// response to write (and the connection disposition). THE shared decision path
/// both the connection loop here and the interpreter loop mirror: readiness /
/// backpressure shed to `503`, a static frame when the correlation-id policy is
/// off already handled the fast path, dispatch under a `request_timeout` with
/// the handler on the blocking pool (so a synchronous handler can't starve the
/// reactor and the timeout can actually fire), panic → `500`, and — when the
/// config enables it — the correlation-id stamp.
async fn handle_request(
    handler: &Arc<SharedHandler>,
    _statics: &Option<Arc<StaticResponses>>,
    lifecycle: &Lifecycle,
    req: HttpRequest,
    keep_alive: bool,
) -> (HttpResponse, bool) {
    let started = std::time::Instant::now();
    let client_cid = req.headers.get("x-correlation-id").cloned();
    let stamp = lifecycle.config().stamp_correlation_id;
    // Captured before `req` moves into the handler, so the access log carries
    // method/path WITHOUT ever retaining an arbitrary request header VALUE.
    let method = req.method.clone();
    let path = req.path.clone();

    // A configured probe path answers BEFORE admission, so a monitor asking
    // whether this process is alive gets an answer during an overload rather than
    // the overload itself. Nothing is reserved unless a path was named, so by
    // default this never fires and the handler owns every route.
    if let Some(mut resp) = lifecycle.probe_response(&path) {
        let cid = lifecycle.correlation_id(client_cid.as_deref());
        if stamp {
            stamp_correlation_id(&mut resp, &cid);
        }
        access_log(lifecycle, &method, &path, resp.status, &cid, started);
        return (resp, keep_alive);
    }

    // Admission: readiness / drain / backpressure. A shed request never reaches
    // the handler and never occupies a slot beyond the try_acquire.
    let guard = match lifecycle.admit() {
        Admission::Admit(g) => g,
        Admission::NotReady | Admission::Overloaded => {
            let mut resp = service_unavailable();
            let cid = lifecycle.correlation_id(client_cid.as_deref());
            if stamp {
                stamp_correlation_id(&mut resp, &cid);
            }
            access_log(lifecycle, &method, &path, resp.status, &cid, started);
            // A shed request always closes: no keep-alive on a `503` (the peer
            // should back off, and the connection carried no slot).
            return (resp, false);
        }
    };

    // The correlation id is decided up front so it is stable across the timeout
    // and the assigned-id path (a no-id request must still get an id even on a
    // timeout/panic response). It is always computed (for the access log), but
    // only stamped on the wire when the config enables it.
    let cid = lifecycle.correlation_id(client_cid.as_deref());

    let h = Arc::clone(handler);
    let timeout = lifecycle.config().request_timeout;
    // Run the (possibly blocking) handler on the blocking pool so the timeout
    // can fire even if the handler parks synchronously — a synchronous handler
    // can never starve the reactor, and a slow one can never wedge a slot.
    let call = tokio::task::spawn_blocking(move || {
        crate::failure::caught(std::panic::AssertUnwindSafe(|| h(req)))
    });
    let out = tokio::time::timeout(timeout, call).await;
    // The slot frees here regardless of outcome; a timed-out handler keeps
    // running detached but no longer holds a slot (bounded, never wedged).
    drop(guard);
    lifecycle.notify_slot_freed();
    let mut resp = match out {
        Ok(Ok(Ok(resp))) => resp,           // handler returned
        Ok(Ok(Err(_))) => internal_error(), // handler panicked → 500
        Ok(Err(_)) => internal_error(),     // JoinError (blocking task failed)
        Err(_) => service_unavailable(),    // deadline elapsed → 503, slot freed
    };

    // Response-body cap: a handler that produced a body past the server's
    // `max_response_body` is refused with a defined `500` — the monstrous body
    // is never serialized onto the wire, defending server and peer memory.
    if resp.body.len() > lifecycle.config().max_response_body {
        resp = internal_error();
    }

    if stamp {
        stamp_correlation_id(&mut resp, &cid);
    }
    access_log(lifecycle, &method, &path, resp.status, &cid, started);
    (resp, keep_alive)
}

/// Emit the structured, secret-safe access line for a request — opt-in via
/// `LOGOS_HTTP_ACCESS_LOG` (any value) so it never spams the default path or
/// the byte-differential (the line is stderr, never the wire). The line carries
/// method/path/status/correlation-id/duration and STRUCTURALLY never an
/// arbitrary request header value.
fn access_log(
    lifecycle: &Lifecycle,
    method: &str,
    path: &str,
    status: u16,
    cid: &str,
    started: std::time::Instant,
) {
    let _ = lifecycle;
    if std::env::var_os("LOGOS_HTTP_ACCESS_LOG").is_some() {
        eprintln!(
            "{}",
            super::lifecycle::access_log_line(method, path, status, cid, started.elapsed())
        );
    }
}

/// Bind `addr` and serve forever — THE AOT entry, and the engine-selection
/// seam: `LOGOS_HTTP_ENGINE=uring|tokio` picks the engine, defaulting to the
/// Linux thread-per-core io_uring engine wherever a probe ring initializes
/// (else tokio), always announced with one stderr line. Handler calls are
/// serialized either way (the `FnMut` contract); [`serve_with_statics`] is the
/// shared-handler entry that also carries the static-response fast path.
pub async fn serve<F>(addr: &str, handler: F) -> std::io::Result<()>
where
    F: FnMut(HttpRequest) -> HttpResponse + Send + 'static,
{
    serve_shared(addr, serialize_handler(handler), None, no_upgrades()).await
}

/// [`serve`] with a table of protocol-upgrade paths (see [`UpgradeHandler`]) — a `GET` to one of
/// them carrying `Upgrade: websocket` is handed to its handler once the handshake is answered, on
/// either engine. A compiled backend whose pages talk to the relay registers `/_relay` here
/// (`relay::RelayHub::upgrades`).
pub async fn serve_with_upgrades<F>(
    addr: &str,
    handler: F,
    upgrades: Vec<(&str, UpgradeHandler)>,
) -> std::io::Result<()>
where
    F: FnMut(HttpRequest) -> HttpResponse + Send + 'static,
{
    let upgrades: Upgrades = Arc::new(
        upgrades
            .into_iter()
            .map(|(p, h)| (p.to_string(), h))
            .collect(),
    );
    serve_shared(addr, serialize_handler(handler), None, upgrades).await
}

/// [`serve`] for a handler that cannot leave the thread it was made on — one that holds the
/// program's own bindings, whose collections are not `Send`. The handler runs on the thread that
/// awaits this future, one request at a time, exactly as [`serve`] serializes its calls; the engine
/// runs on a thread and runtime of its own and hands each request across, so the caller's runtime
/// may be of any flavor. A panic inside a call answers the engines' shared `500`.
pub async fn serve_here<F>(addr: &str, handler: F) -> std::io::Result<()>
where
    F: FnMut(HttpRequest) -> HttpResponse,
{
    serve_here_with(addr.to_string(), handler, no_upgrades()).await
}

/// [`serve_here`] with a table of protocol-upgrade paths, as [`serve_with_upgrades`].
pub async fn serve_here_with_upgrades<F>(
    addr: &str,
    handler: F,
    upgrades: Vec<(&str, UpgradeHandler)>,
) -> std::io::Result<()>
where
    F: FnMut(HttpRequest) -> HttpResponse,
{
    let upgrades: Upgrades = Arc::new(
        upgrades
            .into_iter()
            .map(|(p, h)| (p.to_string(), h))
            .collect(),
    );
    serve_here_with(addr.to_string(), handler, upgrades).await
}

async fn serve_here_with<F>(addr: String, mut handler: F, upgrades: Upgrades) -> std::io::Result<()>
where
    F: FnMut(HttpRequest) -> HttpResponse,
{
    type Reply = std::sync::mpsc::SyncSender<HttpResponse>;
    let (requests, mut inbox) = tokio::sync::mpsc::unbounded_channel::<(HttpRequest, Reply)>();
    let forward: Arc<SharedHandler> = Arc::new(move |req| {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        if requests.send((req, reply)).is_err() {
            return internal_error();
        }
        answer.recv().unwrap_or_else(|_| internal_error())
    });
    let (finished, outcome) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("logos-http".to_string())
        .spawn(move || {
            let served = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .and_then(|engine| engine.block_on(serve_shared(&addr, forward, None, upgrades)));
            let _ = finished.send(served);
        })?;
    // The engine thread holds the only sender, so the inbox closes when the engine stops serving.
    while let Some((req, reply)) = inbox.recv().await {
        let answer = crate::failure::caught(std::panic::AssertUnwindSafe(|| handler(req)))
            .unwrap_or_else(|_| internal_error());
        let _ = reply.send(answer);
    }
    outcome.await.unwrap_or_else(|_| {
        Err(std::io::Error::other(
            "the server thread ended without an outcome",
        ))
    })
}

/// Bind `addr` and serve forever with a SHARED handler (`Fn + Send + Sync` —
/// the compiled-handler contract; calls may run concurrently across shards or
/// connection tasks) and an optional pre-serialized [`StaticResponses`] table
/// both engines consult before dispatching. Engine selection as in [`serve`].
pub async fn serve_with_statics<F>(
    addr: &str,
    handler: F,
    statics: Option<Arc<StaticResponses>>,
) -> std::io::Result<()>
where
    F: Fn(HttpRequest) -> HttpResponse + Send + Sync + 'static,
{
    serve_shared(addr, Arc::new(handler), statics, no_upgrades()).await
}

/// The one selection seam behind [`serve`]/[`serve_with_statics`]/[`serve_with_upgrades`]:
/// choose, announce, run — and if the chosen io_uring engine fails to BIND (distinct from the
/// probe), fall back to tokio with a second loud line, never a silent downgrade.
async fn serve_shared(
    addr: &str,
    handler: Arc<SharedHandler>,
    statics: Option<Arc<StaticResponses>>,
    upgrades: Upgrades,
) -> std::io::Result<()> {
    let choice = engine::choose_engine();
    engine::announce("serve", &choice);
    if matches!(choice.kind, EngineKind::Uring) {
        #[cfg(all(target_os = "linux", feature = "io-uring"))]
        {
            match super::engine_uring::UringServer::bind_with_upgrades(
                addr,
                super::engine_uring::EngineConfig::from_env(),
                Arc::clone(&handler),
                statics.clone(),
                Arc::clone(&upgrades),
            ) {
                Ok(server) => {
                    announce_serving(server.local_addr());
                    // Serving never returns; the engine lives as long as this
                    // future is polled (i.e. forever).
                    let _server = server;
                    std::future::pending::<()>().await;
                    unreachable!("a pending future never resolves");
                }
                Err(e) => {
                    eprintln!(
                        "logos-http: serve: engine=tokio (io_uring engine failed to bind: {e})"
                    );
                }
            }
        }
        #[cfg(not(all(target_os = "linux", feature = "io-uring")))]
        unreachable!("choose_engine only selects uring where the engine is compiled in");
    }
    let listener = Listener::bind(addr).await?;
    announce_serving(listener.local_addr()?);
    serve_loop(
        listener,
        handler,
        statics,
        ready_serve_lifecycle(),
        upgrades,
    )
    .await
}

/// A running server spawned onto the tokio runtime. Dropping the handle aborts
/// the accept loop (mirrors `relay::RelayServer`). The handler must be `Send`;
/// the interpreter's `!Send` handler uses [`Listener`]/[`Conn`] directly instead.
///
/// The handle also carries the [`Lifecycle`], so an embedder can flip readiness
/// ([`HttpServer::set_ready`]) and drain gracefully ([`HttpServer::drain`])
/// exactly as an orchestrator's SIGTERM handler would.
pub struct HttpServer {
    addr: SocketAddr,
    task: tokio::task::JoinHandle<()>,
    lifecycle: Lifecycle,
}

impl HttpServer {
    /// The bound address (resolve `:0` to its real port for clients).
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
    /// The `http://…` base URL a client dials.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Flip the readiness gate: `false` sheds new requests with `503` (a load
    /// balancer stops routing), `true` serves them — independent of liveness
    /// (the accept loop stays up either way).
    pub fn set_ready(&self, ready: bool) {
        self.lifecycle.set_ready(ready);
    }

    /// Whether the server is currently serving (ready and not draining).
    pub fn is_ready(&self) -> bool {
        self.lifecycle.is_ready()
    }

    /// Gracefully drain: stop accepting new connections (new requests shed with
    /// `503`), wait up to `deadline` for in-flight requests to finish, and
    /// return the still-running count as [`DrainOutcome::dropped`] (`0` = every
    /// in-flight request completed). Never hangs past `deadline`.
    pub async fn drain(&self, deadline: Duration) -> DrainOutcome {
        self.lifecycle.drain(deadline).await
    }

    /// The shared lifecycle handle (for advanced embedders).
    pub fn lifecycle(&self) -> &Lifecycle {
        &self.lifecycle
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Bind `addr`, spawn the accept loop onto the tokio runtime, and return a handle
/// carrying the bound port. For tests and embedders (the handler is `Send`).
/// Always the TOKIO engine: the handle's drop-aborts contract needs a task,
/// not OS threads — engine selection belongs to the blocking [`serve`] entry
/// (the io_uring engine's own handle is `engine_uring::UringServer`).
pub async fn spawn<F>(addr: &str, handler: F) -> std::io::Result<HttpServer>
where
    F: FnMut(HttpRequest) -> HttpResponse + Send + 'static,
{
    let listener = Listener::bind(addr).await?;
    let addr = listener.local_addr()?;
    let handler = serialize_handler(handler);
    // The plain spawn keeps the historical wire shape (no correlation-id header,
    // started ready) so existing tests and the cross-tier differential are
    // unperturbed — the full lifecycle (backpressure/timeout/drain) still runs.
    let lifecycle = ready_serve_lifecycle();
    let loop_lifecycle = lifecycle.clone();
    let task = tokio::spawn(async move {
        let _ = serve_loop(listener, handler, None, loop_lifecycle, no_upgrades()).await;
    });
    Ok(HttpServer {
        addr,
        task,
        lifecycle,
    })
}

/// Bind `addr`, spawn the LIFECYCLE-AWARE accept loop, and return a handle that
/// exposes the full production lifecycle: [`HttpServer::set_ready`],
/// [`HttpServer::drain`], and — because `config.stamp_correlation_id` defaults
/// on — an `X-Correlation-Id` on every response. THE entry an embedder (and the
/// AOT `Serve` lowering, via a config seam) uses for a server that "stays
/// serving": graceful drain, readiness gating, bounded backpressure, per-request
/// time bounds, malformed-flood survival, and correlation ids.
///
/// The server starts READY (accepting traffic the moment it is bound, the
/// deployed default); an orchestrator that wants a warm-up window flips it with
/// `set_ready(false)` up front and back on once dependencies are wired.
///
/// Always the tokio engine: the drain/readiness handle needs a task, not the
/// io_uring engine's OS threads (whose blocking `serve` entry has no handle to
/// flip readiness on anyway — see the module docs).
pub async fn spawn_with_lifecycle<F>(
    addr: &str,
    handler: F,
    config: LifecycleConfig,
) -> std::io::Result<HttpServer>
where
    F: Fn(HttpRequest) -> HttpResponse + Send + Sync + 'static,
{
    let listener = Listener::bind(addr).await?;
    let addr = listener.local_addr()?;
    let handler: Arc<SharedHandler> = Arc::new(handler);
    let lifecycle = Lifecycle::new(config);
    lifecycle.set_ready(true);
    let loop_lifecycle = lifecycle.clone();
    let task = tokio::spawn(async move {
        let _ = serve_loop(listener, handler, None, loop_lifecycle, no_upgrades()).await;
    });
    Ok(HttpServer {
        addr,
        task,
        lifecycle,
    })
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_system/http/server/tests.rs"]
mod tests;
