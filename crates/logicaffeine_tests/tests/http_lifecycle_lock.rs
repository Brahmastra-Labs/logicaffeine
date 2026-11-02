//! G15 — PRODUCTION SERVER LIFECYCLE. A server that is SERIOUS AND STAYS
//! SERVING: it drains gracefully (never drops an in-flight request), gates
//! readiness separately from liveness, sheds load with bounded backpressure
//! (503, never an unbounded hang), bounds per-request time, survives malformed
//! floods, and stamps every response/log with a correlation id (secret-safe).
//!
//! Built ONCE in the shared http core (server.rs) so BOTH serving tiers — the
//! interpreter (`largo serve`) and the compiled AOT binary (`largo build`) —
//! inherit the identical, proven lifecycle.
//!
//!  ⚠️  YOU DO NOT GET TO WEAKEN THIS LOCK.  ⚠️  "Stays serving" is the whole
//!  contract; a server that hangs under load or drops in-flight work fails it.
#![cfg(all(not(target_arch = "wasm32"), feature = "http"))]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use logicaffeine_system::http::server::{spawn_with_lifecycle, LifecycleConfig};
use logicaffeine_system::http::wire::{HttpRequest, HttpResponse};

fn ok_body(body: &str) -> HttpResponse {
    let mut headers = std::collections::BTreeMap::new();
    headers.insert("Content-Type".to_string(), "text/plain".to_string());
    HttpResponse {
        status: 200,
        headers,
        body: body.as_bytes().to_vec(),
    }
}

/// Raw one-shot request over a std socket (blocking) on a background thread, so
/// the async server drives concurrently. Returns the full response bytes.
fn raw_request(port: u16, raw: &str) -> std::thread::JoinHandle<Vec<u8>> {
    let raw = raw.to_string();
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        s.write_all(raw.as_bytes()).unwrap();
        let mut buf = Vec::new();
        let _ = s.read_to_end(&mut buf);
        buf
    })
}

