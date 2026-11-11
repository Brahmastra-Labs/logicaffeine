//! Shapes: transposes, head slices, joins, and their whole-tile forms.

use super::*;

/// `Kt[j·rows + i] = X[i·cols + j]` — a TRANSPOSE IS A MAP, not an operator.
///
/// ★ A STAGE'S RESULT IS PLACED BY ITS ITERATION SPACE, so a flat result must iterate the flat output. Its
/// element `c = j·rows + i` (new row `j`, new column `i`) is `X[i·cols + j]`, and inside new row `j`'s band
/// `[j·rows, (j+1)·rows)` that offset is `c·cols + j·(1 − rows·cols)` — affine. So a matrix transpose is `cols`
/// bands of one read each: a map per band, still no operator.
///
/// ★ A VECTOR TRANSPOSES TO ITSELF. A `1 × n` row and an `n × 1` column are the same flat sequence, so their
/// transpose is one plain read — no bands at all.
pub fn from_transpose(
    spec: &TransposeSpec,
    names: &StageNames,
    float: bool,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols) = (spec.rows, spec.cols);
    let n = rows * cols;
    if n == 0 {
        return Err(SpatialReject::NotLowered("a transpose of an empty tensor"));
    }
    let mut b = FlatBuild::on(Datapath::of(float), target)?;
    let xin = b.input(names.operand(0).0, n, names.operand(0).1, target);
    let cols_t = tiles_of(n, target);
    let out_buf = b.t.buffer("out", cols_t, b.precision);
    let body_and_slots = if rows == 1 || cols == 1 {
        (
            TensorBody::Operand(0),
            vec![Slot {
                buf: xin,
                map: at(0),
            }],
        )
    } else {
        let mut slots = Vec::with_capacity(cols);
        let mut bands = Vec::with_capacity(cols);
        for j in 0..cols {
            let base = j as i64 * (1 - n as i64);
            slots.push(Slot {
                buf: xin,
                map: ElemAccess {
                    base,
                    terms: vec![(LV_GEN_COL, cols as i64)],
                    indirect: None,
                },
            });
            bands.push(((j + 1) * rows, TensorBody::Operand(slots.len() - 1)));
        }
        (banded(bands), slots)
    };
    let (body, slots) = body_and_slots;
    let s = b.compute(
        "transpose",
        flat_space(n),
        slots,
        ident_map(wide(cols_t, target)),
        body,
        out_buf,
    );
    b.finish(
        &names.out,
        n,
        vec![s],
        out_buf,
        format!("transpose_{rows}x{cols}"),
        NumericsClaim::Exact,
        target,
    )
}

/// `P = the first|last K of every N of X` — group `g`'s `K` elements from `from` land at `[g·K, (g+1)·K)`: one band per
/// group, each an affine read `X[c + g·(N − K) + from]`.
pub fn from_head_slice(
    spec: &HeadSliceSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (every, from, width) = (spec.every, spec.from, spec.width);
    let groups = if every == 0 { 0 } else { spec.n / every };
    let n = groups * width;
    if n == 0 || from + width > every {
        return Err(SpatialReject::NotLowered("a head slice of nothing, or a part past its group"));
    }
    if whole_tiles(&[every, from, width], target) {
        return tile_head_slice(spec, names, groups, target);
    }
    let mut b = FlatBuild::columns(n, target)?;
    let xin = b.input(names.operand(0).0, spec.n, names.operand(0).1, target);
    let precision = b.precision;
    let out_buf = b.result_page("out", n, precision, target);
    let mut slots = Vec::with_capacity(groups);
    let mut bands = Vec::with_capacity(groups);
    for g in 0..groups {
        slots.push(Slot {
            buf: xin,
            map: at((g * (every - width) + from) as i64),
        });
        bands.push(((g + 1) * width, TensorBody::Operand(g)));
    }
    let chain = b.pointwise("head_slice", n, slots, banded(bands), out_buf, target)?;
    b.finish(
        &names.out,
        n,
        chain,
        out_buf,
        format!("head_slice_{groups}x{width}_of_{every}"),
        NumericsClaim::Exact,
        target,
    )
}

