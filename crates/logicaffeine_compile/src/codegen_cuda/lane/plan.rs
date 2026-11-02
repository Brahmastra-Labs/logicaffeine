//! A step graph lowered to launches of the kernel library. The lane decides no fusion of its own: the step, lifted into
//! the one IR, is optimized by the shared table under the lane's shim ([`LaneShim`]), which answers whether a fused
//! class has a launch; each op the optimizer leaves takes the launches its link names ([`link`]), a class the lane
//! runs whole one launch, a class it runs as its parts each part's. Then the lane's schedule sets how the launches run
//! — four rows a warp for a wide product (`rows_a_warp`), the shared-row attention, the split counts, a batch's
//! sequences as one launch (`sequences`) — and every tensor is given its buffers.
//!
//! A row read at a literal index of a computed tensor (`row 4 of bxo`) is no launch at all: a view into the tensor —
//! unless the tensor is a table written in place before the row's last reader, where the row is copied when taken.
//! A view a projection reads quantized is a view into its tensor's quantized form too (when it starts on a whole
//! block), so the tensor is quantized once, in its producer, for every row a projection reads of it.

use std::collections::{BTreeMap, BTreeSet};

use crate::codegen_cuda::knobs::{CachePrecision, CudaKnobs, StatePrecision};
use crate::codegen_cuda::qwen_emit::Q8MatKind;
use crate::model::quant::GgmlType;
use crate::spatial::passes::recognize_stages::TensorOpSpec;
use crate::tensor_recognize::qwen::{Activation, MoeAct};
use crate::tensor_recognize::{ElementwiseOp, Epsilon};

use crate::codegen_cuda::lane_link::link;
use crate::codegen_cuda::lane_shim::{chunks_apart, LaneShim};
use crate::tensor_graph::fuse::{gates_rows, optimize, steps};
use crate::tensor_graph::graph::{Storage, Sym, TensorGraph};
use crate::tensor_graph::symbols::ClassId;

pub use crate::tensor_graph::graph::{SeqIo, StepIo};

use super::graph::{Role, StepGraph};
use super::kernels;
use super::weights::Placement;
use super::LaneError;

/// The most rows the GEMV kernel multiplies in one launch (its warp per output row holds an accumulator per row); a
/// projection of more rows runs on the tensor cores, or (under the precise numerics) as slabs of this many rows.
pub const GEMV_ROWS: usize = 16;

/// The largest prefill chunk a lane compiles.
pub const MAX_CHUNK: usize = 512;

/// The most positions one query of an attention over a cache of `len` rows attends: the cache, or its window.
pub fn attended_span(len: usize, window: Option<usize>) -> usize {
    window.map_or(len, |w| w.min(len))
}

/// The values of an activation's quantized form that share one scale.
pub const QBLOCK: usize = 32;

/// A view `offset` elements into a tensor reads the tensor's quantized form in place: it starts on a whole block.
pub fn quant_in_place(offset: usize) -> bool {
    offset % QBLOCK == 0
}

/// Which row of a table a write starts at: the row a parameter names (1-based), or a fixed row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowSel {
    Param(String),
    Fixed(usize),
}

/// A part of every unit of a wider tensor: unit `u`'s values start `u·pitch + offset` in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Part {
    pub pitch: usize,
    pub offset: usize,
}

/// Where a kernel writes its result: a buffer, or the rows of a table of rows `pitch` wide from a starting row — the
/// whole of it, or (`unit`) a part of every unit of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dst {
    pub buf: String,
    pub row: Option<(RowSel, usize)>,
    pub unit: Option<Part>,
}

impl Dst {
    pub fn to(buf: &str) -> Dst {
        Dst { buf: buf.to_string(), row: None, unit: None }
    }

    /// The tensor the result is the whole of, in its own buffer.
    pub fn own(&self) -> Option<&str> {
        (self.row.is_none() && self.unit.is_none()).then_some(self.buf.as_str())
    }
}

/// What the arg-max that closes a step writes besides its result: the token the next decode step reads (every form's
/// arg-max writes the decode step's; a batch's sequence `item` writes item `item` of its tokens) and the position the
/// sequence's next step reads; and the run generated so far, which only the host reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Advance {
    pub token: String,
    pub position: String,
    pub item: usize,
}

/// The norm a norm launch takes of its added operand in the same block before adding it (`sandwich_norm`): the weight
/// and epsilon of the norm that operand's own launch took.
#[derive(Clone, Debug, PartialEq)]
pub struct PreNorm {
    pub w: Option<String>,
    pub eps: Epsilon,
}

/// A part of a GEMM in parts: its matrix, bias, destination and outputs a row.
#[derive(Clone, Debug, PartialEq)]
pub struct GemmPart {
    pub w: String,
    pub bias: Option<String>,
    pub y: Dst,
    pub n: usize,
}

/// One launch of a library kernel. A producer names the tensor it computes (`out`); `f32` when it writes the tensor's
/// `f32` buffer, `quant` when it writes its quantized form. Row counts are the block's (1 for a decode step). A `part`
/// is where a launch reads its input `x` unit by unit: a part of every unit of a wider tensor (none, the whole of `x`).
#[derive(Clone, Debug, PartialEq)]
pub enum Launch {
    Gather { table: String, index: String, out: String, cols: usize, rows: usize, format: Option<GgmlType>, scale: Option<f64> },
    View { src: String, offset: usize, out: String },
    /// `into`: where its `f32` result lands when not in `out`'s own buffer.
    Norm { x: String, part: Option<Part>, add: Option<String>, pre: Option<PreNorm>, sum: Option<String>, w: Option<String>, out: String, into: Option<Dst>, f32: bool, quant: bool, rows: usize, cols: usize, eps: Epsilon },
    Quant { x: String, n: usize },
    /// A projection of `m` rows, `per_warp` output rows a warp; one of four rows a warp quantizes its output in its
    /// epilogue (`quant`, keeping the `f32` result when `f32`). On the tensor cores, `tail` is how its last wave is
    /// filled (`kernels::gemm_blk_tail`): the tiles a block each, and the parts every later tile is cut into.
    Gemv { x: String, w: String, format: GgmlType, bias: Option<String>, y: Dst, k: usize, n: usize, m: usize, per_warp: usize, f32: bool, quant: bool, tail: Option<(usize, usize)>, batch: bool },
    /// An `f32` projection of `m` rows; `split` is the parts it cuts `k` into: each tile's in tiles
    /// (`Launch::multiplies_in_tiles`, `kernels::gemm_f32_splits`), each output row's over a block's warps otherwise
    /// (`kernels::gemv_f32_parts`).
    GemvF32 { x: String, w: String, bias: Option<String>, y: Dst, k: usize, n: usize, m: usize, split: usize, batch: bool },
    /// Projections of one activation's `m` rows by matrices in one format, each into a buffer of its own or rows of a
    /// table, as one launch over their outputs side by side: a tensor-core GEMM above the GEMV's rows
    /// (`merged_projections`; `tail` as a projection's, over the parts' tiles together), the GEMV in parts at or below
    /// them (`gemv_parts`; `per_warp` output rows a warp, as each part's own GEMV took them).
    GemmParts { x: String, format: GgmlType, k: usize, m: usize, parts: Vec<GemmPart>, tail: Option<(usize, usize)>, per_warp: usize, batch: bool },
    Binary { op: ElementwiseOp, a: String, b: String, gate: Option<Activation>, row: Option<usize>, out: String, f32: bool, quant: bool, n: usize },
    Scale { x: String, y: String, n: usize, factor: f64 },
    SwiGlu { g: String, u: String, out: String, f32: bool, quant: bool, n: usize },
    Unary { act: Activation, x: String, y: String, n: usize },
    Rope { x: String, part: Option<Part>, norm: Option<(String, Epsilon)>, cos: String, sin: String, pos: Option<String>, y: Dst, heads: usize, per_row: usize, hd: usize, half: usize },
    /// Rotations of a step's one row by one table at one position — a layer's query heads and key heads, each normed
    /// first where the model norms them — as one launch over their heads (`rope_parts`), each head computed as its own
    /// rotation's launch computes it.
    /// `seqs`: a batch's rotations over its sequences (`sequences`), a side's row `r` sequence `r`, its position item
    /// `r` of `pos`, its destination row in its own table `RopeSide::yz` values on from the last row's.
    RopeParts { sides: Vec<RopeSide>, cos: String, sin: String, pos: Option<String>, hd: usize, half: usize, seqs: bool },
    /// `seq`: a batch's rows set over its sequences (`sequences`): row `r` of `src` into its own table, `seq` values on
    /// from the last row's, at its own index, item `r` of the index.
    /// `f32` and `quant`: the rows' values are set, and their quantized form too, into the table's — a batch's
    /// sequence's row, quantized where its producer stored it (`row_write_quant`, PARITY-BATCH-ROW-QUANT), its rows whole
    /// 32-value blocks, its values set only where something reads the table `f32`.
    SetRow { table: String, index: RowSel, src: String, cols: usize, count: usize, seq: Option<usize>, f32: bool, quant: bool },
    /// `vd` the value head width; the value rows `vstride` apart, the heads `vd` wide from `voff`; `scale` the scores'.
    /// A block takes `per_block` query heads of one group together; a row's span is cut into at most `splits` chunks,
    /// a block each when `parallel`, all in its row's block otherwise. With a `split`, each query and key is read in
    /// two parts. `shared`: the most chunks each row's span is cut into when every head reads its one key/value group's
    /// rows together, each cached row read once for all of them (`lk_attention_shared`). `seq`: a batch's attention over
    /// its sequences (`sequences`), row `r` sequence `r`, its caches `seq` values on from the last row's, its position
    /// item `r` of `pos`.
    /// `quant`: the attention quantizes its output in its own store for the projection that reads it (keeping it `f32`
    /// too when `f32`), as `lk_quant` would — the decode step's and a short chunk's kernels (`lk_attention`,
    /// `lk_attention_shared` and its fold), never those in tiles.
    /// `gate`: the attention multiplies each output by its activation of the gate's value at the same place as it stores
    /// it, before it quantizes it (`attn_output_gate`) — the gate's values read from `goff` on: a batch's sequence's row
    /// of the batch's gate, 0 otherwise.
    Attention { q: String, kc: String, vc: String, pos: Option<String>, sinks: Option<String>, out: String, heads: usize, group: usize, hd: usize, vd: usize, len: usize, rows: usize, window: Option<usize>, vstride: usize, voff: usize, scale: f64, per_block: usize, splits: usize, parallel: bool, split: Option<Split>, shared: Option<usize>, seq: Option<usize>, f32: bool, quant: bool, gate: Option<(String, Activation)>, goff: usize },
    /// `advance`: what the arg-max that closes the step moves on besides its result (`argmax_advance`).
    /// `rows`: a batch's arg-maxes over its `rows` sequences (`sequences`), each row's logits, result and advance its
    /// own; 1 otherwise.
    ArgMax { x: String, out: String, n: usize, advance: Option<Advance>, rows: usize },
    Conv { x: String, w: String, ring: String, y: String, chan: usize, taps: usize, rows: usize, silu: bool, l2: Option<(usize, Epsilon)> },
    /// Causal convolutions of a step's one row through the SiLU, each over its own input, weights and ring, as one launch
    /// (`conv_parts`): a block `block` channels of one part, the parts' blocks one after another; a normed part's heads
    /// `block` wide, each normed to unit length with `eps`.
    /// `seqs`: a batch's convolutions over its `seqs` sequences (`sequences`), each part's input row, ring and output
    /// the sequence's part of one array of them all; 1 otherwise.
    ConvParts { parts: Vec<ConvPart>, taps: usize, block: usize, eps: Option<Epsilon>, seqs: usize },
    L2Norm { x: String, y: String, rows: usize, cols: usize, eps: Epsilon },
    Decay { ar: String, dt: String, al: String, y: String, n: usize, heads: usize },
    Gates { br: String, ar: String, dt: String, al: String, beta: String, decay: String, n: usize, heads: usize },
    /// A step's two Gated DeltaNet gates from its one activation row (`gate_projections`): each head's write strength
    /// `sigmoid(x·wb)` and forget gate `(1 + e^(x·wa + dt))^al`, the head's two `f32` dots summed as its own projections
    /// sum them (`parts` warps a head, `kernels::gemv_f32_parts`) and its gates as `Gates` computes them, in one launch.
    GateProj { x: String, wb: String, wa: String, dt: String, al: String, beta: String, decay: String, k: usize, heads: usize, parts: usize },
    /// `seqs`: a batch's delta rules over its `seqs` sequences, a row and a memory each (`sequences`), each input,
    /// gate, memory and output the sequence's part of one array of them all; 1 otherwise.
    Delta { q: String, k: String, v: String, z: String, b: String, a: String, st: String, gn: String, out: String, f32: bool, quant: bool, hv: usize, hk: usize, d: usize, rows: usize, eps: Epsilon, seqs: usize },
    /// A mixture of `experts` experts of `dff` keeping `k`, and the `shared` experts every token takes, summed after —
    /// its gate and up rows `per_warp` a warp — over `rows` rows of `x` routed each on its own, every expert reading its
    /// weights once for all the rows that chose it. `router_split`: the parts an `f32` router's logits GEMM cuts each
    /// tile's `k` into (`kernels::gemm_f32_splits`) on the tensor cores.
    Moe { x: String, wr: String, wg: String, wu: String, wd: String, formats: [GgmlType; 3], y: Dst, d: usize, dff: usize, experts: usize, k: usize, router: Router, rb: Option<String>, biases: [Option<String>; 3], renormalize: bool, act: MoeAct, shared: Option<SharedExperts>, per_warp: usize, rows: usize, router_split: usize, batch: bool },
    /// `y[g·width + i] = x[g·every + from + i]` over `groups` groups.
    Slice { x: String, y: String, groups: usize, every: usize, from: usize, width: usize },
    /// Each of the `rows · heads` units' `ha` values of `a`, then its `hb` values of `b`. `yz`: a batch's join into its
    /// sequences' caches (`sequences`), row `r` of `rows` into its own table, `yz` values on from the last's, at its own
    /// position; 0 otherwise.
    Join { a: String, b: String, y: Dst, rows: usize, heads: usize, ha: usize, hb: usize, yz: usize },
    /// The per-head matmul over `m` rows and `heads` heads (`a` inputs, `b` outputs a head): `rows_out` when the matrix's
    /// rows are the heads' outputs, its rows its inputs otherwise. `shared`: every head reads its row's one unit of `x`
    /// (the `part` of each row). `upto`: the rows multiplied are those before the position the parameter names plus
    /// the count given, at most `m`.
    HeadGemv { x: String, part: Option<Part>, w: String, format: GgmlType, y: Dst, m: usize, heads: usize, a: usize, b: usize, rows_out: bool, shared: bool, upto: Option<(String, usize)>, batch: bool },
    /// Two projections of `x` through a gate: `out = act(x·wg + bg)·(x·wu + bu)`, SwiGLU when `act` is none — each
    /// matrix `n` rows of `k`, `m` activation rows, `per_warp` output rows a warp. On the tensor cores, one GEMM of the
    /// gate's and up's tiles side by side (`lk_gemm_blk`'s `G` 5), `tail` the gate's own GEMM's over twice its tiles.
    GemvGlu { x: String, wg: String, wu: String, formats: [GgmlType; 2], biases: [Option<String>; 2], act: Option<Activation>, out: String, f32: bool, quant: bool, k: usize, n: usize, m: usize, per_warp: usize, tail: Option<(usize, usize)>, batch: bool },
    /// One launch running `first`, then `then` over what `first` wrote, in the blocks that finish what `then` reads: a
    /// projection with the residual add and the next norm in its epilogue (`gemv_norm_epilogue`), run by its last block;
    /// projections of one activation in parts with their heads' norms and rotations (`qkv_head_epilogue`), each head
    /// turned by the block that writes its last rows; the embedding's gather with the first norm (`embed_norm`), each
    /// row normed by the block that gathers it; or a mixture — alone, or folded with its sum with another array — with
    /// the residual add and the next norm (`moe_deferred_finalize`), run by the decode step's last down block or by a
    /// chunk's combine, a block a row. A projection's blocks count in as they finish, and the finishing block reads their
    /// results back through L2; every block runs `then`'s work as `then`'s own launch would, so each computes what it
    /// computes apart.
    Fold { first: Box<Launch>, then: Box<Launch> },
}

