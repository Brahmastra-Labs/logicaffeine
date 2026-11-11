//! Gathers and selection: rows read and written by index, and the arg-max.

use super::*;

/// `lo = (index − 1)·cols` — the flat offset of a 1-based row, computed on the INTEGER datapath (at `ix`, the
/// index type) from a one-element index operand. Returns the stages and the buffer holding `lo`.
///
/// ★ THE MULTIPLY MAY NOT WRAP. On a 32-bit datapath `(index − 1)·cols` wraps once the index passes
/// `INT_MAX / cols`, and a wrapped offset can land inside the table — a silently wrong row. So the row is GUARDED
/// first, by a gather of stride zero: `index − 1 + B ∈ [0, 2B)` with `B = ⌊INT_MAX / cols⌋`. That range is exactly
/// `|index − 1| ≤ B` even where the addition itself wraps, and inside it the multiply cannot overflow. A row just
/// outside the table passes the guard and faults at the offset's own range check, which names the offset.
pub(super) fn row_offset_stages(
    b: &mut FlatBuild,
    index: BufferId,
    cols: usize,
    ix: Precision,
    target: &SpatialTarget,
) -> (Vec<StageId>, BufferId) {
    let one = wide(1, target);
    let bound = crate::spatial::target::int_precision_max(ix).expect("an index type is an integer")
        / cols as i64;
    let biased = b.t.scratch("row_biased", 1, ix);
    let s0 = b.compute(
        "row_biased",
        flat_space(1),
        vec![Slot {
            buf: index,
            map: fixed(0),
        }],
        ident_map(one),
        *add(sub(op(0), konst(1.0)), konst(bound as f64)),
        biased,
    );
    let guard = ElemAccess {
        base: 0,
        terms: Vec::new(),
        indirect: Some(IndirectTerm {
            operand: 0,
            stride: 0,
            modulus: 2 * bound as usize,
        }),
    };
    let row = b.t.scratch("row_guarded", 1, ix);
    let s1 = b.compute(
        "row_guarded",
        flat_space(1),
        vec![Slot {
            buf: biased,
            map: guard,
        }],
        ident_map(one),
        *sub(op(0), konst(bound as f64)),
        row,
    );
    let lo = b.t.scratch("row_offset", 1, ix);
    let s2 = b.compute(
        "row_offset",
        flat_space(1),
        vec![Slot {
            buf: row,
            map: fixed(0),
        }],
        ident_map(one),
        *mul(op(0), konst(cols as f64)),
        lo,
    );
    (vec![s0, s1, s2], lo)
}

/// A literal row index, checked at compile time: the flat offset of row `index` of a `rows`-row table.
pub(super) fn literal_row_offset(index: i64, rows: usize, cols: usize) -> Result<i64, SpatialReject> {
    if index < 1 || index as usize > rows {
        return Err(SpatialReject::NotLowered("a literal row index outside its table — the program fails at run time on every tier, so there is no section to lower"));
    }
    Ok((index - 1) * cols as i64)
}

/// `x[j] = T[lo + j]` — the embedding lookup: row `index` (1-based) of a flat `rows·cols` table.
///
/// ★ THE ROW IS DATA. A named index is an operand: its flat offset `lo = (index − 1)·cols` is computed on the
/// integer datapath and the gather reads `T` through an [`IndirectTerm`] of extent `rows·cols` — so a token
/// id past the table (or below 1: `lo < 0`) FAULTS in the evaluator, exactly where every tier's run-time
/// check fails, and is never wrapped into somebody else's row.
///
/// `dense`: the table is laid out as a matrix, a tile row holding `tile` rows — see `dense_row_gather`.
pub fn from_row_gather(
    spec: &RowGatherSpec,
    names: &StageNames,
    float: bool,
    dense: bool,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols) = (spec.rows, spec.cols);
    if rows == 0 || cols == 0 {
        return Err(SpatialReject::NotLowered(
            "a row gather from an empty table",
        ));
    }
    if dense {
        return dense_row_gather(spec, names, float, target);
    }
    if cols % target.tile.cols == 0 {
        return tile_row_gather(spec, names, float, target);
    }
    let n = rows * cols;
    let mut b = FlatBuild::new(target)?;
    let (t_name, t_fill) = names.operand(0);
    let dt = table_precision(&b, float, t_fill, target)?;
    let table = b.input_as(t_name, n, t_fill, dt, target);
    let out_buf = b.t.buffer("row", tiles_of(cols, target), dt);
    let ow = wide(tiles_of(cols, target), target);
    let chain = match spec.index {
        RowIndex::Literal(i) => {
            let lo = literal_row_offset(i, rows, cols)?;
            vec![b.compute(
                "gather_row",
                flat_space(cols),
                vec![Slot {
                    buf: table,
                    map: at(lo),
                }],
                ident_map(ow),
                TensorBody::Operand(0),
                out_buf,
            )]
        }
        RowIndex::Named(_) => {
            let (i_name, i_fill) = names.operand(1);
            let ix = index_type(target, index_space(n as u64, cols as u64, i_fill))?;
            let index = b.input_as(i_name, 1, i_fill, ix, target);
            let (mut chain, lo) = row_offset_stages(&mut b, index, cols, ix, target);
            let gather = ElemAccess {
                base: 0,
                terms: vec![(LV_GEN_COL, 1)],
                indirect: Some(IndirectTerm {
                    operand: 1,
                    stride: 1,
                    modulus: n,
                }),
            };
            chain.push(b.compute(
                "gather_row",
                flat_space(cols),
                vec![
                    Slot {
                        buf: table,
                        map: gather,
                    },
                    Slot {
                        buf: lo,
                        map: fixed(0),
                    },
                ],
                ident_map(ow),
                TensorBody::Operand(0),
                out_buf,
            ));
            chain
        }
    };
    let claim = if float {
        approx("row gather", dt)
    } else {
        NumericsClaim::Exact
    };
    b.finish_as(
        &names.out,
        cols,
        chain,
        out_buf,
        format!("row_gather_{rows}x{cols}"),
        claim,
        dt,
        target,
    )
}

/// `X = rows I of T` — the block gather: row `k` of `X` is row `I[k]` (1-based) of a flat `rows·cols` table, for every
/// index of `I` at once, in ONE section; `X` is written as flat rows (the layout pass bridges it once where the program
/// lays `X` out as a matrix).
///
/// ★ EVERY INDEX IS DATA, CHECKED WHERE IT IS READ: taken less one and read through a guard of stride zero against the
/// table's rows, so an index outside the table faults where every tier's check fails instead of reading another row.
///   · Rows of WHOLE TILES (a real embedding table): item `(k, b)` moves row `k`'s `b`-th `blk`-tile block. It reads its
///     own index — element `k` of `I`, `k` taken from a compile-time table holding it in tile `k` — and loads the row's
///     block through an [`IndirectTile`] over the guarded `I[k] − 1`, so exactly the gathered rows' tiles move.
///   · Rows narrower than a tile: one item gathers the block from the table held whole — one band of the output per
///     row, each reading the table at its row's offset `(I[k] − 1)·cols` — refused by name when the table or the block
///     is wider than a site's page.
pub fn from_rows_gather(
    spec: &RowsGatherSpec,
    names: &StageNames,
    float: bool,
    dense: bool,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols, n) = (spec.rows, spec.cols, spec.count);
    if rows == 0 || cols == 0 || n == 0 {
        return Err(SpatialReject::NotLowered(
            "a rows gather from an empty table, or of no indices",
        ));
    }
    if dense {
        return Err(SpatialReject::NotLowered(
            "a rows gather from a table laid out as a matrix",
        ));
    }
    if cols % target.tile.cols == 0 {
        return tile_rows_gather(spec, names, float, target);
    }
    let tn = rows * cols;
    if tiles_of(tn, target) > row_block_tiles(target)
        || tiles_of(n * cols, target) > row_block_tiles(target)
    {
        return Err(SpatialReject::NotLowered("a rows gather of rows narrower than a tile holds its table and its block whole, and this one is wider than a site's page"));
    }
    let mut b = FlatBuild::new(target)?;
    let (t_name, t_fill) = names.operand(0);
    let dt = table_precision(&b, float, t_fill, target)?;
    let table = b.input_as(t_name, tn, t_fill, dt, target);
    let (i_name, i_fill) = names.operand(1);
    let ix = index_type(target, index_space(tn as u64, cols as u64, i_fill))?;
    let idx = b.input_as(i_name, n, i_fill, ix, target);
    let (i_t, w_n) = (tiles_of(n, target), wide(tiles_of(n, target), target));
    let biased = b.t.scratch("rows_biased", i_t, ix);
    let s0 = b.compute(
        "rows_biased",
        flat_space(n),
        vec![Slot {
            buf: idx,
            map: at(0),
        }],
        ident_map(w_n),
        *sub(op(0), konst(1.0)),
        biased,
    );
    let guard = ElemAccess {
        base: 0,
        terms: vec![(LV_GEN_COL, 1)],
        indirect: Some(IndirectTerm {
            operand: 0,
            stride: 0,
            modulus: rows,
        }),
    };
    let guarded = b.t.scratch("rows_guarded", i_t, ix);
    let s1 = b.compute(
        "rows_guarded",
        flat_space(n),
        vec![Slot {
            buf: biased,
            map: guard,
        }],
        ident_map(w_n),
        TensorBody::Operand(0),
        guarded,
    );
    let lo = b.t.scratch("rows_offset", i_t, ix);
    let s2 = b.compute(
        "rows_offset",
        flat_space(n),
        vec![Slot {
            buf: guarded,
            map: at(0),
        }],
        ident_map(w_n),
        *mul(op(0), konst(cols as f64)),
        lo,
    );
    // Band `k` — the output's columns `[k·cols, (k + 1)·cols)` — reads the table at `c − k·cols + (I[k] − 1)·cols`.
    let (mut slots, mut bands) = (Vec::new(), Vec::new());
    for k in 0..n {
        slots.push(Slot {
            buf: lo,
            map: fixed(k as i64),
        });
        let offset = slots.len() - 1;
        slots.push(Slot {
            buf: table,
            map: ElemAccess {
                indirect: Some(IndirectTerm {
                    operand: offset,
                    stride: 1,
                    modulus: tn,
                }),
                ..at(-((k * cols) as i64))
            },
        });
        bands.push(((k + 1) * cols, TensorBody::Operand(slots.len() - 1)));
    }
    let out_t = tiles_of(n * cols, target);
    let out_buf = b.t.buffer("rows", out_t, dt);
    let s3 = b.compute(
        "gather_rows",
        flat_space(n * cols),
        slots,
        ident_map(wide(out_t, target)),
        banded(bands),
        out_buf,
    );
    let claim = if float {
        approx("rows gather", dt)
    } else {
        NumericsClaim::Exact
    };
    b.finish_as(
        &names.out,
        n * cols,
        vec![s0, s1, s2, s3],
        out_buf,
        format!("rows_gather_{n}x{rows}x{cols}"),
        claim,
        dt,
        target,
    )
}

