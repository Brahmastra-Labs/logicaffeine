//! ★ LOCK — a LOGOS program can open a VERIFIED TLS connection and speak a protocol over it. ★
//!
//! `logicaffeine_system`'s `tcp_tls_lock` proves the Rust layer. This proves the half that
//! matters to somebody writing LOGOS: `tcpConnectTls` is a native the language can call, it
//! answers the same opaque handle `tcpConnect` does, and every other socket native works on it
//! without knowing it is secured.
//!
//! ★ WHY THIS WAS THE LARGEST GAP IN THE FRAMEWORK. There was no TLS anywhere, so `pgConnect`
//! spoke cleartext over plain TCP and could reach **no managed PostgreSQL at all** — RDS, Neon,
//! Supabase and Cloud SQL each require it — and no LOGOS program could call an HTTPS API, which
//! rules out every payment processor, OAuth endpoint, object store and webhook. The live Postgres
//! proof we had ran against a locally-booted PG16, which is exactly the environment where the
//! absence is invisible.
//!
//! ★ TWO REGISTRIES, ONE POLICY. The interpreter and VM drive sockets through tokio; the AOT tier
//! drives them synchronously. Both read `logicaffeine_system::tcp::tls_config`, so which
//! certificates are accepted cannot differ between the tier an author tests on and the tier they
//! ship. This file exercises the async one.

use std::io::Write as _;
use std::sync::Arc;
use std::time::Duration;

use logicaffeine_compile::interpret_for_ui;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn run(program: &str) -> logicaffeine_compile::interpreter::InterpreterResult {
    tokio::time::timeout(Duration::from_secs(20), interpret_for_ui(program))
        .await
        .expect("the program must not hang")
}

/// A TLS echo server on an ephemeral port. Returns its port and the PEM of its own certificate,
/// which the caller may trust through `LOGOS_TLS_EXTRA_ROOT`.
async fn tls_echo_server() -> (u16, String) {
    // ★ THE SERVER SIDE NEEDS THE PROVIDER TOO, and it is built BEFORE anything calls into
    // `logicaffeine_system::tcp::tls_config` (which installs one for the client). With more than
    // one crypto backend reachable — and Cargo feature unification guarantees that here, because
    // some other crate in this workspace's graph enables `aws-lc-rs` — `ServerConfig::builder()`
    // panics rather than choosing. Installing is idempotent: it errors only if one is already set.
    let _ = rustls::crypto::ring::default_provider().install_default();
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
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(cfg));

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut s) = acceptor.accept(sock).await else {
                    return;
                };
                let mut buf = [0u8; 4096];
                loop {
                    match s.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if s.write_all(&buf[..n]).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        }
    });
    (port, pem)
}

/// ★ ONE TRUSTED SERVER PER PROCESS, because the client config is built once per process and
/// cached. My first version stood up a fresh server with a fresh self-signed certificate in each
/// test and trusted it — which works under nextest (a process per test) and fails under
/// `cargo test` (one process, threads): the second server's certificate is checked against the
/// FIRST one's root and the handshake dies with `BadSignature`.
///
/// The comment on this file predicted exactly that hazard and I wrote the code that hits it
/// anyway. Sharing one server and one root is correct under BOTH runners, which is what a test
/// should be — a test that only passes under one runner is a test that will be run under the
/// other one day.
/// ★ THE SERVER OWNS ITS OWN THREAD AND RUNTIME, and the two failures that led here are the
/// point.
///
/// First attempt: a fresh server per test, each with its own self-signed certificate, trusted
/// through `LOGOS_TLS_EXTRA_ROOT`. That works under nextest (a process per test) and fails under
/// `cargo test` (one process): the client config is built ONCE per process and cached, so the
/// second server's certificate is checked against the first server's root — `BadSignature`.
///
/// Second attempt: one shared server behind a `OnceCell`. But every `#[tokio::test]` builds its
/// OWN runtime and drops it at the end of the test, taking the spawned server with it — so the
/// next test got `Connection refused`.
///
/// A server that must outlive any one test cannot live on any one test's runtime. It gets a
/// dedicated OS thread with a runtime of its own, started once, and the port comes back over a
/// channel. Correct under both runners, which is what a test has to be.
fn shared_server() -> u16 {
    use std::sync::OnceLock;
    static PORT: OnceLock<u16> = OnceLock::new();
    *PORT.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("server runtime");
            rt.block_on(async move {
                let (port, pem) = tls_echo_server().await;
                let dir = Box::leak(Box::new(tempfile::tempdir().expect("dir")));
                let ca = dir.path().join("ca.pem");
                let mut f = std::fs::File::create(&ca).expect("create");
                f.write_all(pem.as_bytes()).expect("write");
                std::env::set_var("LOGOS_TLS_EXTRA_ROOT", &ca);
                tx.send(port).expect("hand the port back");
                // Hold the runtime open so the accept loop keeps serving every test.
                std::future::pending::<()>().await;
            });
        });
        rx.recv().expect("the server starts")
    })
}

