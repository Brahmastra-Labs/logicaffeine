//! Carried state: decays and the gated delta rule, flat, blocked, widened and chunked.

use super::*;

/// `y[i] = (1 + e^(ar[i] + dt[i]))^(alog[i])` — Gated DeltaNet's forget gate.
///
/// ★ A REAL POWER NEEDS A REAL LOG, AND THAT IS WHY `SfpuKind::Ln` EXISTS. `base^rate` with a
/// data-dependent, non-integer, negative `rate` is `exp(rate · ln base)` and nothing else; no
/// composition of exp, reciprocal and rsqrt reaches it. The base is `1 + e^u > 1`, so the log is
/// always taken of a positive number.
pub fn from_decay(
    spec: &DecaySpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    if spec.rows > 1 {
        return dense_decay(spec, names, target);
    }
    let n = spec.n;
    let mut b = FlatBuild::new(target)?;
    let ar = b.input(names.operand(0).0, n, names.operand(0).1, target);
    let dt = b.input(names.operand(1).0, n, names.operand(1).1, target);
    let al = b.input(names.operand(2).0, n, names.operand(2).1, target);
    let cols_t = tiles_of(n, target);
    let out_buf = b.t.buffer("out", cols_t, b.precision);
    let w = wide(cols_t, target);
    let base = add(konst(1.0), unary(SfpuKind::Exp, add(op(0), op(1))));
    let body = *unary(SfpuKind::Exp, mul(op(2), unary(SfpuKind::Ln, base)));
    let slots = vec![
        Slot {
            buf: ar,
            map: at(0),
        },
        Slot {
            buf: dt,
            map: at(0),
        },
        Slot {
            buf: al,
            map: at(0),
        },
    ];
    let s = b.compute("decay", flat_space(n), slots, ident_map(w), body, out_buf);
    let claim = approx("decay", b.precision);
    b.finish(
        &names.out,
        n,
        vec![s],
        out_buf,
        format!("decay_{n}"),
        claim,
        target,
    )
}

/// [`from_decay`] over a block of rows — the `rows × heads` matrix a projection wrote, every row decayed by the same
/// per-head bias and rate. One item per tile of the matrix; the bias and rate tiles of its columns are read down
/// every row of it (a map with no row term — a broadcast is a missing term, not an operator).
pub(super) fn dense_decay(
    spec: &DecaySpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, heads) = (spec.rows, spec.n / spec.rows.max(1));
    if rows * heads != spec.n || heads == 0 {
        return Err(SpatialReject::NotLowered(
            "a block decay whose rows do not divide its elements",
        ));
    }
    let (tr, tw) = (target.tile.rows, target.tile.cols);
    let (rows_t, cols_t) = (rows.div_ceil(tr), heads.div_ceil(tw));
    let dt = elem_precision(target)?;
    let mut t = Template::new(target)?;
    let (ar_name, ar_fill) = names.operand(0);
    let (dt_name, dt_fill) = names.operand(1);
    let (al_name, al_fill) = names.operand(2);
    let ar = t.tensor(
        ar_name,
        TensorRole::Input,
        rows,
        heads,
        dt,
        crate::spatial::passes::from_tensor_op::init_of(ar_fill, spec.n),
    );
    let bias = t.tensor(
        dt_name,
        TensorRole::Input,
        1,
        heads,
        dt,
        crate::spatial::passes::from_tensor_op::init_of(dt_fill, heads),
    );
    let rate = t.tensor(
        al_name,
        TensorRole::Input,
        1,
        heads,
        dt,
        crate::spatial::passes::from_tensor_op::init_of(al_fill, heads),
    );
    let out_t = t.tensor(
        &names.out,
        TensorRole::Output,
        rows,
        heads,
        dt,
        TensorInit::Zero,
    );
    let (ar_in, bias_in, rate_in, out) = (
        t.buffer("in_ar", 1, dt),
        t.buffer("in_bias", 1, dt),
        t.buffer("in_rate", 1, dt),
        t.buffer("out", 1, dt),
    );
    let l_ar = t.load_block_at(
        "load_ar",
        ar,
        0,
        vec![(LV_ITEM_Y, cols_t), (LV_ITEM_X, 1)],
        1,
        1,
        cols_t,
        ar_in,
    );
    let l_bias = t.load_block_at("load_bias", bias, 0, vec![(LV_ITEM_X, 1)], 1, 1, 1, bias_in);
    let l_rate = t.load_block_at("load_rate", rate, 0, vec![(LV_ITEM_X, 1)], 1, 1, 1, rate_in);
    let down_rows = ElemAccess {
        base: 0,
        terms: vec![(LV_GEN_COL, 1)],
        indirect: None,
    };
    let base = add(konst(1.0), unary(SfpuKind::Exp, add(op(0), op(1))));
    let s = t.stage(
        "decay",
        StageKind::Compute(ComputeOp::Generic {
            iters: crate::spatial::passes::from_tensor_op::space(target, tw, None),
            ins: vec![ident_map(tw), down_rows.clone(), down_rows],
            out: ident_map(tw),
            body: *unary(SfpuKind::Exp, mul(op(2), unary(SfpuKind::Ln, base))),
        }),
        vec![ar_in, bias_in, rate_in],
        vec![out],
    );
    let st = t.store_block(
        "store_decay",
        out_t,
        0,
        vec![(LV_ITEM_Y, cols_t), (LV_ITEM_X, 1)],
        1,
        1,
        cols_t,
        out,
    );
    for (from, buf) in [(l_ar, ar_in), (l_bias, bias_in), (l_rate, rate_in)] {
        t.edge(from, buf, s);
    }
    t.edge(s, out, st);
    let items: Vec<WorkItem> = (0..rows_t)
        .flat_map(|r| (0..cols_t).map(move |c| (r, c)))
        .enumerate()
        .map(|(id, (r, c))| WorkItem {
            id: WorkItemId::new(id),
            coord: (r, c),
            out: TileRect {
                row0_t: r,
                col0_t: c,
                h_t: 1,
                w_t: 1,
            },
        })
        .collect();
    let mut prog = t.finish(
        format!("decay_{rows}x{heads}"),
        items,
        AccPolicy::for_result(dt),
        approx("decay", dt),
        target,
    )?;
    prog.work.plan = Some(BlockPlan {
        grid: (rows_t, cols_t),
        ..tile_row_plan(1, 1, dt)
    });
    Ok(prog)
}

/// The Gated DeltaNet step over `hv` value heads reading `hk` key heads at head dim `d`:
/// `u = S·k`, `S ← α·S + β·(v − α·u)·kᵀ`, `o = S·q / √d`, RMSNorm'd, scaled by `GN`, gated by `silu(z)`.
///
/// ★ THREE SECTIONS, BECAUSE THE MEMORY IS READ ONCE AND WRITTEN ONCE. Section 1 reads `S` to form the
/// prediction `u`; section 2 writes the new `S` from the old one and `u`; section 3 reads the new `S`
/// to form the output. Each boundary is a barrier, so "read the old memory, then write, then read the
/// new one" is enforced rather than hoped for, and `StaleStateRead` catches any scheduler that tries
/// otherwise. Fusing them would make a wrong answer indistinguishable from a right one.
///
/// ★ A BLOCK OF ROWS IS THE STEP, ROW AFTER ROW. The delta rule is a recurrence — row `r` reads the memory row `r − 1`
/// wrote — so a block of tokens is its rows' three sections in turn against the one memory, each reading its row of the
/// operands in place and writing its row of the output. `dense` names the block's tables the program lays out as
/// matrices; the output is written in its layout.
pub fn from_gated_delta(
    spec: &GatedDeltaSpec,
    names: &StageNames,
    dense: &BTreeSet<String>,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let (hv, hk, d, rows) = (spec.hv, spec.hk, spec.d, spec.rows);
    if hv == 0 || hk == 0 || d == 0 || rows == 0 {
        return Err(SpatialReject::NotLowered(
            "a gated delta step with no heads, no head dimension or no rows",
        ));
    }
    // The flat form keeps the whole `hv·d²` memory as one row; a memory past one accumulator page of its row is blocked
    // by head: a head of whole tiles as it is, any other widened to whole tiles.
    if row_needs_blocking(hv * d * d, target) {
        let carried = names.operand(6).0;
        return if d % target.tile.cols == 0 {
            blocked_gated_delta(spec, names, dense, d, &|_| carried.to_string(), target)
        } else {
            widened_gated_delta(spec, names, target)
        };
    }
    let mut sections = Vec::new();
    for r in 0..rows {
        sections.extend(flat_gated_delta_row(spec, names, r, target)?);
        // A row of the flat output shares its tiles with the rows beside it, so each row's output is written into
        // the table by the row write that merges one row into its tiles.
        if rows > 1 {
            let at = RowIndex::Literal(r as i64 + 1);
            let write = SetRowSpec {
                table: spec.out,
                index: at,
                source: spec.out,
                rows,
                cols: hv * d,
                count: 1,
            };
            let written = StageNames {
                out: names.out.clone(),
                ins: vec![names.out.clone(), format!("{}__o{r}", names.out)],
                fills: vec![Fill::ConstF(0f64.to_bits()), Fill::Unknown],
            };
            sections.push(from_set_row(
                &write,
                &written,
                true,
                dense.contains(&names.out),
                None,
                target,
            )?);
        }
    }
    Ok(sections)
}