/// [`from_rows_gather`] for a table whose rows are WHOLE TILES: item `(k, b)` moves row `I[k]`'s `b`-th `blk`-tile block
/// into row `k` of `X`.
pub(super) fn tile_rows_gather(
    spec: &RowsGatherSpec,
    names: &StageNames,
    float: bool,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols, n) = (spec.rows, spec.cols, spec.count);
    let tw = target.tile.cols;
    let row_t = cols / tw;
    let blk = row_block(row_t, target);
    let tn = rows * cols;
    let (t_name, t_fill) = names.operand(0);
    let dt = if float {
        elem_precision(target)?
    } else {
        index_type(target, fill_abs_bound(t_fill).unwrap_or(0))?
    };
    let (i_name, i_fill) = names.operand(1);
    let ix = index_type(target, index_space(rows.max(n) as u64, 1, i_fill))?;
    let mut t = Template::new(target)?;
    let table = t.tensor(
        t_name,
        TensorRole::Input,
        1,
        tn,
        dt,
        crate::spatial::passes::from_tensor_op::init_of(t_fill, tn),
    );
    let indices = t.tensor(
        i_name,
        TensorRole::Input,
        1,
        n,
        ix,
        crate::spatial::passes::from_tensor_op::init_of(i_fill, n),
    );
    // Item `k`'s own number, in tile `k` of a compile-time table: what makes element `k` of `I` an item's own read.
    let item_of = t.tensor(
        &format!("{}__rows_item", names.out),
        TensorRole::Param,
        1,
        n * tw,
        ix,
        TensorInit::Values(
            (0..n as i64)
                .flat_map(|k| std::iter::once(k).chain(std::iter::repeat_n(0, tw - 1)))
                .collect(),
        ),
    );
    let w1 = wide(1, target);
    let scalar = |t: &mut Template,
                  name: &str,
                  ins: Vec<BufferId>,
                  maps: Vec<ElemAccess>,
                  body: TensorBody,
                  out: BufferId| {
        t.stage(
            name,
            StageKind::Compute(ComputeOp::Generic {
                iters: flat_space(1),
                ins: maps,
                out: ident_map(w1),
                body,
            }),
            ins,
            vec![out],
        )
    };
    let i_t = tiles_of(n, target);
    let iin = t.buffer("in_indices", i_t, ix);
    let li = t.load("load_indices", indices, vec![], i_t, iin);
    let kin = t.buffer("in_item", 1, ix);
    let lk = t.load_block_at("load_item", item_of, 0, vec![(LV_ITEM_Y, 1)], 1, 1, 1, kin);
    // `I[k] − 1`: element `k` of the indices, then checked against the table's rows.
    let pick = ElemAccess {
        base: 0,
        terms: Vec::new(),
        indirect: Some(IndirectTerm {
            operand: 1,
            stride: 1,
            modulus: n,
        }),
    };
    let biased = t.scratch_deep("index_biased", 1, 1, ix);
    let s_biased = scalar(
        &mut t,
        "index_biased",
        vec![iin, kin],
        vec![pick, fixed(0)],
        *sub(op(0), konst(1.0)),
        biased,
    );
    t.edge(li, iin, s_biased);
    t.edge(lk, kin, s_biased);
    let row0 = t.buffer("row_index", 1, ix);
    let guard = ElemAccess {
        base: 0,
        terms: Vec::new(),
        indirect: Some(IndirectTerm {
            operand: 0,
            stride: 0,
            modulus: rows,
        }),
    };
    let s_row = scalar(
        &mut t,
        "row_index",
        vec![biased],
        vec![guard],
        TensorBody::Operand(0),
        row0,
    );
    t.edge(s_biased, biased, s_row);
    let origin = TileAddr {
        base: 0,
        steps: vec![(LV_ITEM_X, blk)],
        indirect: Some(IndirectTile {
            index: row0,
            tile_stride: row_t,
            elems_per_tile: 1,
            bound: (0, rows - 1),
        }),
    };
    let row = t.buffer("in_row", blk, dt);
    let load = t.stage(
        "gather_row",
        StageKind::Load {
            tensor: table,
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
        vec![row0],
        vec![row],
    );
    t.edge(s_row, row0, load);
    let (copy, out) = copy_page(&mut t, row, blk, dt, target);
    t.edge(load, row, copy);
    let x = t.tensor(
        &names.out,
        TensorRole::Output,
        1,
        n * cols,
        dt,
        TensorInit::Zero,
    );
    let st = t.store(
        &format!("store_{}", names.out),
        x,
        vec![(LV_ITEM_Y, row_t), (LV_ITEM_X, blk)],
        blk,
        out,
    );
    t.edge(copy, out, st);
    let items: Vec<WorkItem> = (0..n)
        .flat_map(|k| (0..row_t / blk).map(move |b| (k, b)))
        .enumerate()
        .map(|(id, (k, b))| WorkItem {
            id: WorkItemId::new(id),
            coord: (k, b),
            out: TileRect {
                row0_t: 0,
                col0_t: k * row_t + b * blk,
                h_t: 1,
                w_t: blk,
            },
        })
        .collect();
    let claim = if float {
        approx("rows gather", dt)
    } else {
        NumericsClaim::Exact
    };
    t.finish(
        format!("rows_gather_{n}x{rows}x{cols}"),
        items,
        AccPolicy::F32,
        claim,
        target,
    )
}

/// The address of item `i`'s `blk`-tile block of a whole-tile row of a flat `rows × (row_t · tile)` table. A
/// literal row is affine. A named row adds an [`IndirectTile`] over the 0-based row `index − 1`, computed per item
/// on the integer datapath (at the target's index type) into a FIFO the movement engine waits on and retires once
/// the address is resolved. Returns the address and the index path's `(stage, buffer)`.
pub(super) fn row_block_address(
    t: &mut Template,
    index: RowIndex,
    operand: (&str, &Fill),
    rows: usize,
    row_t: usize,
    blk: usize,
    target: &SpatialTarget,
) -> Result<(TileAddr, Option<(StageId, BufferId)>), SpatialReject> {
    let steps = vec![(LV_ITEM_X, blk)];
    match index {
        RowIndex::Literal(i) => {
            if i < 1 || i as usize > rows {
                return Err(SpatialReject::NotLowered("a literal row index outside its table — the program fails at run time on every tier, so there is no section to lower"));
            }
            Ok((
                TileAddr {
                    base: (i as usize - 1) * row_t,
                    steps,
                    indirect: None,
                },
                None,
            ))
        }
        RowIndex::Named(_) => {
            let ix = index_type(target, index_space(rows as u64, 1, operand.1))?;
            let tok = t.tensor(
                operand.0,
                TensorRole::Input,
                1,
                1,
                ix,
                crate::spatial::passes::from_tensor_op::init_of(operand.1, 1),
            );
            let tin = t.buffer(&format!("in_{}", operand.0), 1, ix);
            let load = t.load(&format!("load_{}", operand.0), tok, vec![], 1, tin);
            let row0 = t.buffer("row_index", 1, ix);
            let w1 = wide(1, target);
            let s = t.stage(
                "row_index",
                StageKind::Compute(ComputeOp::Generic {
                    iters: flat_space(1),
                    ins: vec![fixed(0)],
                    out: ident_map(w1),
                    body: *sub(op(0), konst(1.0)),
                }),
                vec![tin],
                vec![row0],
            );
            t.edge(load, tin, s);
            let indirect = Some(IndirectTile {
                index: row0,
                tile_stride: row_t,
                elems_per_tile: 1,
                bound: (0, rows - 1),
            });
            Ok((
                TileAddr {
                    base: 0,
                    steps,
                    indirect,
                },
                Some((s, row0)),
            ))
        }
    }
}

/// The items of a whole-tile row moved `blk` tiles at a time: item `i` owns tiles `[i·blk, (i+1)·blk)` of the row.
pub(super) fn row_items(row_t: usize, blk: usize) -> Vec<WorkItem> {
    (0..row_t / blk)
        .map(|i| WorkItem {
            id: WorkItemId::new(i),
            coord: (0, i),
            out: TileRect {
                row0_t: 0,
                col0_t: i * blk,
                h_t: 1,
                w_t: blk,
            },
        })
        .collect()
}

/// A tile copy of one `blk`-tile page through the tile datapath.
pub(super) fn copy_page(
    t: &mut Template,
    from: BufferId,
    blk: usize,
    dt: Precision,
    target: &SpatialTarget,
) -> (StageId, BufferId) {
    let out = t.buffer("row", blk, dt);
    let w = wide(blk, target);
    let s = t.stage(
        "copy_row",
        StageKind::Compute(ComputeOp::Generic {
            iters: flat_space(w),
            ins: vec![ident_map(w)],
            out: ident_map(w),
            body: TensorBody::Operand(0),
        }),
        vec![from],
        vec![out],
    );
    (s, out)
}

/// `x = row i of T` for a table whose rows are WHOLE TILES — the real-width embedding lookup. Each work item
/// Loads its `blk` tiles of the row (an [`IndirectTile`] for a named row) and passes them out, so exactly the
/// row's tiles move and a table far wider than a site is never brought on chip.
pub(super) fn tile_row_gather(
    spec: &RowGatherSpec,
    names: &StageNames,
    float: bool,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols) = (spec.rows, spec.cols);
    let row_t = cols / target.tile.cols;
    let blk = row_block(row_t, target);
    let n = rows * cols;
    let (t_name, t_fill) = names.operand(0);
    let dt = if float {
        elem_precision(target)?
    } else {
        index_type(target, fill_abs_bound(t_fill).unwrap_or(0))?
    };
    let mut t = Template::new(target)?;
    let table = t.tensor(
        t_name,
        TensorRole::Input,
        1,
        n,
        dt,
        crate::spatial::passes::from_tensor_op::init_of(t_fill, n),
    );
    let index_operand = if matches!(spec.index, RowIndex::Named(_)) {
        names.operand(1)
    } else {
        (t_name, t_fill)
    };
    let (addr, index) =
        row_block_address(&mut t, spec.index, index_operand, rows, row_t, blk, target)?;
    let x = t.tensor(
        &names.out,
        TensorRole::Output,
        1,
        cols,
        dt,
        TensorInit::Zero,
    );
    let row = t.buffer("in_row", blk, dt);
    let load = t.stage(
        "gather_row",
        StageKind::Load {
            tensor: table,
            pattern: AccessPattern {
                origin: addr,
                block_h: 1,
                block_w: blk,
                stride_w: 1,
                stride_h: row_t,
            },
            num_blocks: 1,
            multicast: None,
        },
        index.map(|(_, b)| vec![b]).unwrap_or_default(),
        vec![row],
    );
    if let Some((s, b)) = index {
        t.edge(s, b, load);
    }
    let (copy, out) = copy_page(&mut t, row, blk, dt, target);
    t.edge(load, row, copy);
    let st = t.store(
        &format!("store_{}", names.out),
        x,
        vec![(LV_ITEM_X, blk)],
        blk,
        out,
    );
    t.edge(copy, out, st);
    let claim = if float {
        approx("row gather", dt)
    } else {
        NumericsClaim::Exact
    };
    t.finish(
        format!("row_gather_{rows}x{cols}"),
        row_items(row_t, blk),
        AccPolicy::F32,
        claim,
        target,
    )
}

/// `Set row i of T to S` for a table whose rows are WHOLE TILES — the real-width KV-cache append. Each work item
/// Stores its `blk` tiles of the source into the row (an [`IndirectTile`] scatter for a named row), so exactly
/// the row's tiles move.
///
/// ★ THE TABLE IS `State`, NOT AN `Output`: the step updates it IN PLACE. An output must be written whole; this
/// section writes one row and every other tile carries the value it came in with — which is why the table is
/// declared with its SOURCE fill, the value the untouched rows hold.
pub(super) fn tile_set_row(
    spec: &SetRowSpec,
    names: &StageNames,
    float: bool,
    held: Option<Precision>,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    // A block of `count` rows is `count·row_t` contiguous tiles from its first row's: the one-row scatter, wider, its
    // first row bounded so the last is the table's.
    let span_t = spec.count * (spec.cols / target.tile.cols);
    fitting_block(span_t, &|blk| tile_set_row_in(spec, names, float, held, blk, target), target)
}

