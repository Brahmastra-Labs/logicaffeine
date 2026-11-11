//! Mixtures of experts: routing, the experts' products and their combination, whole and at real width.

use super::*;

/// Where a mixture's optional operands sit past `[x, wr, wg, wu, wd]`: the router's bias, then the gate, up and down
/// biases that are present, in that order.
pub(super) struct MoeOperands {
    router_bias: Option<usize>,
    /// The gate, up and down biases' operands, each a row per expert.
    biases: [Option<usize>; 3],
    /// The selection bias's operand (`biased by B`).
    selection_bias: Option<usize>,
}

impl MoeOperands {
    fn of(spec: &MoeSpec) -> MoeOperands {
        let mut next = 5;
        let mut take = |present: bool| {
            present.then(|| {
                next += 1;
                next - 1
            })
        };
        let router_bias = take(spec.router_bias.is_some());
        let biases = [take(spec.biases[0].is_some()), take(spec.biases[1].is_some()), take(spec.biases[2].is_some())];
        let selection_bias = take(spec.selection_bias.is_some());
        MoeOperands { router_bias, biases, selection_bias }
    }
}

/// A mixture's sigmoid scores, `P[e] = 1/(1 + e^(0 − L[e]))`, at `f`'s precision — the desugar's arrangement.
pub(super) fn moe_sigmoid(
    logits: &str,
    probs: &str,
    ne: usize,
    mut f: FlatBuild,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let l = f.input(logits, ne, &Fill::Unknown, target);
    let out = f.t.buffer("probs", tiles_of(ne, target), f.precision);
    let s = f.compute(
        "sigmoid",
        flat_space(ne),
        vec![Slot { buf: l, map: at(0) }],
        ident_map(wide(tiles_of(ne, target), target)),
        *unary(SfpuKind::Recip, add(konst(1.0), unary(SfpuKind::Exp, sub(konst(0.0), op(0))))),
        out,
    );
    let claim = approx("moe router sigmoid", f.precision);
    f.finish(probs, ne, vec![s], out, format!("moe_sigmoid_{ne}"), claim, target)
}

/// The scores the routing's arg-max reads: `max(P[e], −1e30)`, a NaN score as `−1e30`. The arg-max fold is seeded by
/// the first score where the desugar's scan seeds `−1e30` and never picks a NaN; on a softmax's scores the two agree
/// (one NaN logit makes every score NaN), but a sigmoid scores each logit alone, so a NaN first score would seed the
/// fold and win. The weights are still read from `P`. The copy keeps the scores' own precision: it decides the
/// experts, so it is never rounded to the section's datapath.
pub(super) fn routed_scores(f: &mut FlatBuild, p: BufferId, ne: usize, target: &SpatialTarget) -> (StageId, BufferId) {
    let e_t = tiles_of(ne, target);
    let routed = f.t.scratch_deep("routed", 1, e_t, f.t.dtype(p));
    let stage = f.compute(
        "routed",
        flat_space(ne),
        vec![Slot { buf: p, map: at(0) }],
        ident_map(wide(e_t, target)),
        TensorBody::Max(op(0), konst(-1.0e30)),
        routed,
    );
    (stage, routed)
}

/// The scores a mixture's routing rounds choose by, from its scores `p` and at their precision: `p` itself for a softmax;
/// `max(P[e] + B[e], −1e30)` with a selection bias and `max(P[e], −1e30)` for a sigmoid without one (a NaN is never
/// routed ahead of a number); then, under a group limit, [`moe_groups`]'s. The weights are still read from `P`. Returns
/// the stages, in order, and the buffer the rounds scan.
pub(super) fn moe_choosing(
    f: &mut FlatBuild,
    spec: &MoeSpec,
    names: &StageNames,
    operands: &MoeOperands,
    p: BufferId,
    target: &SpatialTarget,
) -> Result<(Vec<StageId>, BufferId), SpatialReject> {
    let ne = spec.ne;
    let e_t = tiles_of(ne, target);
    let dt = f.t.dtype(p);
    let mut stages = Vec::new();
    let sel = match (operands.selection_bias, spec.score) {
        (Some(bi), _) => {
            let (b_name, b_fill) = names.operand(bi);
            // Read by the one stage of the section's one item: one page.
            let b = f.input_paged(b_name, ne, b_fill, dt, 1, target);
            let sel = f.t.scratch_deep("selection", 1, e_t, dt);
            stages.push(f.compute(
                "selection",
                flat_space(ne),
                vec![Slot { buf: p, map: at(0) }, Slot { buf: b, map: at(0) }],
                ident_map(wide(e_t, target)),
                TensorBody::Max(add(op(0), op(1)), konst(MASKED)),
                sel,
            ));
            sel
        }
        (None, MoeScore::Sigmoid) => {
            let (stage, routed) = routed_scores(f, p, ne, target);
            stages.push(stage);
            routed
        }
        (None, MoeScore::Softmax) => p,
    };
    match spec.groups {
        Some(groups) => {
            let (more, chosen) = moe_groups(f, spec, groups, sel, target)?;
            stages.extend(more);
            Ok((stages, chosen))
        }
        None => Ok((stages, sel)),
    }
}

/// The penalty that masks the earlier picks of a routing: `Σ_j −1e30·[p_j ≤ c < e_j]` over the column `c`, the picks'
/// bounds at operands `first`, `first + 1`, … two by two — zero on every other column. Added to a score, a picked column
/// falls to `−1e30` (an f32 sum absorbs the score), below the arg-max's seed, so no column is picked twice; the body
/// grows by one term a pick, never by a page.
pub(super) fn picked_penalty(picks: usize, first: usize) -> Option<Box<TensorBody>> {
    (0..picks)
        .map(|j| {
            Box::new(TensorBody::Select {
                cond: Pred::AxisGe(LV_GEN_COL, PredRhs::Operand(first + 2 * j)),
                then: Box::new(TensorBody::Select {
                    cond: Pred::AxisLt(LV_GEN_COL, PredRhs::Operand(first + 2 * j + 1)),
                    then: konst(MASKED),
                    otherwise: konst(0.0),
                }),
                otherwise: konst(0.0),
            })
        })
        .reduce(|a, b| add(a, b))
}

/// Round `r` of a routing: the arg-max over `n` columns of `scores` (read through `read`) less the earlier picks
/// ([`picked_penalty`]) into the scalar `{name}{r}`, then — where `more` rounds follow — its end `{name}_end{r}`, one past
/// it. The fold is seeded by the first score where the desugar's scan seeds `−1e30`; the two agree on every vector a
/// softmax produces — entries in `[0, 1]` (the picked ones at `−1e30`), or all NaN, since one NaN logit makes the sum
/// NaN — and on an all-NaN vector both pick the first. Returns the pick and its end.
#[allow(clippy::too_many_arguments)]
pub(super) fn moe_pick(
    f: &mut FlatBuild,
    chain: &mut Vec<StageId>,
    (name, stage): (&str, &str),
    r: usize,
    (scores, read, n): (BufferId, ElemAccess, usize),
    earlier: &[(BufferId, BufferId)],
    more: bool,
    ix: Precision,
    target: &SpatialTarget,
) -> (BufferId, Option<BufferId>) {
    let one_w = wide(1, target);
    let mut slots = vec![Slot { buf: scores, map: read }];
    for (pick, end) in earlier {
        slots.push(Slot { buf: *pick, map: fixed(0) });
        slots.push(Slot { buf: *end, map: fixed(0) });
    }
    let body = match picked_penalty(earlier.len(), 1) {
        Some(penalty) => *add(op(0), penalty),
        None => TensorBody::Operand(0),
    };
    let pick = f.t.scratch_deep(&format!("{name}{r}"), 1, 1, ix);
    chain.push(f.compute(&format!("{stage}{r}"), head_space(1, n, Some(ReducePool::ArgMax)), slots, per_row_map(one_w), body, pick));
    if !more {
        return (pick, None);
    }
    let end = f.t.scratch_deep(&format!("{name}_end{r}"), 1, 1, ix);
    chain.push(f.compute(&format!("{name}_end{r}"), flat_space(1), vec![Slot { buf: pick, map: fixed(0) }], ident_map(one_w), *add(op(0), konst(1.0)), end));
    (pick, Some(end))
}

/// The group limit over the selection scores `sel`, as llama.cpp's `build_moe_ffn` takes it: each of `count` groups of
/// `per = ne / count` experts scored by its two best selection scores — the best, then the best of the rest with the
/// best's slot masked (a group of one by its one) — then `kept` rounds each choosing the best group not yet chosen
/// ([`moe_pick`]), and the selection scores of the chosen groups' experts kept in a row that is `−1e30` elsewhere: an
/// expert of a group not chosen is never picked. Returns the stages and that row.
///
/// ★ A GROUP'S VALUE IS ITS ROW. A fold over each group's `per` selection scores writes group `g`'s value on row `g` of
/// its page — a per-row result lands on its own row — so every stage over the groups walks rows, the arg-max over them
/// reads row `c` as its column `c`, and the groups are at most a tile's rows.
pub(super) fn moe_groups(
    f: &mut FlatBuild,
    spec: &MoeSpec,
    groups: MoeGroups,
    sel: BufferId,
    target: &SpatialTarget,
) -> Result<(Vec<StageId>, BufferId), SpatialReject> {
    let MoeGroups { count, kept } = groups;
    if count > target.tile.rows {
        return Err(SpatialReject::NotLowered("a group limit over more groups than a page has rows"));
    }
    let (ne, per) = (spec.ne, spec.ne / count);
    let e_t = tiles_of(ne, target);
    let (ew, one_w) = (wide(e_t, target), wide(1, target));
    let dt = f.t.dtype(sel);
    let ix = index_type(target, ne as u64 + 1)?;
    let rows = || map2(0, per as i64, 1);
    let own = || per_row_map(one_w);
    let each = |reduce: Option<ReducePool>| head_space(count, if reduce.is_some() { per } else { 1 }, reduce);
    let mut stages = Vec::new();
    let best = f.t.scratch_deep("group_best", 1, 1, dt);
    stages.push(f.compute("group_best", each(Some(ReducePool::Max)), vec![Slot { buf: sel, map: rows() }], own(), TensorBody::Operand(0), best));
    let score = if per == 1 {
        best
    } else {
        let at_best = f.t.scratch_deep("group_at_best", 1, 1, ix);
        stages.push(f.compute("group_at_best", each(Some(ReducePool::ArgMax)), vec![Slot { buf: sel, map: rows() }], own(), TensorBody::Operand(0), at_best));
        let past_best = f.t.scratch_deep("group_past_best", 1, 1, ix);
        stages.push(f.compute("group_past_best", each(None), vec![Slot { buf: at_best, map: own() }], own(), *add(op(0), konst(1.0)), past_best));
        let second = f.t.scratch_deep("group_second", 1, 1, dt);
        let rest = TensorBody::Select {
            cond: Pred::AxisGe(LV_GEN_COL, PredRhs::Operand(1)),
            then: Box::new(TensorBody::Select { cond: Pred::AxisLt(LV_GEN_COL, PredRhs::Operand(2)), then: konst(MASKED), otherwise: op(0) }),
            otherwise: op(0),
        };
        stages.push(f.compute(
            "group_second",
            each(Some(ReducePool::Max)),
            vec![Slot { buf: sel, map: rows() }, Slot { buf: at_best, map: own() }, Slot { buf: past_best, map: own() }],
            own(),
            rest,
            second,
        ));
        let score = f.t.scratch_deep("group_score", 1, 1, dt);
        stages.push(f.compute("group_score", each(None), vec![Slot { buf: best, map: own() }, Slot { buf: second, map: own() }], own(), *add(op(0), op(1)), score));
        score
    };
    // Group `c`'s score, read as column `c` of the arg-max: row `c` of the page.
    let by_row = ElemAccess { base: 0, terms: vec![(LV_GEN_COL, one_w as i64)], indirect: None };
    let mut picked = Vec::new();
    let mut spans = Vec::new();
    for r in 0..kept {
        let (g, end) = moe_pick(f, &mut stages, ("group", "group"), r, (score, by_row.clone(), count), &picked, r + 1 < kept, ix, target);
        picked.extend(end.map(|end| (g, end)));
        // Its experts, `[g·per, g·per + per)`.
        let from = f.t.scratch_deep(&format!("group{r}_from"), 1, 1, ix);
        stages.push(f.compute(&format!("group{r}_from"), flat_space(1), vec![Slot { buf: g, map: fixed(0) }], ident_map(one_w), *mul(op(0), konst(per as f64)), from));
        let to = f.t.scratch_deep(&format!("group{r}_to"), 1, 1, ix);
        stages.push(f.compute(&format!("group{r}_to"), flat_space(1), vec![Slot { buf: from, map: fixed(0) }], ident_map(one_w), *add(op(0), konst(per as f64)), to));
        spans.push((from, to));
    }
    // `sel[c] + (1 − Σ_j [from_j ≤ c < to_j])·(−1e30)`: the chosen groups are disjoint, so an expert of one keeps its
    // selection score exactly (`+ 0·(−1e30)`), and every other falls to `−1e30`.
    let inside = (0..kept)
        .map(|j| {
            Box::new(TensorBody::Select {
                cond: Pred::AxisGe(LV_GEN_COL, PredRhs::Operand(1 + 2 * j)),
                then: Box::new(TensorBody::Select { cond: Pred::AxisLt(LV_GEN_COL, PredRhs::Operand(2 + 2 * j)), then: konst(1.0), otherwise: konst(0.0) }),
                otherwise: konst(0.0),
            })
        })
        .reduce(|a, b| add(a, b))
        .expect("a group limit keeps at least one group");
    let mut slots = vec![Slot { buf: sel, map: at(0) }];
    for (from, to) in spans {
        slots.push(Slot { buf: from, map: fixed(0) });
        slots.push(Slot { buf: to, map: fixed(0) });
    }
    let chosen = f.t.scratch_deep("chosen", 1, e_t, dt);
    stages.push(f.compute("chosen", flat_space(ne), slots, ident_map(ew), *add(op(0), mul(sub(konst(1.0), inside), konst(MASKED))), chosen));
    Ok((stages, chosen))
}

