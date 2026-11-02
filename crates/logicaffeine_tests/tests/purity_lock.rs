//! ★ LOCK — a function that changes a program variable is not pure, on any path that asks. ★
//!
//! Memoization answers a repeated call from a table, and the reflection-symmetry rewrite runs half a search and doubles
//! its count; both are sound only for a function whose call does nothing but compute its result. A function that sets,
//! pushes to or otherwise changes a variable of the program (directly, or through a function it calls) has an effect
//! each call must perform, so neither may touch it. Each row runs on the tree-walker for the reference, then on the VM,
//! the optimized run path, the Rust backend and both C backends.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test purity_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::{assert_c_output, assert_exact_output, build_c_release};
use logicaffeine_compile::compile::{
    compile_to_c_via_bytecode, tw_outcome, vm_outcome, vm_outcome_tiered,
};
use logicaffeine_compile::optimization::Tier;

fn every_tier(src: &str) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker refused:\n{src}");
    let expected = tw.output.trim_end();
    assert_eq!(
        vm_outcome(src).output.trim_end(),
        expected,
        "the VM's answer:\n{src}"
    );
    assert_eq!(
        vm_outcome_tiered(src, &[], Tier::T3, None)
            .output
            .trim_end(),
        expected,
        "the run path's answer:\n{src}"
    );
    assert_exact_output(src, expected);
    assert_c_output(src, expected);
    let c = compile_to_c_via_bytecode(src)
        .unwrap_or_else(|e| panic!("the bytecode C generator refused: {e:?}"));
    let dir = tempfile::tempdir().expect("temp dir");
    let (c_path, bin) = (dir.path().join("main.c"), dir.path().join("main"));
    std::fs::write(&c_path, &c).expect("write the C");
    assert!(
        build_c_release(&c_path, &bin)
            .expect("run the C compiler")
            .status
            .success(),
        "the bytecode C compiles"
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
fn a_recursion_that_counts_its_calls_is_not_memoized() {
    every_tier(
        "## To fibc (n: Int) -> Int:
    Set calls to calls + 1.
    If n is less than 2:
        Return n.
    Return fibc(n - 1) + fibc(n - 2).

## Main
Let mutable calls be 0.
Show fibc(18).
Show calls.
",
    );
}

#[test]
fn a_recursion_whose_callee_counts_its_calls_is_not_memoized() {
    every_tier(
        "## To tick (k: Int) -> Int:
    Set calls to calls + k.
    Return 0.

## To fibt (n: Int) -> Int:
    Let z be tick(1).
    If n is less than 2:
        Return n.
    Return fibt(n - 1) + fibt(n - 2).

## Main
Let mutable calls be 0.
Show fibt(18).
Show calls.
",
    );
}

#[test]
fn a_recursion_that_records_its_arguments_is_not_memoized() {
    every_tier(
        "## To fibr (n: Int) -> Int:
    Push n to seen.
    If n is less than 2:
        Return n.
    Return fibr(n - 1) + fibr(n - 2).

## Main
Let mutable seen be a new Seq of Int.
Show fibr(15).
Show length of seen.
Show item 7 of seen.
",
    );
}

/// The reference is the tree-walker, which runs every call: a pure recursion small enough for it, which a backend that
/// memoizes still answers alike.
#[test]
fn a_pure_recursion_answers_alike() {
    every_tier(
        "## To fib (n: Int) -> Int:
    If n is less than 2:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Show fib(22).
",
    );
}
