//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! ALGEBRA EXPANSION LOCK (B-P3) — the primitives the closed taxonomy genuinely lacked, each held
//! to a reference written out LONGHAND, never to what the compiler computed before.
//!
//! `spatial_algebra_lock` (in `logicaffeine-compile`) closes the taxonomy: every primitive is
//! classified and evaluated. This file pins what each new primitive MEANS, on hand-built programs
//! whose only novelty is the primitive under test:
//!
//!   · a flat-affine, SIGNED access map — `base + Σ var·stride` over the operand's row-major page,
//!     so a transpose is a map (row stride 1, column stride W), a shifted window is a base, and an
//!     out-of-page read is 0.0 rather than a fault (an unselected `Select` branch may point outside);
//!   · a reduction over WHICHEVER axis is marked — a column-reduce writes one scalar per row across
//!     the tile, a row-reduce writes one scalar per column down the tile (the symmetric convention);
//!   · `ReducePool::ArgMax` — the first maximal index wins (the VM's `>` scan), output `Int64`;
//!   · `TensorBody::Select` on an axis predicate whose bound is a constant OR read from an operand;
//!   · `SfpuKind::Ln`, and the GDN decay `(1+e^u)^R = exp(R·ln(1+e^u))` composed from it;
//!   · `IndirectTerm` / `IndirectTile` with their B-P5 SEMANTICS: a gather COMPILED by the passes certifies, the
//!     evaluator reads the operand at the index the data names, and an index outside its extent or bound faults —
//!     never wraps;
//!   · `TensorRole::{Activation, State}` and `TensorSpace::Sram` as TYPES: refused by `validate_tensors`
//!     naming the phase (B-P7 / B-P6) that admits them.
//!
//! And the printers: for each new feature, Tensix / fabric / CUDA either REFUSE BY NAME (a reason that passes the
//! totality lock's refusal bar, copied here verbatim and locked equal to the original) or lower it to exactly what the
//! evaluator computes — the fabric's RTL under iverilog, every output the evaluator's in the twin's cycles — and the
//! RDU lane, which ships the IR itself, must lower every one of them, the compiled gather included, with a certificate
//! that rechecks against it.
//!
//! ★ HOW THE PROGRAMS ARE BUILT. Through the SAME `Template` builder `from_tensor_op.rs` builds every
//! recognized workload with (made `pub` for this lock and for the B-P4 compositions) — one tile-row
//! per tensor, one Load per operand, one `Generic` stage, one Store — then `run_passes` to Verified.
//! Nothing here reads the compiler's answer back as the reference: every expected value is computed
//! in this file from the definition in the test's doc comment.
//!
//! ★ MUTATION PROBE: in `eval.rs`'s `read_flat`, drop `base` from the offset → `flat_affine_maps_read_the_page_row_major`
//!   goes red on the negative-base case (run 2026-09-18, red, restored).
//! ★ MUTATION PROBE: in `eval.rs`, fold the column axis when the ROW axis is the one marked `Reduce` →
//!   `a_reduction_folds_whichever_axis_is_marked` goes red (run 2026-09-18, red, restored).
//! ★ MUTATION PROBE: in `eval.rs`'s ArgMax step use `>=` instead of `>` → `argmax_returns_the_first_maximal_index`
//!   goes red on the tie rows (run 2026-09-18, red, restored).
//! ★ MUTATION PROBE: in `eval.rs`, evaluate `Select` as its `then` branch alone →
//!   `select_is_piecewise_along_an_axis_and_may_read_an_operand_bound` goes red; `dropping_the_axis_predicate_changes_the_answer`
//!   is the in-file witness that the predicate is load-bearing (run 2026-09-18, red, restored).
//! ★ MUTATION PROBE: in `eval.rs`, `SfpuKind::Ln => v.log2()` → `ln_is_the_natural_log_and_decay_composes_from_it`
//!   goes red (run 2026-09-18, red, restored).
//! ★ MUTATION PROBE: delete the canonical-map guard (`broadcast_class(..).is_none()`) in BOTH
//!   `tensix/kernel.rs`'s `Generic` arm AND `tensix/host.rs`'s golden → `every_printer_refuses_each_new_feature_by_name_or_lowers_it_to_the_evaluator_s_answer`
//!   goes red on the Tensix × transpose cell: "the printer LOWERED a program with a feature this lock cannot hold to
//!   the evaluator" (run 2026-09-18, red, restored). Deleting the KERNEL's guard alone stays green — the host golden
//!   refuses the same map on its own, which is the two-guard depth this lock wants, not a gap in it.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_algebra_expansion_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "common/fabric_rtl.rs"]
mod fabric_rtl;
#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::spatial::certificate::certify_choreography;
use logicaffeine_compile::spatial::eval::{eval_choreography, ChoreoFault, Inputs, Value};
use logicaffeine_compile::spatial::ir::*;
use logicaffeine_compile::spatial::lower::{
    cuda::CudaPersistentBackend, fabric::FabricBackend, rdu::RduSpatialBackend,
    tensix::TensixBackend, SpatialArtifact, SpatialBackend, SpatialLowerError,
};
use logicaffeine_compile::spatial::passes::from_tensor_op::{
    const_map, ident_map, per_col_map, per_row_map, Template,
};
use logicaffeine_compile::spatial::passes::{SpatialReject, LV_GEN_COL, LV_GEN_ROW};
use logicaffeine_compile::spatial::pipeline::{compile_program_to_spatial, run_passes};
use logicaffeine_compile::spatial::target::{precision_is_int, SpatialTarget};

// ══════════════════════════════════════════════════════════════════════════════════════════════════
// Building a one-stage program — the only compiler API these tests use to CONSTRUCT
// ══════════════════════════════════════════════════════════════════════════════════════════════════

/// One operand: a tensor one tile tall and `cols` wide, with its deterministic fill.
struct In {
    name: &'static str,
    cols: usize,
    dtype: Precision,
    init: TensorInit,
}

/// The output: one tile tall, `cols` wide.
struct Out {
    cols: usize,
    dtype: Precision,
}

fn f(vals: &[f64]) -> TensorInit {
    TensorInit::FloatValues(vals.iter().map(|v| v.to_bits()).collect())
}

fn parallel(var: LoopVar, extent: usize) -> IterSpec {
    IterSpec {
        var,
        extent,
        kind: IterKind::Parallel,
    }
}

fn reduce(var: LoopVar, extent: usize, pool: ReducePool) -> IterSpec {
    IterSpec {
        var,
        extent,
        kind: IterKind::Reduce { pool },
    }
}

fn op(i: usize) -> Box<TensorBody> {
    Box::new(TensorBody::Operand(i))
}

fn konst(v: f64) -> Box<TensorBody> {
    Box::new(TensorBody::Const(v.to_bits()))
}

/// A template with ONE `Generic` stage over `ins`, writing `out`; every tensor one tile-row, one
/// work item covering the output's tile-row. This is how `from_tensor_op` builds a stage, minus the
/// recognizer.
fn one_generic(
    target: &SpatialTarget,
    name: &str,
    ins: &[In],
    out: Out,
    iters: Vec<IterSpec>,
    maps: Vec<ElemAccess>,
    omap: ElemAccess,
    body: TensorBody,
) -> SpatialProgram {
    let t = target.tile.rows;
    let mut tp = Template::new(target).expect("a template builder");
    let mut loads = Vec::new();
    let mut bufs = Vec::new();
    for i in ins {
        let id = tp.tensor(
            i.name,
            TensorRole::Input,
            t,
            i.cols,
            i.dtype,
            i.init.clone(),
        );
        let cols_t = i.cols.div_ceil(t);
        let b = tp.buffer(&format!("in_{}", i.name), cols_t, i.dtype);
        let l = tp.load(&format!("load_{}", i.name), id, vec![], cols_t, b);
        loads.push((l, b));
        bufs.push(b);
    }
    let y = tp.tensor(
        "y",
        TensorRole::Output,
        t,
        out.cols,
        out.dtype,
        TensorInit::Zero,
    );
    let out_ct = out.cols.div_ceil(t);
    let ob = tp.buffer("out", out_ct, out.dtype);
    let g = tp.stage(
        "generic",
        StageKind::Compute(ComputeOp::Generic {
            iters,
            ins: maps,
            out: omap,
            body,
        }),
        bufs,
        vec![ob],
    );
    let st = tp.store("store_y", y, vec![], out_ct, ob);
    for (l, b) in loads {
        tp.edge(l, b, g);
    }
    tp.edge(g, ob, st);
    let items = vec![WorkItem {
        id: WorkItemId::new(0),
        coord: (0, 0),
        out: TileRect {
            row0_t: 0,
            col0_t: 0,
            h_t: 1,
            w_t: out_ct,
        },
    }];
    let acc = if precision_is_int(out.dtype) {
        AccPolicy::I64
    } else {
        AccPolicy::F32
    };
    tp.finish(
        name.to_string(),
        items,
        acc,
        NumericsClaim::Approx {
            reason: "a hand-built B-P3 probe".into(),
        },
        target,
    )
    .unwrap_or_else(|e| {
        panic!(
            "{name} on {}: the template must validate: {e:?}",
            target.name
        )
    })
}

