//! The FULL-STACK TODO DEPLOY SMOKE — the capstone "it actually deploys" lock.
//!
//! `examples/fullstack_todo/` is a real pure-LOGOS web app: `src/server.lg` is the
//! backend (routes + the todo store), `pages/app.lg` is the reactive frontend
//! compiled to WASM. This test copies that EXACT example, runs `largo build
//! --release` on it, and then exercises the resulting ONE self-contained binary
//! end to end — the deploy scenario, run from an empty directory with nothing but
//! `PORT` (and `TODO_STORE` to keep the file store hermetic):
//!
//!   * the embedded front page (`/`), the compiled page module (`/app.wasm`, real
//!     wasm magic), and the static asset (`/style.css`, byte-exact) all serve from
//!     inside the binary;
//!   * the LOGOS backend serves the real todo CRUD over `/api/todos`, and STATE
//!     PERSISTS across requests through the file store — an add is visible on the
//!     next GET, a delete removes it, exactly as a deployed app must behave;
//!   * an unknown path falls through to the backend's 404.
//!
//! `#[ignore]`: it compiles an AOT release binary (slow). Run deliberately:
//!   cargo test -p logicaffeine-tests --test todo_deploy_e2e -- --ignored --nocapture

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn shared_cache() -> PathBuf {
    std::env::temp_dir().join("logos_e2e_cache")
}

#[path = "support/under_test.rs"]
mod under_test;

#[path = "support/served.rs"]
mod served;
use served::Server;

fn largo() -> Command {
    Command::new(under_test::largo())
}

/// Recursively copy a directory tree (the example → a scratch build dir, so the
/// repo's `examples/` is never touched by the build).
fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
}

/// Send one raw request (any method) and return the FULL raw response. Opts out
/// of keep-alive so read-to-EOF frames the response; retries the connect until
/// the server is up (or `timeout`).
fn request(port: u16, method: &str, path: &str, body: &str, timeout: Duration) -> Vec<u8> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) {
            let req = format!(
                "{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let _ = s.write_all(req.as_bytes());
            let mut raw = Vec::new();
            let _ = s.read_to_end(&mut raw);
            if !raw.is_empty() {
                return raw;
            }
        }
        assert!(
            Instant::now() < deadline,
            "the todo binary never answered {method} {path} on port {port}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn get(port: u16, path: &str) -> Vec<u8> {
    request(port, "GET", path, "", Duration::from_secs(30))
}
fn post(port: u16, path: &str, body: &str) -> Vec<u8> {
    request(port, "POST", path, body, Duration::from_secs(30))
}

fn body(raw: &[u8]) -> &[u8] {
    raw.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| &raw[i + 4..])
        .unwrap_or(&[])
}
fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}
fn body_str(raw: &[u8]) -> String {
    String::from_utf8_lossy(body(raw)).into_owned()
}

