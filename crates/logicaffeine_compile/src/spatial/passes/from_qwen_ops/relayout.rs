//! Relayouts between flat rows and dense matrices.

use super::*;

/// The plan of a relayout: one item per `blk`-tile block of each of `tiles_r` dense tile rows.
pub(super) fn relayout_plan(tiles_r: usize, row_t: usize, blk: usize, dt: Precision) -> BlockPlan {
    BlockPlan {
        grid: (tiles_r, row_t / blk),
        ..tile_row_plan(row_t, blk, dt)
    }
}

pub(super) fn relayout_items(tiles_r: usize, row_t: usize, blk: usize) -> Vec<WorkItem> {
    (0..tiles_r)
        .flat_map(|r| (0..row_t / blk).map(move |b| (r, b)))
        .enumerate()
        .map(|(id, (r, b))| WorkItem {
            id: WorkItemId::new(id),
            coord: (r, b),
            out: TileRect {
                row0_t: r,
                col0_t: b * blk,
                h_t: 1,
                w_t: blk,
            },
        })
        .collect()
}

/// Whether a `rows × cols` block moves between flat rows and a dense matrix a tile row at a time: whole tiles across.
pub(super) fn relayout_tiles(cols: usize, target: &SpatialTarget) -> bool {
    cols % target.tile.cols == 0
}

