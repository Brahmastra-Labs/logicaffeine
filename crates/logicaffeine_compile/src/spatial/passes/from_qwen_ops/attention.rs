//! Attention: the flat sections, the flash fold over whole heads and over windows, grouped decodes and latent tables.

use super::*;

/// A section writing `out`, a flat `1 × n` tensor whose element `e` is the sink logit of query head `head_of(e)` (or
/// [`MASKED`] for `None`, a row no head fills): the per-row sinks a softmax over the heads' rows reads, from the
/// attention's one logit per head (`sinks`, `per_row` of them). Runs of elements with one head share a band.
pub(super) fn spread_sinks(
    sinks: (&str, &Fill),
    per_row: usize,
    out: &str,
    n: usize,
    head_of: &dyn Fn(usize) -> Option<usize>,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let mut f = FlatBuild::new(target)?;
    let sk = f.input(sinks.0, per_row, sinks.1, target);
    let cols_t = tiles_of(n, target);
    let out_buf = f.t.buffer("sinks", cols_t, f.precision);
    let mut slots: Vec<Slot> = Vec::new();
    let mut bands: Vec<(usize, TensorBody)> = Vec::new();
    let mut e = 0;
    while e < n {
        let head = head_of(e);
        let end = (e..n).find(|x| head_of(*x) != head).unwrap_or(n);
        let body = match head {
            Some(h) => {
                slots.push(Slot { buf: sk, map: fixed(h as i64) });
                op(slots.len() - 1)
            }
            None => konst(MASKED),
        };
        bands.push((end, *body));
        e = end;
    }
    if slots.is_empty() {
        slots.push(Slot { buf: sk, map: fixed(0) });
    }
    let stage = f.compute("spread_sinks", flat_space(n), slots, ident_map(wide(cols_t, target)), banded(bands), out_buf);
    let claim = NumericsClaim::Exact;
    f.finish(out, n, vec![stage], out_buf, format!("spread_sinks_{n}"), claim, target)
}

/// GQA attention of a block of query rows over a cache of `l` rows: `score[j] = (q·k_j)/√hd`, row-softmax, then
/// the weighted V sum — THREE sections, because a softmax cannot be one `Generic` (its maximum and its
/// denominator are reductions whose results the next stage broadcasts back). One row is decode's single query; a
/// causal block of `T` rows is a prefill chunk's queries, row `r` attending one position past row `r − 1`.
pub fn from_attention(
    spec: &AttentionSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    attention_as(spec, names, attention_form(spec, target), target)
}

/// [`from_attention`] in the form `form`.
pub(super) fn attention_as(
    spec: &AttentionSpec,
    names: &StageNames,
    form: HeadForm,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let (h, kv, hd, l, t) = (spec.h, spec.kv, spec.hd, spec.l, spec.rows);
    if h == 0 || kv == 0 || hd == 0 || l == 0 || t == 0 {
        return Err(SpatialReject::NotLowered(
            "an attention with no heads, no query rows, no head dimension or an empty cache",
        ));
    }
    if spec.window.is_some_and(|w| w.span() == 0) {
        return Err(SpatialReject::NotLowered("a window of no positions — every row's softmax would be empty"));
    }
    // The sinks' operand follows `Q, K, V` and a computed position.
    let sinks = spec.sinks.map(|_| 3 + usize::from(matches!(spec.pos, Some(RowIndex::Named(_)))));
    let heads = QueryHeads {
        rows: t,
        per_row: h,
        group: (h / kv).max(1),
    };
    // The positions attended: the whole cache, a literal prefix of it, or — masked — the first `P + r` of query row
    // `r`, for a `P` the program computes (its operand follows `Q, K, V`) or a literal one under a block of rows.
    let (attended, limit) = match spec.pos {
        None if t > 1 => {
            return Err(SpatialReject::NotLowered(
                "a block of query rows with no causal bound",
            ))
        }
        None => (l, None),
        Some(RowIndex::Literal(p)) => {
            if p < 1 || p as usize + (t - 1) > l {
                return Err(SpatialReject::NotLowered("a literal attention length outside its cache — the program fails at run time on every tier, so there is no section to lower"));
            }
            let p = p as usize;
            if t == 1 {
                (p, None)
            } else {
                (p + (t - 1), Some(Bound::Literal(p)))
            }
        }
        Some(RowIndex::Named(_)) => (l, Some(Bound::Operand(3))),
    };
    // ★ A WINDOW KEEPS THE LAST `W` OF EACH ROW'S POSITIONS, so its rows are always masked: an unmasked decode row's bound
    // is the whole of what it attends.
    let limit = match (spec.window, limit) {
        (Some(_), None) => Some(Bound::Literal(attended)),
        (_, limit) => limit,
    };
    let tag = if t == 1 { "attn" } else { "causal_attn" };
    let scale = spec.scale.map_or(1.0 / (hd as f64).sqrt(), |s| s.get());
    // ★ A HEAD IN WHOLE TILES TAKES THE FLASH FORM — every real attention layer does. Query head `v` reads K/V head
    // `(v mod h) / group`, which is not affine in `v`, so the item carries both — `(v, K/V head)` — and every address
    // is affine in its own coordinate; the cache streams in blocks. A head that is not whole tiles takes it over its
    // window where the three sections fit no site ([`attention_form`]).
    if form != HeadForm::Flat {
        let g = FlashGeometry {
            q: heads,
            kvw: kv,
            hd,
            vd: spec.vd,
            v_part: None,
            rows: l,
            l: attended,
            limit,
            window: spec.window,
            sinks,
            grouped: false,
            scale,
            cap: spec.softcap.map(|c| c.get()),
        };
        // ★ ONE ROW'S QUERY HEADS THAT SHARE A K/V HEAD STREAM IT ONCE. A decode step's heads fold one item per K/V
        // head when the group fits a tile's rows and its regrouping fits a site; otherwise an item per query head.
        if t == 1 && heads.group > 1 && heads.group <= target.tile.rows && !row_needs_blocking(hd.max(spec.vd), target) {
            if let Ok(sections) = grouped_decode_attention(names, g, tag, target) {
                return Ok(sections);
            }
        }
        return flash_attention(names, g, tag, target);
    }
    attention_sections(
        names,
        heads,
        HeadWidths { kv, hd, vd: spec.vd, scale, cap: spec.softcap.map(|c| c.get()) },
        Cache {
            rows: l,
            attended,
            limit,
            window: spec.window,
        },
        sinks,
        tag,
        target,
    )
}

/// The query heads an attention computes: `rows` query rows of `per_row` heads each, a row's heads reading K/V heads
/// in groups of `group`. Query head `v` is head `v mod per_row` of row `v / per_row`: its query and its output sit
/// at `v·hd` of the flat `rows × (per_row·hd)` block, and it reads K/V head `(v mod per_row) / group`.
///
/// ★ ONE SCHEME, EVERY ATTENTION. Decode is one row whose heads share K/V heads (GQA); a prefill chunk's causal
/// block is `T` such rows; full attention is `t` rows whose heads read their own K/V head (`group = 1`).
#[derive(Clone, Copy, Debug)]
pub(super) struct QueryHeads {
    rows: usize,
    per_row: usize,
    group: usize,
}

impl QueryHeads {
    fn count(&self) -> usize {
        self.rows * self.per_row
    }
    fn row_of(&self, v: usize) -> usize {
        v / self.per_row
    }
    fn kv_of(&self, v: usize) -> usize {
        (v % self.per_row) / self.group
    }
}

/// The first row's live length `P`, for a bound the cache's extent does not fix: query row `r` attends the first
/// `P + r` positions.
#[derive(Clone, Copy, Debug)]
pub(super) enum Bound {
    /// Fixed by the program, and checked against the cache when the attention is lowered.
    Literal(usize),
    /// Held by operand `i`, and checked at run time so every row stays inside the cache: `1 ≤ P ≤ rows − (T − 1)`.
    Operand(usize),
}

/// The cache an attention reads: `rows` positions declared, the first `attended` of them covered, and — masked —
/// the bound each query row's live positions end at, and the window of positions before it a row keeps.
#[derive(Clone, Copy, Debug)]
pub(super) struct Cache {
    rows: usize,
    attended: usize,
    limit: Option<Bound>,
    window: Option<Window>,
}

/// The largest `P` a block of `q_rows` query rows admits over a cache of `rows`: its last row's positions all cached.
pub(super) fn bound_max(rows: usize, q_rows: usize) -> usize {
    rows + 1 - q_rows
}

/// The widths of an attention's heads: `kv` K/V heads per position, keys `hd` and values `vd` wide, and the factor
/// every score takes.
#[derive(Clone, Copy, Debug)]
pub(super) struct HeadWidths {
    kv: usize,
    hd: usize,
    vd: usize,
    scale: f64,
    /// The soft cap every scaled score takes before the mask, `c·tanh(s/c)`.
    cap: Option<f64>,
}