fn verified(target: &SpatialTarget, p: SpatialProgram) -> SpatialProgram {
    let name = p.name.clone();
    run_passes(p, target)
        .unwrap_or_else(|e| panic!("{name} on {}: the passes must verify: {e:?}", target.name))
}

fn output_of(p: &SpatialProgram) -> Value {
    let out = p
        .tensors
        .iter()
        .find(|t| t.role == TensorRole::Output)
        .expect("an output")
        .id;
    let o = eval_choreography(p, &Inputs::default())
        .unwrap_or_else(|e| panic!("{}: the choreography must evaluate: {e:?}", p.name));
    o.tensors[&out].clone()
}

fn floats(v: &Value) -> Vec<f64> {
    match v {
        Value::Float(x) => x.clone(),
        Value::Int(x) => x.iter().map(|i| *i as f64).collect(),
    }
}

/// `x[r][c] = r·W + c` — every element distinct and exactly representable, so a wrong read is a
/// wrong NUMBER rather than a coincidence.
fn ramp(rows: usize, cols: usize) -> Vec<f64> {
    (0..rows * cols).map(|p| p as f64).collect()
}

/// The reference the flat-affine contract is stated against: a page is a `rows × width` row-major
/// plane; a signed offset outside it reads 0.0. Written here, from the definition, not from `eval.rs`.
fn read_flat_reference(page: &[f64], rows: usize, width: usize, off: i64) -> f64 {
    if off < 0 || off >= (rows * width) as i64 {
        0.0
    } else {
        page[off as usize]
    }
}

// ══════════════════════════════════════════════════════════════════════════════════════════════════
// 1. Flat-affine, signed maps
// ══════════════════════════════════════════════════════════════════════════════════════════════════

/// `Y[r][c] = X[offset(r, c)]` with `offset = base + r·s_r + c·s_c` over X's row-major page.
///   · identity `(s_r, s_c) = (W, 1)` reproduces X;
///   · swapped strides `(1, W)` read `X[c][r]` — the transpose, as a MAP, not an operator;
///   · a negative base reads 0.0 at the one point that falls before the page and the flat
///     predecessor everywhere else (a base is an offset into the PAGE, not into the row);
///   · a base past the end reads 0.0 at the last element.
#[test]
fn flat_affine_maps_read_the_page_row_major() {
    let wh = SpatialTarget::wormhole();
    let t = wh.tile.rows;
    let w = t;
    let x = ramp(t, w);
    let space = || vec![parallel(LV_GEN_ROW, t), parallel(LV_GEN_COL, w)];
    let program = |name: &str, map: ElemAccess| -> SpatialProgram {
        verified(
            &wh,
            one_generic(
                &wh,
                name,
                &[In {
                    name: "x",
                    cols: w,
                    dtype: Precision::Fp32,
                    init: f(&x),
                }],
                Out {
                    cols: w,
                    dtype: Precision::Fp32,
                },
                space(),
                vec![map],
                ident_map(w),
                TensorBody::Operand(0),
            ),
        )
    };

    let identity = floats(&output_of(&program("identity", ident_map(w))));
    assert_eq!(
        identity, x,
        "the identity map (row stride W, column stride 1) must reproduce X exactly"
    );

    let transposed = floats(&output_of(&program(
        "transpose",
        ElemAccess {
            base: 0,
            terms: vec![(LV_GEN_ROW, 1), (LV_GEN_COL, w as i64)],
            indirect: None,
        },
    )));
    for r in 0..t {
        for c in 0..w {
            assert_eq!(
                transposed[r * w + c],
                x[c * w + r],
                "swapped strides must read X[c][r] at ({r},{c})"
            );
        }
    }
    assert_ne!(
        transposed, identity,
        "the transpose of a ramp differs from the ramp — the map is load-bearing"
    );

    let shifted = floats(&output_of(&program(
        "shift_back",
        ElemAccess {
            base: -1,
            terms: vec![(LV_GEN_ROW, w as i64), (LV_GEN_COL, 1)],
            indirect: None,
        },
    )));
    for r in 0..t {
        for c in 0..w {
            let off = (r * w + c) as i64 - 1;
            assert_eq!(
                shifted[r * w + c],
                read_flat_reference(&x, t, w, off),
                "base -1 at ({r},{c}) must read flat offset {off}"
            );
        }
    }
    assert_eq!(
        shifted[0], 0.0,
        "the first element sits before the page under base -1 and must read 0.0, never fault"
    );
    assert_eq!(
        shifted[w],
        x[w - 1],
        "row 1, column 0 under base -1 is the LAST element of row 0 — the page is flat"
    );

    let shifted_fwd = floats(&output_of(&program(
        "shift_fwd",
        ElemAccess {
            base: 1,
            terms: vec![(LV_GEN_ROW, w as i64), (LV_GEN_COL, 1)],
            indirect: None,
        },
    )));
    assert_eq!(
        shifted_fwd[t * w - 1],
        0.0,
        "the last element sits past the page under base +1 and must read 0.0"
    );
    assert_eq!(shifted_fwd[0], x[1], "base +1 reads the flat successor");
}

// ══════════════════════════════════════════════════════════════════════════════════════════════════
// 2. Axis-general reduction
// ══════════════════════════════════════════════════════════════════════════════════════════════════

