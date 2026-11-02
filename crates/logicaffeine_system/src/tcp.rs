//! TCP client natives (tcp.lg): the SYNC implementations the AOT tier calls via
//! `map_native_function`, plus the shared error-message formatters every tier
//! reuses — one wording, so the tiers cannot drift even in their `Err` text.
//!
//! Std-only (no tokio): the AOT tier blocks the calling thread, exactly like
//! `file::read`. The async tiers (tree-walker/VM) live in the compile crate's
//! `TcpRegistry` over tokio and reuse the formatters below.
//!
//! Connections are process-global, keyed by an opaque `Int` handle — the value a
//! LOGOS program threads through `tcpSend`/`tcpRecv`/`tcpClose`. A closed or
//! never-opened handle is always an `Err` value, never a panic.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, OnceLock};

use logicaffeine_data::LogosSeq;

/// The largest single `tcpRecv`/`tcpRecvExact` request (64 MiB) — a guard against
/// a hostile or corrupted length field, not a protocol limit (loop for more).
pub const MAX_RECV: i64 = 64 * 1024 * 1024;

pub fn err_invalid_port(port: i64) -> String {
    format!("tcp: invalid port {port} (1..=65535)")
}
pub fn err_connect(host: &str, port: i64, e: &dyn std::fmt::Display) -> String {
    format!("tcp: connect to {host}:{port} failed: {e}")
}
pub fn err_unknown_conn(id: i64) -> String {
    format!("tcp: closed or unknown connection {id}")
}
pub fn err_invalid_len(len: i64) -> String {
    format!("tcp: invalid length {len} (0..={MAX_RECV})")
}
pub fn err_send(e: &dyn std::fmt::Display) -> String {
    format!("tcp: send failed: {e}")
}
pub fn err_recv(e: &dyn std::fmt::Display) -> String {
    format!("tcp: recv failed: {e}")
}
pub fn err_torn(got: usize, want: i64) -> String {
    format!("tcp: connection closed after {got} of {want} bytes")
}
pub fn err_timeout() -> String {
    "tcp: operation timed out".to_string()
}
pub fn err_listen(port: i64, e: &dyn std::fmt::Display) -> String {
    format!("tcp: listen on port {port} failed: {e}")
}
pub fn err_accept(e: &dyn std::fmt::Display) -> String {
    format!("tcp: accept failed: {e}")
}
pub fn err_shutdown(e: &dyn std::fmt::Display) -> String {
    format!("tcp: shutdown failed: {e}")
}
pub fn err_invalid_timeout(millis: i64) -> String {
    format!("tcp: invalid timeout {millis} ms (0 clears, otherwise 1..)")
}