/// The three sections every unmasked attention is: scores, the row softmax, the weighted V sum — over the
/// [`QueryHeads`], each attending over `l` positions of a K holding `kv` heads of `hd` per position
/// (`k[j·(kv·hd) + kvh·hd + e]`) and a V holding `kv` heads of `vd` (`v[j·(kv·vd) + kvh·vd + e]`).
pub(super) fn attention_sections(
    names: &StageNames,
    heads: QueryHeads,
    widths: HeadWidths,
    cache: Cache,
    sinks: Option<usize>,
    tag: &str,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let HeadWidths { kv, hd, vd, scale, cap } = widths;
    let h = heads.count();
    let kv_of = |v: usize| heads.kv_of(v);
    let l = cache.attended;
    let kv_len = cache.rows * kv * hd;
    let v_len = cache.rows * kv * vd;
    let (q_name, q_fill) = names.operand(0);
    let (k_name, k_fill) = names.operand(1);
    let (v_name, v_fill) = names.operand(2);
    let scores_name = format!("{}_scores", names.out);
    let probs_name = format!("{}_probs", names.out);

    // ── Section 1: the scores, `h · l` of them, flat as `1 × (h·l)`.
    let score = {
        let n = h * l;
        let mut f = FlatBuild::columns(n, target)?;
        let q = f.input(q_name, h * hd, q_fill, target);
        let k = f.input_held(k_name, kv_len, k_fill, cache_precision(target)?, target);
        let precision = f.precision;
        let out_buf = f.result_page("scores", n, precision, target);
        let tile_elems = target.tile.rows * target.tile.cols;
        let ones = f.t.tensor(
            "at_scaler",
            TensorRole::Param,
            target.tile.rows,
            target.tile.cols,
            f.precision,
            TensorInit::FloatValues(vec![1.0f64.to_bits(); tile_elems]),
        );
        let ones_in = f.t.buffer("in_scaler", 1, f.precision);
        let ls = f.t.load("load_scaler", ones, vec![], 1, ones_in);
        f.loads.push((ls, ones_in));
        // The folded axis is the ROW (`e` over the head dimension); the flat output column is
        // `c = head·l + j`, so the scores come out as a flat `1 × (h·l)` row.
        let mut slots = vec![Slot {
            buf: ones_in,
            map: const_map(),
        }];
        let mut bands = Vec::new();
        for head in 0..h {
            let kvh = kv_of(head);
            // `q[head·hd + e]`: constant along the output column, affine in the folded row.
            slots.push(Slot {
                buf: q,
                map: ElemAccess {
                    base: (head * hd) as i64,
                    terms: vec![(LV_GEN_ROW, 1)],
                    indirect: None,
                },
            });
            let q_slot = slots.len() - 1;
            // `kc[j·(kv·hd) + kvh·hd + e]` with `j = c − head·l`.
            let j_stride = (kv * hd) as i64;
            let base = (kvh * hd) as i64 - (head * l) as i64 * j_stride;
            slots.push(Slot {
                buf: k,
                map: ElemAccess {
                    base,
                    terms: vec![(LV_GEN_COL, j_stride), (LV_GEN_ROW, 1)],
                    indirect: None,
                },
            });
            let k_slot = slots.len() - 1;
            bands.push((
                (head + 1) * l,
                *mul(mul(op(q_slot), op(k_slot)), konst(scale)),
            ));
        }
        let mut chain = Vec::new();
        let scored = match (cache.limit, cap) {
            (None, None) => out_buf,
            _ => f.block_scratch("raw_scores", n, target),
        };
        chain.extend(f.fold("attn_scores", n, (hd, ReducePool::Sum), slots, banded(bands), scored, target)?);
        // ★ A SOFT CAP takes every scaled score before the mask, so a masked score stays masked.
        let scored = match cap {
            None => scored,
            Some(c) => {
                let to = if cache.limit.is_some() { f.block_scratch("capped_scores", n, target) } else { out_buf };
                chain.extend(f.pointwise("attn_cap", n, vec![Slot { buf: scored, map: at(0) }], *capped(op(0), c), to, target)?);
                to
            }
        };
        if let Some(bound) = cache.limit {
            // ★ THE MASK. Head `v`'s position `j` sits at column `v·l + j`, so it is live iff the column is below
            // `v·l + P + r`, `r` the head's query row: one threshold per head — a constant for a literal `P`, else built
            // from `P` once `P` is checked to keep every row inside the cache.
            let row_base = |v: usize| v * l + heads.row_of(v);
            let mut slots = vec![Slot {
                buf: scored,
                map: at(0),
            }];
            let below: Vec<PredRhs> = match bound {
                Bound::Literal(p) => (0..h).map(|v| PredRhs::Const(row_base(v) + p)).collect(),
                Bound::Operand(li) => {
                    let (pn, pf) = names.operand(li);
                    let ix = index_type(target, cache.rows as u64)?;
                    let p_in = f.input_as(pn, 1, pf, ix, target);
                    let (mut guard, pc) =
                        checked_count(&mut f, p_in, bound_max(cache.rows, heads.rows), ix, target);
                    chain.append(&mut guard);
                    let thr = f.t.scratch_deep("thresholds", 1, tiles_of(h, target), ix);
                    let bands: Vec<(usize, TensorBody)> = (0..h)
                        .map(|v| (v + 1, *add(konst(row_base(v) as f64), op(0))))
                        .collect();
                    chain.push(f.compute(
                        "thresholds",
                        flat_space(h),
                        vec![Slot {
                            buf: pc,
                            map: fixed(0),
                        }],
                        ident_map(wide(tiles_of(h, target), target)),
                        banded(bands),
                        thr,
                    ));
                    (0..h)
                        .map(|v| {
                            slots.push(Slot {
                                buf: thr,
                                map: fixed(v as i64),
                            });
                            PredRhs::Operand(slots.len() - 1)
                        })
                        .collect()
                }
            };
            // A window keeps the last `W` positions below each threshold — the column also at or past it less `W` — and a
            // chunk of `C` the positions of the row's own chunk: head `v` attending `n` keeps from column
            // `v·l + ⌊(n − 1)/C⌋·C`, `n` its threshold less `v·l`.
            let floors: Vec<Option<PredRhs>> = match (cache.window, &bound) {
                (None, _) => vec![None; h],
                (Some(w), Bound::Literal(p)) => (0..h)
                    .map(|v| {
                        let floor = v * l + w.first(p + heads.row_of(v)) - 1;
                        (floor > v * l).then_some(PredRhs::Const(floor))
                    })
                    .collect(),
                (Some(Window::Chunked(c)), Bound::Operand(_)) => {
                    let thr = match below[0] {
                        PredRhs::Operand(i) => slots[i].buf,
                        PredRhs::Const(_) => unreachable!("a computed bound's thresholds are an operand"),
                    };
                    let ix = index_type(target, cache.rows as u64)?;
                    let floor = f.t.scratch_deep("floors", 1, tiles_of(h, target), ix);
                    let c = c as f64;
                    let bands: Vec<(usize, TensorBody)> = (0..h)
                        .map(|v| {
                            let base = (v * l) as f64;
                            let n = sub(op(0), konst(base));
                            (v + 1, *add(konst(base), mul(Box::new(TensorBody::FloorDiv(sub(n, konst(1.0)), konst(c))), konst(c))))
                        })
                        .collect();
                    chain.push(f.compute(
                        "floors",
                        flat_space(h),
                        vec![Slot { buf: thr, map: ident_map(wide(tiles_of(h, target), target)) }],
                        ident_map(wide(tiles_of(h, target), target)),
                        banded(bands),
                        floor,
                    ));
                    (0..h)
                        .map(|v| {
                            slots.push(Slot { buf: floor, map: fixed(v as i64) });
                            Some(PredRhs::Operand(slots.len() - 1))
                        })
                        .collect()
                }
                (Some(Window::Sliding(w)), Bound::Operand(_)) => {
                    let thr = match below[0] {
                        PredRhs::Operand(i) => slots[i].buf,
                        PredRhs::Const(_) => unreachable!("a computed bound's thresholds are an operand"),
                    };
                    let ix = index_type(target, cache.rows as u64)?;
                    let floor = f.t.scratch_deep("floors", 1, tiles_of(h, target), ix);
                    chain.push(f.compute(
                        "floors",
                        flat_space(h),
                        vec![Slot { buf: thr, map: ident_map(wide(tiles_of(h, target), target)) }],
                        ident_map(wide(tiles_of(h, target), target)),
                        *sub(op(0), konst(w as f64)),
                        floor,
                    ));
                    (0..h)
                        .map(|v| {
                            slots.push(Slot { buf: floor, map: fixed(v as i64) });
                            Some(PredRhs::Operand(slots.len() - 1))
                        })
                        .collect()
                }
            };
            let bands: Vec<(usize, TensorBody)> = below
                .into_iter()
                .zip(floors)
                .enumerate()
                .map(|(v, (rhs, floor))| {
                    let kept = match floor {
                        Some(floor) => Box::new(TensorBody::Select {
                            cond: Pred::AxisGe(LV_GEN_COL, floor),
                            then: op(0),
                            otherwise: konst(MASKED),
                        }),
                        None => op(0),
                    };
                    (
                        (v + 1) * l,
                        TensorBody::Select {
                            cond: Pred::AxisLt(LV_GEN_COL, rhs),
                            then: kept,
                            otherwise: konst(MASKED),
                        },
                    )
                })
                .collect();
            chain.extend(f.pointwise("mask", n, slots, banded(bands), out_buf, target)?);
        }
        let claim = approx("attention scores", f.precision);
        f.finish(
            &scores_name,
            n,
            chain,
            out_buf,
            format!("{tag}_scores_{h}x{l}"),
            claim,
            target,
        )?
    };

    // ── Section 2: the row softmax over each head's `l` scores, each head's sink in its denominator. A decode row's
    // softmax rows are its heads; a causal block's rows repeat them, so the sinks are first spread over the rows.
    let sink_rows_name = format!("{}__sink_rows", names.out);
    let (sink_rows, spread) = match sinks.map(|i| names.operand(i)) {
        None => (None, None),
        Some(sk) if heads.rows == 1 => (Some(sk), None),
        Some(sk) => {
            let spread = spread_sinks(sk, heads.per_row, &sink_rows_name, h, &|v| Some(v % heads.per_row), target)?;
            (Some((sink_rows_name.as_str(), &Fill::Unknown)), Some(spread))
        }
    };
    let softmax = flat_softmax_rows_at(
        (&scores_name, &Fill::Unknown),
        &probs_name,
        h,
        l,
        format!("{tag}_softmax_{h}x{l}"),
        elem_precision(target)?,
        sink_rows,
        target,
    )?;

    // ── Section 3: the weighted V sum, `ctx[head·vd + e] = Σ_j p[head·l + j]·vc[j·(kv·vd) + kvh·vd + e]`.
    let weighted = {
        let n = h * vd;
        let mut f = FlatBuild::columns(n, target)?;
        let p = f.input(&probs_name, h * l, &Fill::Unknown, target);
        let v = f.input_held(v_name, v_len, v_fill, cache_precision(target)?, target);
        let precision = f.precision;
        let out_buf = f.result_page("out", n, precision, target);
        let tile_elems = target.tile.rows * target.tile.cols;
        let ones = f.t.tensor(
            "wv_scaler",
            TensorRole::Param,
            target.tile.rows,
            target.tile.cols,
            f.precision,
            TensorInit::FloatValues(vec![1.0f64.to_bits(); tile_elems]),
        );
        let ones_in = f.t.buffer("in_scaler", 1, f.precision);
        let ls = f.t.load("load_scaler", ones, vec![], 1, ones_in);
        f.loads.push((ls, ones_in));
        // The folded axis is the ROW (`j` over the cache); the flat output column is `c = head·vd + e`.
        let mut slots = vec![Slot {
            buf: ones_in,
            map: const_map(),
        }];
        let mut bands = Vec::new();
        for head in 0..h {
            let kvh = kv_of(head);
            // `p[head·l + j]`: constant along the output column, affine in the folded row.
            slots.push(Slot {
                buf: p,
                map: ElemAccess {
                    base: (head * l) as i64,
                    terms: vec![(LV_GEN_ROW, 1)],
                    indirect: None,
                },
            });
            let p_slot = slots.len() - 1;
            // `vc[j·(kv·vd) + kvh·vd + e]` with `e = c − head·vd`.
            let base = (kvh * vd) as i64 - (head * vd) as i64;
            slots.push(Slot {
                buf: v,
                map: ElemAccess {
                    base,
                    terms: vec![(LV_GEN_ROW, (kv * vd) as i64), (LV_GEN_COL, 1)],
                    indirect: None,
                },
            });
            let v_slot = slots.len() - 1;
            bands.push(((head + 1) * vd, *mul(op(p_slot), op(v_slot))));
        }
        let chain = f.fold("attn_weighted", n, (l, ReducePool::Sum), slots, banded(bands), out_buf, target)?;
        let claim = approx("attention weighted sum", f.precision);
        f.finish(
            &names.out,
            n,
            chain,
            out_buf,
            format!("{tag}_wv_{h}x{vd}"),
            claim,
            target,
        )?
    };
    Ok([score].into_iter().chain(spread).chain([softmax, weighted]).collect())
}

