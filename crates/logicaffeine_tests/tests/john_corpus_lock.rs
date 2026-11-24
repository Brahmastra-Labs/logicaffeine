//! ★ LOCK — the programs handed to John Whaley run alike on every tier, and can never stop doing so unnoticed. ★
//!
//! `EXAMPLES_FOR_JOHN.pdf` shows each of its run programs printing the same bytes on the tree-walker, the VM, the VM
//! with its JIT, the generated Rust, C and WebAssembly. The programs are copied here byte for byte (`john_corpus/`,
//! each equal to its `work/john/examples` original, which the document's own build holds both ways), with the
//! arguments the document runs them on, and every tier is held to the tree-walker's bytes. A tier that differs where
//! the document does not declare it is a regression; a declared gap that closes fails until it is struck from the list,
//! so the document and this lock move together. The seeded program's schedules are held per seed: at each seed the VM
//! prints the tree-walker's order, and the seeds give more than one order.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test john_corpus_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "tier_parity/mod.rs"]
mod tier_parity;

use std::collections::BTreeSet;

use logicaffeine_compile::ui_bridge::{run_treewalker_concurrent_seeded, run_vm_concurrent_seeded};
use tier_parity::{Backend, Program};

/// Each run program, with the arguments the document runs it on.
const PROGRAMS: &[(&str, &str, &[&str])] = &[
    ("points_to", include_str!("john_corpus/points_to.lg"), &[]),
    ("lending", include_str!("john_corpus/lending.lg"), &[]),
    ("overflow", include_str!("john_corpus/overflow.lg"), &[]),
    ("divisor", include_str!("john_corpus/divisor.lg"), &["7", "1000000"]),
    ("divisor_changing", include_str!("john_corpus/divisor_changing.lg"), &["5"]),
    ("flag", include_str!("john_corpus/flag.lg"), &["0"]),
    ("digit_sum", include_str!("john_corpus/digit_sum.lg"), &["7", "1000000"]),
    ("range_past", include_str!("john_corpus/range_past.lg"), &[]),
    ("rare", include_str!("john_corpus/rare.lg"), &["10000"]),
];

/// The tiers the document shows: the tree-walker, the VM, the VM with its JIT, the generated Rust, C and WebAssembly.
const BACKENDS: &[Backend] =
    &[Backend::TreeWalker, Backend::Vm, Backend::VmForge, Backend::RustRelease, Backend::C, Backend::Wasm];

/// The differences the document declares: `overflow`'s counter past 2^63, held in a list, which the generated Rust
/// stops on where the other tiers go on exactly (B10).
const DECLARED_GAPS: &[(&str, Backend)] = &[("overflow", Backend::RustRelease)];

/// The seeded program and the seeds the document shows it at.
const SLEEPERS: &str = include_str!("john_corpus/sleepers.lg");
const SEEDS: &[u64] = &[0, 1, 2, 3, 7, 42];

fn programs() -> Vec<Program> {
    PROGRAMS
        .iter()
        .map(|(id, source, args)| {
            let mut p = Program::new(*id, *source);
            p.args = args.iter().map(|a| a.to_string()).collect();
            p
        })
        .collect()
}

/// The child side of every isolated backend run. Does nothing unless `LOGOS_PARITY_JOB` is set.
#[test]
#[ignore = "the worker the parity sweep re-executes this binary into"]
fn tier_parity_worker() {
    tier_parity::worker_main();
}

#[test]
fn every_program_here_is_one_the_document_runs() {
    let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work/john/examples");
    if !examples.is_dir() {
        return;
    }
    for (id, source, _) in PROGRAMS.iter().copied().chain([("sleepers", SLEEPERS, &[][..])]) {
        let original = std::fs::read_to_string(examples.join(format!("{id}.lg"))).expect("the document's program");
        assert_eq!(source, original, "john_corpus/{id}.lg is a byte copy of work/john/examples/{id}.lg");
    }
}

#[test]
fn every_tier_prints_the_tree_walkers_bytes_where_the_document_says_it_does() {
    let rows = tier_parity::sweep(&programs(), BACKENDS, 4).unwrap_or_else(|why| panic!("the sweep could not start: {why}"));
    let differ: BTreeSet<(String, Backend)> =
        rows.iter().filter(|row| !row.matches()).map(|row| (row.program.clone(), row.backend)).collect();
    let declared: BTreeSet<(String, Backend)> = DECLARED_GAPS.iter().map(|(p, b)| (p.to_string(), *b)).collect();
    let new: Vec<String> = rows
        .iter()
        .filter(|row| !row.matches() && !declared.contains(&(row.program.clone(), row.backend)))
        .map(|row| format!("{} on {}: {} {}", row.program, row.backend.name(), row.status(), row.detail()))
        .collect();
    assert!(new.is_empty(), "{} runs differ from the tree-walker where the document says they agree:\n{}", new.len(), new.join("\n"));
    let closed: Vec<String> =
        declared.difference(&differ).map(|(p, b)| format!("{p} on {}", b.name())).collect();
    assert!(closed.is_empty(), "declared gaps that now agree — strike them here and on the document's page: {closed:?}");
}

#[test]
fn the_seeded_program_replays_each_seeds_order_on_the_vm() {
    let mut orders = BTreeSet::new();
    for &seed in SEEDS {
        let tree = run_treewalker_concurrent_seeded(SLEEPERS, seed);
        let vm = run_vm_concurrent_seeded(SLEEPERS, seed);
        assert_eq!(tree.error, None, "seed {seed}: the tree-walker raised");
        assert_eq!(vm.error, None, "seed {seed}: the VM raised");
        assert_eq!(vm.lines, tree.lines, "seed {seed}: the VM prints the tree-walker's order");
        let mut sorted = tree.lines.clone();
        sorted.sort();
        assert_eq!(sorted, ["1", "2", "3", "4"], "seed {seed}: each task shows its number once");
        orders.insert(tree.lines);
    }
    assert!(orders.len() > 1, "the seed decides the schedule: {orders:?}");
}