#[test]
#[ignore = "compiles an AOT release binary via cargo (slow); run deliberately"]
fn the_fullstack_todo_app_deploys_as_one_binary_and_serves_the_whole_crud() {
    // Copy the REAL example into a scratch dir (repo `examples/` stays pristine).
    let build_dir = tempfile::tempdir().unwrap();
    let root = build_dir.path().join("fullstack_todo");
    let example = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join("fullstack_todo");
    assert!(
        example.join("Largo.toml").exists(),
        "the example must exist at {}",
        example.display()
    );
    copy_dir(&example, &root);

    // `largo build --release` — compile pages/app.lg → wasm, embed the site, and
    // compile the backend into ONE self-contained native binary.
    let build = largo()
        .args(["build", "--release"])
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", shared_cache())
        .output()
        .expect("largo build --release spawns");
    assert!(
        build.status.success(),
        "largo build --release failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );

    // The stable, deployable artifact B4 lifts out of the buried cargo tree.
    let artifact = root.join("target").join("release").join("fullstack_todo");
    assert!(
        artifact.exists(),
        "the deployable binary was not produced at {}",
        artifact.display()
    );

    // Deploy it: run from an EMPTY directory with nothing but PORT — plus a
    // hermetic TODO_STORE so the file-backed store lives in this test's tempdir
    // (the AOT binary reads real env, unlike the sealed tree-walker). TODO_DB_URL
    // stays unset, so the file store arm runs (no database needed for the smoke).
    let store = build_dir.path().join("todo_store.txt");
    let spawn_dir = tempfile::tempdir().unwrap();
    let mut todo = Command::new(&artifact);
    todo.env("PORT", "0")
        .env("TODO_STORE", &store)
        .env_remove("TODO_DB_URL")
        .current_dir(spawn_dir.path());
    let server = Server::spawn(todo);
    let port = server.port();

    // ── The embedded frontend ────────────────────────────────────────────────
    // `/` — the front page (from pages/app.lg), 200 text/html, loading its module.
    let index = get(port, "/");
    assert!(
        index.starts_with(b"HTTP/1.1 200 OK\r\n"),
        "index status:\n{}",
        String::from_utf8_lossy(&index)
    );
    assert!(
        contains(&index, b"Content-Type: text/html"),
        "index content type:\n{}",
        String::from_utf8_lossy(&index)
    );
    // The shim's name is CONTENT-ADDRESSED (`/app.<hash>.js`) so it can be cached forever — the
    // shape rather than a literal, since the hash is of the bytes (`asset_cache_lock`).
    let index_body = body_str(&index);
    let shim = index_body
        .split('"')
        .find(|t| t.starts_with("./app.") && t.ends_with(".js"))
        .map(|t| t.trim_start_matches('.').to_string())
        .unwrap_or_else(|| panic!("the front page loads its module:\n{index_body}"));

    // The compiled reactive frontend, application/wasm, real magic — fetched by the name the SHIM
    // asks for, so the reference and the asset are proven to agree rather than assumed to.
    let shim_body = body_str(&get(port, &shim));
    let module = shim_body
        .split('"')
        .find(|t| t.trim_start_matches("./").starts_with("app.") && t.ends_with(".wasm"))
        .map(|t| format!("/{}", t.trim_start_matches("./")))
        .unwrap_or_else(|| panic!("the shim fetches its module:\n{shim_body}"));
    let wasm = get(port, &module);
    assert!(
        contains(&wasm, b"application/wasm"),
        "wasm content type:\n{}",
        String::from_utf8_lossy(&wasm[..wasm.len().min(200)])
    );
    assert_eq!(
        &body(&wasm)[..4],
        b"\0asm",
        "the frontend body is a real wasm module"
    );

    // `/style.css` — the static asset, byte-for-byte from inside the binary.
    let css_disk = std::fs::read(root.join("static/style.css")).unwrap();
    let css = get(port, "/style.css");
    assert!(
        contains(&css, b"text/css"),
        "css content type:\n{}",
        String::from_utf8_lossy(&css[..css.len().min(200)])
    );
    assert_eq!(
        body(&css),
        css_disk.as_slice(),
        "the css bytes are served verbatim from the binary"
    );

    // ── The LOGOS backend CRUD, with real persistence ────────────────────────
    // Fresh store: the list is empty.
    let list0 = get(port, "/api/todos");
    assert!(
        list0.starts_with(b"HTTP/1.1 200 OK\r\n"),
        "GET /api/todos status:\n{}",
        String::from_utf8_lossy(&list0)
    );
    // ★ AN EMPTY LIST IS `[]`, NOT AN EMPTY BODY. The old wire answered nothing at all for
    // "no rows"; JSON answers a value the program HAS. Everything below is the derived
    // `toJsonSeq_Todo`, read off a running binary rather than predicted.
    assert_eq!(
        body_str(&list0).trim(),
        "[]",
        "a fresh store lists no todos"
    );

    // Add "buy milk" → the backend appends and returns the fresh list.
    let add1 = post(port, "/api/todos", "text=buy%20milk");
    assert!(
        contains(&add1, b"application/json"),
        "an add answers JSON:\n{}",
        String::from_utf8_lossy(&add1)
    );
    assert_eq!(
        body_str(&add1),
        "[{\"id\":1,\"text\":\"buy milk\",\"done\":false}]",
        "POST add returns the new list"
    );

    // A NEW GET reflects the persisted state (proving the file store survived the
    // stateless handler between two independent requests).
    assert_eq!(
        body_str(&get(port, "/api/todos")),
        "[{\"id\":1,\"text\":\"buy milk\",\"done\":false}]",
        "the add persisted across requests"
    );

    // Add "write tests" → two rows.
    let add2 = post(port, "/api/todos", "text=write%20tests");
    assert_eq!(
        body_str(&add2),
        "[{\"id\":1,\"text\":\"buy milk\",\"done\":false},{\"id\":2,\"text\":\"write tests\",\"done\":false}]",
        "the second add appends"
    );

    // Delete id 1 → only "write tests" remains, and it persists.
    let del = post(port, "/api/todos/delete/1", "");
    assert_eq!(
        body_str(&del),
        "[{\"id\":2,\"text\":\"write tests\",\"done\":false}]",
        "delete removes the row and returns the fresh list"
    );
    assert_eq!(
        body_str(&get(port, "/api/todos")),
        "[{\"id\":2,\"text\":\"write tests\",\"done\":false}]",
        "the delete persisted"
    );

    // ★ TOGGLE, which this test has never exercised. It is the one route whose answer CHANGES a
    // row rather than adding or removing one, so a serializer that dropped a Bool field, or a
    // path parameter that never bound, would show up here and nowhere else.
    let tog = post(port, "/api/todos/toggle/2", "");
    assert_eq!(
        body_str(&tog),
        "[{\"id\":2,\"text\":\"write tests\",\"done\":true}]",
        "toggle flips `done` on the row its PATH names"
    );
    assert_eq!(
        body_str(&get(port, "/api/todos")),
        "[{\"id\":2,\"text\":\"write tests\",\"done\":true}]",
        "and the toggle persisted"
    );

    // ── Routing edge ─────────────────────────────────────────────────────────
    // An unknown path with no asset and no API route falls through to 404.
    let missing = get(port, "/no-such-thing");
    assert!(
        missing.starts_with(b"HTTP/1.1 404"),
        "an unmatched path is a 404:\n{}",
        String::from_utf8_lossy(&missing)
    );
}
