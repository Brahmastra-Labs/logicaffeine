//! `largo run` under limits: `--max-steps` (exit 122), `--timeout` (exit
//! 124), `--seed`, and the `logos-run/1` `--json` envelope.
//!
//! The agent contract: generated code can never hang the caller, exhaustion
//! is a distinct exit code at a deterministic op, and a seeded run reproduces
//! byte-for-byte.

#[path = "cli_common/mod.rs"]
mod common;

use common::*;
use serde_json::Value;
use tempfile::tempdir;

const INFINITE: &str = "# Main\n\n## Main\n\nLet mutable x be 1.\nWhile x is greater than 0:\n    Set x to x plus 1.\nShow x.\n";

const PRINTING_LOOP: &str = "# Main\n\n## Main\n\nLet mutable i be 1.\nWhile i is greater than 0:\n    Show i.\n    Set i to i plus 1.\n";

const RACY: &str = "\
# Main

## To worker:
    Show \"worker\".

## Main

Launch a task to worker.
Show \"main\".
";

fn project(source: &str) -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "limits");
    std::fs::write(dir.path().join("src/main.lg"), source).unwrap();
    dir
}

#[test]
fn max_steps_exits_122_with_partial_output() {
    let dir = project(PRINTING_LOOP);
    let out = largo_in(dir.path(), &["run", "--interpret", "--max-steps", "10000"]);
    assert_eq!(
        out.status.code(),
        Some(122),
        "budget exhaustion is exit 122"
    );
    assert!(
        !stdout(&out).is_empty(),
        "partial output is delivered, never dropped"
    );
    assert!(
        strip_ansi(&stderr(&out)).contains("step budget"),
        "stderr names the budget: {}",
        stderr(&out)
    );
}

#[test]
fn timeout_kills_interpret_run_exit_124() {
    let dir = project(INFINITE);
    let start = std::time::Instant::now();
    let out = largo_in(dir.path(), &["run", "--interpret", "--timeout", "500ms"]);
    assert_eq!(out.status.code(), Some(124), "wall-clock death is exit 124");
    assert!(
        start.elapsed() < std::time::Duration::from_secs(30),
        "the deadline is enforced, not advisory"
    );
}

#[test]
fn timeout_kills_compiled_run_exit_124() {
    let dir = project(INFINITE);
    let out = largo_in(dir.path(), &["run", "--timeout", "10s"]);
    assert_eq!(
        out.status.code(),
        Some(124),
        "the compiled child dies at the deadline too: {}",
        stderr(&out)
    );
}

#[test]
fn seed_flag_reproduces_select_order() {
    let dir = project(RACY);
    let first = largo_in(dir.path(), &["run", "--interpret", "--seed", "7"]);
    let second = largo_in(dir.path(), &["run", "--interpret", "--seed", "7"]);
    assert_eq!(first.status.code(), Some(0), "stderr: {}", stderr(&first));
    assert_eq!(
        stdout(&first),
        stdout(&second),
        "same seed, byte-identical output"
    );
}

#[test]
fn run_json_budget_exceeded_envelope() {
    let dir = project(PRINTING_LOOP);
    let out = largo_in(
        dir.path(),
        &["run", "--interpret", "--max-steps", "10000", "--json"],
    );
    assert_eq!(out.status.code(), Some(122));
    let text = stdout(&out);
    let v: Value = serde_json::from_str(text.trim())
        .unwrap_or_else(|e| panic!("stdout must be one JSON document ({e}): {text}"));
    assert_eq!(v["schema"], "logos-run/1", "{v}");
    assert_eq!(v["outcome"], "budget-exceeded", "{v}");
    assert!(
        v["lines"].as_array().is_some_and(|l| !l.is_empty()),
        "partial lines ride the envelope: {v}"
    );
    assert_eq!(v["steps_used"], 10000, "{v}");
}

#[test]
fn run_json_reports_the_steps_a_finished_run_took() {
    let dir = project(
        "# Main\n\n## Main\n\nLet mutable s be 0.\nRepeat for i from 1 to 200:\n    Set s to s plus i.\nShow s.\n",
    );
    let out = largo_in(
        dir.path(),
        &["run", "--interpret", "--max-steps", "10000000", "--json"],
    );
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let v: Value = serde_json::from_str(stdout(&out).trim()).unwrap();
    assert_eq!(v["outcome"], "ok", "{v}");
    let used = v["steps_used"].as_u64().unwrap_or_else(|| panic!("steps_used is a count: {v}"));
    assert!(used > 200, "a finished run reports what it spent: {v}");
    assert!(used < 10_000_000, "and not the whole budget: {v}");
}

#[test]
fn run_json_ok_envelope() {
    let dir = project("# Main\n\n## Main\n\nShow \"done\".\n");
    let out = largo_in(dir.path(), &["run", "--interpret", "--json"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let v: Value = serde_json::from_str(stdout(&out).trim()).unwrap();
    assert_eq!(v["schema"], "logos-run/1", "{v}");
    assert_eq!(v["outcome"], "ok", "{v}");
    assert_eq!(v["lines"], serde_json::json!(["done"]), "{v}");
}
