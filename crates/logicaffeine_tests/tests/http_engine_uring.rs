//! The io_uring HTTP engine battery — mirrors the tokio `server.rs` battery
//! law for law (correct responses, keep-alive, pipelining, `Connection: close`,
//! parse-error 400, panic 500 + survive, stall concurrency, in-flight cap) and
//! adds the engine-only surfaces: the pre-serialized static-response fast path
//! and the byte-parity differential against the tokio engine on the same box.
//!
//! Deterministic: loopback sockets, no sleeps except the stall simulation, and
//! every engine is constructed with an explicit [`EngineConfig`] (no env reads,
//! so parallel tests cannot race on process-global state).
#![cfg(all(target_os = "linux", feature = "http", feature = "io-uring"))]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use logicaffeine_system::http::engine::StaticResponses;
use logicaffeine_system::http::engine_uring::{EngineConfig, UringServer};
use logicaffeine_system::http::wire::HttpResponse;

fn text_ok(body: &str) -> HttpResponse {
    let mut headers = BTreeMap::new();
    headers.insert("Content-Type".to_string(), "text/plain".to_string());
    HttpResponse {
        status: 200,
        headers,
        body: body.as_bytes().to_vec(),
    }
}

fn one_shard() -> EngineConfig {
    EngineConfig {
        shards: 1,
        max_inflight: 64,
    }
}

fn echo_engine(cfg: EngineConfig) -> UringServer {
    UringServer::bind(
        "127.0.0.1:0",
        cfg,
        Arc::new(|req: logicaffeine_system::http::wire::HttpRequest| {
            text_ok(&format!("p={}", req.path))
        }),
        None,
    )
    .expect("engine binds")
}

/// One-shot raw exchange: connect, send `raw`, half-close the write side, read
/// to EOF — the same client shape as the tokio battery's `roundtrip`.
fn roundtrip(addr: std::net::SocketAddr, raw: &[u8]) -> Vec<u8> {
    let mut s = TcpStream::connect(addr).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    s.write_all(raw).expect("write request");
    s.shutdown(std::net::Shutdown::Write)
        .expect("shutdown write half");
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).expect("read response");
    buf
}

/// The total frame length (header block + `Content-Length` body) of the
/// response at the head of `buf`, once the header block is complete.
fn framed_len(buf: &[u8]) -> Option<usize> {
    let sep = buf.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
    let head = String::from_utf8_lossy(&buf[..sep]);
    let mut content_length = 0usize;
    for line in head.split("\r\n") {
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    Some(sep + content_length)
}

/// Read exactly one `Content-Length`-framed response — the client side of
/// keep-alive, where the server (correctly) holds the socket open.
fn read_one_response(s: &mut TcpStream) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        if let Some(total) = framed_len(&buf) {
            if buf.len() >= total {
                buf.truncate(total);
                return buf;
            }
        }
        let n = s.read(&mut chunk).expect("read response bytes");
        assert!(
            n > 0,
            "server closed mid-response; got {:?}",
            String::from_utf8_lossy(&buf)
        );
        buf.extend_from_slice(&chunk[..n]);
    }
}

#[test]
fn engine_answers_a_get_with_the_handlers_response() {
    let server = echo_engine(one_shard());
    let resp = roundtrip(
        server.local_addr(),
        b"GET /hello HTTP/1.1\r\nHost: x\r\n\r\n",
    );
    let text = String::from_utf8(resp).unwrap();
    assert!(
        text.starts_with("HTTP/1.1 200 OK\r\n"),
        "status line: {text:?}"
    );
    assert!(text.contains("Content-Type: text/plain\r\n"));
    assert!(text.ends_with("p=/hello"), "body: {text:?}");
}

#[test]
fn engine_reads_the_post_body() {
    let server = UringServer::bind(
        "127.0.0.1:0",
        one_shard(),
        Arc::new(|req: logicaffeine_system::http::wire::HttpRequest| {
            text_ok(&format!("echo:{}", String::from_utf8_lossy(&req.body)))
        }),
        None,
    )
    .expect("engine binds");
    let resp = roundtrip(
        server.local_addr(),
        b"POST /submit HTTP/1.1\r\nContent-Length: 5\r\n\r\nhello",
    );
    assert!(String::from_utf8(resp).unwrap().ends_with("echo:hello"));
}

