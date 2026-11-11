//! Norms and rows: per-row reductions and their rescales, flat, grouped and blocked, and the row softmax.

use super::*;

// ── The per-head family: a reduction, then a banded rescale ───────────────────────────────────────

/// The chain every per-row normalization shares: square → fold the row → transform the row scalar.
/// Returns the chain and each row's scale ([`RowScales`]); the caller adds the banded rescale that consumes it.
pub(super) fn row_scale_chain(
    b: &mut FlatBuild,
    xin: BufferId,
    rows: usize,
    cols: usize,
    xform: SfpuKind,
    target: &SpatialTarget,
) -> (Vec<StageId>, RowScales) {
    let tile_elems = target.tile.rows * target.tile.cols;
    // The reduce engine's scaler operand, at 1.0: Tensix's `reduce_tile` multiplies every term by a
    // constant tile and REQUIRES one. Every divide in these chains happens ONCE, in the transform.
    let ones = b.t.tensor(
        "row_scaler",
        TensorRole::Param,
        target.tile.rows,
        target.tile.cols,
        b.precision,
        TensorInit::FloatValues(vec![1.0f64.to_bits(); tile_elems]),
    );
    let ones_in = b.t.buffer("in_scaler", 1, b.precision);
    let l = b.t.load("load_scaler", ones, vec![], 1, ones_in);
    b.loads.push((l, ones_in));

    let sq_tiles = tiles_of(cols, target);
    let sqw = wide(sq_tiles, target);
    let one_w = wide(1, target);
    // ★ A PAGE HOLDS A TILE'S ROWS, so the rows run a tile's rows at a time: chunk `k` holds rows `[k·R, k·R + R)`, its
    // square, its sums and its scales in pages of its own.
    let tr = target.tile.rows;
    let mut chain = Vec::new();
    let mut pages = Vec::new();
    for k in 0..rows.div_ceil(tr) {
        let (first, count) = (k * tr, tr.min(rows - k * tr));
        let named = |what: &str| if k == 0 { what.to_string() } else { format!("{what}_{k}") };
        let xsq = b.scratch(&named("xsq"), sq_tiles);
        // The square reads the FLAT input through a 2-D space (`row` is the head) and writes the
        // `rows × cols` plane the reduction folds.
        chain.push(b.compute(
            &named("square"),
            head_space(count, cols, None),
            vec![
                Slot { buf: xin, map: map2((first * cols) as i64, cols as i64, 1) },
                Slot { buf: xin, map: map2((first * cols) as i64, cols as i64, 1) },
            ],
            ident_map(sqw),
            *mul(op(0), op(1)),
            xsq,
        ));
        let ssum = b.scratch(&named("row_sum"), 1);
        chain.push(b.compute(
            &named("reduce_rows"),
            head_space(count, cols, Some(ReducePool::Sum)),
            vec![Slot { buf: xsq, map: map2(0, sqw as i64, 1) }, Slot { buf: ones_in, map: const_map() }],
            per_row_map(one_w),
            TensorBody::Operand(0),
            ssum,
        ));
        let scale = b.scratch(&named("row_scale"), 1);
        chain.push(b.compute(
            &named("row_xform"),
            head_space(count, target.tile.cols, None),
            vec![Slot { buf: ssum, map: per_row_map(one_w) }],
            per_row_map(one_w),
            *unary(xform, op(0)),
            scale,
        ));
        pages.push(scale);
    }
    (chain, RowScales { pages, rows_a_page: tr, one_w })
}

/// Each row's scale out of [`row_scale_chain`]: row `h` at entry `(h mod R)·one_w` of chunk `h / R`'s page, `R` a
/// tile's rows.
pub(super) struct RowScales {
    pages: Vec<BufferId>,
    rows_a_page: usize,
    one_w: usize,
}

impl RowScales {
    /// The slot that reads row `h`'s scale.
    pub(super) fn of(&self, h: usize) -> Slot {
        Slot { buf: self.pages[h / self.rows_a_page], map: fixed(((h % self.rows_a_page) * self.one_w) as i64) }
    }
}

/// `y[r, i] = x[r, i] / sqrt(Σ_i x[r, i]² + 1e-6)` over `rows` heads of `cols` — per-head L2
/// normalization on a FLAT `1 × rows·cols` tensor.
pub fn from_l2norm(
    spec: &L2NormSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols) = (spec.rows, spec.cols);
    let by_head = !row_needs_blocking(cols, target);
    if row_needs_blocking(rows * cols, target) && by_head {
        return blocked_l2norm(spec, names, target);
    }
    let flat = flat_l2norm(spec, names, target)?;
    if by_head {
        whole_row_or(flat, || blocked_l2norm(spec, names, target), target)
    } else {
        Ok(flat)
    }
}

/// [`from_l2norm`]'s flat form: the whole `rows·cols` row resident, each head's sum in its own row of the scratch and its
/// rescale a band.
pub(super) fn flat_l2norm(
    spec: &L2NormSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols) = (spec.rows, spec.cols);
    let n = rows * cols;
    let mut b = FlatBuild::new(target)?;
    let xin = b.input(names.operand(0).0, n, names.operand(0).1, target);
    let cols_t = tiles_of(n, target);
    let out_buf = b.t.buffer("out", cols_t, b.precision);
    let ow = wide(cols_t, target);
    // `over: 1` — l2norm divides by the root SUM of squares, not the root mean.
    let (mut chain, scales) = row_scale_chain(
        &mut b,
        xin,
        rows,
        cols,
        SfpuKind::Rsqrt {
            eps_nano: eps_nano(spec.eps)?,
            over: 1,
        },
        target,
    );
    // ★ THE BANDED RESCALE. Head `h` occupies the flat columns `[h·cols, (h+1)·cols)`, and its scale
    // sits broadcast across tile row `h` — so inside the band the scale's map is a CONSTANT offset.
    let mut slots = vec![Slot {
        buf: xin,
        map: at(0),
    }];
    let mut bands = Vec::new();
    for h in 0..rows {
        slots.push(scales.of(h));
        bands.push(((h + 1) * cols, *mul(op(0), op(slots.len() - 1))));
    }
    let s = b.compute(
        "rescale",
        flat_space(n),
        slots,
        ident_map(ow),
        banded(bands),
        out_buf,
    );
    chain.push(s);
    let claim = approx("per-head l2 normalization", b.precision);
    b.finish(
        &names.out,
        n,
        chain,
        out_buf,
        format!("l2norm_{rows}x{cols}"),
        claim,
        target,
    )
}

/// The COMPOSITION-LAYOUT twin of [`crate::spatial::passes::from_tensor_op::from_fused_row`]'s RMSNorm: the same chain
/// (square → fold → rsqrt-once → rescale → per-column weight), over a FLAT `1 × rows·cols` tensor
/// instead of a `rows × cols` one.
///
/// ★ WHY A TWIN AND NOT A PARAMETER. `from_fused_row` gives the tensor the shape the SOURCE declared —
/// right for a program that is one op, and exactly what a hundred green locks pin. Inside a composition
/// the same tensor is a `1 × n` activation whichever way the sugar views it, and the two views put
/// element `cols` in different places. Both lower the identical arithmetic in the identical order, so
/// they agree to the last bit; `spatial_qwen_ops_lock` checks that against the VM rather than against
/// each other.
pub fn from_row_norm_flat(
    spec: &FusedRowSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    use crate::tensor_recognize::RowReduceKind;
    match spec.reduce {
        RowReduceKind::RmsNorm => {}
        RowReduceKind::Softmax if spec.weight.is_none() && spec.residual.is_none() && spec.bias.is_none() => {
            // A row's exponentials are a page of their own, so a row past the page is the streamed form's, which takes
            // one row (`streamed_softmax`).
            if row_needs_blocking(spec.cols, target) {
                return Err(SpatialReject::NotLowered("a softmax over several rows each wider than an accumulator page — a row's statistics are streamed for one row"));
            }
            let (x, fill) = names.operand(0);
            return flat_softmax_rows((x, fill), &names.out, spec.rows, spec.cols, format!("softmax_flat_{}x{}", spec.rows, spec.cols), target);
        }
        _ => return Err(SpatialReject::NotLowered("a composed layernorm row op (or a softmax with a weight, residual or bias) — no flat twin yet")),
    }
    let (rows, cols) = (spec.rows, spec.cols);
    let n = rows * cols;
    // ★ A PER-HEAD NORM REDUCES PER HEAD, so the head is the item — the same fact `blocked_l2norm` turns
    // on. The flat form keeps all `rows·cols` resident: at Qwen3.8's `24 heads x 256` that is 192 tiles
    // against eight. One head is `cols = 256`, eight tiles, and the fold is over exactly the elements the
    // norm is defined on.
    let by_head = !row_needs_blocking(cols, target) && spec.residual.is_none() && spec.bias.is_none();
    if row_needs_blocking(n, target) && by_head {
        return blocked_head_rms(spec, names, target);
    }
    let flat = flat_row_norm(spec, names, target)?;
    if by_head {
        whole_row_or(flat, || blocked_head_rms(spec, names, target), target)
    } else {
        Ok(flat)
    }
}

