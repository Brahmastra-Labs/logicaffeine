//! The shared TCP connection registry for the ASYNC tiers (tree-walker + VM task
//! driver) — the tcp.lg natives' runtime. Mirrors [`net_inbox::NetInbox`]'s shape:
//! each holder embeds a `TcpRegistry` by value, and BOTH tiers funnel through the
//! ONE value-level entry point [`TcpRegistry::exec_values`], so argument decoding,
//! byte conversion, semantics, and even `Err` wording cannot drift between tiers.
//! The wording itself is shared further still: every message comes from
//! `logicaffeine_system::tcp`'s formatters, which the sync AOT implementation
//! also uses — one wording, every tier.
//!
//! Semantics (locked by `tcp_natives.rs` / `tcp_cross_tier.rs`):
//! - `recv` blocks for ≥1 byte; clean EOF is `Ok([])` — a value, not an error.
//! - `recv_exact` demands the full count; early close is a torn-frame `Err`.
//! - any operation on a closed/unknown handle is an `Err`, never a panic.

use std::collections::HashMap;

// tokio (async sockets) cannot target `wasm32-unknown-unknown` — it has no sockets — and
// `logicaffeine_system` does not re-export `tokio` there. The compile crate is built for wasm32 to
// harvest the numeric-tower rlibs for the WASM tier (`build_bigint_runtime`), so the socket machinery
// MUST be gated off that target (it is tree-shaken away regardless); a zero-field `TcpRegistry` stub
// keeps the interpreter/VM field + `exec_values` API target-uniform.
#[cfg(not(target_arch = "wasm32"))]
use logicaffeine_system::tcp as tcpmsg;
#[cfg(not(target_arch = "wasm32"))]
use logicaffeine_system::tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(not(target_arch = "wasm32"))]
use logicaffeine_system::tokio::net::TcpStream;

use crate::interpreter::RuntimeValue;
#[cfg(not(target_arch = "wasm32"))]
use crate::semantics::builtins::{byte_seq, bytes_to_seq, result_err, result_ok};

/// The five tcp.lg natives, in declaration order. `from_name` is the single
/// name→op table every tier's interception consults (arity included).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TcpOp {
    Connect,
    Send,
    Recv,
    RecvExact,
    Close,
    SetTimeout,
    Listen,
    Accept,
    ListenerPort,
    ShutdownWrite,
    /// `tcpConnectTls(host, port)` — connect and complete a TLS handshake, answering the same
    /// opaque handle every other op takes.
    ConnectTls,
    /// `tcpUpgradeTls(conn, host)` — wrap an ESTABLISHED plain connection in TLS, in place. This
    /// is the one PostgreSQL needs: SSLRequest goes over the plain socket, a single `S` comes
    /// back, and only then does the same socket become a session.
    UpgradeTls,
}

impl TcpOp {
    /// The op for a call site `name(argc args)` — `None` when the name/arity is
    /// not a tcp native. The caller still applies its declarer-wins guard.
    pub(crate) fn from_name(name: &str, argc: usize) -> Option<TcpOp> {
        match (name, argc) {
            ("tcpConnect", 2) => Some(TcpOp::Connect),
            ("tcpSend", 2) => Some(TcpOp::Send),
            ("tcpRecv", 2) => Some(TcpOp::Recv),
            ("tcpRecvExact", 2) => Some(TcpOp::RecvExact),
            ("tcpClose", 1) => Some(TcpOp::Close),
            ("tcpSetTimeout", 2) => Some(TcpOp::SetTimeout),
            ("tcpListen", 1) => Some(TcpOp::Listen),
            ("tcpAccept", 1) => Some(TcpOp::Accept),
            ("tcpListenerPort", 1) => Some(TcpOp::ListenerPort),
            ("tcpShutdownWrite", 1) => Some(TcpOp::ShutdownWrite),
            ("tcpConnectTls", 2) => Some(TcpOp::ConnectTls),
            ("tcpUpgradeTls", 2) => Some(TcpOp::UpgradeTls),
            _ => None,
        }
    }

