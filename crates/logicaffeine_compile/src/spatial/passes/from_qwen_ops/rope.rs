//! Rotations: the rotary embedding over flat rows, a head an item, and through a window of its cache.

use super::*;

/// RoPE-NEOX over `h` heads of `hd` dims, rotating each head's leading `2·half` dims as `(i, i+half)`
/// pairs and passing the remaining `hd − 2·half` through — Qwen3.8's partial rotary.
///
/// ★ THREE REGIONS PER HEAD, ALL BANDS OF ONE FLAT AXIS. Head `h`'s rotated first half is the columns
/// `[h·hd, h·hd + half)`, its second half `[h·hd + half, h·hd + 2·half)` and its pass-through tail
/// `[h·hd + 2·half, (h+1)·hd)`. Each bound is a CONSTANT, so the whole op is one `Generic` with
/// `3·h − 1` predicates and no per-head stages at all. `q` is read through three shifted maps (`c`,
/// `c + half`, `c − half`) shared by every head; the cosine/sine tables are read through a per-head
/// base, because their index is the position WITHIN the head.
///
/// ★ A CACHE IS READ WHERE THE POSITION POINTS, IN THE ROPE'S OWN SECTION. With `from position P` the tables are
/// `cache` rows of angles and row `r` of the block turns by row `P + r`. Rows of WHOLE TILES are fetched alone: the
/// block's `rows·half` angles are a tile run from tile `(P − 1)·half_t` — a literal in the load's base, a named `P` an
/// [`IndirectTile`] over the guarded `P − 1` — so a real model's cache of thousands of positions never comes on chip.
/// Rows narrower than a tile have no tile of their own: the cache is loaded whole and every table read adds the flat
/// offset `(P − 1)·half` — a literal as one more constant, a named one as an [`IndirectTerm`] checked against the
/// positions the block admits. Either way a position past the cache faults where every tier's check fails instead of
/// reading angles that are not there.
pub fn from_rope(
    spec: &RopeSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let (heads, hd, half, rows) = (spec.h, spec.hd, spec.half, spec.rows);
    if heads == 0 || hd == 0 || half == 0 || 2 * half > hd {
        return Err(SpatialReject::NotLowered(
            "a rope whose rotated width does not fit its head dimension",
        ));
    }
    if rows == 0 || heads % rows != 0 {
        return Err(SpatialReject::NotLowered(
            "a rope whose table rows do not divide its heads",
        ));
    }
    if let Some((RowIndex::Literal(p), cache)) = spec.from {
        if p < 1 || p as usize + rows - 1 > cache {
            return Err(SpatialReject::NotLowered("a literal rope position whose block leaves the cache — the program fails at run time on every tier, so there is no section to lower"));
        }
    }
    // ★ A ROTARY EMBEDDING NEVER CROSSES A HEAD, so the head is the natural work item. The flat form
    // keeps all `heads·hd` elements resident and tells heads apart with three bands EACH; at Qwen3.8's
    // attention width that is 192 tiles against an eight-tile budget and 72 bands. One head is `hd = 256`
    // — eight tiles, exactly the budget — and inside it the band structure collapses to the three regions
    // the rotation actually has, because the page IS the head.
    // A head is the item only when it is whole tiles; a head of 80 shares tiles with its neighbours, so the rotation is
    // the one flat stage, blocked by the result's columns past an accumulator page.
    let heads_whole = !row_needs_blocking(hd, target) && hd % target.tile.cols == 0;
    if heads_whole && row_needs_blocking(heads * hd, target) {
        return Ok(vec![blocked_rope(spec, names, target)?]);
    }
    // ★ A WHOLE ROW THAT FITS NO SITE TURNS A HEAD AN ITEM. The flat form keeps the row, its shifted reads and its
    // result resident, and a flat page holds one row of each tile: on a part of small tiles that is past a site's memory
    // well before it is past the accumulator's page. A head that is not whole tiles is read through its window and
    // written into whole tiles of its own, the heads packed side by side after ([`pack_heads`]).
    let flat = flat_rope(spec, names, target)?;
    if fits_a_site(&flat, target) {
        return Ok(vec![flat]);
    }
    if heads_whole {
        return Ok(vec![blocked_rope(spec, names, target)?]);
    }
    if !row_needs_blocking(hd, target) {
        let turned = blocked_rope(spec, &StageNames { out: flash_heads_name(names), ..names.clone() }, target)?;
        let mut sections = vec![turned];
        sections.extend(pack_heads(&flash_heads_name(names), &names.out, (1, heads, hd), elem_precision(target)?, target)?);
        return Ok(sections);
    }
    Ok(vec![flat])
}