/// [`from_row_norm_flat`]'s flat form of the RMS norm: the whole `rows·cols` row resident, the residual added first, each
/// row's scale in its own row of a page, and the rescale a band a row.
pub(super) fn flat_row_norm(
    spec: &FusedRowSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols) = (spec.rows, spec.cols);
    let n = rows * cols;
    let mut b = FlatBuild::new(target)?;
    let xin = b.input(names.operand(0).0, n, names.operand(0).1, target);
    // The spec's operand order is `[x, weight?, residual?, bias?]`.
    let mut rest = 1usize;
    let w_in = spec.weight.map(|_| {
        let i = rest;
        rest += 1;
        b.input(names.operand(i).0, spec.weight_rows.max(1) * cols, names.operand(i).1, target)
    });
    let res_in = spec.residual.map(|_| {
        let i = rest;
        rest += 1;
        b.input(names.operand(i).0, n, names.operand(i).1, target)
    });
    if spec.bias.is_some() {
        return Err(SpatialReject::NotLowered(
            "a composed rmsnorm with a bias term",
        ));
    }
    let cols_t = tiles_of(n, target);
    let out_buf = b.t.buffer("out", cols_t, b.precision);
    let ow = wide(cols_t, target);
    // A fused residual add is part of the value that is normalized, so it happens first and its result
    // is what both the reduction and the rescale read.
    let mut chain: Vec<StageId> = Vec::new();
    let src = match res_in {
        Some(r) => {
            let xr = b.scratch("xr", cols_t);
            let s = b.compute(
                "add_residual",
                flat_space(n),
                vec![
                    Slot {
                        buf: xin,
                        map: at(0),
                    },
                    Slot { buf: r, map: at(0) },
                ],
                ident_map(ow),
                *add(op(0), op(1)),
                xr,
            );
            chain.push(s);
            xr
        }
        None => xin,
    };
    let (mut norm_chain, scales) = row_scale_chain(
        &mut b,
        src,
        rows,
        cols,
        SfpuKind::Rsqrt {
            eps_nano: eps_nano(spec.eps)?,
            over: cols,
        },
        target,
    );
    chain.append(&mut norm_chain);
    let mut slots = vec![Slot {
        buf: src,
        map: at(0),
    }];
    let mut bands = Vec::new();
    for h in 0..rows {
        let head_base = (h * cols) as i64;
        slots.push(scales.of(h));
        let scale_slot = slots.len() - 1;
        let value = match w_in {
            Some(wb) => {
                // Row `h` reads the weight's row `h mod weight_rows`: a hyper-connected residual's lane, or the one row.
                let w_base = ((h % spec.weight_rows.max(1)) * cols) as i64;
                slots.push(Slot {
                    buf: wb,
                    map: at(w_base - head_base),
                });
                mul(mul(op(0), op(scale_slot)), op(slots.len() - 1))
            }
            None => mul(op(0), op(scale_slot)),
        };
        bands.push(((h + 1) * cols, *value));
    }
    let s = b.compute(
        "rescale",
        flat_space(n),
        slots,
        ident_map(ow),
        banded(bands),
        out_buf,
    );
    chain.push(s);
    let claim = approx("row reduction", b.precision);
    b.finish(
        &names.out,
        n,
        chain,
        out_buf,
        format!("rmsnorm_flat_{rows}x{cols}"),
        claim,
        target,
    )
}

/// `y[r, j] = exp(x[r, j] − max_r) / Σ_j exp(x[r, j] − max_r)` over `rows` rows of `cols`, on a FLAT
/// `1 × rows·cols` activation — the row softmax of the composition layout. Attention's probabilities are this
/// section (heads as rows, cache positions as columns), and so is a standalone `softmax of … over rows`: one
/// chain — row max, centred exp, row sum, one reciprocal per row, banded rescale — in one place.
pub(super) fn flat_softmax_rows(
    input: (&str, &Fill),
    out_name: &str,
    rows: usize,
    cols: usize,
    label: String,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    flat_softmax_rows_at(
        input,
        out_name,
        rows,
        cols,
        label,
        elem_precision(target)?,
        None,
        target,
    )
}

/// [`flat_softmax_rows`] with its input and result at `precision` — the accumulator's width for a softmax whose
/// result DECIDES something (a router's expert choice), where a bf16 rounding of a logit moves its weight by
/// `e^(δ)` and can flip a top-K pick.
///
/// `sinks` names a flat tensor of one logit per row that joins the row's softmax and takes no value (gpt-oss's attention
/// sinks): the row's maximum starts from it and its term joins the denominator, as ggml's `soft_max_ext` takes them.
#[allow(clippy::too_many_arguments)]
pub(super) fn flat_softmax_rows_at(
    input: (&str, &Fill),
    out_name: &str,
    rows: usize,
    cols: usize,
    label: String,
    precision: Precision,
    sinks: Option<(&str, &Fill)>,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (h, l) = (rows, cols);
    let n = h * l;
    let mut f = FlatBuild::columns(n, target)?;
    f.precision = precision;
    let sc = f.input(input.0, n, input.1, target);
    let out_buf = f.result_page("probs", n, precision, target);
    let tile_elems = target.tile.rows * target.tile.cols;
    // A composition unifies tensors by name, so a shared constant's name says what it holds: a tile of ones at the
    // target's element precision is `sm_scaler`, and at another precision (a router's fp32 softmax) it says which.
    let scaler = if precision == elem_precision(target)? {
        "sm_scaler".to_string()
    } else {
        format!("sm_scaler_{}", format!("{precision:?}").to_lowercase())
    };
    let ones = f.t.tensor(
        &scaler,
        TensorRole::Param,
        target.tile.rows,
        target.tile.cols,
        f.precision,
        TensorInit::FloatValues(vec![1.0f64.to_bits(); tile_elems]),
    );
    let ones_in = f.t.buffer("in_scaler", 1, f.precision);
    let ls = f.t.load("load_scaler", ones, vec![], 1, ones_in);
    f.loads.push((ls, ones_in));
    let mut sink: Option<BufferId> = None;
    let one_w = wide(1, target);
    let exw = wide(tiles_of(l, target), target);
    // ★ A PAGE HOLDS A TILE'S ROWS, so the rows' statistics run a tile's rows at a time: chunk `k` holds rows
    // `[k·R, k·R + R)` (`R` the tile's rows), each in pages of its own — max over the row, then Σ exp(x − max), then
    // one reciprocal a row — and the rescale reads row `v` from chunk `v / R`'s pages at its row `v mod R` there.
    let tr = target.tile.rows;
    let mut chain = Vec::new();
    let mut chunks: Vec<(BufferId, BufferId)> = Vec::new();
    for k in 0..h.div_ceil(tr) {
        let (first, count) = (k * tr, tr.min(h - k * tr));
        let named = |what: &str| if k == 0 { what.to_string() } else { format!("{what}_{k}") };
        let rmax = f.scratch(&named("row_max"), 1);
        chain.push(f.compute(
            &named("row_max"),
            head_space(count, l, Some(ReducePool::Max)),
            vec![Slot { buf: sc, map: map2((first * l) as i64, l as i64, 1) }, Slot { buf: ones_in, map: const_map() }],
            per_row_map(one_w),
            TensorBody::Operand(0),
            rmax,
        ));
        // A row's sink: element `r` of a flat `1 × rows` tensor, loaded once.
        let sink_map = ElemAccess { base: first as i64, terms: vec![(LV_GEN_ROW, 1)], indirect: None };
        if let (Some((name, fill)), None) = (sinks, sink) {
            sink = Some(f.input(name, h, fill, target));
        }
        let rmax = match sink {
            Some(sk) => {
                let with_sink = f.scratch(&named("row_max_sink"), 1);
                chain.push(f.compute(
                    &named("row_max_sink"),
                    head_space(count, target.tile.cols, None),
                    vec![Slot { buf: rmax, map: per_row_map(one_w) }, Slot { buf: sk, map: sink_map.clone() }],
                    per_row_map(one_w),
                    TensorBody::Max(op(0), op(1)),
                    with_sink,
                ));
                with_sink
            }
            None => rmax,
        };
        let ex = f.scratch(&named("expx"), tiles_of(l, target));
        chain.push(f.compute(
            &named("exp_centred"),
            head_space(count, l, None),
            vec![Slot { buf: sc, map: map2((first * l) as i64, l as i64, 1) }, Slot { buf: rmax, map: per_row_map(one_w) }],
            ident_map(exw),
            *unary(SfpuKind::Exp, sub(op(0), op(1))),
            ex,
        ));
        let rsum = f.scratch(&named("row_sum"), 1);
        chain.push(f.compute(
            &named("row_sum"),
            head_space(count, l, Some(ReducePool::Sum)),
            vec![Slot { buf: ex, map: map2(0, exw as i64, 1) }, Slot { buf: ones_in, map: const_map() }],
            per_row_map(one_w),
            TensorBody::Operand(0),
            rsum,
        ));
        let rsum = match sink {
            Some(sk) => {
                let with_sink = f.scratch(&named("row_sum_sink"), 1);
                chain.push(f.compute(
                    &named("row_sum_sink"),
                    head_space(count, target.tile.cols, None),
                    vec![
                        Slot { buf: rsum, map: per_row_map(one_w) },
                        Slot { buf: sk, map: sink_map },
                        Slot { buf: rmax, map: per_row_map(one_w) },
                    ],
                    per_row_map(one_w),
                    *add(op(0), unary(SfpuKind::Exp, sub(op(1), op(2)))),
                    with_sink,
                ));
                with_sink
            }
            None => rsum,
        };
        let inv = f.scratch(&named("row_inv"), 1);
        chain.push(f.compute(
            &named("recip"),
            head_space(count, target.tile.cols, None),
            vec![Slot { buf: rsum, map: per_row_map(one_w) }],
            per_row_map(one_w),
            *unary(SfpuKind::Recip, op(0)),
            inv,
        ));
        chunks.push((ex, inv));
    }
    // A chunk's `ex` is its rows' plane: row `v`'s position `j` sits at `(v mod R)·exw + j`, so the flat output column
    // `c = v·l + j` reads it at `c + (v mod R)·exw − v·l` — one map a row, all affine.
    let mut slots: Vec<Slot> = Vec::new();
    let mut bands = Vec::new();
    for v in 0..h {
        let ((ex, inv), r) = (chunks[v / tr], v % tr);
        slots.push(Slot { buf: ex, map: at((r * exw) as i64 - (v * l) as i64) });
        let e_slot = slots.len() - 1;
        slots.push(Slot { buf: inv, map: fixed((r * one_w) as i64) });
        let i_slot = slots.len() - 1;
        bands.push(((v + 1) * l, *mul(op(e_slot), op(i_slot))));
    }
    chain.extend(f.pointwise("scale", n, slots, banded(bands), out_buf, target)?);
    let claim = approx("row softmax", f.precision);
    f.finish(out_name, n, chain, out_buf, label, claim, target)
}

