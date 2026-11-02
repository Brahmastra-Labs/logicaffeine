//! G15 FOLLOW-UP, CLOSED — `largo serve` inherits the FULL production lifecycle.
//!
//! This file replaces the RED marker that used to sit in `logicaffeine_tests`,
//! where it could never have run: that crate has no dependency on the `largo`
//! binary, so `CARGO_BIN_EXE_largo-under-test` does not exist there and the test could not
//! spawn the thing it was written to test. It lives here, beside
//! `http_differential.rs`, which spawns servers for a living.
//!
//! The two seams the marker named now exist, and they were NOT built for the
//! interpreter alone — the compiled tier had no external control either. Both
//! come from one place:
//!
//!   1. THE CONTROL SEAM. `LifecycleConfig::from_env` is read by
//!      `ready_serve_lifecycle`, which every serve entry reaches, so a compiled
//!      binary honors the lifecycle names with no change to code generation —
//!      the same trick `env_port_override` plays for the port. `SIGTERM` and
//!      `SIGINT` resolve `shutdown_requested`, a FUTURE (not a spawned task, so
//!      the `!Send` interpreted and bytecode loops can select on it too): stop
//!      advertising readiness, stop accepting, finish the in-flight work.
//!
//!   2. THE PROBES. `LOGOS_READY_PATH` and `LOGOS_HEALTH_PATH` are answered
//!      before admission, so a monitor gets a reply during an overload. Both are
//!      opt-in: with neither set no route is taken from the program, which is why
//!      the cross-tier byte-for-byte differential is unmoved.
//!
//!   3. THE PER-REQUEST BUDGET. The interpreter's handler touches `&mut self` and
//!      runs inline on the one loop thread, so it cannot be raced against a timer
//!      the way the compiled tier's `Send` handler is moved to `spawn_blocking`.
//!      It is enforced from the INSIDE instead, at the same per-statement
//!      checkpoint the op budget already uses, and honestly: the deadline is
//!      observed BETWEEN statements, so it bounds a runaway loop rather than a
//!      runaway primitive. A single long-running builtin still finishes.
//!
//! YOU DO NOT GET TO WEAKEN THESE. A server that will not drain drops the
//! requests it was already serving; one that cannot be probed cannot be
//! deployed. Fix the tier, never the assert.

#![cfg(not(target_arch = "wasm32"))]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[path = "support/under_test.rs"]
mod under_test;

fn largo() -> Command {
    Command::new(under_test::largo())
}

/// A spawned server, killed on drop so a failing assert never leaks a process.
struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A project whose handler answers instantly on `/` and slowly on `/slow`, so a
/// drain has something real to finish.
fn scaffold(dir: &std::path::Path) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Largo.toml"),
        "[package]\nname = \"lifecycle_probe\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/main.lg"),
        "## To handle (request: Request) -> Response:\n\
             \x20   If request's path is equal to \"/slow\":\n\
             \x20       Sleep 1200.\n\
             \x20       Return plain(\"slow done\").\n\
             \x20   If request's path is equal to \"/spin\":\n\
             \x20       Let mutable n be 0.\n\
             \x20       While n is at least 0:\n\
             \x20           Set n to n + 1.\n\
             \x20       Return plain(\"never\").\n\
             \x20   Return plain(\"fast\").\n\
             \n\
             ## Main\n\
             Serve on port 8080 with handle.\n",
    )
    .unwrap();
}

/// Spawn `largo serve` on a port the system chooses (`LOGOS_PORT=0`) and return it with the port its
/// banner names.
///
/// The banner is read off stdout rather than guessed at with a connect poll: a
/// successful connect proves only that the kernel accepted a SYN, while the
/// banner is the server itself saying it is listening, and where.
fn serve(dir: &std::path::Path, env: &[(&str, &str)]) -> (Server, u16) {
    scaffold(dir);
    let mut cmd = largo();
    cmd.arg("serve")
        .current_dir(dir)
        .env("LOGOS_PORT", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("largo serve spawns");
    let stdout = child.stdout.take().expect("stdout piped");
    let mut lines = BufReader::new(stdout).lines();
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut banner = None;
    while Instant::now() < deadline {
        match lines.next() {
            Some(Ok(line)) if line.contains("Serving on http://") => {
                banner = Some(line);
                break;
            }
            Some(Ok(_)) => continue,
            _ => break,
        }
    }
    assert!(
        banner.is_some(),
        "`largo serve` must announce the bound port on stdout — it is how anything \
         downstream knows the server is up, and for the whole life of this feature it \
         was buffered into output that a never-returning server never flushes"
    );
    let port = banner
        .as_deref()
        .and_then(|line| line.trim().rsplit(':').next())
        .and_then(|port| port.trim_end_matches('/').parse::<u16>().ok())
        .expect("the banner names the port the server bound");
    (Server(child), port)
}

/// One request over a fresh connection, read to end of stream.
fn request(port: u16, raw: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut sock = loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(s) => break s,
            Err(e) => {
                assert!(Instant::now() < deadline, "server never accepted: {e}");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    sock.set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    sock.write_all(raw.as_bytes()).unwrap();
    let mut buf = Vec::new();
    let _ = sock.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).to_string()
}

fn get(port: u16, path: &str) -> String {
    request(
        port,
        &format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"),
    )
}

fn status_of(response: &str) -> u16 {
    response
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("no status line in response: {response:?}"))
}

