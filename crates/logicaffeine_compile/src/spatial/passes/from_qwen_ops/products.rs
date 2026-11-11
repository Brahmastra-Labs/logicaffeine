//! Products per head.

use super::*;

/// `Y = X matmul W across H heads` — the weighted-sum shape: the folded axis is the ROW (`e` over a head's `a` inputs),
/// the flat output column `c = u·b + r` for unit `u` (row `u / H`, head `h = u mod H`), and each unit's band reads
/// `X[u·a + e]` and `W[(h·a + e)·b + c − u·b]`.
pub fn from_head_matmul(
    spec: &HeadMatmulSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (m, heads, a, bw) = (spec.m, spec.heads, spec.a, spec.b);
    let (units, n) = (m * heads, m * heads * bw);
    if n == 0 || a == 0 {
        return Err(SpatialReject::NotLowered("a per-head matmul of nothing"));
    }
    if m == 1 && whole_tiles(&[a, bw], target) {
        return tile_head_matmul(spec, names, target);
    }
    let mut f = FlatBuild::columns(n, target)?;
    let x = f.input(names.operand(0).0, m * heads * a, names.operand(0).1, target);
    let w = f.input(names.operand(1).0, heads * a * bw, names.operand(1).1, target);
    let precision = f.precision;
    let out_buf = f.result_page("out", n, precision, target);
    let tile_elems = target.tile.rows * target.tile.cols;
    let ones = f.t.tensor(
        "hm_scaler",
        TensorRole::Param,
        target.tile.rows,
        target.tile.cols,
        f.precision,
        TensorInit::FloatValues(vec![1.0f64.to_bits(); tile_elems]),
    );
    let ones_in = f.t.buffer("in_scaler", 1, f.precision);
    let ls = f.t.load("load_scaler", ones, vec![], 1, ones_in);
    f.loads.push((ls, ones_in));
    let mut slots = vec![Slot { buf: ones_in, map: const_map() }];
    let mut bands = Vec::with_capacity(units);
    for u in 0..units {
        let h = u % heads;
        slots.push(Slot {
            buf: x,
            map: ElemAccess { base: (u * a) as i64, terms: vec![(LV_GEN_ROW, 1)], indirect: None },
        });
        let x_slot = slots.len() - 1;
        slots.push(Slot {
            buf: w,
            map: ElemAccess {
                base: (h * a * bw) as i64 - (u * bw) as i64,
                terms: vec![(LV_GEN_ROW, bw as i64), (LV_GEN_COL, 1)],
                indirect: None,
            },
        });
        let w_slot = slots.len() - 1;
        bands.push(((u + 1) * bw, *mul(op(x_slot), op(w_slot))));
    }
    let chain = f.fold("head_matmul", n, (a, ReducePool::Sum), slots, banded(bands), out_buf, target)?;
    let claim = approx("per-head matmul", f.precision);
    f.finish(&names.out, n, chain, out_buf, format!("head_matmul_{m}x{heads}x{a}x{bw}"), claim, target)
}