/// One rotation of a merged launch (`Launch::RopeParts`): its input (read in place as a part of every unit of a wider
/// tensor where it names one), its norm, its destination, and its heads, `per_row` of them a row.
#[derive(Clone, Debug, PartialEq)]
pub struct RopeSide {
    pub x: String,
    pub part: Option<Part>,
    pub norm: Option<(String, Epsilon)>,
    pub y: Dst,
    pub heads: usize,
    pub per_row: usize,
    /// A batch's side (`RopeParts::seqs`): the values between one sequence's table and the next's; 0 otherwise.
    pub yz: usize,
}

/// One causal convolution of a merged launch (`Launch::ConvParts`): its input, weights, ring and output, its channels,
/// and whether each head of its output is normed to unit length.
#[derive(Clone, Debug, PartialEq)]
pub struct ConvPart {
    pub x: String,
    pub w: String,
    pub ring: String,
    pub y: String,
    pub chan: usize,
    pub l2: bool,
}

/// An attention's queries and keys in two parts, split at `own`: a query's first `own` values its unit's in `q`
/// (`q_pitch` apart a (row, head)), the rest its unit's in `q_tail`; a key's first `own` its group's in the keys, the
/// rest its row's in `k_tail`, every group's alike — an expanded latent attention's queries, each head's unrotated part
/// read in place in the query projection and its rotated part, and its keys, each head's expansion and the rotated key
/// the heads share, read in place in the cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Split {
    pub own: usize,
    pub q_pitch: usize,
    pub q_tail: (String, Part),
    pub k_tail: (String, Part),
}

/// How an expert mixture's router weights are held: quantized checkpoint rows of a format, or `f32` values (an
/// expert's row of the input's width at a time) — the router reads the input `f32` then, under every numerics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Router {
    Rows(GgmlType),
    F32,
}

/// The shared experts a mixture takes into its own launches (`shared_expert`): one SwiGLU FFN of `dff`, its gate, up and
/// down projections read in place in their `formats`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedExperts {
    pub wg: String,
    pub wu: String,
    pub wd: String,
    pub formats: [GgmlType; 3],
    pub dff: usize,
}

/// How a launch touches a tensor: through its `f32` (or `i32`) buffer, or its quantized activation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Touch {
    Plain,
    Quant,
}

fn strs<'a>(v: impl IntoIterator<Item = &'a String>) -> Vec<&'a str> {
    v.into_iter().map(String::as_str).collect()
}

impl Launch {
    /// The tensors the launch reads (a parameter naming a row included).
    pub fn reads(&self) -> Vec<&str> {
        fn row_param(d: &Dst) -> Vec<&str> {
            match &d.row {
                Some((RowSel::Param(p), _)) => vec![p.as_str()],
                _ => vec![],
            }
        }
        let mut v: Vec<&str> = match self {
            Launch::Gather { table, index, .. } => vec![table, index],
            Launch::View { src, .. } => vec![src],
            Launch::Norm { x, add, pre, w, into, .. } => {
                let mut r = strs([Some(x), add.as_ref(), w.as_ref(), pre.as_ref().and_then(|p| p.w.as_ref())].into_iter().flatten());
                r.extend(into.iter().flat_map(row_param));
                r
            }
            Launch::Quant { x, .. } => vec![x],
            Launch::Gemv { x, w, bias, y, .. } | Launch::GemvF32 { x, w, bias, y, .. } => {
                let mut r = vec![x.as_str(), w.as_str()];
                r.extend(bias.as_deref());
                r.extend(row_param(y));
                r
            }
            Launch::GemmParts { x, parts, .. } => {
                let mut r = vec![x.as_str()];
                r.extend(parts.iter().flat_map(|p| std::iter::once(p.w.as_str()).chain(p.bias.as_deref()).chain(row_param(&p.y))));
                r
            }
            Launch::Binary { a, b, .. } => vec![a, b],
            Launch::Scale { x, .. } => vec![x],
            Launch::SwiGlu { g, u, .. } => vec![g, u],
            Launch::Unary { x, .. } => vec![x],
            Launch::Rope { x, norm, cos, sin, pos, y, .. } => {
                let mut r = strs([Some(x), norm.as_ref().map(|(w, _)| w), Some(cos), Some(sin), pos.as_ref()].into_iter().flatten());
                r.extend(row_param(y));
                r
            }
            Launch::RopeParts { sides, cos, sin, pos, .. } => {
                let mut r = Vec::new();
                for side in sides {
                    r.extend(strs([Some(&side.x), side.norm.as_ref().map(|(w, _)| w)].into_iter().flatten()));
                    r.extend(row_param(&side.y));
                }
                r.extend(strs([Some(cos), Some(sin), pos.as_ref()].into_iter().flatten()));
                r
            }
            Launch::SetRow { table, index, src, .. } => match index {
                RowSel::Param(p) => vec![table, p, src],
                RowSel::Fixed(_) => vec![table, src],
            },
            Launch::Attention { q, kc, vc, pos, sinks, split, gate, .. } => {
                let mut r = strs([Some(q), Some(kc), Some(vc), pos.as_ref(), sinks.as_ref(), gate.as_ref().map(|(g, _)| g)].into_iter().flatten());
                r.extend(split.iter().flat_map(|s| [s.q_tail.0.as_str(), s.k_tail.0.as_str()]));
                r
            }
            Launch::ArgMax { x, .. } => vec![x],
            Launch::Conv { x, w, ring, .. } => vec![x, w, ring],
            Launch::ConvParts { parts, .. } => parts.iter().flat_map(|p| [p.x.as_str(), p.w.as_str(), p.ring.as_str()]).collect(),
            Launch::L2Norm { x, .. } => vec![x],
            Launch::Decay { ar, dt, al, .. } => vec![ar, dt, al],
            Launch::Gates { br, ar, dt, al, .. } => vec![br, ar, dt, al],
            Launch::GateProj { x, wb, wa, dt, al, .. } => vec![x, wb, wa, dt, al],
            Launch::Delta { q, k, v, z, b, a, st, gn, .. } => vec![q, k, v, z, b, a, st, gn],
            Launch::Moe { x, wr, wg, wu, wd, y, rb, biases, shared, .. } => {
                let mut r = vec![x.as_str(), wr.as_str(), wg.as_str(), wu.as_str(), wd.as_str()];
                r.extend(shared.iter().flat_map(|e| [e.wg.as_str(), e.wu.as_str(), e.wd.as_str()]));
                r.extend(rb.as_deref());
                r.extend(biases.iter().flatten().map(String::as_str));
                r.extend(row_param(y));
                r
            }
            Launch::Slice { x, .. } => vec![x],
            Launch::Join { a, b, y, .. } => {
                let mut r = vec![a.as_str(), b.as_str()];
                r.extend(row_param(y));
                r
            }
            Launch::HeadGemv { x, w, y, upto, .. } => {
                let mut r = vec![x.as_str(), w.as_str()];
                r.extend(row_param(y));
                r.extend(upto.as_ref().map(|(p, _)| p.as_str()));
                r
            }
            Launch::GemvGlu { x, wg, wu, biases, .. } => {
                let mut r = vec![x.as_str(), wg.as_str(), wu.as_str()];
                r.extend(biases.iter().flatten().map(String::as_str));
                r
            }
            // What `then` reads of what `first` wrote is handed over within the launch, read by none of its own.
            Launch::Fold { first, then } => {
                let made = first.writes();
                first.reads().into_iter().chain(then.reads().into_iter().filter(|t| !made.contains(t))).collect()
            }
        };
        v.dedup();
        v
    }

    /// The arrays the launch's kernels take in any storage type, reading each value widened exactly and writing it
    /// rounded to the nearest: an attention's keys, values and split keys' rest, a delta rule's memory, a slice's source,
    /// a tiled per-head matmul's activation, and every destination a producer writes into a table's rows (`lk_out`).
    /// A view touches nothing itself.
    pub fn typed(&self, numerics: Q8MatKind) -> Vec<&str> {
        match self {
            Launch::View { .. } => self.reads().into_iter().chain(self.writes()).collect(),
            Launch::Attention { kc, vc, split, .. } => {
                let mut t = vec![kc.as_str(), vc.as_str()];
                t.extend(split.iter().map(|s| s.k_tail.0.as_str()));
                t
            }
            Launch::Delta { st, .. } => vec![st],
            Launch::Slice { x, .. } => vec![x],
            Launch::HeadGemv { x, y, .. } if self.heads_in_tiles(numerics) => vec![x, y.buf.as_str()],
            Launch::HeadGemv { y, .. } | Launch::Gemv { y, .. } | Launch::GemvF32 { y, .. } | Launch::Rope { y, .. } | Launch::Join { y, .. } => {
                vec![y.buf.as_str()]
            }
            Launch::GemmParts { parts, .. } => parts.iter().map(|p| p.y.buf.as_str()).collect(),
            Launch::RopeParts { sides, .. } => sides.iter().map(|s| s.y.buf.as_str()).collect(),
            Launch::Norm { into: Some(d), .. } => vec![d.buf.as_str()],
            Launch::SetRow { table, .. } => vec![table],
            Launch::Fold { first, then } => first.typed(numerics).into_iter().chain(then.typed(numerics)).collect(),
            _ => Vec::new(),
        }
    }

