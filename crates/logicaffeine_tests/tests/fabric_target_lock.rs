//! fabric_target_lock — the ADL spine: one certified placer, retargeted across fabric shapes.
//!
//! `verified_placer_lock` proves `route_loop_body` is semantics-preserving for the FIXED shape it derives
//! from a body's cell count. The MLAR/ADL ask is to place the SAME dataflow onto a DESCRIBED fabric — a
//! `FabricTarget` naming the array size, the ALU/func-unit capability set, the memory depth, and (later) the
//! AGU / SIMD lanes — and to do it so every target either **provably computes the VM's function** or **refuses
//! with a typed error**, never a wrong chip. This lock pins exactly that contract:
//!
//!  1. `place_with_target` on a CAPABLE target is certified semantics-preserving over the whole bounded family
//!     (the `verified_placer_lock` theorem, now through the target's capability gate + `pad_grid` to the
//!     described shape — so padding a placed grid onto a larger described array is proven to preserve the
//!     computed function, for ALL inputs).
//!  2. A target that LACKS an op the program needs `Err(Unplaceable::MissingCapability { .. })`, and the error
//!     names the missing op — a typed refusal, not a silently-wrong placement.
//!  3. A target SMALLER than the placed dataflow `Err(Unplaceable::TooSmall { .. })`.
//!  4. The certificate is not vacuous: a grid placed for one body must NOT certify as a DIFFERENT datapath.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(fabric_target_lock)'`

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::bitblast::lower_bool;
use logicaffeine_compile::codegen_sva::fabric_config::certify_config_table;
use logicaffeine_compile::codegen_sva::fabric_grid::{
    eval_grid, grid_output_to_bounded, route_loop_body,
};
use logicaffeine_compile::codegen_sva::fabric_rtl::decision_fabric_from_source;
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    eval_fabric, fabric_to_bounded, AluOp, FabricOp, FuncUnit, LoopCell, Src,
};
use logicaffeine_compile::codegen_sva::fabric_target::{
    compile_to_config_table_on_target, place_with_target, Capability, Dataflow, FabricTarget,
    MemHierarchy, MemTier, Precision, Unplaceable,
};
use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;
use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
use logicaffeine_proof::ProofExpr;

// The bounded family the placer is certified over — identical to `verified_placer_lock` so the ADL inherits
// exactly the proven family, now placed THROUGH a `FabricTarget` onto a described (padded) array.
const N_IN: usize = 2;
const CONSTS: &[i64] = &[1, 2];
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
const W: u32 = 4;

fn leaves() -> Vec<Src> {
    let mut v: Vec<Src> = (0..N_IN).map(Src::In).collect();
    v.extend(CONSTS.iter().map(|c| Src::Const(*c)));
    v
}

