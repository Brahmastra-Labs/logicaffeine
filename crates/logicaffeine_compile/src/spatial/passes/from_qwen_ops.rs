//! `from_qwen_ops` — every op of an authored transformer forward, lowered as a COMPOSITION of the closed
//! compute algebra: chains of [`ComputeOp::Generic`] and [`ComputeOp::MatmulTiles`], and nothing else.
//!
//! ★ NOT ONE NEW `ComputeOp`. Rope, silu, sigmoid, gelu, decay, l2norm, transpose, a causal conv ring, a
//! Gated-DeltaNet step and GQA attention all lower through this file, and `spatial_algebra_lock`'s frozen
//! per-printer arm counts do not move for any of them. That is the campaign's whole claim, and it holds
//! because B-P3 made the pieces general: a reduction is an ITERATOR KIND, a broadcast is an ABSENT TERM, a
//! transform is an `SfpuKind`, a piecewise region is a `Select` on an AXIS, and an access map is a signed
//! FLAT affine offset rather than one term per axis.
//!
//! ── The layout contract of a composed program ────────────────────────────────────────────────────────
//!
//! ★ EVERY ACTIVATION IS FLAT: `1 × n`. This is not a convenience, it is what makes composition possible
//! at all. An `ElemAccess` is a flat offset into a page read as `tile.rows × (page_tiles · tile.cols)`, so
//! element `p` of a `1 × n` tensor is at offset `p` — the SAME number the language's flat `item p` uses.
//! Give the same tensor a `2 × 4` shape in one section and a `1 × 8` shape in the next and the two
//! sections disagree about where element 4 lives, with no affine map able to reconcile them (the
//! conversion needs `p / cols`, which is not affine in `p`). Flat is the one layout every op can both
//! produce and consume.
//!
//! ★ A 2-D VIEW IS A BAND, NOT A RESHAPE. Where an op is per-head — rope's rotations, l2norm across
//! heads, a GDN head's memory — the heads are contiguous RANGES of the flat column axis, and a range is
//! a `Pred::AxisLt` on a CONSTANT. So one `Generic` computes every head by nesting one `Select` per head
//! (`banded`), and inside head `h`'s band an operand that is indexed per-head is read through that
//! head's own base (`c - h·width`), while a per-head SCALAR is read through a CONSTANT map: the
//! reduction that produced it broadcast it across the whole of its tile row, and inside one band the
//! row is fixed. No divisibility, no reshape, no gather.

use std::collections::BTreeSet;

use super::from_tensor_op::{
    const_map, eps_nano, ident_map, items_per_tile, page_width, per_col_map, per_row_map, MatmulNames,
    Template,
};
use super::{SpatialReject, LV_GEN_COL, LV_GEN_K, LV_GEN_ROW, LV_ITEM_X, LV_ITEM_Y, LV_KBLOCK};
use crate::codegen_sva::fabric_target::Precision;
use crate::spatial::ir::*;
use crate::spatial::pipeline::StageNames;
use crate::spatial::target::{precision_int_bits, SpatialTarget};
use crate::tensor_recognize::qwen::Window;
use crate::tensor_recognize::qwen::{
    ArgMaxSpec, AttentionSpec, CausalConvSpec, DecaySpec, FullAttentionSpec, GateAct, GatedDeltaSpec,
    GeluSpec, HeadSliceSpec, IntClampSpec, IntRequantSpec, JoinSpec, L2NormSpec, LutAttentionSpec, MoeGroups, MoeScore, MoeSpec,
    ReluSpec,
    RopeSpec, RowGatherSpec, RowIndex, RowsGatherSpec, ScaleSpec, SetRowSpec, SigmoidSpec, SiluSpec, SoftcapSpec,
    TransposeSpec,
};
use crate::tensor_recognize::{Fill, FusedRowSpec, HeadMatmulSpec};

// ── The flat vocabulary ───────────────────────────────────────────────────────────────────────────

/// The iteration space of a FLAT stage: ONE row, `n` columns. The evaluator writes the result at
/// `plane[c / tile][(r % tile) · tile + c % tile]`, so a single-row space lands the whole answer in tile
/// row 0 — which IS the `1 × n` layout every activation has.
fn flat_space(n: usize) -> Vec<IterSpec> {
    vec![
        IterSpec {
            var: LV_GEN_ROW,
            extent: 1,
            kind: IterKind::Parallel,
        },
        IterSpec {
            var: LV_GEN_COL,
            extent: n.max(1),
            kind: IterKind::Parallel,
        },
    ]
}

/// A 2-D stage's space: `rows` parallel rows by `cols` columns, the columns either walked or folded.
fn head_space(rows: usize, cols: usize, reduce: Option<ReducePool>) -> Vec<IterSpec> {
    vec![
        IterSpec {
            var: LV_GEN_ROW,
            extent: rows.max(1),
            kind: IterKind::Parallel,
        },
        IterSpec {
            var: LV_GEN_COL,
            extent: cols.max(1),
            kind: match reduce {
                Some(pool) => IterKind::Reduce { pool },
                None => IterKind::Parallel,
            },
        },
    ]
}

/// A flat map along the column axis with a signed base: element `c + base` of the operand's page.
fn at(base: i64) -> ElemAccess {
    ElemAccess {
        base,
        terms: vec![(LV_GEN_COL, 1)],
        indirect: None,
    }
}

/// A map that reads ONE fixed element of the operand's page — the shape a per-head scalar takes inside
/// a band, where the head (and therefore the tile row the reduction broadcast it across) is constant.
fn fixed(base: i64) -> ElemAccess {
    ElemAccess {
        base,
        terms: Vec::new(),
        indirect: None,
    }
}

/// A 2-D map over a page `width` elements wide: `base + row·row_stride + col·col_stride`.
fn map2(base: i64, row_stride: i64, col_stride: i64) -> ElemAccess {
    ElemAccess {
        base,
        terms: vec![(LV_GEN_ROW, row_stride), (LV_GEN_COL, col_stride)],
        indirect: None,
    }
}

fn op(i: usize) -> Box<TensorBody> {
    Box::new(TensorBody::Operand(i))
}

fn konst(v: f64) -> Box<TensorBody> {
    Box::new(TensorBody::Const(v.to_bits()))
}

fn add(a: Box<TensorBody>, b: Box<TensorBody>) -> Box<TensorBody> {
    Box::new(TensorBody::Add(a, b))
}

fn sub(a: Box<TensorBody>, b: Box<TensorBody>) -> Box<TensorBody> {
    Box::new(TensorBody::Sub(a, b))
}

fn mul(a: Box<TensorBody>, b: Box<TensorBody>) -> Box<TensorBody> {
    Box::new(TensorBody::Mul(a, b))
}

fn unary(kind: SfpuKind, a: Box<TensorBody>) -> Box<TensorBody> {
    Box::new(TensorBody::Unary { kind, arg: a })
}

/// `x` soft-capped at `c`, `c·tanh(x/c)`, as `2c·σ(2x/c) − c`: one exponential and one reciprocal, reaching `±c` in both
/// tails.
fn capped(x: Box<TensorBody>, c: f64) -> Box<TensorBody> {
    let sigmoid = unary(SfpuKind::Recip, add(konst(1.0), unary(SfpuKind::Exp, mul(x, konst(-2.0 / c)))));
    sub(mul(konst(2.0 * c), sigmoid), konst(c))
}

/// `x / (1 + e^-x)` written out of the algebra rather than as `SfpuKind::Silu`, for the places a gate is
/// one factor of a larger body (Gated DeltaNet's `silu(z)`); the standalone op uses the fused kind.
fn silu_expr(x: Box<TensorBody>) -> Box<TensorBody> {
    Box::new(TensorBody::Unary {
        kind: SfpuKind::Silu,
        arg: x,
    })
}

/// A Gated DeltaNet's output gate on `z`: `silu(z)`, or `σ(z)` as `1/(1 + e^(−z))`.
fn delta_gate(gate: GateAct, z: Box<TensorBody>) -> Box<TensorBody> {
    match gate {
        GateAct::Silu => silu_expr(z),
        GateAct::Sigmoid => unary(SfpuKind::Recip, add(konst(1.0), unary(SfpuKind::Exp, mul(z, konst(-1.0))))),
    }
}

/// Nested `Select`s over the flat column axis: `bands[k] = (hi, body)` computes `body` for every column
/// below `hi` that the earlier bands did not claim. The bands must be given in increasing `hi`, and the
/// last one is the fall-through — so a partition of `[0, n)` costs exactly `bands.len() - 1` predicates
/// and every column is covered.
fn banded(bands: Vec<(usize, TensorBody)>) -> TensorBody {
    let mut it = bands.into_iter().rev();
    let (_, mut acc) = it.next().expect("a banded body has at least one band");
    for (hi, body) in it {
        acc = TensorBody::Select {
            cond: Pred::AxisLt(LV_GEN_COL, PredRhs::Const(hi)),
            then: Box::new(body),
            otherwise: Box::new(acc),
        };
    }
    acc
}

/// Which of the target's elementwise datapaths a stage's body runs on: the float one every float op takes, or the
/// integer one an int8 model's requant, saturation and ReLU take — named by the op, never guessed from its body (a
/// float ReLU's `max(x, 0.0)` is integral arithmetic over float data).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Datapath {
    Float,
    Int,
}

impl Datapath {
    /// The datapath a value of the walk's element kind rides.
    fn of(float: bool) -> Datapath {
        if float {
            Datapath::Float
        } else {
            Datapath::Int
        }
    }

    fn precision(self, target: &SpatialTarget) -> Result<Precision, SpatialReject> {
        match self {
            Datapath::Float => elem_precision(target),
            Datapath::Int => super::from_tensor_op::elementwise_int(target).ok_or(
                SpatialReject::MissingCompute {
                    needed: "an integer elementwise datapath",
                    target: target.name,
                },
            ),
        }
    }
}

/// The float precision the target's elementwise datapath offers (the RDU's witnessed one is bf16).
pub(crate) fn elem_precision(target: &SpatialTarget) -> Result<Precision, SpatialReject> {
    super::from_tensor_op::elementwise_float(target).ok_or(SpatialReject::MissingCompute {
        needed: "a float elementwise datapath",
        target: target.name,
    })
}

/// The width in elements of a buffer holding `page_tiles` tiles.
fn wide(page_tiles: usize, target: &SpatialTarget) -> usize {
    page_width(page_tiles, target)
}

/// Tiles a flat `1 × n` row occupies.
fn tiles_of(n: usize, target: &SpatialTarget) -> usize {
    n.div_ceil(target.tile.cols).max(1)
}

/// The ONE work item of a flat section: the whole `1 × cols_t` tile row.
///
/// ★ ONE ITEM, NOT ONE PER TILE. `place` clones the WHOLE program per site, so a vector split into one
/// item per tile becomes that many copies of every stage — which is how a 64-lane decision program ended
/// up with two identical `Scalar` stages carrying the same 75-cell body. A stage whose compute covers
/// the entire row in one pass has exactly one item, and the site count follows the work rather than the
/// tile count.
fn whole_row(cols_t: usize) -> Vec<WorkItem> {
    vec![WorkItem {
        id: WorkItemId::new(0),
        coord: (0, 0),
        out: TileRect {
            row0_t: 0,
            col0_t: 0,
            h_t: 1,
            w_t: cols_t,
        },
    }]
}

/// A flat operand slot: which source tensor it reads and through which map.
struct Slot {
    buf: BufferId,
    map: ElemAccess,
}

/// A flat section under construction: the loads of its operands, then ONE `Generic` over the whole row — or, for a
/// result wider than an accumulator page, over one block of it an item ([`FlatBuild::column_blocked`]).
struct FlatBuild {
    t: Template,
    precision: Precision,
    loads: Vec<(StageId, BufferId)>,
    blocks: Option<ColumnBlocks>,
}

/// A flat result blocked by columns: `items` items of `blk` result tiles each.
#[derive(Clone, Copy, Debug)]
struct ColumnBlocks {
    blk: usize,
    items: usize,
}

impl FlatBuild {
    fn new(target: &SpatialTarget) -> Result<FlatBuild, SpatialReject> {
        FlatBuild::on(Datapath::Float, target)
    }

    /// A flat section whose activations ride `datapath`.
    fn on(datapath: Datapath, target: &SpatialTarget) -> Result<FlatBuild, SpatialReject> {
        FlatBuild::at(datapath.precision(target)?, target)
    }

    /// A flat section whose activations are `precision`, chosen by its caller.
    fn at(precision: Precision, target: &SpatialTarget) -> Result<FlatBuild, SpatialReject> {
        Ok(FlatBuild {
            t: Template::new(target)?,
            precision,
            loads: Vec::new(),
            blocks: None,
        })
    }

    /// A float section whose `n`-column result is wider than an accumulator page: its pointwise stage
    /// ([`Self::pointwise`]) runs an item per block of `blk` result tiles, every operand loaded whole, so the page an item
    /// computes is one block. A result a page holds is the one whole-row item, as [`Self::new`] makes it.
    fn columns(n: usize, target: &SpatialTarget) -> Result<FlatBuild, SpatialReject> {
        let mut f = FlatBuild::new(target)?;
        if row_needs_blocking(n, target) {
            let cols_t = tiles_of(n, target);
            let blk = row_block(cols_t, target);
            f.blocks = Some(ColumnBlocks { blk, items: cols_t / blk });
        }
        Ok(f)
    }

    /// The buffer a section's `n`-column result is computed into: the whole row, or one block of it.
    fn result_page(&mut self, name: &str, n: usize, precision: Precision, target: &SpatialTarget) -> BufferId {
        let tiles = self.blocks.map_or(tiles_of(n, target), |b| b.blk);
        self.t.buffer(name, tiles, precision)
    }

