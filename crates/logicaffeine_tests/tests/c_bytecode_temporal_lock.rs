//! ★ LOCK — a time value on the bytecode walker is the value it is, or a refusal by name. ★
//!
//! The walker holds a date as its days since the epoch, an instant and a time of day as nanoseconds, and a span as
//! its months and days packed in one slot. Those numbers are not the values: a date plus a number of days, a date
//! beside an instant, an ordering of spans are each the runtime's to answer, and machine arithmetic on the slots
//! would print a date no tier computes. So a time value shows and orders against its own kind as the VM does, and
//! every other operation on one is refused by name at generation, never computed.

#![cfg(not(target_arch = "wasm32"))]

#[path = "tier_parity/mod.rs"]
mod tier_parity;

use tier_parity::{Backend, Program, Verdict};

/// Programs the walker runs as the VM does: each kind of time constant shown, and two of one kind ordered.
const RUNS: &[(&str, &str)] = &[
    (
        "dates_show_and_order",
        "## Main\nLet d be 2026-01-01.\nLet e be 2026-02-01.\nShow d.\nShow e.\nShow d < e.\nShow e <= d.\nShow d = d.\n",
    ),
    ("a_span_shows", "## Main\nLet s be 2 weeks.\nShow s.\nLet t be 3 months.\nShow t.\n"),
    ("a_time_of_day_shows_and_orders", "## Main\nLet a be 9am.\nLet b be 4pm.\nShow a.\nShow b.\nShow a < b.\n"),
];

/// Programs that combine a time value as the walker's slots cannot: each is refused at generation.
const REFUSED: &[(&str, &str)] = &[
    ("a_date_plus_a_number", "## Main\nLet d be 2026-01-01.\nShow d + 1.\n"),
    ("a_date_plus_a_span", "## Main\nLet d be 2026-01-01.\nLet s be 2 weeks.\nShow d + s.\n"),
    ("spans_ordered", "## Main\nLet s be 2 weeks.\nLet t be 3 days.\nShow s < t.\n"),
    ("a_date_beside_a_number", "## Main\nLet d be 2026-01-01.\nShow d < 5.\n"),
];

/// The child side of every isolated backend run. Does nothing unless `LOGOS_PARITY_JOB` is set.
#[test]
#[ignore = "the worker the parity sweep re-executes this binary into"]
fn tier_parity_worker() {
    tier_parity::worker_main();
}

#[test]
fn every_program_here_reaches_the_backends() {
    let refused: Vec<&str> = RUNS
        .iter()
        .chain(REFUSED)
        .filter(|(_, source)| tier_parity::rejection(source).is_some())
        .map(|(id, _)| *id)
        .collect();
    assert!(refused.is_empty(), "the front end refuses {refused:?}, which pin nothing");
}

#[test]
fn a_time_value_shows_and_orders_on_the_walker_as_on_the_vm() {
    let programs: Vec<Program> = RUNS
        .iter()
        .map(|(id, source)| Program::new(format!("time/{id}"), *source))
        .collect();
    let rows = tier_parity::sweep(&programs, &[Backend::TreeWalker, Backend::CBytecode], 4)
        .unwrap_or_else(|why| panic!("the sweep could not start: {why}"));
    let bad: Vec<String> = rows
        .iter()
        .filter(|row| !row.matches())
        .map(|row| format!("{} on {}: {} {}", row.program, row.backend.name(), row.status(), row.detail()))
        .collect();
    assert!(bad.is_empty(), "{} runs differ from the VM:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn arithmetic_on_a_time_value_is_refused_by_name_never_computed() {
    for (id, source) in REFUSED {
        match logicaffeine_compile::compile::compile_to_c_via_bytecode(source) {
            Ok(_) => panic!("{id}: the walker lowered an operation on a time value it computes on raw slots"),
            Err(e) => {
                let why = format!("{e:?}");
                assert!(why.contains("not yet lowered"), "{id}: the walker must name what it refuses: {why}");
            }
        }
        let program = Program::new(format!("time/{id}"), *source);
        let verdict = tier_parity::run(Backend::CBytecode, &program, None);
        assert!(
            matches!(verdict, Verdict::Refused(_) | Verdict::Unavailable(_)),
            "{id}: the walker {} a program it must refuse",
            verdict.label()
        );
    }
}
