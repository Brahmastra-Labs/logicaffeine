//! The bounded ALU-body family every fabric-target placement is certified over, and the SAT check that a
//! placement THROUGH a described target computes the body's function for ALL inputs. Shared (via
//! `#[path = "common/bounded_family.rs"]`) by the per-target locks so a new target (Plasticine, …) inherits
//! exactly the family `verified_placer_lock` / `fabric_target_lock` proved, not a smaller one.
#![allow(dead_code)]

use logicaffeine_compile::codegen_sva::bitblast::lower_bool;
use logicaffeine_compile::codegen_sva::fabric_grid::{
    eval_grid, grid_output_to_bounded, route_loop_body,
};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    eval_fabric, fabric_to_bounded, AluOp, FabricOp, LoopCell, Src,
};
use logicaffeine_compile::codegen_sva::fabric_target::{place_with_target, FabricTarget};
use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;
use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
use logicaffeine_proof::ProofExpr;

pub const N_IN: usize = 2;
pub const CONSTS: &[i64] = &[1, 2];
pub const OPS: &[AluOp] = &[
    AluOp::Add,
    AluOp::Sub,
    AluOp::Mul,
    AluOp::And,
    AluOp::Or,
    AluOp::Xor,
    AluOp::Shl,
    AluOp::Shr,
];
pub const MAX_CELLS: usize = 2;
pub const W: u32 = 4;

pub fn leaves() -> Vec<Src> {
    let mut v: Vec<Src> = (0..N_IN).map(Src::In).collect();
    v.extend(CONSTS.iter().map(|c| Src::Const(*c)));
    v
}

pub fn enumerate_bodies(k: usize) -> Vec<(Vec<FabricOp>, usize)> {
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

/// The whole family (1..=MAX_CELLS cells).
pub fn whole_family() -> Vec<(Vec<FabricOp>, usize)> {
    let mut family = Vec::new();
    for k in 1..=MAX_CELLS {
        let bodies = enumerate_bodies(k);
        assert!(!bodies.is_empty(), "the {k}-cell family must be non-empty");
        family.extend(bodies);
    }
    family
}

pub fn to_cells(body: &[FabricOp]) -> Vec<LoopCell> {
    body.iter()
        .map(|o| LoopCell::Arith {
            op: o.op,
            a: o.a,
            b: o.b,
        })
        .collect()
}

/// The natural placement envelope (rows, cols) the whole family needs — a target must be at least this big.
pub fn family_envelope() -> (usize, usize) {
    let (mut mr, mut mc) = (1usize, 1usize);
    for (body, result) in whole_family() {
        let g = route_loop_body(&to_cells(&body), N_IN, &[Src::Val(result)]);
        mr = mr.max(g.rows);
        mc = mc.max(g.cols);
    }
    (mr, mc)
}

/// Prove `eval_grid(place_with_target(body, target)) == eval_fabric(body)` for ALL inputs at width `width`.
pub fn placement_is_certified_on(
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

/// Concrete-eval agreement on a few points (also proves the placed+padded grid RUNS), then the SAT certificate,
/// over the WHOLE family; returns the family size.
pub fn certify_whole_family_on(target: &FabricTarget) -> usize {
    let family = whole_family();
    for (body, result) in &family {
        for &(x, y) in &[(0i64, 0i64), (1, 2), (3, 5), (7, 1), (2, 6)] {
            let g = place_with_target(&to_cells(body), N_IN, &[Src::Val(*result)], target).unwrap();
            let placed = eval_grid(&g, &[x, y]);
            let abstract_ = eval_fabric(body, &[x, y], *result);
            assert_eq!(
                placed, abstract_,
                "placed {placed} != datapath {abstract_} for {body:?} at ({x},{y}) on {}",
                target.name
            );
        }
        assert!(
            placement_is_certified_on(target, body, *result, W),
            "not certified on {}: {body:?} (result {result})",
            target.name
        );
    }
    family.len()
}