/// ★ A SOFTMAX OVER A ROW PAST A PAGE IS STREAMED A BLOCK AT A TIME. Each block of the row — a section a block, reading
/// only its block — finds its maximum `m_b` and `s_b = Σ e^(x − m_b)`; one section combines them into the row's
/// `m = max m_b` and `Z = Σ s_b·e^(m_b − m)`, packed at elements 0 and 1 of the row's page. What follows reads every
/// element of the row once more, as `e^(x − m)·(1/Z)`. The statistics stay at the accumulator's width; `x` is read at
/// `x_precision`, the precision its other sections hold it at. Returns the sections and the name of the row's page.
pub(super) fn softmax_row_statistics(
    (x, x_fill, x_precision): (&str, &Fill, Precision),
    y: &str,
    n: usize,
    (label, what): (&str, &str),
    target: &SpatialTarget,
) -> Result<(Vec<SpatialProgram>, String), SpatialReject> {
    let acc = AccPolicy::F32.interm_precision();
    let one = wide(1, target);
    let blocks = router_blocks(n, target);
    let stats = |b: usize| format!("{y}__stats{b}");
    let norm = format!("{y}__norm");
    let mut out = Vec::new();
    for (b, &(first, len)) in blocks.iter().enumerate() {
        let mut f = FlatBuild::at(acc, target)?;
        let l = f.input_slice(x, n, first, len, x_fill, x_precision, target);
        let m = f.t.scratch_deep("block_max", 1, 1, acc);
        let s1 = f.compute("block_max", head_space(1, len, Some(ReducePool::Max)), vec![Slot { buf: l, map: map2(0, 0, 1) }], per_row_map(one), TensorBody::Operand(0), m);
        let sum = f.t.scratch_deep("block_sum", 1, 1, acc);
        let s2 = f.compute(
            "block_sum",
            head_space(1, len, Some(ReducePool::Sum)),
            vec![Slot { buf: l, map: map2(0, 0, 1) }, Slot { buf: m, map: fixed(0) }],
            per_row_map(one),
            *unary(SfpuKind::Exp, sub(op(0), op(1))),
            sum,
        );
        let page = f.t.buffer("stats", 1, acc);
        let s3 = f.compute(
            "pack_stats",
            flat_space(2),
            vec![Slot { buf: m, map: fixed(0) }, Slot { buf: sum, map: fixed(0) }],
            ident_map(one),
            banded(vec![(1, TensorBody::Operand(0)), (2, TensorBody::Operand(1))]),
            page,
        );
        let claim = approx(&format!("{what} block"), acc);
        out.push(f.finish(&stats(b), 2, vec![s1, s2, s3], page, format!("{label}_block{b}_{n}"), claim, target)?);
    }
    // The row's maximum and sum, from the blocks' (m_b, s_b) at elements 0 and 1 of their pages.
    let mut f = FlatBuild::at(acc, target)?;
    let pages: Vec<BufferId> = (0..blocks.len()).map(|b| f.input_paged(&stats(b), 2, &Fill::Unknown, acc, 1, target)).collect();
    let m = f.t.scratch_deep("row_max", 1, 1, acc);
    let maxes = (1..pages.len()).fold(TensorBody::Operand(0), |acc_body, b| TensorBody::Max(Box::new(acc_body), op(b)));
    let s1 = f.compute("row_max", flat_space(1), pages.iter().map(|&buf| Slot { buf, map: fixed(0) }).collect(), ident_map(one), maxes, m);
    let mut slots: Vec<Slot> = pages.iter().flat_map(|&buf| [Slot { buf, map: fixed(0) }, Slot { buf, map: fixed(1) }]).collect();
    slots.push(Slot { buf: m, map: fixed(0) });
    let row_m = 2 * pages.len();
    let z_body = (0..pages.len())
        .map(|b| mul(op(2 * b + 1), unary(SfpuKind::Exp, sub(op(2 * b), op(row_m)))))
        .reduce(add)
        .expect("a wide row has blocks");
    let z = f.t.scratch_deep("row_sum", 1, 1, acc);
    let s2 = f.compute("row_sum", flat_space(1), slots, ident_map(one), *z_body, z);
    let page = f.t.buffer("norm", 1, acc);
    let s3 = f.compute(
        "pack_norm",
        flat_space(2),
        vec![Slot { buf: m, map: fixed(0) }, Slot { buf: z, map: fixed(0) }],
        ident_map(one),
        banded(vec![(1, TensorBody::Operand(0)), (2, TensorBody::Operand(1))]),
        page,
    );
    let claim = approx(&format!("{what} row"), acc);
    out.push(f.finish(&norm, 2, vec![s1, s2, s3], page, format!("{label}_row_{n}"), claim, target)?);
    Ok((out, norm))
}