/// `J = A joined with B across H heads` — unit `u` (row `u / H`, head `u mod H`) holds its `ha` elements of `A` then its
/// `hb` of `B` at `[u·(ha + hb), (u+1)·(ha + hb))`: two bands per unit, `A[c − u·hb]` then `B[c − (u+1)·ha]`.
pub fn from_join(spec: &JoinSpec, names: &StageNames, target: &SpatialTarget) -> Result<SpatialProgram, SpatialReject> {
    let (rows, heads) = (spec.rows, spec.heads);
    if heads == 0 || spec.wa % heads != 0 || spec.wb % heads != 0 {
        return Err(SpatialReject::NotLowered("a join whose widths do not divide among its heads"));
    }
    let (ha, hb) = (spec.wa / heads, spec.wb / heads);
    let (units, w) = (rows * heads, ha + hb);
    let n = units * w;
    if n == 0 {
        return Err(SpatialReject::NotLowered("a join of nothing"));
    }
    if ha > 0 && hb > 0 && whole_tiles(&[ha, hb], target) {
        return tile_join(names, (rows, heads), (ha, hb), target);
    }
    let mut b = FlatBuild::columns(n, target)?;
    let ain = b.input(names.operand(0).0, rows * spec.wa, names.operand(0).1, target);
    let bin = b.input(names.operand(1).0, rows * spec.wb, names.operand(1).1, target);
    let precision = b.precision;
    let out_buf = b.result_page("out", n, precision, target);
    let mut slots = Vec::with_capacity(2 * units);
    let mut bands = Vec::with_capacity(2 * units);
    for u in 0..units {
        let (ui, ha_i, hb_i) = (u as i64, ha as i64, hb as i64);
        if ha > 0 {
            slots.push(Slot { buf: ain, map: at(-ui * hb_i) });
            bands.push((u * w + ha, TensorBody::Operand(slots.len() - 1)));
        }
        if hb > 0 {
            slots.push(Slot { buf: bin, map: at(-(ui + 1) * ha_i) });
            bands.push(((u + 1) * w, TensorBody::Operand(slots.len() - 1)));
        }
    }
    let chain = b.pointwise("join", n, slots, banded(bands), out_buf, target)?;
    b.finish(
        &names.out,
        n,
        chain,
        out_buf,
        format!("join_{rows}x{heads}_{ha}+{hb}"),
        NumericsClaim::Exact,
        target,
    )
}

// ── The latent-attention family at whole-tile widths ─────────────────────────────────────────────────
//
// ★ AT WHOLE-TILE WIDTHS A SLICE AND A JOIN ARE TILE MOVES, AND A PER-HEAD PRODUCT IS A PER-HEAD GEMV. The flat forms
// above hold the whole row in one accumulator page and every weight in one site, which is free at a toy's widths and
// impossible at DeepSeek-V2's: sixteen heads of 576 are 288 tiles against eight, and one head's latent expansion is
// 128 × 512. Every published latent attention's widths are whole tiles (128 and 64 for the query's halves, 512 for the
// latent, 128 for the values), so these forms take them: each item moves or computes one block of the result, and only
// that block's operands leave memory.

/// Whether every width is a whole number of tiles.
pub(super) fn whole_tiles(widths: &[usize], target: &SpatialTarget) -> bool {
    widths.iter().all(|w| w % target.tile.cols == 0)
}

/// The items of a flat row of `groups` runs of `run_t` tiles, `blk` tiles an item: item `(g, j)` owns tiles
/// `[g·run_t + j·blk, g·run_t + (j+1)·blk)`.
pub(super) fn grouped_block_items(groups: usize, run_t: usize, blk: usize) -> Vec<WorkItem> {
    (0..groups)
        .flat_map(|g| (0..run_t / blk).map(move |j| (g, j)))
        .enumerate()
        .map(|(id, (g, j))| WorkItem {
            id: WorkItemId::new(id),
            coord: (g, j),
            out: TileRect {
                row0_t: 0,
                col0_t: g * run_t + j * blk,
                h_t: 1,
                w_t: blk,
            },
        })
        .collect()
}