    /// Stable numeric encoding for the bytecode op / `VmBlock` payload.
    pub(crate) fn code(self) -> u8 {
        match self {
            TcpOp::Connect => 0,
            TcpOp::Send => 1,
            TcpOp::Recv => 2,
            TcpOp::RecvExact => 3,
            TcpOp::Close => 4,
            TcpOp::SetTimeout => 5,
            TcpOp::Listen => 6,
            TcpOp::Accept => 7,
            TcpOp::ListenerPort => 8,
            TcpOp::ShutdownWrite => 9,
            TcpOp::ConnectTls => 10,
            TcpOp::UpgradeTls => 11,
        }
    }

    pub(crate) fn from_code(code: u8) -> Option<TcpOp> {
        match code {
            0 => Some(TcpOp::Connect),
            1 => Some(TcpOp::Send),
            2 => Some(TcpOp::Recv),
            3 => Some(TcpOp::RecvExact),
            4 => Some(TcpOp::Close),
            5 => Some(TcpOp::SetTimeout),
            6 => Some(TcpOp::Listen),
            7 => Some(TcpOp::Accept),
            8 => Some(TcpOp::ListenerPort),
            9 => Some(TcpOp::ShutdownWrite),
            10 => Some(TcpOp::ConnectTls),
            11 => Some(TcpOp::UpgradeTls),
            _ => None,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// A connection's byte stream on the async tier: plain, or wrapped in a TLS session.
///
/// ★ MIRRORS `logicaffeine_system::tcp::Conn` DELIBERATELY. Two runtimes need two stream types,
/// but they must present the same surface to LOGOS and trust the same roots — the config comes
/// from `logicaffeine_system::tcp::tls_config`, which is the single policy both tiers read.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) enum AsyncConn {
    Plain(TcpStream),
    Tls(Box<logicaffeine_system::tokio_rustls::client::TlsStream<TcpStream>>),
}

#[cfg(not(target_arch = "wasm32"))]
impl AsyncConn {
    async fn read_buf(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use logicaffeine_system::tokio::io::AsyncReadExt;
        match self {
            AsyncConn::Plain(s) => s.read(buf).await,
            AsyncConn::Tls(s) => s.read(buf).await,
        }
    }
    async fn write_all_buf(&mut self, buf: &[u8]) -> std::io::Result<()> {
        use logicaffeine_system::tokio::io::AsyncWriteExt;
        match self {
            AsyncConn::Plain(s) => s.write_all(buf).await,
            AsyncConn::Tls(s) => s.write_all(buf).await,
        }
    }
    async fn shutdown_write(&mut self) -> std::io::Result<()> {
        use logicaffeine_system::tokio::io::AsyncWriteExt;
        match self {
            AsyncConn::Plain(s) => s.shutdown().await,
            AsyncConn::Tls(s) => s.shutdown().await,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct TcpRegistry {
    conns: HashMap<i64, AsyncConn>,
    /// Listening sockets (`tcpListen`), same id space as connections.
    listeners: HashMap<i64, logicaffeine_system::tokio::net::TcpListener>,
    /// Per-handle recv/send/accept timeout (`tcpSetTimeout`, Python
    /// `settimeout` style) — expiry is the shared `tcp: operation timed out`.
    timeouts: HashMap<i64, std::time::Duration>,
    next: i64,
}

/// wasm32 has no sockets — a zero-field stub whose ops all error (see the module-top note).
#[cfg(target_arch = "wasm32")]
pub(crate) struct TcpRegistry;

#[cfg(target_arch = "wasm32")]
impl TcpRegistry {
    pub(crate) fn new() -> Self {
        TcpRegistry
    }

    pub(crate) async fn exec_values(
        &mut self,
        _op: TcpOp,
        _vals: Vec<RuntimeValue>,
    ) -> Result<RuntimeValue, String> {
        Err("tcp: networking is unavailable on the wasm32 target".to_string())
    }
}

/// Apply the connection's timeout (when set) to one IO future — expiry is the
/// tier-shared timeout error.
#[cfg(not(target_arch = "wasm32"))]
async fn with_timeout<T>(
    dur: Option<std::time::Duration>,
    fut: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    match dur {
        Some(d) => match logicaffeine_system::tokio::time::timeout(d, fut).await {
            Ok(r) => r,
            Err(_) => Err(tcpmsg::err_timeout()),
        },
        None => fut.await,
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl TcpRegistry {
    pub(crate) fn new() -> Self {
        TcpRegistry {
            conns: HashMap::new(),
            listeners: HashMap::new(),
            timeouts: HashMap::new(),
            next: 1,
        }
    }

    /// THE tier-shared entry point: evaluated argument values in, the LOGOS
    /// `Result` value out. Wrong argument SHAPES (an interpreter/VM bug, not a
    /// program condition) are a runtime error via `Err`.
    pub(crate) async fn exec_values(
        &mut self,
        op: TcpOp,
        vals: Vec<RuntimeValue>,
    ) -> Result<RuntimeValue, String> {
        fn int_arg(v: &RuntimeValue, what: &str) -> Result<i64, String> {
            match v {
                RuntimeValue::Int(n) => Ok(*n),
                other => Err(format!(
                    "tcp: {what} must be an Int, got {}",
                    other.type_name()
                )),
            }
        }
        fn text_arg(v: &RuntimeValue, what: &str) -> Result<String, String> {
            match v {
                RuntimeValue::Text(t) => Ok(t.to_string()),
                other => Err(format!(
                    "tcp: {what} must be Text, got {}",
                    other.type_name()
                )),
            }
        }
        Ok(match op {
            // ★ CONNECT THEN UPGRADE, so there is exactly one handshake path. A separate
            // connect-with-TLS would be a second place for the verification policy to live.
            TcpOp::ConnectTls => {
                let host = text_arg(&vals[0], "host")?;
                let port = int_arg(&vals[1], "port")?;
                match self.connect(&host, port).await {
                    Ok(id) => match self.upgrade_tls(id, &host).await {
                        Ok(()) => result_ok(RuntimeValue::Int(id)),
                        Err(e) => {
                            let _ = self.close(id);
                            result_err(e)
                        }
                    },
                    Err(e) => result_err(e),
                }
            }
            TcpOp::UpgradeTls => {
                let conn = int_arg(&vals[0], "connection")?;
                let host = text_arg(&vals[1], "host")?;
                match self.upgrade_tls(conn, &host).await {
                    Ok(()) => result_ok(RuntimeValue::Nothing),
                    Err(e) => result_err(e),
                }
            }
            TcpOp::Connect => {
                let host = match &vals[0] {
                    RuntimeValue::Text(t) => t.to_string(),
                    other => {
                        return Err(format!("tcp: host must be Text, got {}", other.type_name()))
                    }
                };
                let port = int_arg(&vals[1], "port")?;
                match self.connect(&host, port).await {
                    Ok(id) => result_ok(RuntimeValue::Int(id)),
                    Err(e) => result_err(e),
                }
            }
            TcpOp::Send => {
                let conn = int_arg(&vals[0], "conn")?;
                let data = byte_seq(&vals[1])?;
                match self.send(conn, &data).await {
                    Ok(n) => result_ok(RuntimeValue::Int(n)),
                    Err(e) => result_err(e),
                }
            }
            TcpOp::Recv => {
                let conn = int_arg(&vals[0], "conn")?;
                let max = int_arg(&vals[1], "maxLen")?;
                match self.recv(conn, max).await {
                    Ok(bytes) => result_ok(bytes_to_seq(&bytes)),
                    Err(e) => result_err(e),
                }
            }
            TcpOp::RecvExact => {
                let conn = int_arg(&vals[0], "conn")?;
                let len = int_arg(&vals[1], "len")?;
                match self.recv_exact(conn, len).await {
                    Ok(bytes) => result_ok(bytes_to_seq(&bytes)),
                    Err(e) => result_err(e),
                }
            }
            TcpOp::Close => {
                let conn = int_arg(&vals[0], "conn")?;
                match self.close(conn) {
                    Ok(()) => result_ok(RuntimeValue::Nothing),
                    Err(e) => result_err(e),
                }
            }
            TcpOp::SetTimeout => {
                let conn = int_arg(&vals[0], "conn")?;
                let millis = int_arg(&vals[1], "millis")?;
                match self.set_timeout(conn, millis) {
                    Ok(()) => result_ok(RuntimeValue::Nothing),
                    Err(e) => result_err(e),
                }
            }
            TcpOp::Listen => {
                let port = int_arg(&vals[0], "port")?;
                match self.listen(port).await {
                    Ok(id) => result_ok(RuntimeValue::Int(id)),
                    Err(e) => result_err(e),
                }
            }
            TcpOp::Accept => {
                let listener = int_arg(&vals[0], "listener")?;
                match self.accept(listener).await {
                    Ok(id) => result_ok(RuntimeValue::Int(id)),
                    Err(e) => result_err(e),
                }
            }
            TcpOp::ListenerPort => {
                let listener = int_arg(&vals[0], "listener")?;
                match self.listener_port(listener) {
                    Ok(p) => result_ok(RuntimeValue::Int(p)),
                    Err(e) => result_err(e),
                }
            }
            TcpOp::ShutdownWrite => {
                let conn = int_arg(&vals[0], "conn")?;
                match self.shutdown_write(conn).await {
                    Ok(()) => result_ok(RuntimeValue::Nothing),
                    Err(e) => result_err(e),
                }
            }
        })
    }

    async fn listen(&mut self, port: i64) -> Result<i64, String> {
        if !(0..=65535).contains(&port) {
            return Err(tcpmsg::err_invalid_port(port));
        }
        match logicaffeine_system::tokio::net::TcpListener::bind(("0.0.0.0", port as u16)).await {
            Ok(listener) => {
                let id = self.next;
                self.next += 1;
                self.listeners.insert(id, listener);
                Ok(id)
            }
            Err(e) => Err(tcpmsg::err_listen(port, &e)),
        }
    }

    async fn accept(&mut self, id: i64) -> Result<i64, String> {
        let dur = self.timeouts.get(&id).copied();
        let listener = self
            .listeners
            .get_mut(&id)
            .ok_or_else(|| tcpmsg::err_unknown_conn(id))?;
        let (stream, _peer) = with_timeout(dur, async {
            listener.accept().await.map_err(|e| tcpmsg::err_accept(&e))
        })
        .await?;
        stream
            .set_nodelay(true)
            .map_err(|e| tcpmsg::err_accept(&e))?;
        let conn_id = self.next;
        self.next += 1;
        self.conns.insert(conn_id, AsyncConn::Plain(stream));
        Ok(conn_id)
    }

    fn listener_port(&self, id: i64) -> Result<i64, String> {
        let listener = self
            .listeners
            .get(&id)
            .ok_or_else(|| tcpmsg::err_unknown_conn(id))?;
        listener
            .local_addr()
            .map(|a| a.port() as i64)
            .map_err(|e| tcpmsg::err_accept(&e))
    }

    /// Wrap an established plain connection in TLS, in place.
    ///
    /// ★ THE ROOTS COME FROM `logicaffeine_system::tcp::tls_config`, which the synchronous AOT
    /// path also reads. Two socket registries, two runtimes, ONE verification policy — anything
    /// else drifts, and the axis it drifts along is which certificates are accepted.
    async fn upgrade_tls(&mut self, conn: i64, host: &str) -> Result<(), String> {
        let cfg = logicaffeine_system::tcp::tls_config()?;
        let server = logicaffeine_system::tokio_rustls::rustls::pki_types::ServerName::try_from(
            host.to_string(),
        )
        .map_err(|_| format!("tls: '{host}' is not a valid server name"))?;
        let entry = self
            .conns
            .remove(&conn)
            .ok_or_else(|| tcpmsg::err_unknown_conn(conn))?;
        let plain = match entry {
            AsyncConn::Tls(s) => {
                // Put it back: refusing must not consume the connection.
                self.conns.insert(conn, AsyncConn::Tls(s));
                return Err(format!("tls: connection {conn} is already secured"));
            }
            AsyncConn::Plain(s) => s,
        };
        let connector = logicaffeine_system::tokio_rustls::TlsConnector::from(cfg);
        match connector.connect(server, plain).await {
            Ok(stream) => {
                self.conns.insert(conn, AsyncConn::Tls(Box::new(stream)));
                Ok(())
            }
            // ★ THE CONNECTION IS GONE ON FAILURE, and that is deliberate: a socket that has seen
            // a failed handshake is not a usable plain socket, and handing one back would invite a
            // caller to keep talking in the clear on a channel it asked to secure.
            Err(e) => Err(format!("tls: handshake with '{host}' failed: {e}")),
        }
    }

    async fn shutdown_write(&mut self, conn: i64) -> Result<(), String> {
        let stream = self
            .conns
            .get_mut(&conn)
            .ok_or_else(|| tcpmsg::err_unknown_conn(conn))?;
        stream
            .shutdown_write()
            .await
            .map_err(|e| tcpmsg::err_shutdown(&e))
    }

    fn set_timeout(&mut self, conn: i64, millis: i64) -> Result<(), String> {
        if millis < 0 {
            return Err(tcpmsg::err_invalid_timeout(millis));
        }
        if !self.conns.contains_key(&conn) && !self.listeners.contains_key(&conn) {
            return Err(tcpmsg::err_unknown_conn(conn));
        }
        if millis == 0 {
            self.timeouts.remove(&conn);
        } else {
            self.timeouts
                .insert(conn, std::time::Duration::from_millis(millis as u64));
        }
        Ok(())
    }

    async fn connect(&mut self, host: &str, port: i64) -> Result<i64, String> {
        if !(1..=65535).contains(&port) {
            return Err(tcpmsg::err_invalid_port(port));
        }
        match TcpStream::connect((host, port as u16)).await {
            Ok(stream) => {
                // Interactive protocol clients disable Nagle (libpq/tokio-postgres
                // do) — small request/response frames must not wait for coalescing.
                stream
                    .set_nodelay(true)
                    .map_err(|e| tcpmsg::err_connect(host, port, &e))?;
                let id = self.next;
                self.next += 1;
                self.conns.insert(id, AsyncConn::Plain(stream));
                Ok(id)
            }
            Err(e) => Err(tcpmsg::err_connect(host, port, &e)),
        }
    }

    async fn send(&mut self, conn: i64, data: &[u8]) -> Result<i64, String> {
        let dur = self.timeouts.get(&conn).copied();
        let stream = self
            .conns
            .get_mut(&conn)
            .ok_or_else(|| tcpmsg::err_unknown_conn(conn))?;
        with_timeout(dur, async {
            stream
                .write_all_buf(data)
                .await
                .map_err(|e| tcpmsg::err_send(&e))?;
            Ok(data.len() as i64)
        })
        .await
    }

    async fn recv(&mut self, conn: i64, max_len: i64) -> Result<Vec<u8>, String> {
        if !(0..=tcpmsg::MAX_RECV).contains(&max_len) {
            return Err(tcpmsg::err_invalid_len(max_len));
        }
        let dur = self.timeouts.get(&conn).copied();
        let stream = self
            .conns
            .get_mut(&conn)
            .ok_or_else(|| tcpmsg::err_unknown_conn(conn))?;
        with_timeout(dur, async {
            let mut buf = vec![0u8; max_len as usize];
            let n = stream
                .read_buf(&mut buf)
                .await
                .map_err(|e| tcpmsg::err_recv(&e))?;
            buf.truncate(n);
            Ok(buf)
        })
        .await
    }

    async fn recv_exact(&mut self, conn: i64, len: i64) -> Result<Vec<u8>, String> {
        if !(0..=tcpmsg::MAX_RECV).contains(&len) {
            return Err(tcpmsg::err_invalid_len(len));
        }
        let dur = self.timeouts.get(&conn).copied();
        let stream = self
            .conns
            .get_mut(&conn)
            .ok_or_else(|| tcpmsg::err_unknown_conn(conn))?;
        with_timeout(dur, async {
            let mut buf = vec![0u8; len as usize];
            let mut got = 0usize;
            while got < buf.len() {
                match stream.read_buf(&mut buf[got..]).await {
                    Ok(0) => return Err(tcpmsg::err_torn(got, len)),
                    Ok(n) => got += n,
                    Err(e) => return Err(tcpmsg::err_recv(&e)),
                }
            }
            Ok(buf)
        })
        .await
    }

    fn close(&mut self, conn: i64) -> Result<(), String> {
        self.timeouts.remove(&conn);
        if let Some(stream) = self.conns.remove(&conn) {
            drop(stream);
            return Ok(());
        }
        if let Some(listener) = self.listeners.remove(&conn) {
            drop(listener);
            return Ok(());
        }
        Err(tcpmsg::err_unknown_conn(conn))
    }
}