/// [`from_rope`]'s flat form: the one rotation stage over the whole row, its three regions a band each per head.
pub(super) fn flat_rope(spec: &RopeSpec, names: &StageNames, target: &SpatialTarget) -> Result<SpatialProgram, SpatialReject> {
    let (heads, hd, half, rows) = (spec.h, spec.hd, spec.half, spec.rows);
    let table_rows = spec.from.map_or(rows, |(_, cache)| cache);
    // Head `h` turns by table row `h / per_row`: its reads of the tables start `row·half` in.
    let per_row = heads / rows;
    let row_shift = |h: usize| ((h / per_row) * half) as i64;
    let n = heads * hd;
    let mut b = FlatBuild::columns(n, target)?;
    let q = b.input(names.operand(0).0, n, names.operand(0).1, target);
    let tw = target.tile.cols;
    // The tables, and the position's offset into them still to be added to every table read: none (a table row per
    // row, one row for all, or a cache whose block is fetched alone), a literal, or data.
    let (rc, rs, mut chain, shift, position) = match spec.from {
        Some((from, cache)) if half % tw == 0 => {
            let (rc, rs) = cache_block_loads(&mut b, names, from, cache, rows, half / tw, target)?;
            (rc, rs, Vec::new(), 0i64, None)
        }
        Some((from, cache)) if tiles_of(cache * half, target) > row_block_tiles(target) => {
            let (rc, rs, at) = cache_window_loads(&mut b, names, from, cache, rows, half, target)?;
            match at {
                WindowOffset::Literal(shift) => (rc, rs, Vec::new(), shift, None),
                WindowOffset::Data { stage, offset, extent } => (rc, rs, vec![stage], 0, Some((offset, extent))),
            }
        }
        from => {
            let n_table = table_rows * half;
            let rc = b.input(names.operand(1).0, n_table, names.operand(1).1, target);
            let rs = b.input(names.operand(2).0, n_table, names.operand(2).1, target);
            match from {
                None => (rc, rs, Vec::new(), 0i64, None),
                Some((RowIndex::Literal(p), _)) => {
                    (rc, rs, Vec::new(), (p - 1) * half as i64, None)
                }
                Some((RowIndex::Named(_), cache)) => {
                    let (p_name, p_fill) = names.operand(3);
                    let ix = index_type(
                        target,
                        index_space((cache * half) as u64, half as u64, p_fill),
                    )?;
                    let index = b.input_as(p_name, 1, p_fill, ix, target);
                    let (stages, lo) = row_offset_stages(&mut b, index, half, ix, target);
                    (rc, rs, stages, 0, Some((lo, (cache - rows + 1) * half)))
                }
            }
        }
    };
    let precision = b.precision;
    let out_buf = b.result_page("out", n, precision, target);

    let mut slots = vec![
        Slot { buf: q, map: at(0) }, // q[c]
        Slot {
            buf: q,
            map: at(half as i64),
        }, // q[c + half]
        Slot {
            buf: q,
            map: at(-(half as i64)),
        }, // q[c - half]
    ];
    let (q_at, q_up, q_dn) = (0usize, 1usize, 2usize);
    let offset = position.map(|(lo, extent)| {
        slots.push(Slot {
            buf: lo,
            map: fixed(0),
        });
        IndirectTerm {
            operand: slots.len() - 1,
            stride: 1,
            modulus: extent,
        }
    });
    let table = |off: i64| ElemAccess {
        indirect: offset,
        ..at(off)
    };
    let mut bands = Vec::new();
    for h in 0..heads {
        let base = (h * hd) as i64 - row_shift(h) - shift;
        // Region 1: out = q[c]·rc[c − h·hd + row·half] − q[c + half]·rs[c − h·hd + row·half].
        slots.push(Slot {
            buf: rc,
            map: table(-base),
        });
        let rc1 = slots.len() - 1;
        slots.push(Slot {
            buf: rs,
            map: table(-base),
        });
        let rs1 = slots.len() - 1;
        bands.push((
            h * hd + half,
            *sub(mul(op(q_at), op(rc1)), mul(op(q_up), op(rs1))),
        ));
        // Region 2: out = q[c − half]·rs[c − half − h·hd + row·half] + q[c]·rc[c − half − h·hd + row·half].
        slots.push(Slot {
            buf: rs,
            map: table(-base - half as i64),
        });
        let rs2 = slots.len() - 1;
        slots.push(Slot {
            buf: rc,
            map: table(-base - half as i64),
        });
        let rc2 = slots.len() - 1;
        bands.push((
            h * hd + 2 * half,
            *add(mul(op(q_dn), op(rs2)), mul(op(q_at), op(rc2))),
        ));
        // Region 3: the pass-through tail (empty when the rotary is full).
        if 2 * half < hd {
            bands.push(((h + 1) * hd, TensorBody::Operand(q_at)));
        }
    }
    chain.extend(b.pointwise("rope", n, slots, banded(bands), out_buf, target)?);
    let claim = approx("rotary embedding", b.precision);
    b.finish(
        &names.out,
        n,
        chain,
        out_buf,
        rope_name(spec),
        claim,
        target,
    )
}

/// Where a block's first table row sits in the window [`cache_window_loads`] loads: a compile-time element offset, or one
/// computed from the position into `offset` by `stage`, below `extent`.
pub(super) enum WindowOffset {
    Literal(i64),
    Data { stage: StageId, offset: BufferId, extent: usize },
}

