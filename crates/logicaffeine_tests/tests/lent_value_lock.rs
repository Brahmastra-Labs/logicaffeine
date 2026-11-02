//! ★ LOCK — a value lent to a `mutable` parameter is its caller's, and nobody else's, on every compiled tier. ★
//!
//! `## To keep (v: mutable Seq of Int)` lends the caller's list: a change through `v` reaches the caller, and the
//! call site isolates the list first, so the function holds it alone and changes it in place. Once another place
//! holds the value too — a global, a local, a record's field, an item of a list — that place has a copy: the VM's
//! value semantics, which a later change through `v` must not reach. The C bytecode walker changed a lent value in
//! place however many places held it, so a global stored from `v` grew with every later `Push … to v`. Both C
//! backends, both WebAssembly modules and RISC-V are held to the VM here.

#![cfg(not(target_arch = "wasm32"))]

#[path = "tier_parity/mod.rs"]
mod tier_parity;

use tier_parity::{Backend, Program};

const PROGRAMS: &[(&str, &str)] = &[
    (
        "a_global_stored_from_a_lent_list_keeps_its_own_copy",
        "## To keep (v: mutable Seq of Int):\n    Set xs to v.\n    Push 7 to v.\n\n## To show_xs:\n    Show xs.\n\n## Main\nLet mutable xs be [1].\nLet mutable a be [1, 2].\nCall keep with a.\nCall show_xs.\nShow a.\n",
    ),
    (
        "a_local_copy_of_a_lent_list_keeps_its_own",
        "## To keep (v: mutable Seq of Int):\n    Let b be v.\n    Push 7 to v.\n    Show b.\n    Show v.\n\n## Main\nLet mutable a be [1, 2].\nCall keep with a.\nShow a.\n",
    ),
    (
        "a_field_stored_from_a_lent_list_keeps_its_own_copy",
        "## A Box has:\n    an items, which is Seq of Int.\n\n## To keep (v: mutable Seq of Int) -> Box:\n    Let b be a new Box with items v.\n    Push 7 to v.\n    Return b.\n\n## Main\nLet mutable a be [1, 2].\nLet b be keep(a).\nShow b's items.\nShow a.\n",
    ),
    (
        "a_list_holding_a_lent_list_keeps_its_own_copy",
        "## To keep (v: mutable Seq of Int):\n    Let outer be [v, [9]].\n    Push 7 to v.\n    Show outer.\n    Show v.\n\n## Main\nLet mutable a be [1, 2].\nCall keep with a.\nShow a.\n",
    ),
    (
        "a_lent_list_changed_in_place_reaches_its_caller",
        "## To grow (v: mutable Seq of Int):\n    Repeat for i from 1 to 3:\n        Push i to v.\n\n## Main\nLet mutable a be [0].\nCall grow with a.\nShow a.\nCall grow with a.\nShow a.\n",
    ),
    (
        "a_lent_list_shared_then_grown_still_reaches_its_caller",
        "## To keep (v: mutable Seq of Int):\n    Set xs to v.\n    Push 7 to v.\n    Push 8 to v.\n\n## To show_xs:\n    Show xs.\n\n## Main\nLet mutable xs be [1].\nLet mutable a be [1, 2].\nCall keep with a.\nShow a.\nCall show_xs.\n",
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
fn a_lent_value_shared_with_another_place_is_copied_before_it_changes() {
    let programs: Vec<Program> = PROGRAMS
        .iter()
        .map(|(id, source)| Program::new(format!("lent/{id}"), *source))
        .collect();
    let rows = tier_parity::sweep(
        &programs,
        &[
            Backend::TreeWalker,
            Backend::C,
            Backend::CBytecode,
            Backend::Wasm,
            Backend::WasmLinked,
            Backend::RiscV,
        ],
        4,
    )
    .unwrap_or_else(|why| panic!("the sweep could not start: {why}"));
    let bad: Vec<String> = rows
        .iter()
        .filter(|row| !row.matches())
        .map(|row| format!("{} on {}: {} {}", row.program, row.backend.name(), row.status(), row.detail()))
        .collect();
    assert!(bad.is_empty(), "{} runs differ from the VM:\n{}", bad.len(), bad.join("\n"));
}
