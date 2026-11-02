//! ★ LOCK — a backend whose pages talk to the relay answers `/_relay` itself, on every tier. ★
//!
//! A page reaches the relay only through the server that served it: the browser dials `/_relay` on
//! the page's own origin. `largo serve` answered it (`cli_relay_lock`), and nothing else did — so a
//! page's `Publish`, its `When a message arrives on …` handlers and its `shared on` stores all went
//! silent the moment the app was deployed, on a compiled binary and on the interpreter alike.
//!
//! The rule needs no new syntax: the backend serving pages that publish, listen on a topic or share
//! a store answers `/_relay` from its own `Serve` — the interpreter for the pages its program
//! declares (`analysis::relay::pages_use_the_relay`), a compiled binary for the `pages/` it embeds
//! (`logos_assets::USES_RELAY`). Both HTTP engines hand a registered upgrade over through the one
//! `try_upgrade` decision, and a binary with no pages compiles to exactly the `serve` it always did.
//!
//! Every relay row is two real WebSocket clients on real loopback sockets: both subscribe, one
//! publishes, and only the other receives it.

#![cfg(not(target_arch = "wasm32"))]

mod common;

#[path = "cli_common/mod.rs"]
mod cli;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use logicaffeine_compile::compile::compile_to_rust;
use logicaffeine_system::http::wire::{HttpRequest, HttpResponse};
use logicaffeine_system::relay::{RelayClient, RelayHub};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn wait_for_port(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "nothing ever listened on {port}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Two clients on `/_relay` at `port`: both subscribe to one topic, A publishes, B receives it, and
/// A never hears its own message back.
fn assert_two_clients_relay(port: u16) {
    let rt = tokio::runtime::Runtime::new().expect("a runtime for the clients");
    rt.block_on(async {
        let url = format!("ws://127.0.0.1:{port}/_relay");
        let dial = || async {
            tokio::time::timeout(Duration::from_secs(10), RelayClient::connect(&url))
                .await
                .expect("the client dials /_relay in time")
                .expect("the websocket handshake completes")
        };
        let mut a = dial().await;
        let mut b = dial().await;
        a.subscribe("room")
            .await
            .expect("A's subscription is acknowledged");
        b.subscribe("room")
            .await
            .expect("B's subscription is acknowledged");
        a.publish("room", b"hello from a".to_vec())
            .expect("A publishes");
        let (topic, data) = tokio::time::timeout(Duration::from_secs(5), b.next_event())
            .await
            .expect("B hears A in time")
            .expect("an event arrives");
        assert_eq!(
            (topic.as_str(), data.as_slice()),
            ("room", b"hello from a".as_slice())
        );
        let echoed = tokio::time::timeout(Duration::from_millis(300), a.next_event()).await;
        assert!(echoed.is_err(), "the publisher never hears its own message");
    });
}

/// One raw request and everything the server wrote back before closing (or going quiet).
fn raw_exchange(port: u16, request: &[u8]) -> Vec<u8> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_millis(1500)))
        .unwrap();
    s.write_all(request).unwrap();
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }
    out
}

fn ok(body: &str) -> HttpResponse {
    let mut headers = std::collections::BTreeMap::new();
    headers.insert("Content-Type".to_string(), "text/plain".to_string());
    HttpResponse {
        status: 200,
        headers,
        body: body.as_bytes().to_vec(),
    }
}

/// `serve_with_upgrades` on the engine `LOGOS_HTTP_ENGINE` names, with the relay registered.
fn start_engine(engine: &str) -> u16 {
    // SAFETY-free on edition 2021; nextest runs each test in its own process, so the engine choice
    // reaches only this server.
    std::env::set_var("LOGOS_HTTP_ENGINE", engine);
    let port = free_port();
    std::thread::spawn(move || {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async move {
                let hub = RelayHub::new();
                let addr = format!("127.0.0.1:{port}");
                logicaffeine_system::http::server::serve_with_upgrades(
                    &addr,
                    |req: HttpRequest| ok(&format!("page {}", req.path)),
                    hub.upgrades(),
                )
                .await
                .expect("serve");
            });
    });
    wait_for_port(port);
    port
}

