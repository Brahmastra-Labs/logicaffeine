//! ★ LOCK — a program that mounts a journal or writes a file builds for the browser and fails there by name. ★
//!
//! The tree-walker and the VM run `Mount … at …` and `Write … to file …` against the sandboxed virtual file
//! system. A page in the browser has none, so its module fails the turn with `VFS_IN_THE_BROWSER` at the
//! statement: the program still builds (the guide's Persistent Counter and Persistent + Synced examples stay in
//! `wasm_aot_coverage`), what ran before the statement has already shown, and nothing after it runs.

#![cfg(feature = "wasm-jit")]

#[path = "compile_common/mod.rs"]
mod common;

use common::wasm_host::run_outcome;
use logicaffeine_compile::vm::wasm::VFS_IN_THE_BROWSER;

#[test]
fn a_mount_builds_for_the_browser_and_fails_naming_the_missing_file_system() {
    let src = "## Definition\nA Counter is Shared and has:\n    a value, which is ConvergentCount.\n\n## Main\nShow \"before\".\nLet mutable c: Persistent Counter be mounted at \"counter.lsf\".\nIncrease c's value by 1.\nShow c's value.\n";
    let run = run_outcome(src).unwrap_or_else(|e| {
        panic!("the program builds for the browser and reports its failure: {e}")
    });
    assert_eq!(
        run.output.trim(),
        "before",
        "what ran before the mount has shown, and nothing after it"
    );
    assert_eq!(
        run.failure.as_deref(),
        Some(VFS_IN_THE_BROWSER),
        "the page names what it is missing"
    );
}

#[test]
fn a_file_write_builds_for_the_browser_and_fails_naming_the_missing_file_system() {
    let src = "## Main\n    Show \"before\".\n    Write \"x\" to file \"out.txt\".\n    Show \"after\".\n";
    let run = run_outcome(src).unwrap_or_else(|e| {
        panic!("the program builds for the browser and reports its failure: {e}")
    });
    assert_eq!(
        run.output.trim(),
        "before",
        "what ran before the write has shown, and nothing after it"
    );
    assert_eq!(
        run.failure.as_deref(),
        Some(VFS_IN_THE_BROWSER),
        "the page names what it is missing"
    );
}
