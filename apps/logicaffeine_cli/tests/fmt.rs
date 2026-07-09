//! `largo fmt [PATHS…] [--check]` — format LOGOS sources in place.

mod common;

use common::*;
use tempfile::tempdir;

const TABBY: &str = "# Main\n\n## Main\n\tLet x be 5.   \n\tShow x.\n";
const CLEAN: &str = "# Main\n\n## Main\n    Let x be 5.\n    Show x.\n";

/// `largo fmt` rewrites every project source file to the canonical style.
#[test]
fn fmt_rewrites_tabby_project_files() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "fmt_proj");
    std::fs::write(dir.path().join("src/main.lg"), TABBY).unwrap();
    std::fs::write(dir.path().join("src/helper.lg"), "## To help:\n\tShow 1.\n").unwrap();

    let out = largo_in(dir.path(), &["fmt"]);
    assert_eq!(out.status.code(), Some(0), "fmt: {}", stderr(&out));

    let main = std::fs::read_to_string(dir.path().join("src/main.lg")).unwrap();
    assert_eq!(main, CLEAN, "main.lg must be canonicalized");
    let helper = std::fs::read_to_string(dir.path().join("src/helper.lg")).unwrap();
    assert!(!helper.contains('\t'), "helper.lg must be formatted too");
}

/// `--check` lists dirty files, exits 1, and modifies NOTHING.
#[test]
fn fmt_check_reports_dirty_without_writing() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "fmt_check");
    std::fs::write(dir.path().join("src/main.lg"), TABBY).unwrap();

    let out = largo_in(dir.path(), &["fmt", "--check"]);
    assert_eq!(out.status.code(), Some(1), "dirty --check must exit 1");
    assert!(
        stdout(&out).contains("main.lg"),
        "must name the dirty file:\n{}",
        stdout(&out)
    );
    let content = std::fs::read_to_string(dir.path().join("src/main.lg")).unwrap();
    assert_eq!(content, TABBY, "--check must not modify files");
}

/// A clean project passes `--check` with exit 0.
#[test]
fn fmt_check_clean_project_passes() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "fmt_clean");
    std::fs::write(dir.path().join("src/main.lg"), CLEAN).unwrap();

    let out = largo_in(dir.path(), &["fmt", "--check"]);
    assert_eq!(out.status.code(), Some(0), "clean --check: {}", stderr(&out));
}

/// Formatting is a fixed point: a second run changes nothing.
#[test]
fn fmt_is_a_fixed_point() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "fmt_fixed");
    std::fs::write(dir.path().join("src/main.lg"), TABBY).unwrap();

    let first = largo_in(dir.path(), &["fmt"]);
    assert_eq!(first.status.code(), Some(0));
    let after_first = std::fs::read_to_string(dir.path().join("src/main.lg")).unwrap();

    let second = largo_in(dir.path(), &["fmt"]);
    assert_eq!(second.status.code(), Some(0));
    let after_second = std::fs::read_to_string(dir.path().join("src/main.lg")).unwrap();
    assert_eq!(after_first, after_second);

    let check = largo_in(dir.path(), &["fmt", "--check"]);
    assert_eq!(check.status.code(), Some(0), "formatted project must pass --check");
}

/// Explicit paths work outside any project.
#[test]
fn fmt_explicit_path_outside_project() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("loose.lg");
    std::fs::write(&file, "## Main\n\tShow 1.\n").unwrap();

    let out = largo_in(dir.path(), &["fmt", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "fmt FILE: {}", stderr(&out));
    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "## Main\n    Show 1.\n");
}

/// A missing explicit path is an error.
#[test]
fn fmt_missing_path_fails() {
    let dir = tempdir().unwrap();
    let out = largo_in(dir.path(), &["fmt", "no_such_file.lg"]);
    assert_eq!(out.status.code(), Some(1));
}
