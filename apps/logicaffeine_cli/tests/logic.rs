//! `largo logic` — English → First-Order Logic from the terminal.
//!
//! Every assertion oracles against the `logicaffeine_language` compile API
//! the command wraps: the CLI must agree with the library byte-for-byte.

mod common;

use std::io::Write;
use std::process::Stdio;

use common::*;
use logicaffeine_language::{compile, CompileOptions, OutputFormat};
use tempfile::tempdir;

const SENTENCE: &str = "Every cat sleeps.";

fn oracle(format: OutputFormat) -> String {
    logicaffeine_language::compile::compile_with_options(
        SENTENCE,
        CompileOptions { format, pragmatic: false },
    )
    .expect("oracle sentence must compile")
}

/// An inline sentence prints exactly the library's Unicode FOL.
#[test]
fn inline_sentence_matches_library() {
    let out = largo().args(["logic", SENTENCE]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "logic: {}", stderr(&out));
    assert_eq!(stdout(&out).trim_end(), compile(SENTENCE).unwrap());
}

/// Each `--format` agrees with the corresponding library option.
#[test]
fn formats_match_library_options() {
    for (flag, format, must_contain) in [
        ("unicode", OutputFormat::Unicode, "∀"),
        ("latex", OutputFormat::LaTeX, "\\forall"),
        ("ascii", OutputFormat::SimpleFOL, "("),
        ("kripke", OutputFormat::Kripke, "("),
    ] {
        let out = largo()
            .args(["logic", SENTENCE, "--format", flag])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "--format {flag}: {}", stderr(&out));
        let got = stdout(&out);
        let got = got.trim_end();
        assert_eq!(got, oracle(format), "--format {flag} must match the library");
        assert!(got.contains(must_contain), "--format {flag} sanity: {got}");
    }
}

/// `--file` compiles the file's contents.
#[test]
fn file_input_works() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("sentence.txt");
    std::fs::write(&path, SENTENCE).unwrap();
    let out = largo()
        .args(["logic", "--file", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim_end(), compile(SENTENCE).unwrap());
}

/// Piped stdin is the input when no sentence or file is given.
#[test]
fn stdin_input_works() {
    let mut child = largo()
        .arg("logic")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(SENTENCE.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim_end(), compile(SENTENCE).unwrap());
}

/// Empty input is a usage error.
#[test]
fn empty_input_is_usage_error() {
    let mut child = largo()
        .arg("logic")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(2), "empty input must be a usage error");
}

/// `--all-readings` lists every reading, numbered, matching the library's
/// scope permutations + parse forest (deduplicated, in that order).
#[test]
fn all_readings_match_library() {
    let ambiguous = "Every woman loves a man.";
    let opts = CompileOptions { format: OutputFormat::Unicode, pragmatic: false };
    let mut expected: Vec<String> = Vec::new();
    for r in logicaffeine_language::compile::compile_all_scopes_with_options(ambiguous, opts)
        .unwrap_or_default()
    {
        if !expected.contains(&r) {
            expected.push(r);
        }
    }
    for r in logicaffeine_language::compile::compile_forest_with_options(ambiguous, opts) {
        if !expected.contains(&r) {
            expected.push(r);
        }
    }
    assert!(expected.len() >= 2, "oracle sentence must be ambiguous");

    let out = largo()
        .args(["logic", ambiguous, "--all-readings"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    for (i, reading) in expected.iter().enumerate() {
        let line = format!("{}. {}", i + 1, reading);
        assert!(text.contains(&line), "missing reading {line:?} in:\n{text}");
    }
}

/// `--discourse` feeds each non-empty line as one sentence with shared
/// anaphora context, matching `compile_discourse`.
#[test]
fn discourse_matches_library() {
    let lines = ["A farmer owns a donkey.", "He feeds it."];
    let expected = logicaffeine_language::compile::compile_discourse_with_options(
        &lines,
        CompileOptions::default(),
    )
    .expect("discourse oracle must compile");

    let mut child = largo()
        .args(["logic", "--discourse"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(lines.join("\n").as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim_end(), expected);
}

/// `--pragmatic` matches `compile_pragmatic` (scalar implicature on).
#[test]
fn pragmatic_matches_library() {
    let scalar = "Some cats sleep.";
    let expected = logicaffeine_language::compile::compile_pragmatic(scalar)
        .expect("pragmatic oracle must compile");
    let out = largo().args(["logic", scalar, "--pragmatic"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim_end(), expected);
}

/// A parse failure reports on stderr (socratic explanation), exits 1, and
/// keeps stdout clean.
#[test]
fn parse_error_reports_socratically() {
    let out = largo().args(["logic", "the the the."]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "garbage must fail");
    assert_eq!(stdout(&out), "", "stdout stays clean on error");
    assert!(!stderr(&out).trim().is_empty(), "stderr must explain");
}

/// FOL output through a pipe carries no ANSI by default.
#[test]
fn piped_fol_has_no_ansi() {
    let out = largo().args(["logic", SENTENCE]).output().unwrap();
    assert!(!has_ansi(&stdout(&out)), "no ANSI in piped FOL");
}

/// `--discourse` and `--all-readings` are mutually exclusive.
#[test]
fn discourse_conflicts_with_all_readings() {
    let out = largo()
        .args(["logic", SENTENCE, "--discourse", "--all-readings"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}
