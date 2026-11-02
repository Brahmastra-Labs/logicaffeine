//! `largo explain <topic>` — offline lookup of error codes and constructs.
//!
//! One verb, two namespaces: `logos-diagnostic/1` error codes (from the
//! canonical catalog) first, then language constructs (from the teach
//! lessons). Works anywhere — no project required.

#[path = "cli_common/mod.rs"]
mod common;

use common::*;

#[test]
fn explain_error_code_prints_catalog_entry() {
    let out = largo().args(["explain", "zero-index"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let text = strip_ansi(&stdout(&out));
    assert!(text.contains("zero-index"), "{text}");
    assert!(text.contains("info"), "severity is shown: {text}");
    assert!(
        text.contains("Use 1-based indexing"),
        "the quickfix note is shown: {text}"
    );
    assert!(
        text.contains("item 0"),
        "the socratic explanation is shown: {text}"
    );
}

#[test]
fn explain_construct_prints_lesson() {
    let out = largo().args(["explain", "Let"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let text = strip_ansi(&stdout(&out));
    assert!(text.contains("Let"), "{text}");
    assert!(
        text.contains("Example"),
        "lessons carry a runnable example: {text}"
    );
}

#[test]
fn explain_unknown_topic_exits_1_with_nearest_suggestion() {
    let out = largo().args(["explain", "zero-indx"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = strip_ansi(&stderr(&out));
    assert!(
        err.contains("zero-index"),
        "the nearest real topic is suggested: {err}"
    );
}

#[test]
fn explain_json_is_pure_json() {
    let out = largo()
        .args(["explain", "zero-index", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let text = stdout(&out);
    let v: serde_json::Value = serde_json::from_str(text.trim())
        .unwrap_or_else(|e| panic!("stdout must be pure JSON ({e}): {text}"));
    assert_eq!(v["schema"], "logos-explain/1", "{v}");
    assert_eq!(v["entries"][0]["code"], "zero-index", "{v}");
}