/// The divisor a renormalization takes: the kept weights' sum, clamped at 2⁻¹⁴ as llama.cpp clamps it where the sum
/// can fall below that ([`MoeSpec::sum_can_clamp`]).
pub(super) fn renormalizing_sum(spec: &MoeSpec, sum: Box<TensorBody>) -> Box<TensorBody> {
    if spec.sum_can_clamp() {
        Box::new(TensorBody::Max(sum, konst(6.103515625e-5)))
    } else {
        sum
    }
}

/// A round's weight read out of the scores: the routed scale multiplies it here when nothing renormalizes it after.
pub(super) fn round_weight(spec: &MoeSpec) -> TensorBody {
    match spec.scale {
        Some(_) if !spec.renormalize => *mul(op(0), konst(spec.scale())),
        _ => TensorBody::Operand(0),
    }
}

/// A mixture's gated activation of `gate` and `up`: SwiGLU's `silu(gate)·up`, or gpt-oss's
/// `min(gate, L)·σ(α·min(gate, L))·(clamp(up, −L, L) + 1)` for its slope `α` — the sigmoid term as `silu(α·g)/α`, one
/// SFPU op.
pub(super) fn moe_gated(act: crate::tensor_recognize::qwen::MoeAct, gate: Box<TensorBody>, up: Box<TensorBody>) -> Box<TensorBody> {
    match act {
        crate::tensor_recognize::qwen::MoeAct::Swiglu => mul(unary(SfpuKind::Silu, gate), up),
        crate::tensor_recognize::qwen::MoeAct::ClampedSwiglu { limit, slope } => {
            let (limit, slope) = (f64::from_bits(limit), f64::from_bits(slope));
            let g = Box::new(TensorBody::Min(gate, konst(limit)));
            let u = Box::new(TensorBody::Max(Box::new(TensorBody::Min(up, konst(limit))), konst(-limit)));
            mul(mul(unary(SfpuKind::Silu, mul(konst(slope), g)), konst(1.0 / slope)), add(u, konst(1.0)))
        }
    }
}

/// The Mixture-of-Experts FFN — route `x` to its top-`k` of `ne` experts and combine their SwiGLUs by the
/// renormalized router weight:
///
/// ```text
///   L = x·Wr                  router logits (1 × ne)
///   P = softmax(L)
///   b_1..b_k                  the k largest of P, first maximum on ties (k rounds of arg-max over a mask)
///   out = Σ_i P[b_i]·down_{b_i}(silu(gate_{b_i}·x) ⊙ up_{b_i}·x)  /  Σ_i P[b_i]
/// ```
///
/// ★ ONLY THE CHOSEN EXPERTS' RUNS MOVE — THAT IS WHAT THE GATHER IS FOR. The desugar computes every expert
/// and weights the unchosen ones by zero (a mask the VM can run); a chip that did the same would stream all
/// `ne` experts' weights to use `k` of them. Expert `b`'s gate row `o` lives at `(b·dff + o)·d + t` — the
/// affine `o·d + t` past its run's first element `b·(dff·d)` — so each round loads the whole tiles around its
/// expert's run of each table and reads the run at its offset in the page (`ExpertTable`): `k` runs move,
/// and an expert index past the table faults. A table no more tiles than its `k` windows loads once, and the
/// rounds read their runs out of it through an [`IndirectTerm`] of stride the run and extent `ne`.
///
/// Three sections: the router (a reduction), the row softmax (`flat_softmax_rows`), and the routing plus
/// the experts. The weights sum is divided out once at the end: `Σ P[b]·y_b / Σ P[b]` is the desugar's
/// `Σ (P[b]/Σ P)·y_b`.
pub fn from_moe(
    spec: &MoeSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let (d, dff, ne, k) = (spec.d, spec.dff, spec.ne, spec.k);
    if d == 0 || dff == 0 || ne == 0 || k == 0 || k > ne {
        return Err(SpatialReject::NotLowered("a mixture-of-experts block with no width, no experts, or more chosen experts than there are"));
    }
    if spec.weigh_inputs && spec.renormalize {
        return Err(SpatialReject::NotLowered(
            "a mixture whose renormalized weights scale its experts' inputs — each round is weighed as it is picked",
        ));
    }
    let operands = MoeOperands::of(spec);
    if moe_blocks(spec, target) {
        return blocked_moe(spec, names, target);
    }
    let (x_name, x_fill) = names.operand(0);
    let (wr_name, wr_fill) = names.operand(1);
    let (wg_name, wg_fill) = names.operand(2);
    let (wu_name, wu_fill) = names.operand(3);
    let (wd_name, wd_fill) = names.operand(4);
    let logits_name = format!("{}_logits", names.out);
    let probs_name = format!("{}_probs", names.out);
    let row = |var: LoopVar, stride: i64| ElemAccess {
        base: 0,
        terms: vec![(var, stride)],
        indirect: None,
    };

    // ★ THE ROUTER DECIDES THE EXPERTS, SO ITS LOGITS AND SCORES STAY AT THE ACCUMULATOR'S WIDTH, as the blocked form
    // keeps them: two experts whose scores differ by less than the datapath's rounding would otherwise be picked as the
    // datapath rounds them, not as the model scores them. X and Wr are read as the datapath stores them — X is one
    // tensor, so one datatype, in every section of the block.
    let route = AccPolicy::for_result(Datapath::Float.precision(target)?).interm_precision();
    // ── Section 1: the router logits, `L[e] = Σ_t x[t]·Wr[t·ne + e]`.
    let router = {
        let mut f = FlatBuild::new(target)?;
        let x = f.input(x_name, d, x_fill, target);
        let wr = f.input(wr_name, d * ne, wr_fill, target);
        let out_buf = f.t.buffer("logits", tiles_of(ne, target), route);
        // A router bias adds after the product: the product lands in its own page first.
        let product = match operands.router_bias {
            Some(_) => f.t.scratch_deep("router_product", 1, tiles_of(ne, target), route),
            None => out_buf,
        };
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
                extent: ne,
                kind: IterKind::Parallel,
            },
        ];
        let s = f.compute(
            "router",
            iters,
            vec![
                Slot {
                    buf: x,
                    map: row(LV_GEN_ROW, 1),
                },
                Slot {
                    buf: wr,
                    map: map2(0, ne as i64, 1),
                },
            ],
            per_col_map(),
            *mul(op(0), op(1)),
            product,
        );
        let mut chain = vec![s];
        if let Some(bi) = operands.router_bias {
            let (b_name, b_fill) = names.operand(bi);
            let b = f.input_as(b_name, ne, b_fill, route, target);
            chain.push(f.compute(
                "router_bias",
                flat_space(ne),
                vec![Slot { buf: product, map: at(0) }, Slot { buf: b, map: at(0) }],
                ident_map(wide(tiles_of(ne, target), target)),
                *add(op(0), op(1)),
                out_buf,
            ));
        }
        let claim = approx("moe router", route);
        f.finish_as(&logits_name, ne, chain, out_buf, format!("moe_router_{d}x{ne}"), claim, route, target)?
    };

    // ── Section 2: the router's scores, a softmax or each logit's sigmoid.
    let softmax = match spec.score {
        MoeScore::Softmax => {
            flat_softmax_rows_at((&logits_name, &Fill::Unknown), &probs_name, 1, ne, format!("moe_softmax_{ne}"), route, None, target)?
        }
        MoeScore::Sigmoid => moe_sigmoid(&logits_name, &probs_name, ne, FlatBuild::at(route, target)?, target)?,
    };

    // ── Section 3: `k` routing rounds, each gathering one expert's weights, then the renormalized combine.
    let experts = {
        let mut f = FlatBuild::new(target)?;
        // ★ ONE PAGE OF EACH INTERMEDIATE. The section is one work item, and every round's intermediates are written
        // once and read within the item's own pass, so none needs a second page — `K` rounds of double-buffered
        // pages are what overflow a site once a mixture's biases add three more a round.
        let acc_p = AccPolicy::for_result(f.precision).interm_precision();
        let page = |f: &mut FlatBuild, name: &str, tiles: usize| f.t.scratch_deep(name, 1, tiles, acc_p);
        let p = f.input_as(&probs_name, ne, &Fill::Unknown, route, target);
        let x = f.input(x_name, d, x_fill, target);
        let wg = ExpertTable::new(&mut f, (wg_name, wg_fill), ne, dff * d, k, target);
        let wu = ExpertTable::new(&mut f, (wu_name, wu_fill), ne, dff * d, k, target);
        let wd = ExpertTable::new(&mut f, (wd_name, wd_fill), ne, d * dff, k, target);
        let one_w = wide(1, target);
        let (dff_t, d_t) = (tiles_of(dff, target), tiles_of(d, target));
        let (dffw, dw) = (wide(dff_t, target), wide(d_t, target));
        // `into[o] = Σ_t x[t]·W[o·fold + t]` over the chosen expert's run of `W`.
        let expert_gemv = |f: &mut FlatBuild, name: &str, run: ExpertRun, x: BufferId, fold: usize, width: usize, into: BufferId| {
            let iters = vec![
                IterSpec {
                    var: LV_GEN_ROW,
                    extent: fold,
                    kind: IterKind::Reduce {
                        pool: ReducePool::Sum,
                    },
                },
                IterSpec {
                    var: LV_GEN_COL,
                    extent: width,
                    kind: IterKind::Parallel,
                },
            ];
            let mut slots = vec![
                Slot {
                    buf: x,
                    map: row(LV_GEN_ROW, 1),
                },
                Slot {
                    buf: run.page,
                    map: run.map(vec![(LV_GEN_COL, fold as i64), (LV_GEN_ROW, 1)], 2),
                },
            ];
            slots.extend(run.slot());
            f.compute(name, iters, slots, per_col_map(), *mul(op(0), op(1)), into)
        };
        // Each expert bias table, `ne` runs of its projection's width.
        let bias_tables: Vec<Option<ExpertTable>> = (0..3)
            .map(|i| {
                operands.biases[i].map(|bi| {
                    let width = if i == 2 { d } else { dff };
                    ExpertTable::new(&mut f, names.operand(bi), ne, width, k, target)
                })
            })
            .collect();
        // `out[c] = y[c] + B[c]` over the chosen expert's run of `B`: a projection's output and its expert's bias row.
        let with_bias = |f: &mut FlatBuild, name: &str, y: BufferId, run: ExpertRun, width: usize, tiles: usize| {
            let into = page(f, name, tiles);
            let mut slots = vec![Slot { buf: y, map: at(0) }, Slot { buf: run.page, map: run.map(vec![(LV_GEN_COL, 1)], 2) }];
            slots.extend(run.slot());
            let stage = f.compute(name, flat_space(width), slots, ident_map(wide(tiles, target)), *add(op(0), op(1)), into);
            (stage, into)
        };
        // An expert pick is in `[0, ne)`; its end is one past.
        let ix = index_type(target, ne as u64 + 1)?;
        let (mut chain, choosing) = moe_choosing(&mut f, spec, names, &operands, p, target)?;
        let mut acc: Option<BufferId> = None;
        let mut wsum: Option<BufferId> = None;
        let mut picked = Vec::new();
        for r in 0..k {
            // The arg-max of what is left: the chosen expert, 0-based, on the integer datapath.
            let (pick, end) = moe_pick(&mut f, &mut chain, ("pick", "route"), r, (choosing, map2(0, 0, 1), ne), &picked, r + 1 < k, ix, target);
            picked.extend(end.map(|end| (pick, end)));
            // Its router weight, gathered out of P.
            let weight = page(&mut f, &format!("weight{r}"), 1);
            let from_p = ElemAccess {
                base: 0,
                terms: Vec::new(),
                indirect: Some(IndirectTerm {
                    operand: 1,
                    stride: 1,
                    modulus: ne,
                }),
            };
            chain.push(f.compute(
                &format!("weight{r}"),
                flat_space(1),
                vec![
                    Slot {
                        buf: p,
                        map: from_p,
                    },
                    Slot {
                        buf: pick,
                        map: fixed(0),
                    },
                ],
                ident_map(one_w),
                round_weight(spec),
                weight,
            ));
            // The input the expert reads: `x`, or `x` scaled by its weight when the weights scale the inputs.
            let x_in = if spec.weigh_inputs {
                let xs = page(&mut f, &format!("scaled_x{r}"), d_t);
                chain.push(f.compute(
                    &format!("scaled_x{r}"),
                    flat_space(d),
                    vec![Slot { buf: weight, map: fixed(0) }, Slot { buf: x, map: at(0) }],
                    ident_map(dw),
                    *mul(op(0), op(1)),
                    xs,
                ));
                xs
            } else {
                x
            };
            // The chosen expert's SwiGLU: gate and up gathered GEMVs, the gated product, the gathered down GEMV.
            let gate = page(&mut f, &format!("gate{r}"), dff_t);
            let run = wg.run(&mut f, &mut chain, &format!("gate{r}"), pick, ne, target)?;
            chain.push(expert_gemv(&mut f, &format!("gate{r}"), run, x_in, d, dff, gate));
            let up = page(&mut f, &format!("up{r}"), dff_t);
            let run = wu.run(&mut f, &mut chain, &format!("up{r}"), pick, ne, target)?;
            chain.push(expert_gemv(&mut f, &format!("up{r}"), run, x_in, d, dff, up));
            let (gate, up) = {
                let mut biased = [gate, up];
                for (i, part) in ["gate", "up"].into_iter().enumerate() {
                    if let Some(table) = &bias_tables[i] {
                        let run = table.run(&mut f, &mut chain, &format!("{part}_bias{r}"), pick, ne, target)?;
                        let (stage, into) = with_bias(&mut f, &format!("{part}_bias{r}"), biased[i], run, dff, dff_t);
                        chain.push(stage);
                        biased[i] = into;
                    }
                }
                (biased[0], biased[1])
            };
            let hidden = page(&mut f, &format!("hidden{r}"), dff_t);
            chain.push(f.compute(
                &format!("hidden{r}"),
                flat_space(dff),
                vec![
                    Slot {
                        buf: gate,
                        map: at(0),
                    },
                    Slot {
                        buf: up,
                        map: at(0),
                    },
                ],
                ident_map(dffw),
                *moe_gated(spec.act, op(0), op(1)),
                hidden,
            ));
            let y = page(&mut f, &format!("down{r}"), d_t);
            let run = wd.run(&mut f, &mut chain, &format!("down{r}"), pick, ne, target)?;
            chain.push(expert_gemv(&mut f, &format!("down{r}"), run, hidden, dff, d, y));
            let y = match &bias_tables[2] {
                Some(table) => {
                    let run = table.run(&mut f, &mut chain, &format!("down_bias{r}"), pick, ne, target)?;
                    let (stage, into) = with_bias(&mut f, &format!("down_bias{r}"), y, run, d, d_t);
                    chain.push(stage);
                    into
                }
                None => y,
            };
            // Accumulate the weighted expert output and the weight; an expert whose input took its weight adds as it is.
            let next_acc = page(&mut f, &format!("acc{r}"), d_t);
            chain.push(match acc {
                None if spec.weigh_inputs => f.compute(
                    &format!("acc{r}"),
                    flat_space(d),
                    vec![Slot { buf: y, map: at(0) }],
                    ident_map(dw),
                    TensorBody::Operand(0),
                    next_acc,
                ),
                Some(prev) if spec.weigh_inputs => f.compute(
                    &format!("acc{r}"),
                    flat_space(d),
                    vec![Slot { buf: prev, map: at(0) }, Slot { buf: y, map: at(0) }],
                    ident_map(dw),
                    *add(op(0), op(1)),
                    next_acc,
                ),
                None => f.compute(
                    &format!("acc{r}"),
                    flat_space(d),
                    vec![
                        Slot {
                            buf: weight,
                            map: fixed(0),
                        },
                        Slot { buf: y, map: at(0) },
                    ],
                    ident_map(dw),
                    *mul(op(0), op(1)),
                    next_acc,
                ),
                Some(prev) => f.compute(
                    &format!("acc{r}"),
                    flat_space(d),
                    vec![
                        Slot {
                            buf: prev,
                            map: at(0),
                        },
                        Slot {
                            buf: weight,
                            map: fixed(0),
                        },
                        Slot { buf: y, map: at(0) },
                    ],
                    ident_map(dw),
                    *add(op(0), mul(op(1), op(2))),
                    next_acc,
                ),
            });
            acc = Some(next_acc);
            wsum = Some(match wsum {
                None => weight,
                Some(prev) => {
                    let next = page(&mut f, &format!("wsum{r}"), 1);
                    chain.push(f.compute(
                        &format!("wsum{r}"),
                        flat_space(1),
                        vec![
                            Slot {
                                buf: prev,
                                map: fixed(0),
                            },
                            Slot {
                                buf: weight,
                                map: fixed(0),
                            },
                        ],
                        ident_map(one_w),
                        *add(op(0), op(1)),
                        next,
                    ));
                    next
                }
            });
        }
        let (acc, wsum) = (acc.expect("k ≥ 1 rounds"), wsum.expect("k ≥ 1 rounds"));
        let out_buf = f.t.buffer("out", d_t, f.precision);
        chain.push(f.compute(
            "combine",
            flat_space(d),
            vec![
                Slot {
                    buf: acc,
                    map: at(0),
                },
                Slot {
                    buf: wsum,
                    map: fixed(0),
                },
            ],
            ident_map(dw),
            match (spec.renormalize, spec.scale) {
                (true, None) => *mul(op(0), unary(SfpuKind::Recip, renormalizing_sum(spec, op(1)))),
                (true, Some(_)) => *mul(mul(op(0), unary(SfpuKind::Recip, renormalizing_sum(spec, op(1)))), konst(spec.scale())),
                (false, _) => TensorBody::Operand(0),
            },
            out_buf,
        ));
        let claim = approx("mixture of experts", f.precision);
        f.finish(
            &names.out,
            d,
            chain,
            out_buf,
            format!("moe_{ne}x{k}_{d}x{dff}"),
            claim,
            target,
        )?
    };
    Ok(vec![router, softmax, experts])
}

