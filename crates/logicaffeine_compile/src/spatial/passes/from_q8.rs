//! Q8_0 weights on the chip — a product `y = x · W` of up to a tile's rows over a weight the deployment stores Q8_0.
//!
//! ★ A DECODE STEP IS WEIGHT-BOUND, SO A WEIGHT MOVES AS IT IS STORED. Q8_0 keeps each column's rows in blocks of 32:
//! one scale `d` and 32 int8 values `q`, `w = q·d`. The weight is two tensors on the chip — `W__q8`, the int8 values
//! (`K × N`), and `W__q8_scale`, one fp32 scale per block (`K/32 × N`) — about 1.13 bytes a weight where a bf16 copy
//! moves 2. The product FOLDS its K blocks of 1,024 rows — 32 Q8_0 blocks, one tile row of scales — into an fp32
//! carry: per block, ONE stage a row, `y[n] += Σ_b s[b, n]·Σ_i x[32b + i]·q[32b + i, n]` — the lanes contracted over
//! the int8 tile rows, the block's 32 Q8_0 blocks folded down each column with their scales. An int8 times a bf16
//! activation is exact in fp32, and the scale is fp32.
//!
//! `x` is read as the `K/32 × 32` matrix of its 32-element blocks — one tile per K block — a view named for the value
//! it views ([`q8_rows_name`]), made once and read by every product of that value. A BATCH of `m` rows (up to a tile's)
//! is `m` such views interleaved by K block, and the fold runs each row's block dots against the SAME int8 and scale
//! pages: the weight moves once for all the rows.

use super::from_qwen_ops::{elem_precision, index_type, relayout_rows_to_dense, row_block_tiles};
use super::from_tensor_op::{ident_map, init_of, page_width, per_col_map, MatmulNames, Template};
use super::{SpatialReject, LV_GEN_COL, LV_GEN_K, LV_GEN_ROW, LV_ITEM_X, LV_ITEM_Y, LV_KBLOCK};
use crate::codegen_sva::fabric_target::Precision;
use crate::spatial::ir::*;
use crate::spatial::target::SpatialTarget;
use crate::tensor_recognize::{Fill, MatmulSpec};

/// Rows of a Q8_0 block.
pub const Q8_BLOCK: usize = 32;

/// Which part of a Q8_0 weight a stored tensor holds — what the host repacks from the checkpoint's blocks into it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Q8Part {
    /// The int8 values, `K × N`.
    Values,
    /// One fp32 scale per 32 rows of each column, `K/32 × N`.
    Scales,
}

impl Q8Part {
    /// The manifest's name for the repack.
    pub fn repack(self) -> &'static str {
        match self {
            Q8Part::Values => "q8_0.values",
            Q8Part::Scales => "q8_0.scales",
        }
    }
}

/// The tensor holding `part` of the Q8_0 weight `weight`.
pub fn stored_name(weight: &str, part: Q8Part) -> String {
    match part {
        Q8Part::Values => format!("{weight}__q8"),
        Q8Part::Scales => format!("{weight}__q8_scale"),
    }
}

/// The `K/32 × 32` view of the one-row activation `x` a Q8_0 product reads — `x`'s 32-element blocks as the rows of a
/// matrix, one tile per K block. It is named for the VALUE it views: `x` after the program has written it `writes`
/// times. Every product of one value reads one view, and a value written again is viewed again under a new name (an
/// activation has one writer).
pub fn q8_rows_name(x: &str, writes: usize) -> String {
    format!("{x}__q8_rows{writes}")
}

/// [`q8_rows_name`] for a product that runs on `chip` of a tensor-parallel deployment: a chip reads only what lives on
/// it, so each chip makes its own view of a replicated value, shared by that chip's products.
pub fn q8_rows_name_on(x: &str, writes: usize, chip: Option<u16>) -> String {
    match chip {
        Some(c) => format!("{}_c{c}", q8_rows_name(x, writes)),
        None => q8_rows_name(x, writes),
    }
}

/// The Q8_0 weight a tensor named by [`stored_name`] is part of, and the part.
pub fn stored_part(name: &str) -> Option<(&str, Q8Part)> {
    match (name.strip_suffix("__q8_scale"), name.strip_suffix("__q8")) {
        (Some(w), _) => Some((w, Q8Part::Scales)),
        (None, Some(w)) => Some((w, Q8Part::Values)),
        (None, None) => None,
    }
}

