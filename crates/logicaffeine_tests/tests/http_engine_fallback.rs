//! Engine-selection fallback honesty: `LOGOS_HTTP_ENGINE=uring` when the ring
//! cannot initialize (simulated with `LOGOS_HTTP_FORCE_FALLBACK=1`) must serve
//! via tokio — correctly — and say so on stderr (asserted at the law level;
//! the stderr line itself is asserted end-to-end by the CLI engine
//! differential, where the server is a child process whose stderr we capture).
//!
//! This is the ONLY test in this binary: it mutates process-global env vars, so
//! it must never share a process with parallel tests.
#![cfg(all(not(target_arch = "wasm32"), feature = "http"))]

use std::collections::BTreeMap;

use logicaffeine_system::http::engine::{choose_engine, EngineKind};
use logicaffeine_system::http::wire::HttpResponse;

#[tokio::test]
async fn forced_fallback_serves_via_tokio_and_says_why() {
    std::env::set_var("LOGOS_HTTP_ENGINE", "uring");
    std::env::set_var("LOGOS_HTTP_FORCE_FALLBACK", "1");

    // The selection law: uring requested, ring init forced to fail → tokio,
    // with a reason that names the forced fallback (the honesty line).
    let choice = choose_engine();
    assert_eq!(
        choice.kind,
        EngineKind::Tokio,
        "forced fallback must select tokio"
    );
    assert!(
        choice.reason.contains("LOGOS_HTTP_FORCE_FALLBACK"),
        "the reason must name the forced fallback: {:?}",
        choice.reason
    );

    // And the selected path actually serves: `serve` (the AOT entry) under the
    // forced-fallback env answers requests via the tokio engine.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind probe port");
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let server = tokio::spawn(async move {
        let _ = logicaffeine_system::http::server::serve(&addr.to_string(), |_req| {
            let mut headers = BTreeMap::new();
            headers.insert("Content-Type".to_string(), "text/plain".to_string());
            HttpResponse {
                status: 200,
                headers,
                body: b"fallback ok".to_vec(),
            }
        })
        .await;
    });

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let body = loop {
        match tokio::net::TcpStream::connect(addr).await {
            Ok(mut s) => {
                s.write_all(b"GET /probe HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
                    .await
                    .expect("write");
                let mut buf = Vec::new();
                s.read_to_end(&mut buf).await.expect("read");
                if !buf.is_empty() {
                    break String::from_utf8(buf).unwrap();
                }
            }
            Err(_) => {}
        }
        assert!(
            std::time::Instant::now() < deadline,
            "fallback server never answered"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    };
    assert!(body.starts_with("HTTP/1.1 200 OK\r\n"), "got: {body:?}");
    assert!(body.ends_with("fallback ok"), "got: {body:?}");
    server.abort();

    std::env::remove_var("LOGOS_HTTP_ENGINE");
    std::env::remove_var("LOGOS_HTTP_FORCE_FALLBACK");
}
