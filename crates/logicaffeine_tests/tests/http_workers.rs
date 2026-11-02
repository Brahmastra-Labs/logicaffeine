//! A4a — CONCURRENT HTTP workers on the interpreter tier. The `Serve` loop must
//! overlap connections (a stalled client can never head-of-line-block the
//! server) and survive handler failures (a runtime error in one request is a
//! 500 for THAT request, never death of the server).
//!
//! Before this lock the interpreter served strictly sequentially (accept → read
//! → handle → write → repeat) and a handler runtime error KILLED the accept
//! loop — one bad request took the whole site down (found during G8).
//!
//!  ⚠️  YOU DO NOT GET TO WEAKEN THIS LOCK TO MAKE IT PASS.  ⚠️

use std::time::Duration;

use logicaffeine_compile::interpret_for_ui;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Start `program` (a Serve program) on the interpreter as a background task;
/// returns the abort handle. The server never returns — the guard kills it.
struct ServerTask(tokio::task::JoinHandle<()>);
impl Drop for ServerTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn spawn_server(program: String) -> ServerTask {
    ServerTask(tokio::task::spawn_local(async move {
        let _ = interpret_for_ui(&program).await;
    }))
}

/// Connect with retry until the server is up.
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

/// One-shot GET. This client frames the response by reading to EOF, so it must
/// opt out of HTTP/1.1 persistence with `Connection: close` — a keep-alive
/// server (correctly) holds the socket open otherwise and `read_to_end` would
/// wait forever.
async fn get(port: u16, path: &str) -> String {
    let mut s = connect(port).await;
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes())
        .await
        .expect("write");
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.expect("read");
    String::from_utf8_lossy(&buf).into_owned()
}

fn serve_program(port: u16) -> String {
    format!(
        "## To handle (request: Request) -> Response:\n\
         \x20   If request's path is equal to \"/boom\":\n\
         \x20       Let xs be [1].\n\
         \x20       Let x be item 99 of xs.\n\
         \x20       Return plain(\"unreachable\").\n\
         \x20   Return plain(request's path).\n\
         \n\
         ## Main\n\
         Serve on port {port} with handle.\n"
    )
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// A STALLED connection (headers promised, body never sent) must not block a
/// later, complete request — connections overlap.
#[tokio::test]
async fn a_stalled_connection_does_not_block_the_next_request() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let port = free_port();
            let _server = spawn_server(serve_program(port));

            // Connection A: promise a body, never deliver it — then HOLD the socket.
            let mut stalled = connect(port).await;
            stalled
                .write_all(b"POST /slow HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\n")
                .await
                .expect("write stalled head");

            // Connection B: a complete request, issued while A is stalled.
            let resp = tokio::time::timeout(Duration::from_secs(5), get(port, "/fast"))
                .await
                .expect("request B must complete while A is stalled");
            assert!(
                resp.contains("200") && resp.ends_with("/fast"),
                "B did not get its own response: {resp:?}"
            );
            drop(stalled);
        })
        .await;
}

/// A handler RUNTIME ERROR is a 500 for that request — and the server keeps
/// serving afterwards. One bad request must never take the site down.
#[tokio::test]
async fn a_handler_error_is_a_500_and_the_server_survives() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let port = free_port();
            let _server = spawn_server(serve_program(port));

            let boom = tokio::time::timeout(Duration::from_secs(5), get(port, "/boom"))
                .await
                .expect("the failing request still gets a response");
            assert!(
                boom.contains("500"),
                "a handler runtime error must be a 500 response, got: {boom:?}"
            );

            let after = tokio::time::timeout(Duration::from_secs(5), get(port, "/alive"))
                .await
                .expect("the server must keep serving after a handler error");
            assert!(
                after.contains("200") && after.ends_with("/alive"),
                "server died after a handler error: {after:?}"
            );
        })
        .await;
}

/// Several interleaved in-flight requests all complete with THEIR OWN bodies —
/// no response ever crosses connections.
#[tokio::test]
async fn interleaved_requests_get_their_own_responses() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let port = free_port();
            let _server = spawn_server(serve_program(port));
            // Warm up (server up).
            let _ = get(port, "/warm").await;

            let mut handles = Vec::new();
            for i in 0..8 {
                handles.push(tokio::task::spawn_local(async move {
                    let path = format!("/req{i}");
                    let resp = get(port, &path).await;
                    (path, resp)
                }));
            }
            for h in handles {
                let (path, resp) = h.await.expect("request task");
                assert!(
                    resp.contains("200") && resp.ends_with(&path),
                    "response crossed connections for {path}: {resp:?}"
                );
            }
        })
        .await;
}