// ── Mixture of experts at real width ──────────────────────────────────────────────────────────────────
//
// ★ ONLY THE CHOSEN EXPERTS' WEIGHTS MOVE, AND THEY MOVE BY TILE. Expert `e`'s gate rows are tile rows
// `[e·dff/32, (e+1)·dff/32)` of the stacked `(E·dff) × d` weight — one contiguous run of `(dff/32)·(d/32)` tiles —
// so each chosen expert's GEMV reads its weights through an [`IndirectTile`] whose index is the expert's pick,
// and the other `E − K` experts never leave memory. A GEMV row is a DOT PRODUCT across a tile row of the weight,
// and a row of Qwen3-30B-A3B's `d = 2048` is 64 tiles; so the reduction streams in CHUNKS of the row, each item a
// block of 32 outputs, each chunk a partial sum, and a second section adds the partials. Seven sections:
//
//   router (the matmul it is) → softmax → routing (top-K picks and weights, at the index datatype)
//   → gate/up partials (tile-gathered) → hidden = silu(Σ gate)·(Σ up) → down partials (tile-gathered)
//   → combine: Σ_r w_r·(Σ down_r) / Σ_r w_r
//
// The partials and the hidden activation stay at the accumulator's width; only the block's output leaves at the
// datapath's precision.

/// A flat table of `ne` expert runs of `span` elements each, read by every routing round at its chosen expert's run.
///
/// ★ A ROUND MOVES ITS EXPERT'S RUN, OR THE TABLE MOVES ONCE, WHICHEVER IS FEWER TILES. Run `e` starts at element
/// `e·span`, mid-tile unless `span` is whole tiles. A round loads the window of whole tiles around its run — `W` tiles,
/// the most any run reaches from its first element's offset in its tile, started at `min(⌊e·span/32⌋, T − W)` so it ends
/// inside the table — and reads the run at its offset `e·span − 32·start` in the page; a run of whole tiles is its own
/// window, addressed by the pick alone. `k` rounds move `k·W` tiles of a `T`-tile table; where that is not fewer, the
/// table loads once and each round reads its run out of it by element. Either way the section's one item fills each
/// page once, so each is one page deep.
pub(super) enum ExpertTable {
    Whole { page: BufferId, span: usize },
    Windows { tensor: TensorId, span: usize, window_t: usize, table_t: usize },
}

/// How a round reads its chosen expert's run: the page holding it, and the element term that moves the read to the
/// run — `(index buffer, stride, extent)`: the pick by the run's length in a whole table, the run's offset in a window
/// that does not start at it.
pub(super) struct ExpertRun {
    page: BufferId,
    by: Option<(BufferId, i64, usize)>,
}

impl ExpertRun {
    /// The map reading element `Σ terms` of the run, its moving term reading the stage's operand `operand`.
    fn map(&self, terms: Vec<(LoopVar, i64)>, operand: usize) -> ElemAccess {
        ElemAccess {
            base: 0,
            terms,
            indirect: self.by.map(|(_, stride, modulus)| IndirectTerm { operand, stride, modulus }),
        }
    }

    /// The operand the moving term reads, when the run has one.
    fn slot(&self) -> Option<Slot> {
        self.by.map(|(buf, _, _)| Slot { buf, map: fixed(0) })
    }
}

impl ExpertTable {
    fn new(f: &mut FlatBuild, (name, fill): (&str, &Fill), ne: usize, span: usize, k: usize, target: &SpatialTarget) -> ExpertTable {
        let tw = target.tile.cols;
        let table_t = tiles_of(ne * span, target);
        let window_t = (0..ne).map(|e| (e * span % tw + span).div_ceil(tw)).max().unwrap_or(1).min(table_t);
        if k * window_t >= table_t {
            let precision = f.precision;
            return ExpertTable::Whole { page: f.input_paged(name, ne * span, fill, precision, 1, target), span };
        }
        let tensor = f.t.tensor(name, TensorRole::Input, 1, ne * span, f.precision, crate::spatial::passes::from_tensor_op::init_of(fill, ne * span));
        ExpertTable::Windows { tensor, span, window_t, table_t }
    }

