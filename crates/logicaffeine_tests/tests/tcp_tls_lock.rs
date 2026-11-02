//! ★ LOCK — the socket natives can speak TLS, and they VERIFY. ★
//!
//! There was no TLS anywhere in the workspace. `addr.rs` recognised `wss`/`tls` as scheme names
//! for port defaults and nothing implemented them, so:
//!
//!   * `pgConnect` spoke cleartext over plain TCP and could therefore reach **no managed
//!     PostgreSQL at all** — RDS, Neon, Supabase and Cloud SQL every one require TLS. The live
//!     proof we had ran against a locally-booted PG16, which is exactly the environment where the
//!     gap is invisible;
//!   * no LOGOS program could call an HTTPS API, so a backend could reach no payment processor,
//!     no OAuth endpoint, no object store and no webhook.
//!
//! ★ THE HANDLE IS UNCHANGED, which is the design. `tcpConnectTls` answers the same opaque `Int`
//! every other native takes, so `postgres.lg` — two thousand lines of pure LOGOS speaking the wire,
//! SCRAM-SHA-256 included — gains TLS without one line changing.
//!
//! ★ AND VERIFICATION IS THE POINT, NOT THE ENCRYPTION. TLS that accepts any certificate is a
//! comfortable-looking way to be exactly as exposed as cleartext, so the first thing pinned here is
//! that an untrusted certificate is REFUSED.
//!
//! ★ NOTE ON ISOLATION, AND THE BUG IT FOUND. This file used to say the client config was built
//! once per process and that a threaded runner would therefore race — stated as a known limitation.
//! It then raced, exactly as written, the first time the suite ran under `cargo test` rather than
//! nextest: one test must trust the test CA and another must trust nothing, and whichever reached
//! `connect_tls` first froze the `OnceLock` for both.
//!
//! That was not a test problem. A program reaching two databases behind two different private
//! authorities had the same bug, and its symptom is an `UnknownIssuer` for no visible reason — the
//! kind that ends with somebody looking for a way to turn verification off. `tls_config` now caches
//! per root set, so the variable means what it says on every connection.
//!
//! What remains is genuinely local: `LOGOS_TLS_EXTRA_ROOT` is process-global state, so two threads
//! setting it at once would still interleave. [`with_tls_root`] holds a mutex across setting the
//! variable and using it, which makes this file correct under a threaded runner AND under nextest's
//! process-per-test without depending on which one is running it.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

/// Run `f` with `LOGOS_TLS_EXTRA_ROOT` set to `root` (or unset for `None`), holding a process-wide
/// lock for the whole window so a sibling test cannot change it underneath.
fn with_tls_root<T>(root: Option<&Path>, f: impl FnOnce() -> T) -> T {
    static ENV: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = ENV
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    match root {
        Some(p) => std::env::set_var("LOGOS_TLS_EXTRA_ROOT", p),
        None => std::env::remove_var("LOGOS_TLS_EXTRA_ROOT"),
    }
    let out = f();
    std::env::remove_var("LOGOS_TLS_EXTRA_ROOT");
    out
}

/// A TLS server on an ephemeral port, echoing one small frame. Returns its port and its CA PEM.
fn tls_echo_server() -> (u16, String, std::thread::JoinHandle<()>) {
    // The SERVER side needs the crypto provider too, and nothing else in this test installs it —
    // see `ensure_crypto_provider`. Without this, a runner that gives each test its own process
    // (nextest does; `cargo test` does not) panics here for the one test that never connects.
    logicaffeine_system::tcp::ensure_crypto_provider();
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).expect("cert");
    let pem = cert.serialize_pem().expect("pem");
    let der = cert.serialize_der().expect("der");
    let key = cert.serialize_private_key_der();

    let cfg = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![rustls::pki_types::CertificateDer::from(der)],
            rustls::pki_types::PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(
                key,
            )),
        )
        .expect("server config");

    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = l.local_addr().unwrap().port();
    let h = std::thread::spawn(move || {
        if let Ok((sock, _)) = l.accept() {
            let Ok(conn) = rustls::ServerConnection::new(Arc::new(cfg)) else {
                return;
            };
            let mut s = rustls::StreamOwned::new(conn, sock);
            let mut b = [0u8; 4];
            if let Ok(n) = s.read(&mut b) {
                let _ = s.write_all(&b[..n]);
                let _ = s.flush();
            }
        }
    });
    (port, pem, h)
}