/// The softmax of one row past a page, or past a site's memory before the page: [`softmax_row_statistics`] over the row,
/// then `e^(x − m)·(1/Z)` a tile an item ([`rescale_row`]), so no section holds more of the row than a block of it. Its
/// input and result are the target's elements, as the whole-row form's are. A softmax over several such rows is refused
/// by name: its rows' statistics would be read a row a band by items that each own a tile.
pub fn streamed_softmax(
    spec: &FusedRowSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    use crate::tensor_recognize::RowReduceKind;
    if spec.reduce != RowReduceKind::Softmax || spec.weight.is_some() || spec.residual.is_some() || spec.bias.is_some() {
        return Err(SpatialReject::NotLowered("a streamed softmax of a row op that is not a plain softmax"));
    }
    if spec.rows != 1 {
        return Err(SpatialReject::NotLowered("a softmax over several rows each past a page or a site — a row's statistics are streamed for one row"));
    }
    let n = spec.cols;
    let precision = elem_precision(target)?;
    let (x, x_fill) = names.operand(0);
    let (mut out, norm) = softmax_row_statistics((x, x_fill, precision), &names.out, n, ("softmax", "row softmax"), target)?;
    let acc = AccPolicy::F32.interm_precision();
    out.push(rescale_row(
        (x, x_fill),
        (None, None),
        RowScalars { name: &norm, len: 2, precision: acc },
        &|x, s| mul(unary(SfpuKind::Exp, sub(x, s[0].clone())), unary(SfpuKind::Recip, s[1].clone())),
        &names.out,
        n,
        ("softmax_rescale", "row softmax"),
        precision,
        target,
    )?);
    Ok(out)
}

// ── The blocked row reduction ─────────────────────────────────────────────────────────────────────
//
// ★ A ROW REDUCTION OVER A ROW THAT DOES NOT FIT IS NOT A HARDER REDUCTION — IT IS A STREAMED ONE.
//
// The whole-row rmsnorm keeps `cols_t` tiles resident and folds them in one pass. At Qwen3.8's real
// `d = 5120` that is 160 tiles against an eight-tile accumulator, so the form is refused outright
// (`AccTilesExceeded`) — and raising the budget is not available to us: it is a property of the silicon.
//
// The way out is not a new primitive. `sum(x²)` IS a matmul — `x² · 1` — and a matmul over a long
// contraction is precisely the thing this compiler already streams, block by block, with the
// accumulation living in the register file across blocks. So the reduction is expressed in the operation
// that already solved this problem, and the row op becomes four SECTIONS, each of which independently
// fits a site:
//
//   1. `xsq  = (x + residual)²`     pointwise, one item per tile
//   2. `ssq  = xsq · ones`          matmul, 1xN by Nx1 — streams over K blocks
//   3. `inv  = rsqrt(ssq / n)`      pointwise on a single tile
//   4. `y    = x · w · inv`         pointwise, one item per tile, `inv` broadcast
//
// ★ THE NUMERICS ARE THE ONES THE FUSED FORM ALREADY EARNED. The scale is applied ONCE, at step 4, and
// the division by `n` rides inside `Rsqrt { over }` rather than being smeared across the reduction's
// terms — the same arrangement `from_fused_row` documents, and for the same measured reason. The
// accumulation path stays at the accumulator's width; only the output rounds.
//
// ★ A STACK OF ROWS IS THE SAME FOUR SECTIONS OVER A MATRIX. `B` sequences normalized at once are `B × d`:
// the squares and the rescale map tile by tile over the 2-D grid, the sum is the `B × d` by `d × 1` matmul —
// every row's contraction in the one weight-free pass — and the scale is one per row, broadcast along it.
pub fn blocked_rms_norm(
    spec: &FusedRowSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    use crate::tensor_recognize::{MatmulSpec, RowReduceKind};
    if spec.reduce != RowReduceKind::RmsNorm {
        return Err(SpatialReject::NotLowered(
            "a blocked softmax or layernorm row op — only rmsnorm streams today",
        ));
    }
    if spec.bias.is_some() {
        return Err(SpatialReject::NotLowered(
            "a blocked rmsnorm with a bias term",
        ));
    }
    if spec.weight_rows > 1 {
        return Err(SpatialReject::NotLowered(
            "a norm in lanes over a dense table — its rescale reads one weight row down every row (`weight_rows` 1)",
        ));
    }
    let (rows, n) = (spec.rows, spec.cols);
    if rows == 0 {
        return Err(SpatialReject::NotLowered("a blocked rmsnorm over no rows"));
    }
    let precision = elem_precision(target)?;
    let (xn, xf) = names.operand(0);
    // The spec's operand order is `[x, weight?, residual?, bias?]`.
    let mut rest = 1usize;
    let weight = spec.weight.map(|_| {
        let i = rest;
        rest += 1;
        names.operand(i)
    });
    let residual = spec.residual.map(|_| {
        let i = rest;
        rest += 1;
        names.operand(i)
    });
    let y = &names.out;
    let sq_name = format!("{y}__xsq");
    let ssq_name = format!("{y}__ssq");
    let inv_name = format!("{y}__inv");
    let ones_name = format!("{y}__ones");
    // The value that is normalized: `x`, or `x + residual` when the add is fused in. Both the reduction
    // and the rescale read it, so when a residual is present it is materialized once under its own name.
    // ★ AND WITH ITS REAL FILL. `x` is read by two sections (the squares and the rescale) and stored by
    // neither, so if both declare it as `External` the composition unifies a tensor nobody ever gave data
    // to. When the residual add is fused in, the base IS produced by a section and carries no fill of its
    // own; when it is not, the base is the source operand and must keep the fill the source gave it.
    let (base_name, base_fill): (String, Fill) = match residual {
        Some(_) => (format!("{y}__xr"), Fill::Unknown),
        None => (xn.to_string(), xf.clone()),
    };

    let mut out = Vec::new();
    if let Some((rn, rf)) = residual {
        out.push(match rows {
            1 => blocked_binary_map(
                "xr",
                "add",
                |a, b| add(a, b),
                (xn, xf),
                (rn, rf),
                &base_name,
                n,
                target,
            )?,
            _ => matrix_binary_map(
                "xr",
                "add",
                |a, b| add(a, b),
                (xn, xf),
                (rn, rf),
                &base_name,
                (rows, n),
                target,
            )?,
        });
    }
    // 1. the squares
    let square = |x: Box<TensorBody>| mul(x.clone(), x);
    out.push(match rows {
        1 => blocked_unary_map(
            "xsq",
            "square",
            Datapath::Float,
            square,
            (&base_name, &base_fill),
            &sq_name,
            n,
            target,
        )?,
        _ => matrix_unary_map(
            "xsq",
            "square",
            Datapath::Float,
            square,
            (&base_name, &base_fill),
            &sq_name,
            (rows, n),
            target,
        )?,
    });
    // 2. the sum, as the contraction it is
    let ones = MatmulNames {
        a: sq_name.clone(),
        b: ones_name,
        c: ssq_name.clone(),
    };
    let mm = MatmulSpec {
        m: rows,
        n: 1,
        k: n,
        a: Default::default(),
        b: Default::default(),
        c: Default::default(),
    };
    out.push(crate::spatial::passes::from_tensor_op::from_matmul(
        &mm,
        &ones,
        (&Fill::Unknown, &Fill::ConstF(1.0f64.to_bits())),
        (None, None),
        NumericsPolicy::Exact,
        target,
    )?);
    // 3. the scale, computed once per row
    let eps = eps_nano(spec.eps)?;
    let rsqrt = |s| {
        Box::new(TensorBody::Unary {
            kind: SfpuKind::Rsqrt {
                eps_nano: eps,
                over: n,
            },
            arg: s,
        })
    };
    out.push(match rows {
        1 => blocked_unary_map(
            "rms_scale",
            "rsqrt",
            Datapath::Float,
            rsqrt,
            (&ssq_name, &Fill::Unknown),
            &inv_name,
            1,
            target,
        )?,
        _ => matrix_unary_map(
            "rms_scale",
            "rsqrt",
            Datapath::Float,
            rsqrt,
            (&ssq_name, &Fill::Unknown),
            &inv_name,
            (rows, 1),
            target,
        )?,
    });
    // 4. the rescale, with each row's scalar broadcast across every tile of its row
    let scaled = |x, s| mul(x, s);
    out.push(match rows {
        1 => rescale_row((&base_name, &base_fill), (weight, None), RowScalars::one(&inv_name, precision), &|x, s| scaled(x, s[0].clone()), y, n, ("rmsnorm_rescale", "rmsnorm"), precision, target)?,
        _ => rescale_rows((&base_name, &base_fill), (weight, None), &inv_name, &scaled, y, (rows, n), ("rmsnorm_rescale", "rmsnorm"), precision, target)?,
    });
    Ok(out)
}

