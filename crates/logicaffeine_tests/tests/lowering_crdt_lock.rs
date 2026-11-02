//! lowering_crdt_lock — `Shared` struct CRDTs (which emit `NewCrdt` on a field default-fill) LOWER on the
//! whole-program fabric ≡ the VM. This is the last piece of the "full language surface on the fabric": a
//! `Shared` struct with a `ConvergentCount` / `SharedSequence` / `SharedSet` field, created and read back,
//! runs on the crossbar-lowered whole-program fabric identically to the VM.
//!
//! (The `NewCrdt` op is emitted inside a compiler-generated struct CONSTRUCTOR, not the user function, so the
//! op-reachability lock's `function_op_names(f)` welding does not surface it — that is a welding blind spot,
//! not a functional gap. This lock proves the functional truth directly: the programs lower and run ≡VM.)
//!
//! Run: `cargo test -p logicaffeine-tests --test lowering_crdt_lock`

use logicaffeine_compile::codegen_sva::fabric_rtl::decision_fabric_from_source;
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    eval_wprog_config, wprog_from_decision_fabric,
};
use logicaffeine_compile::compile::vm_outcome;

fn vm_f(src: &str, x: i64) -> Option<i64> {
    let out = vm_outcome(&format!("{src}## Main\n    Show f({x}).\n"));
    if out.error.is_some() {
        return None;
    }
    out.output.split_whitespace().next()?.parse::<i64>().ok()
}

/// Lower `f` via the crossbar (decision fabric → whole-program config) and assert it equals the VM for `xs`.
fn assert_crdt_lowers_eq_vm(label: &str, src: &str, xs: &[i64]) {
    let df = decision_fabric_from_source(src, "f");
    let cfg = wprog_from_decision_fabric(&df);
    for &x in xs {
        let vm = vm_f(src, x).unwrap_or_else(|| panic!("{label}: VM errored on {x}"));
        let fab = eval_wprog_config(&cfg, &[x]);
        assert_eq!(fab, vm, "{label}: fabric {fab} != VM {vm} on input {x}");
    }
}

#[test]
fn convergent_count_struct_lowers_on_the_fabric() {
    // NewCrdt kind 4 (ConvergentCount default-fill) + CrdtBump.
    assert_crdt_lowers_eq_vm(
        "ConvergentCount",
        "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## To f (v: Int) -> Int:\n    Let mutable c be a new Counter.\n    Increase c's points by v.\n    Return c's points.\n",
        &[0, 7, 20, 100],
    );
}

#[test]
fn shared_sequence_struct_lowers_on_the_fabric() {
    // NewCrdt kind 1 (SharedSequence default-fill) on a MIXED struct (Int + CRDT field) + CrdtAppend + length.
    assert_crdt_lowers_eq_vm(
        "SharedSequence",
        "## Definition\nA Room is Shared and has:\n    capacity: Int.\n    members: SharedSequence of Int.\n## To f (v: Int) -> Int:\n    Let mutable r be a new Room with capacity v.\n    Append 5 to r's members.\n    Return r's capacity + the length of r's members.\n",
        &[0, 7, 20],
    );
}

#[test]
fn shared_set_struct_default_fill_lowers_on_the_fabric() {
    // NewCrdt kind 0 (SharedSet default-fill); the CRDT field is never explicitly set — pure default-fill.
    assert_crdt_lowers_eq_vm(
        "SharedSet default-fill",
        "## Definition\nA Group is Shared and has:\n    id: Int.\n    tags: SharedSet of Int.\n## To f (v: Int) -> Int:\n    Let mutable g be a new Group with id v.\n    Return v + the length of g's tags.\n",
        &[0, 7, 20],
    );
}
