//! A3 — the `Serve` keystone on the interpreter tier.
//!
//! `Serve on port <p> with <handler>` binds an HTTP server and, per request,
//! re-enters the LOGOS handler function with a `Request` value and writes back
//! its `Response`. This proves the host→LOGOS callback: the tokio accept loop
//! calls `call_function_with_values` on the interpreter (single-threaded, `!Send`)
//! inline, with no task spawn.
//!
//! Each test runs the interpreter on a background thread (the server blocks
//! forever) and dials it with a plain std client.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

/// Grab an OS-assigned free port, then release it for the LOGOS program to bind.
fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    l.local_addr().expect("addr").port()
}

/// Run a LOGOS program (that ends in a `Serve`) on a background thread.
fn spawn_server(src: String) {
    std::thread::spawn(move || {
        let _ = futures::executor::block_on(logicaffeine_compile::interpret_for_ui(&src));
    });
}

/// Send a raw request, retrying the connect until the server is up, and read
/// the whole response. Every raw request here carries `Connection: close`
/// (see [`get`]/[`post`]): this client frames responses by reading to EOF, so
/// it must opt out of the server's HTTP/1.1 keep-alive.
fn request(port: u16, raw: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(mut s) => {
                s.write_all(raw.as_bytes()).expect("write request");
                let mut buf = String::new();
                s.read_to_string(&mut buf).expect("read response");
                return buf;
            }
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => panic!("server never came up on port {port}: {e}"),
        }
    }
}

/// Like [`request`] but returns the RAW response bytes (for binary bodies).
fn request_raw(port: u16, raw: &str) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(mut s) => {
                s.write_all(raw.as_bytes()).expect("write");
                let mut buf = Vec::new();
                s.read_to_end(&mut buf).expect("read");
                return buf;
            }
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => panic!("server never came up on port {port}: {e}"),
        }
    }
}

fn get(port: u16, path: &str) -> String {
    request(
        port,
        &format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"),
    )
}

fn post(port: u16, path: &str, body: &str) -> String {
    request(
        port,
        &format!(
            "POST {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ),
    )
}

#[test]
fn interpreter_serves_a_request_via_a_logos_handler() {
    let port = free_port();
    spawn_server(format!(
        "## To handle (request: Request) -> Response:\n    \
             Return plain(request's path).\n\n\
         ## Main\n\
         Serve on port {port} with handle.\n"
    ));
    let resp = get(port, "/hello");
    assert!(
        resp.starts_with("HTTP/1.1 200 OK\r\n"),
        "status line:\n{resp}"
    );
    assert!(
        resp.contains("Content-Type: text/plain\r\n"),
        "headers:\n{resp}"
    );
    assert!(resp.ends_with("/hello"), "body:\n{resp}");
}

#[test]
fn interpreter_echoes_a_post_body_through_the_handler() {
    let port = free_port();
    spawn_server(format!(
        "## To handle (request: Request) -> Response:\n    \
             Return plain(request's body).\n\n\
         ## Main\n\
         Serve on port {port} with handle.\n"
    ));
    let resp = post(port, "/submit", "hello-body");
    assert!(resp.ends_with("hello-body"), "body:\n{resp}");
}

#[test]
fn interpreter_serves_raw_binary_bytes() {
    // A `bytesResponse` sends a raw byte body (here the wasm magic `\0asm`) verbatim
    // — the backend can serve a `.wasm`/image, not just text.
    let port = free_port();
    spawn_server(format!(
        "## To handle (request: Request) -> Response:\n    \
             Return bytesResponse(\"application/wasm\", [0, 97, 115, 109]).\n\n\
         ## Main\n\
         Serve on port {port} with handle.\n"
    ));
    let resp = request_raw(
        port,
        "GET /app.wasm HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    );
    let head = String::from_utf8_lossy(&resp);
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "status:\n{head}");
    assert!(
        head.contains("Content-Type: application/wasm\r\n"),
        "headers:\n{head}"
    );
    assert!(head.contains("Content-Length: 4\r\n"), "length:\n{head}");
    let sep = resp
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("header/body separator");
    assert_eq!(&resp[sep + 4..], &[0u8, 97, 115, 109], "raw body bytes");
}

#[test]
fn interpreter_serves_a_file_as_wasm_bytes() {
    // The single-binary deployment path: `readBytes` a file off disk and serve it
    // verbatim — this is how `largo serve` hands the browser the `app.wasm`.
    let contents: [u8; 8] = [0, 97, 115, 109, 1, 0, 0, 0]; // wasm magic + version
    let path = std::env::temp_dir().join(format!("logos_serve_app_{}.wasm", std::process::id()));
    std::fs::write(&path, contents).unwrap();
    let port = free_port();
    spawn_server(format!(
        "## To handle (request: Request) -> Response:\n    \
             Return bytesResponse(\"application/wasm\", readBytes(\"{}\")).\n\n\
         ## Main\n\
         Serve on port {port} with handle.\n",
        path.display()
    ));
    let resp = request_raw(
        port,
        "GET /app.wasm HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    );
    let head = String::from_utf8_lossy(&resp);
    assert!(
        head.contains("Content-Type: application/wasm\r\n"),
        "headers:\n{head}"
    );
    let sep = resp
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("separator");
    assert_eq!(
        &resp[sep + 4..],
        &contents,
        "served file bytes must be verbatim"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn interpreter_routes_on_path_and_returns_404() {
    let port = free_port();
    spawn_server(format!(
        "## To handle (request: Request) -> Response:\n    \
             If request's path is equal to \"/\":\n        \
                 Return html(\"<h1>home</h1>\").\n    \
             Return notFound().\n\n\
         ## Main\n\
         Serve on port {port} with handle.\n"
    ));
    let home = get(port, "/");
    assert!(home.starts_with("HTTP/1.1 200 OK\r\n"), "home:\n{home}");
    assert!(
        home.contains("Content-Type: text/html\r\n"),
        "home headers:\n{home}"
    );
    assert!(home.ends_with("<h1>home</h1>"), "home body:\n{home}");

    let missing = get(port, "/nope");
    assert!(
        missing.starts_with("HTTP/1.1 404 Not Found\r\n"),
        "missing:\n{missing}"
    );
}
