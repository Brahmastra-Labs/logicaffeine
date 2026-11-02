//! Machine-readable envelopes: `largo check --json`, `fmt --check --json`,
//! and `build --json`.
//!
//! The contract agents build against: stdout carries exactly one JSON
//! document with a `schema` discriminator; prose stays on stderr; exit codes
//! are unchanged from the human paths. Diagnostics are `logos-diagnostic/1`
//! objects — the same shape the MCP `logos_check` tool returns.

#[path = "cli_common/mod.rs"]
mod common;

use common::*;
use serde_json::Value;
use tempfile::tempdir;

/// A project whose entry trips the zero-index diagnostic on line 5.
const ZERO_INDEX: &str = "# Main\n\n## Main\n\nLet x be item 0 of xs.\n";

fn parse_stdout(out: &std::process::Output) -> Value {
    let text = stdout(out);
    serde_json::from_str(text.trim())
        .unwrap_or_else(|e| panic!("stdout must be one JSON document ({e}): {text}"))
}

#[test]
fn check_json_clean_project_prints_ok_envelope_exit_0() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "clean");

    let out = largo_in(dir.path(), &["check", "--json"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));

    let v = parse_stdout(&out);
    assert_eq!(v["schema"], "logos-check/1", "{v}");
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["diagnostics"].as_array().unwrap().len(), 0, "{v}");
}

#[test]
fn check_json_parse_error_prints_diagnostic_exit_1() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "broken");
    std::fs::write(dir.path().join("src/main.lg"), ZERO_INDEX).unwrap();

    let out = largo_in(dir.path(), &["check", "--json"]);
    assert_eq!(out.status.code(), Some(1), "findings keep exit 1");

    let v = parse_stdout(&out);
    assert_eq!(v["schema"], "logos-check/1", "{v}");
    assert_eq!(v["ok"], false, "{v}");
    let d = &v["diagnostics"][0];
    assert_eq!(d["code"], "zero-index", "{v}");
    assert_eq!(d["span"]["start_line"], 5, "{v}");
    assert!(
        d["file"].as_str().is_some_and(|f| f.ends_with("main.lg")),
        "the diagnostic names the entry file: {v}"
    );
    assert!(
        d["explanation"].as_str().is_some_and(|e| !e.is_empty()),
        "{v}"
    );
}

#[test]
fn check_json_stdout_is_pure_json() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "pure");
    std::fs::write(dir.path().join("src/main.lg"), ZERO_INDEX).unwrap();

    let out = largo_in(dir.path(), &["check", "--json"]);
    let text = stdout(&out);
    assert!(
        text.trim_start().starts_with('{'),
        "no banner before the JSON: {text}"
    );
    assert!(!has_ansi(&text), "no ANSI in the JSON document: {text}");
    // The human rendering still lands on stderr, where it always was.
    assert!(
        !stderr(&out).is_empty(),
        "prose stays on stderr for humans watching a pipe"
    );
}

#[test]
fn fmt_check_json_reports_dirty_and_clean() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "fmtjson");
    // The canonical tabby fixture from the fmt suite: needs reformatting.
    std::fs::write(
        dir.path().join("src/main.lg"),
        "# Main\n\n## Main\n\tLet x be 5.   \n\tShow x.\n",
    )
    .unwrap();

    let out = largo_in(dir.path(), &["fmt", "--check", "--json"]);
    assert_eq!(out.status.code(), Some(1), "dirty keeps exit 1");
    let v = parse_stdout(&out);
    assert_eq!(v["schema"], "logos-fmt/1", "{v}");
    assert_eq!(v["clean"], false, "{v}");
    let unformatted = v["unformatted"].as_array().unwrap();
    assert_eq!(unformatted.len(), 1, "{v}");
    assert!(unformatted[0].as_str().unwrap().ends_with("main.lg"), "{v}");

    // And the clean case: fix the file, ask again.
    std::fs::write(
        dir.path().join("src/main.lg"),
        "# Main\n\n## Main\n    Let x be 5.\n    Show x.\n",
    )
    .unwrap();
    let out = largo_in(dir.path(), &["fmt", "--check", "--json"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let v = parse_stdout(&out);
    assert_eq!(v["clean"], true, "{v}");
    assert_eq!(v["unformatted"].as_array().unwrap().len(), 0, "{v}");
}

#[test]
fn build_json_compile_error_prints_check_envelope() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "buildjson");
    std::fs::write(dir.path().join("src/main.lg"), ZERO_INDEX).unwrap();

    let out = largo_in(dir.path(), &["build", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    let v = parse_stdout(&out);
    assert_eq!(v["schema"], "logos-check/1", "{v}");
    assert_eq!(v["ok"], false, "{v}");
    assert_eq!(v["diagnostics"][0]["code"], "zero-index", "{v}");
}
