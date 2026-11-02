//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! A SECTION'S SECOND OUTPUT — a tensor of ANOTHER SHAPE stored beside its result — keeps the shape it has.
//!
//! A work item's rectangle is its block of the section's RESULT, and only of that. A section that also writes a second
//! tensor — a row statistic beside a matrix, a partial of another width — addresses it by its own store's pattern:
//!   • placement reshapes only the result's store to a site's block, so the second store keeps its pattern;
//!   • the validator covers the second tensor by the tiles its store ADDRESSES for each item, not by the rectangle.
//!
//! Pinned:
//!   1. The template — `r = x + 1`, two tiles per item, and `s = rowsum(x)`, one tile per item — verifies on the SN50,
//!      its second store still writes one tile, and it computes both tensors exactly.
//!   2. A second store collapsed onto the first item's tile, or stored at the result's width, is refused by name
//!      (`OutputOverlap` on the second tensor).
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_second_output_lock`
//!
//! ★ MUTATION PROBES (each run 2026-09-27 in a private copy — break, observe red; logs `logs/sn50/probe_second_*.txt`):
//!   • placement reshapes every store to the item's block → the second output is stored two tiles wide:
//!     `OutputOverlap` on `s`, the section no longer verifies — both tests red;
//!   • the validator covers every tensor by the item's rectangle → the same `OutputOverlap`, at the template — both red.
//!   (A validator that admits a tile stored twice is caught by `spatial_moe_real_width_lock`, whose collapsed chunks
//!   write one tile exactly twice; here three items collapse onto one tile.)
//! ════════════════════════════════════════════════════════════════════════════════════════════════
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Value};
use logicaffeine_compile::spatial::ir::*;
use logicaffeine_compile::spatial::passes::from_tensor_op::{
    const_map, ident_map, per_row_map, Template,
};
use logicaffeine_compile::spatial::passes::{LV_GEN_COL, LV_GEN_ROW, LV_ITEM_Y};
use logicaffeine_compile::spatial::pipeline::run_passes;
use logicaffeine_compile::spatial::target::SpatialTarget;

const ITEMS: usize = 3;

fn val(p: usize) -> f64 {
    (((p * 7919 + 13) % 33) as f64 - 16.0) / 8.0
}

/// `r = x + 1` (the result, two tiles per item) and `s = rowsum(x)` across each row (one tile per item).
fn two_outputs(target: &SpatialTarget) -> (SpatialProgram, Vec<f64>, Vec<f64>) {
    let t = target.tile.rows;
    let dt = Precision::Fp32;
    let mut tp = Template::new(target).expect("a template");
    let x: Vec<f64> = (0..ITEMS * t * 2 * t).map(val).collect();
    let xt = tp.tensor(
        "x",
        TensorRole::Input,
        ITEMS * t,
        2 * t,
        dt,
        TensorInit::FloatValues(x.iter().map(|v| v.to_bits()).collect()),
    );
    let rt = tp.tensor(
        "r",
        TensorRole::Output,
        ITEMS * t,
        2 * t,
        dt,
        TensorInit::Zero,
    );
    let st = tp.tensor("s", TensorRole::Output, ITEMS * t, t, dt, TensorInit::Zero);
    let ones = tp.tensor(
        "ones",
        TensorRole::Param,
        t,
        t,
        dt,
        TensorInit::ConstF(1.0f64.to_bits()),
    );
    let x_in = tp.buffer("in_x", 2, dt);
    let one_in = tp.buffer_deep("in_ones", 1, 1, dt);
    let r_out = tp.buffer("out_r", 2, dt);
    let s_out = tp.buffer("out_s", 1, dt);
    let lx = tp.load_block_at("load_x", xt, 0, vec![(LV_ITEM_Y, 2)], 1, 2, 2, x_in);
    let lo = tp.load_block_at("load_ones", ones, 0, Vec::new(), 1, 1, 1, one_in);
    let iters = |cols: usize, reduce: bool| {
        vec![
            IterSpec {
                var: LV_GEN_ROW,
                extent: t,
                kind: IterKind::Parallel,
            },
            IterSpec {
                var: LV_GEN_COL,
                extent: cols,
                kind: if reduce {
                    IterKind::Reduce {
                        pool: ReducePool::Sum,
                    }
                } else {
                    IterKind::Parallel
                },
            },
        ]
    };
    let s_r = tp.stage(
        "plus_one",
        StageKind::Compute(ComputeOp::Generic {
            iters: iters(2 * t, false),
            ins: vec![ident_map(2 * t)],
            out: ident_map(2 * t),
            body: TensorBody::Add(
                Box::new(TensorBody::Operand(0)),
                Box::new(TensorBody::Const(1.0f64.to_bits())),
            ),
        }),
        vec![x_in],
        vec![r_out],
    );
    let s_s = tp.stage(
        "row_sum",
        StageKind::Compute(ComputeOp::Generic {
            iters: iters(2 * t, true),
            ins: vec![ident_map(2 * t), const_map()],
            out: per_row_map(t),
            body: TensorBody::Operand(0),
        }),
        vec![x_in, one_in],
        vec![s_out],
    );
    let wr = tp.store_block("store_r", rt, 0, vec![(LV_ITEM_Y, 2)], 1, 2, 2, r_out);
    let ws = tp.store_block("store_s", st, 0, vec![(LV_ITEM_Y, 1)], 1, 1, 1, s_out);
    tp.edge(lx, x_in, s_r);
    tp.edge(lo, one_in, s_s);
    tp.edge(s_r, r_out, wr);
    tp.edge(s_s, s_out, ws);
    let items = (0..ITEMS)
        .map(|y| WorkItem {
            id: WorkItemId::new(y),
            coord: (y, 0),
            out: TileRect {
                row0_t: y,
                col0_t: 0,
                h_t: 1,
                w_t: 2,
            },
        })
        .collect();
    let p = tp
        .finish(
            "second_output".to_string(),
            items,
            AccPolicy::F32,
            NumericsClaim::Exact,
            target,
        )
        .expect("the template validates");
    let r: Vec<f64> = x.iter().map(|v| v + 1.0).collect();
    let mut s = vec![0.0; ITEMS * t * t];
    for row in 0..ITEMS * t {
        let sum: f64 = x[row * 2 * t..(row + 1) * 2 * t].iter().sum();
        for c in 0..t {
            s[row * t + c] = sum;
        }
    }
    (p, r, s)
}

fn floats(v: &Value) -> Vec<f64> {
    match v {
        Value::Float(x) => x.clone(),
        Value::Int(x) => x.iter().map(|i| *i as f64).collect(),
    }
}

#[test]
fn a_second_output_keeps_its_shape_through_placement_and_computes() {
    let target = SpatialTarget::sn50_rdu();
    let (p, r, s) = two_outputs(&target);
    let v =
        run_passes(p, &target).unwrap_or_else(|e| panic!("the two-output section verifies: {e:?}"));
    let id = |name: &str| v.tensors.iter().find(|t| t.name == name).expect(name).id;
    let (rt, st) = (id("r"), id("s"));
    for stage in &v.stages {
        if let StageKind::Store {
            tensor, pattern, ..
        } = &stage.kind
        {
            let want = if *tensor == rt {
                (1, 2)
            } else if *tensor == st {
                (1, 1)
            } else {
                continue;
            };
            assert_eq!(
                (pattern.block_h, pattern.block_w),
                want,
                "{}: a store keeps the shape of what it writes",
                stage.name
            );
        }
    }
    let out = eval_choreography(&v, &Inputs::default())
        .unwrap_or_else(|e| panic!("the section evaluates: {e:?}"));
    assert_eq!(floats(&out.tensors[&rt]), r, "the result");
    assert_eq!(floats(&out.tensors[&st]), s, "the second output");
}

#[test]
fn a_misaddressed_second_store_is_refused_by_name() {
    let target = SpatialTarget::sn50_rdu();
    let (p, _, _) = two_outputs(&target);
    let v = run_passes(p, &target).expect("the two-output section verifies");
    let st = v.tensors.iter().find(|t| t.name == "s").expect("s").id;
    let misaddressed = |edit: &dyn Fn(&mut AccessPattern)| {
        let mut bad = v.clone();
        for stage in bad.stages.iter_mut() {
            if let StageKind::Store {
                tensor, pattern, ..
            } = &mut stage.kind
            {
                if *tensor == st {
                    edit(pattern);
                }
            }
        }
        bad.validate()
    };
    let collapsed = misaddressed(&|pat| pat.origin.steps.clear());
    assert!(
        matches!(collapsed, Err(IrViolation::OutputOverlap { tensor, .. }) if tensor == st),
        "every item storing the first item's tile: {collapsed:?}"
    );
    let widened = misaddressed(&|pat| pat.block_w = 2);
    assert!(
        matches!(widened, Err(IrViolation::OutputOverlap { tensor, .. }) if tensor == st),
        "the second output stored at the result's width: {widened:?}"
    );
}
