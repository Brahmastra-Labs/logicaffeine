//! ★ LOCK — a value taken from a list computes as the VM's on every backend that compiles a program. ★
//!
//! `Pop from xs into v` gives `v` the last item, or nothing when `xs` is empty; the VM then computes with the value
//! `v` holds, and refuses an operator over nothing in its own words. A compiled backend that holds a popped value as
//! a present-or-nothing box reads the number it holds wherever a numeric operator takes it — in a sum, a product, a
//! quotient, an order test, an accumulation, in Main and in a function — and stops with the VM's sentence over
//! nothing. Each program reads its numbers at run time, and runs again folded.

#![cfg(not(target_arch = "wasm32"))]

#[path = "tier_parity/mod.rs"]
mod tier_parity;

use tier_parity::{Backend, Program};

const PROGRAMS: &[(&str, &str)] = &[
    (
        "an_integer_popped_in_a_function_and_in_main",
        "## To lastHalf (xs: Seq of Int) -> Int:\n    Let mutable ys be copy of xs.\n    Pop from ys into last.\n    Return last / 2.\n\n## Main\nShow lastHalf([parseInt(\"3\"), parseInt(\"15\")]).\nLet mutable zs be [parseInt(\"4\"), parseInt(\"9\")].\nPop from zs into top.\nShow top + 1.\nShow top * top - 1.\n",
    ),
    (
        "a_real_popped",
        "## Main\nLet mutable xs be [1.5, 2.25 * parseInt(\"2\")].\nPop from xs into top.\nShow top * 2.\nShow top + 1.\nShow top / 4.\n",
    ),
    (
        "compared_and_accumulated",
        "## Main\nLet mutable xs be [parseInt(\"1\"), parseInt(\"5\")].\nPop from xs into top.\nIf top is greater than 2:\n    Show \"big\".\nShow top is less than parseInt(\"3\").\nLet mutable total be parseInt(\"10\").\nPop from xs into a.\nSet total to total + a.\nShow total.\n",
    ),
    (
        "a_popped_nothing_refuses_a_sum",
        "## Main\nLet mutable zs be a new Seq of Int.\nPop from zs into top.\nShow top.\nShow \"before\".\nShow top + parseInt(\"1\").\n",
    ),
    (
        "a_popped_nothing_refuses_an_order_test",
        "## Main\nLet mutable zs be a new Seq of Int.\nPop from zs into top.\nShow \"before\".\nIf top is less than parseInt(\"3\"):\n    Show \"less\".\n",
    ),
    (
        "a_popped_nothing_refuses_a_real_product",
        "## Main\nLet mutable zs be a new Seq of Real.\nPop from zs into top.\nShow \"before\".\nShow top * parseInt(\"2\").\n",
    ),
    (
        "a_real_read_and_multiplied",
        "## Main\nLet xs be [1.5, 2.25 * parseInt(\"2\")].\nLet top be item 2 of xs.\nShow top * 2.\nLet y be 1.5 * parseInt(\"3\").\nShow y * 2.\n",
    ),
];

/// The backends that compile a program to run it: the engines, the generated Rust, both C lowerings, both
/// WebAssembly modules and RISC-V.
const BACKENDS: &[Backend] = &[
    Backend::TreeWalker,
    Backend::VmOptimized,
    Backend::VmForge,
    Backend::Interpret,
    Backend::RustDebug,
    Backend::RustRelease,
    Backend::C,
    Backend::CBytecode,
    Backend::Wasm,
    Backend::WasmLinked,
    Backend::RiscV,
];

/// The child side of every isolated backend run. Does nothing unless `LOGOS_PARITY_JOB` is set.
#[test]
#[ignore = "the worker the parity sweep re-executes this binary into"]
fn tier_parity_worker() {
    tier_parity::worker_main();
}

/// Declares what reads a program's numbers at run time, so that no tier folds them into constants.
const NATIVE: &str = "## To native parseInt (s: Text) -> Int\n\n";

/// `source` with each number it reads at run time written as the literal it reads, which the optimizing tiers fold.
fn folded(source: &str) -> String {
    let mut out = String::new();
    let mut rest = source;
    while let Some(at) = rest.find("parseInt(\"") {
        out.push_str(&rest[..at]);
        let digits = &rest[at + "parseInt(\"".len()..];
        let end = digits.find("\")").expect("every parseInt here closes");
        out.push_str(&digits[..end]);
        rest = &digits[end + 2..];
    }
    out.push_str(rest);
    out
}

/// Each program twice: reading its numbers at run time, and folded.
fn programs() -> Vec<Program> {
    PROGRAMS
        .iter()
        .flat_map(|(id, source)| {
            [
                Program::new(format!("popped/{id}"), format!("{NATIVE}{source}")),
                Program::new(format!("popped/{id}/folded"), folded(source)),
            ]
        })
        .collect()
}

#[test]
fn every_program_here_reaches_the_backends() {
    let refused: Vec<String> = programs()
        .into_iter()
        .filter(|p| tier_parity::rejection(&p.source).is_some())
        .map(|p| p.id)
        .collect();
    assert!(refused.is_empty(), "the front end refuses {refused:?}, which pin nothing");
}

#[test]
fn every_program_here_reads_its_numbers_at_run_time_and_its_twin_folds_them() {
    for (id, source) in PROGRAMS {
        assert!(source.contains("parseInt(\""), "{id} reads no number at run time, so a tier may fold it whole");
        assert!(!folded(source).contains("parseInt"), "{id}'s folded twin still reads a number at run time");
    }
}

#[test]
fn every_backend_computes_a_value_taken_from_a_list_as_the_vm_does() {
    let rows = tier_parity::sweep(&programs(), BACKENDS, 4).unwrap_or_else(|why| panic!("the sweep could not start: {why}"));
    let bad: Vec<String> = rows
        .iter()
        .filter(|row| !row.matches())
        .map(|row| format!("{} on {}: {} {}", row.program, row.backend.name(), row.status(), row.detail()))
        .collect();
    assert!(bad.is_empty(), "{} runs differ from the VM:\n{}", bad.len(), bad.join("\n"));
}
