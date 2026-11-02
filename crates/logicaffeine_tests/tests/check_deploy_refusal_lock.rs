//! ★ LOCK — `largo check` fails on a program `largo build` would refuse. ★
//!
//! `largo check` compiles a project's entry to Rust and discards it. That compile hands back the Rust
//! of a program it REFUSED — the refusal is a `compile_error!` inside it, which only rustc ever
//! reads — so a backend could check clean and then not build: `examples/guestbook` did, for as long
//! as a `-> View` function had no AOT-Rust form. A check reads that Rust now and names each refusal.
//! A browser program ships as WebAssembly (`largo serve`), not as a native binary, so a DOM program's
//! entry is not read against the native tier.

#[path = "cli_common/mod.rs"]
mod common;
use common::{largo_in, scaffold, stderr, stdout};

fn project(entry: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path(), "site");
    std::fs::write(dir.path().join("src/main.lg"), entry).unwrap();
    dir
}

/// A route the path matcher cannot bind on the native tier: the tree-walker drops it, and the
/// binary carries a refusal for it.
const REFUSED_BACKEND: &str = "# Prices\n\n## Route GET /price/:amount (amount: Real) -> Text:\n    Return \"x\".\n\n## Main\nServe on port 8080.\n";

#[test]
fn a_backend_largo_build_would_refuse_does_not_check() {
    let dir = project(REFUSED_BACKEND);
    let out = largo_in(dir.path(), &["check"]);
    let err = stderr(&out);
    assert!(
        !out.status.success(),
        "a backend `largo build` refuses must not check clean:\n{err}"
    );
    assert!(
        err.contains("`largo build` refuses this program"),
        "the failure says what refuses it:\n{err}"
    );
    assert!(
        err.contains("this `## Route` cannot run on this tier yet"),
        "and names the refusal itself:\n{err}"
    );
}

#[test]
fn the_refusal_is_a_deploy_refusal_diagnostic() {
    let dir = project(REFUSED_BACKEND);
    let out = largo_in(dir.path(), &["check", "--json"]);
    let envelope: serde_json::Value =
        serde_json::from_str(&stdout(&out)).expect("a logos-check/1 envelope");
    assert_eq!(envelope["ok"], false);
    let codes: Vec<&str> = envelope["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["code"].as_str())
        .collect();
    assert!(
        codes.contains(&"deploy-refusal"),
        "the refusal is its own diagnostic: {codes:?}"
    );
}

#[test]
fn a_backend_that_builds_checks_clean() {
    let dir = project("# Hello\n\n## Route GET /hello -> Text:\n    Return \"hi\".\n\n## Main\nServe on port 8080.\n");
    let out = largo_in(dir.path(), &["check"]);
    assert!(
        out.status.success(),
        "a backend that builds checks clean:\n{}",
        stderr(&out)
    );
}

#[test]
fn a_browser_program_is_not_read_against_the_native_tier() {
    let dir = project(
        "# Counter\n\n## Component App\nState:\n    Let n be 0.\nActions:\n    To bump:\n        Set n to n + 1.\n\
         View:\n    <button on-click={bump}>{n}</button>\n\n## Main\nMount App.\n",
    );
    let out = largo_in(dir.path(), &["check"]);
    assert!(
        out.status.success(),
        "a DOM program ships as WebAssembly, so its native Rust is not its tier:\n{}",
        stderr(&out)
    );
}