/// `C = lut attention(Q, K, V)` — an int8 model's single-query attention, integer throughout, in THREE sections: the
/// scores `s = q·k_j` over the cache; per head, the weights `w_j = E[clamp(((max − s_j)·M) >> S, 0, n − 1)]` for the
/// positions attended and 0 past them; and the context `⌊Σ_j w_j·v_j / Σ_j w_j⌋`. A masked position is fenced off
/// only where it could change an answer — the maximum and the weights — and the sums need no mask: a weight of 0 adds
/// nothing. Every value the masked step computes is one the unmasked step could: a masked position folds into the
/// maximum as the head's first score, which every `P ≥ 1` attends, so no sentinel widens what the maximum holds.
pub fn from_lut_attention(
    spec: &LutAttentionSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let (h, kv, hd, l, n) = (spec.h, spec.kv, spec.hd, spec.l, spec.table_len);
    if h == 0 || kv == 0 || hd == 0 || l == 0 || n == 0 || h % kv != 0 {
        return Err(SpatialReject::NotLowered(
            "a LUT attention with no heads, no head dimension, no cache or no table, or query heads that do not group over its K/V heads",
        ));
    }
    let group = h / kv;
    // The positions attended: the whole cache, a literal prefix of it, or — masked — the first `P` for a `P` the
    // program computes (its operand follows `Q, K, V, E`).
    let (attended, bound) = match spec.pos {
        None => (l, None),
        Some(RowIndex::Literal(p)) => {
            if p < 1 || p as usize > l {
                return Err(SpatialReject::NotLowered("a literal attention length outside its cache — the program fails at run time on every tier, so there is no section to lower"));
            }
            (p as usize, None)
        }
        Some(RowIndex::Named(_)) => (l, Some(4usize)),
    };
    let (q_name, q_fill) = names.operand(0);
    let (k_name, k_fill) = names.operand(1);
    let (v_name, v_fill) = names.operand(2);
    let (e_name, e_fill) = names.operand(3);
    let scores_name = format!("{}_scores", names.out);
    let weights_name = format!("{}_weights", names.out);
    let kv_len = l * kv * hd;
    let nl = h * attended;
    let tag = format!("lut_attn_{h}x{attended}");

    // ── Section 1: the scores, `h · attended` of them, flat: `s[head·attended + j] = Σ_e q[head·hd + e]·k[j, kvh, e]`.
    let score = {
        let mut f = FlatBuild::on(Datapath::Int, target)?;
        let q = f.input(q_name, h * hd, q_fill, target);
        let k = f.input(k_name, kv_len, k_fill, target);
        let out_buf = f.t.buffer("scores", tiles_of(nl, target), f.precision);
        let iters = vec![
            IterSpec {
                var: LV_GEN_ROW,
                extent: hd,
                kind: IterKind::Reduce {
                    pool: ReducePool::Sum,
                },
            },
            IterSpec {
                var: LV_GEN_COL,
                extent: nl,
                kind: IterKind::Parallel,
            },
        ];
        let mut slots = Vec::new();
        let mut bands = Vec::new();
        let j_stride = (kv * hd) as i64;
        for head in 0..h {
            let kvh = head / group;
            slots.push(Slot {
                buf: q,
                map: ElemAccess {
                    base: (head * hd) as i64,
                    terms: vec![(LV_GEN_ROW, 1)],
                    indirect: None,
                },
            });
            let q_slot = slots.len() - 1;
            slots.push(Slot {
                buf: k,
                map: ElemAccess {
                    base: (kvh * hd) as i64 - (head * attended) as i64 * j_stride,
                    terms: vec![(LV_GEN_COL, j_stride), (LV_GEN_ROW, 1)],
                    indirect: None,
                },
            });
            let k_slot = slots.len() - 1;
            bands.push(((head + 1) * attended, *mul(op(q_slot), op(k_slot))));
        }
        let s = f.compute("lut_scores", iters, slots, per_col_map(), banded(bands), out_buf);
        f.finish(&scores_name, nl, vec![s], out_buf, format!("{tag}_scores"), NumericsClaim::Exact, target)?
    };

    // ── Section 2: per head, the row maximum over the attended positions, the LUT index, and the weight.
    let weights = {
        let mut f = FlatBuild::on(Datapath::Int, target)?;
        let sc = f.input(&scores_name, nl, &Fill::Unknown, target);
        let e = f.input(e_name, n, e_fill, target);
        let one_w = wide(1, target);
        let flat_w = wide(tiles_of(nl, target), target);
        let mut chain = Vec::new();
        // When the bound is the program's: `P` checked to `1 ≤ P ≤ L` (or the run stops), and head `head`'s
        // threshold `head·attended + P` — its position `j` sits at flat column `head·attended + j`, live iff below it.
        let masked = match bound {
            Some(li) => {
                let (pn, pf) = names.operand(li);
                let ix = index_type(target, l as u64)?;
                let p_in = f.input_as(pn, 1, pf, ix, target);
                let (mut guard, pc) = checked_count(&mut f, p_in, l, ix, target);
                chain.append(&mut guard);
                let thr = f.t.scratch_deep("thresholds", 1, tiles_of(h, target), ix);
                let bands: Vec<(usize, TensorBody)> = (0..h)
                    .map(|head| (head + 1, *add(konst((head * attended) as f64), op(0))))
                    .collect();
                chain.push(f.compute(
                    "thresholds",
                    flat_space(h),
                    vec![Slot { buf: pc, map: fixed(0) }],
                    ident_map(wide(tiles_of(h, target), target)),
                    banded(bands),
                    thr,
                ));
                Some((pc, thr))
            }
            None => None,
        };
        // A body that holds at a live position and is `otherwise` past `P`, against the bound in slot `bound_slot`.
        let fenced = |bound_slot: Option<usize>, then: Box<TensorBody>, otherwise: Box<TensorBody>| -> TensorBody {
            match bound_slot {
                Some(bs) => TensorBody::Select {
                    cond: Pred::AxisLt(LV_GEN_COL, PredRhs::Operand(bs)),
                    then,
                    otherwise,
                },
                None => *then,
            }
        };
        // The row maximum: ONE FOLD PER HEAD, each a single row over its own slice of the flat scores — a page holds
        // `t` rows, and on a scalar-tile part that is one. The fold's column IS `j`, read against `P` itself.
        let mut rmax = Vec::with_capacity(h);
        for head in 0..h {
            let m = f.scratch(&format!("row_max_{head}"), 1);
            let mut slots = vec![Slot {
                buf: sc,
                map: at((head * attended) as i64),
            }];
            let p_slot = masked.map(|(pc, _)| {
                slots.push(Slot { buf: pc, map: fixed(0) });
                slots.len() - 1
            });
            // Past `P` the fold takes the head's first score again: attended at every `P ≥ 1`, never above the maximum.
            let first = masked.map(|_| {
                slots.push(Slot {
                    buf: sc,
                    map: fixed((head * attended) as i64),
                });
                slots.len() - 1
            });
            chain.push(f.compute(
                &format!("row_max_{head}"),
                head_space(1, attended, Some(ReducePool::Max)),
                slots,
                per_row_map(one_w),
                fenced(p_slot, op(0), first.map_or_else(|| konst(0.0), op)),
                m,
            ));
            rmax.push(m);
        }
        // `idx[head·attended + j] = clamp(((max_head − s_j)·M) >> S, 0, n − 1)`, 0 past `P`.
        let idx = f.scratch("lut_index", tiles_of(nl, target));
        let mut slots = vec![Slot { buf: sc, map: at(0) }];
        let mut bands = Vec::new();
        for (head, &m) in rmax.iter().enumerate() {
            slots.push(Slot { buf: m, map: fixed(0) });
            let mx = slots.len() - 1;
            let thr_slot = masked.map(|(_, thr)| {
                slots.push(Slot {
                    buf: thr,
                    map: fixed(head as i64),
                });
                slots.len() - 1
            });
            let index = Box::new(TensorBody::Min(
                Box::new(TensorBody::Max(
                    Box::new(TensorBody::Shr {
                        arg: mul(sub(op(mx), op(0)), konst(spec.m as f64)),
                        by: spec.s,
                    }),
                    konst(0.0),
                )),
                konst((n - 1) as f64),
            ));
            bands.push(((head + 1) * attended, fenced(thr_slot, index, konst(0.0))));
        }
        chain.push(f.compute(
            "lut_index",
            flat_space(nl),
            slots,
            ident_map(flat_w),
            banded(bands),
            idx,
        ));
        // `w = E[idx]` at a live position, 0 past `P`: a weight of 0 adds nothing to either sum.
        let out_buf = f.t.buffer("weights", tiles_of(nl, target), f.precision);
        let mut slots = vec![
            Slot { buf: idx, map: at(0) },
            Slot {
                buf: e,
                map: ElemAccess {
                    base: 0,
                    terms: Vec::new(),
                    indirect: Some(IndirectTerm {
                        operand: 0,
                        stride: 1,
                        modulus: n,
                    }),
                },
            },
        ];
        let mut bands = Vec::new();
        for head in 0..h {
            let thr_slot = masked.map(|(_, thr)| {
                slots.push(Slot {
                    buf: thr,
                    map: fixed(head as i64),
                });
                slots.len() - 1
            });
            bands.push(((head + 1) * attended, fenced(thr_slot, op(1), konst(0.0))));
        }
        chain.push(f.compute(
            "lut_weight",
            flat_space(nl),
            slots,
            ident_map(flat_w),
            banded(bands),
            out_buf,
        ));
        f.finish(&weights_name, nl, chain, out_buf, format!("{tag}_weights"), NumericsClaim::Exact, target)?
    };

    // ── Section 3: the context, `ctx[head·hd + e] = ⌊Σ_j w[head·attended + j]·v[j, kvh, e] / Σ_j w[head·attended + j]⌋`.
    let context = {
        let mut f = FlatBuild::on(Datapath::Int, target)?;
        let w = f.input(&weights_name, nl, &Fill::Unknown, target);
        let v = f.input(v_name, kv_len, v_fill, target);
        let one_w = wide(1, target);
        let nh = h * hd;
        let mut chain = Vec::new();
        // One denominator fold per head, as the maxima are.
        let mut den = Vec::with_capacity(h);
        for head in 0..h {
            let d = f.scratch(&format!("den_{head}"), 1);
            chain.push(f.compute(
                &format!("den_{head}"),
                head_space(1, attended, Some(ReducePool::Sum)),
                vec![Slot {
                    buf: w,
                    map: at((head * attended) as i64),
                }],
                per_row_map(one_w),
                TensorBody::Operand(0),
                d,
            ));
            den.push(d);
        }
        let num = f.scratch("num", tiles_of(nh, target));
        let iters = vec![
            IterSpec {
                var: LV_GEN_ROW,
                extent: attended,
                kind: IterKind::Reduce {
                    pool: ReducePool::Sum,
                },
            },
            IterSpec {
                var: LV_GEN_COL,
                extent: nh,
                kind: IterKind::Parallel,
            },
        ];
        let mut slots = Vec::new();
        let mut bands = Vec::new();
        for head in 0..h {
            let kvh = head / group;
            slots.push(Slot {
                buf: w,
                map: ElemAccess {
                    base: (head * attended) as i64,
                    terms: vec![(LV_GEN_ROW, 1)],
                    indirect: None,
                },
            });
            let w_slot = slots.len() - 1;
            slots.push(Slot {
                buf: v,
                map: ElemAccess {
                    base: (kvh * hd) as i64 - (head * hd) as i64,
                    terms: vec![(LV_GEN_ROW, (kv * hd) as i64), (LV_GEN_COL, 1)],
                    indirect: None,
                },
            });
            let v_slot = slots.len() - 1;
            bands.push(((head + 1) * hd, *mul(op(w_slot), op(v_slot))));
        }
        chain.push(f.compute("num", iters, slots, per_col_map(), banded(bands), num));
        let out_buf = f.t.buffer("out", tiles_of(nh, target), f.precision);
        let mut slots = vec![Slot { buf: num, map: at(0) }];
        let mut bands = Vec::new();
        for (head, &d) in den.iter().enumerate() {
            slots.push(Slot { buf: d, map: fixed(0) });
            let ds = slots.len() - 1;
            bands.push(((head + 1) * hd, TensorBody::FloorDiv(op(0), op(ds))));
        }
        chain.push(f.compute(
            "floor_div",
            flat_space(nh),
            slots,
            ident_map(wide(tiles_of(nh, target), target)),
            banded(bands),
            out_buf,
        ));
        f.finish(&names.out, nh, chain, out_buf, format!("{tag}_context"), NumericsClaim::Exact, target)?
    };
    Ok(vec![score, weights, context])
}

/// `C = full attention(Q, K, V)` over `t` positions and `h` heads — `t` query rows of `h` heads, each reading its
/// own K/V head and attending over all `t` positions. Unmasked, as the vision tower's block is: every row sees
/// every position.
pub fn from_full_attention(
    spec: &FullAttentionSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    full_attention_as(spec, names, full_attention_form(spec, target), target)
}

/// [`from_full_attention`] in the form `form`.
pub(super) fn full_attention_as(
    spec: &FullAttentionSpec,
    names: &StageNames,
    form: HeadForm,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let (t, h, hd) = (spec.t, spec.h, spec.hd);
    if t == 0 || h == 0 || hd == 0 {
        return Err(SpatialReject::NotLowered(
            "a full attention with no positions, no heads or no head dimension",
        ));
    }
    let heads = QueryHeads {
        rows: t,
        per_row: h,
        group: 1,
    };
    if form != HeadForm::Flat {
        return flash_attention(
            names,
            FlashGeometry {
                q: heads,
                kvw: h,
                hd,
                vd: hd,
                v_part: None,
                rows: t,
                l: t,
                limit: None,
                window: None,
                sinks: None,
                grouped: false,
                scale: 1.0 / (hd as f64).sqrt(),
                cap: None,
            },
            "full_attn",
            target,
        );
    }
    attention_sections(
        names,
        heads,
        HeadWidths { kv: h, hd, vd: hd, scale: 1.0 / (hd as f64).sqrt(), cap: None },
        Cache {
            rows: t,
            attended: t,
            limit: None,
            window: None,
        },
        None,
        "full_attn",
        target,
    )
}

// ── Flash attention: the real-width form ──────────────────────────────────────────────────────────────
//
// ★ AN ATTENTION HEAD NEVER FITS A SITE AT REAL LENGTH, SO IT STREAMS. The cache streams past each item in BLOCKS —
// one tile row of positions of the item's K/V head — as the K blocks of a FOLD ([`FoldLevel`]): each block's scores
// `S = Q·Kᵀ/√hd` fold into a running softmax that never leaves the site, `m' = max(m, rowmax S)`, `α = e^(m − m')`,
// `P = e^(S − m')`, `s' = α·s + rowsum P`, `O' = α·O + P·V`. The rescale is the identity `e^(x−m) = e^(x−m')·
// e^(m−m')`, so nothing is approximated, and the epilogue's `O / s` is the attention. An item is a TILE of query
// rows of one head — FlashAttention-2's outer loop; one row, for a decode step — so the rows of a tile share every
// block they read, and nothing but the attention itself is ever stored.
//
// ★ A BLOCK IS ONE TILE ROW OF THE CACHE. The cache is dense tiles, `rows × (kvw·hd)`: block `b` of K/V head `x` is
// tile row `b`'s `hd / tile` tiles from `x·hd`, an address affine in the block. Row `r` of query tile `q` sees the
// positions below its limit — `P + q·tile + r` for a causal block, `P` for a decode row, `l` for a full attention —
// so block `b` masks its positions from `limit − start` on, `start = b·tile` carried from block to block as a
// counter. A tail block's positions past `l` fall to the same limit.
//
// ★ THE RUNNING STATE KEEPS THE ACCUMULATOR'S WIDTH. `m` is an exponent shift: rounded to bf16 it would move every
// weight of the row by up to 1%. `m`, `s` and `O` are fp32 carries; only the attention leaves at the datapath's
// precision.

