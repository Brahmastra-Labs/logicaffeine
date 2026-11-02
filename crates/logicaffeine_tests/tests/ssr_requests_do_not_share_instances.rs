//! LOCK — two requests a server renders never share an instance.
//!
//! The probe (7.7): the compiled server never runs two route handlers at once — a handler that takes
//! the program's bindings is served on one thread (`serve_here`), every other one behind a mutex
//! (`serialize_handler`) — so no two renders race over the page runtime's globals. What two
//! requests can share is what a render leaves behind in the program between them. A server render
//! leaves nothing: a component it draws is its initial state drawn by `__serve_<C>`, into a root
//! discarded once serialized, on a tape set aside and restored around it. So each request sees the
//! state its own props and route give it, and a request whose render fails leaves nothing for the
//! next to find.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use logicaffeine_compile::compile::{compile_program_full, tw_outcome, vm_outcome};

#[path = "common/mod.rs"]
mod common;

#[path = "support/served.rs"]
mod served;
use served::Server;

/// A component whose state starts from the prop its placement hands it, served by a route whose path
/// decides the prop; a route whose render fails part-way; a route that renders cleanly.
const SITE: &str = "\
## Component Hello (who: Text)
State:
    Let greeting be \"Hi \" + who.
    Let visits be 0.
View:
    <p>{greeting} {visits}</p>

## Component Broken
State:
    Let xs be [1].
View:
    <div><p>half</p><p>{item 5 of xs}</p></div>

## Route GET /hi/:who (who: Text) -> View:
    <main><Hello who={who}/></main>

## Route GET /broken -> View:
    <main><Broken/></main>

## Route GET /plain -> View:
    <main><p>plain</p></main>
";

fn with_main(main: &str) -> String {
    format!("{SITE}\n## Main\n{main}")
}

const SEQUENCE: &str = "Show route(\"GET\", \"/hi/ada\").\nShow route(\"GET\", \"/hi/bob\").\nShow route(\"GET\", \"/hi/ada\").\n";

const EXPECTED: [&str; 3] = [
    "<main><p>Hi ada 0</p></main>",
    "<main><p>Hi bob 0</p></main>",
    "<main><p>Hi ada 0</p></main>",
];

#[test]
fn each_request_draws_the_instance_its_own_props_make() {
    let src = with_main(SEQUENCE);
    for (tier, out) in [("tree-walker", tw_outcome(&src)), ("VM", vm_outcome(&src))] {
        assert_eq!(out.error, None, "{tier}:\n{src}");
        let lines: Vec<&str> = out.output.lines().collect();
        assert_eq!(
            lines, EXPECTED,
            "{tier}: each request's `Hello` starts from its own prop"
        );
    }
}

#[test]
fn each_request_draws_its_own_instance_as_compiled_rust() {
    common::assert_output_lines(&with_main(SEQUENCE), &EXPECTED);
}

#[test]
fn a_request_whose_render_fails_leaves_nothing_for_the_next() {
    let src = with_main("Let answered be route(\"GET\", \"/broken\").\nShow answered.\n");
    let out = tw_outcome(&src);
    assert!(
        out.error.is_some() || out.output.contains("500") || !out.output.contains("half"),
        "the broken render does not answer half a page: {out:?}"
    );
    let src =
        with_main("Show routeStatus(\"GET\", \"/broken\").\nShow route(\"GET\", \"/plain\").\n");
    for (tier, out) in [("tree-walker", tw_outcome(&src)), ("VM", vm_outcome(&src))] {
        assert_eq!(out.error, None, "{tier}:\n{src}");
        let lines: Vec<&str> = out.output.lines().collect();
        assert_eq!(
            lines.last().copied(),
            Some("<main><p>plain</p></main>"),
            "{tier}: nothing of the failed render reaches the next: {lines:?}"
        );
        assert!(!out.output.contains("half"), "{tier}: {lines:?}");
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the workspace root")
        .to_path_buf()
}

/// The site compiled to Rust and built as a binary serving on its program's port, 8080, unless
/// `LOGOS_PORT` says otherwise.
fn build_server() -> PathBuf {
    let src = with_main("Serve on port 8080.\n");
    let out = compile_program_full(&src).unwrap_or_else(|e| panic!("the site compiles: {e:?}"));
    let dir = std::env::temp_dir().join(format!("logos_ssr_instances_{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).expect("the project directory");
    let root = workspace_root();
    let manifest = format!(
        "[package]\nname = \"ssr_instances\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n\
         logicaffeine-data = {{ path = \"{0}/crates/logicaffeine_data\" }}\n\
         logicaffeine-system = {{ path = \"{0}/crates/logicaffeine_system\", features = [\"full\"] }}\n\
         tokio = {{ version = \"1\", features = [\"rt-multi-thread\", \"macros\"] }}\n\
         serde = {{ version = \"1\", features = [\"derive\"] }}\nrayon = \"1\"\nmemchr = \"2\"\n",
        root.display()
    );
    std::fs::write(dir.join("Cargo.toml"), manifest).expect("the manifest");
    std::fs::write(dir.join("src/main.rs"), &out.rust_code).expect("the program");
    let lock = root.join("Cargo.lock");
    if lock.exists() {
        std::fs::copy(&lock, dir.join("Cargo.lock")).expect("the lockfile");
    }
    let target = std::env::temp_dir().join("logos_ssr_instances_target");
    let built = Command::new("cargo")
        .args(["build", "--quiet", "--offline"])
        .current_dir(&dir)
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .expect("cargo runs");
    assert!(
        built.status.success(),
        "the server builds:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    target.join("debug/ssr_instances")
}

fn get(port: u16, path: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(mut s) => {
                s.set_read_timeout(Some(Duration::from_secs(10))).ok();
                write!(
                    s,
                    "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
                )
                .expect("the request");
                let mut raw = String::new();
                s.read_to_string(&mut raw).ok();
                return raw
                    .split_once("\r\n\r\n")
                    .map(|(_, body)| body.to_string())
                    .unwrap_or(raw);
            }
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => panic!("the server never answered: {e}"),
        }
    }
}

/// Two requests at once, on the compiled server, each render its own instance.
#[test]
#[ignore = "builds an AOT server via cargo (slow); run in the full suite"]
fn concurrent_requests_on_the_compiled_server_render_their_own_instances() {
    let bin = build_server();
    let mut compiled = Command::new(bin);
    compiled.env("LOGOS_PORT", "0");
    let server = Server::spawn(compiled);
    let port = server.port();
    get(port, "/plain");
    let paths = [
        "/hi/ada", "/hi/bob", "/broken", "/hi/cy", "/plain", "/hi/ada",
    ];
    for _ in 0..8 {
        let answers: Vec<(String, String)> = std::thread::scope(|scope| {
            let handles: Vec<_> = paths
                .iter()
                .map(|p| scope.spawn(move || (p.to_string(), get(port, p))))
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("a request thread"))
                .collect()
        });
        for (path, body) in answers {
            match path.as_str() {
                "/hi/ada" => assert_eq!(body, "<main><p>Hi ada 0</p></main>"),
                "/hi/bob" => assert_eq!(body, "<main><p>Hi bob 0</p></main>"),
                "/hi/cy" => assert_eq!(body, "<main><p>Hi cy 0</p></main>"),
                "/plain" => assert_eq!(body, "<main><p>plain</p></main>"),
                _ => assert!(
                    !body.contains("half"),
                    "a failed render answers no half page: {body}"
                ),
            }
        }
    }
}