/// [`tile_set_row`] in blocks of `blk` tiles of the rows' run.
pub(super) fn tile_set_row_in(
    spec: &SetRowSpec,
    names: &StageNames,
    float: bool,
    held: Option<Precision>,
    blk: usize,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols, count) = (spec.rows, spec.cols, spec.count);
    let row_t = cols / target.tile.cols;
    let span_t = count * row_t;
    let n = rows * cols;
    let (t_name, t_fill) = names.operand(0);
    let (s_name, s_fill) = names.operand(1);
    let dt = if float {
        row_float(held, target)?
    } else {
        index_type(
            target,
            fill_abs_bound(t_fill)
                .unwrap_or(0)
                .max(fill_abs_bound(s_fill).unwrap_or(0)),
        )?
    };
    let mut t = Template::new(target)?;
    let src = t.tensor(
        s_name,
        TensorRole::Input,
        1,
        count * cols,
        dt,
        crate::spatial::passes::from_tensor_op::init_of(s_fill, count * cols),
    );
    let index_operand = if matches!(spec.index, RowIndex::Named(_)) {
        names.operand(2)
    } else {
        (s_name, s_fill)
    };
    let (addr, index) = row_block_address(
        &mut t,
        spec.index,
        index_operand,
        rows + 1 - count,
        row_t,
        blk,
        target,
    )?;
    let table = t.tensor(
        t_name,
        TensorRole::State,
        1,
        n,
        dt,
        crate::spatial::passes::from_tensor_op::init_of(t_fill, n),
    );
    let sin = t.buffer(&format!("in_{s_name}"), blk, dt);
    let ls = t.load(
        &format!("load_{s_name}"),
        src,
        vec![(LV_ITEM_X, blk)],
        blk,
        sin,
    );
    let (copy, out) = copy_page(&mut t, sin, blk, dt, target);
    t.edge(ls, sin, copy);
    let st = t.stage(
        &format!("scatter_{t_name}"),
        StageKind::Store {
            tensor: table,
            pattern: AccessPattern {
                origin: addr,
                block_h: 1,
                block_w: blk,
                stride_w: 1,
                stride_h: row_t,
            },
            sub_blocks: (1, 1),
        },
        std::iter::once(out).chain(index.map(|(_, b)| b)).collect(),
        vec![],
    );
    t.edge(copy, out, st);
    if let Some((s, b)) = index {
        t.edge(s, b, st);
    }
    let claim = if float {
        approx("row write", dt)
    } else {
        NumericsClaim::Exact
    };
    let mut prog = t.finish(
        set_row_name(spec, ""),
        row_items(span_t, blk),
        AccPolicy::F32,
        claim,
        target,
    )?;
    // ★ THE PLAN IS THE ROW'S, NOT THE TABLE'S. `trivial_plan` sizes its grid from the result tensor, and the
    // result here is the whole carried table — a grid of every row, where the work is one.
    prog.work.plan = Some(BlockPlan {
        per_site_m: 1,
        per_site_n: blk,
        grid: (1, span_t / blk),
        inner: InnerLoop {
            num_blocks: 1,
            block_k_tiles: 1,
        },
        block: BlockShape {
            osh: 1,
            osw: blk,
            ibw: 1,
        },
        acc: AccPolicy::for_result(dt),
        pack_accumulate: false,
        engines: EnginePacking::LoadsTogether,
        granularity: Granularity::PerTile,
    });
    Ok(prog)
}

/// Where row `r = i − 1` of a DENSE `rows × cols` table lives: tile row `⌊r / tile⌋`, at in-tile row `r mod tile`.
///
/// ★ THE DIVISION STAYS IN THE ADDRESS. `⌊r / tile⌋` is what an [`IndirectTile`] with `elems_per_tile = tile`
/// computes, so the tile row is addressed by `r` itself. The in-tile row `r mod tile` is `r − tile·⌊r / tile⌋`:
/// the second term is read the same way from a compile-time table whose tile `q` holds `tile·q` — one tile moved,
/// no division on the datapath.
pub(super) struct DenseRow {
    /// One tile-row address per movement that touches the row, each with the index copy it waits on.
    tile_rows: Vec<(TileAddr, Option<(StageId, BufferId)>)>,
    in_tile: InTileRow,
}

/// The in-tile row of a dense row: known at compile time, or computed onto the integer datapath.
#[derive(Clone, Copy)]
pub(super) enum InTileRow {
    Fixed(usize),
    Data {
        row: BufferId,
        stage: StageId,
        ix: Precision,
    },
}

/// The addresses of row `index` of a dense `rows`-row table whose rows are `row_t` tiles, for `movements` tile
/// moves of `steps` — every consumer of the index waits on its own copy of `r = i − 1`.
pub(super) fn dense_row(
    t: &mut Template,
    index: RowIndex,
    operand: (&str, &Fill),
    table: &str,
    rows: usize,
    row_t: usize,
    steps: &[(LoopVar, usize)],
    movements: usize,
    target: &SpatialTarget,
) -> Result<DenseRow, SpatialReject> {
    let tr = target.tile.rows;
    match index {
        RowIndex::Literal(i) => {
            if i < 1 || i as usize > rows {
                return Err(SpatialReject::NotLowered("a literal row index outside its table — the program fails at run time on every tier, so there is no section to lower"));
            }
            let r = i as usize - 1;
            let at = TileAddr {
                base: (r / tr) * row_t,
                steps: steps.to_vec(),
                indirect: None,
            };
            Ok(DenseRow {
                tile_rows: vec![(at, None); movements],
                in_tile: InTileRow::Fixed(r % tr),
            })
        }
        RowIndex::Named(_) => {
            let (i_name, i_fill) = operand;
            let ix = index_type(target, index_space(rows as u64, 1, i_fill))?;
            let pos = t.tensor(
                i_name,
                TensorRole::Input,
                1,
                1,
                ix,
                crate::spatial::passes::from_tensor_op::init_of(i_fill, 1),
            );
            let w1 = wide(1, target);
            let row_index = |t: &mut Template, what: &str| {
                let tin = t.buffer(&format!("in_{i_name}_{what}"), 1, ix);
                let load = t.load(&format!("load_{i_name}_{what}"), pos, vec![], 1, tin);
                let r = t.buffer(&format!("row_{what}"), 1, ix);
                let s = t.stage(
                    &format!("row_{what}"),
                    StageKind::Compute(ComputeOp::Generic {
                        iters: flat_space(1),
                        ins: vec![fixed(0)],
                        out: ident_map(w1),
                        body: *sub(op(0), konst(1.0)),
                    }),
                    vec![tin],
                    vec![r],
                );
                t.edge(load, tin, s);
                (s, r)
            };
            let names = ["gather", "scatter"];
            let feeds: Vec<(StageId, BufferId)> = (0..movements)
                .map(|m| row_index(t, names.get(m).copied().unwrap_or("move")))
                .collect();
            let (s_base, r_base) = row_index(t, "base");
            let (s_in, r_in) = row_index(t, "in_tile");
            let tile_rows = feeds
                .into_iter()
                .map(|(s, index)| {
                    (
                        TileAddr {
                            base: 0,
                            steps: steps.to_vec(),
                            indirect: Some(IndirectTile {
                                index,
                                tile_stride: row_t,
                                elems_per_tile: tr,
                                bound: (0, rows - 1),
                            }),
                        },
                        Some((s, index)),
                    )
                })
                .collect();
            let rows_t = rows.div_ceil(tr);
            let bases: Vec<i64> = (0..rows_t * target.tile.cols)
                .map(|e| ((e / target.tile.cols) * tr) as i64)
                .collect();
            let base_table = t.tensor(
                &format!("{table}__row_bases"),
                TensorRole::Param,
                1,
                rows_t * target.tile.cols,
                ix,
                TensorInit::Values(bases),
            );
            let base_in = t.buffer("in_row_base", 1, ix);
            let base_addr = TileAddr {
                base: 0,
                steps: Vec::new(),
                indirect: Some(IndirectTile {
                    index: r_base,
                    tile_stride: 1,
                    elems_per_tile: tr,
                    bound: (0, rows - 1),
                }),
            };
            let l_base = t.stage(
                "load_row_base",
                StageKind::Load {
                    tensor: base_table,
                    pattern: AccessPattern {
                        origin: base_addr,
                        block_h: 1,
                        block_w: 1,
                        stride_w: 1,
                        stride_h: 1,
                    },
                    num_blocks: 1,
                    multicast: None,
                },
                vec![r_base],
                vec![base_in],
            );
            t.edge(s_base, r_base, l_base);
            let row = t.scratch_deep("in_tile_row_index", 1, 1, ix);
            let stage = t.stage(
                "in_tile_row_index",
                StageKind::Compute(ComputeOp::Generic {
                    iters: flat_space(1),
                    ins: vec![fixed(0), fixed(0)],
                    out: ident_map(w1),
                    body: *sub(op(0), op(1)),
                }),
                vec![r_in, base_in],
                vec![row],
            );
            t.edge(s_in, r_in, stage);
            t.edge(l_base, base_in, stage);
            Ok(DenseRow {
                tile_rows,
                in_tile: InTileRow::Data { row, stage, ix },
            })
        }
    }
}

/// `Set row i of T to S` for a DENSE table — `rows × cols`, a tile row holding `tile` rows: the KV cache a flash
/// attention reads, a stacked batch a matmul reads. Row `r = i − 1` lives where [`dense_row`] says, so each work
/// item READS its `blk` tiles of that tile row, replaces in-tile row `r mod tile` with its `blk` tiles of the
/// source, and WRITES them back: `T'(y, c) = (y = r mod tile) ? S(c) : T(y, c)` over the tile row. A row that is
/// not whole tiles is the same write over its last, partial tile.
///
/// ★ THE TABLE IS `State`, read and written in one section: the write is downstream of the read on every site, so
/// the other rows of the tile row carry the value they came in with and nothing races the rewrite.
pub(super) fn dense_set_row(
    spec: &SetRowSpec,
    names: &StageNames,
    float: bool,
    held: Option<Precision>,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    fitting_block(spec.cols.div_ceil(target.tile.cols), &|blk| dense_set_row_in(spec, names, float, held, blk, target), target)
}

