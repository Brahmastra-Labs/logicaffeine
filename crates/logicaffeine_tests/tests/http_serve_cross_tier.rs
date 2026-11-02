//! ════════════════════════════════════════════════════════════════════════════════════════════
//! A4b — CROSS-TIER HTTP SERVE LOCK: the `Serve` HTTP server runs byte-identically on the
//! BYTECODE VM and the TREE-WALKER. Both tiers stand up the SAME `Serve` program bound to the
//! SAME free port (one at a time), are fed the SAME scripted requests over a loopback socket,
//! and MUST return byte-identical response bytes — AND identical handler stdout ordering.
//!
//! The interpreter path is `interpret_for_ui` (A4a: `serve_http_concurrent` — the ORACLE).
//! The VM path is `run_vm_io_async` (A4b: `Op::Serve` → `VmBlock::Serve` → the async io driver
//! that re-enters the handler AS VM BYTECODE per connection). The handler executes as genuine
//! bytecode on the VM tier — never a silent tree-walker substitution.
//!
//!  ⚠️  YOU DO NOT GET TO WEAKEN THIS LOCK TO MAKE IT PASS.  ⚠️
//!  A divergence here means HTTP serving drifted between tiers — fix the TIER (`run_vm_io_async`
//!  / `Op::Serve` / the shared request/response marshaling), never relax the `assert_eq!`.
//!  Strictly monotone: add shapes, never remove them.
//! ════════════════════════════════════════════════════════════════════════════════════════════

use std::time::Duration;

use logicaffeine_compile::{interpret_for_ui, run_vm_io_async};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// A server task aborts on drop so the never-returning accept loop never leaks
/// across tests.
struct ServerTask(tokio::task::JoinHandle<()>);
impl Drop for ServerTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Start `program` on ONE tier as a background local task. The VM tier runs on
/// `run_vm_io_async`; the tree-walker on `interpret_for_ui`. Neither returns —
/// the guard kills it at the end of the test.
fn spawn_server(program: String, use_vm: bool) -> ServerTask {
    ServerTask(tokio::task::spawn_local(async move {
        if use_vm {
            let _ = run_vm_io_async(&program).await;
        } else {
            let _ = interpret_for_ui(&program).await;
        }
    }))
}

/// Connect with retry until the server binds.
async fn connect(port: u16) -> TcpStream {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        match TcpStream::connect(("127.0.0.1", port)).await {
            Ok(s) => return s,
            Err(e) => {
                if std::time::Instant::now() > deadline {
                    panic!("server never came up on {port}: {e}");
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }
    }
}

/// Send one raw request over a fresh `Connection: close` socket and read the
/// whole framed response to EOF. `Connection: close` opts out of keep-alive so
/// `read_to_end` frames the response for us.
async fn round_trip(port: u16, raw: &str) -> Vec<u8> {
    let mut s = connect(port).await;
    s.write_all(raw.as_bytes()).await.expect("write");
    let mut buf = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut buf))
        .await
        .expect("response must not hang")
        .expect("read");
    buf
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// A router with a GET route, a POST echo, a `Show`-ing route (stdout ordering),
/// and a fall-through 404. The `Serving on …` banner line and per-request
/// `Show` output are the observable stdout both tiers must agree on.
fn serve_program(port: u16) -> String {
    format!(
        "## To handle (request: Request) -> Response:\n\
         \x20   If request's path is equal to \"/hello\":\n\
         \x20       Show \"hit /hello\".\n\
         \x20       Return plain(\"hello, world\").\n\
         \x20   If request's method is equal to \"POST\":\n\
         \x20       Return plain(request's body).\n\
         \x20   If request's path is equal to \"/missing\":\n\
         \x20       Return notFound().\n\
         \x20   Return plain(request's path).\n\
         \n\
         ## Main\n\
         Serve on port {port} with handle.\n"
    )
}

/// The scripted requests, byte-for-byte, applied to BOTH tiers in order.
fn scripted_requests() -> Vec<String> {
    vec![
        // A GET route that also `Show`s.
        "GET /hello HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n".to_string(),
        // A POST echo of the body.
        "POST /echo HTTP/1.1\r\nHost: x\r\nContent-Length: 11\r\nConnection: close\r\n\r\nhello there".to_string(),
        // An explicit 404.
        "GET /missing HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n".to_string(),
        // A fall-through echo of the path.
        "GET /anything HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n".to_string(),
        // A body larger than the server accepts, DECLARED rather than sent — the
        // refusal both tiers owe a client that announces too much.
        "POST /big HTTP/1.1\r\nHost: x\r\nContent-Length: 99999999\r\nConnection: close\r\n\r\n"
            .to_string(),
        // Bytes that are not a request at all.
        "NOT-A-REQUEST\r\n\r\n".to_string(),
    ]
}

/// Drive one tier through the whole script, returning each response's bytes.
async fn responses_on_tier(use_vm: bool) -> Vec<Vec<u8>> {
    let port = free_port();
    let _server = spawn_server(serve_program(port), use_vm);
    let mut out = Vec::new();
    for req in scripted_requests() {
        out.push(round_trip(port, &req).await);
    }
    out
}

/// The wire response bytes must be byte-identical across the VM and tree-walker
/// tiers for every scripted request.
#[tokio::test]
async fn vm_serve_response_bytes_match_the_interpreter() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let interp = responses_on_tier(false).await;
            let vm = responses_on_tier(true).await;
            assert_eq!(
                interp.len(),
                vm.len(),
                "tier response counts differ: interp {} vm {}",
                interp.len(),
                vm.len()
            );
            for (i, (a, b)) in interp.iter().zip(vm.iter()).enumerate() {
                assert_eq!(
                    String::from_utf8_lossy(a),
                    String::from_utf8_lossy(b),
                    "response #{i} diverged between the interpreter and the VM tier"
                );
            }
        })
        .await;
}