fn send_sigterm(child: &Child) {
    // SAFETY: kill(2) on the server this test spawned and has not yet reaped.
    let ok = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) } == 0;
    assert!(ok, "SIGTERM must reach the server process");
}

#[test]
fn largo_serve_answers_its_configured_probes_and_leaves_every_other_path_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (server, port) = serve(
        dir.path(),
        &[
            ("LOGOS_READY_PATH", "/readyz"),
            ("LOGOS_HEALTH_PATH", "/healthz"),
        ],
    );

    assert_eq!(
        status_of(&get(port, "/readyz")),
        200,
        "a serving server is ready"
    );
    assert_eq!(
        status_of(&get(port, "/healthz")),
        200,
        "a serving server is alive"
    );

    let own = get(port, "/");
    assert_eq!(status_of(&own), 200);
    assert!(
        own.ends_with("fast"),
        "the program still owns its own routes: {own:?}"
    );
    drop(server);
}

#[test]
fn no_path_is_taken_from_the_program_when_no_probe_is_configured() {
    // The default must reserve nothing. This is what keeps the interpreted and
    // compiled servers byte-identical, and it is why the probes are opt-in.
    let dir = tempfile::tempdir().unwrap();
    let (server, port) = serve(dir.path(), &[]);
    let probe = get(port, "/readyz");
    assert_eq!(status_of(&probe), 200);
    assert!(
        probe.ends_with("fast"),
        "with no probe configured `/readyz` is the program's own route, answered by its \
         handler — not a reserved word: {probe:?}"
    );
    drop(server);
}

#[test]
fn a_stop_request_drains_in_flight_work_instead_of_dropping_it() {
    // The guarantee that matters on a deploy: the request already being served
    // when the orchestrator says stop must still get its answer.
    let dir = tempfile::tempdir().unwrap();
    let (mut server, port) = serve(dir.path(), &[("LOGOS_DRAIN_MS", "10000")]);

    let slow = std::thread::spawn(move || get(port, "/slow"));
    std::thread::sleep(Duration::from_millis(400));
    send_sigterm(&server.0);

    let answer = slow.join().expect("the in-flight request thread finishes");
    assert_eq!(
        status_of(&answer),
        200,
        "a request in flight when the stop arrived must still be answered: {answer:?}"
    );
    assert!(
        answer.ends_with("slow done"),
        "and answered completely: {answer:?}"
    );

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match server.0.try_wait().expect("try_wait") {
            Some(_) => break,
            None => {
                assert!(
                    Instant::now() < deadline,
                    "the server must exit after draining"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

#[test]
fn a_drained_server_releases_its_port_for_a_successor() {
    // What a rolling deploy actually needs: the old process finishes the work it
    // had, then gets out of the way so the new one can bind the same port. Note
    // the shape this asserts — a drain STOPS ACCEPTING rather than staying up and
    // shedding, so during a drain the port becomes unreachable rather than
    // answering 503. That is the stronger signal, and it is why the readiness
    // probe's own draining behavior is locked as a unit law in
    // `logicaffeine_system::http::lifecycle` (where readiness can be observed
    // without a socket) rather than through a live port here.
    let dir = tempfile::tempdir().unwrap();
    let (mut server, port) = serve(dir.path(), &[("LOGOS_DRAIN_MS", "10000")]);

    assert_eq!(
        status_of(&get(port, "/")),
        200,
        "serving before the stop request"
    );

    let slow = std::thread::spawn(move || get(port, "/slow"));
    std::thread::sleep(Duration::from_millis(400));
    send_sigterm(&server.0);

    let answer = slow.join().expect("the in-flight request thread finishes");
    assert_eq!(
        status_of(&answer),
        200,
        "it still finishes what it started: {answer:?}"
    );

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match server.0.try_wait().expect("try_wait") {
            Some(_) => break,
            None => {
                assert!(
                    Instant::now() < deadline,
                    "the server must exit after draining"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }

    let successor = std::net::TcpListener::bind(("127.0.0.1", port));
    assert!(
        successor.is_ok(),
        "a drained server must release its port so the next deploy can bind it: {:?}",
        successor.err()
    );
}

#[test]
fn a_handler_that_never_finishes_is_bounded_and_the_server_survives_it() {
    // The guarantee the compiled tier gets by racing a timer, on the tier that
    // cannot: a request whose handler loops forever is abandoned with a `503`,
    // and the very next request is still served. Without it one bad route wedges
    // the single loop thread and the whole site stops answering.
    let dir = tempfile::tempdir().unwrap();
    let (server, port) = serve(dir.path(), &[("LOGOS_REQUEST_TIMEOUT_MS", "700")]);

    let spun = get(port, "/spin");
    assert_eq!(
        status_of(&spun),
        503,
        "a handler past its budget is abandoned with a 503, not left running: {spun:?}"
    );

    let after = get(port, "/");
    assert_eq!(
        status_of(&after),
        200,
        "and the server still serves: {after:?}"
    );
    assert!(after.ends_with("fast"));
    drop(server);
}