/// [`dense_set_row`] in column blocks of `blk` tiles.
pub(super) fn dense_set_row_in(
    spec: &SetRowSpec,
    names: &StageNames,
    float: bool,
    held: Option<Precision>,
    blk: usize,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols) = (spec.rows, spec.cols);
    let row_t = cols.div_ceil(target.tile.cols);
    let n = rows * cols;
    let (t_name, t_fill) = names.operand(0);
    let (s_name, s_fill) = names.operand(1);
    let dt = if float {
        row_float(held, target)?
    } else {
        index_type(
            target,
            fill_abs_bound(t_fill)
                .unwrap_or(0)
                .max(fill_abs_bound(s_fill).unwrap_or(0)),
        )?
    };
    let mut t = Template::new(target)?;
    let src = t.tensor(
        s_name,
        TensorRole::Input,
        1,
        cols,
        dt,
        crate::spatial::passes::from_tensor_op::init_of(s_fill, cols),
    );
    let table = t.tensor(
        t_name,
        TensorRole::State,
        rows,
        cols,
        dt,
        crate::spatial::passes::from_tensor_op::init_of(t_fill, n),
    );
    let old = t.buffer("in_tile_row", blk, dt);
    let sin = t.buffer(&format!("in_{s_name}"), blk, dt);
    let out = t.buffer("tile_row", blk, dt);
    let steps = vec![(LV_ITEM_X, blk)];
    let pattern = |origin: TileAddr| AccessPattern {
        origin,
        block_h: 1,
        block_w: blk,
        stride_w: 1,
        stride_h: row_t,
    };
    let index_operand = if matches!(spec.index, RowIndex::Named(_)) {
        names.operand(2)
    } else {
        (s_name, s_fill)
    };
    let at = dense_row(
        &mut t,
        spec.index,
        index_operand,
        t_name,
        rows,
        row_t,
        &steps,
        2,
        target,
    )?;
    let (load_addr, load_feed) = at.tile_rows[0].clone();
    let (store_addr, store_feed) = at.tile_rows[1].clone();
    // The in-tile row the select replaces: `[lo, lo + 1)`.
    let w1 = wide(1, target);
    let in_tile = match at.in_tile {
        InTileRow::Fixed(y) => RowSelect::Fixed(y),
        InTileRow::Data {
            row: lo,
            stage: s_lo,
            ix,
        } => {
            let hi = t.scratch_deep("in_tile_row_end", 1, 1, ix);
            let s_hi = t.stage(
                "in_tile_row_end",
                StageKind::Compute(ComputeOp::Generic {
                    iters: flat_space(1),
                    ins: vec![fixed(0)],
                    out: ident_map(w1),
                    body: *add(op(0), konst(1.0)),
                }),
                vec![lo],
                vec![hi],
            );
            t.edge(s_lo, lo, s_hi);
            RowSelect::Data {
                lo,
                hi,
                stages: (s_lo, s_hi),
            }
        }
    };
    let l_old = t.stage(
        &format!("gather_{t_name}"),
        StageKind::Load {
            tensor: table,
            pattern: pattern(load_addr),
            num_blocks: 1,
            multicast: None,
        },
        load_feed.map(|(_, b)| vec![b]).unwrap_or_default(),
        vec![old],
    );
    if let Some((s, b)) = load_feed {
        t.edge(s, b, l_old);
    }
    let l_src = t.load(&format!("load_{s_name}"), src, steps.clone(), blk, sin);
    // Operand 0 the tile row, operand 1 the source row (its page's row 0, broadcast down the tile row).
    let w = wide(blk, target);
    let source_row = ElemAccess {
        base: 0,
        terms: vec![(LV_GEN_COL, 1)],
        indirect: None,
    };
    let (ins, maps, lo_rhs, hi_rhs) = match in_tile {
        RowSelect::Fixed(y) => (
            vec![old, sin],
            vec![ident_map(w), source_row],
            PredRhs::Const(y),
            PredRhs::Const(y + 1),
        ),
        RowSelect::Data { lo, hi, .. } => (
            vec![old, sin, lo, hi],
            vec![ident_map(w), source_row, fixed(0), fixed(0)],
            PredRhs::Operand(2),
            PredRhs::Operand(3),
        ),
    };
    let body = TensorBody::Select {
        cond: Pred::AxisGe(LV_GEN_ROW, lo_rhs),
        then: Box::new(TensorBody::Select {
            cond: Pred::AxisLt(LV_GEN_ROW, hi_rhs),
            then: op(1),
            otherwise: op(0),
        }),
        otherwise: op(0),
    };
    let s_rw = t.stage(
        "write_row",
        StageKind::Compute(ComputeOp::Generic {
            iters: crate::spatial::passes::from_tensor_op::space(target, w, None),
            ins: maps,
            out: ident_map(w),
            body,
        }),
        ins,
        vec![out],
    );
    t.edge(l_old, old, s_rw);
    t.edge(l_src, sin, s_rw);
    if let RowSelect::Data {
        hi,
        stages: (_, s_hi),
        ..
    } = in_tile
    {
        t.edge(s_hi, hi, s_rw);
    }
    let store_ins: Vec<BufferId> = std::iter::once(out)
        .chain(store_feed.map(|(_, b)| b))
        .collect();
    let st = t.stage(
        &format!("scatter_{t_name}"),
        StageKind::Store {
            tensor: table,
            pattern: pattern(store_addr),
            sub_blocks: (1, 1),
        },
        store_ins,
        vec![],
    );
    t.edge(s_rw, out, st);
    if let Some((s, b)) = store_feed {
        t.edge(s, b, st);
    }
    let claim = if float {
        approx("row write", dt)
    } else {
        NumericsClaim::Exact
    };
    let mut prog = t.finish(
        format!("set_row_{rows}x{cols}_dense"),
        row_items(row_t, blk),
        AccPolicy::F32,
        claim,
        target,
    )?;
    prog.work.plan = Some(tile_row_plan(row_t, blk, dt));
    Ok(prog)
}

/// `Set rows from P of T to X` for a DENSE table — the KV cache a prefill chunk appends its `count` keys or values to,
/// `X` a dense `count × cols` block. Rows `r … r + count − 1` (`r = P − 1`) are replaced; the block starts at in-tile
/// row `o = r mod tile` of tile row `K0 = ⌊r / tile⌋`, both DATA, and touches at most `m = min(⌈count/tile⌉ + 1, n_t)`
/// tile rows of the table's `n_t`.
///
/// ★ THE ITEMS' TILE ROWS ARE DISTINCT BY THEIR COORDINATE, NOT BY THE DATA. Item `(i, b)` rewrites column block `b` of
/// tile row `K0' + i`, `K0' = min(K0, n_t − m)`: affine in `i` over a data start, so every item's tile is its own for
/// every `P` — which the certificate's dry run, taking each index at its low bound, sees — and `[K0', K0' + m)` always
/// lies in the table and covers the block's tile rows. With `c = r − tile·K0'` (data), row `y` of item `i` is the
/// block's row `s = y + i·tile − c`: membership is `c − i·tile ≤ y < count + c − i·tile`, and `s` lies in source tile
/// rows `i − δ − 1` and `i − δ` (`δ = K0 − K0'`), loaded as pages `A` and `B`, at in-tile rows `y + tile − o` (for
/// `y < o`) and `y − o`. A clamped source tile row only ever holds non-members, and a tile row the block misses is
/// written back unchanged.
///
/// Every number an address needs is a compile-time table read through the data: `K0` and `tile·K0` at tile `⌊r/tile⌋`;
/// `K0'`, `tile·K0'` and `m − 1 − δ` at tile `K0`; `i·tile`; the clamped source tile rows at tile `i + m − 1 − δ`. The
/// scalars are computed once into scratch and copied into one FIFO per movement that waits on them.
pub(super) fn dense_set_rows(
    spec: &SetRowSpec,
    names: &StageNames,
    float: bool,
    held: Option<Precision>,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    if spec.cols % target.tile.cols != 0 {
        return Err(SpatialReject::NotLowered(
            "a block of rows into a dense table whose rows are not whole tiles",
        ));
    }
    fitting_block(spec.cols / target.tile.cols, &|blk| dense_set_rows_in(spec, names, float, held, blk, target), target)
}

