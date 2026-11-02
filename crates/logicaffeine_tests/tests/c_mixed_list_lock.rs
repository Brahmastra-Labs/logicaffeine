//! ★ LOCK — a list whose items differ in kind is the VM's list on C, or refused by name. ★
//!
//! The VM keeps every item of a list the value it is: in `[sqrt(2.0), 1, 7]` the second item is the `Int` 1, and
//! `item 3 of xs / 2` is the integer quotient 3. Both C backends hold a list in one element type, taken from its
//! first item, so a list of mixed kinds is one they cannot hold: the bytecode walker pushed an integer's bits as a
//! real (`[1.4142135623730951, 0.0000…005, …]`) and the AST generator widened the integers, dividing 7 by 2 as
//! reals. Each refuses such a list at generation, naming it, and keeps every list of one kind as it was.

#![cfg(not(target_arch = "wasm32"))]

#[path = "tier_parity/mod.rs"]
mod tier_parity;

use tier_parity::{Backend, Program, Verdict};

/// Lists whose items differ in kind.
const MIXED: &[(&str, &str)] = &[
    ("a_real_then_integers", "## Main\nLet xs be [sqrt(2.0), 1, 7].\nShow xs.\nShow item 3 of xs / 2.\n"),
    ("an_integer_then_a_real", "## Main\nLet xs be [1, 2.5].\nShow xs.\n"),
    ("a_boolean_then_an_integer", "## Main\nLet xs be [true, 1].\nShow xs.\n"),
    ("an_integer_then_text", "## Main\nLet xs be [1, \"a\"].\nShow xs.\n"),
    ("text_then_an_integer", "## Main\nLet xs be [\"a\", 1].\nShow xs.\n"),
];

/// Lists of one kind, which both backends keep running as the VM does.
const UNIFORM: &[(&str, &str)] = &[
    ("integers", "## Main\nLet xs be [3, 1, 2].\nShow xs.\nShow item 3 of xs / 2.\n"),
    ("reals", "## Main\nLet xs be [1.5, 2.25, sqrt(2.0)].\nShow xs.\n"),
    ("texts", "## Main\nLet xs be [\"a\", \"b\"].\nShow xs.\n"),
    ("booleans", "## Main\nLet xs be [true, false].\nShow xs.\n"),
    ("computed_integers", "## Main\nLet n be 4.\nLet xs be [n, n * n, n - 7].\nShow xs.\n"),
];

/// The child side of every isolated backend run. Does nothing unless `LOGOS_PARITY_JOB` is set.
#[test]
#[ignore = "the worker the parity sweep re-executes this binary into"]
fn tier_parity_worker() {
    tier_parity::worker_main();
}

fn programs(set: &[(&str, &str)]) -> Vec<Program> {
    set.iter().map(|(id, source)| Program::new(format!("list/{id}"), *source)).collect()
}

#[test]
fn every_program_here_reaches_the_backends() {
    let refused: Vec<&str> = MIXED
        .iter()
        .chain(UNIFORM)
        .filter(|(_, source)| tier_parity::rejection(source).is_some())
        .map(|(id, _)| *id)
        .collect();
    assert!(refused.is_empty(), "the front end refuses {refused:?}, which pin nothing");
}

#[test]
fn a_list_of_items_of_different_kinds_is_refused_by_name_never_misread() {
    let rows = tier_parity::sweep(&programs(MIXED), &[Backend::C, Backend::CBytecode], 4)
        .unwrap_or_else(|why| panic!("the sweep could not start: {why}"));
    let bad: Vec<String> = rows
        .iter()
        .filter(|row| row.backend != tier_parity::REFERENCE)
        .filter(|row| match &row.verdict {
            Verdict::Refused(why) => !why.contains("items differ in kind"),
            Verdict::Observed(_) => !row.matches(),
            _ => false,
        })
        .map(|row| format!("{} on {}: {} {}", row.program, row.backend.name(), row.status(), row.detail()))
        .collect();
    assert!(bad.is_empty(), "{} runs neither match the VM nor refuse by name:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn a_list_of_one_kind_runs_on_both_c_backends_as_on_the_vm() {
    let rows = tier_parity::sweep(&programs(UNIFORM), &[Backend::TreeWalker, Backend::C, Backend::CBytecode], 4)
        .unwrap_or_else(|why| panic!("the sweep could not start: {why}"));
    let bad: Vec<String> = rows
        .iter()
        .filter(|row| !row.matches())
        .map(|row| format!("{} on {}: {} {}", row.program, row.backend.name(), row.status(), row.detail()))
        .collect();
    assert!(bad.is_empty(), "{} runs differ from the VM:\n{}", bad.len(), bad.join("\n"));
}
