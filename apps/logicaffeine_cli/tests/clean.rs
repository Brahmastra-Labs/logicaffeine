//! `largo clean` — remove build artifacts.

mod common;

use common::*;
use tempfile::tempdir;

/// `largo clean` removes the project's `target/` directory.
#[test]
fn clean_removes_target_dir() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "clean_proj");
    // Produce a target/ quickly via the built-in wasm backend (no cargo).
    let build = largo_in(dir.path(), &["build", "--emit", "wasm"]);
    assert_eq!(build.status.code(), Some(0), "wasm build: {}", stderr(&build));
    assert!(dir.path().join("target").exists(), "build must create target/");

    let out = largo_in(dir.path(), &["clean"]);
    assert_eq!(out.status.code(), Some(0), "clean: {}", stderr(&out));
    assert!(!dir.path().join("target").exists(), "clean must remove target/");
}

/// A second `largo clean` with nothing to remove still succeeds.
#[test]
fn clean_is_idempotent() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "clean_twice");
    let first = largo_in(dir.path(), &["clean"]);
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    let second = largo_in(dir.path(), &["clean"]);
    assert_eq!(second.status.code(), Some(0), "{}", stderr(&second));
}

/// `largo clean --all` also removes the `.logos-native/` bundle cache.
#[test]
fn clean_all_removes_native_bundle_cache() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "clean_all");
    std::fs::create_dir_all(dir.path().join(".logos-native")).unwrap();
    std::fs::write(dir.path().join(".logos-native/cached.so"), b"x").unwrap();

    let plain = largo_in(dir.path(), &["clean"]);
    assert_eq!(plain.status.code(), Some(0), "{}", stderr(&plain));
    assert!(
        dir.path().join(".logos-native").exists(),
        "plain clean must NOT touch .logos-native/"
    );

    let all = largo_in(dir.path(), &["clean", "--all"]);
    assert_eq!(all.status.code(), Some(0), "{}", stderr(&all));
    assert!(
        !dir.path().join(".logos-native").exists(),
        "clean --all must remove .logos-native/"
    );
}

/// Outside a project, `largo clean` fails with the standard project error.
#[test]
fn clean_outside_project_fails() {
    let dir = tempdir().unwrap();
    let out = largo_in(dir.path(), &["clean"]);
    assert_eq!(out.status.code(), Some(1));
    let err = strip_ansi(&stderr(&out));
    assert!(err.contains("not in a LOGOS project"), "stderr:\n{err}");
}