    /// The tensors the launch writes (a cache it appends to, a memory it updates, included).
    pub fn writes(&self) -> Vec<&str> {
        match self {
            Launch::Gather { out, .. }
            | Launch::View { out, .. }
            | Launch::Binary { out, .. }
            | Launch::SwiGlu { out, .. }
            | Launch::Attention { out, .. }
            | Launch::GemvGlu { out, .. } => vec![out],
            Launch::ArgMax { out, advance, .. } => {
                let mut w = vec![out.as_str()];
                w.extend(advance.iter().flat_map(|a| [a.token.as_str(), a.position.as_str()]));
                w
            }
            Launch::Slice { y, .. } => vec![y],
            Launch::Norm { out, into, sum, .. } => {
                let mut w = vec![into.as_ref().map_or(out.as_str(), |d| d.buf.as_str())];
                w.extend(sum.as_deref());
                w
            }
            Launch::Delta { out, st, .. } => vec![out, st],
            Launch::Quant { x, .. } => vec![x],
            Launch::GemmParts { parts, .. } => parts.iter().map(|p| p.y.buf.as_str()).collect(),
            Launch::RopeParts { sides, .. } => sides.iter().map(|s| s.y.buf.as_str()).collect(),
            Launch::Gemv { y, .. }
            | Launch::GemvF32 { y, .. }
            | Launch::Rope { y, .. }
            | Launch::Moe { y, .. }
            | Launch::Join { y, .. }
            | Launch::HeadGemv { y, .. } => vec![y.buf.as_str()],
            Launch::Unary { y, .. } | Launch::Scale { y, .. } | Launch::L2Norm { y, .. } | Launch::Decay { y, .. } => vec![y],
            Launch::SetRow { table, .. } => vec![table],
            Launch::Conv { y, ring, .. } => vec![y, ring],
            Launch::ConvParts { parts, .. } => parts.iter().flat_map(|p| [p.y.as_str(), p.ring.as_str()]).collect(),
            Launch::Gates { beta, decay, .. } | Launch::GateProj { beta, decay, .. } => vec![beta, decay],
            Launch::Fold { first, then } => first.writes().into_iter().chain(then.writes()).collect(),
        }
    }

    /// The tensors the launch reads and writes, each in the form it touches: a projection reads its activation
    /// quantized unless the numerics keep it `f32`; a producer writes its result `f32`, quantized, or both. A view
    /// touches nothing: what its readers touch of it, they touch of its tensor.
    pub fn touches(&self, numerics: Q8MatKind) -> (Vec<(&str, Touch)>, Vec<(&str, Touch)>) {
        fn plain(v: Vec<&str>) -> Vec<(&str, Touch)> {
            v.into_iter().map(|t| (t, Touch::Plain)).collect()
        }
        fn produced(out: &str, f32: bool, quant: bool) -> Vec<(&str, Touch)> {
            [(f32, Touch::Plain), (quant, Touch::Quant)].into_iter().filter(|(on, _)| *on).map(|(_, f)| (out, f)).collect()
        }
        let reads = match (self, self.projected_input()) {
            (Launch::View { .. }, _) => return (Vec::new(), Vec::new()),
            (Launch::Fold { first, then }, _) => {
                let made = first.touches(numerics).1;
                first.touches(numerics).0.into_iter().chain(then.touches(numerics).0.into_iter().filter(|t| !made.contains(t))).collect()
            }
            (_, Some(x)) if numerics != Q8MatKind::Dequant => {
                let mut r = plain(self.reads());
                if !self.reads_f32_input() {
                    r.retain(|(t, _)| *t != x);
                }
                r.push((x, Touch::Quant));
                r
            }
            (Launch::SetRow { table, src, f32, quant, .. }, _) => {
                let mut r: Vec<(&str, Touch)> = plain(self.reads()).into_iter().filter(|(t, _)| *f32 || (*t != table.as_str() && *t != src.as_str())).collect();
                if *quant {
                    r.push((src.as_str(), Touch::Quant));
                }
                r
            }
            _ => plain(self.reads()),
        };
        let writes = match self {
            Launch::Norm { out, into, f32, quant, sum, .. } => {
                let mut w = match into {
                    Some(d) => [(*f32, d.buf.as_str(), Touch::Plain), (*quant, out.as_str(), Touch::Quant)]
                        .into_iter()
                        .filter(|(on, ..)| *on)
                        .map(|(_, t, f)| (t, f))
                        .collect(),
                    None => produced(out, *f32, *quant),
                };
                w.extend(sum.iter().map(|s| (s.as_str(), Touch::Plain)));
                w
            }
            Launch::Binary { out, f32, quant, .. }
            | Launch::SwiGlu { out, f32, quant, .. }
            | Launch::GemvGlu { out, f32, quant, .. }
            | Launch::Attention { out, f32, quant, .. } => produced(out, *f32, *quant),
            Launch::Delta { out, f32, quant, st, .. } => {
                let mut w = produced(out, *f32, *quant);
                w.push((st.as_str(), Touch::Plain));
                w
            }
            Launch::Quant { x, .. } => vec![(x.as_str(), Touch::Quant)],
            Launch::SetRow { table, f32, quant, .. } => produced(table, *f32, *quant),
            Launch::Fold { first, then } => first.touches(numerics).1.into_iter().chain(then.touches(numerics).1).collect(),
            _ => plain(self.writes()),
        };
        (reads, writes)
    }

    /// Whether the launch multiplies on the int8 tensor cores under `numerics`: a quantized projection of more rows than
    /// the GEMV takes, its activation quantized, by `lk_gemm_blk` in any format, Q8_0's split arrays included — several
    /// side by side in parts, a gated pair as one paired GEMM — and an expert mixture over as many rows, each expert's
    /// matrices by the grouped `lk_gemm_blk`.
    pub fn on_tensor_cores(&self, numerics: Q8MatKind) -> bool {
        matches!(self, Launch::Gemv { m, batch: false, .. } | Launch::GemmParts { m, batch: false, .. } | Launch::GemvGlu { m, batch: false, .. }
            | Launch::Moe { rows: m, batch: false, .. } if *m > GEMV_ROWS)
            && numerics != Q8MatKind::Dequant
    }

    /// Whether a batch's projection runs on the tensor cores in its rows' own order (`lk_gemv_mma`): a projection, its
    /// parts or its gated pair over Q8_0 matrices of a batch's `GEMV_MMA_MIN_ROWS` or more rows, under the quantized
    /// numerics, every part a whole number of 8-output tiles and `k` a whole number of 32-value blocks.
    pub fn batch_on_tensor_cores(&self, numerics: Q8MatKind) -> bool {
        let rows = |m: usize| m >= kernels::GEMV_MMA_MIN_ROWS;
        numerics != Q8MatKind::Dequant
            && match self {
                Launch::Gemv { format: GgmlType::Q8_0, k, n, m, f32: true, quant: false, batch: true, .. } => rows(*m) && n % 8 == 0 && k % 32 == 0,
                Launch::GemmParts { format: GgmlType::Q8_0, k, m, parts, batch: true, .. } => rows(*m) && k % 32 == 0 && parts.iter().all(|p| p.n % 8 == 0),
                Launch::GemvGlu { formats: [GgmlType::Q8_0, GgmlType::Q8_0], k, n, m, batch: true, .. } => rows(*m) && n % 32 == 0 && k % 32 == 0,
                _ => false,
            }
    }

    /// Whether the launch is a batch's projection (`batch`): its rows are the batch's sequences, each summed as the
    /// decode step sums its one row — row by row whatever their count, never in tiles.
    pub fn is_batch(&self) -> bool {
        matches!(self, Launch::Gemv { batch: true, .. } | Launch::GemvF32 { batch: true, .. } | Launch::GemmParts { batch: true, .. }
            | Launch::GemvGlu { batch: true, .. } | Launch::Moe { batch: true, .. } | Launch::HeadGemv { batch: true, .. })
    }

    /// A projection's outputs a row: a GEMM in parts' over its parts together; none for any other launch.
    pub fn gemm_cols(&self) -> usize {
        match self {
            Launch::Gemv { n, .. } => *n,
            Launch::GemmParts { parts, .. } => parts.iter().map(|p| p.n).sum(),
            _ => 0,
        }
    }

    /// The launches this launch runs the work of: a fold's first's and then's, any other launch itself — what a question
    /// about one kernel's reads or kind asks of each.
    pub fn parts(&self) -> Vec<&Launch> {
        match self {
            Launch::Fold { first, then } => first.parts().into_iter().chain(then.parts()).collect(),
            l => vec![l],
        }
    }

    /// The mixture a fold finishes with the next norm, for the schedule to set how it runs: the launch itself where it is
    /// a mixture.
    pub fn mixture_mut(&mut self) -> Option<&mut Launch> {
        if matches!(self, Launch::Moe { .. }) {
            return Some(self);
        }
        let Launch::Fold { first, then } = self else { return None };
        if !matches!(then.as_ref(), Launch::Norm { .. }) {
            return None;
        }
        match first.as_mut() {
            m @ Launch::Moe { .. } => Some(m),
            Launch::Fold { first: m, then: sum } if matches!((m.as_ref(), sum.as_ref()), (Launch::Moe { .. }, Launch::Binary { .. })) => Some(m.as_mut()),
            _ => None,
        }
    }

