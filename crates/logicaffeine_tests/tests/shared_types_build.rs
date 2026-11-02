//! `largo build` of programs whose generated Rust names crates directly — a `Shared` struct derives
//! `serde::Serialize`, so the generated project must declare `serde` or it cannot compile. The test
//! harness's own projects always carried `serde`, which is how a `Shared` program could pass every
//! differential test and still fail to build for a user (the Studio's Shared Map example did).
//! Real cargo builds: slow by nature, and the only honest proof.

#[path = "cli_common/mod.rs"]
mod common;

use common::*;
use tempfile::tempdir;

fn project_with(name: &str, source: &str) -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), name);
    std::fs::write(dir.path().join("src/main.lg"), source).unwrap();
    dir
}

#[test]
fn a_shared_map_program_builds_and_runs() {
    let dir = project_with(
        "shared_map",
        "# Shared Map\n\n## Definition\nA Warehouse is Shared and has:\n    stock: a SharedMap from Text to Int.\n\n\
         ## Main\nLet mutable w be a new Warehouse.\nSet w's stock[\"wood\"] to 50.\nSet w's stock[\"stone\"] to 30.\n\
         Show w's stock[\"wood\"].\nShow w's stock.\n",
    );
    let out = largo_in(dir.path(), &["run", "-q"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "largo run failed:\n{}",
        stderr(&out)
    );
    assert_eq!(stdout(&out).trim(), "50\n{stone: 30, wood: 50}");
}

#[test]
fn a_shared_counter_program_builds_and_runs() {
    let dir = project_with(
        "shared_counter",
        "## Definition\nA Clicks is Shared and has:\n    total: ConvergentCount.\n\n\
         ## Main\nLet mutable a be a new Clicks.\nIncrease a's total by 2.\nLet mutable b be a new Clicks.\n\
         Increase b's total by 3.\nMerge b into a.\nShow a's total.\n",
    );
    let out = largo_in(dir.path(), &["run", "-q"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "largo run failed:\n{}",
        stderr(&out)
    );
    assert_eq!(stdout(&out).trim(), "5");
}

#[test]
fn a_requires_serde_is_not_declared_twice() {
    let dir = project_with(
        "requires_serde",
        "## Requires\n    The \"serde\" crate version \"1\" with features \"derive\".\n\n\
         ## Definition\nA Clicks is Shared and has:\n    total: ConvergentCount.\n\n\
         ## Main\nLet mutable a be a new Clicks.\nIncrease a's total by 4.\nShow a's total.\n",
    );
    let out = largo_in(dir.path(), &["run", "-q"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "largo run failed:\n{}",
        stderr(&out)
    );
    assert_eq!(stdout(&out).trim(), "4");
}
