//! ★ LOCK — recursion unrolled into its caller writes through an in-out parameter at every level. ★
//!
//! The unfold pass inlines a recursive function into itself a few levels deep, binding each argument to a fresh name so
//! it is evaluated once. A `mutable` parameter is not a value to copy: it is the caller's variable, lent in-out, and it
//! is also how a compiled tier hands a function the program's bindings. Bound to a copy, every write an unrolled level
//! made was lost: a search that counted its calls in a program variable printed 11 where the tree-walker printed 191.
//! The unrolled copy now names the caller's variable itself. Each row runs on the tree-walker, the VM, the optimized run
//! path, the Rust backend and both C backends, where an in-out parameter is a pointer to the caller's variable.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test in_out_unrolling_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::{assert_c_output, assert_exact_output, build_c_release};
use logicaffeine_compile::compile::{
    compile_to_c_via_bytecode, tw_outcome, vm_outcome, vm_outcome_tiered,
};
use logicaffeine_compile::optimization::Tier;

/// `src` prints what the tree-walker prints on the VM, the optimized run path, the Rust backend and both C backends.
fn every_aot_tier(src: &str) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker refused:\n{src}");
    let expected = tw.output.trim_end();
    let vm = vm_outcome(src);
    assert_eq!(vm.output.trim_end(), expected, "the VM's answer:\n{src}");
    let run = vm_outcome_tiered(src, &[], Tier::T3, None);
    assert_eq!(
        run.output.trim_end(),
        expected,
        "the optimized run path's answer:\n{src}"
    );
    assert_exact_output(src, expected);
    assert_c_output(src, expected);
    let c = compile_to_c_via_bytecode(src)
        .unwrap_or_else(|e| panic!("the bytecode C generator refused: {e:?}\n{src}"));
    let dir = tempfile::tempdir().expect("temp dir");
    let (c_path, bin) = (dir.path().join("main.c"), dir.path().join("main"));
    std::fs::write(&c_path, &c).expect("write the C");
    let built = build_c_release(&c_path, &bin).expect("run the C compiler");
    assert!(
        built.status.success(),
        "the bytecode C does not compile:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let run = std::process::Command::new(&bin)
        .output()
        .expect("run the bytecode C");
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim_end(),
        expected,
        "the bytecode C's answer:\n{src}"
    );
}

#[test]
fn an_unrolled_search_counts_in_a_program_variable_at_every_level() {
    every_aot_tier(
        "## To tick (by: Int) -> Int:
    Set visits to visits + by.
    Return 0.

## To solve (row: Int, cols: Int, d1: Int, d2: Int, n: Int) -> Int:
    If row equals n:
        Return 1.
    Let z be tick(1).
    Let all be (1 shifted left by n) - 1.
    Let mutable available be all & ~(cols | d1 | d2).
    Let mutable count be 0.
    While available is not 0:
        Let bit be available & (0 - available).
        Set available to available xor bit.
        Set count to count + solve(row + 1, cols | bit, (d1 | bit) shifted left by 1, (d2 | bit) shifted right by 1, n).
    Return count.

## Main
Let mutable visits be 0.
Show solve(0, 0, 0, 0, 6).
Show visits.
",
    );
}

#[test]
fn an_unrolled_search_writes_through_a_declared_in_out_parameter() {
    every_aot_tier(
        "## To walk (seen: mutable Int, row: Int, n: Int) -> Int:
    If row equals n:
        Return 1.
    Set seen to seen + 1.
    Let mutable count be 0.
    Let mutable k be 0.
    While k is less than 3:
        Set count to count + walk(seen, row + 1, n).
        Set k to k + 1.
    Return count.

## Main
Let mutable seen be 0.
Show walk(seen, 0, 5).
Show seen.
",
    );
}