    /// The mixture a fold finishes with the next norm, its norm, and — where the norm adds the mixture's sum with another
    /// array — that array and the sum.
    pub fn finished_mixture(&self) -> Option<(&Launch, &Launch, Option<(&str, &str)>)> {
        let Launch::Fold { first, then } = self else { return None };
        if !matches!(then.as_ref(), Launch::Norm { .. }) {
            return None;
        }
        match first.as_ref() {
            m @ Launch::Moe { .. } => Some((m, then, None)),
            Launch::Fold { first: m, then: sum } => match (m.as_ref(), sum.as_ref()) {
                (Launch::Moe { y, .. }, Launch::Binary { a, b, out, .. }) => {
                    Some((m.as_ref(), then, Some((if *a == y.buf { b.as_str() } else { a.as_str() }, out.as_str()))))
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// The activation rows a block of a dense tensor-core GEMM takes under `numerics` (`kernels::gemm_blk_rows`).
    pub fn gemm_rows(&self, numerics: Q8MatKind) -> usize {
        let format = match self {
            Launch::Gemv { format, .. } | Launch::GemmParts { format, .. } | Launch::GemvGlu { formats: [format, _], .. } => *format,
            _ => return kernels::GEMM_BLK_TILE_M,
        };
        kernels::gemm_blk_rows(kernels::format_arg(format), numerics == Q8MatKind::TwoPassInt8)
    }

    /// The tiles a tensor-core GEMM's outputs a row take: of `GEMM_TILE_N` outputs, a gated pair's of half as many
    /// hidden values (its gate's and up's side by side).
    pub fn gemm_tiles_wide(&self) -> usize {
        match self {
            Launch::GemvGlu { n, .. } => n.div_ceil(kernels::GEMM_TILE_N / 2),
            _ => self.gemm_cols().div_ceil(kernels::GEMM_TILE_N),
        }
    }

    /// Whether an `f32` projection multiplies its rows as one `f32` GEMM of 64 × 64 tiles under `numerics`
    /// (`lk_gemm_f32`): a chunk of more rows than the GEMV takes under the quantized numerics — the precise numerics sum
    /// each row's dots in `f64` — its rows whole `float4`s, its result its own buffer.
    pub fn multiplies_in_tiles(&self, numerics: Q8MatKind) -> bool {
        numerics != Q8MatKind::Dequant
            && matches!(self, Launch::GemvF32 { k, y, m, batch: false, .. } if *m > GEMV_ROWS && k % 4 == 0 && y.own().is_some())
    }

    /// Whether an expert mixture on the tensor cores multiplies each expert's gate and up together (`lk_gemm_blk`'s
    /// paired mode, the gated activation and its quantizer in the GEMM's epilogue): its gate and up in one format.
    pub fn pairs_gate_and_up(&self, numerics: Q8MatKind) -> bool {
        matches!(self, Launch::Moe { formats, .. } if formats[0] == formats[1]) && self.on_tensor_cores(numerics)
    }

    /// Whether the launch walks its delta rule a chunk of rows at a time under `numerics` (`lk_delta_prep`, then
    /// `lk_delta_state`): a delta rule over a chunk of more rows than the GEMV takes, under the quantized numerics, its
    /// heads a width the chunked rule takes (`kernels::delta_chunks_take`).
    pub fn delta_in_chunks(&self, numerics: Q8MatKind) -> bool {
        numerics != Q8MatKind::Dequant && matches!(self, Launch::Delta { rows, d, .. } if *rows > GEMV_ROWS && kernels::delta_chunks_take(*d))
    }

    /// Whether the launch attends on the f16 tensor cores under `numerics` (`lk_attention_mma`): it attends in tiles
    /// under the one-word numerics — llama.cpp's precision, whose attention rounds its operands to f16 — its heads and
    /// values whole multiples of 16 and at most 256 wide.
    pub fn attends_on_tensor_cores(&self, numerics: Q8MatKind) -> bool {
        numerics == Q8MatKind::Int8Act
            && self.attends_in_tiles(numerics)
            && matches!(self, Launch::Attention { hd, vd, .. } if hd % 16 == 0 && vd % 16 == 0 && *hd <= 256 && *vd <= 256)
    }

    /// Whether the launch attends on the int8 tensor cores in two words under `numerics` (`lk_attention_i8`, its keys and
    /// values taken into two words once by `lk_kv_two_words`): it attends in tiles under the two-word numerics, its heads
    /// and values whole multiples of 32 and at most 256 wide.
    pub fn attends_in_two_words(&self, numerics: Q8MatKind) -> bool {
        numerics == Q8MatKind::TwoPassInt8
            && self.attends_in_tiles(numerics)
            && matches!(self, Launch::Attention { hd, vd, .. } if hd % 32 == 0 && vd % 32 == 0 && *hd <= 256 && *vd <= 256)
    }

    /// Whether the launch attends in tiles in flash style under `numerics` (`lk_attention_flash`): it attends in tiles
    /// and on neither the f16 nor the int8 tensor cores, its heads and values at most 256 wide; wider ones take
    /// `lk_attention_tiled`.
    pub fn attends_flash(&self, numerics: Q8MatKind) -> bool {
        self.attends_in_tiles(numerics)
            && !self.attends_on_tensor_cores(numerics)
            && !self.attends_in_two_words(numerics)
            && matches!(self, Launch::Attention { hd, vd, .. } if *hd <= 256 && *vd <= 256)
    }

    /// Whether the launch attends in tiles under `numerics` (`lk_attention_flash`, or `lk_attention_tiled` for wider
    /// heads): an attention over a chunk of more rows than the GEMV takes, under the quantized numerics, its keys,
    /// values and outputs read and written four at a time and its values at most 512 wide.
    pub fn attends_in_tiles(&self, numerics: Q8MatKind) -> bool {
        let fits = |rows: usize, hd: usize, vd: usize, vstride: usize, voff: usize, kvw: usize| {
            rows > GEMV_ROWS && hd % 4 == 0 && vd % 4 == 0 && vd <= 512 && vstride % 4 == 0 && voff % 4 == 0 && kvw % 4 == 0
        };
        // A split query and key: each part four-value words.
        let split_fits = |s: &Option<Split>| {
            s.as_ref().map_or(true, |s| {
                [s.own, s.q_pitch, s.q_tail.1.pitch, s.q_tail.1.offset, s.k_tail.1.pitch, s.k_tail.1.offset].iter().all(|v| v % 4 == 0)
            })
        };
        matches!(self, Launch::Attention { rows, hd, vd, vstride, voff, heads, group, split, seq: None, .. }
            if fits(*rows, *hd, *vd, *vstride, *voff, heads / group * split.as_ref().map_or(*hd, |s| s.own)) && split_fits(split))
            && numerics != Q8MatKind::Dequant
    }

    /// Whether the launch is a per-head matmul run in tiles under `numerics` (`lk_head_tiled`): over a chunk of more rows
    /// than the GEMV takes, under the quantized numerics, its inputs read four words at a time.
    pub fn heads_in_tiles(&self, numerics: Q8MatKind) -> bool {
        let fits = |m: usize, a: usize, pitch: usize, offset: usize| m > GEMV_ROWS && a % 4 == 0 && pitch % 4 == 0 && offset % 4 == 0;
        matches!(self, Launch::HeadGemv { m, a, part, batch: false, .. } if fits(*m, *a, part.map_or(*a, |p| p.pitch), part.map_or(0, |p| p.offset)))
            && numerics != Q8MatKind::Dequant
    }

    /// Whether the launch writes all `len` elements of tensor `t`'s array `touch` — a result in its own buffer, every
    /// element of it — and not rows of a table, a part of every unit of a wider tensor, or carried state it updates.
    pub fn writes_whole(&self, t: &str, touch: Touch, len: usize) -> bool {
        let own = |d: &Dst| d.own() == Some(t);
        let count = match self {
            Launch::View { .. } | Launch::SetRow { .. } => return false,
            Launch::Gather { rows, cols, .. } | Launch::L2Norm { rows, cols, .. } => rows * cols,
            Launch::Norm { out, into, sum, rows, cols, .. } => {
                let whole = match touch {
                    Touch::Quant => out == t,
                    Touch::Plain => sum.as_deref() == Some(t) || into.as_ref().map_or(out == t, own),
                };
                if !whole {
                    return false;
                }
                rows * cols
            }
            Launch::Quant { n, .. }
            | Launch::Binary { n, .. }
            | Launch::Scale { n, .. }
            | Launch::SwiGlu { n, .. }
            | Launch::Unary { n, .. }
            | Launch::Decay { n, .. }
            | Launch::Gates { n, .. } => *n,
            Launch::GateProj { heads, .. } => *heads,
            Launch::ArgMax { .. } => 1,
            Launch::Gemv { y, n, m, .. } | Launch::GemvF32 { y, n, m, .. } if own(y) => n * m,
            Launch::GemmParts { parts, m, .. } => match parts.iter().find(|p| own(&p.y)) {
                Some(p) => p.n * m,
                None => return false,
            },
            Launch::GemvGlu { n, m, .. } => n * m,
            Launch::Rope { y, heads, hd, .. } if own(y) => heads * hd,
            Launch::RopeParts { sides, hd, .. } => match sides.iter().find(|s| own(&s.y)) {
                Some(s) => s.heads * hd,
                None => return false,
            },
            Launch::Attention { rows, heads, vd, .. } => rows * heads * vd,
            Launch::Conv { ring, rows, chan, .. } if ring != t => rows * chan,
            Launch::ConvParts { parts, seqs, .. } => match parts.iter().find(|p| p.y == t) {
                Some(p) => p.chan * seqs,
                None => return false,
            },
            Launch::Delta { st, rows, hv, d, seqs, .. } if st != t => rows * hv * d * seqs,
            Launch::Moe { y, rows, d, .. } if own(y) => rows * d,
            Launch::Slice { groups, width, .. } => groups * width,
            Launch::Join { y, rows, heads, ha, hb, .. } if own(y) => rows * heads * (ha + hb),
            Launch::HeadGemv { y, m, heads, b, .. } if own(y) => m * heads * b,
            Launch::Fold { first, then } => return first.writes_whole(t, touch, len) || then.writes_whole(t, touch, len),
            _ => return false,
        };
        count == len
    }

    /// The part of every unit of tensor `t` (`len` elements) the launch writes, `(pitch, from, width)`: a result written
    /// in place as a part of every unit of a wider tensor (`join_in_place`), all of its units.
    pub fn writes_part(&self, t: &str, len: usize) -> Option<(usize, usize, usize)> {
        let d = match self {
            Launch::Norm { into: Some(d), .. } => d,
            Launch::Rope { y, .. } | Launch::HeadGemv { y, .. } => y,
            _ => return None,
        };
        let (units, width) = self.unit_output()?;
        match (&d.row, &d.unit) {
            (None, Some(p)) if d.buf == t && units * p.pitch == len && p.offset + width <= p.pitch => Some((p.pitch, p.offset, width)),
            _ => None,
        }
    }

    /// The activation a product by checkpoint rows reads quantized, under every numerics but the precise one. A
    /// per-head matmul — a latent's expansion half — reads its activation `f32` under every numerics
    /// ([`kernels::head_mode`]).
    pub fn projected_input(&self) -> Option<&str> {
        match self {
            Launch::Gemv { x, .. } | Launch::GemmParts { x, .. } | Launch::Moe { x, .. } | Launch::GemvGlu { x, .. } => Some(x),
            Launch::Fold { first, .. } => first.projected_input(),
            _ => None,
        }
    }

    /// Whether a projection that reads its input quantized reads it `f32` as well: an expert mixture's `f32` router
    /// does.
    fn reads_f32_input(&self) -> bool {
        match self {
            Launch::Fold { first, .. } => first.reads_f32_input(),
            l => matches!(l, Launch::Moe { router: Router::F32, .. }),
        }
    }

    /// The tensor a producer computes into a buffer of its own — `None` for a launch whose result lands in rows of a
    /// table, in a part of another tensor, or in carried state alone.
    fn result(&self) -> Option<&str> {
        match self {
            Launch::Gemv { y, .. }
            | Launch::GemvF32 { y, .. }
            | Launch::Rope { y, .. }
            | Launch::Moe { y, .. }
            | Launch::Join { y, .. }
            | Launch::HeadGemv { y, .. } => y.own(),
            Launch::Norm { out, into, .. } => into.is_none().then_some(out.as_str()),
            Launch::SetRow { .. } | Launch::Quant { .. } | Launch::Gates { .. } | Launch::GateProj { .. } => None,
            Launch::Fold { then, .. } => then.result(),
            _ => self.writes().first().copied(),
        }
    }

    /// The units a launch writes its result in, `(units, width)`, when it can write them as a part of every unit of a
    /// wider tensor (`join_in_place`).
    fn unit_output(&self) -> Option<(usize, usize)> {
        match self {
            Launch::Norm { rows, cols, f32: true, quant: false, .. } => Some((*rows, *cols)),
            Launch::Rope { heads, hd, .. } => Some((*heads, *hd)),
            Launch::HeadGemv { m, heads, b, .. } => Some((m * heads, *b)),
            _ => None,
        }
    }
}

/// A fusion rule that fired: its name and the graph ops it joined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fusion {
    pub rule: &'static str,
    pub ops: Vec<usize>,
}

/// The element type of a buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DType {
    F32,
    I32,
    /// A recurrent memory stored in bfloat16 between steps (`CudaKnobs::recurrent_state`).
    Bf16,
    /// An attention's cache stored in binary16 (`CudaKnobs::cache_precision`).
    F16,
}

impl DType {
    /// The CUDA element type a buffer of it is declared with.
    pub fn c_type(self) -> &'static str {
        match self {
            DType::F32 => "float",
            DType::I32 => "int",
            DType::Bf16 => "__nv_bfloat16",
            DType::F16 => "__half",
        }
    }

    /// The bytes of one element.
    pub fn bytes(self) -> usize {
        match self {
            DType::F32 | DType::I32 => 4,
            DType::Bf16 | DType::F16 => 2,
        }
    }
}

/// What a buffer holds and how it is filled before the first step.
#[derive(Clone, Debug, PartialEq)]
pub enum Fill {
    /// A run-time input the generation loop sets (the tokens, the position).
    Param,
    /// Carried state, or a table a step fills row by row: zero before the first step.
    Zero,
    /// A constant, from the checkpoint or the compiler.
    Constant(Placement),
    /// Computed by the step.
    Computed,
    /// A row of another tensor, read in place: `offset` elements into `of` — its quantized form too, `offset` into
    /// `of`'s, when the offset is whole blocks ([`quant_in_place`]).
    Alias { of: String, offset: usize },
}

/// One tensor's buffers.
#[derive(Clone, Debug, PartialEq)]
pub struct Buffer {
    pub name: String,
    pub len: usize,
    pub dtype: DType,
    pub fill: Fill,
    /// The tensor's `f32` (or `i32`) buffer is allocated.
    pub plain: bool,
    /// Its quantized form is allocated: int8 values (two words in the two-pass numerics) and a scale per 32.
    pub quantized: bool,
}

/// A step lowered: its launches in order (with the graph ops each computes — none for a quantize), its buffers, and
/// the fusions that made the launches.
#[derive(Clone, Debug, PartialEq)]
pub struct Schedule {
    pub launches: Vec<Launch>,
    pub ops: Vec<Vec<usize>>,
    pub buffers: BTreeMap<String, Buffer>,
    pub fusions: Vec<Fusion>,
    pub numerics: Q8MatKind,
    pub io: StepIo,
}

impl Schedule {
    /// How many launches of each kind the step makes (a view is none).
    pub fn census(&self) -> BTreeMap<&'static str, usize> {
        let mut c = BTreeMap::new();
        for l in self.launches.iter().filter(|l| !matches!(l, Launch::View { .. })) {
            *c.entry(kind(l)).or_insert(0) += 1;
        }
        c
    }

    /// How many times `rule` fired.
    pub fn fired(&self, rule: &str) -> usize {
        self.fusions.iter().filter(|f| f.rule == rule).count()
    }
}