/// ★ A CACHE TABLE PAST A PAGE IS READ THROUGH A WINDOW. The `rows` rows a rope block reads of a cache of `cache`
/// positions whose rows are `half` wide — not whole tiles, so the run starts mid-tile — begin at element `e = (P − 1)·half`.
/// Each table loads the `W = ⌈(tile − 1 + rows·half) / tile⌉` whole tiles from `min(⌊e / tile⌋, T − W)`, so the window
/// holds every row of the block and ends inside the table, and the block's first row sits at `e − tile·start` in it. A
/// literal `P` fixes all three at compile time; a named one is loaded, decremented and checked once (a guard of stride
/// zero keeping the block's last row among the cache's, as a row index is), and each table's load reads its own start
/// (a FIFO has one reader) while the rotation reads the offset. Returns the cosine and sine windows and the offset.
pub(super) fn cache_window_loads(
    b: &mut FlatBuild,
    names: &StageNames,
    from: RowIndex,
    cache: usize,
    rows: usize,
    half: usize,
    target: &SpatialTarget,
) -> Result<(BufferId, BufferId, WindowOffset), SpatialReject> {
    let (tw, precision) = (target.tile.cols, b.precision);
    let n_table = cache * half;
    let table_t = tiles_of(n_table, target);
    let window_t = (tw - 1 + rows * half).div_ceil(tw).min(table_t);
    let last = table_t - window_t;
    let declare = |b: &mut FlatBuild, k: usize| {
        let (name, fill) = names.operand(k);
        (
            b.t.tensor(name, TensorRole::Input, 1, n_table, precision, crate::spatial::passes::from_tensor_op::init_of(fill, n_table)),
            b.t.buffer(&format!("in_{name}"), window_t, precision),
        )
    };
    let ((ct, cin), (st, sin)) = (declare(b, 1), declare(b, 2));
    let load = |b: &mut FlatBuild, name: &str, tensor: TensorId, out: BufferId, origin: TileAddr, index: Option<(StageId, BufferId)>| {
        let l = b.t.stage(
            name,
            StageKind::Load {
                tensor,
                pattern: AccessPattern { origin, block_h: 1, block_w: window_t, stride_w: 1, stride_h: window_t },
                num_blocks: 1,
                multicast: None,
            },
            index.map(|(_, i)| vec![i]).unwrap_or_default(),
            vec![out],
        );
        if let Some((s, i)) = index {
            b.t.edge(s, i, l);
        }
        b.loads.push((l, out));
    };
    match from {
        RowIndex::Literal(p) => {
            if p < 1 || p as usize + rows - 1 > cache {
                return Err(SpatialReject::NotLowered("a literal rope position whose block leaves the cache — the program fails at run time on every tier, so there is no section to lower"));
            }
            let e = (p as usize - 1) * half;
            let start = (e / tw).min(last);
            for (name, tensor, out) in [("load_rc_window", ct, cin), ("load_rs_window", st, sin)] {
                load(b, name, tensor, out, TileAddr { base: start, steps: Vec::new(), indirect: None }, None);
            }
            Ok((cin, sin, WindowOffset::Literal((e - start * tw) as i64)))
        }
        RowIndex::Named(_) => {
            let (p_name, p_fill) = names.operand(3);
            let ix = index_type(target, index_space(n_table as u64, half as u64, p_fill))?;
            let w1 = wide(1, target);
            let pt = b.t.tensor(p_name, TensorRole::Input, 1, 1, ix, crate::spatial::passes::from_tensor_op::init_of(p_fill, 1));
            let pin = b.t.buffer("in_position", 1, ix);
            let lp = b.t.load("load_position", pt, vec![], 1, pin);
            let scalar = |b: &mut FlatBuild, name: &str, from: BufferId, map: ElemAccess, body: TensorBody, out: BufferId| {
                b.t.stage(
                    name,
                    StageKind::Compute(ComputeOp::Generic { iters: flat_space(1), ins: vec![map], out: ident_map(w1), body }),
                    vec![from],
                    vec![out],
                )
            };
            let biased = b.t.scratch_deep("position_biased", 1, 1, ix);
            let s_biased = scalar(b, "position_biased", pin, fixed(0), *sub(op(0), konst(1.0)), biased);
            b.t.edge(lp, pin, s_biased);
            let row = b.t.scratch_deep("position_row", 1, 1, ix);
            let guard = ElemAccess {
                base: 0,
                terms: Vec::new(),
                indirect: Some(IndirectTerm { operand: 0, stride: 0, modulus: cache - rows + 1 }),
            };
            let s_row = scalar(b, "position_row", biased, guard, TensorBody::Operand(0), row);
            b.t.edge(s_biased, biased, s_row);
            let first = b.t.scratch_deep("position_first", 1, 1, ix);
            let s_first = scalar(b, "position_first", row, fixed(0), *mul(op(0), konst(half as f64)), first);
            b.t.edge(s_row, row, s_first);
            let start_of = |e: Box<TensorBody>| Box::new(TensorBody::Min(Box::new(TensorBody::FloorDiv(e, konst(tw as f64))), konst(last as f64)));
            for (name, tensor, out) in [("load_rc_window", ct, cin), ("load_rs_window", st, sin)] {
                let start = b.t.buffer(&format!("{name}_start"), 1, ix);
                let s_start = scalar(b, &format!("{name}_start"), first, fixed(0), *start_of(op(0)), start);
                b.t.edge(s_first, first, s_start);
                let indirect = Some(IndirectTile { index: start, tile_stride: 1, elems_per_tile: 1, bound: (0, last) });
                load(b, name, tensor, out, TileAddr { base: 0, steps: Vec::new(), indirect }, Some((s_start, start)));
            }
            let offset = b.t.scratch("window_offset", 1, ix);
            let s_offset = scalar(b, "window_offset", first, fixed(0), *sub(op(0), mul(start_of(op(0)), konst(tw as f64))), offset);
            b.t.edge(s_first, first, s_offset);
            Ok((cin, sin, WindowOffset::Data { stage: s_offset, offset, extent: window_t * tw - rows * half + 1 }))
        }
    }
}

