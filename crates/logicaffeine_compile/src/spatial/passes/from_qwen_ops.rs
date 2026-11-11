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
use crate::tensor_recognize::qwen::{LaneMeanSpec, LaneSpreadSpec, LaneSumsSpec, NgramIdsSpec, SignedRootSpec};
use crate::codegen_sva::fabric_rtl::{AluOp, CmpKind, LoopCell, Src};
use crate::spatial::wide_int::{lower as lane_program, Op64};

mod pointwise;
mod ngram;
mod shape;
mod products;
mod rows;
mod rope;
mod recurrent;
mod convolution;
mod attention;
mod relayout;
mod gathers;
mod experts;
mod written;

pub use self::pointwise::*;
pub use self::ngram::*;
pub use self::shape::*;
pub use self::products::*;
pub use self::rows::*;
pub use self::rope::*;
pub use self::recurrent::*;
pub use self::convolution::*;
pub use self::attention::*;
pub(crate) use self::relayout::*;
pub use self::gathers::*;
pub use self::experts::*;
pub use self::written::*;

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

/// ★ A PRODUCT'S OPERANDS ARE HELD AT THE PRODUCTS' FLOAT. A matrix path multiplies in the narrowest float it offers,
/// so that float is what a value read only into products is stored at where the elementwise datapath also holds it —
/// an expert's weights, which its gate, up and down products read, and a key or value cache ([`cache_precision`]) — and
/// their sums stay at the accumulator's width. An fp32 island beside bf16 products rounds such a value once, on its way
/// in, and it moves the bytes a bf16 one does. A target with no float product, or one whose elementwise datapath lacks
/// it, holds it at its elementwise float.
pub(crate) fn products_float(target: &SpatialTarget) -> Result<Precision, SpatialReject> {
    match [Precision::Bf16, Precision::Fp16, Precision::Fp32].into_iter().find(|p| target.matmul_supports(*p)) {
        Some(p) if target.generic_holds(p) => Ok(p),
        _ => elem_precision(target),
    }
}