/// A launch's kind, as a census counts it.
pub fn kind(l: &Launch) -> &'static str {
    match l {
        Launch::Gather { .. } => "gather",
        Launch::View { .. } => "view",
        Launch::Norm { .. } => "rmsnorm",
        Launch::Quant { .. } => "quant",
        Launch::Gemv { .. } => "gemv_q8",
        Launch::GemvF32 { .. } => "gemv_f32",
        Launch::GemmParts { m, batch, .. } if *m <= GEMV_ROWS || *batch => "gemv_parts",
        Launch::GemmParts { .. } => "gemm_parts",
        Launch::Binary { .. } => "binary",
        Launch::Scale { .. } => "scale",
        Launch::SwiGlu { .. } => "swiglu",
        Launch::Unary { .. } => "unary",
        Launch::Rope { .. } | Launch::RopeParts { .. } => "rope",
        Launch::SetRow { .. } => "set_row",
        Launch::Attention { .. } => "attention",
        Launch::ArgMax { .. } => "argmax",
        Launch::Conv { .. } | Launch::ConvParts { .. } => "conv",
        Launch::L2Norm { .. } => "l2norm",
        Launch::Decay { .. } => "decay",
        Launch::Gates { .. } | Launch::GateProj { .. } => "gates",
        Launch::Delta { .. } => "delta",
        Launch::Moe { .. } => "moe",
        Launch::Slice { .. } => "slice",
        Launch::Join { .. } => "join",
        Launch::HeadGemv { .. } => "head_gemv",
        Launch::GemvGlu { .. } => "gemv_glu",
        Launch::Fold { first, .. } if matches!(first.as_ref(), Launch::Gather { .. }) => "gather_norm",
        Launch::Fold { then, .. } if matches!(then.as_ref(), Launch::RopeParts { .. }) => "gemv_rope",
        l @ Launch::Fold { .. } if l.finished_mixture().is_some() => "moe_norm",
        Launch::Fold { .. } => "gemv_norm",
    }
}

use slots::Slots;

mod slots {
    use super::Launch;

    /// The launches being scheduled, each with the graph ops it computes: the schedule sets how each runs, never which
    /// launches there are.
    pub(super) struct Slots {
        slots: Vec<(Launch, Vec<usize>)>,
    }

    impl Slots {
        pub(super) fn new(slots: Vec<(Launch, Vec<usize>)>) -> Self {
            Slots { slots }
        }

        pub(super) fn len(&self) -> usize {
            self.slots.len()
        }

        pub(super) fn slot(&self, i: usize) -> Option<&(Launch, Vec<usize>)> {
            self.slots.get(i)
        }

        pub(super) fn slot_mut(&mut self, i: usize) -> Option<&mut (Launch, Vec<usize>)> {
            self.slots.get_mut(i)
        }

        pub(super) fn get(&self, i: usize) -> &Launch {
            &self.slots[i].0
        }

        pub(super) fn get_mut(&mut self, i: usize) -> &mut Launch {
            &mut self.slots[i].0
        }

        pub(super) fn live(&self) -> impl Iterator<Item = (usize, &Launch)> {
            self.slots.iter().enumerate().map(|(i, (l, _))| (i, l))
        }

        /// The slot that computes `t` into its own buffer.
        pub(super) fn producer(&self, t: &str) -> Option<usize> {
            self.slots.iter().position(|(l, _)| l.result() == Some(t))
        }

        pub(super) fn into_inner(self) -> Vec<(Launch, Vec<usize>)> {
            self.slots
        }
    }
}

/// The step graph `g` lowered under `numerics` and `knobs`: `constants` holds the placement of every constant the
/// graph declares, `io` the step's carried names, `held` the steps of [`RULES`] held back — a shared rule, or a step of
/// the lane's schedule.
pub fn plan(
    g: &StepGraph,
    _names: &dyn Fn(crate::intern::Symbol) -> String,
    constants: BTreeMap<String, Placement>,
    numerics: Q8MatKind,
    knobs: &CudaKnobs,
    io: StepIo,
    held: &[&str],
) -> Result<Schedule, LaneError> {
    assert!(held.iter().all(|r| RULES.contains(r)), "every rule held back is one of {RULES:?}: {held:?}");
    let knobs = CudaKnobs { activations: numerics, ..*knobs };
    let shim = LaneShim { knobs, batch: !io.more.is_empty(), held: RULES.iter().copied().filter(|r| held.contains(r)).collect() };
    let steps = steps(&shim);
    let shared: Vec<&str> = held.iter().copied().filter(|r| steps.contains(r)).collect();
    let lifted = g.lifted.placed(
        &|t| match constants.get(t) {
            Some(Placement::Rows(r)) => Storage::Rows { format: r.ty, transposed: r.transposed },
            _ => Storage::Values,
        },
        Some(io.clone()),
    );
    let o = optimize(&lifted, &shim, &shared);
    let linked = link(&o.graph, &constants, &shim, &io).map_err(|(k, why)| {
        let op = o.graph.ops[k].covers.iter().copied().min().unwrap_or(0);
        LaneError::NoKernel { op, kind: g.ops[op].spec.kind(), out: g.ops[op].out.clone(), why }
    })?;
    if numerics != Q8MatKind::Dequant {
        whole_blocks(g, &linked)?;
    }
    // The firings in the order the steps take them — the merge of products on the tensor cores but a schedule of the
    // link's, which records none — each with the lifted ops of the launch computing its result, as that launch lists
    // them: a latent attention taken whole runs as its halves and the attention, and records the attention's; a batch's
    // gated attentions run as each sequence's attention and its row's set, and record them all.
    let recorded = |covers: &[usize]| -> Vec<usize> {
        let op = o.graph.ops.iter().find(|op| covered(op) == covers);
        if op.is_some_and(gates_batch_rows) {
            return covers.to_vec();
        }
        let result = op.map(|op| op.out.as_str());
        linked
            .iter()
            .find(|(l, ops)| {
                let mut set = ops.clone();
                set.sort_unstable();
                set == covers || (ops.iter().all(|k| covers.contains(k)) && result.is_some_and(|out| l.writes().contains(&out)))
            })
            .map_or_else(|| covers.to_vec(), |(_, ops)| ops.clone())
    };
    let mut fusions: Vec<Fusion> =
        o.fired.iter().filter(|f| f.rule != "merged_projections").map(|f| Fusion { rule: f.rule, ops: recorded(&f.covers) }).collect();
    let mut slots = Slots::new(linked);
    if !held.contains(&"rows_a_warp") {
        rows_a_warp(&mut slots, &mut fusions, &o.graph);
    }
    shared_rows(&mut slots, numerics, &knobs);
    for i in 0..slots.len() {
        if let Some((l, _)) = slots.slot_mut(i) {
            let l = if l.finished_mixture().is_some() { l.mixture_mut().expect("a finished mixture") } else { l };
            split_counts(l, numerics, &knobs);
        }
    }
    let (mut launches, mut ops): (Vec<Launch>, Vec<Vec<usize>>) = slots.into_inner().into_iter().unzip();
    // The tensors the link's own launches compute, which the step graph does not hold: a latent's expanded keys and
    // values, every cached row through each half.
    let planned: BTreeMap<String, usize> = launches
        .iter()
        .filter_map(|l| match l {
            Launch::HeadGemv { y, m, heads, b, shared: true, .. } => Some((y.buf.clone(), m * heads * b)),
            _ => None,
        })
        .collect();
    // The matrices read as values — a product's, a mixture's router — which the lane stores transposed.
    let transposed_values: BTreeSet<String> = g
        .ops
        .iter()
        .filter(|op| matches!(op.spec, TensorOpSpec::Matmul(_) | TensorOpSpec::Moe(_)))
        .map(|op| &op.ins[1])
        .filter(|w| matches!(constants.get(*w), Some(Placement::Values(_))))
        .cloned()
        .collect();
    let mut buffers = buffers(g, &planned, &launches, constants, &transposed_values, numerics, &io)?;
    // A gated-delta memory in the precision the `recurrent_state` knob stores it in, before a batch's memories are laid
    // out as one array of them.
    if knobs.recurrent_state == StatePrecision::Bf16 {
        for l in &launches {
            if let Launch::Delta { st, .. } = l {
                if let Some(b) = buffers.get_mut(st) {
                    b.dtype = DType::Bf16;
                }
            }
        }
    }
    // An attention's cache in the precision the `cache_precision` knob stores it in, and every view into it alike.
    if knobs.cache_precision == CachePrecision::F16 {
        for c in g.caches() {
            if let Some(b) = buffers.get_mut(&c) {
                b.dtype = DType::F16;
            }
        }
    }
    let views: Vec<(String, DType)> = buffers
        .values()
        .filter(|b| matches!(b.fill, Fill::Alias { .. }))
        .map(|b| (b.name.clone(), buffers[&span(&buffers, &b.name).0].dtype))
        .collect();
    for (name, dtype) in views {
        buffers.get_mut(&name).expect("a view").dtype = dtype;
    }
    if !io.more.is_empty() && !held.contains(&"sequences") {
        sequences(&mut launches, &mut ops, &mut buffers, &mut fusions, &io, knobs.chip.sms);
    }
    // An array stored in 16 bits is read and written only by kernels that take it so.
    for (i, l) in launches.iter().enumerate() {
        let typed = l.typed(numerics);
        if l.reads().into_iter().chain(l.writes()).any(|t| buffers.get(t).is_some_and(|b| b.dtype.bytes() == 2) && !typed.contains(&t)) {
            let op = ops[i].first().copied().unwrap_or(0);
            return Err(LaneError::NoKernel {
                op,
                kind: g.ops[op].spec.kind(),
                out: g.ops[op].out.clone(),
                why: "its kernel takes an array the step stores in 16 bits only as f32".to_string(),
            });
        }
    }
    Ok(Schedule { launches, ops, buffers, fusions, numerics, io })
}

/// The array a tensor's values live in and where, `(array, from, to)`: its own, or through its views those it is part of.
fn span(buffers: &BTreeMap<String, Buffer>, name: &str) -> (String, usize, usize) {
    let len = buffers.get(name).map_or(0, |b| b.len);
    match buffers.get(name).map(|b| &b.fill) {
        Some(Fill::Alias { of, offset }) => {
            let (root, from, _) = span(buffers, of);
            (root, from + offset, from + offset + len)
        }
        _ => (name.to_string(), 0, len),
    }
}

/// Where a batch's sequence `b`'s array lives in the batch's one array of them: `(array, sequence, values a sequence)`,
/// for a tensor that is a part of another at `b` times one width — a sequence's row of a stacked tensor, or its part of
/// one array of every sequence's (`sequences`).
fn sequence_part(buffers: &BTreeMap<String, Buffer>, name: &str, width: usize) -> Option<(String, usize)> {
    match &buffers.get(name)?.fill {
        Fill::Alias { of, offset } if width > 0 && offset % width == 0 => Some((of.clone(), offset / width)),
        _ => None,
    }
}

