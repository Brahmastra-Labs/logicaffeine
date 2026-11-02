//! B2 — full-stack `largo serve`: ONE public port serving built pages,
//! static assets, and the LOGOS backend (a child process) behind a proxy.
//!
//! We scaffold with `largo new <name> --web` exactly as a user would, spawn the
//! real binary, and dial the public port: `/` is the built front page,
//! `/app.wasm` the compiled page module, `/style.css` the static asset, and
//! anything unmatched (the `/api/...` routes) proxies to the backend child
//! running the entry's `Serve` statement on a largo-chosen port
//! (`LOGOS_PORT`). A static file shadows a backend route (exact match wins);
//! a dead backend answers a themed 502 overlay instead of a hang.

#[path = "cli_common/mod.rs"]
mod common;
use common::http::Server;

use common::{largo, largo_in, stderr};

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::{Duration, Instant};

/// Scaffold a `--web` project named `name` under `dir` (the real user path).
fn scaffold_web(dir: &Path, name: &str) {
    let out = largo_in(dir, &["new", name, "--web"]);
    assert!(out.status.success(), "largo new --web: {}", stderr(&out));
}

/// GET `path`, returning the FULL raw response (headers + body). The request
/// opts out of keep-alive so reading to EOF frames the response. Retries the
/// connect until the server is up.
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
            "largo serve never answered {path} on port {port}"
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

#[test]
fn fullstack_serve_unifies_pages_static_and_backend_on_one_port() {
    let dir = tempfile::tempdir().unwrap();
    scaffold_web(dir.path(), "site");
    let root = dir.path().join("site");

    // A static file that SHADOWS a proxied path: the asset must win over the
    // backend's catch-all route.
    std::fs::write(root.join("static/hello.txt"), "from the asset").unwrap();

    let mut cmd = largo();
    cmd.args(["serve", "--port", "0"]).current_dir(&root);
    let server = Server::spawn(cmd);
    let port = server.port();

    // `/` is the built front page (pages/app.lg → wasm + host HTML).
    let index = get_text(port, "/");
    assert!(
        index.starts_with("HTTP/1.1 200 OK\r\n"),
        "index status:\n{index}"
    );
    assert!(
        index.contains("Content-Type: text/html"),
        "index content type:\n{index}"
    );
    assert!(
        index.contains("app.js"),
        "the page HTML loads its module script:\n{index}"
    );

    // `/app.wasm` is the compiled page module with its content type.
    let wasm = get_raw(port, "/app.wasm");
    assert!(
        String::from_utf8_lossy(&wasm).contains("application/wasm"),
        "wasm content type:\n{}",
        String::from_utf8_lossy(&wasm[..wasm.len().min(200)])
    );
    assert_eq!(
        &body(&wasm)[..4],
        b"\0asm",
        "the body is a real wasm module"
    );

    // `/style.css` is the scaffold's static asset, byte-for-byte.
    let css_disk = std::fs::read_to_string(root.join("static/style.css")).unwrap();
    let css = get_raw(port, "/style.css");
    assert!(
        String::from_utf8_lossy(&css).contains("text/css"),
        "css content type:\n{}",
        String::from_utf8_lossy(&css[..css.len().min(200)])
    );
    assert_eq!(
        body(&css),
        css_disk.as_bytes(),
        "the css bytes are served verbatim"
    );

    // A backend route reaches the LOGOS handler THROUGH the proxy.
    let api = get_text(port, "/api/hello");
    assert!(api.starts_with("HTTP/1.1 200 OK\r\n"), "api status:\n{api}");
    assert!(
        api.contains("Hello from site!"),
        "the backend handler's response came through the proxy:\n{api}"
    );
    // …and a text/plain response is NEVER touched by the reload injector.
    assert!(
        !api.contains("/__gen"),
        "the reload client must not leak into non-HTML responses:\n{api}"
    );

    // A proxied text/html response carries the injected reload client (dev
    // only), with Content-Length recomputed to frame the spliced body.
    let about_raw = get_raw(port, "/api/about");
    let about = String::from_utf8_lossy(&about_raw).into_owned();
    assert!(
        about.starts_with("HTTP/1.1 200 OK\r\n"),
        "about status:\n{about}"
    );
    assert!(
        about.contains("<h1>site</h1>"),
        "the handler's HTML came through:\n{about}"
    );
    assert!(
        about.contains("/__gen"),
        "proxied text/html gets the reload client injected:\n{about}"
    );
    let declared: usize = about
        .lines()
        .find_map(|l| l.strip_prefix("Content-Length: "))
        .expect("proxied response declares Content-Length")
        .trim()
        .parse()
        .expect("Content-Length is a number");
    assert_eq!(
        declared,
        body(&about_raw).len(),
        "Content-Length reframes the injected body exactly:\n{about}"
    );

    // Precedence: the static file shadows the backend's catch-all (which would
    // have answered 404 Not Found).
    let shadowed = get_text(port, "/hello.txt");
    assert!(
        shadowed.starts_with("HTTP/1.1 200 OK\r\n"),
        "asset wins:\n{shadowed}"
    );
    assert!(
        shadowed.ends_with("from the asset"),
        "asset body:\n{shadowed}"
    );

    // An unshadowed unknown path proxies to the backend's catch-all.
    let missing = get_text(port, "/no-such-thing");
    assert!(
        missing.starts_with("HTTP/1.1 404"),
        "unmatched paths proxy to the backend:\n{missing}"
    );

    // The generation counter is live for the reload client.
    let gen = get_text(port, "/__gen");
    assert!(gen.starts_with("HTTP/1.1 200 OK\r\n"), "gen status:\n{gen}");
    assert_eq!(
        String::from_utf8_lossy(body(&get_raw(port, "/__gen"))).trim(),
        "1",
        "initial generation is 1"
    );
}

#[test]
fn a_dead_backend_answers_a_themed_502_not_a_hang() {
    let dir = tempfile::tempdir().unwrap();
    scaffold_web(dir.path(), "site");
    let root = dir.path().join("site");

    // A backend that CRASHES before its Serve statement executes: the AST
    // still carries `Serve` (so routing picks the full-stack path), but the
    // child dies on boot.
    std::fs::write(
        root.join("src/main.lg"),
        "# Site\n\n\
         ## To handle (request: Request) -> Response:\n    \
             Return plain(\"unreachable\").\n\n\
         ## Main\n\n\
         Let xs be [1].\n\
         Show item 5 of xs.\n\
         Serve on port 8080 with handle.\n",
    )
    .unwrap();

    let mut cmd = largo();
    cmd.args(["serve", "--port", "0"]).current_dir(&root);
    let server = Server::spawn(cmd);
    let port = server.port();

    // Pages still serve (the front page is independent of the backend).
    let index = get_text(port, "/");
    assert!(
        index.starts_with("HTTP/1.1 200 OK\r\n"),
        "pages survive a dead backend:\n{index}"
    );

    // A proxied path answers a themed 502 overlay — never a hang or a reset.
    let api = get_text(port, "/api/hello");
    assert!(
        api.starts_with("HTTP/1.1 502"),
        "dead backend is a 502:\n{api}"
    );
    assert!(api.contains(":root {"), "the 502 page is themed:\n{api}");
    assert!(
        api.to_lowercase().contains("backend"),
        "the 502 page says what died:\n{api}"
    );
}
