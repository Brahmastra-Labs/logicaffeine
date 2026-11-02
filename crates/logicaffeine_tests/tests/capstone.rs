//! Capstone — the hackathon website's LOGOS backend.
//!
//! `site.lg` serves the HTML shell at `/` and the schedule at `/api/schedule`
//! (the SAME string the LOGOS→WASM frontend fetches and renders — see the compile
//! crate's `wasm_dom.rs::capstone_frontend_builds_page_and_renders_fetched_schedule`).
//! Together the two tests prove the full loop: a LOGOS backend serves the data a
//! LOGOS frontend fetches and renders — a website that runs on LOGOS, end to end.

#[path = "cli_common/mod.rs"]
mod common;
use common::http::Server;
use common::scaffold;

/// The contract shared with the frontend test.
const SCHEDULE: &str = "9am Keynote / 10am Lunch / 2pm Hack";

fn get(server: &mut Server, path: &str) -> String {
    String::from_utf8_lossy(&server.get(path)).into_owned()
}

#[test]
fn capstone_backend_serves_the_html_shell_and_schedule_api() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path(), "site");
    let site = format!(
        "# Site\n\n\
         ## To handle (request: Request) -> Response:\n    \
             If request's path is equal to \"/api/schedule\":\n        \
                 Return plain(\"{SCHEDULE}\").\n    \
             If request's path is equal to \"/\":\n        \
                 Return html(\"<h1>LOGOS Hackathon</h1><div id='schedule'></div><script type='module' src='/app.js'></script>\").\n    \
             Return notFound().\n\n\
         ## Main\n\
         Serve on port 8080 with handle.\n"
    );
    std::fs::write(dir.path().join("src/main.lg"), site).unwrap();

    let mut server = Server::start(dir.path());

    // The schedule API returns the exact string the frontend renders.
    let sched = get(&mut server, "/api/schedule");
    assert!(
        sched.starts_with("HTTP/1.1 200 OK\r\n"),
        "schedule status:\n{sched}"
    );
    assert!(sched.ends_with(SCHEDULE), "schedule body:\n{sched}");

    // The HTML shell loads the LOGOS→WASM frontend.
    let home = get(&mut server, "/");
    assert!(
        home.contains("Content-Type: text/html\r\n"),
        "home headers:\n{home}"
    );
    assert!(
        home.contains("<h1>LOGOS Hackathon</h1>"),
        "home body:\n{home}"
    );
    assert!(home.contains("id='schedule'"), "home body:\n{home}");

    // Unknown routes 404.
    assert!(get(&mut server, "/missing").starts_with("HTTP/1.1 404 Not Found\r\n"));
}