/// Whether a product over a Q8_0 weight folds on `target`: up to a tile's rows, whole Q8_0 blocks of K, on a square tile
/// as tall as a Q8_0 block. Its columns are the weight's column tiles, the last padded with zero columns as every tensor's
/// last tile is — a tensor-parallel chip's 24 of a gate's 48 heads, a width of 40 — so the pad's values and scales are
/// zero and its sums land only in the result's own pad.
pub fn q8_gemv_applies(spec: &MatmulSpec, target: &SpatialTarget) -> bool {
    let (tr, tw) = (target.tile.rows, target.tile.cols);
    (1..=tr).contains(&spec.m) && tr == Q8_BLOCK && tw == tr && spec.k > 0 && spec.k % Q8_BLOCK == 0 && spec.n > 0
}

/// The int8 values and fp32 scales of a Q8_0 weight from its declared `k × n` values — `None` when a block's values
/// are not an int8 multiple of its own scale `max|w| / 127`.
fn quantize(w: &[f64], k: usize, n: usize) -> Option<(Vec<i64>, Vec<f64>)> {
    let (mut q, mut s) = (vec![0i64; k * n], vec![0.0; (k / Q8_BLOCK) * n]);
    for b in 0..k / Q8_BLOCK {
        for col in 0..n {
            let at = |i: usize| (Q8_BLOCK * b + i) * n + col;
            let amax = (0..Q8_BLOCK).map(|i| w[at(i)].abs()).fold(0.0, f64::max);
            if amax == 0.0 {
                continue;
            }
            let d = amax / 127.0;
            // The scale is stored in fp32: it must round-trip, and so must every value it scales.
            if (d as f32) as f64 != d {
                return None;
            }
            for i in 0..Q8_BLOCK {
                let v = w[at(i)] / d;
                let r = v.round();
                if (v - r).abs() > 1e-9 || r.abs() > 127.0 || r * d != w[at(i)] {
                    return None;
                }
                q[at(i)] = r as i64;
            }
            s[b * n + col] = d;
        }
    }
    Some((q, s))
}

/// `y = x · W` for an `m`-row `x` and a Q8_0 `W`: the reshape of `x` into its blocks, then the fold — or `None` when
/// the fold would move more than reading the weight dense.
///
/// ★ A BATCH READS ITS VIEW ONCE PER COLUMN GROUP. Every item folds all `m` rows against its own columns of the weight,
/// so the view's `m` tiles per K block are read by every item: `m · 2` bytes per K element per group of `32·wb`
/// columns, against the weight's `1.125 · 32·wb`. A site holds `wb` of 2 at a batch of 8 — the view adds ~22% — and
/// fewer at a larger batch, where the view would outweigh what the int8 values save. So the fold is taken only while
/// its bytes stay below the bf16 weight's own, which bounds every dense lowering from below.
pub fn q8_gemv(
    spec: &MatmulSpec,
    names: &MatmulNames,
    fills: (&Fill, &Fill),
    view: &str,
    target: &SpatialTarget,
) -> Result<Option<Vec<SpatialProgram>>, SpatialReject> {
    let (m, k, n) = (spec.m, spec.k, spec.n);
    let tw = target.tile.cols;
    let precision = elem_precision(target)?;
    let acc = AccPolicy::F32.interm_precision();
    let (values_init, scales_init) = match fills.1 {
        Fill::Unknown => (TensorInit::External, TensorInit::External),
        // A weight the program declares and never sets is zeros: every value and every scale 0, nothing to quantize.
        Fill::Const(0) => (TensorInit::Zero, TensorInit::Zero),
        Fill::ConstF(b) if f64::from_bits(*b) == 0.0 => (TensorInit::Zero, TensorInit::Zero),
        fill => {
            let init = init_of(fill, k * n);
            let w: Vec<f64> = (0..k * n)
                .map(|p| init.element_f64(p).unwrap_or(0.0))
                .collect();
            let (q, s) = quantize(&w, k, n).ok_or_else(|| SpatialReject::NotQ8_0 {
                weight: names.b.clone(),
            })?;
            (
                TensorInit::Values(q),
                TensorInit::FloatValues(s.iter().map(|d| d.to_bits()).collect()),
            )
        }
    };
    // ★ `x` AS ITS BLOCKS: a flat row holds 32 values per tile; its `K/32 × 32` view ([`q8_rows_name`]) holds a K
    // block of 1,024 in one.
    let mut out = if m == 1 {
        relayout_rows_to_dense(
            (&names.a, fills.0),
            view,
            k / Q8_BLOCK,
            Q8_BLOCK,
            precision,
            target,
        )?
    } else {
        relayout_rows_to_q8_view((&names.a, fills.0), view, m, k, precision, target)?
    };
    let budget = target.max_local_mem_bytes();
    let n_t = n.div_ceil(tw);
    // The widest block of columns a site holds — at most the accumulator's tiles, each width built and measured. A
    // fold that cannot be BUILT is refused with its own reason; a batch whose rows outgrow a site at one column tile
    // reads the weight dense.
    let mut chosen = None;
    for wb in (1..=n_t.min(row_block_tiles(target)))
        .rev()
        .filter(|w| n_t % w == 0)
    {
        let fold = q8_fold(
            names,
            view,
            (m, k, n),
            wb,
            (&values_init, &scales_init),
            precision,
            acc,
            target,
        )?;
        if fold
            .buffers
            .iter()
            .map(|b| fold.buffer_bytes(b))
            .sum::<usize>()
            <= budget
        {
            chosen = Some((wb, fold));
            break;
        }
    }
    let Some((wb, fold)) = chosen else {
        return Ok(None);
    };
    // Both weights move whole column tiles, the last one's pad with them.
    let (tile, padded) = (target.tile.rows * tw, n_t * tw);
    let weight = k * padded + 4 * (k / Q8_BLOCK) * padded;
    let view_reads = (n_t / wb)
        * k.div_ceil(tile)
        * m
        * tile
        * crate::spatial::target::precision_bytes(precision);
    if weight + view_reads >= 2 * k * padded {
        return Ok(None);
    }
    out.push(fold);
    Ok(Some(out))
}

