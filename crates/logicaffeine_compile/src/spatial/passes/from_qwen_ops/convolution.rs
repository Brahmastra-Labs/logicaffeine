//! Causal convolutions over a ring of past rows.

use super::*;

// ── The carried-state family ──────────────────────────────────────────────────────────────────────

/// `Y[c] = Σ_t CS'[t, c]·WC[t, c]` where `CS' = shift(CS) ← X` — the depthwise causal conv1d over a
/// `taps`-deep decode ring, as TWO sections.
///
/// ★ THE RING IS WRITTEN IN ONE SECTION AND READ IN THE NEXT, AND THAT IS THE WHOLE POINT. The update
/// and the reduction are separated by a barrier, so the order between them is a fact the certificate
/// checks (`StateOrder`) and the evaluator enforces (`StaleStateRead`) rather than an accident of which
/// engine happened to run first. Fold them into one section and a reversed scheduler would compute the
/// convolution against LAST token's ring and nothing would notice.
pub fn from_causal_conv(
    spec: &CausalConvSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let (taps, chan) = (spec.taps, spec.chan);
    if taps == 0 || chan == 0 {
        return Err(SpatialReject::NotLowered(
            "a causal conv with no taps or no channels",
        ));
    }
    if spec.rows > 1 {
        return block_causal_conv(spec, names, target);
    }
    if ring_is_matrix(spec, target) {
        return blocked_causal_conv(spec, names, target);
    }
    flat_causal_conv(spec, names, target)
}

/// Whether a conv ring of `taps × chan` is held as the matrix it is — a column block of every tap an item — rather than
/// as one flat row: when the row is wider than an accumulator page, or the single step's flat form does not fit a site
/// (a flat row fills a tile row of the datapath's tile a row of elements, so its pages grow with the tile's height). One
/// decision for the single step and a block of rows, so a prefill chunk leaves the ring in the layout a decode reads.
pub(super) fn ring_is_matrix(spec: &CausalConvSpec, target: &SpatialTarget) -> bool {
    if row_needs_blocking(spec.taps * spec.chan, target) {
        return true;
    }
    let step = CausalConvSpec { rows: 1, ..spec.clone() };
    let names = StageNames { out: "y".into(), ins: vec!["x".into(), "w".into(), "cs".into()], fills: vec![Fill::Unknown; 3] };
    flat_causal_conv(&step, &names, target).map_or(true, |sections| sections.iter().any(|p| site_footprint(p) > target.max_local_mem_bytes()))
}