fn assert_engine_serves_the_relay(engine: &str) {
    let port = start_engine(engine);
    assert_two_clients_relay(port);
    let page = raw_exchange(
        port,
        b"GET /home HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    );
    let page = String::from_utf8_lossy(&page);
    assert!(
        page.starts_with("HTTP/1.1 200") && page.ends_with("page /home"),
        "{engine}: an ordinary request is untouched:\n{page}"
    );
    // A request pipelined ahead of the upgrade is answered first, then the handshake.
    let both = raw_exchange(
        port,
        b"GET /first HTTP/1.1\r\nHost: x\r\n\r\nGET /_relay HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
    );
    let both = String::from_utf8_lossy(&both);
    let first = both
        .find("page /first")
        .unwrap_or_else(|| panic!("{engine}: the pipelined request is answered:\n{both}"));
    let switch = both
        .find("HTTP/1.1 101 Switching Protocols")
        .unwrap_or_else(|| panic!("{engine}: then the upgrade:\n{both}"));
    assert!(first < switch, "{engine}: in order:\n{both}");
    assert!(
        both.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo="),
        "{engine}: RFC 6455's own accept key:\n{both}"
    );
    // A handshake that cannot complete is dropped with no response at all.
    let refused = raw_exchange(
        port,
        b"GET /_relay HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n",
    );
    assert!(
        refused.is_empty(),
        "{engine}: an upgrade with no key gets nothing back: {:?}",
        String::from_utf8_lossy(&refused)
    );
}

#[test]
fn the_tokio_engine_hands_the_relay_its_connections() {
    assert_engine_serves_the_relay("tokio");
}

#[cfg(all(target_os = "linux", feature = "io-uring"))]
#[test]
fn the_io_uring_engine_hands_the_relay_its_connections() {
    assert_engine_serves_the_relay("uring");
}

/// A page that publishes, and a backend that serves it.
const PUBLISHING_BACKEND: &str = "\
## Page /
State:
    Let sent be 0.
Actions:
    To ping:
        Publish \"hi\" on \"room\".
View:
    <button on-click={ping}>ping</button>

## To handle (request: Request) -> Response:
    Return plain(\"backend\").

## Main
Serve on port 8080 with handle.
";

/// The same backend with no page on the relay.
const QUIET_BACKEND: &str = "\
## Page /
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <button on-click={bump}>{n}</button>

## To handle (request: Request) -> Response:
    Return plain(\"backend\").

## Main
Serve on port 8080 with handle.
";

/// The deploy tier's Rust for `source`, which must carry no refusal: `compile_to_rust` returns `Ok`
/// for a program it refused, with the refusal embedded as a `compile_error!`.
fn deploys(source: &str) -> String {
    let rust = compile_to_rust(source).expect("compiles");
    let refusals: Vec<&str> = rust
        .lines()
        .filter(|l| l.contains("compile_error!"))
        .collect();
    assert!(
        refusals.is_empty(),
        "the binary would not build:\n{}",
        refusals.join("\n")
    );
    rust
}

#[test]
fn only_an_embedded_site_compiles_the_relay_into_its_serve() {
    let plain = deploys(SITE_BACKEND);
    assert!(
        plain.contains("logicaffeine_system::http::server::serve(&"),
        "the plain serve:\n{plain}"
    );
    assert!(
        !plain.contains("serve_with_upgrades") && !plain.contains("RelayHub"),
        "no relay:\n{plain}"
    );
    let embedded = logicaffeine_compile::codegen::with_embed_assets(|| deploys(SITE_BACKEND));
    assert!(
        embedded.contains("serve_with_upgrades(")
            && embedded.contains("if logos_assets::USES_RELAY { logicaffeine_system::relay::RelayHub::new().upgrades() }"),
        "an embedded site's own pages decide it:\n{embedded}"
    );
}

/// The backend of a full-stack project: a handler, and pages beside it in `pages/`.
const SITE_BACKEND: &str = "# Relay site\n\n## To handle (request: Request) -> Response:\n    Return plain(\"backend\").\n\n## Main\n\nServe on port 8080 with handle.\n";

/// The site's one page publishes.
const SITE_PAGE: &str = "# Room\n\n## Page /\n\nState:\n    Let sent be 0.\n\nActions:\n    To ping:\n        Publish \"hi\" on \"room\".\n\nView:\n    <button on-click={ping}>ping</button>\n";

#[test]
#[ignore = "builds a full-stack binary with `largo build`; run in the full suite"]
fn a_compiled_full_stack_app_whose_page_publishes_answers_the_relay() {
    let dir = tempfile::tempdir().unwrap();
    let made = cli::largo_in(dir.path(), &["new", "relaysite", "--web"]);
    assert!(
        made.status.success(),
        "largo new --web: {}",
        cli::stderr(&made)
    );
    let root = dir.path().join("relaysite");
    std::fs::write(root.join("src/main.lg"), SITE_BACKEND).unwrap();
    std::fs::write(root.join("pages/app.lg"), SITE_PAGE).unwrap();
    let built = cli::largo_in(&root, &["build"]);
    assert!(
        built.status.success(),
        "largo build:\n{}",
        cli::stderr(&built)
    );
    let artifact = root.join("target/debug/relaysite");
    assert!(
        artifact.exists(),
        "the build leaves its binary at {}",
        artifact.display()
    );
    let mut site = std::process::Command::new(&artifact);
    site.env("PORT", "0").current_dir(dir.path());
    let server = cli::http::Server::spawn(site);
    let port = server.port();
    assert_two_clients_relay(port);
    let front = raw_exchange(
        port,
        b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    );
    assert!(
        front.starts_with(b"HTTP/1.1 200"),
        "the page itself still serves:\n{}",
        String::from_utf8_lossy(&front)
    );
}

#[test]
fn an_interpreted_backend_whose_page_publishes_answers_the_relay() {
    let port = free_port();
    std::env::set_var("LOGOS_PORT", port.to_string());
    std::thread::spawn(|| logicaffeine_compile::interpret_for_ui_sync(PUBLISHING_BACKEND));
    wait_for_port(port);
    assert_two_clients_relay(port);
}

#[test]
fn an_interpreted_backend_with_no_page_on_the_relay_answers_no_upgrade() {
    let port = free_port();
    std::env::set_var("LOGOS_PORT", port.to_string());
    std::thread::spawn(|| logicaffeine_compile::interpret_for_ui_sync(QUIET_BACKEND));
    wait_for_port(port);
    let answer = raw_exchange(
        port,
        b"GET /_relay HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
    );
    let answer = String::from_utf8_lossy(&answer);
    assert!(
        !answer.contains("101 Switching Protocols"),
        "no relay to switch to:\n{answer}"
    );
    assert!(
        answer.contains("backend"),
        "the request reaches the handler like any other:\n{answer}"
    );
}

#[test]
fn an_embedded_site_knows_whether_its_pages_use_the_relay() {
    for (page, uses) in [
        ("## Page /\nState:\n    Let n be 0.\nActions:\n    To ping:\n        Publish \"hi\" on \"room\".\nView:\n    <button on-click={ping}>ping</button>\n", true),
        ("## Page /\nState:\n    Let n be 0.\nView:\n    <p>{n}</p>\n", false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let pages = dir.path().join("pages");
        std::fs::create_dir_all(&pages).unwrap();
        std::fs::write(pages.join("app.lg"), page).unwrap();
        let module = logicaffeine_cli::assets_module(&pages, &dir.path().join("static")).expect("the site plans");
        assert!(module.contains(&format!("pub const USES_RELAY: bool = {uses};")), "{page}\n{module}");
    }
}