fn get(path: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// GRACEFUL DRAIN: an in-flight request completes even though shutdown was
/// requested while it was running; new connections are refused during drain.
#[test]
fn drain_finishes_inflight_and_refuses_new() {
    rt().block_on(async {
        // A handler that blocks until released, so we can drain mid-request.
        let gate = Arc::new(tokio::sync::Notify::new());
        let gate2 = gate.clone();
        let server = spawn_with_lifecycle(
            "127.0.0.1:0",
            move |_req: HttpRequest| {
                // Block synchronously until the test releases the gate.
                let g = gate2.clone();
                futures::executor::block_on(async move { g.notified().await });
                ok_body("done")
            },
            LifecycleConfig::default(),
        )
        .await
        .expect("server spawns");
        let port = server.addr().port();

        let inflight = raw_request(port, &get("/slow"));
        tokio::time::sleep(Duration::from_millis(150)).await; // ensure it's in the handler

        // Request graceful drain (generous deadline). It must NOT return until
        // the in-flight request is done — so release the gate shortly after.
        let drainer = tokio::spawn(async move { server.drain(Duration::from_secs(5)).await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        gate.notify_waiters();

        let outcome = drainer.await.expect("drain task");
        let resp = inflight.join().expect("inflight thread");
        assert!(
            String::from_utf8_lossy(&resp).contains("200") && resp.windows(4).any(|w| w == b"done"),
            "the in-flight request must complete during graceful drain, got {:?}",
            String::from_utf8_lossy(&resp)
        );
        assert_eq!(
            outcome.dropped, 0,
            "graceful drain dropped {} in-flight requests",
            outcome.dropped
        );
    });
}

/// DRAIN DEADLINE: an in-flight request that never finishes must not hang the
/// drain forever — it returns after the deadline, reporting the force-closed count.
#[test]
fn drain_deadline_force_closes_and_reports() {
    rt().block_on(async {
        let server = spawn_with_lifecycle(
            "127.0.0.1:0",
            move |_req: HttpRequest| {
                futures::executor::block_on(async {
                    tokio::time::sleep(Duration::from_secs(3600)).await; // never returns in time
                });
                ok_body("never")
            },
            LifecycleConfig::default(),
        )
        .await
        .expect("spawns");
        let port = server.addr().port();
        let _inflight = raw_request(port, &get("/hang"));
        tokio::time::sleep(Duration::from_millis(150)).await;

        let start = std::time::Instant::now();
        let outcome = server.drain(Duration::from_millis(300)).await;
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "drain must honor its deadline, not hang"
        );
        assert_eq!(
            outcome.dropped, 1,
            "the one stuck request must be reported force-closed"
        );
    });
}

/// READINESS ≠ LIVENESS: a not-ready server sheds requests with 503; once ready
/// it serves 200. (Liveness = the accept loop is up, which spawn already implies.)
#[test]
fn readiness_gates_traffic_separately_from_liveness() {
    rt().block_on(async {
        let server = spawn_with_lifecycle(
            "127.0.0.1:0",
            move |_req: HttpRequest| ok_body("live"),
            LifecycleConfig::default(),
        )
        .await
        .expect("spawns");
        let port = server.addr().port();

        // Not ready yet → 503 Service Unavailable, fast (not a hang).
        server.set_ready(false);
        let before = raw_request(port, &get("/x")).join().unwrap();
        assert!(
            String::from_utf8_lossy(&before).contains("503"),
            "a not-ready server must shed with 503, got {:?}",
            String::from_utf8_lossy(&before)
        );

        server.set_ready(true);
        let after = raw_request(port, &get("/x")).join().unwrap();
        assert!(
            String::from_utf8_lossy(&after).contains("200"),
            "a ready server serves, got {:?}",
            String::from_utf8_lossy(&after)
        );
        let _ = server.drain(Duration::from_secs(1)).await;
    });
}

/// BOUNDED BACKPRESSURE: with a small in-flight cap saturated by blocked
/// requests, an extra request is shed with 503 promptly — never queued into an
/// unbounded hang. The server stays responsive afterward.
#[test]
fn overload_sheds_with_503_not_a_hang() {
    rt().block_on(async {
        let gate = Arc::new(tokio::sync::Notify::new());
        let g = gate.clone();
        let server = spawn_with_lifecycle(
            "127.0.0.1:0",
            move |_req: HttpRequest| {
                let g = g.clone();
                futures::executor::block_on(async move { g.notified().await });
                ok_body("done")
            },
            LifecycleConfig {
                max_inflight: 2,
                ..LifecycleConfig::default()
            },
        )
        .await
        .expect("spawns");
        let port = server.addr().port();
        server.set_ready(true);

        // Saturate the 2 slots with blocked requests.
        let _a = raw_request(port, &get("/1"));
        let _b = raw_request(port, &get("/2"));
        tokio::time::sleep(Duration::from_millis(200)).await;

        // The 3rd must be shed promptly with 503, not hang.
        let start = std::time::Instant::now();
        let third = raw_request(port, &get("/3")).join().unwrap();
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "overloaded server must shed fast, not hang"
        );
        assert!(
            String::from_utf8_lossy(&third).contains("503"),
            "over-cap request must be 503, got {:?}",
            String::from_utf8_lossy(&third)
        );
        gate.notify_waiters();
        let _ = server.drain(Duration::from_secs(2)).await;
    });
}

/// PER-REQUEST TIME BOUND: a handler that runs past the request deadline yields
/// a 503/504 and frees the slot — a slow handler can't wedge a connection forever.
#[test]
fn request_time_bound_frees_the_slot() {
    rt().block_on(async {
        let server = spawn_with_lifecycle(
            "127.0.0.1:0",
            move |_req: HttpRequest| {
                futures::executor::block_on(async {
                    tokio::time::sleep(Duration::from_secs(30)).await
                });
                ok_body("late")
            },
            LifecycleConfig {
                request_timeout: Duration::from_millis(300),
                ..LifecycleConfig::default()
            },
        )
        .await
        .expect("spawns");
        let port = server.addr().port();
        server.set_ready(true);

        let resp = raw_request(port, &get("/slow")).join().unwrap();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("503") || s.contains("504"),
            "a handler past the request deadline must yield 503/504, got {s:?}"
        );
        let _ = server.drain(Duration::from_secs(1)).await;
    });
}

/// CORRELATION ID: every response carries a correlation id; if the client sent
/// one it is echoed, otherwise the server assigns one. (Secret-safe logging is
/// asserted by the server never echoing arbitrary request header VALUES back.)
#[test]
fn every_response_carries_a_correlation_id() {
    rt().block_on(async {
        let server = spawn_with_lifecycle(
            "127.0.0.1:0",
            move |_req: HttpRequest| ok_body("ok"),
            LifecycleConfig::default(),
        )
        .await
        .expect("spawns");
        let port = server.addr().port();
        server.set_ready(true);

        // Client supplies a correlation id → echoed back.
        let raw =
            "GET /x HTTP/1.1\r\nHost: x\r\nX-Correlation-Id: abc123\r\nConnection: close\r\n\r\n";
        let resp = raw_request(port, raw).join().unwrap();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.to_lowercase().contains("x-correlation-id: abc123"),
            "a client correlation id must be echoed, got headers:\n{s}"
        );

        // No client id → the server assigns one (present, non-empty).
        let resp2 = raw_request(port, &get("/y")).join().unwrap();
        let s2 = String::from_utf8_lossy(&resp2);
        assert!(
            s2.to_lowercase().contains("x-correlation-id:"),
            "the server must assign a correlation id when none was sent, got:\n{s2}"
        );
        let _ = server.drain(Duration::from_secs(1)).await;
    });
}