/// Where a flash attention's query heads sit. Item `(v, x)` owns query head `v` of [`QueryHeads`] `q` and reads its
/// K/V head `x` of a cache of `rows` positions holding `kvw` heads each; a head is `hd` wide. The blocks cover the
/// first `l` positions, and `limit` masks query row `r`'s positions from `P + r` on.
///
/// `grouped`: the queries are ONE row regrouped by K/V head ([`grouped_decode_attention`]) — a `(kvw · tile) × hd`
/// matrix whose tile row `x` holds K/V head `x`'s query heads, one per row. Item `x` is then K/V head `x`, and every
/// row is limited by the one query row's `P`.
#[derive(Clone, Copy, Debug)]
pub(super) struct FlashGeometry {
    q: QueryHeads,
    kvw: usize,
    hd: usize,
    /// The values' width a head: the keys' `hd`, or narrower (a latent attention's values are its keys' latent part).
    vd: usize,
    /// The value pages this fold computes, `(first, count)`, when not all of them: a fold too large for a site is one
    /// fold per page ([`flash_attention`]).
    v_part: Option<(usize, usize)>,
    rows: usize,
    l: usize,
    limit: Option<Bound>,
    /// A window: row `r` keeps only the last `W` of the positions below its limit, or those of its own chunk of `C`.
    window: Option<Window>,
    /// The operand holding one sink logit per query head, which joins each head's softmax and takes no value.
    sinks: Option<usize>,
    grouped: bool,
    /// The factor every score takes.
    scale: f64,
    /// The soft cap every scaled score takes before its block's maximum, `c·tanh(s/c)`.
    cap: Option<f64>,
}

/// Whether keys `hd` and values `vd` wide take the flash form on `target`: whole tiles. A key of any width is one
/// contraction into a tile of scores; values wider than an accumulator page fold a page at a time.
pub(super) fn flash_applies(hd: usize, vd: usize, target: &SpatialTarget) -> bool {
    hd % target.tile.cols == 0 && vd % target.tile.cols == 0 && hd > 0 && vd > 0
}

/// Whether heads that are not whole tiles take the flash form over their windows on `target` ([`flash_fold`]): keys and
/// values one width, a square tile, and a head's values one accumulator page.
pub(super) fn windows_apply(hd: usize, vd: usize, target: &SpatialTarget) -> bool {
    hd == vd && hd > 0 && target.tile.rows == target.tile.cols && vd.div_ceil(target.tile.cols) <= row_block_tiles(target)
}

/// How an attention's heads lower on `target`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HeadForm {
    /// Heads of whole tiles: the flash fold.
    Flash,
    /// Heads that start or end inside a tile, where the three flat sections fit no site: the flash fold over each head's
    /// window of tiles, its heads packed into the result after it ([`pack_heads`]).
    Windowed,
    /// The three sections, every operand a flat row loaded whole ([`attention_sections`]).
    Flat,
}

/// ★ A HEAD THAT IS NOT WHOLE TILES STAYS FLAT WHERE A SITE HOLDS IT, AND STREAMS WHERE NONE DOES. The three flat sections
/// load the cache whole into each site, which a small cache fits and a real one never does; past that, the cache streams
/// past the flash fold in blocks, each head read through its window. `flat` builds the three sections, measured as built,
/// so the choice cannot disagree with the sections it judges.
pub(super) fn head_form(
    hd: usize,
    vd: usize,
    flat: impl FnOnce() -> Result<Vec<SpatialProgram>, SpatialReject>,
    target: &SpatialTarget,
) -> HeadForm {
    if flash_applies(hd, vd, target) {
        HeadForm::Flash
    } else if windows_apply(hd, vd, target) && !flat().is_ok_and(|ps| ps.iter().all(|p| fits_a_site(p, target))) {
        HeadForm::Windowed
    } else {
        HeadForm::Flat
    }
}

/// Names for an attention's sections built only to be measured: each operand a placeholder of no known fill.
pub(super) fn measured_names() -> StageNames {
    StageNames {
        out: "attention".into(),
        ins: ["q", "k", "v", "position", "sinks"].map(String::from).to_vec(),
        fills: vec![Fill::Unknown; 5],
    }
}

/// The form [`from_attention`] lowers `spec` in on `target`.
pub(super) fn attention_form(spec: &AttentionSpec, target: &SpatialTarget) -> HeadForm {
    if spec.kv == 0 || spec.h % spec.kv != 0 {
        return HeadForm::Flat;
    }
    head_form(spec.hd, spec.vd, || attention_as(spec, &measured_names(), HeadForm::Flat, target), target)
}

/// The form [`from_full_attention`] lowers `spec` in on `target`.
pub(super) fn full_attention_form(spec: &FullAttentionSpec, target: &SpatialTarget) -> HeadForm {
    head_form(spec.hd, spec.hd, || full_attention_as(spec, &measured_names(), HeadForm::Flat, target), target)
}

/// Whether an attention reads its K and V as DENSE tiles on `target` — the flash form's cache layout, over whole heads or
/// over their windows. A table an attention reads dense is written dense too, so this is the one place the two agree.
pub fn attention_reads_dense(spec: &AttentionSpec, target: &SpatialTarget) -> bool {
    attention_form(spec, target) != HeadForm::Flat
}

/// [`attention_reads_dense`] for a full attention, whose query heads read K/V heads one to one.
pub fn full_attention_reads_dense(spec: &FullAttentionSpec, target: &SpatialTarget) -> bool {
    full_attention_form(spec, target) != HeadForm::Flat
}

/// The mask value a masked score takes: far below any real score, and still finite in every float format.
pub(super) const MASKED: f64 = -1.0e30;

/// The fan-ins of a merge tree over `nb` partials, bottom level first: the fewest levels any fan-in up to `fmax`
/// allows, and at each level the SMALLEST fan-in that still reaches one partial in the levels left — so the tree
/// pads as little as its depth allows. Their product is the partial slot count, `≥ nb`.
pub(super) fn merge_fans(nb: usize, fmax: usize) -> Vec<usize> {
    if nb <= 1 {
        return Vec::new();
    }
    let reaches = |f: usize, k: u32, r: usize| f.checked_pow(k).is_none_or(|p| p >= r);
    let mut levels = 1u32;
    while !reaches(fmax, levels, nb) {
        levels += 1;
    }
    let mut fans = Vec::new();
    let mut r = nb;
    for k in (1..=levels).rev() {
        let f = (2..=fmax).find(|f| reaches(*f, k, r)).unwrap_or(fmax);
        fans.push(f);
        r = r.div_ceil(f);
    }
    fans
}

/// `P`, checked to be in `1..=rows`, from a one-element count operand — a guard of stride zero on `P − 1`, the
/// same range check a row index gets. Returns the stages and the buffer holding the checked `P`.
pub(super) fn checked_count(
    f: &mut FlatBuild,
    count: BufferId,
    rows: usize,
    ix: Precision,
    target: &SpatialTarget,
) -> (Vec<StageId>, BufferId) {
    let one = wide(1, target);
    let biased = f.t.scratch_deep("count_biased", 1, 1, ix);
    let s0 = f.compute(
        "count_biased",
        flat_space(1),
        vec![Slot {
            buf: count,
            map: fixed(0),
        }],
        ident_map(one),
        *sub(op(0), konst(1.0)),
        biased,
    );
    let checked = f.t.scratch_deep("count_checked", 1, 1, ix);
    let guard = ElemAccess {
        base: 0,
        terms: Vec::new(),
        indirect: Some(IndirectTerm {
            operand: 0,
            stride: 0,
            modulus: rows,
        }),
    };
    let s1 = f.compute(
        "count_checked",
        flat_space(1),
        vec![Slot {
            buf: biased,
            map: guard,
        }],
        ident_map(one),
        *add(op(0), konst(1.0)),
        checked,
    );
    (vec![s0, s1], checked)
}

pub(super) fn flash_scaler_name(names: &StageNames) -> String {
    format!("{}__flash_scaler", names.out)
}

/// The flash fold of an attention: ONE section, an item per query tile and head, the cache streaming past as its
/// K blocks.
/// A flash fold's query layout: the query rows it holds, the heads a row carries, the query tile rows, and its items.
pub(super) fn fold_items(g: &FlashGeometry, target: &SpatialTarget) -> (usize, usize, usize, usize) {
    let tr = target.tile.rows;
    let (rows, per_row) = if g.grouped { (g.kvw * tr, 1) } else { (g.q.rows, g.q.per_row) };
    let tiles_q = rows.div_ceil(tr);
    (rows, per_row, tiles_q, tiles_q * per_row)
}

/// Where a windowed flash fold's heads sit — heads that are not whole tiles. Head `v` of a row of `n` heads `w` wide holds
/// columns `v·w … v·w + w − 1`, which may start inside a tile, so an item reads the RUN of tiles they fall in and the head
/// from its offset in the first. The run is the most tiles any head of the row straddles; a window starts at its head's
/// first tile, or `run` tiles before the row's end where that would pass it, so every window lies inside its row and every
/// offset leaves the head inside its window.
#[derive(Clone, Debug)]
pub(super) struct FoldWindows {
    /// The tiles a query head's window takes.
    q_run: usize,
    /// The tiles a K/V head's window takes, and a row of the cache.
    kv_run: usize,
    kv_row_t: usize,
    /// Each item's windows, by item.
    at: Vec<ItemWindows>,
}

/// One item's windows: the tile of the dense queries its query head's window starts at and the head's offset in it, and
/// the tile of a cache row its K/V head's window starts at and that head's offset in it.
#[derive(Clone, Copy, Debug)]
pub(super) struct ItemWindows {
    q_first: usize,
    q_offset: usize,
    kv_first: usize,
    kv_offset: usize,
}

impl FoldWindows {
    fn of(g: &FlashGeometry, target: &SpatialTarget) -> Result<FoldWindows, SpatialReject> {
        if !windows_apply(g.hd, g.vd, target) {
            return Err(SpatialReject::NotLowered("a flash fold over heads that are not whole tiles, of keys and values two widths, on a tile that is not square, or of values wider than an accumulator page"));
        }
        let tw = target.tile.cols;
        let (_, per_row, _, items) = fold_items(g, target);
        let q_row_t = (per_row * g.hd).div_ceil(tw);
        let kv_row_t = (g.kvw * g.hd).div_ceil(tw);
        let (q_run, kv_run) = (window_run(per_row, g.hd, tw), window_run(g.kvw, g.hd, tw));
        let at = (0..items)
            .map(|y| {
                let (qt, v) = (y / per_row, y % per_row);
                let (q_first, q_offset) = head_window(v * g.hd, q_row_t, q_run, tw);
                // A regrouped item is its K/V head, its queries that head's tile row.
                let x = if g.grouped { y } else { g.q.kv_of(v) };
                let (kv_first, kv_offset) = head_window(x * g.hd, kv_row_t, kv_run, tw);
                ItemWindows { q_first: qt * q_row_t + q_first, q_offset, kv_first, kv_offset }
            })
            .collect();
        Ok(FoldWindows { q_run, kv_run, kv_row_t, at })
    }

    /// The largest entry of the items' table.
    fn largest(&self) -> usize {
        self.at.iter().map(|a| a.q_first.max(a.q_offset).max(a.kv_offset)).max().unwrap_or(0)
    }
}

/// The tiles the window of a head `w` wide takes in a row of `n` such heads: the most any of them straddles, at most the
/// row.
pub(super) fn window_run(n: usize, w: usize, tw: usize) -> usize {
    (0..n).map(|v| ((v * w) % tw + w).div_ceil(tw)).max().unwrap_or(1).min((n * w).div_ceil(tw))
}

