//! Wave C — `largo serve --watch` for a DOM app.
//!
//! A DOM project (a `## Component`/`View:` program, no `Serve` statement) is BUILT to a wasm bundle
//! and served statically; `--watch` rebuilds on source change and bumps a `/__gen`
//! counter the injected browser client polls to live-reload. We spawn the real binary,
//! dial the bundle, then edit the source and watch the generation tick.

#[path = "cli_common/mod.rs"]
mod common;
use common::http::Server;
use common::scaffold;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Command;
use std::time::{Duration, Instant};

fn largo_bin() -> Command {
    common::largo()
}

/// GET `path` and return the FULL raw response bytes (headers + body); the request
/// carries `Connection: close` (this client frames the response by reading to EOF,
/// so it opts out of the server's keep-alive). Retries the connect until the
/// server is up.
fn get_raw(port: u16, path: &str) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(mut s) => {
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

fn get_text(port: u16, path: &str) -> String {
    String::from_utf8_lossy(&get_raw(port, path)).into_owned()
}

/// The response body (everything after the `\r\n\r\n` header terminator).
fn body(raw: &[u8]) -> &[u8] {
    raw.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| &raw[i + 4..])
        .unwrap_or(&[])
}

#[test]
fn serve_watch_serves_the_bundle_and_live_reloads_on_change() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path(), "site");
    let entry = dir.path().join("src/main.lg");
    std::fs::write(
        &entry,
        "## Component App\nView:\n    <h1>hello</h1>\n\n## Main\nMount App.\n",
    )
    .unwrap();
    let mut cmd = largo_bin();
    cmd.args(["serve", "--watch", "--port", "0"])
        .current_dir(dir.path());
    let server = Server::spawn(cmd);
    let port = server.port();

    // `/` is the host HTML that loads the module script for this project.
    let index = get_text(port, "/");
    assert!(
        index.starts_with("HTTP/1.1 200 OK\r\n"),
        "index status line:\n{index}"
    );
    assert!(
        index.contains("site.js"),
        "the HTML loads the app's module script:\n{index}"
    );

    // `/site.wasm` is the compiled module (starts with the wasm magic `\0asm`).
    let wasm = get_raw(port, "/site.wasm");
    assert!(
        String::from_utf8_lossy(&wasm).contains("application/wasm"),
        "wasm is served with its content type:\n{}",
        String::from_utf8_lossy(&wasm[..wasm.len().min(200)])
    );
    assert_eq!(
        &body(&wasm)[..4],
        b"\0asm",
        "the body is a real wasm module"
    );

    // `/site.js` carries the live-reload poll client.
    let js = get_text(port, "/site.js");
    assert!(
        js.contains("/__gen"),
        "the served JS embeds the live-reload client:\n{js}"
    );

    // The generation counter starts at 1.
    let gen0 = String::from_utf8_lossy(body(&get_raw(port, "/__gen")))
        .trim()
        .to_string();
    assert_eq!(gen0, "1", "initial generation is 1");

    // Edit the source → the watcher rebuilds → the generation bumps (the browser would
    // reload). Poll until it ticks, or fail after a generous window.
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(
        &entry,
        "## Component App\nView:\n    <h1>changed</h1>\n\n## Main\nMount App.\n",
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let g = String::from_utf8_lossy(body(&get_raw(port, "/__gen")))
            .trim()
            .to_string();
        if g != "1" {
            assert_eq!(g, "2", "the edit bumped the generation exactly once");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the generation never bumped after the source edit"
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    // The rebuilt bundle reflects the edit.
    let index2 = get_text(port, "/");
    assert!(
        index2.starts_with("HTTP/1.1 200 OK\r\n"),
        "still serving after rebuild:\n{index2}"
    );

    // A BAD edit doesn't crash the server — `/` serves the themed error overlay with the
    // offending source line + a caret, and keeps polling so a fix snaps it back to green.
    std::fs::write(
        &entry,
        "## Component App\nView:\n    <h1>oops\n\n## Main\nMount App.\n",
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let g = String::from_utf8_lossy(body(&get_raw(port, "/__gen")))
            .trim()
            .to_string();
        if g == "3" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the generation never bumped after the bad edit"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let overlay = get_text(port, "/");
    assert!(
        overlay.contains("LOGOS build error"),
        "the overlay is served:\n{overlay}"
    );
    assert!(
        overlay.contains("class=\"caret\"") && overlay.contains('^'),
        "with a source caret"
    );
    assert!(overlay.contains("h1"), "showing the offending source line");
    assert!(overlay.contains(":root {"), "themed with the design tokens");
}

/// ★ (a) + (c) — a page that is BROKEN FROM THE VERY FIRST SAVE the server ever sees must still
/// bind the port and serve the located error, not refuse to start at all: a watch-mode dev server
/// exists to survive the author's mistakes, including the first one (starting `largo serve
/// --watch` on a page mid-typo, or on a fresh scaffold nobody has finished writing yet). The next
/// save that fixes the page must recover it with no restart.
#[test]
fn serve_watch_starts_on_a_broken_page_and_recovers_on_the_fixing_save() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path(), "site");
    let entry = dir.path().join("src/main.lg");
    // Broken from the very first build the server ever attempts: an unclosed `<h1>`.
    std::fs::write(
        &entry,
        "## Component App\nView:\n    <h1>oops\n\n## Main\nMount App.\n",
    )
    .unwrap();
    let mut cmd = largo_bin();
    cmd.args(["serve", "--watch", "--port", "0"])
        .current_dir(dir.path());
    let server = Server::spawn(cmd);
    let port = server.port();

    // (a) — the port binds and `/` serves the located error; it must not refuse to start.
    let overlay = get_text(port, "/");
    assert!(
        overlay.starts_with("HTTP/1.1 200 OK\r\n"),
        "the server must bind and answer even when the STARTING page is broken:\n{overlay}"
    );
    assert!(
        overlay.contains("LOGOS build error"),
        "the overlay is served:\n{overlay}"
    );
    assert!(
        overlay.contains("class=\"caret\"") && overlay.contains('^'),
        "with a source caret:\n{overlay}"
    );
    assert!(
        overlay.contains("h1"),
        "showing the offending source line:\n{overlay}"
    );
    let gen0 = String::from_utf8_lossy(body(&get_raw(port, "/__gen")))
        .trim()
        .to_string();
    assert_eq!(gen0, "1", "the first (broken) build is still generation 1");

    // (c) — the fixing save recovers the page, with no restart.
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(
        &entry,
        "## Component App\nView:\n    <h1>fixed</h1>\n\n## Main\nMount App.\n",
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let g = String::from_utf8_lossy(body(&get_raw(port, "/__gen")))
            .trim()
            .to_string();
        if g != "1" {
            assert_eq!(g, "2", "the fix bumped the generation exactly once");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the generation never bumped after the fixing save"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let recovered = get_text(port, "/");
    assert!(
        !recovered.contains("LOGOS build error"),
        "the overlay clears after the fix:\n{recovered}"
    );
    assert!(
        recovered.contains("site.js"),
        "the real page is back:\n{recovered}"
    );
}

/// (b) — a page that starts CLEAN and breaks MID-SESSION (an author's typo saved while `--watch`
/// is already running) must not take the server down: `/` keeps answering, now with the located
/// error, and — the other half of the same guarantee — a further fixing save recovers it. This is
/// the mirror of the test above (which starts broken); together they lock both directions.
#[test]
fn serve_watch_a_broken_save_mid_session_keeps_the_server_up_and_shows_the_error() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path(), "site");
    let entry = dir.path().join("src/main.lg");
    std::fs::write(
        &entry,
        "## Component App\nView:\n    <h1>hello</h1>\n\n## Main\nMount App.\n",
    )
    .unwrap();
    let mut cmd = largo_bin();
    cmd.args(["serve", "--watch", "--port", "0"])
        .current_dir(dir.path());
    let server = Server::spawn(cmd);
    let port = server.port();

    let gen0 = String::from_utf8_lossy(body(&get_raw(port, "/__gen")))
        .trim()
        .to_string();
    assert_eq!(gen0, "1", "starts clean");

    // Break it mid-session.
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(
        &entry,
        "## Component App\nView:\n    <h1>oops\n\n## Main\nMount App.\n",
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let g = String::from_utf8_lossy(body(&get_raw(port, "/__gen")))
            .trim()
            .to_string();
        if g == "2" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the generation never bumped after the mid-session break — the server may have gone down"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let overlay = get_text(port, "/");
    assert!(
        overlay.starts_with("HTTP/1.1 200 OK\r\n"),
        "the server stays up and answers:\n{overlay}"
    );
    assert!(
        overlay.contains("LOGOS build error"),
        "the located error is shown:\n{overlay}"
    );

    // The fixing save recovers it, with no restart.
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(
        &entry,
        "## Component App\nView:\n    <h1>fixed</h1>\n\n## Main\nMount App.\n",
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let g = String::from_utf8_lossy(body(&get_raw(port, "/__gen")))
            .trim()
            .to_string();
        if g == "3" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the generation never bumped after the fixing save"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let recovered = get_text(port, "/");
    assert!(
        !recovered.contains("LOGOS build error"),
        "recovered after the fix:\n{recovered}"
    );
}

/// A DOM app whose entry `Import`s another module: when the IMPORTED module breaks, the overlay
/// names THAT module and its own line — not the entry that imported it, and not line 1 of the
/// combined source the two are compiled from together.
#[test]
fn serve_watch_overlay_names_the_imported_files_own_line() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path(), "site");
    let entry = dir.path().join("src/main.lg");
    let shared = dir.path().join("src/shared.lg");
    std::fs::write(
        &entry,
        "# Main\n\nImport Shared.\n\n## Component App\nView:\n    <h1>{greeting()}</h1>\n\n## Main\nMount App.\n",
    )
    .unwrap();
    std::fs::write(
        &shared,
        "# Shared\n\n## To greeting () -> Text:\n    Return \"hi\".\n",
    )
    .unwrap();
    let mut cmd = largo_bin();
    cmd.args(["serve", "--watch", "--port", "0"])
        .current_dir(dir.path());
    let server = Server::spawn(cmd);
    let port = server.port();

    let gen0 = String::from_utf8_lossy(body(&get_raw(port, "/__gen")))
        .trim()
        .to_string();
    assert_eq!(gen0, "1", "the import compiles cleanly at first");

    // Break the IMPORTED file, on a known line — the entry itself is never touched.
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(
        &shared,
        "# Shared\n\n## To greeting () -> Text:\n    Retrun \"hi\".\n",
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let g = String::from_utf8_lossy(body(&get_raw(port, "/__gen")))
            .trim()
            .to_string();
        if g == "2" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the generation never bumped after breaking the import"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let overlay = get_text(port, "/");
    assert!(
        overlay.contains("shared.lg, line 4"),
        "the overlay names the imported file at its own line:\n{overlay}"
    );
    assert!(
        !overlay.contains("main.lg"),
        "the mistake is in shared.lg, not the entry that imported it:\n{overlay}"
    );
}