    /// Round `part`'s run, chosen by `pick`: the window's index and offset stages pushed onto `chain`, its load wired to
    /// the stage that first reads the page.
    fn run(
        &self,
        f: &mut FlatBuild,
        chain: &mut Vec<StageId>,
        part: &str,
        pick: BufferId,
        ne: usize,
        target: &SpatialTarget,
    ) -> Result<ExpertRun, SpatialReject> {
        let (tensor, span, window_t, table_t) = match *self {
            ExpertTable::Whole { page, span } => return Ok(ExpertRun { page, by: Some((pick, span as i64, ne)) }),
            ExpertTable::Windows { tensor, span, window_t, table_t } => (tensor, span, window_t, table_t),
        };
        let tw = target.tile.cols;
        let one = wide(1, target);
        let ix = index_type(target, (ne * span + tw) as u64)?;
        let whole_tiles = span % tw == 0;
        let first = || TensorBody::Min(Box::new(TensorBody::FloorDiv(mul(op(0), konst(span as f64)), konst(tw as f64))), konst((table_t - window_t) as f64));
        // Each gather retires its index page, so the load reads a page of its own: the pick, or the window's first tile.
        let index = f.t.scratch_deep(&format!("{part}_window"), 1, 1, ix);
        let body = if whole_tiles { TensorBody::Operand(0) } else { first() };
        let s = f.compute(&format!("{part}_window"), flat_space(1), vec![Slot { buf: pick, map: fixed(0) }], ident_map(one), body, index);
        chain.push(s);
        let indirect = if whole_tiles {
            IndirectTile { index, tile_stride: window_t, elems_per_tile: 1, bound: (0, ne - 1) }
        } else {
            IndirectTile { index, tile_stride: 1, elems_per_tile: 1, bound: (0, table_t - window_t) }
        };
        let page = f.t.buffer_deep(&format!("in_{part}"), 1, window_t, f.precision);
        let l = f.t.stage(
            &format!("load_{part}"),
            StageKind::Load {
                tensor,
                pattern: AccessPattern {
                    origin: TileAddr { base: 0, steps: Vec::new(), indirect: Some(indirect) },
                    block_h: 1,
                    block_w: window_t,
                    stride_w: 1,
                    stride_h: window_t,
                },
                num_blocks: 1,
                multicast: None,
            },
            vec![index],
            vec![page],
        );
        f.t.edge(s, index, l);
        f.loads.push((l, page));
        if whole_tiles {
            return Ok(ExpertRun { page, by: None });
        }
        let offset = f.t.scratch_deep(&format!("{part}_offset"), 1, 1, ix);
        chain.push(f.compute(
            &format!("{part}_offset"),
            flat_space(1),
            vec![Slot { buf: pick, map: fixed(0) }],
            ident_map(one),
            *sub(mul(op(0), konst(span as f64)), mul(Box::new(first()), konst(tw as f64))),
            offset,
        ));
        Ok(ExpertRun { page, by: Some((offset, 1, window_t * tw - span + 1)) })
    }
}

/// Whether a mixture-of-experts block takes the blocked form: widths in whole tiles, and a router softmax and a
/// pick table that one accumulator page holds.
pub(super) fn moe_blocks(spec: &MoeSpec, target: &SpatialTarget) -> bool {
    let tw = target.tile.cols;
    spec.d % tw == 0
        && spec.dff % tw == 0
        && (!row_needs_blocking(spec.ne, target) || spec.groups.is_none())
}

/// The names of a blocked MoE's internal tensors, all derived from the block's binder.
pub(super) struct MoeNames {
    /// The router's product before its bias, when it has one.
    router_product: String,
    logits: String,
    probs: String,
    picks: String,
    weights: String,
    gate_part: String,
    up_part: String,
    hidden: String,
    down_part: String,
    scaler: String,
}

impl MoeNames {
    fn of(names: &StageNames) -> MoeNames {
        let n = |s: &str| format!("{}__moe_{s}", names.out);
        MoeNames {
            router_product: n("router_product"),
            logits: n("logits"),
            probs: n("probs"),
            picks: n("picks"),
            weights: n("weights"),
            gate_part: n("gate_part"),
            up_part: n("up_part"),
            hidden: n("hidden"),
            down_part: n("down_part"),
            scaler: n("scaler"),
        }
    }
}

/// The largest divisor of `n` (a chunk of a row's tiles) whose section fits a site — each candidate is BUILT and
/// measured, so the sizing can never disagree with the section it sizes. When none fits, a chunk of one tile that
/// cannot be BUILT is refused with its own reason, and one that is built but too large as `too_large`.
pub(super) fn fitting_chunk(
    n: usize,
    build: &dyn Fn(usize) -> Result<SpatialProgram, SpatialReject>,
    too_large: &'static str,
    target: &SpatialTarget,
) -> Result<usize, SpatialReject> {
    let budget = target.max_local_mem_bytes();
    if let Some(c) = (1..=n)
        .rev()
        .filter(|c| n % c == 0)
        .find(|c| build(*c).is_ok_and(|p| site_footprint(&p) <= budget))
    {
        return Ok(c);
    }
    build(1)?;
    Err(SpatialReject::NotLowered(too_large))
}

pub(super) fn blocked_moe(
    spec: &MoeSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let (d, dff, ne) = (spec.d, spec.dff, spec.ne);
    let tw = target.tile.cols;
    let (d_t, f_t) = (d / tw, dff / tw);
    let mn = MoeNames::of(names);
    let (x_name, x_fill) = names.operand(0);
    let (wr_name, wr_fill) = names.operand(1);
    // The router is `x · Wr` — a `1 × d` by `d × E` product, streamed by the matmul lowering.
    let router_spec = crate::tensor_recognize::MatmulSpec {
        m: 1,
        n: ne,
        k: d,
        a: Default::default(),
        b: Default::default(),
        c: Default::default(),
    };
    let operands = MoeOperands::of(spec);
    let router_names = MatmulNames {
        a: x_name.to_string(),
        b: wr_name.to_string(),
        c: if operands.router_bias.is_some() { mn.router_product.clone() } else { mn.logits.clone() },
    };
    let router = crate::spatial::passes::from_tensor_op::from_matmul(
        &router_spec,
        &router_names,
        (x_fill, wr_fill),
        (None, None),
        NumericsPolicy::Exact,
        target,
    )?;
    // ★ X IS ONE TENSOR, SO ONE DATATYPE. The router reads it as a matmul operand; the expert chunks read the same
    // bytes, so every section declares X at the operand precision the router's matmul chose.
    let x_dtype = router
        .tensors
        .iter()
        .find(|t| t.name == x_name)
        .map(|t| t.dtype)
        .ok_or(SpatialReject::NotLowered(
            "the router matmul does not read the block's input",
        ))?;
    // ★ A ROUTER ROW WIDER THAN ONE ACCUMULATOR PAGE (more than 256 experts: Kimi K2's 384, the 512 of Qwen3-Next, the
    // Qwen 3.5 mixtures and qwen4_exp) is scored and routed in blocks of the row.
    let wide_row = row_needs_blocking(ne, target);
    // A router bias adds to the product, in a section of its own.
    let router_bias = match operands.router_bias {
        None => None,
        Some(bi) if wide_row => Some(wide_router_bias(&mn, names.operand(bi), ne, target)?),
        Some(bi) => {
            let mut f = FlatBuild::at(AccPolicy::F32.interm_precision(), target)?;
            let product = f.input(&mn.router_product, ne, &Fill::Unknown, target);
            let (b_name, b_fill) = names.operand(bi);
            let b = f.input(b_name, ne, b_fill, target);
            let out = f.t.buffer("logits", tiles_of(ne, target), f.precision);
            let s = f.compute(
                "router_bias",
                flat_space(ne),
                vec![Slot { buf: product, map: at(0) }, Slot { buf: b, map: at(0) }],
                ident_map(wide(tiles_of(ne, target), target)),
                *add(op(0), op(1)),
                out,
            );
            let claim = approx("moe router bias", f.precision);
            Some(f.finish(&mn.logits, ne, vec![s], out, format!("moe_router_bias_{ne}"), claim, target)?)
        }
    };
    // The router's logits and scores stay at the accumulator's width: they decide the experts.
    let scores = match (spec.score, wide_row) {
        (MoeScore::Softmax, true) => wide_softmax(&mn, ne, target)?,
        (MoeScore::Sigmoid, true) => vec![wide_sigmoid(&mn, ne, target)?],
        (MoeScore::Softmax, false) => vec![flat_softmax_rows_at(
            (&mn.logits, &Fill::Unknown),
            &mn.probs,
            1,
            ne,
            format!("moe_softmax_{ne}"),
            AccPolicy::F32.interm_precision(),
            None,
            target,
        )?],
        (MoeScore::Sigmoid, false) => vec![moe_sigmoid(&mn.logits, &mn.probs, ne, FlatBuild::at(AccPolicy::F32.interm_precision(), target)?, target)?],
    };
    let routing = if wide_row { wide_moe_routing(spec, names, &operands, &mn, target)? } else { vec![moe_routing(spec, names, &operands, &mn, target)?] };
    let cd = fitting_chunk(
        d_t,
        &|c| moe_up_partials(spec, names, &mn, c, x_dtype, target),
        "a mixture-of-experts gate/up chunk of one tile does not fit a site",
        target,
    )?;
    let up = moe_up_partials(spec, names, &mn, cd, x_dtype, target)?;
    let hidden = moe_hidden(spec, names, &operands, &mn, d_t / cd, target)?;
    let cdd = fitting_chunk(
        f_t,
        &|c| moe_down_partials(spec, names, &mn, c, target),
        "a mixture-of-experts down chunk of one tile does not fit a site",
        target,
    )?;
    let down = moe_down_partials(spec, names, &mn, cdd, target)?;
    let combine = moe_combine(spec, names, &operands, &mn, f_t / cdd, target)?;
    Ok([router].into_iter().chain(router_bias).chain(scores).chain(routing).chain([up, hidden, down, combine]).collect())
}

/// The routing section: `K` rounds of arg-max over what is left of the router probabilities, each pick (at the
/// index datatype) and its probability written to tile `r` of the pick and weight tables — a tile per pick, so
/// each is element 0 of the page an expert's tile gather waits on.
pub(super) fn moe_routing(
    spec: &MoeSpec,
    names: &StageNames,
    operands: &MoeOperands,
    mn: &MoeNames,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (ne, k) = (spec.ne, spec.k);
    let tw = target.tile.cols;
    let acc = AccPolicy::F32.interm_precision();
    let ix = index_type(target, ne as u64 + 1)?;
    let mut f = FlatBuild::new(target)?;
    f.precision = acc;
    // ★ ONE PAGE OF EVERYTHING. The section is one item: each page is filled once and read within the item's pass, so a
    // second page buys nothing, and at f32 every scalar is a whole tile — two pages of each is what overflowed a site at
    // DeepSeek-V3's 256 experts, top 8 of the top 4 of 8 groups.
    let p = f.input_paged(&mn.probs, ne, &Fill::Unknown, acc, 1, target);
    let one_w = wide(1, target);
    let (mut chain, choosing) = moe_choosing(&mut f, spec, names, operands, p, target)?;
    let mut picks = Vec::new();
    let mut weights = Vec::new();
    // ★ NO PAGE A ROUND. Each round's arg-max reads the one choosing row less a penalty on the earlier picks, scalars all;
    // a row of masks a round is what overflowed a site at 256 experts, top 8, at f32.
    let mut picked = Vec::new();
    for r in 0..k {
        let (pick, end) = moe_pick(&mut f, &mut chain, ("pick", "route"), r, (choosing, map2(0, 0, 1), ne), &picked, r + 1 < k, ix, target);
        picked.extend(end.map(|end| (pick, end)));
        let weight = f.t.scratch_deep(&format!("weight{r}"), 1, 1, acc);
        let from_p = ElemAccess {
            base: 0,
            terms: Vec::new(),
            indirect: Some(IndirectTerm {
                operand: 1,
                stride: 1,
                modulus: ne,
            }),
        };
        chain.push(f.compute(
            &format!("weight{r}"),
            flat_space(1),
            vec![
                Slot {
                    buf: p,
                    map: from_p,
                },
                Slot {
                    buf: pick,
                    map: fixed(0),
                },
            ],
            ident_map(one_w),
            round_weight(spec),
            weight,
        ));
        picks.push(pick);
        weights.push(weight);
    }
    let kw = wide(k, target);
    let pack = |f: &mut FlatBuild, name: &str, parts: &[BufferId], dt: Precision| {
        let slots: Vec<Slot> = parts
            .iter()
            .map(|b| Slot {
                buf: *b,
                map: fixed(0),
            })
            .collect();
        let bands: Vec<(usize, TensorBody)> = (0..parts.len())
            .map(|r| ((r + 1) * tw, TensorBody::Operand(r)))
            .collect();
        let out = f.t.buffer_deep(name, 1, k, dt);
        (
            f.compute(
                &format!("pack_{name}"),
                flat_space(k * tw),
                slots,
                ident_map(kw),
                banded(bands),
                out,
            ),
            out,
        )
    };
    let (sp, pick_out) = pack(&mut f, "picks", &picks, ix);
    let (sw, weight_out) = pack(&mut f, "weights", &weights, acc);
    chain.push(sp);
    chain.push(sw);
    let pick_t = f.t.tensor(
        &mn.picks,
        TensorRole::Output,
        1,
        k * tw,
        ix,
        TensorInit::Zero,
    );
    let weight_t = f.t.tensor(
        &mn.weights,
        TensorRole::Output,
        1,
        k * tw,
        acc,
        TensorInit::Zero,
    );
    let st_p = f.t.store("store_picks", pick_t, Vec::new(), k, pick_out);
    let st_w =
        f.t.store("store_weights", weight_t, Vec::new(), k, weight_out);
    let claim = approx("mixture-of-experts routing", f.precision);
    f.wire_and_finish_many(
        chain,
        &[(pick_out, st_p), (weight_out, st_w)],
        k,
        format!("moe_routing_{ne}x{k}"),
        claim,
        target,
    )
}

