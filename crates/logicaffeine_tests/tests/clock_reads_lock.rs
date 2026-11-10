//! ★ LOCK — a clock read reads the clock afresh, on every tier. ★
//!
//! `now` is no variable: each read reads the clock when it runs. A pass that equates two reads, forwards a binding of
//! one to a later read, or moves one out of a loop changes what a program prints: `Let before be now.`, time passing,
//! then `Show now is greater than before.` is true, whatever pass took the two `now`s for one name. These programs run
//! on the real clock with time passing between the reads — `wait` reads the clock until two milliseconds have gone by,
//! which no tier can fold away — and print the tree-walker's answer on every in-process tier, in the generated Rust,
//! in C and on RISC-V, each reading the system clock (no `LOGOS_FIXED_*` pins it here).
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test clock_reads_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

#[path = "support/riscv_runner.rs"]
mod riscv_runner;

use common::{assert_all_tiers, assert_c_output, assert_exact_output};

/// A function that returns once the clock has moved two milliseconds past its first read (or after a bound of spins,
/// so a tier that read the clock once would still finish, and fail on what it prints).
const WAIT: &str = "## To wait -> Int:
    Let until be now + 2 milliseconds.
    Let mutable spins be 0.
    While now is less than until and spins is less than 5000000:
        Set spins to spins + 1.
    Return 0.
";

/// `main` after [`WAIT`] prints `printed` on every in-process tier, in the generated Rust, in C and on RISC-V.
fn assert_read_afresh(main: &str, printed: &str) {
    let source = format!("{WAIT}\n{main}");
    assert_all_tiers(&source, printed);
    assert_exact_output(&source, printed);
    assert_c_output(&source, printed);
    if common::require_tool("riscv64-linux-gnu-gcc") && common::require_tool("qemu-riscv64") {
        let asm = logicaffeine_compile::compile::compile_to_riscv(&source)
            .unwrap_or_else(|e| panic!("LOGOS→RISC-V compile error: {e:?}\n\nSource:\n{source}"));
        let run = riscv_runner::run(&riscv_runner::link(&asm, "clock_reads"), &[], &[]);
        assert!(run.success, "the RISC-V run failed: {}\n\nSource:\n{source}", run.describe());
        assert_eq!(run.stdout.trim(), printed, "RISC-V\n\nSource:\n{source}");
    }
}

#[test]
fn a_read_after_time_passes_is_later_than_one_bound_before() {
    assert_read_afresh(
        "## Main\nLet before be now.\nLet n be wait().\nShow now is greater than before.\nShow n.\n",
        "true\n0",
    );
}

#[test]
fn a_binding_holds_the_instant_its_read_read() {
    assert_read_afresh(
        "## Main\nLet a be now.\nLet n be wait().\nLet b be now.\nShow b is greater than a.\nShow a equals a.\nShow b equals a.\n",
        "true\ntrue\nfalse",
    );
}

#[test]
fn an_expression_over_a_read_is_computed_again_after_time_passes() {
    assert_read_afresh(
        "## Main\nLet x be now + 1 hours.\nLet n be wait().\nLet y be now + 1 hours.\nShow y is greater than x.\n",
        "true",
    );
}

#[test]
fn a_read_in_a_loop_reads_the_clock_each_time_round() {
    assert_read_afresh(
        "## Main\nLet mutable stamps be a new Seq of Moment.\nRepeat for i from 1 to 3:\n    Let n be wait().\n    Push now + 1 hours to stamps.\nShow item 2 of stamps is greater than item 1 of stamps.\nShow item 3 of stamps is greater than item 2 of stamps.\n",
        "true\ntrue",
    );
}

#[test]
fn a_function_returning_a_read_reads_at_each_call() {
    assert_read_afresh(
        "## To stamp -> Moment:\n    Return now.\n\n## Main\nLet a be stamp().\nLet n be wait().\nLet b be stamp().\nShow b is greater than a.\n",
        "true",
    );
}

#[test]
fn a_read_as_a_loop_condition_is_read_each_time_round() {
    assert_read_afresh(
        "## Main\nLet until be now + 3 milliseconds.\nLet mutable rounds be 0.\nWhile now is less than until and rounds is less than 5000000:\n    Set rounds to rounds + 1.\nShow now is at least until.\n",
        "true",
    );
}
