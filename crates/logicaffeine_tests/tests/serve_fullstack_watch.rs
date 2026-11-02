//! B3 — full-stack `largo serve` watch + hot reload.
//!
//! One running server, edited live: a `pages/` edit rebuilds that page in
//! memory and bumps `/__gen`; a backend (`src/`) edit kills the child,
//! respawns it on the same port, waits for it to accept, THEN bumps `/__gen`
//! (so the browser reloads into the new backend, never a dead one); a
//! `static/` edit reloads the asset; a bad page edit serves the themed error
//! overlay while the last-good bundle stays live, and a fix snaps back.
//! Deadlines are generous — this box runs concurrent agents.

#[path = "cli_common/mod.rs"]
mod common;
use common::http::Server;

use common::{largo, largo_in, stderr};

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

/// GET `path`, returning the FULL raw response; retries until the server is up.
fn get_raw(port: u16, path: &str) -> Vec<u8> {
    // 300s, not the round 60s this used to be. MEASURED on this box: alone these tests take 7-13s,
    // and under the crate's own 448-test parallel run they were starved past 60s and reported as
    // "never answered" / "generation never ticked" — a broken dev server, which they were not.
    // What is being waited FOR is a full Rust backend build (`largo serve` compiles and spawns the
    // project's AOT backend, and rebuilds it on every edit), so the budget has to be sized for a
    // compile on a loaded machine rather than for a request. It is a HANG backstop, not a target:
    // the wait ends the moment the server answers.
    let deadline = Instant::now() + Duration::from_secs(300);
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
                if !raw.is_empty() {
                    return raw;
                }
            }
            Err(_) => {}
        }
        assert!(
            Instant::now() < deadline,
            "server never answered {path} on port {port}"
        );
        std::thread::sleep(Duration::from_millis(50));
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

/// The current `/__gen` value.
fn generation(port: u16) -> u64 {
    String::from_utf8_lossy(body(&get_raw(port, "/__gen")))
        .trim()
        .parse()
        .expect("generation is a number")
}

/// Wait until `/__gen` moves past `last`, returning the new value.
fn wait_for_tick(port: u16, last: u64, why: &str) -> u64 {
    // 300s, not the round 60s this used to be. MEASURED on this box: alone these tests take 7-13s,
    // and under the crate's own 448-test parallel run they were starved past 60s and reported as
    // "never answered" / "generation never ticked" — a broken dev server, which they were not.
    // What is being waited FOR is a full Rust backend build (`largo serve` compiles and spawns the
    // project's AOT backend, and rebuilds it on every edit), so the budget has to be sized for a
    // compile on a loaded machine rather than for a request. It is a HANG backstop, not a target:
    // the wait ends the moment the server answers.
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        let g = generation(port);
        if g > last {
            return g;
        }
        assert!(Instant::now() < deadline, "generation never ticked: {why}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn edits_to_pages_backend_and_static_hot_reload() {
    let dir = tempfile::tempdir().unwrap();
    let out = largo_in(dir.path(), &["new", "site", "--web"]);
    assert!(out.status.success(), "largo new --web: {}", stderr(&out));
    let root = dir.path().join("site");

    let mut cmd = largo();
    cmd.args(["serve", "--port", "0"]).current_dir(&root);
    let server = Server::spawn(cmd);
    let port = server.port();

    let mut gen = generation(port);
    assert_eq!(gen, 1, "initial generation is 1");
    let wasm0 = body(&get_raw(port, "/app.wasm")).to_vec();
    assert_eq!(&wasm0[..4], b"\0asm", "initial page module serves");

    // ---- 1. PAGE edit → the page rebuilds in memory + /__gen ticks. ----
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(
        root.join("pages/app.lg"),
        "# App\n\n## Page /\n\nView:\n    <h1>the edited page</h1>\n",
    )
    .unwrap();
    gen = wait_for_tick(port, gen, "after the pages/app.lg edit");
    let wasm1 = body(&get_raw(port, "/app.wasm")).to_vec();
    assert_eq!(&wasm1[..4], b"\0asm", "rebuilt module is real wasm");
    assert_ne!(
        wasm1, wasm0,
        "the page edit changed the served bundle bytes"
    );

    // ---- 2. BACKEND edit → child killed, respawned, THEN /__gen ticks. ----
    let before = get_text(port, "/api/hello");
    assert!(
        before.contains("Hello from site!"),
        "baseline api response:\n{before}"
    );
    std::thread::sleep(Duration::from_millis(50));
    let main = std::fs::read_to_string(root.join("src/main.lg")).unwrap();
    std::fs::write(
        root.join("src/main.lg"),
        main.replace("Hello from site!", "Hello from the EDITED backend!"),
    )
    .unwrap();
    gen = wait_for_tick(port, gen, "after the src/main.lg edit");
    // The tick lands only after the fresh child accepts, so the very next
    // proxied response must already be the new backend's.
    let after = get_text(port, "/api/hello");
    assert!(
        after.contains("Hello from the EDITED backend!"),
        "the respawned backend answers through the proxy:\n{after}"
    );

    // ---- 3. STATIC edit → the asset reloads + /__gen ticks. ----
    std::thread::sleep(Duration::from_millis(50));
    let css = std::fs::read_to_string(root.join("static/style.css")).unwrap();
    std::fs::write(
        root.join("static/style.css"),
        format!("{css}\n.hot-reload-probe {{ color: red }}\n"),
    )
    .unwrap();
    gen = wait_for_tick(port, gen, "after the static/style.css edit");
    let css_served = get_text(port, "/style.css");
    assert!(
        css_served.contains(".hot-reload-probe"),
        "the edited css is served from memory:\n{css_served}"
    );

    // ---- 4. BAD page edit → themed overlay + last-good bundle stays live. ----
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(
        root.join("pages/app.lg"),
        "# App\n\n## Page /\n\nView:\n    <h1>oops\n",
    )
    .unwrap();
    gen = wait_for_tick(port, gen, "after the bad page edit");
    let overlay = get_text(port, "/");
    assert!(
        overlay.contains("LOGOS build error"),
        "the overlay is served:\n{overlay}"
    );
    assert!(
        overlay.contains("class=\"caret\"") && overlay.contains('^'),
        "with a source caret:\n{overlay}"
    );
    let wasm_last_good = body(&get_raw(port, "/app.wasm")).to_vec();
    assert_eq!(
        wasm_last_good, wasm1,
        "the last-good page module stays live under the overlay"
    );

    // ---- 5. FIX the page → back to green. ----
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(
        root.join("pages/app.lg"),
        "# App\n\n## Page /\n\nView:\n    <h1>recovered</h1>\n",
    )
    .unwrap();
    wait_for_tick(port, gen, "after the page fix");
    let recovered = get_text(port, "/");
    assert!(
        !recovered.contains("LOGOS build error"),
        "the overlay clears after the fix:\n{recovered}"
    );
    assert!(
        recovered.contains("app.js"),
        "the page HTML is back:\n{recovered}"
    );
}