fn enumerate_bodies(k: usize) -> Vec<(Vec<FabricOp>, usize)> {
    let leaf = leaves();
    let mut bodies: Vec<(Vec<FabricOp>, usize)> = Vec::new();
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

fn to_cells(body: &[FabricOp]) -> Vec<LoopCell> {
    body.iter()
        .map(|o| LoopCell::Arith {
            op: o.op,
            a: o.a,
            b: o.b,
        })
        .collect()
}

/// Prove `eval_grid(place_with_target(body, target)) == eval_fabric(body)` for ALL inputs at width `width`.
fn placement_is_certified_on(
    target: &FabricTarget,
    body: &[FabricOp],
    result: usize,
    width: u32,
) -> bool {
    let cells = to_cells(body);
    let g = place_with_target(&cells, N_IN, &[Src::Val(result)], target)
        .expect("a capable target must place every body in the bounded family");
    let grid = grid_output_to_bounded(&g, 0, width, &|k| format!("in{k}"));
    let datapath = fabric_to_bounded(body, result, width);
    let claim = BoundedExpr::Eq(Box::new(grid), Box::new(datapath));
    let p =
        lower_bool(&claim).expect("the placement-equivalence claim lowers to a bounded formula");
    prove_unsat(&ProofExpr::Not(Box::new(p))) == UnsatOutcome::Refuted
}

#[test]
fn place_with_target_is_certified_over_the_whole_bounded_family_on_a_capable_target() {
    // Gather the whole family, size a described array that fits every member (+2 margin so `pad_grid` runs and
    // the certification proves padding onto a LARGER described array is semantics-preserving).
    let mut family: Vec<(Vec<FabricOp>, usize)> = Vec::new();
    for k in 1..=MAX_CELLS {
        let bodies = enumerate_bodies(k);
        assert!(!bodies.is_empty(), "the {k}-cell family must be non-empty");
        family.extend(bodies);
    }
    let (mut mr, mut mc) = (1usize, 1usize);
    for (body, result) in &family {
        let g = route_loop_body(&to_cells(body), N_IN, &[Src::Val(*result)]);
        mr = mr.max(g.rows);
        mc = mc.max(g.cols);
    }
    let target = FabricTarget::full(mr + 2, mc + 2, 64);

    let mut total = 0usize;
    for (body, result) in &family {
        total += 1;
        // Concrete-eval agreement first (also proves the placed+padded grid RUNS).
        for &(x, y) in &[(0i64, 0i64), (1, 2), (3, 5), (7, 1), (2, 6)] {
            let g =
                place_with_target(&to_cells(body), N_IN, &[Src::Val(*result)], &target).unwrap();
            let placed = eval_grid(&g, &[x, y]);
            let abstract_ = eval_fabric(body, &[x, y], *result);
            assert_eq!(
                placed, abstract_,
                "placed {placed} != datapath {abstract_} for {body:?} at ({x},{y})"
            );
        }
        assert!(
            placement_is_certified_on(&target, body, *result, W),
            "place_with_target is NOT certified semantics-preserving for {body:?} (result {result})"
        );
    }
    assert!(
        total >= 1000,
        "the certified family must be genuinely large (got {total})"
    );
    println!(
        "ADL: place_with_target certified semantics-preserving over ALL {total} bodies on a {}x{} described \
         array (padded from natural), by our own SAT — one placer, retargeted, still verified",
        mr + 2,
        mc + 2
    );
}

#[test]
fn a_target_missing_an_op_refuses_with_a_typed_error_that_names_the_op() {
    // A TINY target with only Add/Sub. A program that needs Mul must be REFUSED — never placed as some other op.
    let tiny = FabricTarget::with_alu_ops("tiny", 8, 8, 64, &[AluOp::Add, AluOp::Sub]);
    let body = vec![FabricOp {
        op: AluOp::Mul,
        a: Src::In(0),
        b: Src::In(1),
    }];
    let err = place_with_target(&to_cells(&body), N_IN, &[Src::Val(0)], &tiny)
        .expect_err("a target without Mul must refuse a Mul program");
    match &err {
        Unplaceable::MissingCapability { needed, .. } => {
            assert_eq!(
                *needed,
                Capability::Alu(AluOp::Mul),
                "the refusal must name the missing op (Mul)"
            );
        }
        other => panic!("expected MissingCapability, got {other:?}"),
    }
    assert!(
        format!("{err:?}").contains("Mul"),
        "the typed error's message must name Mul: {err:?}"
    );

    // Sanity: the SAME program places fine on a target that HAS Mul — refusal is about capability, not the program.
    let ok = FabricTarget::with_alu_ops("hasmul", 8, 8, 64, &[AluOp::Add, AluOp::Sub, AluOp::Mul]);
    assert!(place_with_target(&to_cells(&body), N_IN, &[Src::Val(0)], &ok).is_ok());
}

#[test]
fn a_target_smaller_than_the_dataflow_refuses_with_a_typed_too_small_error() {
    // A 1x1 described array cannot hold a two-cell dataflow — it must refuse, not silently truncate.
    let cramped = FabricTarget::full(1, 1, 64);
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
    let err = place_with_target(&to_cells(&body), N_IN, &[Src::Val(1)], &cramped)
        .expect_err("a 1x1 array must refuse a 2-cell dataflow");
    assert!(
        matches!(err, Unplaceable::TooSmall { .. }),
        "expected TooSmall, got {err:?}"
    );
}

#[test]
fn the_named_presets_expose_the_capability_knobs() {
    // tiny: a pure combinational scalar array — full ALU, but NO functional units, NO state plane.
    let tiny = FabricTarget::tiny();
    assert!(tiny.provides(Capability::Alu(AluOp::Add)));
    assert!(tiny.provides(Capability::Alu(AluOp::Mul)));
    assert!(
        !tiny.provides(Capability::Func(FuncUnit::DivS)),
        "tiny has no functional units"
    );
    assert!(
        !tiny.provides(Capability::Memory),
        "tiny has no state plane"
    );

    // wearable: the realistic edge target — ALU + division/float functional units + a heap.
    let wearable = FabricTarget::wearable();
    assert!(wearable.provides(Capability::Alu(AluOp::Add)));
    assert!(
        wearable.provides(Capability::Func(FuncUnit::DivS)),
        "wearable carries the divider"
    );
    assert!(
        wearable.provides(Capability::Func(FuncUnit::FAddF)),
        "wearable carries the float lane"
    );
    assert!(
        wearable.provides(Capability::Memory),
        "wearable carries a heap"
    );

    // wide: the superset — everything the fabric can place.
    let wide = FabricTarget::wide();
    for op in [AluOp::Add, AluOp::UShr] {
        assert!(wide.provides(Capability::Alu(op)));
    }
    assert!(wide.provides(Capability::Func(FuncUnit::PopCount)));
    assert!(wide.provides(Capability::Memory));
    assert!(
        wide.cols >= wearable.cols && wearable.cols >= tiny.cols,
        "the presets scale up in size"
    );

    // The SIMD + AGU knobs: scalar tiny has neither; wearable/wide carry both.
    assert!(tiny.simd.is_none() && !tiny.agu, "tiny is scalar, no AGU");
    assert!(
        wearable.simd.is_some() && wearable.agu,
        "wearable carries packed SIMD + an AGU"
    );
    assert!(
        wide.simd.is_some() && wide.agu,
        "wide carries packed SIMD + an AGU"
    );

    // The knob bites: a division program is REFUSED on tiny (no divider), placed on wearable.
    let div = vec![LoopCell::Func {
        unit: FuncUnit::DivS,
        a: Src::In(0),
        b: Src::In(1),
    }];
    match place_with_target(&div, N_IN, &[Src::Val(0)], &tiny) {
        Err(Unplaceable::MissingCapability { needed, .. }) => {
            assert_eq!(needed, Capability::Func(FuncUnit::DivS));
        }
        other => {
            panic!("tiny must refuse a division program with MissingCapability, got {other:?}")
        }
    }
    assert!(
        place_with_target(&div, N_IN, &[Src::Val(0)], &wearable).is_ok(),
        "wearable (which carries the divider) must place a division program"
    );
}

#[test]
fn a_program_corpus_reconfigures_on_one_described_array_and_certifies() {
    // A corpus of whole functions, each if-converted to a decision island, all placed on ONE described array
    // and reconfig-certified: `config == k ⇒ the described array ≡ island k`, for ALL inputs. This is the ADL
    // payoff — retarget a whole program set onto a chosen fabric shape without giving up the ≡VM proof.
    let sources: &[(&str, &str)] = &[
        ("## To affine (x: Int) -> Int:\n    Return 3 * x + 7.\n", "affine"),
        (
            "## To absval (a: Int) -> Int:\n    Let result be a.\n    If a is less than 0:\n        Set result to 0 - a.\n    Return result.\n",
            "absval",
        ),
        (
            "## To min2 (a: Int, b: Int) -> Int:\n    Let result be b.\n    If a is less than b:\n        Set result to a.\n    Return result.\n",
            "min2",
        ),
    ];
    // Size a described array that fits the whole corpus (+2 margin so every island pads onto it).
    let (mut mr, mut mc) = (1usize, 1usize);
    for (src, name) in sources {
        let df = decision_fabric_from_source(src, name);
        let g = route_loop_body(&df.cells, df.n_in, &[df.result]);
        mr = mr.max(g.rows);
        mc = mc.max(g.cols);
    }
    let target = FabricTarget::full(mr + 2, mc + 2, 8);
    let table =
        compile_to_config_table_on_target(sources, &target, 8, 2).expect("corpus places on target");
    assert_eq!(
        (table.rows, table.cols),
        (mr + 2, mc + 2),
        "the corpus is placed on the DESCRIBED array"
    );
    let verdicts = certify_config_table(&table);
    assert!(
        verdicts.iter().all(|&v| v),
        "each config must certify ≡ its island on the described array: {verdicts:?}"
    );

    // The knob bites for a corpus too: a described array too small for the corpus is refused, not truncated.
    let cramped = FabricTarget::full(2, 1, 8);
    assert!(
        matches!(
            compile_to_config_table_on_target(sources, &cramped, 8, 2),
            Err(Unplaceable::TooSmall { .. })
        ),
        "a corpus that does not fit the described array must be refused with TooSmall"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// Phase 0 — the KNOB SPINE. FabricTarget grows into the full accelerator descriptor: a dataflow set
// (reduction baseline + the 2-D systolic modes), a precision set, a three-tier memory hierarchy, and a
// fusion window. Each is a DIAL, and each dial GATES — a lowering that demands a dial the target lacks is
// refused with a typed `MissingCapability`, never silently placed. These locks pin the gate contract so
// Phases 1+ (weight-stationary, precision, memory, fusion) attach behind a proven refusal, not on faith.
// ─────────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn the_dataflow_dial_gates_weight_stationary() {
    // Reduction is the universal baseline — every target has the 1-D dot-product MAC lane it ships today.
    let red = FabricTarget::tiny();
    assert!(
        red.provides(Capability::Dataflow(Dataflow::Reduction)),
        "every target must advertise the reduction baseline"
    );
    // A reduction-only target REFUSES a weight-stationary GEMM request, naming the missing dataflow.
    match red.require_gemm(Dataflow::WeightStationary, Precision::Int8) {
        Err(Unplaceable::MissingCapability { needed, .. }) => {
            assert_eq!(needed, Capability::Dataflow(Dataflow::WeightStationary));
        }
        other => panic!("a reduction-only target must refuse weight-stationary, got {other:?}"),
    }
    // The superset target advertises every dataflow mode, so it accepts the weight-stationary request.
    let ws = FabricTarget::wide();
    assert!(ws.provides(Capability::Dataflow(Dataflow::WeightStationary)));
    assert!(
        ws.require_gemm(Dataflow::WeightStationary, Precision::Int8)
            .is_ok(),
        "a target advertising weight-stationary must accept a weight-stationary GEMM request"
    );
}

#[test]
fn the_precision_dial_gates_formats() {
    // tiny is scalar-int only: it computes int64, and refuses bf16 (no float-MAC tile behind that dial yet).
    let t = FabricTarget::tiny();
    assert!(t
        .require_gemm(Dataflow::Reduction, Precision::Int64)
        .is_ok());
    match t.require_gemm(Dataflow::Reduction, Precision::Bf16) {
        Err(Unplaceable::MissingCapability { needed, .. }) => {
            assert_eq!(needed, Capability::Precision(Precision::Bf16));
        }
        other => panic!("tiny must refuse bf16 with a typed refusal, got {other:?}"),
    }
    // The superset carries every precision.
    assert!(FabricTarget::wide()
        .require_gemm(Dataflow::Reduction, Precision::Bf16)
        .is_ok());
}

#[test]
fn the_memory_and_fusion_dials_are_exposed_and_gate() {
    let tiny = FabricTarget::tiny();
    assert_eq!(tiny.mem, MemHierarchy::NONE, "tiny has no memory hierarchy");
    assert!(
        !tiny.provides(Capability::MemTier(MemTier::Hbm)),
        "tiny has no HBM tier"
    );
    assert!(
        !tiny.provides(Capability::Fusion(2)),
        "tiny fuses nothing beyond a single op"
    );

    let wide = FabricTarget::wide();
    assert!(
        wide.provides(Capability::MemTier(MemTier::Sram))
            && wide.provides(Capability::MemTier(MemTier::Hbm)),
        "the superset carries an on-chip SRAM tier AND an HBM tier"
    );
    assert!(
        wide.provides(Capability::Fusion(4)),
        "the superset fuses a whole block"
    );
}

#[test]
fn every_preset_carries_the_reduction_baseline_invariant() {
    // Pit-of-success invariant: no matter how the dials are set, a target ALWAYS has the reduction dataflow
    // (the mode every fabric ships). A preset that dropped it would break the toggle policy — RED here.
    for t in [
        FabricTarget::tiny(),
        FabricTarget::wearable(),
        FabricTarget::wide(),
        FabricTarget::full(8, 8, 64),
        FabricTarget::with_alu_ops("alu", 8, 8, 64, &[AluOp::Add]),
    ] {
        assert!(
            t.provides(Capability::Dataflow(Dataflow::Reduction)),
            "preset {} dropped the reduction baseline",
            t.name
        );
    }
}

#[test]
fn the_placement_certificate_is_not_vacuous_on_a_target() {
    // Placing `In0+In1` but claiming it equals `In0-In1` must NOT refute (they differ whenever In1 != 0).
    let target = FabricTarget::full(8, 8, 64);
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
    let g = place_with_target(&to_cells(&a), N_IN, &[Src::Val(0)], &target).unwrap();
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