/// The blocks of a router row of `ne` experts, `(first tile, experts)` each: an accumulator page of experts a block, the
/// last the rest.
pub(super) fn router_blocks(ne: usize, target: &SpatialTarget) -> Vec<(usize, usize)> {
    let blk = row_block_tiles(target);
    let span = blk * target.tile.cols;
    (0..ne.div_ceil(span)).map(|b| (b * blk, span.min(ne - b * span))).collect()
}

/// A wide router's bias, `L[e] = (x·Wr)[e] + B[e]`, a block of the row an item.
pub(super) fn wide_router_bias(mn: &MoeNames, (b_name, b_fill): (&str, &Fill), ne: usize, target: &SpatialTarget) -> Result<SpatialProgram, SpatialReject> {
    let mut f = FlatBuild::columns(ne, target)?;
    f.precision = AccPolicy::F32.interm_precision();
    let product = f.input(&mn.router_product, ne, &Fill::Unknown, target);
    let b = f.input(b_name, ne, b_fill, target);
    let out = f.result_page("logits", ne, f.precision, target);
    let chain = f.pointwise("router_bias", ne, vec![Slot { buf: product, map: at(0) }, Slot { buf: b, map: at(0) }], *add(op(0), op(1)), out, target)?;
    let claim = approx("moe router bias", f.precision);
    f.finish(&mn.logits, ne, chain, out, format!("moe_router_bias_{ne}"), claim, target)
}

/// A wide router's sigmoid scores, `P[e] = 1/(1 + e^(0 − L[e]))` as [`moe_sigmoid`] writes them, a block of the row an
/// item.
pub(super) fn wide_sigmoid(mn: &MoeNames, ne: usize, target: &SpatialTarget) -> Result<SpatialProgram, SpatialReject> {
    let mut f = FlatBuild::columns(ne, target)?;
    f.precision = AccPolicy::F32.interm_precision();
    let l = f.input(&mn.logits, ne, &Fill::Unknown, target);
    let out = f.result_page("probs", ne, f.precision, target);
    let body = *unary(SfpuKind::Recip, add(konst(1.0), unary(SfpuKind::Exp, sub(konst(0.0), op(0)))));
    let chain = f.pointwise("sigmoid", ne, vec![Slot { buf: l, map: at(0) }], body, out, target)?;
    let claim = approx("moe router sigmoid", f.precision);
    f.finish(&mn.probs, ne, chain, out, format!("moe_sigmoid_{ne}"), claim, target)
}

/// A wide router's softmax: [`softmax_row_statistics`] over the logits, then `e^(L − m)·(1/Z)` a block of the row an item.
pub(super) fn wide_softmax(mn: &MoeNames, ne: usize, target: &SpatialTarget) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let acc = AccPolicy::F32.interm_precision();
    let (mut out, norm) = softmax_row_statistics((&mn.logits, &Fill::Unknown, acc), &mn.probs, ne, ("moe_softmax", "moe router softmax"), target)?;
    let mut f = FlatBuild::columns(ne, target)?;
    f.precision = acc;
    let l = f.input(&mn.logits, ne, &Fill::Unknown, target);
    let nrm = f.input_paged(&norm, 2, &Fill::Unknown, acc, 1, target);
    let probs = f.result_page("probs", ne, acc, target);
    let body = *mul(unary(SfpuKind::Exp, sub(op(0), op(1))), unary(SfpuKind::Recip, op(2)));
    let chain = f.pointwise(
        "softmax",
        ne,
        vec![Slot { buf: l, map: at(0) }, Slot { buf: nrm, map: fixed(0) }, Slot { buf: nrm, map: fixed(1) }],
        body,
        probs,
        target,
    )?;
    let claim = approx("moe router softmax", acc);
    out.push(f.finish(&mn.probs, ne, chain, probs, format!("moe_softmax_{ne}"), claim, target)?);
    Ok(out)
}

/// A wide router's routing. Each block of the row (a section a block) computes its choosing scores as [`moe_choosing`]
/// does and takes its top `K` by [`moe_pick`], packing each pick's expert, choosing score and weight `P[e]` into a tile
/// of candidates, an unused slot's score masked. One section then takes `K` rounds of [`moe_pick`] over the blocks'
/// candidates in row order, each round's pick the candidate's expert and its weight the candidate's, and writes the
/// pick and weight tables as [`moe_routing`] does. The row's top `K` are each among their block's, and among equal
/// scores a round's lowest candidate is its lowest expert, so the picks are the row scan's. A group limit over a row
/// this wide is refused.
pub(super) fn wide_moe_routing(
    spec: &MoeSpec,
    names: &StageNames,
    operands: &MoeOperands,
    mn: &MoeNames,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    if spec.groups.is_some() {
        return Err(SpatialReject::NotLowered("a group limit over a router row wider than an accumulator page"));
    }
    let (ne, k) = (spec.ne, spec.k);
    let tw = target.tile.cols;
    let acc = AccPolicy::F32.interm_precision();
    let ix = index_type(target, ne as u64 + 1)?;
    let one_w = wide(1, target);
    let blocks = router_blocks(ne, target);
    let cand = |kind: &str, b: usize| format!("{}__cand_{kind}{b}", mn.picks);
    // A tile of `parts`' scalars, part `r` at element `r`, every later element `rest`.
    let pack = |f: &mut FlatBuild, chain: &mut Vec<StageId>, name: &str, parts: &[BufferId], rest: f64, dt: Precision| {
        let slots = parts.iter().map(|&buf| Slot { buf, map: fixed(0) }).collect();
        let mut bands: Vec<(usize, TensorBody)> = (0..parts.len()).map(|r| (r + 1, TensorBody::Operand(r))).collect();
        bands.push((tw, TensorBody::Const(rest.to_bits())));
        let out = f.t.buffer_deep(name, 1, 1, dt);
        chain.push(f.compute(&format!("pack_{name}"), flat_space(tw), slots, ident_map(one_w), banded(bands), out));
        out
    };
    let mut out = Vec::new();
    for (b, &(first, n)) in blocks.iter().enumerate() {
        let mut f = FlatBuild::at(acc, target)?;
        let p = f.input_slice(&mn.probs, ne, first, n, &Fill::Unknown, acc, target);
        let n_t = tiles_of(n, target);
        let mut chain = Vec::new();
        let choosing = match (operands.selection_bias, spec.score) {
            (Some(bi), _) => {
                let (b_name, b_fill) = names.operand(bi);
                let bias = f.input_slice(b_name, ne, first, n, b_fill, acc, target);
                let sel = f.t.scratch_deep("selection", 1, n_t, acc);
                chain.push(f.compute(
                    "selection",
                    flat_space(n),
                    vec![Slot { buf: p, map: at(0) }, Slot { buf: bias, map: at(0) }],
                    ident_map(wide(n_t, target)),
                    TensorBody::Max(add(op(0), op(1)), konst(MASKED)),
                    sel,
                ));
                sel
            }
            (None, MoeScore::Sigmoid) => {
                let (stage, routed) = routed_scores(&mut f, p, n, target);
                chain.push(stage);
                routed
            }
            (None, MoeScore::Softmax) => p,
        };
        let kb = k.min(n);
        let at_pick = ElemAccess { base: 0, terms: Vec::new(), indirect: Some(IndirectTerm { operand: 1, stride: 1, modulus: n }) };
        let (mut picked, mut ids, mut scores, mut weights) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for r in 0..kb {
            let (pick, end) = moe_pick(&mut f, &mut chain, ("pick", "route"), r, (choosing, map2(0, 0, 1), n), &picked, r + 1 < kb, ix, target);
            picked.extend(end.map(|end| (pick, end)));
            let id = f.t.scratch_deep(&format!("expert{r}"), 1, 1, ix);
            chain.push(f.compute(&format!("expert{r}"), flat_space(1), vec![Slot { buf: pick, map: fixed(0) }], ident_map(one_w), *add(op(0), konst((first * tw) as f64)), id));
            let score = f.t.scratch_deep(&format!("score{r}"), 1, 1, acc);
            chain.push(f.compute(
                &format!("score{r}"),
                flat_space(1),
                vec![Slot { buf: choosing, map: at_pick.clone() }, Slot { buf: pick, map: fixed(0) }],
                ident_map(one_w),
                TensorBody::Operand(0),
                score,
            ));
            let weight = f.t.scratch_deep(&format!("weight{r}"), 1, 1, acc);
            chain.push(f.compute(
                &format!("weight{r}"),
                flat_space(1),
                vec![Slot { buf: p, map: at_pick.clone() }, Slot { buf: pick, map: fixed(0) }],
                ident_map(one_w),
                TensorBody::Operand(0),
                weight,
            ));
            ids.push(id);
            scores.push(score);
            weights.push(weight);
        }
        let ids_out = pack(&mut f, &mut chain, "ids", &ids, 0.0, ix);
        let scores_out = pack(&mut f, &mut chain, "scores", &scores, MASKED, acc);
        let weights_out = pack(&mut f, &mut chain, "weights", &weights, 0.0, acc);
        let mut stores = Vec::new();
        for (kind, buf, dt) in [("ids", ids_out, ix), ("scores", scores_out, acc), ("weights", weights_out, acc)] {
            let tensor = f.t.tensor(&cand(kind, b), TensorRole::Output, 1, tw, dt, TensorInit::Zero);
            stores.push((buf, f.t.store(&format!("store_{kind}"), tensor, Vec::new(), 1, buf)));
        }
        let claim = approx("mixture-of-experts routing block", acc);
        out.push(f.wire_and_finish_many(chain, &stores, 1, format!("moe_candidates{b}_{ne}x{k}"), claim, target)?);
    }
    // The candidates of every block in row order, `K` rounds over them.
    let nb = blocks.len();
    let mut f = FlatBuild::at(acc, target)?;
    let mut chain = Vec::new();
    let mut row = |f: &mut FlatBuild, kind: &str, dt: Precision| {
        let pages: Vec<BufferId> = (0..nb).map(|b| f.input_paged(&cand(kind, b), tw, &Fill::Unknown, dt, 1, target)).collect();
        let slots = pages.iter().enumerate().map(|(b, &buf)| Slot { buf, map: at(-((b * tw) as i64)) }).collect();
        let bands = (0..nb).map(|b| ((b + 1) * tw, TensorBody::Operand(b))).collect();
        let all = f.t.scratch_deep(&format!("{kind}_row"), 1, nb, dt);
        chain.push(f.compute(&format!("{kind}_row"), flat_space(nb * tw), slots, ident_map(wide(nb, target)), banded(bands), all));
        all
    };
    let (ids, scores, weights) = (row(&mut f, "ids", ix), row(&mut f, "scores", acc), row(&mut f, "weights", acc));
    let at_pick = ElemAccess { base: 0, terms: Vec::new(), indirect: Some(IndirectTerm { operand: 1, stride: 1, modulus: nb * tw }) };
    let (mut picked, mut picks, mut chosen) = (Vec::new(), Vec::new(), Vec::new());
    for r in 0..k {
        let (at, end) = moe_pick(&mut f, &mut chain, ("candidate", "choose"), r, (scores, map2(0, 0, 1), nb * tw), &picked, r + 1 < k, ix, target);
        picked.extend(end.map(|end| (at, end)));
        let pick = f.t.scratch_deep(&format!("pick{r}"), 1, 1, ix);
        chain.push(f.compute(&format!("pick{r}"), flat_space(1), vec![Slot { buf: ids, map: at_pick.clone() }, Slot { buf: at, map: fixed(0) }], ident_map(one_w), TensorBody::Operand(0), pick));
        let weight = f.t.scratch_deep(&format!("weight{r}"), 1, 1, acc);
        chain.push(f.compute(&format!("weight{r}"), flat_space(1), vec![Slot { buf: weights, map: at_pick.clone() }, Slot { buf: at, map: fixed(0) }], ident_map(one_w), round_weight(spec), weight));
        picks.push(pick);
        chosen.push(weight);
    }
    // A tile of the pick and weight tables a pick, as [`moe_routing`] writes them, packed and stored in parts of at most
    // an accumulator page: qwen4_exp's top 10 are more picks than a page holds.
    let blk = row_block_tiles(target);
    let mut stores = Vec::new();
    for (name, parts, dt, tensor) in [("picks", &picks, ix, &mn.picks), ("weights", &chosen, acc, &mn.weights)] {
        let t = f.t.tensor(tensor, TensorRole::Output, 1, k * tw, dt, TensorInit::Zero);
        for (c, first) in (0..k).step_by(blk).enumerate() {
            let part = &parts[first..(first + blk).min(k)];
            let slots = part.iter().map(|&buf| Slot { buf, map: fixed(0) }).collect();
            let bands = (0..part.len()).map(|r| ((r + 1) * tw, TensorBody::Operand(r))).collect();
            let buf = f.t.buffer_deep(&format!("{name}{c}"), 1, part.len(), dt);
            chain.push(f.compute(&format!("pack_{name}{c}"), flat_space(part.len() * tw), slots, ident_map(wide(part.len(), target)), banded(bands), buf));
            stores.push((buf, f.t.store_block_over(&format!("store_{name}{c}"), t, first, Vec::new(), 1, part.len(), k, (1, 1), buf)));
        }
    }
    let claim = approx("mixture-of-experts routing", acc);
    out.push(f.wire_and_finish_many(chain, &stores, k, format!("moe_routing_{ne}x{k}"), claim, target)?);
    Ok(out)
}