/// `to` — a DENSE `rows × cols` matrix — from `from`, the same matrix as flat rows.
///
/// ★ A PAGE IS A FLAT ROW-MAJOR PLANE, SO A RELAYOUT IS AN AFFINE MAP. Item `(R, b)` loads the flat tiles dense tile
/// row `R` holds — its `h` rows' `blk`-tile segments, `row_t` tiles apart — as ONE page of `h·blk` tiles side by
/// side, and writes `out(y, c) = in[y·(blk·tile) + c]`: row `y` of the tile row is the page's `y`-th segment. Every
/// item of a section loads the same `h` rows, so rows that are not whole tile rows take TWO sections — the whole tile
/// rows, then the last tile row's `rows mod tile` rows — each writing its own tiles of `to` (a part of it, so each
/// declares it updated in part). A row that is not whole tiles has no segments of its own; then the block must be one
/// small tile row, loaded whole — `out(y, c) = in[y·cols + c]` below `rows` and `cols`, zero past them.
pub(crate) fn relayout_rows_to_dense(
    from: (&str, &Fill),
    to: &str,
    rows: usize,
    cols: usize,
    dt: Precision,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let (tr, tw) = (target.tile.rows, target.tile.cols);
    let row_t = cols.div_ceil(tw);
    let below = |axis: LoopVar, bound: usize, then: Box<TensorBody>| TensorBody::Select {
        cond: Pred::AxisLt(axis, PredRhs::Const(bound)),
        then,
        otherwise: konst(0.0),
    };
    let name = format!("relayout_dense_{rows}x{cols}");
    let declare = |t: &mut Template, role: TensorRole| {
        let src = t.tensor(
            from.0,
            TensorRole::Input,
            1,
            rows * cols,
            dt,
            crate::spatial::passes::from_tensor_op::init_of(from.1, rows * cols),
        );
        (src, t.tensor(to, role, rows, cols, dt, TensorInit::Zero))
    };
    if relayout_tiles(cols, target) {
        // Rows `first … first + n − 1`, a whole number of tile rows or the last tile row's rest, as one section; `part`
        // when the section writes only part of `to`.
        let rows_section =
            |first: usize, n: usize, part: bool| -> Result<SpatialProgram, SpatialReject> {
                let h = n.min(tr);
                let tiles_r = n.div_ceil(tr);
                let r0 = first / tr;
                // A page holds `h` rows' segments of `blk` tiles each: the widest block of columns the site holds — built
                // and measured, so the sizing cannot disagree with the section it sizes.
                let tiled = |blk: usize| -> Result<SpatialProgram, SpatialReject> {
                    let mut t = Template::new(target)?;
                    let (src, dst) = declare(
                        &mut t,
                        if part {
                            TensorRole::State
                        } else {
                            TensorRole::Output
                        },
                    );
                    let w = wide(blk, target);
                    let inp = t.buffer("in_rows", h * blk, dt);
                    let out = t.buffer("out", blk, dt);
                    let load = t.load_block_at(
                        "load_rows",
                        src,
                        first * row_t,
                        vec![(LV_ITEM_Y, tr * row_t), (LV_ITEM_X, blk)],
                        h,
                        blk,
                        row_t,
                        inp,
                    );
                    let s = t.stage(
                        "relayout",
                        StageKind::Compute(ComputeOp::Generic {
                            iters: crate::spatial::passes::from_tensor_op::space(target, w, None),
                            ins: vec![ident_map(w)],
                            out: ident_map(w),
                            body: below(LV_GEN_ROW, h, op(0)),
                        }),
                        vec![inp],
                        vec![out],
                    );
                    let st = t.store_block(
                        "store_dense",
                        dst,
                        r0 * row_t,
                        vec![(LV_ITEM_Y, row_t), (LV_ITEM_X, blk)],
                        1,
                        blk,
                        row_t,
                        out,
                    );
                    t.edge(load, inp, s);
                    t.edge(s, out, st);
                    let items: Vec<WorkItem> = relayout_items(tiles_r, row_t, blk)
                        .into_iter()
                        .map(|mut it| {
                            it.out.row0_t += r0;
                            it
                        })
                        .collect();
                    let n_items = items.len();
                    let mut prog = t.finish(
                        format!("{name}_rows{first}"),
                        items,
                        AccPolicy::for_result(dt),
                        NumericsClaim::Exact,
                        target,
                    )?;
                    let plan = relayout_plan(tiles_r, row_t, blk, dt);
                    prog.work.plan = Some(if part {
                        BlockPlan {
                            grid: (1, n_items),
                            ..plan
                        }
                    } else {
                        plan
                    });
                    Ok(prog)
                };
                let budget = target.max_local_mem_bytes();
                (1..=row_block(row_t, target))
                    .rev()
                    .filter(|d| row_t % d == 0)
                    .map(tiled)
                    .find(|p| p.as_ref().is_ok_and(|p| site_footprint(p) <= budget))
                    .unwrap_or(Err(SpatialReject::NotLowered(
                        "a relayout of one tile row of flat rows does not fit a site",
                    )))
            };
        let whole = if rows <= tr { rows } else { rows - rows % tr };
        return if whole == rows {
            Ok(vec![rows_section(0, rows, false)?])
        } else {
            Ok(vec![
                rows_section(0, whole, true)?,
                rows_section(whole, rows - whole, true)?,
            ])
        };
    }
    // Rows narrower than a tile: a dense tile row's `tile` rows are ONE contiguous run of `tile·cols` flat elements,
    // tile-aligned when `tile·cols` is whole tiles, so each tile row is an item that loads its run as one page and
    // reads `out(y, c) = in[y·cols + c]`. The last tile row's shorter run is a section of its own.
    let run_t = (tr * cols).div_ceil(tw);
    let page_t = if rows <= tr {
        tiles_of(rows * cols, target)
    } else {
        run_t
    };
    if (rows > tr && (tr * cols) % tw != 0) || page_t > row_block_tiles(target) {
        return Err(SpatialReject::NotLowered("a block of rows whose relayout into a matrix is neither whole tiles across nor a tile row of narrow rows one page holds"));
    }
    let narrow_section =
        |first: usize, n: usize, part: bool| -> Result<SpatialProgram, SpatialReject> {
            let (h, tiles_r, r0) = (n.min(tr), n.div_ceil(tr), first / tr);
            let n_t = tiles_of(h * cols, target);
            let mut t = Template::new(target)?;
            let (src, dst) = declare(
                &mut t,
                if part {
                    TensorRole::State
                } else {
                    TensorRole::Output
                },
            );
            let inp = t.buffer("in_rows", n_t, dt);
            let out = t.buffer("out", row_t, dt);
            let load = t.load_block_at(
                "load_rows",
                src,
                r0 * run_t,
                vec![(LV_ITEM_Y, run_t)],
                1,
                n_t,
                n_t,
                inp,
            );
            let flat = ElemAccess {
                base: 0,
                terms: vec![(LV_GEN_ROW, cols as i64), (LV_GEN_COL, 1)],
                indirect: None,
            };
            let w = wide(row_t, target);
            let s = t.stage(
                "relayout",
                StageKind::Compute(ComputeOp::Generic {
                    iters: crate::spatial::passes::from_tensor_op::space(target, w, None),
                    ins: vec![flat],
                    out: ident_map(w),
                    body: below(LV_GEN_ROW, h, Box::new(below(LV_GEN_COL, cols, op(0)))),
                }),
                vec![inp],
                vec![out],
            );
            let st = t.store_block(
                "store_dense",
                dst,
                r0 * row_t,
                vec![(LV_ITEM_Y, row_t)],
                1,
                row_t,
                row_t,
                out,
            );
            t.edge(load, inp, s);
            t.edge(s, out, st);
            let items: Vec<WorkItem> = relayout_items(tiles_r, row_t, row_t)
                .into_iter()
                .map(|mut it| {
                    it.out.row0_t += r0;
                    it
                })
                .collect();
            let n_items = items.len();
            let mut prog = t.finish(
                format!("{name}_rows{first}"),
                items,
                AccPolicy::for_result(dt),
                NumericsClaim::Exact,
                target,
            )?;
            let plan = relayout_plan(tiles_r, row_t, row_t, dt);
            prog.work.plan = Some(if part {
                BlockPlan {
                    grid: (1, n_items),
                    ..plan
                }
            } else {
                plan
            });
            Ok(prog)
        };
    let whole = if rows <= tr { rows } else { rows - rows % tr };
    if whole == rows {
        Ok(vec![narrow_section(0, rows, false)?])
    } else {
        Ok(vec![
            narrow_section(0, whole, true)?,
            narrow_section(whole, rows - whole, true)?,
        ])
    }
}