/// `Y = X matmul W across H heads` for one row at whole-tile widths: item `(h, o)` is output tile `o` of head `h`,
/// `Σ_e X[h·a + e]·W[h·a + e, o·32 + c]` — head `h`'s `a/32` input tiles against the `a/32 × 1` tile column `o` of
/// its rows of `W`, which `W`'s declaration as the `(H·a) × b` matrix it is lays out as tiles. Needs a square tile: a
/// weight tile's rows are the input's elements. Only the head's rows of
/// one tile column move an item, so `W` is never on chip whole.
pub(super) fn tile_head_matmul(
    spec: &HeadMatmulSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (heads, a, bw) = (spec.heads, spec.a, spec.b);
    let (tr, tw) = (target.tile.rows, target.tile.cols);
    if tr != tw {
        return Err(SpatialReject::NotLowered("a per-head matmul in tiles on a tile that is not square — a weight tile's rows are the input's elements"));
    }
    let (a_t, b_t) = (a / tw, bw / tw);
    let precision = elem_precision(target)?;
    let (x_name, x_fill) = names.operand(0);
    let (w_name, w_fill) = names.operand(1);
    let mut t = Template::new(target)?;
    let scaler = t.tensor("hm_scaler", TensorRole::Param, tr, tw, precision, TensorInit::FloatValues(vec![1.0f64.to_bits(); tr * tw]));
    let x = t.tensor(x_name, TensorRole::Input, 1, heads * a, precision, crate::spatial::passes::from_tensor_op::init_of(x_fill, heads * a));
    let w = t.tensor(w_name, TensorRole::Param, heads * a, bw, precision, crate::spatial::passes::from_tensor_op::init_of(w_fill, heads * a * bw));
    let y = t.tensor(&names.out, TensorRole::Output, 1, heads * bw, precision, TensorInit::Zero);
    let sc_in = t.buffer_deep("in_scaler", 1, 1, precision);
    let x_in = t.buffer("in_x", a_t, precision);
    let w_in = t.buffer("in_w", a_t, precision);
    let out = t.buffer("out", 1, precision);
    let l_sc = t.load_block_at("load_scaler", scaler, 0, Vec::new(), 1, 1, 1, sc_in);
    let l_x = t.load_block_at("load_x", x, 0, vec![(LV_ITEM_Y, a_t)], 1, a_t, heads * a_t, x_in);
    let l_w = t.load_block_at("load_w", w, 0, vec![(LV_ITEM_Y, a_t * b_t), (LV_ITEM_X, 1)], a_t, 1, b_t, w_in);
    // ★ A PAGE IS ONE PLANE OF ITS TILES SIDE BY SIDE, so the weight's `a/32` stacked tiles lie as tile `k` at columns
    // `[32k, 32k + 32)`: input `e = 32k + r`, output `c` is at row `r`, column `32k + c`, and the input row's `e` at
    // column `32k + r` of row 0. The fold runs over both: the tile's rows `r` and the tiles `k`.
    let s = t.stage(
        "head_dot",
        StageKind::Compute(ComputeOp::Generic {
            iters: vec![
                IterSpec { var: LV_GEN_ROW, extent: tr, kind: IterKind::Reduce { pool: ReducePool::Sum } },
                IterSpec { var: LV_GEN_COL, extent: tw, kind: IterKind::Parallel },
                IterSpec { var: LV_GEN_K, extent: a_t, kind: IterKind::Reduce { pool: ReducePool::Sum } },
            ],
            ins: vec![
                ElemAccess { base: 0, terms: vec![(LV_GEN_ROW, (a_t * tw) as i64), (LV_GEN_K, tw as i64), (LV_GEN_COL, 1)], indirect: None },
                ElemAccess { base: 0, terms: vec![(LV_GEN_K, tw as i64), (LV_GEN_ROW, 1)], indirect: None },
                const_map(),
            ],
            out: per_col_map(),
            body: *mul(op(0), op(1)),
        }),
        vec![w_in, x_in, sc_in],
        vec![out],
    );
    let st = t.store_block(&format!("store_{}", names.out), y, 0, vec![(LV_ITEM_Y, b_t), (LV_ITEM_X, 1)], 1, 1, heads * b_t, out);
    for (from, buf, to) in [(l_w, w_in, s), (l_x, x_in, s), (l_sc, sc_in, s), (s, out, st)] {
        t.edge(from, buf, to);
    }
    let prog = t.finish(
        format!("head_matmul_1x{heads}x{a}x{bw}_tiled"),
        grouped_block_items(heads, b_t, 1),
        AccPolicy::F32,
        approx("per-head matmul", precision),
        target,
    )?;
    if site_footprint(&prog) > target.max_local_mem_bytes() {
        return Err(SpatialReject::NotLowered("a per-head matmul whose head's inputs and weight column do not fit a site"));
    }
    Ok(prog)
}