/// True when an IO error is the stream's read/write deadline expiring.
fn is_timeout(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

struct Registry {
    /// ★ EACH CONNECTION CARRIES ITS OWN LOCK, and the registry's lock is held only long enough
    /// to look one up. Holding the REGISTRY lock across a blocking syscall meant one stalled peer
    /// froze every other connection — and froze `close`/`set_timeout`/`shutdown_write` too, so the
    /// calls used to CANCEL a hang queued behind the hang. `accept` already knew this ("Lock per
    /// poll so a long accept never wedges other connections"); the other three did not.
    ///
    /// Per-connection rather than a cloned handle because a future TLS stream is a stateful
    /// SESSION that cannot be duplicated: two operations on one connection must serialize, two
    /// on different connections must not.
    conns: HashMap<i64, ConnEntry>,
    listeners: HashMap<i64, std::net::TcpListener>,
    /// Accept timeout per listener — std has no accept deadline, so a timed
    /// listener is switched non-blocking and polled against the deadline.
    listener_timeouts: HashMap<i64, std::time::Duration>,
    next: i64,
}

/// One connection: the stream its I/O serializes on, and a SEPARATE handle for control.
///
/// ★ THE CONTROL HANDLE IS THE POINT. `set_timeout`, `shutdown_write` and `close` are how a
/// program cancels a hung connection, so they must not queue behind the very read they are
/// cancelling. `try_clone` duplicates the descriptor — same socket, independent handle — so a
/// deadline or a FIN can be applied while a reader is parked inside `read`. Closing shuts the
/// socket down through it, which is what actually wakes the blocked reader rather than merely
/// dropping a map entry the reader still holds an `Arc` to.
struct ConnEntry {
    io: Arc<Mutex<Conn>>,
    ctl: TcpStream,
}

/// A connection's byte stream: plain, or wrapped in a TLS session.
///
/// ★ THE HANDLE STAYS THE SAME, WHICH IS THE WHOLE DESIGN. `tcpSend`/`tcpRecv`/`tcpRecvExact`
/// dispatch on this and nothing above them changes — so `postgres.lg`, two thousand lines of pure
/// LOGOS speaking the wire over these natives with SCRAM-SHA-256 included, gains TLS without one
/// line changing. The same is true of anything else ever written over TCP.
///
/// A TLS session is STATEFUL and cannot be duplicated, which is why the entry above serializes I/O
/// on its own lock and keeps a separate raw descriptor for control: a deadline or a FIN still
/// applies while a reader is parked, but two reads of one session can never interleave.
enum Conn {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Conn::Plain(s) => s.read(buf),
            Conn::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Conn::Plain(s) => s.write(buf),
            Conn::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Conn::Plain(s) => s.flush(),
            Conn::Tls(s) => s.flush(),
        }
    }
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        Mutex::new(Registry {
            conns: HashMap::new(),
            listeners: HashMap::new(),
            listener_timeouts: HashMap::new(),
            next: 1,
        })
    })
}

/// The connection's own lock, looked up under the registry lock and returned so the caller can do
/// its blocking I/O with the registry FREE. This is the whole fix for the wedging described on
/// [`Registry::conns`].
fn conn_handle(conn: i64) -> Result<Arc<Mutex<Conn>>, String> {
    let reg = registry().lock().unwrap();
    reg.conns
        .get(&conn)
        .map(|e| e.io.clone())
        .ok_or_else(|| err_unknown_conn(conn))
}

/// Wrap an accepted or connected stream, duplicating its descriptor for the control handle.
fn new_entry(stream: TcpStream) -> Result<ConnEntry, String> {
    let ctl = stream.try_clone().map_err(|e| err_recv(&e))?;
    Ok(ConnEntry {
        io: Arc::new(Mutex::new(Conn::Plain(stream))),
        ctl,
    })
}

fn seq_to_bytes(data: &[i64]) -> Vec<u8> {
    data.iter().map(|v| (v & 0xff) as u8).collect()
}

fn bytes_to_seq(bytes: Vec<u8>) -> LogosSeq<i64> {
    LogosSeq::from_vec(bytes.into_iter().map(|b| b as i64).collect())
}

/// Open a TCP connection; `Ok` carries the opaque connection handle.
pub fn connect(host: String, port: i64) -> Result<i64, String> {
    if !(1..=65535).contains(&port) {
        return Err(err_invalid_port(port));
    }
    match TcpStream::connect((host.as_str(), port as u16)) {
        Ok(stream) => {
            // Interactive protocol clients disable Nagle (libpq/tokio-postgres do) —
            // small request/response frames must not wait for coalescing.
            stream
                .set_nodelay(true)
                .map_err(|e| err_connect(&host, port, &e))?;
            let mut reg = registry().lock().unwrap();
            let id = reg.next;
            reg.next += 1;
            reg.conns.insert(id, new_entry(stream)?);
            Ok(id)
        }
        Err(e) => Err(err_connect(&host, port, &e)),
    }
}

