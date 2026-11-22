//! ★ LOCK — httpclient.lg speaks HTTP/1.1 from LOGOS, over the socket natives, bounded. ★
//!
//! `httpRequest(method, url, headers, body, timeoutMs)` is written in LOGOS over `tcpConnect` /
//! `tcpConnectTls`, the way postgres.lg speaks its protocol. These locks run it against a local
//! server whose every answer is scripted, so each promise is checked byte for byte:
//!
//! * the request on the wire: request line, `Host` (port included when it is not the default),
//!   `Connection: close`, `Accept-Encoding: identity`, `Content-Length` for a body, the caller's headers;
//! * all three body framings — `Content-Length`, chunked (decoded, trailers ignored), close-delimited;
//! * a non-2xx status is an `Ok` carrying it; a repeated header's values are joined with `, `;
//! * no body for `HEAD` or `204` whatever the headers say;
//! * a body that ends short of its framing is an `Err`, never a truncated `Ok`;
//! * a server that never answers is an `Err` at the deadline;
//! * a URL with credentials, an unknown scheme, a header value carrying CR/LF and a header the client
//!   owns (`Host`, `Content-Length`, `Transfer-Encoding`, `Connection`) are refused before connecting;
//! * `https://` verifies the server's certificate (a private root through `LOGOS_TLS_EXTRA_ROOT`);
//! * the compiled binary answers exactly as the tree-walker does.
//!
//! YOU DO NOT GET TO WEAKEN THIS LOCK TO MAKE IT PASS.

#![allow(dead_code)]
mod common;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use logicaffeine_compile::interpret_for_ui;

/// What the server last received, raw, keyed by path — the request half of every lock.
fn received() -> &'static Mutex<std::collections::HashMap<String, String>> {
    static R: OnceLock<Mutex<std::collections::HashMap<String, String>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// The scripted answer for a path (the raw bytes the server writes), or `None` to never answer.
fn script(path: &str) -> Option<Vec<u8>> {
    let body = |status: &str, headers: &str, body: &str| -> Option<Vec<u8>> {
        Some(format!("HTTP/1.1 {status}\r\n{headers}\r\n{body}").into_bytes())
    };
    match path {
        "/length" => body("200 OK", "Content-Length: 11\r\nContent-Type: text/plain\r\n", "hello world"),
        "/chunked" => body(
            "200 OK",
            "Transfer-Encoding: chunked\r\n",
            "5\r\nhello\r\n1;ext=1\r\n \r\nB\r\nchunked wor\r\n2\r\nld\r\n0\r\nX-Trailer: t\r\n\r\n",
        ),
        "/close" => body("200 OK", "", "until the server closes"),
        "/missing" => body("404 Not Found", "Content-Length: 9\r\n", "not found"),
        "/repeat" => body("200 OK", "Set-Cookie: a=1\r\nSet-Cookie: b=2\r\nContent-Length: 0\r\n", ""),
        "/nocontent" => body("204 No Content", "Content-Length: 5\r\n", ""),
        "/short" => body("200 OK", "Content-Length: 10\r\n", "12345"),
        "/utf8" => body("200 OK", "Content-Length: 7\r\n", "héllo!"),
        "/silent" => None,
        "/echo" => Some(Vec::new()), // answered from the request itself, below
        _ => body("500 Internal Server Error", "Content-Length: 7\r\n", "unknown"),
    }
}

/// Read one whole request (headers, then `Content-Length` body bytes) from `s`.
fn read_request(s: &mut impl Read) -> String {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        if s.read(&mut byte).unwrap_or(0) == 0 {
            break;
        }
        buf.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&buf).to_string();
    let len = head
        .lines()
        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
        .unwrap_or(0);
    let mut body = vec![0u8; len];
    let _ = s.read_exact(&mut body);
    head + &String::from_utf8_lossy(&body)
}

fn serve_one(mut s: impl Read + Write) {
    let req = read_request(&mut s);
    let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
    received().lock().unwrap().insert(path.clone(), req.clone());
    let route = path.split('?').next().unwrap_or("/").to_string();
    match script(&route) {
        None => std::thread::sleep(Duration::from_secs(30)),
        Some(bytes) if route == "/echo" => {
            let _ = bytes;
            let reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}", req.len(), req);
            let _ = s.write_all(reply.as_bytes());
        }
        Some(bytes) => {
            let _ = s.write_all(&bytes);
        }
    }
    let _ = s.flush();
}

/// A plain HTTP server on a fresh port, one thread per connection.
fn http_server() -> u16 {
    static PORT: OnceLock<u16> = OnceLock::new();
    *PORT.get_or_init(|| {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                std::thread::spawn(move || serve_one(s));
            }
        });
        port
    })
}

