//! dae_lock — Decoupled Access-Execute: the address lane splits off the compute path, provably free.
//!
//! Two locks over one corpus of straight-line bodies:
//!  * EQUIVALENCE — the decoupled schedule computes the SAME result AND leaves the SAME heap as the coupled
//!    body, over a large input×heap grid. The transform is a topological re-layout of the same cells, so this
//!    is semantics-preservation by construction, checked to absurdity.
//!  * PERF (the ratchet) — `decoupled_critical_depth ≤ coupled_critical_depth` for every body, STRICTLY less
//!    whenever an address chain fed the compute path, and the AGU lane is pure (no heap tile ever hoisted).
//!    This is HiVec's "keep the array fed": the compute clock stops paying for address arithmetic.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(dae_lock)'`

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::dae::{
    coupled_critical_depth, decouple_access, eval_dae, is_heap_cell, schedule_for_target,
};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    eval_decision_fabric_heap, AluOp, DecisionFabric, LoopCell, Src,
};
use logicaffeine_compile::codegen_sva::fabric_target::FabricTarget;

struct Body {
    name: &'static str,
    n_in: usize,
    cells: Vec<LoopCell>,
    result: Src,
}

fn arith(op: AluOp, a: Src, b: Src) -> LoopCell {
    LoopCell::Arith { op, a, b }
}

/// The corpus. `addr_heavy`/`mixed`/`ptr_chase` carry address arithmetic (a decoupling win); `pure` has no
/// memory (must be a no-op for the ratchet — equality, never a regression).
fn corpus() -> Vec<Body> {
    use AluOp::*;
    use Src::{Const as C, In, Val};
    vec![
        // A deep address chain `(p+i)*k` feeding a load feeding compute — the canonical DAE win.
        Body {
            name: "addr_heavy",
            n_in: 3,
            cells: vec![
                arith(Add, In(0), In(1)),            // v0 = p + i
                arith(Mul, Val(0), In(2)),           // v1 = (p+i) * k   (address, depth 2)
                LoopCell::HeapRead { addr: Val(1) }, // v2 = heap[v1]
                arith(Add, Val(2), In(1)),           // v3 = load + i    (compute)
            ],
            result: Val(3),
        },
        // An address feeding a store.
        Body {
            name: "mixed",
            n_in: 3,
            cells: vec![
                arith(Add, In(0), In(1)), // v0 = p + i (address)
                LoopCell::HeapWrite {
                    addr: Val(0),
                    val: In(2),
                }, // heap[v0] = k
            ],
            result: Val(1),
        },
        // Pointer chase: the second address depends on a LOADED value, so it must NOT be hoisted.
        Body {
            name: "ptr_chase",
            n_in: 3,
            cells: vec![
                arith(Add, In(0), In(1)),            // v0 = p + i (address, untainted)
                LoopCell::HeapRead { addr: Val(0) }, // v1 = heap[v0]  (taints downstream)
                arith(Add, Val(1), In(2)),           // v2 = load + j  (tainted address)
                LoopCell::HeapRead { addr: Val(2) }, // v3 = heap[v2]
            ],
            result: Val(3),
        },
        // Pure compute — no state plane. Decoupling must be a no-op.
        Body {
            name: "pure",
            n_in: 2,
            cells: vec![arith(Add, In(0), In(1)), arith(Mul, Val(0), C(3))],
            result: Val(1),
        },
    ]
}

fn eval_coupled(b: &Body, inputs: &[i64], heap: &mut Vec<i64>) -> i64 {
    let df = DecisionFabric {
        n_in: b.n_in,
        cells: b.cells.clone(),
        result: b.result,
        // The DAE lane never carries an escaping first-return (dae.rs constructs the fabric with
        // `false`); these bodies are plain arithmetic/heap sequences that end in `result`.
        escaped_first_return: false,
    };
    eval_decision_fabric_heap(&df, inputs, heap)
}

