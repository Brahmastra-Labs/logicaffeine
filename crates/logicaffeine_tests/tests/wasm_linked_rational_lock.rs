//! ★ LOCK — a linked module orders, equates and accumulates rationals as the VM does. ★
//!
//! A linked module holds a `Rational` as a handle into base's own `Rational`, so it compares one exactly through
//! the runtime, against another rational or against an integer, and adds onto one in place (`Set q to q + …`), as
//! the VM does: never refused, never computed on a rounded float.

#![cfg(not(target_arch = "wasm32"))]

#[path = "tier_parity/mod.rs"]
mod tier_parity;

use tier_parity::{Backend, Program};

const PROGRAMS: &[(&str, &str)] = &[
    (
        "two_rationals",
        "## Main\nLet q: Rational be 1 / 3.\nLet p: Rational be 1 / 2.\nShow q < p.\nShow q > p.\nShow q <= q.\nShow q >= p.\nShow q = 1 / 3.\nShow q != p.\n",
    ),
    (
        "a_rational_beside_an_integer",
        "## Main\nLet q: Rational be 7 / 2.\nShow q > 3.\nShow q < 4.\nShow 3 < q.\nShow 4 <= q.\nShow q = 3.\nShow q != 4.\n",
    ),
    (
        "a_rational_decides_a_branch",
        "## Main\nLet q: Rational be 1 / 3.\nIf q < 1 / 2:\n    Show \"less\".\nOtherwise:\n    Show \"more\".\n",
    ),
    (
        "a_whole_rational_equals_its_integer",
        "## Main\nLet q: Rational be 6 / 3.\nShow q = 2.\nShow q < 2.\nShow q >= 2.\n",
    ),
    (
        "a_rational_accumulates_in_place",
        "## Main\nLet mutable total: Rational be 1 / 2.\nRepeat for i from 1 to 4:\n    Set total to total + 1 / 3.\nShow total.\nShow total > 1.\n",
    ),
    (
        "a_rational_loop_bound",
        "## Main\nLet mutable q: Rational be 1 / 3.\nLet mutable n be 0.\nWhile q < 2:\n    Set q to q + 1 / 3.\n    Set n to n + 1.\nShow n.\nShow q.\n",
    ),
];

/// The child side of every isolated backend run. Does nothing unless `LOGOS_PARITY_JOB` is set.
#[test]
#[ignore = "the worker the parity sweep re-executes this binary into"]
fn tier_parity_worker() {
    tier_parity::worker_main();
}

#[test]
fn every_program_here_reaches_the_backends() {
    let refused: Vec<&str> = PROGRAMS
        .iter()
        .filter(|(_, source)| tier_parity::rejection(source).is_some())
        .map(|(id, _)| *id)
        .collect();
    assert!(refused.is_empty(), "the front end refuses {refused:?}, which pin nothing");
}

#[test]
fn a_linked_module_computes_with_rationals_as_the_vm_does() {
    let programs: Vec<Program> = PROGRAMS
        .iter()
        .map(|(id, source)| Program::new(format!("linked-rational/{id}"), *source))
        .collect();
    let rows = tier_parity::sweep(&programs, &[Backend::TreeWalker, Backend::WasmLinked], 4)
        .unwrap_or_else(|why| panic!("the sweep could not start: {why}"));
    let bad: Vec<String> = rows
        .iter()
        .filter(|row| !row.matches())
        .map(|row| format!("{} on {}: {} {}", row.program, row.backend.name(), row.status(), row.detail()))
        .collect();
    assert!(bad.is_empty(), "{} runs differ from the VM:\n{}", bad.len(), bad.join("\n"));
}