/// [`dense_set_rows`] in column blocks of `blk` tiles.
pub(super) fn dense_set_rows_in(
    spec: &SetRowSpec,
    names: &StageNames,
    float: bool,
    held: Option<Precision>,
    blk: usize,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols, count) = (spec.rows, spec.cols, spec.count);
    let (tr, tw) = (target.tile.rows, target.tile.cols);
    let row_t = cols / tw;
    let w = wide(blk, target);
    let n_t = rows.div_ceil(tr);
    let src_t = count.div_ceil(tr);
    let m = (src_t + 1).min(n_t);
    let (t_name, t_fill) = names.operand(0);
    let (s_name, s_fill) = names.operand(1);
    let dt = if float {
        row_float(held, target)?
    } else {
        index_type(
            target,
            fill_abs_bound(t_fill)
                .unwrap_or(0)
                .max(fill_abs_bound(s_fill).unwrap_or(0)),
        )?
    };
    let ix = index_type(target, (rows + tr * m) as u64)?;
    let mut t = Template::new(target)?;
    let table = t.tensor(
        t_name,
        TensorRole::State,
        rows,
        cols,
        dt,
        crate::spatial::passes::from_tensor_op::init_of(t_fill, rows * cols),
    );
    let src = t.tensor(
        s_name,
        TensorRole::Input,
        count,
        cols,
        dt,
        crate::spatial::passes::from_tensor_op::init_of(s_fill, count * cols),
    );
    // `P`: the operand, or the literal as a one-element table — either way checked below.
    let first = match spec.index {
        RowIndex::Named(_) => {
            let (p_name, p_fill) = names.operand(2);
            t.tensor(
                p_name,
                TensorRole::Input,
                1,
                1,
                ix,
                crate::spatial::passes::from_tensor_op::init_of(p_fill, 1),
            )
        }
        RowIndex::Literal(i) => {
            if i < 1 || i as usize + count - 1 > rows {
                return Err(SpatialReject::NotLowered("a literal block of rows outside its table — the program fails at run time on every tier, so there is no section to lower"));
            }
            t.tensor(
                &format!("{t_name}__block_first"),
                TensorRole::Param,
                1,
                1,
                ix,
                TensorInit::Values(vec![i]),
            )
        }
    };
    let param = |t: &mut Template, what: &str, values: Vec<i64>| {
        let n = values.len();
        let tiles = values
            .into_iter()
            .flat_map(|v| std::iter::once(v).chain(std::iter::repeat_n(0, tw - 1)))
            .collect();
        t.tensor(
            &format!("{t_name}__block_{what}"),
            TensorRole::Param,
            1,
            n * tw,
            ix,
            TensorInit::Values(tiles),
        )
    };
    let start = |q: usize| q.min(n_t - m);
    let tile_rows = param(&mut t, "tile_row", (0..n_t as i64).collect());
    let tile_bases = param(
        &mut t,
        "tile_base",
        (0..n_t).map(|q| (q * tr) as i64).collect(),
    );
    let starts = param(&mut t, "start", (0..n_t).map(|q| start(q) as i64).collect());
    let start_bases = param(
        &mut t,
        "start_base",
        (0..n_t).map(|q| (start(q) * tr) as i64).collect(),
    );
    let source_shift = param(
        &mut t,
        "source_shift",
        (0..n_t).map(|q| (m - 1 - (q - start(q))) as i64).collect(),
    );
    let item_rows = param(
        &mut t,
        "item_row",
        (0..m).map(|i| (i * tr) as i64).collect(),
    );
    let clamp = |k: i64| k.clamp(0, src_t as i64 - 1);
    let sources_a = param(
        &mut t,
        "source_a",
        (0..2 * m as i64 - 1).map(|k| clamp(k - m as i64)).collect(),
    );
    let sources_b = param(
        &mut t,
        "source_b",
        (0..2 * m as i64 - 1)
            .map(|k| clamp(k - m as i64 + 1))
            .collect(),
    );
    let w1 = wide(1, target);
    let scalar = |t: &mut Template,
                  name: &str,
                  ins: Vec<BufferId>,
                  maps: Vec<ElemAccess>,
                  body: TensorBody,
                  out: BufferId| {
        t.stage(
            name,
            StageKind::Compute(ComputeOp::Generic {
                iters: flat_space(1),
                ins: maps,
                out: ident_map(w1),
                body,
            }),
            ins,
            vec![out],
        )
    };
    // A scalar's FIFO copy for one movement to wait on.
    let fifo = |t: &mut Template, name: &str, from: (StageId, BufferId)| {
        let out = t.buffer(name, 1, ix);
        let s = scalar(
            t,
            name,
            vec![from.1],
            vec![fixed(0)],
            TensorBody::Operand(0),
            out,
        );
        t.edge(from.0, from.1, s);
        (s, out)
    };
    // One element of a table, at `steps` plus — when given — the tile a data index selects.
    let lookup = |t: &mut Template,
                  name: &str,
                  from: TensorId,
                  steps: Vec<(LoopVar, usize)>,
                  index: Option<((StageId, BufferId), usize, (usize, usize))>| {
        let out = t.buffer(name, 1, ix);
        let indirect = index.map(|((_, b), per, bound)| IndirectTile {
            index: b,
            tile_stride: 1,
            elems_per_tile: per,
            bound,
        });
        let load = t.stage(
            &format!("load_{name}"),
            StageKind::Load {
                tensor: from,
                pattern: AccessPattern {
                    origin: TileAddr {
                        base: 0,
                        steps,
                        indirect,
                    },
                    block_h: 1,
                    block_w: 1,
                    stride_w: 1,
                    stride_h: 1,
                },
                num_blocks: 1,
                multicast: None,
            },
            index.map(|((_, b), _, _)| vec![b]).unwrap_or_default(),
            vec![out],
        );
        if let Some(((s, b), _, _)) = index {
            t.edge(s, b, load);
        }
        (load, out)
    };
    // `r = P − 1`, checked so every row of the block is one of the table's: a guard of stride zero, as a row index is.
    let p_in = t.buffer("in_first", 1, ix);
    let l_p = t.load_block_at("load_first", first, 0, Vec::new(), 1, 1, 1, p_in);
    let biased = t.scratch_deep("first_biased", 1, 1, ix);
    let s_biased = scalar(
        &mut t,
        "first_biased",
        vec![p_in],
        vec![fixed(0)],
        *sub(op(0), konst(1.0)),
        biased,
    );
    t.edge(l_p, p_in, s_biased);
    let r = t.scratch_deep("first_row", 1, 1, ix);
    let guard = ElemAccess {
        base: 0,
        terms: Vec::new(),
        indirect: Some(IndirectTerm {
            operand: 0,
            stride: 0,
            modulus: rows - count + 1,
        }),
    };
    let s_r = scalar(
        &mut t,
        "first_row",
        vec![biased],
        vec![guard],
        TensorBody::Operand(0),
        r,
    );
    t.edge(s_biased, biased, s_r);
    let r_at = |t: &mut Template, what: &str| fifo(t, &format!("first_row_{what}"), (s_r, r));
    let span = (0, rows - count);
    // `K0`, into scratch for its own lookups.
    let r_k0 = r_at(&mut t, "tile_row");
    let (l_k0, k0_in) = lookup(
        &mut t,
        "tile_row",
        tile_rows,
        Vec::new(),
        Some((r_k0, tr, span)),
    );
    let k0 = t.scratch_deep("tile_row", 1, 1, ix);
    let s_k0 = scalar(
        &mut t,
        "tile_row",
        vec![k0_in],
        vec![fixed(0)],
        TensorBody::Operand(0),
        k0,
    );
    t.edge(l_k0, k0_in, s_k0);
    let by_k0 = |t: &mut Template,
                 what: &str,
                 from: TensorId,
                 steps: Vec<(LoopVar, usize)>,
                 bound: (usize, usize)| {
        let k = fifo(t, &format!("tile_row_{what}"), (s_k0, k0));
        lookup(t, what, from, steps, Some((k, 1, bound)))
    };
    // `o = r − tile·K0`.
    let r_base = r_at(&mut t, "tile_base");
    let (l_base, base) = lookup(
        &mut t,
        "tile_base",
        tile_bases,
        Vec::new(),
        Some((r_base, tr, span)),
    );
    let o = t.scratch_deep("block_offset", 1, 1, ix);
    let s_o = scalar(
        &mut t,
        "block_offset",
        vec![r, base],
        vec![fixed(0), fixed(0)],
        *sub(op(0), op(1)),
        o,
    );
    t.edge(l_base, base, s_o);
    // The membership thresholds: `lo = r − tile·K0' − i·tile`, `hi = lo + count`.
    let (l_sb, start_base) = by_k0(&mut t, "start_base", start_bases, Vec::new(), (0, n_t - 1));
    let (l_it, it) = lookup(&mut t, "item_row", item_rows, vec![(LV_ITEM_Y, 1)], None);
    let lo = t.scratch_deep("member_lo", 1, 1, ix);
    let s_lo = scalar(
        &mut t,
        "member_lo",
        vec![r, start_base, it],
        vec![fixed(0), fixed(0), fixed(0)],
        *sub(sub(op(0), op(1)), op(2)),
        lo,
    );
    t.edge(l_sb, start_base, s_lo);
    t.edge(l_it, it, s_lo);
    let hi = t.scratch_deep("member_hi", 1, 1, ix);
    let s_hi = scalar(
        &mut t,
        "member_hi",
        vec![lo],
        vec![fixed(0)],
        *add(op(0), konst(count as f64)),
        hi,
    );
    t.edge(s_lo, lo, s_hi);
    // The two source tile rows the item's rows come from: table tile `i + m − 1 − δ`.
    let source_page = |t: &mut Template, what: &str, rows_of: TensorId| {
        let shift = by_k0(
            t,
            &format!("{what}_shift"),
            source_shift,
            Vec::new(),
            (0, n_t - 1),
        );
        let (l_at, at) = lookup(
            t,
            &format!("{what}_row"),
            rows_of,
            vec![(LV_ITEM_Y, 1)],
            Some((shift, 1, (0, m - 1))),
        );
        let page = t.buffer(&format!("in_{what}"), blk, dt);
        let origin = TileAddr {
            base: 0,
            steps: vec![(LV_ITEM_X, blk)],
            indirect: Some(IndirectTile {
                index: at,
                tile_stride: row_t,
                elems_per_tile: 1,
                bound: (0, src_t - 1),
            }),
        };
        let load = t.stage(
            &format!("load_{what}"),
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
            vec![at],
            vec![page],
        );
        t.edge(l_at, at, load);
        (load, page)
    };
    let (l_a, page_a) = source_page(&mut t, "source_a", sources_a);
    let (l_b, page_b) = source_page(&mut t, "source_b", sources_b);
    // The item's tile row `K0' + i`, once per movement that addresses it.
    let tile_row_at = |t: &mut Template, what: &str| {
        let (l_s, s) = by_k0(
            t,
            &format!("start_{what}"),
            starts,
            Vec::new(),
            (0, n_t - 1),
        );
        let addr = TileAddr {
            base: 0,
            steps: vec![(LV_ITEM_Y, row_t), (LV_ITEM_X, blk)],
            indirect: Some(IndirectTile {
                index: s,
                tile_stride: row_t,
                elems_per_tile: 1,
                bound: (0, n_t - m),
            }),
        };
        (addr, (l_s, s))
    };
    let (old_at, (l_s_old, s_old)) = tile_row_at(&mut t, "gather");
    let (new_at, (l_s_new, s_new)) = tile_row_at(&mut t, "scatter");
    let old = t.buffer("in_tile_row", blk, dt);
    let l_old = t.stage(
        &format!("gather_{t_name}"),
        StageKind::Load {
            tensor: table,
            pattern: AccessPattern {
                origin: old_at,
                block_h: 1,
                block_w: blk,
                stride_w: 1,
                stride_h: row_t,
            },
            num_blocks: 1,
            multicast: None,
        },
        vec![s_old],
        vec![old],
    );
    t.edge(l_s_old, s_old, l_old);
    // Operands: 0 the tile row, 1 and 2 the source halves, 3 `o`, 4 `lo`, 5 `hi`.
    let shifted = |base: i64| ElemAccess {
        base,
        terms: vec![(LV_GEN_ROW, w as i64), (LV_GEN_COL, 1)],
        indirect: Some(IndirectTerm {
            operand: 3,
            stride: -(w as i64),
            modulus: tr,
        }),
    };
    let from_block = TensorBody::Select {
        cond: Pred::AxisLt(LV_GEN_ROW, PredRhs::Operand(3)),
        then: op(1),
        otherwise: op(2),
    };
    let body = TensorBody::Select {
        cond: Pred::AxisGe(LV_GEN_ROW, PredRhs::Operand(4)),
        then: Box::new(TensorBody::Select {
            cond: Pred::AxisLt(LV_GEN_ROW, PredRhs::Operand(5)),
            then: Box::new(from_block),
            otherwise: op(0),
        }),
        otherwise: op(0),
    };
    let out = t.buffer("tile_row", blk, dt);
    let s_rw = t.stage(
        "write_rows",
        StageKind::Compute(ComputeOp::Generic {
            iters: crate::spatial::passes::from_tensor_op::space(target, w, None),
            ins: vec![
                ident_map(w),
                shifted((tr * w) as i64),
                shifted(0),
                fixed(0),
                fixed(0),
                fixed(0),
            ],
            out: ident_map(w),
            body,
        }),
        vec![old, page_a, page_b, o, lo, hi],
        vec![out],
    );
    for (from, buf) in [(l_old, old), (l_a, page_a), (l_b, page_b)] {
        t.edge(from, buf, s_rw);
    }
    t.edge(s_o, o, s_rw);
    t.edge(s_hi, hi, s_rw);
    let st = t.stage(
        &format!("scatter_{t_name}"),
        StageKind::Store {
            tensor: table,
            pattern: AccessPattern {
                origin: new_at,
                block_h: 1,
                block_w: blk,
                stride_w: 1,
                stride_h: row_t,
            },
            sub_blocks: (1, 1),
        },
        vec![out, s_new],
        vec![],
    );
    t.edge(s_rw, out, st);
    t.edge(l_s_new, s_new, st);
    let items: Vec<WorkItem> = (0..m)
        .flat_map(|i| (0..row_t / blk).map(move |b| (i, b)))
        .enumerate()
        .map(|(id, (i, b))| WorkItem {
            id: WorkItemId::new(id),
            coord: (i, b),
            out: TileRect {
                row0_t: i,
                col0_t: b * blk,
                h_t: 1,
                w_t: blk,
            },
        })
        .collect();
    let claim = if float {
        approx("row block write", dt)
    } else {
        NumericsClaim::Exact
    };
    let mut prog = t.finish(
        set_row_name(spec, "_dense"),
        items,
        AccPolicy::F32,
        claim,
        target,
    )?;
    // An in-place update of carried state: its grid is its items' blocks, one per item.
    prog.work.plan = Some(BlockPlan {
        grid: (1, m * (row_t / blk)),
        ..tile_row_plan(row_t, blk, dt)
    });
    Ok(prog)
}

/// `x = row i of T` for a DENSE table — a stacked batch a matmul wrote, `rows × cols` with a tile row holding
/// `tile` rows. Each work item Loads its `blk` tiles of tile row `⌊r / tile⌋` ([`dense_row`]) and lifts in-tile row
/// `r mod tile` into row 0 of the result's page: `x(c) = T(r mod tile, c)`, the in-tile row a constant offset for
/// a literal row and an [`IndirectTerm`] one page row wide for a named one.
pub(super) fn dense_row_gather(
    spec: &RowGatherSpec,
    names: &StageNames,
    float: bool,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols) = (spec.rows, spec.cols);
    let tr = target.tile.rows;
    let row_t = cols.div_ceil(target.tile.cols);
    let blk = row_block(row_t, target);
    let n = rows * cols;
    let (t_name, t_fill) = names.operand(0);
    let dt = if float {
        elem_precision(target)?
    } else {
        index_type(target, fill_abs_bound(t_fill).unwrap_or(0))?
    };
    let mut t = Template::new(target)?;
    let table = t.tensor(
        t_name,
        TensorRole::Input,
        rows,
        cols,
        dt,
        crate::spatial::passes::from_tensor_op::init_of(t_fill, n),
    );
    let steps = vec![(LV_ITEM_X, blk)];
    let index_operand = if matches!(spec.index, RowIndex::Named(_)) {
        names.operand(1)
    } else {
        (t_name, t_fill)
    };
    let at = dense_row(
        &mut t,
        spec.index,
        index_operand,
        t_name,
        rows,
        row_t,
        &steps,
        1,
        target,
    )?;
    let (addr, feed) = at.tile_rows[0].clone();
    let x = t.tensor(
        &names.out,
        TensorRole::Output,
        1,
        cols,
        dt,
        TensorInit::Zero,
    );
    let tile_row = t.buffer("in_tile_row", blk, dt);
    let load = t.stage(
        "gather_tile_row",
        StageKind::Load {
            tensor: table,
            pattern: AccessPattern {
                origin: addr,
                block_h: 1,
                block_w: blk,
                stride_w: 1,
                stride_h: row_t,
            },
            num_blocks: 1,
            multicast: None,
        },
        feed.map(|(_, b)| vec![b]).unwrap_or_default(),
        vec![tile_row],
    );
    if let Some((s, b)) = feed {
        t.edge(s, b, load);
    }
    let w = wide(blk, target);
    let (ins, maps) = match at.in_tile {
        InTileRow::Fixed(y) => (vec![tile_row], vec![at_row(y, w)]),
        InTileRow::Data { row, .. } => (
            vec![tile_row, row],
            vec![
                ElemAccess {
                    base: 0,
                    terms: vec![(LV_GEN_COL, 1)],
                    indirect: Some(IndirectTerm {
                        operand: 1,
                        stride: w as i64,
                        modulus: tr * w,
                    }),
                },
                fixed(0),
            ],
        ),
    };
    let out = t.buffer("row", blk, dt);
    let s_row = t.stage(
        "lift_row",
        StageKind::Compute(ComputeOp::Generic {
            iters: flat_space(w),
            ins: maps,
            out: ident_map(w),
            body: TensorBody::Operand(0),
        }),
        ins,
        vec![out],
    );
    t.edge(load, tile_row, s_row);
    if let InTileRow::Data { row, stage, .. } = at.in_tile {
        t.edge(stage, row, s_row);
    }
    let st = t.store(&format!("store_{}", names.out), x, steps, blk, out);
    t.edge(s_row, out, st);
    let claim = if float {
        approx("row gather", dt)
    } else {
        NumericsClaim::Exact
    };
    let mut prog = t.finish(
        format!("row_gather_{rows}x{cols}_dense"),
        row_items(row_t, blk),
        AccPolicy::F32,
        claim,
        target,
    )?;
    prog.work.plan = Some(tile_row_plan(row_t, blk, dt));
    Ok(prog)
}