/// Row `r` of [`from_gated_delta`]'s flat form — its prediction, its memory update and its output — reading the row's
/// operands in place: row `r` of `Q` and `K` from element `r·hk·d`, of `V` and `Z` from `r·hv·d`, of `B` and `A` from
/// `r·hv`. A one-row step is row 0 of itself.
pub(super) fn flat_gated_delta_row(
    spec: &GatedDeltaSpec,
    names: &StageNames,
    r: usize,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let (hv, hk, d, rows) = (spec.hv, spec.hk, spec.d, spec.rows);
    let s_len = hv * d * d;
    let out_len = hv * d;
    let (qk_at, v_at, h_at) = ((r * hk * d) as i64, (r * out_len) as i64, (r * hv) as i64);
    let tag = if rows == 1 {
        String::new()
    } else {
        format!("_row{r}")
    };
    let o_name = if rows == 1 {
        names.out.clone()
    } else {
        format!("{}__o{r}", names.out)
    };
    let (q_name, q_fill) = names.operand(0);
    let (k_name, k_fill) = names.operand(1);
    let (v_name, v_fill) = names.operand(2);
    let (z_name, z_fill) = names.operand(3);
    let (b_name, b_fill) = names.operand(4);
    let (a_name, a_fill) = names.operand(5);
    let (st_name, st_fill) = names.operand(6);
    let (gn_name, gn_fill) = names.operand(7);
    let u_name = if rows == 1 {
        format!("{}_u", names.out)
    } else {
        format!("{}_u{r}", names.out)
    };

    // ── Section 1: `u[h·d + a] = Σ_b S[h·d² + a·d + b]·k[(h mod hk)·d + b]`, the prediction from the
    // memory BEFORE it is updated. The reduced axis is the column `b`; the head and row `a` ride the
    // flat output column through per-head bands.
    let predict = {
        let mut f = FlatBuild::new(target)?;
        let st = f.input(st_name, s_len, st_fill, target);
        let k = f.input(k_name, rows * hk * d, k_fill, target);
        let cols_t = tiles_of(out_len, target);
        let out_buf = f.t.buffer("u", cols_t, f.precision);
        let tile_elems = target.tile.rows * target.tile.cols;
        let ones = f.t.tensor(
            "gd_scaler",
            TensorRole::Param,
            target.tile.rows,
            target.tile.cols,
            f.precision,
            TensorInit::FloatValues(vec![1.0f64.to_bits(); tile_elems]),
        );
        let ones_in = f.t.buffer("in_scaler", 1, f.precision);
        let l = f.t.load("load_scaler", ones, vec![], 1, ones_in);
        f.loads.push((l, ones_in));
        // ★ THE FOLD IS ON THE ROW, THE ANSWER IS ON THE COLUMN. Folding the row axis writes the
        // result DOWN every row of its column, so row 0 of column `c` holds it — which is exactly the
        // flat `1 × n` layout the next section reads, and it spans as many tiles as the answer needs.
        // Folding the column axis instead leaves one value per ROW, and a flat reader would see only
        // the first of them: the same shape, a completely different answer.
        let iters = vec![
            IterSpec {
                var: LV_GEN_ROW,
                extent: d,
                kind: IterKind::Reduce {
                    pool: ReducePool::Sum,
                },
            },
            IterSpec {
                var: LV_GEN_COL,
                extent: out_len,
                kind: IterKind::Parallel,
            },
        ];
        let mut slots = vec![Slot {
            buf: ones_in,
            map: const_map(),
        }];
        let mut bands = Vec::new();
        for h in 0..hv {
            // `S[h·d² + a·d + b]` with `a = c − h·d` is `c·d + b` — the per-head term cancels, so the
            // memory is read by ONE map for every head.
            slots.push(Slot {
                buf: st,
                map: ElemAccess {
                    base: 0,
                    terms: vec![(LV_GEN_COL, d as i64), (LV_GEN_ROW, 1)],
                    indirect: None,
                },
            });
            let s_slot = slots.len() - 1;
            // `k[(h mod hk)·d + b]` — constant along the output column, affine in the folded row.
            slots.push(Slot {
                buf: k,
                map: ElemAccess {
                    base: ((h % hk) * d) as i64 + qk_at,
                    terms: vec![(LV_GEN_ROW, 1)],
                    indirect: None,
                },
            });
            let k_slot = slots.len() - 1;
            bands.push(((h + 1) * d, *mul(op(s_slot), op(k_slot))));
        }
        let body = banded(bands);
        let s = f.compute("gd_predict", iters, slots, per_col_map(), body, out_buf);
        let claim = approx("gated-delta prediction", f.precision);
        f.finish(
            &u_name,
            out_len,
            vec![s],
            out_buf,
            format!("gd_predict_{hv}x{d}{tag}"),
            claim,
            target,
        )?
    };

    // ── Section 2: the rank-1 memory update `S[h, a, c] = α_h·S[h, a, c] + coef[h, a]·k[h, c]` with
    // `coef[h, a] = β_h·(v[h·d + a] − α_h·u[h·d + a])`.
    let update = {
        let mut f = FlatBuild::new(target)?;
        let st = f.input(st_name, s_len, st_fill, target);
        let u = f.input(&u_name, out_len, &Fill::Unknown, target);
        let v = f.input(v_name, rows * out_len, v_fill, target);
        let k = f.input(k_name, rows * hk * d, k_fill, target);
        let al = f.input(a_name, rows * hv, a_fill, target);
        let be = f.input(b_name, rows * hv, b_fill, target);
        let cols_t = tiles_of(s_len, target);
        let out_buf = f.t.buffer("state", cols_t, f.precision);
        let ow = wide(cols_t, target);
        // The flat state column is `c = h·d² + a·d + j`. Inside head `h`'s band, `a = (c − h·d²) / d`
        // and `j = (c − h·d²) % d` — NEITHER is affine, so the head's band is split further into `d`
        // ROW bands, one per `a`, inside which both are affine again. `d` is small (128 in Qwen3.8's
        // real config, 4 in the tiny one) and the bands are constants, so this is a wide body rather
        // than a deep one.
        let mut slots = vec![Slot {
            buf: st,
            map: at(0),
        }];
        let st_slot = 0usize;
        let mut bands = Vec::new();
        for h in 0..hv {
            let hb = (h * d * d) as i64;
            slots.push(Slot {
                buf: al,
                map: fixed(h as i64 + h_at),
            });
            let a_slot = slots.len() - 1;
            slots.push(Slot {
                buf: be,
                map: fixed(h as i64 + h_at),
            });
            let b_slot = slots.len() - 1;
            for a in 0..d {
                let row_base = hb + (a * d) as i64;
                // v[h·d + a] and u[h·d + a] are fixed within this row band.
                slots.push(Slot {
                    buf: v,
                    map: fixed((h * d + a) as i64 + v_at),
                });
                let v_slot = slots.len() - 1;
                slots.push(Slot {
                    buf: u,
                    map: fixed((h * d + a) as i64),
                });
                let u_slot = slots.len() - 1;
                // k[(h mod hk)·d + j] with j = c − row_base.
                slots.push(Slot {
                    buf: k,
                    map: at(((h % hk) * d) as i64 + qk_at - row_base),
                });
                let k_slot = slots.len() - 1;
                let coef = mul(op(b_slot), sub(op(v_slot), mul(op(a_slot), op(u_slot))));
                let value = add(mul(op(a_slot), op(st_slot)), mul(coef, op(k_slot)));
                bands.push((row_base as usize + d, *value));
            }
        }
        let s = f.compute(
            "gd_update",
            flat_space(s_len),
            slots,
            ident_map(ow),
            banded(bands),
            out_buf,
        );
        let claim = approx("gated-delta memory update", f.precision);
        f.finish(
            st_name,
            s_len,
            vec![s],
            out_buf,
            format!("gd_update_{hv}x{d}{tag}"),
            claim,
            target,
        )?
    };

    // ── Section 3: `o = S_new·q / √d`, RMSNorm over the head, scaled by `GN`, gated by `silu(z)`.
    let read = {
        let mut f = FlatBuild::new(target)?;
        let st = f.input(st_name, s_len, st_fill, target);
        let q = f.input(q_name, rows * hk * d, q_fill, target);
        let z = f.input(z_name, rows * out_len, z_fill, target);
        let gn = f.input(gn_name, d, gn_fill, target);
        let cols_t = tiles_of(out_len, target);
        let out_buf = f.t.buffer("out", cols_t, f.precision);
        let ow = wide(cols_t, target);
        let tile_elems = target.tile.rows * target.tile.cols;
        let ones = f.t.tensor(
            "gd_scaler",
            TensorRole::Param,
            target.tile.rows,
            target.tile.cols,
            f.precision,
            TensorInit::FloatValues(vec![1.0f64.to_bits(); tile_elems]),
        );
        let ones_in = f.t.buffer("in_scaler", 1, f.precision);
        let l = f.t.load("load_scaler", ones, vec![], 1, ones_in);
        f.loads.push((l, ones_in));
        let ovec = f.scratch("gd_o", cols_t);
        let iters = vec![
            IterSpec {
                var: LV_GEN_ROW,
                extent: d,
                kind: IterKind::Reduce {
                    pool: ReducePool::Sum,
                },
            },
            IterSpec {
                var: LV_GEN_COL,
                extent: out_len,
                kind: IterKind::Parallel,
            },
        ];
        let inv_sqrt_d = 1.0f64 / (d as f64).sqrt();
        let mut slots = vec![Slot {
            buf: ones_in,
            map: const_map(),
        }];
        let mut bands = Vec::new();
        for h in 0..hv {
            slots.push(Slot {
                buf: st,
                map: ElemAccess {
                    base: 0,
                    terms: vec![(LV_GEN_COL, d as i64), (LV_GEN_ROW, 1)],
                    indirect: None,
                },
            });
            let s_slot = slots.len() - 1;
            slots.push(Slot {
                buf: q,
                map: ElemAccess {
                    base: ((h % hk) * d) as i64 + qk_at,
                    terms: vec![(LV_GEN_ROW, 1)],
                    indirect: None,
                },
            });
            let q_slot = slots.len() - 1;
            bands.push((
                (h + 1) * d,
                *mul(mul(op(s_slot), op(q_slot)), konst(inv_sqrt_d)),
            ));
        }
        let s1 = f.compute("gd_read", iters, slots, per_col_map(), banded(bands), ovec);
        // The per-head RMSNorm of the ungated output, then the weight and the gate.
        let (mut chain, scales) = row_scale_chain(
            &mut f,
            ovec,
            hv,
            d,
            SfpuKind::Rsqrt {
                eps_nano: eps_nano(spec.eps)?,
                over: d,
            },
            target,
        );
        chain.insert(0, s1);
        let mut slots = vec![
            Slot {
                buf: ovec,
                map: at(0),
            },
            Slot {
                buf: z,
                map: at(v_at),
            },
        ];
        let (o_slot, z_slot) = (0usize, 1usize);
        let mut bands = Vec::new();
        for h in 0..hv {
            let head_base = (h * d) as i64;
            slots.push(scales.of(h));
            let sc = slots.len() - 1;
            slots.push(Slot {
                buf: gn,
                map: at(-head_base),
            });
            let g = slots.len() - 1;
            bands.push((
                (h + 1) * d,
                *mul(mul(op(g), mul(op(o_slot), op(sc))), delta_gate(spec.gate, op(z_slot))),
            ));
        }
        let s2 = f.compute(
            "gd_gate",
            flat_space(out_len),
            slots,
            ident_map(ow),
            banded(bands),
            out_buf,
        );
        chain.push(s2);
        let claim = approx("gated-delta output", f.precision);
        f.finish(
            &o_name,
            out_len,
            chain,
            out_buf,
            format!("gd_read_{hv}x{d}{tag}"),
            claim,
            target,
        )?
    };
    Ok(vec![predict, update, read])
}