/// A TLS server with a self-signed `localhost` certificate trusted through `LOGOS_TLS_EXTRA_ROOT`.
fn https_server() -> u16 {
    static PORT: OnceLock<u16> = OnceLock::new();
    *PORT.get_or_init(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).expect("cert");
        let pem = cert.serialize_pem().expect("pem");
        let der = cert.serialize_der().expect("der");
        let key = cert.serialize_private_key_der();
        let cfg = Arc::new(
            rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![rustls::pki_types::CertificateDer::from(der)],
                    rustls::pki_types::PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(key)),
                )
                .expect("server config"),
        );
        let dir = Box::leak(Box::new(tempfile::tempdir().expect("dir")));
        let ca = dir.path().join("ca.pem");
        std::fs::write(&ca, pem).expect("write ca");
        std::env::set_var("LOGOS_TLS_EXTRA_ROOT", &ca);
        let l = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let cfg = cfg.clone();
                std::thread::spawn(move || {
                    let conn = rustls::ServerConnection::new(cfg).expect("tls conn");
                    serve_one(rustls::StreamOwned::new(conn, s));
                });
            }
        });
        port
    })
}

/// Run `program` on the tree-walker under a generous outer bound.
fn run(program: &str) -> (Option<String>, String) {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("rt");
    let out = rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(60), interpret_for_ui(program))
            .await
            .expect("the program must not hang")
    });
    (out.error, out.lines.join("\n"))
}

/// A program that requests `url` and shows the status, the named headers and the body (bracketed),
/// or the refusal.
fn shows(method: &str, url: &str, headers: &[(&str, &str)], body: &str, timeout_ms: i64, show_headers: &[&str]) -> String {
    let mut src = String::from("## Main\nLet mutable h be a new Map of Text to Text.\n");
    for (k, v) in headers {
        src.push_str(&format!("Set item \"{k}\" of h to \"{v}\".\n"));
    }
    src.push_str(&format!(
        "Inspect httpRequest(\"{method}\", \"{url}\", h, \"{body}\", {timeout_ms}):\n\
         \x20   When Ok (r):\n\
         \x20       Show r's status.\n"
    ));
    for name in show_headers {
        src.push_str(&format!("        Show \"{name}: \" + item \"{name}\" of (r's headers).\n"));
    }
    src.push_str(
        "        Show \"[\" + r's body + \"]\".\n\
         \x20   When Err (e):\n\
         \x20       Show \"refused: \" + e.\n",
    );
    src
}

fn assert_shows(program: &str, expected: &str) {
    let (error, output) = run(program);
    assert_eq!(error, None, "the program runs:\n{program}");
    assert_eq!(output.trim(), expected, "\n{program}");
}

// ── body framings ─────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_content_length_body_is_read_exactly() {
    let p = http_server();
    assert_shows(
        &shows("GET", &format!("http://127.0.0.1:{p}/length"), &[], "", 10_000, &["content-type"]),
        "200\ncontent-type: text/plain\n[hello world]",
    );
}

#[test]
fn a_chunked_body_is_decoded_and_its_trailers_ignored() {
    let p = http_server();
    assert_shows(&shows("GET", &format!("http://127.0.0.1:{p}/chunked"), &[], "", 10_000, &[]), "200\n[hello chunked world]");
}

#[test]
fn a_close_delimited_body_runs_to_the_close() {
    let p = http_server();
    assert_shows(&shows("GET", &format!("http://127.0.0.1:{p}/close"), &[], "", 10_000, &[]), "200\n[until the server closes]");
}

#[test]
fn a_utf8_body_is_text() {
    let p = http_server();
    assert_shows(&shows("GET", &format!("http://127.0.0.1:{p}/utf8"), &[], "", 10_000, &[]), "200\n[héllo!]");
}

// ── statuses and headers ──────────────────────────────────────────────────────────────────────────

#[test]
fn a_non_2xx_status_is_an_ok_carrying_it() {
    let p = http_server();
    assert_shows(&shows("GET", &format!("http://127.0.0.1:{p}/missing"), &[], "", 10_000, &[]), "404\n[not found]");
}

#[test]
fn a_repeated_header_is_joined() {
    let p = http_server();
    assert_shows(
        &shows("GET", &format!("http://127.0.0.1:{p}/repeat"), &[], "", 10_000, &["set-cookie"]),
        "200\nset-cookie: a=1, b=2\n[]",
    );
}

#[test]
fn head_and_204_carry_no_body() {
    let p = http_server();
    assert_shows(&shows("GET", &format!("http://127.0.0.1:{p}/nocontent"), &[], "", 10_000, &[]), "204\n[]");
    assert_shows(&shows("HEAD", &format!("http://127.0.0.1:{p}/length"), &[], "", 10_000, &[]), "200\n[]");
}

// ── the request on the wire ───────────────────────────────────────────────────────────────────────