#[test]
fn engine_keep_alive_serves_two_requests_down_one_connection() {
    let server = echo_engine(one_shard());
    let mut s = TcpStream::connect(server.local_addr()).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();

    s.write_all(b"GET /one HTTP/1.1\r\nHost: x\r\n\r\n")
        .expect("write 1");
    let first = String::from_utf8(read_one_response(&mut s)).unwrap();
    assert!(
        first.contains("Connection: keep-alive\r\n"),
        "first: {first:?}"
    );
    assert!(first.ends_with("p=/one"), "first: {first:?}");

    s.write_all(b"GET /two HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .expect("write 2");
    let mut rest = Vec::new();
    s.read_to_end(&mut rest).expect("read to close");
    let second = String::from_utf8(rest).unwrap();
    assert!(
        second.contains("Connection: close\r\n"),
        "second: {second:?}"
    );
    assert!(second.ends_with("p=/two"), "second: {second:?}");
}

#[test]
fn engine_serves_pipelined_requests_in_order() {
    let server = echo_engine(one_shard());
    let mut s = TcpStream::connect(server.local_addr()).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    s.write_all(
        b"GET /first HTTP/1.1\r\nHost: x\r\n\r\nGET /second HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    )
    .expect("write both");
    let mut all = Vec::new();
    s.read_to_end(&mut all).expect("read both");
    let text = String::from_utf8(all).unwrap();
    let first_at = text.find("p=/first").expect("first response present");
    let second_at = text.find("p=/second").expect("second response present");
    assert!(first_at < second_at, "responses out of order: {text:?}");
}

#[test]
fn engine_connection_close_is_honored_with_an_actual_close() {
    let server = echo_engine(one_shard());
    let mut s = TcpStream::connect(server.local_addr()).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    // No half-close from the client: the SERVER must close after answering.
    s.write_all(b"GET /bye HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .expect("write");
    let mut all = Vec::new();
    s.read_to_end(&mut all).expect("read to server close");
    let text = String::from_utf8(all).unwrap();
    assert!(text.contains("Connection: close\r\n"), "got: {text:?}");
    assert!(text.ends_with("p=/bye"), "got: {text:?}");
}

#[test]
fn engine_answers_400_on_malformed_request_and_closes() {
    let server = echo_engine(one_shard());
    let mut s = TcpStream::connect(server.local_addr()).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    // No half-close: the 400 must arrive AND the server must close on its own.
    s.write_all(b"GARBAGE\r\n\r\n").expect("write");
    let mut all = Vec::new();
    s.read_to_end(&mut all).expect("read to server close");
    let text = String::from_utf8(all).unwrap();
    assert!(
        text.starts_with("HTTP/1.1 400 Bad Request\r\n"),
        "got: {text:?}"
    );
    assert!(text.ends_with("Bad Request"), "got: {text:?}");
}

#[test]
fn engine_answers_400_on_a_truncated_request_at_eof() {
    let server = echo_engine(one_shard());
    // Promise a 5-byte body, deliver none, half-close: the tokio battery law
    // (EOF with leftover bytes) answers 400.
    let resp = roundtrip(
        server.local_addr(),
        b"POST /x HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\n",
    );
    let text = String::from_utf8(resp).unwrap();
    assert!(
        text.starts_with("HTTP/1.1 400 Bad Request\r\n"),
        "got: {text:?}"
    );
}

#[test]
fn engine_handler_panic_is_a_500_and_the_connection_survives() {
    let server = UringServer::bind(
        "127.0.0.1:0",
        one_shard(),
        Arc::new(|req: logicaffeine_system::http::wire::HttpRequest| {
            if req.path == "/boom" {
                panic!("handler exploded");
            }
            text_ok(&format!("p={}", req.path))
        }),
        None,
    )
    .expect("engine binds");

    // The panic answers 500 on the SAME keep-alive connection, which survives
    // to serve the next request.
    let mut s = TcpStream::connect(server.local_addr()).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    s.write_all(b"GET /boom HTTP/1.1\r\nHost: x\r\n\r\n")
        .expect("write boom");
    let boom = String::from_utf8(read_one_response(&mut s)).unwrap();
    assert!(
        boom.starts_with("HTTP/1.1 500 Internal Server Error\r\n"),
        "panicking handler must answer 500: {boom:?}"
    );
    assert!(boom.ends_with("internal error"), "500 body: {boom:?}");
    assert!(
        !boom.contains("exploded"),
        "failure detail must never reach the wire: {boom:?}"
    );

    s.write_all(b"GET /alive HTTP/1.1\r\nHost: x\r\n\r\n")
        .expect("write after boom");
    let after = String::from_utf8(read_one_response(&mut s)).unwrap();
    assert!(
        after.ends_with("p=/alive"),
        "connection must survive the panic: {after:?}"
    );

    // And the server as a whole keeps serving fresh connections.
    let fresh = roundtrip(
        server.local_addr(),
        b"GET /fresh HTTP/1.1\r\nHost: x\r\n\r\n",
    );
    assert!(String::from_utf8(fresh).unwrap().ends_with("p=/fresh"));
}

#[test]
fn engine_a_stalled_connection_does_not_block_the_shard() {
    // ONE shard, so this proves intra-shard concurrency: connection A promises
    // a body and stalls; connection B on the same shard must still be answered.
    let server = echo_engine(one_shard());
    let mut stalled = TcpStream::connect(server.local_addr()).expect("connect A");
    stalled
        .write_all(b"POST /slow HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\n")
        .expect("write stalled head");

    let resp = roundtrip(
        server.local_addr(),
        b"GET /fast HTTP/1.1\r\nHost: x\r\n\r\n",
    );
    assert!(String::from_utf8(resp).unwrap().ends_with("p=/fast"));
    drop(stalled);
}

#[test]
fn engine_in_flight_cap_parks_excess_connections_until_a_slot_frees() {
    // Cap 2 on one shard: two held keep-alive connections occupy both slots; a
    // third connection's request is parked (unanswered) until one closes.
    let server = echo_engine(EngineConfig {
        shards: 1,
        max_inflight: 2,
    });
    let addr = server.local_addr();

    let mut a = TcpStream::connect(addr).expect("connect A");
    a.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    a.write_all(b"GET /a HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
    assert!(String::from_utf8(read_one_response(&mut a))
        .unwrap()
        .ends_with("p=/a"));

    let mut b = TcpStream::connect(addr).expect("connect B");
    b.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    b.write_all(b"GET /b HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
    assert!(String::from_utf8(read_one_response(&mut b))
        .unwrap()
        .ends_with("p=/b"));

    // Third connection: accepted at the TCP level but parked by the engine.
    let mut c = TcpStream::connect(addr).expect("connect C");
    c.write_all(b"GET /c HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .unwrap();
    c.set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    let mut probe = [0u8; 1];
    match c.read(&mut probe) {
        Ok(0) => panic!("parked connection must not be closed while the cap holds"),
        Ok(_) => panic!("parked connection must not be answered while both slots are held"),
        Err(e) => assert!(
            matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ),
            "expected a read timeout while parked, got: {e:?}"
        ),
    }

    // Freeing one slot un-parks C and its request is answered.
    drop(a);
    c.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    let mut all = Vec::new();
    c.read_to_end(&mut all).expect("read C to close");
    assert!(String::from_utf8(all).unwrap().ends_with("p=/c"));
    drop(b);
}

#[test]
fn engine_static_table_bytes_are_identical_to_the_handler_path() {
    // The SAME HttpResponse served two ways — through the pre-serialized static
    // table and through the handler — must be byte-identical on the wire, for
    // both the keep-alive and close variants.
    let page = || {
        let mut headers = BTreeMap::new();
        headers.insert("Content-Type".to_string(), "text/html".to_string());
        HttpResponse {
            status: 200,
            headers,
            body: b"<h1>home</h1>".to_vec(),
        }
    };
    let mut statics = StaticResponses::new();
    statics.insert("/", &page());

    let with_table = UringServer::bind(
        "127.0.0.1:0",
        one_shard(),
        Arc::new(move |_req: logicaffeine_system::http::wire::HttpRequest| {
            panic!("the static table must answer / before the handler runs")
        }),
        Some(Arc::new(statics)),
    )
    .expect("engine binds");
    let handler_only = UringServer::bind(
        "127.0.0.1:0",
        one_shard(),
        Arc::new(move |_req: logicaffeine_system::http::wire::HttpRequest| page()),
        None,
    )
    .expect("engine binds");

    for raw in [
        b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n".as_slice(),
        b"GET / HTTP/1.1\r\nHost: x\r\n\r\n".as_slice(),
    ] {
        let mut sa = TcpStream::connect(with_table.local_addr()).unwrap();
        sa.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        sa.write_all(raw).unwrap();
        let a = read_one_response(&mut sa);
        let mut sb = TcpStream::connect(handler_only.local_addr()).unwrap();
        sb.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        sb.write_all(raw).unwrap();
        let b = read_one_response(&mut sb);
        assert_eq!(
            a,
            b,
            "static-table bytes diverged from handler bytes:\n table={:?}\n handler={:?}",
            String::from_utf8_lossy(&a),
            String::from_utf8_lossy(&b)
        );
    }

    // Non-GET and unknown paths fall through to the handler (which panics in
    // the table engine — surfaced as the shared 500, proving the fall-through).
    let miss = roundtrip(
        with_table.local_addr(),
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\n\r\n",
    );
    assert!(
        String::from_utf8_lossy(&miss).starts_with("HTTP/1.1 500"),
        "a POST must bypass the static table and reach the handler"
    );
}

#[test]
fn engine_multi_shard_serves_many_connections() {
    // Several shards, several concurrent client threads — every request is
    // answered correctly regardless of which shard the kernel picks.
    let server = echo_engine(EngineConfig {
        shards: 4,
        max_inflight: 64,
    });
    let addr = server.local_addr();
    let mut joins = Vec::new();
    for t in 0..8 {
        joins.push(std::thread::spawn(move || {
            for i in 0..25 {
                let raw = format!("GET /t{t}i{i} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
                let mut s = TcpStream::connect(addr).expect("connect");
                s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
                s.write_all(raw.as_bytes()).unwrap();
                let mut buf = Vec::new();
                s.read_to_end(&mut buf).unwrap();
                let text = String::from_utf8(buf).unwrap();
                assert!(text.ends_with(&format!("p=/t{t}i{i}")), "got: {text:?}");
            }
        }));
    }
    for j in joins {
        j.join().expect("client thread");
    }
}

/// The byte-parity differential: the SAME handler served by the tokio engine
/// and the uring engine must produce identical response bytes for the whole
/// probe battery — one-shot probes, the keep-alive lane, and the pipelined lane.
#[test]
fn engine_bytes_match_the_tokio_engine_for_the_probe_battery() {
    fn handler(req: logicaffeine_system::http::wire::HttpRequest) -> HttpResponse {
        if req.path == "/boom" {
            panic!("boom");
        }
        let mut headers = BTreeMap::new();
        headers.insert("Content-Type".to_string(), "text/plain".to_string());
        let q = req
            .query
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");
        let body = format!(
            "m={} p={} q=[{}] b={}",
            req.method,
            req.path,
            q,
            String::from_utf8_lossy(&req.body)
        );
        HttpResponse {
            status: 200,
            headers,
            body: body.into_bytes(),
        }
    }

    let uring = UringServer::bind("127.0.0.1:0", one_shard(), Arc::new(handler), None)
        .expect("uring engine binds");
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let tokio_server = rt
        .block_on(logicaffeine_system::http::server::spawn(
            "127.0.0.1:0",
            handler,
        ))
        .expect("tokio server binds");
    let (ua, ta) = (uring.local_addr(), tokio_server.addr());

    let probes: Vec<&[u8]> = vec![
        b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        b"GET /about HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        b"GET /a%20b?x=1&y=%2F HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        b"POST /submit HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Length: 7\r\n\r\n{\"a\":1}",
        b"GET /boom HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        b"GARBAGE\r\n\r\n",
        b"POST /trunc HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\n",
    ];
    for (i, raw) in probes.iter().enumerate() {
        let u = roundtrip(ua, raw);
        let t = roundtrip(ta, raw);
        assert_eq!(
            u,
            t,
            "engine divergence on probe {i}:\n uring={:?}\n tokio={:?}",
            String::from_utf8_lossy(&u),
            String::from_utf8_lossy(&t)
        );
    }

    // Keep-alive lane: two requests down one socket, byte-for-byte per engine.
    let keepalive = |addr: std::net::SocketAddr| -> (Vec<u8>, Vec<u8>) {
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        s.write_all(b"GET /one HTTP/1.1\r\nHost: x\r\n\r\n")
            .unwrap();
        let first = read_one_response(&mut s);
        s.write_all(b"GET /two HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut second = Vec::new();
        s.read_to_end(&mut second).unwrap();
        (first, second)
    };
    assert_eq!(keepalive(ua), keepalive(ta), "keep-alive lane diverged");

    // Pipelined lane: two requests in ONE write.
    let pipelined = |addr: std::net::SocketAddr| -> Vec<u8> {
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        s.write_all(
            b"GET /p1 HTTP/1.1\r\nHost: x\r\n\r\nGET /p2 HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        )
        .unwrap();
        let mut all = Vec::new();
        s.read_to_end(&mut all).unwrap();
        all
    };
    assert_eq!(pipelined(ua), pipelined(ta), "pipelined lane diverged");
}
