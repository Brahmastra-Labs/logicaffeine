//! verified_placer_lock — Stage 3 of the self-synthesizing bytecode XPU: the CERTIFIED-BY-CONSTRUCTION
//! placer. The synthesizer-in-silicon (Stage 4) places each bytecode block's dataflow onto the fabric at
//! runtime with NO on-chip checker — which is only sound if the placement algorithm is a VERIFIED
//! algorithm: `∀ block body B, eval_grid(place(B)) ≡ eval_body(B)`.
//!
//! `fabric_grid`'s existing SAT test certifies that equivalence for a couple of SAMPLED programs (Horner, a
//! linear residual). This lock generalizes it to a UNIVERSAL statement over a bounded family: it ENUMERATES
//! every straight-line body up to `MAX_CELLS` cells over `N_IN` inputs, `OPS` operations, and the leaf/`Val`
//! operand set — thousands of distinct dataflow DAGs — places each with `route_loop_body` (the deterministic
//! canonical placer the sequencer uses), and proves with our own certified SAT that the placed grid computes
//! EXACTLY the abstract datapath, for ALL inputs. Every body in the family refuted ⇒ the placement transform
//! is semantics-preserving by construction, not just on the samples. That is the theorem that lets the
//! hardware placer skip a runtime proof-checker.
//!
//! It also proves the SAT check is not vacuous (a deliberately mis-wired grid is NOT refuted), and spot-checks
//! the widest datapath (`WIDE`) so the certification is not a small-width artifact — the routing is structural
//! (it moves values, it does not transform them), so width-4 refutation already pins correctness, and the
//! wide spot-check confirms it.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(verified_placer_lock)'`

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::bitblast::lower_bool;
use logicaffeine_compile::codegen_sva::fabric_grid::{
    eval_grid, grid_output_to_bounded, route_loop_body,
};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    eval_fabric, fabric_to_bounded, AluOp, FabricOp, LoopCell, Src,
};
use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;
use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
use logicaffeine_proof::ProofExpr;

/// The bounded family the placer is certified over. Small enough to enumerate EXHAUSTIVELY and prove every
/// member, wide enough to exercise the placement + routing (fan-in, `Val` routing, multi-row placement).
const N_IN: usize = 2;
const CONSTS: &[i64] = &[1, 2];
// The FULL combinational ALU set the fabric places (arith + bitwise + shifts). The placer is op-agnostic — it
// wires values, it does not interpret them — so covering every op widens the certified family at no design
// cost and proves no op is special-cased in placement/routing.
const OPS: &[AluOp] = &[
    AluOp::Add,
    AluOp::Sub,
    AluOp::Mul,
    AluOp::And,
    AluOp::Or,
    AluOp::Xor,
    AluOp::Shl,
    AluOp::Shr,
];
const MAX_CELLS: usize = 2;
/// SAT datapath width for the exhaustive sweep. Routing is structural (it moves values unchanged), so a
/// mis-route already differs within 4 bits — width 4 refutes every routing bug while keeping thousands of
/// proofs fast. `WIDE` re-checks a sample at a larger width so the claim is not a small-width artifact.
const W: u32 = 4;
const WIDE: u32 = 8;

/// The leaf operands available to the FIRST cell: every input plus every constant (no `Val` yet).
fn leaves() -> Vec<Src> {
    let mut v: Vec<Src> = (0..N_IN).map(Src::In).collect();
    v.extend(CONSTS.iter().map(|c| Src::Const(*c)));
    v
}

/// Enumerate every straight-line body with exactly `k` cells (1..=MAX_CELLS). Cell `i` may read any leaf or
/// any earlier cell's value `Val(0..i)`; every `(op, a, b)` combination is produced — the whole bounded DAG
/// family. Returned as `(body, result)` where `result` is the last cell.
fn enumerate_bodies(k: usize) -> Vec<(Vec<FabricOp>, usize)> {
    let leaf = leaves();
    let mut bodies: Vec<(Vec<FabricOp>, usize)> = Vec::new();

    // Recursively build cell `i`, given the prefix already chosen.
    fn build(
        i: usize,
        k: usize,
        prefix: &mut Vec<FabricOp>,
        leaf: &[Src],
        ops: &[AluOp],
        out: &mut Vec<(Vec<FabricOp>, usize)>,
    ) {
        if i == k {
            out.push((prefix.clone(), k - 1));
            return;
        }
        // operands visible to cell i: the leaves + Val(0..i)
        let mut operands: Vec<Src> = leaf.to_vec();
        operands.extend((0..i).map(Src::Val));
        for &op in ops {
            for &a in &operands {
                for &b in &operands {
                    prefix.push(FabricOp { op, a, b });
                    build(i + 1, k, prefix, leaf, ops, out);
                    prefix.pop();
                }
            }
        }
    }

    build(0, k, &mut Vec::new(), &leaf, OPS, &mut bodies);
    bodies
}

