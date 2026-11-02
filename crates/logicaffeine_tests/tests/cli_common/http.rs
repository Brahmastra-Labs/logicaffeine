//! Dialling a `largo serve` the tests spawned, and the request helpers the CLI suites share. The server
//! itself — spawned on a port the system chooses, dialed where it announces — is `support/served.rs`.

#[path = "../support/served.rs"]
mod served;
pub use served::{announced_port, Server};

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;

impl Server {
    /// `largo serve` in `dir` on a port the system chooses — `--port 0`, and `LOGOS_PORT=0` for a backend
    /// whose own `Serve` binds — dialed where it says it listens.
    pub fn start(dir: &Path) -> Server {
        let mut cmd = super::largo();
        cmd.args(["serve", "--port", "0"])
            .current_dir(dir)
            .env("LOGOS_PORT", "0");
        Server::spawn(cmd)
    }
}

/// The response's status code.
pub fn status(raw: &[u8]) -> u16 {
    String::from_utf8_lossy(raw)
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// The response body, after the header terminator.
pub fn body(raw: &[u8]) -> String {
    let at = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| i + 4)
        .unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[at..]).into_owned()
}

/// The value of a response header, matched case-insensitively.
pub fn header(raw: &[u8], name: &str) -> Option<String> {
    let text = String::from_utf8_lossy(raw);
    let head = text.split("\r\n\r\n").next().unwrap_or("");
    head.lines().skip(1).find_map(|line| {
        let (k, v) = line.split_once(':')?;
        k.trim()
            .eq_ignore_ascii_case(name)
            .then(|| v.trim().to_string())
    })
}

/// One request — `method` on `path` with `headers` and `body` — answered in full, once the server
/// is already up. The request opts out of keep-alive so reading to EOF frames the response.
pub fn request(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Vec<u8> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("the server is listening");
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes())
        .expect("write the request head");
    s.write_all(body.as_bytes())
        .expect("write the request body");
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).expect("read the response");
    raw
}
