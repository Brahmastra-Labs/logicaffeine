//! G5 — ONE assembled module closure for every `largo` command.
//!
//! The MaCSad audit's false-green: `largo test` read ONLY the entry file
//! (imported test modules silently missed while reporting success), executed
//! the entry's `## Main` while running tests (a `Serve` entry blocked on its
//! port instead of isolating pure tests), and exited 0 with zero discovered
//! tests. The DOM watcher fingerprinted only the entry's immediate directory,
//! so an edit to a nested imported module never rebuilt.
//!
//! Contract locked here, through the real binary exactly as a user runs it:
//! check / test / emit / build / serve consume the SAME assembled module
//! closure; the watcher tracks every file in that closure; zero discovered
//! tests and zero-match filters FAIL loudly; the report names every test it
//! ran.

#[path = "cli_common/mod.rs"]
mod common;
use common::http::Server;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use common::{largo, largo_in, stderr, stdout};

/// Write a `Largo.toml` (entry `src/main.lg`) plus each `(relative-to-src, body)`
/// module — creating subdirectories as needed for nested layouts.
fn project(dir: &Path, name: &str, files: &[(&str, &str)]) {
    std::fs::write(
        dir.join("Largo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n"),
    )
    .unwrap();
    for (rel, body) in files {
        let path = dir.join("src").join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
}

/// The three-file test project: the entry imports a sibling module and a NESTED
/// module, each contributing a `## Test`, and the entry's own test calls an
/// imported function. `largo test` must run all three.
fn test_closure_project(dir: &Path) {
    project(
        dir,
        "app",
        &[
            (
                "main.lg",
                "# App\n\nImport Mathx.\nImport Geo from \"deep/geo.lg\".\n\n\
             ## Main\nShow double(21).\n\n\
             ## Test: entry test uses an imported function\n\
             \x20   Expect that double(4) is 8.\n",
            ),
            (
                "mathx.lg",
                "# Mathx\n\n\
             ## To double (n: Int) -> Int:\n    Return n times 2.\n\n\
             ## Test: imported double doubles\n\
             \x20   Expect that double(6) is 12.\n",
            ),
            (
                "deep/geo.lg",
                "# Geo\n\n\
             ## To area (s: Int) -> Int:\n    Return s times s.\n\n\
             ## Test: nested area computes\n\
             \x20   Expect that area(3) is 9.\n",
            ),
        ],
    );
}

// ============================================================================
// 1. `largo test` discovers + runs tests from the whole import closure, and
//    the report names every test identity it ran.
// ============================================================================

#[test]
fn largo_test_discovers_and_runs_imported_module_tests() {
    let dir = tempfile::tempdir().unwrap();
    test_closure_project(dir.path());

    let out = largo_in(dir.path(), &["test"]);
    let text = stdout(&out);
    assert!(
        out.status.success(),
        "largo test must be green over the closure\nstdout: {text}\nstderr: {}",
        stderr(&out)
    );
    // Exact discovered test identities, one PASS line each — the report is the
    // machine-readable ledger of what actually ran.
    for name in [
        "entry test uses an imported function",
        "imported double doubles",
        "nested area computes",
    ] {
        assert!(
            text.contains(&format!("PASS  {name}")),
            "report must name {name:?}:\n{text}"
        );
    }
    assert!(
        text.contains("3 passed"),
        "all three closure tests ran:\n{text}"
    );

    // The JSON report carries the same identities for machines.
    let json_out = largo_in(dir.path(), &["test", "--format", "json"]);
    let json = stdout(&json_out);
    assert!(
        json_out.status.success(),
        "json run green:\n{}",
        stderr(&json_out)
    );
    for name in [
        "entry test uses an imported function",
        "imported double doubles",
        "nested area computes",
    ] {
        assert!(
            json.contains(name),
            "json report must list {name:?}:\n{json}"
        );
    }
}

/// Editing a NESTED imported module's test must be picked up by the next run —
/// the closure is re-assembled fresh, never cached stale.
#[test]
fn largo_test_reruns_a_changed_nested_imported_test() {
    let dir = tempfile::tempdir().unwrap();
    test_closure_project(dir.path());

    let green = largo_in(dir.path(), &["test"]);
    assert!(
        green.status.success(),
        "starting point is green:\n{}",
        stderr(&green)
    );

    // Break the nested module's test: the very next run must rediscover, rerun,
    // and FAIL with that test's exact identity.
    std::fs::write(
        dir.path().join("src/deep/geo.lg"),
        "# Geo\n\n\
         ## To area (s: Int) -> Int:\n    Return s times s.\n\n\
         ## Test: nested area computes\n\
         \x20   Expect that area(3) is 10.\n",
    )
    .unwrap();
    let red = largo_in(dir.path(), &["test"]);
    let text = stdout(&red);
    assert!(
        !red.status.success(),
        "the broken nested test must fail the run:\n{text}"
    );
    assert!(
        text.contains("FAIL  nested area computes"),
        "the failure names the nested test:\n{text}"
    );
}

// ============================================================================
// 2. Test isolation: a `Serve` entry's `## Main` never executes during
//    `largo test` — pure tests run and finish; the port is never bound.
// ============================================================================

#[test]
fn largo_test_isolates_pure_tests_from_a_serve_main() {
    let dir = tempfile::tempdir().unwrap();
    project(
        dir.path(),
        "srv",
        &[(
            "main.lg",
            "# Srv\n\n\
         ## To handle (r: Request) -> Response:\n    Return ok(\"hi\").\n\n\
         ## Main\nServe on port 18085 with handle.\n\n\
         ## Test: pure arithmetic\n\
         \x20   Expect that 2 plus 2 is 4.\n",
        )],
    );

    // The port must be free beforehand for the not-bound probe to be meaningful.
    let port_was_free = TcpListener::bind("127.0.0.1:18085").is_ok();

    let mut child = largo()
        .arg("test")
        .current_dir(dir.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("largo test spawns");

    // A test run over a Serve entry must FINISH (Main is not part of any test's
    // world). If Serve executed, it would block forever — kill and fail.
    let deadline = Instant::now() + Duration::from_secs(120);
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break status,
            None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("largo test blocked — the entry's `## Main` (Serve) executed during tests");
            }
        }
    };
    let mut text = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    let mut err = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut err)
        .unwrap();

    assert!(
        status.success(),
        "the pure test passes\nstdout: {text}\nstderr: {err}"
    );
    assert!(
        text.contains("PASS  pure arithmetic"),
        "the pure test ran:\n{text}"
    );
    assert!(
        !text.contains("Serving on http://") && !err.contains("Serving on http://"),
        "a test run never reaches the entry's `Serve`, so nothing announces a server:\nstdout: {text}\nstderr: {err}"
    );
    if port_was_free {
        assert!(
            TcpStream::connect_timeout(
                &"127.0.0.1:18085".parse().unwrap(),
                Duration::from_millis(500)
            )
            .is_err(),
            "the Serve port must never have been bound by a test run"
        );
    }
}