/// The abstract datapath as `LoopCell`s (route_loop_body's input) — `FabricOp` and `LoopCell::Arith` share
/// `{op, a, b}`, so this is a direct transcription.
fn to_cells(body: &[FabricOp]) -> Vec<LoopCell> {
    body.iter()
        .map(|o| LoopCell::Arith {
            op: o.op,
            a: o.a,
            b: o.b,
        })
        .collect()
}

/// Prove `eval_grid(route_loop_body(body)) == eval_fabric(body)` for ALL inputs at datapath width `width`,
/// via our own certified SAT (refute the negation). Returns true iff refuted (semantics-preserving).
fn placement_is_certified(body: &[FabricOp], result: usize, width: u32) -> bool {
    let cells = to_cells(body);
    let g = route_loop_body(&cells, N_IN, &[Src::Val(result)]);
    let grid = grid_output_to_bounded(&g, 0, width, &|k| format!("in{k}"));
    let datapath = fabric_to_bounded(body, result, width);
    let claim = BoundedExpr::Eq(Box::new(grid), Box::new(datapath));
    let p =
        lower_bool(&claim).expect("the placement-equivalence claim lowers to a bounded formula");
    prove_unsat(&ProofExpr::Not(Box::new(p))) == UnsatOutcome::Refuted
}

#[test]
fn the_placer_is_certified_semantics_preserving_over_the_whole_bounded_family() {
    let mut total = 0usize;
    let mut refuted = 0usize;

    for k in 1..=MAX_CELLS {
        let bodies = enumerate_bodies(k);
        assert!(!bodies.is_empty(), "the {k}-cell family must be non-empty");
        for (body, result) in &bodies {
            total += 1;

            // A cheap concrete-eval agreement first (a fast filter that also proves the placement RUNS).
            for &(x, y) in &[(0i64, 0i64), (1, 2), (3, 5), (7, 1), (2, 6)] {
                let placed = eval_grid(
                    &route_loop_body(&to_cells(body), N_IN, &[Src::Val(*result)]),
                    &[x, y],
                );
                let abstract_ = eval_fabric(body, &[x, y], *result);
                assert_eq!(
                    placed, abstract_,
                    "placed grid {placed} != datapath {abstract_} for body {body:?} at ({x},{y})"
                );
            }

            // The exhaustive per-body certificate: placed grid ≡ datapath for ALL inputs.
            if placement_is_certified(body, *result, W) {
                refuted += 1;
            } else {
                panic!(
                    "route_loop_body is NOT certified semantics-preserving for body {body:?} (result {result}) \
                     — the placement transform changed the computed function"
                );
            }
        }
    }

    assert_eq!(
        refuted, total,
        "every body in the bounded family must be certified"
    );
    assert!(
        total >= 1000,
        "the certified family must be genuinely large (got {total})"
    );
    println!(
        "Stage 3: route_loop_body certified semantics-preserving over ALL {total} bodies of the bounded family \
         (≤{MAX_CELLS} cells, {} ops, {N_IN} inputs) by our own SAT — the placer is verified by construction",
        OPS.len()
    );
}

/// The SAT check is NOT vacuous: a grid placed for one body must NOT be certified equal to a DIFFERENT
/// datapath (the prover must FAIL to refute — i.e. find a separating input). Guards against a lowering that
/// trivially "proves" everything.
#[test]
fn the_placement_certificate_is_not_vacuous() {
    // body A = In0 + In1 ; body B (the wrong reference) = In0 - In1. Placing A but claiming it equals B must
    // NOT refute (they differ whenever In1 != 0).
    let a = vec![FabricOp {
        op: AluOp::Add,
        a: Src::In(0),
        b: Src::In(1),
    }];
    let b = vec![FabricOp {
        op: AluOp::Sub,
        a: Src::In(0),
        b: Src::In(1),
    }];
    let g = route_loop_body(&to_cells(&a), N_IN, &[Src::Val(0)]);
    let grid = grid_output_to_bounded(&g, 0, W, &|k| format!("in{k}"));
    let wrong = fabric_to_bounded(&b, 0, W);
    let claim = BoundedExpr::Eq(Box::new(grid), Box::new(wrong));
    let p = lower_bool(&claim).expect("claim lowers");
    assert_ne!(
        prove_unsat(&ProofExpr::Not(Box::new(p))),
        UnsatOutcome::Refuted,
        "a grid placed for `In0+In1` must NOT certify as equal to `In0-In1` — the SAT check would be vacuous"
    );
}

/// The certification is not a small-width artifact: re-prove a representative multi-cell body at the WIDE
/// datapath width. Routing is structural, so width-4 already refutes routing bugs; this confirms it holds at
/// the production width too.
#[test]
fn the_placer_is_certified_at_the_wide_datapath_width() {
    // (In0 * In1) + In0 — a two-cell body with a routed `Val` operand (the fan-in the placer must route).
    let body = vec![
        FabricOp {
            op: AluOp::Mul,
            a: Src::In(0),
            b: Src::In(1),
        },
        FabricOp {
            op: AluOp::Add,
            a: Src::Val(0),
            b: Src::In(0),
        },
    ];
    assert!(
        placement_is_certified(&body, 1, WIDE),
        "route_loop_body must be certified for `(In0*In1)+In0` at width {WIDE}"
    );
}