/// The client TLS configuration: the bundled Mozilla roots, plus any extra root the operator
/// names in `LOGOS_TLS_EXTRA_ROOT` (a PEM file).
///
/// ★ PUBLIC BECAUSE BOTH TIERS MUST SHARE IT. The interpreter and VM drive sockets through tokio
/// and the AOT tier drives them synchronously, so there are two socket registries — but there must
/// be exactly ONE set of trusted roots and one verification policy. Two copies would drift, and the
/// direction they drift in is "which certificates are accepted".
///
/// ★ BUNDLED ROOTS, NOT THE OS STORE, so a compiled binary trusts the same authorities wherever it
/// runs — the same reason every other input here is pinned. An OS store makes a build's behaviour
/// depend on the machine it happens to execute on.
///
/// ★ AND THE EXTRA ROOT IS A REAL FEATURE, not test scaffolding: a private CA is how a
/// self-hosted PostgreSQL or an internal API is reached, and without a seam for it those are
/// simply unreachable. It also lets the socket tests stand up a hermetic TLS server rather than
/// proving TLS against the internet.
/// Install the process-level rustls `CryptoProvider`, once, idempotently.
///
/// ★ IT HAS TO BE CALLABLE FROM BOTH SIDES, and it used to be reachable only from one.
///
/// rustls refuses to guess when more than one crypto backend is compiled in and PANICS at the
/// first handshake — and Cargo feature unification makes that unavoidable by declaration alone,
/// since another crate in this workspace's graph enables `aws-lc-rs` and a feature enabled
/// anywhere is enabled everywhere.
///
/// The install used to live inside [`tls_config`]'s cache, which only runs on the CLIENT path. A
/// process that builds a `ServerConfig` first therefore panicked. Under `cargo test` that never
/// showed: every test shares one process, so whichever test connected first installed the provider
/// and every later server builder inherited it. Under nextest — one process PER TEST — the test
/// that only ever builds a server got a process with no provider, and died. The ordering was doing
/// the work, and the ordering is not a guarantee.
pub fn ensure_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // Errors only if a provider is ALREADY set, which is a fine outcome.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

pub fn tls_config() -> Result<Arc<rustls::ClientConfig>, String> {
    // ★ CACHED BY THE ROOTS IT WAS BUILT FROM, not once for the life of the process.
    //
    // A `OnceLock` froze whichever root set the FIRST connection happened to see. A program that
    // reaches two databases behind two different private authorities then had its second
    // `LOGOS_TLS_EXTRA_ROOT` silently ignored — the connection either fails with `UnknownIssuer`
    // for no visible reason, or, worse, the reader concludes the variable does nothing and reaches
    // for a way to skip verification. Keying the cache on the variable's current value costs one
    // string comparison per connect and makes the setting mean what it says every time.
    //
    // It is also what makes `tcp_tls_lock` deterministic: one of its tests must trust the test CA
    // and another must trust nothing, and under a threaded runner the `OnceLock` gave whichever
    // ran first the power to decide for both. The lock's own doc comment predicted that race; it
    // is fixed here rather than worked around there, because the product had the same bug.
    static CONFIGS: OnceLock<Mutex<HashMap<String, Result<Arc<rustls::ClientConfig>, String>>>> =
        OnceLock::new();
    let key = std::env::var("LOGOS_TLS_EXTRA_ROOT").unwrap_or_default();
    let cache = CONFIGS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(hit) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return hit.clone();
    }
    let built = (|| {
        // ★ CHOOSE THE PROVIDER EXPLICITLY. rustls refuses to guess when more than one crypto
        // backend is compiled in, and panics at the first handshake with "Could not
        // automatically determine the process-level CryptoProvider". Cargo FEATURE UNIFICATION
        // makes that unavoidable by declaration alone: some other crate in this workspace's
        // graph enables `aws-lc-rs`, and a feature enabled anywhere is enabled everywhere, so
        // `default-features = false` on our own dependency cannot switch it off.
        //
        // Installing it here rather than at every entry point means the choice is made once,
        // in the same place the roots are decided, and cannot be forgotten by a new caller.
        // `install_default` errors only if a provider is ALREADY set, which is a fine outcome.
        ensure_crypto_provider();
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        if !key.is_empty() {
            let path = key.clone();
            let pem = std::fs::read(&path)
                .map_err(|e| format!("tls: cannot read LOGOS_TLS_EXTRA_ROOT '{path}': {e}"))?;
            let mut added = 0usize;
            for der in rustls_pemfile_certs(&pem) {
                roots
                    .add(der)
                    .map_err(|e| format!("tls: '{path}' is not a usable certificate: {e}"))?;
                added += 1;
            }
            if added == 0 {
                return Err(format!("tls: '{path}' contains no CERTIFICATE block"));
            }
        }
        Ok(Arc::new(
            rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        ))
    })();
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, built.clone());
    built
}