#[test]
fn decoupling_preserves_result_and_heap_over_a_large_input_and_heap_grid() {
    let seeds: &[Vec<i64>] = &[vec![], vec![10, 20, 30, 40, 50, 60, 70, 80], vec![-1, 0, 7]];
    let mut checked = 0usize;
    for b in corpus() {
        let sched = decouple_access(&b.cells, b.result, b.n_in);
        // p,i address the heap (`mixed` STORES at p+i), so keep them non-negative and small — a negative store
        // address is a multi-terabyte `heap.resize` in the model (identically on both sides, so not an
        // equivalence signal, just an OOM). The value/multiplier axis `k` still ranges over negatives.
        for p in 0..=5 {
            for i in 0..=5 {
                for k in -3..=3 {
                    let inputs = [p, i, k];
                    let inp = &inputs[..b.n_in];
                    for seed in seeds {
                        let mut h_coupled = seed.clone();
                        let mut h_dae = seed.clone();
                        let coupled = eval_coupled(&b, inp, &mut h_coupled);
                        let dae = eval_dae(&sched, inp, &mut h_dae);
                        assert_eq!(
                            coupled, dae,
                            "{}: result differs at inputs {inp:?} seed {seed:?}",
                            b.name
                        );
                        assert_eq!(
                            h_coupled, h_dae,
                            "{}: heap state differs at inputs {inp:?} seed {seed:?}",
                            b.name
                        );
                        checked += 1;
                    }
                }
            }
        }
    }
    assert!(
        checked >= 1000,
        "the equivalence grid must be genuinely large (got {checked})"
    );
    println!("DAE: decoupled ≡ coupled (result + heap) over {checked} input×heap cases");
}

#[test]
fn the_agu_lane_is_pure_and_the_depth_ratchet_holds_strictly_when_addresses_feed_compute() {
    let mut saw_strict_win = false;
    for b in corpus() {
        let sched = decouple_access(&b.cells, b.result, b.n_in);
        let coupled = coupled_critical_depth(&b.cells, b.result);
        let decoupled = sched.decoupled_critical_depth();

        // The AGU lane never carries a memory tile — hoisting it cannot reorder the state plane.
        assert!(
            sched.agu_is_pure(),
            "{}: an AGU-lane cell is a heap tile — memory could be reordered",
            b.name
        );
        assert!(
            sched
                .layout
                .iter()
                .zip(&sched.is_agu)
                .filter(|(_, a)| **a)
                .all(|(c, _)| !is_heap_cell(c)),
            "{}: AGU purity must hold cell-by-cell",
            b.name
        );

        // The ratchet: the decoupled compute path is never deeper than the coupled one.
        assert!(
            decoupled <= coupled,
            "{}: decoupled depth {decoupled} > coupled depth {coupled} — decoupling made it WORSE",
            b.name
        );

        match b.name {
            // Bodies whose addresses fed the compute path must get STRICTLY shallower.
            "addr_heavy" | "mixed" | "ptr_chase" => {
                assert!(
                    decoupled < coupled,
                    "{}: expected a strict decoupling win, got {decoupled} !< {coupled}",
                    b.name
                );
                assert!(
                    sched.agu_len >= 1,
                    "{}: an address-bearing body must populate the AGU lane",
                    b.name
                );
                saw_strict_win = true;
            }
            // Pure compute: nothing to decouple ⇒ identical depth, no regression.
            "pure" => {
                assert_eq!(
                    decoupled, coupled,
                    "pure body must be unchanged by decoupling"
                );
                assert_eq!(sched.agu_len, 0, "pure body must have an empty AGU lane");
            }
            other => panic!("unexpected corpus body {other}"),
        }
    }
    assert!(
        saw_strict_win,
        "the corpus must exercise at least one strict decoupling win"
    );
}

#[test]
fn the_agu_knob_gates_decoupling_on_the_target() {
    // The descriptor knob: a target WITH an AGU decouples the address lane; one WITHOUT leaves the body coupled.
    let b = &corpus()[0]; // addr_heavy
    let coupled = coupled_critical_depth(&b.cells, b.result);

    let with_agu = schedule_for_target(&b.cells, b.result, b.n_in, &FabricTarget::wearable());
    assert!(
        with_agu.agu_len >= 1,
        "an AGU target must populate the address lane"
    );
    assert!(
        with_agu.decoupled_critical_depth() < coupled,
        "an AGU target must shorten the compute path ({} !< {coupled})",
        with_agu.decoupled_critical_depth()
    );

    let no_agu = schedule_for_target(&b.cells, b.result, b.n_in, &FabricTarget::tiny());
    assert_eq!(
        no_agu.agu_len, 0,
        "a target without an AGU must leave the body coupled"
    );
    assert_eq!(
        no_agu.decoupled_critical_depth(),
        coupled,
        "a no-AGU schedule must have the coupled depth"
    );

    // Both schedules still compute the coupled result (identity, or a proven reordering).
    let inputs = [2i64, 1, 3];
    for sched in [&with_agu, &no_agu] {
        let mut h_ref = vec![10, 20, 30, 40, 50, 60, 70, 80];
        let mut h = h_ref.clone();
        let want = eval_coupled(b, &inputs, &mut h_ref);
        assert_eq!(
            eval_dae(sched, &inputs, &mut h),
            want,
            "both schedules ≡ the coupled body"
        );
    }
}