    /// `dst[c] = body(c)` over the result's `n` columns, every map and predicate written in RESULT columns. In a
    /// column-blocked section the stage walks its item's block, and a column `c` of the block is result column
    /// `c + o` for the item's first column `o`, read from a compile-time table one page an item: a map that walks the
    /// columns gathers through `o`; a predicate's constant bound `k` becomes the table's `k − o`; a bound or a gather
    /// index the section computes is moved by `o` in a stage of its own first. Returns the stages, in order.
    fn pointwise(
        &mut self,
        name: &str,
        n: usize,
        slots: Vec<Slot>,
        body: TensorBody,
        dst: BufferId,
        target: &SpatialTarget,
    ) -> Result<Vec<StageId>, SpatialReject> {
        let Some(ColumnBlocks { blk, items }) = self.blocks else {
            let w = wide(self.t.page_tiles(dst), target);
            return Ok(vec![self.compute(name, flat_space(n), slots, ident_map(w), body, dst)]);
        };
        let tw = target.tile.cols;
        let span = items * blk * tw;
        // The constant column bounds, each a table entry.
        let mut bounds: Vec<usize> = Vec::new();
        fn constant_bounds(b: &TensorBody, out: &mut Vec<usize>) {
            match b {
                TensorBody::Operand(_) | TensorBody::Const(_) => {}
                TensorBody::Add(x, y)
                | TensorBody::Sub(x, y)
                | TensorBody::Mul(x, y)
                | TensorBody::Min(x, y)
                | TensorBody::Max(x, y)
                | TensorBody::FloorDiv(x, y) => {
                    constant_bounds(x, out);
                    constant_bounds(y, out);
                }
                TensorBody::Unary { arg, .. } | TensorBody::Shr { arg, .. } => constant_bounds(arg, out),
                TensorBody::Select { cond, then, otherwise } => {
                    if let (LV_GEN_COL, PredRhs::Const(k)) = (cond.var(), cond.rhs()) {
                        if !out.contains(&k) {
                            out.push(k);
                        }
                    }
                    constant_bounds(then, out);
                    constant_bounds(otherwise, out);
                }
            }
        }
        constant_bounds(&body, &mut bounds);
        // Item `i`'s page: its first column `o = i·blk·tile`, then each bound less `o`.
        let per = (1 + bounds.len()).div_ceil(tw);
        let magnitude = span.max(bounds.iter().copied().max().unwrap_or(0)) + span;
        let ix = index_type(target, magnitude as u64)?;
        let values: Vec<i64> = (0..items)
            .flat_map(|i| {
                let o = (i * blk * tw) as i64;
                let mut page: Vec<i64> = std::iter::once(o).chain(bounds.iter().map(|k| *k as i64 - o)).collect();
                page.resize(per * tw, 0);
                page
            })
            .collect();
        let table = self.t.tensor(&format!("{name}__columns"), TensorRole::Param, 1, items * per * tw, ix, TensorInit::Values(values));
        let at_table = self.t.buffer_deep(&format!("in_{name}_columns"), 1, per, ix);
        let l_table = self.t.load_block_at(&format!("load_{name}_columns"), table, 0, vec![(LV_ITEM_X, per)], 1, per, items * per, at_table);
        self.loads.push((l_table, at_table));
        let mut stages = Vec::new();
        // The block's slots: the first column, each bound, then the section's own, then what is moved by `o`.
        let first = Slot { buf: at_table, map: fixed(0) };
        let mut out: Vec<Slot> = std::iter::once(first).chain((0..bounds.len()).map(|m| Slot { buf: at_table, map: fixed(1 + m as i64) })).collect();
        let shift = out.len();
        let one = wide(1, target);
        let mut moved = |f: &mut FlatBuild, what: &str, from: &Slot, body: Box<TensorBody>| {
            let page = f.t.scratch_deep(&format!("{name}_{what}"), 1, 1, ix);
            stages.push(f.compute(&format!("{name}_{what}"), flat_space(1), vec![Slot { buf: at_table, map: fixed(0) }, Slot { buf: from.buf, map: from.map.clone() }], ident_map(one), *body, page));
            page
        };
        let mut own = Vec::with_capacity(slots.len());
        let mut extra: Vec<Slot> = Vec::new();
        for (j, slot) in slots.iter().enumerate() {
            let walks = slot.map.terms.iter().find(|(v, _)| *v == LV_GEN_COL).map_or(0, |(_, s)| *s);
            let map = match (walks, slot.map.indirect) {
                (0, _) => slot.map.clone(),
                (s, None) => ElemAccess {
                    indirect: Some(IndirectTerm { operand: 0, stride: s, modulus: span + 1 }),
                    ..slot.map.clone()
                },
                // A gather already indexed by data: its index moves by `o` too, shifted to stay non-negative.
                (s, Some(ind)) => {
                    let lift = ind.stride.unsigned_abs() as usize * ind.modulus;
                    let index = &slots[ind.operand];
                    let page = moved(self, &format!("index{j}"), index, add(add(mul(op(0), konst(s as f64)), mul(op(1), konst(ind.stride as f64))), konst(lift as f64)));
                    extra.push(Slot { buf: page, map: fixed(0) });
                    ElemAccess {
                        base: slot.map.base - lift as i64,
                        terms: slot.map.terms.clone(),
                        indirect: Some(IndirectTerm { operand: usize::MAX - (extra.len() - 1), stride: 1, modulus: s.unsigned_abs() as usize * span + 2 * lift + 1 }),
                    }
                }
            };
            own.push(Slot { buf: slot.buf, map });
        }
        // A bound the section computes moves by `o`.
        let mut computed: Vec<(usize, usize)> = Vec::new();
        fn computed_bounds(b: &TensorBody, out: &mut Vec<usize>) {
            match b {
                TensorBody::Operand(_) | TensorBody::Const(_) => {}
                TensorBody::Add(x, y)
                | TensorBody::Sub(x, y)
                | TensorBody::Mul(x, y)
                | TensorBody::Min(x, y)
                | TensorBody::Max(x, y)
                | TensorBody::FloorDiv(x, y) => {
                    computed_bounds(x, out);
                    computed_bounds(y, out);
                }
                TensorBody::Unary { arg, .. } | TensorBody::Shr { arg, .. } => computed_bounds(arg, out),
                TensorBody::Select { cond, then, otherwise } => {
                    if let (LV_GEN_COL, PredRhs::Operand(o)) = (cond.var(), cond.rhs()) {
                        if !out.contains(&o) {
                            out.push(o);
                        }
                    }
                    computed_bounds(then, out);
                    computed_bounds(otherwise, out);
                }
            }
        }
        let mut operands = Vec::new();
        computed_bounds(&body, &mut operands);
        for o in operands {
            let page = moved(self, &format!("bound{o}"), &slots[o], sub(op(1), op(0)));
            extra.push(Slot { buf: page, map: fixed(0) });
            computed.push((o, extra.len() - 1));
        }
        let base = shift + own.len();
        // The placeholders for the moved indices name their slot past the section's own.
        for slot in &mut own {
            if let Some(ind) = slot.map.indirect.as_mut() {
                if ind.operand > usize::MAX / 2 {
                    ind.operand = base + (usize::MAX - ind.operand);
                }
            }
        }
        out.extend(own);
        out.extend(extra);
        fn rewrite(b: &TensorBody, shift: usize, bounds: &[usize], computed: &[(usize, usize)], base: usize) -> TensorBody {
            let r = |x: &TensorBody| Box::new(rewrite(x, shift, bounds, computed, base));
            match b {
                TensorBody::Operand(i) => TensorBody::Operand(i + shift),
                TensorBody::Const(c) => TensorBody::Const(*c),
                TensorBody::Add(x, y) => TensorBody::Add(r(x), r(y)),
                TensorBody::Sub(x, y) => TensorBody::Sub(r(x), r(y)),
                TensorBody::Mul(x, y) => TensorBody::Mul(r(x), r(y)),
                TensorBody::Min(x, y) => TensorBody::Min(r(x), r(y)),
                TensorBody::Max(x, y) => TensorBody::Max(r(x), r(y)),
                TensorBody::FloorDiv(x, y) => TensorBody::FloorDiv(r(x), r(y)),
                TensorBody::Shr { arg, by } => TensorBody::Shr { arg: r(arg), by: *by },
                TensorBody::Unary { kind, arg } => TensorBody::Unary { kind: *kind, arg: r(arg) },
                TensorBody::Select { cond, then, otherwise } => {
                    let rhs = match (cond.var(), cond.rhs()) {
                        (LV_GEN_COL, PredRhs::Const(k)) => PredRhs::Operand(1 + bounds.iter().position(|b| *b == k).expect("every bound is in the table")),
                        (LV_GEN_COL, PredRhs::Operand(o)) => {
                            PredRhs::Operand(base + computed.iter().find(|(from, _)| *from == o).expect("every computed bound is moved").1)
                        }
                        (_, PredRhs::Operand(o)) => PredRhs::Operand(o + shift),
                        (_, rhs) => rhs,
                    };
                    let cond = match cond {
                        Pred::AxisLt(v, _) => Pred::AxisLt(*v, rhs),
                        Pred::AxisGe(v, _) => Pred::AxisGe(*v, rhs),
                    };
                    TensorBody::Select { cond, then: r(then), otherwise: r(otherwise) }
                }
            }
        }
        let body = rewrite(&body, shift, &bounds, &computed, base);
        let w = wide(blk, target);
        stages.push(self.compute(name, flat_space(blk * tw), out, ident_map(w), body, dst));
        Ok(stages)
    }

    /// Declare a flat `1 × n` operand tensor and load the whole row into a buffer.
    fn input(&mut self, name: &str, n: usize, fill: &Fill, target: &SpatialTarget) -> BufferId {
        let p = self.precision;
        self.input_as(name, n, fill, p, target)
    }

    /// [`Self::input`] at an explicit element precision — an integer table, or an INDEX (the target's address
    /// integer, [`index_type`]: a float datapath holds integers exactly only up to its mantissa).
    fn input_as(
        &mut self,
        name: &str,
        n: usize,
        fill: &Fill,
        precision: Precision,
        target: &SpatialTarget,
    ) -> BufferId {
        self.input_paged(name, n, fill, precision, 2, target)
    }

    /// [`Self::input_as`] into a buffer of `depth` pages: one for an operand a one-item section loads once.
    fn input_paged(
        &mut self,
        name: &str,
        n: usize,
        fill: &Fill,
        precision: Precision,
        depth: usize,
        target: &SpatialTarget,
    ) -> BufferId {
        let cols_t = tiles_of(n, target);
        let id = self.t.tensor(
            name,
            TensorRole::Input,
            1,
            n,
            precision,
            super::from_tensor_op::init_of(fill, n),
        );
        let buf = self.t.buffer_deep(&format!("in_{name}"), depth, cols_t, precision);
        let l = self
            .t
            .load(&format!("load_{name}"), id, vec![], cols_t, buf);
        self.loads.push((l, buf));
        buf
    }

    /// Tiles `first..first + tiles_of(n)` of a `1 × total` input, `n` elements from tile `first`, into a page of their
    /// own loaded once: one block of a row wider than a page.
    #[allow(clippy::too_many_arguments)]
    fn input_slice(
        &mut self,
        name: &str,
        total: usize,
        first: usize,
        n: usize,
        fill: &Fill,
        precision: Precision,
        target: &SpatialTarget,
    ) -> BufferId {
        let id = self.t.tensor(name, TensorRole::Input, 1, total, precision, super::from_tensor_op::init_of(fill, total));
        let tiles = tiles_of(n, target);
        let buf = self.t.buffer_deep(&format!("in_{name}"), 1, tiles, precision);
        let l = self.t.load_block_at(&format!("load_{name}"), id, first, vec![], 1, tiles, tiles_of(total, target), buf);
        self.loads.push((l, buf));
        buf
    }

    /// The accumulation-path scratch between two compute stages, in the accumulator the section's precision folds
    /// in: fp32 for a float section, an integer as wide as the datapath for an integer one.
    fn scratch(&mut self, name: &str, page_tiles: usize) -> BufferId {
        let p = AccPolicy::for_result(self.precision).interm_precision();
        self.t.scratch(name, page_tiles, p)
    }

    fn compute(
        &mut self,
        name: &str,
        iters: Vec<IterSpec>,
        slots: Vec<Slot>,
        out_map: ElemAccess,
        body: TensorBody,
        dst: BufferId,
    ) -> StageId {
        let ins: Vec<BufferId> = slots.iter().map(|s| s.buf).collect();
        let maps: Vec<ElemAccess> = slots.into_iter().map(|s| s.map).collect();
        self.t.stage(
            name,
            StageKind::Compute(ComputeOp::Generic {
                iters,
                ins: maps,
                out: out_map,
                body,
            }),
            ins,
            vec![dst],
        )
    }

    /// Wire every load to the first chain stage that reads it, then chain the stages, then the store.
    /// ★ A SECTION ALWAYS CALLS ITS RESULT AN `Output`. Whether it is really an activation handed to
    /// the next section, or the carried state of the step, is not knowable from inside one section —
    /// it is a property of the WHOLE program, so `compose` decides it and a section lowered in
    /// isolation stays a program that validates on its own.
    fn finish(
        self,
        out_name: &str,
        n: usize,
        chain: Vec<StageId>,
        out_buf: BufferId,
        name: String,
        claim: NumericsClaim,
        target: &SpatialTarget,
    ) -> Result<SpatialProgram, SpatialReject> {
        let p = self.precision;
        self.finish_as(out_name, n, chain, out_buf, name, claim, p, target)
    }

    /// [`Self::finish`] with the result tensor at an explicit precision (an index, an integer row).
    #[allow(clippy::too_many_arguments)]
    fn finish_as(
        mut self,
        out_name: &str,
        n: usize,
        chain: Vec<StageId>,
        out_buf: BufferId,
        name: String,
        claim: NumericsClaim,
        out_precision: Precision,
        target: &SpatialTarget,
    ) -> Result<SpatialProgram, SpatialReject> {
        let cols_t = tiles_of(n, target);
        let y = self.t.tensor(
            out_name,
            TensorRole::Output,
            1,
            n,
            out_precision,
            TensorInit::Zero,
        );
        let st = match self.blocks {
            None => self.t.store(&format!("store_{out_name}"), y, vec![], cols_t, out_buf),
            Some(ColumnBlocks { blk, .. }) => self.t.store(&format!("store_{out_name}"), y, vec![(LV_ITEM_X, blk)], blk, out_buf),
        };
        self.wire_and_finish(chain, out_buf, st, cols_t, name, claim, target)
    }

    /// Wire every load to the first chain stage that reads it, chain the stages, drain the last into the
    /// store `st`, and finish over one whole-row work item `cols_t` tiles wide — the part of [`Self::finish`]
    /// shared by a section whose store is not the plain whole-row one (a scatter).
    #[allow(clippy::too_many_arguments)]
    fn wire_and_finish(
        self,
        chain: Vec<StageId>,
        out_buf: BufferId,
        st: StageId,
        cols_t: usize,
        name: String,
        claim: NumericsClaim,
        target: &SpatialTarget,
    ) -> Result<SpatialProgram, SpatialReject> {
        self.wire_and_finish_many(chain, &[(out_buf, st)], cols_t, name, claim, target)
    }

    /// [`Self::wire_and_finish`] for a section with SEVERAL results, each drained by its own store — every
    /// store fed by the chain stage that produces its buffer.
    fn wire_and_finish_many(
        mut self,
        chain: Vec<StageId>,
        stores: &[(BufferId, StageId)],
        cols_t: usize,
        name: String,
        claim: NumericsClaim,
        target: &SpatialTarget,
    ) -> Result<SpatialProgram, SpatialReject> {
        for (l, b) in std::mem::take(&mut self.loads) {
            let consumer = *chain.iter().find(|c| self.t.stage_reads(**c, b)).ok_or(
                SpatialReject::NotLowered("an operand nothing in the chain reads"),
            )?;
            self.t.edge(l, b, consumer);
        }
        for i in 0..chain.len() {
            let b = self.t.stage_output(chain[i]);
            if let Some(next) = chain
                .iter()
                .skip(i + 1)
                .find(|c| self.t.stage_reads(**c, b))
            {
                self.t.edge(chain[i], b, *next);
            }
        }
        for (b, st) in stores {
            let producer = *chain
                .iter()
                .rev()
                .find(|c| self.t.stage_output(**c) == *b)
                .ok_or(SpatialReject::NotLowered(
                    "a stored buffer no stage of the chain produces",
                ))?;
            self.t.edge(producer, *b, *st);
        }
        let acc = AccPolicy::for_result(self.precision);
        let items = match self.blocks {
            None => whole_row(cols_t),
            Some(ColumnBlocks { blk, .. }) => row_items(cols_t, blk),
        };
        self.t.finish(name, items, acc, claim, target)
    }
}

/// The claim every float composition earns: the chain rounds at each stage boundary exactly where the
/// datapath does, and the reference is the VM's f64 arithmetic.
fn approx(what: &str, p: Precision) -> NumericsClaim {
    NumericsClaim::Approx {
        reason: format!("{what} in {p:?}"),
    }
}

/// What a map's numbers promise: exact when it computes integers with integer arithmetic — an int8 model's
/// requant, saturation and ReLU, evaluated in `i64` — and [`approx`] in the float precision otherwise.
fn claim_of(what: &str, body: &TensorBody, p: Precision) -> NumericsClaim {
    if precision_int_bits(p).is_some() && body.integral() {
        NumericsClaim::Exact
    } else {
        approx(what, p)
    }
}

// ── The element-wise family: one `Generic`, one body ──────────────────────────────────────────────

/// A unary map `y = f(x)` over a `rows × cols` tensor: a row in the row form below, a matrix tile by tile.
fn unary_map(
    name: &str,
    kind: &str,
    datapath: Datapath,
    body: impl Fn(Box<TensorBody>) -> Box<TensorBody>,
    x: (&str, &Fill),
    out: &str,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, n) = shape;
    if rows > 1 {
        return matrix_unary_map(name, kind, datapath, body, x, out, shape, target);
    }
    let cols_t = tiles_of(n, target);
    if cols_t > row_block_tiles(target) {
        return blocked_unary_map(name, kind, datapath, body, x, out, n, target);
    }
    let mut b = FlatBuild::on(datapath, target)?;
    let xin = b.input(x.0, n, x.1, target);
    let out_buf = b.t.buffer("out", cols_t, b.precision);
    let w = wide(cols_t, target);
    let s = b.compute(
        kind,
        flat_space(n),
        vec![Slot {
            buf: xin,
            map: at(0),
        }],
        ident_map(w),
        *body(op(0)),
        out_buf,
    );
    let claim = claim_of(kind, &body(op(0)), b.precision);
    b.finish(
        out,
        n,
        vec![s],
        out_buf,
        format!("{name}_{n}"),
        claim,
        target,
    )
}