/// A LayerNorm over rows past a site's page, streamed as [`blocked_rms_norm`] streams the RMS norm, in TWO passes so a
/// mean far from the row's spread never cancels against it: a sum of squares taken about zero would hold the square of
/// the mean beside the variance, and in a long row of values near 64 with a spread of a quarter that is every bit the
/// accumulator has. The sums are handed on at the accumulator's width (each contraction keeps its output there under
/// exact numerics), so a row's mean is the fp32 sum's. Six sections, each of which fits a site:
///
///   1. `sum = x · ones`              the row's sum, the contraction it is
///   2. `xc  = x − sum / n`           the row's scalar broadcast along it
///   3. `xsq = xc²`                   pointwise
///   4. `ssq = xsq · ones`            the centred squares' sum
///   5. `inv = rsqrt(ssq / n + ε)`    one scalar a row
///   6. `y   = xc · w · inv + b`      the row's scalar broadcast along it, then the bias
pub fn blocked_layer_norm(
    spec: &FusedRowSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    use crate::tensor_recognize::{MatmulSpec, RowReduceKind};
    if spec.reduce != RowReduceKind::LayerNorm {
        return Err(SpatialReject::NotLowered("a blocked layernorm of a row op that is not one"));
    }
    if spec.weight_rows > 1 {
        return Err(SpatialReject::NotLowered(
            "a norm in lanes over a dense table — its rescale reads one weight row down every row (`weight_rows` 1)",
        ));
    }
    let (rows, n) = (spec.rows, spec.cols);
    if rows == 0 {
        return Err(SpatialReject::NotLowered("a blocked layernorm over no rows"));
    }
    let precision = elem_precision(target)?;
    let (xn, xf) = names.operand(0);
    // The spec's operand order is `[x, weight?, residual?, bias?]`.
    let mut rest = 1usize;
    let mut next = |present: bool| {
        present.then(|| {
            rest += 1;
            names.operand(rest - 1)
        })
    };
    let (weight, residual, bias) = (next(spec.weight.is_some()), next(spec.residual.is_some()), next(spec.bias.is_some()));
    let y = &names.out;
    let [sum_name, xc_name, sq_name, ssq_name, inv_name, ones_name] =
        ["sum", "xc", "xsq", "ssq", "inv", "ones"].map(|part| format!("{y}__{part}"));
    // The value that is normalized: `x`, or `x + residual` materialized once when the add is fused in.
    let (base_name, base_fill): (String, Fill) = match residual {
        Some(_) => (format!("{y}__xr"), Fill::Unknown),
        None => (xn.to_string(), xf.clone()),
    };
    let mut out = Vec::new();
    if let Some((rn, rf)) = residual {
        out.push(match rows {
            1 => blocked_binary_map("xr", "add", |a, b| add(a, b), (xn, xf), (rn, rf), &base_name, n, target)?,
            _ => matrix_binary_map("xr", "add", |a, b| add(a, b), (xn, xf), (rn, rf), &base_name, (rows, n), target)?,
        });
    }
    // A row's sum, the contraction of `a` with a column of ones.
    let row_sum = |a: &str, a_fill: &Fill, c: &str| {
        let mm = MatmulSpec { m: rows, n: 1, k: n, a: Default::default(), b: Default::default(), c: Default::default() };
        let names = MatmulNames { a: a.to_string(), b: ones_name.clone(), c: c.to_string() };
        crate::spatial::passes::from_tensor_op::from_matmul(&mm, &names, (a_fill, &Fill::ConstF(1.0f64.to_bits())), (None, None), NumericsPolicy::Exact, target)
    };
    // 1. the sum
    out.push(row_sum(&base_name, &base_fill, &sum_name)?);
    // 2. the row centred on its mean
    let inv_n = 1.0 / n as f64;
    let centred = move |x, s| sub(x, mul(s, konst(inv_n)));
    out.push(match rows {
        1 => rescale_row((&base_name, &base_fill), (None, None), RowScalars::one(&sum_name, precision), &|x, s| centred(x, s[0].clone()), &xc_name, n, ("layernorm_centre", "layernorm"), precision, target)?,
        _ => rescale_rows((&base_name, &base_fill), (None, None), &sum_name, &centred, &xc_name, (rows, n), ("layernorm_centre", "layernorm"), precision, target)?,
    });
    // 3. the centred squares
    let square = |x: Box<TensorBody>| mul(x.clone(), x);
    out.push(match rows {
        1 => blocked_unary_map("xsq", "square", Datapath::Float, square, (&xc_name, &Fill::Unknown), &sq_name, n, target)?,
        _ => matrix_unary_map("xsq", "square", Datapath::Float, square, (&xc_name, &Fill::Unknown), &sq_name, (rows, n), target)?,
    });
    // 4. their sum
    out.push(row_sum(&sq_name, &Fill::Unknown, &ssq_name)?);
    // 5. the scale, computed once per row
    let eps = eps_nano(spec.eps)?;
    let rsqrt = |s| Box::new(TensorBody::Unary { kind: SfpuKind::Rsqrt { eps_nano: eps, over: n }, arg: s });
    out.push(match rows {
        1 => blocked_unary_map("ln_scale", "rsqrt", Datapath::Float, rsqrt, (&ssq_name, &Fill::Unknown), &inv_name, 1, target)?,
        _ => matrix_unary_map("ln_scale", "rsqrt", Datapath::Float, rsqrt, (&ssq_name, &Fill::Unknown), &inv_name, (rows, 1), target)?,
    });
    // 6. the rescale, then the bias
    let scaled = |x, s| mul(x, s);
    out.push(match rows {
        1 => rescale_row((&xc_name, &Fill::Unknown), (weight, bias), RowScalars::one(&inv_name, precision), &|x, s| scaled(x, s[0].clone()), y, n, ("layernorm_rescale", "layernorm"), precision, target)?,
        _ => rescale_rows((&xc_name, &Fill::Unknown), (weight, bias), &inv_name, &scaled, y, (rows, n), ("layernorm_rescale", "layernorm"), precision, target)?,
    });
    Ok(out)
}

/// [`rescale_row`] over a `rows × n` MATRIX: `y[r][c] = apply(x[r][c] · w[c], s[r]) + b[c]`, one item per tile of the
/// 2-D grid. The weight and the bias are the item's tiles of their one row, broadcast down the tile's rows; the row's
/// scalar is the item's tile of the `rows × 1` column, broadcast along them.
#[allow(clippy::too_many_arguments)]
pub(super) fn rescale_rows(
    (x, x_fill): (&str, &Fill),
    (weight, bias): (Option<(&str, &Fill)>, Option<(&str, &Fill)>),
    inv: &str,
    apply: &dyn Fn(Box<TensorBody>, Box<TensorBody>) -> Box<TensorBody>,
    y: &str,
    shape: (usize, usize),
    (label, what): (&str, &str),
    precision: Precision,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, n) = shape;
    let (rows_t, cols_t) = (
        rows.div_ceil(target.tile.rows),
        n.div_ceil(target.tile.cols),
    );
    let steps = crate::spatial::passes::from_tensor_op::tile_grid_steps(rows_t, cols_t);
    let mut t = Template::new(target)?;
    let xt = t.tensor(
        x,
        TensorRole::Input,
        rows,
        n,
        precision,
        crate::spatial::passes::from_tensor_op::init_of(x_fill, rows * n),
    );
    let it = t.tensor(
        inv,
        TensorRole::Input,
        rows,
        1,
        precision,
        TensorInit::External,
    );
    let yt = t.tensor(y, TensorRole::Output, rows, n, precision, TensorInit::Zero);
    let xin = t.buffer("in_x", 1, precision);
    let iin = t.buffer("in_inv", 1, precision);
    let ob = t.buffer("out", 1, precision);
    let lx = t.load("load_x", xt, steps.clone(), 1, xin);
    let row_steps: Vec<(LoopVar, usize)> = if rows_t > 1 {
        vec![(LV_ITEM_Y, 1)]
    } else {
        Vec::new()
    };
    let li = t.load("load_inv", it, row_steps, 1, iin);
    let w = target.tile.cols;
    let (wbuf, lw) = match weight {
        Some((wn, wf)) => {
            let wt = t.tensor(
                wn,
                TensorRole::Param,
                1,
                n,
                precision,
                crate::spatial::passes::from_tensor_op::init_of(wf, n),
            );
            let b = t.buffer("in_w", 1, precision);
            let l = t.load("load_w", wt, vec![(LV_ITEM_X, 1)], 1, b);
            (Some(b), Some(l))
        }
        None => (None, None),
    };
    let (bbuf, lb) = match bias {
        Some((bn, bf)) => {
            let bt = t.tensor(bn, TensorRole::Param, 1, n, precision, crate::spatial::passes::from_tensor_op::init_of(bf, n));
            let b = t.buffer("in_b", 1, precision);
            (Some(b), Some(t.load("load_b", bt, vec![(LV_ITEM_X, 1)], 1, b)))
        }
        None => (None, None),
    };
    let mut ins = vec![xin];
    let mut maps = vec![ident_map(w)];
    let mut body = op(0);
    if let Some(b) = wbuf {
        ins.push(b);
        maps.push(crate::spatial::passes::from_tensor_op::per_col_map());
        body = mul(body, op(ins.len() - 1));
    }
    ins.push(iin);
    maps.push(crate::spatial::passes::from_tensor_op::per_row_map(w));
    body = apply(body, op(ins.len() - 1));
    if let Some(b) = bbuf {
        ins.push(b);
        maps.push(crate::spatial::passes::from_tensor_op::per_col_map());
        body = add(body, op(ins.len() - 1));
    }
    let s = t.stage(
        "rescale",
        StageKind::Compute(ComputeOp::Generic {
            iters: crate::spatial::passes::from_tensor_op::space(target, w, None),
            ins: maps,
            out: ident_map(w),
            body: *body,
        }),
        ins,
        vec![ob],
    );
    let st = t.store(&format!("store_{y}"), yt, steps, 1, ob);
    t.edge(lx, xin, s);
    if let (Some(l), Some(b)) = (lw, wbuf) {
        t.edge(l, b, s);
    }
    t.edge(li, iin, s);
    if let (Some(l), Some(b)) = (lb, bbuf) {
        t.edge(l, b, s);
    }
    t.edge(s, ob, st);
    t.finish(
        format!("{label}_{rows}x{n}"),
        crate::spatial::passes::from_tensor_op::items_per_tile_grid(rows_t, cols_t),
        AccPolicy::F32,
        approx(what, precision),
        target,
    )
}