/// Every `CERTIFICATE` block in a PEM file, as DER. Written here rather than pulling a crate for
/// it: the format is a base64 body between two marker lines.
fn rustls_pemfile_certs(pem: &[u8]) -> Vec<rustls::pki_types::CertificateDer<'static>> {
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END: &str = "-----END CERTIFICATE-----";
    let text = String::from_utf8_lossy(pem);
    let mut out = Vec::new();
    let mut rest = text.as_ref();
    while let Some(b) = rest.find(BEGIN) {
        let after = &rest[b + BEGIN.len()..];
        let Some(e) = after.find(END) else { break };
        let body: String = after[..e].chars().filter(|c| !c.is_whitespace()).collect();
        if let Some(der) = base64_decode(&body) {
            out.push(rustls::pki_types::CertificateDer::from(der));
        }
        rest = &after[e + END.len()..];
    }
    out
}

/// Standard base64 decode (no padding-length validation beyond what the alphabet implies).
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut out = Vec::new();
    for c in s.bytes() {
        if c == b'=' {
            break;
        }
        let v = A.iter().position(|&a| a == c)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Open a TCP connection and complete a TLS handshake over it; `Ok` carries the same opaque
/// handle every other native takes, so `tcpSend`/`tcpRecv` work unchanged.
pub fn connect_tls(host: String, port: i64) -> Result<i64, String> {
    let id = connect(host.clone(), port)?;
    match upgrade_tls(id, host) {
        Ok(()) => Ok(id),
        Err(e) => {
            let _ = close(id);
            Err(e)
        }
    }
}

/// Wrap an ESTABLISHED plain connection in TLS, in place.
///
/// ★ THIS IS THE ONE PostgreSQL NEEDS. Its negotiation is not "connect with TLS": the client sends
/// an 8-byte SSLRequest over the plain socket, reads a single `S` or `N`, and only then hands the
/// same socket to TLS. A connect-only API cannot express that. It also gives SMTP STARTTLS and
/// HTTP `CONNECT` proxying for nothing.
pub fn upgrade_tls(conn: i64, host: String) -> Result<(), String> {
    let cfg = tls_config()?;
    let server = rustls::pki_types::ServerName::try_from(host.clone())
        .map_err(|_| format!("tls: '{host}' is not a valid server name"))?
        .to_owned();
    let handle = conn_handle(conn)?;
    let mut guard = handle.lock().unwrap();
    let plain = match &mut *guard {
        Conn::Tls(_) => return Err(format!("tls: connection {conn} is already secured")),
        Conn::Plain(s) => s.try_clone().map_err(|e| err_recv(&e))?,
    };
    let client = rustls::ClientConnection::new(cfg, server)
        .map_err(|e| format!("tls: handshake could not start for '{host}': {e}"))?;
    let mut stream = rustls::StreamOwned::new(client, plain);
    // ★ HANDSHAKE NOW, NOT ON FIRST WRITE. A caller that gets `Ok` has a VERIFIED peer; deferring
    // it would report success for a certificate that is about to be rejected, and the first
    // `tcpSend` would fail with an error about sending.
    stream
        .conn
        .complete_io(&mut stream.sock)
        .map_err(|e| format!("tls: handshake with '{host}' failed: {e}"))?;
    *guard = Conn::Tls(Box::new(stream));
    Ok(())
}

/// Set the connection's read+write timeout in milliseconds (`0` clears it) —
/// Python `settimeout` style: it applies to every subsequent recv/send, whose
/// expiry is the shared `tcp: operation timed out` error.
pub fn set_timeout(conn: i64, millis: i64) -> Result<(), String> {
    if millis < 0 {
        return Err(err_invalid_timeout(millis));
    }
    let dur = (millis > 0).then(|| std::time::Duration::from_millis(millis as u64));
    let mut reg = registry().lock().unwrap();
    if let Some(entry) = reg.conns.get(&conn) {
        // Through the CONTROL handle, so arming a deadline on a connection that is already
        // blocked in `read` works — that is the case a deadline exists for.
        entry.ctl.set_read_timeout(dur).map_err(|e| err_recv(&e))?;
        entry.ctl.set_write_timeout(dur).map_err(|e| err_send(&e))?;
        return Ok(());
    }
    if reg.listeners.contains_key(&conn) {
        match dur {
            Some(d) => reg.listener_timeouts.insert(conn, d),
            None => reg.listener_timeouts.remove(&conn),
        };
        return Ok(());
    }
    Err(err_unknown_conn(conn))
}

/// Send every byte (each Int masked to 0–255); `Ok` carries the byte count.
/// `data` arrives as the AOT tier's borrowed `Seq of Int` parameter form.
pub fn send(conn: i64, data: &[i64]) -> Result<i64, String> {
    let bytes = seq_to_bytes(data);
    let handle = conn_handle(conn)?;
    let mut stream = handle.lock().unwrap();
    stream.write_all(&bytes).map_err(|e| {
        if is_timeout(&e) {
            err_timeout()
        } else {
            err_send(&e)
        }
    })?;
    Ok(bytes.len() as i64)
}

/// Block until at least one byte arrives (or EOF), returning up to `max_len`
/// bytes. A clean EOF is `Ok` of the empty Seq — a value, not an error.
pub fn recv(conn: i64, max_len: i64) -> Result<LogosSeq<i64>, String> {
    if !(0..=MAX_RECV).contains(&max_len) {
        return Err(err_invalid_len(max_len));
    }
    let handle = conn_handle(conn)?;
    let mut stream = handle.lock().unwrap();
    let mut buf = vec![0u8; max_len as usize];
    let n = stream.read(&mut buf).map_err(|e| {
        if is_timeout(&e) {
            err_timeout()
        } else {
            err_recv(&e)
        }
    })?;
    buf.truncate(n);
    Ok(bytes_to_seq(buf))
}

/// Block until exactly `len` bytes arrive; a peer closing early is an `Err`
/// (a torn frame is never silently truncated — protocol framing depends on it).
pub fn recv_exact(conn: i64, len: i64) -> Result<LogosSeq<i64>, String> {
    if !(0..=MAX_RECV).contains(&len) {
        return Err(err_invalid_len(len));
    }
    let handle = conn_handle(conn)?;
    let mut stream = handle.lock().unwrap();
    let mut buf = vec![0u8; len as usize];
    let mut got = 0usize;
    while got < buf.len() {
        match stream.read(&mut buf[got..]) {
            Ok(0) => return Err(err_torn(got, len)),
            Ok(n) => got += n,
            Err(e) if is_timeout(&e) => return Err(err_timeout()),
            Err(e) => return Err(err_recv(&e)),
        }
    }
    Ok(bytes_to_seq(buf))
}

/// Close the connection or listener; the handle is dead afterwards.
pub fn close(conn: i64) -> Result<(), String> {
    let mut reg = registry().lock().unwrap();
    reg.listener_timeouts.remove(&conn);
    if let Some(entry) = reg.conns.remove(&conn) {
        // ★ DROP, DO NOT SHUT DOWN — and this was a real regression, caught by
        // `http_workers::a_handler_error_is_a_500_and_the_server_survives`. Forcing
        // `shutdown(Both)` here looked like the way to make `close` wake a reader parked on the
        // duplicated descriptor, but a socket shut down with unread data in its receive buffer is
        // RESET rather than closed, so a client waiting on an already-written 500 got a connection
        // reset instead of the response. Closing a connection is not the same act as cancelling
        // one.
        //
        // Cancellation has its own verbs and they work while a read is parked, which is the point
        // of the control descriptor: `tcpSetTimeout` arms a deadline and `tcpShutdownWrite` sends
        // FIN, both without taking the io lock. `close` stays a graceful release.
        drop(entry);
        return Ok(());
    }
    if let Some(listener) = reg.listeners.remove(&conn) {
        drop(listener);
        return Ok(());
    }
    Err(err_unknown_conn(conn))
}

/// Bind a listening socket (passive open; port `0` = ephemeral).
pub fn listen(port: i64) -> Result<i64, String> {
    if !(0..=65535).contains(&port) {
        return Err(err_invalid_port(port));
    }
    match std::net::TcpListener::bind(("0.0.0.0", port as u16)) {
        Ok(listener) => {
            let mut reg = registry().lock().unwrap();
            let id = reg.next;
            reg.next += 1;
            reg.listeners.insert(id, listener);
            Ok(id)
        }
        Err(e) => Err(err_listen(port, &e)),
    }
}

/// Accept one queued connection (blocks; a `set_timeout` on the listener applies).
pub fn accept(listener: i64) -> Result<i64, String> {
    let deadline = {
        let reg = registry().lock().unwrap();
        if !reg.listeners.contains_key(&listener) {
            return Err(err_unknown_conn(listener));
        }
        reg.listener_timeouts
            .get(&listener)
            .map(|d| std::time::Instant::now() + *d)
    };
    loop {
        // Lock per poll so a long accept never wedges other connections.
        let accepted = {
            let mut reg = registry().lock().unwrap();
            let l = reg
                .listeners
                .get_mut(&listener)
                .ok_or_else(|| err_unknown_conn(listener))?;
            if deadline.is_some() {
                l.set_nonblocking(true).map_err(|e| err_accept(&e))?;
            }
            match l.accept() {
                Ok((stream, _peer)) => {
                    stream.set_nonblocking(false).map_err(|e| err_accept(&e))?;
                    stream.set_nodelay(true).map_err(|e| err_accept(&e))?;
                    let id = reg.next;
                    match new_entry(stream) {
                        Ok(entry) => {
                            reg.next += 1;
                            reg.conns.insert(id, entry);
                            Some(Ok(id))
                        }
                        Err(e) => Some(Err(e)),
                    }
                }
                Err(e) if is_timeout(&e) && deadline.is_some() => None,
                Err(e) => Some(Err(err_accept(&e))),
            }
        };
        match accepted {
            Some(result) => return result,
            None => {
                if let Some(dl) = deadline {
                    if std::time::Instant::now() >= dl {
                        return Err(err_timeout());
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }
}

/// The local port a listener is bound to (how an ephemeral bind is learned).
pub fn listener_port(listener: i64) -> Result<i64, String> {
    let reg = registry().lock().unwrap();
    let l = reg
        .listeners
        .get(&listener)
        .ok_or_else(|| err_unknown_conn(listener))?;
    l.local_addr()
        .map(|a| a.port() as i64)
        .map_err(|e| err_accept(&e))
}

/// Half-close: send FIN on the write side (`shutdown(SHUT_WR)`), keep reading.
pub fn shutdown_write(conn: i64) -> Result<(), String> {
    let reg = registry().lock().unwrap();
    let entry = reg.conns.get(&conn).ok_or_else(|| err_unknown_conn(conn))?;
    entry
        .ctl
        .shutdown(std::net::Shutdown::Write)
        .map_err(|e| err_shutdown(&e))
}