/// The GET route returns its body; both tiers must serve `200` with the exact
/// same body regardless of which one hosts the accept loop.
#[tokio::test]
async fn vm_serve_get_route_is_correct_on_both_tiers() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            for use_vm in [false, true] {
                let port = free_port();
                let _server = spawn_server(serve_program(port), use_vm);
                let resp = round_trip(
                    port,
                    "GET /hello HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
                )
                .await;
                let text = String::from_utf8_lossy(&resp);
                assert!(
                    text.starts_with("HTTP/1.1 200"),
                    "{} tier did not answer 200: {text:?}",
                    if use_vm { "VM" } else { "interp" }
                );
                assert!(
                    text.ends_with("hello, world"),
                    "{} tier body wrong: {text:?}",
                    if use_vm { "VM" } else { "interp" }
                );
            }
        })
        .await;
}

/// A POST echoes its body; both tiers must round-trip the exact request body.
#[tokio::test]
async fn vm_serve_post_echo_is_correct_on_both_tiers() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            for use_vm in [false, true] {
                let port = free_port();
                let _server = spawn_server(serve_program(port), use_vm);
                let resp = round_trip(
                    port,
                    "POST /echo HTTP/1.1\r\nHost: x\r\nContent-Length: 11\r\nConnection: close\r\n\r\nhello there",
                )
                .await;
                let text = String::from_utf8_lossy(&resp);
                assert!(
                    text.starts_with("HTTP/1.1 200") && text.ends_with("hello there"),
                    "{} tier POST echo wrong: {text:?}",
                    if use_vm { "VM" } else { "interp" }
                );
            }
        })
        .await;
}

/// The handler's `Show` side effects and its `Response` must run IDENTICALLY as
/// bytecode and as tree-walked code — proving the VM tier genuinely executes the
/// handler body (Show ordering included), never a silent substitution. The
/// handler is exercised through a direct top-level call (not the accept loop, so
/// the program completes and its output surfaces) under the SAME `LOGOS_SEED`.
#[tokio::test]
async fn vm_serve_handler_show_ordering_matches_across_tiers() {
    std::env::set_var("LOGOS_SEED", "7");
    let program = "## To handle (request: Request) -> Response:\n\
         \x20   Show \"handling \".\n\
         \x20   Show request's path.\n\
         \x20   Return plain(request's path).\n\
         \n\
         ## Main\n\
         Let h be a new Map of Text to Text.\n\
         Let q be a new Map of Text to Text.\n\
         Let req be a new Request with method \"GET\" and path \"/p1\" and body \"\" and headers h and query q.\n\
         Let r1 be handle(req).\n\
         Show r1's body.\n\
         Let req2 be a new Request with method \"GET\" and path \"/p2\" and body \"\" and headers h and query q.\n\
         Let r2 be handle(req2).\n\
         Show r2's body.\n";
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let interp = interpret_for_ui(program).await;
            let vm = run_vm_io_async(program).await;
            assert_eq!(interp.error, None, "interp errored: {:?}", interp.error);
            assert_eq!(vm.error, None, "VM errored: {:?}", vm.error);
            assert_eq!(
                interp.lines, vm.lines,
                "handler Show ordering diverged between the interpreter and the VM tier"
            );
            assert!(
                !interp.lines.is_empty(),
                "the handler must actually Show — an empty output means the body never ran"
            );
        })
        .await;
    std::env::remove_var("LOGOS_SEED");
}

/// An explicit 404 must be a 404 on both tiers.
#[tokio::test]
async fn vm_serve_404_is_correct_on_both_tiers() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            for use_vm in [false, true] {
                let port = free_port();
                let _server = spawn_server(serve_program(port), use_vm);
                let resp = round_trip(
                    port,
                    "GET /missing HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
                )
                .await;
                let text = String::from_utf8_lossy(&resp);
                assert!(
                    text.starts_with("HTTP/1.1 404"),
                    "{} tier did not answer 404: {text:?}",
                    if use_vm { "VM" } else { "interp" }
                );
            }
        })
        .await;
}

/// A client that announces more body than the server accepts gets `413 Payload Too
/// Large`, and one that sends nonsense gets `400 Bad Request` — on BOTH tiers.
///
/// These are different answers on purpose: "too big" is a client that can retry
/// smaller, "malformed" is a client that cannot. The tree-walker applied the shared
/// `read_error_response` law and answered 413; the VM's accept loop answered a flat
/// 400 for every read error, so the tiers disagreed about a request no test sent.
///
/// YOU DO NOT GET TO WEAKEN THIS. Fix the tier, never the assert.
#[tokio::test]
async fn an_oversized_body_is_413_and_nonsense_is_400_on_both_tiers() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            for use_vm in [false, true] {
                let tier = if use_vm { "the VM" } else { "the tree-walker" };
                let port = free_port();
                let _server = spawn_server(serve_program(port), use_vm);

                let big = round_trip(
                    port,
                    "POST /big HTTP/1.1\r\nHost: x\r\nContent-Length: 99999999\r\nConnection: close\r\n\r\n",
                )
                .await;
                let big = String::from_utf8_lossy(&big).to_string();
                assert!(
                    big.starts_with("HTTP/1.1 413 "),
                    "{tier} must answer 413 to a declared body over the limit, got: {big:?}"
                );

                let junk = round_trip(port, "NOT-A-REQUEST\r\n\r\n").await;
                let junk = String::from_utf8_lossy(&junk).to_string();
                assert!(
                    junk.starts_with("HTTP/1.1 400 "),
                    "{tier} must answer 400 to bytes that are not a request, got: {junk:?}"
                );
            }
        })
        .await;
}