/// `y = body(a, b)` over a `1 × n` row a site holds whole: one item over the row, both operands loaded whole and mapped
/// together — the form [`unary_map`] gives a row, for an element-wise op of two rows. The caller has chosen the
/// precision and the claim (the element-wise lowering, from its operands' fills).
#[allow(clippy::too_many_arguments)]
pub(crate) fn row_binary_map(
    name: String,
    kind: &str,
    body: TensorBody,
    a: (&str, &Fill),
    b: (&str, &Fill),
    out: &str,
    n: usize,
    precision: Precision,
    claim: NumericsClaim,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let cols_t = tiles_of(n, target);
    let mut fb = FlatBuild::at(precision, target)?;
    let ain = fb.input(a.0, n, a.1, target);
    let bin = fb.input(b.0, n, b.1, target);
    let out_buf = fb.t.buffer("out", cols_t, precision);
    let w = wide(cols_t, target);
    let s = fb.compute(
        kind,
        flat_space(n),
        vec![Slot { buf: ain, map: at(0) }, Slot { buf: bin, map: at(0) }],
        ident_map(w),
        body,
        out_buf,
    );
    fb.finish(out, n, vec![s], out_buf, name, claim, target)
}

/// How many tiles of one row a single site may hold resident. A flat section keeps its whole row in
/// local memory, which is free while the row is a handful of tiles and impossible once it is not: at
/// Qwen3.8's real `d = 5120` a row is 160 tiles against an accumulator budget of eight, and the
/// whole-row form is refused as `AccTilesExceeded` long before local memory is consulted.
pub fn row_needs_blocking(cols: usize, target: &SpatialTarget) -> bool {
    tiles_of(cols, target) > row_block_tiles(target)
}

pub(crate) fn row_block_tiles(target: &SpatialTarget) -> usize {
    target.effective_acc_tiles(AccPolicy::F32, true).max(1)
}

/// ★ A POINTWISE MAP HAS NO CROSS-ELEMENT DEPENDENCE, so a row wider than a site can hold is simply
/// WORK, not a problem: one item per tile, a one-tile page, and the row streams. This is the same shape
/// `from_tensor_op::from_elementwise` has always had.
///
/// The whole-row form above stays for narrow rows on purpose — see `whole_row`: `place` clones the
/// program per site, so splitting a two-tile vector into two items buys nothing and doubles the stages.
/// The rule is therefore about the ROW, not about the operation: one item while the row fits the site's
/// budget, one item per tile once it does not.
#[allow(clippy::too_many_arguments)]
fn blocked_unary_map(
    name: &str,
    kind: &str,
    datapath: Datapath,
    body: impl Fn(Box<TensorBody>) -> Box<TensorBody>,
    x: (&str, &Fill),
    out: &str,
    n: usize,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let precision = datapath.precision(target)?;
    let cols_t = tiles_of(n, target);
    let mut t = Template::new(target)?;
    let xt = t.tensor(
        x.0,
        TensorRole::Input,
        1,
        n,
        precision,
        super::from_tensor_op::init_of(x.1, n),
    );
    let yt = t.tensor(out, TensorRole::Output, 1, n, precision, TensorInit::Zero);
    let xin = t.buffer("in_x", 1, precision);
    let out_buf = t.buffer("out", 1, precision);
    let lx = t.load("load_x", xt, vec![(LV_ITEM_X, 1)], 1, xin);
    let w = wide(1, target);
    let s = t.stage(
        kind,
        StageKind::Compute(ComputeOp::Generic {
            iters: flat_space(w),
            ins: vec![ident_map(w)],
            out: ident_map(w),
            body: *body(op(0)),
        }),
        vec![xin],
        vec![out_buf],
    );
    let st = t.store(
        &format!("store_{out}"),
        yt,
        vec![(LV_ITEM_X, 1)],
        1,
        out_buf,
    );
    t.edge(lx, xin, s);
    t.edge(s, out_buf, st);
    t.finish(
        format!("{name}_{n}"),
        items_per_tile(cols_t),
        AccPolicy::for_result(precision),
        claim_of(kind, &body(op(0)), precision),
        target,
    )
}

/// A unary map over a MATRIX: the tensors keep the program's `rows × cols`, and every tile of the 2-D tile grid
/// is one work item mapped over its full `tile × tile` page — the tiling a matmul reads the same tensor in, so a
/// composition joins the two without a layout conversion.
#[allow(clippy::too_many_arguments)]
fn matrix_unary_map(
    name: &str,
    kind: &str,
    datapath: Datapath,
    body: impl Fn(Box<TensorBody>) -> Box<TensorBody>,
    x: (&str, &Fill),
    out: &str,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols) = shape;
    let precision = datapath.precision(target)?;
    let mut t = Template::new(target)?;
    let xt = t.tensor(
        x.0,
        TensorRole::Input,
        rows,
        cols,
        precision,
        super::from_tensor_op::init_of(x.1, rows * cols),
    );
    let yt = t.tensor(
        out,
        TensorRole::Output,
        rows,
        cols,
        precision,
        TensorInit::Zero,
    );
    let (rows_t, cols_t) = (
        rows.div_ceil(target.tile.rows),
        cols.div_ceil(target.tile.cols),
    );
    let steps = super::from_tensor_op::tile_grid_steps(rows_t, cols_t);
    let xin = t.buffer("in_x", 1, precision);
    let out_buf = t.buffer("out", 1, precision);
    let lx = t.load("load_x", xt, steps.clone(), 1, xin);
    let w = target.tile.cols;
    let s = t.stage(
        kind,
        StageKind::Compute(ComputeOp::Generic {
            iters: super::from_tensor_op::space(target, w, None),
            ins: vec![ident_map(w)],
            out: ident_map(w),
            body: *body(op(0)),
        }),
        vec![xin],
        vec![out_buf],
    );
    let st = t.store(&format!("store_{out}"), yt, steps, 1, out_buf);
    t.edge(lx, xin, s);
    t.edge(s, out_buf, st);
    t.finish(
        format!("{name}_{rows}x{cols}"),
        super::from_tensor_op::items_per_tile_grid(rows_t, cols_t),
        AccPolicy::for_result(precision),
        claim_of(kind, &body(op(0)), precision),
        target,
    )
}

/// `y = silu(x)` — `x / (1 + e^-x)`, one fused SFPU kind.
pub fn from_silu(
    spec: &SiluSpec,
    names: &StageNames,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    unary_map(
        "silu",
        "silu",
        Datapath::Float,
        |x| silu_expr(x),
        names.operand(0),
        &names.out,
        pointwise_shape(shape, spec.n),
        target,
    )
}

/// `y = (x · M) >> S` — an int8 model's requant: the exact product, then an arithmetic shift.
pub fn from_int_requant(
    spec: &IntRequantSpec,
    names: &StageNames,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (m, s) = (spec.m as f64, spec.s);
    unary_map(
        "int_requant",
        "int_requant",
        Datapath::Int,
        |x| Box::new(TensorBody::Shr { arg: mul(x, konst(m)), by: s }),
        names.operand(0),
        &names.out,
        pointwise_shape(shape, spec.n),
        target,
    )
}

/// `y = min(max(x, LO), HI)` — saturation into `[LO, HI]`.
pub fn from_int_clamp(
    spec: &IntClampSpec,
    names: &StageNames,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (lo, hi) = (spec.lo as f64, spec.hi as f64);
    unary_map(
        "int_clamp",
        "int_clamp",
        Datapath::Int,
        |x| Box::new(TensorBody::Min(Box::new(TensorBody::Max(x, konst(lo))), konst(hi))),
        names.operand(0),
        &names.out,
        pointwise_shape(shape, spec.n),
        target,
    )
}

/// `y = max(x, 0)`, in the tensor's element kind.
pub fn from_relu(
    spec: &ReluSpec,
    names: &StageNames,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    unary_map(
        "relu",
        "relu",
        if spec.int { Datapath::Int } else { Datapath::Float },
        |x| Box::new(TensorBody::Max(x, konst(0.0))),
        names.operand(0),
        &names.out,
        pointwise_shape(shape, spec.n),
        target,
    )
}

/// The shape a pointwise op maps over: the program's `rows × cols` when it accounts for the `n` elements the
/// recognizer counted, a `1 × n` row otherwise.
fn pointwise_shape(shape: (usize, usize), n: usize) -> (usize, usize) {
    if shape.0 * shape.1 == n {
        shape
    } else {
        (1, n)
    }
}

/// `y = sigmoid(x)` — `1/(1 + e^-x)`, the reciprocal of the silu denominator. No new SFPU kind: `Exp`
/// and `Recip` already exist and the composition of the two IS the sigmoid.
pub fn from_sigmoid(
    spec: &SigmoidSpec,
    names: &StageNames,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    unary_map(
        "sigmoid",
        "sigmoid",
        Datapath::Float,
        |x| {
            unary(
                SfpuKind::Recip,
                add(konst(1.0), unary(SfpuKind::Exp, sub(konst(0.0), x))),
            )
        },
        names.operand(0),
        &names.out,
        pointwise_shape(shape, spec.n),
        target,
    )
}

/// `y = gelu(x)` — ggml's tanh approximation in its one-exponential form `x / (1 + e^-2z)` with
/// `z = √(2/π)·(x + 0.044715·x³)`, exactly as the parser desugars it (same arrangement, same tails).
pub fn from_gelu(
    spec: &GeluSpec,
    names: &StageNames,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    const SQRT_2_OVER_PI: f64 = 0.797_884_560_802_865_4;
    const K: f64 = 0.044_715;
    unary_map(
        "gelu",
        "gelu",
        Datapath::Float,
        |x| {
            let cube = mul(x.clone(), mul(x.clone(), x.clone()));
            let z = mul(konst(SQRT_2_OVER_PI), add(x.clone(), mul(konst(K), cube)));
            let two_z = mul(konst(2.0), z);
            mul(
                x,
                unary(
                    SfpuKind::Recip,
                    add(konst(1.0), unary(SfpuKind::Exp, sub(konst(0.0), two_z))),
                ),
            )
        },
        names.operand(0),
        &names.out,
        pointwise_shape(shape, spec.n),
        target,
    )
}

/// `y = x·C` — one multiply by the program's constant.
pub fn from_scale(
    spec: &ScaleSpec,
    names: &StageNames,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let c = spec.factor.get();
    unary_map(
        "scale",
        "scale",
        Datapath::Float,
        |x| mul(x, konst(c)),
        names.operand(0),
        &names.out,
        pointwise_shape(shape, spec.n),
        target,
    )
}

