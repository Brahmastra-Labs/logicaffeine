//! LOCK — a runtime failure ends the program unless a boundary catches it, and a served request is such a boundary.
//!
//! A generated program's failure hook reports a failure and ends the program with exit status 1, as every tier does,
//! unless `logicaffeine_system::failure` says a boundary on this thread will catch it: a served request, which
//! answers `500` while the server keeps serving, or a task the replay scheduler polls. The runtime catches a panic in
//! no other way, and a program with such a boundary is built to unwind in release, where every other one aborts:
//! both project writers, `write_cargo_project` and `largo build`, take the panic strategy from the compiler.

#![cfg(not(target_arch = "wasm32"))]

#[path = "support/served.rs"]
mod served;

use std::path::{Path, PathBuf};
use std::process::Command;

use logicaffeine_compile::compile::{compile_program_full, write_cargo_project};
use served::Server;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root")
}

/// Every `.rs` file under `dir`.
fn sources(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(d) = pending.pop() {
        for entry in std::fs::read_dir(&d).expect("a source directory") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// A site with a route whose render fails part-way and a route that renders cleanly.
const SITE: &str = "\
## Component Broken
State:
    Let xs be [1].
View:
    <div><p>half</p><p>{item 5 of xs}</p></div>

## Route GET /broken -> View:
    <main><Broken/></main>

## Route GET /plain -> View:
    <main><p>plain</p></main>

## Main
Serve on port 8080.
";

#[test]
fn the_runtime_catches_a_panic_only_through_the_failure_boundary() {
    let src = root().join("crates/logicaffeine_system/src");
    let mut raw = Vec::new();
    for file in sources(&src) {
        if file.ends_with("failure.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&file).expect("a source file");
        for (i, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if !code.starts_with("//") && code.contains("catch_unwind") {
                raw.push(format!("{}:{}: {}", file.display(), i + 1, code));
            }
        }
    }
    assert!(
        raw.is_empty(),
        "catch a panic through `failure::caught`, so a generated program's failure hook knows it is caught:\n{}",
        raw.join("\n")
    );
}

#[test]
fn a_generated_program_ends_on_a_failure_no_boundary_catches() {
    let out = compile_program_full("## Main\nShow 1.\n").expect("the program compiles");
    assert!(
        out.rust_code
            .contains("if !logicaffeine_system::failure::is_caught() { std::process::exit(1); }"),
        "the failure hook ends the program unless a boundary catches the failure:\n{}",
        out.rust_code
    );
}

#[test]
fn a_served_program_unwinds_in_its_generated_release_project() {
    let out = compile_program_full(SITE).expect("the site compiles");
    let project = tempfile::tempdir().expect("a project directory");
    write_cargo_project(&out.rust_code, &out.dependencies, project.path())
        .expect("the project is written");
    let manifest = std::fs::read_to_string(project.path().join("Cargo.toml")).expect("the manifest");
    assert!(
        manifest.contains("panic = \"unwind\""),
        "a served request catches its failure, so the release build must unwind:\n{manifest}"
    );
}

#[test]
fn every_project_writer_takes_the_release_panic_strategy_from_the_compiler() {
    let root = root();
    let mut literal = Vec::new();
    for dir in ["crates/logicaffeine_compile/src", "apps/logicaffeine_cli/src"] {
        for file in sources(&root.join(dir)) {
            let text = std::fs::read_to_string(&file).expect("a source file");
            for (i, line) in text.lines().enumerate() {
                if line.contains("panic = \\\"abort\\\"") || line.contains("panic = \\\"unwind\\\"") {
                    literal.push(format!("{}:{}", file.display(), i + 1));
                }
            }
        }
    }
    assert!(
        literal.is_empty(),
        "a generated project's panic strategy comes from `codegen::release_panic_strategy`, never a literal:\n{}",
        literal.join("\n")
    );
    for writer in [
        "crates/logicaffeine_compile/src/compile.rs",
        "apps/logicaffeine_cli/src/project/build.rs",
    ] {
        let text = std::fs::read_to_string(root.join(writer)).expect("a project writer");
        assert!(
            text.contains("release_panic_strategy(") && text.contains("[profile.release]"),
            "{writer} writes a release profile whose panic strategy the compiler decides"
        );
    }
}

/// The site built as `largo build --release` ships it, from the project `write_cargo_project` writes: a failed
/// render answers `500`, its reason goes to stderr, and the server keeps serving.
#[test]
#[ignore = "builds a release server via cargo (slow); run in the full suite"]
fn a_release_server_answers_a_failed_request_and_keeps_serving() {
    let out = compile_program_full(SITE).expect("the site compiles");
    let project = tempfile::tempdir().expect("a project directory");
    write_cargo_project(&out.rust_code, &out.dependencies, project.path())
        .expect("the project is written");
    let target = std::env::temp_dir().join("logos_failure_boundary_release_target");
    let built = Command::new("cargo")
        .args(["build", "--release", "--offline", "--quiet"])
        .current_dir(project.path())
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .expect("cargo runs");
    assert!(
        built.status.success(),
        "the release server builds:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let mut server_cmd = Command::new(target.join("release/logos_output"));
    server_cmd.env("LOGOS_PORT", "0");
    let mut server = Server::spawn(server_cmd);
    for round in 0..3 {
        let broken = String::from_utf8_lossy(&server.get("/broken")).to_string();
        assert!(
            broken.starts_with("HTTP/1.1 500") && !broken.contains("half"),
            "round {round}: a failed render answers 500 and no half page:\n{broken}"
        );
        let plain = String::from_utf8_lossy(&server.get("/plain")).to_string();
        assert!(
            plain.starts_with("HTTP/1.1 200") && plain.ends_with("<main><p>plain</p></main>"),
            "round {round}: the server keeps serving after a failed request:\n{plain}\n{}",
            server.stderr()
        );
    }
    assert!(!server.exited(), "the server is still serving");
}
