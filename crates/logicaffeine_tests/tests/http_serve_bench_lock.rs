//! A4a — the interpreter `Serve` throughput ratchet.
//!
//! This is a REGRESSION TRIPWIRE, not a benchmark: the floors carry ~5x
//! headroom over the measured wall time on the reference dev box, so only a
//! catastrophic regression (losing keep-alive, serializing connections behind
//! a stall, per-request re-parse of the program, …) trips them. The real
//! numbers print to stdout — run it on purpose:
//!
//! ```text
//! cargo test -p logicaffeine-tests --test http_serve_bench_lock -- --ignored --nocapture
//! ```
//!
//! Two lanes, both asserting every response is correct BEFORE timing counts:
//! - 200 sequential requests down ONE keep-alive connection (framing +
//!   connection-reuse latency);
//! - 200 requests across 8 concurrent keep-alive connections (the overlap
//!   path the http_workers lock proves correct, here proven fast).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Run a LOGOS Serve program on a background thread — the `largo serve`
/// hosting shape (no ambient tokio runtime; the interpreter stands up its own).
fn spawn_server(src: String) {
    std::thread::spawn(move || {
        let _ = futures::executor::block_on(logicaffeine_compile::interpret_for_ui(&src));
    });
}

fn serve_program(port: u16) -> String {
    format!(
        "## To handle (request: Request) -> Response:\n    \
             Return plain(request's path).\n\n\
         ## Main\n\
         Serve on port {port} with handle.\n"
    )
}

/// Connect with retry until the server is up.
fn connect(port: u16) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(s) => {
                s.set_read_timeout(Some(Duration::from_secs(30)))
                    .expect("read timeout");
                s.set_nodelay(true).expect("nodelay");
                return s;
            }
            Err(e) => {
                if Instant::now() > deadline {
                    panic!("server never came up on {port}: {e}");
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
}

/// The total frame length (header block + `Content-Length` body) of the
/// response at the head of `buf`, once its header block is complete.
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

/// Read exactly one framed response off a keep-alive socket.
fn read_framed_response(s: &mut TcpStream) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        if let Some(total) = framed_len(&buf) {
            if buf.len() >= total {
                buf.truncate(total);
                return buf;
            }
        }
        match s.read(&mut chunk) {
            Ok(0) => panic!(
                "server closed mid-exchange; got {:?}",
                String::from_utf8_lossy(&buf)
            ),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) => panic!("read failed: {e}"),
        }
    }
}

/// Fire `count` sequential keep-alive GETs down one socket; every response must
/// be a 200 echoing its own path. Returns the wall time of the exchange.
fn run_keepalive_lane(port: u16, tag: &str, count: usize) -> Duration {
    let mut s = connect(port);
    let started = Instant::now();
    for i in 0..count {
        let path = format!("/{tag}{i}");
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
            .expect("write request");
        let resp = read_framed_response(&mut s);
        assert!(
            resp.starts_with(b"HTTP/1.1 200 OK\r\n"),
            "request {path}: {:?}",
            String::from_utf8_lossy(&resp)
        );
        assert!(
            resp.ends_with(path.as_bytes()),
            "request {path} got a foreign body: {:?}",
            String::from_utf8_lossy(&resp)
        );
    }
    started.elapsed()
}

#[test]
#[ignore = "bench: run on purpose"]
fn interpreter_serve_throughput_holds_the_floor() {
    let port = free_port();
    spawn_server(serve_program(port));
    // Warm up: the first exchange absorbs program parse + server bind.
    run_keepalive_lane(port, "warm", 1);

    // Lane 1: 200 sequential requests, ONE keep-alive connection.
    const SEQUENTIAL: usize = 200;
    let seq_wall = run_keepalive_lane(port, "seq", SEQUENTIAL);
    let seq_rps = SEQUENTIAL as f64 / seq_wall.as_secs_f64();
    println!("sequential keep-alive: {SEQUENTIAL} requests in {seq_wall:?} = {seq_rps:.0} req/s");

    // Lane 2: 200 requests across 8 concurrent keep-alive connections.
    const WORKERS: usize = 8;
    const PER_WORKER: usize = 25;
    let started = Instant::now();
    let handles: Vec<_> = (0..WORKERS)
        .map(|w| {
            std::thread::spawn(move || run_keepalive_lane(port, &format!("w{w}q"), PER_WORKER))
        })
        .collect();
    for h in handles {
        h.join().expect("worker lane");
    }
    let conc_wall = started.elapsed();
    let conc_rps = (WORKERS * PER_WORKER) as f64 / conc_wall.as_secs_f64();
    println!(
        "concurrent keep-alive: {} requests over {WORKERS} connections in {conc_wall:?} = {conc_rps:.0} req/s",
        WORKERS * PER_WORKER
    );

    // The floors: measured 2026-09-01 on the reference dev box (debug build)
    // at 16.0ms sequential / 12.8ms concurrent per 200-request lane
    // (12,486 / 15,682 req/s). Budgeting a generous 100ms per lane for a
    // loaded CI box and applying the 5x headroom on top of that gives 500ms —
    // only a catastrophic regression (losing keep-alive, per-request
    // reconnect, serialized connections) can trip it.
    assert!(
        seq_wall < Duration::from_millis(500),
        "sequential lane regressed: 200 keep-alive requests took {seq_wall:?} (floor 500ms; \
         reference 16ms)"
    );
    assert!(
        conc_wall < Duration::from_millis(500),
        "concurrent lane regressed: 200 requests over 8 connections took {conc_wall:?} \
         (floor 500ms; reference 13ms)"
    );
}