pub(super) fn gcd(a: usize, b: usize) -> usize {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// The window of `run` tiles a head whose first column is `c0` is read through, in a row of `row_t` tiles: its first tile,
/// and the head's offset in it.
pub(super) fn head_window(c0: usize, row_t: usize, run: usize, tw: usize) -> (usize, usize) {
    let first = (c0 / tw).min(row_t - run);
    (first, c0 - first * tw)
}

/// The table a windowed flash fold reads each item's windows from: two tiles an item, its queries' first tile in element 0
/// of the first, its queries' and its K/V head's offsets in elements 0 and 1 of the second.
pub(super) fn flash_windows_name(names: &StageNames) -> String {
    format!("{}__flash_windows", names.out)
}

/// The result a windowed flash fold writes, each head in whole tiles of its own: [`pack_heads`] packs it into the
/// attention's.
pub(super) fn flash_heads_name(names: &StageNames) -> String {
    format!("{}__heads", names.out)
}

/// The tensor a flash fold reads its rows' sinks from: one tile per sink tile ([`sink_tiles`]), row `r`'s sink in
/// element `r`.
pub(super) fn flash_sinks_name(names: &StageNames) -> String {
    format!("{}__flash_sinks", names.out)
}

/// The compile-time table naming each item's query head, whose sink tile the item reads ([`sink_tiles`]).
pub(super) fn flash_sink_heads_name(names: &StageNames) -> String {
    format!("{}__flash_sink_heads", names.out)
}

/// ★ A SINK IS ITS QUERY HEAD'S, WHATEVER THE ROW. A fold of one query tile has an item per head, each reading its own
/// sink tile; a regrouped decode an item per K/V head, its query heads down the tile. A fold of several query tiles has
/// an item per tile and head, and its sinks are one tile per head — `per_row`, never the chunk's rows — each item reading
/// its head's through the index a compile-time table gives it. Returns the sink tiles and whether the items read them
/// through that table.
pub(super) fn sink_tiles(g: &FlashGeometry, target: &SpatialTarget) -> (usize, bool) {
    let (_, per_row, tiles_q, items) = fold_items(g, target);
    if g.grouped || tiles_q == 1 {
        (items, false)
    } else {
        (per_row, true)
    }
}

pub(super) fn flash_fold(
    names: &StageNames,
    g: FlashGeometry,
    tag: &str,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    paged(|pages| flash_fold_paged(names, g, tag, target, pages), target)
}

/// [`flash_fold`] with `pages` pages in each streamed key and value page and in the output page.
pub(super) fn flash_fold_paged(
    names: &StageNames,
    g: FlashGeometry,
    tag: &str,
    target: &SpatialTarget,
    pages: usize,
) -> Result<SpatialProgram, SpatialReject> {
    let (tr, tw) = (target.tile.rows, target.tile.cols);
    let head_t = g.hd / tw;
    let (rows, per_row, tiles_q, items) = fold_items(&g, target);
    let width = per_row * g.hd;
    let width_t = width.div_ceil(tw);
    // ★ A HEAD THAT IS NOT WHOLE TILES IS READ THROUGH ITS WINDOW: the run of tiles its columns fall in, from an offset
    // inside the first ([`FoldWindows`]). An item's queries' window starts where a compile-time table says; its K/V
    // head's window starts at the tile its coordinate carries, so the cache still streams by affine addresses.
    let windows = (g.hd % tw != 0 || g.vd % tw != 0).then(|| FoldWindows::of(&g, target)).transpose()?;
    let q_t = windows.as_ref().map_or(head_t, |w| w.q_run);
    // ★ THE VALUES FOLD AN ACCUMULATOR PAGE AT A TIME. A row's running output is `vd` wide; past a page it is `nv`
    // blocks of `vt` tiles, each its own carry, block product, update and normalization, all sharing one row's scores,
    // maximum and sum.
    let v_t = g.vd.div_ceil(tw);
    let vt = row_block(v_t, target);
    let (j0, nv) = g.v_part.unwrap_or((0, v_t / vt));
    // The fold's own part of a head's values: `nv` pages from page `j0`.
    let part_t = nv * vt;
    // A windowed head is stored in whole tiles of its own; one head a row is the row itself, the same tiles.
    let out_width = if windows.is_some() && per_row == 1 { g.vd } else { per_row * part_t * tw };
    let out_width_t = per_row * part_t;
    let part = |name: &str, j: usize| if nv == 1 { name.to_string() } else { format!("{name}{j}") };
    // The columns of a value page an item computes: the page, or a windowed head's `vd`.
    let o_w = if windows.is_some() { g.vd } else { vt * tw };
    let nb = g.l.div_ceil(tr);
    let precision = elem_precision(target)?;
    let acc = AccPolicy::F32.interm_precision();
    let ix = index_type(target, (g.rows + tiles_q * tr) as u64)?;
    let (q_name, q_fill) = names.operand(0);
    let (k_name, k_fill) = names.operand(1);
    let (v_name, v_fill) = names.operand(2);
    let kv_len = g.rows * g.kvw * g.hd;
    let v_len = g.rows * g.kvw * g.vd;
    let scale = g.scale;
    let (w1, wd) = (wide(1, target), wide(vt, target));
    let mut t = Template::new(target)?;
    let scaler = t.tensor(
        &flash_scaler_name(names),
        TensorRole::Param,
        tr,
        tw,
        precision,
        TensorInit::ConstF(1.0f64.to_bits()),
    );
    // The queries and the result are DENSE — a tile row holds `tile` query rows of every head.
    let q = t.tensor(
        q_name,
        TensorRole::Input,
        rows,
        width,
        precision,
        crate::spatial::passes::from_tensor_op::init_of(q_fill, rows * width),
    );
    // The cache is held at the products' float and widens into the fold's pages on the way in.
    let cache = cache_precision(target)?;
    let k = t.tensor(
        k_name,
        TensorRole::Input,
        g.rows,
        g.kvw * g.hd,
        cache,
        crate::spatial::passes::from_tensor_op::init_of(k_fill, kv_len),
    );
    let v = t.tensor(
        v_name,
        TensorRole::Input,
        g.rows,
        g.kvw * g.vd,
        cache,
        crate::spatial::passes::from_tensor_op::init_of(v_fill, v_len),
    );
    let c = t.tensor(
        &names.out,
        TensorRole::Output,
        rows,
        out_width,
        precision,
        TensorInit::Zero,
    );
    let generic =
        |iters: Vec<IterSpec>, ins: Vec<ElemAccess>, out: ElemAccess, body: TensorBody| {
            StageKind::Compute(ComputeOp::Generic {
                iters,
                ins,
                out,
                body,
            })
        };
    let m2 = |terms: &[(LoopVar, i64)]| ElemAccess {
        base: 0,
        terms: terms.to_vec(),
        indirect: None,
    };
    let contract = |r: usize, c: usize, k: usize| {
        vec![
            IterSpec {
                var: LV_GEN_ROW,
                extent: r,
                kind: IterKind::Parallel,
            },
            IterSpec {
                var: LV_GEN_COL,
                extent: c,
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

    // ── The prologue, once per item: the scaler, the query tile, and each row's limit.
    let sc_in = t.buffer_deep("in_scaler", 1, 1, precision);
    let q_in = t.buffer_deep("in_q", 1, q_t, precision);
    let l_sc = t.load_block_at("load_scaler", scaler, 0, Vec::new(), 1, 1, 1, sc_in);
    // Item `y` is tile row `y / per_row`, head `y mod per_row` of the dense queries: linear tile `y·head_t`. A windowed
    // item reads two tiles of its table: its queries' first tile, then the offsets of its queries and its K/V head.
    let (l_q, offsets) = match &windows {
        None => (t.load_block_at("load_q", q, 0, vec![(LV_ITEM_Y, head_t)], 1, head_t, width_t, q_in), None),
        Some(w) => {
            let wx = index_type(target, w.largest() as u64)?;
            let entries: Vec<i64> = w
                .at
                .iter()
                .flat_map(|a| {
                    let first = std::iter::once(a.q_first as i64).chain(std::iter::repeat_n(0, tw - 1));
                    let shifts = [a.q_offset as i64, a.kv_offset as i64].into_iter().chain(std::iter::repeat_n(0, tw - 2));
                    first.chain(shifts)
                })
                .collect();
            let table = t.tensor(&flash_windows_name(names), TensorRole::Param, 1, items * 2 * tw, wx, TensorInit::Values(entries));
            let first_in = t.buffer_deep("in_q_first", 1, 1, wx);
            let l_first = t.load_block_at("load_q_first", table, 0, vec![(LV_ITEM_Y, 2)], 1, 1, 1, first_in);
            let off_in = t.buffer_deep("in_head_offsets", 1, 1, wx);
            let l_off = t.load_block_at("load_head_offsets", table, 1, vec![(LV_ITEM_Y, 2)], 1, 1, 1, off_in);
            let firsts = w.at.iter().map(|a| a.q_first);
            let bound = (firsts.clone().min().unwrap_or(0), firsts.max().unwrap_or(0));
            let l_q = t.stage(
                "load_q",
                StageKind::Load {
                    tensor: q,
                    pattern: AccessPattern {
                        origin: TileAddr {
                            base: 0,
                            steps: Vec::new(),
                            indirect: Some(IndirectTile { index: first_in, tile_stride: 1, elems_per_tile: 1, bound }),
                        },
                        block_h: 1,
                        block_w: q_t,
                        stride_w: 1,
                        stride_h: width_t,
                    },
                    num_blocks: 1,
                    multicast: None,
                },
                vec![first_in],
                vec![q_in],
            );
            t.edge(l_first, first_in, l_q);
            (l_q, Some((l_off, off_in)))
        }
    };
    // ★ A MASK ONLY WHERE A POSITION CAN FALL PAST A LIMIT: a causal or decode bound, or a cache that ends inside a
    // tile row. Its block start moves only when there is more than one block — one block starts at 0, and its limit is
    // the row's own.
    let masked = g.limit.is_some() || g.l % tr != 0 || g.window.is_some();
    let counted = masked && nb > 1;
    let row_lim = masked.then(|| t.scratch_deep("row_limit", 1, 1, ix));
    // The input holding `P`, when the program computes it: the fold's blocks then run to the position.
    let mut position: Option<TensorId> = None;
    // ★ ROW `r` OF QUERY TILE `q` SEES THE POSITIONS BELOW `P + q·tile + r`. `P` is checked first when the program
    // computes it; `q·tile` comes from a compile-time table, one tile per item; `r` from a tile holding `r` in row `r`.
    // A full attention's rows all see its `l` positions.
    if let Some(row_lim) = row_lim {
        match g.limit {
            None => {
                t.stage(
                    "row_limit",
                    generic(
                        head_space(tr, 1, None),
                        Vec::new(),
                        ident_map(w1),
                        *konst(g.l as f64),
                    ),
                    Vec::new(),
                    vec![row_lim],
                );
            }
            Some(bound) => {
                let (mut ins, mut maps, mut feeds) = (Vec::new(), Vec::new(), Vec::new());
                let p = match bound {
                    Bound::Literal(p) => konst(p as f64),
                    Bound::Operand(li) => {
                        let (pn, pf) = names.operand(li);
                        let pt = t.tensor(
                            pn,
                            TensorRole::Input,
                            1,
                            1,
                            ix,
                            crate::spatial::passes::from_tensor_op::init_of(pf, 1),
                        );
                        position = Some(pt);
                        let p_in = t.buffer_deep(&format!("in_{pn}"), 1, 1, ix);
                        let l_p = t.load_block_at(
                            &format!("load_{pn}"),
                            pt,
                            0,
                            Vec::new(),
                            1,
                            1,
                            1,
                            p_in,
                        );
                        let biased = t.scratch_deep("count_biased", 1, 1, ix);
                        let s_b = t.stage(
                            "count_biased",
                            generic(
                                flat_space(1),
                                vec![fixed(0)],
                                ident_map(w1),
                                *sub(op(0), konst(1.0)),
                            ),
                            vec![p_in],
                            vec![biased],
                        );
                        t.edge(l_p, p_in, s_b);
                        let guard = ElemAccess {
                            base: 0,
                            terms: Vec::new(),
                            indirect: Some(IndirectTerm {
                                operand: 0,
                                stride: 0,
                                modulus: bound_max(g.rows, g.q.rows),
                            }),
                        };
                        ins.push(biased);
                        maps.push(guard);
                        feeds.push((s_b, biased));
                        add(op(0), konst(1.0))
                    }
                };
                // A regrouped tile's rows are heads of the one query row: every row's limit is `P`.
                let limit = if g.grouped {
                    p
                } else {
                    let tile_rows: Vec<i64> = (0..items)
                        .flat_map(|y| {
                            std::iter::once(((y / per_row) * tr) as i64)
                                .chain(std::iter::repeat_n(0, tw - 1))
                        })
                        .collect();
                    let tiles_t = t.tensor(
                        &format!("{}__flash_tile_rows", names.out),
                        TensorRole::Param,
                        1,
                        items * tw,
                        ix,
                        TensorInit::Values(tile_rows),
                    );
                    let q_row = t.buffer_deep("in_tile_row", 1, 1, ix);
                    let l_qr = t.load_block_at(
                        "load_tile_row",
                        tiles_t,
                        0,
                        vec![(LV_ITEM_Y, 1)],
                        1,
                        1,
                        1,
                        q_row,
                    );
                    ins.push(q_row);
                    maps.push(fixed(0));
                    feeds.push((l_qr, q_row));
                    let q_row_op = op(ins.len() - 1);
                    let row_index = t.tensor(
                        &format!("{}__flash_row_index", names.out),
                        TensorRole::Param,
                        tr,
                        tw,
                        ix,
                        TensorInit::Values((0..tr * tw).map(|e| (e / tw) as i64).collect()),
                    );
                    let r_in = t.buffer_deep("in_row_index", 1, 1, ix);
                    let l_r =
                        t.load_block_at("load_row_index", row_index, 0, Vec::new(), 1, 1, 1, r_in);
                    ins.push(r_in);
                    maps.push(per_row_map(w1));
                    feeds.push((l_r, r_in));
                    let r_op = op(ins.len() - 1);
                    add(add(p, q_row_op), r_op)
                };
                let s = t.stage(
                    "row_limit",
                    generic(head_space(tr, 1, None), maps, ident_map(w1), *limit),
                    ins,
                    vec![row_lim],
                );
                for (from, buf) in feeds {
                    t.edge(from, buf, s);
                }
            }
        }
    }
    // The carried state, and its first page: the empty softmax (`m = −10³⁰`, `s = 0`, `O = 0`) at block start 0 — or,
    // with sinks, each row's sink as its maximum and a sum of one.
    let m_run = t.scratch_deep("run_max", 2, 1, acc);
    let s_run = t.scratch_deep("run_sum", 2, 1, acc);
    let o_runs: Vec<BufferId> = (0..nv).map(|j| t.scratch_deep(&part("run_out", j), 2, vt, acc)).collect();
    match g.sinks {
        None => {
            t.stage(
                "init_max",
                generic(head_space(tr, tw, None), Vec::new(), ident_map(w1), *konst(MASKED)),
                Vec::new(),
                vec![m_run],
            );
        }
        Some(_) => {
            let (sink_t, by_head) = sink_tiles(&g, target);
            let sinks = t.tensor(
                &flash_sinks_name(names),
                TensorRole::Input,
                1,
                sink_t * tw,
                precision,
                crate::spatial::passes::from_tensor_op::init_of(&Fill::Unknown, sink_t * tw),
            );
            let sk_in = t.buffer_deep("in_sinks", 1, 1, precision);
            let l_sk = if by_head {
                let hx = index_type(target, per_row as u64)?;
                let entries: Vec<i64> =
                    (0..items).flat_map(|y| std::iter::once((y % per_row) as i64).chain(std::iter::repeat_n(0, tw - 1))).collect();
                let table = t.tensor(&flash_sink_heads_name(names), TensorRole::Param, 1, items * tw, hx, TensorInit::Values(entries));
                let head_in = t.buffer_deep("in_sink_head", 1, 1, hx);
                let l_head = t.load_block_at("load_sink_head", table, 0, vec![(LV_ITEM_Y, 1)], 1, 1, 1, head_in);
                let l = t.stage(
                    "load_sinks",
                    StageKind::Load {
                        tensor: sinks,
                        pattern: AccessPattern {
                            origin: TileAddr {
                                base: 0,
                                steps: Vec::new(),
                                indirect: Some(IndirectTile { index: head_in, tile_stride: 1, elems_per_tile: 1, bound: (0, per_row - 1) }),
                            },
                            block_h: 1,
                            block_w: 1,
                            stride_w: 1,
                            stride_h: sink_t,
                        },
                        num_blocks: 1,
                        multicast: None,
                    },
                    vec![head_in],
                    vec![sk_in],
                );
                t.edge(l_head, head_in, l);
                l
            } else {
                t.load_block_at("load_sinks", sinks, 0, vec![(LV_ITEM_Y, 1)], 1, 1, 1, sk_in)
            };
            let row_of_page = ElemAccess {
                base: 0,
                terms: vec![(LV_GEN_ROW, 1)],
                indirect: None,
            };
            let s = t.stage(
                "init_max",
                generic(head_space(tr, tw, None), vec![row_of_page], ident_map(w1), *op(0)),
                vec![sk_in],
                vec![m_run],
            );
            t.edge(l_sk, sk_in, s);
        }
    }
    t.stage(
        "init_sum",
        generic(
            head_space(tr, tw, None),
            Vec::new(),
            ident_map(w1),
            *konst(if g.sinks.is_some() { 1.0 } else { 0.0 }),
        ),
        Vec::new(),
        vec![s_run],
    );
    for (j, &o_run) in o_runs.iter().enumerate() {
        t.stage(
            &part("init_out", j),
            generic(
                head_space(tr, o_w, None),
                Vec::new(),
                ident_map(wd),
                *konst(0.0),
            ),
            Vec::new(),
            vec![o_run],
        );
    }
    let start = counted.then(|| {
        let start = t.scratch_deep("block_start", 2, 1, ix);
        t.stage(
            "init_start",
            generic(flat_space(1), Vec::new(), ident_map(w1), *konst(0.0)),
            Vec::new(),
            vec![start],
        );
        start
    });

    // ── The body, once per block: block `b` of K/V head `x` is tile row `b` of the cache from tile `x·head_t`.
    // A windowed block of K/V head `x` is tile row `b`'s window run from the tile the item's coordinate carries.
    let (kv_t, kv_step, row_stride) = windows.as_ref().map_or((head_t, head_t, g.kvw * head_t), |w| (w.kv_run, 1, w.kv_row_t));
    let (v_page_t, v_step, v_row_stride) = windows.as_ref().map_or((vt, v_t, g.kvw * v_t), |w| (w.kv_run, 1, w.kv_row_t));
    let v_first = |j: usize| if windows.is_some() { 0 } else { (j0 + j) * vt };
    let k_in = t.buffer_deep("in_k", pages, kv_t, precision);
    let v_ins: Vec<BufferId> = (0..nv).map(|j| t.buffer_deep(&part("in_v", j), pages, v_page_t, precision)).collect();
    // Block `b` of K/V head `x`, `cols` tiles from tile `base` of the head's run of `head` tiles in a row of `stride`.
    let cache_block = |t: &mut Template, name: &str, tensor: TensorId, (base, cols, head, stride): (usize, usize, usize, usize), into: BufferId| {
        let origin = TileAddr {
            base,
            steps: vec![(LV_ITEM_X, head), (LV_KBLOCK, stride)],
            indirect: None,
        };
        t.stage(
            name,
            StageKind::Load {
                tensor,
                pattern: AccessPattern {
                    origin,
                    block_h: 1,
                    block_w: cols,
                    stride_w: 1,
                    stride_h: stride,
                },
                num_blocks: nb,
                multicast: None,
            },
            Vec::new(),
            vec![into],
        )
    };
    let l_k = cache_block(&mut t, "load_k", k, (0, kv_t, kv_step, row_stride), k_in);
    let l_vs: Vec<StageId> = (0..nv)
        .map(|j| cache_block(&mut t, &part("load_v", j), v, (v_first(j), v_page_t, v_step, v_row_stride), v_ins[j]))
        .collect();
    // Each row's limit in the block's own positions: its limit less the block's start.
    let lim = match (row_lim, start) {
        (Some(row_lim), Some(start)) => {
            let lim_b = t.scratch_deep("block_limit", 1, 1, ix);
            t.stage(
                "block_limit",
                generic(
                    head_space(tr, 1, None),
                    vec![per_row_map(w1), fixed(0)],
                    ident_map(w1),
                    *sub(op(0), op(1)),
                ),
                vec![row_lim, start],
                vec![lim_b],
            );
            Some(lim_b)
        }
        (row_lim, _) => row_lim,
    };
    // A window keeps the last `W` positions below each row's limit: the block's floor is its limit less `W`. A chunk of
    // `C` keeps the row's own chunk: its floor, `⌊(limit − 1)/C⌋·C`, less the block's start.
    let floor = match (g.window, lim) {
        (Some(Window::Chunked(c)), Some(_)) => {
            let row_lim = row_lim.expect("a chunked row is masked");
            let floor_b = t.scratch_deep("block_floor", 1, 1, ix);
            let c = c as f64;
            let chunk = mul(Box::new(TensorBody::FloorDiv(sub(op(0), konst(1.0)), konst(c))), konst(c));
            let (ins, maps, body) = match start {
                Some(start) => (vec![row_lim, start], vec![per_row_map(w1), fixed(0)], *sub(chunk, op(1))),
                None => (vec![row_lim], vec![per_row_map(w1)], *chunk),
            };
            t.stage("block_floor", generic(head_space(tr, 1, None), maps, ident_map(w1), body), ins, vec![floor_b]);
            Some(floor_b)
        }
        (Some(Window::Sliding(w)), Some(lim_b)) => {
            let floor_b = t.scratch_deep("block_floor", 1, 1, ix);
            t.stage(
                "block_floor",
                generic(head_space(tr, 1, None), vec![per_row_map(w1)], ident_map(w1), *sub(op(0), konst(w as f64))),
                vec![lim_b],
                vec![floor_b],
            );
            Some(floor_b)
        }
        _ => None,
    };
    // A windowed head's element `e` sits at its window's offset plus `e`: a gather whose index, the item's offset, is one
    // value for every element. Operand `at` of the stage reads the offset.
    let through = |terms: &[(LoopVar, i64)], at: usize, run: usize| ElemAccess {
        base: 0,
        terms: terms.to_vec(),
        indirect: Some(IndirectTerm { operand: at, stride: 1, modulus: run * tw - g.hd + 1 }),
    };
    // `S[r, j] = Σ_e Q[r, e]·K[j, e]/√hd` — the tile's rows against the block's positions, one contraction.
    let (score_maps, score_ins) = match (&windows, offsets) {
        (Some(w), Some((_, off))) => (
            vec![
                through(&[(LV_GEN_ROW, (w.q_run * tw) as i64), (LV_GEN_K, 1)], 2, w.q_run),
                through(&[(LV_GEN_COL, (w.kv_run * tw) as i64), (LV_GEN_K, 1)], 3, w.kv_run),
                fixed(0),
                fixed(1),
            ],
            vec![q_in, k_in, off, off],
        ),
        _ => {
            let hd = g.hd as i64;
            (vec![m2(&[(LV_GEN_ROW, hd), (LV_GEN_K, 1)]), m2(&[(LV_GEN_COL, hd), (LV_GEN_K, 1)])], vec![q_in, k_in])
        }
    };
    let s_buf = t.scratch_deep("scores", 1, 1, acc);
    let s_scores = t.stage(
        "scores",
        generic(contract(tr, tr, g.hd), score_maps, ident_map(w1), *mul(mul(op(0), op(1)), konst(scale))),
        score_ins,
        vec![s_buf],
    );
    // ★ A SOFT CAP takes every scaled score before the block's maximum and the mask.
    let s_buf = match g.cap {
        None => s_buf,
        Some(c) => {
            let capped_buf = t.scratch_deep("capped_scores", 1, 1, acc);
            t.stage(
                "score_cap",
                generic(head_space(tr, tr, None), vec![ident_map(w1)], ident_map(w1), *capped(op(0), c)),
                vec![s_buf],
                vec![capped_buf],
            );
            capped_buf
        }
    };
    // A position is live below its row's limit in the block and, windowed, at or past its floor; with no mask, every
    // position is.
    let masking = lim.is_some();
    let windowed = floor.is_some();
    let live = |then: Box<TensorBody>, otherwise: Box<TensorBody>, at: usize| {
        if masking {
            let kept = if windowed {
                Box::new(TensorBody::Select {
                    cond: Pred::AxisGe(LV_GEN_COL, PredRhs::Operand(at + 1)),
                    then,
                    otherwise: otherwise.clone(),
                })
            } else {
                then
            };
            TensorBody::Select {
                cond: Pred::AxisLt(LV_GEN_COL, PredRhs::Operand(at)),
                then: kept,
                otherwise,
            }
        } else {
            *then
        }
    };
    let with_limit = |mut ins: Vec<BufferId>, mut maps: Vec<ElemAccess>, map: ElemAccess| -> (Vec<BufferId>, Vec<ElemAccess>) {
        for b in lim.into_iter().chain(floor) {
            ins.push(b);
            maps.push(map.clone());
        }
        (ins, maps)
    };
    let m_blk = t.scratch_deep("block_max", 1, 1, acc);
    let (max_ins, max_maps) = with_limit(vec![s_buf, sc_in], vec![ident_map(w1), const_map()], per_row_map(w1));
    let s_mblk = t.stage(
        "block_max",
        generic(
            head_space(tr, tr, Some(ReducePool::Max)),
            max_maps,
            per_row_map(w1),
            live(op(0), konst(MASKED), 2),
        ),
        max_ins,
        vec![m_blk],
    );
    let m_new = t.scratch_deep("new_max", 1, 1, acc);
    let pair = vec![
        IterSpec {
            var: LV_GEN_ROW,
            extent: tr,
            kind: IterKind::Parallel,
        },
        IterSpec {
            var: LV_GEN_COL,
            extent: 2,
            kind: IterKind::Reduce {
                pool: ReducePool::Max,
            },
        },
    ];
    let pick = TensorBody::Select {
        cond: Pred::AxisLt(LV_GEN_COL, PredRhs::Const(1)),
        then: op(0),
        otherwise: op(1),
    };
    t.stage(
        "new_max",
        generic(
            pair,
            vec![per_row_map(w1), per_row_map(w1), const_map()],
            per_row_map(w1),
            pick,
        ),
        vec![m_run, m_blk, sc_in],
        vec![m_new],
    );
    let alpha = t.scratch_deep("rescale", 1, 1, acc);
    t.stage(
        "rescale",
        generic(
            head_space(tr, 1, None),
            vec![per_row_map(w1), per_row_map(w1)],
            ident_map(w1),
            *unary(SfpuKind::Exp, sub(op(0), op(1))),
        ),
        vec![m_run, m_new],
        vec![alpha],
    );
    let p_buf = t.scratch_deep("probs", 1, 1, acc);
    let (p_ins, p_maps) = with_limit(vec![s_buf, m_new], vec![ident_map(w1), per_row_map(w1)], per_row_map(w1));
    t.stage(
        "probs",
        generic(
            head_space(tr, tr, None),
            p_maps,
            ident_map(w1),
            live(unary(SfpuKind::Exp, sub(op(0), op(1))), konst(0.0), 2),
        ),
        p_ins,
        vec![p_buf],
    );
    let l_blk = t.scratch_deep("block_sum", 1, 1, acc);
    t.stage(
        "block_sum",
        generic(
            head_space(tr, tr, Some(ReducePool::Sum)),
            vec![ident_map(w1), const_map()],
            per_row_map(w1),
            TensorBody::Operand(0),
        ),
        vec![p_buf, sc_in],
        vec![l_blk],
    );
    // `O_b[r, e] = Σ_j P[r, j]·V[j, e]` — one contraction over the block's positions, a value page at a time; a windowed
    // head's `vd` values from its window's offset.
    let o_blks: Vec<(StageId, BufferId)> = (0..nv)
        .map(|j| {
            let o_blk = t.scratch_deep(&part("block_out", j), 1, vt, acc);
            let probs = m2(&[(LV_GEN_ROW, w1 as i64), (LV_GEN_K, 1)]);
            let (maps, ins) = match (&windows, offsets) {
                (Some(w), Some((_, off))) => (
                    vec![probs, through(&[(LV_GEN_K, (w.kv_run * tw) as i64), (LV_GEN_COL, 1)], 2, w.kv_run), fixed(1)],
                    vec![p_buf, v_ins[j], off],
                ),
                _ => (vec![probs, m2(&[(LV_GEN_K, (vt * tw) as i64), (LV_GEN_COL, 1)])], vec![p_buf, v_ins[j]]),
            };
            let s_oblk = t.stage(
                &part("block_out", j),
                generic(contract(tr, o_w, tr), maps, ident_map(wd), *mul(op(0), op(1))),
                ins,
                vec![o_blk],
            );
            (s_oblk, o_blk)
        })
        .collect();
    // The updates, each reading the carry it writes: the block folds into the running state.
    t.stage(
        "fold_sum",
        generic(
            head_space(tr, 1, None),
            vec![per_row_map(w1), per_row_map(w1), per_row_map(w1)],
            ident_map(w1),
            *add(mul(op(0), op(1)), op(2)),
        ),
        vec![s_run, alpha, l_blk],
        vec![s_run],
    );
    for (j, &o_run) in o_runs.iter().enumerate() {
        t.stage(
            &part("fold_out", j),
            generic(
                head_space(tr, o_w, None),
                vec![ident_map(wd), per_row_map(w1), ident_map(wd)],
                ident_map(wd),
                *add(mul(op(0), op(1)), op(2)),
            ),
            vec![o_run, alpha, o_blks[j].1],
            vec![o_run],
        );
    }
    t.stage(
        "fold_max",
        generic(
            head_space(tr, 1, None),
            vec![per_row_map(w1), per_row_map(w1)],
            ident_map(w1),
            TensorBody::Operand(1),
        ),
        vec![m_run, m_new],
        vec![m_run],
    );
    if let Some(start) = start {
        t.stage(
            "next_start",
            generic(
                flat_space(1),
                vec![fixed(0)],
                ident_map(w1),
                *add(op(0), konst(tr as f64)),
            ),
            vec![start],
            vec![start],
        );
    }

    // ── The epilogue, once per item: the attention, into the item's tile of the dense result.
    let mut epilogue = Vec::new();
    for (j, &o_run) in o_runs.iter().enumerate() {
        let out = t.buffer_deep(&part("out", j), pages, vt, precision);
        let s_norm = t.stage(
            &part("normalize", j),
            generic(
                head_space(tr, o_w, None),
                vec![ident_map(wd), per_row_map(w1)],
                ident_map(wd),
                *mul(op(0), unary(SfpuKind::Recip, op(1))),
            ),
            vec![o_run, s_run],
            vec![out],
        );
        let st = t.store_block(
            &part("store_ctx", j),
            c,
            j * vt,
            vec![(LV_ITEM_Y, part_t)],
            1,
            vt,
            out_width_t,
            out,
        );
        epilogue.push((s_norm, out, st));
    }
    for (from, buf, to) in [(l_sc, sc_in, s_mblk), (l_q, q_in, s_scores), (l_k, k_in, s_scores)] {
        t.edge(from, buf, to);
    }
    if let Some((l_off, off)) = offsets {
        t.edge(l_off, off, s_scores);
    }
    for j in 0..nv {
        t.edge(l_vs[j], v_ins[j], o_blks[j].0);
    }
    for (s_norm, out, st) in epilogue {
        t.edge(s_norm, out, st);
    }
    let work: Vec<WorkItem> = (0..items)
        .map(|y| {
            let (qt, h) = (y / per_row, y % per_row);
            let kv_head = match &windows {
                Some(w) => w.at[y].kv_first,
                None if g.grouped => y,
                None => g.q.kv_of(h),
            };
            WorkItem {
                id: WorkItemId::new(y),
                coord: (y, kv_head),
                out: TileRect {
                    row0_t: qt,
                    col0_t: h * part_t,
                    h_t: 1,
                    w_t: part_t,
                },
            }
        })
        .collect();
    // ★ THE BLOCKS RUN TO THE POSITION. Query row `r` sees the positions below `P + r`, so the last of the `rows`
    // query rows reaches `P + rows − 1` and no row reads a block past `⌈(P + rows − 1) / tile⌉`: every later block is
    // masked whole, its update the identity. That count is the fold's trip, read from `P` before the program runs —
    // a decode step at position `P` streams `⌈P / tile⌉` blocks of its cache, not the `⌈l / tile⌉` it is declared with.
    // It admits exactly the `P` the guard does, `1 ≤ P ≤ rows − (T − 1)`, so an out-of-cache `P` is refused against the
    // cache before any op runs; its greatest count is the cache's `⌈l / tile⌉` tile rows.
    let k_trip = position.filter(|_| counted).map(|pt| {
        let reach = g.q.rows - 1;
        t.trip(Trip {
            tensor: pt,
            elem: 0,
            admit: (1, bound_max(g.rows, g.q.rows) as i64),
            bias: reach as i64,
            per: tr,
        })
    });
    let values = match g.v_part {
        Some((first, count)) => format!("_v{}_pages{first}+{count}", g.vd),
        None if g.vd != g.hd => format!("_v{}", g.vd),
        None => String::new(),
    };
    let mut prog = t.finish(
        format!("{tag}_flash_blocks_{items}x{}{values}_l{}_b{tr}", g.hd, g.l),
        work,
        AccPolicy::F32,
        approx("attention", precision),
        target,
    )?;
    prog.work.k_trip = k_trip;
    // ★ THE BLOCKS ARE THE CONTRACTION'S K BLOCKS. An item's result is one tile row of the dense result; the cache's
    // `⌈l / tile⌉` blocks are what it folds over.
    prog.work.plan = Some(BlockPlan {
        per_site_m: 1,
        per_site_n: part_t,
        grid: (tiles_q, per_row),
        inner: InnerLoop {
            num_blocks: nb,
            block_k_tiles: 1,
        },
        block: BlockShape {
            osh: 1,
            osw: part_t,
            ibw: 1,
        },
        acc: AccPolicy::F32,
        pack_accumulate: false,
        engines: EnginePacking::LoadsTogether,
        granularity: Granularity::PerTile,
    });
    prog.work.k_tiles = nb;
    Ok(prog)
}

/// Attention in the flash form: one fold section ([`flash_fold`]), refused by name when a site cannot hold it.
pub(super) fn flash_attention(
    names: &StageNames,
    g: FlashGeometry,
    tag: &str,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    if target.tile.rows != target.tile.cols {
        return Err(SpatialReject::NotLowered("a flash attention fold on a tile that is not square — a block's positions are its scores' columns"));
    }
    // ★ A WINDOWED FOLD WRITES EACH HEAD INTO WHOLE TILES OF ITS OWN. Two heads that share a tile of the result are two
    // items on two sites, and a store moves whole tiles, so neither could write its half; each head's values land in a
    // tile run of their own ([`flash_heads_name`]), and one more section packs the heads side by side ([`pack_heads`]).
    let windowed = g.hd % target.tile.cols != 0 || g.vd % target.tile.cols != 0;
    let packed = windowed && fold_items(&g, target).1 > 1;
    let fold_names = if packed { StageNames { out: flash_heads_name(names), ..names.clone() } } else { names.clone() };
    let p = flash_fold(&fold_names, g, tag, target)?;
    if site_footprint(&p) > target.max_local_mem_bytes() {
        if windowed {
            return Err(SpatialReject::NotLowered("a flash attention fold over heads that are not whole tiles — one tile of queries against one tile row of its K/V head's window — does not fit a site"));
        }
        return value_pages(names, g, tag, target);
    }
    let mut sections = Vec::new();
    // ★ A SINK IS THE RUNNING SOFTMAX'S FIRST TERM. Each fold row starts from its head's sink logit as its maximum, with a
    // sum of one and no value, so every block's update carries the sink's term to the end. The sinks are first spread
    // one tile per sink tile ([`sink_tiles`]), a row's sink in its own column ([`flash_sinks_name`]).
    if let Some(si) = g.sinks {
        let (tw, (_, per_row, _, _)) = (target.tile.cols, fold_items(&g, target));
        let (sink_t, _) = sink_tiles(&g, target);
        let head_of = |e: usize| {
            let (y, row) = (e / tw, e % tw);
            if g.grouped {
                (row < g.q.group).then_some(y * g.q.group + row)
            } else {
                Some(y % per_row)
            }
        };
        sections.push(spread_sinks(names.operand(si), g.q.per_row, &flash_sinks_name(&fold_names), sink_t * tw, &head_of, target)?);
    }
    sections.push(p);
    if packed {
        sections.extend(pack_heads(&fold_names.out, &names.out, (g.q.rows, g.q.per_row, g.vd), elem_precision(target)?, target)?);
    }
    Ok(sections)
}

/// A decode attention whose fold does not fit a site, as one fold per value page: fold `j` streams the keys and its
/// page of every head's values into `{out}__values{j}`, and tile joins lay the pages side by side per head into the
/// result. Each fold reads the keys again — the price of a site that cannot hold a head's every value page.
pub(super) fn value_pages(
    names: &StageNames,
    g: FlashGeometry,
    tag: &str,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let too_large = SpatialReject::NotLowered("a flash attention fold — one tile of queries against one tile row of its K/V head — does not fit a site");
    let tw = target.tile.cols;
    let v_t = g.vd / tw;
    let vt = row_block(v_t, target);
    let pages = v_t / vt;
    if pages < 2 || g.grouped || g.q.rows != 1 || g.v_part.is_some() {
        return Err(too_large);
    }
    let page_name = |j: usize| format!("{}__values{j}", names.out);
    let mut sections = Vec::new();
    for j in 0..pages {
        let mut fold_names = names.clone();
        fold_names.out = page_name(j);
        let fold = flash_attention(&fold_names, FlashGeometry { v_part: Some((j, 1)), ..g }, tag, target)?;
        sections.extend(fold);
    }
    let page = vt * tw;
    let mut joined = page_name(0);
    for j in 1..pages {
        let out = if j + 1 == pages { names.out.clone() } else { format!("{}__values0to{j}", names.out) };
        let join_names = StageNames { out: out.clone(), ins: vec![joined, page_name(j)], fills: vec![Fill::Unknown, Fill::Unknown] };
        sections.push(tile_join(&join_names, (1, g.q.per_row), (j * page, page), target)?);
        joined = out;
    }
    Ok(sections)
}

/// `to` — `rows` rows of `heads` heads `w` wide, packed — from `from`, the same heads each in `⌈w / tile⌉` whole tiles of
/// its own: a windowed flash fold's result ([`flash_heads_name`]). Both are dense, a tile row holding `tile` rows.
///
/// ★ HEADS REALIGN WITH TILES EVERY `P = tile / gcd(w, tile)` HEADS. A group of `P` packed heads is `P·w` columns, a whole
/// number of tiles from a tile edge, so an item per tile row and group moves its group's `P` padded runs into its packed
/// tiles: packed column `c` of the group is head `p = ⌊c / w⌋`'s column `c − p·w`, at `c + p·(⌈w / tile⌉·tile − w)` of the
/// padded page — one affine map a head, a band each. The heads past the last whole group are a section of their own,
/// the row's last tiles; each section then writes a part of `to`. An item writes its tiles a page at a time.
pub(super) fn pack_heads(
    from: &str,
    to: &str,
    (rows, heads, w): (usize, usize, usize),
    dt: Precision,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let (tr, tw) = (target.tile.rows, target.tile.cols);
    let head_t = w.div_ceil(tw);
    let pad = head_t * tw - w;
    let period = tw / gcd(w, tw);
    let (in_row_t, out_row_t) = (heads * head_t, (heads * w).div_ceil(tw));
    let (rows_t, h) = (rows.div_ceil(tr), rows.min(tr));
    let (groups, rest) = (heads / period, heads % period);
    let part = groups > 0 && rest > 0;
    // Groups `0 … across − 1` of `n` heads each from head `first`.
    let section = |first: usize, n: usize, across: usize| -> Result<SpatialProgram, SpatialReject> {
        let (in_t, out_t) = (n * head_t, (n * w).div_ceil(tw));
        let (in_first, out_first) = (first * head_t, first * w / tw);
        let blk = row_block(out_t, target);
        let mut t = Template::new(target)?;
        let src = t.tensor(from, TensorRole::Input, rows, in_row_t * tw, dt, TensorInit::Zero);
        let dst = t.tensor(to, if part { TensorRole::State } else { TensorRole::Output }, rows, heads * w, dt, TensorInit::Zero);
        let page = t.buffer("in_heads", in_t, dt);
        let load = t.load_block_at(
            "load_heads",
            src,
            in_first,
            vec![(LV_ITEM_Y, in_row_t), (LV_ITEM_X, period * head_t)],
            1,
            in_t,
            in_row_t,
            page,
        );
        let mut computes = Vec::new();
        for b in 0..out_t / blk {
            let lo = b * blk * tw;
            let hi = (lo + blk * tw).min(n * w);
            let mut maps = Vec::new();
            let mut bands = Vec::new();
            for p in lo / w..hi.div_ceil(w) {
                maps.push(ElemAccess {
                    base: (lo + p * pad) as i64,
                    terms: vec![(LV_GEN_ROW, (in_t * tw) as i64), (LV_GEN_COL, 1)],
                    indirect: None,
                });
                bands.push((((p + 1) * w).min(hi) - lo, TensorBody::Operand(maps.len() - 1)));
            }
            // Columns past the row's last head, inside its last tile, hold nothing.
            if hi < lo + blk * tw {
                bands.push((blk * tw, *konst(0.0)));
            }
            let out = t.buffer(&format!("out{b}"), blk, dt);
            let s = t.stage(
                &format!("pack{b}"),
                StageKind::Compute(ComputeOp::Generic {
                    iters: head_space(h, blk * tw, None),
                    ins: maps.clone(),
                    out: ident_map(wide(blk, target)),
                    body: banded(bands),
                }),
                vec![page; maps.len()],
                vec![out],
            );
            let st = t.store_block(
                &format!("store_packed{b}"),
                dst,
                out_first + b * blk,
                vec![(LV_ITEM_Y, out_row_t), (LV_ITEM_X, period * w / tw)],
                1,
                blk,
                out_row_t,
                out,
            );
            t.edge(s, out, st);
            computes.push(s);
        }
        t.edge(load, page, computes[0]);
        let items: Vec<WorkItem> = (0..rows_t)
            .flat_map(|r| (0..across).map(move |x| (r, x)))
            .enumerate()
            .map(|(id, (r, x))| WorkItem {
                id: WorkItemId::new(id),
                coord: (r, x),
                out: TileRect { row0_t: r, col0_t: out_first + x * period * w / tw, h_t: 1, w_t: out_t },
            })
            .collect();
        let n_items = items.len();
        let mut prog = t.finish(format!("pack_heads_{rows}x{heads}x{w}_from{first}"), items, AccPolicy::for_result(dt), NumericsClaim::Exact, target)?;
        prog.work.plan = Some(BlockPlan {
            per_site_m: 1,
            per_site_n: out_t,
            grid: if part { (1, n_items) } else { (rows_t, across) },
            inner: InnerLoop { num_blocks: 1, block_k_tiles: 1 },
            block: BlockShape { osh: 1, osw: blk, ibw: 1 },
            acc: AccPolicy::for_result(dt),
            pack_accumulate: false,
            engines: EnginePacking::LoadsTogether,
            granularity: Granularity::PerTile,
        });
        Ok(prog)
    };
    let mut sections = Vec::new();
    if groups > 0 {
        sections.push(section(0, period, groups)?);
    }
    if rest > 0 {
        sections.push(section(groups * period, rest, 1)?);
    }
    Ok(sections)
}

/// A one-row attention whose query heads share K/V heads, folded ONE ITEM PER K/V HEAD: three sections.
///   1. `{out}__gqa_q` — the row's heads regrouped, a `(kvw · tile) × hd` matrix: K/V head `x`'s `group` query heads
///      are rows `0 … group − 1` of tile row `x`, the rows past them zero. Item `x` loads its heads' `group · hd`
///      elements of the row as one page and reads row `j`, column `e` at `j·hd + e` of it.
///   2. The flash fold over it ([`flash_fold`], `grouped`): item `x` streams K/V head `x`'s cache once for all
///      `group` rows, every row limited by the row's `P`.
///   3. `out` — the fold's `{out}__gqa_ctx` back as the row: each query head's row lifted out of its tile row into its
///      `hd` elements ([`relayout_dense_picks_to_rows`]).
///
/// Refused when a regrouping page does not fit a site; the caller then folds an item per query head.
pub(super) fn grouped_decode_attention(
    names: &StageNames,
    g: FlashGeometry,
    tag: &str,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    if target.tile.rows != target.tile.cols {
        return Err(SpatialReject::NotLowered(
            "a grouped decode attention on a tile that is not square",
        ));
    }
    let (tr, tw) = (target.tile.rows, target.tile.cols);
    let (kv, group, hd) = (g.kvw, g.q.group, g.hd);
    // ★ A GROUP OF HEADS THAT ARE NOT WHOLE TILES IS STILL A RUN OF WHOLE TILES when the group's `group·hd` values are: its
    // regrouped rows start at column 0 whatever `hd` is, so only the K/V head's window carries an offset.
    if (group * hd) % tw != 0 || (group * g.vd) % tw != 0 {
        return Err(SpatialReject::NotLowered("a grouped decode attention whose groups of query heads are not whole tiles"));
    }
    let head_t = hd.div_ceil(tw);
    let (heads_t, row_t) = (group * hd / tw, g.q.per_row * hd / tw);
    let precision = elem_precision(target)?;
    let (q_name, q_fill) = names.operand(0);
    let grouped_q = format!("{}__gqa_q", names.out);
    let grouped_ctx = format!("{}__gqa_ctx", names.out);
    let flat = |terms: &[(LoopVar, i64)]| ElemAccess {
        base: 0,
        terms: terms.to_vec(),
        indirect: None,
    };
    let below = |bound: usize| Pred::AxisLt(LV_GEN_ROW, PredRhs::Const(bound));
    let budget = target.max_local_mem_bytes();
    let fits = |p: SpatialProgram| {
        if site_footprint(&p) <= budget {
            Ok(p)
        } else {
            Err(SpatialReject::NotLowered(
                "a regrouping of one K/V head's query heads does not fit a site",
            ))
        }
    };

    let regroup = paged(
        |pages| {
            let mut t = Template::new(target)?;
            let q = t.tensor(
                q_name,
                TensorRole::Input,
                1,
                g.q.per_row * hd,
                precision,
                crate::spatial::passes::from_tensor_op::init_of(q_fill, g.q.per_row * hd),
            );
            let gq = t.tensor(
                &grouped_q,
                TensorRole::Output,
                kv * tr,
                hd,
                precision,
                TensorInit::Zero,
            );
            let inp = t.buffer_deep("in_heads", pages, heads_t, precision);
            let out = t.buffer_deep("out", pages, head_t, precision);
            let load = t.load_block_at(
                "load_heads",
                q,
                0,
                vec![(LV_ITEM_Y, heads_t)],
                1,
                heads_t,
                row_t,
                inp,
            );
            let s = t.stage(
                "regroup",
                StageKind::Compute(ComputeOp::Generic {
                    iters: head_space(tr, hd, None),
                    ins: vec![flat(&[(LV_GEN_ROW, hd as i64), (LV_GEN_COL, 1)])],
                    out: ident_map(wide(head_t, target)),
                    body: TensorBody::Select {
                        cond: below(group),
                        then: op(0),
                        otherwise: konst(0.0),
                    },
                }),
                vec![inp],
                vec![out],
            );
            let st = t.store_block(
                "store_groups",
                gq,
                0,
                vec![(LV_ITEM_Y, head_t)],
                1,
                head_t,
                head_t,
                out,
            );
            t.edge(load, inp, s);
            t.edge(s, out, st);
            let items = (0..kv)
                .map(|x| WorkItem {
                    id: WorkItemId::new(x),
                    coord: (x, 0),
                    out: TileRect {
                        row0_t: x,
                        col0_t: 0,
                        h_t: 1,
                        w_t: head_t,
                    },
                })
                .collect();
            let mut regroup = t.finish(
                format!("{tag}_gqa_groups_{kv}x{group}"),
                items,
                AccPolicy::for_result(precision),
                NumericsClaim::Exact,
                target,
            )?;
            regroup.work.plan = Some(BlockPlan {
                grid: (kv, 1),
                ..tile_row_plan(head_t, head_t, precision)
            });
            Ok(regroup)
        },
        target,
    )?;

    let mut fold_names = names.clone();
    fold_names.out = grouped_ctx.clone();
    fold_names.ins[0] = grouped_q.clone();
    fold_names.fills[0] = Fill::Unknown;
    let fold = flash_attention(
        &fold_names,
        FlashGeometry { grouped: true, ..g },
        tag,
        target,
    )?;

    // Query head `v` is row `v mod group` of tile row `v / group`.
    let picks: Vec<usize> = (0..g.q.per_row)
        .map(|v| (v / group) * tr + v % group)
        .collect();
    let ungroup = if g.vd % tw == 0 {
        relayout_dense_picks_to_rows((&grouped_ctx, &Fill::Unknown), &names.out, kv * tr, &picks, g.vd, precision, target)?
    } else {
        ungroup_rows(&grouped_ctx, &names.out, (kv, group, g.vd), precision, target)?
    };

    let mut sections = vec![fits(regroup)?];
    sections.extend(fold);
    sections.push(fits(ungroup)?);
    Ok(sections)
}

/// `to` — the one row of `kv·group` query heads `w` wide — from `from`, a dense `(kv·tile) × w` matrix whose tile row `x`
/// holds K/V head `x`'s `group` query heads as its first rows ([`grouped_decode_attention`]'s result), for heads `w` that
/// are not whole tiles. Item `x` loads tile row `x` and writes the group's `group·w` values — whole tiles, a group being
/// whole tiles — a page at a time: packed column `c` of the group is row `k = ⌊c / w⌋`'s column `c − k·w`, at
/// `c + k·(⌈w / tile⌉·tile − w)` of the page read flat — one affine map a row, a band each.
pub(super) fn ungroup_rows(
    from: &str,
    to: &str,
    (kv, group, w): (usize, usize, usize),
    dt: Precision,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (tr, tw) = (target.tile.rows, target.tile.cols);
    let row_t = w.div_ceil(tw);
    let pad = row_t * tw - w;
    let out_t = group * w / tw;
    let blk = row_block(out_t, target);
    let mut t = Template::new(target)?;
    let src = t.tensor(from, TensorRole::Input, kv * tr, w, dt, TensorInit::Zero);
    let dst = t.tensor(to, TensorRole::Output, 1, kv * group * w, dt, TensorInit::Zero);
    let page = t.buffer("in_rows", row_t, dt);
    let load = t.load_block_at("load_rows", src, 0, vec![(LV_ITEM_Y, row_t)], 1, row_t, row_t, page);
    let mut computes = Vec::new();
    for b in 0..out_t / blk {
        let (lo, hi) = (b * blk * tw, (b + 1) * blk * tw);
        let mut maps = Vec::new();
        let mut bands = Vec::new();
        for k in lo / w..hi.div_ceil(w) {
            maps.push(ElemAccess { base: (lo + k * pad) as i64, terms: vec![(LV_GEN_COL, 1)], indirect: None });
            bands.push((((k + 1) * w).min(hi) - lo, TensorBody::Operand(maps.len() - 1)));
        }
        let out = t.buffer(&format!("out{b}"), blk, dt);
        let s = t.stage(
            &format!("ungroup{b}"),
            StageKind::Compute(ComputeOp::Generic { iters: flat_space(blk * tw), ins: maps.clone(), out: ident_map(wide(blk, target)), body: banded(bands) }),
            vec![page; maps.len()],
            vec![out],
        );
        let st = t.store_block(&format!("store_row{b}"), dst, b * blk, vec![(LV_ITEM_Y, out_t)], 1, blk, out_t, out);
        t.edge(s, out, st);
        computes.push(s);
    }
    t.edge(load, page, computes[0]);
    let items = (0..kv)
        .map(|x| WorkItem { id: WorkItemId::new(x), coord: (x, 0), out: TileRect { row0_t: 0, col0_t: x * out_t, h_t: 1, w_t: out_t } })
        .collect();
    let mut prog = t.finish(format!("ungroup_rows_{kv}x{group}x{w}"), items, AccPolicy::for_result(dt), NumericsClaim::Exact, target)?;
    prog.work.plan = Some(BlockPlan {
        per_site_m: 1,
        per_site_n: out_t,
        grid: (1, kv),
        inner: InnerLoop { num_blocks: 1, block_k_tiles: 1 },
        block: BlockShape { osh: 1, osw: blk, ibw: 1 },
        acc: AccPolicy::for_result(dt),
        pack_accumulate: false,
        engines: EnginePacking::LoadsTogether,
        granularity: Granularity::PerTile,
    });
    Ok(prog)
}