/// Row `y` of a page `w` elements wide, walked along its columns.
pub(super) fn at_row(y: usize, w: usize) -> ElemAccess {
    at((y * w) as i64)
}

/// Which in-tile row a dense row write replaces: known at compile time, or computed into `[lo, hi)`.
#[derive(Clone, Copy)]
pub(super) enum RowSelect {
    Fixed(usize),
    Data {
        lo: BufferId,
        hi: BufferId,
        stages: (StageId, StageId),
    },
}

/// `t = argmax(X) + 1` — an `ArgMax` fold (first maximum, ties to the lowest index: the VM's `>` scan) onto
/// the integer datapath, then LOGOS's 1-based count.
pub fn from_argmax(
    spec: &ArgMaxSpec,
    names: &StageNames,
    float: bool,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let n = spec.n;
    if n == 0 {
        return Err(SpatialReject::NotLowered("an argmax of an empty tensor"));
    }
    if tiles_of(n, target) > row_block_tiles(target) {
        return blocked_argmax(spec, names, float, target);
    }
    let whole = whole_argmax(spec, names, float, target)?;
    if fits_a_site(&whole, target) {
        Ok(vec![whole])
    } else {
        blocked_argmax(spec, names, float, target)
    }
}

/// [`from_argmax`]'s whole-row form: the row resident, one `ArgMax` fold over it.
pub(super) fn whole_argmax(
    spec: &ArgMaxSpec,
    names: &StageNames,
    float: bool,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let n = spec.n;
    let mut b = FlatBuild::on(Datapath::of(float), target)?;
    let (x_name, x_fill) = names.operand(0);
    let dt = table_precision(&b, float, x_fill, target)?;
    let x = b.input_as(x_name, n, x_fill, dt, target);
    let one_w = wide(1, target);
    // The result is a 1-based position in `[1, n]`.
    let ix = index_type(target, n as u64)?;
    let at0 = b.t.scratch("argmax", 1, ix);
    let s1 = b.compute(
        "argmax",
        head_space(1, n, Some(ReducePool::ArgMax)),
        vec![Slot {
            buf: x,
            map: map2(0, 0, 1),
        }],
        per_row_map(one_w),
        TensorBody::Operand(0),
        at0,
    );
    let out_buf = b.t.buffer("index", 1, ix);
    let s2 = b.compute(
        "one_based",
        flat_space(1),
        vec![Slot {
            buf: at0,
            map: fixed(0),
        }],
        ident_map(one_w),
        *add(op(0), konst(1.0)),
        out_buf,
    );
    b.finish_as(
        &names.out,
        1,
        vec![s1, s2],
        out_buf,
        format!("argmax_{n}"),
        NumericsClaim::Exact,
        ix,
        target,
    )
}

/// `t = argmax(X) + 1` over a row wider than a site holds — the decode step's `argmax of logits` over a real
/// vocabulary. Sections: the BLOCKS, then a merge tree.
///
/// ★ EXACTLY THE VM'S SCAN, NaN INCLUDED. The VM seeds its best with item 1 and replaces it only on a strictly
/// greater item, and the [`ReducePool::ArgMax`] fold is that scan. Cut into blocks, the row's scan is: block 0's
/// scan (seeded by item 1 — a NaN there is never replaced, which makes it the answer), then every later block's
/// scan seeded by `−∞` (a NaN at a later block's start must not capture the block, since the VM never lets a NaN
/// replace anything), then a scan over the blocks' results in position order. So every block folds `blk` tiles
/// behind ONE seed element — item 1 for block 0 (a duplicate the scan cannot be moved by: nothing is `>` itself),
/// `−∞` for the rest — and reports the VALUE it holds (read back at its index, so block 0's NaN travels) and that
/// value's position in the row. Each merge level scans `fan` results the same way; only the first result of the
/// first group can be a NaN, and it is exactly the one the VM keeps.
///
/// The row's last block starts `blk` tiles before its end, overlapping the one before it — harmless, the overlap
/// reports a position twice and the earlier report is kept. Its padding past `n` is masked, and a pad block of the
/// tree (past the row) is masked whole. ★ A MASKED POSITION FOLDS AS THE SCAN'S IDENTITY — `−∞` for a float row,
/// the type's minimum for an integer one — never a large finite negative: a row of logits below `−1e30` is a row
/// like any other. A masked position never replaces the seed, and a live value equal to the mask is kept because
/// it comes first.
///
/// A float row's partials are fp32: its values are the datapath's (exact in fp32) and a position below `2^24` is
/// exact too. An integer row's partials are its own integer type.
pub(super) fn blocked_argmax(
    spec: &ArgMaxSpec,
    names: &StageNames,
    float: bool,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    // ★ A BLOCK IS A PAGE OF THE ROW WHERE A SITE HOLDS ONE, AND HALF AS MUCH UNTIL ONE DOES: a part of small tiles runs
    // out of a site's memory before its page (the RTX 3060's 64-tile page of 16 × 16 tiles does not fit a block). Two
    // tiles is the floor: the merge tree's fan is the block's width, and a fan of one never narrows the blocks.
    let mut blk = row_block_tiles(target).min(tiles_of(spec.n, target));
    loop {
        let sections = blocked_argmax_in(spec, names, float, blk, target)?;
        if blk <= 2 || sections.iter().all(|p| fits_a_site(p, target)) {
            return Ok(sections);
        }
        blk = (blk / 2).max(2);
    }
}