/// SURVIVES A MALFORMED FLOOD: garbage/torn requests get 400 and the server
/// keeps serving valid traffic — a hostile client can't take the site down.
#[test]
fn malformed_flood_does_not_take_the_server_down() {
    rt().block_on(async {
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        let server = spawn_with_lifecycle(
            "127.0.0.1:0",
            move |_req: HttpRequest| {
                h.fetch_add(1, Ordering::SeqCst);
                ok_body("ok")
            },
            LifecycleConfig::default(),
        )
        .await
        .expect("spawns");
        let port = server.addr().port();
        server.set_ready(true);

        // A flood of garbage. (`raw_request` takes a `&str`, so the fixture is
        // UTF-8 control bytes rather than a raw 0xFF — a `\xff` cannot appear in
        // a Rust `str` literal at all; the assertion below is unchanged.)
        for _ in 0..20 {
            let _ = raw_request(port, "not a real http request \x00\x7f\r\n\r\n").join();
        }
        // A valid request still succeeds.
        let good = raw_request(port, &get("/still-up")).join().unwrap();
        assert!(
            String::from_utf8_lossy(&good).contains("200"),
            "server must survive a malformed flood and still serve valid traffic"
        );
        assert!(
            hits.load(Ordering::SeqCst) >= 1,
            "the valid request reached the handler"
        );
        let _ = server.drain(Duration::from_secs(1)).await;
    });
}

// ---------------------------------------------------------------------------
// EXTENDED LIFECYCLE COVERAGE — restart, exactly-once-under-drain-race,
// readiness flap, correlation-id uniqueness. These push the "stays serving"
// contract past the 7 core cases: a drained server frees its port for a
// successor, a side effect commits exactly once even when drain races the
// handler, readiness can flip repeatedly, and concurrent no-id requests never
// collide on an id.
// ---------------------------------------------------------------------------

