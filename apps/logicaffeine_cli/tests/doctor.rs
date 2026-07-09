//! `largo doctor` — environment diagnostics.
//!
//! Contract: network trouble is a warning, never a failure (doctor must be
//! useful offline); a broken project is a hard failure; output is a ✓/!/✗
//! report with no ANSI on pipes.

mod common;

use common::*;
use tempfile::tempdir;

/// On a healthy dev box, doctor reports and exits 0, mentioning the
/// toolchain and node checks.
#[test]
fn doctor_reports_dev_environment() {
    let dir = tempdir().unwrap();
    let out = largo_in(dir.path(), &["doctor"]);
    assert_eq!(out.status.code(), Some(0), "doctor: {}", stderr(&out));
    let text = strip_ansi(&stdout(&out));
    assert!(text.contains("cargo"), "toolchain line expected:\n{text}");
    assert!(text.contains("node"), "node line expected:\n{text}");
    assert!(text.contains("registry"), "registry line expected:\n{text}");
}

/// An unreachable registry is a warning — doctor still exits 0.
#[test]
fn doctor_unreachable_registry_is_warn_not_fail() {
    let dir = tempdir().unwrap();
    let out = largo_in(
        dir.path(),
        &["doctor", "--registry", "http://127.0.0.1:1"],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "network trouble must not fail doctor: {}",
        stderr(&out)
    );
    let text = strip_ansi(&stdout(&out));
    assert!(
        text.contains('!') || text.to_lowercase().contains("unreachable"),
        "must warn about the registry:\n{text}"
    );
}

/// Inside a project with a corrupt manifest, doctor fails with a ✗ line.
#[test]
fn doctor_broken_project_fails() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("Largo.toml"), "this is [not toml").unwrap();
    let out = largo_in(dir.path(), &["doctor"]);
    assert_eq!(out.status.code(), Some(1), "broken manifest must fail doctor");
    let text = strip_ansi(&stdout(&out));
    assert!(text.contains('✗'), "must show the failing check:\n{text}");
}

/// A healthy project gets a ✓ project line.
#[test]
fn doctor_healthy_project_passes() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "doc_proj");
    let out = largo_in(dir.path(), &["doctor"]);
    assert_eq!(out.status.code(), Some(0), "doctor: {}", stderr(&out));
    let text = strip_ansi(&stdout(&out));
    assert!(text.contains("project"), "project line expected:\n{text}");
}

/// Piped doctor output carries no ANSI by default.
#[test]
fn doctor_piped_output_has_no_ansi() {
    let dir = tempdir().unwrap();
    let out = largo_in(dir.path(), &["doctor"]);
    assert!(!has_ansi(&stdout(&out)), "no ANSI on pipes:\n{:?}", stdout(&out));
}