// ============================================================================
// 3. Zero discovered tests ⇒ failure; a zero-match filter ⇒ failure.
// ============================================================================

#[test]
fn largo_test_fails_loudly_with_zero_discovered_tests() {
    let dir = tempfile::tempdir().unwrap();
    project(
        dir.path(),
        "zero",
        &[("main.lg", "# Zero\n\n## Main\nShow 1.\n")],
    );

    let out = largo_in(dir.path(), &["test"]);
    assert!(
        !out.status.success(),
        "zero discovered tests is a FAILURE, never a green run\nstdout: {}",
        stdout(&out)
    );
    let err = stderr(&out);
    assert!(
        err.contains("no tests were discovered"),
        "the message says why:\n{err}"
    );
}

#[test]
fn largo_test_fails_loudly_when_a_filter_matches_nothing() {
    let dir = tempfile::tempdir().unwrap();
    test_closure_project(dir.path());

    let out = largo_in(dir.path(), &["test", "no_such_test_anywhere"]);
    assert!(
        !out.status.success(),
        "a filter matching zero tests is a FAILURE\nstdout: {}",
        stdout(&out)
    );
    let err = stderr(&out);
    assert!(
        err.contains("no_such_test_anywhere"),
        "the message names the filter:\n{err}"
    );
    assert!(
        err.contains("matched none"),
        "the message says it matched nothing:\n{err}"
    );
}

// ============================================================================
// 4. `largo serve --watch` (DOM path): the build consumes the closure, and the
//    fingerprint covers EVERY closure file — an edit to a NESTED imported
//    module rebuilds and live-reloads.
// ============================================================================

/// GET `path` and return the FULL raw response bytes; retries until the server is up.
fn get_raw(port: u16, path: &str) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(mut s) => {
                // `Connection: close`: this client frames the response by
                // reading to EOF, so it opts out of the server's keep-alive.
                s.write_all(
                    format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
                        .as_bytes(),
                )
                .expect("write request");
                let mut raw = Vec::new();
                s.read_to_end(&mut raw).expect("read response");
                return raw;
            }
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => panic!("largo serve --watch never came up on port {port}: {e}"),
        }
    }
}

/// The response body (everything after the `\r\n\r\n` header terminator).
fn body(raw: &[u8]) -> &[u8] {
    raw.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| &raw[i + 4..])
        .unwrap_or(&[])
}