/// ★ A KEY OR VALUE CACHE IS HELD AT THE PRODUCTS' FLOAT ([`products_float`]). An attention reads its cache only into
/// its score and value products; a row write moves the cache in it.
pub(crate) fn cache_precision(target: &SpatialTarget) -> Result<Precision, SpatialReject> {
    products_float(target)
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
    /// The pages that hold one block of a column-blocked section: a later stage reads them in the block's own columns.
    local: Vec<BufferId>,
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
            local: Vec::new(),
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
        let buf = self.t.buffer(name, tiles, precision);
        if self.blocks.is_some() {
            self.local.push(buf);
        }
        buf
    }

    /// The scratch between two stages of a chain over an `n`-column result: the whole row, or the item's block of it,
    /// which the next stage reads in the block's own columns.
    fn block_scratch(&mut self, name: &str, n: usize, target: &SpatialTarget) -> BufferId {
        let tiles = self.blocks.map_or(tiles_of(n, target), |b| b.blk);
        let buf = self.scratch(name, tiles);
        if self.blocks.is_some() {
            self.local.push(buf);
        }
        buf
    }

    /// `dst[c] = body(c)` over the result's `n` columns, every map and predicate written in RESULT columns. In a
    /// column-blocked section the stage walks its item's block, and a column `c` of the block is result column
    /// `c + o` for the item's first column `o`, read from a compile-time table one page an item: a map that walks the
    /// columns gathers through `o`; a predicate's constant bound `k` becomes the table's `k − o`; a bound or a gather
    /// index the section computes is moved by `o` in a stage of its own first. A page of the block itself
    /// ([`Self::block_scratch`]) is read in the block's columns. Returns the stages, in order.
    fn pointwise(
        &mut self,
        name: &str,
        n: usize,
        slots: Vec<Slot>,
        body: TensorBody,
        dst: BufferId,
        target: &SpatialTarget,
    ) -> Result<Vec<StageId>, SpatialReject> {
        self.columnwise(name, n, None, slots, body, dst, target)
    }

    /// `dst[c] = pool_r body(r, c)` over the result's `n` columns, `r` folded over `extent` rows: a [`Self::pointwise`]
    /// stage whose ROW axis is a reduction, so a weighted sum or a dot product a column blocks past a page as a
    /// pointwise stage does.
    #[allow(clippy::too_many_arguments)]
    fn fold(
        &mut self,
        name: &str,
        n: usize,
        (extent, pool): (usize, ReducePool),
        slots: Vec<Slot>,
        body: TensorBody,
        dst: BufferId,
        target: &SpatialTarget,
    ) -> Result<Vec<StageId>, SpatialReject> {
        self.columnwise(name, n, Some((extent, pool)), slots, body, dst, target)
    }

    #[allow(clippy::too_many_arguments)]
    fn columnwise(
        &mut self,
        name: &str,
        n: usize,
        fold: Option<(usize, ReducePool)>,
        slots: Vec<Slot>,
        body: TensorBody,
        dst: BufferId,
        target: &SpatialTarget,
    ) -> Result<Vec<StageId>, SpatialReject> {
        let space = |cols: usize| match fold {
            None => flat_space(cols),
            Some((extent, pool)) => vec![
                IterSpec { var: LV_GEN_ROW, extent, kind: IterKind::Reduce { pool } },
                IterSpec { var: LV_GEN_COL, extent: cols, kind: IterKind::Parallel },
            ],
        };
        let out_map = |w: usize| if fold.is_some() { per_col_map() } else { ident_map(w) };
        let Some(ColumnBlocks { blk, items }) = self.blocks else {
            let w = wide(self.t.page_tiles(dst), target);
            return Ok(vec![self.compute(name, space(n), slots, out_map(w), body, dst)]);
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
        // `len` entries read through `from`, each moved by `o` in `body`, into a page of their own.
        let mut moved = |f: &mut FlatBuild, what: &str, from: Slot, len: usize, body: Box<TensorBody>| {
            let cols_t = tiles_of(len, target);
            let page = f.t.scratch_deep(&format!("{name}_{what}"), 1, cols_t, ix);
            stages.push(f.compute(&format!("{name}_{what}"), flat_space(len), vec![Slot { buf: at_table, map: fixed(0) }, from], ident_map(wide(cols_t, target)), *body, page));
            page
        };
        let mut own = Vec::with_capacity(slots.len());
        let mut extra: Vec<Slot> = Vec::new();
        // A moved index is one page however many reads go through it: `(walk, index operand, index stride, lift)` names
        // it, and every read of it shares the page (a rope's tables read the one position from every head's band).
        let mut indices: Vec<((i64, usize, i64, usize), usize)> = Vec::new();
        for (j, slot) in slots.iter().enumerate() {
            let walks = slot.map.terms.iter().find(|(v, _)| *v == LV_GEN_COL).map_or(0, |(_, s)| *s);
            let walks = if self.local.contains(&slot.buf) { 0 } else { walks };
            let map = match (walks, slot.map.indirect) {
                (0, _) => slot.map.clone(),
                (s, None) => ElemAccess {
                    indirect: Some(IndirectTerm { operand: 0, stride: s, modulus: span + 1 }),
                    ..slot.map.clone()
                },
                // A gather already indexed by data: its index moves by `o` too, shifted to stay non-negative.
                (s, Some(ind)) => {
                    let lift = ind.stride.unsigned_abs() as usize * ind.modulus;
                    let key = (s, ind.operand, ind.stride, lift);
                    let k = match indices.iter().find(|(seen, _)| *seen == key) {
                        Some((_, k)) => *k,
                        None => {
                            let index = &slots[ind.operand];
                            let page = moved(self, &format!("index{j}"), Slot { buf: index.buf, map: index.map.clone() }, 1, add(add(mul(op(0), konst(s as f64)), mul(op(1), konst(ind.stride as f64))), konst(lift as f64)));
                            extra.push(Slot { buf: page, map: fixed(0) });
                            indices.push((key, extra.len() - 1));
                            extra.len() - 1
                        }
                    };
                    ElemAccess {
                        base: slot.map.base - lift as i64,
                        terms: slot.map.terms.clone(),
                        indirect: Some(IndirectTerm { operand: usize::MAX - k, stride: 1, modulus: s.unsigned_abs() as usize * span + 2 * lift + 1 }),
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
        // Bounds read at fixed entries of one vector (an attention's thresholds, one a head) move as the vector, in one
        // stage, as long as its last entry read; any other bound moves alone.
        let entry = |s: &Slot| (s.map.terms.is_empty() && s.map.indirect.is_none() && s.map.base >= 0).then_some(s.map.base as usize);
        let mut vectors: Vec<(BufferId, usize, Option<BufferId>)> = Vec::new();
        for &o in &operands {
            if let Some(k) = entry(&slots[o]) {
                match vectors.iter_mut().find(|(from, _, _)| *from == slots[o].buf) {
                    Some((_, len, _)) => *len = (*len).max(k + 1),
                    None => vectors.push((slots[o].buf, k + 1, None)),
                }
            }
        }
        for o in operands {
            let slot = &slots[o];
            let page_slot = match entry(slot) {
                Some(k) => {
                    let v = vectors.iter().position(|(from, _, _)| *from == slot.buf).expect("every vector is measured");
                    let page = match vectors[v].2 {
                        Some(page) => page,
                        None => {
                            let page = moved(self, &format!("bounds{o}"), Slot { buf: slot.buf, map: at(0) }, vectors[v].1, sub(op(1), op(0)));
                            vectors[v].2 = Some(page);
                            page
                        }
                    };
                    Slot { buf: page, map: fixed(k as i64) }
                }
                None => Slot { buf: moved(self, &format!("bound{o}"), Slot { buf: slot.buf, map: slot.map.clone() }, 1, sub(op(1), op(0))), map: fixed(0) },
            };
            extra.push(page_slot);
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
        stages.push(self.compute(name, space(blk * tw), out, out_map(w), body, dst));
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

    /// [`Self::input`] of a tensor the program holds at `held` — a key or value cache at [`cache_precision`] — loaded into
    /// a page of the section's own precision, the load widening it on the way in.
    fn input_held(&mut self, name: &str, n: usize, fill: &Fill, held: Precision, target: &SpatialTarget) -> BufferId {
        self.input_stored(name, n, fill, held, self.precision, 2, target)
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
        self.input_stored(name, n, fill, precision, precision, depth, target)
    }

    /// A `1 × n` input stored at `stored`, loaded into a buffer of `depth` pages at `precision`.
    #[allow(clippy::too_many_arguments)]
    fn input_stored(
        &mut self,
        name: &str,
        n: usize,
        fill: &Fill,
        stored: Precision,
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
            stored,
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

    /// [`Self::finish_as`] for a chain whose buffers each feed EVERY later stage that reads them (a load or a stage's
    /// result fanned out), the result stored `out_precision`.
    #[allow(clippy::too_many_arguments)]
    fn finish_fanned(
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
        let readers = |t: &Template, b: BufferId, after: usize| -> Vec<StageId> {
            chain.iter().skip(after).copied().filter(|c| t.stage_reads(*c, b)).collect()
        };
        for (l, b) in std::mem::take(&mut self.loads) {
            let to = readers(&self.t, b, 0);
            if to.is_empty() {
                return Err(SpatialReject::NotLowered("an operand nothing in the chain reads"));
            }
            self.t.edge_many(l, b, to);
        }
        for i in 0..chain.len() {
            let b = self.t.stage_output(chain[i]);
            let to = readers(&self.t, b, i + 1);
            if !to.is_empty() {
                self.t.edge_many(chain[i], b, to);
            }
        }
        let cols_t = tiles_of(n, target);
        let y = self.t.tensor(out_name, TensorRole::Output, 1, n, out_precision, TensorInit::Zero);
        let st = self.t.store(&format!("store_{out_name}"), y, vec![], cols_t, out_buf);
        let producer = *chain
            .iter()
            .rev()
            .find(|c| self.t.stage_output(**c) == out_buf)
            .ok_or(SpatialReject::NotLowered("a stored buffer no stage of the chain produces"))?;
        self.t.edge(producer, out_buf, st);
        let acc = AccPolicy::for_result(self.precision);
        self.t.finish(name, whole_row(cols_t), acc, claim, target)
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

/// Bytes of local memory one site of a section holds: every buffer, at its declared depth.
fn site_footprint(p: &SpatialProgram) -> usize {
    p.buffers.iter().map(|b| p.buffer_bytes(b)).sum()
}

/// Whether one site holds a section: every buffer at its declared depth within the memory a site can be configured with
/// — the measure the tile pass refuses a section planned whole by.
pub(crate) fn fits_a_site(p: &SpatialProgram, target: &SpatialTarget) -> bool {
    site_footprint(p) <= target.max_local_mem_bytes()
}

/// ★ A WHOLE ROW ONE SITE CANNOT HOLD IS WORK FOR SEVERAL ITEMS. [`row_needs_blocking`] asks the accumulator's page; a
/// whole-row form also keeps its row, its intermediates and its result resident in one site, and on a part of small
/// tiles that is past the site's memory well before it is past the page — the RTX 3060's 16 × 16 tiles fill a block's
/// memory at 28 tiles of a row, inside its 64-tile page. `whole` stands where a site holds it; where none does, `pieces`
/// — the op's form that takes a head, a tile or a block of the row an item — computes the same values.
fn whole_row_or(
    whole: SpatialProgram,
    pieces: impl FnOnce() -> Result<SpatialProgram, SpatialReject>,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    if fits_a_site(&whole, target) {
        Ok(whole)
    } else {
        pieces()
    }
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

/// How many tiles of a whole-tile row ONE work item moves: the largest divisor of the row's tiles the
/// accumulator holds, since the row passes through the tile datapath one accumulator page at a time. A row that
/// fits is one item; Qwen3.8's `d = 5120` (160 tiles) is twenty items of eight.
fn row_block(row_t: usize, target: &SpatialTarget) -> usize {
    (1..=row_t.min(row_block_tiles(target)))
        .rev()
        .find(|d| row_t % d == 0)
        .unwrap_or(1)
}

/// ★ A COLUMN BLOCK IS WHAT A PAGE HOLDS WHERE A SITE HOLDS ITS SECTION, AND LESS WHERE IT DOES NOT. `build(blk)` is a
/// section over a row of `row_t` tiles in blocks of `blk`; [`row_block`]'s block, the largest divisor a page holds, is
/// kept where a site holds the section built with it, and otherwise the largest smaller divisor whose section a site
/// holds is — a part of small tiles (the RTX 3060's 16 × 16) runs out of a site's memory before its page. Each candidate
/// is built and measured as the tile pass measures it, so the sizing never disagrees with the section it sizes; where
/// none fits, the page's block stands and is refused there by name.
fn fitting_block(
    row_t: usize,
    build: &dyn Fn(usize) -> Result<SpatialProgram, SpatialReject>,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let first = row_block(row_t, target);
    let whole = build(first)?;
    if fits_a_site(&whole, target) {
        return Ok(whole);
    }
    for blk in (1..first).rev().filter(|d| row_t % d == 0) {
        let p = build(blk)?;
        if fits_a_site(&p, target) {
            return Ok(p);
        }
    }
    Ok(whole)
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

/// A `Generic` compute stage's kind.
fn generic_op(iters: Vec<IterSpec>, ins: Vec<ElemAccess>, out: ElemAccess, body: TensorBody) -> StageKind {
    StageKind::Compute(ComputeOp::Generic { iters, ins, out, body })
}