/// The view of an `m × K` matrix `x` (`1 < m ≤ tile`): the matrix whose tile row `kb·m + r` is row `r`'s K block
/// `kb` — its elements `1024·kb …` as rows of 32 ([`q8_view_rows`]). The whole blocks are one section and a tail another,
/// each writing its part of the view.
fn relayout_rows_to_q8_view(
    x: (&str, &Fill),
    view: &str,
    m: usize,
    k: usize,
    dt: Precision,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let block = target.tile.rows * Q8_BLOCK;
    let (nb, tail) = (k / block, k % block);
    let part = nb > 0 && tail > 0;
    let mut out = Vec::new();
    if nb > 0 {
        out.push(q8_view_section(
            x,
            view,
            (m, k),
            (0, nb, block),
            part,
            dt,
            target,
        )?);
    }
    if tail > 0 {
        out.push(q8_view_section(
            x,
            view,
            (m, k),
            (nb, 1, tail),
            part,
            dt,
            target,
        )?);
    }
    Ok(out)
}

/// K blocks `kb0 … kb0 + kbs − 1` of [`relayout_rows_to_q8_view`], each `width` elements of every row.
///
/// Item `(kb, c)` loads the block's tiles of `x` — every row — as one page and writes `rows` view tiles, rows `c·rows …`
/// of the batch, as one page: view page element `(b, 32ρ + i)` is `x` page element `(c·rows + ρ, 32b + i)`, at offset
/// `W·(c·rows + ρ) + 32b + i` for the page's width `W`. The column's row `ρ` and lane `i` are not affine in it, so
/// `W·(c·rows + ρ) + i` comes from a table over the column, one tile per chunk `c`; `rows` divides `m` and fits the
/// accumulator.
fn q8_view_section(
    x: (&str, &Fill),
    view: &str,
    (m, k): (usize, usize),
    (kb0, kbs, width): (usize, usize, usize),
    part: bool,
    dt: Precision,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (tr, tw) = (target.tile.rows, target.tile.cols);
    let (x_t, page_t, g) = (k / tw, width / tw, width / Q8_BLOCK);
    let page_w = page_t * tw;
    let rows = (1..=m.min(row_block_tiles(target)))
        .rev()
        .find(|d| m % d == 0)
        .unwrap_or(1);
    let chunks = m / rows;
    let ix = index_type(target, (m * page_w) as u64)?;
    let mut t = Template::new(target)?;
    let src = t.tensor(x.0, TensorRole::Input, m, k, dt, init_of(x.1, m * k));
    let dst = t.tensor(
        view,
        if part {
            TensorRole::State
        } else {
            TensorRole::Output
        },
        q8_view_rows(m, k),
        Q8_BLOCK,
        dt,
        TensorInit::Zero,
    );
    let lanes: Vec<i64> = (0..chunks * rows * tw)
        .map(|c| ((c / tw) * page_w + c % tw) as i64)
        .collect();
    let table = t.tensor(
        &format!("{view}__lanes{kb0}"),
        TensorRole::Param,
        1,
        chunks * rows * tw,
        ix,
        TensorInit::Values(lanes),
    );
    let x_in = t.buffer("in_block", page_t, dt);
    let lane_in = t.buffer_deep("in_lanes", 1, rows, ix);
    let out = t.buffer("out", rows, dt);
    let l_x = t.load_block_at(
        "load_block",
        src,
        kb0 * (tr * Q8_BLOCK / tw),
        vec![(LV_ITEM_Y, page_t)],
        1,
        page_t,
        x_t,
        x_in,
    );
    let l_lanes = t.load_block_at(
        "load_lanes",
        table,
        0,
        vec![(LV_ITEM_X, rows)],
        1,
        rows,
        chunks * rows,
        lane_in,
    );
    let lift = ElemAccess {
        base: 0,
        terms: vec![(LV_GEN_ROW, tw as i64)],
        indirect: Some(IndirectTerm {
            operand: 1,
            stride: 1,
            modulus: m * page_w,
        }),
    };
    let s = t.stage(
        "lift_rows",
        StageKind::Compute(ComputeOp::Generic {
            iters: vec![
                IterSpec {
                    var: LV_GEN_ROW,
                    extent: g,
                    kind: IterKind::Parallel,
                },
                IterSpec {
                    var: LV_GEN_COL,
                    extent: rows * tw,
                    kind: IterKind::Parallel,
                },
            ],
            ins: vec![
                lift,
                ElemAccess {
                    base: 0,
                    terms: vec![(LV_GEN_COL, 1)],
                    indirect: None,
                },
            ],
            out: ident_map(page_width(rows, target)),
            body: TensorBody::Operand(0),
        }),
        vec![x_in, lane_in],
        vec![out],
    );
    let st = t.store_block(
        "store_view",
        dst,
        kb0 * m,
        vec![(LV_ITEM_Y, m), (LV_ITEM_X, rows)],
        rows,
        1,
        1,
        out,
    );
    t.edge(l_x, x_in, s);
    t.edge(l_lanes, lane_in, s);
    t.edge(s, out, st);
    let items: Vec<WorkItem> = (0..kbs)
        .flat_map(|kb| (0..chunks).map(move |c| (kb, c)))
        .enumerate()
        .map(|(id, (kb, c))| WorkItem {
            id: WorkItemId::new(id),
            coord: (kb, c),
            out: TileRect {
                row0_t: (kb0 + kb) * m + c * rows,
                col0_t: 0,
                h_t: rows,
                w_t: 1,
            },
        })
        .collect();
    let n_items = items.len();
    let mut p = t.finish(
        format!("q8_view_{m}x{k}_from{kb0}"),
        items,
        AccPolicy::for_result(dt),
        NumericsClaim::Exact,
        target,
    )?;
    p.work.plan = Some(BlockPlan {
        per_site_m: rows,
        per_site_n: 1,
        // A section writing PART of the view counts its items, as every part-writing relayout does.
        grid: if part {
            (1, n_items)
        } else {
            (kbs * chunks, 1)
        },
        inner: InnerLoop {
            num_blocks: 1,
            block_k_tiles: 1,
        },
        block: BlockShape {
            osh: rows,
            osw: 1,
            ibw: 1,
        },
        acc: AccPolicy::for_result(dt),
        pack_accumulate: false,
        engines: EnginePacking::LoadsTogether,
        granularity: Granularity::PerTile,
    });
    Ok(p)
}