/// A batch's step laid out and launched over its sequences. First every array a sequence of its own computes or carries
/// (`X_s<b>`, one for each of the `B` sequences, alike in length) becomes sequence `b`'s part of one array of them all:
/// its row of the table each sequence's array is set into whole, when there is one — the attended rows, set into the
/// output projection's input — or of a new array `X_s`. That changes no value the step computes. Then each set of `B`
/// launches alike but for their sequence — a layer's rotations, its value rows' writes into the caches, its attentions —
/// runs as one launch whose rows are the sequences, at the last one's place, each row reading its own position (item
/// `b` of the positions, `pos_s0` on) and its own caches; a set moves only where nothing between its first and its last
/// reads what it writes or writes what it reads, but the other sets.
fn sequences(launches: &mut Vec<Launch>, ops: &mut Vec<Vec<usize>>, buffers: &mut BTreeMap<String, Buffer>, fusions: &mut Vec<Fusion>, io: &StepIo, sms: usize) {
    let bsz = 1 + io.more.len();
    let positions: Vec<String> = io.positions().into_iter().map(String::from).collect();
    // The positions are the program's own array already, and the logits rows of the head's: the rest stack.
    let io_names: BTreeSet<String> = io.positions().into_iter().chain(io.more.iter().map(|q| q.logits.as_str())).chain([io.logits.as_str()]).map(String::from).collect();
    // The families: every sequence's array alike, carried or computed — plain, quantized where each member's producer
    // quantizes it, or both, its rows whole blocks where quantized, so each member's forms are its parts of the array's.
    let families: Vec<(String, Vec<String>)> = buffers
        .keys()
        .filter_map(|name| name.strip_suffix("_s0"))
        .map(|base| (base.to_string(), (0..bsz).map(|b| format!("{base}_s{b}")).collect::<Vec<String>>()))
        .filter(|(_, members)| {
            let first = &buffers[&members[0]];
            !io_names.contains(&members[0])
                && matches!(first.fill, Fill::Zero | Fill::Computed)
                && (first.plain || first.quantized)
                && (!first.quantized || quant_in_place(first.len))
                && members.iter().all(|m| {
                    buffers.get(m).is_some_and(|b| {
                        b.len == first.len && b.dtype == first.dtype && b.fill == first.fill && b.plain == first.plain && b.quantized == first.quantized
                    })
                })
        })
        .collect();
    // Each name's readers and writers, in order. A set a family's member is set by leaves the step once every family
    // is laid out; until then it is passed over, as gone.
    let (mut readers, mut writers): (std::collections::HashMap<String, Vec<usize>>, std::collections::HashMap<String, Vec<usize>>) = Default::default();
    for (i, l) in launches.iter().enumerate() {
        for t in l.reads().into_iter().collect::<BTreeSet<_>>() {
            readers.entry(t.to_string()).or_default().push(i);
        }
        for t in l.writes().into_iter().collect::<BTreeSet<_>>() {
            writers.entry(t.to_string()).or_default().push(i);
        }
    }
    let mut gone: BTreeSet<usize> = BTreeSet::new();
    for (base, members) in families {
        let len = buffers[&members[0]].len;
        // Each member set whole into its row of one table, by its only reader: the table's rows are the members.
        let into: Option<(String, Vec<usize>)> = {
            let sets: Vec<Option<(usize, String)>> = members
                .iter()
                .enumerate()
                .map(|(b, m)| {
                    let readers: Vec<usize> = readers.get(m).map_or_else(Vec::new, |at| at.iter().copied().filter(|i| !gone.contains(i)).collect());
                    match readers[..] {
                        [j] => match &launches[j] {
                            Launch::SetRow { table, index: RowSel::Fixed(r), src, cols, count: 1, seq: None, f32, quant }
                                if src == m && *cols == len && *r == b + 1 && *f32 == buffers[m].plain && *quant == buffers[m].quantized =>
                            {
                                Some((j, table.clone()))
                            }
                            _ => None,
                        },
                        _ => None,
                    }
                })
                .collect();
            match sets.into_iter().collect::<Option<Vec<_>>>() {
                Some(sets)
                    if sets.iter().all(|(_, t)| *t == sets[0].1)
                        && buffers.get(&sets[0].1).is_some_and(|t| {
                            let first = &buffers[&members[0]];
                            t.len == bsz * len && (t.plain || !first.plain) && (t.quantized || !first.quantized)
                        }) =>
                {
                    Some((sets[0].1.clone(), sets.iter().map(|(j, _)| *j).collect()))
                }
                _ => None,
            }
        };
        let array = match &into {
            Some((table, _)) => table.clone(),
            None => {
                let array = format!("{base}_s");
                if buffers.contains_key(&array) {
                    continue;
                }
                let first = buffers[&members[0]].clone();
                buffers.insert(array.clone(), Buffer { name: array.clone(), len: bsz * len, ..first });
                array
            }
        };
        for (b, m) in members.iter().enumerate() {
            buffers.get_mut(m).expect("a member").fill = Fill::Alias { of: array.clone(), offset: b * len };
        }
        if let Some((_, sets)) = into {
            // Each member's set made by its producer, which now writes the table's row: the set's graph op is its.
            for (b, &j) in sets.iter().enumerate() {
                if let Some(p) = writers.get(&members[b]).and_then(|at| at.iter().copied().find(|i| !gone.contains(i))) {
                    let set = ops[j].clone();
                    ops[p].extend(set);
                    ops[p].sort_unstable();
                }
            }
            gone.extend(sets);
        }
    }
    for j in gone.into_iter().rev() {
        launches.remove(j);
        ops.remove(j);
    }
    // Every name's span, as `span` finds it: the buffers are laid out by here.
    let mut spans = SpanCache::default();
    // A sequence's position: item `b` of the positions.
    let seq_of_pos = |p: &Option<String>| p.as_ref().and_then(|p| positions.iter().position(|q| q == p));
    // Each kind of per-sequence launch: its sequence and the launch over all of them, built from sequence 0's — the
    // attentions first, then the cache rows' writes, then the rotations, so each set moves past only the later
    // sequences' launches of the kinds before it, never one its own sequence's launch reads.
    type Merge = fn(&[&Launch], &BTreeMap<String, Buffer>, &str) -> Option<Launch>;
    let kinds: [(&str, Merge); 8] = [
        ("delta", merge_deltas),
        ("gates", merge_gates),
        ("conv", merge_convs),
        ("attention", merge_attentions),
        ("join", merge_joins),
        ("set_row", merge_row_sets),
        ("rope", merge_rotations),
        ("argmax", merge_argmaxes),
    ];
    // The launches as slots: a merged set's members but its last leave theirs empty, closed up once every kind is merged.
    let mut slots: Vec<Option<(Launch, Vec<usize>)>> = std::mem::take(launches).into_iter().zip(std::mem::take(ops)).map(Some).collect();
    let live = |slots: &[Option<(Launch, Vec<usize>)>], from: usize, to: usize| -> Vec<usize> {
        (from..to).filter(|&j| slots[j].is_some()).collect()
    };
    for (what, merge) in kinds {
        // The per-sequence launches of this kind, by sequence, in order: a run of `bsz` from sequence 0 to the last.
        for i in 0..slots.len() {
            let Some((first, _)) = &slots[i] else { continue };
            if kind(first) != what || launch_sequence(first, &seq_of_pos) != Some(0) {
                continue;
            }
            let mut at = vec![i];
            for j in i + 1..slots.len() {
                if at.len() == bsz {
                    break;
                }
                if let Some((l, _)) = &slots[j] {
                    if kind(l) == what && launch_sequence(l, &seq_of_pos) == Some(at.len()) {
                        at.push(j);
                    }
                }
            }
            let members: Vec<&Launch> = at.iter().map(|&j| &slots[j].as_ref().expect("a member").0).collect();
            let merged = if at.len() == bsz { merge(&members, buffers, &positions[0]) } else { None };
            let Some(mut merged) = merged else { continue };
            // A merged attention's rows are every sequence's: whether its chunks take blocks of their own is decided
            // for them all, the arithmetic the same either way.
            if let Launch::Attention { rows, heads, per_block, splits, parallel, .. } = &mut merged {
                *parallel = chunks_apart(*splits, *rows, *heads / *per_block, sms);
            }
            let last = *at.last().expect("a run");
            // Each member moves to the last's place past the launches between: none may write what it reads, or read
            // or write what it writes, an array's part overlapping the whole of it. One sweep from the first member to
            // the last: each launch between met against every member before it, what they touch and what they write.
            let (mut touched, mut written) = (Spans::default(), Spans::default());
            let mut crossed = false;
            for j in live(&slots, at[0], last) {
                let l = &slots[j].as_ref().expect("live").0;
                let (reads, writes) = (l.reads(), l.writes());
                if at.binary_search(&j).is_ok() {
                    for t in reads.iter().chain(&writes) {
                        touched.add(spans.of(buffers, t));
                    }
                    for t in &writes {
                        written.add(spans.of(buffers, t));
                    }
                } else if writes.iter().any(|t| touched.meets(spans.of(buffers, t))) || reads.iter().any(|t| written.meets(spans.of(buffers, t))) {
                    crossed = true;
                    break;
                }
            }
            if crossed {
                continue;
            }
            let mut joined: Vec<usize> = at.iter().flat_map(|&j| slots[j].as_ref().expect("a member").1.clone()).collect();
            joined.sort_unstable();
            fusions.push(Fusion { rule: "sequences", ops: joined.clone() });
            slots[last] = Some((merged, joined));
            for &j in &at[..at.len() - 1] {
                slots[j] = None;
            }
        }
    }
    (*launches, *ops) = slots.into_iter().flatten().unzip();
}

/// Whether `op` is a batch's gated attentions (`attn_output_gate` over its sequences' rows), or their fold with the
/// quantize of the product: the op the lane runs as each sequence's attention and its row's set.
pub fn gates_batch_rows(op: &crate::tensor_graph::graph::Op) -> bool {
    let Sym::Fused(f) = &op.sym else { return false };
    match f.class {
        ClassId::AttnOutputGate => gates_rows(&f.parts),
        ClassId::GluActMulQuant => gates_batch_rows(&f.parts[0]),
        _ => false,
    }
}

/// Each name's span (`span`), found once.
#[derive(Default)]
struct SpanCache(std::collections::HashMap<String, (String, usize, usize)>);

impl SpanCache {
    fn of(&mut self, buffers: &BTreeMap<String, Buffer>, name: &str) -> (String, usize, usize) {
        if let Some(s) = self.0.get(name) {
            return s.clone();
        }
        let s = span(buffers, name);
        self.0.insert(name.to_string(), s.clone());
        s
    }
}

/// Spans gathered by array, each array's parts as a set of positions — the overlapping ones joined, so a span meets
/// the set where it meets one of them — and its empty ones apart: an empty span at `p` meets a part around `p` but
/// never another empty one, as `span`'s parts overlap.
#[derive(Default)]
struct Spans(std::collections::HashMap<String, (BTreeMap<usize, usize>, BTreeSet<usize>)>);

impl Spans {
    fn add(&mut self, (root, from, to): (String, usize, usize)) {
        let (parts, empty) = self.0.entry(root).or_default();
        if from == to {
            empty.insert(from);
            return;
        }
        let (mut from, mut to) = (from, to);
        // Every part overlapping the new one joins it.
        let joined: Vec<usize> = parts.range(..to).rev().take_while(|(_, &e)| e > from).map(|(&s, _)| s).collect();
        for s in joined {
            let e = parts.remove(&s).expect("a part");
            (from, to) = (from.min(s), to.max(e));
        }
        parts.insert(from, to);
    }

    /// Whether `[from, to)` overlaps a span gathered: some part `[s, e)` with `from < e` and `s < to`.
    fn meets(&self, (root, from, to): (String, usize, usize)) -> bool {
        let Some((parts, empty)) = self.0.get(&root) else { return false };
        let part = parts.range(..to.max(from + 1)).next_back().is_some_and(|(&s, &e)| s < to && from < e);
        part || (from < to && empty.range(from + 1..to).next().is_some())
    }
}

/// The batch sequence a per-sequence launch is for: the item of the positions it reads — or, for one that reads no
/// position, the sequence whose array it writes, `X_s<b>`.
fn launch_sequence(l: &Launch, seq_of_pos: &dyn Fn(&Option<String>) -> Option<usize>) -> Option<usize> {
    let of_name = |t: &str| t.rsplit_once("_s").and_then(|(_, b)| (!b.is_empty() && b.bytes().all(|c| c.is_ascii_digit())).then(|| b.parse().ok()).flatten());
    match l {
        Launch::RopeParts { pos, seqs: false, .. } | Launch::Attention { pos, seq: None, rows: 1, .. } => seq_of_pos(pos),
        Launch::SetRow { index: RowSel::Param(p), seq: None, count: 1, .. } => seq_of_pos(&Some(p.clone())),
        Launch::Delta { out, rows: 1, seqs: 1, .. } => of_name(out),
        Launch::ArgMax { advance: Some(a), rows: 1, .. } => seq_of_pos(&Some(a.position.clone())),
        Launch::Join { y, rows: 1, yz: 0, .. } => match &y.row {
            Some((RowSel::Param(p), _)) => seq_of_pos(&Some(p.clone())),
            Some(_) => None,
            None => of_name(&y.buf),
        },
        Launch::Gates { beta, .. } => of_name(beta),
        Launch::ConvParts { parts, seqs: 1, .. } => of_name(&parts[0].y),
        _ => None,
    }
}

/// A layer's joins, one launch a sequence joining its row's units into its own tensor or into its own cache's row at its
/// position, as one launch whose rows are the sequences.
fn merge_joins(members: &[&Launch], buffers: &BTreeMap<String, Buffer>, positions: &str) -> Option<Launch> {
    let Launch::Join { y, rows: 1, heads, ha, hb, yz: 0, .. } = members[0] else { return None };
    let width = heads * (ha + hb);
    let table = match &y.row {
        None => width,
        Some((RowSel::Param(_), _)) => buffers.get(&y.buf)?.len,
        Some(_) => return None,
    };
    let parts = |l: &Launch| match l {
        Launch::Join { a, b, y: Dst { buf, row, unit: None }, rows: 1, heads: h, ha: x, hb: z, yz: 0 }
            if (h, x, z) == (heads, ha, hb) && row.as_ref().map(|(_, p)| p) == y.row.as_ref().map(|(_, p)| p) =>
        {
            Some((sequence_part(buffers, a, heads * ha)?, sequence_part(buffers, b, heads * hb)?, sequence_part(buffers, buf, table)?))
        }
        _ => None,
    };
    let ((ar, _), (br, _), (yr, _)) = parts(members[0])?;
    for (s, l) in members.iter().enumerate() {
        if parts(l)? != ((ar.clone(), s), (br.clone(), s), (yr.clone(), s)) {
            return None;
        }
    }
    let (row, yz) = match &y.row {
        Some((_, pitch)) => (Some((RowSel::Param(positions.to_string()), *pitch)), table),
        None => (None, 0),
    };
    Some(Launch::Join { a: ar, b: br, y: Dst { buf: yr, row, unit: None }, rows: members.len(), heads: *heads, ha: *ha, hb: *hb, yz })
}