/// `y = C·tanh(x/C)` — each element soft-capped (`capped`'s form: one exponential and one reciprocal).
pub fn from_softcap(
    spec: &SoftcapSpec,
    names: &StageNames,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let c = spec.cap.get();
    unary_map(
        "softcap",
        "softcap",
        Datapath::Float,
        |x| capped(x, c),
        names.operand(0),
        &names.out,
        pointwise_shape(shape, spec.n),
        target,
    )
}

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
fn dense_decay(
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
        super::from_tensor_op::init_of(ar_fill, spec.n),
    );
    let bias = t.tensor(
        dt_name,
        TensorRole::Input,
        1,
        heads,
        dt,
        super::from_tensor_op::init_of(dt_fill, heads),
    );
    let rate = t.tensor(
        al_name,
        TensorRole::Input,
        1,
        heads,
        dt,
        super::from_tensor_op::init_of(al_fill, heads),
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
            iters: super::from_tensor_op::space(target, tw, None),
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

/// `Y = X matmul W across H heads` — the weighted-sum shape: the folded axis is the ROW (`e` over a head's `a` inputs),
/// the flat output column `c = u·b + r` for unit `u` (row `u / H`, head `h = u mod H`), and each unit's band reads
/// `X[u·a + e]` and `W[(h·a + e)·b + c − u·b]`.
pub fn from_head_matmul(
    spec: &HeadMatmulSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (m, heads, a, bw) = (spec.m, spec.heads, spec.a, spec.b);
    let (units, n) = (m * heads, m * heads * bw);
    if n == 0 || a == 0 {
        return Err(SpatialReject::NotLowered("a per-head matmul of nothing"));
    }
    if m == 1 && whole_tiles(&[a, bw], target) {
        return tile_head_matmul(spec, names, target);
    }
    let mut f = FlatBuild::new(target)?;
    let x = f.input(names.operand(0).0, m * heads * a, names.operand(0).1, target);
    let w = f.input(names.operand(1).0, heads * a * bw, names.operand(1).1, target);
    let cols_t = tiles_of(n, target);
    let out_buf = f.t.buffer("out", cols_t, f.precision);
    let tile_elems = target.tile.rows * target.tile.cols;
    let ones = f.t.tensor(
        "hm_scaler",
        TensorRole::Param,
        target.tile.rows,
        target.tile.cols,
        f.precision,
        TensorInit::FloatValues(vec![1.0f64.to_bits(); tile_elems]),
    );
    let ones_in = f.t.buffer("in_scaler", 1, f.precision);
    let ls = f.t.load("load_scaler", ones, vec![], 1, ones_in);
    f.loads.push((ls, ones_in));
    let iters = vec![
        IterSpec {
            var: LV_GEN_ROW,
            extent: a,
            kind: IterKind::Reduce { pool: ReducePool::Sum },
        },
        IterSpec {
            var: LV_GEN_COL,
            extent: n,
            kind: IterKind::Parallel,
        },
    ];
    let mut slots = vec![Slot { buf: ones_in, map: const_map() }];
    let mut bands = Vec::with_capacity(units);
    for u in 0..units {
        let h = u % heads;
        slots.push(Slot {
            buf: x,
            map: ElemAccess { base: (u * a) as i64, terms: vec![(LV_GEN_ROW, 1)], indirect: None },
        });
        let x_slot = slots.len() - 1;
        slots.push(Slot {
            buf: w,
            map: ElemAccess {
                base: (h * a * bw) as i64 - (u * bw) as i64,
                terms: vec![(LV_GEN_ROW, bw as i64), (LV_GEN_COL, 1)],
                indirect: None,
            },
        });
        let w_slot = slots.len() - 1;
        bands.push(((u + 1) * bw, *mul(op(x_slot), op(w_slot))));
    }
    let s = f.compute("head_matmul", iters, slots, per_col_map(), banded(bands), out_buf);
    let claim = approx("per-head matmul", f.precision);
    f.finish(&names.out, n, vec![s], out_buf, format!("head_matmul_{m}x{heads}x{a}x{bw}"), claim, target)
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
fn whole_tiles(widths: &[usize], target: &SpatialTarget) -> bool {
    widths.iter().all(|w| w % target.tile.cols == 0)
}

/// The items of a flat row of `groups` runs of `run_t` tiles, `blk` tiles an item: item `(g, j)` owns tiles
/// `[g·run_t + j·blk, g·run_t + (j+1)·blk)`.
fn grouped_block_items(groups: usize, run_t: usize, blk: usize) -> Vec<WorkItem> {
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
fn tile_head_slice(
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
    let x = t.tensor(x_name, TensorRole::Input, 1, spec.n, precision, super::from_tensor_op::init_of(x_fill, spec.n));
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
fn tile_join(
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
    let a = t.tensor(a_name, TensorRole::Input, 1, units * ha, precision, super::from_tensor_op::init_of(a_fill, units * ha));
    let b = t.tensor(b_name, TensorRole::Input, 1, units * hb, precision, super::from_tensor_op::init_of(b_fill, units * hb));
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

/// `Y = X matmul W across H heads` for one row at whole-tile widths: item `(h, o)` is output tile `o` of head `h`,
/// `Σ_e X[h·a + e]·W[h·a + e, o·32 + c]` — head `h`'s `a/32` input tiles against the `a/32 × 1` tile column `o` of
/// its rows of `W`, which `W`'s declaration as the `(H·a) × b` matrix it is lays out as tiles. Needs a square tile: a
/// weight tile's rows are the input's elements. Only the head's rows of
/// one tile column move an item, so `W` is never on chip whole.
fn tile_head_matmul(
    spec: &HeadMatmulSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (heads, a, bw) = (spec.heads, spec.a, spec.b);
    let (tr, tw) = (target.tile.rows, target.tile.cols);
    if tr != tw {
        return Err(SpatialReject::NotLowered("a per-head matmul in tiles on a tile that is not square — a weight tile's rows are the input's elements"));
    }
    let (a_t, b_t) = (a / tw, bw / tw);
    let precision = elem_precision(target)?;
    let (x_name, x_fill) = names.operand(0);
    let (w_name, w_fill) = names.operand(1);
    let mut t = Template::new(target)?;
    let scaler = t.tensor("hm_scaler", TensorRole::Param, tr, tw, precision, TensorInit::FloatValues(vec![1.0f64.to_bits(); tr * tw]));
    let x = t.tensor(x_name, TensorRole::Input, 1, heads * a, precision, super::from_tensor_op::init_of(x_fill, heads * a));
    let w = t.tensor(w_name, TensorRole::Param, heads * a, bw, precision, super::from_tensor_op::init_of(w_fill, heads * a * bw));
    let y = t.tensor(&names.out, TensorRole::Output, 1, heads * bw, precision, TensorInit::Zero);
    let sc_in = t.buffer_deep("in_scaler", 1, 1, precision);
    let x_in = t.buffer("in_x", a_t, precision);
    let w_in = t.buffer("in_w", a_t, precision);
    let out = t.buffer("out", 1, precision);
    let l_sc = t.load_block_at("load_scaler", scaler, 0, Vec::new(), 1, 1, 1, sc_in);
    let l_x = t.load_block_at("load_x", x, 0, vec![(LV_ITEM_Y, a_t)], 1, a_t, heads * a_t, x_in);
    let l_w = t.load_block_at("load_w", w, 0, vec![(LV_ITEM_Y, a_t * b_t), (LV_ITEM_X, 1)], a_t, 1, b_t, w_in);
    // ★ A PAGE IS ONE PLANE OF ITS TILES SIDE BY SIDE, so the weight's `a/32` stacked tiles lie as tile `k` at columns
    // `[32k, 32k + 32)`: input `e = 32k + r`, output `c` is at row `r`, column `32k + c`, and the input row's `e` at
    // column `32k + r` of row 0. The fold runs over both: the tile's rows `r` and the tiles `k`.
    let s = t.stage(
        "head_dot",
        StageKind::Compute(ComputeOp::Generic {
            iters: vec![
                IterSpec { var: LV_GEN_ROW, extent: tr, kind: IterKind::Reduce { pool: ReducePool::Sum } },
                IterSpec { var: LV_GEN_COL, extent: tw, kind: IterKind::Parallel },
                IterSpec { var: LV_GEN_K, extent: a_t, kind: IterKind::Reduce { pool: ReducePool::Sum } },
            ],
            ins: vec![
                ElemAccess { base: 0, terms: vec![(LV_GEN_ROW, (a_t * tw) as i64), (LV_GEN_K, tw as i64), (LV_GEN_COL, 1)], indirect: None },
                ElemAccess { base: 0, terms: vec![(LV_GEN_K, tw as i64), (LV_GEN_ROW, 1)], indirect: None },
                const_map(),
            ],
            out: per_col_map(),
            body: *mul(op(0), op(1)),
        }),
        vec![w_in, x_in, sc_in],
        vec![out],
    );
    let st = t.store_block(&format!("store_{}", names.out), y, 0, vec![(LV_ITEM_Y, b_t), (LV_ITEM_X, 1)], 1, 1, heads * b_t, out);
    for (from, buf, to) in [(l_w, w_in, s), (l_x, x_in, s), (l_sc, sc_in, s), (s, out, st)] {
        t.edge(from, buf, to);
    }
    let prog = t.finish(
        format!("head_matmul_1x{heads}x{a}x{bw}_tiled"),
        grouped_block_items(heads, b_t, 1),
        AccPolicy::F32,
        approx("per-head matmul", precision),
        target,
    )?;
    if site_footprint(&prog) > target.max_local_mem_bytes() {
        return Err(SpatialReject::NotLowered("a per-head matmul whose head's inputs and weight column do not fit a site"));
    }
    Ok(prog)
}

// ── The per-head family: a reduction, then a banded rescale ───────────────────────────────────────

/// The chain every per-row normalization shares: square → fold the row → transform the row scalar.
/// Returns `(chain, scale_buffer, scale_width)`; the caller adds the banded rescale that consumes it.
fn row_scale_chain(
    b: &mut FlatBuild,
    xin: BufferId,
    rows: usize,
    cols: usize,
    xform: SfpuKind,
    target: &SpatialTarget,
) -> (Vec<StageId>, BufferId, usize) {
    let xw = wide(b.t.page_tiles(xin), target);
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
    let xsq = b.scratch("xsq", sq_tiles);
    // The square reads the FLAT input through a 2-D space (`row` is the head) and writes the
    // `rows × cols` plane the reduction folds.
    let s1 = b.compute(
        "square",
        head_space(rows, cols, None),
        vec![
            Slot {
                buf: xin,
                map: map2(0, cols as i64, 1),
            },
            Slot {
                buf: xin,
                map: map2(0, cols as i64, 1),
            },
        ],
        ident_map(sqw),
        *mul(op(0), op(1)),
        xsq,
    );
    let ssum = b.scratch("row_sum", 1);
    let s2 = b.compute(
        "reduce_rows",
        head_space(rows, cols, Some(ReducePool::Sum)),
        vec![
            Slot {
                buf: xsq,
                map: map2(0, sqw as i64, 1),
            },
            Slot {
                buf: ones_in,
                map: const_map(),
            },
        ],
        per_row_map(wide(1, target)),
        TensorBody::Operand(0),
        ssum,
    );
    let one_w = wide(1, target);
    let scale = b.scratch("row_scale", 1);
    let s3 = b.compute(
        "row_xform",
        head_space(rows, target.tile.cols, None),
        vec![Slot {
            buf: ssum,
            map: per_row_map(one_w),
        }],
        per_row_map(one_w),
        *unary(xform, op(0)),
        scale,
    );
    let _ = xw;
    (vec![s1, s2, s3], scale, one_w)
}

/// `y[r, i] = x[r, i] / sqrt(Σ_i x[r, i]² + 1e-6)` over `rows` heads of `cols` — per-head L2
/// normalization on a FLAT `1 × rows·cols` tensor.
pub fn from_l2norm(
    spec: &L2NormSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols) = (spec.rows, spec.cols);
    let n = rows * cols;
    if row_needs_blocking(n, target) && !row_needs_blocking(cols, target) {
        return blocked_l2norm(spec, names, target);
    }
    let mut b = FlatBuild::new(target)?;
    let xin = b.input(names.operand(0).0, n, names.operand(0).1, target);
    let cols_t = tiles_of(n, target);
    let out_buf = b.t.buffer("out", cols_t, b.precision);
    let ow = wide(cols_t, target);
    // `over: 1` — l2norm divides by the root SUM of squares, not the root mean.
    let (mut chain, scale, one_w) = row_scale_chain(
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
        slots.push(Slot {
            buf: scale,
            map: fixed((h * one_w) as i64),
        });
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

/// The COMPOSITION-LAYOUT twin of [`super::from_tensor_op::from_fused_row`]'s RMSNorm: the same chain
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
    if row_needs_blocking(n, target)
        && !row_needs_blocking(cols, target)
        && spec.residual.is_none()
        && spec.bias.is_none()
    {
        return blocked_head_rms(spec, names, target);
    }
    let mut b = FlatBuild::new(target)?;
    let xin = b.input(names.operand(0).0, n, names.operand(0).1, target);
    // The spec's operand order is `[x, weight?, residual?, bias?]`.
    let mut rest = 1usize;
    let w_in = spec.weight.map(|_| {
        let i = rest;
        rest += 1;
        b.input(names.operand(i).0, cols, names.operand(i).1, target)
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
    let (mut norm_chain, scale, one_w) = row_scale_chain(
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
        slots.push(Slot {
            buf: scale,
            map: fixed((h * one_w) as i64),
        });
        let scale_slot = slots.len() - 1;
        let value = match w_in {
            Some(wb) => {
                slots.push(Slot {
                    buf: wb,
                    map: at(-head_base),
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
fn flat_softmax_rows(
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
fn flat_softmax_rows_at(
    input: (&str, &Fill),
    out_name: &str,
    rows: usize,
    cols: usize,
    label: String,
    precision: Precision,
    sinks: Option<(&str, &Fill)>,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let mut f = FlatBuild::new(target)?;
    f.precision = precision;
    let (h, l) = (rows, cols);
    let n = h * l;
    let sc = f.input(input.0, n, input.1, target);
    let cols_t = tiles_of(n, target);
    let out_buf = f.t.buffer("probs", cols_t, f.precision);
    let ow = wide(cols_t, target);
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
    let one_w = wide(1, target);
    // max over the head's row, then Σ exp(x − max), then exp(x − max)/Σ.
    let rmax = f.scratch("row_max", 1);
    let s1 = f.compute(
        "row_max",
        head_space(h, l, Some(ReducePool::Max)),
        vec![
            Slot {
                buf: sc,
                map: map2(0, l as i64, 1),
            },
            Slot {
                buf: ones_in,
                map: const_map(),
            },
        ],
        per_row_map(one_w),
        TensorBody::Operand(0),
        rmax,
    );
    // A row's sink: element `r` of a flat `1 × rows` tensor.
    let sink_map = ElemAccess {
        base: 0,
        terms: vec![(LV_GEN_ROW, 1)],
        indirect: None,
    };
    let sink = sinks.map(|(name, fill)| f.input(name, h, fill, target));
    let mut sink_stages = Vec::new();
    let rmax = match sink {
        Some(sk) => {
            let with_sink = f.scratch("row_max_sink", 1);
            sink_stages.push(f.compute(
                "row_max_sink",
                head_space(h, target.tile.cols, None),
                vec![
                    Slot {
                        buf: rmax,
                        map: per_row_map(one_w),
                    },
                    Slot {
                        buf: sk,
                        map: sink_map.clone(),
                    },
                ],
                per_row_map(one_w),
                TensorBody::Max(op(0), op(1)),
                with_sink,
            ));
            with_sink
        }
        None => rmax,
    };
    let ex = f.scratch("expx", tiles_of(l, target));
    let exw = wide(tiles_of(l, target), target);
    let s2 = f.compute(
        "exp_centred",
        head_space(h, l, None),
        vec![
            Slot {
                buf: sc,
                map: map2(0, l as i64, 1),
            },
            Slot {
                buf: rmax,
                map: per_row_map(one_w),
            },
        ],
        ident_map(exw),
        *unary(SfpuKind::Exp, sub(op(0), op(1))),
        ex,
    );
    let rsum = f.scratch("row_sum", 1);
    let s3 = f.compute(
        "row_sum",
        head_space(h, l, Some(ReducePool::Sum)),
        vec![
            Slot {
                buf: ex,
                map: map2(0, exw as i64, 1),
            },
            Slot {
                buf: ones_in,
                map: const_map(),
            },
        ],
        per_row_map(one_w),
        TensorBody::Operand(0),
        rsum,
    );
    let rsum = match sink {
        Some(sk) => {
            let with_sink = f.scratch("row_sum_sink", 1);
            sink_stages.push(f.compute(
                "row_sum_sink",
                head_space(h, target.tile.cols, None),
                vec![
                    Slot {
                        buf: rsum,
                        map: per_row_map(one_w),
                    },
                    Slot {
                        buf: sk,
                        map: sink_map,
                    },
                    Slot {
                        buf: rmax,
                        map: per_row_map(one_w),
                    },
                ],
                per_row_map(one_w),
                *add(op(0), unary(SfpuKind::Exp, sub(op(1), op(2)))),
                with_sink,
            ));
            with_sink
        }
        None => rsum,
    };
    let inv = f.scratch("row_inv", 1);
    let s4 = f.compute(
        "recip",
        head_space(h, target.tile.cols, None),
        vec![Slot {
            buf: rsum,
            map: per_row_map(one_w),
        }],
        per_row_map(one_w),
        *unary(SfpuKind::Recip, op(0)),
        inv,
    );
    // `ex` is the `h × l` plane: head `head`'s row `j` sits at `head·exw + j`, so the flat output
    // column `c = head·l + j` reads it at `c + head·(exw − l)` — one map per head, all affine.
    let mut slots: Vec<Slot> = Vec::new();
    let mut bands = Vec::new();
    for head in 0..h {
        slots.push(Slot {
            buf: ex,
            map: at((head * (exw - l)) as i64),
        });
        let e_slot = slots.len() - 1;
        slots.push(Slot {
            buf: inv,
            map: fixed((head * one_w) as i64),
        });
        let i_slot = slots.len() - 1;
        bands.push(((head + 1) * l, *mul(op(e_slot), op(i_slot))));
    }
    let s5 = f.compute(
        "scale",
        flat_space(n),
        slots,
        ident_map(ow),
        banded(bands),
        out_buf,
    );
    let claim = approx("row softmax", f.precision);
    let chain = match sink_stages.as_slice() {
        [max, sum] => vec![s1, *max, s2, s3, *sum, s4, s5],
        _ => vec![s1, s2, s3, s4, s5],
    };
    f.finish(
        out_name,
        n,
        chain,
        out_buf,
        label,
        claim,
        target,
    )
}

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
) -> Result<SpatialProgram, SpatialReject> {
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
    let table_rows = spec.from.map_or(rows, |(_, cache)| cache);
    if let Some((RowIndex::Literal(p), cache)) = spec.from {
        if p < 1 || p as usize + rows - 1 > cache {
            return Err(SpatialReject::NotLowered("a literal rope position whose block leaves the cache — the program fails at run time on every tier, so there is no section to lower"));
        }
    }
    // Head `h` turns by table row `h / per_row`: its reads of the tables start `row·half` in.
    let per_row = heads / rows;
    let row_shift = |h: usize| ((h / per_row) * half) as i64;
    let n = heads * hd;
    // ★ A ROTARY EMBEDDING NEVER CROSSES A HEAD, so the head is the natural work item. The flat form
    // keeps all `heads·hd` elements resident and tells heads apart with three bands EACH; at Qwen3.8's
    // attention width that is 192 tiles against an eight-tile budget and 72 bands. One head is `hd = 256`
    // — eight tiles, exactly the budget — and inside it the band structure collapses to the three regions
    // the rotation actually has, because the page IS the head.
    // A head is the item only when it is whole tiles; a head of 80 shares tiles with its neighbours, so the rotation is
    // the one flat stage, blocked by the result's columns past an accumulator page.
    if row_needs_blocking(n, target) && !row_needs_blocking(hd, target) && hd % target.tile.cols == 0 {
        return blocked_rope(spec, names, target);
    }
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
        from => {
            let n_table = table_rows * half;
            if from.is_some() && tiles_of(n_table, target) > row_block_tiles(target) {
                return Err(SpatialReject::NotLowered("a rope reading a cache whose rows are narrower than a tile loads the cache whole, and this cache is wider than a site's page"));
            }
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

/// The `rows` rows a rope block reads of a cache of `cache` positions whose rows are `half_t` whole tiles: ONE tile run
/// per table from tile `(P − 1)·half_t`, loaded into buffers the rope's compute reads as a table row per row. A literal
/// `P` is the load's base; a named one is loaded, decremented and checked once — a guard of stride zero keeping the
/// block's last row among the cache's, as a row index is — and handed to each table's load as an [`IndirectTile`]
/// through a FIFO copy of its own (a FIFO has one reader). Returns the cosine and sine buffers.
fn cache_block_loads(
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
                super::from_tensor_op::init_of(fill, n),
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
                super::from_tensor_op::init_of(p_fill, 1),
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
fn rope_name(spec: &RopeSpec) -> String {
    let rows = if spec.rows > 1 {
        format!("_rows{}", spec.rows)
    } else {
        String::new()
    };
    format!("rope_{}x{}_half{}{rows}", spec.h, spec.hd, spec.half)
}

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
fn ring_is_matrix(spec: &CausalConvSpec, target: &SpatialTarget) -> bool {
    if row_needs_blocking(spec.taps * spec.chan, target) {
        return true;
    }
    let step = CausalConvSpec { rows: 1, ..spec.clone() };
    let names = StageNames { out: "y".into(), ins: vec!["x".into(), "w".into(), "cs".into()], fills: vec![Fill::Unknown; 3] };
    flat_causal_conv(&step, &names, target).map_or(true, |sections| sections.iter().any(|p| site_footprint(p) > target.max_local_mem_bytes()))
}

/// The single step's conv over a flat ring: the ring update, then the reduction against the weight.
fn flat_causal_conv(
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
fn block_causal_conv(
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
            super::from_tensor_op::init_of(fill, taps * chan),
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
            super::from_tensor_op::init_of(x_fill, rows * chan),
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
                iters: super::from_tensor_op::space(target, wc_w, None),
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
            super::from_tensor_op::init_of(x_fill, rows * chan),
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
                super::from_tensor_op::space(target, wc_w, None),
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
                target,
            )?);
        }
    }
    Ok(sections)
}

/// Row `r` of [`from_gated_delta`]'s flat form — its prediction, its memory update and its output — reading the row's
/// operands in place: row `r` of `Q` and `K` from element `r·hk·d`, of `V` and `Z` from `r·hv·d`, of `B` and `A` from
/// `r·hv`. A one-row step is row 0 of itself.
fn flat_gated_delta_row(
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
        let (mut chain, scale, one_w) = row_scale_chain(
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
            slots.push(Slot {
                buf: scale,
                map: fixed((h * one_w) as i64),
            });
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

/// A section writing `out`, a flat `1 × n` tensor whose element `e` is the sink logit of query head `head_of(e)` (or
/// [`MASKED`] for `None`, a row no head fills): the per-row sinks a softmax over the heads' rows reads, from the
/// attention's one logit per head (`sinks`, `per_row` of them). Runs of elements with one head share a band.
fn spread_sinks(
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
    // is affine in its own coordinate; the cache streams in blocks.
    if attention_reads_dense(h, kv, hd, spec.vd, target) {
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
struct QueryHeads {
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
enum Bound {
    /// Fixed by the program, and checked against the cache when the attention is lowered.
    Literal(usize),
    /// Held by operand `i`, and checked at run time so every row stays inside the cache: `1 ≤ P ≤ rows − (T − 1)`.
    Operand(usize),
}

/// The cache an attention reads: `rows` positions declared, the first `attended` of them covered, and — masked —
/// the bound each query row's live positions end at, and the window of positions before it a row keeps.
#[derive(Clone, Copy, Debug)]
struct Cache {
    rows: usize,
    attended: usize,
    limit: Option<Bound>,
    window: Option<Window>,
}

/// The largest `P` a block of `q_rows` query rows admits over a cache of `rows`: its last row's positions all cached.
fn bound_max(rows: usize, q_rows: usize) -> usize {
    rows + 1 - q_rows
}

/// The widths of an attention's heads: `kv` K/V heads per position, keys `hd` and values `vd` wide, and the factor
/// every score takes.
#[derive(Clone, Copy, Debug)]
struct HeadWidths {
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
fn attention_sections(
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
        let mut f = FlatBuild::new(target)?;
        let q = f.input(q_name, h * hd, q_fill, target);
        let k = f.input(k_name, kv_len, k_fill, target);
        let n = h * l;
        let cols_t = tiles_of(n, target);
        let out_buf = f.t.buffer("scores", cols_t, f.precision);
        let ow = wide(cols_t, target);
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
                extent: n,
                kind: IterKind::Parallel,
            },
        ];
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
            _ => f.scratch("raw_scores", cols_t),
        };
        chain.push(f.compute(
            "attn_scores",
            iters,
            slots,
            per_col_map(),
            banded(bands),
            scored,
        ));
        // ★ A SOFT CAP takes every scaled score before the mask, so a masked score stays masked.
        let scored = match cap {
            None => scored,
            Some(c) => {
                let to = if cache.limit.is_some() { f.scratch("capped_scores", cols_t) } else { out_buf };
                chain.push(f.compute("attn_cap", flat_space(n), vec![Slot { buf: scored, map: at(0) }], ident_map(ow), *capped(op(0), c), to));
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
            chain.push(f.compute(
                "mask",
                flat_space(n),
                slots,
                ident_map(ow),
                banded(bands),
                out_buf,
            ));
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
        let mut f = FlatBuild::new(target)?;
        let p = f.input(&probs_name, h * l, &Fill::Unknown, target);
        let v = f.input(v_name, v_len, v_fill, target);
        let n = h * vd;
        let cols_t = tiles_of(n, target);
        let out_buf = f.t.buffer("out", cols_t, f.precision);
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
        let iters = vec![
            IterSpec {
                var: LV_GEN_ROW,
                extent: l,
                kind: IterKind::Reduce {
                    pool: ReducePool::Sum,
                },
            },
            IterSpec {
                var: LV_GEN_COL,
                extent: n,
                kind: IterKind::Parallel,
            },
        ];
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
        let s = f.compute(
            "attn_weighted",
            iters,
            slots,
            per_col_map(),
            banded(bands),
            out_buf,
        );
        let claim = approx("attention weighted sum", f.precision);
        f.finish(
            &names.out,
            n,
            vec![s],
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
    if full_attention_reads_dense(hd, target) {
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

/// The plan of a relayout: one item per `blk`-tile block of each of `tiles_r` dense tile rows.
fn relayout_plan(tiles_r: usize, row_t: usize, blk: usize, dt: Precision) -> BlockPlan {
    BlockPlan {
        grid: (tiles_r, row_t / blk),
        ..tile_row_plan(row_t, blk, dt)
    }
}

fn relayout_items(tiles_r: usize, row_t: usize, blk: usize) -> Vec<WorkItem> {
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
fn relayout_tiles(cols: usize, target: &SpatialTarget) -> bool {
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
            super::from_tensor_op::init_of(from.1, rows * cols),
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
                            iters: super::from_tensor_op::space(target, w, None),
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
                    iters: super::from_tensor_op::space(target, w, None),
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
fn relayout_dense_picks_to_rows(
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
        super::from_tensor_op::init_of(from.1, rows * cols),
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
fn relayout_small_dense_to_rows(
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
        super::from_tensor_op::init_of(from.1, rows * cols),
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
struct FlashGeometry {
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
fn flash_applies(hd: usize, vd: usize, target: &SpatialTarget) -> bool {
    hd % target.tile.cols == 0 && vd % target.tile.cols == 0 && hd > 0 && vd > 0
}

/// Whether an attention of `h` query heads over `kv` K/V heads, keys `hd` and values `vd` wide, reads its K and V as
/// DENSE tiles on `target` — the flash form's cache layout. A table an attention reads dense is written dense too, so
/// this is the one place the two agree.
pub fn attention_reads_dense(h: usize, kv: usize, hd: usize, vd: usize, target: &SpatialTarget) -> bool {
    flash_applies(hd, vd, target) && kv > 0 && h % kv == 0
}

/// [`attention_reads_dense`] for a full attention, whose query heads read K/V heads one to one.
pub fn full_attention_reads_dense(hd: usize, target: &SpatialTarget) -> bool {
    flash_applies(hd, hd, target)
}

/// The mask value a masked score takes: far below any real score, and still finite in every float format.
const MASKED: f64 = -1.0e30;

/// The fan-ins of a merge tree over `nb` partials, bottom level first: the fewest levels any fan-in up to `fmax`
/// allows, and at each level the SMALLEST fan-in that still reaches one partial in the levels left — so the tree
/// pads as little as its depth allows. Their product is the partial slot count, `≥ nb`.
fn merge_fans(nb: usize, fmax: usize) -> Vec<usize> {
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
fn checked_count(
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

fn flash_scaler_name(names: &StageNames) -> String {
    format!("{}__flash_scaler", names.out)
}

/// Bytes of local memory one site of a section holds: every buffer, at its declared depth.
fn site_footprint(p: &SpatialProgram) -> usize {
    p.buffers.iter().map(|b| p.buffer_bytes(b)).sum()
}

/// ★ TWO PAGES WHERE THEY FIT, ONE WHERE THEY DO NOT. A buffer an item fills or drains once holds a second page, so the
/// next item begins while this one's pages drain; where the second pages leave no room in a site (fp32 at a real
/// width), one page computes the same values, an item's pages draining before the next item's begin. `build(pages)`
/// is the section with that many pages in each such buffer.
fn paged(
    build: impl Fn(usize) -> Result<SpatialProgram, SpatialReject>,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let doubled = build(2)?;
    if site_footprint(&doubled) <= target.max_local_mem_bytes() {
        Ok(doubled)
    } else {
        build(1)
    }
}

/// The flash fold of an attention: ONE section, an item per query tile and head, the cache streaming past as its
/// K blocks.
/// A flash fold's query layout: the query rows it holds, the heads a row carries, the query tile rows, and its items.
fn fold_items(g: &FlashGeometry, target: &SpatialTarget) -> (usize, usize, usize, usize) {
    let tr = target.tile.rows;
    let (rows, per_row) = if g.grouped { (g.kvw * tr, 1) } else { (g.q.rows, g.q.per_row) };
    let tiles_q = rows.div_ceil(tr);
    (rows, per_row, tiles_q, tiles_q * per_row)
}

/// The tensor a flash fold reads its rows' sinks from: one tile per item, row `r`'s sink in element `r`.
fn flash_sinks_name(names: &StageNames) -> String {
    format!("{}__flash_sinks", names.out)
}

fn flash_fold(
    names: &StageNames,
    g: FlashGeometry,
    tag: &str,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    paged(|pages| flash_fold_paged(names, g, tag, target, pages), target)
}

/// [`flash_fold`] with `pages` pages in each streamed key and value page and in the output page.
fn flash_fold_paged(
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
    let width_t = per_row * head_t;
    // ★ THE VALUES FOLD AN ACCUMULATOR PAGE AT A TIME. A row's running output is `vd` wide; past a page it is `nv`
    // blocks of `vt` tiles, each its own carry, block product, update and normalization, all sharing one row's scores,
    // maximum and sum.
    let v_t = g.vd / tw;
    let vt = row_block(v_t, target);
    let (j0, nv) = g.v_part.unwrap_or((0, v_t / vt));
    // The fold's own part of a head's values: `nv` pages from page `j0`.
    let part_t = nv * vt;
    let out_width = per_row * part_t * tw;
    let out_width_t = per_row * part_t;
    let part = |name: &str, j: usize| if nv == 1 { name.to_string() } else { format!("{name}{j}") };
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
        super::from_tensor_op::init_of(q_fill, rows * width),
    );
    let k = t.tensor(
        k_name,
        TensorRole::Input,
        g.rows,
        g.kvw * g.hd,
        precision,
        super::from_tensor_op::init_of(k_fill, kv_len),
    );
    let v = t.tensor(
        v_name,
        TensorRole::Input,
        g.rows,
        g.kvw * g.vd,
        precision,
        super::from_tensor_op::init_of(v_fill, v_len),
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
    let q_in = t.buffer_deep("in_q", 1, head_t, precision);
    let l_sc = t.load_block_at("load_scaler", scaler, 0, Vec::new(), 1, 1, 1, sc_in);
    // Item `y` is tile row `y / per_row`, head `y mod per_row` of the dense queries: linear tile `y·head_t`.
    let l_q = t.load_block_at(
        "load_q",
        q,
        0,
        vec![(LV_ITEM_Y, head_t)],
        1,
        head_t,
        width_t,
        q_in,
    );
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
                            super::from_tensor_op::init_of(pf, 1),
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
            let sinks = t.tensor(
                &flash_sinks_name(names),
                TensorRole::Input,
                1,
                items * tw,
                precision,
                super::from_tensor_op::init_of(&Fill::Unknown, items * tw),
            );
            let sk_in = t.buffer_deep("in_sinks", 1, 1, precision);
            let l_sk = t.load_block_at("load_sinks", sinks, 0, vec![(LV_ITEM_Y, 1)], 1, 1, 1, sk_in);
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
                head_space(tr, vt * tw, None),
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
    let row_stride = g.kvw * head_t;
    let v_row_stride = g.kvw * v_t;
    let k_in = t.buffer_deep("in_k", pages, head_t, precision);
    let v_ins: Vec<BufferId> = (0..nv).map(|j| t.buffer_deep(&part("in_v", j), pages, vt, precision)).collect();
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
    let l_k = cache_block(&mut t, "load_k", k, (0, head_t, head_t, row_stride), k_in);
    let l_vs: Vec<StageId> = (0..nv)
        .map(|j| cache_block(&mut t, &part("load_v", j), v, ((j0 + j) * vt, vt, v_t, v_row_stride), v_ins[j]))
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
    let hd = g.hd as i64;
    // `S[r, j] = Σ_e Q[r, e]·K[j, e]/√hd` — the tile's rows against the block's positions, one contraction.
    let s_buf = t.scratch_deep("scores", 1, 1, acc);
    let s_scores = t.stage(
        "scores",
        generic(
            contract(tr, tr, g.hd),
            vec![
                m2(&[(LV_GEN_ROW, hd), (LV_GEN_K, 1)]),
                m2(&[(LV_GEN_COL, hd), (LV_GEN_K, 1)]),
            ],
            ident_map(w1),
            *mul(mul(op(0), op(1)), konst(scale)),
        ),
        vec![q_in, k_in],
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
    // `O_b[r, e] = Σ_j P[r, j]·V[j, e]` — one contraction over the block's positions, a value page at a time.
    let vw = (vt * tw) as i64;
    let o_blks: Vec<(StageId, BufferId)> = (0..nv)
        .map(|j| {
            let o_blk = t.scratch_deep(&part("block_out", j), 1, vt, acc);
            let s_oblk = t.stage(
                &part("block_out", j),
                generic(
                    contract(tr, vt * tw, tr),
                    vec![
                        m2(&[(LV_GEN_ROW, w1 as i64), (LV_GEN_K, 1)]),
                        m2(&[(LV_GEN_K, vw), (LV_GEN_COL, 1)]),
                    ],
                    ident_map(wd),
                    *mul(op(0), op(1)),
                ),
                vec![p_buf, v_ins[j]],
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
                head_space(tr, vt * tw, None),
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
                head_space(tr, vt * tw, None),
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
    for j in 0..nv {
        t.edge(l_vs[j], v_ins[j], o_blks[j].0);
    }
    for (s_norm, out, st) in epilogue {
        t.edge(s_norm, out, st);
    }
    let work: Vec<WorkItem> = (0..items)
        .map(|y| {
            let (qt, h) = (y / per_row, y % per_row);
            let kv_head = if g.grouped { y } else { g.q.kv_of(h) };
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
fn flash_attention(
    names: &StageNames,
    g: FlashGeometry,
    tag: &str,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    if target.tile.rows != target.tile.cols {
        return Err(SpatialReject::NotLowered("a flash attention fold on a tile that is not square — a block's positions are its scores' columns"));
    }
    let p = flash_fold(names, g, tag, target)?;
    if site_footprint(&p) > target.max_local_mem_bytes() {
        return value_pages(names, g, tag, target);
    }
    // ★ A SINK IS THE RUNNING SOFTMAX'S FIRST TERM. Each fold row starts from its head's sink logit as its maximum, with a
    // sum of one and no value, so every block's update carries the sink's term to the end. The sinks are first spread
    // one tile per item, a row's sink in its own column ([`flash_sinks_name`]).
    let Some(si) = g.sinks else { return Ok(vec![p]) };
    let (tw, (_, per_row, _, items)) = (target.tile.cols, fold_items(&g, target));
    let head_of = |e: usize| {
        let (y, row) = (e / tw, e % tw);
        if g.grouped {
            (row < g.q.group).then_some(y * g.q.group + row)
        } else {
            Some(y % per_row)
        }
    };
    let spread = spread_sinks(names.operand(si), g.q.per_row, &flash_sinks_name(names), items * tw, &head_of, target)?;
    Ok(vec![spread, p])
}

/// A decode attention whose fold does not fit a site, as one fold per value page: fold `j` streams the keys and its
/// page of every head's values into `{out}__values{j}`, and tile joins lay the pages side by side per head into the
/// result. Each fold reads the keys again — the price of a site that cannot hold a head's every value page.
fn value_pages(
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
fn grouped_decode_attention(
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
    let head_t = hd / tw;
    let (heads_t, row_t) = (group * head_t, g.q.per_row * head_t);
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
                super::from_tensor_op::init_of(q_fill, g.q.per_row * hd),
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
    let ungroup = relayout_dense_picks_to_rows(
        (&grouped_ctx, &Fill::Unknown),
        &names.out,
        kv * tr,
        &picks,
        g.vd,
        precision,
        target,
    )?;

    let mut sections = vec![fits(regroup)?];
    sections.extend(fold);
    sections.push(fits(ungroup)?);
    Ok(sections)
}

/// ★ AN INDEX RIDES THE TARGET'S ADDRESS INTEGER, SIZED BY ITS INDEX SPACE. Never the float datapath — bf16
/// holds integers exactly only up to 256, and a Qwen3.8 token id reaches 248,319 — and never a width the chip
/// does not have: on an RDU every index is INT32, the widest integer the PCU computes in. `space` is the largest
/// magnitude the index arithmetic of the lowering can reach; a space past every integer the target has is
/// refused as the precision it would need (`MissingPrecision`), never wrapped.
pub(crate) fn index_type(target: &SpatialTarget, space: u64) -> Result<Precision, SpatialReject> {
    target
        .index_precision_for(space)
        .map_err(|needed| SpatialReject::MissingPrecision {
            needed,
            target: target.name,
        })
}

/// The largest magnitude a recognized INTEGER fill declares — `None` for a fill the compiler cannot see.
fn fill_abs_bound(fill: &Fill) -> Option<u64> {
    match fill {
        Fill::Const(c) => Some(c.unsigned_abs()),
        Fill::Pattern { modulus, shift } => Some(
            shift
                .unsigned_abs()
                .max((modulus - 1 + shift).unsigned_abs()),
        ),
        Fill::Overrides { base, sets } => Some(
            sets.iter()
                .map(|(_, v)| v.unsigned_abs())
                .fold(fill_abs_bound(base)?, u64::max),
        ),
        Fill::ConstF(_) | Fill::FloatOverrides { .. } | Fill::RealPattern { .. } | Fill::Unknown => None,
    }
}

/// The index space of an index OPERAND: the `span` the lowering's own arithmetic reaches for an in-range index,
/// widened by whatever value the operand's fill declares — a declared index past its table must still be HELD
/// exactly to be refused.
fn index_space(span: u64, per_unit: u64, operand_fill: &Fill) -> u64 {
    span.max(
        fill_abs_bound(operand_fill)
            .map_or(0, |v| v.saturating_add(1).saturating_mul(per_unit.max(1))),
    )
}

/// A table's element precision in a section: the float datapath for a Real table; for an Int one, the
/// address-wide integer that holds every value its fill declares (an undeclared fill is its producer's bytes,
/// read at the datapath's integer).
fn table_precision(
    b: &FlatBuild,
    float: bool,
    fill: &Fill,
    target: &SpatialTarget,
) -> Result<Precision, SpatialReject> {
    if float {
        Ok(b.precision)
    } else {
        index_type(target, fill_abs_bound(fill).unwrap_or(0))
    }
}

/// The element precision of a row WRITE: the table's, widened to hold the source row's declared values too — the
/// written table holds both.
fn row_write_precision(
    b: &FlatBuild,
    float: bool,
    table: &Fill,
    source: &Fill,
    target: &SpatialTarget,
) -> Result<Precision, SpatialReject> {
    if float {
        return Ok(b.precision);
    }
    index_type(
        target,
        fill_abs_bound(table)
            .unwrap_or(0)
            .max(fill_abs_bound(source).unwrap_or(0)),
    )
}

/// `lo = (index − 1)·cols` — the flat offset of a 1-based row, computed on the INTEGER datapath (at `ix`, the
/// index type) from a one-element index operand. Returns the stages and the buffer holding `lo`.
///
/// ★ THE MULTIPLY MAY NOT WRAP. On a 32-bit datapath `(index − 1)·cols` wraps once the index passes
/// `INT_MAX / cols`, and a wrapped offset can land inside the table — a silently wrong row. So the row is GUARDED
/// first, by a gather of stride zero: `index − 1 + B ∈ [0, 2B)` with `B = ⌊INT_MAX / cols⌋`. That range is exactly
/// `|index − 1| ≤ B` even where the addition itself wraps, and inside it the multiply cannot overflow. A row just
/// outside the table passes the guard and faults at the offset's own range check, which names the offset.
fn row_offset_stages(
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
fn literal_row_offset(index: i64, rows: usize, cols: usize) -> Result<i64, SpatialReject> {
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
fn tile_rows_gather(
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
        super::from_tensor_op::init_of(t_fill, tn),
    );
    let indices = t.tensor(
        i_name,
        TensorRole::Input,
        1,
        n,
        ix,
        super::from_tensor_op::init_of(i_fill, n),
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

/// How many tiles of a whole-tile row ONE work item moves: the largest divisor of the row's tiles the
/// accumulator holds, since the row passes through the tile datapath one accumulator page at a time. A row that
/// fits is one item; Qwen3.8's `d = 5120` (160 tiles) is twenty items of eight.
fn row_block(row_t: usize, target: &SpatialTarget) -> usize {
    (1..=row_t.min(row_block_tiles(target)))
        .rev()
        .find(|d| row_t % d == 0)
        .unwrap_or(1)
}

/// The address of item `i`'s `blk`-tile block of a whole-tile row of a flat `rows × (row_t · tile)` table. A
/// literal row is affine. A named row adds an [`IndirectTile`] over the 0-based row `index − 1`, computed per item
/// on the integer datapath (at the target's index type) into a FIFO the movement engine waits on and retires once
/// the address is resolved. Returns the address and the index path's `(stage, buffer)`.
fn row_block_address(
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
                super::from_tensor_op::init_of(operand.1, 1),
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
fn row_items(row_t: usize, blk: usize) -> Vec<WorkItem> {
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
fn copy_page(
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
fn tile_row_gather(
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
        super::from_tensor_op::init_of(t_fill, n),
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
fn tile_set_row(
    spec: &SetRowSpec,
    names: &StageNames,
    float: bool,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols, count) = (spec.rows, spec.cols, spec.count);
    let row_t = cols / target.tile.cols;
    // A block of `count` rows is `count·row_t` contiguous tiles from its first row's: the one-row scatter, wider, its
    // first row bounded so the last is the table's.
    let span_t = count * row_t;
    let blk = row_block(span_t, target);
    let n = rows * cols;
    let (t_name, t_fill) = names.operand(0);
    let (s_name, s_fill) = names.operand(1);
    let dt = if float {
        elem_precision(target)?
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
        super::from_tensor_op::init_of(s_fill, count * cols),
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
        super::from_tensor_op::init_of(t_fill, n),
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
struct DenseRow {
    /// One tile-row address per movement that touches the row, each with the index copy it waits on.
    tile_rows: Vec<(TileAddr, Option<(StageId, BufferId)>)>,
    in_tile: InTileRow,
}

/// The in-tile row of a dense row: known at compile time, or computed onto the integer datapath.
#[derive(Clone, Copy)]
enum InTileRow {
    Fixed(usize),
    Data {
        row: BufferId,
        stage: StageId,
        ix: Precision,
    },
}

/// The addresses of row `index` of a dense `rows`-row table whose rows are `row_t` tiles, for `movements` tile
/// moves of `steps` — every consumer of the index waits on its own copy of `r = i − 1`.
fn dense_row(
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
                super::from_tensor_op::init_of(i_fill, 1),
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

/// The plan of a section that moves one tile row, `row_t / blk` items across it.
fn tile_row_plan(row_t: usize, blk: usize, dt: Precision) -> BlockPlan {
    BlockPlan {
        per_site_m: 1,
        per_site_n: blk,
        grid: (1, row_t / blk),
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
fn dense_set_row(
    spec: &SetRowSpec,
    names: &StageNames,
    float: bool,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols) = (spec.rows, spec.cols);
    let row_t = cols.div_ceil(target.tile.cols);
    let blk = row_block(row_t, target);
    let n = rows * cols;
    let (t_name, t_fill) = names.operand(0);
    let (s_name, s_fill) = names.operand(1);
    let dt = if float {
        elem_precision(target)?
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
        super::from_tensor_op::init_of(s_fill, cols),
    );
    let table = t.tensor(
        t_name,
        TensorRole::State,
        rows,
        cols,
        dt,
        super::from_tensor_op::init_of(t_fill, n),
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
            iters: super::from_tensor_op::space(target, w, None),
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
fn dense_set_rows(
    spec: &SetRowSpec,
    names: &StageNames,
    float: bool,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols, count) = (spec.rows, spec.cols, spec.count);
    let (tr, tw) = (target.tile.rows, target.tile.cols);
    if cols % tw != 0 {
        return Err(SpatialReject::NotLowered(
            "a block of rows into a dense table whose rows are not whole tiles",
        ));
    }
    let row_t = cols / tw;
    let blk = row_block(row_t, target);
    let w = wide(blk, target);
    let n_t = rows.div_ceil(tr);
    let src_t = count.div_ceil(tr);
    let m = (src_t + 1).min(n_t);
    let (t_name, t_fill) = names.operand(0);
    let (s_name, s_fill) = names.operand(1);
    let dt = if float {
        elem_precision(target)?
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
        super::from_tensor_op::init_of(t_fill, rows * cols),
    );
    let src = t.tensor(
        s_name,
        TensorRole::Input,
        count,
        cols,
        dt,
        super::from_tensor_op::init_of(s_fill, count * cols),
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
                super::from_tensor_op::init_of(p_fill, 1),
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
            iters: super::from_tensor_op::space(target, w, None),
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
fn dense_row_gather(
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
        super::from_tensor_op::init_of(t_fill, n),
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
fn at_row(y: usize, w: usize) -> ElemAccess {
    at((y * w) as i64)
}

/// Which in-tile row a dense row write replaces: known at compile time, or computed into `[lo, hi)`.
#[derive(Clone, Copy)]
enum RowSelect {
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
    .map(|p| vec![p])
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
fn blocked_argmax(
    spec: &ArgMaxSpec,
    names: &StageNames,
    float: bool,
    target: &SpatialTarget,
) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let n = spec.n;
    let tw = target.tile.cols;
    let blk = row_block_tiles(target);
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
            super::from_tensor_op::init_of(x_fill, n),
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
            super::from_tensor_op::items_per_tile(slots),
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
                super::from_tensor_op::items_per_tile(1),
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
                super::from_tensor_op::items_per_tile(n_out),
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
            dense_set_row(spec, names, float, target)
        } else {
            dense_set_rows(spec, names, float, target)
        };
    }
    if cols % target.tile.cols == 0 {
        return tile_set_row(spec, names, float, target);
    }
    // ★ A BLOCK OF ROWS IS ONE CONTIGUOUS RUN of `count·cols` elements from row `P − 1`: the one-row write, wider.
    let n = rows * cols;
    let span = count * cols;
    let mut b = FlatBuild::columns(n, target)?;
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
fn set_row_name(spec: &SetRowSpec, layout: &str) -> String {
    let block = if spec.count > 1 {
        format!("_block{}", spec.count)
    } else {
        String::new()
    };
    format!("set_row_{}x{}{block}{layout}", spec.rows, spec.cols)
}

/// Where a mixture's optional operands sit past `[x, wr, wg, wu, wd]`: the router's bias, then the gate, up and down
/// biases that are present, in that order.
struct MoeOperands {
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
fn moe_sigmoid(
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
fn routed_scores(f: &mut FlatBuild, p: BufferId, ne: usize, target: &SpatialTarget) -> (StageId, BufferId) {
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
fn moe_choosing(
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
fn picked_penalty(picks: usize, first: usize) -> Option<Box<TensorBody>> {
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
fn moe_pick(
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
fn moe_groups(
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
fn renormalizing_sum(spec: &MoeSpec, sum: Box<TensorBody>) -> Box<TensorBody> {
    if spec.sum_can_clamp() {
        Box::new(TensorBody::Max(sum, konst(6.103515625e-5)))
    } else {
        sum
    }
}

/// A round's weight read out of the scores: the routed scale multiplies it here when nothing renormalizes it after.
fn round_weight(spec: &MoeSpec) -> TensorBody {
    match spec.scale {
        Some(_) if !spec.renormalize => *mul(op(0), konst(spec.scale())),
        _ => TensorBody::Operand(0),
    }
}

/// A mixture's gated activation of `gate` and `up`: SwiGLU's `silu(gate)·up`, or gpt-oss's
/// `min(gate, L)·σ(α·min(gate, L))·(clamp(up, −L, L) + 1)` for its slope `α` — the sigmoid term as `silu(α·g)/α`, one
/// SFPU op.
fn moe_gated(act: crate::tensor_recognize::qwen::MoeAct, gate: Box<TensorBody>, up: Box<TensorBody>) -> Box<TensorBody> {
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
enum ExpertTable {
    Whole { page: BufferId, span: usize },
    Windows { tensor: TensorId, span: usize, window_t: usize, table_t: usize },
}

/// How a round reads its chosen expert's run: the page holding it, and the element term that moves the read to the
/// run — `(index buffer, stride, extent)`: the pick by the run's length in a whole table, the run's offset in a window
/// that does not start at it.
struct ExpertRun {
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
        let tensor = f.t.tensor(name, TensorRole::Input, 1, ne * span, f.precision, super::from_tensor_op::init_of(fill, ne * span));
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
fn moe_blocks(spec: &MoeSpec, target: &SpatialTarget) -> bool {
    let tw = target.tile.cols;
    spec.d % tw == 0
        && spec.dff % tw == 0
        && (!row_needs_blocking(spec.ne, target) || spec.groups.is_none())
}

/// The names of a blocked MoE's internal tensors, all derived from the block's binder.
struct MoeNames {
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
fn fitting_chunk(
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

fn blocked_moe(
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
    let router = super::from_tensor_op::from_matmul(
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
fn moe_routing(
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
fn router_blocks(ne: usize, target: &SpatialTarget) -> Vec<(usize, usize)> {
    let blk = row_block_tiles(target);
    let span = blk * target.tile.cols;
    (0..ne.div_ceil(span)).map(|b| (b * blk, span.min(ne - b * span))).collect()
}

/// A wide router's bias, `L[e] = (x·Wr)[e] + B[e]`, a block of the row an item.
fn wide_router_bias(mn: &MoeNames, (b_name, b_fill): (&str, &Fill), ne: usize, target: &SpatialTarget) -> Result<SpatialProgram, SpatialReject> {
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
fn wide_sigmoid(mn: &MoeNames, ne: usize, target: &SpatialTarget) -> Result<SpatialProgram, SpatialReject> {
    let mut f = FlatBuild::columns(ne, target)?;
    f.precision = AccPolicy::F32.interm_precision();
    let l = f.input(&mn.logits, ne, &Fill::Unknown, target);
    let out = f.result_page("probs", ne, f.precision, target);
    let body = *unary(SfpuKind::Recip, add(konst(1.0), unary(SfpuKind::Exp, sub(konst(0.0), op(0)))));
    let chain = f.pointwise("sigmoid", ne, vec![Slot { buf: l, map: at(0) }], body, out, target)?;
    let claim = approx("moe router sigmoid", f.precision);
    f.finish(&mn.probs, ne, chain, out, format!("moe_sigmoid_{ne}"), claim, target)
}

/// A wide router's softmax, `P = e^(L − m)/Z` over its row, at the accumulator's width: each block's maximum `m_b` and
/// sum `s_b = Σ e^(L − m_b)` (a section a block), the row's `m = max m_b` and `Z = Σ_b s_b·e^(m_b − m)` (one section),
/// then `e^(L − m)·(1/Z)`, a block of the row an item.
fn wide_softmax(mn: &MoeNames, ne: usize, target: &SpatialTarget) -> Result<Vec<SpatialProgram>, SpatialReject> {
    let acc = AccPolicy::F32.interm_precision();
    let one = wide(1, target);
    let blocks = router_blocks(ne, target);
    let stats = |b: usize| format!("{}__stats{b}", mn.probs);
    let norm = format!("{}__norm", mn.probs);
    let mut out = Vec::new();
    for (b, &(first, n)) in blocks.iter().enumerate() {
        let mut f = FlatBuild::at(acc, target)?;
        let l = f.input_slice(&mn.logits, ne, first, n, &Fill::Unknown, acc, target);
        let m = f.t.scratch_deep("block_max", 1, 1, acc);
        let s1 = f.compute("block_max", head_space(1, n, Some(ReducePool::Max)), vec![Slot { buf: l, map: map2(0, 0, 1) }], per_row_map(one), TensorBody::Operand(0), m);
        let sum = f.t.scratch_deep("block_sum", 1, 1, acc);
        let s2 = f.compute(
            "block_sum",
            head_space(1, n, Some(ReducePool::Sum)),
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
        let claim = approx("moe router softmax block", acc);
        out.push(f.finish(&stats(b), 2, vec![s1, s2, s3], page, format!("moe_softmax_block{b}_{ne}"), claim, target)?);
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
    let claim = approx("moe router softmax row", acc);
    out.push(f.finish(&norm, 2, vec![s1, s2, s3], page, format!("moe_softmax_row_{ne}"), claim, target)?);
    // `e^(L − m)·(1/Z)`, a block of the row an item.
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
fn wide_moe_routing(
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
fn chunked_plan(chunks: usize, items: usize) -> BlockPlan {
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
fn row_dot(chunk_elems: usize) -> (Vec<IterSpec>, Vec<ElemAccess>) {
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
fn expert_chunk(
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
        steps: vec![(LV_ITEM_X, cols_t), (super::LV_SUB_H, chunk)],
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
fn moe_up_partials(
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
        super::from_tensor_op::init_of(x_fill, d),
    );
    let wg = t.tensor(
        wg_name,
        TensorRole::Param,
        ne * dff,
        d,
        precision,
        super::from_tensor_op::init_of(wg_fill, ne * dff * d),
    );
    let wu = t.tensor(
        wu_name,
        TensorRole::Param,
        ne * dff,
        d,
        precision,
        super::from_tensor_op::init_of(wu_fill, ne * dff * d),
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
    let wg_in = t.buffer("in_gate", cd, precision);
    let wu_in = t.buffer("in_up", cd, precision);
    let g_out = t.buffer("gate_out", 1, acc);
    let u_out = t.buffer("up_out", 1, acc);
    let hold = |mut v: Vec<(LoopVar, usize)>| {
        v.push((super::LV_SUB_H, 0));
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
    let l_x = t.load_block_at("load_x", x, 0, vec![(super::LV_SUB_H, cd)], 1, cd, cd, x_in);
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
    let steps = vec![(LV_ITEM_Y, f_t * nc), (LV_ITEM_X, nc), (super::LV_SUB_H, 1)];
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
        approx("mixture-of-experts gate/up partials", precision),
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
fn expert_bias_tile(
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
fn moe_hidden(
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
        let table = t.tensor(b_name, TensorRole::Param, 1, spec.ne * dff, acc, super::from_tensor_op::init_of(b_fill, spec.ne * dff));
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
fn moe_down_partials(
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
        precision,
        super::from_tensor_op::init_of(wd_fill, ne * d * dff),
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
    let w_in = t.buffer("in_down", cd, precision);
    let d_out = t.buffer("down_out", 1, acc);
    let hold = |mut v: Vec<(LoopVar, usize)>| {
        v.push((super::LV_SUB_H, 0));
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
        vec![(LV_ITEM_Y, f_t), (super::LV_SUB_H, cd)],
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
        vec![(LV_ITEM_Y, d_t * nc), (LV_ITEM_X, nc), (super::LV_SUB_H, 1)],
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
        approx("mixture-of-experts down partials", precision),
        target,
    )?;
    prog.work.plan = Some(chunked_plan(nc, k * d_t));
    prog.work.k_tiles = 1;
    Ok(prog)
}

/// The combine: per tile of the `d` outputs, each round's down partials added, plus its expert's down bias when the
/// mixture has one, weighted by its router probability, and divided by the kept probabilities' sum when the router
/// renormalizes — the desugar's `Σ (P[b]/Σ P)·y_b`, or `Σ P[b]·y_b`.
fn moe_combine(
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
        let table = t.tensor(b_name, TensorRole::Param, 1, spec.ne * d, acc, super::from_tensor_op::init_of(b_fill, spec.ne * d));
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

/// The same nesting as [`banded`], predicated on a chosen axis — the reductions put the head on the ROW
/// axis (so the folded axis can be the column), and a band is a band whichever axis carries it.
fn banded_on(axis: LoopVar, bands: Vec<(usize, TensorBody)>) -> TensorBody {
    let mut it = bands.into_iter().rev();
    let (_, mut acc) = it.next().expect("a banded body has at least one band");
    for (hi, body) in it {
        acc = TensorBody::Select {
            cond: Pred::AxisLt(axis, PredRhs::Const(hi)),
            then: Box::new(body),
            otherwise: Box::new(acc),
        };
    }
    acc
}

/// Every compute stage of a composition is one of the CLOSED algebra's two shapes — the property
/// `each_composition_is_a_chain_of_the_closed_algebra` asserts, exposed so a lock and a ledger read the
/// same predicate rather than two copies of it.
pub fn is_closed_algebra(p: &SpatialProgram) -> bool {
    p.stages.iter().all(|s| match &s.kind {
        StageKind::Compute(op) => matches!(
            op,
            ComputeOp::Generic { .. } | ComputeOp::MatmulTiles { .. }
        ),
        _ => true,
    })
}

/// The compute shapes a program uses, named — for a ledger row or a refusal.
pub fn compute_shapes(p: &SpatialProgram) -> BTreeSet<&'static str> {
    p.stages
        .iter()
        .filter_map(|s| match &s.kind {
            StageKind::Compute(ComputeOp::Generic { .. }) => Some("Generic"),
            StageKind::Compute(ComputeOp::MatmulTiles { .. }) => Some("MatmulTiles"),
            StageKind::Compute(ComputeOp::Requant { .. }) => Some("Requant"),
            StageKind::Compute(ComputeOp::Bias { .. }) => Some("Bias"),
            StageKind::Compute(ComputeOp::Relu) => Some("Relu"),
            StageKind::Compute(ComputeOp::Scalar(_)) | StageKind::Scalar(_) => Some("Scalar"),
            _ => None,
        })
        .collect()
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
    out.push(super::from_tensor_op::from_matmul(
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
    out.push(match rows {
        1 => rescale_row(
            &base_name, &base_fill, weight, &inv_name, y, n, precision, target,
        )?,
        _ => rescale_rows(
            &base_name,
            &base_fill,
            weight,
            &inv_name,
            y,
            (rows, n),
            precision,
            target,
        )?,
    });
    Ok(out)
}

/// [`blocked_binary_map`] over a MATRIX: one item per tile of the 2-D grid, as [`matrix_unary_map`].
fn matrix_binary_map(
    name: &str,
    kind: &str,
    body: impl Fn(Box<TensorBody>, Box<TensorBody>) -> Box<TensorBody>,
    a: (&str, &Fill),
    b: (&str, &Fill),
    out: &str,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols) = shape;
    let precision = elem_precision(target)?;
    let mut t = Template::new(target)?;
    let at_ = t.tensor(
        a.0,
        TensorRole::Input,
        rows,
        cols,
        precision,
        super::from_tensor_op::init_of(a.1, rows * cols),
    );
    let bt = t.tensor(
        b.0,
        TensorRole::Input,
        rows,
        cols,
        precision,
        super::from_tensor_op::init_of(b.1, rows * cols),
    );
    let yt = t.tensor(
        out,
        TensorRole::Output,
        rows,
        cols,
        precision,
        TensorInit::Zero,
    );
    let (rows_t, cols_t) = (
        rows.div_ceil(target.tile.rows),
        cols.div_ceil(target.tile.cols),
    );
    let steps = super::from_tensor_op::tile_grid_steps(rows_t, cols_t);
    let ain = t.buffer("in_a", 1, precision);
    let bin = t.buffer("in_b", 1, precision);
    let ob = t.buffer("out", 1, precision);
    let la = t.load("load_a", at_, steps.clone(), 1, ain);
    let lb = t.load("load_b", bt, steps.clone(), 1, bin);
    let w = target.tile.cols;
    let s = t.stage(
        kind,
        StageKind::Compute(ComputeOp::Generic {
            iters: super::from_tensor_op::space(target, w, None),
            ins: vec![ident_map(w), ident_map(w)],
            out: ident_map(w),
            body: *body(op(0), op(1)),
        }),
        vec![ain, bin],
        vec![ob],
    );
    let st = t.store(&format!("store_{out}"), yt, steps, 1, ob);
    t.edge(la, ain, s);
    t.edge(lb, bin, s);
    t.edge(s, ob, st);
    t.finish(
        format!("{name}_{rows}x{cols}"),
        super::from_tensor_op::items_per_tile_grid(rows_t, cols_t),
        AccPolicy::F32,
        approx(kind, precision),
        target,
    )
}

/// [`rescale_row`] over a `rows × n` MATRIX: `y[r][c] = x[r][c] · w[c] · inv[r]`, one item per tile of the 2-D
/// grid. The weight is the item's tile of the one weight row, broadcast down the tile's rows; the scale is the
/// item's tile of the `rows × 1` column, broadcast along them.
fn rescale_rows(
    x: &str,
    x_fill: &Fill,
    weight: Option<(&str, &Fill)>,
    inv: &str,
    y: &str,
    shape: (usize, usize),
    precision: Precision,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, n) = shape;
    let (rows_t, cols_t) = (
        rows.div_ceil(target.tile.rows),
        n.div_ceil(target.tile.cols),
    );
    let steps = super::from_tensor_op::tile_grid_steps(rows_t, cols_t);
    let mut t = Template::new(target)?;
    let xt = t.tensor(
        x,
        TensorRole::Input,
        rows,
        n,
        precision,
        super::from_tensor_op::init_of(x_fill, rows * n),
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
                super::from_tensor_op::init_of(wf, n),
            );
            let b = t.buffer("in_w", 1, precision);
            let l = t.load("load_w", wt, vec![(LV_ITEM_X, 1)], 1, b);
            (Some(b), Some(l))
        }
        None => (None, None),
    };
    let mut ins = vec![xin];
    let mut maps = vec![ident_map(w)];
    let mut body = op(0);
    if let Some(b) = wbuf {
        ins.push(b);
        maps.push(super::from_tensor_op::per_col_map());
        body = mul(body, op(ins.len() - 1));
    }
    ins.push(iin);
    maps.push(super::from_tensor_op::per_row_map(w));
    body = mul(body, op(ins.len() - 1));
    let s = t.stage(
        "rescale",
        StageKind::Compute(ComputeOp::Generic {
            iters: super::from_tensor_op::space(target, w, None),
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
    t.edge(s, ob, st);
    t.finish(
        format!("rmsnorm_rescale_{rows}x{n}"),
        super::from_tensor_op::items_per_tile_grid(rows_t, cols_t),
        AccPolicy::F32,
        approx("rmsnorm", precision),
        target,
    )
}

/// A blocked pointwise map of TWO equal-length rows — the fused residual add.
fn blocked_binary_map(
    name: &str,
    kind: &str,
    body: impl Fn(Box<TensorBody>, Box<TensorBody>) -> Box<TensorBody>,
    a: (&str, &Fill),
    b: (&str, &Fill),
    out: &str,
    n: usize,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let precision = elem_precision(target)?;
    let cols_t = tiles_of(n, target);
    let mut t = Template::new(target)?;
    let at_ = t.tensor(
        a.0,
        TensorRole::Input,
        1,
        n,
        precision,
        super::from_tensor_op::init_of(a.1, n),
    );
    let bt = t.tensor(
        b.0,
        TensorRole::Input,
        1,
        n,
        precision,
        super::from_tensor_op::init_of(b.1, n),
    );
    let yt = t.tensor(out, TensorRole::Output, 1, n, precision, TensorInit::Zero);
    let ain = t.buffer("in_a", 1, precision);
    let bin = t.buffer("in_b", 1, precision);
    let ob = t.buffer("out", 1, precision);
    let la = t.load("load_a", at_, vec![(LV_ITEM_X, 1)], 1, ain);
    let lb = t.load("load_b", bt, vec![(LV_ITEM_X, 1)], 1, bin);
    let w = wide(1, target);
    let s = t.stage(
        kind,
        StageKind::Compute(ComputeOp::Generic {
            iters: flat_space(w),
            ins: vec![ident_map(w), ident_map(w)],
            out: ident_map(w),
            body: *body(op(0), op(1)),
        }),
        vec![ain, bin],
        vec![ob],
    );
    let st = t.store(&format!("store_{out}"), yt, vec![(LV_ITEM_X, 1)], 1, ob);
    t.edge(la, ain, s);
    t.edge(lb, bin, s);
    t.edge(s, ob, st);
    t.finish(
        format!("{name}_{n}"),
        items_per_tile(cols_t),
        AccPolicy::F32,
        approx(kind, precision),
        target,
    )
}

/// `y[i] = x[i] · w[i] · inv` — one item per tile, with `inv` a single tile read through a fixed map so
/// the same scalar reaches every block of the row.
fn rescale_row(
    x: &str,
    x_fill: &Fill,
    weight: Option<(&str, &Fill)>,
    inv: &str,
    y: &str,
    n: usize,
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
        super::from_tensor_op::init_of(x_fill, n),
    );
    let it = t.tensor(
        inv,
        TensorRole::Input,
        1,
        1,
        precision,
        TensorInit::External,
    );
    let yt = t.tensor(y, TensorRole::Output, 1, n, precision, TensorInit::Zero);
    let xin = t.buffer("in_x", 1, precision);
    let iin = t.buffer("in_inv", 1, precision);
    let ob = t.buffer("out", 1, precision);
    let lx = t.load("load_x", xt, vec![(LV_ITEM_X, 1)], 1, xin);
    let li = t.load("load_inv", it, vec![], 1, iin);
    let w = wide(1, target);
    let (wbuf, lw) = match weight {
        Some((wn, wf)) => {
            let wt = t.tensor(
                wn,
                TensorRole::Param,
                1,
                n,
                precision,
                super::from_tensor_op::init_of(wf, n),
            );
            let b = t.buffer("in_w", 1, precision);
            let l = t.load("load_w", wt, vec![(LV_ITEM_X, 1)], 1, b);
            (Some(b), Some(l))
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
    ins.push(iin);
    maps.push(fixed(0));
    body = mul(body, op(ins.len() - 1));
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
    t.edge(s, ob, st);
    t.finish(
        format!("rmsnorm_rescale_{n}"),
        items_per_tile(cols_t),
        AccPolicy::F32,
        approx("rmsnorm", precision),
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
fn blocked_l2norm(
    spec: &L2NormSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols) = (spec.rows, spec.cols);
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
        super::from_tensor_op::init_of(xf, n),
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

/// ★ A CAUSAL CONV BLOCKS BY CHANNEL, AND NEEDS NO HALO. `y[c] = Σ_t W[t,c]·CS[t,c]` — every channel
/// reads only its OWN column of the ring, so channels are independent and a block of them is a complete
/// sub-problem. (The "causal" axis is the ring's TAP axis, which is already resident in full; the halo a
/// sliding-window convolution would need does not arise.)
///
/// The ring is `taps × chan` stored flat, so one channel tile's taps sit `chan/tile` TILES apart — a
/// strided `block_h = taps` load. The loaded page is a `32 × taps·32` plane whose tile index is the tap,
/// so inside the page the tap's stride is one TILE WIDTH, not `chan`. That last sentence is the whole
/// difference between this and the flat form, and getting it wrong reads another channel's history.
fn blocked_causal_conv(
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
            super::from_tensor_op::init_of(cs_fill, ring_len),
        );
        let x = t.tensor(
            x_name,
            TensorRole::Input,
            1,
            chan,
            precision,
            super::from_tensor_op::init_of(x_fill, chan),
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
            super::from_tensor_op::init_of(cs_fill, ring_len),
        );
        let wc = t.tensor(
            w_name,
            TensorRole::Param,
            taps,
            chan,
            precision,
            super::from_tensor_op::init_of(w_fill, ring_len),
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
fn blocked_gated_delta(
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
            super::from_tensor_op::from_matmul(
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
            super::from_tensor_op::init_of(st_fill, s_rows * d),
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
            super::from_tensor_op::init_of(q_fill, rows * hk * d),
        );
        let k = t.tensor(
            k_name,
            TensorRole::Input,
            1,
            rows * hk * d,
            precision,
            super::from_tensor_op::init_of(k_fill, rows * hk * d),
        );
        let v = t.tensor(
            v_name,
            TensorRole::Input,
            1,
            rows * out_len,
            precision,
            super::from_tensor_op::init_of(v_fill, rows * out_len),
        );
        let z = t.tensor(
            z_name,
            TensorRole::Input,
            1,
            rows * out_len,
            precision,
            super::from_tensor_op::init_of(z_fill, rows * out_len),
        );
        let gn = t.tensor(
            gn_name,
            TensorRole::Param,
            1,
            d,
            precision,
            super::from_tensor_op::init_of(gn_fill, d),
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
                        iters: super::from_tensor_op::space(target, w, None),
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
fn widened_gated_delta(
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
fn joined_zero_runs(runs: Vec<(usize, Option<usize>)>) -> Vec<(usize, Option<usize>)> {
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
fn moved_runs(
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
    let source = t.tensor(src.0, TensorRole::Input, 1, src_len, dt, super::from_tensor_op::init_of(src.1, src_len));
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

/// `⌈log₂ c⌉`, at least 1: how many factors `(I + N)(I + N²)(I + N⁴)…` invert `I + N` for a strictly lower `c × c` `N`
/// — `N^{2^m}` vanishes once `2^m ≥ c`.
fn solve_factors(c: usize) -> usize {
    (usize::BITS - c.saturating_sub(1).leading_zeros()).max(1) as usize
}

/// Whether a block of `rows` rows is lowered a chunk of `tile` rows at a time: its chunks take fewer sections than its
/// rows stepped one at a time.
fn takes_chunks(rows: usize, tile: usize) -> bool {
    rows > 1 && chunked_sections(rows, tile) < stepped_sections(rows)
}

/// Sections the step takes over a block of `rows` rows, a row at a time: per row one pass over the memory; per block
/// the two gate broadcasts.
fn stepped_sections(rows: usize) -> usize {
    2 + rows
}

/// Sections a chunked block of `rows` rows takes on `tile`-row chunks: per chunk the gates, `A` and `QKΓ`, the solve's
/// factors, `U`/`W`, `Δ`, the output and the memory update; per block the two gate broadcasts.
fn chunked_sections(rows: usize, tile: usize) -> usize {
    2 + (0..rows.div_ceil(tile))
        .map(|c| 6 + solve_factors((rows - c * tile).min(tile)))
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
/// Every product is ONE contraction (`LV_GEN_K`), and the unit lower-triangular solve is the product
/// `(I + N)(I + N²)(I + N⁴)…` with `N = −A` — `⌈log₂ C⌉` doublings, not `C` steps. The memory is read and written once
/// per chunk instead of three times per row.
///
/// A chunk is ONE TILE ROW of the block's rows, so a head's chunk of `Q`, `K`, `V`, `Z` is a `1 × nb` page of the DENSE
/// table — the tables are read dense, and the layout pass bridges a flat one once. A computed page has one consumer, so
/// what the algebra uses twice (`G`, `A`, the solve's factors, `U`, `W`, `Δ`) crosses a section as a per-chunk tensor,
/// loaded once per reader. A partial last chunk masks its rows on the contraction variable; its padding rows compute to
/// zero, never to a NaN (a masked exponential is SELECTED away, never multiplied by zero).
fn chunked_gated_delta(
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
            super::from_tensor_op::from_matmul(
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
            super::from_tensor_op::init_of(q_fill, rows * qk_w),
        )
    };
    let k_tensor = |t: &mut Template| {
        t.tensor(
            k_name,
            TensorRole::Input,
            rows,
            qk_w,
            precision,
            super::from_tensor_op::init_of(k_fill, rows * qk_w),
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
        let factors = solve_factors(cc);
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

        // ── The solve: `X = (I + N)(I + N²)(I + N⁴)…` with `N = −A`, factor by factor. Section `j` forms `X_{j−1}` from
        // `X_{j−2}` and `P_{j−1} = N^{2^{j−1}}`, and squares `P_{j−1}` into `P_j` for the section after it.
        for j in 0..factors {
            sections.push({
                let mut t = Template::new(target)?;
                // `P_j`: `A²` at the first factor (`N² = A²`), then the previous power squared.
                if j + 1 < factors {
                    let src = chunk_tensor(
                        &mut t,
                        c,
                        &if j == 0 {
                            "a".to_string()
                        } else {
                            format!("p{j}")
                        },
                        TensorRole::Input,
                        hv * tw,
                    );
                    let dst = chunk_tensor(
                        &mut t,
                        c,
                        &format!("p{}", j + 1),
                        TensorRole::Output,
                        hv * tw,
                    );
                    let (pin, pin2) = (
                        t.buffer("in_p", 1, precision),
                        t.buffer("in_p2", 1, precision),
                    );
                    let lp = t.load_block_at("load_p", src, 0, head_steps(1), 1, 1, hv, pin);
                    let lp2 = t.load_block_at("load_p2", src, 0, head_steps(1), 1, 1, hv, pin2);
                    let out = t.buffer("p_sq", 1, precision);
                    let s = t.stage(
                        "gd_square",
                        generic(
                            contract(tr, tw, tr),
                            vec![
                                m(&[(LV_GEN_ROW, w1 as i64), (LV_GEN_K, 1)]),
                                m(&[(LV_GEN_K, w1 as i64), (LV_GEN_COL, 1)]),
                            ],
                            ident_map(w1),
                            *mul(op(0), op(1)),
                        ),
                        vec![pin, pin2],
                        vec![out],
                    );
                    let st = t.store_block("store_p", dst, 0, head_steps(1), 1, 1, hv, out);
                    t.edge(lp, pin, s);
                    t.edge(lp2, pin2, s);
                    t.edge(s, out, st);
                }
                let x_out = chunk_tensor(&mut t, c, &format!("x{j}"), TensorRole::Output, hv * tw);
                let out = t.buffer("x", 1, precision);
                if j == 0 {
                    // `X₀ = I − A`.
                    let a = chunk_tensor(&mut t, c, "a", TensorRole::Input, hv * tw);
                    let id = param(&mut t, "eye", eye());
                    let (ain, iin) = (
                        t.buffer("in_a", 1, precision),
                        t.buffer("in_eye", 1, precision),
                    );
                    let la = t.load_block_at("load_a", a, 0, head_steps(1), 1, 1, hv, ain);
                    let li = t.load("load_eye", id, vec![], 1, iin);
                    let s = t.stage(
                        "gd_x0",
                        generic(
                            pointwise(tw),
                            vec![ident_map(w1), ident_map(w1)],
                            ident_map(w1),
                            *sub(op(1), op(0)),
                        ),
                        vec![ain, iin],
                        vec![out],
                    );
                    t.edge(la, ain, s);
                    t.edge(li, iin, s);
                    let st = t.store_block("store_x", x_out, 0, head_steps(1), 1, 1, hv, out);
                    t.edge(s, out, st);
                } else {
                    // `X_j = X_{j−1} + X_{j−1}·P_j`.
                    let x_prev = chunk_tensor(
                        &mut t,
                        c,
                        &format!("x{}", j - 1),
                        TensorRole::Input,
                        hv * tw,
                    );
                    let p = chunk_tensor(&mut t, c, &format!("p{j}"), TensorRole::Input, hv * tw);
                    let (xin, pin, xin2) = (
                        t.buffer("in_x", 1, precision),
                        t.buffer("in_xp_p", 1, precision),
                        t.buffer("in_x_add", 1, precision),
                    );
                    let lx = t.load_block_at("load_x", x_prev, 0, head_steps(1), 1, 1, hv, xin);
                    let lp = t.load_block_at("load_xp_p", p, 0, head_steps(1), 1, 1, hv, pin);
                    let lx2 =
                        t.load_block_at("load_x_add", x_prev, 0, head_steps(1), 1, 1, hv, xin2);
                    let xp = t.buffer("xp", 1, precision);
                    let s1 = t.stage(
                        "gd_xp",
                        generic(
                            contract(tr, tw, tr),
                            vec![
                                m(&[(LV_GEN_ROW, w1 as i64), (LV_GEN_K, 1)]),
                                m(&[(LV_GEN_K, w1 as i64), (LV_GEN_COL, 1)]),
                            ],
                            ident_map(w1),
                            *mul(op(0), op(1)),
                        ),
                        vec![xin, pin],
                        vec![xp],
                    );
                    let s2 = t.stage(
                        "gd_x",
                        generic(
                            pointwise(tw),
                            vec![ident_map(w1), ident_map(w1)],
                            ident_map(w1),
                            *add(op(0), op(1)),
                        ),
                        vec![xp, xin2],
                        vec![out],
                    );
                    t.edge(lx, xin, s1);
                    t.edge(lp, pin, s1);
                    t.edge(s1, xp, s2);
                    t.edge(lx2, xin2, s2);
                    let st = t.store_block("store_x", x_out, 0, head_steps(1), 1, 1, hv, out);
                    t.edge(s2, out, st);
                }
                finish(t, &format!("solve{j}"), c, tile_items())?
            });
        }
        let x_last = format!("x{}", factors - 1);

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
                super::from_tensor_op::init_of(v_fill, rows * v_w),
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
                super::from_tensor_op::init_of(st_fill, s_rows * d),
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
                super::from_tensor_op::init_of(st_fill, s_rows * d),
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
                super::from_tensor_op::init_of(z_fill, rows * v_w),
            );
            let gn = t.tensor(
                gn_name,
                TensorRole::Param,
                1,
                d,
                precision,
                super::from_tensor_op::init_of(gn_fill, d),
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
                super::from_tensor_op::init_of(st_fill, s_rows * d),
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
                v.push((super::LV_SUB_H, 0));
                v
            };
            let row_steps = vec![
                (LV_ITEM_Y, nb * nb),
                (LV_ITEM_X, hk * nb * nb),
                (super::LV_SUB_H, nb),
            ];
            let d_steps = vec![(LV_ITEM_Y, nb), (LV_ITEM_X, hk * nb), (super::LV_SUB_H, 1)];
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

/// A `Generic` compute stage's kind.
fn generic_op(iters: Vec<IterSpec>, ins: Vec<ElemAccess>, out: ElemAccess, body: TensorBody) -> StageKind {
    StageKind::Compute(ComputeOp::Generic { iters, ins, out, body })
}

/// `C = A ⊕ B` over a flat block of `n / w` rows of `w`, `B` one row: a band per row, each reading `B` from its row's
/// start — the one pointwise stage, blocked by the result's columns past an accumulator page.
#[allow(clippy::too_many_arguments)]
pub(crate) fn flat_row_broadcast(
    kind: crate::tensor_recognize::ElementwiseOp,
    names: &super::from_tensor_op::ElementwiseNames,
    fills: (&Fill, &Fill),
    n: usize,
    w: usize,
    precision: Precision,
    claim: NumericsClaim,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let mut b = FlatBuild::columns(n, target)?;
    b.precision = precision;
    let a_in = b.input(&names.a, n, fills.0, target);
    let row = b.input(&names.b, w, fills.1, target);
    let out = b.result_page("out", n, precision, target);
    let mut slots = vec![Slot { buf: a_in, map: at(0) }];
    let mut bands = Vec::new();
    for r in 0..n / w {
        slots.push(Slot { buf: row, map: at(-((r * w) as i64)) });
        let rhs = op(slots.len() - 1);
        let combined = match kind {
            crate::tensor_recognize::ElementwiseOp::Add => *add(op(0), rhs),
            crate::tensor_recognize::ElementwiseOp::Mul => *mul(op(0), rhs),
        };
        bands.push(((r + 1) * w, combined));
    }
    let chain = b.pointwise("elementwise_row", n, slots, banded(bands), out, target)?;
    b.finish_as(&names.c, n, chain, out, format!("elementwise_{}_row_{}x{w}", kind.c_op(), n / w), claim, precision, target)
}

/// [`from_rope`] with the HEAD as the work item — see the note there for why that is the natural unit.
fn blocked_rope(
    spec: &RopeSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (heads, hd, half, rows) = (spec.h, spec.hd, spec.half, spec.rows);
    let n = heads * hd;
    let head_t = tiles_of(hd, target);
    let half_t = tiles_of(half, target);
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
        super::from_tensor_op::init_of(qf, n),
    );
    let ct = t.tensor(
        cn,
        TensorRole::Input,
        1,
        table_rows * half,
        precision,
        super::from_tensor_op::init_of(cf, table_rows * half),
    );
    let st = t.tensor(
        sn,
        TensorRole::Input,
        1,
        table_rows * half,
        precision,
        super::from_tensor_op::init_of(sf, table_rows * half),
    );
    let yt = t.tensor(
        &names.out,
        TensorRole::Output,
        1,
        n,
        precision,
        TensorInit::Zero,
    );
    let tw = target.tile.cols;
    let table_t = tiles_of(table_rows * half, target);
    let window_t = if windowed { (tw - 1 + half).div_ceil(tw).min(table_t) } else { half_t };
    let qin = t.buffer_deep("in_q", 1, head_t, precision);
    let cin = t.buffer_deep("in_rc", 1, window_t, precision);
    let sin = t.buffer_deep("in_rs", 1, window_t, precision);
    let ob = t.buffer_deep("out", 1, head_t, precision);
    // One table row is the SAME for every head of its row, so the tables' tile address moves only with the row — and,
    // reading a cache, starts at the position's row `P − 1`: a literal in the base; a named one an [`IndirectTile`] over
    // `P − 1`, computed per item on the integer datapath into a FIFO its load retires (one per table: a FIFO has one
    // reader) — behind a guard of stride zero that keeps the block's last row among the cache's, as a row index is.
    let lq = t.load_block_at(
        "load_q",
        qt,
        0,
        row_steps(per_row * head_t, vec![(LV_ITEM_X, head_t)]),
        1,
        head_t,
        head_t,
        qin,
    );
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
                    super::from_tensor_op::init_of(pf, 1),
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
    let mut bands = vec![
        (half, *sub(mul(op(0), op(3)), mul(op(1), op(4)))),
        (2 * half, *add(mul(op(2), op(5)), mul(op(0), op(6)))),
    ];
    if 2 * half < hd {
        bands.push((hd, TensorBody::Operand(0)));
    }
    let s = t.stage(
        "rope",
        StageKind::Compute(ComputeOp::Generic {
            iters: flat_space(w),
            ins: slots,
            out: ident_map(w),
            body: banded(bands),
        }),
        ins,
        vec![ob],
    );
    let store = t.store_block(
        "store_y",
        yt,
        0,
        row_steps(per_row * head_t, vec![(LV_ITEM_X, head_t)]),
        1,
        head_t,
        head_t,
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
                    col0_t: h * head_t,
                    h_t: 1,
                    w_t: head_t,
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

/// [`from_row_norm_flat`] with the HEAD as the work item: `y[h, i] = x[h, i]·w[i]·rsqrt(mean_i x[h, i]²)`.
/// The weight is a per-COLUMN table shared by every head, so its tile address does not move with the item.
fn blocked_head_rms(
    spec: &FusedRowSpec,
    names: &StageNames,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (rows, cols) = (spec.rows, spec.cols);
    let n = rows * cols;
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
        super::from_tensor_op::init_of(xf, n),
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
    let step = vec![(LV_ITEM_X, head_t)];
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
                cols,
                precision,
                super::from_tensor_op::init_of(wf, cols),
            );
            let b = t.buffer_deep("in_w", 1, head_t, precision);
            let l = t.load_block_at("load_w", wt, 0, vec![], 1, head_t, head_t, b);
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
            coord: (0, h),
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
