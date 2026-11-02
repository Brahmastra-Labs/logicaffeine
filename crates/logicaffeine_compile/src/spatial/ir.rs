//! `SpatialProgram` — the spatial IR proper.
//!
//! Tensors live in global memory. Stages move tiles (`Load`/`Store`), receive multicast blocks (`Recv`),
//! compute on tiles (`Compute`) or run a scalar dataflow body (`Scalar`). Stages exchange tiles through
//! bounded [`Buffer`]s along typed [`Edge`]s. The `tile` pass fixes the per-site block plan, `place` pins
//! every stage to a [`Site`] and engine, `fuse_readers` introduces multicast [`Group`]s, `size_buffers`
//! fixes depths and offsets, `synthesize_handshakes` emits the per-engine [`Schedule`] of [`ChoreoOp`]s,
//! and `verify` attaches the certificate. Every phase marker is an `Option` in strict order — a program is
//! always in exactly one [`Phase`] and [`SpatialProgram::validate`] holds after every pass.
//!
//! There is deliberately NO strategy enum in this file: [`PlanSummary`] REPORTS the emergent plan (how many
//! multicast groups, which block shape, how many engine-program classes) from the structure itself.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::certificate::ChoreographyCertificate;
use super::target::{
    precision_bytes, precision_int_bits, DataflowPolicy, HandshakeKind, Interconnect, MeshShape,
    TileShape,
};
use crate::codegen_sva::fabric_rtl::{LoopCell, Src};
use crate::codegen_sva::fabric_target::Precision;

macro_rules! id_newtype {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub struct $name(pub u32);
        impl $name {
            pub fn idx(self) -> usize {
                self.0 as usize
            }
            pub fn new(i: usize) -> Self {
                $name(i as u32)
            }
        }
    };
}

id_newtype!(
    #[doc = "A global tensor."]
    TensorId
);
id_newtype!(
    #[doc = "A stage."]
    StageId
);
id_newtype!(
    #[doc = "A buffer."]
    BufferId
);
id_newtype!(
    #[doc = "An edge."]
    EdgeId
);
id_newtype!(
    #[doc = "A multicast group."]
    GroupId
);

/// WHICH RDU. A tensor-parallel program is one program over many chips; the chip is a COORDINATE of a
/// site, not a separate compilation. Absent (or zero) everywhere in a single-chip program, so every
/// artifact and fixture written before tensor parallelism serializes and parses byte-identically.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ChipId(pub u16);

fn chip_is_zero(c: &u16) -> bool {
    *c == 0
}

fn one_bank() -> usize {
    1
}

fn is_one_bank(b: &usize) -> bool {
    *b == 1
}
id_newtype!(
    #[doc = "A semaphore."]
    SemId
);
id_newtype!(
    #[doc = "A work item (one output block)."]
    WorkItemId
);
id_newtype!(
    #[doc = "A loop variable of the choreography."]
    LoopVar
);
id_newtype!(
    #[doc = "A loop count the program reads from its data ([`Trip`])."]
    TripId
);

/// A LOGICAL mesh coordinate. Physical (NOC) coordinates are a lowering concern and never enter the IR.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Site {
    pub x: u16,
    pub y: u16,
    /// The RDU this site is on. Zero for every single-chip program, and skipped when it is zero, so a
    /// one-chip placement serializes exactly as it did before tensor parallelism existed.
    #[serde(default, skip_serializing_if = "chip_is_zero")]
    pub chip: u16,
}

impl Site {
    pub fn new(x: usize, y: usize) -> Site {
        Site {
            x: x as u16,
            y: y as u16,
            chip: 0,
        }
    }
    /// The same mesh coordinate on another RDU.
    pub fn on_chip(chip: ChipId, x: usize, y: usize) -> Site {
        Site {
            x: x as u16,
            y: y as u16,
            chip: chip.0,
        }
    }
    pub fn chip_id(self) -> ChipId {
        ChipId(self.chip)
    }
}

/// An inclusive rectangle of sites.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rect {
    pub x0: u16,
    pub y0: u16,
    pub x1: u16,
    pub y1: u16,
}

impl Rect {
    pub fn contains(&self, s: Site) -> bool {
        s.x >= self.x0 && s.x <= self.x1 && s.y >= self.y0 && s.y <= self.y1
    }
    /// Sites in row-major order (y outer, x inner).
    pub fn sites(&self) -> Vec<Site> {
        let mut v = Vec::new();
        for y in self.y0..=self.y1 {
            for x in self.x0..=self.x1 {
                v.push(Site { x, y, chip: 0 });
            }
        }
        v
    }
    pub fn len(&self) -> usize {
        (self.x1 - self.x0 + 1) as usize * (self.y1 - self.y0 + 1) as usize
    }
}

/// Which engine of a site runs a stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Engine {
    Movement(u8),
    Compute,
}

/// A NOC channel index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NocId(pub u8);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TensorRole {
    Input,
    Output,
    Param,
    /// Produced by one section of a program and consumed by a later one — stored AND loaded inside a
    /// single program, which `Input` and `Output` each forbid. It starts a run EMPTY, like an output:
    /// a section that reads it before its producer wrote it faults, which is what makes the section
    /// barrier load-bearing rather than decorative.
    Activation,
    /// Carried ACROSS steps (the GDN memory, the conv ring, the KV cache): read at the start of a step
    /// and written once at its end. It starts a run FILLED, like an input, and leaves it in the
    /// outputs, so two steps of the same program chain through it.
    State,
}

impl TensorRole {
    /// Whether a run STARTS with this tensor's data present. An activation does not: it is produced
    /// inside the run, so reading it early is a fault, not a stale value.
    pub fn starts_filled(self) -> bool {
        matches!(
            self,
            TensorRole::Input | TensorRole::Param | TensorRole::State
        )
    }
    /// Whether a `Store` may write this tensor.
    pub fn is_written(self) -> bool {
        matches!(
            self,
            TensorRole::Output | TensorRole::Activation | TensorRole::State
        )
    }
    /// Whether a `Load` may read this tensor.
    pub fn is_read(self) -> bool {
        !matches!(self, TensorRole::Output)
    }
    /// Whether EVERY tile of this tensor must be written exactly once by the run. A state tensor is
    /// the one role that may be updated in part (a ring that shifts, a cache row that is appended),
    /// so it is held to "at most once" instead.
    pub fn is_fully_covered(self) -> bool {
        matches!(self, TensorRole::Output | TensorRole::Activation)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TensorSpace {
    Dram,
    Hbm,
    /// The on-chip tier a `State` or `Activation` tensor lives in to persist between sections (and
    /// between steps) without a round trip — the SN50's SRAM-to-SRAM streaming form.
    Sram,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Layout {
    RowMajorTiles,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ElemShape {
    pub rows: usize,
    pub cols: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TileGrid {
    pub rows_t: usize,
    pub cols_t: usize,
}

impl TileGrid {
    pub fn count(&self) -> usize {
        self.rows_t * self.cols_t
    }
}

/// How a tensor's contents are produced — recognized from the source's array-build loop, so a host can
/// reproduce the data and the numerics claim can bound the operand range.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TensorInit {
    Zero,
    /// `value[p] = (p % modulus) + shift`.
    Pattern {
        modulus: i64,
        shift: i64,
    },
    Values(Vec<i64>),
    /// Float values (as bits, so the init stays hashable).
    FloatValues(Vec<u64>),
    /// ★ ONE CONSTANT, NOT N COPIES OF IT. A tensor every element of which is the same float is a fact
    /// about the tensor, not `n` facts — but the only float fill used to be `FloatValues`, so declaring
    /// one cost a `Vec` as long as the tensor. On Qwen3.8 that is not a rounding error: a single
    /// full-width layer materialized 386,754,784 elements — 3.1 GB — and the real 64-layer model with its
    /// 5120 x 248320 output projection exhausted 114 GB of a 125 GB box before it finished lowering.
    /// Held symbolically the same declaration is eight bytes.
    ConstF(u64),
    /// A vendor-style bf16 random fill (seed + shift in thousandths) — carried so a recorded PCC is reproducible.
    RandomBf16 {
        seed: u32,
        shift_milli: i32,
    },
    /// Supplied at run time.
    External,
}

impl TensorInit {
    /// A bound on `|value|`, when the fill makes one derivable.
    pub fn abs_bound(&self) -> Option<i64> {
        match self {
            TensorInit::Zero => Some(0),
            TensorInit::Pattern { modulus, shift } => {
                let lo = *shift;
                let hi = (modulus - 1) + shift;
                Some(lo.abs().max(hi.abs()))
            }
            TensorInit::Values(v) => Some(v.iter().map(|x| x.abs()).max().unwrap_or(0)),
            TensorInit::FloatValues(_)
            | TensorInit::ConstF(_)
            | TensorInit::RandomBf16 { .. }
            | TensorInit::External => None,
        }
    }
    /// The least and the greatest value of an integer fill.
    pub fn range(&self) -> Option<(i64, i64)> {
        match self {
            TensorInit::Zero => Some((0, 0)),
            TensorInit::Pattern { modulus, shift } => Some((*shift, (modulus - 1) + shift)),
            TensorInit::Values(v) => Some((v.iter().copied().min().unwrap_or(0), v.iter().copied().max().unwrap_or(0))),
            TensorInit::FloatValues(_)
            | TensorInit::ConstF(_)
            | TensorInit::RandomBf16 { .. }
            | TensorInit::External => None,
        }
    }
    /// The element at flat position `p`, for the deterministic integer fills.
    pub fn element(&self, p: usize) -> Option<i64> {
        match self {
            TensorInit::Zero => Some(0),
            TensorInit::Pattern { modulus, shift } => Some((p as i64 % modulus) + shift),
            TensorInit::Values(v) => v.get(p).copied(),
            TensorInit::FloatValues(_)
            | TensorInit::ConstF(_)
            | TensorInit::RandomBf16 { .. }
            | TensorInit::External => None,
        }
    }
    /// The element at flat position `p` as a float, for every deterministic fill.
    pub fn element_f64(&self, p: usize) -> Option<f64> {
        match self {
            TensorInit::FloatValues(v) => v.get(p).map(|b| f64::from_bits(*b)),
            TensorInit::ConstF(b) => Some(f64::from_bits(*b)),
            other => other.element(p).map(|x| x as f64),
        }
    }
    pub fn is_float(&self) -> bool {
        matches!(self, TensorInit::FloatValues(_) | TensorInit::ConstF(_))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TensorDecl {
    pub id: TensorId,
    pub name: String,
    pub role: TensorRole,
    pub elems: ElemShape,
    pub tiles: TileGrid,
    pub dtype: Precision,
    pub layout: Layout,
    pub space: TensorSpace,
    pub init: TensorInit,
    /// Declared value range (inclusive), when known — feeds the numerics claim.
    pub range: Option<(i64, i64)>,
    /// Which RDU holds this tensor, once a shard has split it. `None` on every unsharded program, and on
    /// the replicated tensors (activations, norms) of a sharded one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chip: Option<ChipId>,
}

impl TensorDecl {
    pub fn tile_index(&self, row_t: usize, col_t: usize) -> usize {
        row_t * self.tiles.cols_t + col_t
    }
    /// A bound on `|value|` from the fill or the declared range.
    pub fn abs_bound(&self) -> Option<i64> {
        self.init
            .abs_bound()
            .or_else(|| self.range.map(|(lo, hi)| lo.abs().max(hi.abs())))
    }
}

/// A value-dependent tile address: the tile is chosen by an INDEX read from a buffer, not by the loop
/// variables — an embedding row, a KV-cache slot, a flash-attention block, a chosen expert's weights.
///
/// Semantics: the index `v` is element 0 of the `index` buffer's front page (the buffer is an operand of the
/// Load or Store; the schedule waits on it before the transfer and retires it after); the address gains
/// `(v div elems_per_tile) · tile_stride` tiles. A `v` outside `bound` (or not a whole number) is a fault in
/// the data run, never a wrapped row; the token-only dry run, which has no index to read, checks the tile
/// block at BOTH ends of `bound` against the tensor, so a certificate covers every index the bound admits.
/// Printers with no gather instruction refuse it by name (`SpatialProgram::has_indirect_access`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IndirectTile {
    /// The buffer whose front page carries the index.
    pub index: BufferId,
    /// Tiles per unit of index (a row of `tile_stride` tiles per indexed row).
    pub tile_stride: usize,
    /// Elements a unit of index spans — how the index is scaled before it is tile-aligned.
    pub elems_per_tile: usize,
    /// The declared inclusive range of the index; a gather without one earns no certificate (B-P5).
    pub bound: (usize, usize),
}

/// A tile id that is affine in the choreography's loop variables: `base + Σ env[var]·stride`, plus an
/// optional value-dependent term ([`IndirectTile`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TileAddr {
    pub base: usize,
    pub steps: Vec<(LoopVar, usize)>,
    #[serde(default)]
    pub indirect: Option<IndirectTile>,
}

impl TileAddr {
    pub fn constant(base: usize) -> TileAddr {
        TileAddr {
            base,
            steps: Vec::new(),
            indirect: None,
        }
    }
    /// The AFFINE part of the address. An indirect term is not resolved here — it has no value without
    /// the index buffer's data, which is why the evaluator owns that resolution.
    pub fn resolve(&self, env: &BTreeMap<LoopVar, usize>) -> usize {
        self.base
            + self
                .steps
                .iter()
                .map(|(v, s)| env.get(v).copied().unwrap_or(0) * s)
                .sum::<usize>()
    }
}

/// A rectangular block of tiles read or written per step: `block_h × block_w` tiles starting at `origin`,
/// consecutive tiles `stride_w` apart along a row and rows `stride_h` apart.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccessPattern {
    pub origin: TileAddr,
    pub block_h: usize,
    pub block_w: usize,
    pub stride_w: usize,
    pub stride_h: usize,
}

impl AccessPattern {
    pub fn block_tiles(&self) -> usize {
        self.block_h * self.block_w
    }

    /// The tiles the pattern addresses for the item at `coord` over its `sub_blocks`, in order — `None` when the
    /// address depends on anything else: a data index (`IndirectTile`), or a loop variable other than the item's
    /// coordinates and its sub-block.
    pub fn tiles_at(
        &self,
        coord: (usize, usize),
        sub_blocks: (usize, usize),
    ) -> Option<Vec<usize>> {
        use super::passes::{LV_ITEM_X, LV_ITEM_Y, LV_SUB_H, LV_SUB_W};
        if self.origin.indirect.is_some()
            || self
                .origin
                .steps
                .iter()
                .any(|(v, _)| ![LV_ITEM_Y, LV_ITEM_X, LV_SUB_H, LV_SUB_W].contains(v))
        {
            return None;
        }
        let mut tiles = Vec::with_capacity(sub_blocks.0 * sub_blocks.1 * self.block_tiles());
        for sh in 0..sub_blocks.0 {
            for sw in 0..sub_blocks.1 {
                let at = self
                    .origin
                    .steps
                    .iter()
                    .fold(self.origin.base, |a, (v, stride)| {
                        let value = match *v {
                            LV_ITEM_Y => coord.0,
                            LV_ITEM_X => coord.1,
                            LV_SUB_H => sh,
                            _ => sw,
                        };
                        a + value * stride
                    });
                for h in 0..self.block_h {
                    for w in 0..self.block_w {
                        tiles.push(at + h * self.stride_h + w * self.stride_w);
                    }
                }
            }
        }
        Some(tiles)
    }
}

/// The compute block shape in tiles: output sub-block `osh × osw`, inner-dimension block `ibw`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BlockShape {
    pub osh: usize,
    pub osw: usize,
    pub ibw: usize,
}

impl BlockShape {
    pub fn acc_tiles(&self) -> usize {
        self.osh * self.osw
    }
}

/// The accumulator format of a tile matmul.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AccPolicy {
    I32,
    I64,
    F32,
}

impl AccPolicy {
    /// The accumulator a program producing a `p` result folds in: a float result accumulates in fp32; an
    /// integer one in an accumulator as wide as the result's datapath — an INT32 result in an INT32
    /// accumulator, never a 64-bit one the part may not have.
    pub fn for_result(p: Precision) -> AccPolicy {
        match p {
            Precision::Int32 => AccPolicy::I32,
            Precision::Int8 | Precision::Int16 | Precision::Int64 => AccPolicy::I64,
            Precision::Fp8 | Precision::Bf16 | Precision::Fp16 | Precision::Fp32 => AccPolicy::F32,
        }
    }
    /// The narrowest precision that carries a spilled partial sum losslessly.
    pub fn interm_precision(self) -> Precision {
        match self {
            AccPolicy::I32 => Precision::Int32,
            AccPolicy::I64 => Precision::Int64,
            AccPolicy::F32 => Precision::Fp32,
        }
    }
    pub fn bits(self) -> u32 {
        match self {
            AccPolicy::I32 => 32,
            AccPolicy::I64 => 64,
            AccPolicy::F32 => 32,
        }
    }
}

/// Whether a partial sum may round when it spills to a narrower intermediate buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NumericsPolicy {
    Exact,
    AllowSpillRounding,
}

/// What the program claims about its result versus the exact integer reference.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NumericsClaim {
    Exact,
    Approx { reason: String },
}

/// Whether the matrix engine is driven one tile per call or one sub-block per call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Granularity {
    PerTile,
    PerSubBlock,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ComputeOp {
    MatmulTiles {
        block: BlockShape,
        /// Sub-blocks per site block: `(per_site_m / osh, per_site_n / osw)`.
        sub_blocks: (usize, usize),
        num_blocks: usize,
        operand: Precision,
        acc: AccPolicy,
        dataflow: DataflowPolicy,
        interm: Option<BufferId>,
        granularity: Granularity,
    },
    Requant {
        m: i64,
        s: u32,
    },
    Bias {
        bias: BufferId,
    },
    Relu,
    /// ★ THE GENERAL FORM — an iteration space, an affine access map per operand, and a scalar body.
    ///
    /// Everything the vector/SFPU path does is one of these. The two facts that make it general:
    ///
    ///   **A reduction is not an operator, it is an ITERATOR.** An axis marked
    ///   [`IterKind::Reduce`] is folded away and does not appear in `out`.
    ///
    ///   **A broadcast is not an operator, it is an ABSENT TERM.** An operand whose map omits an
    ///   axis is CONSTANT along it. A per-row scalar omits the column axis; a per-column weight
    ///   omits the row axis. Two operators (`RowRescale`, `RowBroadcast`) become one map read two
    ///   ways, and the printer recovers `mul_tiles_bcast_cols` vs `mul_tiles_bcast_rows` from which
    ///   axis is missing — which is the compiler doing the work instead of a human naming the case.
    ///
    /// The iteration space is at ELEMENT granularity (a row axis over the tile's 32 rows, a column
    /// axis over `col_tiles * 32`), because that is the only granularity at which a row reduction
    /// and a column broadcast are expressible at all; tile-level indices cannot say "constant along
    /// the columns inside a tile".
    Generic {
        iters: Vec<IterSpec>,
        /// One map per operand, positionally matching the stage's `ins`.
        ins: Vec<ElemAccess>,
        out: ElemAccess,
        body: TensorBody,
    },
    /// A scalar dataflow body applied element-wise over the input tiles (the choreography form of a
    /// `StageKind::Scalar` stage).
    Scalar(ScalarBody),
}

/// A [`ComputeOp`] as the shapes a printer that computes in software tells apart: a tile product, an elementwise map
/// over the acquired accumulator, the general form, a scalar body. Such a printer matches on what an op computes and
/// never on which epilogue it is. Requant, bias and ReLU are each an [`AccMap`](ComputeShape::AccMap), whose body it
/// prints as it prints a Generic's.
#[derive(Clone, Debug)]
pub enum ComputeShape<'a> {
    Tiles {
        block: &'a BlockShape,
        sub_blocks: (usize, usize),
        operand: Precision,
    },
    /// Element `e` of every accumulator tile `k` becomes `body`, its operand `i` the stage's `ins[i]` read as `reads[i]`
    /// says. Its arithmetic wraps at 64 bits, as the epilogues it stands for do.
    AccMap {
        body: TensorBody,
        reads: Vec<AccRead>,
    },
    Generic {
        iters: &'a [IterSpec],
        ins: &'a [ElemAccess],
        out: &'a ElemAccess,
        body: &'a TensorBody,
    },
    Scalar(&'a ScalarBody),
}

/// How an [`AccMap`](ComputeShape::AccMap) reads an operand for element `e` of accumulator tile `k`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccRead {
    /// Element `e` of the page's tile `k`.
    Tile,
    /// The element in `e`'s column of the page's tile `k mod n`, `n` the page's tiles: a row broadcast down every tile.
    RowOfTile,
}