/// `P = the first|last K of every N of X` at whole-tile widths: block `j` of group `g`'s part is the block at tile
/// `g·N_t + from_t + j·blk` of `X`, an affine tile copy.
pub(super) fn tile_head_slice(
    spec: &HeadSliceSpec,
    names: &StageNames,
    groups: usize,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let tw = target.tile.cols;
    let (every_t, from_t, width_t) = (spec.every / tw, spec.from / tw, spec.width / tw);
    let blk = row_block(width_t, target);
    let precision = elem_precision(target)?;
    let (x_name, x_fill) = names.operand(0);
    let mut t = Template::new(target)?;
    let x = t.tensor(x_name, TensorRole::Input, 1, spec.n, precision, crate::spatial::passes::from_tensor_op::init_of(x_fill, spec.n));
    let y = t.tensor(&names.out, TensorRole::Output, 1, groups * spec.width, precision, TensorInit::Zero);
    let xin = t.buffer("in_x", blk, precision);
    let lx = t.load_block_at("load_x", x, from_t, vec![(LV_ITEM_Y, every_t), (LV_ITEM_X, blk)], 1, blk, spec.n / tw, xin);
    let (copy, out) = copy_page(&mut t, xin, blk, precision, target);
    let st = t.store_block(
        &format!("store_{}", names.out),
        y,
        0,
        vec![(LV_ITEM_Y, width_t), (LV_ITEM_X, blk)],
        1,
        blk,
        groups * width_t,
        out,
    );
    t.edge(lx, xin, copy);
    t.edge(copy, out, st);
    t.finish(
        format!("head_slice_{groups}x{}_of_{}_tiled", spec.width, spec.every),
        grouped_block_items(groups, width_t, blk),
        AccPolicy::for_result(precision),
        NumericsClaim::Exact,
        target,
    )
}