/// The plan of a section whose item walks `chunks` sub-blocks, one output tile per pass.
pub(super) fn chunked_plan(chunks: usize, items: usize) -> BlockPlan {
    BlockPlan {
        per_site_m: chunks,
        per_site_n: 1,
        grid: (items, 1),
        inner: InnerLoop {
            num_blocks: 1,
            block_k_tiles: 1,
        },
        block: BlockShape {
            osh: 1,
            osw: 1,
            ibw: 1,
        },
        acc: AccPolicy::F32,
        pack_accumulate: false,
        engines: EnginePacking::LoadsTogether,
        granularity: Granularity::PerTile,
    }
}

/// A partial GEMV of one expert's rows: 32 outputs of a tile row of the weight against one `chunk`-tile piece
/// of the input — `Σ_t W[o, t]·v[t]` over the chunk, the weight page read as `o·(chunk·32) + t`.
pub(super) fn row_dot(chunk_elems: usize) -> (Vec<IterSpec>, Vec<ElemAccess>) {
    (
        vec![
            IterSpec {
                var: LV_GEN_ROW,
                extent: chunk_elems,
                kind: IterKind::Reduce {
                    pool: ReducePool::Sum,
                },
            },
            IterSpec {
                var: LV_GEN_COL,
                extent: 32,
                kind: IterKind::Parallel,
            },
        ],
        vec![
            ElemAccess {
                base: 0,
                terms: vec![(LV_GEN_COL, chunk_elems as i64), (LV_GEN_ROW, 1)],
                indirect: None,
            },
            ElemAccess {
                base: 0,
                terms: vec![(LV_GEN_ROW, 1)],
                indirect: None,
            },
            const_map(),
        ],
    )
}

/// A load of `chunk` tiles of a chosen expert's weight: tile row `ITEM_X` of the expert's `rows_t × cols_t` slab,
/// chunk `LV_SUB_H` of the row, the slab picked by the index in `pick`.
#[allow(clippy::too_many_arguments)]
pub(super) fn expert_chunk(
    t: &mut Template,
    name: &str,
    weight: TensorId,
    pick: BufferId,
    experts: usize,
    rows_t: usize,
    cols_t: usize,
    chunk: usize,
    into: BufferId,
) -> StageId {
    let origin = TileAddr {
        base: 0,
        steps: vec![(LV_ITEM_X, cols_t), (crate::spatial::passes::LV_SUB_H, chunk)],
        indirect: Some(IndirectTile {
            index: pick,
            tile_stride: rows_t * cols_t,
            elems_per_tile: 1,
            bound: (0, experts - 1),
        }),
    };
    t.stage(
        name,
        StageKind::Load {
            tensor: weight,
            pattern: AccessPattern {
                origin,
                block_h: 1,
                block_w: chunk,
                stride_w: 1,
                stride_h: cols_t,
            },
            num_blocks: 1,
            multicast: None,
        },
        vec![pick],
        vec![into],
    )
}

/// Gate and up partial sums: item `(r, ot)` is round `r`'s tile row `ot` of outputs; its sub-blocks walk the `d`
/// input in chunks of `cd` tiles.
pub(super) fn moe_up_partials(
    spec: &MoeSpec,
    names: &StageNames,
    mn: &MoeNames,
    cd: usize,
    x_dtype: Precision,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (d, dff, ne, k) = (spec.d, spec.dff, spec.ne, spec.k);
    let tw = target.tile.cols;
    let (d_t, f_t) = (d / tw, dff / tw);
    let nc = d_t / cd;
    let precision = elem_precision(target)?;
    // The experts' gate and up weights are the products' operands, held at the products' float.
    let operand = products_float(target)?;
    let acc = AccPolicy::F32.interm_precision();
    let ix = index_type(target, ne as u64 + 1)?;
    let (x_name, x_fill) = names.operand(0);
    let (wg_name, wg_fill) = names.operand(2);
    let (wu_name, wu_fill) = names.operand(3);
    let mut t = Template::new(target)?;
    let scaler = t.tensor(
        &mn.scaler,
        TensorRole::Param,
        target.tile.rows,
        tw,
        precision,
        TensorInit::ConstF(1.0f64.to_bits()),
    );
    let picks = t.tensor(
        &mn.picks,
        TensorRole::Input,
        1,
        k * tw,
        ix,
        TensorInit::External,
    );
    let x = t.tensor(
        x_name,
        TensorRole::Input,
        1,
        d,
        x_dtype,
        crate::spatial::passes::from_tensor_op::init_of(x_fill, d),
    );
    let wg = t.tensor(
        wg_name,
        TensorRole::Param,
        ne * dff,
        d,
        operand,
        crate::spatial::passes::from_tensor_op::init_of(wg_fill, ne * dff * d),
    );
    let wu = t.tensor(
        wu_name,
        TensorRole::Param,
        ne * dff,
        d,
        operand,
        crate::spatial::passes::from_tensor_op::init_of(wu_fill, ne * dff * d),
    );
    let gp = t.tensor(
        &mn.gate_part,
        TensorRole::Output,
        k * f_t * nc * target.tile.rows,
        tw,
        acc,
        TensorInit::Zero,
    );
    let up = t.tensor(
        &mn.up_part,
        TensorRole::Output,
        k * f_t * nc * target.tile.rows,
        tw,
        acc,
        TensorInit::Zero,
    );
    let sc_in = t.buffer_deep("in_scaler", 1, 1, precision);
    let pg_in = t.buffer_deep("in_pick_gate", 1, 1, ix);
    let pu_in = t.buffer_deep("in_pick_up", 1, 1, ix);
    let x_in = t.buffer("in_x", cd, x_dtype);
    let wg_in = t.buffer("in_gate", cd, operand);
    let wu_in = t.buffer("in_up", cd, operand);
    let g_out = t.buffer("gate_out", 1, acc);
    let u_out = t.buffer("up_out", 1, acc);
    let hold = |mut v: Vec<(LoopVar, usize)>| {
        v.push((crate::spatial::passes::LV_SUB_H, 0));
        v
    };
    let l_sc = t.load_block_at("load_scaler", scaler, 0, hold(Vec::new()), 1, 1, 1, sc_in);
    // Every gather retires its index page, so each expert load waits on a pick of its own.
    let l_pg = t.load_block_at(
        "load_pick_gate",
        picks,
        0,
        hold(vec![(LV_ITEM_Y, 1)]),
        1,
        1,
        1,
        pg_in,
    );
    let l_pu = t.load_block_at(
        "load_pick_up",
        picks,
        0,
        hold(vec![(LV_ITEM_Y, 1)]),
        1,
        1,
        1,
        pu_in,
    );
    let l_x = t.load_block_at("load_x", x, 0, vec![(crate::spatial::passes::LV_SUB_H, cd)], 1, cd, cd, x_in);
    let l_wg = expert_chunk(&mut t, "load_gate", wg, pg_in, ne, f_t, d_t, cd, wg_in);
    let l_wu = expert_chunk(&mut t, "load_up", wu, pu_in, ne, f_t, d_t, cd, wu_in);
    let (iters, maps) = row_dot(cd * tw);
    let s_g = t.stage(
        "gate_dot",
        StageKind::Compute(ComputeOp::Generic {
            iters: iters.clone(),
            ins: maps.clone(),
            out: per_col_map(),
            body: *mul(op(0), op(1)),
        }),
        vec![wg_in, x_in, sc_in],
        vec![g_out],
    );
    let s_u = t.stage(
        "up_dot",
        StageKind::Compute(ComputeOp::Generic {
            iters,
            ins: maps,
            out: per_col_map(),
            body: *mul(op(0), op(1)),
        }),
        vec![wu_in, x_in, sc_in],
        vec![u_out],
    );
    let steps = vec![(LV_ITEM_Y, f_t * nc), (LV_ITEM_X, nc), (crate::spatial::passes::LV_SUB_H, 1)];
    let st_g = t.store_block_over("store_gate", gp, 0, steps.clone(), 1, 1, 1, (nc, 1), g_out);
    let st_u = t.store_block_over("store_up", up, 0, steps, 1, 1, 1, (nc, 1), u_out);
    for (from, buf, to) in [
        (l_pg, pg_in, l_wg),
        (l_pu, pu_in, l_wu),
        (l_wg, wg_in, s_g),
        (l_x, x_in, s_g),
        (l_sc, sc_in, s_g),
        (l_wu, wu_in, s_u),
        (s_g, g_out, st_g),
        (s_u, u_out, st_u),
    ] {
        t.edge(from, buf, to);
    }
    let items: Vec<WorkItem> = (0..k)
        .flat_map(|r| (0..f_t).map(move |ot| (r, ot)))
        .enumerate()
        .map(|(id, (r, ot))| WorkItem {
            id: WorkItemId::new(id),
            coord: (r, ot),
            out: TileRect {
                row0_t: (r * f_t + ot) * nc,
                col0_t: 0,
                h_t: nc,
                w_t: 1,
            },
        })
        .collect();
    let mut prog = t.finish(
        format!("moe_up_{ne}x{k}_{d}x{dff}_c{cd}"),
        items,
        AccPolicy::F32,
        approx("mixture-of-experts gate/up partials", operand),
        target,
    )?;
    prog.work.plan = Some(chunked_plan(nc, k * f_t));
    prog.work.k_tiles = 1;
    Ok(prog)
}