/// A batch's arg-maxes, one launch a sequence over its row of the head's logits advancing its own token, position and
/// run, as one launch over the sequences.
fn merge_argmaxes(members: &[&Launch], buffers: &BTreeMap<String, Buffer>, positions: &str) -> Option<Launch> {
    let Launch::ArgMax { x, out, n, advance: Some(a), rows: 1 } = members[0] else { return None };
    let (xa, _) = sequence_part(buffers, x, *n)?;
    let (oa, _) = sequence_part(buffers, out, 1)?;
    for (s, l) in members.iter().enumerate() {
        let Launch::ArgMax { x, out, n: m, advance: Some(b), rows: 1 } = l else { return None };
        if m != n || b.token != a.token || b.item != s || sequence_part(buffers, x, *n) != Some((xa.clone(), s)) || sequence_part(buffers, out, 1) != Some((oa.clone(), s)) {
            return None;
        }
    }
    let advance = Some(Advance { token: a.token.clone(), position: positions.to_string(), item: 0 });
    Some(Launch::ArgMax { x: xa, out: oa, n: *n, advance, rows: members.len() })
}

/// A recurrent layer's delta rules, one launch a sequence over its one row and its own memory, as one launch over the
/// sequences: each input, gate, memory and output the sequence's part of one array of them all.
fn merge_deltas(members: &[&Launch], buffers: &BTreeMap<String, Buffer>, _: &str) -> Option<Launch> {
    let Launch::Delta { q, k, v, z, b, a, st, gn, out, f32, quant, hv, hk, d, rows: 1, eps, .. } = members[0] else { return None };
    let widths = [(q, hk * d), (k, hk * d), (v, hv * d), (z, hv * d), (b, *hv), (a, *hv), (st, hv * d * d), (out, hv * d)];
    let arrays: Vec<String> = widths.iter().map(|(t, w)| sequence_part(buffers, t, *w).map(|(arr, _)| arr)).collect::<Option<_>>()?;
    for (s, l) in members.iter().enumerate() {
        let Launch::Delta { q, k, v, z, b, a, st, gn: g, out, f32: f, quant: qu, hv: h, hk: kk, d: dd, rows: 1, eps: e, .. } = l else { return None };
        let names = [q, k, v, z, b, a, st, out];
        if (g, f, qu, h, kk, dd, e) != (gn, f32, quant, hv, hk, d, eps)
            || names.iter().zip(&widths).zip(&arrays).any(|((t, (_, w)), arr)| sequence_part(buffers, t, *w) != Some((arr.clone(), s)))
        {
            return None;
        }
    }
    let [q, k, v, z, b, a, st, out]: [String; 8] = arrays.try_into().ok()?;
    Some(Launch::Delta { q, k, v, z, b, a, st, gn: gn.clone(), out, f32: *f32, quant: *quant, hv: *hv, hk: *hk, d: *d, rows: 1, eps: *eps, seqs: members.len() })
}

/// A recurrent layer's gates, one launch a sequence over its row of the gate projections, as one launch over every
/// sequence's row: each gate's value is its element's alone.
fn merge_gates(members: &[&Launch], buffers: &BTreeMap<String, Buffer>, _: &str) -> Option<Launch> {
    let Launch::Gates { br, ar, dt, al, beta, decay, n, heads } = members[0] else { return None };
    let names = [br, ar, beta, decay];
    let arrays: Vec<String> = names.iter().map(|t| sequence_part(buffers, t, *n).map(|(arr, _)| arr)).collect::<Option<_>>()?;
    for (s, l) in members.iter().enumerate() {
        let Launch::Gates { br, ar, dt: d, al: a, beta, decay, n: m, heads: h } = l else { return None };
        if (d, a, m, h) != (dt, al, n, heads) || [br, ar, beta, decay].iter().zip(&arrays).any(|(t, arr)| sequence_part(buffers, t, *n) != Some((arr.clone(), s))) {
            return None;
        }
    }
    let [br, ar, beta, decay]: [String; 4] = arrays.try_into().ok()?;
    Some(Launch::Gates { br, ar, dt: dt.clone(), al: al.clone(), beta, decay, n: n * members.len(), heads: *heads })
}

/// A recurrent layer's convolutions, one launch a sequence of its parts, as one launch over the sequences: each part's
/// input row, ring and output the sequence's part of one array of them all.
fn merge_convs(members: &[&Launch], buffers: &BTreeMap<String, Buffer>, _: &str) -> Option<Launch> {
    let Launch::ConvParts { parts, taps, block, eps, seqs: 1 } = members[0] else { return None };
    let mut merged = Vec::new();
    for (k, part) in parts.iter().enumerate() {
        let ring = taps * part.chan;
        if buffers.get(&part.ring)?.len != ring {
            return None;
        }
        let (x, _) = sequence_part(buffers, &part.x, part.chan)?;
        let (r, _) = sequence_part(buffers, &part.ring, ring)?;
        let (y, _) = sequence_part(buffers, &part.y, part.chan)?;
        for (s, l) in members.iter().enumerate() {
            let Launch::ConvParts { parts: ps, taps: t, block: bl, eps: e, seqs: 1 } = l else { return None };
            let o = ps.get(k)?;
            if ps.len() != parts.len() || (t, bl, e) != (taps, block, eps) || (o.chan, o.l2, &o.w) != (part.chan, part.l2, &part.w)
                || sequence_part(buffers, &o.x, part.chan) != Some((x.clone(), s))
                || sequence_part(buffers, &o.ring, ring) != Some((r.clone(), s))
                || sequence_part(buffers, &o.y, part.chan) != Some((y.clone(), s))
            {
                return None;
            }
        }
        merged.push(ConvPart { x, w: part.w.clone(), ring: r, y, chan: part.chan, l2: part.l2 });
    }
    Some(Launch::ConvParts { parts: merged, taps: *taps, block: *block, eps: *eps, seqs: members.len() })
}

/// A layer's rotations, one launch a sequence of its query and its key — the query's row of the stacked projection into
/// its row of the queries, the key's into its cache's row at its position — as one launch over the sequences.
fn merge_rotations(members: &[&Launch], buffers: &BTreeMap<String, Buffer>, positions: &str) -> Option<Launch> {
    let Launch::RopeParts { sides, cos, sin, hd, half, .. } = members[0] else { return None };
    let mut merged: Vec<RopeSide> = Vec::new();
    for (k, side) in sides.iter().enumerate() {
        if side.part.is_some() || side.y.unit.is_some() || side.heads != side.per_row {
            return None;
        }
        let width = side.heads * hd;
        // Each sequence's input row, at its place in one stacked tensor.
        let (x, _) = sequence_part(buffers, &side.x, width)?;
        let cache = side.y.row.as_ref();
        let (y, table) = match cache {
            None => (sequence_part(buffers, &side.y.buf, width)?.0, 0),
            Some(_) => {
                let len = buffers.get(&side.y.buf)?.len;
                (sequence_part(buffers, &side.y.buf, len)?.0, len)
            }
        };
        for (b, l) in members.iter().enumerate() {
            let Launch::RopeParts { sides: s, cos: c, sin: sn, hd: h, half: hf, .. } = l else { return None };
            let o = s.get(k)?;
            let width_ok = sequence_part(buffers, &o.x, width) == Some((x.clone(), b));
            let y_ok = match (cache, &o.y.row) {
                (None, None) => sequence_part(buffers, &o.y.buf, width) == Some((y.clone(), b)),
                (Some((_, pitch)), Some((RowSel::Param(_), p))) => p == pitch && sequence_part(buffers, &o.y.buf, table) == Some((y.clone(), b)),
                _ => false,
            };
            if s.len() != sides.len() || c != cos || sn != sin || h != hd || hf != half || o.norm != side.norm || o.heads != side.heads || !width_ok || !y_ok {
                return None;
            }
        }
        let row = cache.map(|(_, pitch)| (RowSel::Param(positions.to_string()), *pitch));
        merged.push(RopeSide {
            x,
            part: None,
            norm: side.norm.clone(),
            y: Dst { buf: y, row, unit: None },
            heads: side.heads * members.len(),
            per_row: side.per_row,
            yz: table,
        });
    }
    Some(Launch::RopeParts { sides: merged, cos: cos.clone(), sin: sin.clone(), pos: Some(positions.to_string()), hd: *hd, half: *half, seqs: true })
}

/// A layer's value rows, one launch a sequence setting its row of the stacked projection into its cache's row at its
/// position, as one launch over the sequences.
fn merge_row_sets(members: &[&Launch], buffers: &BTreeMap<String, Buffer>, positions: &str) -> Option<Launch> {
    let Launch::SetRow { table, src, cols, f32: true, quant: false, .. } = members[0] else { return None };
    let len = buffers.get(table)?.len;
    let (t, _) = sequence_part(buffers, table, len)?;
    let (x, _) = sequence_part(buffers, src, *cols)?;
    for (b, l) in members.iter().enumerate() {
        let Launch::SetRow { table: tb, src: sb, cols: cb, count: 1, f32: true, quant: false, .. } = l else { return None };
        if cb != cols || sequence_part(buffers, tb, len) != Some((t.clone(), b)) || sequence_part(buffers, sb, *cols) != Some((x.clone(), b)) {
            return None;
        }
    }
    Some(Launch::SetRow { table: t, index: RowSel::Param(positions.to_string()), src: x, cols: *cols, count: members.len(), seq: Some(len), f32: true, quant: false })
}

/// A layer's attentions, one launch a sequence over its own caches up to its position, as one launch whose rows are the
/// sequences: each row's query and result its row of one array, its caches its part of one array of them.
fn merge_attentions(members: &[&Launch], buffers: &BTreeMap<String, Buffer>, positions: &str) -> Option<Launch> {
    let Launch::Attention { q, kc, vc, out, heads, hd, vd, split: None, gate, goff, .. } = members[0] else { return None };
    let len = buffers.get(kc)?.len;
    if buffers.get(vc)?.len != len {
        return None;
    }
    let (qa, _) = sequence_part(buffers, q, heads * hd)?;
    let (oa, _) = sequence_part(buffers, out, heads * vd)?;
    let (ka, _) = sequence_part(buffers, kc, len)?;
    let (va, _) = sequence_part(buffers, vc, len)?;
    for (b, l) in members.iter().enumerate() {
        let Launch::Attention { q: qb, kc: kb, vc: vb, out: ob, pos: _, gate: gb, goff: gob, .. } = l else { return None };
        let same = |x: &Launch| match (x, members[0]) {
            (
                Launch::Attention { sinks, heads, group, hd, vd, len, window, vstride, voff, scale, per_block, splits, parallel, split, shared, f32, quant, .. },
                Launch::Attention { sinks: s0, heads: h0, group: g0, hd: d0, vd: v0, len: l0, window: w0, vstride: vs0, voff: vo0, scale: sc0, per_block: p0, splits: sp0, parallel: pa0, split: st0, shared: sh0, f32: f0, quant: qu0, .. },
            ) => {
                (sinks, heads, group, hd, vd, len, window) == (s0, h0, g0, d0, v0, l0, w0)
                    && (vstride, voff, per_block, splits, parallel, split, shared) == (vs0, vo0, p0, sp0, pa0, st0, sh0)
                    && (f32, quant) == (f0, qu0)
                    && scale.to_bits() == sc0.to_bits()
            }
            _ => false,
        };
        // A gated sequence's gate is its row of one gate, as its output is its row of one table.
        let gated_alike = gb == gate && (gate.is_none() || *gob == goff + b * heads * vd);
        if !same(l)
            || !gated_alike
            || sequence_part(buffers, qb, heads * hd) != Some((qa.clone(), b))
            || sequence_part(buffers, ob, heads * vd) != Some((oa.clone(), b))
            || sequence_part(buffers, kb, len) != Some((ka.clone(), b))
            || sequence_part(buffers, vb, len) != Some((va.clone(), b))
        {
            return None;
        }
    }
    let mut merged = members[0].clone();
    if let Launch::Attention { q, kc, vc, out, pos, rows, seq, .. } = &mut merged {
        *q = qa;
        *kc = ka;
        *vc = va;
        *out = oa;
        *pos = Some(positions.to_string());
        *rows = members.len();
        *seq = Some(len);
    }
    Some(merged)
}

/// An attention below the tiles whose values are its key rows' own part, under the quantized numerics, every query
/// head reading the one key/value group (a latent attention's absorbed form): one block takes every head and each
/// cached row is read once for all of them (`lk_attention_shared`), where head blocks of their own would each read the
/// whole span. Its span is cut into the decode step's chunks in every form, a block each, folded by a launch of their
/// own (`lk_attention_fold`). Taken where two steps' rows fit one block's shared memory. The values are the key rows'
/// part whether the attention reads them in place (`strided_values`) or a launch slices them out first, so the choice
/// and the results are the same with that rule held back.
fn shared_rows(s: &mut Slots, numerics: Q8MatKind, knobs: &CudaKnobs) {
    if numerics == Q8MatKind::Dequant {
        return;
    }
    for i in 0..s.len() {
        let Some((l, _)) = s.slot(i) else { continue };
        let fits = !l.attends_in_tiles(numerics)
            && matches!(l, Launch::Attention { heads, group, hd, vd, split: None, .. }
                if group == heads && *heads <= kernels::ATTN_SHARED_HEADS && hd % 4 == 0 && vd % 2 == 0)
            && values_in_key_rows(s, i);
        if !fits {
            continue;
        }
        if let Launch::Attention { heads, hd, len, window, shared, .. } = s.get_mut(i) {
            if kernels::attn_shared_smem_bytes(*heads, *hd) <= knobs.chip.smem_kib_per_block * 1024 {
                *shared = Some(kernels::attn_shared_splits(attended_span(*len, *window), knobs.chip.sms));
            }
        }
    }
}