/// `largo emit wasm -o <path>` — the closure-assembling wasm reference the
/// served bundle must match byte-for-byte (wasm emission is deterministic).
fn emit_wasm_reference(project: &Path, out: &Path) -> Vec<u8> {
    let out_arg = out.to_str().unwrap().to_string();
    let emitted = largo()
        .args(["emit", "wasm", "-o", &out_arg])
        .current_dir(project)
        .output()
        .expect("largo emit wasm spawns");
    assert!(
        emitted.status.success(),
        "emit wasm reference: {}",
        String::from_utf8_lossy(&emitted.stderr)
    );
    std::fs::read(out).expect("emitted wasm exists")
}

#[test]
fn serve_watch_rebuilds_when_a_nested_imported_module_changes() {
    let dir = tempfile::tempdir().unwrap();
    project(
        dir.path(),
        "site",
        &[
            (
                "main.lg",
                "# Site\n\nImport Msg from \"ui/msg.lg\".\n\n\
             ## Component App\nState:\n    Let g be greeting().\nView:\n    <h1>{g}</h1>\n\n\
             ## Main\nMount App.\n",
            ),
            (
                "ui/msg.lg",
                "# Msg\n\n## To greeting () -> Text:\n    Return \"hello\".\n",
            ),
        ],
    );
    let refs = tempfile::tempdir().unwrap();
    let mut cmd = largo();
    cmd.args(["serve", "--watch", "--port", "0"])
        .current_dir(dir.path());
    let server = Server::spawn(cmd);
    let port = server.port();

    // The INITIAL build consumed the closure: the served wasm is byte-for-byte
    // the module `largo emit wasm` assembles from entry + imports. An
    // entry-only build produces a different (stub) module.
    let wasm0 = body(&get_raw(port, "/site.wasm")).to_vec();
    let ref0 = emit_wasm_reference(dir.path(), &refs.path().join("ref0.wasm"));
    assert_eq!(
        wasm0,
        ref0,
        "serve must build the SAME closure emit builds ({} vs {} bytes)",
        wasm0.len(),
        ref0.len()
    );
    let gen0 = String::from_utf8_lossy(body(&get_raw(port, "/__gen")))
        .trim()
        .to_string();
    assert_eq!(gen0, "1", "initial generation is 1");

    // Edit the NESTED imported module → the watcher must see it (recursive,
    // whole-closure fingerprint), rebuild FROM THE CLOSURE, and bump /__gen.
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(
        dir.path().join("src/ui/msg.lg"),
        "# Msg\n\n## To greeting () -> Text:\n    Return \"goodbye\".\n",
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let g = String::from_utf8_lossy(body(&get_raw(port, "/__gen")))
            .trim()
            .to_string();
        if g != "1" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the generation never bumped after the NESTED imported module edit — \
             the watcher does not track the closure"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let rebuilt = body(&get_raw(port, "/site.wasm")).to_vec();
    assert_ne!(
        rebuilt, wasm0,
        "the rebuild folded in the imported module's new content"
    );
    let ref1 = emit_wasm_reference(dir.path(), &refs.path().join("ref1.wasm"));
    assert_eq!(
        rebuilt, ref1,
        "the rebuilt bundle still matches the closure emit"
    );
}

// ============================================================================
// 5. Closure parity: the SAME multi-file program is accepted by check, test,
//    emit (c + rust), and build --emit wasm — a module missed by any one of
//    them is the false-green the audit hit.
// ============================================================================

#[test]
fn check_test_emit_and_build_consume_the_same_closure() {
    let dir = tempfile::tempdir().unwrap();
    test_closure_project(dir.path());

    let check = largo_in(dir.path(), &["check"]);
    assert!(check.status.success(), "check: {}", stderr(&check));

    let test = largo_in(dir.path(), &["test"]);
    assert!(test.status.success(), "test: {}", stderr(&test));
    assert!(
        stdout(&test).contains("3 passed"),
        "test saw the whole closure:\n{}",
        stdout(&test)
    );

    let emit_c = largo_in(dir.path(), &["emit", "c"]);
    assert!(emit_c.status.success(), "emit c: {}", stderr(&emit_c));
    assert!(
        stdout(&emit_c).contains("double"),
        "emitted C carries the imported function"
    );

    let emit_rust = largo_in(dir.path(), &["emit", "rust"]);
    assert!(
        emit_rust.status.success(),
        "emit rust: {}",
        stderr(&emit_rust)
    );
    assert!(
        stdout(&emit_rust).contains("double"),
        "emitted Rust carries the imported function"
    );

    let build = largo_in(dir.path(), &["build", "--emit", "wasm"]);
    assert!(
        build.status.success(),
        "build --emit wasm: {}",
        stderr(&build)
    );
    let wasm = std::fs::read(dir.path().join("target/app.wasm")).expect("wasm artifact exists");
    assert_eq!(&wasm[0..4], b"\0asm", "a real wasm module was produced");
}
