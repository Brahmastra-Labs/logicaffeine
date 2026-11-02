//! LOCK — a page's scripted requests carry the visitor's CSRF token on every serving tier.
//!
//! With a `## Session`, the router refuses a state-changing request without the visitor's token
//! (`csrf_automatic_lock`). A server-rendered `<form>` carries it in `_csrf`; a PAGE's own scripted
//! request (`Post … to "/api"` from a lowered page) sends it as `x-csrf-token`, read from the
//! document's `<meta name="csrf-token">` (`web_host::with_csrf_meta`, `runtime.js` and the recorder
//! alike). Nothing gave a served page that meta, so every page request that changes state was
//! refused with 403.
//!
//! The backend answers the visitor's token at one reserved path (`CSRF_TOKEN_PATH`), minting the
//! visit's nonce and setting its cookie as `csrf()` always does, never cached; whatever composes a
//! page document asks it and writes the meta in.

#[path = "cli_common/mod.rs"]
mod common;

use common::http;
use logicaffeine_compile::compile::{compile_to_rust, tw_outcome};
use logicaffeine_language::parser::lgx_lower::CSRF_TOKEN_PATH;
use logicaffeine_system::csrf::CSRF_META_NAME;
use std::path::{Path, PathBuf};
use std::process::Command;

const SESSION: &str = "## Session\n\x20   a user, which is Text.\n\n";

/// A `Request` with a cookie jar, one more header and a body.
const MK: &str = "## To mk (m: Text) and (p: Text) and (c: Text) and (hn: Text) and (hv: Text) and (b: Text) -> Request:\n\
                 \x20   Let h be a new Map of Text to Text.\n\
                 \x20   Set item \"cookie\" of h to c.\n\
                 \x20   If hn is not equal to \"\":\n\
                 \x20       Set item hn of h to hv.\n\
                 \x20   Return a new Request with method m and path p and body b and headers h \
                  and query (a new Map of Text to Text).\n\n";

const ROUTES: &str = "## Route POST /save -> Text:\n\x20   Return \"saved\".\n\n";

fn run(subject: &str, src: &str) -> String {
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "`{subject}` must run:\n{src}");
    out.output.trim().to_string()
}

/// Ask the reserved path for a token (a fresh visitor), then POST `/save` with the cookie it set
/// and `header_value` in `x-csrf-token`; show what each step answered.
fn page_exchange(header_value: &str) -> String {
    let src = format!(
        "{SESSION}{MK}{ROUTES}\
         ## Main\n\
         Let t be __lgx_route_handler(mk(\"GET\", \"{CSRF_TOKEN_PATH}\", \"\", \"\", \"\", \"\")).\n\
         Show t's status.\n\
         Show headerIn(t's headers, \"cache-control\").\n\
         Let jar be cutBefore(headerIn(t's headers, \"set-cookie\"), \";\").\n\
         If jar is equal to \"\":\n\x20   Show \"no cookie\".\n\
         Let token be t's body.\n\
         Let s be __lgx_route_handler(mk(\"POST\", \"/save\", jar, \"x-csrf-token\", {header_value}, \"\")).\n\
         Show s's status.\n"
    );
    run("a page's token exchange", &src)
}

#[test]
fn the_reserved_path_answers_the_visitors_token_which_the_header_carries_back() {
    assert_eq!(page_exchange("token"), "200\nno-store\n200");
}

#[test]
fn a_header_carrying_another_value_is_refused() {
    // Anti-vacuity: the POST is accepted because the header carries THIS visitor's token.
    assert_eq!(page_exchange("token + \"0\""), "200\nno-store\n403");
}

#[test]
fn the_route_walks_agree_about_the_reserved_path() {
    let src = format!(
        "{SESSION}{ROUTES}## Main\n\
         Show routeStatus(\"GET\", \"{CSRF_TOKEN_PATH}\").\n\
         Show routeContentType(\"GET\", \"{CSRF_TOKEN_PATH}\").\n"
    );
    let out = run("the route walks", &src);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.first(), Some(&"200"), "{out}");
    assert!(
        lines.get(1).is_some_and(|l| l.starts_with("text/plain")),
        "{out}"
    );
}

#[test]
fn without_a_session_the_reserved_path_is_not_served() {
    let src = format!("{ROUTES}## Main\nShow routeStatus(\"GET\", \"{CSRF_TOKEN_PATH}\").\n");
    assert_eq!(run("no session", &src), "404");
}

#[test]
fn it_builds_on_the_tier_a_server_ships_as() {
    let src = format!("{SESSION}{ROUTES}## Main\nServe on port 8080.\n");
    let rust = compile_to_rust(&src).unwrap_or_else(|e| panic!("the server compiles: {e:?}"));
    assert!(
        !rust.contains("compile_error!"),
        "no refusal in the emitted server"
    );
}

/// A full-stack project's backend: a `## Session`, and one route that changes state.
const SESSION_BACKEND: &str = "# Site\n\n\
                               ## Session\n\x20   a user, which is Text.\n\n\
                               ## Route POST /api/save -> Text:\n\x20   Return \"saved\".\n\n\
                               ## Main\n\nServe on port 8080.\n";