/// The symmetric output convention, stated from the definition:
///   · column-reduce (fold `c`): `Y[r][*] = pool_c X[r][c]` — one scalar per row, written ACROSS the
///     whole output tile, so a per-row broadcast reads it back from column 0;
///   · row-reduce (fold `r`): `Y[*][c] = pool_r X[r][c]` — one scalar per column, written DOWN the
///     tile, so a per-column broadcast reads it back from row 0;
///   · both folded: one scalar over every element, written to the whole tile.
/// An output map that varies along a reduced axis is refused, naming the reduced axis.
#[test]
fn a_reduction_folds_whichever_axis_is_marked() {
    let wh = SpatialTarget::wormhole();
    let t = wh.tile.rows;
    let w = 2 * t;
    // Distinct, sign-varied values so a max is never an accident of ordering: x[r][c] = (r·W + c) − 40·(c % 3).
    let x: Vec<f64> = (0..t * w)
        .map(|p| p as f64 - 40.0 * ((p % w) % 3) as f64)
        .collect();
    let build = |name: &str, iters: Vec<IterSpec>, omap: ElemAccess, out_cols: usize| -> Vec<f64> {
        floats(&output_of(&verified(
            &wh,
            one_generic(
                &wh,
                name,
                &[In {
                    name: "x",
                    cols: w,
                    dtype: Precision::Fp32,
                    init: f(&x),
                }],
                Out {
                    cols: out_cols,
                    dtype: Precision::Fp32,
                },
                iters,
                vec![ident_map(w)],
                omap,
                TensorBody::Operand(0),
            ),
        )))
    };
    let sum = |it: &mut dyn Iterator<Item = f64>| it.fold(0.0f64, |a, v| a + v);
    let max = |it: &mut dyn Iterator<Item = f64>| it.fold(f64::NEG_INFINITY, |a, v| a.max(v));

    // Column-reduce: one scalar per row, across the tile.
    for (pool, fold) in [
        (
            ReducePool::Sum,
            &sum as &dyn Fn(&mut dyn Iterator<Item = f64>) -> f64,
        ),
        (ReducePool::Max, &max),
    ] {
        let y = build(
            &format!("col_{pool:?}"),
            vec![parallel(LV_GEN_ROW, t), reduce(LV_GEN_COL, w, pool)],
            per_row_map(t),
            t,
        );
        for r in 0..t {
            let expect = fold(&mut (0..w).map(|c| x[r * w + c]));
            for c in 0..t {
                assert_eq!(y[r * t + c], expect, "column-{pool:?} at row {r}: every column of the output tile carries the row's fold");
            }
        }
    }
    // Row-reduce: one scalar per column, down the tile.
    for (pool, fold) in [
        (
            ReducePool::Sum,
            &sum as &dyn Fn(&mut dyn Iterator<Item = f64>) -> f64,
        ),
        (ReducePool::Max, &max),
    ] {
        let y = build(
            &format!("row_{pool:?}"),
            vec![reduce(LV_GEN_ROW, t, pool), parallel(LV_GEN_COL, w)],
            per_col_map(),
            w,
        );
        for c in 0..w {
            let expect = fold(&mut (0..t).map(|r| x[r * w + c]));
            for r in 0..t {
                assert_eq!(
                    y[r * w + c],
                    expect,
                    "row-{pool:?} at column {c}: every row of the output carries the column's fold"
                );
            }
        }
    }
    // Both folded: the grand total over every element, on the whole tile.
    let y = build(
        "both_sum",
        vec![
            reduce(LV_GEN_ROW, t, ReducePool::Sum),
            reduce(LV_GEN_COL, w, ReducePool::Sum),
        ],
        const_map(),
        t,
    );
    let total = sum(&mut x.iter().copied());
    assert!(
        y.iter().all(|v| *v == total),
        "both axes folded: every element of the output tile is the grand total {total}"
    );

    // The refusal: an output that varies along the folded axis names the reduced axis.
    let bad = verified(
        &wh,
        one_generic(
            &wh,
            "row_sum_bad_out",
            &[In {
                name: "x",
                cols: w,
                dtype: Precision::Fp32,
                init: f(&x),
            }],
            Out {
                cols: w,
                dtype: Precision::Fp32,
            },
            vec![
                reduce(LV_GEN_ROW, t, ReducePool::Sum),
                parallel(LV_GEN_COL, w),
            ],
            vec![ident_map(w)],
            ident_map(w),
            TensorBody::Operand(0),
        ),
    );
    match eval_choreography(&bad, &Inputs::default()) {
        Err(ChoreoFault::Unsupported(reason)) => assert!(
            reason.contains("reduced axis"),
            "the refusal must name the reduced axis, got: {reason}"
        ),
        other => {
            panic!("an output map varying along a reduced axis must be refused, got {other:?}")
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════════════════════════
// 3. ArgMax
// ══════════════════════════════════════════════════════════════════════════════════════════════════

/// `argmax` = the FIRST index at which the maximum occurs — the VM's scan `if v > best { best = v; idx = i }`,
/// so ties go to the lowest index. Column-argmax yields one index per row; row-argmax one per column.
/// The output is `Int64`: an index is not a value.
#[test]
fn argmax_returns_the_first_maximal_index() {
    let wh = SpatialTarget::wormhole();
    let t = wh.tile.rows;
    let w = 2 * t;
    // Row r: the maximum 9.0 planted at columns {r % w, (r + 40) % w} (a TIE), everything else below it,
    // with a decoy runner-up 8.5 after the second copy so "last max" and "any max" both differ.
    let mut x = vec![0.0f64; t * w];
    for r in 0..t {
        for c in 0..w {
            x[r * w + c] = ((r * 7 + c * 3) % 11) as f64 * 0.5;
        }
        let first = r % w;
        let second = (r + 40) % w;
        x[r * w + first] = 9.0;
        x[r * w + second] = 9.0;
        x[r * w + (second + 1) % w] = 8.5;
    }
    let expect_col: Vec<usize> = (0..t)
        .map(|r| {
            (0..w).fold(0usize, |best, c| {
                if x[r * w + c] > x[r * w + best] {
                    c
                } else {
                    best
                }
            })
        })
        .collect();
    let y = one_generic(
        &wh,
        "argmax_cols",
        &[In {
            name: "x",
            cols: w,
            dtype: Precision::Fp32,
            init: f(&x),
        }],
        Out {
            cols: t,
            dtype: Precision::Int64,
        },
        vec![
            parallel(LV_GEN_ROW, t),
            reduce(LV_GEN_COL, w, ReducePool::ArgMax),
        ],
        vec![ident_map(w)],
        per_row_map(t),
        TensorBody::Operand(0),
    );
    let out = output_of(&verified(&wh, y));
    let Value::Int(idx) = &out else {
        panic!("argmax must produce an Int64 tensor, got {out:?}")
    };
    for r in 0..t {
        assert_eq!(
            idx[r * t] as usize,
            expect_col[r],
            "row {r}: the FIRST maximal column (ties to the lowest index)"
        );
        assert_eq!(
            idx[r * t],
            idx[r * t + t - 1],
            "row {r}: the index is written across the whole output tile"
        );
        assert_eq!(
            expect_col[r],
            r.min((r + 40) % w).min(r % w),
            "the planted tie: the lower of the two planted columns wins"
        );
    }
    // Row-argmax: one index per column, down the tile.
    let expect_row: Vec<usize> = (0..w)
        .map(|c| {
            (0..t).fold(0usize, |best, r| {
                if x[r * w + c] > x[best * w + c] {
                    r
                } else {
                    best
                }
            })
        })
        .collect();
    let y = one_generic(
        &wh,
        "argmax_rows",
        &[In {
            name: "x",
            cols: w,
            dtype: Precision::Fp32,
            init: f(&x),
        }],
        Out {
            cols: w,
            dtype: Precision::Int64,
        },
        vec![
            reduce(LV_GEN_ROW, t, ReducePool::ArgMax),
            parallel(LV_GEN_COL, w),
        ],
        vec![ident_map(w)],
        per_col_map(),
        TensorBody::Operand(0),
    );
    let out = output_of(&verified(&wh, y));
    let Value::Int(idx) = &out else {
        panic!("row argmax must produce an Int64 tensor, got {out:?}")
    };
    for c in 0..w {
        assert_eq!(
            idx[c] as usize, expect_row[c],
            "column {c}: the FIRST maximal row"
        );
        assert_eq!(
            idx[c],
            idx[(t - 1) * w + c],
            "column {c}: the index is written down the whole tile"
        );
    }
    // The structural half: an ArgMax whose output is not an integer buffer is a validation error.
    let bad = one_generic(
        &wh,
        "argmax_float_out",
        &[In {
            name: "x",
            cols: w,
            dtype: Precision::Fp32,
            init: f(&x),
        }],
        Out {
            cols: t,
            dtype: Precision::Int64,
        },
        vec![
            parallel(LV_GEN_ROW, t),
            reduce(LV_GEN_COL, w, ReducePool::ArgMax),
        ],
        vec![ident_map(w)],
        per_row_map(t),
        TensorBody::Operand(0),
    );
    let mut q = bad.clone();
    let ob = q
        .stages
        .iter()
        .find(|s| matches!(s.kind, StageKind::Compute(_)))
        .unwrap()
        .outs[0];
    q.buffers[ob.idx()].dtype = Precision::Fp32;
    assert!(
        matches!(
            q.validate(),
            Err(IrViolation::ArgMaxOutputNotInteger { .. })
        ),
        "an index-valued fold into a float buffer must be refused: {:?}",
        q.validate()
    );
}

/// A page holds a tile's rows: a point's row lands at tile row `r mod rows`, so a `Generic` whose parallel row axis is
/// longer than the tile writes two rows into one place. The validator refuses it, naming the rows and the tile's, on
/// the program every backend lowers — not first where an evaluator meets it.
#[test]
fn a_generic_with_more_parallel_rows_than_a_tile_is_refused() {
    let wh = SpatialTarget::wormhole();
    let (t, w) = (wh.tile.rows, wh.tile.cols);
    let copy = one_generic(
        &wh,
        "rows_of_a_tile",
        &[In { name: "x", cols: w, dtype: Precision::Fp32, init: TensorInit::Zero }],
        Out { cols: w, dtype: Precision::Fp32 },
        vec![parallel(LV_GEN_ROW, t), parallel(LV_GEN_COL, w)],
        vec![ident_map(w)],
        ident_map(w),
        TensorBody::Operand(0),
    );
    assert!(copy.validate().is_ok(), "a tile's rows: {:?}", copy.validate());
    let mut longer = copy.clone();
    for s in &mut longer.stages {
        if let StageKind::Compute(ComputeOp::Generic { iters, .. }) = &mut s.kind {
            iters[0].extent = t + 1;
        }
    }
    match longer.validate() {
        Err(IrViolation::GenericRowsPastTile { rows, tile_rows, .. }) => assert_eq!((rows, tile_rows), (t + 1, t)),
        other => panic!("{} parallel rows on a {t}-row tile: {other:?}", t + 1),
    }
    // A folded row axis holds no rows of its own: as long as it likes.
    let mut folded = copy.clone();
    for s in &mut folded.stages {
        if let StageKind::Compute(ComputeOp::Generic { iters, out, .. }) = &mut s.kind {
            iters[0] = reduce(LV_GEN_ROW, 4 * t, ReducePool::Sum);
            *out = per_col_map();
        }
    }
    assert!(folded.validate().is_ok(), "{} folded rows: {:?}", 4 * t, folded.validate());
}

// ══════════════════════════════════════════════════════════════════════════════════════════════════
// 4. Select
// ══════════════════════════════════════════════════════════════════════════════════════════════════

/// `Select { cond, then, else }` = `cond(r, c) ? then : else` at every point, where `cond` is
/// `axis < bound` or `axis >= bound` and the bound is a constant or an operand's value at that point.
///   · `AxisLt(COL, Const 2)`: columns 0 and 1 take the then-branch, the rest the else-branch;
///   · `AxisLt(ROW, Operand 2)` with operand 2 a constant page holding 3.0: rows 0..3 take the
///     then-branch, rows ≥ 3 the else-branch — data-dependent, branch-free;
///   · `AxisGe` is the complement.
#[test]
fn select_is_piecewise_along_an_axis_and_may_read_an_operand_bound() {
    let wh = SpatialTarget::wormhole();
    let t = wh.tile.rows;
    let w = t;
    let x = ramp(t, w);
    let z: Vec<f64> = x.iter().map(|v| -v - 1000.0).collect();
    let bound = vec![3.0f64; t * w];

    let y = one_generic(
        &wh,
        "select_col_const",
        &[In {
            name: "x",
            cols: w,
            dtype: Precision::Fp32,
            init: f(&x),
        }],
        Out {
            cols: w,
            dtype: Precision::Fp32,
        },
        vec![parallel(LV_GEN_ROW, t), parallel(LV_GEN_COL, w)],
        vec![ident_map(w)],
        ident_map(w),
        TensorBody::Select {
            cond: Pred::AxisLt(LV_GEN_COL, PredRhs::Const(2)),
            then: op(0),
            otherwise: Box::new(TensorBody::Mul(op(0), konst(2.0))),
        },
    );
    let out = floats(&output_of(&verified(&wh, y)));
    for r in 0..t {
        for c in 0..w {
            let expect = if c < 2 {
                x[r * w + c]
            } else {
                2.0 * x[r * w + c]
            };
            assert_eq!(out[r * w + c], expect, "AxisLt(COL, 2) at ({r},{c})");
        }
    }

    let ins = [
        In {
            name: "x",
            cols: w,
            dtype: Precision::Fp32,
            init: f(&x),
        },
        In {
            name: "z",
            cols: w,
            dtype: Precision::Fp32,
            init: f(&z),
        },
        In {
            name: "bound",
            cols: w,
            dtype: Precision::Fp32,
            init: f(&bound),
        },
    ];
    let y = one_generic(
        &wh,
        "select_row_operand",
        &ins,
        Out {
            cols: w,
            dtype: Precision::Fp32,
        },
        vec![parallel(LV_GEN_ROW, t), parallel(LV_GEN_COL, w)],
        vec![ident_map(w), ident_map(w), const_map()],
        ident_map(w),
        TensorBody::Select {
            cond: Pred::AxisLt(LV_GEN_ROW, PredRhs::Operand(2)),
            then: op(0),
            otherwise: op(1),
        },
    );
    let out = floats(&output_of(&verified(&wh, y)));
    for r in 0..t {
        for c in 0..w {
            let expect = if r < 3 { x[r * w + c] } else { z[r * w + c] };
            assert_eq!(
                out[r * w + c],
                expect,
                "AxisLt(ROW, Operand(2) = 3.0) at ({r},{c}): rows ≥ 3 take the else branch"
            );
        }
    }

    let y = one_generic(
        &wh,
        "select_row_ge",
        &ins,
        Out {
            cols: w,
            dtype: Precision::Fp32,
        },
        vec![parallel(LV_GEN_ROW, t), parallel(LV_GEN_COL, w)],
        vec![ident_map(w), ident_map(w), const_map()],
        ident_map(w),
        TensorBody::Select {
            cond: Pred::AxisGe(LV_GEN_ROW, PredRhs::Operand(2)),
            then: op(0),
            otherwise: op(1),
        },
    );
    let out = floats(&output_of(&verified(&wh, y)));
    for r in 0..t {
        for c in 0..w {
            let expect = if r >= 3 { x[r * w + c] } else { z[r * w + c] };
            assert_eq!(
                out[r * w + c],
                expect,
                "AxisGe(ROW, Operand(2) = 3.0) at ({r},{c}) is the complement"
            );
        }
    }
}

/// ★ THE PREDICATE IS LOAD-BEARING. The same program with its `Select` replaced by the then-branch
/// alone (and, separately, the else-branch alone) must compute something ELSE — otherwise the test
/// above could pass with an evaluator that ignores the condition.
#[test]
fn dropping_the_axis_predicate_changes_the_answer() {
    let wh = SpatialTarget::wormhole();
    let t = wh.tile.rows;
    let w = t;
    let x = ramp(t, w);
    let build = |name: &str, body: TensorBody| -> Vec<f64> {
        floats(&output_of(&verified(
            &wh,
            one_generic(
                &wh,
                name,
                &[In {
                    name: "x",
                    cols: w,
                    dtype: Precision::Fp32,
                    init: f(&x),
                }],
                Out {
                    cols: w,
                    dtype: Precision::Fp32,
                },
                vec![parallel(LV_GEN_ROW, t), parallel(LV_GEN_COL, w)],
                vec![ident_map(w)],
                ident_map(w),
                body,
            ),
        )))
    };
    let piecewise = build(
        "select",
        TensorBody::Select {
            cond: Pred::AxisLt(LV_GEN_COL, PredRhs::Const(2)),
            then: op(0),
            otherwise: Box::new(TensorBody::Mul(op(0), konst(2.0))),
        },
    );
    let then_only = build("then_only", TensorBody::Operand(0));
    let else_only = build("else_only", TensorBody::Mul(op(0), konst(2.0)));
    assert_ne!(
        piecewise, then_only,
        "a Select evaluated as its then-branch alone must differ"
    );
    assert_ne!(
        piecewise, else_only,
        "a Select evaluated as its else-branch alone must differ"
    );
    assert_eq!(
        &piecewise[..2],
        &then_only[..2],
        "the selected columns agree with the then-branch"
    );
    assert_eq!(
        &piecewise[2..w],
        &else_only[2..w],
        "the unselected columns agree with the else-branch"
    );
}

// ══════════════════════════════════════════════════════════════════════════════════════════════════
// 5. Ln and the decay composition
// ══════════════════════════════════════════════════════════════════════════════════════════════════

/// `Ln` is the natural logarithm: the machine's output is `f32(ln x)` bit-for-bit (the body is
/// evaluated in f64 and narrowed once at the accumulator). The GDN decay `(1+e^u)^R` is composed as
/// `exp(R·ln(1+e^u))` — no `pow` primitive exists, and none is needed: the identity holds to 1e-12
/// relative in f64 (asserted longhand here, without the compiler) and the machine's f32 output sits
/// within two f32 ulps of the longhand power.
#[test]
fn ln_is_the_natural_log_and_decay_composes_from_it() {
    let wh = SpatialTarget::wormhole();
    let t = wh.tile.rows;
    let w = t;
    // u ∈ [−2, 2) in exact eighths; a per-row exponent R ∈ {0.5, 1.5, 3.0, 0.125}.
    let u: Vec<f64> = (0..t * w)
        .map(|p| ((p * 5) % 32) as f64 / 8.0 - 2.0)
        .collect();
    let r_of_row = |r: usize| [0.5f64, 1.5, 3.0, 0.125][r % 4];
    let rr: Vec<f64> = (0..t * w).map(|p| r_of_row(p / w)).collect();
    // Shifted to [1.5, 5.5): away from x = 1, the one point where ln and log2 agree (both 0).
    let positive: Vec<f64> = u.iter().map(|v| v + 3.5).collect();

    let y = one_generic(
        &wh,
        "ln",
        &[In {
            name: "x",
            cols: w,
            dtype: Precision::Fp32,
            init: f(&positive),
        }],
        Out {
            cols: w,
            dtype: Precision::Fp32,
        },
        vec![parallel(LV_GEN_ROW, t), parallel(LV_GEN_COL, w)],
        vec![ident_map(w)],
        ident_map(w),
        TensorBody::Unary {
            kind: SfpuKind::Ln,
            arg: op(0),
        },
    );
    let out = floats(&output_of(&verified(&wh, y)));
    for p in 0..t * w {
        assert_eq!(
            out[p],
            (positive[p].ln() as f32) as f64,
            "ln at flat {p}: f32(ln x) exactly"
        );
        assert_ne!(
            out[p],
            (positive[p].log2() as f32) as f64,
            "ln is not log2 at flat {p}"
        );
    }

    // decay = exp(R · ln(1 + exp(u))), R read per row through a per-row map on a one-tile page.
    let body = TensorBody::Unary {
        kind: SfpuKind::Exp,
        arg: Box::new(TensorBody::Mul(
            op(1),
            Box::new(TensorBody::Unary {
                kind: SfpuKind::Ln,
                arg: Box::new(TensorBody::Add(
                    konst(1.0),
                    Box::new(TensorBody::Unary {
                        kind: SfpuKind::Exp,
                        arg: op(0),
                    }),
                )),
            }),
        )),
    };
    let ins = [
        In {
            name: "u",
            cols: w,
            dtype: Precision::Fp32,
            init: f(&u),
        },
        In {
            name: "r",
            cols: w,
            dtype: Precision::Fp32,
            init: f(&rr),
        },
    ];
    let y = one_generic(
        &wh,
        "decay",
        &ins,
        Out {
            cols: w,
            dtype: Precision::Fp32,
        },
        vec![parallel(LV_GEN_ROW, t), parallel(LV_GEN_COL, w)],
        vec![ident_map(w), per_row_map(w)],
        ident_map(w),
        body,
    );
    let out = floats(&output_of(&verified(&wh, y)));
    for p in 0..t * w {
        let (uu, r) = (u[p], r_of_row(p / w));
        let longhand_power = (1.0 + uu.exp()).powf(r);
        let composed = (r * (1.0 + uu.exp()).ln()).exp();
        assert!((composed - longhand_power).abs() <= 1e-12 * longhand_power.abs(), "f64 identity at flat {p}: exp(R ln(1+e^u)) = (1+e^u)^R within 1e-12 ({composed} vs {longhand_power})");
        let ulp = (longhand_power as f32).abs() * f32::EPSILON as f32;
        assert!((out[p] - longhand_power).abs() <= 2.0 * ulp as f64, "decay at flat {p}: machine {} vs longhand {longhand_power} (R={r}, u={uu}) exceeds 2 f32 ulps", out[p]);
    }
}

// ══════════════════════════════════════════════════════════════════════════════════════════════════
// 6. The printers refuse the new features by name; the RDU lowers them
// ══════════════════════════════════════════════════════════════════════════════════════════════════

// ── The refusal-quality bar ───────────────────────────────────────────────────────────────────────
//
// ★ COPIED VERBATIM FROM `spatial_backend_totality_lock.rs`, and `the_refusal_bar_is_the_totality_locks_verbatim`
// below holds this block byte-equal to the original between the two `// ──` markers, so the two locks
// can never hold a refusal to different bars.

/// Words that carry no information about WHAT was refused. A reason made only of these is a shrug.
const STOPWORDS: &[&str] = &[
    "the", "that", "this", "with", "without", "than", "then", "from", "into", "onto", "over",
    "under", "when", "where", "which", "whose", "while", "because", "only", "also", "each",
    "every", "some", "must", "cannot", "never", "always", "does", "done", "have", "has", "had",
    "been", "being", "its", "their", "them", "they", "there", "here", "what", "would", "could",
    "should", "upon", "about", "above", "below", "after", "before", "between", "beyond", "least",
    "most", "more", "less", "one", "two", "and", "but", "nor", "neither", "either", "both", "not",
    "are", "was", "were", "for", "per", "via", "still", "yet",
];

/// Reasons that are placeholders rather than refusals. A backend that says one of these has told you
/// nothing you did not already know from the fact that it failed.
const PLACEHOLDERS: &[&str] = &[
    "",
    "todo",
    "fixme",
    "tbd",
    "n/a",
    "na",
    "none",
    "unknown",
    "error",
    "unsupported",
    "unimplemented",
    "not supported",
    "not implemented",
    "not yet implemented",
    "no",
    "?",
    "-",
    "--",
    "xxx",
    "wip",
    "unhandled",
    "other",
    "op",
    "invalid",
    "failed",
    "failure",
    "bad",
    "oops",
    "sorry",
    "this workload",
    "this op",
];

/// Substrings that mark unfinished text wherever they appear.
const PLACEHOLDER_MARKERS: &[&str] = &[
    "todo",
    "fixme",
    "unimplemented",
    "not implemented",
    "not yet implemented",
    "xxx",
    "???",
];

/// The IR's own nouns. A refusal must speak at least one of them — that is what "names the op or the
/// shape" means operationally: the reason points at an object the program actually contains, not at
/// the backend's mood. A vocabulary word matches a token exactly, or is a prefix of it when it is at
/// least four characters (so `tile` covers `tiled`/`tiles` and `dequant` covers `dequantize`).
///
/// ★ THIS LIST GROWS, IT NEVER SHRINKS. A new refusal whose noun is missing here is a signal to add
/// the noun, not to soften the refusal or delete the check.
const IR_VOCABULARY: &[&str] = &[
    // Program objects.
    "tensor",
    "stage",
    "buffer",
    "edge",
    "semaphore",
    "handshake",
    "schedule",
    "placement",
    "site",
    "engine",
    "kernel",
    "program",
    "tile",
    "page",
    "block",
    "plan",
    "grid",
    "item",
    "work",
    "loop",
    "phase",
    "group",
    "barrier",
    "multicast",
    "mcast",
    "fifo",
    "circular",
    "operand",
    "fill",
    "external",
    "range",
    "stride",
    "lattice",
    "memory",
    "register",
    "descriptor",
    "target",
    "fabric",
    "gpu",
    "rdu",
    "tensix",
    "wmma",
    "noc",
    "shape",
    "layout",
    "space",
    "data",
    "value",
    "counter",
    "certificate",
    "choreography",
    "census",
    // Stage and compute-op names.
    "load",
    "store",
    "recv",
    "compute",
    "matmul",
    "gemm",
    "scalar",
    "elementwise",
    "swiglu",
    "rmsnorm",
    "softmax",
    "layernorm",
    "reduce",
    "row",
    "rescale",
    "broadcast",
    "requant",
    "dequant",
    "quant",
    "bias",
    "relu",
    "pack",
    "accum",
    "spill",
    "epilogue",
    "prologue",
    "oracle",
    "golden",
    "host",
    "serialize",
    "deadlock",
    "align",
    // Numerics.
    "precision",
    "numerics",
    "integer",
    "float",
    "int8",
    "int16",
    "int32",
    "int64",
    "bf16",
    "fp8",
    "fp16",
    "fp32",
    "mantissa",
    "overflow",
    // Roles.
    "input",
    "output",
    "param",
];

/// Hold a refusal to the bar. `Ok(())`, or the RULE it broke — the message names the rule so a fix is
/// obvious from the failure alone.
fn refusal_quality(reason: &str) -> Result<(), String> {
    let trimmed = reason.trim();
    if trimmed.is_empty() {
        return Err("R1: the reason is empty — a refusal must say what it refused".to_string());
    }
    let lower = trimmed.to_ascii_lowercase();
    if PLACEHOLDERS.contains(&lower.as_str()) {
        return Err(format!("R4: `{trimmed}` is a placeholder, not a reason — name the op, the shape or the object the backend could not take"));
    }
    for marker in PLACEHOLDER_MARKERS {
        if lower.contains(*marker) {
            return Err(format!("R4: `{trimmed}` carries the unfinished-text marker `{marker}` — finish the sentence or delete it"));
        }
    }
    let toks = tokens(trimmed);
    if toks.len() < 2 {
        return Err(format!(
            "R2: `{trimmed}` is a single token — a refusal is a phrase, not a label"
        ));
    }
    if !toks
        .iter()
        .any(|t| t.chars().count() >= 4 && !STOPWORDS.contains(&t.as_str()))
    {
        return Err(format!(
            "R3: `{trimmed}` is made only of function words — it names nothing"
        ));
    }
    if !toks.iter().any(|t| names_an_ir_object(t)) {
        return Err(format!(
            "R5: `{trimmed}` names no object of the spatial IR. A refusal must point at what it \
             refused (the op, the shape, the buffer, the precision). If the wording IS specific and \
             the noun is simply missing from `IR_VOCABULARY`, ADD THE NOUN — do not weaken this check"
        ));
    }
    Ok(())
}

fn tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

fn names_an_ir_object(token: &str) -> bool {
    IR_VOCABULARY
        .iter()
        .any(|v| *v == token || (v.len() >= 4 && token.starts_with(*v)))
}

// ── Rendering a refusal ───────────────────────────────────────────────────────────────────────────

/// The block between the two markers above must be the totality lock's, byte for byte.
#[test]
fn the_refusal_bar_is_the_totality_locks_verbatim() {
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let block = |file: &str| -> String {
        let text =
            std::fs::read_to_string(here.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        let start = text
            .find("// ── The refusal-quality bar")
            .unwrap_or_else(|| panic!("{file}: the refusal-bar marker is missing"));
        let end = text
            .find("// ── Rendering a refusal")
            .unwrap_or_else(|| panic!("{file}: the rendering marker is missing"));
        // Skip the marker's own comment paragraph: compare from the first `const`.
        let body = &text[start..end];
        let first_const = body
            .find("/// Words that carry")
            .expect("the STOPWORDS doc comment");
        body[first_const..].to_string()
    };
    let (mine, theirs) = (
        block("spatial_algebra_expansion_lock.rs"),
        block("spatial_backend_totality_lock.rs"),
    );
    assert!(
        mine.len() > 2000,
        "the copied refusal bar is suspiciously short ({} bytes)",
        mine.len()
    );
    assert_eq!(mine, theirs, "the refusal-quality bar in this file drifted from spatial_backend_totality_lock.rs — copy the block again (grow IR_VOCABULARY THERE, then here)");
}

fn lower_reason(e: &SpatialLowerError) -> String {
    match e {
        SpatialLowerError::Unsupported(what) => (*what).to_string(),
        other => format!("{other:?}"),
    }
}

/// Every backend on the target it is built for — the pairing the totality lock uses.
fn backends() -> Vec<(&'static str, Box<dyn SpatialBackend>)> {
    vec![
        (
            "Tensix",
            Box::new(TensixBackend::new(SpatialTarget::wormhole())) as Box<dyn SpatialBackend>,
        ),
        (
            "fabric",
            Box::new(FabricBackend::new(SpatialTarget::sky_fabric(64, 64)))
                as Box<dyn SpatialBackend>,
        ),
        (
            "CUDA",
            Box::new(CudaPersistentBackend::new(SpatialTarget::rtx3060()))
                as Box<dyn SpatialBackend>,
        ),
        (
            "RDU",
            Box::new(RduSpatialBackend::new()) as Box<dyn SpatialBackend>,
        ),
    ]
}

/// The five features that reach a printer as a VERIFIED program, built for `target`'s tile shape.
/// Bodies are the plain two-operand add wherever the feature is a MAP (transpose, indirect), so a
/// printer that ignores maps and pattern-matches the body would lower them — wrongly — and go red.
fn feature_programs(target: &SpatialTarget) -> Vec<(&'static str, SpatialProgram)> {
    let t = target.tile.rows;
    let w = 2 * t;
    let dtype = if target.name.contains("fabric") {
        Precision::Int64
    } else {
        Precision::Fp32
    };
    let vals: Vec<f64> = (0..t * w).map(|p| (p % 7) as f64 + 1.0).collect();
    let init = || {
        if dtype == Precision::Int64 {
            TensorInit::Values(vals.iter().map(|v| *v as i64).collect())
        } else {
            f(&vals)
        }
    };
    let x = || In {
        name: "x",
        cols: w,
        dtype,
        init: init(),
    };
    let z = || In {
        name: "z",
        cols: w,
        dtype,
        init: init(),
    };
    let add = || TensorBody::Add(op(0), op(1));
    let pp = || vec![parallel(LV_GEN_ROW, t), parallel(LV_GEN_COL, w)];
    let strided = ElemAccess {
        base: 0,
        terms: vec![(LV_GEN_ROW, 1), (LV_GEN_COL, w as i64)],
        indirect: None,
    };
    vec![
        (
            "transpose map",
            one_generic(
                target,
                "transpose",
                &[x(), z()],
                Out { cols: w, dtype },
                pp(),
                vec![strided, ident_map(w)],
                ident_map(w),
                add(),
            ),
        ),
        (
            "row reduce",
            one_generic(
                target,
                "row_reduce",
                &[x()],
                Out { cols: w, dtype },
                vec![
                    reduce(LV_GEN_ROW, t, ReducePool::Sum),
                    parallel(LV_GEN_COL, w),
                ],
                vec![ident_map(w)],
                per_col_map(),
                TensorBody::Operand(0),
            ),
        ),
        (
            "argmax",
            one_generic(
                target,
                "argmax",
                &[x()],
                Out {
                    cols: t,
                    dtype: Precision::Int64,
                },
                vec![
                    parallel(LV_GEN_ROW, t),
                    reduce(LV_GEN_COL, w, ReducePool::ArgMax),
                ],
                vec![ident_map(w)],
                per_row_map(t),
                TensorBody::Operand(0),
            ),
        ),
        (
            "select",
            one_generic(
                target,
                "select",
                &[x()],
                Out { cols: w, dtype },
                pp(),
                vec![ident_map(w)],
                ident_map(w),
                TensorBody::Select {
                    cond: Pred::AxisLt(LV_GEN_COL, PredRhs::Const(1)),
                    then: op(0),
                    otherwise: konst(0.0),
                },
            ),
        ),
        (
            "ln",
            one_generic(
                target,
                "ln",
                &[x()],
                Out { cols: w, dtype },
                pp(),
                vec![ident_map(w)],
                ident_map(w),
                TensorBody::Unary {
                    kind: SfpuKind::Ln,
                    arg: op(0),
                },
            ),
        ),
    ]
}

/// The fill of the gather's operand and of its index: `p mod 5`, so every index is in `[0, 5)`.
fn gather_fill(t: usize, w: usize) -> Vec<f64> {
    (0..t * w).map(|p| (p % 5) as f64).collect()
}

/// `y[p] = x[p + idx[p]] + idx[p]` — the add with its first operand GATHERED through the second (index `idx`,
/// stride 1, extent `w`), COMPILED: the indirect term is in the template the passes are given, and the passes
/// certify it themselves.
fn indirect_program(target: &SpatialTarget) -> SpatialProgram {
    let t = target.tile.rows;
    let w = 2 * t;
    let dtype = if target.name.contains("fabric") {
        Precision::Int64
    } else {
        Precision::Fp32
    };
    let vals = gather_fill(t, w);
    let init = || {
        if dtype == Precision::Int64 {
            TensorInit::Values(vals.iter().map(|v| *v as i64).collect())
        } else {
            f(&vals)
        }
    };
    let gathered = ElemAccess {
        indirect: Some(IndirectTerm {
            operand: 1,
            stride: 1,
            modulus: w,
        }),
        ..ident_map(w)
    };
    let p = one_generic(
        target,
        "gather",
        &[
            In {
                name: "x",
                cols: w,
                dtype,
                init: init(),
            },
            In {
                name: "idx",
                cols: w,
                dtype,
                init: init(),
            },
        ],
        Out { cols: w, dtype },
        vec![parallel(LV_GEN_ROW, t), parallel(LV_GEN_COL, w)],
        vec![gathered, ident_map(w)],
        ident_map(w),
        TensorBody::Add(op(0), op(1)),
    );
    let q = verified(target, p);
    let carried = q.stages.iter().any(|s| matches!(&s.kind, StageKind::Compute(ComputeOp::Generic { ins, .. }) if ins[0].indirect.is_some()));
    assert!(
        carried,
        "{}: the compiled program still gathers",
        target.name
    );
    assert!(
        q.certificate.as_ref().is_some_and(|c| c.recheck(&q)),
        "{}: the passes certified the gather, and the certificate rechecks",
        target.name
    );
    q
}

/// A feature a printer lowers is held to the evaluator where the printer's own run is at hand: the fabric's RTL under
/// iverilog. A printer this lock cannot run lowers nothing unheld.
fn lowered_as_the_evaluator(bname: &str, feature: &str, p: &SpatialProgram, target: &SpatialTarget, art: SpatialArtifact) -> String {
    match (bname, art) {
        ("fabric", SpatialArtifact::Verilog(v)) => {
            if common::require_tool("iverilog") {
                fabric_rtl::runs_as_the_evaluator(&format!("fabric × {feature}"), p, target, &v, &spatial_support::inputs_from_fills(p));
                format!("  {bname:<7} × {feature:<14} lower (RTL ≡ evaluator, twin's cycles)")
            } else {
                format!("  {bname:<7} × {feature:<14} lower (RTL unrun: no iverilog)")
            }
        }
        (_, other) => panic!("{bname} × {feature}: the printer LOWERED a program with a feature this lock cannot hold to the evaluator: {other:?}"),
    }
}

#[test]
fn every_printer_refuses_each_new_feature_by_name_or_lowers_it_to_the_evaluator_s_answer() {
    let mut matrix: Vec<String> = Vec::new();
    for (bname, backend) in backends() {
        let target = backend.target().clone();
        for (feature, template) in feature_programs(&target) {
            let p = verified(&target, template);
            let outcome = backend.lower(&p);
            match (bname, outcome) {
                ("RDU", Ok(SpatialArtifact::SdkGated(art))) => {
                    assert!(art.json.contains("\"program\""), "RDU × {feature}: the neutral artifact carries the program");
                    matrix.push(format!("  RDU     × {feature:<14} lower (SdkGated)"));
                }
                ("RDU", other) => panic!("RDU × {feature}: the neutral lane ships the IR and must lower it, got {other:?}"),
                (_, Err(SpatialLowerError::Unsupported(reason))) => {
                    refusal_quality(reason).unwrap_or_else(|rule| panic!("{bname} × {feature}: the refusal `{reason}` fails the bar — {rule}"));
                    matrix.push(format!("  {bname:<7} × {feature:<14} refuse: {reason}"));
                }
                (_, Err(other)) => panic!("{bname} × {feature}: must refuse with `Unsupported(<reason>)`, got {}", lower_reason(&other)),
                (_, Ok(art)) => matrix.push(lowered_as_the_evaluator(bname, feature, &p, &target, art)),
            }
        }
        // The gather, compiled for this backend's own target. The RDU ships it — the neutral artifact carries the
        // gathering program and a certificate that rechecks against it; a printer with no gather refuses by name.
        let g = indirect_program(&target);
        match (bname, backend.lower(&g)) {
            ("RDU", Ok(SpatialArtifact::SdkGated(art))) => {
                assert!(
                    art.json.contains("\"modulus\""),
                    "RDU × indirect: the neutral artifact carries the gather's indirect term"
                );
                assert!(
                    art.certificate.recheck(&g),
                    "RDU × indirect: the shipped certificate rechecks against the compiled gather"
                );
                matrix.push("  RDU     × indirect       lower (SdkGated)".to_string());
            }
            ("RDU", other) => {
                panic!("RDU × indirect: the neutral lane ships a compiled gather, got {other:?}")
            }
            (_, Err(SpatialLowerError::Unsupported(reason))) => {
                refusal_quality(reason).unwrap_or_else(|rule| {
                    panic!("{bname} × indirect: the refusal `{reason}` fails the bar — {rule}")
                });
                // The CUDA lane's general program computes integers and refuses this float gather by its float operand
                // before it reads the access; every other printer names the indirect access itself.
                let named = if bname == "CUDA" {
                    reason.contains("float") && reason.contains("general GPU program")
                } else {
                    reason.to_ascii_lowercase().contains("indirect")
                };
                assert!(named, "{bname} × indirect: the refusal must name what the printer lacks, got: {reason}");
                matrix.push(format!("  {bname:<7} × indirect       refuse: {reason}"));
            }
            (_, Ok(art)) => matrix.push(lowered_as_the_evaluator(bname, "indirect", &g, &target, art)),
            (_, other) => panic!("{bname} × indirect: must refuse by name, got {other:?}"),
        }
    }
    println!("printer × feature:\n{}", matrix.join("\n"));
    assert_eq!(
        matrix.len(),
        4 * 6,
        "the matrix must be full: four backends × six features"
    );
}

// ══════════════════════════════════════════════════════════════════════════════════════════════════
// 7. Indirect access: the B-P5 semantics — a compiled gather certifies, reads where its index points,
//    and faults outside its extent or bound
// ══════════════════════════════════════════════════════════════════════════════════════════════════

#[test]
fn indirect_access_has_its_b_p5_semantics() {
    let wh = SpatialTarget::wormhole();
    let t = wh.tile.rows;
    let w = 2 * t;

    // (a) The data run reads operand 0 at `p + idx[p]` — the index read through its own map at the same point;
    //     an offset past the page reads 0, as every flat map does.
    let g = indirect_program(&wh);
    let v = gather_fill(t, w);
    let want: Vec<f64> = (0..t * w)
        .map(|p| v.get(p + v[p] as usize).copied().unwrap_or(0.0) + v[p])
        .collect();
    assert_eq!(
        floats(&output_of(&g)),
        want,
        "the gather reads where its index points"
    );

    // (b) The certificate is the passes' own: minted for the gathering program, and re-derived from scratch.
    let mut fresh = g.clone();
    fresh.certificate = None;
    assert!(
        certify_choreography(&fresh).is_ok(),
        "certify_choreography certifies the compiled gather directly, too"
    );

    // (c) An index outside its extent faults, naming the index and the extent — never a wrapped read.
    let mut bad = g.clone();
    let mut outside = v.clone();
    outside[7] = w as f64;
    for td in &mut bad.tensors {
        if td.name == "idx" {
            td.init = f(&outside);
        }
    }
    match eval_choreography(&bad, &Inputs::default()) {
        Err(ChoreoFault::GatherOutOfRange { index, extent, .. }) => assert_eq!(
            (index, extent),
            (w as f64, w),
            "the fault names the index and the extent"
        ),
        other => panic!("an index outside its extent must fault, got {other:?}"),
    }

    // (d) A TILE address, compiled from source for the SN50 (where an index has a datatype): the Load reads the
    //     row the data names, and a row past the table's bound faults, naming the row and the bound.
    let src = "## Main\nLet T be a 4 by 64 tensor of Int.\nSet item 129 of T to 7.\nSet item 192 of T to 9.\nLet tok be 3.\nLet x be row tok of T.\nShow item 1 of x.\nShow item 64 of x.\n";
    let pl = compile_program_to_spatial(src, &SpatialTarget::sn50_rdu())
        .expect("the walk lifts the row gather");
    let sp = pl
        .whole()
        .unwrap_or_else(|| panic!("the tile gather compiles: {}", pl.refusal()))
        .clone();
    assert!(sp.stages.iter().any(|s| matches!(&s.kind, StageKind::Load { pattern, .. } if pattern.origin.indirect.is_some())), "the row moves through an indirect tile address");
    let x = sp
        .tensors
        .iter()
        .find(|td| td.name == "x")
        .expect("the gathered row")
        .id;
    match &eval_choreography(&sp, &Inputs::default())
        .expect("the tile gather evaluates")
        .tensors[&x]
    {
        Value::Int(row) => assert_eq!((row[0], row[63]), (7, 9), "row 3 of T, not another row"),
        other => panic!("an Int row, got {other:?}"),
    }
    let mut past = sp.clone();
    for td in &mut past.tensors {
        if td.name == "tok" {
            td.init = TensorInit::Values(vec![5]);
        }
    }
    match eval_choreography(&past, &Inputs::default()) {
        Err(ChoreoFault::TileGatherOutOfBound { index, bound, .. }) => assert_eq!(
            (index, bound),
            (4.0, (0, 3)),
            "the fault names the 0-based row and the table's rows"
        ),
        other => panic!("a row past the table must fault, got {other:?}"),
    }
}

/// The structural half: an indirect term must name an operand the stage has, a modulus that is not
/// zero, and an OUTPUT map may carry one only where the stage's result reaches a scatter Store. The
/// operand-count and body-operand rules are locked alongside, because the evaluator indexes pages by
/// them and a panic there is not a refusal.
#[test]
fn an_indirect_term_without_its_index_operand_is_a_validation_error() {
    let wh = SpatialTarget::wormhole();
    let t = wh.tile.rows;
    let w = t;
    let x = ramp(t, w);
    let base = || {
        one_generic(
            &wh,
            "one_operand",
            &[In {
                name: "x",
                cols: w,
                dtype: Precision::Fp32,
                init: f(&x),
            }],
            Out {
                cols: w,
                dtype: Precision::Fp32,
            },
            vec![parallel(LV_GEN_ROW, t), parallel(LV_GEN_COL, w)],
            vec![ident_map(w)],
            ident_map(w),
            TensorBody::Operand(0),
        )
    };
    let generic_of = |p: &SpatialProgram| {
        p.stages
            .iter()
            .position(|s| matches!(s.kind, StageKind::Compute(_)))
            .unwrap()
    };
    let with_op = |edit: &dyn Fn(&mut ComputeOp)| -> SpatialProgram {
        let mut p = base();
        let i = generic_of(&p);
        if let StageKind::Compute(op) = &mut p.stages[i].kind {
            edit(op);
        }
        p
    };
    let set_in_map = |edit: &dyn Fn(&mut ElemAccess)| {
        with_op(&|op| {
            if let ComputeOp::Generic { ins, .. } = op {
                edit(&mut ins[0])
            }
        })
    };

    let p = set_in_map(&|m| {
        m.indirect = Some(IndirectTerm {
            operand: 1,
            stride: 1,
            modulus: w,
        })
    });
    assert!(
        matches!(
            p.validate(),
            Err(IrViolation::IndirectOperandOutOfRange {
                operand: 1,
                operands: 1,
                ..
            })
        ),
        "an index operand the stage does not have: {:?}",
        p.validate()
    );

    let p = set_in_map(&|m| {
        m.indirect = Some(IndirectTerm {
            operand: 0,
            stride: 1,
            modulus: 0,
        })
    });
    assert!(
        matches!(p.validate(), Err(IrViolation::IndirectMalformed { .. })),
        "a zero modulus: {:?}",
        p.validate()
    );

    let p = with_op(&|op| {
        if let ComputeOp::Generic { out, .. } = op {
            out.indirect = Some(IndirectTerm {
                operand: 0,
                stride: 1,
                modulus: w,
            })
        }
    });
    assert!(
        matches!(
            p.validate(),
            Err(IrViolation::IndirectOutWithoutScatter { .. })
        ),
        "an indirect OUTPUT map with no scatter store behind it: {:?}",
        p.validate()
    );

    let p = with_op(&|op| {
        if let ComputeOp::Generic { ins, .. } = op {
            ins.push(const_map())
        }
    });
    assert!(
        matches!(
            p.validate(),
            Err(IrViolation::GenericOperandCount {
                operands: 1,
                maps: 2,
                ..
            })
        ),
        "more maps than operands: {:?}",
        p.validate()
    );

    let p = with_op(&|op| {
        if let ComputeOp::Generic { body, .. } = op {
            *body = TensorBody::Operand(3)
        }
    });
    assert!(
        matches!(
            p.validate(),
            Err(IrViolation::GenericOperandOutOfRange {
                operand: 3,
                operands: 1,
                ..
            })
        ),
        "a body naming an absent operand: {:?}",
        p.validate()
    );

    let p = with_op(&|op| {
        if let ComputeOp::Generic { body, .. } = op {
            *body = TensorBody::Select {
                cond: Pred::AxisLt(LV_GEN_COL, PredRhs::Operand(2)),
                then: op_(0),
                otherwise: op_(0),
            }
        }
    });
    assert!(
        matches!(
            p.validate(),
            Err(IrViolation::GenericOperandOutOfRange {
                operand: 2,
                operands: 1,
                ..
            })
        ),
        "a predicate naming an absent operand: {:?}",
        p.validate()
    );

    // A tile address: the index buffer must be one of the stage's operands.
    let mut p = base();
    let stray = p.buffers[1].id;
    if let StageKind::Load { pattern, .. } = &mut p.stages[0].kind {
        pattern.origin.indirect = Some(IndirectTile {
            index: stray,
            tile_stride: 1,
            elems_per_tile: 1024,
            bound: (0, 0),
        });
    }
    assert!(
        matches!(
            p.validate(),
            Err(IrViolation::IndirectIndexNotAnOperand { .. })
        ),
        "an index buffer the load does not read: {:?}",
        p.validate()
    );
    let mut p = base();
    if let StageKind::Load { pattern, .. } = &mut p.stages[0].kind {
        pattern.origin.indirect = Some(IndirectTile {
            index: BufferId::new(99),
            tile_stride: 1,
            elems_per_tile: 1024,
            bound: (0, 0),
        });
    }
    assert!(
        matches!(p.validate(), Err(IrViolation::DanglingBuffer { .. })),
        "a dangling index buffer: {:?}",
        p.validate()
    );

    // Non-vacuity: the untouched program validates.
    assert!(base().validate().is_ok());
}

fn op_(i: usize) -> Box<TensorBody> {
    op(i)
}

// ══════════════════════════════════════════════════════════════════════════════════════════════════
// 8. Activation / State / Sram: types now, semantics in B-P6 / B-P7 — refused naming the phase
// ══════════════════════════════════════════════════════════════════════════════════════════════════

#[test]
fn activation_and_state_roles_are_refused_by_validation_by_name_until_b_p6() {
    let wh = SpatialTarget::wormhole();
    let t = wh.tile.rows;
    let w = t;
    let x = ramp(t, w);
    let base = || {
        one_generic(
            &wh,
            "roles",
            &[In {
                name: "x",
                cols: w,
                dtype: Precision::Fp32,
                init: f(&x),
            }],
            Out {
                cols: w,
                dtype: Precision::Fp32,
            },
            vec![parallel(LV_GEN_ROW, t), parallel(LV_GEN_COL, w)],
            vec![ident_map(w)],
            ident_map(w),
            TensorBody::Operand(0),
        )
    };

    let mut p = base();
    p.tensors[0].role = TensorRole::Activation;
    match p.validate() {
        Err(IrViolation::TensorRoleDeferred {
            role: TensorRole::Activation,
            until,
            ..
        }) => assert_eq!(
            until, "B-P7",
            "an activation is a SECTION concern — the composition phase"
        ),
        other => panic!("an Activation tensor must be refused by validation, got {other:?}"),
    }
    assert!(
        format!("{:?}", p.validate().unwrap_err()).contains("B-P7"),
        "the refusal names the phase in its Debug form"
    );
    assert!(
        matches!(run_passes(p, &wh), Err(SpatialReject::Invalid(_))),
        "and the pipeline refuses it the same way"
    );

    let mut p = base();
    p.tensors[0].role = TensorRole::State;
    match p.validate() {
        Err(IrViolation::TensorRoleDeferred {
            role: TensorRole::State,
            until,
            ..
        }) => assert_eq!(until, "B-P6", "state is the cross-token recurrence phase"),
        other => panic!("a State tensor must be refused by validation, got {other:?}"),
    }

    let mut p = base();
    p.tensors[0].space = TensorSpace::Sram;
    match p.validate() {
        Err(IrViolation::TensorSpaceDeferred {
            space: TensorSpace::Sram,
            until,
            ..
        }) => assert_eq!(until, "B-P6", "the SRAM tier arrives with resident state"),
        other => panic!("an Sram tensor must be refused by validation, got {other:?}"),
    }

    // The three new declarations serialize and round-trip — the neutral artifact will carry them
    // the day they are admitted, and an old dump without them still parses.
    let mut p = base();
    p.tensors[0].role = TensorRole::State;
    p.tensors[0].space = TensorSpace::Sram;
    let dump = p.neutral_dump();
    assert!(
        dump.contains("\"State\"") && dump.contains("\"Sram\""),
        "the dump names the new role and space"
    );
    assert_eq!(
        SpatialProgram::parse_neutral(&dump).expect("parses"),
        p,
        "round-trip"
    );
    // A dump written BEFORE B-P3 has no `indirect` key anywhere: strip every one and it must still parse.
    fn strip_indirect(v: &mut serde_json::Value) -> usize {
        match v {
            serde_json::Value::Object(m) => {
                let removed = m.remove("indirect").is_some() as usize;
                removed + m.values_mut().map(strip_indirect).sum::<usize>()
            }
            serde_json::Value::Array(a) => a.iter_mut().map(strip_indirect).sum(),
            _ => 0,
        }
    }
    let mut old: serde_json::Value =
        serde_json::from_str(&base().neutral_dump()).expect("the dump is JSON");
    let removed = strip_indirect(&mut old);
    assert!(removed > 0, "the dump carries `indirect` fields (a map and a tile address each have one) — nothing was stripped");
    let parsed = SpatialProgram::parse_neutral(&old.to_string());
    assert!(
        parsed.is_ok(),
        "a dump written before B-P3 (no `indirect` fields) must still parse: {:?}",
        parsed.err()
    );
    assert_eq!(
        parsed.unwrap(),
        base(),
        "and it is the same program — the new fields default to their absence"
    );
}
