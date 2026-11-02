//! ★ LOCK — a loop over a set walks it on C as on the VM. ★
//!
//! `Repeat for x in s` over a `Set` binds each element once, in the order it was first added, as the
//! interpreter's set iterates and as its `Show` prints it. The direct C backends hold a set in an open-addressed
//! table with an insertion `order` array; a loop over one must read that array, never the table as if it were a
//! sequence. Each program runs on both C backends against the VM, the whole outcome compared.

#![cfg(not(target_arch = "wasm32"))]

#[path = "tier_parity/mod.rs"]
mod tier_parity;

use tier_parity::{Backend, Program};

const PROGRAMS: &[(&str, &str)] = &[
    (
        "ints_in_insertion_order",
        "## Main\nLet mutable s be a new Set of Int.\nAdd 5 to s.\nAdd 9 to s.\nAdd 5 to s.\nAdd 0 - 3 to s.\nRepeat for x in s:\n    Show x.\n",
    ),
    (
        "texts_in_insertion_order",
        "## Main\nLet mutable t be a new Set of Text.\nAdd \"b\" to t.\nAdd \"a\" to t.\nAdd \"b\" to t.\nRepeat for y in t:\n    Show y.\n",
    ),
    (
        "a_removed_element_is_skipped",
        "## Main\nLet mutable s be a new Set of Int.\nAdd 1 to s.\nAdd 2 to s.\nAdd 3 to s.\nRemove 2 from s.\nRepeat for x in s:\n    Show x.\n",
    ),
    (
        "a_sum_over_a_set",
        "## Main\nLet mutable s be a new Set of Int.\nRepeat for i from 1 to 100:\n    Add i % 7 to s.\nLet mutable total be 0.\nRepeat for x in s:\n    Set total to total + x.\nShow total.\nShow length of s.\n",
    ),
    (
        "an_empty_set_runs_no_pass",
        "## Main\nLet s be a new Set of Int.\nRepeat for x in s:\n    Show x.\nShow \"done\".\n",
    ),
];

/// A program the front end refuses fails alike on every backend: each program here must reach the backends.
/// The child side of every isolated backend run. Does nothing unless `LOGOS_PARITY_JOB` is set.
#[test]
#[ignore = "the worker the parity sweep re-executes this binary into"]
fn tier_parity_worker() {
    tier_parity::worker_main();
}

#[test]
fn every_set_program_reaches_the_backends() {
    let refused: Vec<&str> = PROGRAMS
        .iter()
        .filter(|(_, source)| tier_parity::rejection(source).is_some())
        .map(|(id, _)| *id)
        .collect();
    assert!(refused.is_empty(), "the front end refuses {refused:?}");
}

#[test]
fn a_loop_over_a_set_runs_on_both_c_backends_as_on_the_vm() {
    let programs: Vec<Program> = PROGRAMS
        .iter()
        .map(|(id, source)| Program::new(format!("set_iteration/{id}"), *source))
        .collect();
    let rows = tier_parity::sweep(
        &programs,
        &[Backend::TreeWalker, Backend::C, Backend::CBytecode],
        4,
    )
    .unwrap_or_else(|why| panic!("the sweep could not start: {why}"));
    let bad: Vec<String> = rows
        .iter()
        .filter(|row| !row.matches())
        .map(|row| {
            format!(
                "{} on {}: {} {}",
                row.program,
                row.backend.name(),
                row.status(),
                row.detail()
            )
        })
        .collect();
    assert!(
        bad.is_empty(),
        "{} (program, backend) runs differ from the VM:\n{}",
        bad.len(),
        bad.join("\n")
    );
}