/// The single step's conv over a flat ring: the ring update, then the reduction against the weight.
pub(super) fn flat_causal_conv(
    spec: &CausalConvSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let (taps, chan) = (spec.taps, spec.chan);
    let ring_len = taps * chan;
    let (x_name, x_fill) = names.operand(0);
    let (w_name, w_fill) = names.operand(1);
    let (cs_name, cs_fill) = names.operand(2);

    // ── Section 1: the ring update. `CS'[t, c] = t < taps-1 ? CS[t+1, c] : X[c]`, over the flat ring.
    let shift = {
        let mut b = FlatBuild::new(target)?;
        let cs = b.input(cs_name, ring_len, cs_fill, target);
        let x = b.input(x_name, chan, x_fill, target);
        let cols_t = tiles_of(ring_len, target);
        let out_buf = b.t.buffer("ring", cols_t, b.precision);
        let ow = wide(cols_t, target);
        // Two bands: the shifted rows, then the appended newest row.
        let slots = vec![
            Slot {
                buf: cs,
                map: at(chan as i64),
            }, // CS[c + chan] — one row up
            Slot {
                buf: x,
                map: at(-(((taps - 1) * chan) as i64)),
            }, // X[c - (taps-1)·chan]
        ];
        let body = banded(vec![
            ((taps - 1) * chan, TensorBody::Operand(0)),
            (ring_len, TensorBody::Operand(1)),
        ]);
        let s = b.compute(
            "ring_shift",
            flat_space(ring_len),
            slots,
            ident_map(ow),
            body,
            out_buf,
        );
        let claim = NumericsClaim::Exact;
        b.finish(
            cs_name,
            ring_len,
            vec![s],
            out_buf,
            format!("conv_ring_{taps}x{chan}"),
            claim,
            target,
        )?
    };

    // ── Section 2: the tap reduction against the UPDATED ring.
    let reduce = {
        let mut b = FlatBuild::new(target)?;
        let cs = b.input(cs_name, ring_len, cs_fill, target);
        let wc = b.input(w_name, ring_len, w_fill, target);
        let cols_t = tiles_of(chan, target);
        let out_buf = b.t.buffer("out", cols_t, b.precision);
        let tile_elems = target.tile.rows * target.tile.cols;
        let ones = b.t.tensor(
            "conv_scaler",
            TensorRole::Param,
            target.tile.rows,
            target.tile.cols,
            b.precision,
            TensorInit::FloatValues(vec![1.0f64.to_bits(); tile_elems]),
        );
        let ones_in = b.t.buffer("in_scaler", 1, b.precision);
        let l = b.t.load("load_scaler", ones, vec![], 1, ones_in);
        b.loads.push((l, ones_in));
        // ★ THE TAP AXIS IS THE REDUCED ONE, AND IT IS THE *ROW*. Folding the ROW axis leaves one value
        // per COLUMN, written DOWN the output tile — so the answer is present at row 0 of every column,
        // which IS the flat `1 × chan` layout the next section reads. (Folding the column axis instead
        // would leave one value per ROW, and a flat reader would see only the first head's.) The channel
        // is the column, the tap is the row, and `CS[t, c]` is the flat offset `t·chan + c`.
        let iters = vec![
            IterSpec {
                var: LV_GEN_ROW,
                extent: taps,
                kind: IterKind::Reduce {
                    pool: ReducePool::Sum,
                },
            },
            IterSpec {
                var: LV_GEN_COL,
                extent: chan,
                kind: IterKind::Parallel,
            },
        ];
        let slots = vec![
            Slot {
                buf: cs,
                map: map2(0, chan as i64, 1),
            },
            Slot {
                buf: wc,
                map: map2(0, chan as i64, 1),
            },
            Slot {
                buf: ones_in,
                map: const_map(),
            },
        ];
        let s = b.compute(
            "tap_reduce",
            iters,
            slots,
            per_col_map(),
            *mul(op(0), op(1)),
            out_buf,
        );
        let claim = approx("causal convolution", b.precision);
        b.finish(
            &names.out,
            chan,
            vec![s],
            out_buf,
            format!("conv_taps_{taps}x{chan}"),
            claim,
            target,
        )?
    };
    Ok(vec![shift, reduce])
}