impl ComputeOp {
    /// The op's [`ComputeShape`]. A requant's multiplier enters its body as a constant, which holds it exactly while it
    /// is within ±2⁵³.
    pub fn shape(&self) -> Result<ComputeShape<'_>, &'static str> {
        let konst = |v: i64| Box::new(TensorBody::Const((v as f64).to_bits()));
        let x = || Box::new(TensorBody::Operand(0));
        Ok(match self {
            ComputeOp::MatmulTiles { block, sub_blocks, operand, .. } => ComputeShape::Tiles {
                block,
                sub_blocks: *sub_blocks,
                operand: *operand,
            },
            ComputeOp::Requant { m, s } => {
                if (*m as f64) as i64 != *m {
                    return Err("a requant multiplier past ±2⁵³");
                }
                ComputeShape::AccMap {
                    body: TensorBody::Shr { arg: Box::new(TensorBody::Mul(konst(*m), x())), by: *s },
                    reads: vec![AccRead::Tile],
                }
            }
            ComputeOp::Bias { .. } => ComputeShape::AccMap {
                body: TensorBody::Add(x(), Box::new(TensorBody::Operand(1))),
                reads: vec![AccRead::Tile, AccRead::RowOfTile],
            },
            ComputeOp::Relu => ComputeShape::AccMap {
                body: TensorBody::Max(x(), konst(0)),
                reads: vec![AccRead::Tile],
            },
            ComputeOp::Generic { iters, ins, out, body } => ComputeShape::Generic { iters, ins, out, body },
            ComputeOp::Scalar(b) => ComputeShape::Scalar(b),
        })
    }
}

/// One axis of a [`ComputeOp::Generic`] iteration space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IterSpec {
    pub var: LoopVar,
    pub extent: usize,
    pub kind: IterKind,
}

/// Whether an axis survives into the output or is folded away.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IterKind {
    Parallel,
    /// Folded. An axis marked this way must NOT appear in the output's access map.
    Reduce {
        pool: ReducePool,
    },
}

/// A value-dependent element term: `(operand's value at this point) · stride`, taken modulo `modulus`
/// — the element-level gather (a top-k index into a row, a token id into a table row). Declared in
/// B-P3 as a type (validated: `operand` must be one of the stage's operands, `modulus` nonzero); its
/// evaluation and certificate semantics are B-P5, and until then the evaluator REFUSES a map that
/// carries one rather than dropping the term.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IndirectTerm {
    pub operand: usize,
    pub stride: i64,
    pub modulus: usize,
}

/// A FLAT, SIGNED affine map from the iteration space to one operand's element offset:
/// `base + Σ var·stride`, over the operand's page read as a row-major plane of `tile.rows` rows and
/// `page_tiles · tile.cols` columns. The evaluator recovers `(row, col) = (off / width, off % width)`,
/// and an offset outside the plane reads 0.0 — never a fault — so an unselected [`TensorBody::Select`]
/// branch may point past the page.
///
/// ★ AN ABSENT AXIS IS A BROADCAST. This is deliberately the same shape as [`TileAddr`], which has
/// described data MOVEMENT since the IR began (`base + Σ var·stride`, axes it does not mention being
/// axes it does not move along). Compute was the half that never got it, and that asymmetry is why
/// every new workload used to need a new operator.
///
/// ★ THE ROW STRIDE IS THE PAGE WIDTH. Because the offset is flat, the identity over a page `W`
/// elements wide is `[(row, W), (col, 1)]`, a per-row scalar is `[(row, W)]`, a per-column value is
/// `[(col, 1)]`, and a TRANSPOSE is `[(row, 1), (col, W)]` — a map, not an operator. B-P3 made the
/// map signed and flat exactly so that transpose, head slicing, a rope pair's `c ± hd/2` read and a
/// window shift are all maps; before it the evaluator read each axis' own term only, and an operand's
/// row could never depend on the column iterator.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct ElemAccess {
    pub base: i64,
    pub terms: Vec<(LoopVar, i64)>,
    #[serde(default)]
    pub indirect: Option<IndirectTerm>,
}

impl ElemAccess {
    /// The affine offset under `env` (the indirect term, if any, is the evaluator's to add).
    pub fn offset(&self, env: &BTreeMap<LoopVar, usize>) -> i64 {
        self.base
            + self
                .terms
                .iter()
                .map(|(v, s)| env.get(v).copied().unwrap_or(0) as i64 * s)
                .sum::<i64>()
    }
    /// The affine offset at one point `(r, c)` of a `(row, col)` iteration space.
    pub fn offset_at(&self, row: LoopVar, r: usize, col: LoopVar, c: usize) -> i64 {
        self.base
            + self
                .terms
                .iter()
                .map(|(v, s)| {
                    if *v == row {
                        r as i64 * s
                    } else if *v == col {
                        c as i64 * s
                    } else {
                        0
                    }
                })
                .sum::<i64>()
    }
    /// [`Self::offset_at`] at a point of a space with a contraction axis `k` at `x` as well.
    pub fn offset_at_k(
        &self,
        row: LoopVar,
        r: usize,
        col: LoopVar,
        c: usize,
        k: LoopVar,
        x: usize,
    ) -> i64 {
        self.offset_at(row, r, col, c)
            + self
                .terms
                .iter()
                .filter(|(v, _)| *v == k)
                .map(|(_, s)| x as i64 * s)
                .sum::<i64>()
    }
    /// Whether this operand varies along `var` — false is exactly "broadcast along `var`".
    pub fn varies_along(&self, var: LoopVar) -> bool {
        self.terms.iter().any(|(v, s)| *v == var && *s != 0)
    }
    /// The `(varies along row, varies along col)` class of a map that is EXACTLY one of the four
    /// tile-aligned broadcast forms over a page `width` elements wide — identity `[(row, W), (col, 1)]`,
    /// per-row `[(row, W)]`, per-column `[(col, 1)]`, constant `[]` — with base 0 and no indirect
    /// term. `None` for every other map (a transpose, a shifted window, a strided read, a gather).
    ///
    /// ★ THIS IS THE PRINTERS' GUARD. A printer that reads only [`varies_along`](Self::varies_along)
    /// would spell a transposed map as a tile-for-tile copy; a printer with no instruction for a
    /// general map must refuse it by name, and this is the predicate that says which maps those are.
    pub fn broadcast_class(
        &self,
        row: LoopVar,
        col: LoopVar,
        width: usize,
    ) -> Option<(bool, bool)> {
        if self.base != 0 || self.indirect.is_some() {
            return None;
        }
        let (mut row_seen, mut col_seen) = (false, false);
        for (v, s) in &self.terms {
            if *s == 0 {
                continue;
            }
            if *v == row && !row_seen && *s == width as i64 {
                row_seen = true;
            } else if *v == col && !col_seen && *s == 1 {
                col_seen = true;
            } else {
                return None;
            }
        }
        Some((row_seen, col_seen))
    }
}

/// The scalar expression a [`ComputeOp::Generic`] applies at each point of its iteration space.
///
/// Deliberately NOT [`ScalarBody`]: that is the CGRA's integer `LoopCell` datapath, which has no
/// home for a float transcendental. Where a body happens to be integer-expressible the fabric
/// lowering can convert it; otherwise it refuses by name, which is what it already does today.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TensorBody {
    /// Operand `i`, indexed through `ins[i]`.
    Operand(usize),
    /// A literal, as f64 bits so the op stays `Hash + Eq`.
    Const(u64),
    Add(Box<TensorBody>, Box<TensorBody>),
    Sub(Box<TensorBody>, Box<TensorBody>),
    Mul(Box<TensorBody>, Box<TensorBody>),
    /// The smaller and the larger of two values: a clamp is one of each, a ReLU a `Max` with 0.
    Min(Box<TensorBody>, Box<TensorBody>),
    Max(Box<TensorBody>, Box<TensorBody>),
    /// `⌊a / b⌋`, rounding toward −∞ — the VM's `//`. A zero divisor faults.
    FloorDiv(Box<TensorBody>, Box<TensorBody>),
    /// `a >> by`, an arithmetic shift (`⌊a / 2^by⌋`): an int8 model's requant divides by its power of two this way.
    Shr {
        arg: Box<TensorBody>,
        by: u32,
    },
    Unary {
        kind: SfpuKind,
        arg: Box<TensorBody>,
    },
    /// `cond ? then : otherwise` on an AXIS predicate — piecewise along the iteration space, decided
    /// by POSITION rather than by value, which no arithmetic on values can do. Both branches are
    /// evaluated (branch-free), so an unselected branch may read outside its page and yield 0.0. A
    /// rope window, a conv ring shift and the `j < pos` attention mask are each one of these.
    Select {
        cond: Pred,
        then: Box<TensorBody>,
        otherwise: Box<TensorBody>,
    },
}

impl TensorBody {
    /// Every node of the body, each after its operands and the left operand first — the order whose position names a
    /// node wherever a body's nodes are counted: its proven intervals, a run's observations, a datapath's wires.
    pub fn post_order(&self) -> Vec<&TensorBody> {
        fn walk<'a>(b: &'a TensorBody, out: &mut Vec<&'a TensorBody>) {
            match b {
                TensorBody::Operand(_) | TensorBody::Const(_) => {}
                TensorBody::Add(x, y)
                | TensorBody::Sub(x, y)
                | TensorBody::Mul(x, y)
                | TensorBody::Min(x, y)
                | TensorBody::Max(x, y)
                | TensorBody::FloorDiv(x, y) => {
                    walk(x, out);
                    walk(y, out);
                }
                TensorBody::Unary { arg, .. } | TensorBody::Shr { arg, .. } => walk(arg, out),
                TensorBody::Select { then, otherwise, .. } => {
                    walk(then, out);
                    walk(otherwise, out);
                }
            }
            out.push(b);
        }
        let mut out = Vec::new();
        walk(self, &mut out);
        out
    }
    /// The largest operand index the body names anywhere — in a leaf or in a predicate's bound — so a
    /// validator can refuse a body that would index past the stage's operands before an evaluator does.
    pub fn max_operand(&self) -> Option<usize> {
        match self {
            TensorBody::Operand(i) => Some(*i),
            TensorBody::Const(_) => None,
            TensorBody::Add(x, y)
            | TensorBody::Sub(x, y)
            | TensorBody::Mul(x, y)
            | TensorBody::Min(x, y)
            | TensorBody::Max(x, y)
            | TensorBody::FloorDiv(x, y) => x.max_operand().max(y.max_operand()),
            TensorBody::Unary { arg, .. } | TensorBody::Shr { arg, .. } => arg.max_operand(),
            TensorBody::Select {
                cond,
                then,
                otherwise,
            } => cond
                .operand()
                .max(then.max_operand())
                .max(otherwise.max_operand()),
        }
    }
    /// Whether a predicate anywhere in the body reads its bound from operand `i`.
    pub fn pred_names_operand(&self, i: usize) -> bool {
        match self {
            TensorBody::Operand(_) | TensorBody::Const(_) => false,
            TensorBody::Add(x, y)
            | TensorBody::Sub(x, y)
            | TensorBody::Mul(x, y)
            | TensorBody::Min(x, y)
            | TensorBody::Max(x, y)
            | TensorBody::FloorDiv(x, y) => x.pred_names_operand(i) || y.pred_names_operand(i),
            TensorBody::Unary { arg, .. } | TensorBody::Shr { arg, .. } => arg.pred_names_operand(i),
            TensorBody::Select {
                cond,
                then,
                otherwise,
            } => {
                cond.operand() == Some(i)
                    || then.pred_names_operand(i)
                    || otherwise.pred_names_operand(i)
            }
        }
    }
    /// Whether the body is integer arithmetic throughout: no transcendental, and every literal a whole number an
    /// `f64` holds exactly (below 2⁵³). Over integer operands such a body is evaluated in `i64`, exactly.
    pub fn integral(&self) -> bool {
        match self {
            TensorBody::Operand(_) => true,
            TensorBody::Const(bits) => {
                let v = f64::from_bits(*bits);
                v.fract() == 0.0 && v.abs() < 9_007_199_254_740_992.0
            }
            TensorBody::Add(x, y)
            | TensorBody::Sub(x, y)
            | TensorBody::Mul(x, y)
            | TensorBody::Min(x, y)
            | TensorBody::Max(x, y)
            | TensorBody::FloorDiv(x, y) => x.integral() && y.integral(),
            TensorBody::Shr { arg, by } => *by <= 63 && arg.integral(),
            TensorBody::Unary { .. } => false,
            TensorBody::Select {
                then, otherwise, ..
            } => then.integral() && otherwise.integral(),
        }
    }
}

/// An axis predicate: which side of a bound the current point of the iteration space is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Pred {
    /// `axis < bound`.
    AxisLt(LoopVar, PredRhs),
    /// `axis >= bound` — the complement, its own variant so a window's two halves are each one predicate.
    AxisGe(LoopVar, PredRhs),
}

impl Pred {
    pub fn var(&self) -> LoopVar {
        match self {
            Pred::AxisLt(v, _) | Pred::AxisGe(v, _) => *v,
        }
    }
    pub fn rhs(&self) -> PredRhs {
        match self {
            Pred::AxisLt(_, r) | Pred::AxisGe(_, r) => *r,
        }
    }
    /// The operand the bound is read from, when it is not a constant.
    pub fn operand(&self) -> Option<usize> {
        match self.rhs() {
            PredRhs::Operand(i) => Some(i),
            PredRhs::Const(_) => None,
        }
    }
    /// The same predicate with an operand-bound re-indexed through `shift` (fusion moves operands).
    pub fn map_operand(&self, shift: &dyn Fn(usize) -> usize) -> Pred {
        let rhs = match self.rhs() {
            PredRhs::Operand(i) => PredRhs::Operand(shift(i)),
            c => c,
        };
        match self {
            Pred::AxisLt(v, _) => Pred::AxisLt(*v, rhs),
            Pred::AxisGe(v, _) => Pred::AxisGe(*v, rhs),
        }
    }
}

/// The bound of an axis predicate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PredRhs {
    /// Fixed by the program — the rotary half-width, the conv tap count.
    Const(usize),
    /// Read from operand `i` at the current point (through its own map) — data-dependent, branch-free:
    /// `j < pos` follows the token position without recompiling the program.
    Operand(usize),
}

/// The SFPU's element-wise repertoire. A new transform is a new KIND, never a new `ComputeOp`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SfpuKind {
    Exp,
    Recip,
    /// `1/sqrt(x/over + eps)` — the divide happens ONCE, here. See `RowXformKind::Rsqrt` for why
    /// scale-once is part of the contract rather than something a reduction's scaler may absorb.
    Rsqrt {
        eps_nano: u64,
        over: usize,
    },
    /// `x · sigmoid(x)` — one fused SFPU instruction on Tensix (`silu_tile`).
    Silu,
    Relu,
    /// The natural logarithm — the inverse of `Exp`, which no composition of the others reaches. The
    /// GDN decay `(1+e^u)^R` is `exp(R·ln(1+e^u))`: a real power needs a real log.
    Ln,
}

/// The pooling function of a reduction (over whichever axis is marked [`IterKind::Reduce`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReducePool {
    Sum,
    Max,
    /// The VM's `>` scan, exactly: the first element is the running best, and a later one replaces it only
    /// when strictly greater — so ties go to the lowest index, a NaN first is never replaced, and a NaN later
    /// never replaces. A scan seeded by a constant instead is this fold over the constant followed by the
    /// row. An index is not a value: the stage's output buffer must be an integer precision.
    ArgMax,
}

/// How a reduced per-row scalar becomes the multiplier the rescale broadcasts back across the row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RowXformKind {
    /// `1/sqrt(s/over + eps)`.
    ///
    /// ★ THE DIVIDE LIVES HERE, ONCE, ON PURPOSE. It used to be folded into the reduction's scaler
    /// as `1/cols` — but the reduce engine applies its scaler to EVERY TERM, so the chain computed
    /// `Σ(xᵢ²·k)` where the specification computes `(Σxᵢ²)/cols`. Equal in exact arithmetic,
    /// measurably different in floating point: each term picks up its own rounding. On the RDU's
    /// bf16 datapath the legalized rmsnorm drifted a full ulp from the VM and
    /// `spatial_front_end_lock` caught it.
    ///
    /// Widening the intermediates to fp32 changed the answer but did not fix it — which is the
    /// lesson. A decomposition's numerics are not governed by intermediate PRECISION alone, but by
    /// which arithmetic is performed and where it narrows. Scale-once-at-the-end is part of the
    /// contract, so it is written into the op rather than left to a pass to remember.
    Rsqrt { eps_nano: u64, over: usize },
}

/// A scalar dataflow body (the CGRA's `LoopCell` datapath) applied element-wise over its input tiles.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ScalarBody {
    pub n_in: usize,
    pub cells: Vec<LoopCell>,
    pub result: Src,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StageKind {
    Load {
        tensor: TensorId,
        pattern: AccessPattern,
        num_blocks: usize,
        /// This load is the SENDER of a multicast group: after landing a block it multicasts it to the group.
        multicast: Option<GroupId>,
    },
    Store {
        tensor: TensorId,
        pattern: AccessPattern,
        sub_blocks: (usize, usize),
    },
    /// The receiver side of a multicast group: its output buffer is filled by the group's sender.
    Recv {
        group: GroupId,
    },
    /// CROSS-CHIP REDUCTION. Every chip of `group` holds a PARTIAL of `tensor` — the full output shape,
    /// summed over only its slice of the contraction axis. This stage combines them, and until it runs the
    /// tensor does not hold the answer.
    ///
    /// ★ ITS ARITHMETIC IS ALREADY IN THE CLOSED ALGEBRA. An all-reduce's combine is an N-ary elementwise
    /// fold — for `Sum`, `Add(Add(op0, op1), op2)…` — which `ComputeOp::Generic` expresses exactly. So
    /// `combine` is a Generic built from the pool and the operand count, and every consumer downstream
    /// (the scheduler, the evaluator, the cost model) sees an ordinary compute op rather than a new form.
    /// What is genuinely NEW here is the TOPOLOGY, not the arithmetic: this stage's operands live on
    /// different chips, which is what the certificate's `UnreducedShard` obligation is about.
    ///
    /// ★ AND THE MEMBERSHIP IS NOT STORED, because it cannot then disagree with the truth. The chips
    /// taking part are exactly the sites of the buffers in `ins` — `Group` is the multicast RECTANGLE
    /// abstraction and a rectangle of (x, y) cannot name a set of chips, so widening it here would stretch
    /// a type to carry something it was not built for. A derived answer beats a field that can drift.
    AllReduce {
        pool: ReducePool,
        tensor: TensorId,
        combine: Box<ComputeOp>,
    },
    Compute(ComputeOp),
    Scalar(ScalarBody),
}

impl StageKind {
    /// Rename every buffer the stage's KIND names beyond its ports — an indirect address's index buffer, a
    /// matmul's spill buffer, a bias. A pass that renumbers buffers maps these with `ins`/`outs`, or the stage
    /// silently points at somebody else's buffer.
    pub fn remap_buffers(&mut self, f: &dyn Fn(BufferId) -> BufferId) {
        match self {
            StageKind::Load { pattern, .. } | StageKind::Store { pattern, .. } => {
                if let Some(ind) = &mut pattern.origin.indirect {
                    ind.index = f(ind.index);
                }
            }
            StageKind::Compute(ComputeOp::MatmulTiles { interm, .. }) => *interm = interm.map(f),
            StageKind::Compute(ComputeOp::Bias { bias }) => *bias = f(*bias),
            _ => {}
        }
    }

