//! The mission check behind "serve websites fully in LOGOS": a `Serve` HANDLER
//! must be able to do NESTED IO mid-request — the shape of "on GET /api/todos,
//! query the database and return the rows". The dev tier (`largo serve` =
//! interpreter) and deploy tier (`largo build` = AOT) are what actually serve;
//! the VM tier's straight-line-only Serve (A4b) is fine because nobody deploys
//! on the VM. This lock proves the INTERPRETER serve path runs a handler that
//! opens a TCP connection, talks to a peer, and returns the reply — the exact
//! capability a Postgres-backed route needs.
//!
//!  ⚠️  If this goes RED, the website story is broken at its core — fix the
//!  interpreter serve path, never relax the assert.

use std::time::Duration;

use logicaffeine_compile::interpret_for_ui;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

struct ServerTask(tokio::task::JoinHandle<()>);
impl Drop for ServerTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

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

async fn get(port: u16, path: &str) -> String {
    let mut s = connect(port).await;
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes())
        .await
        .expect("write");
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.expect("read");
    String::from_utf8_lossy(&buf).into_owned()
}

/// A handler that, per request, opens a TCP connection to an upstream, sends the
/// request path, and returns the upstream's reply — nested IO INSIDE the handler,
/// on the interpreter serve path.
#[tokio::test]
async fn interpreter_serve_handler_does_nested_tcp_io() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            // An upstream that replies "upstream:<whatever it received>".
            let upstream = TcpListener::bind("127.0.0.1:0").await.expect("upstream binds");
            let up_port = upstream.local_addr().unwrap().port();
            tokio::task::spawn_local(async move {
                while let Ok((mut s, _)) = upstream.accept().await {
                    tokio::task::spawn_local(async move {
                        let mut buf = [0u8; 64];
                        if let Ok(n) = s.read(&mut buf).await {
                            let mut reply = b"upstream:".to_vec();
                            reply.extend_from_slice(&buf[..n]);
                            let _ = s.write_all(&reply).await;
                        }
                    });
                }
            });

            let serve_port = std::net::TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
                .port();

            // The handler: connect upstream, send 5 bytes ("hello"), read the
            // reply (up to 32 bytes), return it as the HTTP body. All nested IO.
            let program = format!(
                "## To handle (request: Request) -> Response:\n\
                 \x20   Let c be tcpConnect(\"127.0.0.1\", {up_port}).\n\
                 \x20   Inspect c:\n\
                 \x20       When Ok (conn):\n\
                 \x20           Let sent be tcpSend(conn, text_bytes(\"hello\")).\n\
                 \x20           Let got be tcpRecvExact(conn, 14).\n\
                 \x20           Inspect got:\n\
                 \x20               When Ok (data):\n\
                 \x20                   Let closed be tcpClose(conn).\n\
                 \x20                   Return plain(text_from_bytes(data)).\n\
                 \x20               When Err (e):\n\
                 \x20                   Return plain(e).\n\
                 \x20       When Err (e):\n\
                 \x20           Return plain(e).\n\
                 \n\
                 ## Main\n\
                 Serve on port {serve_port} with handle.\n"
            );
            let _server = ServerTask(tokio::task::spawn_local(async move {
                let _ = interpret_for_ui(&program).await;
            }));

            let resp = tokio::time::timeout(Duration::from_secs(8), get(serve_port, "/api"))
                .await
                .expect("the handler's nested-IO request completes");
            assert!(
                resp.contains("200") && resp.contains("upstream:hello"),
                "handler must complete its nested TCP round-trip and return the upstream reply; got: {resp:?}"
            );
        })
        .await;
}
