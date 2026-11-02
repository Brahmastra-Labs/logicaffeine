//! Import-hygiene lints — the pit-of-success guardrails: unused / duplicate /
//! self-import findings, plus a "did you mean" suggestion for a mistyped module.

#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::path::Path;

use tempfile::tempdir;

use logicaffeine_compile::analysis::{
    assemble_project, lint_imports, ImportLintKind, LintSeverity,
};

const UTIL: &str = "# Util\n\n## To double (n: Int) -> Int:\n    Return n times 2.\n";

fn assemble_err(entry: &Path) -> String {
    match assemble_project(entry) {
        Ok(_) => panic!("expected an assembly error"),
        Err(e) => e,
    }
}

/// An import whose names are never referenced is a WARNING.
#[test]
fn unused_import_warns() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("util.lg"), UTIL).unwrap();
    let main = dir.path().join("main.lg");
    fs::write(&main, "# App\n\nImport Util.\n\n## Main\nShow 42.\n").unwrap();

    let lints = lint_imports(&main).unwrap();
    assert!(
        lints.iter().any(|l| l.kind == ImportLintKind::Unused
            && l.severity == LintSeverity::Warning
            && l.module == "Util"),
        "an unused import must warn, got: {lints:?}"
    );
}

/// A USED import produces NO lint (the crucial no-false-positive lock).
#[test]
fn used_import_produces_no_lint() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("util.lg"), UTIL).unwrap();
    let main = dir.path().join("main.lg");
    fs::write(
        &main,
        "# App\n\nImport Util.\n\n## Main\nShow double(21).\n",
    )
    .unwrap();

    assert!(
        lint_imports(&main).unwrap().is_empty(),
        "a correctly-used import must produce no lint"
    );
}

/// The same module imported twice is a WARNING.
#[test]
fn duplicate_import_warns() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("util.lg"), UTIL).unwrap();
    let main = dir.path().join("main.lg");
    fs::write(
        &main,
        "# App\n\nImport Util.\nImport Helper from \"util.lg\".\n\n## Main\nShow double(21).\n",
    )
    .unwrap();

    let lints = lint_imports(&main).unwrap();
    assert!(
        lints.iter().any(|l| l.kind == ImportLintKind::Duplicate),
        "a duplicate import must warn, got: {lints:?}"
    );
}

/// A module importing itself is an ERROR.
#[test]
fn self_import_errors() {
    let dir = tempdir().unwrap();
    let main = dir.path().join("main.lg");
    fs::write(&main, "# App\n\nImport Main.\n\n## Main\nShow 42.\n").unwrap();

    let lints = lint_imports(&main).unwrap();
    assert!(
        lints
            .iter()
            .any(|l| l.kind == ImportLintKind::SelfImport && l.severity == LintSeverity::Error),
        "a self-import must error, got: {lints:?}"
    );
}

/// A mistyped module name suggests the closest real module.
#[test]
fn unknown_module_suggests_closest() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("geometry.lg"),
        "# Geometry\n\n## To area (n: Int) -> Int:\n    Return n.\n",
    )
    .unwrap();
    let main = dir.path().join("main.lg");
    fs::write(&main, "# App\n\nImport Geometrey.\n\n## Main\nShow 42.\n").unwrap();

    let err = assemble_err(&main);
    assert!(
        err.contains("Geometry"),
        "a mistyped module must suggest the closest name, got: {err}"
    );
}