    pub fn role(&self) -> SiteRole {
        match self {
            StageKind::Load { .. } | StageKind::Store { .. } | StageKind::Recv { .. } => {
                SiteRole::Movement
            }
            // The collective FOLDS its operands, so it occupies a compute engine like any other fold.
            StageKind::AllReduce { .. } => SiteRole::Compute,
            StageKind::Compute(_) | StageKind::Scalar(_) => SiteRole::Compute,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SiteRole {
    Compute,
    Movement,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stage {
    pub id: StageId,
    pub name: String,
    pub kind: StageKind,
    pub role: SiteRole,
    pub ins: Vec<BufferId>,
    pub outs: Vec<BufferId>,
    pub site: Option<Site>,
    pub item: Option<WorkItemId>,
    /// The template stage this per-site instance was cloned from.
    pub origin: Option<StageId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MemSpace {
    Local,
    Acc,
    Pmu,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum BufferKind {
    Fifo {
        depth_pages: usize,
        page_tiles: usize,
    },
    /// A spill buffer for partial sums (a FIFO of sub-block pages the compute stage both produces and consumes).
    Interm {
        depth_pages: usize,
        page_tiles: usize,
    },
    Accumulator {
        tiles: usize,
    },
}

impl BufferKind {
    pub fn tiles(&self) -> usize {
        match self {
            BufferKind::Fifo {
                depth_pages,
                page_tiles,
            }
            | BufferKind::Interm {
                depth_pages,
                page_tiles,
            } => depth_pages * page_tiles,
            BufferKind::Accumulator { tiles } => *tiles,
        }
    }
    /// Pages a FIFO-like buffer holds.
    pub fn depth_pages(&self) -> usize {
        match self {
            BufferKind::Fifo { depth_pages, .. } | BufferKind::Interm { depth_pages, .. } => {
                *depth_pages
            }
            BufferKind::Accumulator { .. } => 1,
        }
    }
    pub fn page_tiles(&self) -> usize {
        match self {
            BufferKind::Fifo { page_tiles, .. } | BufferKind::Interm { page_tiles, .. } => {
                *page_tiles
            }
            BufferKind::Accumulator { tiles } => *tiles,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Buffer {
    pub id: BufferId,
    pub name: String,
    pub kind: BufferKind,
    pub dtype: Precision,
    pub space: MemSpace,
    pub site: Option<Site>,
    pub origin: Option<BufferId>,
    /// This buffer shares its memory region with another (the vendor's interm-aliases-output trick); the
    /// schedule must then carry the guard that makes the aliasing safe.
    pub alias_of: Option<BufferId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Handshake {
    Fifo,
    Semaphore {
        ready: SemId,
        valid: SemId,
        ready_count: usize,
    },
    Barrier {
        group: GroupId,
    },
    ValidReady,
    Token,
}

impl Handshake {
    pub fn kind(&self) -> HandshakeKind {
        match self {
            Handshake::Fifo => HandshakeKind::Fifo,
            Handshake::Semaphore { .. } => HandshakeKind::Semaphore,
            Handshake::Barrier { .. } => HandshakeKind::Barrier,
            Handshake::ValidReady => HandshakeKind::ValidReady,
            Handshake::Token => HandshakeKind::Token,
        }
    }
    /// A same-site producer→consumer hand-off (the buffer has exactly one producer and one consumer).
    pub fn is_local(&self) -> bool {
        matches!(self, Handshake::Fifo | Handshake::ValidReady)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub id: EdgeId,
    pub from: StageId,
    pub buffer: BufferId,
    pub to: Vec<StageId>,
    pub handshake: Handshake,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TileRect {
    pub row0_t: usize,
    pub col0_t: usize,
    pub h_t: usize,
    pub w_t: usize,
}

impl TileRect {
    pub fn tiles(&self, cols_t: usize) -> Vec<usize> {
        let mut v = Vec::with_capacity(self.h_t * self.w_t);
        for r in self.row0_t..self.row0_t + self.h_t {
            for c in self.col0_t..self.col0_t + self.w_t {
                v.push(r * cols_t + c);
            }
        }
        v
    }
}

/// Where a stage runs in its item's fold over K blocks — see [`SpatialProgram::fold_levels`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FoldLevel {
    /// Once per item, before the blocks: what reads only values fixed for the item, and each carry's init.
    Prologue,
    /// Once per K block: what reads a value that moves with the block, and each carry's update.
    Body,
    /// Once per item, after the blocks: what reads the carried values the blocks leave.
    Epilogue,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItem {
    pub id: WorkItemId,
    /// `(oy, ox)` in blocks.
    pub coord: (usize, usize),
    pub out: TileRect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InnerLoop {
    pub num_blocks: usize,
    pub block_k_tiles: usize,
}

/// The per-site block plan the `tile` pass fixes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlockPlan {
    pub per_site_m: usize,
    pub per_site_n: usize,
    /// `(n_y, n_x)` blocks.
    pub grid: (usize, usize),
    pub inner: InnerLoop,
    pub block: BlockShape,
    pub acc: AccPolicy,
    /// The packer accumulates partial sums in place in the intermediate buffer, so the register file is
    /// reloaded once (at the last K block) instead of once per block.
    pub pack_accumulate: bool,
    /// How the site's movement work is packed onto its engines.
    pub engines: EnginePacking,
    /// Whether the compute engine is handed a whole sub-block per call or one tile at a time.
    pub granularity: Granularity,
}

impl BlockPlan {
    /// ★ THE BLOCK THE SITE AT GRID POSITION `(i, j)` ACTUALLY COVERS — one definition, shared.
    ///
    /// A grid extent need not divide the tile count: the last row and column carry what is LEFT. `place`
    /// emits each site's patterns from this and `work` prices them from the same function, because two
    /// copies of a boundary rule disagree the first time either moves — and a planner that ranks by a
    /// closed form the emitter does not honour is the defect this campaign already removed once.
    ///
    /// The sub-block must DIVIDE the extent it walks, or the store writes past a row and wraps into the
    /// next. Raggedness must not compound, so the largest divisor of the extent that is no bigger than the
    /// nominal sub-block is taken; 1 always divides, so a legal choice always exists.
    ///
    /// Returns `(rows, cols, sub_block_h, sub_block_w)`. On a grid whose extents DO divide, every site
    /// returns the nominal block and this is the identity.
    pub fn site_block(
        &self,
        mt: usize,
        nt: usize,
        i: usize,
        j: usize,
    ) -> (usize, usize, usize, usize) {
        let h = self.per_site_m.min(mt.saturating_sub(i * self.per_site_m));
        let w = self.per_site_n.min(nt.saturating_sub(j * self.per_site_n));
        let fit = |nominal: usize, extent: usize| -> usize {
            (1..=nominal.min(extent))
                .rev()
                .find(|d| extent % d == 0)
                .unwrap_or(1)
        };
        (h, w, fit(self.block.osh, h), fit(self.block.osw, w))
    }

    /// Every `(i, j)` in the grid that carries work, with its block — the boundary included.
    pub fn site_blocks(
        &self,
        mt: usize,
        nt: usize,
    ) -> Vec<((usize, usize), (usize, usize, usize, usize))> {
        let (ny, nx) = self.grid;
        let mut v = Vec::with_capacity(ny * nx);
        for i in 0..ny {
            for j in 0..nx {
                let b = self.site_block(mt, nt, i, j);
                if b.0 > 0 && b.1 > 0 {
                    v.push(((i, j), b));
                }
            }
        }
        v
    }
}

/// Which movement engine carries which stream. Engines run concurrently, so the packing decides what overlaps
/// what: keeping both operand loads together serialises them against each other, while giving one operand an
/// engine to itself and pairing the other with the store overlaps block k's output write with block k+1's
/// operand reads. Both are legal on any target with two movement engines; which one wins is measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EnginePacking {
    /// Both loads on one engine, the store on the other.
    LoadsTogether,
    /// The first operand alone on one engine; the second operand and the store share the other.
    OperandWithStore,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkList {
    pub items: Vec<WorkItem>,
    pub k_tiles: usize,
    pub plan: Option<BlockPlan>,
    /// The trip the plan's K blocks run to: every K-block loop the handshakes synthesize counts it, at most the plan's
    /// `num_blocks`. `None`: every loop runs all of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub k_trip: Option<TripId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Direction {
    Row,
    Col,
}

#[derive(PartialOrd, Ord, Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Operand {
    A,
    B,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub id: GroupId,
    pub rect: Rect,
    pub sender: Site,
    pub direction: Direction,
    pub operand: Operand,
    pub noc: NocId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineSlot {
    pub site: Site,
    pub engine: Engine,
    pub noc: Option<NocId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    /// The mesh the program was placed onto (the target's deployment grid).
    pub mesh: MeshShape,
    /// By stage id.
    pub site_of: Vec<Site>,
    /// By stage id.
    pub engine_of: Vec<EngineSlot>,
    pub groups: Vec<Group>,
    pub items_of_site: Vec<(Site, Vec<WorkItemId>)>,
}

impl Placement {
    pub fn items_at(&self, site: Site) -> &[WorkItemId] {
        self.items_of_site
            .iter()
            .find(|(s, _)| *s == site)
            .map(|(_, v)| v.as_slice())
            .unwrap_or(&[])
    }
    pub fn sites(&self) -> BTreeSet<Site> {
        self.site_of.iter().copied().collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BufferAt {
    pub site: Site,
    pub space: MemSpace,
    pub offset_bytes: u32,
    pub bytes: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryMap {
    /// By buffer id.
    pub buffer_at: Vec<BufferAt>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SemRole {
    ReadyCounter {
        expected: usize,
    },
    ValidFlag,
    /// One half of a section [`Rendezvous`] joining `n` engine programs.
    Barrier {
        n: usize,
        phase: BarrierPhase,
    },
}

/// The two halves of a centralized barrier over `n` engine programs.
///
/// A rendezvous in which every program posts at every participating site costs `n · sites` posts and as
/// many marked-graph places; gathering at one root and releasing from it costs `2n + sites + 1` ops and
/// `n + sites` places, and orders exactly the same events.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BarrierPhase {
    /// Held at ONE root site. Each of the `n` programs posts one arrival there; the root's own program
    /// waits once, for `n`.
    Gather,
    /// Held at every participating site. The root's program posts one release at each site after its
    /// gather wait; each of the `n` programs waits once, for 1, at its own site.
    Release,
}

/// The rendezvous that closes a section: its gather and its release semaphore.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rendezvous {
    pub gather: SemId,
    pub release: SemId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Semaphore {
    pub id: SemId,
    pub init: u32,
    pub role: SemRole,
    /// The multicast operand this semaphore serves: ONE pair (ready counter, valid flag) per operand is
    /// shared by every group of that operand — a site belongs to one row group and one column group, so
    /// the same handle serves all of them (a Tensix core holds only a handful of semaphore slots).
    pub operand: Option<Operand>,
    /// Every site that holds an instance.
    pub sites: Vec<Site>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SemValue {
    Invalid,
    Valid,
    Count(usize),
}

impl SemValue {
    pub fn as_u32(self) -> u32 {
        match self {
            SemValue::Invalid => 0,
            SemValue::Valid => 1,
            SemValue::Count(n) => n as u32,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SemCond {
    Equals(usize),
    AtLeast(usize),
}

/// How many times a loop runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LoopCount {
    /// Fixed when the program is built.
    Const(usize),
    /// The count of the program's trip `trip`, read from its data before it runs; `max` is the trip's greatest
    /// count ([`Trip::hi`]), the bound every static question — a buffer's traffic, a walk's multiplicity — takes.
    Trip { trip: TripId, max: usize },
}

impl LoopCount {
    /// The most times the loop runs: its count when fixed, its trip's greatest count otherwise.
    pub fn bound(self) -> usize {
        match self {
            LoopCount::Const(n) => n,
            LoopCount::Trip { max, .. } => max,
        }
    }

    /// The times the loop runs when each trip `t` counts `counts[t]`.
    pub fn at(self, counts: &[usize]) -> usize {
        match self {
            LoopCount::Const(n) => n,
            LoopCount::Trip { trip, .. } => counts[trip.idx()],
        }
    }

    /// The trip the loop runs to, when its count is read from the data.
    pub fn trip(self) -> Option<TripId> {
        match self {
            LoopCount::Const(_) => None,
            LoopCount::Trip { trip, .. } => Some(trip),
        }
    }
}

/// ★ A LOOP COUNT THE PROGRAM READS FROM ITS OWN DATA. Element `elem` of the input `tensor` — a program argument, set
/// before the program runs and never written by it — gives `v`, and every loop run to the trip runs
/// `n = ⌈(v + bias) / per⌉` times. The program admits the values `admit.0 ..= admit.1`, stated in the value's own
/// units — a position against the cache it indexes — so a value outside them is refused before anything runs, by the
/// same range the program's own checks hold it to, never as a clamped or wrapped count. The counts run from
/// [`Trip::lo`] to [`Trip::hi`], the bound every static question takes (a loop's [`LoopCount::Trip::max`] is exactly
/// it).
///
/// It is the loop bound a Plasticine or Spatial controller takes from a scalar argument, and the one a GPU decode
/// kernel reads from its sequence length: a decode step at position `P` folds the `⌈P / tile⌉` cache blocks its
/// position reaches instead of every block the cache is declared with.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Trip {
    pub tensor: TensorId,
    pub elem: usize,
    pub admit: (i64, i64),
    pub bias: i64,
    pub per: usize,
}

impl Trip {
    /// `⌈(v + bias) / per⌉` — a divisor of zero counts nothing.
    fn count(&self, v: i64) -> i128 {
        if self.per == 0 {
            return 0;
        }
        let (x, per) = (v as i128 + self.bias as i128, self.per as i128);
        if x <= 0 {
            -((-x) / per)
        } else {
            (x + per - 1) / per
        }
    }

    /// The least count: the least admitted value's.
    pub fn lo(&self) -> usize {
        self.count(self.admit.0).max(0) as usize
    }

    /// The greatest count: the greatest admitted value's.
    pub fn hi(&self) -> usize {
        self.count(self.admit.1).max(0) as usize
    }

    /// The count the value `v` gives, or `None` when `v` is not a whole number or not admitted.
    pub fn count_of(&self, v: f64) -> Option<usize> {
        if v.fract() != 0.0 || !v.is_finite() || v < self.admit.0 as f64 || v > self.admit.1 as f64
        {
            return None;
        }
        Some(self.count(v as i64) as usize)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ArchCond {
    Always,
    Blackhole,
}

/// One step of a site engine's choreography.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChoreoOp {
    Reserve {
        buf: BufferId,
        pages: usize,
    },
    Push {
        buf: BufferId,
        pages: usize,
    },
    Wait {
        buf: BufferId,
        pages: usize,
    },
    Pop {
        buf: BufferId,
        pages: usize,
    },
    ReadTiles {
        tensor: TensorId,
        at: TileAddr,
        block_h: usize,
        block_w: usize,
        stride_w: usize,
        stride_h: usize,
        into: BufferId,
    },
    ReadBarrier,
    WriteTiles {
        from: BufferId,
        tensor: TensorId,
        at: TileAddr,
        block_h: usize,
        block_w: usize,
        stride_w: usize,
        stride_h: usize,
    },
    WriteBarrier,
    /// Broadcast a buffer's pages to a group. `hold_path` keeps the reserved broadcast path open for the
    /// transfer that immediately follows to the same rectangle — the flag that tells the receivers the data
    /// has landed. Without it that four-byte flag re-arbitrates and re-reserves the whole rectangle, once
    /// per K block per operand on every sender.
    McastSend {
        buf: BufferId,
        pages: usize,
        group: GroupId,
        hold_path: bool,
    },
    McastSetValid {
        sem: SemId,
        group: GroupId,
    },
    Flush {
        cond: ArchCond,
    },
    SemSet {
        sem: SemId,
        value: SemValue,
    },
    SemInc {
        sem: SemId,
        at: Site,
        by: usize,
    },
    SemWait {
        sem: SemId,
        until: SemCond,
    },
    AcquireAcc {
        tiles: usize,
    },
    ReleaseAcc,
    Compute {
        op: ComputeOp,
        ins: Vec<BufferId>,
        acc_reload: Option<BufferId>,
    },
    /// Write the accumulator's tiles into a page. `accumulate` makes the packer ADD into the page already
    /// there instead of overwriting it — the partial sum then never leaves local memory between K blocks, so
    /// a blocked plan reloads the register file once instead of once per block.
    PackAcc {
        to: BufferId,
        tiles: usize,
        convert: Precision,
        accumulate: bool,
    },
    Barrier {
        group: GroupId,
    },
    /// A counted loop whose body ops stay TAGGED with their stages (attribution survives nesting).
    Loop {
        var: LoopVar,
        count: LoopCount,
        body: Vec<TaggedOp>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaggedOp {
    pub stage: StageId,
    pub op: ChoreoOp,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineProgram {
    pub site: Site,
    pub engine: Engine,
    pub stages: Vec<StageId>,
    pub ops: Vec<TaggedOp>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    pub programs: Vec<EngineProgram>,
}

impl Schedule {
    pub fn program_at(&self, site: Site, engine: Engine) -> Option<&EngineProgram> {
        self.programs
            .iter()
            .find(|p| p.site == site && p.engine == engine)
    }
    /// The ops of one stage, in program order, loop bodies included (a per-stage view of the per-engine
    /// schedule).
    pub fn per_stage(&self, stage: StageId) -> Vec<&ChoreoOp> {
        fn collect<'a>(ops: &'a [TaggedOp], stage: StageId, out: &mut Vec<&'a ChoreoOp>) {
            for t in ops {
                if t.stage == stage {
                    out.push(&t.op);
                }
                if let ChoreoOp::Loop { body, .. } = &t.op {
                    collect(body, stage, out);
                }
            }
        }
        let mut out = Vec::new();
        for p in &self.programs {
            collect(&p.ops, stage, &mut out);
        }
        out
    }
}

/// What a pass recorded the program as needing from its target (kept on the program so `validate` stays
/// target-free while the pipeline gates the requirements against the descriptor).
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Requirements {
    pub handshakes: BTreeSet<HandshakeKind>,
    pub interconnect: BTreeSet<Interconnect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Phase {
    Template,
    Tiled,
    Placed,
    Sized,
    Scheduled,
    Verified,
}

/// A structural invariant the program breaks. Every arm is a lock.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum IrViolation {
    IdNotDense {
        what: &'static str,
        index: usize,
    },
    DanglingTensor {
        stage: StageId,
        tensor: TensorId,
    },
    DanglingBuffer {
        stage: Option<StageId>,
        buffer: BufferId,
    },
    DanglingStage {
        edge: EdgeId,
        stage: StageId,
    },
    DanglingGroup {
        stage: StageId,
        group: GroupId,
    },
    DanglingSemaphore {
        sem: SemId,
    },
    DanglingItem {
        stage: StageId,
        item: WorkItemId,
    },
    EdgeBufferNotInStagePorts {
        edge: EdgeId,
        stage: StageId,
        buffer: BufferId,
    },
    FifoProducerCount {
        buffer: BufferId,
        count: usize,
    },
    FifoConsumerCount {
        buffer: BufferId,
        count: usize,
    },
    /// A carried buffer — one stage reads and writes it — that no other stage writes first.
    CarryWithoutInit {
        buffer: BufferId,
    },
    /// A carried buffer more than one stage updates, or more than one stage initializes.
    CarryUpdatedTwice {
        buffer: BufferId,
    },
    /// A carry's init that reads a value moving with the K blocks: the first page would differ per block.
    CarryInitMovesWithTheBlocks {
        buffer: BufferId,
    },
    /// A carry that is not a scratch ring of two pages — the value before a block's update and the value after.
    CarryNotARing {
        buffer: BufferId,
    },
    LoadReadsOutput {
        stage: StageId,
        tensor: TensorId,
    },
    StoreWritesInput {
        stage: StageId,
        tensor: TensorId,
    },
    TileGridMismatch {
        tensor: TensorId,
    },
    OutputUncovered {
        tensor: TensorId,
        tile: usize,
    },
    OutputOverlap {
        tensor: TensorId,
        tile: usize,
    },
    OperandDtypeMismatch {
        stage: StageId,
        buffer: BufferId,
        expected: Precision,
        found: Precision,
    },
    IntermNarrowerThanAcc {
        stage: StageId,
        interm: BufferId,
        acc: AccPolicy,
        found: Precision,
    },
    AccTilesExceeded {
        stage: StageId,
        needed: usize,
        available: usize,
    },
    BlockDivisibility {
        what: &'static str,
        value: usize,
        divisor: usize,
    },
    GridMismatch {
        expected: (usize, usize),
        found: (usize, usize),
    },
    PhaseOrder {
        have: &'static str,
        missing: &'static str,
    },
    StageUnplaced {
        stage: StageId,
    },
    EngineRoleMismatch {
        stage: StageId,
    },
    EngineSiteMismatch {
        stage: StageId,
    },
    GroupMalformed {
        group: GroupId,
        reason: &'static str,
    },
    GroupMemberMismatch {
        group: GroupId,
        site: Site,
        reason: &'static str,
    },
    CrossSiteLocalEdge {
        edge: EdgeId,
    },
    LocalCrossSiteEdge {
        edge: EdgeId,
    },
    MemoryOverlap {
        site: Site,
        a: BufferId,
        b: BufferId,
    },
    MemoryUnmapped {
        buffer: BufferId,
    },
    DuplicateEngineProgram {
        site: Site,
        engine: Engine,
    },
    CountingImbalance {
        buffer: BufferId,
        reserved: u64,
        pushed: u64,
        popped: u64,
    },
    /// An accumulator was acquired and packed with no compute in between (a skipped compute is never a
    /// legal choreography: the packed tiles would be whatever the register file held).
    AccumulatorNeverComputed {
        site: Site,
        engine: Engine,
    },
    SemaphoreImbalance {
        sem: SemId,
        incs: u64,
        waits: u64,
    },
    /// A counting semaphore was waited on without being put back to zero for the next round — the next
    /// wait would be satisfied by a count that has already been consumed.
    SemaphoreNotReset {
        sem: SemId,
        waits: u64,
        resets: u64,
    },
    AccImbalance {
        site: Site,
        acquires: u64,
        releases: u64,
    },
    HandshakeNotRequired {
        kind: HandshakeKind,
    },
    ItemCountMismatch {
        group: GroupId,
    },
    /// A stage the output-coverage proof credits issues no data movement in the schedule. Coverage is
    /// proven over the WORK LIST; the schedule is what runs.
    StageNotScheduled {
        stage: StageId,
        op: &'static str,
    },
    /// A `Generic` whose access maps do not match its operands one for one.
    GenericOperandCount {
        stage: StageId,
        operands: usize,
        maps: usize,
    },
    /// A `Generic` body (or a predicate's bound) naming an operand the stage does not have — the
    /// evaluator indexes pages by it, and a panic there is not a refusal.
    GenericOperandOutOfRange {
        stage: StageId,
        operand: usize,
        operands: usize,
    },
    /// A `Generic` whose contraction axis — the third iteration variable, `LV_GEN_K` — is malformed: not a sum fold,
    /// over another variable, stepped along by the output or by a map in a space without it, or a fourth axis.
    ContractionMalformed {
        stage: StageId,
        reason: &'static str,
    },
    /// An indirect term whose index operand the stage does not have.
    IndirectOperandOutOfRange {
        stage: StageId,
        operand: usize,
        operands: usize,
    },
    /// An indirect term or tile with a field that cannot mean anything (a zero modulus or stride, an
    /// inverted bound).
    IndirectMalformed {
        stage: StageId,
        reason: &'static str,
    },
    /// A trip that cannot count anything: read from what is not a program input or past its tensor, an empty or
    /// zero-based range, a zero divisor, or a trip no loop runs to.
    TripMalformed {
        trip: TripId,
        reason: &'static str,
    },
    /// A loop that misuses a trip: runs to one the program does not declare, is bounded apart from it, nests inside
    /// another trip loop, or holds a semaphore or multicast op — the forms the certificate's trip window
    /// ([`crate::spatial::certificate::trip_window`]) does not cover.
    TripLoopMalformed {
        reason: &'static str,
    },
    /// An OUTPUT map with an indirect term — a scatter — on a stage whose result reaches no Store with
    /// an indirect tile address to scatter through.
    IndirectOutWithoutScatter {
        stage: StageId,
    },
    /// An indirect tile address whose index buffer is not one of the stage's operands.
    IndirectIndexNotAnOperand {
        stage: StageId,
        buffer: BufferId,
    },
    /// An index-valued (`ArgMax`) reduction packed into a non-integer buffer.
    ArgMaxOutputNotInteger {
        stage: StageId,
        found: Precision,
    },
    /// A tensor role used outside the contract that gives it meaning; `until` names the phase that
    /// owns that contract (`B-P7` sections for an activation, `B-P6` the cross-token carry for state).
    TensorRoleDeferred {
        tensor: TensorId,
        role: TensorRole,
        until: &'static str,
    },
    /// A tensor space used outside the contract that gives it meaning; `until` names the phase.
    TensorSpaceDeferred {
        tensor: TensorId,
        space: TensorSpace,
        until: &'static str,
    },
    /// A section span that is not a scope: an id range that overlaps its neighbour, runs backwards, or
    /// leaves part of an id space in no section at all.
    SectionSpanMalformed {
        section: usize,
        what: &'static str,
    },
    /// A section boundary whose barrier is not a rendezvous: the participants that post it and the
    /// engine programs that wait on it disagree.
    SectionBarrierUnbalanced {
        sem: SemId,
        participants: usize,
        sites: usize,
        incs: u64,
        wait_ops: u64,
    },
    /// A rendezvous op that does not sit on its section boundary in one engine program, so the barrier
    /// balances but does not order that program's work.
    SectionBarrierMisplaced {
        section: usize,
        site: Site,
        engine: Engine,
        what: &'static str,
    },
    /// A stage of one section reads or writes a buffer that belongs to another — sections share
    /// tensors, never local memory.
    SectionBufferEscape {
        stage: StageId,
        buffer: BufferId,
        stage_section: usize,
        buffer_section: usize,
    },
}

/// The emergent plan, REPORTED from structure — never an input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanSummary {
    pub phase: Phase,
    pub sites_used: usize,
    pub grid: (usize, usize),
    pub per_site_block: (usize, usize),
    pub block: Option<BlockShape>,
    pub num_blocks: usize,
    pub streaming: bool,
    pub multicast_groups: usize,
    pub row_groups: usize,
    pub col_groups: usize,
    pub acc: Option<AccPolicy>,
    pub dram_tiles_read: u64,
    pub semaphores: usize,
    pub fifos: usize,
    pub engine_program_classes: usize,
    /// How the site's movement work is packed onto its engines (part of the arrangement a ledger row names).
    pub engines: EnginePacking,
}

/// One of a tensor's two tile-grid extents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DimAxis {
    Rows,
    Cols,
}

/// One SECTION of a composed program: the half-open range this section owns in every id space, the plan
/// and work list it was lowered under, the tensor it writes, and the rendezvous every engine passes
/// through before the next section begins.
///
/// ★ A SECTION IS A SCOPE, NOT A LABEL. Three invariants read the spans rather than the whole program,
/// and each is wrong without them: output coverage is counted over the section's OWN work items (the
/// union across sections would report every tile stored a hundred times), local memory may be REUSED
/// across sections and must not overlap within one (which is the whole point of a persistent decoder),
/// and the item loop of an engine program indexes the items of the section the op belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionSpan {
    pub name: String,
    /// Half-open `[start, end)` in the composed program's stage list.
    pub stages: (usize, usize),
    pub buffers: (usize, usize),
    pub edges: (usize, usize),
    pub semaphores: (usize, usize),
    pub items: (usize, usize),
    pub plan: Option<BlockPlan>,
    pub k_tiles: usize,
    /// The tensor this section writes (its result).
    pub output: Option<TensorId>,
    /// The rendezvous every engine program passes through between its last op of this section and its
    /// first op of the next. `None` on the last section, which has nothing to be ordered before.
    pub barrier_after: Option<Rendezvous>,
}

/// Every site's work items, section by section — see [`SpatialProgram::site_items`].
pub struct SiteItems<'a> {
    sections: usize,
    /// Each site's whole item list: the placement's first entry for the site, as `Placement::items_at` reads it.
    whole: BTreeMap<Site, &'a [WorkItemId]>,
    by_section: BTreeMap<(usize, Site), Vec<WorkItemId>>,
}

impl SiteItems<'_> {
    /// `SpatialProgram::items_at_in(section, site)`, over sections that partition the items (`validate_sections`).
    pub fn at(&self, section: Option<usize>, site: Site) -> &[WorkItemId] {
        match section.filter(|k| *k < self.sections) {
            Some(k) => self
                .by_section
                .get(&(k, site))
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            None => self.whole.get(&site).copied().unwrap_or(&[]),
        }
    }
}

impl SectionSpan {
    pub fn holds_stage(&self, s: StageId) -> bool {
        s.idx() >= self.stages.0 && s.idx() < self.stages.1
    }
    pub fn holds_buffer(&self, b: BufferId) -> bool {
        b.idx() >= self.buffers.0 && b.idx() < self.buffers.1
    }
    pub fn holds_item(&self, i: WorkItemId) -> bool {
        i.idx() >= self.items.0 && i.idx() < self.items.1
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpatialProgram {
    pub name: String,
    pub tile: TileShape,
    pub tensors: Vec<TensorDecl>,
    pub stages: Vec<Stage>,
    pub buffers: Vec<Buffer>,
    pub edges: Vec<Edge>,
    pub semaphores: Vec<Semaphore>,
    /// The sections, in execution order. EMPTY for a single-workload program — which is every program
    /// the one-op front end builds, so the field is absent from their serialization and every existing
    /// artifact and fixture parses and round-trips unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<SectionSpan>,
    /// The loop counts the program reads from its data ([`Trip`]); a loop names one by its index. EMPTY for every
    /// program whose counts are all fixed, so the field is absent from their serialization.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trips: Vec<Trip>,
    /// Where this chip's program halts for tensors to cross to or from another chip ([`LinkPoint`]), in order. EMPTY for
    /// every program that is a whole machine's, so the field is absent from their serialization.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<LinkPoint>,
    pub work: WorkList,
    pub numerics: NumericsPolicy,
    pub claim: NumericsClaim,
    /// Accumulator tiles a compute stage may hold at once (recorded from the target by the passes).
    pub acc_tiles_used: usize,
    /// The banks a site's local memory interleaves its elements over (recorded from the target by the passes): the
    /// consecutive elements a lowering moves in one cycle. One — and absent from the serialization, so every program
    /// built before banks existed parses and round-trips unchanged — where a memory serves an element a cycle.
    #[serde(default = "one_bank", skip_serializing_if = "is_one_bank")]
    pub mem_banks: usize,
    pub requires: Requirements,
    pub placement: Option<Placement>,
    pub memory: Option<MemoryMap>,
    pub schedule: Option<Schedule>,
    #[serde(skip)]
    pub certificate: Option<ChoreographyCertificate>,
}

/// ★ A HALT AT A SECTION BOUNDARY, FOR WHAT CROSSES THE LINK. One chip's program of a program split over chips
/// ([`compile_program_to_chips`](crate::spatial::pipeline::compile_program_to_chips)) stops here so its tensors can
/// cross: first `sends`, which the sections before it have written and another chip reads, are taken from it; then
/// `receives`, which another chip writes and the sections after it read, are put into it; then it continues.
///
/// `after` is the section the halt follows — `None` before the first section, the last section at the program's end,
/// where the chip has finished. Within the chip a halt orders nothing: it only delays a rendezvous's release until the
/// link has moved its tensors, so the chip's choreography, its certificate and its meaning are the same with it or
/// without it, its receives given.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkPoint {
    pub after: Option<usize>,
    pub sends: Vec<TensorId>,
    pub receives: Vec<TensorId>,
}

/// Walk a choreography with loop multiplicities: `f(op, multiplicity)` for every op reached.
/// A loop run to a trip counts its trip's greatest count here ([`LoopCount::bound`]).
pub fn walk_ops(ops: &[ChoreoOp], mult: u64, f: &mut dyn FnMut(&ChoreoOp, u64)) {
    for op in ops {
        f(op, mult);
        if let ChoreoOp::Loop { count, body, .. } = op {
            walk_tagged_ops(body, mult * count.bound() as u64, f);
        }
    }
}

/// [`walk_ops`] over a tagged choreography.
pub fn walk_tagged_ops(ops: &[TaggedOp], mult: u64, f: &mut dyn FnMut(&ChoreoOp, u64)) {
    for t in ops {
        f(&t.op, mult);
        if let ChoreoOp::Loop { count, body, .. } = &t.op {
            walk_tagged_ops(body, mult * count.bound() as u64, f);
        }
    }
}

/// Walk a tagged choreography: `f(tagged op, multiplicity)` for every op reached, loop bodies included, each loop run
/// to a trip counted at its greatest count.
pub fn walk_tagged(ops: &[TaggedOp], mult: u64, f: &mut dyn FnMut(&TaggedOp, u64)) {
    walk_tagged_counted(ops, mult, &|c| c.bound(), f);
}

/// [`walk_tagged`] with each loop counted by `count` — [`LoopCount::bound`] for the static bound, a trip's count at
/// a position for the run a position makes.
pub fn walk_tagged_counted(
    ops: &[TaggedOp],
    mult: u64,
    count: &dyn Fn(LoopCount) -> usize,
    f: &mut dyn FnMut(&TaggedOp, u64),
) {
    for t in ops {
        f(t, mult);
        if let ChoreoOp::Loop { count: c, body, .. } = &t.op {
            walk_tagged_counted(body, mult * count(*c) as u64, count, f);
        }
    }
}

/// The untagged ops of a loop body.
pub fn body_ops(body: &[TaggedOp]) -> Vec<ChoreoOp> {
    body.iter().map(|t| t.op.clone()).collect()
}

impl SpatialProgram {
    pub fn tensor(&self, id: TensorId) -> &TensorDecl {
        &self.tensors[id.idx()]
    }
    pub fn stage(&self, id: StageId) -> &Stage {
        &self.stages[id.idx()]
    }
    pub fn buffer(&self, id: BufferId) -> &Buffer {
        &self.buffers[id.idx()]
    }
    /// ★ THE TILES ONE PAGE OF EACH BUFFER HOLDS, by buffer index: the most tiles any op fills or drains one page of it
    /// with, and never fewer than its declared `page_tiles`.
    ///
    /// A page is what the op that fills it writes. The evaluator keeps each page as that op's list of tiles, and reads
    /// a page's plane at that list's width. The declared page can be smaller. On a target whose accumulator lives in
    /// registers, `tile` declares a matmul's output as one tile per accumulator lane, because the GEMM kernel stores
    /// each lane's tiles straight to the tensor and never stages the block. The choreography still packs the whole
    /// `osh · osw` block into one page of it. A lowering that stages every page through memory lays its slots out at
    /// this extent, or the pack writes past the ring.
    pub fn page_extents(&self) -> Vec<usize> {
        let mut extent: Vec<usize> = self.buffers.iter().map(|b| b.kind.page_tiles()).collect();
        let Some(schedule) = &self.schedule else {
            return extent;
        };
        for program in &schedule.programs {
            let mut acquired = 0;
            walk_tagged_ops(&program.ops, 1, &mut |op, _| {
                let (b, tiles) = match op {
                    ChoreoOp::ReadTiles { into, block_h, block_w, .. } => (*into, block_h * block_w),
                    ChoreoOp::WriteTiles { from, block_h, block_w, .. } => (*from, block_h * block_w),
                    ChoreoOp::PackAcc { to, tiles, .. } => (*to, *tiles),
                    ChoreoOp::Compute { acc_reload: Some(r), .. } => (*r, acquired),
                    ChoreoOp::AcquireAcc { tiles } => {
                        acquired = *tiles;
                        return;
                    }
                    _ => return,
                };
                extent[b.idx()] = extent[b.idx()].max(tiles);
            });
        }
        extent
    }
    pub fn output_tensor(&self) -> Option<&TensorDecl> {
        // A program whose only effect is an IN-PLACE update — a row appended to a carried cache — has no
        // `Output`; its result is the `State` it stores.
        self.tensors.iter().find(|t| t.role == TensorRole::Output).or_else(|| {
            self.tensors.iter().find(|t| t.role == TensorRole::State && self.stages.iter().any(|s| matches!(&s.kind, StageKind::Store { tensor, .. } if *tensor == t.id)))
        })
    }

    /// What a section's work WRITES — the tensor its items' rectangles and its plan's grid are blocks of: the
    /// section's `Output`, or, for a section whose every effect is an in-place update, the state it stores.
    /// `None` names the uncomposed program's one implicit section.
    pub fn section_result(&self, section: Option<usize>) -> Option<&TensorDecl> {
        match section.and_then(|k| self.sections.get(k)) {
            Some(sp) => sp.output.map(|t| self.tensor(t)).or_else(|| {
                self.stages[sp.stages.0..sp.stages.1]
                    .iter()
                    .find_map(|s| match &s.kind {
                        StageKind::Store { tensor, .. }
                            if self.tensor(*tensor).role == TensorRole::State =>
                        {
                            Some(self.tensor(*tensor))
                        }
                        _ => None,
                    })
            }),
            None => self.output_tensor(),
        }
    }

    /// Every CARRIED buffer and the one stage that updates it: a buffer a stage both reads and writes. A carry holds
    /// a value across the K blocks of an item — the running state of a fold — and its first page is written by a
    /// second stage, its init, before the blocks begin.
    pub fn carries(&self) -> BTreeMap<BufferId, StageId> {
        let mut out = BTreeMap::new();
        for s in &self.stages {
            for b in &s.outs {
                if s.ins.contains(b) {
                    out.entry(*b).or_insert(s.id);
                }
            }
        }
        out
    }

    /// Each stage's level in its item's K-block fold ([`FoldLevel`]), derived from the dataflow. A value MOVES WITH
    /// THE BLOCKS when a load that steps `LV_KBLOCK` (or a K-block receive) brings it, or a body stage computes it.
    /// The BODY is every stage that reads such a value, every carry's update, and every stage that reads a carry
    /// and computes what the body reads (a block's limit from its running start); the EPILOGUE is every other
    /// stage that reads a carry — its value after the last block — or what the epilogue computed; the PROLOGUE is
    /// what remains: stages that read only values fixed for the item.
    pub fn fold_levels(&self) -> Vec<FoldLevel> {
        let carries = self.carries();
        let mut level = vec![FoldLevel::Prologue; self.stages.len()];
        let mut producers: BTreeMap<BufferId, Vec<StageId>> = BTreeMap::new();
        let mut readers: BTreeMap<BufferId, Vec<StageId>> = BTreeMap::new();
        for s in &self.stages {
            for b in &s.outs {
                producers.entry(*b).or_default().push(s.id);
            }
            for b in &s.ins {
                readers.entry(*b).or_default().push(s.id);
            }
        }
        loop {
            let mut changed = false;
            for s in &self.stages {
                let reads = |l: FoldLevel| {
                    s.ins.iter().filter(|b| !carries.contains_key(b)).any(|b| {
                        producers
                            .get(b)
                            .is_some_and(|ps| ps.iter().any(|p| level[p.idx()] == l))
                    })
                };
                let reads_carry = s.ins.iter().any(|b| carries.contains_key(b));
                let feeds_body = s.outs.iter().filter(|b| !carries.contains_key(b)).any(|b| {
                    readers.get(b).is_some_and(|rs| {
                        rs.iter()
                            .any(|r| *r != s.id && level[r.idx()] == FoldLevel::Body)
                    })
                });
                let next = match &s.kind {
                    StageKind::Load { pattern, .. }
                        if pattern
                            .origin
                            .steps
                            .iter()
                            .any(|(v, _)| *v == super::passes::LV_KBLOCK) =>
                    {
                        FoldLevel::Body
                    }
                    StageKind::Recv { .. } => FoldLevel::Body,
                    _ if carries.values().any(|c| *c == s.id)
                        || reads(FoldLevel::Body)
                        || (reads_carry && feeds_body) =>
                    {
                        FoldLevel::Body
                    }
                    _ if reads_carry || reads(FoldLevel::Epilogue) => FoldLevel::Epilogue,
                    _ => FoldLevel::Prologue,
                };
                if next != level[s.id.idx()] {
                    level[s.id.idx()] = next;
                    changed = true;
                }
            }
            if !changed {
                return level;
            }
        }
    }

    /// The section a stage belongs to, by index. `None` for an uncomposed program (which is ONE
    /// implicit section, so every section-scoped query falls back to the whole program).
    pub fn section_of_stage(&self, s: StageId) -> Option<usize> {
        self.section_holding(s.idx(), |sp| sp.stages)
    }

    /// The section a buffer belongs to, by index.
    pub fn section_of_buffer(&self, b: BufferId) -> Option<usize> {
        self.section_holding(b.idx(), |sp| sp.buffers)
    }

    /// The section a work item belongs to, by index.
    pub fn section_of_item(&self, i: WorkItemId) -> Option<usize> {
        self.section_holding(i.idx(), |sp| sp.items)
    }

    /// The first section whose `span` holds `id`.
    ///
    /// ★ A LOOKUP IN THE PARTITION, NOT A SCAN OF IT. The spans partition every id space in order
    /// (`validate_sections`), so the holder is the first section whose span ends past `id` — a binary search.
    /// These lookups run per stage in the validators and per executed op in the evaluator, so each costs the log
    /// of the section count, of which a composed layer has thousands. A candidate that does not hold `id` means
    /// the spans do not partition (a program `validate` refuses, which the evaluator may still be handed), and
    /// the answer is then the scan's.
    fn section_holding(
        &self,
        id: usize,
        span: impl Fn(&SectionSpan) -> (usize, usize),
    ) -> Option<usize> {
        let holds = |sp: &SectionSpan| {
            let (a, b) = span(sp);
            a <= id && id < b
        };
        let k = self.sections.partition_point(|sp| span(sp).1 <= id);
        match self.sections.get(k) {
            Some(sp) if holds(sp) => Some(k),
            _ => self.sections.iter().position(holds),
        }
    }

    /// The work items of `site` WITHIN a section — the composed program's item list is the sections'
    /// concatenated, so an unscoped `items_at` would hand a section every other section's items.
    pub fn items_at_in(&self, section: Option<usize>, site: Site) -> Vec<WorkItemId> {
        let all = self
            .placement
            .as_ref()
            .map(|pl| pl.items_at(site).to_vec())
            .unwrap_or_default();
        match section.and_then(|k| self.sections.get(k)) {
            Some(sp) => all.into_iter().filter(|i| sp.holds_item(*i)).collect(),
            None => all,
        }
    }

    /// [`Self::items_at_in`] for every section and site at once, built in one pass over the placement, for the loops
    /// that ask it per stage or per executed op: each question is a lookup, not a copy of the site's item list.
    pub fn site_items(&self) -> SiteItems<'_> {
        let mut whole: BTreeMap<Site, &[WorkItemId]> = BTreeMap::new();
        let mut by_section: BTreeMap<(usize, Site), Vec<WorkItemId>> = BTreeMap::new();
        for (site, items) in self.placement.iter().flat_map(|pl| pl.items_of_site.iter()) {
            if whole.contains_key(site) {
                continue;
            }
            whole.insert(*site, items);
            for i in items {
                if let Some(k) = self.section_of_item(*i) {
                    by_section.entry((k, *site)).or_default().push(*i);
                }
            }
        }
        SiteItems {
            sections: self.sections.len(),
            whole,
            by_section,
        }
    }

    /// The plan a stage was lowered under: its section's, or the program's own.
    pub fn plan_of_stage(&self, s: StageId) -> Option<BlockPlan> {
        self.section_of_stage(s)
            .and_then(|k| self.sections[k].plan)
            .or(self.work.plan)
    }

    /// Every tensor the program carries ACROSS steps — the state a second token chains through.
    pub fn state_tensors(&self) -> Vec<&TensorDecl> {
        self.tensors
            .iter()
            .filter(|t| t.role == TensorRole::State)
            .collect()
    }

    /// The tensor a buffer carries: the tensor of the Load stage that fills the buffer (or its template), or of
    /// the Store stage that drains it.
    fn buffer_tensor(&self, b: BufferId) -> Option<TensorId> {
        let template = self.buffers[b.idx()].origin.unwrap_or(b);
        self.stages.iter().find_map(|s| match &s.kind {
            StageKind::Load { tensor, .. }
                if s.outs.iter().any(|o| {
                    *o == b || *o == template || self.buffers[o.idx()].origin == Some(template)
                }) =>
            {
                Some(*tensor)
            }
            StageKind::Store { tensor, .. }
                if s.ins.iter().any(|i| {
                    *i == b || *i == template || self.buffers[i.idx()].origin == Some(template)
                }) =>
            {
                Some(*tensor)
            }
            _ => None,
        })
    }

    /// The output tensor a compute stage's result reaches, following chained compute stages to the Store.
    ///
    /// The walk is bounded by the STAGE COUNT rather than a tuned constant. Each step moves to a distinct
    /// downstream stage, so a chain longer than the program has stages can only mean a cycle, and a bound
    /// derived from the program cannot be outgrown by a longer fusion chain the way a literal can.
    fn stage_output_tensor(&self, stage: &Stage, depth: usize) -> Option<TensorId> {
        if depth > self.stages.len() {
            return None;
        }
        for out in &stage.outs {
            if let Some(t) = self.buffer_tensor(*out) {
                return Some(t);
            }
            if let Some(next) = self.stages.iter().find(|s| {
                s.id != stage.id && s.ins.contains(out) && matches!(s.kind, StageKind::Compute(_))
            }) {
                if let Some(t) = self.stage_output_tensor(next, depth + 1) {
                    return Some(t);
                }
            }
        }
        None
    }

    /// Tile dimensions the computation forces equal: a GEMM's `A.cols ≡ B.rows` (K), `B.cols ≡ C.cols` (N),
    /// `A.rows ≡ C.rows` (M); an elementwise op's operands and result share both extents; a row op's rows
    /// and its per-column parameters share its width. A lowering prints one runtime symbol per class, so a
    /// kernel's address arithmetic names each extent once — the form the vendor's kernels have.
    pub fn dim_classes(&self) -> Vec<Vec<(TensorId, DimAxis)>> {
        let n = self.tensors.len() * 2;
        let key = |t: TensorId, a: DimAxis| {
            t.idx() * 2
                + match a {
                    DimAxis::Rows => 0,
                    DimAxis::Cols => 1,
                }
        };
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(parent: &mut Vec<usize>, i: usize) -> usize {
            let mut r = i;
            while parent[r] != r {
                r = parent[r];
            }
            let mut c = i;
            while parent[c] != r {
                let next = parent[c];
                parent[c] = r;
                c = next;
            }
            r
        }
        let union = |parent: &mut Vec<usize>, a: usize, b: usize| {
            let (ra, rb) = (find(parent, a), find(parent, b));
            if ra != rb {
                parent[ra.max(rb)] = ra.min(rb);
            }
        };
        for s in &self.stages {
            let StageKind::Compute(op) = &s.kind else {
                continue;
            };
            let ins: Vec<Option<TensorId>> = s.ins.iter().map(|b| self.buffer_tensor(*b)).collect();
            let out = self.stage_output_tensor(s, 0);
            match op {
                ComputeOp::MatmulTiles { .. } => {
                    if let (Some(Some(a)), Some(Some(b))) = (ins.first(), ins.get(1)) {
                        union(&mut parent, key(*a, DimAxis::Cols), key(*b, DimAxis::Rows));
                        if let Some(c) = out {
                            union(&mut parent, key(*a, DimAxis::Rows), key(c, DimAxis::Rows));
                            union(&mut parent, key(*b, DimAxis::Cols), key(c, DimAxis::Cols));
                        }
                    }
                }
                // ★ SHAPE INFERENCE READS THE MAPS TOO. The old rule unified every operand's rows
                // and cols unconditionally, which is right only when every operand is walked along
                // every axis. An operand that is CONSTANT along an axis is a broadcast — a per-column
                // weight has no row dimension to share — so it constrains nothing there, and saying
                // otherwise would force a 1xN weight to have the same height as its input.
                ComputeOp::Generic {
                    iters,
                    ins: maps,
                    out: omap,
                    ..
                } => {
                    for (ax, dim) in [
                        (iters.first(), DimAxis::Rows),
                        (iters.get(1), DimAxis::Cols),
                    ] {
                        let Some(ax) = ax else { continue };
                        let mut sharing: Vec<TensorId> = ins
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| {
                                maps.get(*i).map(|m| m.varies_along(ax.var)).unwrap_or(true)
                            })
                            .filter_map(|(_, t)| *t)
                            .collect();
                        if omap.varies_along(ax.var) {
                            sharing.extend(out);
                        }
                        for w in sharing.windows(2) {
                            union(&mut parent, key(w[0], dim), key(w[1], dim));
                        }
                    }
                }
                _ => {
                    // A unary stage (requant / bias / relu / dequant / pack) preserves its operand's extents.
                    if let (Some(Some(x)), Some(o)) = (ins.first(), out) {
                        union(&mut parent, key(*x, DimAxis::Rows), key(o, DimAxis::Rows));
                        union(&mut parent, key(*x, DimAxis::Cols), key(o, DimAxis::Cols));
                    }
                }
            }
        }
        let mut classes: std::collections::BTreeMap<usize, Vec<(TensorId, DimAxis)>> =
            std::collections::BTreeMap::new();
        for t in &self.tensors {
            for a in [DimAxis::Rows, DimAxis::Cols] {
                let k = key(t.id, a);
                classes
                    .entry(find(&mut parent, k))
                    .or_default()
                    .push((t.id, a));
            }
        }
        classes.into_values().collect()
    }

    /// The representative of a tile dimension's class: the output tensor's extent when the class reaches the
    /// output, else the lowest-numbered tensor's.
    pub fn dim_representative(&self, tensor: TensorId, axis: DimAxis) -> (TensorId, DimAxis) {
        let classes = self.dim_classes();
        let class = classes
            .iter()
            .find(|c| c.contains(&(tensor, axis)))
            .cloned()
            .unwrap_or_else(|| vec![(tensor, axis)]);
        class
            .iter()
            .copied()
            .find(|(t, _)| self.tensors[t.idx()].role == TensorRole::Output)
            .unwrap_or(class[0])
    }
    pub fn buffer_bytes(&self, b: &Buffer) -> usize {
        b.kind.tiles() * self.tile.elems() * precision_bytes(b.dtype)
    }

    pub fn phase(&self) -> Phase {
        if self.certificate.is_some() {
            Phase::Verified
        } else if self.schedule.is_some() {
            Phase::Scheduled
        } else if self.memory.is_some() {
            Phase::Sized
        } else if self.placement.is_some() {
            Phase::Placed
        } else if self.work.plan.is_some() {
            Phase::Tiled
        } else {
            Phase::Template
        }
    }

    /// The deterministic, total, parseable dump of the program (the certificate travels separately).
    pub fn neutral_dump(&self) -> String {
        serde_json::to_string_pretty(self).expect("the spatial IR serializes")
    }

    pub fn parse_neutral(s: &str) -> Result<SpatialProgram, String> {
        serde_json::from_str(s).map_err(|e| e.to_string())
    }

    /// Every distinct site any stage is placed on.
    pub fn sites(&self) -> BTreeSet<Site> {
        self.stages.iter().filter_map(|s| s.site).collect()
    }

    /// The multicast group a Load sends to or a Recv receives from.
    pub fn stage_group(&self, s: &Stage) -> Option<GroupId> {
        match &s.kind {
            StageKind::Load { multicast, .. } => *multicast,
            StageKind::Recv { group } => Some(*group),
            _ => None,
        }
    }

    /// DRAM tiles the Load stages read over the whole run.
    pub fn dram_tiles_read(&self) -> u64 {
        let items_template = self.work.items.len().max(1) as u64;
        let mut total = 0u64;
        for s in &self.stages {
            if let StageKind::Load {
                pattern,
                num_blocks,
                ..
            } = &s.kind
            {
                let per_item = pattern.block_tiles() as u64 * (*num_blocks).max(1) as u64;
                let items = match (&self.placement, s.site) {
                    (Some(p), Some(site)) => p.items_at(site).len().max(1) as u64,
                    _ => items_template,
                };
                total += per_item * items;
            }
        }
        total
    }

    /// A site-independent structural signature of an engine program: the kernel a lowering would emit.
    /// A buffer is keyed by its template AND its page: a kernel counts pages in literals, so two sites whose
    /// pages differ (a boundary site of a grid that does not divide the tile count) cannot share one.
    pub fn engine_signature(&self, p: &EngineProgram) -> String {
        let buf_key = |b: BufferId| -> String {
            let bb = self.buffer(b);
            format!(
                "b{}p{}",
                bb.origin.map(|o| o.idx()).unwrap_or(b.idx()),
                bb.kind.page_tiles()
            )
        };
        let sem_key = |s: SemId| -> String {
            match self.semaphores.get(s.idx()) {
                Some(sem) => format!(
                    "s{:?}{}",
                    sem.role,
                    sem.operand.map(|o| format!("{o:?}")).unwrap_or_default()
                ),
                None => format!("s?{}", s.idx()),
            }
        };
        let group_key = |g: GroupId| -> String {
            self.placement
                .as_ref()
                .and_then(|pl| pl.groups.get(g.idx()))
                .map(|gr| format!("g{:?}{:?}", gr.operand, gr.direction))
                .unwrap_or_else(|| "g".into())
        };
        fn sig(
            ops: &[TaggedOp],
            out: &mut String,
            buf_key: &dyn Fn(BufferId) -> String,
            sem_key: &dyn Fn(SemId) -> String,
            group_key: &dyn Fn(GroupId) -> String,
        ) {
            for t in ops {
                match &t.op {
                    ChoreoOp::Reserve { buf, pages } => {
                        out.push_str(&format!("R({},{});", buf_key(*buf), pages))
                    }
                    ChoreoOp::Push { buf, pages } => {
                        out.push_str(&format!("P({},{});", buf_key(*buf), pages))
                    }
                    ChoreoOp::Wait { buf, pages } => {
                        out.push_str(&format!("W({},{});", buf_key(*buf), pages))
                    }
                    ChoreoOp::Pop { buf, pages } => {
                        out.push_str(&format!("O({},{});", buf_key(*buf), pages))
                    }
                    ChoreoOp::ReadTiles {
                        tensor,
                        at,
                        block_h,
                        block_w,
                        stride_w,
                        stride_h,
                        into,
                    } => out.push_str(&format!(
                        "RT(t{},{:?},{block_h},{block_w},{stride_w},{stride_h},{});",
                        tensor.idx(),
                        at.steps,
                        buf_key(*into)
                    )),
                    ChoreoOp::ReadBarrier => out.push_str("RB;"),
                    ChoreoOp::WriteTiles {
                        from,
                        tensor,
                        at,
                        block_h,
                        block_w,
                        stride_w,
                        stride_h,
                    } => out.push_str(&format!(
                        "WT({},t{},{:?},{block_h},{block_w},{stride_w},{stride_h});",
                        buf_key(*from),
                        tensor.idx(),
                        at.steps
                    )),
                    ChoreoOp::WriteBarrier => out.push_str("WB;"),
                    ChoreoOp::McastSend {
                        buf,
                        pages,
                        group,
                        hold_path,
                    } => out.push_str(&format!(
                        "MS({},{hold_path},{pages},{});",
                        buf_key(*buf),
                        group_key(*group)
                    )),
                    ChoreoOp::McastSetValid { sem, group } => {
                        out.push_str(&format!("MV({},{});", sem_key(*sem), group_key(*group)))
                    }
                    ChoreoOp::Flush { cond } => out.push_str(&format!("F{cond:?};")),
                    ChoreoOp::SemSet { sem, value } => {
                        out.push_str(&format!("SS({},{value:?});", sem_key(*sem)))
                    }
                    ChoreoOp::SemInc { sem, by, .. } => {
                        out.push_str(&format!("SI({},{by});", sem_key(*sem)))
                    }
                    ChoreoOp::SemWait { sem, until } => {
                        out.push_str(&format!("SW({},{until:?});", sem_key(*sem)))
                    }
                    ChoreoOp::AcquireAcc { tiles } => out.push_str(&format!("AA({tiles});")),
                    ChoreoOp::ReleaseAcc => out.push_str("RA;"),
                    ChoreoOp::Compute {
                        op,
                        ins,
                        acc_reload,
                    } => {
                        // Buffer ids inside the op are per-site instances: key them by template too.
                        let op_key = match op {
                            ComputeOp::MatmulTiles { block, sub_blocks, num_blocks, operand, acc, dataflow, interm, granularity } => format!(
                                "MM({block:?},{sub_blocks:?},{num_blocks},{operand:?},{acc:?},{dataflow:?},{:?},{granularity:?})",
                                interm.map(buf_key)
                            ),
                            ComputeOp::Bias { bias } => format!("Bias({})", buf_key(*bias)),
                            other => format!("{other:?}"),
                        };
                        out.push_str(&format!(
                            "C({op_key},[{}],{:?});",
                            ins.iter()
                                .map(|b| buf_key(*b))
                                .collect::<Vec<_>>()
                                .join(","),
                            acc_reload.map(buf_key)
                        ))
                    }
                    ChoreoOp::PackAcc {
                        to,
                        tiles,
                        convert,
                        accumulate,
                    } => out.push_str(&format!(
                        "PA({},{tiles},{convert:?},{accumulate});",
                        buf_key(*to)
                    )),
                    ChoreoOp::Barrier { group } => {
                        out.push_str(&format!("B({});", group_key(*group)))
                    }
                    ChoreoOp::Loop { body, .. } => {
                        out.push_str("L{");
                        sig(body, out, buf_key, sem_key, group_key);
                        out.push_str("}");
                    }
                }
            }
        }
        let mut out = format!("{:?}:", p.engine);
        sig(&p.ops, &mut out, &buf_key, &sem_key, &group_key);
        out
    }

    /// The emergent plan.
    pub fn summary(&self) -> PlanSummary {
        let plan = self.work.plan;
        let (grid, per_site_block, block, num_blocks, acc) = match plan {
            Some(p) => (
                p.grid,
                (p.per_site_m, p.per_site_n),
                Some(p.block),
                p.inner.num_blocks,
                Some(p.acc),
            ),
            None => ((0, 0), (0, 0), None, 0, None),
        };
        let streaming = plan
            .map(|p| {
                p.per_site_m == 1
                    && p.per_site_n == 1
                    && p.block
                        == BlockShape {
                            osh: 1,
                            osw: 1,
                            ibw: 1,
                        }
            })
            .unwrap_or(false);
        let (multicast_groups, row_groups, col_groups) = match &self.placement {
            Some(pl) => (
                pl.groups.len(),
                pl.groups
                    .iter()
                    .filter(|g| g.direction == Direction::Row)
                    .count(),
                pl.groups
                    .iter()
                    .filter(|g| g.direction == Direction::Col)
                    .count(),
            ),
            None => (0, 0, 0),
        };
        let engine_program_classes = match &self.schedule {
            Some(s) => s
                .programs
                .iter()
                .map(|p| self.engine_signature(p))
                .collect::<BTreeSet<_>>()
                .len(),
            None => 0,
        };
        PlanSummary {
            phase: self.phase(),
            sites_used: self.sites().len(),
            grid,
            per_site_block,
            block,
            num_blocks,
            streaming,
            multicast_groups,
            row_groups,
            col_groups,
            acc,
            dram_tiles_read: self.dram_tiles_read(),
            semaphores: self.semaphores.len(),
            fifos: self
                .buffers
                .iter()
                .filter(|b| matches!(b.kind, BufferKind::Fifo { .. }))
                .count(),
            engine_program_classes,
            engines: self
                .work
                .plan
                .map(|pl| pl.engines)
                .unwrap_or(EnginePacking::LoadsTogether),
        }
    }

    /// Every structural invariant, target-free. Runs after every pass.
    pub fn validate(&self) -> Result<(), IrViolation> {
        self.validate_ids()?;
        self.validate_sections()?;
        self.validate_references()?;
        self.validate_ports()?;
        self.validate_carries()?;
        self.validate_tensors()?;
        self.validate_compute()?;
        self.validate_indirect()?;
        self.validate_plan()?;
        self.validate_phase_order()?;
        self.validate_placement()?;
        self.validate_memory()?;
        self.validate_trips()?;
        self.validate_schedule()?;
        self.validate_rendezvous()?;
        self.validate_requirements()?;
        Ok(())
    }

    /// Each trip's greatest count, in trip order — the counts every static question takes.
    pub fn bound_counts(&self) -> Vec<usize> {
        self.trips.iter().map(Trip::hi).collect()
    }

    pub fn trip(&self, id: TripId) -> &Trip {
        &self.trips[id.idx()]
    }

    /// ★ A TRIP IS A PROGRAM ARGUMENT, AND ITS LOOPS ARE THE SIMPLE KIND. The trip is read from an input no stage
    /// writes, so its value is fixed before the first op and every engine that loops to it reads the same count; its
    /// range is a real one; every loop run to it declares its greatest count, nests in no other trip loop, and holds
    /// only buffer handshakes, reads and compute — the loops whose tokens repeat block after block, which is what lets
    /// the certificate prove a window of counts for all of them, and whose stores all sit outside, so the tiles a run
    /// writes are the ones the work list proves covered whatever the count. And every trip counts some loop.
    fn validate_trips(&self) -> Result<(), IrViolation> {
        for (i, t) in self.trips.iter().enumerate() {
            let bad = |reason| {
                Err(IrViolation::TripMalformed {
                    trip: TripId::new(i),
                    reason,
                })
            };
            let Some(td) = self.tensors.get(t.tensor.idx()) else {
                return bad("a trip read from a tensor the program does not declare");
            };
            if td.role != TensorRole::Input {
                return bad("a trip read from a tensor that is not a program input");
            }
            if t.elem >= td.elems.rows * td.elems.cols {
                return bad("a trip read from an element past its tensor");
            }
            if t.per == 0 {
                return bad("a trip counted per zero elements");
            }
            if t.admit.0 > t.admit.1 {
                return bad("a trip that admits no value");
            }
            if t.lo() == 0 {
                return bad("a trip whose least count is zero");
            }
        }
        let Some(sch) = &self.schedule else {
            return Ok(());
        };
        fn walk(
            p: &SpatialProgram,
            ops: &[TaggedOp],
            inside: bool,
            used: &mut [bool],
        ) -> Result<(), IrViolation> {
            for t in ops {
                let ChoreoOp::Loop { count, body, .. } = &t.op else {
                    continue;
                };
                let Some(trip) = count.trip() else {
                    walk(p, body, inside, used)?;
                    continue;
                };
                let bad = |reason| Err(IrViolation::TripLoopMalformed { reason });
                let Some(declared) = p.trips.get(trip.idx()) else {
                    return bad("a loop run to a trip the program does not declare");
                };
                if inside {
                    return bad("a loop run to a trip inside another");
                }
                if count.bound() != declared.hi() {
                    return bad("a loop bounded apart from the trip it runs to");
                }
                let (mut foreign, mut stores) = (false, false);
                walk_tagged_ops(body, 1, &mut |op, _| {
                    foreign |= matches!(
                        op,
                        ChoreoOp::SemSet { .. }
                            | ChoreoOp::SemInc { .. }
                            | ChoreoOp::SemWait { .. }
                            | ChoreoOp::McastSend { .. }
                            | ChoreoOp::McastSetValid { .. }
                            | ChoreoOp::Barrier { .. }
                    );
                    stores |= matches!(op, ChoreoOp::WriteTiles { .. });
                });
                if foreign {
                    return bad("a semaphore or multicast inside a loop run to a trip");
                }
                if stores {
                    return bad(
                        "a store inside a loop run to a trip, covering tiles the data decides",
                    );
                }
                used[trip.idx()] = true;
                walk(p, body, true, used)?;
            }
            Ok(())
        }
        let mut used = vec![false; self.trips.len()];
        for prog in &sch.programs {
            walk(self, &prog.ops, false, &mut used)?;
        }
        if let Some(i) = used.iter().position(|u| !u) {
            return Err(IrViolation::TripMalformed {
                trip: TripId::new(i),
                reason: "a trip no loop runs to",
            });
        }
        Ok(())
    }

    fn validate_ids(&self) -> Result<(), IrViolation> {
        for (i, t) in self.tensors.iter().enumerate() {
            if t.id.idx() != i {
                return Err(IrViolation::IdNotDense {
                    what: "tensor",
                    index: i,
                });
            }
        }
        for (i, s) in self.stages.iter().enumerate() {
            if s.id.idx() != i {
                return Err(IrViolation::IdNotDense {
                    what: "stage",
                    index: i,
                });
            }
        }
        for (i, b) in self.buffers.iter().enumerate() {
            if b.id.idx() != i {
                return Err(IrViolation::IdNotDense {
                    what: "buffer",
                    index: i,
                });
            }
        }
        for (i, e) in self.edges.iter().enumerate() {
            if e.id.idx() != i {
                return Err(IrViolation::IdNotDense {
                    what: "edge",
                    index: i,
                });
            }
        }
        for (i, s) in self.semaphores.iter().enumerate() {
            if s.id.idx() != i {
                return Err(IrViolation::IdNotDense {
                    what: "semaphore",
                    index: i,
                });
            }
        }
        for (i, w) in self.work.items.iter().enumerate() {
            if w.id.idx() != i {
                return Err(IrViolation::IdNotDense {
                    what: "work item",
                    index: i,
                });
            }
        }
        if let Some(pl) = &self.placement {
            for (i, g) in pl.groups.iter().enumerate() {
                if g.id.idx() != i {
                    return Err(IrViolation::IdNotDense {
                        what: "group",
                        index: i,
                    });
                }
            }
        }
        Ok(())
    }

    /// The sections PARTITION every id space, in order — so "the section of x" is a total function and
    /// a scope-aware rule can never be silently skipped by an id that falls in no section.
    fn validate_sections(&self) -> Result<(), IrViolation> {
        if self.sections.is_empty() {
            return Ok(());
        }
        type Span = fn(&SectionSpan) -> (usize, usize);
        let ends: [(&'static str, usize, Span); 5] = [
            ("stages", self.stages.len(), |s| s.stages),
            ("buffers", self.buffers.len(), |s| s.buffers),
            ("edges", self.edges.len(), |s| s.edges),
            ("semaphores", self.semaphores.len(), |s| s.semaphores),
            ("items", self.work.items.len(), |s| s.items),
        ];
        for (what, total, get) in ends {
            let mut cursor = 0usize;
            for (k, sp) in self.sections.iter().enumerate() {
                let (a, b) = get(sp);
                if a != cursor || b < a {
                    return Err(IrViolation::SectionSpanMalformed { section: k, what });
                }
                cursor = b;
            }
            if cursor != total {
                return Err(IrViolation::SectionSpanMalformed {
                    section: self.sections.len() - 1,
                    what,
                });
            }
        }
        // A section's stages touch only that section's buffers. Sections hand values to each other
        // through TENSORS (an activation, a state), never by reaching into each other's local memory —
        // which is exactly what makes re-using the same L1 offsets across sections safe.
        for (k, sp) in self.sections.iter().enumerate() {
            for s in &self.stages[sp.stages.0..sp.stages.1] {
                for b in s.ins.iter().chain(s.outs.iter()) {
                    if !sp.holds_buffer(*b) {
                        let bk = self.section_of_buffer(*b).unwrap_or(usize::MAX);
                        return Err(IrViolation::SectionBufferEscape {
                            stage: s.id,
                            buffer: *b,
                            stage_section: k,
                            buffer_section: bk,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Each section's PHASE: the run of sections one barrier closes, counted from the first.
    pub fn section_phases(&self) -> Vec<usize> {
        let mut phase = 0usize;
        self.sections
            .iter()
            .map(|sp| {
                let here = phase;
                if sp.barrier_after.is_some() {
                    phase += 1;
                }
                here
            })
            .collect()
    }

    fn validate_references(&self) -> Result<(), IrViolation> {
        let nt = self.tensors.len();
        let nb = self.buffers.len();
        let ns = self.stages.len();
        let ng = self.placement.as_ref().map(|p| p.groups.len()).unwrap_or(0);
        for s in &self.stages {
            for b in s.ins.iter().chain(s.outs.iter()) {
                if b.idx() >= nb {
                    return Err(IrViolation::DanglingBuffer {
                        stage: Some(s.id),
                        buffer: *b,
                    });
                }
            }
            match &s.kind {
                StageKind::Load {
                    tensor, multicast, ..
                } => {
                    if tensor.idx() >= nt {
                        return Err(IrViolation::DanglingTensor {
                            stage: s.id,
                            tensor: *tensor,
                        });
                    }
                    if let Some(g) = multicast {
                        if g.idx() >= ng {
                            return Err(IrViolation::DanglingGroup {
                                stage: s.id,
                                group: *g,
                            });
                        }
                    }
                }
                StageKind::Store { tensor, .. } => {
                    if tensor.idx() >= nt {
                        return Err(IrViolation::DanglingTensor {
                            stage: s.id,
                            tensor: *tensor,
                        });
                    }
                }
                StageKind::Recv { group } => {
                    if group.idx() >= ng {
                        return Err(IrViolation::DanglingGroup {
                            stage: s.id,
                            group: *group,
                        });
                    }
                }
                StageKind::Compute(ComputeOp::MatmulTiles {
                    interm: Some(i), ..
                }) => {
                    if i.idx() >= nb {
                        return Err(IrViolation::DanglingBuffer {
                            stage: Some(s.id),
                            buffer: *i,
                        });
                    }
                }
                StageKind::Compute(ComputeOp::Bias { bias }) => {
                    if bias.idx() >= nb {
                        return Err(IrViolation::DanglingBuffer {
                            stage: Some(s.id),
                            buffer: *bias,
                        });
                    }
                }
                _ => {}
            }
            if let Some(it) = s.item {
                if it.idx() >= self.work.items.len() {
                    return Err(IrViolation::DanglingItem {
                        stage: s.id,
                        item: it,
                    });
                }
            }
        }
        for e in &self.edges {
            if e.from.idx() >= ns {
                return Err(IrViolation::DanglingStage {
                    edge: e.id,
                    stage: e.from,
                });
            }
            for t in &e.to {
                if t.idx() >= ns {
                    return Err(IrViolation::DanglingStage {
                        edge: e.id,
                        stage: *t,
                    });
                }
            }
            if e.buffer.idx() >= nb {
                return Err(IrViolation::DanglingBuffer {
                    stage: None,
                    buffer: e.buffer,
                });
            }
            if let Handshake::Semaphore { ready, valid, .. } = &e.handshake {
                for s in [ready, valid] {
                    if s.idx() >= self.semaphores.len() {
                        return Err(IrViolation::DanglingSemaphore { sem: *s });
                    }
                }
            }
        }
        for b in &self.buffers {
            if let Some(a) = b.alias_of {
                if a.idx() >= nb {
                    return Err(IrViolation::DanglingBuffer {
                        stage: None,
                        buffer: a,
                    });
                }
            }
        }
        Ok(())
    }

    fn validate_ports(&self) -> Result<(), IrViolation> {
        let mut producers: BTreeMap<BufferId, usize> = BTreeMap::new();
        let mut consumers: BTreeMap<BufferId, usize> = BTreeMap::new();
        for e in &self.edges {
            let from = self.stage(e.from);
            if !e.handshake.is_local() {
                // Inter-site edges name the SOURCE buffer at the sender; the receivers own their own copies.
                if !from.outs.contains(&e.buffer) {
                    return Err(IrViolation::EdgeBufferNotInStagePorts {
                        edge: e.id,
                        stage: e.from,
                        buffer: e.buffer,
                    });
                }
                continue;
            }
            if !from.outs.contains(&e.buffer) {
                return Err(IrViolation::EdgeBufferNotInStagePorts {
                    edge: e.id,
                    stage: e.from,
                    buffer: e.buffer,
                });
            }
            *producers.entry(e.buffer).or_default() += 1;
            for t in &e.to {
                if !self.stage(*t).ins.contains(&e.buffer) {
                    return Err(IrViolation::EdgeBufferNotInStagePorts {
                        edge: e.id,
                        stage: *t,
                        buffer: e.buffer,
                    });
                }
                *consumers.entry(e.buffer).or_default() += 1;
            }
        }
        for b in &self.buffers {
            if let BufferKind::Fifo { .. } = b.kind {
                let p = producers.get(&b.id).copied().unwrap_or(0);
                let c = consumers.get(&b.id).copied().unwrap_or(0);
                if p != 1 {
                    return Err(IrViolation::FifoProducerCount {
                        buffer: b.id,
                        count: p,
                    });
                }
                if c != 1 {
                    return Err(IrViolation::FifoConsumerCount {
                        buffer: b.id,
                        count: c,
                    });
                }
            }
        }
        Ok(())
    }

    fn validate_carries(&self) -> Result<(), IrViolation> {
        let carries = self.carries();
        if carries.is_empty() {
            return Ok(());
        }
        let levels = self.fold_levels();
        for (buffer, combine) in carries {
            let b = self.buffer(buffer);
            if !matches!(b.kind, BufferKind::Interm { depth_pages: 2, .. }) {
                return Err(IrViolation::CarryNotARing { buffer });
            }
            let updates = self
                .stages
                .iter()
                .filter(|s| s.ins.contains(&buffer) && s.outs.contains(&buffer))
                .count();
            let inits: Vec<&Stage> = self
                .stages
                .iter()
                .filter(|s| s.id != combine && s.outs.contains(&buffer))
                .collect();
            if updates > 1 || inits.len() > 1 {
                return Err(IrViolation::CarryUpdatedTwice { buffer });
            }
            let Some(init) = inits.first() else {
                return Err(IrViolation::CarryWithoutInit { buffer });
            };
            if levels[init.id.idx()] != FoldLevel::Prologue {
                return Err(IrViolation::CarryInitMovesWithTheBlocks { buffer });
            }
        }
        Ok(())
    }

    fn validate_tensors(&self) -> Result<(), IrViolation> {
        let mut loaded = vec![false; self.tensors.len()];
        let mut stores: Vec<Vec<&Stage>> = vec![Vec::new(); self.tensors.len()];
        for s in &self.stages {
            match &s.kind {
                StageKind::Load { tensor, .. } => {
                    if let Some(l) = loaded.get_mut(tensor.idx()) {
                        *l = true;
                    }
                }
                StageKind::Store { tensor, .. } => {
                    if let Some(v) = stores.get_mut(tensor.idx()) {
                        v.push(s);
                    }
                }
                _ => {}
            }
        }
        for t in &self.tensors {
            let rows_t = t.elems.rows.div_ceil(self.tile.rows);
            let cols_t = t.elems.cols.div_ceil(self.tile.cols);
            if t.tiles != (TileGrid { rows_t, cols_t }) {
                return Err(IrViolation::TileGridMismatch { tensor: t.id });
            }
            // ★ A ROLE IS ADMITTED BY ITS CONTRACT, NOT BY ITS NAME. Both of these were flatly refused
            // while the phases that give them meaning were unbuilt; landing those phases did not make
            // the refusal go away, it made it PRECISE. The role is legal exactly where its contract
            // holds, and the violation still names the phase that owns the contract, so a program that
            // claims a lifetime it does not live says which one it broke.
            //
            //   `TensorRole::Activation` — B-P7, sections: produced by one section and consumed by a
            //   later one. A program with no sections has nowhere for that to happen, and a tensor
            //   loaded but never stored (or the reverse) is not an activation, it is an input or an
            //   output wearing the word.
            //
            //   `TensorRole::State` — B-P6, the cross-token carry: the tensor the step UPDATES IN PLACE. A
            //   state nothing stores is an input under another name and is refused. One that is stored but
            //   not loaded by this program is a partial update — a row appended to a cache that another
            //   section, or the next step, reads — and every tile the store does not touch carries.
            let (loaded, stored) = (loaded[t.id.idx()], !stores[t.id.idx()].is_empty());
            match t.role {
                TensorRole::Activation if self.sections.is_empty() || !(loaded && stored) => {
                    return Err(IrViolation::TensorRoleDeferred {
                        tensor: t.id,
                        role: t.role,
                        until: "B-P7",
                    })
                }
                TensorRole::State if !stored => {
                    return Err(IrViolation::TensorRoleDeferred {
                        tensor: t.id,
                        role: t.role,
                        until: "B-P6",
                    })
                }
                _ => {}
            }
            // `TensorSpace::Sram` is where a CARRIED tensor lives so it need not round-trip — B-P6's
            // whole reason for the tier. A tensor with no carried role claiming it is claiming a
            // residency the program never asked for.
            if t.space == TensorSpace::Sram
                && !matches!(t.role, TensorRole::Activation | TensorRole::State)
            {
                return Err(IrViolation::TensorSpaceDeferred {
                    tensor: t.id,
                    space: t.space,
                    until: "B-P6",
                });
            }
        }
        for s in &self.stages {
            match &s.kind {
                StageKind::Load { tensor, .. } => {
                    if !self.tensor(*tensor).role.is_read() {
                        return Err(IrViolation::LoadReadsOutput {
                            stage: s.id,
                            tensor: *tensor,
                        });
                    }
                }
                StageKind::Store { tensor, .. } => {
                    if !self.tensor(*tensor).role.is_written() {
                        return Err(IrViolation::StoreWritesInput {
                            stage: s.id,
                            tensor: *tensor,
                        });
                    }
                }
                _ => {}
            }
        }
        // Every written tile is stored exactly once across the WORK LIST OF ITS OWN SECTION (a state
        // tensor may be updated in part, so it is held to at most once). A carried state is counted section by
        // section — successive sections each update it, ordered by the barriers between them — and every other
        // written tensor across the whole program, since two sections storing one activation are two producers.
        //
        // ★ AN ITEM'S RECTANGLE IS ITS BLOCK OF THE SECTION'S RESULT, and only of that. Any other tensor the section
        // writes — a split softmax's statistics beside its values, of another shape — is covered by the tiles its
        // stores ADDRESS for each item, whenever the address is a function of the item alone; a data-indexed store
        // falls back to the rectangle. So is a result a section stores in parts — an attention whose values are wider
        // than an accumulator page stores each page of an item's block by a store of its own.
        let site_items = std::cell::OnceCell::new();
        for t in self.tensors.iter().filter(|t| t.role.is_written()) {
            let mut counts: BTreeMap<Option<usize>, Vec<usize>> = BTreeMap::new();
            // Placement clones a store per site, so a result's parts are its stores of one section at one site and item.
            let mut parts: BTreeMap<(Option<usize>, Option<Site>, Option<WorkItemId>), usize> = BTreeMap::new();
            for s in &stores[t.id.idx()] {
                *parts.entry((self.section_of_stage(s.id), s.site, s.item)).or_default() += 1;
            }
            for s in &stores[t.id.idx()] {
                let section = self.section_of_stage(s.id);
                let in_parts = parts[&(section, s.site, s.item)] > 1;
                let addressed = match &s.kind {
                    StageKind::Store {
                        pattern,
                        sub_blocks,
                        ..
                    } if self.section_result(section).map(|r| r.id) != Some(t.id) || in_parts => {
                        Some((pattern, *sub_blocks))
                    }
                    _ => None,
                };
                let count = counts
                    .entry(if t.role == TensorRole::State {
                        section
                    } else {
                        None
                    })
                    .or_insert_with(|| vec![0usize; t.tiles.count()]);
                let items: Vec<&WorkItem> = match (s.item, s.site) {
                    (Some(it), _) => vec![&self.work.items[it.idx()]],
                    (None, Some(site)) if self.placement.is_some() => site_items
                        .get_or_init(|| self.site_items())
                        .at(section, site)
                        .iter()
                        .map(|i| &self.work.items[i.idx()])
                        .collect(),
                    _ => match section.and_then(|k| self.sections.get(k)) {
                        Some(sp) => self.work.items[sp.items.0..sp.items.1].iter().collect(),
                        None => self.work.items.iter().collect(),
                    },
                };
                for it in items {
                    let tiles = addressed
                        .and_then(|(pattern, sub_blocks)| pattern.tiles_at(it.coord, sub_blocks))
                        .unwrap_or_else(|| it.out.tiles(t.tiles.cols_t));
                    for tile in tiles {
                        if tile >= count.len() {
                            return Err(IrViolation::OutputOverlap { tensor: t.id, tile });
                        }
                        count[tile] += 1;
                    }
                }
            }
            let uncovered = vec![0usize; t.tiles.count()];
            for count in counts
                .values()
                .chain(counts.is_empty().then_some(&uncovered))
            {
                for (tile, c) in count.iter().enumerate() {
                    if *c == 0 && t.role.is_fully_covered() {
                        return Err(IrViolation::OutputUncovered { tensor: t.id, tile });
                    }
                    if *c > 1 {
                        return Err(IrViolation::OutputOverlap { tensor: t.id, tile });
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_compute(&self) -> Result<(), IrViolation> {
        for s in &self.stages {
            if let StageKind::Compute(ComputeOp::MatmulTiles {
                block,
                operand,
                acc,
                interm,
                ..
            }) = &s.kind
            {
                for b in s.ins.iter().take(2) {
                    let found = self.buffer(*b).dtype;
                    if found != *operand {
                        return Err(IrViolation::OperandDtypeMismatch {
                            stage: s.id,
                            buffer: *b,
                            expected: *operand,
                            found,
                        });
                    }
                }
                if let Some(i) = interm {
                    let found = self.buffer(*i).dtype;
                    if self.numerics == NumericsPolicy::Exact
                        && precision_bytes(found) < precision_bytes(acc.interm_precision())
                    {
                        return Err(IrViolation::IntermNarrowerThanAcc {
                            stage: s.id,
                            interm: *i,
                            acc: *acc,
                            found,
                        });
                    }
                }
                if block.acc_tiles() > self.acc_tiles_used.max(1) {
                    return Err(IrViolation::AccTilesExceeded {
                        stage: s.id,
                        needed: block.acc_tiles(),
                        available: self.acc_tiles_used,
                    });
                }
            }
            if let StageKind::Compute(ComputeOp::Generic {
                iters,
                ins: maps,
                out,
                body,
            }) = &s.kind
            {
                let operands = s.ins.len();
                if maps.len() != operands {
                    return Err(IrViolation::GenericOperandCount {
                        stage: s.id,
                        operands,
                        maps: maps.len(),
                    });
                }
                if let Some(i) = body.max_operand().filter(|i| *i >= operands) {
                    return Err(IrViolation::GenericOperandOutOfRange {
                        stage: s.id,
                        operand: i,
                        operands,
                    });
                }
                // ★ THE CONTRACTION AXIS IS THE THIRD ITERATION VARIABLE AND NOTHING ELSE. It exists to be summed
                // over — `C[r, c] = Σ_k …` — so it is a sum fold over `LV_GEN_K`, the output never steps along it, and
                // a map may step along it only in a space that has it.
                let malformed = |reason| {
                    Err(IrViolation::ContractionMalformed {
                        stage: s.id,
                        reason,
                    })
                };
                match iters.get(2) {
                    Some(_) if iters.len() > 3 => return malformed("a fourth iteration axis — a Generic has rows, columns and one contraction"),
                    Some(k) if k.var != crate::spatial::passes::LV_GEN_K => return malformed("a contraction axis over a variable other than LV_GEN_K"),
                    Some(k) if !matches!(k.kind, IterKind::Reduce { pool: ReducePool::Sum }) => return malformed("a contraction axis that is not a sum fold"),
                    Some(_) if out.varies_along(crate::spatial::passes::LV_GEN_K) => return malformed("an output map that steps along the contraction axis it folds"),
                    None if maps.iter().chain(std::iter::once(out)).any(|m| m.varies_along(crate::spatial::passes::LV_GEN_K)) => {
                        return malformed("a map stepping along a contraction axis the iteration space does not have")
                    }
                    _ => {}
                }
                for m in maps.iter().chain(std::iter::once(out)) {
                    if let Some(ind) = m.indirect {
                        if ind.operand >= operands {
                            return Err(IrViolation::IndirectOperandOutOfRange {
                                stage: s.id,
                                operand: ind.operand,
                                operands,
                            });
                        }
                        if ind.modulus == 0 {
                            return Err(IrViolation::IndirectMalformed {
                                stage: s.id,
                                reason: "an indirect term's modulus must be nonzero",
                            });
                        }
                    }
                }
                // A scatter is an indirect OUTPUT map; it means nothing unless the stage's result is drained
                // by a Store that scatters through an indirect tile address.
                if out.indirect.is_some() {
                    let scattered = s.outs.iter().any(|b| {
                        self.stages.iter().any(|st| matches!(&st.kind, StageKind::Store { pattern, .. } if st.ins.contains(b) && pattern.origin.indirect.is_some()))
                    });
                    if !scattered {
                        return Err(IrViolation::IndirectOutWithoutScatter { stage: s.id });
                    }
                }
                // An index is not a value: an ArgMax fold packs into an integer buffer.
                if iters.iter().any(|a| {
                    matches!(
                        a.kind,
                        IterKind::Reduce {
                            pool: ReducePool::ArgMax
                        }
                    )
                }) {
                    for b in &s.outs {
                        let found = self.buffer(*b).dtype;
                        if precision_int_bits(found).is_none() {
                            return Err(IrViolation::ArgMaxOutputNotInteger { stage: s.id, found });
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Every indirect tile address names an index buffer the stage reads, with fields that can mean
    /// something. (Its evaluation is B-P5; this is the structural half that exists from B-P3.)
    fn validate_indirect(&self) -> Result<(), IrViolation> {
        for s in &self.stages {
            let pattern = match &s.kind {
                StageKind::Load { pattern, .. } | StageKind::Store { pattern, .. } => pattern,
                _ => continue,
            };
            let Some(ind) = pattern.origin.indirect else {
                continue;
            };
            if ind.index.idx() >= self.buffers.len() {
                return Err(IrViolation::DanglingBuffer {
                    stage: Some(s.id),
                    buffer: ind.index,
                });
            }
            if !s.ins.contains(&ind.index) {
                return Err(IrViolation::IndirectIndexNotAnOperand {
                    stage: s.id,
                    buffer: ind.index,
                });
            }
            if ind.tile_stride == 0 || ind.elems_per_tile == 0 {
                return Err(IrViolation::IndirectMalformed {
                    stage: s.id,
                    reason: "an indirect tile's stride and elements-per-tile must be nonzero",
                });
            }
            if ind.bound.0 > ind.bound.1 {
                return Err(IrViolation::IndirectMalformed {
                    stage: s.id,
                    reason: "an indirect tile's bound is inverted",
                });
            }
        }
        Ok(())
    }

    /// Whether any access anywhere in the program — a Load/Store tile address, a `Generic` map, in the
    /// stages or in the schedule's own ops — is value-dependent. A lowering with no gather semantics
    /// refuses the whole program by name on this, before it could print past the term.
    pub fn has_indirect_access(&self) -> bool {
        fn op_has(op: &ComputeOp) -> bool {
            matches!(op, ComputeOp::Generic { ins, out, .. } if ins.iter().chain(std::iter::once(out)).any(|m| m.indirect.is_some()))
        }
        let in_stages = self.stages.iter().any(|s| match &s.kind {
            StageKind::Load { pattern, .. } | StageKind::Store { pattern, .. } => {
                pattern.origin.indirect.is_some()
            }
            StageKind::Compute(op) => op_has(op),
            _ => false,
        });
        if in_stages {
            return true;
        }
        let mut found = false;
        if let Some(sch) = &self.schedule {
            for p in &sch.programs {
                walk_tagged(&p.ops, 1, &mut |t, _| {
                    found |= match &t.op {
                        ChoreoOp::ReadTiles { at, .. } | ChoreoOp::WriteTiles { at, .. } => {
                            at.indirect.is_some()
                        }
                        ChoreoOp::Compute { op, .. } => op_has(op),
                        _ => false,
                    };
                });
            }
        }
        found
    }

    /// Whether a `Generic` is the PLAIN pointwise form and nothing more: every axis parallel, every
    /// operand map the exact identity over its own page, the output map the identity with no indirect
    /// term. The integer fabric's elementwise datapath can express exactly this and refuses the rest
    /// by name; a printer that pattern-matched the body alone would lower a transposed or gathered
    /// operand as if it were the identity.
    pub fn generic_is_plain_pointwise(
        &self,
        iters: &[IterSpec],
        maps: &[ElemAccess],
        out: &ElemAccess,
        ins: &[BufferId],
    ) -> bool {
        let [row_ax, col_ax] = iters else {
            return false;
        };
        if !matches!(row_ax.kind, IterKind::Parallel) || !matches!(col_ax.kind, IterKind::Parallel)
        {
            return false;
        }
        if maps.len() != ins.len()
            || out.indirect.is_some()
            || !out.varies_along(row_ax.var)
            || !out.varies_along(col_ax.var)
        {
            return false;
        }
        maps.iter().zip(ins).all(|(m, b)| {
            let width = self.buffer(*b).kind.page_tiles() * self.tile.cols;
            m.broadcast_class(row_ax.var, col_ax.var, width) == Some((true, true))
        })
    }

    fn validate_plan(&self) -> Result<(), IrViolation> {
        let Some(p) = self.work.plan else {
            return Ok(());
        };
        if p.per_site_m == 0 || p.block.osh == 0 || p.per_site_m % p.block.osh != 0 {
            return Err(IrViolation::BlockDivisibility {
                what: "per_site_m % osh",
                value: p.per_site_m,
                divisor: p.block.osh,
            });
        }
        if p.per_site_n == 0 || p.block.osw == 0 || p.per_site_n % p.block.osw != 0 {
            return Err(IrViolation::BlockDivisibility {
                what: "per_site_n % osw",
                value: p.per_site_n,
                divisor: p.block.osw,
            });
        }
        if p.block.ibw == 0 || self.work.k_tiles % p.block.ibw != 0 {
            return Err(IrViolation::BlockDivisibility {
                what: "k_tiles % ibw",
                value: self.work.k_tiles,
                divisor: p.block.ibw,
            });
        }
        if p.inner.num_blocks != self.work.k_tiles / p.block.ibw
            || p.inner.block_k_tiles != p.block.ibw
        {
            return Err(IrViolation::BlockDivisibility {
                what: "num_blocks",
                value: p.inner.num_blocks,
                divisor: p.block.ibw,
            });
        }
        // The grid is the block grid of what the work WRITES: the whole result tensor when it is written whole; for
        // an in-place update of carried state (a row appended to a cache) only the items' blocks, one per item —
        // the table around them is not work, and a grid of every row would price one append as the whole cache.
        //
        // ★ A COMPOSED PROGRAM'S PLAN IS ITS LAST SECTION'S, so it is held to that section's result and that section's
        // items — not to the first result the program has, nor to every section's items. A program whose every
        // effect is an update of carried state (a table written row by row) has no `Output` to stand in for it.
        let last = self.sections.len().checked_sub(1);
        let items = match last.and_then(|k| self.sections.get(k)) {
            Some(sp) => sp.items.1 - sp.items.0,
            None => self.work.items.len(),
        };
        if let Some(out) = self.section_result(last) {
            let expected = if out.role.is_fully_covered() {
                (
                    out.tiles.rows_t.div_ceil(p.per_site_m),
                    out.tiles.cols_t.div_ceil(p.per_site_n),
                )
            } else {
                (1, items)
            };
            if p.grid != expected {
                return Err(IrViolation::GridMismatch {
                    expected,
                    found: p.grid,
                });
            }
        }
        Ok(())
    }

    fn validate_phase_order(&self) -> Result<(), IrViolation> {
        if self.placement.is_some() && self.work.plan.is_none() {
            return Err(IrViolation::PhaseOrder {
                have: "placement",
                missing: "plan",
            });
        }
        if self.memory.is_some() && self.placement.is_none() {
            return Err(IrViolation::PhaseOrder {
                have: "memory",
                missing: "placement",
            });
        }
        if self.schedule.is_some() && self.memory.is_none() {
            return Err(IrViolation::PhaseOrder {
                have: "schedule",
                missing: "memory",
            });
        }
        if self.certificate.is_some() && self.schedule.is_none() {
            return Err(IrViolation::PhaseOrder {
                have: "certificate",
                missing: "schedule",
            });
        }
        Ok(())
    }

    fn validate_placement(&self) -> Result<(), IrViolation> {
        let Some(pl) = &self.placement else {
            return Ok(());
        };
        if pl.site_of.len() != self.stages.len() || pl.engine_of.len() != self.stages.len() {
            return Err(IrViolation::StageUnplaced {
                stage: StageId::new(pl.site_of.len().min(pl.engine_of.len())),
            });
        }
        for s in &self.stages {
            let Some(site) = s.site else {
                return Err(IrViolation::StageUnplaced { stage: s.id });
            };
            if pl.site_of[s.id.idx()] != site || pl.engine_of[s.id.idx()].site != site {
                return Err(IrViolation::EngineSiteMismatch { stage: s.id });
            }
            let role_ok = match (s.role, pl.engine_of[s.id.idx()].engine) {
                (SiteRole::Movement, Engine::Movement(_)) => true,
                (SiteRole::Compute, Engine::Compute) => true,
                _ => false,
            };
            if !role_ok || s.role != s.kind.role() {
                return Err(IrViolation::EngineRoleMismatch { stage: s.id });
            }
        }
        for e in &self.edges {
            let from_site = self.stage(e.from).site;
            for t in &e.to {
                let to_site = self.stage(*t).site;
                if e.handshake.is_local() && from_site != to_site {
                    return Err(IrViolation::CrossSiteLocalEdge { edge: e.id });
                }
                if !e.handshake.is_local()
                    && from_site == to_site
                    && !matches!(e.handshake, Handshake::Barrier { .. })
                {
                    return Err(IrViolation::LocalCrossSiteEdge { edge: e.id });
                }
            }
        }
        let mut members_of: BTreeMap<(GroupId, Site), Vec<&Stage>> = BTreeMap::new();
        if !pl.groups.is_empty() {
            for s in &self.stages {
                if let (Some(g), Some(site)) = (self.stage_group(s), s.site) {
                    members_of.entry((g, site)).or_default().push(s);
                }
            }
        }
        let site_items = if pl.groups.is_empty() {
            None
        } else {
            Some(self.site_items())
        };
        for g in &pl.groups {
            if !g.rect.contains(g.sender) {
                return Err(IrViolation::GroupMalformed {
                    group: g.id,
                    reason: "sender outside rect",
                });
            }
            match g.direction {
                Direction::Row => {
                    if g.rect.y0 != g.rect.y1 || g.sender.x != g.rect.x0 {
                        return Err(IrViolation::GroupMalformed {
                            group: g.id,
                            reason: "row group must be one row with the sender at its left",
                        });
                    }
                }
                Direction::Col => {
                    if g.rect.x0 != g.rect.x1 || g.sender.y != g.rect.y0 {
                        return Err(IrViolation::GroupMalformed {
                            group: g.id,
                            reason: "column group must be one column with the sender at its top",
                        });
                    }
                }
            }
            if g.rect.len() < 2 {
                return Err(IrViolation::GroupMalformed {
                    group: g.id,
                    reason: "a multicast group needs at least one receiver",
                });
            }
            let mut member_kind: Option<(BufferKind, Precision)> = None;
            let mut member_offset: Option<u32> = None;
            for site in g.rect.sites() {
                let members: &[&Stage] = members_of
                    .get(&(g.id, site))
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                if members.len() != 1 {
                    return Err(IrViolation::GroupMemberMismatch {
                        group: g.id,
                        site,
                        reason: "exactly one member stage per site",
                    });
                }
                let m = members[0];
                let is_sender = site == g.sender;
                match &m.kind {
                    StageKind::Load {
                        multicast: Some(_), ..
                    } if is_sender => {}
                    StageKind::Recv { .. } if !is_sender => {}
                    _ => {
                        return Err(IrViolation::GroupMemberMismatch {
                            group: g.id,
                            site,
                            reason: "sender must be a multicasting Load, receivers must be Recv",
                        })
                    }
                }
                let Some(out) = m.outs.first() else {
                    return Err(IrViolation::GroupMemberMismatch {
                        group: g.id,
                        site,
                        reason: "member has no output buffer",
                    });
                };
                let b = self.buffer(*out);
                match member_kind {
                    None => member_kind = Some((b.kind, b.dtype)),
                    Some(k) => {
                        if k != (b.kind, b.dtype) {
                            return Err(IrViolation::GroupMemberMismatch {
                                group: g.id,
                                site,
                                reason: "member buffers differ in kind or dtype",
                            });
                        }
                    }
                }
                if let Some(mm) = &self.memory {
                    let at = mm
                        .buffer_at
                        .get(out.idx())
                        .ok_or(IrViolation::MemoryUnmapped { buffer: *out })?;
                    match member_offset {
                        None => member_offset = Some(at.offset_bytes),
                        Some(o) => {
                            if o != at.offset_bytes {
                                return Err(IrViolation::GroupMemberMismatch {
                                    group: g.id,
                                    site,
                                    reason:
                                        "multicast lands at one L1 address: member offsets differ",
                                });
                            }
                        }
                    }
                }
                // Sender and receivers run the rounds of the group's OWN section: a composed site also holds
                // other sections' items, which this group never sends.
                let section = self.section_of_stage(m.id);
                let at = |x: Site| site_items.as_ref().map_or(0, |ix| ix.at(section, x).len());
                let (n_items, n_sender) = (at(site), at(g.sender));
                if n_items != n_sender {
                    return Err(IrViolation::ItemCountMismatch { group: g.id });
                }
            }
        }
        Ok(())
    }

    fn validate_memory(&self) -> Result<(), IrViolation> {
        let Some(mm) = &self.memory else {
            return Ok(());
        };
        if mm.buffer_at.len() != self.buffers.len() {
            return Err(IrViolation::MemoryUnmapped {
                buffer: BufferId::new(mm.buffer_at.len()),
            });
        }
        // ★ THE OVERLAP RULE IS PHASE-SCOPED, AND THAT IS THE POINT. Two phases are separated by a barrier every
        // engine passes through, so a later phase's buffers may sit exactly where an earlier one's did — which is what
        // lets a hundred-op forward run in one site's local memory instead of needing a hundred times as much. WITHIN a
        // phase its sections run at once, so two live buffers of the phase — of one section or of two — sharing a byte
        // is `MemoryOverlap`; `sections_reuse_local_memory` proves both halves. (A program whose every section closes on
        // a barrier has one section a phase: the rule is the section-scoped one it always was.)
        let phases = self.section_phases();
        let mut by_site: BTreeMap<(Option<usize>, Site, MemSpace), Vec<(u32, u32, BufferId)>> =
            BTreeMap::new();
        for b in &self.buffers {
            if b.alias_of.is_some() || b.space == MemSpace::Acc {
                continue;
            }
            let at = mm.buffer_at[b.id.idx()];
            let phase = self.section_of_buffer(b.id).map(|k| phases[k]);
            by_site
                .entry((phase, at.site, at.space))
                .or_default()
                .push((at.offset_bytes, at.offset_bytes + at.bytes, b.id));
        }
        for ((_, site, _), mut v) in by_site {
            v.sort();
            for w in v.windows(2) {
                if w[1].0 < w[0].1 {
                    return Err(IrViolation::MemoryOverlap {
                        site,
                        a: w[0].2,
                        b: w[1].2,
                    });
                }
            }
        }
        Ok(())
    }

    fn validate_schedule(&self) -> Result<(), IrViolation> {
        let Some(sch) = &self.schedule else {
            return Ok(());
        };
        let mut seen: BTreeSet<(Site, Engine)> = BTreeSet::new();
        for p in &sch.programs {
            if !seen.insert((p.site, p.engine)) {
                return Err(IrViolation::DuplicateEngineProgram {
                    site: p.site,
                    engine: p.engine,
                });
            }
        }
        // ★ THE SCHEDULE IS THE PROGRAM THE STRUCTURAL PROOFS ARE ABOUT.
        //
        // `validate_tensors` proves every output tile is stored exactly once ACROSS THE WORK LIST, and the
        // certificate's dry run trusts exactly that proof instead of re-checking coverage itself — the
        // comment at `eval.rs`'s `finish` says so: "output coverage is then the structural invariant
        // `validate` proves". Both statements are about STAGES and ITEMS. Neither notices when the
        // SCHEDULE does not issue the stage's data movement.
        //
        // So: delete the `WriteTiles` out of a store stage's op list, and the work list still claims every
        // tile. Coverage passes. The dry run declines to look. A certificate is issued for a program whose
        // real evaluation faults with `OutputTileNeverStored` — which is what
        // `a_certificate_is_never_issued_to_a_program_whose_evaluation_faults` found at case 105.
        //
        // A store stage that writes nothing is not a store stage, whatever the work list says about it.
        //
        // ★ ONE PASS OVER THE OPS, NOT ONE PER STORE. Asking `per_stage` for each store stage rescans
        // EVERY program's ops — and allocates a `Vec` while doing it — so the check costs
        // `stores × ops`. That is invisible on a single-op program and fatal on a composed layer: at
        // Qwen3.8's real widths one layer is ~14,800 stages over a few hundred thousand ops, and this
        // loop alone ran for sixteen minutes and 13 GB before the rest of `validate` was reached. The
        // question it asks is a SET MEMBERSHIP — which stages issue a `WriteTiles` — so the set is built
        // once and consulted in constant time. Same answer, same refusal, linear.
        let writes_tiles: BTreeSet<StageId> = {
            let mut v = BTreeSet::new();
            fn walk(ops: &[TaggedOp], out: &mut BTreeSet<StageId>) {
                for t in ops {
                    if matches!(t.op, ChoreoOp::WriteTiles { .. }) {
                        out.insert(t.stage);
                    }
                    if let ChoreoOp::Loop { body, .. } = &t.op {
                        walk(body, out);
                    }
                }
            }
            for p in &sch.programs {
                walk(&p.ops, &mut v);
            }
            v
        };
        for st in &self.stages {
            if matches!(st.kind, StageKind::Store { .. }) && !writes_tiles.contains(&st.id) {
                return Err(IrViolation::StageNotScheduled {
                    stage: st.id,
                    op: "WriteTiles",
                });
            }
        }
        let mut reserved: BTreeMap<BufferId, u64> = BTreeMap::new();
        let mut pushed: BTreeMap<BufferId, u64> = BTreeMap::new();
        let mut popped: BTreeMap<BufferId, u64> = BTreeMap::new();
        let mut incs: BTreeMap<SemId, u64> = BTreeMap::new();
        let mut waits: BTreeMap<SemId, u64> = BTreeMap::new();
        let mut sets_valid: BTreeMap<(SemId, Site), u64> = BTreeMap::new();
        let mut resets: BTreeMap<SemId, u64> = BTreeMap::new();
        /// Wait OPERATIONS, as distinct from `waits`, which sums the COUNTS being waited for so it can be
        /// compared against `incs`. A counter waited twice for two increments each contributes 4 to
        /// `waits` and 2 here, and it is this one the reset has to match.
        let mut wait_ops: BTreeMap<SemId, u64> = BTreeMap::new();
        let mut invalidations: BTreeMap<(SemId, Site), u64> = BTreeMap::new();
        let mut valid_waits: BTreeMap<(SemId, Site), u64> = BTreeMap::new();
        // A rendezvous is checked site by site: posts by the site they land on, waits by the site of the
        // program that waits and the count it waits for.
        let mut posts_at: BTreeMap<(SemId, Site), u64> = BTreeMap::new();
        let mut barrier_waits: BTreeMap<SemId, Vec<(Site, SemCond, u64)>> = BTreeMap::new();
        for p in &sch.programs {
            let mut acq = 0u64;
            let mut rel = 0u64;
            let mut acq_over: Option<usize> = None;
            let mut computed_since_acquire = true;
            let mut pack_without_compute = false;
            walk_tagged_ops(&p.ops, 1, &mut |op, m| match op {
                ChoreoOp::Compute { .. } => computed_since_acquire = true,
                ChoreoOp::PackAcc { .. } if !computed_since_acquire => pack_without_compute = true,
                ChoreoOp::Reserve { buf, pages } => {
                    *reserved.entry(*buf).or_default() += m * *pages as u64
                }
                ChoreoOp::Push { buf, pages } => {
                    *pushed.entry(*buf).or_default() += m * *pages as u64
                }
                ChoreoOp::Pop { buf, pages } => {
                    *popped.entry(*buf).or_default() += m * *pages as u64
                }
                ChoreoOp::SemInc { sem, at, by } => {
                    *incs.entry(*sem).or_default() += m * *by as u64;
                    if matches!(self.semaphores[sem.idx()].role, SemRole::Barrier { .. }) {
                        *posts_at.entry((*sem, *at)).or_default() += m * *by as u64;
                    }
                }
                // ★ THE RESET IS LOAD-BEARING. A ready counter waited on with `Equals(n)` has to be put
                // back to zero before the next block, or that block's wait sees the PREVIOUS block's
                // count and passes immediately — the sender starts overwriting a buffer whose readers
                // have not finished with it. Nothing here counted the reset, so dropping one was a legal
                // program: `dropping_any_load_bearing_op_is_never_silently_correct` found it by deleting
                // a `SemSet { Count(0) }` and getting a correct answer out of an evaluator that does not
                // model the race.
                ChoreoOp::SemSet {
                    sem,
                    value: SemValue::Count(0),
                } => *resets.entry(*sem).or_default() += m,
                // The receiver's own clearing of its valid flag, which has to happen before it waits on
                // that flag again or the next round is satisfied by the previous round's Valid.
                ChoreoOp::SemSet {
                    sem,
                    value: SemValue::Invalid,
                } => *invalidations.entry((*sem, p.site)).or_default() += m,
                ChoreoOp::SemWait { sem, until } => match self.semaphores[sem.idx()].role {
                    SemRole::ValidFlag => *valid_waits.entry((*sem, p.site)).or_default() += m,
                    role => {
                        let n = match until {
                            SemCond::Equals(n) | SemCond::AtLeast(n) => *n as u64,
                        };
                        *waits.entry(*sem).or_default() += m * n;
                        *wait_ops.entry(*sem).or_default() += m;
                        if matches!(role, SemRole::Barrier { .. }) {
                            barrier_waits
                                .entry(*sem)
                                .or_default()
                                .push((p.site, *until, m));
                        }
                    }
                },
                ChoreoOp::McastSetValid { sem, .. } => {
                    *sets_valid.entry((*sem, p.site)).or_default() += m
                }
                ChoreoOp::AcquireAcc { tiles } => {
                    computed_since_acquire = false;
                    acq += m;
                    if *tiles > self.acc_tiles_used.max(1) {
                        acq_over = Some(*tiles);
                    }
                }
                ChoreoOp::ReleaseAcc => rel += m,
                _ => {}
            });
            if acq != rel {
                return Err(IrViolation::AccImbalance {
                    site: p.site,
                    acquires: acq,
                    releases: rel,
                });
            }
            if pack_without_compute {
                return Err(IrViolation::AccumulatorNeverComputed {
                    site: p.site,
                    engine: p.engine,
                });
            }
            if let Some(t) = acq_over {
                return Err(IrViolation::AccTilesExceeded {
                    stage: p.stages.first().copied().unwrap_or(StageId(0)),
                    needed: t,
                    available: self.acc_tiles_used,
                });
            }
        }
        for b in &self.buffers {
            if b.space == MemSpace::Acc {
                continue;
            }
            let r = reserved.get(&b.id).copied().unwrap_or(0);
            let pu = pushed.get(&b.id).copied().unwrap_or(0);
            let po = popped.get(&b.id).copied().unwrap_or(0);
            if r != pu || pu != po {
                return Err(IrViolation::CountingImbalance {
                    buffer: b.id,
                    reserved: r,
                    pushed: pu,
                    popped: po,
                });
            }
        }
        // ★ EVERY COUNT OF A TRIP BALANCES, NOT ONLY THE GREATEST. The totals above take each trip at its greatest
        // count. A total is a constant plus a multiple of each trip's count, so it balances at every count exactly
        // when the multiples balance too: the pages each handshake moves per round of a trip loop.
        if !self.trips.is_empty() {
            fn per_round(
                ops: &[TaggedOp],
                mult: u64,
                trip: Option<TripId>,
                f: &mut dyn FnMut(&ChoreoOp, u64, Option<TripId>),
            ) {
                for t in ops {
                    f(&t.op, mult, trip);
                    if let ChoreoOp::Loop { count, body, .. } = &t.op {
                        match count.trip() {
                            Some(id) => per_round(body, mult, Some(id), f),
                            None => per_round(body, mult * count.bound() as u64, trip, f),
                        }
                    }
                }
            }
            let mut moved: BTreeMap<(BufferId, TripId), [u64; 3]> = BTreeMap::new();
            for p in &sch.programs {
                let mut acc: BTreeMap<TripId, [u64; 2]> = BTreeMap::new();
                per_round(&p.ops, 1, None, &mut |op, m, trip| {
                    let Some(trip) = trip else { return };
                    match op {
                        ChoreoOp::Reserve { buf, pages } => {
                            moved.entry((*buf, trip)).or_default()[0] += m * *pages as u64
                        }
                        ChoreoOp::Push { buf, pages } => {
                            moved.entry((*buf, trip)).or_default()[1] += m * *pages as u64
                        }
                        ChoreoOp::Pop { buf, pages } => {
                            moved.entry((*buf, trip)).or_default()[2] += m * *pages as u64
                        }
                        ChoreoOp::AcquireAcc { .. } => acc.entry(trip).or_default()[0] += m,
                        ChoreoOp::ReleaseAcc => acc.entry(trip).or_default()[1] += m,
                        _ => {}
                    }
                });
                if let Some([a, r]) = acc.values().find(|[a, r]| a != r) {
                    return Err(IrViolation::AccImbalance {
                        site: p.site,
                        acquires: *a,
                        releases: *r,
                    });
                }
            }
            for ((buffer, _), [r, pu, po]) in &moved {
                if self.buffer(*buffer).space != MemSpace::Acc && (r != pu || pu != po) {
                    return Err(IrViolation::CountingImbalance {
                        buffer: *buffer,
                        reserved: *r,
                        pushed: *pu,
                        popped: *po,
                    });
                }
            }
        }
        for s in &self.semaphores {
            match s.role {
                // ★ A RENDEZVOUS COUNTS PER SITE AND PER PHASE, NOT AS A HAND-OFF.
                //
                // Gather: `n` posts, every one at its single root site, and ONE wait there, for `n`.
                // Release: exactly one post at each of its sites and none elsewhere, and `n` waits, each
                // for 1 at one of those sites. A dropped post, a dropped wait, and a post landed on the
                // wrong site each break one of these equalities. WHERE the ops sit in each program — the
                // property that makes the barrier order anything — is `validate_rendezvous`'s question.
                SemRole::Barrier { n, phase } => {
                    let i = incs.get(&s.id).copied().unwrap_or(0);
                    let ops = wait_ops.get(&s.id).copied().unwrap_or(0);
                    let unbalanced = IrViolation::SectionBarrierUnbalanced {
                        sem: s.id,
                        participants: n,
                        sites: s.sites.len(),
                        incs: i,
                        wait_ops: ops,
                    };
                    let sites: BTreeSet<Site> = s.sites.iter().copied().collect();
                    let (post_each, want_wait_ops, wait_for) = match phase {
                        BarrierPhase::Gather => (n as u64, 1u64, n),
                        BarrierPhase::Release => (1u64, n as u64, 1),
                    };
                    let shape_ok = sites.len() == s.sites.len()
                        && !sites.is_empty()
                        && (phase == BarrierPhase::Release || sites.len() == 1);
                    let posts_ok = i == post_each * sites.len() as u64
                        && sites
                            .iter()
                            .all(|x| posts_at.get(&(s.id, *x)).copied().unwrap_or(0) == post_each);
                    let waiting = barrier_waits.get(&s.id).map(Vec::as_slice).unwrap_or(&[]);
                    let waits_ok = ops == want_wait_ops
                        && waiting.iter().all(|(site, until, _)| {
                            *until == SemCond::AtLeast(wait_for) && sites.contains(site)
                        });
                    if !(shape_ok && posts_ok && waits_ok) {
                        return Err(unbalanced);
                    }
                }
                SemRole::ReadyCounter { .. } => {
                    let i = incs.get(&s.id).copied().unwrap_or(0);
                    let w = waits.get(&s.id).copied().unwrap_or(0);
                    if i != w {
                        return Err(IrViolation::SemaphoreImbalance {
                            sem: s.id,
                            incs: i,
                            waits: w,
                        });
                    }
                    // ★ THE RESET IS LOAD-BEARING. A ready counter waited on with `Equals(n)` must be
                    // put back to zero before the next round, or that round's wait is satisfied by the
                    // PREVIOUS round's count and the sender starts overwriting a buffer whose readers
                    // have not finished with it. Nothing checked it, so deleting a `SemSet { Count(0) }`
                    // produced a program that validated AND evaluated correctly — the evaluator does not
                    // model the race, which is exactly why the graph has to.
                    //
                    // It is matched against wait OPERATIONS, not `waits`: that sums the counts being
                    // waited for so it can be compared with `incs`, so a counter waited twice for two
                    // increments each scores 4 there and 2 resets here. Getting that wrong is what made
                    // the first version of this rule reject every real program.
                    if matches!(s.role, SemRole::ReadyCounter { .. }) {
                        let ops = wait_ops.get(&s.id).copied().unwrap_or(0);
                        let r = resets.get(&s.id).copied().unwrap_or(0);
                        if ops != r {
                            return Err(IrViolation::SemaphoreNotReset {
                                sem: s.id,
                                waits: ops,
                                resets: r,
                            });
                        }
                    }
                }
                SemRole::ValidFlag => {
                    // A shared flag is set by each receiver's OWN sender (the group of the flag's operand that
                    // holds the receiver); every receiver waits exactly as often as its sender sets.
                    for site in &s.sites {
                        let w = valid_waits.get(&(s.id, *site)).copied().unwrap_or(0);
                        if w == 0 {
                            continue;
                        }
                        let setter = self.placement.as_ref().and_then(|pl| {
                            s.operand.and_then(|op| {
                                pl.groups
                                    .iter()
                                    .find(|g| g.operand == op && g.rect.contains(*site))
                                    .map(|g| g.sender)
                            })
                        });
                        let sets = match setter {
                            Some(sender) => sets_valid.get(&(s.id, sender)).copied().unwrap_or(0),
                            None => sets_valid
                                .iter()
                                .filter(|((id, _), _)| *id == s.id)
                                .map(|(_, v)| *v)
                                .sum(),
                        };
                        if w != sets {
                            return Err(IrViolation::SemaphoreImbalance {
                                sem: s.id,
                                incs: sets,
                                waits: w,
                            });
                        }
                        // ★ AND THE RECEIVER MUST CLEAR IT FIRST. A valid flag left Valid from the last
                        // round satisfies this round's wait immediately, so the receiver reads a buffer
                        // the sender has not refilled. The mirror of the ready counter's reset, and found
                        // the same way: the mutation test deleted a `SemSet { Invalid }` and got a correct
                        // answer out of an evaluator that does not model the race.
                        let cleared = invalidations.get(&(s.id, *site)).copied().unwrap_or(0);
                        if cleared != w {
                            return Err(IrViolation::SemaphoreNotReset {
                                sem: s.id,
                                waits: w,
                                resets: cleared,
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Every rendezvous sits on its section boundary in every engine program.
    ///
    /// Counts say the barrier is complete; placement says it ORDERS. In each program, boundary `k` is an
    /// arrival after every op of sections `≤ k` and a release wait before every op of sections `> k`, and
    /// the boundaries follow one another (`release wait k` before `arrival k+1`). The root — the one
    /// program that waits on the gather — arrives, waits, posts every release, then waits for its own, and
    /// no other program posts a release. Under those, any op of section `a` in any program happens before
    /// any op of section `b > a` in any program, through the root of boundary `a`.
    ///
    /// Given the chain, an op of section `s` need only follow release wait `s - 1` and precede arrival `s`,
    /// so the check is one pass over each program.
    fn validate_rendezvous(&self) -> Result<(), IrViolation> {
        let Some(sch) = &self.schedule else {
            return Ok(());
        };
        let mut closing: BTreeMap<SemId, (usize, BarrierPhase)> = BTreeMap::new();
        for (k, sp) in self.sections.iter().enumerate() {
            let Some(r) = sp.barrier_after else { continue };
            let role = |id: SemId| self.semaphores.get(id.idx()).map(|s| s.role);
            let owned = |id: SemId| id.idx() >= sp.semaphores.0 && id.idx() < sp.semaphores.1;
            let well_formed = matches!(
                role(r.gather),
                Some(SemRole::Barrier {
                    phase: BarrierPhase::Gather,
                    ..
                })
            ) && matches!(
                role(r.release),
                Some(SemRole::Barrier {
                    phase: BarrierPhase::Release,
                    ..
                })
            ) && owned(r.gather)
                && owned(r.release);
            if !well_formed {
                return Err(IrViolation::SectionSpanMalformed {
                    section: k,
                    what:
                        "a rendezvous whose semaphores are not its own section's gather and release",
                });
            }
            closing.insert(r.gather, (k, BarrierPhase::Gather));
            closing.insert(r.release, (k, BarrierPhase::Release));
        }
        if let Some(stray) = self
            .semaphores
            .iter()
            .find(|s| matches!(s.role, SemRole::Barrier { .. }) && !closing.contains_key(&s.id))
        {
            let section = self
                .sections
                .iter()
                .position(|sp| {
                    stray.id.idx() >= sp.semaphores.0 && stray.id.idx() < sp.semaphores.1
                })
                .unwrap_or(0);
            return Err(IrViolation::SectionSpanMalformed {
                section,
                what: "a barrier semaphore that closes no section",
            });
        }
        if closing.is_empty() {
            return Ok(());
        }
        let n = self.sections.len();
        let closes: Vec<bool> = self
            .sections
            .iter()
            .map(|sp| sp.barrier_after.is_some())
            .collect();
        let mut section_of = vec![usize::MAX; self.stages.len()];
        for (k, sp) in self.sections.iter().enumerate() {
            for slot in section_of.iter_mut().take(sp.stages.1).skip(sp.stages.0) {
                *slot = k;
            }
        }
        let rendezvous_sem = |op: &ChoreoOp| match op {
            ChoreoOp::SemInc { sem, .. } | ChoreoOp::SemWait { sem, .. } => {
                closing.get(sem).copied()
            }
            _ => None,
        };
        let root_of: BTreeMap<usize, (Site, Engine)> = sch
            .programs
            .iter()
            .flat_map(|p| {
                p.ops
                    .iter()
                    .filter_map(move |t| match (&t.op, rendezvous_sem(&t.op)) {
                        (ChoreoOp::SemWait { .. }, Some((k, BarrierPhase::Gather))) => {
                            Some((k, (p.site, p.engine)))
                        }
                        _ => None,
                    })
            })
            .collect();
        for p in &sch.programs {
            let misplaced =
                |section: usize, what: &'static str| IrViolation::SectionBarrierMisplaced {
                    section,
                    site: p.site,
                    engine: p.engine,
                    what,
                };
            let mut nested: Option<usize> = None;
            for t in &p.ops {
                if let ChoreoOp::Loop { body, .. } = &t.op {
                    walk_tagged_ops(body, 1, &mut |op, _| {
                        if let Some((k, _)) = rendezvous_sem(op) {
                            nested.get_or_insert(k);
                        }
                    });
                }
            }
            if let Some(k) = nested {
                return Err(misplaced(k, "a rendezvous op inside a loop body"));
            }
            // Per boundary: the arrival, the gather wait, the first and last release post, the release wait.
            let mut arrive: Vec<Option<usize>> = vec![None; n];
            let mut gather_wait: Vec<Option<usize>> = vec![None; n];
            let mut posts: Vec<Option<(usize, usize)>> = vec![None; n];
            let mut release_wait: Vec<Option<usize>> = vec![None; n];
            for (i, t) in p.ops.iter().enumerate() {
                let Some((k, phase)) = rendezvous_sem(&t.op) else {
                    continue;
                };
                let slot = match (&t.op, phase) {
                    (ChoreoOp::SemInc { .. }, BarrierPhase::Gather) => &mut arrive[k],
                    (ChoreoOp::SemWait { .. }, BarrierPhase::Gather) => &mut gather_wait[k],
                    (ChoreoOp::SemWait { .. }, BarrierPhase::Release) => &mut release_wait[k],
                    _ => {
                        let span = posts[k].get_or_insert((i, i));
                        span.1 = i;
                        continue;
                    }
                };
                if slot.replace(i).is_some() {
                    return Err(misplaced(k, "a program that passes one rendezvous twice"));
                }
            }
            // `next_arrival[s]`: the arrival of the first boundary at or after section `s`.
            // `release_before[s]`: the release wait of the last boundary before section `s`.
            let mut next_arrival: Vec<Option<usize>> = vec![None; n + 1];
            for k in (0..n).rev() {
                next_arrival[k] = if closes[k] {
                    arrive[k]
                } else {
                    next_arrival[k + 1]
                };
            }
            let mut previous_release: Option<usize> = None;
            let mut release_before: Vec<Option<usize>> = vec![None; n];
            for k in 0..n {
                release_before[k] = previous_release;
                if !closes[k] {
                    continue;
                }
                let (Some(a), Some(w)) = (arrive[k], release_wait[k]) else {
                    return Err(misplaced(
                        k,
                        "a program that does not arrive at, or wait for, a rendezvous",
                    ));
                };
                if a > w {
                    return Err(misplaced(
                        k,
                        "a release wait before the program's own arrival",
                    ));
                }
                if previous_release.is_some_and(|prev| prev > a) {
                    return Err(misplaced(
                        k,
                        "a rendezvous entered before the previous one was released",
                    ));
                }
                let is_root = root_of.get(&k) == Some(&(p.site, p.engine));
                match (is_root, gather_wait[k], posts[k]) {
                    (true, Some(g), Some((first, last))) => {
                        if !(a < g && g < first && last < w) {
                            return Err(misplaced(k, "a root that does not arrive, gather, release and then wait, in that order"));
                        }
                    }
                    (true, _, _) => {
                        return Err(misplaced(k, "a root that gathers without releasing"))
                    }
                    (false, None, None) => {}
                    (false, _, _) => {
                        return Err(misplaced(
                            k,
                            "a release posted by a program that does not hold the gather",
                        ))
                    }
                }
                previous_release = Some(w);
            }
            for (i, t) in p.ops.iter().enumerate() {
                if rendezvous_sem(&t.op).is_some() {
                    continue;
                }
                let Some(&s) = section_of.get(t.stage.idx()).filter(|s| **s < n) else {
                    continue;
                };
                if next_arrival[s].is_some_and(|a| i > a) {
                    return Err(misplaced(
                        s,
                        "an op of the section after the arrival that announces it finished",
                    ));
                }
                if release_before[s].is_some_and(|w| i < w) {
                    return Err(misplaced(
                        s,
                        "an op of the next section before the release that admits it",
                    ));
                }
            }
        }
        Ok(())
    }

    fn validate_requirements(&self) -> Result<(), IrViolation> {
        for e in &self.edges {
            let k = e.handshake.kind();
            if !self.requires.handshakes.contains(&k) {
                return Err(IrViolation::HandshakeNotRequired { kind: k });
            }
        }
        Ok(())
    }
}