/// `to` — the flat rows of `from`, a DENSE `rows × cols` matrix whose rows are whole tiles: item `(y, b)` is row `y`'s
/// `b`-th `blk`-tile segment. It fetches dense tile row `⌊y / tile⌋` through the row itself — an [`IndirectTile`] with
/// `elems_per_tile = tile`, as a dense row read does — lifts in-tile row `y mod tile` out of the page, and writes the
/// segment where flat row `y` holds it. Both numbers come from a compile-time table, two tiles per row, so the section
/// is the same for every row count.
pub(crate) fn relayout_dense_to_rows(
    from: (&str, &Fill),
    to: &str,
    rows: usize,
    cols: usize,
    dt: Precision,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    if cols % target.tile.cols != 0 {
        return relayout_small_dense_to_rows(from, to, rows, cols, dt, target);
    }
    let picks: Vec<usize> = (0..rows).collect();
    relayout_dense_picks_to_rows(from, to, rows, &picks, cols, dt, target)
}

/// [`relayout_dense_to_rows`] of the rows `picks` names: flat row `y` of `to` is row `picks[y]` of `from`, a dense
/// `rows × cols` matrix whose rows are whole tiles.
pub(super) fn relayout_dense_picks_to_rows(
    from: (&str, &Fill),
    to: &str,
    rows: usize,
    picks: &[usize],
    cols: usize,
    dt: Precision,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (tr, tw) = (target.tile.rows, target.tile.cols);
    let row_t = cols / tw;
    let blk = row_block(row_t, target);
    let w = wide(blk, target);
    let ix = index_type(target, rows as u64)?;
    let mut t = Template::new(target)?;
    let src = t.tensor(
        from.0,
        TensorRole::Input,
        rows,
        cols,
        dt,
        crate::spatial::passes::from_tensor_op::init_of(from.1, rows * cols),
    );
    let flat_rows = picks.len();
    let dst = t.tensor(
        to,
        TensorRole::Output,
        1,
        flat_rows * cols,
        dt,
        TensorInit::Zero,
    );
    let at: Vec<i64> = picks
        .iter()
        .flat_map(|y| [*y as i64, (*y % tr) as i64])
        .flat_map(|v| std::iter::once(v).chain(std::iter::repeat_n(0, tw - 1)))
        .collect();
    let table = t.tensor(
        &format!("{to}__row_at"),
        TensorRole::Param,
        1,
        2 * flat_rows * tw,
        ix,
        TensorInit::Values(at),
    );
    let row_in = t.buffer("in_row", 1, ix);
    let in_tile = t.buffer("in_tile_row", 1, ix);
    let l_row = t.load_block_at("load_row", table, 0, vec![(LV_ITEM_Y, 2)], 1, 1, 1, row_in);
    let l_in = t.load_block_at(
        "load_in_tile_row",
        table,
        1,
        vec![(LV_ITEM_Y, 2)],
        1,
        1,
        1,
        in_tile,
    );
    let page = t.buffer("in_tile_rows", blk, dt);
    let origin = TileAddr {
        base: 0,
        steps: vec![(LV_ITEM_X, blk)],
        indirect: Some(IndirectTile {
            index: row_in,
            tile_stride: row_t,
            elems_per_tile: tr,
            bound: (0, rows - 1),
        }),
    };
    let load = t.stage(
        "gather_tile_row",
        StageKind::Load {
            tensor: src,
            pattern: AccessPattern {
                origin,
                block_h: 1,
                block_w: blk,
                stride_w: 1,
                stride_h: row_t,
            },
            num_blocks: 1,
            multicast: None,
        },
        vec![row_in],
        vec![page],
    );
    t.edge(l_row, row_in, load);
    let out = t.buffer("out", blk, dt);
    let lift = ElemAccess {
        base: 0,
        terms: vec![(LV_GEN_COL, 1)],
        indirect: Some(IndirectTerm {
            operand: 1,
            stride: w as i64,
            modulus: tr,
        }),
    };
    let s = t.stage(
        "lift_row",
        StageKind::Compute(ComputeOp::Generic {
            iters: flat_space(w),
            ins: vec![lift, fixed(0)],
            out: ident_map(w),
            body: TensorBody::Operand(0),
        }),
        vec![page, in_tile],
        vec![out],
    );
    t.edge(load, page, s);
    t.edge(l_in, in_tile, s);
    let st = t.store_block(
        "store_row",
        dst,
        0,
        vec![(LV_ITEM_Y, row_t), (LV_ITEM_X, blk)],
        1,
        blk,
        row_t,
        out,
    );
    t.edge(s, out, st);
    let items: Vec<WorkItem> = (0..flat_rows)
        .flat_map(|y| (0..row_t / blk).map(move |b| (y, b)))
        .enumerate()
        .map(|(id, (y, b))| WorkItem {
            id: WorkItemId::new(id),
            coord: (y, b),
            out: TileRect {
                row0_t: 0,
                col0_t: y * row_t + b * blk,
                h_t: 1,
                w_t: blk,
            },
        })
        .collect();
    let name = if flat_rows == rows && picks.iter().enumerate().all(|(y, r)| y == *r) {
        format!("relayout_rows_{rows}x{cols}")
    } else {
        format!("relayout_rows_{flat_rows}_of_{rows}x{cols}")
    };
    let mut prog = t.finish(
        name,
        items,
        AccPolicy::for_result(dt),
        NumericsClaim::Exact,
        target,
    )?;
    // The grid counts the flat result's blocks: one tile row of every row's segments.
    prog.work.plan = Some(BlockPlan {
        grid: (1, flat_rows * (row_t / blk)),
        ..tile_row_plan(row_t, blk, dt)
    });
    Ok(prog)
}

