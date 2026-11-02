//! ★ LOCK — `Project::shape()` and `Project::shape_or_best_effort()` must never disagree once the
//! entry actually parses.
//!
//! `shape_or_best_effort` exists ONLY to cover the case `shape()` refuses (a page that does not
//! parse — `largo serve --watch` starting on, or surviving, a broken save). It must be a pure
//! FALLBACK, never a second opinion: if it drifted from the strict decision on a project whose
//! entry parses fine, two `largo` commands could disagree about what the SAME project is, purely
//! because one called `shape()` and the other called `shape_or_best_effort()`.
//!
//! Swept over every REAL project this repo ships — the checked-in `examples/`, and what `largo
//! new`/`largo new --web`/`largo new --site` scaffold fresh — plus the synthetic edge case that
//! would split the two decisions if the lexical fallback were not as careful as the AST it
//! approximates: a bare `Serve` inside a `## Note` (documentation, never code) or inside a string
//! literal must not be read as a `Serve` STATEMENT by either decision.
//!
//! This file needs `CARGO_BIN_EXE_largo-under-test` (to scaffold the fresh projects with the real binary, the
//! same way an author would) — the exact reason `test_home_lock`'s rule 12 exception names for
//! keeping a new integration-test file in this crate rather than `crates/logicaffeine_tests`.

#[path = "cli_common/mod.rs"]
mod common;
use common::largo_in;

use std::path::{Path, PathBuf};

use logicaffeine_cli::project::shape::{Project, ProjectShape};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// Assert `shape_or_best_effort()` agrees with `shape()` for the project rooted at `root`. Panics
/// if the entry does NOT parse — this lock covers only the case both decisions have a real answer
/// to compare; a broken entry is `shape_or_best_effort`'s reason to exist, not something to agree
/// about (that direction is `serve_watch.rs`'s job).
fn assert_shape_agrees(root: &Path, label: &str) {
    let project = Project::load(root).unwrap_or_else(|e| panic!("{label}: loads: {e}"));
    let strict = project
        .shape()
        .unwrap_or_else(|e| panic!("{label}: entry does not parse ({e:?}) — fix the fixture, this lock only covers projects that DO parse"));
    assert_eq!(
        project.shape_or_best_effort(),
        strict,
        "{label}: shape_or_best_effort() disagreed with shape() on a project that PARSES — the \
         two decisions must never split when there is nothing to fall back from"
    );
}

#[test]
fn every_checked_in_example_agrees() {
    let examples = repo_root().join("examples");
    let mut checked = Vec::new();
    for entry in std::fs::read_dir(&examples).expect("examples/ exists") {
        let dir = entry.expect("dir entry").path();
        if !dir.is_dir() || !dir.join("Largo.toml").exists() {
            continue;
        }
        assert_shape_agrees(&dir, &dir.display().to_string());
        checked.push(dir);
    }
    assert!(
        checked.len() >= 5,
        "expected to check several examples/ projects (site, full-stack and program shapes), checked {}: {checked:?}",
        checked.len()
    );
}

#[test]
fn a_fresh_program_scaffold_agrees() {
    let dir = tempfile::tempdir().unwrap();
    let out = largo_in(dir.path(), &["new", "app"]);
    assert!(
        out.status.success(),
        "largo new: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_shape_agrees(&dir.path().join("app"), "largo new (plain program)");
}

#[test]
fn a_fresh_web_scaffold_agrees() {
    let dir = tempfile::tempdir().unwrap();
    let out = largo_in(dir.path(), &["new", "app", "--web"]);
    assert!(
        out.status.success(),
        "largo new --web: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_shape_agrees(&dir.path().join("app"), "largo new --web");
}

#[test]
fn a_fresh_site_scaffold_agrees() {
    let dir = tempfile::tempdir().unwrap();
    let out = largo_in(dir.path(), &["new", "app", "--site"]);
    assert!(
        out.status.success(),
        "largo new --site: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_shape_agrees(&dir.path().join("app"), "largo new --site");
}

/// ★ THE EDGE CASE THAT WOULD SPLIT THEM — `Serve` inside a `## Note` (documentation) and inside a
/// string literal, with NO real `Stmt::Serve` anywhere. A careless lexical scan reads the bare
/// word and misclassifies a plain program as a backend; the AST-based `shape()` never would.
#[test]
fn serve_inside_a_note_or_a_string_does_not_misroute_either_decision() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Largo.toml"), "[package]\nname = \"p\"\n").unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/main.lg"),
        "## Note\nServe is a word this program never calls as a statement.\n\n\
         ## Main\nShow \"Serve on port 8080 with handle.\".\n",
    )
    .unwrap();
    let project = Project::load(dir.path()).unwrap();
    let strict = project.shape().expect("this program parses cleanly");
    assert_eq!(
        strict,
        ProjectShape::Program,
        "no site, no REAL Serve statement"
    );
    assert_eq!(
        project.shape_or_best_effort(),
        ProjectShape::Program,
        "the lexical fallback must not be fooled by `Serve` in a Note or a string either"
    );
}