/// The `rows` rows a rope block reads of a cache of `cache` positions whose rows are `half_t` whole tiles: ONE tile run
/// per table from tile `(P − 1)·half_t`, loaded into buffers the rope's compute reads as a table row per row. A literal
/// `P` is the load's base; a named one is loaded, decremented and checked once — a guard of stride zero keeping the
/// block's last row among the cache's, as a row index is — and handed to each table's load as an [`IndirectTile`]
/// through a FIFO copy of its own (a FIFO has one reader). Returns the cosine and sine buffers.
pub(super) fn cache_block_loads(
    b: &mut FlatBuild,
    names: &StageNames,
    from: RowIndex,
    cache: usize,
    rows: usize,
    half_t: usize,
    target: &SpatialTarget,
) -> Result<(BufferId, BufferId), SpatialReject> {
    let precision = b.precision;
    let span = rows * half_t;
    let declare = |b: &mut FlatBuild, k: usize| {
        let (name, fill) = names.operand(k);
        let n = cache * half_t * target.tile.cols;
        (
            b.t.tensor(
                name,
                TensorRole::Input,
                1,
                n,
                precision,
                crate::spatial::passes::from_tensor_op::init_of(fill, n),
            ),
            b.t.buffer(&format!("in_{name}"), span, precision),
        )
    };
    let ((ct, cin), (st, sin)) = (declare(b, 1), declare(b, 2));
    let load = |b: &mut FlatBuild,
                name: &str,
                tensor: TensorId,
                out: BufferId,
                origin: TileAddr,
                index: Option<(StageId, BufferId)>| {
        let l = b.t.stage(
            name,
            StageKind::Load {
                tensor,
                pattern: AccessPattern {
                    origin,
                    block_h: 1,
                    block_w: span,
                    stride_w: 1,
                    stride_h: span,
                },
                num_blocks: 1,
                multicast: None,
            },
            index.map(|(_, i)| vec![i]).unwrap_or_default(),
            vec![out],
        );
        if let Some((s, i)) = index {
            b.t.edge(s, i, l);
        }
        b.loads.push((l, out));
    };
    match from {
        RowIndex::Literal(p) => {
            for (name, tensor, out) in [("load_rc", ct, cin), ("load_rs", st, sin)] {
                load(
                    b,
                    name,
                    tensor,
                    out,
                    TileAddr {
                        base: (p as usize - 1) * half_t,
                        steps: Vec::new(),
                        indirect: None,
                    },
                    None,
                );
            }
        }
        RowIndex::Named(_) => {
            let (p_name, p_fill) = names.operand(3);
            let ix = index_type(target, index_space(cache as u64, 1, p_fill))?;
            let w1 = wide(1, target);
            let pt = b.t.tensor(
                p_name,
                TensorRole::Input,
                1,
                1,
                ix,
                crate::spatial::passes::from_tensor_op::init_of(p_fill, 1),
            );
            let pin = b.t.buffer("in_position", 1, ix);
            let lp = b.t.load("load_position", pt, vec![], 1, pin);
            let scalar = |b: &mut FlatBuild,
                          name: &str,
                          from: BufferId,
                          map: ElemAccess,
                          body: TensorBody,
                          out: BufferId| {
                b.t.stage(
                    name,
                    StageKind::Compute(ComputeOp::Generic {
                        iters: flat_space(1),
                        ins: vec![map],
                        out: ident_map(w1),
                        body,
                    }),
                    vec![from],
                    vec![out],
                )
            };
            let biased = b.t.scratch_deep("position_biased", 1, 1, ix);
            let s_biased = scalar(
                b,
                "position_biased",
                pin,
                fixed(0),
                *sub(op(0), konst(1.0)),
                biased,
            );
            b.t.edge(lp, pin, s_biased);
            let row = b.t.scratch_deep("position_row", 1, 1, ix);
            let guard = ElemAccess {
                base: 0,
                terms: Vec::new(),
                indirect: Some(IndirectTerm {
                    operand: 0,
                    stride: 0,
                    modulus: cache - rows + 1,
                }),
            };
            let s_row = scalar(
                b,
                "position_row",
                biased,
                guard,
                TensorBody::Operand(0),
                row,
            );
            b.t.edge(s_biased, biased, s_row);
            for (name, tensor, out) in [("load_rc", ct, cin), ("load_rs", st, sin)] {
                let copy = b.t.buffer(&format!("{name}_row"), 1, ix);
                let s_copy = scalar(
                    b,
                    &format!("{name}_row"),
                    row,
                    fixed(0),
                    TensorBody::Operand(0),
                    copy,
                );
                b.t.edge(s_row, row, s_copy);
                let indirect = Some(IndirectTile {
                    index: copy,
                    tile_stride: half_t,
                    elems_per_tile: 1,
                    bound: (0, cache - rows),
                });
                load(
                    b,
                    name,
                    tensor,
                    out,
                    TileAddr {
                        base: 0,
                        steps: Vec::new(),
                        indirect,
                    },
                    Some((s_copy, copy)),
                );
            }
        }
    }
    Ok((cin, sin))
}