/// Whether attention `j`'s values are its key rows' own part, an even number of values from an even place: read in
/// place in the key rows, or sliced out of them by a launch of their own with the keys untouched between.
fn values_in_key_rows(s: &Slots, j: usize) -> bool {
    let Launch::Attention { kc, vc, heads, group, hd, vd, vstride, voff, .. } = s.get(j) else { return false };
    let (kvw, vw) = (heads / group * hd, heads / group * vd);
    if kc == vc {
        return *vstride == kvw && voff % 2 == 0 && voff + vd <= *hd;
    }
    let Some(i) = s.producer(vc) else { return false };
    matches!(s.get(i), Launch::Slice { x, every, from, width, .. } if x == kc && *every == kvw && from % 2 == 0 && from + vd <= *hd && *width == vw)
        && *voff == 0
        && *vstride == vw
        && s.live().filter(|(k, _)| *k > i && *k < j).all(|(_, l)| !l.writes().contains(&kc.as_str()))
}

/// The steps of a lane a plan can hold back ([`plan`]'s `held`), in the order the lane takes them: the shared rules
/// its kernels link, by their names in the shared table ([`crate::tensor_graph::fuse`]); `rows_a_warp`, the schedule's
/// four rows a warp for a wide product; the decode step's products of one activation as one GEMV in parts
/// (`gemv_parts`, the lane's own rule in the table); and `sequences`, a batch's launches over its sequences as one. The
/// arg-max's advance closes the step and is always taken; the activation quantizer's fusion is the `fuse_quant` knob.
pub const RULES: [&str; 27] = [
    "add_norm",
    "sandwich_norm",
    "gemv_bias",
    "gemv_norm_epilogue",
    "head_norm_rope",
    "slice_read",
    "row_write",
    "qkv_head_epilogue",
    "join_in_place",
    "conv_silu",
    "conv_silu_l2",
    "conv_parts",
    "gates",
    "gate_projections",
    "act_gate",
    "attn_output_gate",
    "glu_pair",
    "shared_expert",
    "moe_rows",
    "moe_deferred_finalize",
    "gather_scale",
    "embed_norm",
    "strided_values",
    "rows_a_warp",
    "rope_parts",
    "gemv_parts",
    "sequences",
];

/// A projection, gated pair or mixture whose shape outruns one output row a warp takes several
/// (`kernels::rows_a_warp`), each row's dot unchanged. A GEMV in parts takes its parts' rows a warp as the link set
/// them, each part recorded as taking them: the parts of the op of `g` that computes the launch's ops.
fn rows_a_warp(s: &mut Slots, fusions: &mut Vec<Fusion>, g: &TensorGraph) {
    for i in 0..s.len() {
        let Some((l, ops)) = s.slot_mut(i) else { continue };
        // A mixture with the next norm in its last step takes its rows as the mixture alone takes them.
        let l = if l.finished_mixture().is_some() { l.mixture_mut().expect("a finished mixture") } else { l };
        let (wide, per_warp) = match l {
            Launch::Gemv { format, k, n, m, per_warp, .. } => (kernels::rows_a_warp(*format, *n, *k, *m), per_warp),
            Launch::GemvGlu { formats: [fg, fu], k, n, m, per_warp, .. } => {
                (kernels::rows_a_warp(*fg, *n, *k, *m).min(kernels::rows_a_warp(*fu, *n, *k, *m)), per_warp)
            }
            Launch::Moe { formats, d, dff, k, shared, per_warp, rows: 1, .. } => {
                let hidden = *k * *dff + shared.as_ref().map_or(0, |e| e.dff);
                let formats = [formats[0], formats[1]].into_iter().chain(shared.iter().flat_map(|e| [e.formats[0], e.formats[1]]));
                (formats.map(|f| kernels::rows_a_warp(f, hidden, *d, 1)).min().expect("the gate's format"), per_warp)
            }
            Launch::GemmParts { per_warp, .. } if *per_warp > 1 => {
                for part in products_of(g, ops) {
                    fusions.push(Fusion { rule: "rows_a_warp", ops: part });
                }
                continue;
            }
            Launch::Fold { first, .. } if matches!(first.as_ref(), Launch::GemmParts { per_warp, .. } | Launch::Gemv { per_warp, .. } if *per_warp > 1) => {
                for part in products_of(g, ops) {
                    fusions.push(Fusion { rule: "rows_a_warp", ops: part });
                }
                continue;
            }
            _ => continue,
        };
        if wide > 1 {
            *per_warp = wide;
            fusions.push(Fusion { rule: "rows_a_warp", ops: ops.clone() });
        }
    }
}

/// The lifted ops of each product a GEMV in parts computes — a part, or the projection of a part that turns its heads
/// in its epilogue — from the op of `g` that computes the launch's `ops`.
fn products_of(g: &TensorGraph, ops: &[usize]) -> Vec<Vec<usize>> {
    products_in(g.ops.iter().find(|op| covered(op) == ops).expect("a GEMV in parts is the op computing its ops"))
}

/// The products a fused op computes: a fold's projection, a GEMV in parts' every part, the producer's where a quantize
/// rides its epilogue.
fn products_in(op: &crate::tensor_graph::graph::Op) -> Vec<Vec<usize>> {
    use ClassId::*;
    let Sym::Fused(f) = &op.sym else { panic!("a GEMV in parts is a fold of its parts") };
    let product = |p: &crate::tensor_graph::graph::Op| match &p.sym {
        Sym::Fused(e) if e.class == QkvHeadEpilogue => covered(&e.parts[0]),
        _ => covered(p),
    };
    match f.class {
        QkvHeadEpilogue | GemvNormEpilogue => vec![covered(&f.parts[0])],
        NormQuant | AddNormQuant | GluActMulQuant | ElementwiseQuant | GdnOutputQuant | GemvQuant | AttnOutputQuant
            if matches!(f.parts[0].sym, Sym::Fused(_)) =>
        {
            products_in(&f.parts[0])
        }
        _ => f.parts.iter().map(product).collect(),
    }
}

/// The lifted ops an op of the IR computes, in order.
fn covered(op: &crate::tensor_graph::graph::Op) -> Vec<usize> {
    let mut c = op.covers.clone();
    c.sort_unstable();
    c
}

/// A launch's split counts: a wide chunk's `f32` router and `f32` projections multiply their rows as one GEMM, its `k`
/// cut to give the chip's blocks work; any other `f32` projection cuts each output row's `k` over a block's warps; the
/// gates from their activation cut each head's dots as the projections they replace cut them.
fn split_counts(l: &mut Launch, numerics: Q8MatKind, knobs: &CudaKnobs) {
    let (on_cores, in_tiles) = (l.on_tensor_cores(numerics), l.multiplies_in_tiles(numerics));
    match l {
        Launch::Moe { router: Router::F32, rows, experts, d, router_split, .. } if on_cores => {
            let tiles = experts.div_ceil(64) * rows.div_ceil(64);
            *router_split = kernels::gemm_f32_splits(tiles, knobs.chip.sms, d.div_ceil(32));
        }
        Launch::GemvF32 { k, n, m, split, .. } if in_tiles => {
            *split = kernels::gemm_f32_splits(n.div_ceil(64) * m.div_ceil(64), knobs.chip.sms, k.div_ceil(32));
        }
        Launch::GemvF32 { k, n, split, .. } => *split = kernels::gemv_f32_parts(*n, *k, knobs.chip.sms),
        Launch::GateProj { k, heads, parts, .. } => *parts = kernels::gemv_f32_parts(*heads, *k, knobs.chip.sms),
        _ => {}
    }
}

/// Every activation a projection reads quantized is whole 32-value blocks, its quantized form's, and so is the tensor
/// a view of it lies in where a view's rows start on a whole block: the tensor is quantized once, for all its views.
fn whole_blocks(g: &StepGraph, launches: &[(Launch, Vec<usize>)]) -> Result<(), LaneError> {
    let whole = |x: &str| {
        let n = g.tensors[x].len();
        match n % QBLOCK {
            0 => Ok(()),
            _ => Err(LaneError::Shape { var: x.to_string(), declared: n, bound: n.next_multiple_of(QBLOCK) }),
        }
    };
    let views: BTreeMap<&str, (&str, usize)> = launches
        .iter()
        .filter_map(|(l, _)| if let Launch::View { src, offset, out } = l { Some((out.as_str(), (src.as_str(), *offset))) } else { None })
        .collect();
    let read: BTreeSet<&str> = launches.iter().filter_map(|(l, _)| l.projected_input()).collect();
    let mut roots = BTreeSet::new();
    for x in read {
        whole(x)?;
        let mut root = x;
        while let Some(&(src, _)) = views.get(root).filter(|(_, offset)| quant_in_place(*offset)) {
            root = src;
        }
        roots.insert(root);
    }
    roots.into_iter().try_for_each(whole)
}

/// Every tensor the launches touch, with the buffers it needs; `planned` holds the lengths of the tensors the plan's
/// own launches compute, which the step graph does not hold.
fn buffers(
    g: &StepGraph,
    planned: &BTreeMap<String, usize>,
    launches: &[Launch],
    mut constants: BTreeMap<String, Placement>,
    transposed: &BTreeSet<String>,
    numerics: Q8MatKind,
    io: &StepIo,
) -> Result<BTreeMap<String, Buffer>, LaneError> {
    let ints: BTreeSet<&str> = launches
        .iter()
        .filter_map(|l| if let Launch::ArgMax { out, .. } = l { Some(out.as_str()) } else { None })
        .collect();
    let views: BTreeMap<&str, (&str, usize)> = launches
        .iter()
        .filter_map(|l| if let Launch::View { src, offset, out } = l { Some((out.as_str(), (src.as_str(), *offset))) } else { None })
        .collect();
    let mut out: BTreeMap<String, Buffer> = BTreeMap::new();
    let mut mark = |out: &mut BTreeMap<String, Buffer>, name: &str, form: Touch| -> Result<(), LaneError> {
        if !out.contains_key(name) {
            if let Some(&len) = planned.get(name) {
                out.insert(name.to_string(), Buffer { name: name.to_string(), len, dtype: DType::F32, fill: Fill::Computed, plain: false, quantized: false });
                let b = out.get_mut(name).expect("inserted");
                match form {
                    Touch::Plain => b.plain = true,
                    Touch::Quant => b.quantized = true,
                }
                return Ok(());
            }
            let t = g.tensors.get(name).ok_or_else(|| LaneError::Unbound(name.to_string()))?;
            let (dtype, fill) = match t.role {
                Role::Param => (DType::I32, Fill::Param),
                Role::State => (DType::F32, Fill::Zero),
                Role::Constant => {
                    let mut p = constants.remove(name).ok_or_else(|| LaneError::Unbound(name.to_string()))?;
                    if let (true, Placement::Values(v)) = (transposed.contains(name), &mut p) {
                        *v = (0..t.cols).flat_map(|c| (0..t.rows).map(move |r| (r, c))).map(|(r, c)| v[r * t.cols + c]).collect();
                    }
                    (DType::F32, Fill::Constant(p))
                }
                Role::Activation => match views.get(name) {
                    Some(&(of, offset)) => (DType::F32, Fill::Alias { of: of.to_string(), offset }),
                    None if ints.contains(name) => (DType::I32, Fill::Computed),
                    None => (DType::F32, Fill::Computed),
                },
            };
            out.insert(name.to_string(), Buffer { name: name.to_string(), len: t.len(), dtype, fill, plain: false, quantized: false });
        }
        let b = out.get_mut(name).expect("inserted");
        match form {
            Touch::Plain => b.plain = true,
            Touch::Quant => b.quantized = true,
        }
        Ok(())
    };
    // A view's `f32` buffer is its tensor's, so reading one reads the other. (Its quantized form is its tensor's in place
    // too, and that tensor is quantized by the launch `quantize` gave it, which marks it.)
    let mut touch = |name: &str, form: Touch| -> Result<(), LaneError> {
        let mut at = Some(name);
        while let Some(name) = at {
            mark(&mut out, name, form)?;
            at = views.get(name).filter(|_| form == Touch::Plain).map(|&(of, _)| of);
        }
        Ok(())
    };
    for l in launches {
        let (reads, writes) = l.touches(numerics);
        for (t, form) in reads.into_iter().chain(writes) {
            touch(t, form)?;
        }
    }
    for name in io.names() {
        touch(name, Touch::Plain)?;
    }
    Ok(out)
}