/// The tile `LV_ITEM_X` of a chosen expert's row of `table` (`ne` rows of `row_t` tiles): the round's pick loaded
/// from the pick table — tile `round.1`, plus `LV_ITEM_Y` when `round.0` is it — then the bias tile gathered by it.
/// Each gather retires its index page, so every bias load waits on a pick page of its own.
#[allow(clippy::too_many_arguments)]
pub(super) fn expert_bias_tile(
    t: &mut Template,
    part: &str,
    mn: &MoeNames,
    table: TensorId,
    round: (LoopVar, usize),
    row_t: usize,
    spec: &MoeSpec,
    target: &SpatialTarget,
) -> Result<(StageId, BufferId), SpatialReject> {
    let ix = index_type(target, spec.ne as u64 + 1)?;
    let acc = AccPolicy::F32.interm_precision();
    let picks = t.tensor(&mn.picks, TensorRole::Input, 1, spec.k * target.tile.cols, ix, TensorInit::External);
    let pick_in = t.buffer_deep(&format!("in_pick_{part}_bias"), 1, 1, ix);
    let bias_in = t.buffer_deep(&format!("in_{part}_bias"), 1, 1, acc);
    let steps = if round.0 == LV_ITEM_Y { vec![(LV_ITEM_Y, 1)] } else { Vec::new() };
    let l_p = t.load_block_at(&format!("load_pick_{part}_bias"), picks, round.1, steps, 1, 1, 1, pick_in);
    let l_b = t.stage(
        &format!("load_{part}_bias"),
        StageKind::Load {
            tensor: table,
            pattern: AccessPattern {
                origin: TileAddr {
                    base: 0,
                    steps: vec![(LV_ITEM_X, 1)],
                    indirect: Some(IndirectTile { index: pick_in, tile_stride: row_t, elems_per_tile: 1, bound: (0, spec.ne - 1) }),
                },
                block_h: 1,
                block_w: 1,
                stride_w: 1,
                stride_h: 1,
            },
            num_blocks: 1,
            multicast: None,
        },
        vec![pick_in],
        vec![bias_in],
    );
    t.edge(l_p, pick_in, l_b);
    Ok((l_b, bias_in))
}