/// A `largo new --web` project named `name` under `dir`, its backend replaced by [`SESSION_BACKEND`].
fn session_site(dir: &Path, name: &str) -> PathBuf {
    let out = common::largo_in(dir, &["new", name, "--web"]);
    assert!(
        out.status.success(),
        "largo new --web: {}",
        common::stderr(&out)
    );
    let root = dir.join(name);
    std::fs::write(root.join("src/main.lg"), SESSION_BACKEND).unwrap();
    std::fs::write(root.join("static/plain.html"), STATIC_DOCUMENT).unwrap();
    root
}

/// A static HTML file beside the pages: a file, served as it is, never a page.
const STATIC_DOCUMENT: &str =
    "<!doctype html><html><head><title>plain</title></head><body>plain</body></html>";

/// The token a page document carries in its `<meta name="csrf-token">`.
fn meta_token(document: &str) -> Option<String> {
    let open = format!("<meta name=\"{CSRF_META_NAME}\" content=\"");
    let from = document.find(&open)? + open.len();
    document[from..].split('"').next().map(str::to_string)
}

/// What every serving tier listening on `port` must show a visitor: the page carries the
/// visitor's own token, sets the cookie that token belongs to, and is never cached; a returning
/// visitor keeps its token and another visitor gets another; the page's token is what the router
/// accepts from that visitor's scripted request, and a different one is refused.
fn serves_each_visitor_their_token(port: u16) {
    let page = http::request(port, "GET", "/", &[], "");
    let shown = String::from_utf8_lossy(&page).into_owned();
    assert_eq!(http::status(&page), 200, "{shown}");
    assert_eq!(
        http::header(&page, "cache-control").as_deref(),
        Some("no-store"),
        "never cached:\n{shown}"
    );
    assert_eq!(
        http::header(&page, "etag"),
        None,
        "a visitor's document carries no validator:\n{shown}"
    );
    let cookie = http::header(&page, "set-cookie")
        .unwrap_or_else(|| panic!("the visit's cookie is set:\n{shown}"));
    let jar = cookie.split(';').next().unwrap().to_string();
    let token = meta_token(&http::body(&page))
        .unwrap_or_else(|| panic!("the page carries a token:\n{shown}"));
    assert!(!token.is_empty(), "{shown}");

    let again = http::request(port, "GET", "/", &[("cookie", &jar)], "");
    assert_eq!(
        meta_token(&http::body(&again)),
        Some(token.clone()),
        "a returning visitor keeps its token"
    );
    let other = http::request(port, "GET", "/", &[], "");
    let theirs = meta_token(&http::body(&other)).expect("another visitor's page carries a token");
    assert_ne!(theirs, token, "two visitors never share a token");

    let saved = http::request(
        port,
        "POST",
        "/api/save",
        &[("cookie", &jar), ("x-csrf-token", &token)],
        "",
    );
    assert_eq!(
        http::status(&saved),
        200,
        "{}",
        String::from_utf8_lossy(&saved)
    );
    assert_eq!(http::body(&saved), "saved");
    let forged = http::request(
        port,
        "POST",
        "/api/save",
        &[("cookie", &jar), ("x-csrf-token", &theirs)],
        "",
    );
    assert_eq!(
        http::status(&forged),
        403,
        "another visitor's token is refused: {}",
        String::from_utf8_lossy(&forged)
    );

    let file = http::request(port, "GET", "/plain.html", &[], "");
    let shown = String::from_utf8_lossy(&file).into_owned();
    assert_eq!(http::status(&file), 200, "{shown}");
    assert!(
        http::body(&file).contains("<head><title>plain</title></head>"),
        "a static document is served as it is:\n{shown}"
    );
    assert_eq!(
        meta_token(&http::body(&file)),
        None,
        "a static document is not a page:\n{shown}"
    );
    assert_eq!(http::header(&file, "set-cookie"), None, "{shown}");
}

#[test]
fn the_dev_server_serves_a_page_with_its_visitors_token() {
    let dir = tempfile::tempdir().unwrap();
    let root = session_site(dir.path(), "site");
    let mut server = http::Server::start(&root);
    let port = server.port();
    server.get("/");
    serves_each_visitor_their_token(port);
}

#[test]
#[ignore = "compiles an AOT binary via cargo (slow); run in the full suite"]
fn the_built_binary_serves_a_page_with_its_visitors_token() {
    let dir = tempfile::tempdir().unwrap();
    let name = format!("csrfsite_{}", std::process::id());
    let root = session_site(dir.path(), &name);
    let cache = std::env::temp_dir().join("logos_e2e_cache");
    let build = common::largo()
        .args(["build", "--release"])
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", &cache)
        .output()
        .expect("largo build --release spawns");
    assert!(
        build.status.success(),
        "largo build --release:\n{}\n{}",
        common::stdout(&build),
        common::stderr(&build)
    );
    let artifact = root.join("target").join("release").join(&name);
    assert!(
        artifact.exists(),
        "the stable artifact is at {}",
        artifact.display()
    );

    let cwd = tempfile::tempdir().unwrap();
    let mut binary = Command::new(&artifact);
    binary.env("PORT", "0").current_dir(cwd.path());
    let server = http::Server::spawn(binary);
    serves_each_visitor_their_token(server.port());
}