/// [`relayout_dense_to_rows`] for a matrix whose rows are not whole tiles: then it must be one small tile row, loaded
/// whole, and flat element `C = y·cols + c` is dense page element `(y, c)` — at plane offset `C + y·(W − cols)`, `W` the
/// page's width — a map affine within row `y`'s band of `cols` columns.
pub(super) fn relayout_small_dense_to_rows(
    from: (&str, &Fill),
    to: &str,
    rows: usize,
    cols: usize,
    dt: Precision,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let row_t = cols.div_ceil(target.tile.cols);
    let n_t = tiles_of(rows * cols, target);
    if rows > target.tile.rows || row_t > row_block_tiles(target) || n_t > row_block_tiles(target) {
        return Err(SpatialReject::NotLowered("a matrix whose relayout into flat rows is neither whole tiles across nor one small tile row"));
    }
    let w = wide(row_t, target);
    let mut t = Template::new(target)?;
    let src = t.tensor(
        from.0,
        TensorRole::Input,
        rows,
        cols,
        dt,
        crate::spatial::passes::from_tensor_op::init_of(from.1, rows * cols),
    );
    let dst = t.tensor(to, TensorRole::Output, 1, rows * cols, dt, TensorInit::Zero);
    let inp = t.buffer("in_dense", row_t, dt);
    let out = t.buffer("out", n_t, dt);
    let load = t.load_block_at("load_dense", src, 0, Vec::new(), 1, row_t, row_t, inp);
    let maps: Vec<ElemAccess> = (0..rows)
        .map(|y| ElemAccess {
            base: (y * (w - cols)) as i64,
            terms: vec![(LV_GEN_COL, 1)],
            indirect: None,
        })
        .collect();
    let bands: Vec<(usize, TensorBody)> = (0..rows)
        .map(|y| ((y + 1) * cols, TensorBody::Operand(y)))
        .collect();
    let s = t.stage(
        "relayout",
        StageKind::Compute(ComputeOp::Generic {
            iters: flat_space(rows * cols),
            ins: maps,
            out: ident_map(wide(n_t, target)),
            body: banded(bands),
        }),
        vec![inp; rows],
        vec![out],
    );
    let st = t.store_block("store_rows", dst, 0, Vec::new(), 1, n_t, n_t, out);
    t.edge(load, inp, s);
    t.edge(s, out, st);
    let items = vec![WorkItem {
        id: WorkItemId::new(0),
        coord: (0, 0),
        out: TileRect {
            row0_t: 0,
            col0_t: 0,
            h_t: 1,
            w_t: n_t,
        },
    }];
    let mut prog = t.finish(
        format!("relayout_rows_{rows}x{cols}"),
        items,
        AccPolicy::for_result(dt),
        NumericsClaim::Exact,
        target,
    )?;
    prog.work.plan = Some(BlockPlan {
        grid: (1, 1),
        ..tile_row_plan(n_t, n_t, dt)
    });
    Ok(prog)
}