/// The scalars a row's rescale reads: element `k` of a `1 × len` tensor at `precision` is scalar `k`.
#[derive(Clone, Copy)]
pub(super) struct RowScalars<'a> {
    name: &'a str,
    len: usize,
    precision: Precision,
}

impl<'a> RowScalars<'a> {
    /// One scalar at the row's own precision: a norm's scale.
    fn one(name: &'a str, precision: Precision) -> RowScalars<'a> {
        RowScalars { name, len: 1, precision }
    }
}

/// `y[i] = apply(x[i] · w[i], s) + b[i]` — one item per tile, with the row's scalars `s` a single tile read through
/// fixed maps so the same scalars reach every block of the row.
#[allow(clippy::too_many_arguments)]
pub(super) fn rescale_row(
    (x, x_fill): (&str, &Fill),
    (weight, bias): (Option<(&str, &Fill)>, Option<(&str, &Fill)>),
    scalars: RowScalars,
    apply: &dyn Fn(Box<TensorBody>, &[Box<TensorBody>]) -> Box<TensorBody>,
    y: &str,
    n: usize,
    (label, what): (&str, &str),
    precision: Precision,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let cols_t = tiles_of(n, target);
    let mut t = Template::new(target)?;
    let xt = t.tensor(
        x,
        TensorRole::Input,
        1,
        n,
        precision,
        crate::spatial::passes::from_tensor_op::init_of(x_fill, n),
    );
    let it = t.tensor(
        scalars.name,
        TensorRole::Input,
        1,
        scalars.len,
        scalars.precision,
        TensorInit::External,
    );
    let yt = t.tensor(y, TensorRole::Output, 1, n, precision, TensorInit::Zero);
    let xin = t.buffer("in_x", 1, precision);
    let iin = t.buffer("in_inv", tiles_of(scalars.len, target), scalars.precision);
    let ob = t.buffer("out", 1, precision);
    let lx = t.load("load_x", xt, vec![(LV_ITEM_X, 1)], 1, xin);
    let li = t.load("load_inv", it, vec![], tiles_of(scalars.len, target), iin);
    let w = wide(1, target);
    let (wbuf, lw) = match weight {
        Some((wn, wf)) => {
            let wt = t.tensor(
                wn,
                TensorRole::Param,
                1,
                n,
                precision,
                crate::spatial::passes::from_tensor_op::init_of(wf, n),
            );
            let b = t.buffer("in_w", 1, precision);
            let l = t.load("load_w", wt, vec![(LV_ITEM_X, 1)], 1, b);
            (Some(b), Some(l))
        }
        None => (None, None),
    };
    let (bbuf, lb) = match bias {
        Some((bn, bf)) => {
            let bt = t.tensor(bn, TensorRole::Param, 1, n, precision, crate::spatial::passes::from_tensor_op::init_of(bf, n));
            let b = t.buffer("in_b", 1, precision);
            (Some(b), Some(t.load("load_b", bt, vec![(LV_ITEM_X, 1)], 1, b)))
        }
        None => (None, None),
    };
    let mut ins = vec![xin];
    let mut maps = vec![ident_map(w)];
    let mut body = op(0);
    if let Some(b) = wbuf {
        ins.push(b);
        maps.push(ident_map(w));
        body = mul(body, op(ins.len() - 1));
    }
    let read: Vec<Box<TensorBody>> = (0..scalars.len)
        .map(|k| {
            ins.push(iin);
            maps.push(fixed(k as i64));
            op(ins.len() - 1)
        })
        .collect();
    body = apply(body, &read);
    if let Some(b) = bbuf {
        ins.push(b);
        maps.push(ident_map(w));
        body = add(body, op(ins.len() - 1));
    }
    let s = t.stage(
        "rescale",
        StageKind::Compute(ComputeOp::Generic {
            iters: flat_space(w),
            ins: maps,
            out: ident_map(w),
            body: *body,
        }),
        ins.clone(),
        vec![ob],
    );
    let st = t.store(&format!("store_{y}"), yt, vec![(LV_ITEM_X, 1)], 1, ob);
    t.edge(lx, xin, s);
    if let (Some(l), Some(b)) = (lw, wbuf) {
        t.edge(l, b, s);
    }
    t.edge(li, iin, s);
    if let (Some(l), Some(b)) = (lb, bbuf) {
        t.edge(l, b, s);
    }
    t.edge(s, ob, st);
    t.finish(
        format!("{label}_{n}"),
        items_per_tile(cols_t),
        AccPolicy::F32,
        approx(what, precision),
        target,
    )
}

/// ★ L2NORM REDUCES PER HEAD, SO IT BLOCKS PER HEAD. The flat form keeps all `rows·cols` elements
/// resident and bands the rescale with constant-offset maps — correct, and impossible once the flat row
/// is 64 tiles. But the REDUCTION was never over the flat row: each head folds only its own `cols`
/// elements, which at Qwen3.8's `gdn_head_dim = 128` is four tiles and fits a site with room to spare.
///
/// So the item becomes the HEAD. Each work item loads one band, folds it, takes the reciprocal root and
/// rescales it — the same chain the flat form runs, with the band as the page instead of the whole row.
/// No banded map is needed at all, because a band is no longer a region inside a bigger page.
pub(super) fn blocked_l2norm(
    spec: &L2NormSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols) = (spec.rows, spec.cols);
    // ★ AN ITEM LOADS ITS HEAD BY WHOLE TILES, so a head that is not whole tiles wide goes a tile-aligned group of heads
    // an item, as the RMS norm's do ([`blocked_head_rms`]).
    if rows > 1 && cols % target.tile.cols != 0 {
        return grouped_row_norm(
            (rows, cols),
            names.operand(0),
            None,
            (&names.out, &format!("{}__ones", names.out)),
            SfpuKind::Rsqrt { eps_nano: eps_nano(spec.eps)?, over: 1 },
            ("l2norm_rows", "per-head l2 normalization"),
            target,
        );
    }
    let n = rows * cols;
    let precision = elem_precision(target)?;
    let band_t = tiles_of(cols, target);
    let mut t = Template::new(target)?;
    let (xn, xf) = names.operand(0);
    let xt = t.tensor(
        xn,
        TensorRole::Input,
        1,
        n,
        precision,
        crate::spatial::passes::from_tensor_op::init_of(xf, n),
    );
    let yt = t.tensor(
        &names.out,
        TensorRole::Output,
        1,
        n,
        precision,
        TensorInit::Zero,
    );
    let acc = AccPolicy::F32.interm_precision();
    let xin = t.buffer("in_x", band_t, precision);
    let ones_t = t.tensor(
        &format!("{}__ones", names.out),
        TensorRole::Param,
        target.tile.rows,
        target.tile.cols,
        precision,
        TensorInit::FloatValues(vec![1.0f64.to_bits(); target.tile.rows * target.tile.cols]),
    );
    let ones_in = t.buffer("in_ones", 1, precision);
    let sq = t.scratch("band_sq", band_t, acc);
    let ssq = t.scratch("band_ssq", 1, acc);
    let inv = t.scratch("band_inv", 1, acc);
    let ob = t.buffer("out", band_t, precision);
    // The band this item owns: `band_t` tiles starting at item `h`.
    let step = vec![(LV_ITEM_X, band_t)];
    let lx = t.load("load_band", xt, step.clone(), band_t, xin);
    let lo = t.load("load_ones", ones_t, vec![], 1, ones_in);
    let w = wide(band_t, target);
    let one = wide(1, target);
    // ★ THE BAND IS ONE HEAD, SO THE 2-D VIEW HAS EXACTLY ONE ROW. `head_space(rows, cols)` is how the
    // flat form views `rows·cols` as a plane whose row is the head; an item that owns a single head sees
    // that plane with `rows = 1`. Reducing with the whole-row `space()` instead would fold 32 tile rows
    // where the band has one, summing 31 rows of padding into the norm.
    let s_sq = t.stage(
        "square",
        StageKind::Compute(ComputeOp::Generic {
            iters: head_space(1, cols, None),
            ins: vec![map2(0, cols as i64, 1), map2(0, cols as i64, 1)],
            out: ident_map(w),
            body: *mul(op(0), op(1)),
        }),
        vec![xin, xin],
        vec![sq],
    );
    let s_sum = t.stage(
        "band_sum",
        StageKind::Compute(ComputeOp::Generic {
            iters: head_space(1, cols, Some(ReducePool::Sum)),
            ins: vec![map2(0, w as i64, 1), const_map()],
            out: per_row_map(one),
            body: TensorBody::Operand(0),
        }),
        vec![sq, ones_in],
        vec![ssq],
    );
    let s_inv = t.stage(
        "band_rsqrt",
        StageKind::Compute(ComputeOp::Generic {
            iters: head_space(1, target.tile.cols, None),
            ins: vec![per_row_map(one)],
            out: per_row_map(one),
            body: TensorBody::Unary {
                kind: SfpuKind::Rsqrt {
                    eps_nano: eps_nano(spec.eps)?,
                    over: 1,
                },
                arg: op(0),
            },
        }),
        vec![ssq],
        vec![inv],
    );
    let s_out = t.stage(
        "rescale",
        StageKind::Compute(ComputeOp::Generic {
            iters: head_space(1, cols, None),
            ins: vec![map2(0, cols as i64, 1), fixed(0)],
            out: ident_map(w),
            body: *mul(op(0), op(1)),
        }),
        vec![xin, inv],
        vec![ob],
    );
    let st = t.store(&format!("store_{}", names.out), yt, step, band_t, ob);
    t.edge(lx, xin, s_sq);
    t.edge(lo, ones_in, s_sum);
    t.edge(s_sq, sq, s_sum);
    t.edge(s_sum, ssq, s_inv);
    t.edge(s_inv, inv, s_out);
    t.edge(s_out, ob, st);
    let items: Vec<WorkItem> = (0..rows)
        .map(|h| WorkItem {
            id: WorkItemId::new(h),
            coord: (0, h),
            out: TileRect {
                row0_t: 0,
                col0_t: h * band_t,
                h_t: 1,
                w_t: band_t,
            },
        })
        .collect();
    t.finish(
        format!("l2norm_{rows}x{cols}"),
        items,
        AccPolicy::F32,
        approx("per-head l2 normalization", precision),
        target,
    )
}