/// [`blocked_argmax`] in blocks of `blk` tiles.
pub(super) fn blocked_argmax_in(
    spec: &ArgMaxSpec,
    names: &StageNames,
    float: bool,
    blk: usize,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let n = spec.n;
    let tw = target.tile.cols;
    let block = blk * tw;
    let x_tiles = tiles_of(n, target);
    let nb = x_tiles.div_ceil(blk);
    let (x_name, x_fill) = names.operand(0);
    let dt = if float {
        elem_precision(target)?
    } else {
        index_type(target, fill_abs_bound(x_fill).unwrap_or(0))?
    };
    let ix = index_type(target, (n + block + 1) as u64)?;
    let (pd, mask) = if float {
        if n >= 1 << 24 {
            return Err(SpatialReject::MissingPrecision {
                needed: Precision::Int64,
                target: target.name,
            });
        }
        (AccPolicy::F32.interm_precision(), f64::NEG_INFINITY)
    } else {
        let wide = index_type(
            target,
            (fill_abs_bound(x_fill).unwrap_or(0) as u64).max((n + block + 1) as u64),
        )?;
        (
            wide,
            -(crate::spatial::target::int_precision_max(wide).expect("an integer type") as f64)
                - 1.0,
        )
    };
    let fans = merge_fans(nb, blk);
    let slots: usize = fans.iter().product::<usize>().max(1);
    let last_start = (x_tiles - blk) * tw;
    // Per block, four index tiles: the start twice (the gather's index, and the position base), the live bound
    // `limit + 1` over the scan's `1 + block` elements (element 0 is the seed), and whether the block is the
    // row's first (its seed is item 1; every other block's is the mask).
    const FIELDS: usize = 4;
    let mut table = vec![0i64; FIELDS * slots * tw];
    for b in 0..slots {
        let (start, live, first) = if b < nb {
            let start = (b * block).min(last_start);
            (start, (n - start).min(block) + 1, usize::from(b == 0))
        } else {
            (0, 1, 0)
        };
        for (field, v) in [start, start, live, first].into_iter().enumerate() {
            table[(FIELDS * b + field) * tw] = v as i64;
        }
    }
    let one = wide(1, target);
    let partial_name = |level: usize| format!("{}__argmax_part{level}", names.out);
    // Pack `(value, position)` into one partial tile.
    let pack = |t: &mut Template, m: BufferId, at: BufferId, into: BufferId| {
        let body = TensorBody::Select {
            cond: Pred::AxisLt(LV_GEN_COL, PredRhs::Const(1)),
            then: op(0),
            otherwise: op(1),
        };
        t.stage(
            "pack_partial",
            StageKind::Compute(ComputeOp::Generic {
                iters: flat_space(2),
                ins: vec![fixed(0), fixed(0)],
                out: ident_map(one),
                body,
            }),
            vec![m, at],
            vec![into],
        )
    };
    // `0 < operand` at the single point of a one-element stage: whether a scan moved off its seed.
    let past_seed = |operand: usize| Pred::AxisLt(LV_GEN_COL, PredRhs::Operand(operand));

    let blocks = {
        let mut t = Template::new(target)?;
        let x = t.tensor(
            x_name,
            TensorRole::Input,
            1,
            n,
            dt,
            crate::spatial::passes::from_tensor_op::init_of(x_fill, n),
        );
        let tab = t.tensor(
            &format!("{}__argmax_blocks", names.out),
            TensorRole::Param,
            1,
            FIELDS * slots * tw,
            ix,
            TensorInit::Values(table),
        );
        let part = t.tensor(
            &partial_name(0),
            TensorRole::Output,
            1,
            slots * tw,
            pd,
            TensorInit::Zero,
        );
        let fields: Vec<BufferId> = ["in_start", "in_base", "in_live", "in_first"]
            .iter()
            .map(|name| t.buffer(name, 1, ix))
            .collect();
        let loads: Vec<StageId> = ["load_start", "load_base", "load_live", "load_first"]
            .iter()
            .enumerate()
            .map(|(field, name)| {
                t.load_block_at(
                    name,
                    tab,
                    field,
                    vec![(LV_ITEM_X, FIELDS)],
                    1,
                    1,
                    1,
                    fields[field],
                )
            })
            .collect();
        let (gather_in, base_in, live_in, first_in) = (fields[0], fields[1], fields[2], fields[3]);
        let x_in = t.buffer("in_x", blk, dt);
        let origin = TileAddr {
            base: 0,
            steps: Vec::new(),
            indirect: Some(IndirectTile {
                index: gather_in,
                tile_stride: 1,
                elems_per_tile: tw,
                bound: (0, last_start),
            }),
        };
        let l_x = t.stage(
            "load_x",
            StageKind::Load {
                tensor: x,
                pattern: AccessPattern {
                    origin,
                    block_h: 1,
                    block_w: blk,
                    stride_w: 1,
                    stride_h: x_tiles,
                },
                num_blocks: 1,
                multicast: None,
            },
            vec![gather_in],
            vec![x_in],
        );
        t.edge(loads[0], gather_in, l_x);
        // The seed: item 1 for the row's first block, the mask for every other.
        let seed = t.scratch_deep("block_seed", 1, 1, pd);
        let s_seed = t.stage(
            "block_seed",
            StageKind::Compute(ComputeOp::Generic {
                iters: flat_space(1),
                ins: vec![fixed(0), fixed(0)],
                out: ident_map(one),
                body: TensorBody::Select {
                    cond: past_seed(1),
                    then: op(0),
                    otherwise: konst(mask),
                },
            }),
            vec![x_in, first_in],
            vec![seed],
        );
        // The scan over `[seed, x_0, …, x_{block−1}]`: element `c ≥ 1` is position `c − 1`, live below the bound.
        let scanned = TensorBody::Select {
            cond: Pred::AxisLt(LV_GEN_COL, PredRhs::Const(1)),
            then: op(2),
            otherwise: Box::new(TensorBody::Select {
                cond: Pred::AxisLt(LV_GEN_COL, PredRhs::Operand(1)),
                then: op(0),
                otherwise: konst(mask),
            }),
        };
        let at = t.scratch_deep("block_argmax", 1, 1, ix);
        let s_at = t.stage(
            "block_argmax",
            StageKind::Compute(ComputeOp::Generic {
                iters: head_space(1, block + 1, Some(ReducePool::ArgMax)),
                ins: vec![map2(-1, 0, 1), fixed(0), fixed(0)],
                out: per_row_map(one),
                body: scanned,
            }),
            vec![x_in, live_in, seed],
            vec![at],
        );
        // The value the scan holds: element `at − 1` of the block, or the seed.
        let held = ElemAccess {
            base: -1,
            terms: Vec::new(),
            indirect: Some(IndirectTerm {
                operand: 1,
                stride: 1,
                modulus: block + 1,
            }),
        };
        let v = t.scratch_deep("block_value", 1, 1, pd);
        let s_v = t.stage(
            "block_value",
            StageKind::Compute(ComputeOp::Generic {
                iters: flat_space(1),
                ins: vec![held, fixed(0), fixed(0)],
                out: ident_map(one),
                body: TensorBody::Select {
                    cond: past_seed(1),
                    then: op(0),
                    otherwise: op(2),
                },
            }),
            vec![x_in, at, seed],
            vec![v],
        );
        // Its position in the row: `base + at − 1`, or `base` for the seed (item 1 itself in the first block).
        let pos = t.scratch_deep("block_position", 1, 1, ix);
        let s_pos = t.stage(
            "block_position",
            StageKind::Compute(ComputeOp::Generic {
                iters: flat_space(1),
                ins: vec![fixed(0), fixed(0)],
                out: ident_map(one),
                body: TensorBody::Select {
                    cond: past_seed(0),
                    then: add(sub(op(0), konst(1.0)), op(1)),
                    otherwise: op(1),
                },
            }),
            vec![at, base_in],
            vec![pos],
        );
        let out = t.buffer("out", 1, pd);
        let s_pack = pack(&mut t, v, pos, out);
        // Each hand-off's edge goes to its FIRST reader; a later stage naming the same page reads it before the
        // last reader pops it.
        for (from, buf, to) in [
            (l_x, x_in, s_seed),
            (loads[3], first_in, s_seed),
            (s_seed, seed, s_at),
            (loads[2], live_in, s_at),
            (s_at, at, s_v),
            (loads[1], base_in, s_pos),
            (s_v, v, s_pack),
            (s_pos, pos, s_pack),
        ] {
            t.edge(from, buf, to);
        }
        let st = t.store("store_partial", part, vec![(LV_ITEM_X, 1)], 1, out);
        t.edge(s_pack, out, st);
        t.finish(
            format!("argmax_blocks_{n}_b{blk}"),
            crate::spatial::passes::from_tensor_op::items_per_tile(slots),
            AccPolicy::F32,
            NumericsClaim::Exact,
            target,
        )?
    };
    let mut out = vec![blocks];

    let mut n_in = slots;
    for (level, fan) in fans.iter().enumerate() {
        let root = level + 1 == fans.len();
        let n_out = n_in / fan;
        let mut t = Template::new(target)?;
        let part_in = t.tensor(
            &partial_name(level),
            TensorRole::Input,
            1,
            n_in * tw,
            pd,
            TensorInit::External,
        );
        let p_in = t.buffer("in_parts", *fan, pd);
        let l_p = t.load("load_parts", part_in, vec![(LV_ITEM_X, *fan)], *fan, p_in);
        // Partial `f` sits at column `f·tile` of the page: its value at `+0`, its position at `+1`.
        let values = ElemAccess {
            base: 0,
            terms: vec![(LV_GEN_COL, tw as i64)],
            indirect: None,
        };
        let f_at = t.scratch_deep("merge_argmax", 1, 1, ix);
        let s_f = t.stage(
            "merge_argmax",
            StageKind::Compute(ComputeOp::Generic {
                iters: head_space(1, *fan, Some(ReducePool::ArgMax)),
                ins: vec![values],
                out: per_row_map(one),
                body: TensorBody::Operand(0),
            }),
            vec![p_in],
            vec![f_at],
        );
        let field = |base: i64| ElemAccess {
            base,
            terms: Vec::new(),
            indirect: Some(IndirectTerm {
                operand: 1,
                stride: tw as i64,
                modulus: *fan,
            }),
        };
        let pos = t.scratch_deep("merge_position", 1, 1, pd);
        let s_pos = t.stage(
            "merge_position",
            StageKind::Compute(ComputeOp::Generic {
                iters: flat_space(1),
                ins: vec![field(1), fixed(0)],
                out: ident_map(one),
                body: TensorBody::Operand(0),
            }),
            vec![p_in, f_at],
            vec![pos],
        );
        let program = if root {
            t.edge(l_p, p_in, s_f);
            t.edge(s_f, f_at, s_pos);
            // The row's first maximum, 1-based.
            let res = t.tensor(&names.out, TensorRole::Output, 1, 1, ix, TensorInit::Zero);
            let o = t.buffer("index", 1, ix);
            let s_o = t.stage(
                "one_based",
                StageKind::Compute(ComputeOp::Generic {
                    iters: flat_space(1),
                    ins: vec![fixed(0)],
                    out: ident_map(one),
                    body: *add(op(0), konst(1.0)),
                }),
                vec![pos],
                vec![o],
            );
            t.edge(s_pos, pos, s_o);
            let st = t.store(&format!("store_{}", names.out), res, vec![], 1, o);
            t.edge(s_o, o, st);
            t.finish(
                format!("argmax_merge{}_f{fan}", level + 1),
                crate::spatial::passes::from_tensor_op::items_per_tile(1),
                AccPolicy::for_result(ix),
                NumericsClaim::Exact,
                target,
            )?
        } else {
            let part_out = t.tensor(
                &partial_name(level + 1),
                TensorRole::Output,
                1,
                n_out * tw,
                pd,
                TensorInit::Zero,
            );
            // The value the scan holds, read back at its index — a NaN the first partial carries travels on.
            let v = t.scratch_deep("merge_value", 1, 1, pd);
            let s_v = t.stage(
                "merge_value",
                StageKind::Compute(ComputeOp::Generic {
                    iters: flat_space(1),
                    ins: vec![field(0), fixed(0)],
                    out: ident_map(one),
                    body: TensorBody::Operand(0),
                }),
                vec![p_in, f_at],
                vec![v],
            );
            t.edge(l_p, p_in, s_f);
            t.edge(s_f, f_at, s_pos);
            let o = t.buffer("out", 1, pd);
            let s_pack = pack(&mut t, v, pos, o);
            t.edge(s_v, v, s_pack);
            t.edge(s_pos, pos, s_pack);
            let st = t.store("store_partial", part_out, vec![(LV_ITEM_X, 1)], 1, o);
            t.edge(s_pack, o, st);
            t.finish(
                format!("argmax_merge{}_f{fan}", level + 1),
                crate::spatial::passes::from_tensor_op::items_per_tile(n_out),
                AccPolicy::F32,
                NumericsClaim::Exact,
                target,
            )?
        };
        out.push(program);
        n_in = n_out;
    }
    Ok(out)
}

/// The float a row write moves: the width the program holds its table at — a key or value cache's, the products' float
/// ([`cache_precision`]) — else the elementwise datapath's. A row write moves values and computes none, so a section at
/// the table's width rounds the source once, on its way in.
fn row_float(held: Option<Precision>, target: &SpatialTarget) -> Result<Precision, SpatialReject> {
    held.map_or_else(|| elem_precision(target), Ok)
}

/// `Set row index of T to S` — the KV-cache append, as ONE rewrite of the carried table:
/// `T'[c] = (lo ≤ c < lo + cols) ? S[c − lo] : T[c]`.
///
/// ★ AN OUT-OF-RANGE ROW MUST STILL FAULT, AND HERE NOTHING WOULD. The source is read through an
/// [`IndirectTerm`] only where the row is selected; with `lo` out of range no position is, so the gather
/// never runs and the write silently vanishes — where every tier fails. So the offset is CHECKED first by a
/// gather of STRIDE ZERO (it reads `lo` itself, and its extent `rows·cols` is the range check): a bounds
/// check is a gather that moves nothing.
pub fn from_set_row(
    spec: &SetRowSpec,
    names: &StageNames,
    float: bool,
    dense: bool,
    held: Option<Precision>,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols, count) = (spec.rows, spec.cols, spec.count);
    if rows == 0 || cols == 0 || count == 0 || count > rows {
        return Err(SpatialReject::NotLowered(
            "a row write into an empty table, or a block of rows taller than its table",
        ));
    }
    if dense {
        return if count == 1 {
            dense_set_row(spec, names, float, held, target)
        } else {
            dense_set_rows(spec, names, float, held, target)
        };
    }
    if cols % target.tile.cols == 0 {
        return tile_set_row(spec, names, float, held, target);
    }
    // ★ A BLOCK OF ROWS IS ONE CONTIGUOUS RUN of `count·cols` elements from row `P − 1`: the one-row write, wider.
    let n = rows * cols;
    let span = count * cols;
    let mut b = FlatBuild::columns(n, target)?;
    if float {
        b.precision = row_float(held, target)?;
    }
    let (t_name, t_fill) = names.operand(0);
    let (s_name, s_fill) = names.operand(1);
    let dt = row_write_precision(&b, float, t_fill, s_fill, target)?;
    let table = b.input_as(t_name, n, t_fill, dt, target);
    let source = b.input_as(s_name, span, s_fill, dt, target);
    let out_buf = b.result_page("table", n, dt, target);
    let one_w = wide(1, target);
    // `(c ≥ lo) ? ((c < hi) ? S : T) : T` — operand 0 is the table, operand 1 the source.
    let in_row = |lo_rhs: PredRhs, hi_rhs: PredRhs| TensorBody::Select {
        cond: Pred::AxisGe(LV_GEN_COL, lo_rhs),
        then: Box::new(TensorBody::Select {
            cond: Pred::AxisLt(LV_GEN_COL, hi_rhs),
            then: op(1),
            otherwise: op(0),
        }),
        otherwise: op(0),
    };
    let chain = match spec.index {
        RowIndex::Literal(i) => {
            let lo = literal_row_offset(i, rows + 1 - count, cols)?;
            let body = in_row(
                PredRhs::Const(lo as usize),
                PredRhs::Const(lo as usize + span),
            );
            b.pointwise(
                "write_row",
                n,
                vec![
                    Slot {
                        buf: table,
                        map: at(0),
                    },
                    Slot {
                        buf: source,
                        map: at(-lo),
                    },
                ],
                body,
                out_buf,
                target,
            )?
        }
        RowIndex::Named(_) => {
            let (i_name, i_fill) = names.operand(2);
            // `row_end = lo + span` reaches one row past the table.
            let ix = index_type(target, index_space((n + cols) as u64, cols as u64, i_fill))?;
            let index = b.input_as(i_name, 1, i_fill, ix, target);
            let (mut chain, lo_raw) = row_offset_stages(&mut b, index, cols, ix, target);
            let lo = b.t.scratch("row_checked", 1, ix);
            // The block's first offset `lo` is checked against the offsets a block of `count` rows may start at.
            let check = ElemAccess {
                base: 0,
                terms: Vec::new(),
                indirect: Some(IndirectTerm {
                    operand: 0,
                    stride: 0,
                    modulus: n - (count - 1) * cols,
                }),
            };
            let s2 = b.compute(
                "row_checked",
                flat_space(1),
                vec![Slot {
                    buf: lo_raw,
                    map: check,
                }],
                ident_map(one_w),
                TensorBody::Operand(0),
                lo,
            );
            let hi = b.t.scratch("row_end", 1, ix);
            let s3 = b.compute(
                "row_end",
                flat_space(1),
                vec![Slot {
                    buf: lo,
                    map: fixed(0),
                }],
                ident_map(one_w),
                *add(op(0), konst(span as f64)),
                hi,
            );
            let src = ElemAccess {
                base: 0,
                terms: vec![(LV_GEN_COL, 1)],
                indirect: Some(IndirectTerm {
                    operand: 2,
                    stride: -1,
                    modulus: n,
                }),
            };
            let body = in_row(PredRhs::Operand(2), PredRhs::Operand(3));
            let write = b.pointwise(
                "write_row",
                n,
                vec![
                    Slot {
                        buf: table,
                        map: at(0),
                    },
                    Slot {
                        buf: source,
                        map: src,
                    },
                    Slot {
                        buf: lo,
                        map: fixed(0),
                    },
                    Slot {
                        buf: hi,
                        map: fixed(0),
                    },
                ],
                body,
                out_buf,
                target,
            )?;
            chain.extend([s2, s3]);
            chain.extend(write);
            chain
        }
    };
    let claim = if float {
        approx("row write", dt)
    } else {
        NumericsClaim::Exact
    };
    b.finish_as(
        t_name,
        n,
        chain,
        out_buf,
        set_row_name(spec, ""),
        claim,
        dt,
        target,
    )
}

