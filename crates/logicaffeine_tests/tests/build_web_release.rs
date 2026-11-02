//! B4 — the "one self-contained binary serves the whole site" lock.
//!
//! `largo build --release` on a `--web` scaffold must produce ONE native binary
//! that, run alone (nothing but `PORT`), serves the WHOLE full-stack site from
//! inside itself: the embedded front page, the compiled page module, the static
//! asset, AND the LOGOS backend route — every zone `largo serve` serves in dev,
//! now baked into the artifact. This is the ground truth the codegen canary
//! (`crates/logicaffeine_tests/tests/serve_embed_assets_lock.rs`) stands in for.
//!
//! We scaffold with `largo new <name> --web` exactly as a user would, build
//! release into a shared cargo cache (so the runtime crates aren't recompiled
//! per run), locate the STABLE artifact (`target/release/<name>` — the deploy
//! target B4 lifts out of the buried cargo tree), spawn it with `PORT`, and dial
//! it with raw `TcpStream` requests. `#[ignore]`: it compiles an AOT binary.

#[path = "cli_common/mod.rs"]
mod common;

use common::largo_in;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

fn shared_cache() -> PathBuf {
    std::env::temp_dir().join("logos_e2e_cache")
}

/// GET `path`, returning the FULL raw response (headers + body). Opts out of
/// keep-alive so reading to EOF frames the response; retries the connect until
/// the server is up (or `timeout`).
fn get_raw(port: u16, path: &str, timeout: Duration) -> Vec<u8> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) {
            let _ = s.write_all(
                format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes(),
            );
            let mut raw = Vec::new();
            let _ = s.read_to_end(&mut raw);
            if !raw.is_empty() {
                return raw;
            }
        }
        assert!(
            Instant::now() < deadline,
            "the web binary never answered {path} on port {port}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The response body (everything after the `\r\n\r\n` header terminator).
fn body(raw: &[u8]) -> &[u8] {
    raw.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| &raw[i + 4..])
        .unwrap_or(&[])
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[test]
#[ignore = "compiles an AOT binary via cargo (slow); run in the full suite"]
fn a_release_web_binary_serves_the_whole_embedded_site() {
    let dir = tempfile::tempdir().unwrap();
    // A unique package name so the cached artifact never collides across runs.
    let name = format!("websrv_{}", std::process::id());

    // Scaffold the real `--web` shape (backend + pages/app.lg + static/style.css).
    let out = largo_in(dir.path(), &["new", &name, "--web"]);
    assert!(
        out.status.success(),
        "largo new --web failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let root = dir.path().join(&name);

    // Build release into the shared cache (runtime crates cached). The stable
    // artifact (`<root>/target/release/<name>`) is copied from cargo's output,
    // which `CARGO_TARGET_DIR` moves into the cache — B4's stable-copy resolves
    // the source from that env var, so the in-project stable path is produced
    // either way. We prefer it and fall back to the cache only defensively.
    let build = largo()
        .args(["build", "--release"])
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", shared_cache())
        .output()
        .expect("largo build --release spawns");
    assert!(
        build.status.success(),
        "largo build --release failed:\n{}\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );

    // The stable, deployable artifact B4 lifts out of the buried cargo tree.
    let stable = root.join("target").join("release").join(&name);
    let cached = shared_cache().join("release").join(&name);
    assert!(
        stable.exists(),
        "the stable artifact was not produced at {} (build must copy it out of the cargo tree)",
        stable.display()
    );
    let artifact = if stable.exists() {
        stable.clone()
    } else {
        cached.clone()
    };

    // Spawn the artifact alone — nothing but PORT. This is the deploy scenario:
    // ONE file, run from anywhere.
    let spawn_dir = tempfile::tempdir().unwrap(); // a cwd with NO project files
    let mut web = Command::new(&artifact);
    web.env("PORT", "0").current_dir(spawn_dir.path());
    let server = common::http::Server::spawn(web);
    let port = server.port();

    let up = Duration::from_secs(30);

    // `/` — the embedded front page (pages/app.lg → host HTML), 200 text/html,
    // loading its module script.
    let index = get_raw(port, "/", up);
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
    // The shim's name is CONTENT-ADDRESSED (`/app.<hash>.js`) so it can be cached forever, which
    // is why this looks for the shape rather than a literal — the hash is of the bytes, and
    // spelling it would pin today's build (`asset_cache_lock`).
    let index_body = String::from_utf8_lossy(body(&index)).into_owned();
    let shim = index_body
        .split('"')
        .find(|t| t.starts_with("./app.") && t.ends_with(".js"))
        .map(|t| t.trim_start_matches('.').to_string())
        .unwrap_or_else(|| panic!("the front page loads its module:\n{index_body}"));

    // The compiled page module, application/wasm, real wasm magic — fetched by the name the SHIM
    // actually asks for, so this proves the reference and the asset agree rather than assuming it.
    let shim_body = String::from_utf8_lossy(body(&get_raw(port, &shim, up))).into_owned();
    let module = shim_body
        .split('"')
        .find(|t| t.trim_start_matches("./").starts_with("app.") && t.ends_with(".wasm"))
        .map(|t| format!("/{}", t.trim_start_matches("./")))
        .unwrap_or_else(|| panic!("the shim fetches its module:\n{shim_body}"));
    let wasm = get_raw(port, &module, up);
    assert!(
        contains(&wasm, b"application/wasm"),
        "wasm content type:\n{}",
        String::from_utf8_lossy(&wasm[..wasm.len().min(200)])
    );
    assert_eq!(
        &body(&wasm)[..4],
        b"\0asm",
        "the body is a real wasm module"
    );

    // `/style.css` — the scaffold's static asset, byte-for-byte, text/css.
    let css_disk = std::fs::read(root.join("static/style.css")).unwrap();
    let css = get_raw(port, "/style.css", up);
    assert!(
        contains(&css, b"text/css"),
        "css content type:\n{}",
        String::from_utf8_lossy(&css[..css.len().min(200)])
    );
    assert_eq!(
        body(&css),
        css_disk.as_slice(),
        "the css bytes are served verbatim from inside the binary"
    );

    // `/api/hello` — the LOGOS backend route reaches the handler (assets-first
    // precedence does not shadow a path with no asset).
    let api = get_raw(port, "/api/hello", up);
    assert!(
        api.starts_with(b"HTTP/1.1 200 OK\r\n"),
        "api status:\n{}",
        String::from_utf8_lossy(&api)
    );
    assert!(
        contains(body(&api), format!("Hello from {name}!").as_bytes()),
        "the backend handler answered:\n{}",
        String::from_utf8_lossy(&api)
    );

    // An unknown path with no asset falls through to the backend's catch-all 404.
    let missing = get_raw(port, "/no-such-thing", up);
    assert!(
        missing.starts_with(b"HTTP/1.1 404"),
        "an unmatched path reaches the backend catch-all:\n{}",
        String::from_utf8_lossy(&missing)
    );
}

fn largo() -> Command {
    common::largo()
}