/// [`grouped_row_norm`] for an RMS norm across heads. A norm in lanes keeps the one-row items it needs to choose its
/// weight row, and is refused here by name.
pub(super) fn grouped_head_rms(spec: &FusedRowSpec, names: &StageNames, target: &SpatialTarget) -> Result<SpatialProgram, SpatialReject> {
    if spec.weight_rows > 1 {
        return Err(SpatialReject::NotLowered("a norm in lanes past one site's page whose rows do not start on tile boundaries"));
    }
    grouped_row_norm(
        (spec.rows, spec.cols),
        names.operand(0),
        spec.weight.map(|_| names.operand(1)),
        (&names.out, &format!("{}__rms_scaler", names.out)),
        SfpuKind::Rsqrt { eps_nano: eps_nano(spec.eps)?, over: spec.cols },
        ("rmsnorm_rows", "per-row rmsnorm"),
        target,
    )
}

/// A per-row norm past one site whose rows are not a whole number of tiles wide: an item a GROUP of `g` rows,
/// `g = tile / gcd(cols, tile)`, so a group's `g·cols` values are whole tiles and every group starts on a tile — its
/// block loads and stores as one. Within the item the square is pointwise over the group, each row's sum folds its own
/// `cols` into the row's own lane of the scratch, `scale` turns each sum into its row's factor (the RMS norm's root mean,
/// the L2 norm's root sum), and each row is rescaled by its own band, times the weight at its column where there is one.
/// A row count no whole number of groups covers, or a group wider than a page, is refused by name.
pub(super) fn grouped_row_norm(
    (rows, cols): (usize, usize),
    (xn, xf): (&str, &Fill),
    weight: Option<(&str, &Fill)>,
    (out, scaler): (&str, &str),
    scale: SfpuKind,
    (label, what): (&str, &str),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let tc = target.tile.cols;
    let refuse = |why| Err(SpatialReject::NotLowered(why));
    let gcd = |mut a: usize, mut b: usize| {
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a
    };
    let g = tc / gcd(cols, tc);
    if rows % g != 0 || g > target.tile.rows {
        return refuse("a per-row norm past one site's page whose rows do not start on tile boundaries, in no whole number of tile-aligned groups");
    }
    let (span, group_t) = (g * cols, g * cols / tc);
    if group_t > row_block_tiles(target) {
        return refuse("a per-row norm past one site's page whose rows do not start on tile boundaries, a tile-aligned group of them wider than a page");
    }
    let n = rows * cols;
    let precision = elem_precision(target)?;
    let acc = AccPolicy::F32.interm_precision();
    let mut t = Template::new(target)?;
    let xt = t.tensor(xn, TensorRole::Input, 1, n, precision, crate::spatial::passes::from_tensor_op::init_of(xf, n));
    let yt = t.tensor(out, TensorRole::Output, 1, n, precision, TensorInit::Zero);
    let ones = t.tensor(
        scaler,
        TensorRole::Param,
        target.tile.rows,
        target.tile.cols,
        precision,
        TensorInit::ConstF(1.0f64.to_bits()),
    );
    let xin = t.buffer_deep("in_x", 1, group_t, precision);
    let oin = t.buffer_deep("in_scaler", 1, 1, precision);
    let sq = t.scratch_deep("group_sq", 1, group_t, acc);
    let ssq = t.scratch_deep("group_ssq", 1, 1, acc);
    let inv = t.scratch_deep("group_inv", 1, 1, acc);
    let ob = t.buffer_deep("out", 1, group_t, precision);
    let step = vec![(LV_ITEM_X, group_t)];
    let lx = t.load_block_at("load_group", xt, 0, step.clone(), 1, group_t, group_t, xin);
    let lo = t.load_block_at("load_scaler", ones, 0, vec![], 1, 1, 1, oin);
    let (w, one) = (wide(group_t, target), wide(1, target));
    let s_sq = t.stage(
        "square",
        StageKind::Compute(ComputeOp::Generic { iters: flat_space(span), ins: vec![ident_map(w), ident_map(w)], out: ident_map(w), body: *mul(op(0), op(1)) }),
        vec![xin, xin],
        vec![sq],
    );
    // Row `r` of the group folds its `cols` values, `r·cols` in, into lane `r` of the scratch.
    let s_sum = t.stage(
        "group_sum",
        StageKind::Compute(ComputeOp::Generic {
            iters: head_space(g, cols, Some(ReducePool::Sum)),
            ins: vec![map2(0, cols as i64, 1), const_map()],
            out: per_row_map(one),
            body: TensorBody::Operand(0),
        }),
        vec![sq, oin],
        vec![ssq],
    );
    let s_inv = t.stage(
        "group_rsqrt",
        StageKind::Compute(ComputeOp::Generic {
            iters: head_space(g, tc, None),
            ins: vec![per_row_map(one)],
            out: per_row_map(one),
            body: TensorBody::Unary { kind: scale, arg: op(0) },
        }),
        vec![ssq],
        vec![inv],
    );
    let weight = match weight {
        Some((wn, wf)) => {
            let w_t = tiles_of(cols, target);
            let wt = t.tensor(wn, TensorRole::Param, 1, cols, precision, crate::spatial::passes::from_tensor_op::init_of(wf, cols));
            let b = t.buffer_deep("in_w", 1, w_t, precision);
            Some((t.load_block_at("load_w", wt, 0, vec![], 1, w_t, w_t, b), b))
        }
        None => None,
    };
    // Row `r`'s band `[r·cols, (r + 1)·cols)`: its values times its own rsqrt (and the weight at its column). An operand
    // a buffer and the map it is read through, in step.
    let (mut bufs, mut maps) = (vec![xin], vec![ident_map(w)]);
    let mut bands = Vec::with_capacity(g);
    for r in 0..g {
        bufs.push(inv);
        maps.push(fixed((r * one) as i64));
        let mut body = mul(op(0), op(maps.len() - 1));
        if let Some((_, b)) = weight {
            bufs.push(b);
            maps.push(at(-((r * cols) as i64)));
            body = mul(body, op(maps.len() - 1));
        }
        bands.push(((r + 1) * cols, *body));
    }
    let s_out = t.stage(
        "rescale",
        StageKind::Compute(ComputeOp::Generic { iters: flat_space(span), ins: maps, out: ident_map(w), body: banded(bands) }),
        bufs,
        vec![ob],
    );
    let store = t.store_block("store_y", yt, 0, step, 1, group_t, group_t, ob);
    t.edge(lx, xin, s_sq);
    t.edge(lo, oin, s_sum);
    t.edge(s_sq, sq, s_sum);
    t.edge(s_sum, ssq, s_inv);
    t.edge(s_inv, inv, s_out);
    if let Some((l, b)) = weight {
        t.edge(l, b, s_out);
    }
    t.edge(s_out, ob, store);
    let items: Vec<WorkItem> = (0..rows / g)
        .map(|i| WorkItem { id: WorkItemId::new(i), coord: (0, i), out: TileRect { row0_t: 0, col0_t: i * group_t, h_t: 1, w_t: group_t } })
        .collect();
    t.finish(format!("{label}_{rows}x{cols}"), items, AccPolicy::F32, approx(what, precision), target)
}

