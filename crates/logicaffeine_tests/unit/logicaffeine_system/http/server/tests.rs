use super::*;
use std::collections::BTreeMap;

#[test]
fn read_error_response_maps_body_too_large_to_413_and_the_rest_to_400() {
    // The one place the 413-vs-400 split is decided: a too-large BODY is a
    // 413 (understood-but-refused), everything else — including a too-large
    // HEADER block, whose head cannot even be parsed — is the malformed 400
    // (the shape the cross-tier http_differential lock pins).
    assert_eq!(read_error_response(&ParseError::BodyTooLarge).status, 413);
    assert_eq!(
        read_error_response(&ParseError::HeadersTooLarge).status,
        400
    );
    assert_eq!(read_error_response(&ParseError::Incomplete).status, 400);
    assert_eq!(
        read_error_response(&ParseError::BadRequest("x".into())).status,
        400
    );
    assert_eq!(
        read_error_response(&ParseError::UnsupportedTransferEncoding).status,
        400
    );
    assert_eq!(payload_too_large().status, 413);
}

#[test]
fn declared_content_length_peeks_only_after_the_header_terminator() {
    // No terminator yet → None (the value is not trustworthy mid-flight).
    assert_eq!(
        declared_content_length(b"POST / HTTP/1.1\r\nContent-Length: 5\r\n"),
        None
    );
    // Terminator present → the declared length.
    assert_eq!(
        declared_content_length(b"POST / HTTP/1.1\r\nContent-Length: 5\r\n\r\nhello"),
        Some(5)
    );
    // Case-insensitive header name.
    assert_eq!(
        declared_content_length(b"POST / HTTP/1.1\r\ncontent-length: 42\r\n\r\n"),
        Some(42)
    );
    // Absent length after a complete head → None (an empty body).
    assert_eq!(
        declared_content_length(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n"),
        None
    );
    // A non-numeric length → None (the ordinary parse then answers 400).
    assert_eq!(
        declared_content_length(b"POST / HTTP/1.1\r\nContent-Length: big\r\n\r\n"),
        None
    );
}

#[test]
fn port_override_prefers_logos_port_then_port_then_none() {
    assert_eq!(port_override(Some("3000"), Some("4000")), Some(3000));
    assert_eq!(port_override(None, Some("4000")), Some(4000));
    assert_eq!(port_override(None, None), None);
    // Whitespace-tolerant; garbage and out-of-range values are ignored,
    // falling through to the next source rather than erroring.
    assert_eq!(port_override(Some(" 8081 "), None), Some(8081));
    assert_eq!(port_override(Some("nope"), Some("4000")), Some(4000));
    assert_eq!(port_override(Some("70000"), None), None);
    assert_eq!(port_override(Some(""), Some("")), None);
}

/// ★ The pure law behind [`env_bind_address`]. A deployed binary that binds
/// loopback is unreachable from outside its own network namespace, so the
/// host has to be an input — and it has to be ONE input, composed in ONE
/// place, or the interpreted and compiled tiers bind different addresses
/// from the same environment.
#[test]
fn bind_address_composes_the_host_and_the_port() {
    // The default is loopback: a dev box does not become a public server
    // because somebody ran a program.
    assert_eq!(bind_address(None, 8080), "127.0.0.1:8080");
    // The one line that makes a container reachable.
    assert_eq!(bind_address(Some("0.0.0.0"), 8080), "0.0.0.0:8080");
    // A specific interface is as ordinary as a wildcard.
    assert_eq!(bind_address(Some("192.168.1.9"), 80), "192.168.1.9:80");
    // Whitespace-tolerant, like every other environment reader here.
    assert_eq!(bind_address(Some("  0.0.0.0 "), 8080), "0.0.0.0:8080");
    // An EMPTY value means "unset", not "bind the empty host" — an unset
    // variable in a shell script expands to exactly this.
    assert_eq!(bind_address(Some(""), 8080), "127.0.0.1:8080");
    assert_eq!(bind_address(Some("   "), 8080), "127.0.0.1:8080");
}

/// ★ An IPv6 host MUST be bracketed. `::` and a port concatenated with a
/// colon is not an address at all (`:::8080` parses as nothing), so a
/// dual-stack deployment would fail to bind with a message about the
/// address rather than about the missing brackets.
#[test]
fn bind_address_brackets_an_ipv6_host() {
    assert_eq!(bind_address(Some("::"), 8080), "[::]:8080");
    assert_eq!(bind_address(Some("::1"), 8080), "[::1]:8080");
    assert_eq!(bind_address(Some("fe80::1"), 443), "[fe80::1]:443");
    // Already bracketed by the author: accepted as written, not doubled.
    assert_eq!(bind_address(Some("[::]"), 8080), "[::]:8080");
    // A hostname carries no colon and is left alone.
    assert_eq!(bind_address(Some("localhost"), 8080), "localhost:8080");
}

/// A tiny raw one-shot HTTP client: open a fresh connection, send `raw`,
/// half-close the write side (the EOF tells the keep-alive server this
/// client is done), and read the whole response to EOF.
async fn roundtrip(addr: SocketAddr, raw: &[u8]) -> Vec<u8> {
    let mut s = TcpStream::connect(addr).await.expect("connect");
    s.write_all(raw).await.expect("write request");
    s.flush().await.expect("flush");
    s.shutdown().await.expect("shutdown write half");
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.expect("read response");
    buf
}

fn text_ok(body: &str) -> HttpResponse {
    let mut headers = BTreeMap::new();
    headers.insert("Content-Type".to_string(), "text/plain".to_string());
    HttpResponse {
        status: 200,
        headers,
        body: body.as_bytes().to_vec(),
    }
}

#[tokio::test]
async fn serve_answers_a_get_with_the_handlers_response() {
    // The handler echoes the request path — proves request→handler→response.
    let server = spawn("127.0.0.1:0", |req| {
        text_ok(&format!("you asked for {}", req.path))
    })
    .await
    .expect("server binds");

    let resp = roundtrip(server.addr(), b"GET /hello HTTP/1.1\r\nHost: x\r\n\r\n").await;
    let text = String::from_utf8(resp).unwrap();
    assert!(
        text.starts_with("HTTP/1.1 200 OK\r\n"),
        "status line: {text:?}"
    );
    assert!(text.contains("Content-Type: text/plain\r\n"));
    assert!(text.ends_with("you asked for /hello"), "body: {text:?}");
}

#[tokio::test]
async fn serve_reads_the_post_body() {
    let server = spawn("127.0.0.1:0", |req| {
        text_ok(&format!("echo:{}", String::from_utf8_lossy(&req.body)))
    })
    .await
    .expect("server binds");

    let resp = roundtrip(
        server.addr(),
        b"POST /submit HTTP/1.1\r\nContent-Length: 5\r\n\r\nhello",
    )
    .await;
    let text = String::from_utf8(resp).unwrap();
    assert!(text.ends_with("echo:hello"), "body: {text:?}");
}

#[tokio::test]
async fn serve_handles_sequential_requests() {
    // Each request is its own connection. A counter in the handler proves
    // both were served by the same loop.
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let n = Arc::new(AtomicUsize::new(0));
    let n2 = n.clone();
    let server = spawn("127.0.0.1:0", move |_req| {
        let i = n2.fetch_add(1, Ordering::SeqCst);
        text_ok(&format!("req#{i}"))
    })
    .await
    .expect("server binds");

    let a = roundtrip(server.addr(), b"GET /a HTTP/1.1\r\n\r\n").await;
    let b = roundtrip(server.addr(), b"GET /b HTTP/1.1\r\n\r\n").await;
    assert!(String::from_utf8(a).unwrap().ends_with("req#0"));
    assert!(String::from_utf8(b).unwrap().ends_with("req#1"));
    assert_eq!(n.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn serve_answers_400_on_malformed_request() {
    let server = spawn("127.0.0.1:0", |_req| text_ok("ok"))
        .await
        .expect("binds");
    let resp = roundtrip(server.addr(), b"GARBAGE\r\n\r\n").await;
    let text = String::from_utf8(resp).unwrap();
    assert!(
        text.starts_with("HTTP/1.1 400 Bad Request\r\n"),
        "got: {text:?}"
    );
}

#[tokio::test]
async fn low_level_listener_and_conn_roundtrip() {
    // The primitives the interpreter drives inline: bind → accept →
    // read_request → write_response.
    let listener = Listener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");

    let client = tokio::spawn(async move { roundtrip(addr, b"GET /low HTTP/1.1\r\n\r\n").await });

    let mut conn = listener.accept().await.expect("accept");
    let req = conn.read_request().await.expect("read");
    assert_eq!(req.method, "GET");
    assert_eq!(req.path, "/low");
    conn.write_response(&text_ok("done")).await.expect("write");
    drop(conn);

    let resp = client.await.unwrap();
    assert!(String::from_utf8(resp).unwrap().ends_with("done"));
}

/// Read exactly one `Content-Length`-framed response off `s` — the client
/// side of keep-alive (read_to_end would block while the server holds the
/// connection open).
async fn read_one_response(s: &mut TcpStream) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if super::super::wire::parse_response(&buf).is_ok() {
            return buf;
        }
        let n = s.read(&mut chunk).await.expect("read response bytes");
        assert!(n > 0, "server closed mid-response; got {buf:?}");
        buf.extend_from_slice(&chunk[..n]);
    }
}

#[tokio::test]
async fn keep_alive_serves_two_requests_down_one_connection() {
    let server = spawn("127.0.0.1:0", |req| text_ok(&format!("path={}", req.path)))
        .await
        .expect("server binds");

    let mut s = TcpStream::connect(server.addr()).await.expect("connect");
    // First request: HTTP/1.1 default persistence — the response says keep-alive.
    s.write_all(b"GET /one HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .expect("write 1");
    let first = read_one_response(&mut s).await;
    let first_text = String::from_utf8(first).unwrap();
    assert!(
        first_text.contains("Connection: keep-alive\r\n"),
        "first: {first_text:?}"
    );
    assert!(first_text.ends_with("path=/one"), "first: {first_text:?}");

    // Second request on the SAME socket: opt out, server closes after.
    s.write_all(b"GET /two HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .expect("write 2");
    let mut rest = Vec::new();
    s.read_to_end(&mut rest).await.expect("read to close");
    let second_text = String::from_utf8(rest).unwrap();
    assert!(
        second_text.contains("Connection: close\r\n"),
        "second: {second_text:?}"
    );
    assert!(
        second_text.ends_with("path=/two"),
        "second: {second_text:?}"
    );
}

#[tokio::test]
async fn keep_alive_serves_pipelined_requests_in_order() {
    // Both requests arrive in ONE write; the connection buffer must frame
    // them separately and answer both, in order.
    let server = spawn("127.0.0.1:0", |req| text_ok(&format!("p={}", req.path)))
        .await
        .expect("server binds");

    let mut s = TcpStream::connect(server.addr()).await.expect("connect");
    s.write_all(
        b"GET /first HTTP/1.1\r\nHost: x\r\n\r\nGET /second HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    )
    .await
    .expect("write both");
    let mut all = Vec::new();
    s.read_to_end(&mut all).await.expect("read both");
    let text = String::from_utf8(all).unwrap();
    let first_at = text.find("p=/first").expect("first response present");
    let second_at = text.find("p=/second").expect("second response present");
    assert!(first_at < second_at, "responses out of order: {text:?}");
}

#[tokio::test]
async fn a_stalled_connection_does_not_block_serve_on() {
    // Connection A promises a body and stalls; connection B must still be
    // answered — connections are concurrent tasks.
    let server = spawn("127.0.0.1:0", |req| text_ok(&format!("p={}", req.path)))
        .await
        .expect("server binds");

    let mut stalled = TcpStream::connect(server.addr()).await.expect("connect A");
    stalled
        .write_all(b"POST /slow HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\n")
        .await
        .expect("write stalled head");

    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        roundtrip(server.addr(), b"GET /fast HTTP/1.1\r\nHost: x\r\n\r\n"),
    )
    .await
    .expect("B must complete while A is stalled");
    assert!(String::from_utf8(resp).unwrap().ends_with("p=/fast"));
    drop(stalled);
}

#[tokio::test]
async fn a_handler_panic_is_a_500_and_serve_on_survives() {
    let server = spawn("127.0.0.1:0", |req| {
        if req.path == "/boom" {
            panic!("handler exploded");
        }
        text_ok(&format!("p={}", req.path))
    })
    .await
    .expect("server binds");

    let boom = roundtrip(server.addr(), b"GET /boom HTTP/1.1\r\nHost: x\r\n\r\n").await;
    let boom_text = String::from_utf8(boom).unwrap();
    assert!(
        boom_text.starts_with("HTTP/1.1 500 Internal Server Error\r\n"),
        "panicking handler must answer 500: {boom_text:?}"
    );
    assert!(
        boom_text.ends_with("internal error"),
        "500 body: {boom_text:?}"
    );
    assert!(
        !boom_text.contains("exploded"),
        "the failure detail must never reach the wire: {boom_text:?}"
    );

    let after = roundtrip(server.addr(), b"GET /alive HTTP/1.1\r\nHost: x\r\n\r\n").await;
    assert!(
        String::from_utf8(after).unwrap().ends_with("p=/alive"),
        "server must keep serving after a handler panic"
    );
}

/// An `UpgradeHandler` that must never be called — dropping into it fails the test loudly
/// (a panic inside a spawned task surfaces as the assertion never seeing its expected value,
/// but the message makes the actual cause obvious in the log rather than a silent hang).
fn unreachable_upgrade(why: &'static str) -> UpgradeHandler {
    Arc::new(move |_stream, _prefix| Box::pin(async move { panic!("{why}") }))
}

#[tokio::test]
async fn serve_on_with_leaves_an_ordinary_request_unaffected_by_a_registered_upgrade_path() {
    let listener = Listener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let upgrade =
        unreachable_upgrade("an ordinary GET to another path must never reach the upgrade handler");
    tokio::spawn(serve_on_with(
        listener,
        |req: HttpRequest| text_ok(&format!("p={}", req.path)),
        vec![("/_relay", upgrade)],
    ));

    let resp = roundtrip(addr, b"GET /hello HTTP/1.1\r\nHost: x\r\n\r\n").await;
    let text = String::from_utf8(resp).unwrap();
    assert!(
        text.starts_with("HTTP/1.1 200 OK\r\n"),
        "status line: {text:?}"
    );
    assert!(text.ends_with("p=/hello"), "body: {text:?}");
}

/// The exact shape `largo serve` uses (`serve_on_shared_with`, the `Fn + Sync` entry) —
/// registering `/_relay` must not perturb any request the ordinary handler answers.
#[tokio::test]
async fn serve_on_shared_with_leaves_an_ordinary_request_unaffected_by_a_registered_upgrade_path() {
    let listener = Listener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let upgrade =
        unreachable_upgrade("an ordinary GET to another path must never reach the upgrade handler");
    tokio::spawn(serve_on_shared_with(
        listener,
        |req: HttpRequest| text_ok(&format!("p={}", req.path)),
        vec![("/_relay", upgrade)],
    ));

    let a = roundtrip(addr, b"GET /a HTTP/1.1\r\nHost: x\r\n\r\n").await;
    let b = roundtrip(addr, b"GET /b HTTP/1.1\r\nHost: x\r\n\r\n").await;
    assert!(String::from_utf8(a).unwrap().ends_with("p=/a"));
    assert!(String::from_utf8(b).unwrap().ends_with("p=/b"));
}

/// A bare request with `path`/`method` plus the given headers (already lowercase-keyed, matching
/// what the real wire parser guarantees — see [`matching_upgrade`]'s own doc comment).
fn req(method: &str, path: &str, headers: &[(&str, &str)]) -> HttpRequest {
    HttpRequest {
        method: method.to_string(),
        path: path.to_string(),
        query: Default::default(),
        headers: headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        body: Vec::new(),
    }
}

fn some_upgrades() -> BTreeMap<String, UpgradeHandler> {
    [(
        "/_relay".to_string(),
        unreachable_upgrade("try_upgrade is a pure decision — it never calls the handler"),
    )]
    .into_iter()
    .collect()
}

/// [`try_upgrade`] is `None` — proceed to the ordinary handler — for every shape of request that
/// is not actually asking to upgrade a registered path: the wrong method, no `Upgrade: websocket`
/// header, and a path nothing is registered for. Pure and synchronous, so this needs no server.
#[test]
fn try_upgrade_is_none_for_anything_that_is_not_a_real_upgrade_request() {
    let upgrades = some_upgrades();
    assert!(
        try_upgrade(&req("GET", "/_relay", &[]), &upgrades).is_none(),
        "no Upgrade header at all"
    );
    assert!(
        try_upgrade(&req("GET", "/_relay", &[("upgrade", "chunked")]), &upgrades).is_none(),
        "an Upgrade header naming something other than websocket"
    );
    assert!(
        try_upgrade(
            &req("POST", "/_relay", &[("upgrade", "websocket")]),
            &upgrades
        )
        .is_none(),
        "not a GET"
    );
    assert!(
        try_upgrade(
            &req("GET", "/not-registered", &[("upgrade", "websocket")]),
            &upgrades
        )
        .is_none(),
        "a path nothing is registered for"
    );
}

/// [`matching_upgrade`] (reached through [`try_upgrade`]) gates on the `Upgrade: websocket`
/// header, not just the path — a plain `GET /_relay` (no such header) is an ordinary request and
/// must reach the ordinary handler. Kept as a real end-to-end test (not just `try_upgrade` in
/// isolation) because this proves `serve_connection` itself takes the `None` branch correctly.
#[tokio::test]
async fn a_get_to_an_upgrade_path_without_the_upgrade_header_reaches_the_ordinary_handler() {
    let listener = Listener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let upgrade =
        unreachable_upgrade("no Upgrade header was sent; the ordinary handler must answer instead");
    tokio::spawn(serve_on_with(
        listener,
        |req: HttpRequest| text_ok(&format!("ordinary:{}", req.path)),
        vec![("/_relay", upgrade)],
    ));

    let resp = roundtrip(addr, b"GET /_relay HTTP/1.1\r\nHost: x\r\n\r\n").await;
    let text = String::from_utf8(resp).unwrap();
    assert!(
        text.starts_with("HTTP/1.1 200 OK\r\n"),
        "status line: {text:?}"
    );
    assert!(text.ends_with("ordinary:/_relay"), "body: {text:?}");
}

/// [`websocket_accept_key`] only computes a value behind the `relay` feature (see its own
/// doc comment: without it, [`matching_upgrade`]'s result is moot) — every test below that
/// exercises a completed handshake needs the feature to mean anything.
#[cfg(feature = "relay")]
mod relay_handshake {
    use super::*;

    /// RFC 6455 §1.3's own worked example: the request key `dGhlIHNhbXBsZSBub25jZQ==` derives
    /// the response key `s3pPLMBiTxaQ9kYGzzhZRbK+xOo=` — checked against the SPEC value, not
    /// merely "some string appeared", so a broken derivation cannot pass by accident. Pure and
    /// synchronous: [`try_upgrade`] is the whole decision, no server needed to exercise it.
    #[test]
    fn try_upgrade_answers_the_rfc6455_worked_example_exactly() {
        let upgrades = some_upgrades();
        let request = req(
            "GET",
            "/_relay",
            &[
                ("upgrade", "websocket"),
                ("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ=="),
            ],
        );
        let (response, _handler) = try_upgrade(&request, &upgrades)
            .expect("a match")
            .expect("a completed handshake");
        assert!(
            response.starts_with("HTTP/1.1 101 Switching Protocols\r\n"),
            "{response:?}"
        );
        assert!(response.contains("Upgrade: websocket\r\n"), "{response:?}");
        assert_eq!(
            response, "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\n",
            "the RFC 6455 worked example's own accept key must appear verbatim"
        );
    }

    /// [`websocket_accept_key`] returns `None` with no `Sec-WebSocket-Key` header, and
    /// [`try_upgrade`] turns that into `Some(Err(()))` — a match that cannot complete, never a
    /// silent `None` that would fall through to the ordinary handler.
    #[test]
    fn try_upgrade_rejects_a_handshake_missing_the_key() {
        let upgrades = some_upgrades();
        let request = req("GET", "/_relay", &[("upgrade", "websocket")]);
        assert!(matches!(try_upgrade(&request, &upgrades), Some(Err(()))));
    }

    /// Once the 101 is written, the socket belongs to the upgrade handler alone — every byte the
    /// client sends afterward must reach it, whether the OS delivered it pipelined with the
    /// handshake `GET` (folded into [`Conn::into_raw_parts`]'s leftover prefix) or on a later
    /// read of the handed-off socket. The handler here echoes back everything it ever sees, and
    /// the client reads to EOF (after half-closing its write side) so the assertion holds
    /// regardless of how the bytes were chunked in transit.
    #[tokio::test]
    async fn an_upgrade_handler_receives_every_byte_the_client_sent_after_the_handshake() {
        let listener = Listener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let upgrade: UpgradeHandler = Arc::new(|mut stream, prefix| {
            Box::pin(async move {
                let mut all = prefix;
                let mut buf = [0u8; 256];
                loop {
                    match stream.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => all.extend_from_slice(&buf[..n]),
                    }
                }
                let _ = stream.write_all(&all).await;
                let _ = stream.shutdown().await;
            })
        });
        tokio::spawn(serve_on_with(
            listener,
            |_req: HttpRequest| text_ok("unused"),
            vec![("/_relay", upgrade)],
        ));

        let mut s = TcpStream::connect(addr).await.expect("connect");
        // The handshake AND a payload past it travel in ONE write — on loopback these usually
        // land in a single read, exercising the prefix path; the handler's loop makes the
        // assertion correct even if the OS splits them across reads instead.
        s.write_all(
            b"GET /_relay HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
              Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\nPIPELINED-PAYLOAD",
        )
        .await
        .expect("write handshake + payload");
        s.shutdown().await.expect("half-close the write side");

        let mut all = Vec::new();
        s.read_to_end(&mut all).await.expect("read to EOF");
        let text = String::from_utf8(all).unwrap();
        let body_at = text
            .find("\r\n\r\n")
            .map(|i| i + 4)
            .expect("handshake response present");
        assert_eq!(
            &text[body_at..],
            "PIPELINED-PAYLOAD",
            "the upgrade handler must see the payload byte-for-byte: {text:?}"
        );
    }

    /// [`websocket_accept_key`] returns `None` with no `Sec-WebSocket-Key` header, and
    /// `serve_connection` treats that as "cannot complete this handshake" — the connection is
    /// dropped with NO response at all, never a fallback answer from the ordinary handler and
    /// never a generic error page. Documented on [`UpgradeHandler`] as the thing that actually
    /// gates the behaviour; this pins it so a future change to that law is a deliberate one.
    #[tokio::test]
    async fn an_upgrade_request_missing_the_key_gets_no_response_and_the_connection_closes() {
        let listener = Listener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let upgrade = unreachable_upgrade(
            "a handshake with no Sec-WebSocket-Key must never reach the handler",
        );
        tokio::spawn(serve_on_with(
            listener,
            |_req: HttpRequest| text_ok("unused"),
            vec![("/_relay", upgrade)],
        ));

        let mut s = TcpStream::connect(addr).await.expect("connect");
        s.write_all(
            b"GET /_relay HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n",
        )
        .await
        .expect("write");
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).await.expect("read to EOF");
        assert!(
            buf.is_empty(),
            "a malformed handshake (no key) must get NO response: {buf:?}"
        );
    }
}

/// [`roundtrip`] once something listens on `addr`, retrying the connect while the server binds.
async fn roundtrip_when_up(addr: SocketAddr, raw: &[u8]) -> Vec<u8> {
    for _ in 0..200 {
        if TcpStream::connect(addr).await.is_ok() {
            return roundtrip(addr, raw).await;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("nothing listened on {addr}");
}

#[tokio::test]
async fn serve_here_runs_a_handler_that_cannot_leave_its_thread() {
    // `Rc` makes the handler neither `Send` nor `Sync`: it can only run where it was made, and the
    // test's runtime is a current-thread one, so the engine cannot be sharing this thread either.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let served = std::rc::Rc::new(std::cell::Cell::new(0));
    let count = std::rc::Rc::clone(&served);
    let bind = addr.to_string();
    let server = serve_here(&bind, move |req| {
        if req.path == "/boom" {
            panic!("handler exploded");
        }
        count.set(count.get() + 1);
        text_ok(&format!("{} #{}", req.path, count.get()))
    });
    let client = async {
        let first = roundtrip_when_up(addr, b"GET /a HTTP/1.1\r\nHost: x\r\n\r\n").await;
        let boom = roundtrip(addr, b"GET /boom HTTP/1.1\r\nHost: x\r\n\r\n").await;
        let second = roundtrip(addr, b"GET /b HTTP/1.1\r\nHost: x\r\n\r\n").await;
        (first, boom, second)
    };
    let (first, boom, second) = tokio::select! {
        outcome = server => panic!("the server stopped serving: {outcome:?}"),
        answers = client => answers,
    };
    assert!(String::from_utf8(first).unwrap().ends_with("/a #1"));
    let boom = String::from_utf8(boom).unwrap();
    assert!(
        boom.starts_with("HTTP/1.1 500 Internal Server Error\r\n"),
        "a panicking call answers 500: {boom:?}"
    );
    assert!(
        !boom.contains("exploded"),
        "the failure detail never reaches the wire: {boom:?}"
    );
    assert!(
        String::from_utf8(second).unwrap().ends_with("/b #2"),
        "the handler's own state carried across calls"
    );
    assert_eq!(served.get(), 2);
}
