//! `largo build` cargo passthrough: phase headers, live-streamed cargo
//! output, and friendly failure classification.
//!
//! These are full-pipeline tests (real cargo builds of generated projects) —
//! slow by nature, and the only honest way to prove the terminal contract.

mod common;

use common::*;
use tempfile::tempdir;

/// A successful build shows largo's phase headers AND cargo's own compile
/// lines (proof that cargo's stderr streams through live instead of being
/// swallowed), ending with a `Finished` line.
#[test]
fn build_streams_cargo_output_and_phase_headers() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "stream_proj");

    let out = largo_in(dir.path(), &["build"]);
    assert_eq!(out.status.code(), Some(0), "build failed:\n{}", stderr(&out));

    let err = strip_ansi(&stderr(&out));
    assert!(
        err.contains("Compiling stream_proj"),
        "largo must print its own Compiling phase header:\n{err}"
    );
    assert!(
        err.contains("Compiling logicaffeine-data"),
        "cargo's own compile lines must stream through:\n{err}"
    );
    assert!(err.contains("Finished"), "must end with a Finished header:\n{err}");
}

/// An unresolvable `## Requires` crate is reported as a dependency problem
/// (not a wall of rustc noise), pointing back at the `## Requires` block.
#[test]
fn bad_requires_dependency_reports_friendly_error() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Largo.toml"),
        "[package]\nname = \"bad_requires\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/main.lg"),
        "# Main\n\n## Requires\n    The \"this-crate-does-not-exist-largo-e2e\" crate version \"99\".\n\n## Main\n\nShow 1.\n",
    )
    .unwrap();

    let out = largo_in(dir.path(), &["build"]);
    assert_eq!(out.status.code(), Some(1), "build must fail");
    let err = strip_ansi(&stderr(&out));
    assert!(
        err.contains("could not be resolved"),
        "must frame the failure as dependency resolution:\n{err}"
    );
    assert!(
        err.contains("## Requires"),
        "must point back at the `## Requires` block:\n{err}"
    );
}

/// `--quiet` suppresses largo's phase headers on a successful build.
#[test]
fn quiet_build_suppresses_phase_headers() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "quiet_build");

    let out = largo_in(dir.path(), &["build", "-q"]);
    assert_eq!(out.status.code(), Some(0), "build failed:\n{}", stderr(&out));
    let err = strip_ansi(&stderr(&out));
    assert!(
        !err.contains("Compiling quiet_build"),
        "-q must suppress largo's phase headers:\n{err}"
    );
    assert_eq!(stdout(&out), "", "-q build must print nothing on stdout");
}
