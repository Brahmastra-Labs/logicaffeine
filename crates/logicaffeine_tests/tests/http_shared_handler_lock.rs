//! ★ LOCK — a SHARED handler answers a fast request while a slow one is still running. ★
//!
//! `serve_on` serializes every call on one mutex. That is deliberate and correct for a LOGOS
//! handler on the interpreter tier, which is `FnMut` and single-threaded — two turns at once there
//! would not be the same program. It is exactly wrong for a handler that is already `Sync`, because
//! it turns every SLOW request into a stop-the-world event.
//!
//! ★ WHAT THAT COST, and it was not hypothetical. `largo serve` answers its own static assets, its
//! pages and the `/__gen` reload poll from the SAME handler that proxies to the backend with a
//! multi-second timeout. Serialized, one hung backend route stalled the CSS, the wasm and the
//! reload poll behind it — so the page could not even refresh to tell you something was wrong. The
//! dev loop wedged on a single bad route, and the symptom looked like the whole server dying.
//!
//! ★ THE CONTROL IS THE POINT. Asserting only that the shared entry is concurrent would pass on a
//! server that was always concurrent, proving nothing about the change. So `serve_on` is held to
//! its OWN contract in the same file: it must still serialize. Both sentences have to stay true —
//! the tiers depend on different halves.

#![cfg(all(not(target_arch = "wasm32"), feature = "http"))]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use logicaffeine_system::http::server::{serve_on, serve_on_shared, Listener};
use logicaffeine_system::http::wire::{HttpRequest, HttpResponse};

/// How long `/slow` occupies the handler. Long enough that a serialized server cannot possibly
/// answer `/fast` inside the window below, short enough not to pad the suite.
const SLOW: Duration = Duration::from_millis(1200);
/// The window `/fast` must answer within while `/slow` is in flight.
const FAST_BUDGET: Duration = Duration::from_millis(600);

fn ok(body: &str) -> HttpResponse {
    let mut headers = std::collections::BTreeMap::new();
    headers.insert("Content-Type".to_string(), "text/plain".to_string());
    HttpResponse {
        status: 200,
        headers,
        body: body.as_bytes().to_vec(),
    }
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// One blocking request on its own thread, answering how long it took.
fn timed_get(port: u16, path: &str) -> std::thread::JoinHandle<(Duration, Vec<u8>)> {
    let raw = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        let started = Instant::now();
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
        s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        s.write_all(raw.as_bytes()).unwrap();
        let mut buf = Vec::new();
        let _ = s.read_to_end(&mut buf);
        (started.elapsed(), buf)
    })
}

async fn bind() -> (Listener, u16) {
    let listener = Listener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("bound address").port();
    (listener, port)
}

#[test]
fn a_shared_handler_answers_a_fast_request_while_a_slow_one_runs() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&calls);
    rt().block_on(async move {
        let (listener, port) = bind().await;
        tokio::spawn(async move {
            let _ = serve_on_shared(listener, move |req: HttpRequest| {
                seen.fetch_add(1, Ordering::SeqCst);
                if req.path == "/slow" {
                    std::thread::sleep(SLOW);
                    ok("slow")
                } else {
                    ok("fast")
                }
            })
            .await;
        });
        // Give the accept loop a moment, then start the slow request FIRST.
        tokio::time::sleep(Duration::from_millis(120)).await;
        let slow = timed_get(port, "/slow");
        tokio::time::sleep(Duration::from_millis(150)).await;
        let fast = timed_get(port, "/fast");

        let (fast_took, fast_body) = fast.join().expect("fast request finished");
        assert!(
            String::from_utf8_lossy(&fast_body).contains("fast"),
            "the fast request must actually be answered, not merely return"
        );
        assert!(
            fast_took < FAST_BUDGET,
            "a fast request took {fast_took:?} while a slow one was in flight — the handler is \
             still being serialized, which is what wedged the dev loop on one hung route"
        );
        let (_, slow_body) = slow.join().expect("slow request finished");
        assert!(
            String::from_utf8_lossy(&slow_body).contains("slow"),
            "and the slow one still completes"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "both requests reached the handler"
        );
    });
}

#[test]
fn serve_on_still_serializes_because_the_interpreter_tier_depends_on_it() {
    // ★ THE CONTROL, and it is not decoration. A LOGOS handler on the interpreter tier is `FnMut`
    // and single-threaded; if `serve_on` quietly became concurrent, two turns would interleave and
    // the tier would stop being the reference. Proving the new entry is concurrent means nothing
    // unless this one is proven to still be exclusive.
    rt().block_on(async move {
        let (listener, port) = bind().await;
        tokio::spawn(async move {
            let _ = serve_on(listener, move |req: HttpRequest| {
                if req.path == "/slow" {
                    std::thread::sleep(SLOW);
                    ok("slow")
                } else {
                    ok("fast")
                }
            })
            .await;
        });
        tokio::time::sleep(Duration::from_millis(120)).await;
        let slow = timed_get(port, "/slow");
        tokio::time::sleep(Duration::from_millis(150)).await;
        let fast = timed_get(port, "/fast");

        let (fast_took, _) = fast.join().expect("fast request finished");
        assert!(
            fast_took >= FAST_BUDGET,
            "serve_on answered a second request in {fast_took:?} while the first was still \
             running — it must SERIALIZE, because the interpreter tier's handler is single-threaded"
        );
        let _ = slow.join();
    });
}