/// The correlation-id value of a raw response (lowercased header match), if any.
fn correlation_id_of(resp: &[u8]) -> Option<String> {
    let s = String::from_utf8_lossy(resp);
    for line in s.split("\r\n") {
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("x-correlation-id") {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

/// RESTART: after a graceful drain the port is freed, so a fresh server can
/// re-bind the SAME port and serve again — the rolling-restart contract
/// (SO_REUSEADDR, no lingering-socket refusal).
#[test]
fn drained_server_frees_the_port_for_a_successor() {
    rt().block_on(async {
        let first = spawn_with_lifecycle(
            "127.0.0.1:0",
            move |_req: HttpRequest| ok_body("v1"),
            LifecycleConfig::default(),
        )
        .await
        .expect("first spawns");
        let port = first.addr().port();

        let hello = raw_request(port, &get("/x")).join().unwrap();
        assert!(
            String::from_utf8_lossy(&hello).contains("v1"),
            "first server serves"
        );

        // Drain and DROP the first server (its accept task aborts, its listener
        // closes) — the port must become re-bindable.
        let _ = first.drain(Duration::from_secs(2)).await;
        drop(first);

        // Re-bind the SAME port. Retry briefly: TCP teardown is not instant.
        let mut second = None;
        for _ in 0..50 {
            match spawn_with_lifecycle(
                &format!("127.0.0.1:{port}"),
                move |_req: HttpRequest| ok_body("v2"),
                LifecycleConfig::default(),
            )
            .await
            {
                Ok(s) => {
                    second = Some(s);
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
        let second = second.expect("a drained port must be re-bindable by a successor");
        assert_eq!(
            second.addr().port(),
            port,
            "the successor took the same port"
        );
        let after = raw_request(port, &get("/x")).join().unwrap();
        assert!(
            String::from_utf8_lossy(&after).contains("v2"),
            "the successor serves"
        );
        let _ = second.drain(Duration::from_secs(1)).await;
    });
}

/// EXACTLY-ONCE UNDER A DRAIN RACE: an in-flight handler with a side effect
/// runs its side effect exactly once even when a graceful drain begins while it
/// executes — no double-commit, no dropped commit (graceful drain finishes the
/// in-flight request, and it was admitted exactly once).
#[test]
fn a_side_effect_commits_exactly_once_even_when_drain_races_it() {
    rt().block_on(async {
        let commits = Arc::new(AtomicUsize::new(0));
        let c = commits.clone();
        let gate = Arc::new(tokio::sync::Notify::new());
        let g = gate.clone();
        let server = spawn_with_lifecycle(
            "127.0.0.1:0",
            move |_req: HttpRequest| {
                let g = g.clone();
                // Park until released, so drain starts mid-handler.
                futures::executor::block_on(async move { g.notified().await });
                // The single side effect — must fire exactly once.
                c.fetch_add(1, Ordering::SeqCst);
                ok_body("committed")
            },
            LifecycleConfig::default(),
        )
        .await
        .expect("spawns");
        let port = server.addr().port();

        let inflight = raw_request(port, &get("/commit"));
        tokio::time::sleep(Duration::from_millis(150)).await; // in the handler, parked

        // Drain races the handler; release it shortly after so it finishes
        // gracefully during the drain window.
        let drainer = tokio::spawn(async move { server.drain(Duration::from_secs(5)).await });
        tokio::time::sleep(Duration::from_millis(80)).await;
        gate.notify_waiters();

        let outcome = drainer.await.expect("drain task");
        let resp = inflight.join().expect("inflight thread");
        assert!(
            String::from_utf8_lossy(&resp).contains("committed"),
            "the in-flight request must complete during graceful drain"
        );
        assert_eq!(
            outcome.dropped, 0,
            "graceful drain dropped an in-flight request"
        );
        assert_eq!(
            commits.load(Ordering::SeqCst),
            1,
            "the side effect must commit EXACTLY once (no double-commit, no drop)"
        );
    });
}

/// READINESS FLAP: readiness can be toggled repeatedly and traffic tracks it
/// each time — a health-check driven cycle never wedges the server.
#[test]
fn readiness_can_flap_and_traffic_tracks_it_each_cycle() {
    rt().block_on(async {
        let server = spawn_with_lifecycle(
            "127.0.0.1:0",
            move |_req: HttpRequest| ok_body("live"),
            LifecycleConfig::default(),
        )
        .await
        .expect("spawns");
        let port = server.addr().port();

        for cycle in 0..4 {
            server.set_ready(false);
            let down = raw_request(port, &get("/x")).join().unwrap();
            assert!(
                String::from_utf8_lossy(&down).contains("503"),
                "cycle {cycle}: not-ready must shed with 503"
            );
            server.set_ready(true);
            let up = raw_request(port, &get("/x")).join().unwrap();
            assert!(
                String::from_utf8_lossy(&up).contains("200"),
                "cycle {cycle}: ready must serve 200"
            );
        }
        let _ = server.drain(Duration::from_secs(1)).await;
    });
}

/// CORRELATION-ID UNIQUENESS: two CONCURRENT requests that send NO correlation
/// id get DISTINCT server-assigned ids — ids never collide under concurrency.
#[test]
fn two_concurrent_no_id_requests_get_distinct_ids() {
    rt().block_on(async {
        let server = spawn_with_lifecycle(
            "127.0.0.1:0",
            move |_req: HttpRequest| ok_body("ok"),
            LifecycleConfig::default(),
        )
        .await
        .expect("spawns");
        let port = server.addr().port();
        server.set_ready(true);

        // Fire many concurrent no-id requests; collect every assigned id.
        let mut handles = Vec::new();
        for _ in 0..16 {
            handles.push(raw_request(port, &get("/x")));
        }
        let mut ids = std::collections::HashSet::new();
        for h in handles {
            let resp = h.join().expect("request thread");
            let cid = correlation_id_of(&resp).expect("server assigns a correlation id");
            assert!(!cid.is_empty(), "assigned id must be non-empty");
            assert!(
                ids.insert(cid.clone()),
                "assigned correlation id {cid:?} was not unique"
            );
        }
        assert_eq!(ids.len(), 16, "every concurrent request got a distinct id");
        let _ = server.drain(Duration::from_secs(1)).await;
    });
}

/// SECRET-SAFE ECHO: a client secret in an ordinary request header (e.g.
/// Authorization) is never reflected back on the wire — the correlation-id path
/// only ever surfaces method/path/status/id, never arbitrary header values.
#[test]
fn a_request_header_secret_is_never_reflected_on_the_wire() {
    rt().block_on(async {
        let server = spawn_with_lifecycle(
            "127.0.0.1:0",
            move |_req: HttpRequest| ok_body("ok"),
            LifecycleConfig::default(),
        )
        .await
        .expect("spawns");
        let port = server.addr().port();
        server.set_ready(true);

        let raw = "GET /x HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer super-secret-token\r\nConnection: close\r\n\r\n";
        let resp = raw_request(port, raw).join().unwrap();
        let s = String::from_utf8_lossy(&resp);
        assert!(!s.contains("super-secret-token"), "a request secret must never echo to the wire");
        assert!(s.to_lowercase().contains("x-correlation-id:"), "the response still carries an id");
        let _ = server.drain(Duration::from_secs(1)).await;
    });
}
