//! `largo prove [FILE] [--trace] [--json]` — kernel-certified theorem proving.
//!
//! Fixtures mirror the engine's own proven corpus: the Tarski congruence
//! development (`## Theory`) and the Socrates syllogism (`## Theorem`).

mod common;

use common::*;
use tempfile::tempdir;

const TARSKI: &str = "\
## Theory Tarski

Axiom pseudo_reflexivity: for all a b, Cong(a, b, b, a).
Axiom inner_transitivity: for all a b c d e f, if Cong(a, b, c, d) and Cong(a, b, e, f) then Cong(c, d, e, f).

Theorem reflexivity: prove for all a b, Cong(a, b, a, b).
Theorem symmetry cites reflexivity: prove for all a b c d, if Cong(a, b, c, d) then Cong(c, d, a, b).
Theorem transitivity cites symmetry: prove for all a b c d e f, if Cong(a, b, c, d) and Cong(c, d, e, f) then Cong(a, b, e, f).
";

const WEAK: &str = "\
## Theory Weak

Axiom pseudo_reflexivity: for all a b, Cong(a, b, b, a).

Theorem reflexivity: prove for all a b, Cong(a, b, a, b).
";

const SOCRATES: &str = "\
## Theorem: Socrates
Given: Socrates is a man.
Given: Every man is mortal.
Prove: Socrates is mortal.
Proof: Auto.
";

fn write_file(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

/// A full Tarski development proves end to end: every theorem ✓, exit 0.
#[test]
fn tarski_theory_all_verified() {
    let dir = tempdir().unwrap();
    let file = write_file(dir.path(), "tarski.lg", TARSKI);
    let out = largo_in(dir.path(), &["prove", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "prove: {}", stderr(&out));
    let text = strip_ansi(&stdout(&out));
    for theorem in ["reflexivity", "symmetry", "transitivity"] {
        assert!(
            text.lines().any(|l| l.contains('✓') && l.contains(theorem)),
            "must show ✓ {theorem}:\n{text}"
        );
    }
}

/// An unfounded theorem is reported ✗ by name and the run exits 1.
#[test]
fn unfounded_theorem_fails_by_name() {
    let dir = tempdir().unwrap();
    let file = write_file(dir.path(), "weak.lg", WEAK);
    let out = largo_in(dir.path(), &["prove", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1), "unfounded theorem must fail the run");
    let text = strip_ansi(&stdout(&out));
    assert!(
        text.lines().any(|l| l.contains('✗') && l.contains("reflexivity")),
        "must show ✗ reflexivity:\n{text}"
    );
}

/// An English `## Theorem` block proves, and `--trace` shows the rendered
/// derivation tree with its inference rules.
#[test]
fn english_theorem_with_trace() {
    let dir = tempdir().unwrap();
    let file = write_file(dir.path(), "socrates.lg", SOCRATES);
    let out = largo_in(dir.path(), &["prove", file.to_str().unwrap(), "--trace"]);
    assert_eq!(out.status.code(), Some(0), "prove --trace: {}", stderr(&out));
    let text = strip_ansi(&stdout(&out));
    assert!(text.contains("Socrates"), "theorem name shown:\n{text}");
    assert!(text.contains("└─"), "derivation tree rendered:\n{text}");
    assert!(text.contains("ModusPonens"), "inference rule visible:\n{text}");
}

/// `--json` emits machine-readable results.
#[test]
fn json_output_is_machine_readable() {
    let dir = tempdir().unwrap();
    let file = write_file(dir.path(), "tarski.lg", TARSKI);
    let out = largo_in(dir.path(), &["prove", file.to_str().unwrap(), "--json"]);
    assert_eq!(out.status.code(), Some(0), "prove --json: {}", stderr(&out));
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("valid JSON");
    assert_eq!(v["all_verified"], true);
    let theorems = v["theorems"].as_array().expect("theorems array");
    assert_eq!(theorems.len(), 3);
    assert_eq!(theorems[0]["name"], "reflexivity");
    assert_eq!(theorems[0]["verified"], true);
}

/// Inside a project, `largo prove` defaults to the entry file.
#[test]
fn prove_defaults_to_project_entry() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "prove_proj");
    std::fs::write(dir.path().join("src/main.lg"), SOCRATES).unwrap();
    let out = largo_in(dir.path(), &["prove"]);
    assert_eq!(out.status.code(), Some(0), "prove (entry): {}", stderr(&out));
    assert!(strip_ansi(&stdout(&out)).contains("Socrates"));
}

/// A malformed development is a clean failure, not a panic.
#[test]
fn malformed_development_fails_cleanly() {
    let dir = tempdir().unwrap();
    let file = write_file(
        dir.path(),
        "bad.lg",
        "## Theory Bad\n\nAxiom broken: for all a, Cong(a.\n",
    );
    let out = largo_in(dir.path(), &["prove", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(!strip_ansi(&stderr(&out)).trim().is_empty(), "must explain the failure");
}

/// A file with nothing to prove is an error, not a silent success.
#[test]
fn nothing_to_prove_is_an_error() {
    let dir = tempdir().unwrap();
    let file = write_file(dir.path(), "empty.lg", "## Main\n    Show 1.\n");
    let out = largo_in(dir.path(), &["prove", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(strip_ansi(&stderr(&out)).contains("no"), "must say nothing was found");
}
