//! G15 — PRODUCTION SERVER LIFECYCLE, THE *BOUNDS* HALF. The MaCSad contract
//! demands every request/response dimension be BOUNDED with a DEFINED status,
//! never a hang or an OOM: request-body size (`413`), header-block size
//! (`413`), a READ DEADLINE on a slow-header / slow-body client (dropped at the
//! deadline, not held forever), and a response-body size cap (a handler that
//! produces a monstrous body is refused with a defined `500`, not shipped).
//!
//! The core lifecycle lock (`http_lifecycle_lock.rs`) proves drain / readiness /
//! backpressure / per-request-time / malformed-flood / correlation-id. THIS lock
//! proves the remaining bounded dimensions the contract names — the ones a
//! `400`-for-everything or an unbounded read would silently fail.
//!
//!  ⚠️  YOU DO NOT GET TO WEAKEN THIS LOCK.  ⚠️  A server that answers `400`
//!  (not `413`) for an oversized body, or holds a slot forever for a stalled
//!  client, or ships an unbounded response body, is not "serious and stays
//!  serving" — it is a memory-exhaustion / slot-exhaustion foothold.
#![cfg(all(not(target_arch = "wasm32"), feature = "http"))]

use std::time::Duration;

use logicaffeine_system::http::server::{spawn_with_lifecycle, LifecycleConfig};
use logicaffeine_system::http::wire::{HttpRequest, HttpResponse};

fn ok_body(body: &[u8]) -> HttpResponse {
    let mut headers = std::collections::BTreeMap::new();
    headers.insert("Content-Type".to_string(), "text/plain".to_string());
    HttpResponse {
        status: 200,
        headers,
        body: body.to_vec(),
    }
}

/// Raw one-shot request over a std socket (blocking) on a background thread, so
/// the async server drives concurrently. Returns the full response bytes.
fn raw_request(port: u16, raw: &[u8]) -> std::thread::JoinHandle<Vec<u8>> {
    let raw = raw.to_vec();
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        s.write_all(&raw).unwrap();
        let mut buf = Vec::new();
        let _ = s.read_to_end(&mut buf);
        buf
    })
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// OVERSIZED REQUEST BODY → 413, not 400. A client that declares a
/// `Content-Length` past the server's body cap must be refused with a defined
/// `413 Payload Too Large` (so a client can tell "too big" from "malformed"),
/// and the server must stay up for the next valid request.
#[test]
fn oversized_request_body_is_a_413_not_a_400() {
    rt().block_on(async {
        // A tiny body cap so the test needn't stream megabytes.
        let server = spawn_with_lifecycle(
            "127.0.0.1:0",
            move |_req: HttpRequest| ok_body(b"ok"),
            LifecycleConfig {
                max_body: 64,
                ..LifecycleConfig::default()
            },
        )
        .await
        .expect("spawns");
        let port = server.addr().port();

        // Declare a body far past the cap. The parser must reject it on the
        // Content-Length alone (never buffering the promised bytes).
        let raw =
            b"POST /big HTTP/1.1\r\nHost: x\r\nContent-Length: 100000\r\nConnection: close\r\n\r\n";
        let resp = raw_request(port, raw).join().unwrap();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.starts_with("HTTP/1.1 413"),
            "an oversized body must be 413 Payload Too Large, got {s:?}"
        );

        // The server survived and serves the next valid request.
        let good = raw_request(
            port,
            b"GET /x HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        )
        .join()
        .unwrap();
        assert!(
            String::from_utf8_lossy(&good).starts_with("HTTP/1.1 200"),
            "server must keep serving after refusing an oversized body"
        );
        let _ = server.drain(Duration::from_secs(1)).await;
    });
}

