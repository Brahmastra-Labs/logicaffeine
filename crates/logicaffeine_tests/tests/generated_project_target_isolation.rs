//! A GENERATED PROJECT BUILDS INTO ITS OWN TARGET, whatever the caller's `CARGO_TARGET_DIR` says.
//!
//! Every project `compile_to_dir` writes is the package `logos_output`. `compile_and_run` and
//! `rustc_check` ran cargo with the caller's environment, so under an exported `CARGO_TARGET_DIR`
//! every generated project built into ONE `target/debug/logos_output`: a program written while
//! another was building found that one's fingerprint fresh, and `cargo run` started whichever
//! program had linked last. Three `phase85_mixed_link` rows run side by side all ran the same
//! program and failed on its assertion. Their siblings in `compile.rs` (the AOT cdylib, the wasm
//! bundle, the native PE) already pin their own target; these two doors now do too.
//!
//! Each test sets `CARGO_TARGET_DIR` for its OWN process (nextest runs every test in a process of
//! its own) to one directory every thread shares, and then asks for several different programs at
//! once. Each must come back with its own answer.

use logicaffeine_compile::compile::{compile_and_run, rustc_check};
use std::path::PathBuf;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "logos_target_isolation_{tag}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// Point every cargo this process spawns at one shared target, the way an exported
/// `CARGO_TARGET_DIR` does.
fn share_one_target(root: &std::path::Path) {
    let shared = root.join("shared-target");
    std::fs::create_dir_all(&shared).expect("the shared target");
    std::env::set_var("CARGO_TARGET_DIR", &shared);
}

#[test]
fn programs_run_at_once_under_one_shared_target_each_run_themselves() {
    let root = scratch("run");
    share_one_target(&root);
    let words = ["alpha", "bravo", "charlie", "delta"];
    let handles: Vec<_> = words
        .iter()
        .map(|word| {
            let dir = root.join(word);
            let word = word.to_string();
            std::thread::spawn(move || {
                let program = format!("## Main\nShow \"{word}\".\n");
                (word, compile_and_run(&program, &dir))
            })
        })
        .collect();
    for handle in handles {
        let (word, ran) = handle.join().expect("the thread");
        let out = ran.unwrap_or_else(|e| panic!("`Show \"{word}\"` must compile and run: {e:?}"));
        assert_eq!(
            out.trim(),
            word,
            "the program printing {word:?} ran another program"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// The same for the deep check: an ill-typed program checked beside a clean one keeps its finding,
/// and the clean one is not handed the other's.
#[test]
fn programs_checked_at_once_under_one_shared_target_each_get_their_own_findings() {
    let root = scratch("check");
    share_one_target(&root);
    let clean = "## To consume (v: Int):\n    Show v.\n\n## Main\nLet a be 5.\nLet c be 1.\nGive a to consume.\nShow c.\n";
    let ill = "## Main\nLet a be 5.\nLet b be 1.\nGive a to b.\nShow b.\n";
    let handles: Vec<_> = [
        ("clean-1", clean),
        ("ill-1", ill),
        ("clean-2", clean),
        ("ill-2", ill),
    ]
    .into_iter()
    .map(|(tag, program)| {
        let dir = root.join(tag);
        std::thread::spawn(move || (tag, rustc_check(program, &dir)))
    })
    .collect();
    for handle in handles {
        let (tag, checked) = handle.join().expect("the thread");
        let refused = match &checked {
            Ok(findings) => !findings.is_empty(),
            Err(_) => true,
        };
        if tag.starts_with("clean") {
            assert!(!refused, "{tag}: a clean program checks clean: {checked:?}");
        } else {
            assert!(
                refused,
                "{tag}: `Give a to b` to an Int is refused by rustc: {checked:?}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}