#[test]
fn an_untrusted_certificate_is_refused() {
    // ★ THE SECURITY PROPERTY, PINNED FIRST. No extra root is trusted here, so the server's
    // self-signed certificate chains to nothing — and a TLS stack that accepted it would leave a
    // program exactly as exposed as cleartext while looking secure.
    let (port, _pem, h) = tls_echo_server();
    let err = with_tls_root(None, || {
        logicaffeine_system::tcp::connect_tls("localhost".into(), port as i64)
            .expect_err("an unknown certificate authority must be refused")
    });
    assert!(
        err.to_lowercase().contains("tls"),
        "the refusal must say it was TLS that refused: {err}"
    );
    let _ = h.join();
}

#[test]
fn a_trusted_certificate_completes_a_handshake_and_carries_bytes() {
    let (port, pem, h) = tls_echo_server();
    let dir = tempfile::tempdir().expect("dir");
    let ca = dir.path().join("ca.pem");
    std::fs::write(&ca, pem).expect("write ca");
    let c = with_tls_root(Some(&ca), || {
        logicaffeine_system::tcp::connect_tls("localhost".into(), port as i64)
            .expect("a trusted certificate must complete the handshake")
    });
    logicaffeine_system::tcp::send(c, &[104, 105]).expect("send over TLS");
    let got = logicaffeine_system::tcp::recv_exact(c, 2).expect("read the echo back");
    assert_eq!(
        got.iter().collect::<Vec<_>>(),
        vec![104, 105],
        "bytes survive the session"
    );
    logicaffeine_system::tcp::close(c).expect("close");
    let _ = h.join();
}

#[test]
fn tls_to_a_plain_port_fails_rather_than_hanging() {
    // A plain listener answers the handshake with nothing a TLS peer can read. The failure must be
    // an error, not a stall — this is the shape a mistyped port takes.
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = l.local_addr().unwrap().port();
    let h = std::thread::spawn(move || {
        if let Ok((s, _)) = l.accept() {
            std::thread::sleep(std::time::Duration::from_millis(200));
            drop(s);
        }
    });
    let started = std::time::Instant::now();
    let err = with_tls_root(None, || {
        logicaffeine_system::tcp::connect_tls("localhost".into(), port as i64)
            .expect_err("a plain peer cannot complete a TLS handshake")
    });
    assert!(
        err.to_lowercase().contains("tls"),
        "named as a TLS failure: {err}"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "it failed rather than hung"
    );
    let _ = h.join();
}

#[test]
fn upgrading_an_already_secured_connection_is_refused_by_name() {
    let (port, pem, h) = tls_echo_server();
    let dir = tempfile::tempdir().expect("dir");
    let ca = dir.path().join("ca.pem");
    std::fs::write(&ca, pem).expect("write ca");
    let c = with_tls_root(Some(&ca), || {
        logicaffeine_system::tcp::connect_tls("localhost".into(), port as i64).expect("connect")
    });
    let err = logicaffeine_system::tcp::upgrade_tls(c, "localhost".into())
        .expect_err("a second handshake on one session is a mistake, not a no-op");
    assert!(err.contains("already secured"), "refused by name: {err}");
    let _ = logicaffeine_system::tcp::close(c);
    let _ = h.join();
}

#[test]
fn a_plain_connection_still_works_unchanged() {
    // The whole point of putting TLS under the handle is that nothing above it moved.
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = l.local_addr().unwrap().port();
    let h = std::thread::spawn(move || {
        if let Ok((mut s, _)) = l.accept() {
            let mut b = [0u8; 4];
            let n = s.read(&mut b).unwrap_or(0);
            let _ = s.write_all(&b[..n]);
        }
    });
    let c = logicaffeine_system::tcp::connect("127.0.0.1".into(), port as i64).expect("connect");
    logicaffeine_system::tcp::send(c, &[7, 8]).expect("send");
    let got = logicaffeine_system::tcp::recv_exact(c, 2).expect("recv");
    assert_eq!(got.iter().collect::<Vec<_>>(), vec![7, 8]);
    logicaffeine_system::tcp::close(c).expect("close");
    let _ = h.join();
}