#[tokio::test]
async fn a_logos_program_speaks_over_a_verified_tls_connection() {
    let port = shared_server();

    // The handle is ordinary: `tcpSend`/`tcpRecvExact`/`tcpClose` do not know it is secured.
    let src = format!(
        "## Main\n\
         Inspect tcpConnectTls(\"localhost\", {port}):\n\
        \x20   When Ok (c):\n\
        \x20       Inspect tcpSend(c, [104, 105]):\n\
        \x20           When Ok (n):\n\
        \x20               Show n.\n\
        \x20           When Err (e):\n\
        \x20               Show e.\n\
        \x20       Inspect tcpRecvExact(c, 2):\n\
        \x20           When Ok (bs):\n\
        \x20               Show item 1 of bs.\n\
        \x20               Show item 2 of bs.\n\
        \x20           When Err (e):\n\
        \x20               Show e.\n\
        \x20       Inspect tcpClose(c):\n\
        \x20           When Ok (u):\n\
        \x20               Show \"closed\".\n\
        \x20           When Err (e):\n\
        \x20               Show e.\n\
        \x20   When Err (e):\n\
        \x20       Show e.\n"
    );
    let out = run(&src).await;
    assert_eq!(
        out.error, None,
        "the program must run cleanly: {:?}",
        out.error
    );
    assert_eq!(
        out.lines,
        vec!["2", "104", "105", "closed"],
        "two bytes sent, the same two echoed back through the session, closed cleanly"
    );
}

#[tokio::test]
async fn an_untrusted_certificate_is_an_err_a_program_can_read() {
    // ★ THE SECURITY PROPERTY, AT THE LANGUAGE LEVEL. No root is trusted here, so the handshake
    // must fail — and it must fail as a VALUE, because every other socket native answers a
    // `Result` and a trap would be unhandleable.
    let (port, _pem) = tls_echo_server().await;
    let src = format!(
        "## Main\n\
         Inspect tcpConnectTls(\"localhost\", {port}):\n\
        \x20   When Ok (c):\n\
        \x20       Show \"connected\".\n\
        \x20   When Err (e):\n\
        \x20       Show \"refused\".\n"
    );
    let out = run(&src).await;
    assert_eq!(
        out.error, None,
        "a refused handshake is a value, not a trap: {:?}",
        out.error
    );
    assert_eq!(
        out.lines,
        vec!["refused"],
        "an unverifiable certificate must not connect"
    );
}

#[tokio::test]
async fn upgrading_a_plain_connection_in_place_is_how_postgres_negotiates() {
    // `tcpUpgradeTls` exists because a connect-with-TLS call cannot express STARTTLS-shaped
    // negotiation. Here the server is TLS from the first byte, so the upgrade happens immediately
    // after connect — the same two steps PostgreSQL takes around its SSLRequest byte.
    let port = shared_server();
    let src = format!(
        "## Main\n\
         Inspect tcpConnect(\"localhost\", {port}):\n\
        \x20   When Ok (c):\n\
        \x20       Inspect tcpUpgradeTls(c, \"localhost\"):\n\
        \x20           When Ok (u):\n\
        \x20               Inspect tcpSend(c, [55]):\n\
        \x20                   When Ok (n):\n\
        \x20                       Show \"secured\".\n\
        \x20                   When Err (e):\n\
        \x20                       Show e.\n\
        \x20           When Err (e):\n\
        \x20               Show e.\n\
        \x20   When Err (e):\n\
        \x20       Show e.\n"
    );
    let out = run(&src).await;
    assert_eq!(out.error, None, "clean run: {:?}", out.error);
    assert_eq!(
        out.lines,
        vec!["secured"],
        "a plain connection became a session in place"
    );
}