/// `hidden = act(Σ gate + b_gate, Σ up + b_up)` for each round's tile of outputs — the partials added, the chosen
/// expert's biases when it has them, then the mixture's gated activation.
pub(super) fn moe_hidden(
    spec: &MoeSpec,
    names: &StageNames,
    operands: &MoeOperands,
    mn: &MoeNames,
    nc: usize,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (dff, k) = (spec.dff, spec.k);
    let tw = target.tile.cols;
    let f_t = dff / tw;
    let precision = elem_precision(target)?;
    let acc = AccPolicy::F32.interm_precision();
    let mut t = Template::new(target)?;
    let scaler = t.tensor(
        &mn.scaler,
        TensorRole::Param,
        target.tile.rows,
        tw,
        precision,
        TensorInit::ConstF(1.0f64.to_bits()),
    );
    let gp = t.tensor(
        &mn.gate_part,
        TensorRole::Input,
        k * f_t * nc * target.tile.rows,
        tw,
        acc,
        TensorInit::External,
    );
    let up = t.tensor(
        &mn.up_part,
        TensorRole::Input,
        k * f_t * nc * target.tile.rows,
        tw,
        acc,
        TensorInit::External,
    );
    let hidden = t.tensor(
        &mn.hidden,
        TensorRole::Output,
        1,
        k * dff,
        acc,
        TensorInit::Zero,
    );
    let sc_in = t.buffer_deep("in_scaler", 1, 1, precision);
    let g_in = t.buffer("in_gate", nc, acc);
    let u_in = t.buffer("in_up", nc, acc);
    let gs = t.scratch_deep("gate_sum", 1, 1, acc);
    let us = t.scratch_deep("up_sum", 1, 1, acc);
    let out = t.buffer("out", 1, acc);
    let l_sc = t.load_block_at("load_scaler", scaler, 0, Vec::new(), 1, 1, 1, sc_in);
    let parts = vec![(LV_ITEM_Y, f_t * nc), (LV_ITEM_X, nc)];
    let l_g = t.load_block_at("load_gate", gp, 0, parts.clone(), nc, 1, 1, g_in);
    let l_u = t.load_block_at("load_up", up, 0, parts, nc, 1, 1, u_in);
    // Partial `c`'s output `o` sits at `c·32 + o` of the page.
    let fold = |t: &mut Template, name: &str, from: BufferId, into: BufferId| {
        let iters = vec![
            IterSpec {
                var: LV_GEN_ROW,
                extent: nc,
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
        let maps = vec![
            ElemAccess {
                base: 0,
                terms: vec![(LV_GEN_ROW, tw as i64), (LV_GEN_COL, 1)],
                indirect: None,
            },
            const_map(),
        ];
        t.stage(
            name,
            StageKind::Compute(ComputeOp::Generic {
                iters,
                ins: maps,
                out: per_col_map(),
                body: TensorBody::Operand(0),
            }),
            vec![from, sc_in],
            vec![into],
        )
    };
    let s_g = fold(&mut t, "gate_sum", g_in, gs);
    let s_u = fold(&mut t, "up_sum", u_in, us);
    let w1 = wide(1, target);
    let mut ins = vec![gs, us];
    let mut terms: Vec<Box<TensorBody>> = vec![op(0), op(1)];
    let mut bias_loads = Vec::new();
    // The round's weight, when it scales the input: `w·(x·W)` is `(w·x)·W`, so it scales the summed products.
    if spec.weigh_inputs {
        let weights = t.tensor(&mn.weights, TensorRole::Input, 1, k * tw, acc, TensorInit::External);
        let w_in = t.buffer_deep("in_weight", 1, 1, acc);
        let l_w = t.load_block_at("load_weight", weights, 0, vec![(LV_ITEM_Y, 1)], 1, 1, 1, w_in);
        terms = vec![mul(op(2), op(0)), mul(op(2), op(1))];
        ins.push(w_in);
        bias_loads.push((l_w, w_in));
    }
    // Each bias the chosen expert has: tile `ot` of its row, gathered by the round's pick.
    for (i, part) in ["gate", "up"].into_iter().enumerate() {
        let Some(bi) = operands.biases[i] else { continue };
        let (b_name, b_fill) = names.operand(bi);
        let table = t.tensor(b_name, TensorRole::Param, 1, spec.ne * dff, acc, crate::spatial::passes::from_tensor_op::init_of(b_fill, spec.ne * dff));
        let (l_b, bias_in) = expert_bias_tile(&mut t, part, mn, table, (LV_ITEM_Y, 0), f_t, spec, target)?;
        terms[i] = add(terms[i].clone(), op(ins.len()));
        ins.push(bias_in);
        bias_loads.push((l_b, bias_in));
    }
    let [gate, up]: [Box<TensorBody>; 2] = terms.try_into().expect("a gate and an up");
    let s_h = t.stage(
        "swiglu",
        StageKind::Compute(ComputeOp::Generic {
            iters: flat_space(tw),
            ins: vec![at(0); ins.len()],
            out: ident_map(w1),
            body: *moe_gated(spec.act, gate, up),
        }),
        ins,
        vec![out],
    );
    for (from, buf) in bias_loads {
        t.edge(from, buf, s_h);
    }
    let st = t.store_block(
        "store_hidden",
        hidden,
        0,
        vec![(LV_ITEM_Y, f_t), (LV_ITEM_X, 1)],
        1,
        1,
        1,
        out,
    );
    for (from, buf, to) in [
        (l_sc, sc_in, s_g),
        (l_g, g_in, s_g),
        (l_u, u_in, s_u),
        (s_g, gs, s_h),
        (s_u, us, s_h),
        (s_h, out, st),
    ] {
        t.edge(from, buf, to);
    }
    let items: Vec<WorkItem> = (0..k)
        .flat_map(|r| (0..f_t).map(move |ot| (r, ot)))
        .enumerate()
        .map(|(id, (r, ot))| WorkItem {
            id: WorkItemId::new(id),
            coord: (r, ot),
            out: TileRect {
                row0_t: 0,
                col0_t: r * f_t + ot,
                h_t: 1,
                w_t: 1,
            },
        })
        .collect();
    t.finish(
        format!("moe_hidden_{k}x{dff}"),
        items,
        AccPolicy::F32,
        approx("mixture-of-experts SwiGLU", precision),
        target,
    )
}

/// Down partial sums: item `(r, ot)` is round `r`'s tile row `ot` of the block's `d` outputs; its sub-blocks walk
/// the hidden activation in chunks of `cd` tiles.
pub(super) fn moe_down_partials(
    spec: &MoeSpec,
    names: &StageNames,
    mn: &MoeNames,
    cd: usize,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (d, dff, ne, k) = (spec.d, spec.dff, spec.ne, spec.k);
    let tw = target.tile.cols;
    let (d_t, f_t) = (d / tw, dff / tw);
    let nc = f_t / cd;
    let precision = elem_precision(target)?;
    // The experts' down weights are the products' operands, held at the products' float.
    let operand = products_float(target)?;
    let acc = AccPolicy::F32.interm_precision();
    let ix = index_type(target, ne as u64 + 1)?;
    let (wd_name, wd_fill) = names.operand(4);
    let mut t = Template::new(target)?;
    let scaler = t.tensor(
        &mn.scaler,
        TensorRole::Param,
        target.tile.rows,
        tw,
        precision,
        TensorInit::ConstF(1.0f64.to_bits()),
    );
    let picks = t.tensor(
        &mn.picks,
        TensorRole::Input,
        1,
        k * tw,
        ix,
        TensorInit::External,
    );
    let hidden = t.tensor(
        &mn.hidden,
        TensorRole::Input,
        1,
        k * dff,
        acc,
        TensorInit::External,
    );
    let wd = t.tensor(
        wd_name,
        TensorRole::Param,
        ne * d,
        dff,
        operand,
        crate::spatial::passes::from_tensor_op::init_of(wd_fill, ne * d * dff),
    );
    let dp = t.tensor(
        &mn.down_part,
        TensorRole::Output,
        k * d_t * nc * target.tile.rows,
        tw,
        acc,
        TensorInit::Zero,
    );
    let sc_in = t.buffer_deep("in_scaler", 1, 1, precision);
    let pk_in = t.buffer_deep("in_pick", 1, 1, ix);
    let h_in = t.buffer("in_hidden", cd, acc);
    let w_in = t.buffer("in_down", cd, operand);
    let d_out = t.buffer("down_out", 1, acc);
    let hold = |mut v: Vec<(LoopVar, usize)>| {
        v.push((crate::spatial::passes::LV_SUB_H, 0));
        v
    };
    let l_sc = t.load_block_at("load_scaler", scaler, 0, hold(Vec::new()), 1, 1, 1, sc_in);
    let l_pk = t.load_block_at(
        "load_pick",
        picks,
        0,
        hold(vec![(LV_ITEM_Y, 1)]),
        1,
        1,
        1,
        pk_in,
    );
    let l_h = t.load_block_at(
        "load_hidden",
        hidden,
        0,
        vec![(LV_ITEM_Y, f_t), (crate::spatial::passes::LV_SUB_H, cd)],
        1,
        cd,
        cd,
        h_in,
    );
    let l_w = expert_chunk(&mut t, "load_down", wd, pk_in, ne, d_t, f_t, cd, w_in);
    let (iters, maps) = row_dot(cd * tw);
    let s_d = t.stage(
        "down_dot",
        StageKind::Compute(ComputeOp::Generic {
            iters,
            ins: maps,
            out: per_col_map(),
            body: *mul(op(0), op(1)),
        }),
        vec![w_in, h_in, sc_in],
        vec![d_out],
    );
    let st = t.store_block_over(
        "store_down",
        dp,
        0,
        vec![(LV_ITEM_Y, d_t * nc), (LV_ITEM_X, nc), (crate::spatial::passes::LV_SUB_H, 1)],
        1,
        1,
        1,
        (nc, 1),
        d_out,
    );
    for (from, buf, to) in [
        (l_pk, pk_in, l_w),
        (l_w, w_in, s_d),
        (l_h, h_in, s_d),
        (l_sc, sc_in, s_d),
        (s_d, d_out, st),
    ] {
        t.edge(from, buf, to);
    }
    let items: Vec<WorkItem> = (0..k)
        .flat_map(|r| (0..d_t).map(move |ot| (r, ot)))
        .enumerate()
        .map(|(id, (r, ot))| WorkItem {
            id: WorkItemId::new(id),
            coord: (r, ot),
            out: TileRect {
                row0_t: (r * d_t + ot) * nc,
                col0_t: 0,
                h_t: nc,
                w_t: 1,
            },
        })
        .collect();
    let mut prog = t.finish(
        format!("moe_down_{ne}x{k}_{d}x{dff}_c{cd}"),
        items,
        AccPolicy::F32,
        approx("mixture-of-experts down partials", operand),
        target,
    )?;
    prog.work.plan = Some(chunked_plan(nc, k * d_t));
    prog.work.k_tiles = 1;
    Ok(prog)
}

/// The combine: per tile of the `d` outputs, each round's down partials added, plus its expert's down bias when the
/// mixture has one, weighted by its router probability, and divided by the kept probabilities' sum when the router
/// renormalizes — the desugar's `Σ (P[b]/Σ P)·y_b`, or `Σ P[b]·y_b`.
pub(super) fn moe_combine(
    spec: &MoeSpec,
    names: &StageNames,
    operands: &MoeOperands,
    mn: &MoeNames,
    nc: usize,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (d, k) = (spec.d, spec.k);
    let tw = target.tile.cols;
    let d_t = d / tw;
    let precision = elem_precision(target)?;
    let acc = AccPolicy::F32.interm_precision();
    let mut t = Template::new(target)?;
    let scaler = t.tensor(
        &mn.scaler,
        TensorRole::Param,
        target.tile.rows,
        tw,
        precision,
        TensorInit::ConstF(1.0f64.to_bits()),
    );
    // The kept experts' weights, unless each already scaled its expert's input.
    let weights = (!spec.weigh_inputs).then(|| t.tensor(&mn.weights, TensorRole::Input, 1, k * tw, acc, TensorInit::External));
    let dp = t.tensor(
        &mn.down_part,
        TensorRole::Input,
        k * d_t * nc * target.tile.rows,
        tw,
        acc,
        TensorInit::External,
    );
    let y = t.tensor(
        &names.out,
        TensorRole::Output,
        1,
        d,
        precision,
        TensorInit::Zero,
    );
    let sc_in = t.buffer_deep("in_scaler", 1, 1, precision);
    let w_in = weights.map(|_| t.buffer("in_weights", k, acc));
    let p_in = t.buffer("in_down", k * nc, acc);
    // ★ A ROUND'S SUM IS A TILE OF THE ACCUMULATOR: more rounds than a page holds (qwen4_exp's top 10) are summed and
    // weighed in groups of at most a page of rounds, and the groups' weighted sums added.
    let blk = row_block_tiles(target);
    let groups: Vec<(usize, usize)> = (0..k).step_by(blk).map(|g0| (g0, (g0 + blk).min(k))).collect();
    let named = |base: &str, g: usize| if groups.len() == 1 { base.to_string() } else { format!("{base}{g}") };
    let pages: Vec<(BufferId, BufferId)> = groups
        .iter()
        .enumerate()
        .map(|(g, &(g0, g1))| (t.scratch_deep(&named("down_rounds", g), 1, g1 - g0, acc), t.scratch_deep(&named("weighted", g), 1, 1, acc)))
        .collect();
    let ws = t.scratch_deep("weight_sum", 1, 1, acc);
    let out = t.buffer("out", 1, precision);
    let l_sc = t.load_block_at("load_scaler", scaler, 0, Vec::new(), 1, 1, 1, sc_in);
    let l_w = weights.zip(w_in).map(|(weights, w_in)| t.load_block_at("load_weights", weights, 0, Vec::new(), 1, k, k, w_in));
    // Round `r`'s partials for this output tile: `nc` tiles at `(r·d_t + ot)·nc` — `k` rows `d_t·nc` apart.
    let l_p = t.load_block_at(
        "load_down",
        dp,
        0,
        vec![(LV_ITEM_X, nc)],
        k,
        nc,
        d_t * nc,
        p_in,
    );
    let mut edges = Vec::new();
    let mut weighed = Vec::new();
    for (g, &(g0, g1)) in groups.iter().enumerate() {
        let len = g1 - g0;
        let (dr, ys) = pages[g];
        // Page tile `r·nc + c` holds round `r`'s partial `c`; output column `q = (r − g0)·32 + o` of the group's round
        // sums reads it at `q + (r·(nc − 1) + g0)·32 + c·32`, one band per round.
        let mut slots = Vec::new();
        let mut bands = Vec::new();
        for (j, r) in (g0..g1).enumerate() {
            slots.push(ElemAccess {
                base: ((r * (nc - 1) + g0) * tw) as i64,
                terms: vec![(LV_GEN_COL, 1), (LV_GEN_ROW, tw as i64)],
                indirect: None,
            });
            bands.push(((j + 1) * tw, TensorBody::Operand(j)));
        }
        slots.push(const_map());
        let mut ins = vec![p_in; len];
        ins.push(sc_in);
        let s_r = t.stage(
            &named("round_sums", g),
            StageKind::Compute(ComputeOp::Generic {
                iters: vec![
                    IterSpec {
                        var: LV_GEN_ROW,
                        extent: nc,
                        kind: IterKind::Reduce {
                            pool: ReducePool::Sum,
                        },
                    },
                    IterSpec {
                        var: LV_GEN_COL,
                        extent: len * tw,
                        kind: IterKind::Parallel,
                    },
                ],
                ins: slots,
                out: per_col_map(),
                body: banded(bands),
            }),
            ins,
            vec![dr],
        );
        // `Σ_r w_r·down_r`, or `Σ_r down_r` when each round's weight scaled its expert's input.
        let round_sum = ElemAccess {
            base: 0,
            terms: vec![(LV_GEN_ROW, tw as i64), (LV_GEN_COL, 1)],
            indirect: None,
        };
        let (sum_ins, sum_maps, sum_body) = match w_in {
            None => (vec![dr, sc_in], vec![round_sum, const_map()], TensorBody::Operand(0)),
            Some(w_in) => (
                vec![w_in, dr, sc_in],
                vec![
                    ElemAccess {
                        base: (g0 * tw) as i64,
                        terms: vec![(LV_GEN_ROW, tw as i64)],
                        indirect: None,
                    },
                    round_sum,
                    const_map(),
                ],
                *mul(op(0), op(1)),
            ),
        };
        let s_y = t.stage(
            &named("weighted", g),
            StageKind::Compute(ComputeOp::Generic {
                iters: vec![
                    IterSpec {
                        var: LV_GEN_ROW,
                        extent: len,
                        kind: IterKind::Reduce {
                            pool: ReducePool::Sum,
                        },
                    },
                    IterSpec {
                        var: LV_GEN_COL,
                        extent: tw,
                        kind: IterKind::Parallel,
                    },
                ],
                ins: sum_maps,
                out: per_col_map(),
                body: sum_body,
            }),
            sum_ins,
            vec![ys],
        );
        if g == 0 {
            edges.extend([(l_sc, sc_in, s_r), (l_p, p_in, s_r)]);
            if let (Some(l_w), Some(w_in)) = (l_w, w_in) {
                edges.push((l_w, w_in, s_y));
            }
        }
        edges.push((s_r, dr, s_y));
        weighed.push((s_y, ys));
    }
    let one_w = wide(1, target);
    // The groups' weighted sums added, when there is more than one.
    let (s_y, ys) = match weighed.as_slice() {
        [one] => *one,
        many => {
            let ys = t.scratch_deep("weighted", 1, 1, acc);
            let body = (1..many.len()).fold(TensorBody::Operand(0), |b, g| *add(Box::new(b), op(g)));
            let s = t.stage(
                "weighted",
                StageKind::Compute(ComputeOp::Generic {
                    iters: flat_space(tw),
                    ins: vec![at(0); many.len()],
                    out: ident_map(one_w),
                    body,
                }),
                many.iter().map(|&(_, b)| b).collect(),
                vec![ys],
            );
            edges.extend(many.iter().map(|&(from, b)| (from, b, s)));
            (s, ys)
        }
    };
    // The final stage's operands: the weighted sum, then the bias mix and the weights' sum when each is there.
    let mut ins = vec![ys];
    let mut maps = vec![at(0)];
    let mut total = op(0);
    let mut into_out = vec![(s_y, ys)];
    // `Σ_r P[b_r]·B[b_r]`: each round's chosen down-bias tile, weighted by its probability.
    if let Some(bi) = operands.biases[2] {
        let (b_name, b_fill) = names.operand(bi);
        let table = t.tensor(b_name, TensorRole::Param, 1, spec.ne * d, acc, crate::spatial::passes::from_tensor_op::init_of(b_fill, spec.ne * d));
        let mut mix_ins = Vec::new();
        let mut mix_maps = Vec::new();
        let mut mix: Option<Box<TensorBody>> = None;
        let mut loads = Vec::new();
        for r in 0..k {
            let (l_b, bias_in) = expert_bias_tile(&mut t, &format!("down{r}"), mn, table, (LV_ITEM_X, r), d_t, spec, target)?;
            let term = match w_in {
                Some(w_in) => {
                    let term = mul(op(mix_ins.len()), op(mix_ins.len() + 1));
                    mix_ins.extend([w_in, bias_in]);
                    mix_maps.extend([fixed((r * tw) as i64), at(0)]);
                    term
                }
                None => {
                    mix_ins.push(bias_in);
                    mix_maps.push(at(0));
                    op(mix_ins.len() - 1)
                }
            };
            mix = Some(match mix {
                None => term,
                Some(m) => add(m, term),
            });
            loads.push((l_b, bias_in));
        }
        let bm = t.scratch_deep("bias_mix", 1, 1, acc);
        let s_b = t.stage(
            "bias_mix",
            StageKind::Compute(ComputeOp::Generic {
                iters: flat_space(tw),
                ins: mix_maps,
                out: ident_map(one_w),
                body: *mix.expect("k ≥ 1 rounds"),
            }),
            mix_ins,
            vec![bm],
        );
        edges.extend(loads.into_iter().map(|(l, b)| (l, b, s_b)));
        total = add(total, op(ins.len()));
        ins.push(bm);
        maps.push(at(0));
        into_out.push((s_b, bm));
    }
    if spec.renormalize {
        let s_w = t.stage(
            "weight_sum",
            StageKind::Compute(ComputeOp::Generic {
                iters: head_space(1, k, Some(ReducePool::Sum)),
                ins: vec![
                    ElemAccess {
                        base: 0,
                        terms: vec![(LV_GEN_COL, tw as i64)],
                        indirect: None,
                    },
                    const_map(),
                ],
                out: per_row_map(one_w),
                body: TensorBody::Operand(0),
            }),
            vec![w_in.expect("a renormalized mixture weighs its outputs"), sc_in],
            vec![ws],
        );
        total = mul(total, unary(SfpuKind::Recip, renormalizing_sum(spec, op(ins.len()))));
        if spec.scale.is_some() {
            total = mul(total, konst(spec.scale()));
        }
        ins.push(ws);
        maps.push(fixed(0));
        into_out.push((s_w, ws));
    }
    let s_o = t.stage(
        if spec.renormalize { "renormalize" } else { "combine" },
        StageKind::Compute(ComputeOp::Generic {
            iters: flat_space(tw),
            ins: maps,
            out: ident_map(one_w),
            body: *total,
        }),
        ins,
        vec![out],
    );
    let st = t.store_block("store_out", y, 0, vec![(LV_ITEM_X, 1)], 1, 1, 1, out);
    edges.extend(into_out.into_iter().map(|(from, buf)| (from, buf, s_o)));
    edges.push((s_o, out, st));
    for (from, buf, to) in edges {
        t.edge(from, buf, to);
    }
    let items: Vec<WorkItem> = (0..d_t)
        .map(|ot| WorkItem {
            id: WorkItemId::new(ot),
            coord: (0, ot),
            out: TileRect {
                row0_t: 0,
                col0_t: ot,
                h_t: 1,
                w_t: 1,
            },
        })
        .collect();
    t.finish(
        format!("moe_combine_{k}x{d}"),
        items,
        AccPolicy::F32,
        approx("mixture-of-experts combine", precision),
        target,
    )
}