#[test]
fn the_request_carries_its_line_host_framing_and_the_callers_headers() {
    let p = http_server();
    let (error, _) = run(&shows(
        "POST",
        &format!("http://127.0.0.1:{p}/echo?q=1"),
        &[("Authorization", "Bearer sk-test"), ("X-Title", "MaCSad")],
        "{{\\\"a\\\": 1}}",
        10_000,
        &[],
    ));
    assert_eq!(error, None);
    let req = received().lock().unwrap().get("/echo?q=1").cloned().expect("the server saw the request");
    let lines: Vec<&str> = req.split("\r\n").collect();
    assert_eq!(lines[0], "POST /echo?q=1 HTTP/1.1", "{req}");
    for want in [
        format!("Host: 127.0.0.1:{p}"),
        "Connection: close".to_string(),
        "Accept-Encoding: identity".to_string(),
        "Content-Length: 8".to_string(),
        "Authorization: Bearer sk-test".to_string(),
        "X-Title: MaCSad".to_string(),
    ] {
        assert!(lines.contains(&want.as_str()), "missing `{want}` in:\n{req}");
    }
    assert!(req.ends_with("\r\n\r\n{\"a\": 1}"), "the body follows the blank line:\n{req}");
}

#[test]
fn post_json_adds_the_content_type() {
    let p = http_server();
    let src = format!(
        "## Main\nLet h be a new Map of Text to Text.\n\
         Inspect httpPostJson(\"http://127.0.0.1:{p}/echo?json\", h, \"[1]\", 10000):\n\
         \x20   When Ok (r):\n\
         \x20       Show r's status.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n"
    );
    assert_shows(&src, "200");
    let req = received().lock().unwrap().get("/echo?json").cloned().expect("seen");
    assert!(req.contains("\r\nContent-Type: application/json\r\n"), "{req}");
}

// ── failures ──────────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_body_that_ends_short_of_its_length_is_an_err() {
    let p = http_server();
    assert_shows(
        &shows("GET", &format!("http://127.0.0.1:{p}/short"), &[], "", 10_000, &[]),
        "refused: httpRequest: the connection closed after 5 of 10 body bytes",
    );
}

#[test]
fn a_server_that_never_answers_is_an_err_at_the_deadline() {
    let p = http_server();
    let started = std::time::Instant::now();
    let (error, output) = run(&shows("GET", &format!("http://127.0.0.1:{p}/silent"), &[], "", 700, &[]));
    assert_eq!(error, None);
    assert!(output.trim().starts_with("refused: httpRequest:"), "{output}");
    assert!(started.elapsed() < Duration::from_secs(20), "returned near the deadline, took {:?}", started.elapsed());
}

#[test]
fn malformed_requests_are_refused_before_connecting() {
    for (url, headers, expected) in [
        ("ftp://example.com/x", vec![], "refused: httpRequest: the url must start with http:// or https://, got ftp://example.com/x"),
        ("https://user:pw@example.com/", vec![], "refused: httpRequest: credentials in the url are refused; send them in a header"),
        ("http://example.com:99999/", vec![], "refused: httpRequest: the port must be between 1 and 65535, got 99999"),
        ("http://example.com/", vec![("Host", "evil")], "refused: httpRequest: the client owns the Host header"),
        ("http://example.com/", vec![("Content-Length", "3")], "refused: httpRequest: the client owns the Content-Length header"),
        ("http://example.com/", vec![("X-A", "a\\r\\nX-Injected: 1")], "refused: httpRequest: header X-A holds a forbidden control byte"),
    ] {
        assert_shows(&shows("GET", url, &headers, "", 5_000, &[]), expected);
    }
}

// ── TLS ───────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn https_verifies_the_certificate_and_speaks_the_same_protocol() {
    let p = https_server();
    assert_shows(&shows("GET", &format!("https://localhost:{p}/chunked"), &[], "", 10_000, &[]), "200\n[hello chunked world]");
}

// ── the compiled tier ─────────────────────────────────────────────────────────────────────────────

#[test]
fn the_compiled_binary_answers_as_the_tree_walker_does() {
    let p = http_server();
    let src = format!(
        "## To report (r: Result of HttpReply and Text):\n\
         \x20   Inspect r:\n\
         \x20       When Ok (x):\n\
         \x20           Show \"{{x's status}} [{{x's body}}]\".\n\
         \x20       When Err (e):\n\
         \x20           Show \"refused: \" + e.\n\
         \n\
         ## Main\n\
         Let h be a new Map of Text to Text.\n\
         report(httpGet(\"http://127.0.0.1:{p}/length\", h, 10000)).\n\
         report(httpGet(\"http://127.0.0.1:{p}/chunked\", h, 10000)).\n\
         report(httpGet(\"http://127.0.0.1:{p}/close\", h, 10000)).\n\
         report(httpGet(\"http://127.0.0.1:{p}/missing\", h, 10000)).\n\
         report(httpGet(\"http://127.0.0.1:{p}/short\", h, 10000)).\n\
         report(httpGet(\"ftp://nowhere/\", h, 10000)).\n"
    );
    let expected = "200 [hello world]\n200 [hello chunked world]\n200 [until the server closes]\n404 [not found]\n\
                    refused: httpRequest: the connection closed after 5 of 10 body bytes\n\
                    refused: httpRequest: the url must start with http:// or https://, got ftp://nowhere/";
    assert_shows(&src, expected);
    let compiled = common::run_logos(&src);
    assert!(compiled.success, "the compiled program runs:\n{}", compiled.stderr);
    assert_eq!(compiled.stdout.trim(), expected, "the compiled binary agrees with the tree-walker");
}