/// `Y = causal conv(X, WC, CS)` over a block of `rows` rows. The rows enter the ring one after another, so with
/// `E = [CS; X]` — the ring's `taps` rows, then the block's — row `r` convolves the window `E[r + 1 … r + taps]` and the
/// ring the block leaves is `E[rows … rows + taps − 1]`. TWO sections: every output row against the ring as it came in,
/// then the ring update — the barrier between them is the order "read the old ring, then write the new one".
///
/// `X` and `Y` are the DENSE `rows × chan` matrices a projection writes and a pointwise op reads. The ring and the weight
/// keep the layout the single step gives them — a `taps × chan` matrix where the ring is wider than a site's row (every
/// tap of a channel tile in one tile), the flat `1 × taps·chan` row otherwise — so a decode step continues from the ring
/// a prefill chunk leaves. A column block is whole tiles of a matrix ring, the whole row of a flat one.
///
/// ★ THE HALO IS THE ONLY NON-UNIFORM READ, AND ITS SOURCE IS DATA. Output row `y` of tile row `R` reads window row
/// `j = y + 1 + t − taps` of the block: its own tile row when `j ≥ 0`, else the row `j` before it — the previous tile
/// row of `X`, or, for the first tile row, the ring. The choice is a per-item threshold from a compile-time table (the
/// tile height for the first tile row, zero after), so every item runs the same stages.
pub(super) fn block_causal_conv(
    spec: &CausalConvSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let (rows, taps, chan) = (spec.rows, spec.taps, spec.chan);
    let (tr, tw) = (target.tile.rows, target.tile.cols);
    if taps > tr {
        return Err(SpatialReject::NotLowered(
            "a block causal conv whose ring is taller than a tile row",
        ));
    }
    // The single step's ring layout, decided the way `from_causal_conv` decides it.
    let matrix_ring = ring_is_matrix(spec, target);
    if matrix_ring && chan % tw != 0 {
        return Err(SpatialReject::NotLowered(
            "a blocked causal conv whose channel count is not a whole number of tiles",
        ));
    }
    let (cols_t, rows_t) = (chan.div_ceil(tw), rows.div_ceil(tr));
    let dt = elem_precision(target)?;
    let ix = index_type(target, (rows_t * cols_t + tr) as u64)?;
    let blk = if matrix_ring {
        row_block(cols_t, target)
    } else {
        cols_t
    };
    let n_b = cols_t / blk;
    let wc_w = wide(blk, target);
    let ring_t = tiles_of(taps * chan, target);
    // Where ring row `k` sits in its page: plane row `k` of a matrix ring's tile row, or `k·chan` along a flat one.
    let seg = if matrix_ring { wc_w } else { chan };
    let ring_tiles = if matrix_ring { blk } else { ring_t };
    let (x_name, x_fill) = names.operand(0);
    let (w_name, w_fill) = names.operand(1);
    let (cs_name, cs_fill) = names.operand(2);
    let ring_tensor = |t: &mut Template, name: &str, role: TensorRole, fill: &Fill| {
        let (r, c) = if matrix_ring {
            (taps, chan)
        } else {
            (1, taps * chan)
        };
        t.tensor(
            name,
            role,
            r,
            c,
            dt,
            crate::spatial::passes::from_tensor_op::init_of(fill, taps * chan),
        )
    };
    let ring_page = |t: &mut Template, name: &str, tensor: TensorId, into: BufferId| {
        if matrix_ring {
            t.load_block_at(
                name,
                tensor,
                0,
                vec![(LV_ITEM_X, blk)],
                1,
                blk,
                cols_t,
                into,
            )
        } else {
            t.load_block_at(name, tensor, 0, Vec::new(), 1, ring_t, ring_t, into)
        }
    };
    let per_tile_row = |t: &mut Template, what: &str, values: Vec<i64>| {
        let tiles = values
            .into_iter()
            .flat_map(|v| std::iter::once(v).chain(std::iter::repeat_n(0, tw - 1)))
            .collect();
        t.tensor(
            &format!("{}__conv_{what}", names.out),
            TensorRole::Param,
            1,
            rows_t * tw,
            ix,
            TensorInit::Values(tiles),
        )
    };
    let rows_map = |base: i64, stride: usize| ElemAccess {
        base,
        terms: vec![(LV_GEN_ROW, stride as i64), (LV_GEN_COL, 1)],
        indirect: None,
    };

    // ── Section 1: every output row, against the ring as it came in.
    let outputs = {
        let mut t = Template::new(target)?;
        let x = t.tensor(
            x_name,
            TensorRole::Input,
            rows,
            chan,
            dt,
            crate::spatial::passes::from_tensor_op::init_of(x_fill, rows * chan),
        );
        let w = ring_tensor(&mut t, w_name, TensorRole::Input, w_fill);
        let cs = ring_tensor(&mut t, cs_name, TensorRole::Input, cs_fill);
        let y = t.tensor(
            &names.out,
            TensorRole::Output,
            rows,
            chan,
            dt,
            TensorInit::Zero,
        );
        let prev_rows = per_tile_row(
            &mut t,
            "previous_row",
            (0..rows_t).map(|r| r.saturating_sub(1) as i64).collect(),
        );
        let halo = per_tile_row(
            &mut t,
            "halo",
            (0..rows_t)
                .map(|r| if r == 0 { tr as i64 } else { 0 })
                .collect(),
        );
        let (c_in, p_in, ring_in, w_in) = (
            t.buffer("in_rows", blk, dt),
            t.buffer("in_previous_rows", blk, dt),
            t.buffer("in_ring", ring_tiles, dt),
            t.buffer("in_weight", ring_tiles, dt),
        );
        let (prev_in, halo_in) = (
            t.buffer("in_previous_row", 1, ix),
            t.buffer("in_halo", 1, ix),
        );
        let l_c = t.load_block_at(
            "load_rows",
            x,
            0,
            vec![(LV_ITEM_Y, cols_t), (LV_ITEM_X, blk)],
            1,
            blk,
            cols_t,
            c_in,
        );
        let l_prev = t.load_block_at(
            "load_previous_row",
            prev_rows,
            0,
            vec![(LV_ITEM_Y, 1)],
            1,
            1,
            1,
            prev_in,
        );
        let origin = TileAddr {
            base: 0,
            steps: vec![(LV_ITEM_X, blk)],
            indirect: Some(IndirectTile {
                index: prev_in,
                tile_stride: cols_t,
                elems_per_tile: 1,
                bound: (0, rows_t - 1),
            }),
        };
        let l_p = t.stage(
            "load_previous_rows",
            StageKind::Load {
                tensor: x,
                pattern: AccessPattern {
                    origin,
                    block_h: 1,
                    block_w: blk,
                    stride_w: 1,
                    stride_h: cols_t,
                },
                num_blocks: 1,
                multicast: None,
            },
            vec![prev_in],
            vec![p_in],
        );
        t.edge(l_prev, prev_in, l_p);
        let l_ring = ring_page(&mut t, "load_ring", cs, ring_in);
        let l_w = ring_page(&mut t, "load_weight", w, w_in);
        let l_halo = t.load_block_at("load_halo", halo, 0, vec![(LV_ITEM_Y, 1)], 1, 1, 1, halo_in);
        let (mut ins, mut maps) = (vec![halo_in], vec![fixed(0)]);
        let mut slot = |b: BufferId, m: ElemAccess| {
            ins.push(b);
            maps.push(m);
            op(ins.len() - 1)
        };
        let mut body: Option<Box<TensorBody>> = None;
        for tap in 0..taps {
            let shift = 1 + tap as i64 - taps as i64;
            let own = slot(c_in, rows_map(shift * wc_w as i64, wc_w));
            let before = slot(p_in, rows_map((tr as i64 + shift) * wc_w as i64, wc_w));
            let ring = slot(ring_in, rows_map((1 + tap as i64) * seg as i64, seg));
            let weight = slot(
                w_in,
                ElemAccess {
                    base: (tap * seg) as i64,
                    terms: vec![(LV_GEN_COL, 1)],
                    indirect: None,
                },
            );
            let halo_read = Box::new(TensorBody::Select {
                cond: Pred::AxisLt(LV_GEN_ROW, PredRhs::Operand(0)),
                then: ring,
                otherwise: before,
            });
            let window = Box::new(TensorBody::Select {
                cond: Pred::AxisGe(LV_GEN_ROW, PredRhs::Const(taps - 1 - tap)),
                then: own,
                otherwise: halo_read,
            });
            let term = mul(weight, window);
            body = Some(match body {
                None => term,
                Some(acc) => add(acc, term),
            });
        }
        let out = t.buffer("out", blk, dt);
        let s = t.stage(
            "conv_rows",
            StageKind::Compute(ComputeOp::Generic {
                iters: crate::spatial::passes::from_tensor_op::space(target, wc_w, None),
                ins: maps,
                out: ident_map(wc_w),
                body: *body.expect("a conv has taps"),
            }),
            ins.clone(),
            vec![out],
        );
        for (from, buf) in [
            (l_c, c_in),
            (l_p, p_in),
            (l_ring, ring_in),
            (l_w, w_in),
            (l_halo, halo_in),
        ] {
            t.edge(from, buf, s);
        }
        let st = t.store_block(
            "store_y",
            y,
            0,
            vec![(LV_ITEM_Y, cols_t), (LV_ITEM_X, blk)],
            1,
            blk,
            cols_t,
            out,
        );
        t.edge(s, out, st);
        let items: Vec<WorkItem> = (0..rows_t)
            .flat_map(|r| (0..n_b).map(move |b| (r, b)))
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
            .collect();
        let mut prog = t.finish(
            format!("conv_rows_{rows}x{chan}_taps{taps}"),
            items,
            AccPolicy::F32,
            approx("causal convolution", dt),
            target,
        )?;
        prog.work.plan = Some(BlockPlan {
            grid: (rows_t, n_b),
            ..tile_row_plan(blk, blk, dt)
        });
        prog
    };

    // ── Section 2: the ring the block leaves, `CS'[t] = E[rows + t]` — the old ring's rows past the block's start, then
    // the block's last rows, each from the tile row of `X` that holds it. Only the sources a band reads are loaded.
    let ring = {
        let mut t = Template::new(target)?;
        let x = t.tensor(
            x_name,
            TensorRole::Input,
            rows,
            chan,
            dt,
            crate::spatial::passes::from_tensor_op::init_of(x_fill, rows * chan),
        );
        let cs = ring_tensor(&mut t, cs_name, TensorRole::State, cs_fill);
        #[derive(Clone, Copy, PartialEq)]
        enum Source {
            Ring,
            Rows(usize),
        }
        // Tap `t`'s source and where its row sits: ring row `rows + t`, or row `xr` of `X` at plane row `xr mod tile`.
        let sources: Vec<(Source, usize)> = (0..taps)
            .map(|tap| {
                let e = rows + tap;
                if e < taps {
                    (Source::Ring, e)
                } else {
                    (Source::Rows((e - taps) / tr), (e - taps) % tr)
                }
            })
            .collect();
        let mut pages: Vec<(Source, StageId, BufferId)> = Vec::new();
        for (source, _) in &sources {
            if pages.iter().any(|(s, _, _)| s == source) {
                continue;
            }
            let (load, buf) = match *source {
                Source::Ring => {
                    let b = t.buffer("in_ring", ring_tiles, dt);
                    (ring_page(&mut t, "load_ring", cs, b), b)
                }
                Source::Rows(tile_row) => {
                    let b = t.buffer(&format!("in_rows_{tile_row}"), blk, dt);
                    (
                        t.load_block_at(
                            &format!("load_rows_{tile_row}"),
                            x,
                            tile_row * cols_t,
                            vec![(LV_ITEM_X, blk)],
                            1,
                            blk,
                            cols_t,
                            b,
                        ),
                        b,
                    )
                }
            };
            pages.push((*source, load, buf));
        }
        let page_of = |source: Source| {
            pages
                .iter()
                .find(|(s, _, _)| *s == source)
                .map(|(_, _, b)| *b)
                .expect("every tap's source is loaded")
        };
        let (mut ins, mut maps, mut bands) = (Vec::new(), Vec::new(), Vec::new());
        for (tap, (source, row)) in sources.iter().enumerate() {
            ins.push(page_of(*source));
            // A matrix ring's tap `t` is plane row `t` of the output: read plane row `row` — `row − t` rows away. A flat
            // ring's tap `t` is the band `[t·chan, (t+1)·chan)` along the output row.
            let (row_stride, base) = match (matrix_ring, source) {
                (true, _) => (wc_w, (*row as i64 - tap as i64) * wc_w as i64),
                (false, Source::Ring) => (seg, (*row as i64 - tap as i64) * seg as i64),
                (false, Source::Rows(_)) => (wc_w, *row as i64 * wc_w as i64 - (tap * seg) as i64),
            };
            let map = if matrix_ring {
                rows_map(base, row_stride)
            } else {
                ElemAccess {
                    base,
                    terms: vec![(LV_GEN_COL, 1)],
                    indirect: None,
                }
            };
            maps.push(map);
            bands.push((
                if matrix_ring {
                    tap + 1
                } else {
                    (tap + 1) * seg
                },
                TensorBody::Operand(ins.len() - 1),
            ));
        }
        let (iters, body, out_w) = if matrix_ring {
            // Rows past the ring's taps are the tile's padding: they carry a zero.
            bands.push((tr, *konst(0.0)));
            (
                crate::spatial::passes::from_tensor_op::space(target, wc_w, None),
                banded_on(LV_GEN_ROW, bands),
                wc_w,
            )
        } else {
            (flat_space(taps * seg), banded(bands), wide(ring_t, target))
        };
        let out = t.buffer("ring", ring_tiles, dt);
        let s = t.stage(
            "ring_update",
            StageKind::Compute(ComputeOp::Generic {
                iters,
                ins: maps,
                out: ident_map(out_w),
                body,
            }),
            ins,
            vec![out],
        );
        for (_, load, buf) in &pages {
            t.edge(*load, *buf, s);
        }
        let st = if matrix_ring {
            t.store_block(
                "store_ring",
                cs,
                0,
                vec![(LV_ITEM_X, blk)],
                1,
                blk,
                cols_t,
                out,
            )
        } else {
            t.store_block("store_ring", cs, 0, Vec::new(), 1, ring_t, ring_t, out)
        };
        t.edge(s, out, st);
        // An item writes its column block of a matrix ring, or — flat — the whole ring.
        let ring_w = if matrix_ring { blk } else { ring_t };
        let items: Vec<WorkItem> = (0..n_b)
            .map(|b| WorkItem {
                id: WorkItemId::new(b),
                coord: (0, b),
                out: TileRect {
                    row0_t: 0,
                    col0_t: b * ring_w,
                    h_t: 1,
                    w_t: ring_w,
                },
            })
            .collect();
        let mut prog = t.finish(
            format!("conv_ring_rows_{rows}x{chan}_taps{taps}"),
            items,
            AccPolicy::F32,
            NumericsClaim::Exact,
            target,
        )?;
        prog.work.plan = Some(BlockPlan {
            grid: (1, n_b),
            ..tile_row_plan(ring_w, ring_w, dt)
        });
        prog
    };
    Ok(vec![outputs, ring])
}