/// ★ GATED DELTANET AT REAL WIDTH — the head becomes the WORK ITEM.
///
/// The flat form keeps the whole `hv·d²` memory resident and distinguishes heads with a `banded` body:
/// at Qwen3.8's real geometry that is a 786,432-element buffer and 6,144 bands. Neither survives contact
/// with a site.
///
/// ★ WHY THE HEAD HAS TO RIDE THE ITEM'S COORDINATE. A `Generic`'s maps and body cannot see WHERE the
/// work item sits — `offset_at` substitutes the two iteration axes and nothing else, so every other loop
/// variable reads as zero. That is precisely why the flat forms use one enormous item: the body can only
/// tell heads apart if every head is inside the page. A LOAD, by contrast, addresses tiles through
/// `TileAddr.steps`, which DO read the item's coordinate — so the head must be selected when the data is
/// fetched, not when it is used.
///
/// ★ AND WHY THE COORDINATE IS `(kh, g)` RATHER THAN `(h, ·)`. The memory is indexed by the v-head `h`
/// while `k` and `q` are indexed by `h mod hk` — 48 v-heads sharing 16 k-heads — and `h mod hk` is not
/// affine in `h`, so no step can express it. Splitting the head as `h = g·hk + kh` and carrying BOTH
/// halves in the item's coordinate makes each index affine in its own axis: `k` steps by `kh`, the memory
/// steps by both. The non-affine operation disappears because it was only ever a decomposition.
///
/// The per-head scalars `α` and `β` are the one thing left that a load cannot reach — element `h` of a
/// 48-long vector lives at tile `h/32`, and that is not affine either — so they are broadcast once, ahead
/// of the step, into one tile per head.
///
/// `head` is the width the step's `1/√d` and its norm read: `spec.d`, or a head's own width where
/// [`widened_gated_delta`] widened it to `spec.d`. `state(i)` names the memory after `i` rows (or chunks): the carried
/// memory's one name for every `i`, updated in place, or a name per version where the memory is the op's own.
pub(super) fn blocked_gated_delta(
    spec: &GatedDeltaSpec,
    names: &StageNames,
    dense: &BTreeSet<String>,
    head: usize,
    state: &dyn Fn(usize) -> String,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let (hv, hk, d, rows) = (spec.hv, spec.hk, spec.d, spec.rows);
    let (tr, tw) = (target.tile.rows, target.tile.cols);
    if d % tw != 0 || hv % hk != 0 {
        return Err(SpatialReject::NotLowered("a blocked gated delta needs a head dimension in whole tiles and v-heads a multiple of k-heads"));
    }
    let nb = d / tw; // tile-rows per head's memory, and tiles per head's vectors
    if takes_chunks(rows, tr) {
        return chunked_gated_delta(spec, names, dense, head, state, target);
    }
    // ★ THIS ONCE DID NOT FINISH, AND THE REASON WAS NOT HERE. Blocking gated delta by head produces a
    // layer of ~14,800 stages, and certifying it ran for sixteen minutes at 13 GB. Every candidate cause
    // was wrong until it was measured: the sections certify in MILLISECONDS, `place` clones 864 stages in
    // 0.6ms, and `compose` joins all 45 sections in 5.1s. The cost was one line in `validate_schedule`
    // asking `per_stage` for every store stage — `stores × ops`, with an allocation per call. Built as a
    // set in one pass it is linear, and the whole layer now lowers in ~77s.
    // ★ THE A-BLOCK IS A STAGE, NOT AN ITEM. A head's memory is an `nb × nb` tile square — sixteen tiles against an
    // eight-tile accumulator — so no one pass writes it. Each a-block (a tile row of the square) is its own chain of
    // stages inside the head's item — its prediction, its look `S·q`, its update, its store — each page `nb` tiles
    // wide, so the accumulator holds one a-block at a time while the item stays the head: `hv` items, not `hv·nb`.
    let precision = elem_precision(target)?;
    let (q_name, q_fill) = names.operand(0);
    let (k_name, k_fill) = names.operand(1);
    let (v_name, v_fill) = names.operand(2);
    let (z_name, z_fill) = names.operand(3);
    let (b_name, b_fill) = names.operand(4);
    let (a_name, a_fill) = names.operand(5);
    let st_fill = names.operand(6).1;
    let (gn_name, gn_fill) = names.operand(7);
    let out_len = hv * d;
    let s_rows = hv * d;
    // Tiles one row spans: of `Q` and `K`, and of `V`, `Z` and the output.
    let (qk_t, v_t) = (hk * nb, hv * nb);
    let a_exp = format!("{}__alpha_t", names.out);
    let b_exp = format!("{}__beta_t", names.out);
    // A block's rows each take the step in turn, so each has its own section.
    let tag = |r: usize| {
        if rows == 1 {
            String::new()
        } else {
            format!("_row{r}")
        }
    };

    // One item per v-head, coordinate `(kh, g)` — what rebuilds `h` for the loads. The item's RECT is its strip of
    // the output, the step's result; the memory's `nb × nb` square it also writes is covered by the tiles its stores
    // address.
    let head_items = |rect: &dyn Fn(usize) -> TileRect| -> Vec<WorkItem> {
        (0..hv)
            .map(|h| WorkItem {
                id: WorkItemId::new(h),
                coord: (h % hk, h / hk),
                out: rect(h),
            })
            .collect()
    };
    // Tile steps that rebuild `h = g·hk + kh` from the coordinate.
    let head_steps = |per_head: usize| vec![(LV_ITEM_Y, per_head), (LV_ITEM_X, hk * per_head)];

    // ★ A BROADCAST IS A MATMUL AGAINST A SELECTION MATRIX.
    //
    // `α` and `β` are per-head SCALARS, and a scalar is the one thing a blocked section cannot reach:
    // element `h` of an `hv`-long vector lives at tile `h/32` offset `h%32`, a LOAD selects only tiles,
    // and a MAP cannot see which item it is running for (`offset_at` substitutes the two iteration axes
    // and nothing else). Every attempt to broadcast them with a `Generic` relocates that problem into the
    // broadcast's own body.
    //
    // But `alpha_t[h·tw + j] = α[h]` is exactly `α · SEL` for the constant `hv × (hv·tw)` matrix with
    // `SEL[h, c] = 1 iff c/tw == h` — and a matmul is the one operation this compiler already streams
    // over a long output, blocking it across items and sites without being asked. So the scalar becomes
    // ONE TILE PER HEAD, which a tile address CAN reach, and no gather is needed at all. The selection
    // matrix is a compile-time fill; the work is `hv·tw` MACs per row, and a block's rows are ONE matmul —
    // row `r`'s tiles are tile row `r / tile`, its values plane row `r mod tile` of each.
    let sel_fill = || -> Fill {
        Fill::Overrides {
            base: Box::new(Fill::Const(0)),
            sets: (0..hv)
                .flat_map(|h| (0..tw).map(move |j| (h * hv * tw + h * tw + j, 1)))
                .collect(),
        }
    };
    let broadcast =
        |src: &str, fill: &Fill, dst: &str, sel: &str| -> Result<SpatialProgram, SpatialReject> {
            let spec = crate::tensor_recognize::MatmulSpec {
                m: rows,
                n: hv * tw,
                k: hv,
                a: Default::default(),
                b: Default::default(),
                c: Default::default(),
            };
            let names = MatmulNames {
                a: src.to_string(),
                b: sel.to_string(),
                c: dst.to_string(),
            };
            crate::spatial::passes::from_tensor_op::from_matmul(
                &spec,
                &names,
                (fill, &sel_fill()),
                (None, None),
                NumericsPolicy::Exact,
                target,
            )
        };
    let alpha_t = broadcast(a_name, a_fill, &a_exp, &format!("{}__sel_a", names.out))?;
    let beta_t = broadcast(b_name, b_fill, &b_exp, &format!("{}__sel_b", names.out))?;

    // ── Row `r`'s step, ONE PASS OVER EACH HEAD'S MEMORY.
    //
    // ★ EVERY TERM OF THE STEP IS LOCAL TO AN A-BLOCK. Row `a` of the prediction `u = S·k`, of the update
    // `S'[a, ·] = α·S[a, ·] + β·(v[a] − α·u[a])·k` and of the output `(S'·q)[a]` read only row `a` of the memory, and the
    // output needs no second read of it: `S'·q = α·(S·q) + β·(v − α·u)·(k·q)`, `k·q` one number per head. So the item —
    // the head — loads each a-block of its memory ONCE, forms `u` and `w = S·q` from it, writes the updated a-block
    // back, and assembles the output from `w`, `u` and `k·q`: the memory is read once and written once a step, where a
    // prediction, an update and a read section each moved it. The output is the section's result; the memory's a-blocks
    // are a second tensor, each store covering the tiles it addresses.
    let o_dense = rows > 1 && dense.contains(&names.out);
    let inv_sqrt_d = 1.0f64 / (head as f64).sqrt();
    // `pages`: how many pages each once-an-item buffer holds (each a-block's store, the norm weight, the gated output).
    let step = |r: usize, pages: usize| -> Result<SpatialProgram, SpatialReject> {
        let mut t = Template::new(target)?;
        let acc = AccPolicy::F32.interm_precision();
        // The output first: the step's result, whose strips are the items' rectangles.
        let y = match (rows, o_dense) {
            (1, _) => t.tensor(
                &names.out,
                TensorRole::Output,
                1,
                out_len,
                precision,
                TensorInit::Zero,
            ),
            (_, false) => t.tensor(
                &names.out,
                TensorRole::State,
                1,
                rows * out_len,
                precision,
                TensorInit::Zero,
            ),
            (_, true) => t.tensor(
                &names.out,
                TensorRole::State,
                rows,
                out_len,
                precision,
                TensorInit::Zero,
            ),
        };
        let st = t.tensor(
            &state(r),
            TensorRole::Input,
            s_rows,
            d,
            precision,
            crate::spatial::passes::from_tensor_op::init_of(st_fill, s_rows * d),
        );
        let stout = t.tensor(
            &state(r + 1),
            TensorRole::State,
            s_rows,
            d,
            precision,
            TensorInit::Zero,
        );
        let q = t.tensor(
            q_name,
            TensorRole::Input,
            1,
            rows * hk * d,
            precision,
            crate::spatial::passes::from_tensor_op::init_of(q_fill, rows * hk * d),
        );
        let k = t.tensor(
            k_name,
            TensorRole::Input,
            1,
            rows * hk * d,
            precision,
            crate::spatial::passes::from_tensor_op::init_of(k_fill, rows * hk * d),
        );
        let v = t.tensor(
            v_name,
            TensorRole::Input,
            1,
            rows * out_len,
            precision,
            crate::spatial::passes::from_tensor_op::init_of(v_fill, rows * out_len),
        );
        let z = t.tensor(
            z_name,
            TensorRole::Input,
            1,
            rows * out_len,
            precision,
            crate::spatial::passes::from_tensor_op::init_of(z_fill, rows * out_len),
        );
        let gn = t.tensor(
            gn_name,
            TensorRole::Param,
            1,
            d,
            precision,
            crate::spatial::passes::from_tensor_op::init_of(gn_fill, d),
        );
        let alpha = t.tensor(
            &a_exp,
            TensorRole::Input,
            rows,
            hv * tw,
            precision,
            TensorInit::External,
        );
        let beta = t.tensor(
            &b_exp,
            TensorRole::Input,
            rows,
            hv * tw,
            precision,
            TensorInit::External,
        );
        let ones = t.tensor(
            &format!("{}__gd_scaler", names.out),
            TensorRole::Param,
            target.tile.rows,
            tw,
            precision,
            TensorInit::FloatValues(vec![1.0f64.to_bits(); target.tile.rows * tw]),
        );
        // The head's operands, each loaded once: `k` and `q` at its k-head, `v` at its v-head, `α` and `β` at plane row
        // `r mod tile` of the head's tile.
        let kin = t.buffer_deep("in_k", 1, nb, precision);
        let qin = t.buffer_deep("in_q", 1, nb, precision);
        let vin = t.buffer_deep("in_v", 1, nb, precision);
        let ain = t.buffer_deep("in_a", 1, 1, precision);
        let bin = t.buffer_deep("in_b", 1, 1, precision);
        let sc_in = t.buffer_deep("in_scaler", 1, 1, precision);
        let lk = t.load_block_at("load_k", k, r * qk_t, vec![(LV_ITEM_Y, nb)], 1, nb, nb, kin);
        let lq = t.load_block_at("load_q", q, r * qk_t, vec![(LV_ITEM_Y, nb)], 1, nb, nb, qin);
        let lv = t.load_block_at("load_v", v, r * v_t, head_steps(nb), 1, nb, nb, vin);
        let la = t.load_block_at("load_a", alpha, (r / tr) * hv, head_steps(1), 1, 1, 1, ain);
        let lb = t.load_block_at("load_b", beta, (r / tr) * hv, head_steps(1), 1, 1, 1, bin);
        let l_sc = t.load("load_scaler", ones, vec![], 1, sc_in);
        let gin = t.buffer_deep("in_gn", pages, nb, precision);
        let lg = t.load("load_gn", gn, vec![], nb, gin);
        let scalar = fixed(((r % tr) * tw) as i64);
        let column = ElemAccess {
            base: 0,
            terms: vec![(LV_GEN_COL, 1)],
            indirect: None,
        };
        let down = ElemAccess {
            base: 0,
            terms: vec![(LV_GEN_ROW, 1)],
            indirect: None,
        };
        // Fold `b` (the ROW), keep `a` (the COLUMN): `Σ_b S[a, b]·x[b]` lands at column `a` of the one-tile page.
        let matvec = |t: &mut Template, name: &str, s: BufferId, x: BufferId, out: BufferId| {
            let iters = vec![
                IterSpec {
                    var: LV_GEN_ROW,
                    extent: d,
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
            let ins = vec![
                ElemAccess {
                    base: 0,
                    terms: vec![(LV_GEN_COL, d as i64), (LV_GEN_ROW, 1)],
                    indirect: None,
                },
                ElemAccess {
                    base: 0,
                    terms: vec![(LV_GEN_ROW, 1)],
                    indirect: None,
                },
                const_map(),
            ];
            t.stage(
                name,
                StageKind::Compute(ComputeOp::Generic {
                    iters,
                    ins,
                    out: per_col_map(),
                    body: *mul(op(0), op(1)),
                }),
                vec![s, x, sc_in],
                vec![out],
            )
        };
        let mut parts = Vec::with_capacity(nb);
        for j in 0..nb {
            let sin = t.buffer_deep(&format!("in_s{j}"), 1, nb, precision);
            let ls = t.load_block_at(
                &format!("load_s{j}"),
                st,
                j * nb,
                head_steps(nb * nb),
                1,
                nb,
                nb,
                sin,
            );
            let u = t.scratch_deep(&format!("u{j}"), 1, 1, acc);
            let w = t.scratch_deep(&format!("w{j}"), 1, 1, acc);
            let s_u = matvec(&mut t, &format!("gd_predict{j}"), sin, kin, u);
            matvec(&mut t, &format!("gd_look{j}"), sin, qin, w);
            // `S'[a, b] = α·S[a, b] + β·(v[a] − α·u[a])·k[b]` — 0:S 1:k 2:α 3:β 4:v 5:u.
            let sout = t.buffer_deep(&format!("out_s{j}"), pages, nb, precision);
            let coef = mul(op(3), sub(op(4), mul(op(2), op(5))));
            let s_upd = t.stage(
                &format!("gd_update{j}"),
                StageKind::Compute(ComputeOp::Generic {
                    iters: vec![
                        IterSpec {
                            var: LV_GEN_ROW,
                            extent: tr,
                            kind: IterKind::Parallel,
                        },
                        IterSpec {
                            var: LV_GEN_COL,
                            extent: d,
                            kind: IterKind::Parallel,
                        },
                    ],
                    ins: vec![
                        map2(0, d as i64, 1),
                        column.clone(),
                        scalar.clone(),
                        scalar.clone(),
                        ElemAccess {
                            base: (j * tw) as i64,
                            terms: vec![(LV_GEN_ROW, 1)],
                            indirect: None,
                        },
                        down.clone(),
                    ],
                    out: map2(0, d as i64, 1),
                    body: *add(mul(op(2), op(0)), mul(coef, op(1))),
                }),
                vec![sin, kin, ain, bin, vin, u],
                vec![sout],
            );
            let store = t.store_block(
                &format!("store_s{j}"),
                stout,
                j * nb,
                head_steps(nb * nb),
                1,
                nb,
                nb,
                sout,
            );
            // Each a-block's write depends on its read: the update reads the prediction formed from the loaded block.
            t.edge(ls, sin, s_u);
            t.edge(s_u, u, s_upd);
            t.edge(s_upd, sout, store);
            parts.push((u, w, j));
        }
        // `k·q`, one number per head, across row 0 of its page.
        let one_w = wide(1, target);
        let kq = t.scratch("kq", 1, acc);
        let s_kq = t.stage(
            "gd_kq",
            StageKind::Compute(ComputeOp::Generic {
                iters: head_space(1, d, Some(ReducePool::Sum)),
                ins: vec![column.clone(), column.clone(), const_map()],
                out: per_row_map(one_w),
                body: *mul(op(0), op(1)),
            }),
            vec![kin, qin, sc_in],
            vec![kq],
        );
        // `o[c] = (α·w[a] + β·(v[c] − α·u[a])·(k·q)) / √d`, `a = c − j·tile` in a-block `j`'s band.
        let ovec = t.scratch("gd_o", nb, acc);
        let (mut o_ins, mut o_maps, mut bands) = (
            vec![ain, bin, vin, kq],
            vec![scalar.clone(), scalar.clone(), column.clone(), fixed(0)],
            Vec::new(),
        );
        for (u, w, j) in &parts {
            let shift = ElemAccess {
                base: -((j * tw) as i64),
                terms: vec![(LV_GEN_COL, 1)],
                indirect: None,
            };
            o_ins.extend([*u, *w]);
            o_maps.extend([shift.clone(), shift]);
            let (us, ws) = (o_ins.len() - 2, o_ins.len() - 1);
            let value = mul(
                add(
                    mul(op(0), op(ws)),
                    mul(mul(op(1), sub(op(2), mul(op(0), op(us)))), op(3)),
                ),
                konst(inv_sqrt_d),
            );
            bands.push(((j + 1) * tw, *value));
        }
        let s_o = t.stage(
            "gd_o",
            StageKind::Compute(ComputeOp::Generic {
                iters: flat_space(d),
                ins: o_maps,
                out: ident_map(wide(nb, target)),
                body: banded(bands),
            }),
            o_ins,
            vec![ovec],
        );
        for (l, b, to) in [
            (lk, kin, s_kq),
            (lq, qin, s_kq),
            (lv, vin, s_o),
            (la, ain, s_o),
            (lb, bin, s_o),
            (l_sc, sc_in, s_kq),
        ] {
            t.edge(l, b, to);
        }
        // The head's RMSNorm, then the weight and the gate.
        let w = wide(nb, target);
        let sq = t.scratch("o_sq", nb, acc);
        let s_sq = t.stage(
            "square",
            StageKind::Compute(ComputeOp::Generic {
                iters: flat_space(w),
                ins: vec![ident_map(w), ident_map(w)],
                out: ident_map(w),
                body: *mul(op(0), op(1)),
            }),
            vec![ovec, ovec],
            vec![sq],
        );
        let ssum = t.scratch("o_sum", 1, acc);
        let s_sum = t.stage(
            "o_sum",
            StageKind::Compute(ComputeOp::Generic {
                iters: head_space(1, d, Some(ReducePool::Sum)),
                ins: vec![map2(0, w as i64, 1), const_map()],
                out: per_row_map(one_w),
                body: TensorBody::Operand(0),
            }),
            vec![sq, sc_in],
            vec![ssum],
        );
        let inv = t.scratch("o_inv", 1, acc);
        let s_inv = t.stage(
            "o_rsqrt",
            StageKind::Compute(ComputeOp::Generic {
                iters: head_space(1, tw, None),
                ins: vec![per_row_map(one_w)],
                out: per_row_map(one_w),
                body: TensorBody::Unary {
                    kind: SfpuKind::Rsqrt {
                        eps_nano: eps_nano(spec.eps)?,
                        over: head,
                    },
                    arg: op(0),
                },
            }),
            vec![ssum],
            vec![inv],
        );
        let ob = t.buffer_deep("out", pages, nb, precision);
        let mut zins = Vec::new();
        for j in 0..nb {
            let zb = t.buffer_deep(&format!("in_z{j}"), 1, 1, precision);
            let lz = t.load_block_at(
                &format!("load_z{j}"),
                z,
                r * v_t + j,
                head_steps(nb),
                1,
                1,
                1,
                zb,
            );
            zins.push((lz, zb, j));
        }
        // `y = o · inv · GN · silu(z)` — the gate read through per-a-block tiles, banded back together.
        let mut ins = vec![ovec, inv, gin];
        let mut maps = vec![ident_map(w), fixed(0), ident_map(w)];
        let mut bands = Vec::new();
        for (_, zb, j) in &zins {
            ins.push(*zb);
            maps.push(ElemAccess {
                base: -((j * tw) as i64),
                terms: vec![(LV_GEN_COL, 1)],
                indirect: None,
            });
            let zslot = ins.len() - 1;
            let gated = mul(mul(mul(op(0), op(1)), op(2)), delta_gate(spec.gate, op(zslot)));
            bands.push(((j + 1) * tw, *gated));
        }
        // Where the head's strip of row `r` sits in the output, and — for a dense output — the tile row it shares.
        let (y_row, y_col) = if o_dense { (r / tr, 0) } else { (0, r * v_t) };
        let y_base = y_row * v_t + y_col;
        let shared_row = if o_dense {
            let yin = t.buffer("in_y", nb, precision);
            let ly = t.load_block_at("load_y", y, y_base, head_steps(nb), 1, nb, nb, yin);
            Some((ly, yin))
        } else {
            None
        };
        let s_out = match shared_row {
            None => t.stage(
                "gd_gate",
                StageKind::Compute(ComputeOp::Generic {
                    iters: flat_space(w),
                    ins: maps,
                    out: ident_map(w),
                    body: banded(bands),
                }),
                ins,
                vec![ob],
            ),
            // Every plane row of the page, row `r mod tile` the new values and the rest the tile row's other rows. The
            // new values are read down the page (their maps have no row term), so plane row `k` holds row `r`'s.
            Some((_, yin)) => {
                let k = r % tr;
                let old = ins.len();
                ins.push(yin);
                maps.push(ident_map(w));
                let maps = maps
                    .into_iter()
                    .enumerate()
                    .map(|(i, m)| if i == 0 || i == 2 { at(0) } else { m })
                    .collect();
                let body = TensorBody::Select {
                    cond: Pred::AxisLt(LV_GEN_ROW, PredRhs::Const(k)),
                    then: op(old),
                    otherwise: Box::new(TensorBody::Select {
                        cond: Pred::AxisLt(LV_GEN_ROW, PredRhs::Const(k + 1)),
                        then: Box::new(banded(bands)),
                        otherwise: op(old),
                    }),
                };
                t.stage(
                    "gd_gate",
                    StageKind::Compute(ComputeOp::Generic {
                        iters: crate::spatial::passes::from_tensor_op::space(target, w, None),
                        ins: maps,
                        out: ident_map(w),
                        body,
                    }),
                    ins,
                    vec![ob],
                )
            }
        };
        let store = t.store_block("store_y", y, y_base, head_steps(nb), 1, nb, nb, ob);
        t.edge(s_o, ovec, s_sq);
        t.edge(s_sq, sq, s_sum);
        t.edge(s_sum, ssum, s_inv);
        t.edge(s_inv, inv, s_out);
        t.edge(lg, gin, s_out);
        for (lz, zb, _) in &zins {
            t.edge(*lz, *zb, s_out);
        }
        if let Some((ly, yin)) = shared_row {
            t.edge(ly, yin, s_out);
        }
        t.edge(s_out, ob, store);
        let items = head_items(&|h| TileRect {
            row0_t: y_row,
            col0_t: y_col + h * nb,
            h_t: 1,
            w_t: nb,
        });
        t.finish(
            format!("gd_step_{hv}x{d}{}", tag(r)),
            items,
            AccPolicy::F32,
            approx("gated-delta step", precision),
            target,
        )
    };

    let mut sections = vec![alpha_t, beta_t];
    for r in 0..rows {
        sections.push(paged(|pages| step(r, pages), target)?);
    }
    Ok(sections)
}

/// ★ A HEAD THAT IS NOT WHOLE TILES IS WIDENED TO THEM. The blocked form addresses a head by whole tiles (its memory
/// an `nb × nb` square of them, its vectors `nb` of them), so a head of 16 or 80 whose memory is past the flat form's one
/// tile has no form of its own. Zeros past a head's `d` are a fixed point of the step: zero entries of `k`, `q` and `v` keep
/// the memory zero past `d` in both axes and the prediction and the output zero past `d`, and leave the norm's sum of
/// squares as it was. So each head is widened to `dp`, the next whole tiles (`Q`, `K`, `V`, `Z` and the norm weight per
/// head, the memory per head in both axes), the blocked step runs at `dp` reading the head's own `d` for its `1/√d`
/// and its norm, and the output and the memory are narrowed back. Every move copies a head's run or writes zeros, so
/// the moves are exact and a non-finite value stays in its own head. Every tensor the program names keeps the program's
/// layout (an edge the program lays out as a matrix is bridged where every op's is, `link::Prelude::agree_layouts`); the
/// widened ones are the op's own.
pub(super) fn widened_gated_delta(
    spec: &GatedDeltaSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let (hv, hk, d, rows) = (spec.hv, spec.hk, spec.d, spec.rows);
    if hv % hk != 0 {
        return Err(SpatialReject::NotLowered("a gated delta whose v-heads are not a multiple of its k-heads"));
    }
    let (tr, tw) = (target.tile.rows, target.tile.cols);
    let dp = d.next_multiple_of(tw);
    let dt = elem_precision(target)?;
    let wide = |what: &str| format!("{}__wide_{what}", names.out);
    // `n` runs of `from` elements, each copied to the start of `to`.
    let runs = |n: usize, from: usize, to: usize| -> Vec<(usize, Option<usize>)> {
        joined_zero_runs((0..n).flat_map(|u| [(u * to + from.min(to), Some(u * from)), ((u + 1) * to, None)]).collect())
    };
    let chunked = takes_chunks(rows, tr);
    let mut sections = Vec::new();
    // `Q`, `K`, `V` and `Z`, each row's heads in turn; the chunked step reads them as matrices.
    let mut widen = |i: usize, heads: usize, what: &str| -> Result<String, SpatialReject> {
        let to = wide(what);
        let flat = if chunked { format!("{to}__rows") } else { to.clone() };
        sections.push(moved_runs(names.operand(i), rows * heads * d, &flat, &runs(rows * heads, d, dp), target)?);
        if chunked {
            sections.extend(relayout_rows_to_dense((&flat, &Fill::Unknown), &to, rows, heads * dp, dt, target)?);
        }
        Ok(to)
    };
    let (q, k, v, z) = (widen(0, hk, "q")?, widen(1, hk, "k")?, widen(2, hv, "v")?, widen(3, hv, "z")?);
    let gn = wide("gn");
    sections.push(moved_runs(names.operand(7), d, &gn, &runs(1, d, dp), target)?);
    // The memory: head `h`'s row `a` of `d` lands at row `h·dp + a` of `dp`, its rows past `d` zero; the step reads it as
    // the `hv·dp × dp` matrix.
    let (st_name, st_fill) = names.operand(6);
    let st = wide("state");
    let st_rows = format!("{st}__rows");
    let into: Vec<(usize, Option<usize>)> = (0..hv * dp)
        .flat_map(|y| {
            let (h, a) = (y / dp, y % dp);
            let copied = (a < d).then_some((y * dp + d, Some((h * d + a) * d)));
            copied.into_iter().chain(std::iter::once(((y + 1) * dp, None)))
        })
        .collect();
    sections.push(moved_runs((st_name, st_fill), hv * d * d, &st_rows, &joined_zero_runs(into), target)?);
    sections.extend(relayout_rows_to_dense((&st_rows, &Fill::Unknown), &st, hv * dp, dp, dt, target)?);
    let out = wide("out");
    let (b, a) = (names.operand(4), names.operand(5));
    let step = StageNames {
        out: out.clone(),
        ins: vec![q, k, v, z, b.0.to_string(), a.0.to_string(), st.clone(), gn],
        fills: vec![Fill::Unknown, Fill::Unknown, Fill::Unknown, Fill::Unknown, b.1.clone(), a.1.clone(), Fill::Unknown, Fill::Unknown],
    };
    // The widened memory is the op's own, so each row's (or chunk's) version has a name of its own and one writer.
    let version = |i: usize| if i == 0 { st.clone() } else { format!("{st}__{i}") };
    sections.extend(blocked_gated_delta(&GatedDeltaSpec { d: dp, ..spec.clone() }, &step, &BTreeSet::new(), d, &version, target)?);
    let last = if chunked { rows.div_ceil(tr) } else { rows };
    // Back to the program's layout: the output's heads, and the memory's rows.
    sections.push(moved_runs((&out, &Fill::Unknown), rows * hv * dp, &names.out, &runs(rows * hv, dp, d), target)?);
    let back = format!("{st}__back");
    sections.push(relayout_dense_to_rows((&version(last), &Fill::Unknown), &back, hv * dp, dp, dt, target)?);
    let from: Vec<(usize, Option<usize>)> = (0..hv * d).map(|y| (y * d + d, Some((y / d * dp + y % d) * dp))).collect();
    sections.push(moved_runs((&back, &Fill::Unknown), hv * dp * dp, st_name, &from, target)?);
    Ok(sections)
}

/// `runs` with every run of zeros joined to the zeros before it.
pub(super) fn joined_zero_runs(runs: Vec<(usize, Option<usize>)>) -> Vec<(usize, Option<usize>)> {
    let mut out: Vec<(usize, Option<usize>)> = Vec::with_capacity(runs.len());
    for (end, from) in runs {
        match (out.last_mut(), from) {
            (Some(last), None) if last.1.is_none() => last.0 = end,
            (Some(last), _) if last.0 == end => {}
            _ => out.push((end, from)),
        }
    }
    out
}

/// A section writing `out`, flat, from runs of `src`, flat: each `(end, from)` ends at element `end` of `out` and copies
/// `src` from element `from` (the run's first element reads `src[from]`), or writes zeros. Movement only: exact, and a
/// non-finite value lands only where its run puts it.
///
/// ★ AN ITEM IS AN OUTPUT TILE, AND IT LOADS ONLY THE SOURCE IT MOVES. A flat row spends a tile per tile-width of
/// elements, so a section that loads its whole source into every item outgrows a site at a widened memory's length.
/// Output tile `i` loads a window of `W` source tiles (`W` the most any tile's runs reach) from the tile a compile-time
/// table names for it, through an [`IndirectTile`], and reads each of its runs at the offset the same table gives: a run
/// that starts mid-tile, in either array, costs nothing more. Tile `i`'s table page holds the window's first tile, then
/// for each of its runs (`R` of them, the most any tile holds, the unused ones empty) the column the zeros before it end
/// at, the column it ends at, and its offset in the window. The page is loaded twice, once as the window's address and
/// once for the body, a loaded page having one reader.
pub(super) fn moved_runs(
    src: (&str, &Fill),
    src_len: usize,
    out: &str,
    runs: &[(usize, Option<usize>)],
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let n = runs.last().map_or(0, |r| r.0);
    if n == 0 || src_len == 0 {
        return Err(SpatialReject::NotLowered("a move of nothing"));
    }
    let tw = target.tile.cols;
    let (n_t, src_t) = (tiles_of(n, target), tiles_of(src_len, target));
    let dt = elem_precision(target)?;
    // Each output tile's runs: its first column in the tile, the column it ends at, the source element it starts at.
    let mut pieces: Vec<Vec<(usize, usize, usize)>> = vec![Vec::new(); n_t];
    let mut start = 0;
    for &(end, from) in runs {
        if let Some(f) = from {
            let mut e = start;
            while e < end {
                let i = e / tw;
                let stop = end.min((i + 1) * tw);
                pieces[i].push((e - i * tw, stop - i * tw, f + (e - start)));
                e = stop;
            }
        }
        start = end;
    }
    let reach = |ps: &[(usize, usize, usize)]| -> Option<(usize, usize)> {
        let lo = ps.iter().map(|p| p.2).min()?;
        let hi = ps.iter().map(|p| p.2 + (p.1 - p.0)).max()?;
        Some((lo / tw, hi.div_ceil(tw)))
    };
    let window = pieces.iter().filter_map(|ps| reach(ps)).map(|(a, b)| b - a).max().unwrap_or(1).clamp(1, src_t);
    let r_max = pieces.iter().map(Vec::len).max().unwrap_or(0).max(1);
    if 1 + 3 * r_max > tw {
        return Err(SpatialReject::NotLowered("a move whose output tiles each take more runs than a table page names"));
    }
    // A run's offset in the window, lifted by a tile so it is never negative: column `c` reads `c + off − tile`.
    let lift = tw as i64;
    let mut values: Vec<i64> = Vec::with_capacity(n_t * tw);
    for ps in &pieces {
        let first = reach(ps).map_or(0, |(a, _)| a.min(src_t - window));
        let mut page = vec![first as i64];
        let mut last = 0usize;
        for k in 0..r_max {
            match ps.get(k) {
                Some(&(a, b, f)) => {
                    page.extend([a as i64, b as i64, f as i64 - (first * tw) as i64 - a as i64 + lift]);
                    last = b;
                }
                None => page.extend([last as i64, last as i64, lift]),
            }
        }
        page.resize(tw, 0);
        values.extend(page);
    }
    let reach_max = values.iter().copied().max().unwrap_or(0).max(0) as usize;
    let ix = index_type(target, reach_max.max(src_t) as u64)?;
    let mut t = Template::new(target)?;
    let source = t.tensor(src.0, TensorRole::Input, 1, src_len, dt, crate::spatial::passes::from_tensor_op::init_of(src.1, src_len));
    let y = t.tensor(out, TensorRole::Output, 1, n, dt, TensorInit::Zero);
    let table = t.tensor(&format!("{out}__runs"), TensorRole::Param, 1, n_t * tw, ix, TensorInit::Values(values));
    let addr = t.buffer("in_runs_window", 1, ix);
    let l_addr = t.load_block_at("load_runs_window", table, 0, vec![(LV_ITEM_X, 1)], 1, 1, n_t, addr);
    let at = t.buffer("in_runs", 1, ix);
    let l_at = t.load_block_at("load_runs", table, 0, vec![(LV_ITEM_X, 1)], 1, 1, n_t, at);
    let page = t.buffer("in_window", window, dt);
    let l_page = t.stage(
        "load_window",
        StageKind::Load {
            tensor: source,
            pattern: AccessPattern {
                origin: TileAddr {
                    base: 0,
                    steps: vec![],
                    indirect: Some(IndirectTile { index: addr, tile_stride: 1, elems_per_tile: 1, bound: (0, src_t - window) }),
                },
                block_h: 1,
                block_w: window,
                stride_w: 1,
                stride_h: src_t,
            },
            num_blocks: 1,
            multicast: None,
        },
        vec![addr],
        vec![page],
    );
    t.edge(l_addr, addr, l_page);
    // Operands: the table's entries for each run, then the window read at each run's offset.
    let (mut bufs, mut maps) = (Vec::new(), Vec::new());
    let entry = |bufs: &mut Vec<BufferId>, maps: &mut Vec<ElemAccess>, k: usize| {
        bufs.push(at);
        maps.push(fixed(k as i64));
        bufs.len() - 1
    };
    let mut bands = Vec::with_capacity(r_max);
    for k in 0..r_max {
        let (a, b, off) = (entry(&mut bufs, &mut maps, 1 + 3 * k), entry(&mut bufs, &mut maps, 2 + 3 * k), entry(&mut bufs, &mut maps, 3 + 3 * k));
        bufs.push(page);
        maps.push(ElemAccess {
            base: -lift,
            terms: vec![(LV_GEN_COL, 1)],
            indirect: Some(IndirectTerm { operand: off, stride: 1, modulus: reach_max + 1 }),
        });
        bands.push((a, b, bufs.len() - 1));
    }
    let below = |bound: usize, then: Box<TensorBody>, otherwise: Box<TensorBody>| {
        Box::new(TensorBody::Select { cond: Pred::AxisLt(LV_GEN_COL, PredRhs::Operand(bound)), then, otherwise })
    };
    let body = bands.iter().rev().fold(konst(0.0), |past, &(a, b, read)| below(a, konst(0.0), below(b, op(read), past)));
    let ob = t.buffer("out", 1, dt);
    let s_move = t.stage(
        "move_runs",
        StageKind::Compute(ComputeOp::Generic { iters: flat_space(tw), ins: maps, out: ident_map(wide(1, target)), body: *body }),
        bufs,
        vec![ob],
    );
    t.edge(l_at, at, s_move);
    t.edge(l_page, page, s_move);
    let st = t.store_block("store_runs", y, 0, vec![(LV_ITEM_X, 1)], 1, 1, n_t, ob);
    t.edge(s_move, ob, st);
    t.finish(format!("move_runs_{out}"), row_items(n_t, 1), AccPolicy::for_result(dt), NumericsClaim::Exact, target)
}

/// `⌈log₂ c⌉`, at least 1: how many levels invert a unit lower-triangular `c × c` matrix by halves — level `ℓ` inverts its
/// diagonal blocks of `2^ℓ` rows, the whole matrix once `2^ℓ ≥ c`.
pub(super) fn solve_levels(c: usize) -> usize {
    (usize::BITS - c.saturating_sub(1).leading_zeros()).max(1) as usize
}

/// Whether a block of `rows` rows is lowered a chunk of `tile` rows at a time: its chunks take fewer sections than its
/// rows stepped one at a time.
pub(super) fn takes_chunks(rows: usize, tile: usize) -> bool {
    rows > 1 && chunked_sections(rows, tile) < stepped_sections(rows)
}

/// Sections the step takes over a block of `rows` rows, a row at a time: per row one pass over the memory; per block
/// the two gate broadcasts.
pub(super) fn stepped_sections(rows: usize) -> usize {
    2 + rows
}

/// Sections a chunked block of `rows` rows takes on `tile`-row chunks: per chunk the gates, `A` and `QKΓ`, the solve's
/// levels, `U`/`W`, `Δ`, the output and the memory update; per block the two gate broadcasts.
pub(super) fn chunked_sections(rows: usize, tile: usize) -> usize {
    2 + (0..rows.div_ceil(tile))
        .map(|c| 6 + solve_levels((rows - c * tile).min(tile)))
        .sum::<usize>()
}

/// ★ A BLOCK OF ROWS IS A CHUNK AT A TIME, NOT A ROW AT A TIME. Unrolling the step over a chunk of `C` rows (the
/// WY/UT form of the delta rule — the Gated DeltaNet paper's chunked algorithm) turns its recurrence into per-head
/// products: with `g_t = Σ_{s≤t} ln α_s`, `Γ_{ti} = e^{g_t − g_i}` (`i ≤ t`), `A_{ti} = β_t Γ_{ti} k_t·k_i` (`i < t`),
/// `T = (I + A)⁻¹ diag(β)`, `U = T V`, `W = T diag(e^g) K`, `Δ = U − W S₀ᵀ`:
///
/// ```text
///   O   = (diag(e^g) Q S₀ᵀ + (QKᵀ ⊙ Γ) Δ) / √d        every row's output, before its norm and gate
///   S_C = e^{g_C} S₀ + Δᵀ diag(e^{g_C − g}) K          the memory the chunk leaves
/// ```
///
/// Every product is ONE contraction (`LV_GEN_K`), and the unit lower-triangular solve inverts `I + A` by halves — each
/// diagonal block from its two halves' inverses, `⌈log₂ C⌉` levels, not `C` steps. The memory is read and written once
/// per chunk instead of three times per row.
///
/// A chunk is ONE TILE ROW of the block's rows, so a head's chunk of `Q`, `K`, `V`, `Z` is a `1 × nb` page of the DENSE
/// table — the tables are read dense, and the layout pass bridges a flat one once. A computed page has one consumer, so
/// what the algebra uses twice (`G`, `A`, the solve's levels, `U`, `W`, `Δ`) crosses a section as a per-chunk tensor,
/// loaded once per reader. A partial last chunk masks its rows on the contraction variable; its padding rows compute to
/// zero, never to a NaN (a masked exponential is SELECTED away, never multiplied by zero).
pub(super) fn chunked_gated_delta(
    spec: &GatedDeltaSpec,
    names: &StageNames,
    dense: &BTreeSet<String>,
    head: usize,
    state: &dyn Fn(usize) -> String,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let (hv, hk, d, rows) = (spec.hv, spec.hk, spec.d, spec.rows);
    let (tr, tw) = (target.tile.rows, target.tile.cols);
    let nb = d / tw;
    let precision = elem_precision(target)?;
    let (q_name, q_fill) = names.operand(0);
    let (k_name, k_fill) = names.operand(1);
    let (v_name, v_fill) = names.operand(2);
    let (z_name, z_fill) = names.operand(3);
    let (b_name, b_fill) = names.operand(4);
    let (a_name, a_fill) = names.operand(5);
    let st_fill = names.operand(6).1;
    let (gn_name, gn_fill) = names.operand(7);
    let (s_rows, qk_w, v_w) = (hv * d, hk * d, hv * d);
    let (qk_t, v_t) = (hk * nb, hv * nb);
    let w1 = wide(1, target);
    let wd = wide(nb, target);
    let a_exp = format!("{}__alpha_t", names.out);
    let b_exp = format!("{}__beta_t", names.out);
    // The output: the program's matrix itself, or a dense twin a relayout turns into the program's flat row.
    let o_dense = dense.contains(&names.out);
    let o_name = if o_dense {
        names.out.clone()
    } else {
        format!("{}__dense", names.out)
    };

    let head_items = |rect: &dyn Fn(usize) -> TileRect| -> Vec<WorkItem> {
        (0..hv)
            .map(|h| WorkItem {
                id: WorkItemId::new(h),
                coord: (h % hk, h / hk),
                out: rect(h),
            })
            .collect()
    };
    let tile_items = || {
        head_items(&|h| TileRect {
            row0_t: 0,
            col0_t: h,
            h_t: 1,
            w_t: 1,
        })
    };
    let strip_items = || {
        head_items(&|h| TileRect {
            row0_t: 0,
            col0_t: h * nb,
            h_t: 1,
            w_t: nb,
        })
    };
    let head_steps = |per_head: usize| vec![(LV_ITEM_Y, per_head), (LV_ITEM_X, hk * per_head)];
    let k_steps = || vec![(LV_ITEM_Y, nb)];

    // ── Per block: the gates as one tile per head — `α_t[r, h·tw + j] = α[r, h]`, row `r` of tile row `r / tile`.
    let sel_fill = || -> Fill {
        Fill::Overrides {
            base: Box::new(Fill::Const(0)),
            sets: (0..hv)
                .flat_map(|h| (0..tw).map(move |j| (h * hv * tw + h * tw + j, 1)))
                .collect(),
        }
    };
    let broadcast =
        |src: &str, fill: &Fill, dst: &str, sel: &str| -> Result<SpatialProgram, SpatialReject> {
            let spec = crate::tensor_recognize::MatmulSpec {
                m: rows,
                n: hv * tw,
                k: hv,
                a: Default::default(),
                b: Default::default(),
                c: Default::default(),
            };
            let names = MatmulNames {
                a: src.to_string(),
                b: sel.to_string(),
                c: dst.to_string(),
            };
            crate::spatial::passes::from_tensor_op::from_matmul(
                &spec,
                &names,
                (fill, &sel_fill()),
                (None, None),
                NumericsPolicy::Exact,
                target,
            )
        };
    let mut sections = vec![
        broadcast(a_name, a_fill, &a_exp, &format!("{}__sel_a", names.out))?,
        broadcast(b_name, b_fill, &b_exp, &format!("{}__sel_b", names.out))?,
    ];

    // Index tiles for the masks, and the identity: `R0[t, i] = t`, `R1[t, i] = t + 1`, `L[t, s] = [s ≤ t]`.
    let index_tile = |f: &dyn Fn(usize, usize) -> f64| -> TensorInit {
        TensorInit::FloatValues((0..tr * tw).map(|e| f(e / tw, e % tw).to_bits()).collect())
    };
    let param = |t: &mut Template, what: &str, init: TensorInit| {
        t.tensor(
            &format!("{}__gd_{what}", names.out),
            TensorRole::Param,
            tr,
            tw,
            precision,
            init,
        )
    };
    let row_index = |plus: usize| index_tile(&move |t, _| (t + plus) as f64);
    let tril = || index_tile(&|t, s| if s <= t { 1.0 } else { 0.0 });
    let eye = || index_tile(&|t, i| if t == i { 1.0 } else { 0.0 });

    // Map helpers over a one-tile page (`w1` wide) and over a head's `1 × nb` page (`wd` wide).
    let m = |terms: &[(LoopVar, i64)]| ElemAccess {
        base: 0,
        terms: terms.to_vec(),
        indirect: None,
    };
    let contract = |rows_x: usize, cols_x: usize, k: usize| {
        vec![
            IterSpec {
                var: LV_GEN_ROW,
                extent: rows_x,
                kind: IterKind::Parallel,
            },
            IterSpec {
                var: LV_GEN_COL,
                extent: cols_x,
                kind: IterKind::Parallel,
            },
            IterSpec {
                var: LV_GEN_K,
                extent: k,
                kind: IterKind::Reduce {
                    pool: ReducePool::Sum,
                },
            },
        ]
    };
    let pointwise = |cols_x: usize| {
        vec![
            IterSpec {
                var: LV_GEN_ROW,
                extent: tr,
                kind: IterKind::Parallel,
            },
            IterSpec {
                var: LV_GEN_COL,
                extent: cols_x,
                kind: IterKind::Parallel,
            },
        ]
    };
    let generic =
        |iters: Vec<IterSpec>, ins: Vec<ElemAccess>, out: ElemAccess, body: TensorBody| {
            StageKind::Compute(ComputeOp::Generic {
                iters,
                ins,
                out,
                body,
            })
        };
    let chunk_tensor = |t: &mut Template, c: usize, what: &str, role: TensorRole, cols: usize| {
        let init = if role == TensorRole::Output {
            TensorInit::Zero
        } else {
            TensorInit::External
        };
        t.tensor(
            &format!("{}__c{c}_{what}", names.out),
            role,
            tr,
            cols,
            precision,
            init,
        )
    };
    let gate_tensor = |t: &mut Template, name: &str| {
        t.tensor(
            name,
            TensorRole::Input,
            rows,
            hv * tw,
            precision,
            TensorInit::External,
        )
    };
    let q_tensor = |t: &mut Template| {
        t.tensor(
            q_name,
            TensorRole::Input,
            rows,
            qk_w,
            precision,
            crate::spatial::passes::from_tensor_op::init_of(q_fill, rows * qk_w),
        )
    };
    let k_tensor = |t: &mut Template| {
        t.tensor(
            k_name,
            TensorRole::Input,
            rows,
            qk_w,
            precision,
            crate::spatial::passes::from_tensor_op::init_of(k_fill, rows * qk_w),
        )
    };
    let finish = |t: Template, what: &str, c: usize, items: Vec<WorkItem>| {
        t.finish(
            format!("gd_chunk{c}_{what}_{hv}x{d}"),
            items,
            AccPolicy::F32,
            approx("chunked gated delta", precision),
            target,
        )
    };

    for c in 0..rows.div_ceil(tr) {
        let cc = (rows - c * tr).min(tr);
        let levels = solve_levels(cc);
        // Row `r` of the block, as a tile of a `rows`-row table: tile row `c`.
        let gate_base = c * hv;

        // ── G and LG: `G[t, ·] = Σ_{s ≤ t} ln α_s`, the cumulative log decay down the rows of a row-broadcast tile, and
        // `LG[t, i] = Σ_{i < s ≤ t} ln α_s`, the log of every ratio `γ_t / γ_i` — each over the chunk's rows `s < C`.
        //
        // ★ A RATIO IS SUMMED, NEVER SUBTRACTED. `g_t` grows with the chunk (`−22` after 32 rows of `α ≈ ½`), where a
        // bf16 page keeps it only to `1/8`; `e^{g_t − g_i}` from two stored sums is then off by up to `e^{±1/8}` exactly
        // where it matters — near the diagonal, where the ratio is close to one. Summed over `(i, t]` alone, a
        // near-diagonal log ratio is a few small terms and keeps its precision.
        sections.push({
            let mut t = Template::new(target)?;
            let at = gate_tensor(&mut t, &a_exp);
            let l = param(&mut t, "tril", tril());
            let (col1, row1) = (
                param(&mut t, "col1", index_tile(&|_, i| (i + 1) as f64)),
                param(&mut t, "row1", row_index(1)),
            );
            let g = chunk_tensor(&mut t, c, "g", TensorRole::Output, hv * tw);
            let lg = chunk_tensor(&mut t, c, "lg", TensorRole::Output, hv * tw);
            let (ain, lin, gout) = (
                t.buffer("in_alpha", 1, precision),
                t.buffer("in_tril", 1, precision),
                t.buffer("g", 1, precision),
            );
            let la = t.load_block_at("load_alpha", at, gate_base, head_steps(1), 1, 1, hv, ain);
            let ll = t.load("load_tril", l, vec![], 1, lin);
            let body = TensorBody::Select {
                cond: Pred::AxisLt(LV_GEN_K, PredRhs::Const(cc)),
                then: mul(op(0), unary(SfpuKind::Ln, op(1))),
                otherwise: konst(0.0),
            };
            let s = t.stage(
                "gd_log_decay",
                generic(
                    contract(tr, tw, tr),
                    vec![
                        m(&[(LV_GEN_ROW, w1 as i64), (LV_GEN_K, 1)]),
                        m(&[(LV_GEN_K, w1 as i64), (LV_GEN_COL, 1)]),
                    ],
                    ident_map(w1),
                    body,
                ),
                vec![lin, ain],
                vec![gout],
            );
            let st = t.store_block("store_g", g, 0, head_steps(1), 1, 1, hv, gout);
            t.edge(ll, lin, s);
            t.edge(la, ain, s);
            t.edge(s, gout, st);
            let (ain2, cin, rin, lgout) = (
                t.buffer("in_alpha_r", 1, precision),
                t.buffer("in_col1", 1, precision),
                t.buffer("in_row1", 1, precision),
                t.buffer("lg", 1, precision),
            );
            let la2 = t.load_block_at("load_alpha_r", at, gate_base, head_steps(1), 1, 1, hv, ain2);
            let lc = t.load("load_col1", col1, vec![], 1, cin);
            let lr = t.load("load_row1", row1, vec![], 1, rin);
            // 0:α_s  1:i + 1  2:t + 1 — `s` counted when `s < C`, `s ≥ i + 1` and `s < t + 1`.
            let within = |then: Box<TensorBody>| {
                let upper = TensorBody::Select {
                    cond: Pred::AxisLt(LV_GEN_K, PredRhs::Operand(2)),
                    then,
                    otherwise: konst(0.0),
                };
                let lower = TensorBody::Select {
                    cond: Pred::AxisGe(LV_GEN_K, PredRhs::Operand(1)),
                    then: Box::new(upper),
                    otherwise: konst(0.0),
                };
                TensorBody::Select {
                    cond: Pred::AxisLt(LV_GEN_K, PredRhs::Const(cc)),
                    then: Box::new(lower),
                    otherwise: konst(0.0),
                }
            };
            let s2 = t.stage(
                "gd_log_ratio",
                generic(
                    contract(tr, tw, tr),
                    vec![m(&[(LV_GEN_K, w1 as i64)]), ident_map(w1), ident_map(w1)],
                    ident_map(w1),
                    within(unary(SfpuKind::Ln, op(0))),
                ),
                vec![ain2, cin, rin],
                vec![lgout],
            );
            let st2 = t.store_block("store_lg", lg, 0, head_steps(1), 1, 1, hv, lgout);
            t.edge(la2, ain2, s2);
            t.edge(lc, cin, s2);
            t.edge(lr, rin, s2);
            t.edge(s2, lgout, st2);
            finish(t, "g", c, tile_items())?
        });

        // ── A and QKΓ: `A[t, i] = β_t Γ[t, i] Σ_b k_t[b] k_i[b]` below the diagonal; `QKΓ[t, i] = Γ[t, i] Σ_b q_t[b] k_i[b]`
        // on and below it.
        sections.push({
            let mut t = Template::new(target)?;
            let (q, k) = (q_tensor(&mut t), k_tensor(&mut t));
            let bt = gate_tensor(&mut t, &b_exp);
            let lg = chunk_tensor(&mut t, c, "lg", TensorRole::Input, hv * tw);
            let (r0, r1) = (
                param(&mut t, "row", row_index(0)),
                param(&mut t, "row1", row_index(1)),
            );
            let a = chunk_tensor(&mut t, c, "a", TensorRole::Output, hv * tw);
            let qk = chunk_tensor(&mut t, c, "qk", TensorRole::Output, hv * tw);
            let part = |t: &mut Template,
                        what: &str,
                        left: TensorId,
                        beta: bool,
                        strict: bool,
                        out_t: TensorId| {
                let lin = t.buffer_deep(&format!("in_{what}_l"), 1, nb, precision);
                let kin = t.buffer_deep(&format!("in_{what}_k"), 1, nb, precision);
                let gin = t.buffer(&format!("in_{what}_lg"), 1, precision);
                let rin = t.buffer(&format!("in_{what}_r"), 1, precision);
                let ll = t.load_block_at(
                    &format!("load_{what}_l"),
                    left,
                    c * qk_t,
                    k_steps(),
                    1,
                    nb,
                    qk_t,
                    lin,
                );
                let lk = t.load_block_at(
                    &format!("load_{what}_k"),
                    k,
                    c * qk_t,
                    k_steps(),
                    1,
                    nb,
                    qk_t,
                    kin,
                );
                let lg = t.load_block_at(
                    &format!("load_{what}_lg"),
                    lg,
                    0,
                    head_steps(1),
                    1,
                    1,
                    hv,
                    gin,
                );
                let lr = t.load(
                    &format!("load_{what}_r"),
                    if strict { r0 } else { r1 },
                    vec![],
                    1,
                    rin,
                );
                // 0:left_t[b]  1:k_i[b]  2:LG[t, i]  3:the mask's row bound  (4:β_t)
                let mut ins = vec![lin, kin, gin, rin];
                let mut maps = vec![
                    m(&[(LV_GEN_ROW, wd as i64), (LV_GEN_K, 1)]),
                    m(&[(LV_GEN_COL, wd as i64), (LV_GEN_K, 1)]),
                    ident_map(w1),
                    ident_map(w1),
                ];
                let mut value = mul(unary(SfpuKind::Exp, op(2)), mul(op(0), op(1)));
                let mut loads = vec![(ll, lin), (lk, kin), (lg, gin), (lr, rin)];
                if beta {
                    let bin = t.buffer(&format!("in_{what}_b"), 1, precision);
                    loads.push((
                        t.load_block_at(
                            &format!("load_{what}_b"),
                            bt,
                            gate_base,
                            head_steps(1),
                            1,
                            1,
                            hv,
                            bin,
                        ),
                        bin,
                    ));
                    ins.push(bin);
                    maps.push(m(&[(LV_GEN_ROW, w1 as i64)]));
                    value = mul(op(4), value);
                }
                let body = TensorBody::Select {
                    cond: Pred::AxisLt(LV_GEN_COL, PredRhs::Operand(3)),
                    then: value,
                    otherwise: konst(0.0),
                };
                let out = t.buffer(what, 1, precision);
                let s = t.stage(
                    &format!("gd_{what}"),
                    generic(contract(tr, tw, d), maps, ident_map(w1), body),
                    ins,
                    vec![out],
                );
                let st = t.store_block(
                    &format!("store_{what}"),
                    out_t,
                    0,
                    head_steps(1),
                    1,
                    1,
                    hv,
                    out,
                );
                for (l, b) in loads {
                    t.edge(l, b, s);
                }
                t.edge(s, out, st);
            };
            part(&mut t, "a", k, true, true, a);
            part(&mut t, "qk", q, false, false, qk);
            finish(t, "a", c, tile_items())?
        });

        // ── The solve: `X = (I + A)⁻¹` by halves. A diagonal block of `I + A` of `2s` rows is `[[I + A₁₁, 0], [A₂₁, I + A₂₂]]`,
        // whose inverse is `[[X₁₁, 0], [−X₂₂ A₂₁ X₁₁, X₂₂]]` from its halves' inverses — so level `ℓ` forms every block of
        // `2^ℓ` rows from the blocks of `2^{ℓ−1}` before it: `X_ℓ = X_{ℓ−1} − X_{ℓ−1} (A ⊙ M_ℓ) X_{ℓ−1}`, `M_ℓ` the lower-left
        // quarter of every block of `2^ℓ` rows. Section `j` is level `j + 1`; level 1's halves are single rows, each its own
        // inverse.
        //
        // ★ EVERY FACTOR IS AN INVERSE. Each `X_ℓ` inverts the chunk's blocks of `2^ℓ` rows, as small as the whole inverse is
        // (an entry is at most `β` where the keys repeat), so no product reaches past the answer. A power series
        // `(I + N)(I + N²)(I + N⁴)…` takes the same `⌈log₂ C⌉` sections through partial sums that grow as the chunk's
        // binomial coefficients (`N¹⁶` past 10⁸ on 32 rows of one repeated key) and cancel back to the answer, which fp32
        // cannot hold.
        let quarter = |level: usize| {
            let (s, h) = (1usize << level, 1usize << (level - 1));
            index_tile(&move |t, i| if t / s == i / s && (t / h) % 2 == 1 && (i / h) % 2 == 0 { 1.0 } else { 0.0 })
        };
        for j in 0..levels {
            sections.push({
                let mut t = Template::new(target)?;
                let a = chunk_tensor(&mut t, c, "a", TensorRole::Input, hv * tw);
                let mask = param(&mut t, &format!("quarter{}", j + 1), quarter(j + 1));
                let x_out = chunk_tensor(&mut t, c, &format!("x{j}"), TensorRole::Output, hv * tw);
                let (ain, min) = (t.buffer("in_a", 1, precision), t.buffer("in_quarter", 1, precision));
                let la = t.load_block_at("load_a", a, 0, head_steps(1), 1, 1, hv, ain);
                let lm = t.load("load_quarter", mask, vec![], 1, min);
                let out = t.buffer("x", 1, precision);
                if j == 0 {
                    // `X₁ = I − A ⊙ M₁`: the rows in pairs, the second of each holding its one entry left of the diagonal.
                    let id = param(&mut t, "eye", eye());
                    let iin = t.buffer("in_eye", 1, precision);
                    let li = t.load("load_eye", id, vec![], 1, iin);
                    let s = t.stage(
                        "gd_x1",
                        generic(
                            pointwise(tw),
                            vec![ident_map(w1), ident_map(w1), ident_map(w1)],
                            ident_map(w1),
                            *sub(op(2), mul(op(0), op(1))),
                        ),
                        vec![ain, min, iin],
                        vec![out],
                    );
                    t.edge(la, ain, s);
                    t.edge(lm, min, s);
                    t.edge(li, iin, s);
                    let st = t.store_block("store_x", x_out, 0, head_steps(1), 1, 1, hv, out);
                    t.edge(s, out, st);
                } else {
                    // `X_{j+1} = X_j − (X_j (A ⊙ M_{j+1})) X_j`.
                    let x_prev = chunk_tensor(&mut t, c, &format!("x{}", j - 1), TensorRole::Input, hv * tw);
                    let (xl, xr, xs) = (
                        t.buffer("in_x_left", 1, precision),
                        t.buffer("in_x_right", 1, precision),
                        t.buffer("in_x_sub", 1, precision),
                    );
                    let lxl = t.load_block_at("load_x_left", x_prev, 0, head_steps(1), 1, 1, hv, xl);
                    let lxr = t.load_block_at("load_x_right", x_prev, 0, head_steps(1), 1, 1, hv, xr);
                    let lxs = t.load_block_at("load_x_sub", x_prev, 0, head_steps(1), 1, 1, hv, xs);
                    let (am, xam, z) = (t.buffer("am", 1, precision), t.buffer("xam", 1, precision), t.buffer("xamx", 1, precision));
                    let left = m(&[(LV_GEN_ROW, w1 as i64), (LV_GEN_K, 1)]);
                    let right = m(&[(LV_GEN_K, w1 as i64), (LV_GEN_COL, 1)]);
                    let s0 = t.stage(
                        "gd_quarter",
                        generic(pointwise(tw), vec![ident_map(w1), ident_map(w1)], ident_map(w1), *mul(op(0), op(1))),
                        vec![ain, min],
                        vec![am],
                    );
                    let s1 = t.stage(
                        "gd_x_am",
                        generic(contract(tr, tw, tr), vec![left.clone(), right.clone()], ident_map(w1), *mul(op(0), op(1))),
                        vec![xl, am],
                        vec![xam],
                    );
                    let s2 = t.stage(
                        "gd_x_am_x",
                        generic(contract(tr, tw, tr), vec![left, right], ident_map(w1), *mul(op(0), op(1))),
                        vec![xam, xr],
                        vec![z],
                    );
                    let s3 = t.stage(
                        "gd_x",
                        generic(pointwise(tw), vec![ident_map(w1), ident_map(w1)], ident_map(w1), *sub(op(0), op(1))),
                        vec![xs, z],
                        vec![out],
                    );
                    t.edge(la, ain, s0);
                    t.edge(lm, min, s0);
                    t.edge(s0, am, s1);
                    t.edge(lxl, xl, s1);
                    t.edge(s1, xam, s2);
                    t.edge(lxr, xr, s2);
                    t.edge(lxs, xs, s3);
                    t.edge(s2, z, s3);
                    let st = t.store_block("store_x", x_out, 0, head_steps(1), 1, 1, hv, out);
                    t.edge(s3, out, st);
                }
                finish(t, &format!("solve{j}"), c, tile_items())?
            });
        }
        let x_last = format!("x{}", levels - 1);

        // ── U and W: `U[t, e] = Σ_i X[t, i] β_i v_i[e]`, `W[t, b] = Σ_i X[t, i] β_i e^{g_i} k_i[b]`.
        sections.push({
            let mut t = Template::new(target)?;
            let x = chunk_tensor(&mut t, c, &x_last, TensorRole::Input, hv * tw);
            let bt = gate_tensor(&mut t, &b_exp);
            let g = chunk_tensor(&mut t, c, "g", TensorRole::Input, hv * tw);
            let v = t.tensor(
                v_name,
                TensorRole::Input,
                rows,
                v_w,
                precision,
                crate::spatial::passes::from_tensor_op::init_of(v_fill, rows * v_w),
            );
            let k = k_tensor(&mut t);
            let u = chunk_tensor(&mut t, c, "u", TensorRole::Output, hv * d);
            let w = chunk_tensor(&mut t, c, "w", TensorRole::Output, hv * d);
            let part = |t: &mut Template,
                        what: &str,
                        right: TensorId,
                        right_base: usize,
                        right_steps: Vec<(LoopVar, usize)>,
                        right_stride: usize,
                        gated: bool,
                        out_t: TensorId| {
                let (xin, bin, rin) = (
                    t.buffer(&format!("in_{what}_x"), 1, precision),
                    t.buffer(&format!("in_{what}_b"), 1, precision),
                    t.buffer_deep(&format!("in_{what}_r"), 1, nb, precision),
                );
                let lx = t.load_block_at(
                    &format!("load_{what}_x"),
                    x,
                    0,
                    head_steps(1),
                    1,
                    1,
                    hv,
                    xin,
                );
                let lb = t.load_block_at(
                    &format!("load_{what}_b"),
                    bt,
                    gate_base,
                    head_steps(1),
                    1,
                    1,
                    hv,
                    bin,
                );
                let lr = t.load_block_at(
                    &format!("load_{what}_r"),
                    right,
                    right_base,
                    right_steps,
                    1,
                    nb,
                    right_stride,
                    rin,
                );
                let mut ins = vec![xin, bin, rin];
                let mut maps = vec![
                    m(&[(LV_GEN_ROW, w1 as i64), (LV_GEN_K, 1)]),
                    m(&[(LV_GEN_K, w1 as i64)]),
                    m(&[(LV_GEN_K, wd as i64), (LV_GEN_COL, 1)]),
                ];
                let mut value = mul(mul(op(0), op(1)), op(2));
                let mut loads = vec![(lx, xin), (lb, bin), (lr, rin)];
                if gated {
                    let gin = t.buffer(&format!("in_{what}_g"), 1, precision);
                    loads.push((
                        t.load_block_at(
                            &format!("load_{what}_g"),
                            g,
                            0,
                            head_steps(1),
                            1,
                            1,
                            hv,
                            gin,
                        ),
                        gin,
                    ));
                    ins.push(gin);
                    maps.push(m(&[(LV_GEN_K, w1 as i64)]));
                    value = mul(value, unary(SfpuKind::Exp, op(3)));
                }
                let out = t.buffer_deep(what, 1, nb, precision);
                let s = t.stage(
                    &format!("gd_{what}"),
                    generic(contract(tr, wd, tr), maps, ident_map(wd), *value),
                    ins,
                    vec![out],
                );
                let st = t.store_block(
                    &format!("store_{what}"),
                    out_t,
                    0,
                    head_steps(nb),
                    1,
                    nb,
                    v_t,
                    out,
                );
                for (l, b) in loads {
                    t.edge(l, b, s);
                }
                t.edge(s, out, st);
            };
            part(&mut t, "u", v, c * v_t, head_steps(nb), v_t, false, u);
            part(&mut t, "w", k, c * qk_t, k_steps(), qk_t, true, w);
            finish(t, "uw", c, strip_items())?
        });

        // ── Δ: `Δ[t, a] = U[t, a] − Σ_b W[t, b] S₀[a, b]`, one a-block (tile) at a time, joined into the head's strip.
        sections.push({
            let mut t = Template::new(target)?;
            let u = chunk_tensor(&mut t, c, "u", TensorRole::Input, hv * d);
            let w = chunk_tensor(&mut t, c, "w", TensorRole::Input, hv * d);
            let st = t.tensor(
                &state(c),
                TensorRole::Input,
                s_rows,
                d,
                precision,
                crate::spatial::passes::from_tensor_op::init_of(st_fill, s_rows * d),
            );
            let dt = chunk_tensor(&mut t, c, "d", TensorRole::Output, hv * d);
            let mut parts = Vec::new();
            for j in 0..nb {
                let (win, sin, uin) = (
                    t.buffer_deep(&format!("in_w{j}"), 1, nb, precision),
                    t.buffer_deep(&format!("in_s{j}"), 1, nb, precision),
                    t.buffer(&format!("in_u{j}"), 1, precision),
                );
                let lw =
                    t.load_block_at(&format!("load_w{j}"), w, 0, head_steps(nb), 1, nb, v_t, win);
                let ls = t.load_block_at(
                    &format!("load_s{j}"),
                    st,
                    j * nb,
                    head_steps(nb * nb),
                    1,
                    nb,
                    nb,
                    sin,
                );
                let lu =
                    t.load_block_at(&format!("load_u{j}"), u, j, head_steps(nb), 1, 1, v_t, uin);
                let ws = t.buffer(&format!("ws{j}"), 1, precision);
                let s1 = t.stage(
                    &format!("gd_ws{j}"),
                    generic(
                        contract(tr, tw, d),
                        vec![
                            m(&[(LV_GEN_ROW, wd as i64), (LV_GEN_K, 1)]),
                            m(&[(LV_GEN_COL, wd as i64), (LV_GEN_K, 1)]),
                        ],
                        ident_map(w1),
                        *mul(op(0), op(1)),
                    ),
                    vec![win, sin],
                    vec![ws],
                );
                let dj = t.scratch_deep(&format!("d{j}"), 1, 1, precision);
                let s2 = t.stage(
                    &format!("gd_d{j}"),
                    generic(
                        pointwise(tw),
                        vec![ident_map(w1), ident_map(w1)],
                        ident_map(w1),
                        *sub(op(0), op(1)),
                    ),
                    vec![uin, ws],
                    vec![dj],
                );
                t.edge(lw, win, s1);
                t.edge(ls, sin, s1);
                t.edge(s1, ws, s2);
                t.edge(lu, uin, s2);
                parts.push((s2, dj));
            }
            let strip = t.buffer_deep("d_strip", 1, nb, precision);
            let jmaps: Vec<ElemAccess> = (0..nb)
                .map(|j| ElemAccess {
                    base: -((j * tw) as i64),
                    terms: vec![(LV_GEN_ROW, w1 as i64), (LV_GEN_COL, 1)],
                    indirect: None,
                })
                .collect();
            let jbands: Vec<(usize, TensorBody)> = (0..nb)
                .map(|j| ((j + 1) * tw, TensorBody::Operand(j)))
                .collect();
            let join = t.stage(
                "d_join",
                generic(pointwise(wd), jmaps, ident_map(wd), banded(jbands)),
                parts.iter().map(|(_, b)| *b).collect(),
                vec![strip],
            );
            for (s, b) in &parts {
                t.edge(*s, *b, join);
            }
            let sto = t.store_block("store_d", dt, 0, head_steps(nb), 1, nb, v_t, strip);
            t.edge(join, strip, sto);
            finish(t, "delta", c, strip_items())?
        });

        // ── The output rows: `o = (e^{g_t} q_t·S₀ᵀ + Σ_i QKΓ[t, i] Δ[i]) / √d`, the head's RMSNorm per row, `GN`, `silu(z)`.
        sections.push({
            let mut t = Template::new(target)?;
            let q = q_tensor(&mut t);
            let st = t.tensor(
                &state(c),
                TensorRole::Input,
                s_rows,
                d,
                precision,
                crate::spatial::passes::from_tensor_op::init_of(st_fill, s_rows * d),
            );
            let qk = chunk_tensor(&mut t, c, "qk", TensorRole::Input, hv * tw);
            let dt = chunk_tensor(&mut t, c, "d", TensorRole::Input, hv * d);
            let g = chunk_tensor(&mut t, c, "g", TensorRole::Input, hv * tw);
            let z = t.tensor(
                z_name,
                TensorRole::Input,
                rows,
                v_w,
                precision,
                crate::spatial::passes::from_tensor_op::init_of(z_fill, rows * v_w),
            );
            let gn = t.tensor(
                gn_name,
                TensorRole::Param,
                1,
                d,
                precision,
                crate::spatial::passes::from_tensor_op::init_of(gn_fill, d),
            );
            let y = t.tensor(
                &o_name,
                TensorRole::State,
                rows,
                v_w,
                precision,
                TensorInit::Zero,
            );
            let inv_sqrt_d = 1.0f64 / (head as f64).sqrt();
            let mut parts = Vec::new();
            for j in 0..nb {
                let (qin, sin) = (
                    t.buffer_deep(&format!("in_q{j}"), 1, nb, precision),
                    t.buffer_deep(&format!("in_s{j}"), 1, nb, precision),
                );
                let (qkin, din, gin) = (
                    t.buffer(&format!("in_qk{j}"), 1, precision),
                    t.buffer(&format!("in_d{j}"), 1, precision),
                    t.buffer(&format!("in_g{j}"), 1, precision),
                );
                let lq = t.load_block_at(
                    &format!("load_q{j}"),
                    q,
                    c * qk_t,
                    k_steps(),
                    1,
                    nb,
                    qk_t,
                    qin,
                );
                let ls = t.load_block_at(
                    &format!("load_s{j}"),
                    st,
                    j * nb,
                    head_steps(nb * nb),
                    1,
                    nb,
                    nb,
                    sin,
                );
                let lqk =
                    t.load_block_at(&format!("load_qk{j}"), qk, 0, head_steps(1), 1, 1, hv, qkin);
                let ld =
                    t.load_block_at(&format!("load_d{j}"), dt, j, head_steps(nb), 1, 1, v_t, din);
                let lg = t.load_block_at(&format!("load_g{j}"), g, 0, head_steps(1), 1, 1, hv, gin);
                let qs = t.buffer(&format!("qs{j}"), 1, precision);
                let s1 = t.stage(
                    &format!("gd_qs{j}"),
                    generic(
                        contract(tr, tw, d),
                        vec![
                            m(&[(LV_GEN_ROW, wd as i64), (LV_GEN_K, 1)]),
                            m(&[(LV_GEN_COL, wd as i64), (LV_GEN_K, 1)]),
                        ],
                        ident_map(w1),
                        *mul(op(0), op(1)),
                    ),
                    vec![qin, sin],
                    vec![qs],
                );
                let qd = t.buffer(&format!("qd{j}"), 1, precision);
                let s2 = t.stage(
                    &format!("gd_qd{j}"),
                    generic(
                        contract(tr, tw, tr),
                        vec![
                            m(&[(LV_GEN_ROW, w1 as i64), (LV_GEN_K, 1)]),
                            m(&[(LV_GEN_K, w1 as i64), (LV_GEN_COL, 1)]),
                        ],
                        ident_map(w1),
                        *mul(op(0), op(1)),
                    ),
                    vec![qkin, din],
                    vec![qd],
                );
                let oj = t.scratch_deep(&format!("o{j}"), 1, 1, precision);
                let s3 = t.stage(
                    &format!("gd_o{j}"),
                    generic(
                        pointwise(tw),
                        vec![ident_map(w1), ident_map(w1), m(&[(LV_GEN_ROW, w1 as i64)])],
                        ident_map(w1),
                        *mul(
                            add(mul(unary(SfpuKind::Exp, op(2)), op(0)), op(1)),
                            konst(inv_sqrt_d),
                        ),
                    ),
                    vec![qs, qd, gin],
                    vec![oj],
                );
                for (l, b, s) in [
                    (lq, qin, s1),
                    (ls, sin, s1),
                    (lqk, qkin, s2),
                    (ld, din, s2),
                    (lg, gin, s3),
                ] {
                    t.edge(l, b, s);
                }
                t.edge(s1, qs, s3);
                t.edge(s2, qd, s3);
                parts.push((s3, oj));
            }
            let acc = AccPolicy::F32.interm_precision();
            let ovec = t.scratch_deep("o_strip", 1, nb, acc);
            let jmaps: Vec<ElemAccess> = (0..nb)
                .map(|j| ElemAccess {
                    base: -((j * tw) as i64),
                    terms: vec![(LV_GEN_ROW, w1 as i64), (LV_GEN_COL, 1)],
                    indirect: None,
                })
                .collect();
            let jbands: Vec<(usize, TensorBody)> = (0..nb)
                .map(|j| ((j + 1) * tw, TensorBody::Operand(j)))
                .collect();
            let join = t.stage(
                "o_join",
                generic(pointwise(wd), jmaps, ident_map(wd), banded(jbands)),
                parts.iter().map(|(_, b)| *b).collect(),
                vec![ovec],
            );
            for (s, b) in &parts {
                t.edge(*s, *b, join);
            }
            // Each row's RMSNorm over the head: the row's sum of squares (a column fold, one value per row), its
            // reciprocal root, then the weight and the gate. The strip is a scratch the sum and the gate both read.
            let ssum = t.scratch("o_sum", 1, acc);
            let s_sum = t.stage(
                "o_sum",
                generic(
                    vec![
                        IterSpec {
                            var: LV_GEN_ROW,
                            extent: tr,
                            kind: IterKind::Parallel,
                        },
                        IterSpec {
                            var: LV_GEN_COL,
                            extent: d,
                            kind: IterKind::Reduce {
                                pool: ReducePool::Sum,
                            },
                        },
                    ],
                    vec![ident_map(wd), ident_map(wd)],
                    per_row_map(w1),
                    *mul(op(0), op(1)),
                ),
                vec![ovec, ovec],
                vec![ssum],
            );
            t.edge(join, ovec, s_sum);
            let inv = t.scratch("o_inv", 1, acc);
            let s_inv = t.stage(
                "o_rsqrt",
                generic(
                    pointwise(tw),
                    vec![per_row_map(w1)],
                    per_row_map(w1),
                    TensorBody::Unary {
                        kind: SfpuKind::Rsqrt {
                            eps_nano: eps_nano(spec.eps)?,
                            over: head,
                        },
                        arg: op(0),
                    },
                ),
                vec![ssum],
                vec![inv],
            );
            t.edge(s_sum, ssum, s_inv);
            let (gin, zin) = (
                t.buffer("in_gn", nb, precision),
                t.buffer_deep("in_z", 1, nb, precision),
            );
            let lgn = t.load("load_gn", gn, vec![], nb, gin);
            let lz = t.load_block_at("load_z", z, c * v_t, head_steps(nb), 1, nb, v_t, zin);
            let ob = t.buffer_deep("out", 1, nb, precision);
            let gate = t.stage(
                "gd_gate",
                generic(
                    pointwise(wd),
                    vec![
                        ident_map(wd),
                        per_row_map(w1),
                        m(&[(LV_GEN_COL, 1)]),
                        ident_map(wd),
                    ],
                    ident_map(wd),
                    *mul(mul(mul(op(0), op(1)), op(2)), delta_gate(spec.gate, op(3))),
                ),
                vec![ovec, inv, gin, zin],
                vec![ob],
            );
            t.edge(s_inv, inv, gate);
            t.edge(lgn, gin, gate);
            t.edge(lz, zin, gate);
            let sto = t.store_block("store_o", y, c * v_t, head_steps(nb), 1, nb, v_t, ob);
            t.edge(gate, ob, sto);
            let items = head_items(&|h| TileRect {
                row0_t: c,
                col0_t: h * nb,
                h_t: 1,
                w_t: nb,
            });
            finish(t, "out", c, items)?
        });

        // ── The memory the chunk leaves: `S_C[a, b] = e^{g_C} S₀[a, b] + Σ_t Δ[t, a] e^{g_C − g_t} k_t[b]`, the head's
        // a-blocks as sub-blocks of its item.
        sections.push({
            let mut t = Template::new(target)?;
            let st = t.tensor(
                &state(c),
                TensorRole::Input,
                s_rows,
                d,
                precision,
                crate::spatial::passes::from_tensor_op::init_of(st_fill, s_rows * d),
            );
            let stout = t.tensor(
                &state(c + 1),
                TensorRole::Output,
                s_rows,
                d,
                precision,
                TensorInit::Zero,
            );
            let dt = chunk_tensor(&mut t, c, "d", TensorRole::Input, hv * d);
            let k = k_tensor(&mut t);
            let g = chunk_tensor(&mut t, c, "g", TensorRole::Input, hv * tw);
            let lg = chunk_tensor(&mut t, c, "lg", TensorRole::Input, hv * tw);
            let hold = |mut v: Vec<(LoopVar, usize)>| {
                v.push((crate::spatial::passes::LV_SUB_H, 0));
                v
            };
            let row_steps = vec![
                (LV_ITEM_Y, nb * nb),
                (LV_ITEM_X, hk * nb * nb),
                (crate::spatial::passes::LV_SUB_H, nb),
            ];
            let d_steps = vec![(LV_ITEM_Y, nb), (LV_ITEM_X, hk * nb), (crate::spatial::passes::LV_SUB_H, 1)];
            let (din, kin, lgin, sin, gin2) = (
                t.buffer_deep("in_d", 1, 1, precision),
                t.buffer_deep("in_k", 1, nb, precision),
                t.buffer_deep("in_lg", 1, 1, precision),
                t.buffer_deep("in_s", 1, nb, precision),
                t.buffer_deep("in_g_c", 1, 1, precision),
            );
            let ld = t.load_block_at("load_d", dt, 0, d_steps, 1, 1, v_t, din);
            let lk = t.load_block_at("load_k", k, c * qk_t, hold(k_steps()), 1, nb, qk_t, kin);
            let llg = t.load_block_at("load_lg", lg, 0, hold(head_steps(1)), 1, 1, hv, lgin);
            let ls = t.load_block_at("load_s", st, 0, row_steps.clone(), 1, nb, nb, sin);
            let lg2 = t.load_block_at("load_g_c", g, 0, hold(head_steps(1)), 1, 1, hv, gin2);
            let g_c = fixed(((cc - 1) * w1) as i64);
            // 0:Δ[t, a]  1:k_t[b]  2:LG[C − 1, t] = ln(γ_C / γ_t) — folded over the chunk's rows `t`.
            let body = TensorBody::Select {
                cond: Pred::AxisLt(LV_GEN_K, PredRhs::Const(cc)),
                then: mul(mul(op(0), op(1)), unary(SfpuKind::Exp, op(2))),
                otherwise: konst(0.0),
            };
            let upd = t.buffer_deep("upd", 1, nb, precision);
            let s1 = t.stage(
                "gd_s_update",
                generic(
                    contract(tr, d, tr),
                    vec![
                        m(&[(LV_GEN_K, w1 as i64), (LV_GEN_ROW, 1)]),
                        m(&[(LV_GEN_K, wd as i64), (LV_GEN_COL, 1)]),
                        ElemAccess {
                            base: ((cc - 1) * w1) as i64,
                            terms: vec![(LV_GEN_K, 1)],
                            indirect: None,
                        },
                    ],
                    ident_map(wd),
                    body,
                ),
                vec![din, kin, lgin],
                vec![upd],
            );
            let ob = t.buffer_deep("out", 1, nb, precision);
            let s2 = t.stage(
                "gd_s_decay",
                generic(
                    pointwise(d),
                    vec![ident_map(wd), ident_map(wd), g_c],
                    ident_map(wd),
                    *add(op(0), mul(unary(SfpuKind::Exp, op(2)), op(1))),
                ),
                vec![upd, sin, gin2],
                vec![ob],
            );
            for (l, b, s) in [
                (ld, din, s1),
                (lk, kin, s1),
                (llg, lgin, s1),
                (ls, sin, s2),
                (lg2, gin2, s2),
            ] {
                t.edge(l, b, s);
            }
            t.edge(s1, upd, s2);
            let sto = t.store_block("store_s", stout, 0, row_steps, 1, nb, nb, ob);
            t.edge(s2, ob, sto);
            let items = head_items(&|h| TileRect {
                row0_t: h * nb,
                col0_t: 0,
                h_t: nb,
                w_t: nb,
            });
            let mut prog = finish(t, "memory", c, items)?;
            prog.work.plan = Some(BlockPlan {
                per_site_m: nb,
                per_site_n: nb,
                grid: (hv, 1),
                inner: InnerLoop {
                    num_blocks: 1,
                    block_k_tiles: 1,
                },
                block: BlockShape {
                    osh: 1,
                    osw: nb,
                    ibw: 1,
                },
                acc: AccPolicy::F32,
                pack_accumulate: false,
                engines: EnginePacking::LoadsTogether,
                granularity: Granularity::PerTile,
            });
            prog.work.k_tiles = 1;
            prog
        });
    }
    if !o_dense {
        sections.push(relayout_dense_to_rows(
            (&o_name, &Fill::Unknown),
            &names.out,
            rows,
            v_w,
            precision,
            target,
        )?);
    }
    Ok(sections)
}