/// A rope program's name: its heads, head dimension and rotated half — and its rows, when there is more than one.
pub(super) fn rope_name(spec: &RopeSpec) -> String {
    let rows = if spec.rows > 1 {
        format!("_rows{}", spec.rows)
    } else {
        String::new()
    };
    format!("rope_{}x{}_half{}{rows}", spec.h, spec.hd, spec.half)
}

/// [`from_rope`] with the HEAD as the work item — see the note there for why that is the natural unit.
pub(super) fn blocked_rope(
    spec: &RopeSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (heads, hd, half, rows) = (spec.h, spec.hd, spec.half, spec.rows);
    let n = heads * hd;
    let head_t = tiles_of(hd, target);
    let half_t = tiles_of(half, target);
    let tw = target.tile.cols;
    // ★ A HEAD THAT IS NOT WHOLE TILES IS READ THROUGH ITS WINDOW: the run of tiles around it from the tile a compile-time
    // table names per item, the head at its offset in them ([`head_window`]). Its turned values land in whole tiles of its
    // own, for [`pack_heads`] to pack.
    let head_windows = hd % tw != 0;
    let (q_t, out_t) = if head_windows { (window_run(heads, hd, tw), hd.div_ceil(tw)) } else { (head_t, head_t) };
    // ★ A HEAD TURNS BY ITS ROW'S TABLE ROW. Item `(y, x)` is head `x` of query row `y`, so the query steps by a row's
    // heads per `y` and the tables by one table row per `y` — which is a whole number of tiles only when `half` is.
    let per_row = heads / rows.max(1);
    // ★ A TABLE ROW THAT IS NOT WHOLE TILES IS READ THROUGH A WINDOW. Row `ρ` starts at element `ρ·half`, mid-tile; the
    // item loads the whole tiles around it and reads the row at its offset in the page — both from the row's first
    // element, computed per item.
    let windowed = (rows > 1 || spec.from.is_some()) && half % target.tile.cols != 0;
    let table_rows = spec.from.map_or(rows, |(_, cache)| cache);
    let row_steps = |per_y: usize, mut v: Vec<(LoopVar, usize)>| {
        if rows > 1 {
            v.insert(0, (LV_ITEM_Y, per_y));
        }
        v
    };
    let precision = elem_precision(target)?;
    let mut t = Template::new(target)?;
    let (qn, qf) = names.operand(0);
    let (cn, cf) = names.operand(1);
    let (sn, sf) = names.operand(2);
    let qt = t.tensor(
        qn,
        TensorRole::Input,
        1,
        n,
        precision,
        crate::spatial::passes::from_tensor_op::init_of(qf, n),
    );
    let ct = t.tensor(
        cn,
        TensorRole::Input,
        1,
        table_rows * half,
        precision,
        crate::spatial::passes::from_tensor_op::init_of(cf, table_rows * half),
    );
    let st = t.tensor(
        sn,
        TensorRole::Input,
        1,
        table_rows * half,
        precision,
        crate::spatial::passes::from_tensor_op::init_of(sf, table_rows * half),
    );
    let yt = t.tensor(
        &names.out,
        TensorRole::Output,
        1,
        heads * out_t * tw,
        precision,
        TensorInit::Zero,
    );
    let table_t = tiles_of(table_rows * half, target);
    let window_t = if windowed { (tw - 1 + half).div_ceil(tw).min(table_t) } else { half_t };
    let qin = t.buffer_deep("in_q", 1, q_t, precision);
    let cin = t.buffer_deep("in_rc", 1, window_t, precision);
    let sin = t.buffer_deep("in_rs", 1, window_t, precision);
    let ob = t.buffer_deep("out", 1, out_t, precision);
    // One table row is the SAME for every head of its row, so the tables' tile address moves only with the row — and,
    // reading a cache, starts at the position's row `P − 1`: a literal in the base; a named one an [`IndirectTile`] over
    // `P − 1`, computed per item on the integer datapath into a FIFO its load retires (one per table: a FIFO has one
    // reader) — behind a guard of stride zero that keeps the block's last row among the cache's, as a row index is.
    // Item `h`'s window: two tiles of the table, its first tile in the first, its head's offset in the second.
    let (lq, q_offset) = if head_windows {
        let n_t = tiles_of(n, target);
        let windows: Vec<(usize, usize)> = (0..heads).map(|h| head_window(h * hd, n_t, q_t, tw)).collect();
        let largest = windows.iter().map(|(first, offset)| (*first).max(*offset)).max().unwrap_or(0);
        let wx = index_type(target, largest as u64)?;
        let entries: Vec<i64> = windows
            .iter()
            .flat_map(|(first, offset)| {
                std::iter::once(*first as i64)
                    .chain(std::iter::repeat_n(0, tw - 1))
                    .chain(std::iter::once(*offset as i64))
                    .chain(std::iter::repeat_n(0, tw - 1))
            })
            .collect();
        let table = t.tensor(&format!("{}__rope_windows", names.out), TensorRole::Param, 1, heads * 2 * tw, wx, TensorInit::Values(entries));
        let first_in = t.buffer_deep("in_q_first", 1, 1, wx);
        let l_first = t.load_block_at("load_q_first", table, 0, row_steps(2 * per_row, vec![(LV_ITEM_X, 2)]), 1, 1, 1, first_in);
        let off_in = t.buffer_deep("in_q_offset", 1, 1, wx);
        let l_off = t.load_block_at("load_q_offset", table, 1, row_steps(2 * per_row, vec![(LV_ITEM_X, 2)]), 1, 1, 1, off_in);
        let firsts = windows.iter().map(|(first, _)| *first);
        let bound = (firsts.clone().min().unwrap_or(0), firsts.max().unwrap_or(0));
        let lq = t.stage(
            "load_q",
            StageKind::Load {
                tensor: qt,
                pattern: AccessPattern {
                    origin: TileAddr { base: 0, steps: Vec::new(), indirect: Some(IndirectTile { index: first_in, tile_stride: 1, elems_per_tile: 1, bound }) },
                    block_h: 1,
                    block_w: q_t,
                    stride_w: 1,
                    stride_h: q_t,
                },
                num_blocks: 1,
                multicast: None,
            },
            vec![first_in],
            vec![qin],
        );
        t.edge(l_first, first_in, lq);
        (lq, Some((l_off, off_in)))
    } else {
        (t.load_block_at("load_q", qt, 0, row_steps(per_row * head_t, vec![(LV_ITEM_X, head_t)]), 1, head_t, head_t, qin), None)
    };
    let position = match spec.from {
        Some((RowIndex::Named(_), cache)) => {
            let (pn, pf) = names.operand(3);
            let ix = index_type(target, index_space(cache as u64, 1, pf))?;
            Some((
                t.tensor(
                    pn,
                    TensorRole::Input,
                    1,
                    1,
                    ix,
                    crate::spatial::passes::from_tensor_op::init_of(pf, 1),
                ),
                ix,
                cache,
            ))
        }
        _ => None,
    };
    let base = match spec.from {
        Some((RowIndex::Literal(p), cache)) => {
            if p < 1 || p as usize + rows - 1 > cache {
                return Err(SpatialReject::NotLowered("a literal rope position whose block leaves the cache — the program fails at run time on every tier, so there is no section to lower"));
            }
            (p as usize - 1) * half_t
        }
        _ => 0,
    };
    let table_load = |t: &mut Template, name: &str, tensor: TensorId, out: BufferId| -> StageId {
        let Some((pt, ix, cache)) = position else {
            return t.load_block_at(
                name,
                tensor,
                base,
                row_steps(half_t, vec![]),
                1,
                half_t,
                half_t,
                out,
            );
        };
        let w1 = wide(1, target);
        let pin = t.buffer(&format!("{name}_position"), 1, ix);
        let lp = t.load(&format!("{name}_position"), pt, vec![], 1, pin);
        let biased = t.scratch_deep(&format!("{name}_biased"), 1, 1, ix);
        let sb = t.stage(
            &format!("{name}_biased"),
            StageKind::Compute(ComputeOp::Generic {
                iters: flat_space(1),
                ins: vec![fixed(0)],
                out: ident_map(w1),
                body: *sub(op(0), konst(1.0)),
            }),
            vec![pin],
            vec![biased],
        );
        t.edge(lp, pin, sb);
        let row0 = t.buffer(&format!("{name}_row"), 1, ix);
        let guard = ElemAccess {
            base: 0,
            terms: Vec::new(),
            indirect: Some(IndirectTerm {
                operand: 0,
                stride: 0,
                modulus: cache - rows + 1,
            }),
        };
        let s = t.stage(
            &format!("{name}_row"),
            StageKind::Compute(ComputeOp::Generic {
                iters: flat_space(1),
                ins: vec![guard],
                out: ident_map(w1),
                body: TensorBody::Operand(0),
            }),
            vec![biased],
            vec![row0],
        );
        t.edge(sb, biased, s);
        let origin = TileAddr {
            base: 0,
            steps: row_steps(half_t, vec![]),
            indirect: Some(IndirectTile {
                index: row0,
                tile_stride: half_t,
                elems_per_tile: 1,
                bound: (0, cache - rows),
            }),
        };
        let l = t.stage(
            name,
            StageKind::Load {
                tensor,
                pattern: AccessPattern {
                    origin,
                    block_h: 1,
                    block_w: half_t,
                    stride_w: 1,
                    stride_h: half_t,
                },
                num_blocks: 1,
                multicast: None,
            },
            vec![row0],
            vec![out],
        );
        t.edge(s, row0, l);
        l
    };
    // The windowed table read: item `(y, ·)`'s row starts at element `e = (P₀ + y)·half` — `P₀` the position less one
    // (checked, as a row index is) or the literal's, `y·half` from a compile-time table one tile per query row — its
    // window at tile `min(⌊e/32⌋, T − W)`, so no window reads past the table, and the row at `e − 32·start` in the page.
    let ix_rows = index_type(target, (table_rows * half + tw) as u64)?;
    let rows_table = windowed.then(|| {
        let literal = match spec.from {
            Some((RowIndex::Literal(p), _)) => p as usize - 1,
            _ => 0,
        };
        let tiles: Vec<i64> = (0..rows).flat_map(|y| std::iter::once(((literal + y) * half) as i64).chain(std::iter::repeat_n(0, tw - 1))).collect();
        t.tensor(&format!("{}__rope_rows", names.out), TensorRole::Param, 1, rows * tw, ix_rows, TensorInit::Values(tiles))
    });
    let first_element = |t: &mut Template, name: &str| -> Result<(Vec<StageId>, BufferId, Precision), SpatialReject> {
        let ix = ix_rows;
        let rows_t = rows_table.expect("a windowed rope declares its rows");
        let at_row = t.buffer_deep(&format!("{name}_row_start"), 1, 1, ix);
        let steps = if rows > 1 { vec![(LV_ITEM_Y, 1)] } else { Vec::new() };
        let l_row = t.load_block_at(&format!("{name}_row_start"), rows_t, 0, steps, 1, 1, 1, at_row);
        let w1 = wide(1, target);
        let e = t.scratch_deep(&format!("{name}_first"), 1, 1, ix);
        let mut stages = vec![l_row];
        match position {
            None => {
                let s = t.stage(&format!("{name}_first"), generic_op(flat_space(1), vec![fixed(0)], ident_map(w1), TensorBody::Operand(0)), vec![at_row], vec![e]);
                t.edge(l_row, at_row, s);
                stages.push(s);
            }
            Some((pt, pix, cache)) => {
                let pin = t.buffer(&format!("{name}_position"), 1, pix);
                let lp = t.load(&format!("{name}_position"), pt, vec![], 1, pin);
                let biased = t.scratch_deep(&format!("{name}_biased"), 1, 1, pix);
                let s_b = t.stage(&format!("{name}_biased"), generic_op(flat_space(1), vec![fixed(0)], ident_map(w1), *sub(op(0), konst(1.0))), vec![pin], vec![biased]);
                t.edge(lp, pin, s_b);
                // `P − 1`, inside the rows a block of `rows` may start at — the stride-zero guard a row index gets.
                let guard = ElemAccess { base: 0, terms: Vec::new(), indirect: Some(IndirectTerm { operand: 0, stride: 0, modulus: cache - rows + 1 }) };
                let s = t.stage(
                    &format!("{name}_first"),
                    generic_op(flat_space(1), vec![guard, fixed(0)], ident_map(w1), *add(mul(op(0), konst(half as f64)), op(1))),
                    vec![biased, at_row],
                    vec![e],
                );
                t.edge(s_b, biased, s);
                t.edge(l_row, at_row, s);
                stages.extend([lp, s_b, s]);
            }
        }
        Ok((stages, e, ix))
    };
    let window_start = |t: &mut Template, name: &str, e: BufferId, ix: Precision| -> (StageId, BufferId) {
        let start = t.scratch_deep(&format!("{name}_window"), 1, 1, ix);
        let last = (table_t - window_t) as f64;
        let body = TensorBody::Min(Box::new(TensorBody::FloorDiv(op(0), konst(tw as f64))), konst(last));
        let s = t.stage(&format!("{name}_window"), generic_op(flat_space(1), vec![fixed(0)], ident_map(wide(1, target)), body), vec![e], vec![start]);
        (s, start)
    };
    let windowed_load = |t: &mut Template, name: &str, tensor: TensorId, out: BufferId| -> Result<(StageId, Vec<StageId>), SpatialReject> {
        let (mut stages, e, ix) = first_element(t, name)?;
        let (s_start, start) = window_start(t, name, e, ix);
        t.edge(*stages.last().expect("a first element"), e, s_start);
        stages.push(s_start);
        let l = t.stage(
            name,
            StageKind::Load {
                tensor,
                pattern: AccessPattern {
                    origin: TileAddr {
                        base: 0,
                        steps: Vec::new(),
                        indirect: Some(IndirectTile { index: start, tile_stride: 1, elems_per_tile: 1, bound: (0, table_t - window_t) }),
                    },
                    block_h: 1,
                    block_w: window_t,
                    stride_w: 1,
                    stride_h: window_t,
                },
                num_blocks: 1,
                multicast: None,
            },
            vec![start],
            vec![out],
        );
        t.edge(s_start, start, l);
        Ok((l, stages))
    };
    let (lc, ls, offset) = if windowed {
        let (lc, _) = windowed_load(&mut t, "load_rc", ct, cin)?;
        let (ls, _) = windowed_load(&mut t, "load_rs", st, sin)?;
        // The row's offset in its window: `e − 32·start`, from a first element of its own.
        let (stages, e, ix) = first_element(&mut t, "rope_offset")?;
        let (s_start, start) = window_start(&mut t, "rope_offset", e, ix);
        t.edge(*stages.last().expect("a first element"), e, s_start);
        let off = t.scratch_deep("rope_offset", 1, 1, ix);
        let s_off = t.stage(
            "rope_offset",
            generic_op(flat_space(1), vec![fixed(0), fixed(0)], ident_map(wide(1, target)), *sub(op(0), mul(op(1), konst(tw as f64)))),
            vec![e, start],
            vec![off],
        );
        t.edge(s_start, start, s_off);
        (lc, ls, Some((s_off, off)))
    } else {
        (table_load(&mut t, "load_rc", ct, cin), table_load(&mut t, "load_rs", st, sin), None)
    };
    let w = wide(head_t, target);
    // Inside one head the three regions are the rotation's own: rotate the low half against the high,
    // the high half against the low, and pass through whatever the rotary does not cover.
    let mut slots: Vec<ElemAccess> = vec![
        at(0),              // 0: q[c]
        at(half as i64),    // 1: q[c + half]
        at(-(half as i64)), // 2: q[c - half]
        at(0),              // 3: rc[c]
        at(0),              // 4: rs[c]
        at(-(half as i64)), // 5: rs[c - half]
        at(-(half as i64)), // 6: rc[c - half]
    ];
    let mut ins = vec![qin, qin, qin, cin, sin, sin, cin];
    // A windowed table is read at its row's offset in the page: operand 7.
    if let Some((_, off)) = offset {
        for slot in &mut slots[3..7] {
            slot.indirect = Some(IndirectTerm { operand: 7, stride: 1, modulus: window_t * tw });
        }
        slots.push(fixed(0));
        ins.push(off);
    }
    // A windowed head's queries at its offset in their window: the operand after the table's.
    if let Some((_, off)) = q_offset {
        let operand = slots.len();
        for slot in &mut slots[0..3] {
            slot.indirect = Some(IndirectTerm { operand, stride: 1, modulus: q_t * tw - hd + 1 });
        }
        slots.push(fixed(0));
        ins.push(off);
    }
    let mut bands = vec![
        (half, *sub(mul(op(0), op(3)), mul(op(1), op(4)))),
        (2 * half, *add(mul(op(2), op(5)), mul(op(0), op(6)))),
    ];
    if 2 * half < hd {
        bands.push((hd, TensorBody::Operand(0)));
    }
    let (iters, out) = if head_windows { (flat_space(hd), ident_map(wide(out_t, target))) } else { (flat_space(w), ident_map(w)) };
    let s = t.stage(
        "rope",
        StageKind::Compute(ComputeOp::Generic {
            iters,
            ins: slots,
            out,
            body: banded(bands),
        }),
        ins,
        vec![ob],
    );
    let store = t.store_block(
        "store_y",
        yt,
        0,
        row_steps(per_row * out_t, vec![(LV_ITEM_X, out_t)]),
        1,
        out_t,
        out_t,
        ob,
    );
    for l in [lq, lc, ls] {
        let b = match l {
            x if x == lq => qin,
            x if x == lc => cin,
            _ => sin,
        };
        t.edge(l, b, s);
    }
    if let Some((s_off, off)) = offset {
        t.edge(s_off, off, s);
    }
    if let Some((l_off, off)) = q_offset {
        t.edge(l_off, off, s);
    }
    t.edge(s, ob, store);
    let items: Vec<WorkItem> = (0..heads)
        .map(|h| {
            let coord = if rows > 1 {
                (h / per_row, h % per_row)
            } else {
                (0, h)
            };
            WorkItem {
                id: WorkItemId::new(h),
                coord,
                out: TileRect {
                    row0_t: 0,
                    col0_t: h * out_t,
                    h_t: 1,
                    w_t: out_t,
                },
            }
        })
        .collect();
    t.finish(
        rope_name(spec),
        items,
        AccPolicy::F32,
        approx("rotary embedding", precision),
        target,
    )
}
