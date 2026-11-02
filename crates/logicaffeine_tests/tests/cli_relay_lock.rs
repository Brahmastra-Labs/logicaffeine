//! ★ LOCK — `largo serve` answers `/_relay` over a REAL WebSocket. ★
//!
//! A browser page's `Raise`/`Publish` reach the interpreter's in-process relay hub with no
//! network involved at all — that path was tested, but nothing ever dialed the actual
//! `largo serve` dev server and spoke the wire protocol to it. A page served by `largo serve`
//! that tried to open `new WebSocket("/_relay")` had never once been proven to get an answer:
//! the same "looks wired up" gap this project keeps a lock for on every other silent-drop
//! surface (`var(--undefined)`, `bind` registering no listener, …).
//!
//! `largo serve` reuses `logicaffeine_system::relay`'s hub rather than reimplementing it —
//! these tests dial the SAME `RelayClient` the in-process suites use, but through a real
//! spawned `largo serve` child and a real loopback socket, against both server shapes that
//! answer `/_relay` (`serve_web`'s pages server and `serve_dom`'s single-file DOM server).

#[path = "cli_common/mod.rs"]
mod common;

use common::{largo_in, stderr};
use logicaffeine_system::relay::RelayClient;
use std::path::Path;
use std::time::Duration;

/// `largo new <name> --web`, run for real — the same site shape `cli_web_theme_lock` scaffolds.
fn scaffold_web(dir: &Path) -> std::path::PathBuf {
    let out = largo_in(dir, &["new", "demo", "--web"]);
    assert!(out.status.success(), "largo new --web: {}", stderr(&out));
    dir.join("demo")
}

/// A single-file `## Component`/`Mount` program with no `Serve` statement — the shape
/// `serve.rs::cmd_serve_project` resolves to `ServePlan::Program` and, once it builds to wasm,
/// hands to `serve_dom::cmd_serve_dom` (the exact recipe `serve_watch.rs` scaffolds).
fn scaffold_dom(dir: &Path) -> std::path::PathBuf {
    common::scaffold(dir, "widget");
    std::fs::write(
        dir.join("src/main.lg"),
        "## Component App\nView:\n    <h1>hello</h1>\n\n## Main\nMount App.\n",
    )
    .expect("write a minimal DOM component");
    dir.to_path_buf()
}

/// Round-trip a publish through `/_relay` on a running server: two clients dial, both subscribe
/// to the same topic, one publishes, and — per `Hub::publish`'s publisher-excluded semantics —
/// only the OTHER client ever sees it. Asserts the real `RelayFrame::SubAck` and `Event` wire
/// shapes, not an in-process shortcut.
async fn assert_relay_roundtrip_excludes_the_publisher(port: u16) {
    let url = format!("ws://127.0.0.1:{port}/_relay");
    let mut a = tokio::time::timeout(Duration::from_secs(10), RelayClient::connect(&url))
        .await
        .expect("client A dials /_relay in time")
        .expect("client A's websocket handshake succeeds");
    let mut b = tokio::time::timeout(Duration::from_secs(10), RelayClient::connect(&url))
        .await
        .expect("client B dials /_relay in time")
        .expect("client B's websocket handshake succeeds");

    a.subscribe("room").await.expect("A's subscribe is acked");
    b.subscribe("room").await.expect("B's subscribe is acked");

    a.publish("room", b"hello from a".to_vec())
        .expect("A publishes");

    let (topic, data) = tokio::time::timeout(Duration::from_secs(5), b.next_event())
        .await
        .expect("B receives A's publish in time")
        .expect("event present");
    assert_eq!(topic, "room");
    assert_eq!(data, b"hello from a");

    // The publisher never receives its own event back.
    let echoed_to_publisher =
        tokio::time::timeout(Duration::from_millis(500), a.next_event()).await;
    assert!(
        echoed_to_publisher.is_err(),
        "the publishing client must not receive its own event back over /_relay"
    );
}

/// A GET to any other path on the same port is untouched by `/_relay` being wired up.
fn assert_ordinary_page_still_serves(server: &mut common::http::Server) {
    let raw = server.get("/");
    assert_eq!(
        common::http::status(&raw),
        200,
        "the site's own page must still answer 200"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn serve_web_answers_relay_over_a_real_websocket() {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = scaffold_web(dir.path());
    let mut server = common::http::Server::start(&root);
    let port = server.port();

    assert_ordinary_page_still_serves(&mut server);
    assert_relay_roundtrip_excludes_the_publisher(port).await;
    // The page route is still answered after a websocket has been dialed against the same loop.
    assert_ordinary_page_still_serves(&mut server);
}

#[tokio::test(flavor = "multi_thread")]
async fn serve_dom_answers_relay_over_a_real_websocket() {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = scaffold_dom(dir.path());
    let mut server = common::http::Server::start(&root);
    let port = server.port();

    assert_ordinary_page_still_serves(&mut server);
    assert_relay_roundtrip_excludes_the_publisher(port).await;
    assert_ordinary_page_still_serves(&mut server);
}
