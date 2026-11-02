//! A5 — the `largo serve` command.
//!
//! A scaffolded project whose entry ends in a `Serve` statement is served by
//! `largo serve`: the command interprets the entry, whose `Serve` binds the port
//! and dispatches requests to the LOGOS handler. We spawn the real binary and
//! dial it.

#[path = "cli_common/mod.rs"]
mod common;
use common::http::Server;
use common::scaffold;

#[test]
fn largo_serve_serves_the_project_over_http() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path(), "site");
    std::fs::write(
        dir.path().join("src/main.lg"),
        "# Main\n\n\
         ## To handle (request: Request) -> Response:\n    \
             Return plain(request's path).\n\n\
         ## Main\n\n\
         Serve on port 8080 with handle.\n",
    )
    .unwrap();

    let mut server = Server::start(dir.path());
    let resp = String::from_utf8_lossy(&server.get("/from-largo")).into_owned();
    assert!(
        resp.starts_with("HTTP/1.1 200 OK\r\n"),
        "status line:\n{resp}"
    );
    assert!(resp.ends_with("/from-largo"), "body:\n{resp}");
}
