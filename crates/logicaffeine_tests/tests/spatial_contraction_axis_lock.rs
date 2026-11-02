//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE CONTRACTION AXIS — a `Generic` folds a third iteration variable, so a small matrix product is a map.
//!
//! A `Generic` computes `out[r, c] = body(operands read at maps(r, c))` over its `(rows, cols)` space, folding the row
//! or the column when that axis is a `Reduce`. A matrix product needs one more variable than that: `C[r, c] =
//! Σ_k A[r, k]·B[k, c]` reads its operands at points no `(r, c)` names alone. The CONTRACTION AXIS is that variable —
//! a third `IterSpec` over `LV_GEN_K`, always a fold, folded innermost at each point of the space — and every map may
//! step along it. With it `KKᵀ`, `QKᵀ`, `T·V` and `Δᵀ·K`, the per-head products of the chunked Gated DeltaNet and of
//! a query tile's attention, are one stage each; without it each is a stage per output row.
//!
//! It is a PARAMETER of the general form, not a new operation: no `ComputeOp` variant, no `IterKind` variant, no print
//! arm (`spatial_algebra_lock` holds all three). A printer that has no sequence for it refuses it BY NAME inside the
//! `Generic` arm it already has; the RDU's neutral lane ships the IR and lowers it.
//!
//! Pinned:
//!   1. A contraction multiplies: `A·Bᵀ` with both operands' `k` along their pages' columns (the `KKᵀ` shape), and
//!      `A·B` with `B`'s `k` down its page's rows (the `T·V` shape), each against a product computed here.
//!   2. It composes with the row and column folds: folded innermost, the outer fold consumes each point's product.
//!   3. It is validated: a third axis that is not a fold, a third axis over any variable but `LV_GEN_K`, an output
//!      that varies along it, a map that steps along it with no such axis, and a fourth axis are each refused.
//!   4. Every printer refuses it by name or lowers it to the evaluator's answer — the fabric's contraction unit runs
//!      in RTL under iverilog to every output the evaluator computes, in the twin's cycles — and the RDU ships it with a
//!      certificate that rechecks.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_contraction_axis_lock`
//!
//! ★ MUTATION PROBES (private copy, every one RED, 2026-09-26):
//!   • the evaluator takes the contraction at `k = 0` instead of folding it → `A·Bᵀ at (0, 0)` and `row 0: Σ_c …`.
//!   • a map's offset ignores the contraction variable → the same two.
//!   • validation drops "a contraction axis that is not a sum fold" → `a third axis that does not fold: must be refused`.
//!   • the fabric's contraction reads its operand at `k = 0` → its RTL is not the evaluator's.
//! ════════════════════════════════════════════════════════════════════════════════════════════════
#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "common/fabric_rtl.rs"]
mod fabric_rtl;
#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Value};
use logicaffeine_compile::spatial::ir::*;
use logicaffeine_compile::spatial::lower::{
    cuda::CudaPersistentBackend, fabric::FabricBackend, rdu::RduSpatialBackend,
    tensix::TensixBackend, SpatialArtifact, SpatialBackend, SpatialLowerError,
};
use logicaffeine_compile::spatial::passes::from_tensor_op::{
    ident_map, per_col_map, per_row_map, Template,
};
use logicaffeine_compile::spatial::passes::{LV_GEN_COL, LV_GEN_K, LV_GEN_ROW};
use logicaffeine_compile::spatial::pipeline::run_passes;
use logicaffeine_compile::spatial::target::{precision_is_int, SpatialTarget};

/// One operand: a tensor one tile tall and `cols` wide, with its values.
struct In {
    name: &'static str,
    cols: usize,
    vals: Vec<f64>,
}

fn parallel(var: LoopVar, extent: usize) -> IterSpec {
    IterSpec {
        var,
        extent,
        kind: IterKind::Parallel,
    }
}

fn fold(var: LoopVar, extent: usize) -> IterSpec {
    IterSpec {
        var,
        extent,
        kind: IterKind::Reduce {
            pool: ReducePool::Sum,
        },
    }
}

fn map(terms: &[(LoopVar, i64)]) -> ElemAccess {
    ElemAccess {
        base: 0,
        terms: terms.to_vec(),
        indirect: None,
    }
}

fn mul01() -> TensorBody {
    TensorBody::Mul(
        Box::new(TensorBody::Operand(0)),
        Box::new(TensorBody::Operand(1)),
    )
}