/// ★ A CAUSAL CONV BLOCKS BY CHANNEL, AND NEEDS NO HALO. `y[c] = Σ_t W[t,c]·CS[t,c]` — every channel
/// reads only its OWN column of the ring, so channels are independent and a block of them is a complete
/// sub-problem. (The "causal" axis is the ring's TAP axis, which is already resident in full; the halo a
/// sliding-window convolution would need does not arise.)
///
/// The ring is `taps × chan` stored flat, so one channel tile's taps sit `chan/tile` TILES apart — a
/// strided `block_h = taps` load. The loaded page is a `32 × taps·32` plane whose tile index is the tap,
/// so inside the page the tap's stride is one TILE WIDTH, not `chan`. That last sentence is the whole
/// difference between this and the flat form, and getting it wrong reads another channel's history.
pub(super) fn blocked_causal_conv(
    spec: &CausalConvSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let (taps, chan) = (spec.taps, spec.chan);
    let tw = target.tile.cols;
    if chan % tw != 0 {
        return Err(SpatialReject::NotLowered(
            "a blocked causal conv whose channel count is not a whole number of tiles",
        ));
    }
    let chan_t = chan / tw;
    let ring_len = taps * chan;
    let precision = elem_precision(target)?;
    let (x_name, x_fill) = names.operand(0);
    let (w_name, w_fill) = names.operand(1);
    let (cs_name, cs_fill) = names.operand(2);

    // ── Section 1: the ring shift, one channel tile per item.
    let shift = {
        let mut t = Template::new(target)?;
        let cs = t.tensor(
            cs_name,
            TensorRole::Input,
            taps,
            chan,
            precision,
            crate::spatial::passes::from_tensor_op::init_of(cs_fill, ring_len),
        );
        let x = t.tensor(
            x_name,
            TensorRole::Input,
            1,
            chan,
            precision,
            crate::spatial::passes::from_tensor_op::init_of(x_fill, chan),
        );
        let outt = t.tensor(
            cs_name,
            TensorRole::Output,
            taps,
            chan,
            precision,
            TensorInit::Zero,
        );
        let cin = t.buffer("in_cs", 1, precision);
        let xin = t.buffer("in_x", 1, precision);
        let ob = t.buffer("ring", 1, precision);
        let lc = t.load("load_ring", cs, vec![(LV_ITEM_X, 1)], 1, cin);
        let lx = t.load("load_x", x, vec![(LV_ITEM_X, 1)], 1, xin);
        // Inside the page the tap is the tile, so one tap up is one tile width along the flat plane.
        let body = TensorBody::Select {
            cond: Pred::AxisLt(LV_GEN_ROW, PredRhs::Const(taps - 1)),
            then: Box::new(TensorBody::Operand(0)),
            otherwise: Box::new(TensorBody::Operand(1)),
        };
        let s = t.stage(
            "ring_shift",
            StageKind::Compute(ComputeOp::Generic {
                iters: head_space(taps, tw, None),
                ins: vec![map2(tw as i64, tw as i64, 1), map2(0, 0, 1)],
                out: map2(0, tw as i64, 1),
                body,
            }),
            vec![cin, xin],
            vec![ob],
        );
        let st = t.store("store_ring", outt, vec![(LV_ITEM_X, 1)], 1, ob);
        t.edge(lc, cin, s);
        t.edge(lx, xin, s);
        t.edge(s, ob, st);
        let items: Vec<WorkItem> = (0..chan_t)
            .map(|c| WorkItem {
                id: WorkItemId::new(c),
                coord: (0, c),
                out: TileRect {
                    row0_t: 0,
                    col0_t: c,
                    h_t: 1,
                    w_t: 1,
                },
            })
            .collect();
        t.finish(
            format!("conv_ring_{taps}x{chan}"),
            items,
            AccPolicy::F32,
            NumericsClaim::Exact,
            target,
        )?
    };

    // ── Section 2: the tap reduction against the updated ring, same blocking.
    let reduce = {
        let mut t = Template::new(target)?;
        let cs = t.tensor(
            cs_name,
            TensorRole::Input,
            taps,
            chan,
            precision,
            crate::spatial::passes::from_tensor_op::init_of(cs_fill, ring_len),
        );
        let wc = t.tensor(
            w_name,
            TensorRole::Param,
            taps,
            chan,
            precision,
            crate::spatial::passes::from_tensor_op::init_of(w_fill, ring_len),
        );
        let yt = t.tensor(
            &names.out,
            TensorRole::Output,
            1,
            chan,
            precision,
            TensorInit::Zero,
        );
        let tile_elems = target.tile.rows * target.tile.cols;
        let ones = t.tensor(
            &format!("{}__conv_scaler", names.out),
            TensorRole::Param,
            target.tile.rows,
            target.tile.cols,
            precision,
            TensorInit::FloatValues(vec![1.0f64.to_bits(); tile_elems]),
        );
        let cin = t.buffer("in_cs", 1, precision);
        let win = t.buffer("in_w", 1, precision);
        let oin = t.buffer("in_scaler", 1, precision);
        let ob = t.buffer("out", 1, precision);
        let lc = t.load("load_ring", cs, vec![(LV_ITEM_X, 1)], 1, cin);
        let lw = t.load("load_w", wc, vec![(LV_ITEM_X, 1)], 1, win);
        let lo = t.load("load_scaler", ones, vec![], 1, oin);
        let iters = vec![
            IterSpec {
                var: LV_GEN_ROW,
                extent: taps,
                kind: IterKind::Reduce {
                    pool: ReducePool::Sum,
                },
            },
            IterSpec {
                var: LV_GEN_COL,
                extent: tw,
                kind: IterKind::Parallel,
            },
        ];
        let s = t.stage(
            "tap_reduce",
            StageKind::Compute(ComputeOp::Generic {
                iters,
                ins: vec![map2(0, tw as i64, 1), map2(0, tw as i64, 1), const_map()],
                out: per_col_map(),
                body: *mul(op(0), op(1)),
            }),
            vec![cin, win, oin],
            vec![ob],
        );
        let st = t.store("store_y", yt, vec![(LV_ITEM_X, 1)], 1, ob);
        t.edge(lc, cin, s);
        t.edge(lw, win, s);
        t.edge(lo, oin, s);
        t.edge(s, ob, st);
        let items: Vec<WorkItem> = (0..chan_t)
            .map(|c| WorkItem {
                id: WorkItemId::new(c),
                coord: (0, c),
                out: TileRect {
                    row0_t: 0,
                    col0_t: c,
                    h_t: 1,
                    w_t: 1,
                },
            })
            .collect();
        t.finish(
            format!("conv_taps_{taps}x{chan}"),
            items,
            AccPolicy::F32,
            approx("causal convolution", precision),
            target,
        )?
    };
    Ok(vec![shift, reduce])
}
