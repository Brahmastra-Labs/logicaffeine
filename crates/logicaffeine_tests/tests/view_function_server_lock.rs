//! ★ LOCK — a view a function returns is served as the markup it renders, on every tier. ★
//!
//! `## To answer (who: Text) -> View:` is a template macro: its value is the view it renders. The
//! tree-walker served one — a route returning `answer(…)` answered the markup — and nothing else
//! could: AOT-Rust carries a `View` as a DOM handle it has no document for, and refused the macro's
//! body by name, so `examples/guestbook`, a server-rendered site with no JavaScript anywhere, did not
//! `largo build`. A statements-bodied `-> View` route was also typed `text/plain` on every tier,
//! because only a template-bodied route counted as a view.
//!
//! On a server a view IS its HTML. Each row names that shape rather than the mechanism that
//! provides it: what a route serves builds on the deploy tier, the bytecode tier serves the same
//! bytes as the tree-walker, both call it `text/html`, and the compiled server answers the same
//! bytes and the same `Content-Type` over a real socket.

#[path = "support/served.rs"]
mod served;
use served::Server;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use logicaffeine_compile::compile::{
    compile_program_full, compile_to_rust, tw_outcome, vm_outcome,
};
use logicaffeine_compile::semantics::dom;

/// A macro, a statements route returning it, and a helper chain between them.
const SITE: &str = "\
## To badge (label: Text) -> View:
    <b class=\"badge\">{label}</b>

## To answer (saved: Bool) and (who: Text) -> View:
    <main>
      {if saved:}
        <h1>Signed</h1>
        <p>Thank you, {who}.</p>
      {else:}
        <h1>Not signed</h1>
      {end}
      <p>{badge(who)}</p>
    </main>

## To page (who: Text) -> View:
    Return answer(true, who).

## Route GET /thanks/:who (who: Text) -> View:
    Return answer(true, who).

## Route GET /helper/:who (who: Text) -> View:
    Let shown be page(who).
    Return shown.