/// `J = A joined with B across H heads` at whole-tile parts: unit `u`'s run of `(ha + hb)/32` tiles is its `ha/32` tiles
/// of `A` then its `hb/32` of `B`, moved `blk` tiles an item (`blk` dividing both parts).
///
/// ★ EVERY ITEM RUNS THE SAME STAGES, SO THE SOURCE IS DATA. Block `s` of a unit comes from `A` for `s < ha/blk` and
/// from `B` after. Each item reads both: `A`'s block `min(s, ha/blk − 1)` and `B`'s block `max(s − ha/blk, 0)`, each
/// an [`IndirectTile`] over the unit's run addressed by a compile-time table one tile per block, and keeps `A`'s where
/// a third table holds the block's width and `B`'s where it holds zero. Every address is in its tensor, whatever the
/// block.
pub(super) fn tile_join(
    names: &StageNames,
    (rows, heads): (usize, usize),
    (ha, hb): (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let tw = target.tile.cols;
    let (ha_t, hb_t) = (ha / tw, hb / tw);
    let units = rows * heads;
    let blk = (1..=ha_t.min(hb_t).min(row_block_tiles(target)))
        .rev()
        .find(|d| ha_t % d == 0 && hb_t % d == 0)
        .unwrap_or(1);
    let (na, nb) = (ha_t / blk, hb_t / blk);
    let blocks = na + nb;
    let precision = elem_precision(target)?;
    let ix = index_type(target, (blk * tw).max(blocks) as u64 + 1)?;
    let (a_name, a_fill) = names.operand(0);
    let (b_name, b_fill) = names.operand(1);
    let mut t = Template::new(target)?;
    let a = t.tensor(a_name, TensorRole::Input, 1, units * ha, precision, crate::spatial::passes::from_tensor_op::init_of(a_fill, units * ha));
    let b = t.tensor(b_name, TensorRole::Input, 1, units * hb, precision, crate::spatial::passes::from_tensor_op::init_of(b_fill, units * hb));
    let y = t.tensor(&names.out, TensorRole::Output, 1, units * (ha + hb), precision, TensorInit::Zero);
    // One value a block, at element 0 of its tile.
    let per_block = |t: &mut Template, what: &str, value: &dyn Fn(usize) -> usize| {
        let tiles = (0..blocks).flat_map(|s| std::iter::once(value(s) as i64).chain(std::iter::repeat_n(0, tw - 1))).collect();
        t.tensor(&format!("{}__join_{what}", names.out), TensorRole::Param, 1, blocks * tw, ix, TensorInit::Values(tiles))
    };
    let from_a = per_block(&mut t, "from_a", &|s| s.min(na - 1));
    let from_b = per_block(&mut t, "from_b", &|s| s.saturating_sub(na).min(nb - 1));
    let keep_a = per_block(&mut t, "keep_a", &|s| if s < na { blk * tw } else { 0 });
    let gather = |t: &mut Template, part: &str, tensor: TensorId, table: TensorId, run_t: usize, bound: usize| {
        let pick = t.buffer_deep(&format!("in_{part}_block"), 1, 1, ix);
        let l_pick = t.load_block_at(&format!("load_{part}_block"), table, 0, vec![(LV_ITEM_X, 1)], 1, 1, 1, pick);
        let page = t.buffer(&format!("in_{part}"), blk, precision);
        let l_page = t.stage(
            &format!("load_{part}"),
            StageKind::Load {
                tensor,
                pattern: AccessPattern {
                    origin: TileAddr {
                        base: 0,
                        steps: vec![(LV_ITEM_Y, run_t)],
                        indirect: Some(IndirectTile { index: pick, tile_stride: blk, elems_per_tile: 1, bound: (0, bound) }),
                    },
                    block_h: 1,
                    block_w: blk,
                    stride_w: 1,
                    stride_h: 1,
                },
                num_blocks: 1,
                multicast: None,
            },
            vec![pick],
            vec![page],
        );
        t.edge(l_pick, pick, l_page);
        (l_page, page)
    };
    let (l_a, a_in) = gather(&mut t, "a", a, from_a, ha_t, na - 1);
    let (l_b, b_in) = gather(&mut t, "b", b, from_b, hb_t, nb - 1);
    let keep_in = t.buffer_deep("in_keep_a", 1, 1, ix);
    let l_keep = t.load_block_at("load_keep_a", keep_a, 0, vec![(LV_ITEM_X, 1)], 1, 1, 1, keep_in);
    let w = wide(blk, target);
    let out = t.buffer("out", blk, precision);
    let s = t.stage(
        "join",
        StageKind::Compute(ComputeOp::Generic {
            iters: flat_space(w),
            ins: vec![ident_map(w), ident_map(w), fixed(0)],
            out: ident_map(w),
            body: TensorBody::Select {
                cond: Pred::AxisLt(LV_GEN_COL, PredRhs::Operand(2)),
                then: op(0),
                otherwise: op(1),
            },
        }),
        vec![a_in, b_in, keep_in],
        vec![out],
    );
    let st = t.store_block(
        &format!("store_{}", names.out),
        y,
        0,
        vec![(LV_ITEM_Y, ha_t + hb_t), (LV_ITEM_X, blk)],
        1,
        blk,
        units * (ha_t + hb_t),
        out,
    );
    for (from, buf, to) in [(l_a, a_in, s), (l_b, b_in, s), (l_keep, keep_in, s), (s, out, st)] {
        t.edge(from, buf, to);
    }
    t.finish(
        format!("join_{rows}x{heads}_{ha}+{hb}_tiled"),
        grouped_block_items(units, ha_t + hb_t, blk),
        AccPolicy::for_result(precision),
        NumericsClaim::Exact,
        target,
    )
}