/// A template with ONE `Generic` stage over `ins`, writing a `cols`-wide output one tile tall — or the error the
/// template's validation gives.
fn try_one_generic(
    target: &SpatialTarget,
    dtype: Precision,
    ins: &[In],
    cols: usize,
    iters: Vec<IterSpec>,
    maps: Vec<ElemAccess>,
    omap: ElemAccess,
    body: TensorBody,
) -> Result<SpatialProgram, String> {
    let t = target.tile.rows;
    let mut tp = Template::new(target).map_err(|e| format!("{e:?}"))?;
    let mut loads = Vec::new();
    let mut bufs = Vec::new();
    for i in ins {
        let init = if precision_is_int(dtype) {
            TensorInit::Values(i.vals.iter().map(|v| *v as i64).collect())
        } else {
            TensorInit::FloatValues(i.vals.iter().map(|v| v.to_bits()).collect())
        };
        let id = tp.tensor(i.name, TensorRole::Input, t, i.cols, dtype, init);
        let cols_t = i.cols.div_ceil(t);
        let b = tp.buffer(&format!("in_{}", i.name), cols_t, dtype);
        let l = tp.load(&format!("load_{}", i.name), id, vec![], cols_t, b);
        loads.push((l, b));
        bufs.push(b);
    }
    let y = tp.tensor("y", TensorRole::Output, t, cols, dtype, TensorInit::Zero);
    let out_ct = cols.div_ceil(t);
    let ob = tp.buffer("out", out_ct, dtype);
    let g = tp.stage(
        "contract",
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
    let acc = if precision_is_int(dtype) {
        AccPolicy::I64
    } else {
        AccPolicy::F32
    };
    tp.finish(
        "contract".to_string(),
        items,
        acc,
        NumericsClaim::Approx {
            reason: "a hand-built contraction probe".into(),
        },
        target,
    )
    .map_err(|e| format!("{e:?}"))
}

fn verified(target: &SpatialTarget, p: SpatialProgram) -> SpatialProgram {
    run_passes(p, target).unwrap_or_else(|e| {
        panic!(
            "{}: the passes must verify a contraction: {e:?}",
            target.name
        )
    })
}

fn output_of(p: &SpatialProgram) -> Vec<f64> {
    let out = p
        .tensors
        .iter()
        .find(|t| t.role == TensorRole::Output)
        .expect("an output")
        .id;
    let o = eval_choreography(p, &Inputs::default())
        .unwrap_or_else(|e| panic!("{}: the choreography must evaluate: {e:?}", p.name));
    match &o.tensors[&out] {
        Value::Float(x) => x.clone(),
        Value::Int(x) => x.iter().map(|i| *i as f64).collect(),
    }
}

/// Small integers, distinct enough that a wrong read is a wrong number.
fn vals(n: usize, salt: usize) -> Vec<f64> {
    (0..n).map(|p| ((p * 7 + salt) % 9) as f64 - 4.0).collect()
}

fn sn50() -> SpatialTarget {
    SpatialTarget::sn50_rdu()
}

// ── 1. A contraction multiplies ──

#[test]
fn a_contraction_multiplies_a_by_b_transposed_and_a_by_b() {
    let target = sn50();
    let t = target.tile.rows;
    let dtype = Precision::Fp32;
    // `C[r, c] = Σ_k A[r, k]·B[c, k]`, `k` along both pages' columns, 2 tiles deep: the `KKᵀ` shape.
    let depth = 2 * t;
    let (a, b) = (vals(t * depth, 1), vals(t * depth, 5));
    let p = try_one_generic(
        &target,
        dtype,
        &[
            In {
                name: "a",
                cols: depth,
                vals: a.clone(),
            },
            In {
                name: "b",
                cols: depth,
                vals: b.clone(),
            },
        ],
        t,
        vec![
            parallel(LV_GEN_ROW, t),
            parallel(LV_GEN_COL, t),
            fold(LV_GEN_K, depth),
        ],
        vec![
            map(&[(LV_GEN_ROW, depth as i64), (LV_GEN_K, 1)]),
            map(&[(LV_GEN_COL, depth as i64), (LV_GEN_K, 1)]),
        ],
        ident_map(t),
        mul01(),
    )
    .expect("an A·Bᵀ contraction validates");
    let got = output_of(&verified(&target, p));
    for r in 0..t {
        for c in 0..t {
            let want: f64 = (0..depth)
                .map(|k| a[r * depth + k] * b[c * depth + k])
                .sum();
            assert_eq!(got[r * t + c], want, "A·Bᵀ at ({r}, {c})");
        }
    }
    // `C[r, e] = Σ_k A[r, k]·B[k, e]`, `B`'s `k` down its page's rows, `e` two tiles wide: the `T·V` shape.
    let wide = 2 * t;
    let (a, b) = (vals(t * t, 2), vals(t * wide, 3));
    let p = try_one_generic(
        &target,
        dtype,
        &[
            In {
                name: "a",
                cols: t,
                vals: a.clone(),
            },
            In {
                name: "b",
                cols: wide,
                vals: b.clone(),
            },
        ],
        wide,
        vec![
            parallel(LV_GEN_ROW, t),
            parallel(LV_GEN_COL, wide),
            fold(LV_GEN_K, t),
        ],
        vec![
            map(&[(LV_GEN_ROW, t as i64), (LV_GEN_K, 1)]),
            map(&[(LV_GEN_K, wide as i64), (LV_GEN_COL, 1)]),
        ],
        ident_map(wide),
        mul01(),
    )
    .expect("an A·B contraction validates");
    let got = output_of(&verified(&target, p));
    for r in 0..t {
        for e in 0..wide {
            let want: f64 = (0..t).map(|k| a[r * t + k] * b[k * wide + e]).sum();
            assert_eq!(got[r * wide + e], want, "A·B at ({r}, {e})");
        }
    }
}

// ── 2. It composes with the row and column folds ──

#[test]
fn a_contraction_folds_innermost_under_a_row_or_column_fold() {
    let target = sn50();
    let t = target.tile.rows;
    let dtype = Precision::Fp32;
    let (a, b) = (vals(t * t, 4), vals(t * t, 6));
    let product = |r: usize, c: usize| -> f64 { (0..t).map(|k| a[r * t + k] * b[c * t + k]).sum() };
    let ins = || {
        [
            In {
                name: "a",
                cols: t,
                vals: a.clone(),
            },
            In {
                name: "b",
                cols: t,
                vals: b.clone(),
            },
        ]
    };
    let maps = || {
        vec![
            map(&[(LV_GEN_ROW, t as i64), (LV_GEN_K, 1)]),
            map(&[(LV_GEN_COL, t as i64), (LV_GEN_K, 1)]),
        ]
    };
    // Folding the columns: one value per row, written across the row.
    let p = try_one_generic(
        &target,
        dtype,
        &ins(),
        t,
        vec![
            parallel(LV_GEN_ROW, t),
            fold(LV_GEN_COL, t),
            fold(LV_GEN_K, t),
        ],
        maps(),
        per_row_map(t),
        mul01(),
    )
    .expect("validates");
    let got = output_of(&verified(&target, p));
    for r in 0..t {
        let want: f64 = (0..t).map(|c| product(r, c)).sum();
        assert_eq!(got[r * t], want, "row {r}: Σ_c (A·Bᵀ)[r, c]");
    }
    // Folding the rows: one value per column, written down the column.
    let p = try_one_generic(
        &target,
        dtype,
        &ins(),
        t,
        vec![
            fold(LV_GEN_ROW, t),
            parallel(LV_GEN_COL, t),
            fold(LV_GEN_K, t),
        ],
        maps(),
        per_col_map(),
        mul01(),
    )
    .expect("validates");
    let got = output_of(&verified(&target, p));
    for c in 0..t {
        let want: f64 = (0..t).map(|r| product(r, c)).sum();
        assert_eq!(got[c], want, "column {c}: Σ_r (A·Bᵀ)[r, c]");
    }
}

// ── 3. It is validated ──

#[test]
fn a_contraction_axis_is_validated() {
    let target = sn50();
    let t = target.tile.rows;
    let dtype = Precision::Fp32;
    let ins = || {
        [
            In {
                name: "a",
                cols: t,
                vals: vals(t * t, 1),
            },
            In {
                name: "b",
                cols: t,
                vals: vals(t * t, 2),
            },
        ]
    };
    let maps = || {
        vec![
            map(&[(LV_GEN_ROW, t as i64), (LV_GEN_K, 1)]),
            map(&[(LV_GEN_COL, t as i64), (LV_GEN_K, 1)]),
        ]
    };
    let pp = || vec![parallel(LV_GEN_ROW, t), parallel(LV_GEN_COL, t)];
    let refused = |what: &str, r: Result<SpatialProgram, String>| {
        let e = r
            .err()
            .unwrap_or_else(|| panic!("{what}: must be refused by validation"));
        assert!(
            e.to_ascii_lowercase().contains("contraction"),
            "{what}: the refusal names the contraction axis: {e}"
        );
    };
    let mut parallel_k = pp();
    parallel_k.push(parallel(LV_GEN_K, t));
    refused(
        "a third axis that does not fold",
        try_one_generic(
            &target,
            dtype,
            &ins(),
            t,
            parallel_k,
            maps(),
            ident_map(t),
            mul01(),
        ),
    );
    let mut other_var = pp();
    other_var.push(fold(LoopVar(3), t));
    refused(
        "a third axis over another variable",
        try_one_generic(
            &target,
            dtype,
            &ins(),
            t,
            other_var,
            maps(),
            ident_map(t),
            mul01(),
        ),
    );
    let mut k = pp();
    k.push(fold(LV_GEN_K, t));
    let varying_out = ElemAccess {
        base: 0,
        terms: vec![(LV_GEN_ROW, t as i64), (LV_GEN_COL, 1), (LV_GEN_K, 1)],
        indirect: None,
    };
    refused(
        "an output that varies along the contraction",
        try_one_generic(
            &target,
            dtype,
            &ins(),
            t,
            k.clone(),
            maps(),
            varying_out,
            mul01(),
        ),
    );
    refused(
        "a map stepping along a contraction the space lacks",
        try_one_generic(
            &target,
            dtype,
            &ins(),
            t,
            pp(),
            maps(),
            ident_map(t),
            mul01(),
        ),
    );
    let mut four = k;
    four.push(fold(LV_GEN_K, t));
    refused(
        "a fourth axis",
        try_one_generic(
            &target,
            dtype,
            &ins(),
            t,
            four,
            maps(),
            ident_map(t),
            mul01(),
        ),
    );
}

// ── 4. Every printer refuses it by name or lowers it to the evaluator's answer; the RDU ships it ──

#[test]
fn every_printer_refuses_a_contraction_by_name_or_lowers_it_to_the_evaluator_s_answer_and_the_rdu_ships_it() {
    let backends: Vec<(&str, Box<dyn SpatialBackend>)> = vec![
        (
            "Tensix",
            Box::new(TensixBackend::new(SpatialTarget::wormhole())),
        ),
        (
            "fabric",
            Box::new(FabricBackend::new(SpatialTarget::sky_fabric(64, 64))),
        ),
        (
            "CUDA",
            Box::new(CudaPersistentBackend::new(SpatialTarget::rtx3060())),
        ),
        ("RDU", Box::new(RduSpatialBackend::new())),
    ];
    for (name, backend) in backends {
        let target = backend.target().clone();
        // A whole tile, `A·Bᵀ` over its rows and columns, on a printer of whole tiles. The fabric's tile is one element
        // and a page one tile row, so there it is one row of 4 against a 4×4 `B` laid in one row: a contraction of 4,
        // which the run it is held to must fold.
        let (rows, n) = if name == "fabric" { (1, 4) } else { (target.tile.rows, target.tile.rows) };
        let dtype = if name == "fabric" {
            Precision::Int64
        } else {
            Precision::Fp32
        };
        let p = try_one_generic(
            &target,
            dtype,
            &[
                In {
                    name: "a",
                    cols: n,
                    vals: vals(rows * n, 1),
                },
                In {
                    name: "b",
                    cols: n * n / rows,
                    vals: vals(n * n, 2),
                },
            ],
            n,
            vec![
                parallel(LV_GEN_ROW, rows),
                parallel(LV_GEN_COL, n),
                fold(LV_GEN_K, n),
            ],
            vec![
                map(&[(LV_GEN_ROW, n as i64), (LV_GEN_K, 1)]),
                map(&[(LV_GEN_COL, n as i64), (LV_GEN_K, 1)]),
            ],
            ident_map(n),
            mul01(),
        )
        .unwrap_or_else(|e| panic!("{name}: the contraction validates: {e}"));
        let p = verified(&target, p);
        match (name, backend.lower(&p)) {
            ("RDU", Ok(SpatialArtifact::SdkGated(art))) => {
                assert!(
                    art.certificate.recheck(&p),
                    "RDU: the shipped certificate rechecks against the contraction"
                );
                assert!(
                    art.json.contains("\"program\""),
                    "RDU: the neutral artifact carries the program"
                );
            }
            ("RDU", other) => panic!(
                "RDU: the neutral lane ships the IR and must lower a contraction, got {other:?}"
            ),
            // The CUDA lane hands any program but a lone tile matmul to its general program, which computes integers and
            // refuses a float operand by name before it reads a stage; every other printer names the contraction.
            ("CUDA", Err(SpatialLowerError::Unsupported(reason))) => {
                assert!(
                    reason.contains("float") && reason.contains("general GPU program"),
                    "CUDA: the refusal names the float operand its general program does not print: {reason}"
                );
            }
            (_, Err(SpatialLowerError::Unsupported(reason))) => {
                assert!(
                    reason.contains("contraction"),
                    "{name}: the refusal names the contraction axis: {reason}"
                );
            }
            ("fabric", Ok(SpatialArtifact::Verilog(v))) => {
                if common::require_tool("iverilog") {
                    fabric_rtl::runs_as_the_evaluator("fabric × contraction", &p, &target, &v, &spatial_support::inputs_from_fills(&p));
                }
            }
            (_, other) => panic!("{name}: must refuse a contraction by name or lower it to the evaluator's answer, got {other:?}"),
        }
    }
}