## Route GET /nobody -> View:
    Return answer(false, \"\").

## Route GET /template -> View:
    <div>{badge(\"t\")}</div>
";

const PATHS: &[&str] = &["/thanks/ada", "/helper/ada", "/nobody", "/template"];

fn with_main(main: &str) -> String {
    format!("{SITE}\n## Main\n{main}")
}

/// What the tree-walker's route table answers for `path`: body, then content type.
fn tree_walker(path: &str) -> (String, String) {
    let src = with_main(&format!(
        "Show route(\"GET\", \"{path}\").\nShow routeContentType(\"GET\", \"{path}\").\n"
    ));
    let out = tw_outcome(&src);
    assert_eq!(
        out.error, None,
        "the tree-walker must serve `{path}`:\n{src}"
    );
    let text = out.output.trim_end().to_string();
    let (body, ctype) = text
        .rsplit_once('\n')
        .unwrap_or_else(|| panic!("two lines for `{path}`: {text:?}"));
    (body.to_string(), ctype.to_string())
}

#[test]
fn a_view_a_route_returns_is_served_as_its_markup() {
    let (body, ctype) = tree_walker("/thanks/ada");
    assert_eq!(
        body, "<main><h1>Signed</h1><p>Thank you, ada.</p><p><b class=\"badge\">ada</b></p></main>",
        "the served bytes are the markup the view renders"
    );
    assert_eq!(
        ctype, "text/html; charset=utf-8",
        "a view is served as HTML, whatever the route's body is"
    );
}

#[test]
fn every_route_serving_a_view_says_html() {
    for path in PATHS {
        let (_, ctype) = tree_walker(path);
        assert_eq!(
            ctype, "text/html; charset=utf-8",
            "`{path}` serves a view, so it is HTML"
        );
    }
}

#[test]
fn a_view_route_runs_its_statements() {
    // `n` is 42 only if the statements ran; a body served as a template would show its own source.
    let src = format!(
        "{SITE}\n## Route GET /computed -> View:\n    Let n be 2.\n    Set n to n * 21.\n    Return badge(\"{{n}}\").\n\n\
         ## Main\nShow route(\"GET\", \"/computed\").\n"
    );
    let out = tw_outcome(&src);
    assert_eq!(out.error, None, "{src}");
    assert_eq!(
        out.output.trim(),
        "<b class=\"badge\">42</b>",
        "a `-> View` route's statements run, and it serves what they return"
    );
    assert_eq!(
        vm_outcome(&src).output.trim(),
        out.output.trim(),
        "and the VM agrees"
    );
}

const PLACED: &str = "\
## To badge (label: Text) -> View:
    <b class=\"badge\">{label}</b>

## Component App
State:
    Let who be \"ada\".
View:
    <p>{badge(who)}</p>
";

#[test]
fn a_view_a_page_places_is_still_drawn_as_nodes() {
    dom::reset();
    let src = format!("{PLACED}\n## Main\nMount App.\n");
    let out = tw_outcome(&src);
    assert_eq!(out.error, None, "{src}");
    let painted = dom::render();
    assert!(
        painted.contains("<p><b class=\"badge\">ada</b></p>"),
        "a view a page places is drawn into the document as nodes: {painted}"
    );
    dom::reset();
    let served = tw_outcome(&format!("{PLACED}\n## Main\nShow html of App.\n"));
    assert_eq!(served.error, None);
    assert!(
        served.output.contains("<p><b class=\"badge\">ada</b></p>"),
        "and the server walk serves the same markup: {}",
        served.output
    );
}

#[test]
fn a_view_placed_by_a_page_and_served_by_a_route_is_both() {
    dom::reset();
    let src = format!(
        "{PLACED}\n## Route GET /b -> View:\n    Return badge(\"route\").\n\n\
         ## Main\nMount App.\nShow route(\"GET\", \"/b\").\n"
    );
    let out = tw_outcome(&src);
    assert_eq!(out.error, None, "{src}");
    assert_eq!(
        out.output.trim(),
        "<b class=\"badge\">route</b>",
        "the route serves the markup"
    );
    assert!(
        dom::render().contains("<p><b class=\"badge\">ada</b></p>"),
        "while the page still draws the view as nodes"
    );
}

#[test]
fn a_view_placed_in_a_served_template_is_markup_not_text() {
    let (body, _) = tree_walker("/template");
    assert_eq!(
        body, "<div><b class=\"badge\">t</b></div>",
        "a view in a hole is markup, never its escaped text"
    );
}

#[test]
fn the_bytecode_tier_serves_the_same_bytes() {
    for path in PATHS {
        let src = with_main(&format!(
            "Show route(\"GET\", \"{path}\").\nShow routeContentType(\"GET\", \"{path}\").\n"
        ));
        let tw = tw_outcome(&src);
        let vm = vm_outcome(&src);
        assert_eq!(vm.error, None, "the VM must serve `{path}`:\n{src}");
        assert_eq!(
            vm.output.trim(),
            tw.output.trim(),
            "`{path}`: the VM serves what the tree-walker serves"
        );
    }
}

#[test]
fn a_site_serving_views_builds_on_the_deploy_tier() {
    let rust = compile_to_rust(&with_main("Serve on port 8080.\n")).expect("compiles");
    let refusals: Vec<&str> = rust
        .lines()
        .filter(|l| l.contains("compile_error!"))
        .map(str::trim)
        .collect();
    assert!(
        refusals.is_empty(),
        "the binary would not build:\n{}",
        refusals.join("\n")
    );
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Build the site as a real server binary, the way `largo build` compiles a backend.
fn build_server() -> PathBuf {
    let root = workspace_root();
    let out = compile_program_full(&with_main("Serve on port 8080.\n")).expect("compiles");
    let project = std::env::temp_dir().join(format!("logos_view_server_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("src/main.rs"), &out.rust_code).unwrap();
    let deps: String = out
        .dependencies
        .iter()
        .map(|d| format!("{} = \"{}\"\n", d.name, d.version))
        .collect();
    std::fs::write(
        project.join("Cargo.toml"),
        format!(
            "[package]\nname = \"logos_view_server\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n\
             logicaffeine-data = {{ path = \"{root}/crates/logicaffeine_data\" }}\n\
             logicaffeine-system = {{ path = \"{root}/crates/logicaffeine_system\", features = [\"full\"] }}\n\
             tokio = {{ version = \"1\", features = [\"rt-multi-thread\", \"macros\"] }}\n\
             serde = {{ version = \"1\", features = [\"derive\"] }}\nrayon = \"1\"\nmemchr = \"2\"\n{deps}",
            root = root.display()
        ),
    )
    .unwrap();
    std::fs::copy(root.join("Cargo.lock"), project.join("Cargo.lock")).unwrap();
    let target = std::env::temp_dir().join("logos_view_server_target");
    let build = Command::new("cargo")
        .args(["build", "--quiet", "--offline"])
        .current_dir(&project)
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .expect("cargo runs");
    assert!(
        build.status.success(),
        "the site does not build:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    target.join("debug").join("logos_view_server")
}

fn get(port: u16, path: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(mut s) => {
                s.write_all(
                    format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
                        .as_bytes(),
                )
                .unwrap();
                let mut buf = String::new();
                s.read_to_string(&mut buf).unwrap();
                return buf;
            }
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => panic!("the compiled server never came up on {port}: {e}"),
        }
    }
}

#[test]
fn the_compiled_server_answers_the_tree_walkers_bytes_and_content_type() {
    let bin = build_server();
    let mut compiled = Command::new(&bin);
    compiled.env("LOGOS_PORT", "0");
    let server = Server::spawn(compiled);
    let port = server.port();
    for path in PATHS {
        let (want_body, want_ctype) = tree_walker(path);
        let response = get(port, path);
        let (head, body) = response
            .split_once("\r\n\r\n")
            .unwrap_or_else(|| panic!("`{path}`: {response:?}"));
        assert!(head.starts_with("HTTP/1.1 200"), "`{path}`: {head}");
        let ctype = head
            .lines()
            .find_map(|l| {
                l.split_once(':')
                    .filter(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                    .map(|(_, v)| v.trim())
            })
            .unwrap_or_else(|| panic!("`{path}` has no Content-Type:\n{head}"));
        assert_eq!(
            ctype, want_ctype,
            "`{path}`: the compiled server and the tree-walker agree on the Content-Type"
        );
        assert_eq!(
            body, want_body,
            "`{path}`: the compiled server serves the tree-walker's bytes"
        );
    }
}
