//! ★ LOCK — `--tiers` refuses a value it does not understand. ★
//!
//! `largo test --tiers all` turns on the cross-tier differential: every passing test's
//! computation is re-run on the bytecode VM and compared byte for byte. The flag was read as
//! `t == "all" || t.split(',').any(|x| x.trim() == "vm")` — so ANY other spelling was silently
//! ignored and the run went green with no differential at all.
//!
//! ★ THE FAILURE IS INVISIBLE AND IT POINTS THE WRONG WAY. `--tiers aot` is exactly what someone
//! reaches for after reading that the project has an AOT tier; it sets `all_tiers = false`, the
//! suite passes, and they believe a guard ran that never did. A typo (`--tiers vm,aot`,
//! `--tiers wasm`) does the same. The flag's whole purpose is to make a claim about coverage, so
//! accepting a word it does nothing with is the one behaviour it must not have.
//!
//! ★ THE SAME CLI ALREADY KNOWS THE RULE. `run.rs` refuses an unsatisfiable `## Require` and says
//! why in a comment — "fail loudly … rather than silently ignore it". Two commands, one binary,
//! opposite answers to the same question; this file makes them agree.
//!
//! ★ AND `vm` WAS UNDISCOVERABLE. The help text documented only `all`, while the code also
//! accepted `vm` — a spelling nobody could have found. The refusal names every value that works,
//! so the error teaches the flag.

#[path = "cli_common/mod.rs"]
mod common;

use common::{largo_in, stderr, stdout};

/// A project with one trivially passing test.
fn project(dir: &std::path::Path) -> std::path::PathBuf {
    let out = largo_in(dir, &["new", "site"]);
    assert!(out.status.success(), "scaffold: {}", stderr(&out));
    let root = dir.join("site");
    std::fs::write(
        root.join("src/main.lg"),
        // ★ THE FORM MATTERS AND IS EASY TO GET WRONG: a `## Test` body is INDENTED and the
        // assertion reads `Expect that … is equal to …`. `Expect 1 + 1 to be 2.` is not a form,
        // and its parse error talks about a missing `:` — which sends you looking at the header.
        "## Test: arithmetic still works\n    Expect that 1 + 1 is equal to 2.\n",
    )
    .unwrap();
    root
}

#[test]
fn an_unrecognised_tier_is_refused_and_names_what_works() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(dir.path());
    let out = largo_in(&root, &["test", "--tiers", "aot"]);
    assert!(
        !out.status.success(),
        "`--tiers aot` must fail rather than run with no differential and report success"
    );
    let said = format!("{}{}", stderr(&out), stdout(&out));
    assert!(
        said.contains("aot"),
        "the refusal names the value given: {said}"
    );
    assert!(
        said.contains("all") && said.contains("vm"),
        "and every value that does work, so the error teaches the flag: {said}"
    );
}

#[test]
fn a_typo_in_a_list_is_refused_rather_than_partially_honoured() {
    // The nastiest shape: `vm` is present so the differential DOES run, and the unknown word is
    // dropped in silence — so the run is green and the author believes `aot` was covered too.
    let dir = tempfile::tempdir().unwrap();
    let root = project(dir.path());
    let out = largo_in(&root, &["test", "--tiers", "vm,aot"]);
    assert!(
        !out.status.success(),
        "a list with an unknown member must be refused whole"
    );
    let said = format!("{}{}", stderr(&out), stdout(&out));
    assert!(
        said.contains("aot"),
        "naming the member that is not understood: {said}"
    );
}

#[test]
fn the_values_that_work_still_work() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(dir.path());
    for value in ["all", "vm", "interpreter", "interpreter,vm"] {
        let out = largo_in(&root, &["test", "--tiers", value]);
        assert!(
            out.status.success(),
            "`--tiers {value}` must still run: {}",
            stderr(&out)
        );
    }
}

#[test]
fn omitting_the_flag_is_still_the_default_single_tier_run() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(dir.path());
    let out = largo_in(&root, &["test"]);
    assert!(
        out.status.success(),
        "no flag, no differential, still green: {}",
        stderr(&out)
    );
}
