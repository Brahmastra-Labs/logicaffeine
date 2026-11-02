//! The head-to-head engine bench: the SAME handler served by the shipped tokio
//! engine and the io_uring thread-per-core engine, hammered by the SAME
//! blocking client threads on the same box — req/s for both, the ratio, and
//! the claim check: uring must not lose the keep-alive lanes.
//!
//! Run on purpose, in release:
//! `cargo test --release -p logicaffeine-tests --features http,io-uring --test http_engine_bench -- --ignored --nocapture`
//!
//! Honesty notes (the numbers this prints are the only numbers we claim):
//! - The tokio side is the SHIPPED `spawn`/`serve_loop` path (per-connection
//!   tasks, handler serialized on a mutex — exactly what a `Serve` runs today).
//! - The uring side is the shipped engine with an explicit config: one shard
//!   per core and a per-shard cap high enough that the kernel's REUSEPORT hash
//!   never parks a bench connection (the cap law itself is covered by the
//!   functional battery; parking would measure the hash, not the engine).
//! - Best-of-3 per lane, generous to scheduler noise.
#![cfg(all(target_os = "linux", feature = "http", feature = "io-uring"))]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Barrier};
use std::time::Instant;

use logicaffeine_system::http::engine_uring::{EngineConfig, UringServer};
use logicaffeine_system::http::wire::{HttpRequest, HttpResponse};

fn handler(req: HttpRequest) -> HttpResponse {
    let mut headers = BTreeMap::new();
    headers.insert("Content-Type".to_string(), "text/plain".to_string());
    HttpResponse {
        status: 200,
        headers,
        body: format!("pong:{}", req.path).into_bytes(),
    }
}

/// The total frame length of the response at the head of `buf`.
fn framed_len(buf: &[u8]) -> Option<usize> {
    let sep = buf.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
    let head = std::str::from_utf8(&buf[..sep]).ok()?;
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

/// One keep-alive worker: `reqs` sequential request/response exchanges down
/// ONE connection, `pipeline` requests per write (1 = plain keep-alive).
fn storm_worker(addr: SocketAddr, reqs: usize, pipeline: usize) {
    let mut s = TcpStream::connect(addr).expect("connect");
    s.set_nodelay(true).expect("client nodelay");
    let request = b"GET /bench HTTP/1.1\r\nHost: x\r\n\r\n";
    let batch: Vec<u8> = request
        .iter()
        .copied()
        .cycle()
        .take(request.len() * pipeline)
        .collect();
    let mut buf = Vec::with_capacity(16 * 1024);
    let mut chunk = [0u8; 16 * 1024];
    let batches = reqs / pipeline;
    for _ in 0..batches {
        s.write_all(&batch).expect("write batch");
        let mut answered = 0;
        while answered < pipeline {
            while let Some(total) = framed_len(&buf) {
                if buf.len() < total {
                    break;
                }
                buf.drain(..total);
                answered += 1;
                if answered == pipeline {
                    break;
                }
            }
            if answered == pipeline {
                break;
            }
            let n = s.read(&mut chunk).expect("read");
            assert!(n > 0, "server closed mid-storm");
            buf.extend_from_slice(&chunk[..n]);
        }
    }
}

/// One measured run: `conns` client threads × `reqs` requests each → req/s.
fn run_storm(addr: SocketAddr, conns: usize, reqs: usize, pipeline: usize) -> f64 {
    let barrier = Arc::new(Barrier::new(conns + 1));
    let mut joins = Vec::new();
    for _ in 0..conns {
        let barrier = Arc::clone(&barrier);
        joins.push(std::thread::spawn(move || {
            barrier.wait();
            storm_worker(addr, reqs, pipeline);
        }));
    }
    barrier.wait();
    let start = Instant::now();
    for j in joins {
        j.join().expect("client thread");
    }
    let secs = start.elapsed().as_secs_f64();
    (conns * reqs) as f64 / secs
}

fn best_of_3(addr: SocketAddr, conns: usize, reqs: usize, pipeline: usize) -> f64 {
    (0..3)
        .map(|_| run_storm(addr, conns, reqs, pipeline))
        .fold(0.0, f64::max)
}

#[test]
#[ignore = "bench: run on purpose"]
fn uring_vs_tokio_head_to_head() {
    let shards = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let uring = UringServer::bind(
        "127.0.0.1:0",
        EngineConfig {
            shards,
            max_inflight: shards * 64,
        },
        Arc::new(handler),
        None,
    )
    .expect("uring engine binds");
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let tokio_server = rt
        .block_on(logicaffeine_system::http::server::spawn(
            "127.0.0.1:0",
            handler,
        ))
        .expect("tokio server binds");
    let (ua, ta) = (uring.local_addr(), tokio_server.addr());

    // (name, conns, reqs/conn, pipeline, is_keep_alive_claim_lane)
    let lanes: &[(&str, usize, usize, usize, bool)] = &[
        ("keep-alive  n=8 ", 8, 4000, 1, true),
        ("keep-alive  n=64", 64, 600, 1, true),
        ("pipelined×16 n=8", 8, 3200, 16, false),
    ];

    // Warm both engines before measuring.
    run_storm(ua, 4, 200, 1);
    run_storm(ta, 4, 200, 1);

    println!();
    println!(
        "engine head-to-head ({} shards, best of 3 runs per lane)",
        shards
    );
    let mut claim_holds = true;
    for &(name, conns, reqs, pipeline, claim_lane) in lanes {
        let u = best_of_3(ua, conns, reqs, pipeline);
        let t = best_of_3(ta, conns, reqs, pipeline);
        let ratio = u / t;
        println!(
            "  {name}: uring {u:>10.0} req/s | tokio {t:>10.0} req/s | uring/tokio = {ratio:.2}x"
        );
        if claim_lane && u < t {
            claim_holds = false;
        }
    }
    assert!(
        claim_holds,
        "the uring engine lost a keep-alive lane to tokio — profile before claiming anything"
    );
}