/// OVERSIZED HEADER BLOCK → 400, NOT 413. The `413`-vs-`400` split is
/// deliberate and pinned: an oversized *body* is a `413` (the server understood
/// the request and refused a too-large payload), but an oversized *header block*
/// is a `400` — the server cannot even parse the request head, so it is
/// malformed, exactly the shape the cross-tier `http_differential` lock demands
/// (both engines answer a clean `400` and close). This test guards that we did
/// not accidentally widen the body-`413` rule to swallow the header case.
#[test]
fn oversized_header_block_stays_a_400() {
    rt().block_on(async {
        let server = spawn_with_lifecycle(
            "127.0.0.1:0",
            move |_req: HttpRequest| ok_body(b"ok"),
            LifecycleConfig::default(),
        )
        .await
        .expect("spawns");
        let port = server.addr().port();

        // A header block past MAX_HEADERS (64 KiB) with no terminator in range.
        let mut raw = Vec::new();
        raw.extend_from_slice(b"GET /x HTTP/1.1\r\n");
        // One giant header value blows past the 64 KiB header cap.
        raw.extend_from_slice(b"X-Huge: ");
        raw.extend(std::iter::repeat(b'A').take(70_000));
        raw.extend_from_slice(b"\r\n\r\n");
        let resp = raw_request(port, &raw).join().unwrap();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.starts_with("HTTP/1.1 400"),
            "an oversized header block must stay a clean 400 (not 413), got first line {:?}",
            s.lines().next()
        );
        let _ = server.drain(Duration::from_secs(1)).await;
    });
}

/// READ DEADLINE ON A SLOW CLIENT (slowloris). A client that connects, sends a
/// partial request head, then STALLS forever must be dropped at the read
/// deadline — the slot/connection is freed, never held open indefinitely. We
/// prove it by keeping the read deadline tiny and asserting the server closes
/// the stalled socket (read-to-EOF returns) well within a bound, AND that the
/// server keeps serving fresh traffic throughout.
#[test]
fn a_slow_header_client_is_dropped_at_the_read_deadline() {
    rt().block_on(async {
        let server = spawn_with_lifecycle(
            "127.0.0.1:0",
            move |_req: HttpRequest| ok_body(b"ok"),
            LifecycleConfig {
                read_deadline: Duration::from_millis(300),
                ..LifecycleConfig::default()
            },
        )
        .await
        .expect("spawns");
        let port = server.addr().port();

        // A slowloris: send a partial header and never finish. The server must
        // drop us at the read deadline — so a blocking read-to-EOF returns
        // (the server closed our socket) within a small multiple of it.
        let slow = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            // A request line but NO terminating blank line — the head never ends.
            s.write_all(b"GET /slow HTTP/1.1\r\nHost: x\r\n").unwrap();
            let start = std::time::Instant::now();
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf); // returns when the server drops us
            (start.elapsed(), buf)
        });

        let (elapsed, _buf) = slow.join().expect("slow thread");
        assert!(
            elapsed < Duration::from_secs(3),
            "a stalled client must be dropped at the read deadline, not held forever (waited {elapsed:?})"
        );

        // The server is unharmed: a fresh valid request still gets a 200.
        let good =
            raw_request(port, b"GET /x HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").join().unwrap();
        assert!(
            String::from_utf8_lossy(&good).starts_with("HTTP/1.1 200"),
            "server must keep serving while a slowloris is dropped"
        );
        let _ = server.drain(Duration::from_secs(1)).await;
    });
}

/// RESPONSE-BODY SIZE CAP. A handler that produces a monstrous response body
/// (larger than the configured `max_response_body`) must be refused with a
/// defined `500` — the giant body is never shipped, defending server (and peer)
/// memory. A normal-sized response is unaffected.
#[test]
fn an_oversized_response_body_is_refused_with_a_500() {
    rt().block_on(async {
        let server = spawn_with_lifecycle(
            "127.0.0.1:0",
            move |req: HttpRequest| {
                if req.path == "/huge" {
                    // Way past the tiny cap below.
                    ok_body(&vec![b'Z'; 200_000])
                } else {
                    ok_body(b"small")
                }
            },
            LifecycleConfig {
                max_response_body: 1024,
                ..LifecycleConfig::default()
            },
        )
        .await
        .expect("spawns");
        let port = server.addr().port();

        let huge = raw_request(
            port,
            b"GET /huge HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        )
        .join()
        .unwrap();
        let s = String::from_utf8_lossy(&huge);
        assert!(
            s.starts_with("HTTP/1.1 500"),
            "an oversized response body must be refused with a 500, got {:?}",
            s.lines().next()
        );
        assert!(
            !s.contains(&"Z".repeat(2000)),
            "the monstrous body must never reach the wire"
        );

        // A normal response is unaffected.
        let small = raw_request(
            port,
            b"GET /small HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        )
        .join()
        .unwrap();
        assert!(
            String::from_utf8_lossy(&small).ends_with("small"),
            "a normal response is served unchanged"
        );
        let _ = server.drain(Duration::from_secs(1)).await;
    });
}