/// The fold: item `y` owns `wb` column tiles of the product for all `m` rows; its K blocks are the weight's tile rows
/// 32 at a time. Row `r`'s block dots read tile `r` of the view's K-block page; every row reads the same int8 and scale
/// pages.
///
/// ★ A K THAT IS NOT WHOLE BLOCKS ENDS IN A TAIL. The last `K mod 1024` rows — whole Q8_0 blocks, `g` of them — are one
/// more block, `g` tile rows of values and `g` scale rows, folded ONCE per item outside the K loop and added into the
/// result: a tensor-parallel chip's rows of a row-split weight (768, 2,176) read as stored like any other.
#[allow(clippy::too_many_arguments)]
fn q8_fold(
    names: &MatmulNames,
    x_tiles: &str,
    (m, k, n): (usize, usize, usize),
    wb: usize,
    (values_init, scales_init): (&TensorInit, &TensorInit),
    precision: Precision,
    acc: Precision,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (tr, tw) = (target.tile.rows, target.tile.cols);
    let block_rows = tr * Q8_BLOCK;
    let (n_t, items, nb, g_tail) = (
        n.div_ceil(tw),
        n.div_ceil(tw) / wb,
        k / block_rows,
        (k % block_rows) / Q8_BLOCK,
    );
    let (w1, wn) = (page_width(1, target), page_width(wb, target));
    let mut t = Template::new(target)?;
    let xt = t.tensor(
        x_tiles,
        TensorRole::Input,
        q8_view_rows(m, k),
        Q8_BLOCK,
        precision,
        TensorInit::External,
    );
    let qt = t.tensor(
        &stored_name(&names.b, Q8Part::Values),
        TensorRole::Param,
        k,
        n,
        Precision::Int8,
        values_init.clone(),
    );
    let st = t.tensor(
        &stored_name(&names.b, Q8Part::Scales),
        TensorRole::Param,
        k / Q8_BLOCK,
        n,
        acc,
        scales_init.clone(),
    );
    let yt = t.tensor(&names.c, TensorRole::Output, m, n, acc, TensorInit::Zero);
    let load = |t: &mut Template,
                name: &str,
                tensor: TensorId,
                (base, steps): (usize, Vec<(LoopVar, usize)>),
                (h, w, stride): (usize, usize, usize),
                blocks: usize,
                into: BufferId| {
        let origin = TileAddr {
            base,
            steps,
            indirect: None,
        };
        t.stage(
            name,
            StageKind::Load {
                tensor,
                pattern: AccessPattern {
                    origin,
                    block_h: h,
                    block_w: w,
                    stride_w: 1,
                    stride_h: stride,
                },
                num_blocks: blocks,
                multicast: None,
            },
            Vec::new(),
            vec![into],
        )
    };
    let map = |base: i64, terms: &[(LoopVar, i64)]| ElemAccess {
        base,
        terms: terms.to_vec(),
        indirect: None,
    };
    let cols = wb * tw;
    let named = |stem: &str, r: usize| {
        if m == 1 {
            stem.to_string()
        } else {
            format!("{stem}_{r}")
        }
    };
    // One block of `g` Q8_0 blocks, every row, ONE stage a row: `y[r, j] = Σ_b s[b, j]·Σ_i x[r, 32b + i]·q[32b + i, j]` —
    // the lanes `i` contracted, the block's `g` Q8_0 blocks `b` folded down each column. Row `r`'s block is tile `r` of
    // the view page, its row `b`; the values' page holds block `b`'s tile row as tiles `b·wb …`, row `i`; the scales'
    // page holds block `b`'s scales in its row `b`. Returns each row's stage and sum.
    let block_sums = |t: &mut Template,
                      stem: &str,
                      (x_in, q_in, s_in): (BufferId, BufferId, BufferId),
                      g: usize|
     -> Vec<(StageId, BufferId)> {
        let q_plane = (g * wb * tw) as i64;
        (0..m)
            .map(|r| {
                let y_blk = t.scratch_deep(&named(stem, r), 1, wb, acc);
                let iters = vec![
                    IterSpec {
                        var: LV_GEN_ROW,
                        extent: g,
                        kind: IterKind::Reduce {
                            pool: ReducePool::Sum,
                        },
                    },
                    IterSpec {
                        var: LV_GEN_COL,
                        extent: cols,
                        kind: IterKind::Parallel,
                    },
                    IterSpec {
                        var: LV_GEN_K,
                        extent: Q8_BLOCK,
                        kind: IterKind::Reduce {
                            pool: ReducePool::Sum,
                        },
                    },
                ];
                let s_blk = t.stage(
                    &named(stem, r),
                    StageKind::Compute(ComputeOp::Generic {
                        iters,
                        ins: vec![
                            map(
                                (r * w1) as i64,
                                &[(LV_GEN_ROW, (m * w1) as i64), (LV_GEN_K, 1)],
                            ),
                            map(
                                0,
                                &[
                                    (LV_GEN_K, q_plane),
                                    (LV_GEN_ROW, cols as i64),
                                    (LV_GEN_COL, 1),
                                ],
                            ),
                            map(0, &[(LV_GEN_ROW, cols as i64), (LV_GEN_COL, 1)]),
                        ],
                        out: per_col_map(),
                        body: TensorBody::Mul(
                            Box::new(TensorBody::Mul(
                                Box::new(TensorBody::Operand(0)),
                                Box::new(TensorBody::Operand(1)),
                            )),
                            Box::new(TensorBody::Operand(2)),
                        ),
                    }),
                    vec![x_in, q_in, s_in],
                    vec![y_blk],
                );
                (s_blk, y_blk)
            })
            .collect()
    };
    // Row `r`'s sum into row `r`: `a + Σ_r [row = r]·sum_r` — a single row's sum is every row's.
    let row_of = |r: usize, at: usize| {
        let op = Box::new(TensorBody::Operand(at));
        let zero = || Box::new(TensorBody::Const(0.0f64.to_bits()));
        if m == 1 {
            op
        } else {
            Box::new(TensorBody::Select {
                cond: Pred::AxisLt(LV_GEN_ROW, PredRhs::Const(r + 1)),
                then: Box::new(TensorBody::Select {
                    cond: Pred::AxisGe(LV_GEN_ROW, PredRhs::Const(r)),
                    then: op,
                    otherwise: zero(),
                }),
                otherwise: zero(),
            })
        }
    };
    let pointwise = vec![
        IterSpec {
            var: LV_GEN_ROW,
            extent: tr,
            kind: IterKind::Parallel,
        },
        IterSpec {
            var: LV_GEN_COL,
            extent: cols,
            kind: IterKind::Parallel,
        },
    ];
    let mut edges = Vec::new();
    // The whole blocks, folded: K block `t` is the view's tile rows `m·t …`, the values' tile rows `32t … 32t + 31`, the
    // scales' tile row `t` — zero before the first, each block's sums folded in, the carry read after the last.
    let run = (nb > 0).then(|| {
        let x_in = t.buffer("in_x", m, precision);
        let q_in = t.buffer("in_q8", tr * wb, Precision::Int8);
        let s_in = t.buffer("in_scale", wb, acc);
        let l_x = load(
            &mut t,
            "load_x",
            xt,
            (0, vec![(LV_KBLOCK, m)]),
            (m, 1, 1),
            nb,
            x_in,
        );
        let l_q = load(
            &mut t,
            "load_q8",
            qt,
            (0, vec![(LV_KBLOCK, tr * n_t), (LV_ITEM_Y, wb)]),
            (tr, wb, n_t),
            nb,
            q_in,
        );
        let l_s = load(
            &mut t,
            "load_scale",
            st,
            (0, vec![(LV_KBLOCK, n_t), (LV_ITEM_Y, wb)]),
            (1, wb, n_t),
            nb,
            s_in,
        );
        let sums = block_sums(&mut t, "block", (x_in, q_in, s_in), Q8_BLOCK);
        edges.extend([
            (l_x, x_in, sums[0].0),
            (l_q, q_in, sums[0].0),
            (l_s, s_in, sums[0].0),
        ]);
        let run = t.scratch_deep("run_sum", 2, wb, acc);
        t.stage(
            "init_sum",
            StageKind::Compute(ComputeOp::Generic {
                iters: pointwise.clone(),
                ins: Vec::new(),
                out: ident_map(wn),
                body: TensorBody::Const(0.0f64.to_bits()),
            }),
            Vec::new(),
            vec![run],
        );
        let update = (0..m).fold(TensorBody::Operand(0), |body, r| {
            TensorBody::Add(Box::new(body), row_of(r, r + 1))
        });
        let mut fold_ins = vec![run];
        fold_ins.extend(sums.iter().map(|(_, b)| *b));
        t.stage(
            "fold_sum",
            StageKind::Compute(ComputeOp::Generic {
                iters: pointwise.clone(),
                ins: vec![ident_map(wn); m + 1],
                out: ident_map(wn),
                body: update,
            }),
            fold_ins,
            vec![run],
        );
        run
    });
    // The tail, once per item: the view's tile rows `m·nb …`, the values' `g` tile rows from `32·nb`, the scales' tile row
    // `nb` (its first `g` rows).
    let tail = (g_tail > 0).then(|| {
        let x_in = t.buffer_deep("in_x_tail", 1, m, precision);
        let q_in = t.buffer_deep("in_q8_tail", 1, g_tail * wb, Precision::Int8);
        let s_in = t.buffer_deep("in_scale_tail", 1, wb, acc);
        let l_x = load(
            &mut t,
            "load_x_tail",
            xt,
            (nb * m, Vec::new()),
            (m, 1, 1),
            1,
            x_in,
        );
        let l_q = load(
            &mut t,
            "load_q8_tail",
            qt,
            (tr * nb * n_t, vec![(LV_ITEM_Y, wb)]),
            (g_tail, wb, n_t),
            1,
            q_in,
        );
        let l_s = load(
            &mut t,
            "load_scale_tail",
            st,
            (nb * n_t, vec![(LV_ITEM_Y, wb)]),
            (1, wb, n_t),
            1,
            s_in,
        );
        let sums = block_sums(&mut t, "tail", (x_in, q_in, s_in), g_tail);
        edges.extend([
            (l_x, x_in, sums[0].0),
            (l_q, q_in, sums[0].0),
            (l_s, s_in, sums[0].0),
        ]);
        sums
    });
    // The product: the carried sum, the tail's rows added in.
    let mut ins = Vec::new();
    let mut body = run.map(|r| {
        ins.push(r);
        TensorBody::Operand(0)
    });
    for (r, (_, b)) in tail.iter().flatten().enumerate() {
        ins.push(*b);
        let term = *row_of(r, ins.len() - 1);
        body = Some(match body {
            Some(sum) => TensorBody::Add(Box::new(sum), Box::new(term)),
            None => term,
        });
    }
    let out = t.buffer("out", wb, acc);
    let body = body.expect("a product has whole blocks or a tail");
    let s_out = t.stage(
        "result",
        StageKind::Compute(ComputeOp::Generic {
            iters: pointwise,
            ins: vec![ident_map(wn); ins.len()],
            out: ident_map(wn),
            body,
        }),
        ins,
        vec![out],
    );
    let st_y = t.store_block("store_y", yt, 0, vec![(LV_ITEM_Y, wb)], 1, wb, wb, out);
    edges.push((s_out, out, st_y));
    for (from, buf, to) in edges {
        t.edge(from, buf, to);
    }
    let work: Vec<WorkItem> = (0..items)
        .map(|y| WorkItem {
            id: WorkItemId::new(y),
            coord: (y, 0),
            out: TileRect {
                row0_t: 0,
                col0_t: y * wb,
                h_t: 1,
                w_t: wb,
            },
        })
        .collect();
    let name = if m == 1 {
        format!("q8_gemv_{k}x{n}_w{wb}")
    } else {
        format!("q8_gemm_{m}x{k}x{n}_w{wb}")
    };
    let mut p = t.finish(
        name,
        work,
        AccPolicy::F32,
        NumericsClaim::Approx {
            reason: format!("a Q8_0 product over {m} {precision:?} row(s)"),
        },
        target,
    )?;
    p.work.plan = Some(BlockPlan {
        per_site_m: 1,
        per_site_n: wb,
        grid: (1, items),
        inner: InnerLoop {
            num_blocks: nb.max(1),
            block_k_tiles: 1,
        },
        block: BlockShape {
            osh: 1,
            osw: wb,
            ibw: 1,
        },
        acc: AccPolicy::F32,
        pack_accumulate: false,
        engines: EnginePacking::LoadsTogether,
        granularity: Granularity::PerTile,
    });
    p.work.k_tiles = nb.max(1);
    Ok(p)
}

/// Rows of the view of an `m`-row `x` over `K`: one row's `K/32` blocks as they fall, or — for a batch — each row's K
/// blocks a whole tile row apiece, the tail's included (its rows past `K mod 1024 / 32` zero).
fn q8_view_rows(m: usize, k: usize) -> usize {
    if m == 1 {
        k / Q8_BLOCK
    } else {
        m * k.div_ceil(Q8_BLOCK * Q8_BLOCK) * Q8_BLOCK
    }
}