/// [`from_row_norm_flat`] with the HEAD as the work item: `y[h, i] = x[h, i]·w[i]·rsqrt(mean_i x[h, i]²)`.
/// The weight is a per-COLUMN table shared by every head, so its tile address does not move with the item.
pub(super) fn blocked_head_rms(
    spec: &FusedRowSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols) = (spec.rows, spec.cols);
    let n = rows * cols;
    // ★ AN ITEM LOADS ITS ROW BY WHOLE TILES, so every row after the first starts on a tile only when the row is whole
    // tiles wide. A row that is not would be read from the tiles of the rows beside it.
    if rows > 1 && cols % target.tile.cols != 0 {
        return grouped_head_rms(spec, names, target);
    }
    let lanes = spec.weight_rows.max(1);
    let head_t = tiles_of(cols, target);
    let precision = elem_precision(target)?;
    let acc = AccPolicy::F32.interm_precision();
    let mut t = Template::new(target)?;
    let (xn, xf) = names.operand(0);
    let xt = t.tensor(
        xn,
        TensorRole::Input,
        1,
        n,
        precision,
        crate::spatial::passes::from_tensor_op::init_of(xf, n),
    );
    let yt = t.tensor(
        &names.out,
        TensorRole::Output,
        1,
        n,
        precision,
        TensorInit::Zero,
    );
    let weight = spec.weight.map(|_| names.operand(1));
    let ones = t.tensor(
        &format!("{}__rms_scaler", names.out),
        TensorRole::Param,
        target.tile.rows,
        target.tile.cols,
        precision,
        TensorInit::ConstF(1.0f64.to_bits()),
    );
    let xin = t.buffer_deep("in_x", 1, head_t, precision);
    let oin = t.buffer_deep("in_scaler", 1, 1, precision);
    let sq = t.scratch_deep("head_sq", 1, head_t, acc);
    let ssq = t.scratch_deep("head_ssq", 1, 1, acc);
    let inv = t.scratch_deep("head_inv", 1, 1, acc);
    let ob = t.buffer_deep("out", 1, head_t, precision);
    // Row `h` is item `(h / lanes, h mod lanes)`: its token and its lane, the lane choosing its row of the weight. With
    // one weight row every row is an item along X.
    let step = if lanes > 1 { vec![(LV_ITEM_Y, lanes * head_t), (LV_ITEM_X, head_t)] } else { vec![(LV_ITEM_X, head_t)] };
    let lx = t.load_block_at("load_head", xt, 0, step.clone(), 1, head_t, head_t, xin);
    let lo = t.load_block_at("load_scaler", ones, 0, vec![], 1, 1, 1, oin);
    let w = wide(head_t, target);
    let one = wide(1, target);
    let s_sq = t.stage(
        "square",
        StageKind::Compute(ComputeOp::Generic {
            iters: head_space(1, cols, None),
            ins: vec![map2(0, cols as i64, 1), map2(0, cols as i64, 1)],
            out: ident_map(w),
            body: *mul(op(0), op(1)),
        }),
        vec![xin, xin],
        vec![sq],
    );
    let s_sum = t.stage(
        "head_sum",
        StageKind::Compute(ComputeOp::Generic {
            iters: head_space(1, cols, Some(ReducePool::Sum)),
            ins: vec![map2(0, w as i64, 1), const_map()],
            out: per_row_map(one),
            body: TensorBody::Operand(0),
        }),
        vec![sq, oin],
        vec![ssq],
    );
    let s_inv = t.stage(
        "head_rsqrt",
        StageKind::Compute(ComputeOp::Generic {
            iters: head_space(1, target.tile.cols, None),
            ins: vec![per_row_map(one)],
            out: per_row_map(one),
            // `over: cols` — the divide by the head width happens ONCE, inside the transform, which is the
            // arrangement `from_fused_row` documents and measured a bf16 ulp on when it did not.
            body: TensorBody::Unary {
                kind: SfpuKind::Rsqrt {
                    eps_nano: eps_nano(spec.eps)?,
                    over: cols,
                },
                arg: op(0),
            },
        }),
        vec![ssq],
        vec![inv],
    );
    let (wbuf, lw) = match weight {
        Some((wn, wf)) => {
            let wt = t.tensor(
                wn,
                TensorRole::Param,
                1,
                lanes * cols,
                precision,
                crate::spatial::passes::from_tensor_op::init_of(wf, lanes * cols),
            );
            let b = t.buffer_deep("in_w", 1, head_t, precision);
            let lane_step = if lanes > 1 { vec![(LV_ITEM_X, head_t)] } else { vec![] };
            let l = t.load_block_at("load_w", wt, 0, lane_step, 1, head_t, head_t, b);
            (Some(b), Some(l))
        }
        None => (None, None),
    };
    let mut ins = vec![xin, inv];
    let mut maps = vec![map2(0, cols as i64, 1), fixed(0)];
    let mut body = mul(op(0), op(1));
    if let Some(b) = wbuf {
        ins.push(b);
        maps.push(map2(0, cols as i64, 1));
        body = mul(body, op(ins.len() - 1));
    }
    let s_out = t.stage(
        "rescale",
        StageKind::Compute(ComputeOp::Generic {
            iters: head_space(1, cols, None),
            ins: maps,
            out: ident_map(w),
            body: *body,
        }),
        ins,
        vec![ob],
    );
    let store = t.store_block("store_y", yt, 0, step, 1, head_t, head_t, ob);
    t.edge(lx, xin, s_sq);
    t.edge(lo, oin, s_sum);
    t.edge(s_sq, sq, s_sum);
    t.edge(s_sum, ssq, s_inv);
    t.edge(s_inv, inv, s_out);
    if let (Some(l), Some(b)) = (lw, wbuf) {
        t.edge(l, b, s_out);
    }
    t.edge(s_out, ob, store);
    let items: Vec<WorkItem> = (0..rows)
        .map(|h| WorkItem {
            id: WorkItemId::new(h),
            coord: if lanes > 1 { (h / lanes, h % lanes) } else { (0, h) },
            out: TileRect {
                row0_t: 0,
                col0_t: h * head_t,
                h_t: 1,
                w_t: head_t,
            },
        })
        .collect();
    t.finish(
        format!("rmsnorm_heads_{rows}x{cols}"),
        items,
        AccPolicy::F32,
        approx("per-head rmsnorm", precision),
        target,
    )
}
