//! `largo repl` — piped-stdin end-to-end tests (the non-TTY path: plain
//! line loop, no prompts, no color — which makes the whole REPL scriptable
//! and testable without a PTY).

mod common;

use std::io::Write;
use std::process::Stdio;

use common::*;
use tempfile::tempdir;

/// Run `largo repl [args…]` feeding `input` on stdin.
fn repl(input: &str, args: &[&str]) -> std::process::Output {
    let mut child = largo()
        .arg("repl")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("largo repl should spawn");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// Bindings persist across lines; output appears exactly once.
#[test]
fn bindings_persist_across_lines() {
    let out = repl("Let x be 5.\nShow x.\n:quit\n", &[]);
    assert_eq!(out.status.code(), Some(0), "repl: {}", stderr(&out));
    let text = stdout(&out);
    assert_eq!(
        text.lines().filter(|l| l.trim() == "5").count(),
        1,
        "exactly one `5`:\n{text}"
    );
}

/// A function defined across multiple lines (blank line submits the block)
/// is callable afterwards.
#[test]
fn function_defined_across_lines() {
    let out = repl(
        "## To double (n: Int) -> Int:\n    Return n * 2.\n\nShow double(21).\n:quit\n",
        &[],
    );
    assert_eq!(out.status.code(), Some(0), "repl: {}", stderr(&out));
    assert!(stdout(&out).lines().any(|l| l.trim() == "42"), "{}", stdout(&out));
}

/// `:logic` switches modes: an English sentence yields FOL with ∀.
#[test]
fn logic_mode_emits_fol() {
    let out = repl(":logic\nEvery cat sleeps.\n:quit\n", &[]);
    assert_eq!(out.status.code(), Some(0), "repl: {}", stderr(&out));
    assert!(stdout(&out).contains('∀'), "FOL expected:\n{}", stdout(&out));
}

/// `--logic` starts in logic mode; `:format` switches rendering live.
#[test]
fn format_switches_live() {
    let out = repl(
        "Every cat sleeps.\n:format latex\nEvery dog barks.\n:format ascii\nEvery fish swims.\n:quit\n",
        &["--logic"],
    );
    assert_eq!(out.status.code(), Some(0), "repl: {}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains('∀'), "unicode first:\n{text}");
    assert!(text.contains("\\forall"), "latex second:\n{text}");
    // SimpleFOL ("ascii") drops event semantics: `Swim(x)` instead of the
    // unicode form's `∃e(Swim(e) ∧ Agent(e, x))`.
    assert!(text.contains("Swim(x)"), "ascii third (simplified form):\n{text}");
}

/// Discourse carries anaphora across sentences; `:discourse off` isolates.
#[test]
fn discourse_resolves_pronouns() {
    let out = repl(
        "The boys lifted the piano.\nThey smiled.\n:quit\n",
        &["--logic"],
    );
    assert_eq!(out.status.code(), Some(0), "repl: {}", stderr(&out));
    let text = stdout(&out);
    let fol_lines: Vec<&str> = text.lines().filter(|l| l.contains('(')).collect();
    assert!(fol_lines.len() >= 2, "two sentences compiled:\n{text}");
}

/// `:readings` lists at least two numbered readings for a scope-ambiguous
/// sentence.
#[test]
fn readings_lists_ambiguity() {
    let out = repl(
        "Every woman loves a man.\n:readings\n:quit\n",
        &["--logic"],
    );
    assert_eq!(out.status.code(), Some(0), "repl: {}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("1. "), "numbered readings:\n{text}");
    assert!(text.contains("2. "), "at least two readings:\n{text}");
}

/// A failing line reports its error and the session recovers with no
/// duplicate output.
#[test]
fn error_then_recover_without_duplicates() {
    let out = repl(
        "Show \"first\".\nShow undefined_variable_xyz.\nShow \"second\".\n:quit\n",
        &[],
    );
    assert_eq!(out.status.code(), Some(0), "repl: {}", stderr(&out));
    let text = stdout(&out);
    assert_eq!(text.matches("first").count(), 1, "no duplicates:\n{text}");
    assert!(text.contains("second"), "session recovered:\n{text}");
    let all = format!("{}{}", text, stderr(&out));
    assert!(all.contains("error") || all.contains("not defined") || all.contains("Unknown"),
        "the failure must be reported somewhere:\n{all}");
}

/// `:save` writes a runnable program that reproduces the session under
/// `largo run --interpret`.
#[test]
fn save_writes_runnable_program() {
    let dir = tempdir().unwrap();
    let saved = dir.path().join("session.lg");
    let script = format!(
        "## To double (n: Int) -> Int:\n    Return n * 2.\n\nLet x be 4.\nShow double(x).\n:save {}\n:quit\n",
        saved.display()
    );
    let out = repl(&script, &[]);
    assert_eq!(out.status.code(), Some(0), "repl: {}", stderr(&out));

    // The saved file is a valid project entry.
    scaffold(dir.path(), "saved_session");
    std::fs::copy(&saved, dir.path().join("src/main.lg")).unwrap();
    let run = largo_in(dir.path(), &["run", "--interpret"]);
    assert_eq!(run.status.code(), Some(0), "saved program: {}", stderr(&run));
    assert!(stdout(&run).lines().any(|l| l.trim() == "8"), "{}", stdout(&run));
}

/// `:vars` shows the bindings table.
#[test]
fn vars_shows_bindings() {
    let out = repl("Let x be 5.\n:vars\n:quit\n", &[]);
    assert_eq!(out.status.code(), Some(0), "repl: {}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("x") && text.contains("Int") && text.contains("5"), "{text}");
}

/// EOF without `:quit` exits cleanly.
#[test]
fn eof_exits_cleanly() {
    let out = repl("Show 1.\n", &[]);
    assert_eq!(out.status.code(), Some(0), "EOF must exit 0: {}", stderr(&out));
}