/// A row write's program name: the table, a block's row count, and its layout's suffix.
pub(super) fn set_row_name(spec: &SetRowSpec, layout: &str) -> String {
    let block = if spec.count > 1 {
        format!("_block{}", spec.count)
    } else {
        String::new()
    };
    format!("set_row_{}x{}{block}{layout}", spec.rows, spec.cols)
}

/// `Repeat for k from 1 to N: … Set row I[k] of T to …` — a scatter of rows ([`ScatterSpec`](crate::tensor_recognize::qwen::ScatterSpec)), lowered OWNER-COMPUTES as
/// ONE rewrite of the carried table: every element `c` of `T` asks every turn whether it lands on `c`'s row, so no two
/// writes ever race for one element and the turns' order is the fold's, not a schedule's.
///
/// Turn `k` lands on the elements `[lo_k, lo_k + cols)`, `lo_k = (I[k] − 1)·cols`, its index taken less one and read
/// through a guard of stride zero against the table's rows — an index outside the table faults where every tier's check
/// fails, and is never wrapped into another row.
///   · KEPT (`Set row at of T to r`): the turns that land on `c` are folded by MAX over `key_k = n + k·cols − lo_k`, and a
///     turn that misses `c` gives zero. Every turn landing on `c` lands on `c`'s own row, with one `lo`, so the largest
///     key is the LAST such turn's, and `c + key − n` is where its source row holds `c`'s value. `T'[c]` reads that where
///     a turn landed — `0 < key`, the flat space's one row being row 0 — and keeps `T[c]` where none did: pure movement,
///     exact whatever the values.
///   · SUMMED (`Let sum be old plus r`): `T'[c] = T[c] + Σ_k [c ∈ turn k]·X[k·cols + c − lo_k]`, the turns summed by the
///     fold's own pool.
///
/// The table and the source are held whole: a turn may land anywhere, so where a site's page does not hold them the
/// scatter is refused by name — a wider one takes its turns sorted by row first, which brings each row's turns together.
pub fn from_scatter(
    spec: &crate::tensor_recognize::qwen::ScatterSpec,
    names: &StageNames,
    float: bool,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    use crate::tensor_recognize::ElementwiseOp;
    let (rows, cols, count) = (spec.rows, spec.cols, spec.count);
    if rows == 0 || cols == 0 || count == 0 || count > spec.source_rows || count > spec.index_len {
        return Err(SpatialReject::NotLowered(
            "a scatter into an empty table, or of more turns than its source has rows or its indices have entries",
        ));
    }
    if spec.fold == Some(ElementwiseOp::Mul) {
        return Err(SpatialReject::NotLowered(
            "a scatter whose turns multiply into the rows they land on — a fold sums or keeps the largest, never multiplies",
        ));
    }
    let (n, xn) = (rows * cols, spec.source_rows * cols);
    if [n, xn, spec.index_len].into_iter().any(|len| row_needs_blocking(len, target)) {
        return Err(SpatialReject::NotLowered(
            "a scatter holds its table, its source and its indices whole, and one of them is wider than a site's page — a wider scatter takes its turns sorted by row",
        ));
    }
    let mut b = FlatBuild::new(target)?;
    let (t_name, t_fill) = names.operand(0);
    let (x_name, x_fill) = names.operand(1);
    let (i_name, i_fill) = names.operand(2);
    // An integer table that sums its turns holds its own values and every turn's.
    let dt = match (float, spec.fold) {
        (false, Some(_)) => {
            let turns = (count as u64).saturating_mul(fill_abs_bound(x_fill).unwrap_or(0));
            index_type(target, fill_abs_bound(t_fill).unwrap_or(0).saturating_add(turns))?
        }
        _ => row_write_precision(&b, float, t_fill, x_fill, target)?,
    };
    let table = b.input_as(t_name, n, t_fill, dt, target);
    let source = b.input_as(x_name, xn, x_fill, dt, target);
    // A kept turn's key reaches `n + (count − 1)·cols`.
    let ix = index_type(target, index_space((n + count * cols) as u64, cols as u64, i_fill))?;
    let idx = b.input_as(i_name, spec.index_len, i_fill, ix, target);
    let (turns_t, turns_w) = (tiles_of(count, target), wide(tiles_of(count, target), target));
    let one_turn = |b: &mut FlatBuild, name: &str, slots: Vec<Slot>, body: TensorBody| {
        let out = b.t.scratch(name, turns_t, ix);
        (b.compute(name, flat_space(count), slots, ident_map(turns_w), body, out), out)
    };
    let (s0, biased) = one_turn(&mut b, "scatter_biased", vec![Slot { buf: idx, map: at(0) }], *sub(op(0), konst(1.0)));
    let guard = ElemAccess {
        base: 0,
        terms: vec![(LV_GEN_COL, 1)],
        indirect: Some(IndirectTerm {
            operand: 0,
            stride: 0,
            modulus: rows,
        }),
    };
    let (s1, guarded) = one_turn(&mut b, "scatter_guarded", vec![Slot { buf: biased, map: guard }], TensorBody::Operand(0));
    let (s2, lo) = one_turn(&mut b, "scatter_lo", vec![Slot { buf: guarded, map: at(0) }], *mul(op(0), konst(cols as f64)));
    let (s3, hi) = one_turn(&mut b, "scatter_hi", vec![Slot { buf: lo, map: at(0) }], *add(op(0), konst(cols as f64)));
    // Turn `k` of a fold over the turns: the space's row is the turn.
    let by_turn = ElemAccess {
        base: 0,
        terms: vec![(LV_GEN_ROW, 1)],
        indirect: None,
    };
    // `value` where `c` lies in the turn's row — operands 0 and 1 its first element and one past its last — and 0 elsewhere.
    let in_turn = |value: TensorBody| TensorBody::Select {
        cond: Pred::AxisGe(LV_GEN_COL, PredRhs::Operand(0)),
        then: Box::new(TensorBody::Select {
            cond: Pred::AxisLt(LV_GEN_COL, PredRhs::Operand(1)),
            then: Box::new(value),
            otherwise: konst(0.0),
        }),
        otherwise: konst(0.0),
    };
    let bounds = || vec![Slot { buf: lo, map: by_turn.clone() }, Slot { buf: hi, map: by_turn.clone() }];
    let out_buf = b.t.buffer("table", tiles_of(n, target), dt);
    let mut chain = vec![s0, s1, s2, s3];
    let kept = spec.fold.is_none();
    if kept {
        let turns = {
            let values: Vec<i64> = (0..count).map(|k| (n + k * cols) as i64).collect();
            let id = b.t.tensor(&format!("{t_name}__turns{count}x{cols}"), TensorRole::Param, 1, count, ix, TensorInit::Values(values));
            let buf = b.t.buffer_deep(&format!("in_{t_name}__turns"), 1, turns_t, ix);
            let l = b.t.load(&format!("load_{t_name}__turns"), id, vec![], turns_t, buf);
            b.loads.push((l, buf));
            buf
        };
        let (s4, key) = one_turn(
            &mut b,
            "scatter_key",
            vec![Slot { buf: turns, map: at(0) }, Slot { buf: lo, map: at(0) }],
            *sub(op(0), op(1)),
        );
        let last = b.t.scratch("scatter_last", tiles_of(n, target), ix);
        let mut slots = bounds();
        slots.push(Slot { buf: key, map: by_turn.clone() });
        let s5 = b.fold("scatter_last", n, (count, ReducePool::Max), slots, in_turn(TensorBody::Operand(2)), last, target)?;
        // Operand 0 the table, 1 the last turn's source row read at `c + key − n`, 2 the key.
        let landed = ElemAccess {
            base: -(n as i64),
            terms: vec![(LV_GEN_COL, 1)],
            indirect: Some(IndirectTerm {
                operand: 2,
                stride: 1,
                modulus: n + count * cols,
            }),
        };
        let body = TensorBody::Select {
            cond: Pred::AxisLt(LV_GEN_ROW, PredRhs::Operand(2)),
            then: op(1),
            otherwise: op(0),
        };
        let slots = vec![Slot { buf: table, map: at(0) }, Slot { buf: source, map: landed }, Slot { buf: last, map: at(0) }];
        let s6 = b.pointwise("scatter_rows", n, slots, body, out_buf, target)?;
        chain.push(s4);
        chain.extend(s5);
        chain.extend(s6);
    } else {
        // Operand 2: the source row turn `k` lands, read at `k·cols + c − lo_k`.
        let from_turn = ElemAccess {
            base: 0,
            terms: vec![(LV_GEN_COL, 1), (LV_GEN_ROW, cols as i64)],
            indirect: Some(IndirectTerm {
                operand: 0,
                stride: -1,
                modulus: n,
            }),
        };
        let sums = if float { b.scratch("scatter_sums", tiles_of(n, target)) } else { b.t.scratch("scatter_sums", tiles_of(n, target), dt) };
        let mut slots = bounds();
        slots.push(Slot { buf: source, map: from_turn });
        let s4 = b.fold("scatter_sums", n, (count, ReducePool::Sum), slots, in_turn(TensorBody::Operand(2)), sums, target)?;
        let slots = vec![Slot { buf: table, map: at(0) }, Slot { buf: sums, map: at(0) }];
        let s5 = b.pointwise("scatter_add", n, slots, *add(op(0), op(1)), out_buf, target)?;
        chain.extend(s4);
        chain.extend(s5);
    }
    let claim = if float {
        approx("scatter", dt)
    } else {
        NumericsClaim::Exact
    };
    let form = if kept { "kept" } else { "summed" };
    b.finish_fanned(t_name, n, chain, out_buf, format!("scatter_{form}_{count}x{rows}x{cols}"), claim, dt, target)
}
