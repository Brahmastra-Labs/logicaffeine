//! The decode engine's machine: a step as the jobs its sequencer runs over the card's memory, every byte where it
//! lives.
//!
//! [`super::program`] says what a step computes; the machine says where each number is kept and how the units move
//! it. The card's memory ([`Layout`]) holds the weight image — every tensor's GGUF bytes in the order the program first
//! reads them — the rotary tables, and a region for every sequence the machine holds: per Gated-DeltaNet block its
//! conv ring, its memory and its log, per attention block its key and value caches, each in the format the knobs name.
//! A step's rows are its row table: each row's sequence, position and token.
//!
//! [`lower`] turns each command into one [`Job`]. The vector unit's jobs ([`VectorJob`]) name what each pass loads
//! into the vector registers — a span of a token-row's slot, a tensor's elements, the row's rotary table, its
//! sequence's ring — the micro-program it runs and what it stores, every pass a row or a head of a row. The array's,
//! the recurrence unit's and the attention unit's jobs name their tensors, slots and blocks. [`Machine::step`] runs the
//! jobs over the memory, reading every value as stored and writing it as stored, and computes what
//! [`super::model::EngineModel::step`] computes, bit for bit: the jobs are what the sequencer issues, and the memory
//! is what the card holds.

use std::collections::{HashMap, HashSet};

use crate::model::quant::{f16_to_f32, GgmlType};
use crate::model::reference::WeightSource;
use crate::model::spec::{Mixer, ModelSpec, Rope};
use crate::spatial::serving::{AttentionMath, StateFlush};

use super::model::{rotary_factors, rotation, runs, CacheFormat, EngineError, EngineNumerics, Replay, StateFormat};
use super::numerics::{
    attend_int8, attend_vector, f32_to_f16, fdot, gdn_flush, gdn_fold_trailing, gdn_read, gdn_row, ldexp, quantize, quantize_blocks,
    reads, silu, fadd, ActRow, ActScaling, CodedRow, FpUnit, Logged, GROUP,
};
use super::program::{Command, Program, Slot};
use super::vector::{self, Op, Regs, SReg, VReg};

type R<T> = Result<T, EngineError>;

/// Bytes every placed thing starts on: one line of the card's memory.
pub const LINE: u64 = 64;

/// Vector and scalar registers a vector job's passes run over.
pub const VREGS: usize = 16;
pub const SREGS: usize = 8;

fn align(at: u64) -> u64 {
    at.div_ceil(LINE) * LINE
}

/// A format of plain floats, which a matrix runs on the vector unit in.
fn is_float(ty: GgmlType) -> bool {
    matches!(ty, GgmlType::F32 | GgmlType::F16 | GgmlType::Bf16)
}

// ── The memory ──────────────────────────────────────────────────────────────────────────────────────

/// A tensor of the weight image: its format, its dims, where its bytes start and how many there are.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub name: String,
    pub ty: GgmlType,
    pub dims: Vec<u64>,
    pub at: u64,
    pub bytes: u64,
    /// For a convolution's kernel, its taps: the image holds it tap by tap, each tap's every channel together — the
    /// transpose of the checkpoint's order, each channel's taps together — so a tap is one contiguous read.
    pub taps: Option<usize>,
    /// A matrix of weight blocks a matvec job streams through the array (a float matrix runs on the vector unit).
    pub streamed: bool,
}

impl Placed {
    /// Bytes of one row of `dims[0]` elements.
    pub fn row_bytes(&self) -> u64 {
        self.ty.bytes_for(self.dims[0] as usize) as u64
    }

    fn elems(&self) -> usize {
        self.dims.iter().product::<u64>() as usize
    }
}

/// Where one block keeps a sequence's state, from the start of the sequence's region.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BlockState {
    Delta {
        /// The conv ring: `taps − 1` rows of `channels` elements, position `p`'s row at `p mod (taps − 1)`.
        ring: u64,
        channels: usize,
        taps: usize,
        /// The memory: per value head `width` rows of `width` elements.
        memory: u64,
        /// A lazily flushed memory's log: the count of tokens it holds and the slot of its oldest (four bytes each),
        /// then a line on, [`Replay::capacity`] slots of `entry` bytes, the log's `i`th token in slot
        /// `(oldest + i) mod capacity`, each every key group's key, every value head's values, every value head's decay
        /// exponent and every value head's gate, each field from a whole beat of sixteen elements ([`LogFields`]).
        log: Option<u64>,
        entry: u64,
        k_heads: usize,
        v_heads: usize,
        width: usize,
    },
    Attention {
        /// Per position, every cached head's key row; per position, every cached head's value row: binary16 elements,
        /// or int8 codes.
        keys: u64,
        values: u64,
        /// An int8 cache's scales, per position every cached head's key row's, then its value row's, one f16 each.
        key_scales: Option<u64>,
        value_scales: Option<u64>,
        kv_heads: usize,
        width: usize,
        value_width: usize,
    },
}

/// The card's memory as a machine lays it out.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub numerics: EngineNumerics,
    /// Every tensor the program reads, in [`Program::tensors`] order.
    pub tensors: Vec<Placed>,
    /// Every rotary table the program turns heads by, in [`Program::ropes`] order, each at its address: per position,
    /// `dims/2` cosines then `dims/2` sines, f32.
    pub rotary: Vec<(Rope, u64)>,
    /// Positions every sequence may reach.
    pub ctx: usize,
    /// Where the sequences' regions start, each `region` bytes, `sequences` of them.
    pub regions: u64,
    pub region: u64,
    pub sequences: usize,
    /// Each layer's place in a region.
    pub blocks: Vec<BlockState>,
    pub bytes: u64,
}

impl Layout {
    /// The memory of `sequences` sequences of `spec` at `numerics`, each reaching `ctx` positions, over the tensors
    /// `program` reads from `source`.
    pub fn of(spec: &ModelSpec, numerics: EngineNumerics, program: &Program, source: &dyn WeightSource, sequences: usize, ctx: usize) -> R<Layout> {
        if ctx == 0 || ctx > spec.ctx_len {
            return Err(EngineError::PastContext { sequence: 0, position: ctx });
        }
        let kernels: HashMap<&str, usize> = program
            .commands
            .iter()
            .filter_map(|c| match c {
                Command::Conv { kernel, taps, .. } => Some((kernel.as_str(), *taps)),
                _ => None,
            })
            .collect();
        let matrices: HashSet<&str> = program
            .commands
            .iter()
            .filter_map(|c| match c {
                Command::Matvec { tensor, .. } => Some(tensor.as_str()),
                _ => None,
            })
            .collect();
        let mut at = 0u64;
        let mut tensors = Vec::new();
        for name in program.tensors() {
            let (ty, dims) = source.describe(name)?;
            let elems = dims.iter().product::<u64>() as usize;
            let bytes = ty.bytes_for(elems) as u64;
            let taps = kernels.get(name).copied();
            if taps.is_some() && ty != GgmlType::F32 {
                return Err(EngineError::NotYet("a convolution kernel not stored as f32"));
            }
            let streamed = matrices.contains(name) && !is_float(ty);
            tensors.push(Placed { name: name.to_string(), ty, dims, at, bytes, taps, streamed });
            at = align(at + bytes);
        }
        let mut rotary = Vec::new();
        for rope in program.ropes() {
            if (rope.dims / 2 * 2 * 4) as u64 % LINE != 0 {
                return Err(EngineError::NotYet("a rotary table whose position the card's lines do not hold whole"));
            }
            rotary.push((rope.clone(), at));
            at = align(at + (ctx * rope.dims / 2 * 2 * 4) as u64);
        }
        let eb = state_bytes(numerics.state);
        let whole = |bytes: u64| bytes % LINE == 0;
        let mut blocks = Vec::new();
        let mut region = 0u64;
        for layer in &spec.layers {
            match &layer.mixer {
                Mixer::GatedDelta(g) => {
                    let channels = 2 * g.k_heads * g.head_dim + g.v_heads * g.head_dim;
                    if !whole(channels as u64 * eb) || !whole(g.head_dim as u64 * eb) || g.head_dim % LANES != 0 {
                        return Err(EngineError::NotYet("a Gated-DeltaNet row the card's lines do not hold whole"));
                    }
                    let ring = region;
                    let memory = align(ring + (g.conv_taps.saturating_sub(1) * channels) as u64 * eb);
                    region = align(memory + (g.v_heads * g.head_dim * g.head_dim) as u64 * eb);
                    let entry = align(LogFields::of(g.k_heads, g.v_heads, g.head_dim).elems as u64 * eb);
                    let log = match numerics.flush {
                        StateFlush::EveryPass => None,
                        StateFlush::Every { .. } | StateFlush::Trailing { .. } => {
                            let at = region;
                            region = align(at + LINE + Replay::capacity(numerics.flush) as u64 * entry);
                            Some(at)
                        }
                    };
                    blocks.push(BlockState::Delta {
                        ring,
                        channels,
                        taps: g.conv_taps,
                        memory,
                        log,
                        entry,
                        k_heads: g.k_heads,
                        v_heads: g.v_heads,
                        width: g.head_dim,
                    });
                }
                Mixer::Attention(a) => {
                    if !whole(cache_row_bytes(numerics.cache, a.head_dim)) || !whole(cache_row_bytes(numerics.cache, a.value_dim)) {
                        return Err(EngineError::NotYet("a cached head row the card's lines do not hold whole"));
                    }
                    let rows = |w: usize| ctx as u64 * a.kv_heads as u64 * cache_row_bytes(numerics.cache, w);
                    let scales = ctx as u64 * a.kv_heads as u64 * 2;
                    let keys = region;
                    let values = align(keys + rows(a.head_dim));
                    region = align(values + rows(a.value_dim));
                    let (key_scales, value_scales) = if numerics.cache.is_int8() {
                        let k = region;
                        region = align(k + scales);
                        let v = region;
                        region = align(v + scales);
                        (Some(k), Some(v))
                    } else {
                        (None, None)
                    };
                    blocks.push(BlockState::Attention {
                        keys,
                        values,
                        key_scales,
                        value_scales,
                        kv_heads: a.kv_heads,
                        width: a.head_dim,
                        value_width: a.value_dim,
                    });
                }
                Mixer::Mla(_) => return Err(EngineError::NotYet("multi-head latent attention")),
            }
        }
        let regions = at;
        let bytes = regions + region * sequences as u64;
        Ok(Layout { numerics, tensors, rotary, ctx, regions, region, sequences, blocks, bytes })
    }

    /// Where sequence `s`'s region starts.
    pub fn region_of(&self, s: usize) -> u64 {
        self.regions + s as u64 * self.region
    }

    /// The most lines a lazily flushed memory's log holds past its count's line, every entry it holds at its longest:
    /// none for a layout of no such memory.
    pub fn log_lines(&self) -> usize {
        let capacity = Replay::capacity(self.numerics.flush);
        self.blocks
            .iter()
            .filter_map(|b| match b {
                BlockState::Delta { log: Some(_), entry, .. } => Some(capacity * (entry / LINE) as usize),
                _ => None,
            })
            .max()
            .unwrap_or(0)
    }
}

/// Bytes of one element of a recurrent state stored as `format`.
pub fn state_bytes(format: StateFormat) -> u64 {
    match format {
        StateFormat::F32 => 4,
        StateFormat::F16 => 2,
    }
}

/// Elements a beat carries: the vector unit's lanes.
pub const LANES: usize = 16;

/// Where a log entry keeps its fields, in elements: every key group's key, every value head's values, every value
/// head's decay exponent, every value head's gate, each field from a whole beat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogFields {
    pub values: usize,
    pub log_decays: usize,
    pub gates: usize,
    pub elems: usize,
}

impl LogFields {
    pub fn of(k_heads: usize, v_heads: usize, width: usize) -> LogFields {
        let beats = |n: usize| n.div_ceil(LANES) * LANES;
        let values = beats(k_heads * width);
        let log_decays = values + beats(v_heads * width);
        let gates = log_decays + beats(v_heads);
        LogFields { values, log_decays, gates, elems: gates + beats(v_heads) }
    }
}

/// Bytes of one head row of `width` elements in a cache of `format`: binary16 elements, or int8 codes.
pub fn cache_row_bytes(format: CacheFormat, width: usize) -> u64 {
    match format {
        CacheFormat::F16 => 2 * width as u64,
        CacheFormat::Int8 | CacheFormat::Int8Hadamard => width as u64,
    }
}

// ── The jobs ────────────────────────────────────────────────────────────────────────────────────────

/// A register a store reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reg {
    V(VReg),
    S(SReg),
}

/// Where a vector job's load reads, for the pass's row and head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Elements from `at + head·stride` of the row's `slot`.
    Slot { slot: Slot, at: usize, stride: usize },
    /// Elements of tensor `tensor` from element `at + head·stride` of the image's order, in f32.
    Tensor { tensor: usize, at: usize, stride: usize },
    /// The row's position's cosines, or sines, in rotary table `table`.
    Rotary { table: usize, sines: bool },
    /// Layer `layer`'s ring row `age` of the row's sequence, the oldest `0`.
    Ring { layer: usize, age: usize },
}

/// Which part of a cache row a store writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CachePart {
    /// Every element as binary16.
    Halves,
    /// Every element as an int8 code.
    Codes,
    /// The row's f16 scale.
    Scale,
}

/// Where a vector job's store writes, for the pass's row and head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sink {
    Slot { slot: Slot, at: usize, stride: usize },
    /// Layer `layer`'s ring, the row's own row in place of the oldest, as the state stores it.
    Ring { layer: usize },
    /// Layer `layer`'s cache at the row's position, the pass's head: its keys, or its values.
    Cache { layer: usize, values: bool, part: CachePart },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Load {
    pub reg: VReg,
    pub from: Source,
    pub len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Put {
    pub reg: Reg,
    pub to: Sink,
    pub len: usize,
}

/// The order a vector job's passes run in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// Row after row, every head of a row before the next row.
    Rows,
    /// Head after head, a head's pass over every row before the next head: a job whose tensor span changes with the
    /// head reads each span once for all the rows.
    Heads,
}

/// A job of the vector unit: for every row of the step, in order, and every head of the row, a pass that loads its
/// registers, runs `program` and stores from them.
#[derive(Clone, Debug, PartialEq)]
pub struct VectorJob {
    pub heads: usize,
    pub order: Order,
    /// Slots the job writes whole, made afresh at its start, each of its width.
    pub fresh: Vec<(Slot, usize)>,
    pub loads: Vec<Load>,
    pub program: Vec<Op>,
    pub puts: Vec<Put>,
}

impl VectorJob {
    /// Whether a pass reads or writes its row's sequence's state, so a sequence's rows must run in order.
    pub fn ordered(&self) -> bool {
        self.loads.iter().any(|l| matches!(l.from, Source::Ring { .. })) || self.puts.iter().any(|p| matches!(p.to, Sink::Ring { .. }))
    }

    /// Whether the job may run a chunk of its vectors at a time, each chunk's elements of every vector it loads and
    /// stores: its rows cross nowhere — one head, every load a vector of one length from a slot, a tensor or a ring,
    /// every store that length from a vector register into a slot or a ring, and every instruction elementwise or a
    /// scalar's, none reducing, turning, rotating or coding across elements — so a chunk of its rows computes that chunk
    /// of its result. A ring's rows are a channel each, so a sequence's tokens still see each other's in order.
    pub fn chunked(&self) -> bool {
        let Some(len) = self.loads.first().map(|l| l.len) else { return false };
        self.heads == 1
            && self.loads.iter().all(|l| l.len == len && matches!(l.from, Source::Slot { .. } | Source::Tensor { .. } | Source::Ring { .. }))
            && self.puts.iter().all(|p| p.len == len && matches!(p.reg, Reg::V(_)) && matches!(p.to, Sink::Slot { .. } | Sink::Ring { .. }))
            && self.program.iter().all(|op| {
                matches!(
                    op,
                    Op::Vec { .. }
                        | Op::VecScalar { .. }
                        | Op::Map { .. }
                        | Op::Scalar { .. }
                        | Op::ScalarMap { .. }
                        | Op::Const { .. }
                        | Op::Scale { .. }
                        | Op::MulAdd { .. }
                        | Op::MulAddScalar { .. }
                )
            })
    }

    /// Whether the job's passes stand alone, so they may run in groups side by side (`logos_vector_side` with groups,
    /// each pass's registers its own): every vector load and store one length, every load a slot's or a tensor's, no
    /// ring row stored, no Hadamard or rotary instruction, every register a pass reads — an instruction's operand, a
    /// store's value — loaded or written earlier in that pass, and no pass loading store elements an earlier pass of its
    /// row stores (a group loads every pass before it stores any); a job of one chunk at a time is none. A pass's values
    /// are then its own whatever passes run beside it.
    pub fn grouped(&self) -> bool {
        let Some(len) = self.loads.first().map(|l| l.len) else { return false };
        let mut v: HashSet<VReg> = self.loads.iter().map(|l| l.reg).collect();
        let mut s: HashSet<SReg> = HashSet::new();
        let mut alone = true;
        for op in &self.program {
            let (rv, rs) = op.reads();
            alone &= rv.iter().all(|r| v.contains(r)) && rs.iter().all(|r| s.contains(r));
            v.extend(op.writes());
            s.extend(op.writes_scalar());
        }
        alone
            && !self.chunked()
            && self.loads.iter().all(|l| l.len == len && matches!(l.from, Source::Slot { .. } | Source::Tensor { .. }))
            && self.puts.iter().all(|p| match p.reg {
                Reg::V(r) => p.len == len && v.contains(&r),
                Reg::S(r) => s.contains(&r),
            })
            && self.puts.iter().all(|p| !matches!(p.to, Sink::Ring { .. }))
            && self.program.iter().all(|op| !matches!(op, Op::Hadamard { .. } | Op::Rotate { .. }))
            && self.puts.iter().all(|p| {
                self.loads.iter().all(|l| match (p.to, l.from) {
                    (Sink::Slot { slot, at, stride }, Source::Slot { slot: from, at: lat, stride: lstride }) if slot == from => (0..self.heads)
                        .all(|h| (h + 1..self.heads).all(|g| at + h * stride + p.len <= lat + g * lstride || lat + g * lstride + l.len <= at + h * stride)),
                    _ => true,
                })
            })
    }

    /// Whether load `i` brings every pass the same elements into a register nothing else of the job writes — a tensor's
    /// span at stride 0, its register no other load's and no instruction's — so a register holding it from the job's
    /// first pass holds it for every pass after.
    pub fn kept(&self, i: usize) -> bool {
        let l = &self.loads[i];
        matches!(l.from, Source::Tensor { stride: 0, .. })
            && self.loads.iter().enumerate().all(|(j, o)| j == i || o.reg != l.reg)
            && self.program.iter().all(|op| op.writes() != Some(l.reg))
    }
}

/// Each job's store slots, as masks of slot codes: those it reads, makes afresh and writes in part, and those live into
/// it, the step taken round until nothing changes.
struct Liveness {
    reads: Vec<u32>,
    fresh: Vec<u32>,
    partial: Vec<u32>,
    live_in: Vec<u32>,
}

impl Liveness {
    fn of(jobs: &[Job]) -> Liveness {
        let mask = |slots: &[Slot]| slots.iter().fold(0u32, |m, s| m | 1u32 << s.code());
        let reads: Vec<u32> = jobs.iter().map(|j| mask(&j.reads())).collect();
        let (fresh, partial): (Vec<u32>, Vec<u32>) = jobs
            .iter()
            .map(|j| {
                let (f, p) = j.writes();
                (mask(&f), mask(&p))
            })
            .unzip();
        let mut l = Liveness { reads, fresh, partial, live_in: vec![0u32; jobs.len()] };
        loop {
            let mut changed = false;
            for i in (0..jobs.len()).rev() {
                let held = l.reads[i] | l.partial[i] | (l.out(i) & !l.fresh[i]);
                if held != l.live_in[i] {
                    l.live_in[i] = held;
                    changed = true;
                }
            }
            if !changed {
                return l;
            }
        }
    }

    /// The slots live out of job `i`: into the next, or after the last into the step's first and the host's logits.
    fn out(&self, i: usize) -> u32 {
        if i + 1 < self.live_in.len() {
            self.live_in[i + 1]
        } else {
            self.live_in[0] | 1u32 << Slot::Logits.code()
        }
    }
}

/// How a matrix job's sums meet the elements of its output slot ([`fold`]): each written as it is, added to the
/// element it lands on (a residual connection, [`Command::Add`]'s), or multiplied by the SiLU of the element, the
/// gate's sum a gated FFN's product takes ([`Command::SwiGlu`]'s, the element its gate).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fold {
    Put,
    Add,
    SwiGlu,
}

impl Fold {
    /// The element a sum leaves where the slot held `old`: `sum`, `old + sum`, `silu(old)·sum`, each as the vector unit
    /// computes the add and the product.
    pub fn apply(self, fp: FpUnit, old: f32, sum: f32) -> f32 {
        match self {
            Fold::Put => sum,
            Fold::Add => fadd(old, sum),
            Fold::SwiGlu => fp.mul(silu(fp, old), sum),
        }
    }

    /// The fold's code in the matrix job's word.
    pub fn code(self) -> u64 {
        match self {
            Fold::Put => 0,
            Fold::Add => 1,
            Fold::SwiGlu => 2,
        }
    }

    /// The fold `code` names, or `None` for a code none does.
    pub fn of_code(code: u64) -> Option<Fold> {
        [Fold::Put, Fold::Add, Fold::SwiGlu].into_iter().find(|f| f.code() == code)
    }
}

/// A recurrence job's read-outs gated and normed where they are made ([`fold_delta_norms`]): each head RMS-normed by
/// tensor `weight` and gated by `silu` of the head's elements of `z`, `eps` the norm's epsilon — the gated norm
/// ([`Command::GatedNorm`]'s job) the read-outs took next, run as part of the job.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeltaNorm {
    pub weight: usize,
    pub z: Slot,
    pub eps: f32,
}

/// One job of the sequencer.
#[derive(Clone, Debug, PartialEq)]
pub enum Job {
    /// Each row's [`Slot::Residual`] its token's row of tensor `tensor`, dequantized.
    Embed { tensor: usize },
    /// The array: tensor `tensor` times each row of `from`, each sum folded into its element of `to` by `fold`.
    Matvec { tensor: usize, from: Slot, to: Slot, fold: Fold },
    Vector(VectorJob),
    /// The recurrence unit: layer `layer`'s memory through each row, each sequence's rows in order, its read-outs gated
    /// and normed by `norm` when it has one.
    Delta { layer: usize, query: Slot, key: Slot, value: Slot, value_at: usize, log_decay: Slot, decay: Slot, gate: Slot, to: Slot, norm: Option<DeltaNorm> },
    /// The attention unit: each row's `heads` query heads over layer `layer`'s cache of its sequence.
    Attend { layer: usize, query: Slot, heads: usize, scale: f32, to: Slot },
    /// Every sequence the step advances moves past its rows.
    Advance,
}

impl Job {
    /// The store slots the job reads, each once.
    pub fn reads(&self) -> Vec<Slot> {
        let mut slots = match self {
            Job::Embed { .. } | Job::Advance => Vec::new(),
            Job::Matvec { from, to, fold: Fold::Add | Fold::SwiGlu, .. } => vec![*from, *to],
            Job::Matvec { from, .. } => vec![*from],
            Job::Vector(v) => v.loads.iter().filter_map(|l| if let Source::Slot { slot, .. } = l.from { Some(slot) } else { None }).collect(),
            Job::Delta { query, key, value, log_decay, decay, gate, norm, .. } => {
                let mut v = vec![*query, *key, *value, *log_decay, *decay, *gate];
                v.extend(norm.map(|n| n.z));
                v
            }
            Job::Attend { query, .. } => vec![*query],
        };
        slots.sort_by_key(|s| s.code());
        slots.dedup();
        slots
    }

    /// The store slots the job writes: those it makes afresh, whole, and those it writes in part, the rest of each as
    /// the job found it.
    pub fn writes(&self) -> (Vec<Slot>, Vec<Slot>) {
        let (fresh, mut partial) = match self {
            Job::Embed { .. } => (vec![Slot::Residual], Vec::new()),
            Job::Matvec { to, .. } | Job::Delta { to, .. } | Job::Attend { to, .. } => (vec![*to], Vec::new()),
            Job::Vector(v) => {
                let fresh: Vec<Slot> = v.fresh.iter().map(|&(s, _)| s).collect();
                let partial = v
                    .puts
                    .iter()
                    .filter_map(|p| if let Sink::Slot { slot, .. } = p.to { Some(slot) } else { None })
                    .filter(|s| !fresh.contains(s))
                    .collect();
                (fresh, partial)
            }
            Job::Advance => (Vec::new(), Vec::new()),
        };
        partial.sort_by_key(|s| s.code());
        partial.dedup();
        (fresh, partial)
    }
}

/// The jobs of `program` over the tensors `layout` places, `eps` the norms' epsilon: one a command, its unit's, and for
/// a rotated cache's attention a second, the vector unit's rotation back.
pub fn lower(program: &Program, layout: &Layout, eps: f32) -> Vec<Job> {
    let index: HashMap<&str, usize> = layout.tensors.iter().enumerate().map(|(i, t)| (t.name.as_str(), i)).collect();
    let t = |name: &str| *index.get(name).unwrap_or_else(|| panic!("{name} is not in the weight image"));
    let elems = |name: &str| layout.tensors[t(name)].elems();
    let fp = layout.numerics.fp_unit;
    let ropes = program.ropes();
    // Each slot's width as the commands so far leave it.
    let mut width: HashMap<Slot, usize> = HashMap::new();
    let whole = |slot: Slot| Source::Slot { slot, at: 0, stride: 0 };
    let into = |slot: Slot| Sink::Slot { slot, at: 0, stride: 0 };
    let ld = |reg: VReg, from: Source, len: usize| Load { reg, from, len };
    let put = |reg: VReg, to: Sink, len: usize| Put { reg: Reg::V(reg), to, len };
    let mut jobs = Vec::with_capacity(program.commands.len());
    for c in &program.commands {
        let job = match c {
            Command::Embed { table } => {
                width.insert(Slot::Residual, layout.tensors[t(table)].dims[0] as usize);
                Job::Embed { tensor: t(table) }
            }
            Command::Norm { weight, from, to } => {
                let n = elems(weight);
                width.insert(*to, n);
                Job::Vector(VectorJob {
                    order: Order::Rows,
                    heads: 1,
                    fresh: vec![(*to, n)],
                    loads: vec![ld(0, whole(*from), n), ld(1, Source::Tensor { tensor: t(weight), at: 0, stride: 0 }, n)],
                    program: vector::rmsnorm(fp, 0, 1, 2, 3, n, eps),
                    puts: vec![put(2, into(*to), n)],
                })
            }
            Command::Matvec { tensor, from, to } => {
                let m = &layout.tensors[t(tensor)];
                let (in_f, out_f) = (m.dims[0] as usize, m.dims[1] as usize);
                width.insert(*to, out_f);
                if !m.streamed {
                    // A float matrix on the vector unit, a weight row a head: each row read once and dotted with every
                    // token-row.
                    Job::Vector(VectorJob {
                        order: Order::Heads,
                        heads: out_f,
                        fresh: vec![(*to, out_f)],
                        loads: vec![ld(0, Source::Tensor { tensor: t(tensor), at: 0, stride: in_f }, in_f), ld(1, whole(*from), in_f)],
                        program: vec![Op::Dot { a: 0, b: 1, d: 0 }],
                        puts: vec![Put { reg: Reg::S(0), to: Sink::Slot { slot: *to, at: 0, stride: 1 }, len: 1 }],
                    })
                } else {
                    Job::Matvec { tensor: t(tensor), from: *from, to: *to, fold: Fold::Put }
                }
            }
            Command::Add { from, to } => Job::Vector(add_job(*from, *to, width[to])),
            Command::SwiGlu { gate, up, to } => {
                let n = width[gate];
                width.insert(*to, n);
                Job::Vector(swiglu_job(*gate, *up, *to, n))
            }
            Command::Conv { layer, kernel, taps, from, to } => {
                let channels = width[from];
                width.insert(*to, channels);
                let taps = *taps;
                let ring: Vec<VReg> = (0..taps as u8 - 1).collect();
                let x = taps as u8 - 1;
                let tap: Vec<VReg> = (taps as u8..2 * taps as u8).collect();
                let (acc, out) = (2 * taps as u8, 2 * taps as u8 + 1);
                let mut loads: Vec<Load> = ring.iter().enumerate().map(|(age, &r)| ld(r, Source::Ring { layer: *layer, age }, channels)).collect();
                loads.push(ld(x, whole(*from), channels));
                loads.extend(tap.iter().enumerate().map(|(j, &r)| ld(r, Source::Tensor { tensor: t(kernel), at: j * channels, stride: 0 }, channels)));
                let mut puts = vec![put(out, into(*to), channels)];
                if taps > 1 {
                    puts.push(put(x, Sink::Ring { layer: *layer }, channels));
                }
                Job::Vector(VectorJob { heads: 1, order: Order::Rows, fresh: vec![(*to, channels)], loads, program: vector::conv(&ring, x, &tap, acc, out), puts })
            }
            Command::L2Heads { from, at, heads, width: w, to } => {
                width.insert(*to, heads * w);
                Job::Vector(VectorJob {
                    order: Order::Rows,
                    heads: *heads,
                    fresh: vec![(*to, heads * w)],
                    loads: vec![ld(0, Source::Slot { slot: *from, at: *at, stride: *w }, *w)],
                    program: vector::l2norm(0, 1, eps),
                    puts: vec![put(1, Sink::Slot { slot: *to, at: 0, stride: *w }, *w)],
                })
            }
            Command::Gates { alpha, beta, dt, a, log_decay, decay, gate } => {
                let n = width[alpha];
                for s in [log_decay, decay, gate] {
                    width.insert(*s, n);
                }
                Job::Vector(VectorJob {
                    order: Order::Rows,
                    heads: 1,
                    fresh: vec![(*log_decay, n), (*decay, n), (*gate, n)],
                    loads: vec![
                        ld(0, whole(*alpha), n),
                        ld(1, Source::Tensor { tensor: t(dt), at: 0, stride: 0 }, n),
                        ld(2, Source::Tensor { tensor: t(a), at: 0, stride: 0 }, n),
                        ld(3, whole(*beta), n),
                    ],
                    program: vector::delta_gates(0, 1, 2, 3, 4, 5, 6),
                    puts: vec![put(4, into(*log_decay), n), put(5, into(*decay), n), put(6, into(*gate), n)],
                })
            }
            Command::Delta { layer, query, key, value, value_at, log_decay, decay, gate, v_heads, width: w, to, .. } => {
                width.insert(*to, v_heads * w);
                Job::Delta {
                    layer: *layer,
                    query: *query,
                    key: *key,
                    value: *value,
                    value_at: *value_at,
                    log_decay: *log_decay,
                    decay: *decay,
                    gate: *gate,
                    to: *to,
                    norm: None,
                }
            }
            Command::GatedNorm { weight, from, z, width: w, to } => {
                let n = width[from];
                width.insert(*to, n);
                Job::Vector(gated_norm_job(fp, t(weight), *from, *z, *w, n, *to, eps))
            }
            Command::Split { from, heads, width: w, query, gate } => {
                width.insert(*query, heads * w);
                width.insert(*gate, heads * w);
                Job::Vector(VectorJob {
                    order: Order::Rows,
                    heads: *heads,
                    fresh: vec![(*query, heads * w), (*gate, heads * w)],
                    loads: vec![ld(0, Source::Slot { slot: *from, at: 0, stride: 2 * w }, *w), ld(1, Source::Slot { slot: *from, at: *w, stride: 2 * w }, *w)],
                    program: vec![],
                    puts: vec![put(0, Sink::Slot { slot: *query, at: 0, stride: *w }, *w), put(1, Sink::Slot { slot: *gate, at: 0, stride: *w }, *w)],
                })
            }
            Command::RmsHeads { weight, slot, width: w } => Job::Vector(VectorJob {
                order: Order::Rows,
                heads: width[slot] / w,
                fresh: vec![],
                loads: vec![ld(0, Source::Slot { slot: *slot, at: 0, stride: *w }, *w), ld(1, Source::Tensor { tensor: t(weight), at: 0, stride: 0 }, *w)],
                program: vector::rmsnorm(fp, 0, 1, 2, 3, *w, eps),
                puts: vec![put(2, Sink::Slot { slot: *slot, at: 0, stride: *w }, *w)],
            }),
            Command::Rotate { slot, width: w, rope } => {
                let table = ropes.iter().position(|r| *r == rope).expect("a rotary table the program turns by");
                let pairs = rope.dims / 2;
                Job::Vector(VectorJob {
                    order: Order::Rows,
                    heads: width[slot] / w,
                    fresh: vec![],
                    loads: vec![
                        ld(0, Source::Slot { slot: *slot, at: 0, stride: *w }, *w),
                        ld(1, Source::Rotary { table, sines: false }, pairs),
                        ld(2, Source::Rotary { table, sines: true }, pairs),
                    ],
                    program: vector::rotary(0, 1, 2, 3, rope.neox),
                    puts: vec![put(3, Sink::Slot { slot: *slot, at: 0, stride: *w }, *w)],
                })
            }
            Command::Hadamard { slot, width: w } => Job::Vector(VectorJob {
                order: Order::Rows,
                heads: width[slot] / w,
                fresh: vec![],
                loads: vec![ld(0, Source::Slot { slot: *slot, at: 0, stride: *w }, *w)],
                program: vec![Op::Hadamard { a: 0, d: 1 }],
                puts: vec![put(1, Sink::Slot { slot: *slot, at: 0, stride: *w }, *w)],
            }),
            Command::CacheAppend { layer, key, value, kv_heads, width: w, value_width: vw } => {
                let loads = vec![ld(0, Source::Slot { slot: *key, at: 0, stride: *w }, *w), ld(1, Source::Slot { slot: *value, at: 0, stride: *vw }, *vw)];
                let cache = |values: bool, part: CachePart| Sink::Cache { layer: *layer, values, part };
                let (program, puts) = if layout.numerics.cache.is_int8() {
                    let mut p = vector::cache_row(0, 2, 0);
                    p.extend(vector::cache_row(1, 3, 2));
                    let puts = vec![
                        put(2, cache(false, CachePart::Codes), *w),
                        Put { reg: Reg::S(0), to: cache(false, CachePart::Scale), len: 1 },
                        put(3, cache(true, CachePart::Codes), *vw),
                        Put { reg: Reg::S(2), to: cache(true, CachePart::Scale), len: 1 },
                    ];
                    (p, puts)
                } else {
                    (vec![], vec![put(0, cache(false, CachePart::Halves), *w), put(1, cache(true, CachePart::Halves), *vw)])
                };
                Job::Vector(VectorJob { heads: *kv_heads, order: Order::Rows, fresh: vec![], loads, program, puts })
            }
            Command::Attend { layer, query, heads, width: w, value_width, scale, to, .. } => {
                width.insert(*to, heads * value_width);
                if layout.numerics.cache == CacheFormat::Int8Hadamard {
                    // A rotated score is the dot of rotated rows, `w` times the plain one: the scale takes `2^-log2 w`;
                    // each head's weighted sum is rotated back, the transform then `2^-log2 value_width`.
                    let scale = ldexp(*scale, -(w.trailing_zeros() as i32));
                    jobs.push(Job::Attend { layer: *layer, query: *query, heads: *heads, scale, to: *to });
                    let head = Sink::Slot { slot: *to, at: 0, stride: *value_width };
                    Job::Vector(VectorJob {
                        order: Order::Rows,
                        heads: *heads,
                        fresh: vec![],
                        loads: vec![ld(0, Source::Slot { slot: *to, at: 0, stride: *value_width }, *value_width)],
                        program: vec![Op::Hadamard { a: 0, d: 1 }, Op::Scale { a: 1, n: -(value_width.trailing_zeros() as i8), d: 2 }],
                        puts: vec![put(2, head, *value_width)],
                    })
                } else {
                    Job::Attend { layer: *layer, query: *query, heads: *heads, scale: *scale, to: *to }
                }
            }
            Command::SigmoidGate { slot, gate } => {
                let n = width[slot];
                Job::Vector(VectorJob {
                    order: Order::Rows,
                    heads: 1,
                    fresh: vec![],
                    loads: vec![ld(0, whole(*slot), n), ld(1, whole(*gate), n)],
                    program: vector::sigmoid_gate(0, 1, 2, 3),
                    puts: vec![put(2, into(*slot), n)],
                })
            }
            Command::Advance => Job::Advance,
        };
        jobs.push(job);
    }
    jobs
}

/// [`Command::Add`]'s job: `from` added into `to`, `n` elements.
/// [`Command::GatedNorm`]'s job: each head of `width` elements of `from` (`n` a row) RMS-normed by tensor `weight` and
/// gated by `silu` of the head's elements of `z`, into `to`, `eps` the norm's epsilon.
#[allow(clippy::too_many_arguments)]
pub fn gated_norm_job(fp: FpUnit, weight: usize, from: Slot, z: Slot, width: usize, n: usize, to: Slot, eps: f32) -> VectorJob {
    let head = |slot: Slot| Source::Slot { slot, at: 0, stride: width };
    VectorJob {
        order: Order::Rows,
        heads: n / width,
        fresh: if to == from || to == z { vec![] } else { vec![(to, n)] },
        loads: vec![
            Load { reg: 0, from: head(from), len: width },
            Load { reg: 1, from: Source::Tensor { tensor: weight, at: 0, stride: 0 }, len: width },
            Load { reg: 2, from: head(z), len: width },
        ],
        program: vector::gdn_out(fp, 0, 1, 2, 3, 4, width, eps),
        puts: vec![Put { reg: Reg::V(3), to: Sink::Slot { slot: to, at: 0, stride: width }, len: width }],
    }
}

fn add_job(from: Slot, to: Slot, n: usize) -> VectorJob {
    let whole = |slot: Slot| Source::Slot { slot, at: 0, stride: 0 };
    VectorJob {
        order: Order::Rows,
        heads: 1,
        fresh: vec![],
        loads: vec![Load { reg: 0, from: whole(to), len: n }, Load { reg: 1, from: whole(from), len: n }],
        program: vec![Op::Vec { op: vector::Bin::Add, a: 0, b: 1, d: 2 }],
        puts: vec![Put { reg: Reg::V(2), to: Sink::Slot { slot: to, at: 0, stride: 0 }, len: n }],
    }
}

/// [`Command::SwiGlu`]'s job: `silu(gate)·up` into `to`, `n` elements.
fn swiglu_job(gate: Slot, up: Slot, to: Slot, n: usize) -> VectorJob {
    let whole = |slot: Slot| Source::Slot { slot, at: 0, stride: 0 };
    VectorJob {
        order: Order::Rows,
        heads: 1,
        fresh: vec![(to, n)],
        loads: vec![Load { reg: 0, from: whole(gate), len: n }, Load { reg: 1, from: whole(up), len: n }],
        program: vector::swiglu(0, 1, 2),
        puts: vec![Put { reg: Reg::V(2), to: Sink::Slot { slot: to, at: 0, stride: 0 }, len: n }],
    }
}

/// The step `jobs` with each matrix whose sums only a residual add or a gated FFN's product takes folding them into
/// that job's output slot ([`Fold`]): a matrix job and the add of its output into another slot become one job adding
/// its sums into that slot; a gate's and an up projection's matrix jobs and their product become the gate's sums into
/// the product's slot and the up projection's folded into them through the gate's SiLU. Jobs fold only where they stand
/// next to each other and the slots the step no longer writes are read by no job after
/// ([`StoreLayout::live_out`]), so every value the step computes is the one it computed, and those slots leave the
/// store.
pub fn fold(jobs: &[Job], layout: &Layout) -> Vec<Job> {
    let live = StoreLayout::live_out(jobs);
    let dead = |slot: Slot, after: usize| live[after] >> slot.code() & 1 == 0;
    let width = |tensor: usize| layout.tensors[tensor].dims[1] as usize;
    let slot_put = |v: &VectorJob| match v.puts.first().map(|p| p.to) {
        Some(Sink::Slot { slot, .. }) => Some(slot),
        _ => None,
    };
    let mut out = Vec::with_capacity(jobs.len());
    let mut i = 0;
    while i < jobs.len() {
        if let (Some(&Job::Matvec { tensor: tg, from: xg, to: g, fold: Fold::Put }), Some(&Job::Matvec { tensor: tu, from: xu, to: u, fold: Fold::Put }), Some(Job::Vector(v))) =
            (jobs.get(i), jobs.get(i + 1), jobs.get(i + 2))
        {
            let n = width(tu);
            if let Some(h) = slot_put(v) {
                let fits = *v == swiglu_job(g, u, h, n) && width(tg) == n && g != u && u != h && xu != g && h != xg && h != xu;
                if fits && (g == h || dead(g, i + 2)) && dead(u, i + 2) {
                    out.push(Job::Matvec { tensor: tg, from: xg, to: h, fold: Fold::Put });
                    out.push(Job::Matvec { tensor: tu, from: xu, to: h, fold: Fold::SwiGlu });
                    i += 3;
                    continue;
                }
            }
        }
        if let (Some(&Job::Matvec { tensor, from, to: o, fold: Fold::Put }), Some(Job::Vector(v))) = (jobs.get(i), jobs.get(i + 1)) {
            if let Some(r) = slot_put(v) {
                if *v == add_job(o, r, width(tensor)) && r != o && dead(o, i + 1) {
                    out.push(Job::Matvec { tensor, from, to: r, fold: Fold::Add });
                    i += 2;
                    continue;
                }
            }
        }
        out.push(jobs[i].clone());
        i += 1;
    }
    out
}

/// The step `jobs` with each recurrence job whose read-outs a gated norm in place takes next carrying that norm
/// ([`DeltaNorm`]): the two jobs become one, the recurrence's read-outs gated and normed where they are made. The norm's
/// gate is a slot other than the read-outs', so it is still the one the norm read. The machine runs a job with a norm
/// as the recurrence then the norm's own job, so every value the step computes is the one it computed.
pub fn fold_delta_norms(jobs: &[Job], layout: &Layout) -> Vec<Job> {
    let fp = layout.numerics.fp_unit;
    let mut out = Vec::with_capacity(jobs.len());
    let mut i = 0;
    while i < jobs.len() {
        if let (Some(Job::Delta { layer, to, norm: None, .. }), Some(Job::Vector(v))) = (jobs.get(i), jobs.get(i + 1)) {
            let BlockState::Delta { v_heads, width, .. } = layout.blocks[*layer] else { unreachable!("a delta block") };
            let parts = match (v.loads.get(1), v.loads.get(2), v.program.get(3)) {
                (Some(Load { from: Source::Tensor { tensor, .. }, .. }), Some(Load { from: Source::Slot { slot, .. }, .. }), Some(Op::Const { value, .. })) => {
                    Some(DeltaNorm { weight: *tensor, z: *slot, eps: *value })
                }
                _ => None,
            };
            if let Some(n) = parts {
                if n.z != *to && *v == gated_norm_job(fp, n.weight, *to, n.z, width, v_heads * width, *to, n.eps) {
                    let mut job = jobs[i].clone();
                    if let Job::Delta { norm, .. } = &mut job {
                        *norm = Some(n);
                    }
                    out.push(job);
                    i += 2;
                    continue;
                }
            }
        }
        out.push(jobs[i].clone());
        i += 1;
    }
    out
}

// ── The machine ─────────────────────────────────────────────────────────────────────────────────────

/// One row of a step's row table: its sequence, its position and token, and where its position's row sits in its
/// sequence's conv rings, `position mod (taps − 1)`, the host's division so the engine never divides.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row {
    pub sequence: usize,
    pub position: usize,
    pub token: u32,
    pub ring: usize,
}

/// The token-row store: each slot's vector for every row of the step, in the step's row order.
pub type Store = HashMap<Slot, Vec<Vec<f32>>>;

/// The machine before one of a step's jobs runs: the job's index (the job count once the last has run), the step's
/// rows, the store and the card's memory.
pub struct Trace<'a> {
    pub job: usize,
    pub rows: &'a [Row],
    pub store: &'a Store,
    pub memory: &'a [u8],
}

/// The orders [`StoreLayout::of`] tries past the widest first, each the one before shuffled, before it takes the shortest
/// row it laid.
pub const STORE_ORDERS: usize = 1 << 12;

/// The token-row store as the engine holds it: beats of [`LANES`] f32 elements, slot `s`'s row `r` from beat
/// `base + r·beats` of its entry, every slot wide enough for the widest vector the step's jobs write to it, and two
/// slots over the same beats only when no point of the step holds both. A greedy store ([`StoreLayout::greedy`]) keeps
/// each row's logits as their arg-max.
#[derive(Clone, Debug, PartialEq)]
pub struct StoreLayout {
    pub rows: usize,
    /// Per slot code: the slot's first beat and its beats a row.
    pub slots: Vec<(u32, u32)>,
    pub beats: u32,
    /// Whether the logits slot holds each row's arg-max in place of its logits.
    pub greedy: bool,
}

impl StoreLayout {
    /// The store `jobs` write over `rows` rows, each slot's width the widest any job writes to it, each slot at the
    /// lowest beat no slot live beside it holds ([`StoreLayout::live_together`]), the widest first — or, where that
    /// leaves a row longer than the most beats a point of the step holds together ([`StoreLayout::widest`]), in the first
    /// order of a fixed sequence of them that lays the row that short, the shortest of them all if none does.
    pub fn of(jobs: &[Job], layout: &Layout, rows: usize) -> StoreLayout {
        StoreLayout::laid(jobs, layout, rows, false)
    }

    /// [`StoreLayout::of`] with the head's read-out never in the store: the logits slot is a beat a row, its first lane
    /// the index [`numerics::argmax`](super::numerics::argmax) picks of the row's logits and its second that logit's
    /// bits, so the store holds what a greedy step reads back however wide the vocabulary.
    pub fn greedy(jobs: &[Job], layout: &Layout, rows: usize) -> StoreLayout {
        StoreLayout::laid(jobs, layout, rows, true)
    }

    fn laid(jobs: &[Job], layout: &Layout, rows: usize, greedy: bool) -> StoreLayout {
        let mut width = vec![0usize; Slot::ALL.len()];
        let mut widen = |slot: Slot, w: usize| width[slot.code() as usize] = width[slot.code() as usize].max(w);
        for job in jobs {
            match job {
                Job::Embed { tensor } => widen(Slot::Residual, layout.tensors[*tensor].dims[0] as usize),
                Job::Matvec { tensor, to, .. } => widen(*to, layout.tensors[*tensor].dims[1] as usize),
                Job::Vector(v) => v.fresh.iter().for_each(|&(s, w)| widen(s, w)),
                Job::Delta { layer, to, .. } => {
                    let BlockState::Delta { v_heads, width, .. } = layout.blocks[*layer] else { unreachable!("a delta block") };
                    widen(*to, v_heads * width);
                }
                Job::Attend { layer, heads, to, .. } => {
                    let BlockState::Attention { value_width, .. } = layout.blocks[*layer] else { unreachable!("an attention block") };
                    widen(*to, heads * value_width);
                }
                Job::Advance => {}
            }
        }
        if greedy {
            width[Slot::Logits.code() as usize] = 2;
        }
        let beats: Vec<u32> = width.iter().map(|w| w.div_ceil(LANES) as u32).collect();
        let clash = StoreLayout::live_together(jobs);
        let first_fit = |order: &[usize]| -> (Vec<(u32, u32)>, u32) {
            let mut slots = vec![(0u32, 0u32); beats.len()];
            let mut placed: Vec<(usize, u32, u32)> = Vec::new();
            for &c in order {
                let span = beats[c] * rows as u32;
                let mut at = 0u32;
                while let Some(&(_, _, end)) = placed.iter().find(|&&(o, b, e)| clash[c] >> o & 1 == 1 && b < at + span && at < e) {
                    at = end;
                }
                placed.push((c, at, at + span));
                slots[c] = (at, beats[c]);
            }
            (slots, placed.iter().map(|&(_, _, e)| e).max().unwrap_or(0))
        };
        let mut order: Vec<usize> = (0..beats.len()).filter(|&c| beats[c] > 0).collect();
        order.sort_by_key(|&c| (std::cmp::Reverse(beats[c]), c));
        let least = StoreLayout::widest_of(jobs, &beats) * rows as u32;
        let mut best = first_fit(&order);
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..STORE_ORDERS {
            if best.1 <= least {
                break;
            }
            for i in (1..order.len()).rev() {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                order.swap(i, (seed % (i as u64 + 1)) as usize);
            }
            let next = first_fit(&order);
            if next.1 < best.1 {
                best = next;
            }
        }
        StoreLayout { rows, slots: best.0, beats: best.1, greedy }
    }

    /// The most beats a row of the store holds at any point of the step `jobs`, the store laid for: the widest set of its
    /// slots a job reads, writes or holds live across it ([`StoreLayout::live_together`]) — no layout's row is shorter.
    pub fn widest(&self, jobs: &[Job]) -> u32 {
        let beats: Vec<u32> = self.slots.iter().map(|&(_, b)| b).collect();
        StoreLayout::widest_of(jobs, &beats)
    }

    fn widest_of(jobs: &[Job], beats: &[u32]) -> u32 {
        let l = Liveness::of(jobs);
        (0..jobs.len())
            .map(|i| {
                let held = l.live_in[i] | l.out(i) | l.reads[i] | l.fresh[i] | l.partial[i];
                (0..beats.len()).filter(|&c| held >> c & 1 == 1).map(|c| beats[c]).sum::<u32>()
            })
            .max()
            .unwrap_or(0)
    }

    /// For each slot code, the slots some point of the step `jobs` holds beside it, as a mask of slot codes: those a
    /// job reads or writes, and those live across it ([`StoreLayout::live_out`]).
    pub fn live_together(jobs: &[Job]) -> Vec<u32> {
        let l = Liveness::of(jobs);
        let mut clash = vec![0u32; Slot::ALL.len()];
        for i in 0..jobs.len() {
            let held = l.live_in[i] | l.out(i) | l.reads[i] | l.fresh[i] | l.partial[i];
            for (c, m) in clash.iter_mut().enumerate() {
                if held >> c & 1 == 1 {
                    *m |= held;
                }
            }
        }
        clash
    }

    /// For each job of the step `jobs`, the slots live out of it, as a mask of slot codes. A slot is live from a job
    /// that makes it afresh to the last job that reads it before it is made again, the step taken round (the next step
    /// follows it, and the host reads the logits after its last job); a job that writes a slot in part leaves it live.
    pub fn live_out(jobs: &[Job]) -> Vec<u32> {
        let l = Liveness::of(jobs);
        (0..jobs.len()).map(|i| l.out(i)).collect()
    }

    /// Where slot `slot`'s row `row` starts.
    pub fn beat(&self, slot: Slot, row: usize) -> u32 {
        let (base, beats) = self.slots[slot.code() as usize];
        base + row as u32 * beats
    }

    /// The store's beats, each element its f32 bits, an element no vector holds zero; a greedy store's logits slot
    /// each row's arg-max and that logit.
    pub fn image(&self, store: &Store) -> Vec<[u32; LANES]> {
        let mut out = vec![[0u32; LANES]; self.beats as usize];
        for (slot, rows) in store {
            for (r, v) in rows.iter().enumerate() {
                let at = self.beat(*slot, r) as usize;
                if self.greedy && *slot == Slot::Logits {
                    let best = super::numerics::argmax(v);
                    out[at][0] = best as u32;
                    out[at][1] = v.get(best).map_or(0, |x| x.to_bits());
                    continue;
                }
                for (i, x) in v.iter().enumerate() {
                    out[at + i / LANES][i % LANES] = x.to_bits();
                }
            }
        }
        out
    }
}

/// A checkpoint on the machine: its layout, its jobs, and the card's memory they run over.
pub struct Machine {
    pub spec: ModelSpec,
    /// The cut of the model's one program the machine's step was read off.
    pub identity: crate::model::program::Identity,
    pub layout: Layout,
    pub jobs: Vec<Job>,
    memory: Vec<u8>,
    /// Each region's sequence's position, or `None` for a region no sequence holds.
    positions: Vec<Option<usize>>,
}

impl Machine {
    /// `spec` at `numerics` with room for `sequences` sequences of `ctx` positions each, its weight image and rotary
    /// tables written from `source`.
    pub fn new(spec: ModelSpec, mut source: impl WeightSource, numerics: EngineNumerics, sequences: usize, ctx: usize) -> R<Machine> {
        runs(&spec, numerics)?;
        let super::program::Lowered { program, identity } = super::program::lower(&spec, numerics, ctx)?;
        let layout = Layout::of(&spec, numerics, &program, &source, sequences, ctx)?;
        let jobs = lower(&program, &layout, spec.rms_eps as f32);
        let mut memory = vec![0u8; layout.bytes as usize];
        for t in &layout.tensors {
            let rows = t.elems() / t.dims[0].max(1) as usize;
            let bytes = source.row_bytes(&t.name, 0, rows)?;
            let at = t.at as usize;
            match t.taps {
                None => memory[at..at + bytes.len()].copy_from_slice(&bytes),
                Some(taps) => {
                    // Each channel's taps together in the checkpoint, each tap's channels together in the image.
                    let channels = t.elems() / taps;
                    for (i, word) in bytes.chunks_exact(4).enumerate() {
                        let (ch, j) = (i / taps, i % taps);
                        let to = at + 4 * (j * channels + ch);
                        memory[to..to + 4].copy_from_slice(word);
                    }
                }
            }
        }
        for (rope, at) in &layout.rotary {
            let pairs = rope.dims / 2;
            let factors = rotary_factors(rope, &mut source)?;
            for p in 0..layout.ctx {
                let base = *at as usize + p * pairs * 8;
                for i in 0..pairs {
                    let (c, s) = rotation(rope, i, p, factors.as_deref());
                    memory[base + 4 * i..base + 4 * i + 4].copy_from_slice(&(c as f32).to_le_bytes());
                    memory[base + 4 * (pairs + i)..base + 4 * (pairs + i) + 4].copy_from_slice(&(s as f32).to_le_bytes());
                }
            }
        }
        let positions = vec![None; layout.sequences];
        Ok(Machine { spec, identity, layout, jobs, memory, positions })
    }

    /// The machine with its step's residual adds and gated products folded into the matrix jobs that alone feed them
    /// ([`fold`]): every value the step computes as it was, in fewer jobs over fewer slots.
    pub fn folded(mut self) -> Machine {
        self.jobs = fold(&self.jobs, &self.layout);
        self
    }

    /// The machine with each recurrence job's gated norm folded into it ([`fold_delta_norms`]): every value the step
    /// computes as it was, the norm made where the read-outs are.
    pub fn delta_normed(mut self) -> Machine {
        self.jobs = fold_delta_norms(&self.jobs, &self.layout);
        self
    }

    /// The card's memory.
    pub fn memory(&self) -> &[u8] {
        &self.memory
    }

    /// A fresh sequence at position 0 in the first region no sequence holds, the region cleared; its handle, the
    /// region's index.
    pub fn sequence(&mut self) -> R<usize> {
        let s = self.positions.iter().position(Option::is_none).ok_or(EngineError::NotYet("more sequences than the machine's regions"))?;
        let at = self.layout.region_of(s) as usize;
        self.memory[at..at + self.layout.region as usize].fill(0);
        self.positions[s] = Some(0);
        Ok(s)
    }

    /// The sequence `sequence` ends: its region is free for the next.
    pub fn release(&mut self, sequence: usize) {
        self.positions[sequence] = None;
    }

    pub fn position(&self, sequence: usize) -> usize {
        self.positions[sequence].expect("a sequence the machine holds")
    }

    /// Run each listed sequence's tokens at its next positions; returns, per entry, the logits after each token, as
    /// [`super::model::EngineModel::step`] does.
    pub fn step(&mut self, entries: &[(usize, Vec<u32>)]) -> R<Vec<Vec<Vec<f32>>>> {
        self.step_traced(entries, &mut |_| {})
    }

    /// [`step`](Self::step), with `trace` shown the machine before each job and after the last.
    pub fn step_traced(&mut self, entries: &[(usize, Vec<u32>)], trace: &mut dyn FnMut(Trace)) -> R<Vec<Vec<Vec<f32>>>> {
        let mut seen = std::collections::HashSet::new();
        for (s, toks) in entries {
            if !seen.insert(*s) {
                return Err(EngineError::NotYet("one sequence twice in a step"));
            }
            let at = self.positions.get(*s).copied().flatten().ok_or(EngineError::NoSequence(*s))?;
            if at + toks.len() > self.layout.ctx {
                return Err(EngineError::PastContext { sequence: *s, position: at + toks.len() });
            }
        }
        let ring = self.ring_rows();
        let mut rows = Vec::new();
        let mut entry_of = Vec::new();
        for (e, (s, toks)) in entries.iter().enumerate() {
            for (i, &token) in toks.iter().enumerate() {
                let position = self.position(*s) + i;
                rows.push(Row { sequence: *s, position, token, ring: position % ring });
                entry_of.push(e);
            }
        }
        let mut store: Store = HashMap::new();
        let jobs = std::mem::take(&mut self.jobs);
        // The store as the engine's holds it: a slot gone once it is dead, its beats another's.
        let live = StoreLayout::live_out(&jobs);
        let mut result = Ok(());
        for (j, job) in jobs.iter().enumerate() {
            trace(Trace { job: j, rows: &rows, store: &store, memory: &self.memory });
            result = self.run(job, &rows, &mut store);
            if result.is_err() {
                break;
            }
            store.retain(|s, _| live[j] >> s.code() & 1 == 1);
        }
        self.jobs = jobs;
        result?;
        trace(Trace { job: self.jobs.len(), rows: &rows, store: &store, memory: &self.memory });
        let mut out: Vec<Vec<Vec<f32>>> = entries.iter().map(|_| Vec::new()).collect();
        for (logits, e) in store.remove(&Slot::Logits).expect("the head's logits").into_iter().zip(entry_of) {
            out[e].push(logits);
        }
        Ok(out)
    }

    fn run(&mut self, job: &Job, rows: &[Row], store: &mut Store) -> R<()> {
        match job {
            Job::Embed { tensor } => {
                let t = &self.layout.tensors[*tensor];
                let rb = t.row_bytes() as usize;
                let out = rows
                    .iter()
                    .map(|r| {
                        let at = t.at as usize + r.token as usize * rb;
                        let mut v = Vec::new();
                        t.ty.dequantize(&self.memory[at..at + rb], &mut v);
                        v
                    })
                    .collect();
                store.insert(Slot::Residual, out);
            }
            Job::Matvec { tensor, from, to, fold } => {
                let sums = self.matvec(*tensor, &store[from])?;
                let out = match fold {
                    Fold::Put => sums,
                    _ => {
                        let fp = self.layout.numerics.fp_unit;
                        let old = &store[to];
                        sums.iter().zip(old).map(|(s, o)| s.iter().zip(o).map(|(&s, &o)| fold.apply(fp, o, s)).collect()).collect()
                    }
                };
                store.insert(*to, out);
            }
            Job::Vector(v) => self.vector(v, rows, store),
            Job::Delta { layer, query, key, value, value_at, log_decay, decay, gate, to, norm } => {
                let reads = self.delta(*layer, rows, store, [*query, *key, *value, *log_decay, *decay, *gate], *value_at);
                store.insert(*to, reads);
                if let Some(n) = norm {
                    let BlockState::Delta { v_heads, width, .. } = self.layout.blocks[*layer] else { unreachable!("a delta block") };
                    let job = gated_norm_job(self.layout.numerics.fp_unit, n.weight, *to, n.z, width, v_heads * width, *to, n.eps);
                    self.vector(&job, rows, store);
                }
            }
            Job::Attend { layer, query, heads, scale, to } => {
                let out = self.attend(*layer, rows, &store[query], *heads, *scale);
                store.insert(*to, out);
            }
            Job::Advance => {
                for r in rows {
                    *self.positions[r.sequence].as_mut().expect("a sequence the machine holds") += 1;
                }
            }
        }
        Ok(())
    }

    /// Tensor `tensor`'s elements in f32.
    fn tensor_values(&self, tensor: usize) -> Vec<f32> {
        let t = &self.layout.tensors[tensor];
        let mut v = Vec::new();
        t.ty.dequantize(&self.memory[t.at as usize..(t.at + t.bytes) as usize], &mut v);
        v
    }

    /// The block-format matrix `tensor` times each of `xs`, as the array computes it.
    fn matvec(&self, tensor: usize, xs: &[Vec<f32>]) -> R<Vec<Vec<f32>>> {
        let t = &self.layout.tensors[tensor];
        let [in_f, out_f] = t.dims[..] else { return Err(EngineError::NotYet("a projection that is not a matrix")) };
        let (in_f, out_f) = (in_f as usize, out_f as usize);
        let rb = t.row_bytes() as usize;
        let row = |r: usize| &self.memory[t.at as usize + r * rb..t.at as usize + (r + 1) * rb];
        let n = self.layout.numerics;
        if !reads(t.ty) {
            return Err(EngineError::Undecoded { tensor: t.name.clone(), ty: t.ty });
        }
        let acts: Vec<ActRow> = match n.scaling {
            ActScaling::Sub32 => {
                if in_f % GROUP != 0 {
                    return Err(EngineError::NotYet("a matrix row that is not a whole number of 256-element groups"));
                }
                xs.iter().map(|x| quantize(x, n.activations)).collect()
            }
            ActScaling::PerBlock => xs.iter().map(|x| quantize_blocks(x, n.activations, t.ty.block_len())).collect(),
        };
        let coded: Vec<CodedRow> = (0..out_f)
            .map(|r| {
                CodedRow::decode(t.ty, row(r)).expect("a format the array reads").map_err(|_| EngineError::Unreadable(format!("{} row {r}", t.name)))
            })
            .collect::<R<_>>()?;
        Ok(acts.iter().map(|a| coded.iter().map(|c| super::numerics::dot(c, a)).collect()).collect())
    }


    fn vector(&mut self, job: &VectorJob, rows: &[Row], store: &mut Store) {
        let fp = self.layout.numerics.fp_unit;
        for &(slot, w) in &job.fresh {
            store.insert(slot, vec![vec![0f32; w]; rows.len()]);
        }
        let tensors: HashMap<usize, Vec<f32>> = job
            .loads
            .iter()
            .filter_map(|l| match l.from {
                Source::Tensor { tensor, .. } => Some(tensor),
                _ => None,
            })
            .map(|t| (t, self.tensor_values(t)))
            .collect();
        let passes: Vec<(usize, usize)> = match job.order {
            Order::Rows => (0..rows.len()).flat_map(|k| (0..job.heads).map(move |h| (k, h))).collect(),
            Order::Heads => (0..job.heads).flat_map(|h| (0..rows.len()).map(move |k| (k, h))).collect(),
        };
        // The registers hold from pass to pass, as the vector unit's do: a kept load is loaded on the job's first pass
        // alone.
        let kept: Vec<bool> = (0..job.loads.len()).map(|i| job.kept(i)).collect();
        let mut regs = Regs::new(VREGS, SREGS);
        for (pass, (k, head)) in passes.into_iter().enumerate() {
            let row = &rows[k];
            {
                for (l, _) in job.loads.iter().zip(&kept).filter(|&(_, &held)| pass == 0 || !held) {
                    regs.v[l.reg as usize] = match l.from {
                        Source::Slot { slot, at, stride } => {
                            let from = at + head * stride;
                            store[&slot][k][from..from + l.len].to_vec()
                        }
                        Source::Tensor { tensor, at, stride } => {
                            let from = at + head * stride;
                            tensors[&tensor][from..from + l.len].to_vec()
                        }
                        Source::Rotary { table, sines } => {
                            self.read_f32(self.layout.rotary[table].1 as usize + (row.position * 2 + sines as usize) * l.len * 4, l.len)
                        }
                        Source::Ring { layer, age } => self.read_state(self.ring_at(row, layer, age), l.len),
                    };
                }
                vector::run(fp, &job.program, &mut regs);
                for p in &job.puts {
                    let v: Vec<f32> = match p.reg {
                        Reg::V(r) => regs.v[r as usize][..p.len].to_vec(),
                        Reg::S(s) => vec![regs.s[s as usize]],
                    };
                    match p.to {
                        Sink::Slot { slot, at, stride } => {
                            let from = at + head * stride;
                            store.get_mut(&slot).unwrap_or_else(|| panic!("{slot:?} stored before the program writes it"))[k][from..from + p.len]
                                .copy_from_slice(&v);
                        }
                        // The row's own ring row takes the place of the oldest, which is the row `taps − 1` back.
                        Sink::Ring { layer } => self.write_state(self.ring_at(row, layer, 0), &v),
                        Sink::Cache { layer, values, part } => match part {
                            CachePart::Halves => {
                                let at = self.cache_at(row.sequence, layer, row.position, head, values);
                                for (i, x) in v.iter().enumerate() {
                                    self.memory[at + 2 * i..at + 2 * i + 2].copy_from_slice(&f32_to_f16(*x).to_le_bytes());
                                }
                            }
                            CachePart::Codes => {
                                let at = self.cache_at(row.sequence, layer, row.position, head, values);
                                for (i, x) in v.iter().enumerate() {
                                    self.memory[at + i] = *x as i8 as u8;
                                }
                            }
                            CachePart::Scale => {
                                let at = self.scale_at(row.sequence, layer, row.position, head, values);
                                self.memory[at..at + 2].copy_from_slice(&f32_to_f16(v[0]).to_le_bytes());
                            }
                        },
                    }
                }
            }
        }
    }

    fn read_f32(&self, at: usize, n: usize) -> Vec<f32> {
        (0..n).map(|i| f32::from_le_bytes(self.memory[at + 4 * i..at + 4 * i + 4].try_into().expect("four bytes"))).collect()
    }

    /// `n` elements of a recurrent state at `at`, as the state stores them.
    fn read_state(&self, at: usize, n: usize) -> Vec<f32> {
        match self.layout.numerics.state {
            StateFormat::F32 => self.read_f32(at, n),
            StateFormat::F16 => (0..n).map(|i| f16_to_f32(u16::from_le_bytes([self.memory[at + 2 * i], self.memory[at + 2 * i + 1]]))).collect(),
        }
    }

    /// `values` stored as the state stores them at `at`.
    fn write_state(&mut self, at: usize, values: &[f32]) {
        match self.layout.numerics.state {
            StateFormat::F32 => {
                for (i, v) in values.iter().enumerate() {
                    self.memory[at + 4 * i..at + 4 * i + 4].copy_from_slice(&v.to_le_bytes());
                }
            }
            StateFormat::F16 => {
                for (i, v) in values.iter().enumerate() {
                    self.memory[at + 2 * i..at + 2 * i + 2].copy_from_slice(&f32_to_f16(*v).to_le_bytes());
                }
            }
        }
    }

    /// The rows every conv ring holds, `taps − 1`, the same for every block (1 for a model of none).
    fn ring_rows(&self) -> usize {
        let mut rows = self.layout.blocks.iter().filter_map(|b| match b {
            BlockState::Delta { taps, .. } => Some(taps.saturating_sub(1).max(1)),
            _ => None,
        });
        let first = rows.next().unwrap_or(1);
        assert!(rows.all(|r| r == first), "every conv ring of one length");
        first
    }

    /// Where `row`'s sequence keeps layer `layer`'s ring row `age` positions past the oldest: the row of position
    /// `p − (taps − 1) + age`, at its index modulo `taps − 1`, which is `row.ring + age` less `taps − 1` once it passes.
    fn ring_at(&self, row: &Row, layer: usize, age: usize) -> usize {
        let BlockState::Delta { ring, channels, taps, .. } = self.layout.blocks[layer] else { unreachable!("a ring in a delta block") };
        let index = (row.ring + age) % (taps - 1);
        (self.layout.region_of(row.sequence) + ring) as usize + index * channels * state_bytes(self.layout.numerics.state) as usize
    }

    /// Where sequence `sequence` keeps layer `layer`'s cached head `head` of position `position`: its keys, or its
    /// values.
    fn cache_at(&self, sequence: usize, layer: usize, position: usize, head: usize, values: bool) -> usize {
        let BlockState::Attention { keys, values: vs, kv_heads, width, value_width, .. } = self.layout.blocks[layer] else {
            unreachable!("a cache in an attention block")
        };
        let (base, w) = if values { (vs, value_width) } else { (keys, width) };
        let row = cache_row_bytes(self.layout.numerics.cache, w) as usize;
        (self.layout.region_of(sequence) + base) as usize + (position * kv_heads + head) * row
    }

    /// A cached head row as the vector units read it: binary16 elements, or int8 codes times their scale.
    fn cache_values(&self, sequence: usize, layer: usize, position: usize, head: usize, values: bool, width: usize) -> Vec<f32> {
        let at = self.cache_at(sequence, layer, position, head, values);
        match self.layout.numerics.cache {
            CacheFormat::F16 => (0..width).map(|i| f16_to_f32(u16::from_le_bytes([self.memory[at + 2 * i], self.memory[at + 2 * i + 1]]))).collect(),
            CacheFormat::Int8 | CacheFormat::Int8Hadamard => {
                let (scale, codes) = self.cache_int8(sequence, layer, position, head, values, width);
                let s = f16_to_f32(scale);
                codes.iter().map(|&c| self.layout.numerics.fp_unit.mul(c as f32, s)).collect()
            }
        }
    }

    /// Where sequence `sequence` keeps the scale of layer `layer`'s int8 cached head `head` of position `position`.
    fn scale_at(&self, sequence: usize, layer: usize, position: usize, head: usize, values: bool) -> usize {
        let BlockState::Attention { key_scales: Some(ks), value_scales: Some(vs), kv_heads, .. } = self.layout.blocks[layer] else {
            unreachable!("an int8 cache's scales")
        };
        (self.layout.region_of(sequence) + if values { vs } else { ks }) as usize + (position * kv_heads + head) * 2
    }

    /// A cached int8 head row as the array reads it: its scale's bits and its codes.
    fn cache_int8(&self, sequence: usize, layer: usize, position: usize, head: usize, values: bool, width: usize) -> (u16, Vec<i8>) {
        let (at, s) = (self.cache_at(sequence, layer, position, head, values), self.scale_at(sequence, layer, position, head, values));
        (u16::from_le_bytes([self.memory[s], self.memory[s + 1]]), self.memory[at..at + width].iter().map(|&b| b as i8).collect())
    }

    /// A Gated-DeltaNet block's memory through each row, each sequence's rows in order, every value row read from and
    /// written back to the sequence's region as the state stores it; with a lazy flush, the row's token logged after
    /// the log's tokens, the query folded through the log, and the rows its [`Replay`] names replayed into the memory
    /// and written back.
    fn delta(&mut self, layer: usize, rows: &[Row], store: &Store, slots: [Slot; 6], value_at: usize) -> Vec<Vec<f32>> {
        let BlockState::Delta { memory, log, entry, k_heads: nk, v_heads: nv, width: hd, .. } = self.layout.blocks[layer] else {
            unreachable!("a delta block")
        };
        let fp = self.layout.numerics.fp_unit;
        let [query, key, value, log_decay, decay, gate] = slots.map(|s| &store[&s]);
        let scale = (1.0 / (hd as f64).sqrt()) as f32;
        let n = self.layout.numerics;
        let eb = state_bytes(n.state) as usize;
        let round = move |v: f32| match n.state {
            StateFormat::F32 => v,
            StateFormat::F16 => f16_to_f32(f32_to_f16(v)),
        };
        let mut reads = Vec::with_capacity(rows.len());
        for (k, row) in rows.iter().enumerate() {
            let region = self.layout.region_of(row.sequence) as usize;
            let mem = region + memory as usize;
            let (q, kk, v) = (&query[k], &key[k], &value[k][value_at..]);
            let (log_decays, decays, betas) = (&log_decay[k], &decay[k], &gate[k]);
            let mut out = vec![0f32; nv * hd];
            let row_at = |h: usize, r: usize| mem + (h * hd + r) * hd * eb;
            match n.flush {
                StateFlush::EveryPass => {
                    for h in 0..nv {
                        let g = h % nk;
                        let (hq, hk) = (&q[g * hd..(g + 1) * hd], &kk[g * hd..(g + 1) * hd]);
                        let kq = fdot(fp, hk, hq);
                        for r in 0..hd {
                            let mut m = self.read_state(row_at(h, r), hd);
                            out[h * hd + r] = gdn_row(fp, &mut m, hk, hq, decays[h], betas[h], v[h * hd + r], kq, scale, round);
                            self.write_state(row_at(h, r), &m);
                        }
                    }
                }
                StateFlush::Every { .. } | StateFlush::Trailing { .. } => {
                    let lg = region + log.expect("a lazily flushed memory's log") as usize;
                    let f = LogFields::of(nk, nv, hd);
                    let capacity = Replay::capacity(n.flush);
                    let word = |m: &[u8], i: usize| u32::from_le_bytes(m[lg + 4 * i..lg + 4 * i + 4].try_into().expect("four bytes")) as usize;
                    let (count, oldest) = (word(&self.memory, 0), word(&self.memory, 1));
                    let entry_at = |i: usize| lg + LINE as usize + (oldest + i) % capacity * entry as usize;
                    let mut logged = vec![0f32; f.elems];
                    logged[..nk * hd].copy_from_slice(kk);
                    logged[f.values..f.values + nv * hd].copy_from_slice(&v[..nv * hd]);
                    logged[f.log_decays..f.log_decays + nv].copy_from_slice(log_decays);
                    logged[f.gates..f.gates + nv].copy_from_slice(betas);
                    self.write_state(entry_at(count), &logged);
                    let count = count + 1;
                    let replay = Replay::at(n.flush, hd, count);
                    let entries: Vec<Vec<f32>> = (0..count).map(|i| self.read_state(entry_at(i), f.elems)).collect();
                    for h in 0..nv {
                        let g = h % nk;
                        let head: Vec<Logged> = entries
                            .iter()
                            .map(|e| Logged {
                                k: &e[g * hd..(g + 1) * hd],
                                v: &e[f.values + h * hd..f.values + (h + 1) * hd],
                                log_decay: e[f.log_decays + h],
                                beta: e[f.gates + h],
                            })
                            .collect();
                        let folds = gdn_fold_trailing(fp, &q[g * hd..(g + 1) * hd], &head, hd, replay.older);
                        for r in 0..hd {
                            let mut m = self.read_state(row_at(h, r), hd);
                            let (y, carried) = &folds[usize::from(r >= replay.replayed)];
                            out[h * hd + r] = gdn_read(fp, &m, y, carried[r], scale);
                            if replay.rows.contains(&r) {
                                gdn_flush(fp, &mut m, &head[..replay.by], r, round);
                                self.write_state(row_at(h, r), &m);
                            }
                        }
                    }
                    let (count, oldest) = (count - replay.drop, (oldest + replay.drop) % capacity);
                    self.memory[lg..lg + 4].copy_from_slice(&(count as u32).to_le_bytes());
                    self.memory[lg + 4..lg + 8].copy_from_slice(&(oldest as u32).to_le_bytes());
                }
            }
            reads.push(out);
        }
        reads
    }

    /// Each row's `heads` query heads over layer `layer`'s cache of its sequence to its own position, the heads in
    /// groups over the cached heads, each score scaled by `scale`.
    fn attend(&self, layer: usize, rows: &[Row], queries: &[Vec<f32>], heads: usize, scale: f32) -> Vec<Vec<f32>> {
        let BlockState::Attention { kv_heads, width: hd, value_width: vd, .. } = self.layout.blocks[layer] else { unreachable!("an attention block") };
        let group = heads / kv_heads;
        let n = self.layout.numerics;
        let fp = n.fp_unit;
        rows.iter()
            .zip(queries)
            .map(|(row, q)| {
                let mut out = vec![0f32; heads * vd];
                for h in 0..heads {
                    let g = h / group;
                    let qh = &q[h * hd..(h + 1) * hd];
                    let oh = match n.attention {
                        AttentionMath::Vector => {
                            let keys: Vec<Vec<f32>> = (0..=row.position).map(|j| self.cache_values(row.sequence, layer, j, g, false, hd)).collect();
                            let values: Vec<Vec<f32>> = (0..=row.position).map(|j| self.cache_values(row.sequence, layer, j, g, true, vd)).collect();
                            let (keys, values): (Vec<&[f32]>, Vec<&[f32]>) = (keys.iter().map(|k| k.as_slice()).collect(), values.iter().map(|v| v.as_slice()).collect());
                            attend_vector(fp, qh, &keys, &values, scale)
                        }
                        AttentionMath::Array => {
                            let keys: Vec<(u16, Vec<i8>)> = (0..=row.position).map(|j| self.cache_int8(row.sequence, layer, j, g, false, hd)).collect();
                            let values: Vec<(u16, Vec<i8>)> = (0..=row.position).map(|j| self.cache_int8(row.sequence, layer, j, g, true, vd)).collect();
                            let keys: Vec<(u16, &[i8])> = keys.iter().map(|(s, c)| (*s, c.as_slice())).collect();
                            let values: Vec<(u16, &[i8])> = values.iter().map(|(s, c)| (*s, c.as_slice())).collect();
                            attend_int8(fp, qh, &keys, &values, scale, n.activations, n.scaling)
                        }
                    };
                    out[h * vd..(h + 1) * vd].copy_from_slice(&oh);
                }
                out
            })
            .collect()
    }
}

// ── The sequencer's words ───────────────────────────────────────────────────────────────────────────

/// A job as the sequencer reads it: 64-bit words, a header and then the job's own.
///
/// The header's `[3:0]` is the unit — 0 the embedding, 1 the array, 2 the vector unit, 3 the recurrence unit, 4 the
/// attention unit, 5 the step's advance — `[4]` a vector job's order (heads first when set), `[12:5]` the layer,
/// `[20:13]` a vector job's fresh slots, `[28:21]` its loads, `[36:29]` its stores, `[52:37]` its instructions. The
/// embedding's word is its tensor; the array's its tensor `[19:0]`, its slots `[24:20]` and `[29:25]` and its fold
/// `[31:30]` ([`Fold::code`]); the recurrence
/// unit's its seven slots five bits each from bit 0 and the values' element `[54:35]`; a vector job's header its chunk
/// flag at `[53]` ([`VectorJob::chunked`]: run a chunk of its vectors at a time) and its group flag at `[54]`
/// ([`VectorJob::grouped`]: its passes stand alone); the attention unit's its query
/// and output slots `[4:0]` and `[9:5]`, its heads `[25:10]` and its score scale's bits `[57:26]`. A vector job's words
/// are its heads, each fresh slot (`[4:0]` the slot, `[36:5]` its width), each load and each store in two words, and
/// each instruction ([`super::rtl::simd::encode`]). A load's first word holds its register `[3:0]`, its source `[7:4]`
/// (0 a slot, 1 a tensor, 2 a rotary table, 3 a ring), the slot, tensor, table or layer `[23:8]`, the ring row's age, the
/// sines' flag or a tensor's kept flag ([`VectorJob::kept`]: loaded on the job's first pass alone) `[31:24]` and its
/// length `[63:32]`; a store's its register `[3:0]`, scalar `[4]`, its sink `[7:5]`
/// (0 a slot, 1 a ring, 2 a cache), the slot or layer `[23:8]`, values `[24]`, the cache part `[26:25]` (halves, codes,
/// scale) and its length `[63:32]`; the second word of each its element `[31:0]` and its stride a head `[63:32]`.
pub fn encode(jobs: &[Job]) -> Vec<u64> {
    let mut w = Vec::new();
    let header = |unit: u64, order: bool, layer: usize, counts: [usize; 4]| -> u64 {
        assert!(layer < 256 && counts[0] < 256 && counts[1] < 256 && counts[2] < 256 && counts[3] < 1 << 16, "a job past its header's fields");
        unit | (order as u64) << 4 | (layer as u64) << 5 | (counts[0] as u64) << 13 | (counts[1] as u64) << 21 | (counts[2] as u64) << 29 | (counts[3] as u64) << 37
    };
    let s = |slot: Slot| slot.code() as u64;
    let n32 = |v: usize| u32::try_from(v).expect("a field within 32 bits") as u64;
    for job in jobs {
        match job {
            Job::Embed { tensor } => w.extend([header(0, false, 0, [0; 4]), n32(*tensor)]),
            Job::Matvec { tensor, from, to, fold } => {
                assert!(*tensor < 1 << 20, "a tensor index past 20 bits");
                w.extend([header(1, false, 0, [0; 4]), *tensor as u64 | s(*from) << 20 | s(*to) << 25 | fold.code() << 30]);
            }
            Job::Delta { layer, query, key, value, value_at, log_decay, decay, gate, to, norm } => {
                assert!(*value_at < 1 << 20, "an element past 20 bits");
                let slots = [*query, *key, *value, *log_decay, *decay, *gate, *to].iter().enumerate().fold(0u64, |a, (i, &x)| a | s(x) << (5 * i));
                let normed = norm.map_or(0, |n| 1 << 55 | s(n.z) << 56);
                w.extend([header(3, false, *layer, [0; 4]), slots | (*value_at as u64) << 35 | normed]);
                if let Some(n) = norm {
                    assert!(n.weight < 1 << 20, "a tensor index past 20 bits");
                    w.push(n.weight as u64 | (n.eps.to_bits() as u64) << 20);
                }
            }
            Job::Attend { layer, query, heads, scale, to } => {
                assert!(*heads < 1 << 16, "heads past 16 bits");
                w.extend([header(4, false, *layer, [0; 4]), s(*query) | s(*to) << 5 | (*heads as u64) << 10 | (scale.to_bits() as u64) << 26]);
            }
            Job::Advance => w.push(header(5, false, 0, [0; 4])),
            Job::Vector(v) => {
                w.push(header(2, v.order == Order::Heads, 0, [v.fresh.len(), v.loads.len(), v.puts.len(), v.program.len()]) | (v.chunked() as u64) << 53 | (v.grouped() as u64) << 54);
                w.push(n32(v.heads));
                for &(slot, width) in &v.fresh {
                    w.push(s(slot) | n32(width) << 5);
                }
                for (i, l) in v.loads.iter().enumerate() {
                    let (kind, what, extra, at, stride) = match l.from {
                        Source::Slot { slot, at, stride } => (0u64, s(slot), 0u64, at, stride),
                        Source::Tensor { tensor, at, stride } => (1, tensor as u64, u64::from(v.kept(i)), at, stride),
                        Source::Rotary { table, sines } => (2, table as u64, sines as u64, 0, 0),
                        Source::Ring { layer, age } => (3, layer as u64, age as u64, 0, 0),
                    };
                    assert!(what < 1 << 16 && extra < 256 && (l.reg as usize) < VREGS, "a load past its fields: {l:?}");
                    w.push(l.reg as u64 | kind << 4 | what << 8 | extra << 24 | n32(l.len) << 32);
                    w.push(n32(at) | n32(stride) << 32);
                }
                for p in &v.puts {
                    let (reg, scalar) = match p.reg {
                        Reg::V(r) => (r as u64, 0u64),
                        Reg::S(r) => (r as u64, 1),
                    };
                    let (kind, what, values, part, at, stride) = match p.to {
                        Sink::Slot { slot, at, stride } => (0u64, s(slot), 0u64, 0u64, at, stride),
                        Sink::Ring { layer } => (1, layer as u64, 0, 0, 0, 0),
                        Sink::Cache { layer, values, part } => {
                            let part = match part {
                                CachePart::Halves => 0,
                                CachePart::Codes => 1,
                                CachePart::Scale => 2,
                            };
                            (2, layer as u64, values as u64, part, 0, 0)
                        }
                    };
                    assert!(what < 1 << 16 && reg < 16, "a store past its fields: {p:?}");
                    w.push(reg | scalar << 4 | kind << 5 | what << 8 | values << 24 | part << 25 | n32(p.len) << 32);
                    w.push(n32(at) | n32(stride) << 32);
                }
                w.extend(v.program.iter().map(super::rtl::simd::encode));
            }
        }
    }
    w
}

/// The jobs `words` stand for, or where a word stands for none.
pub fn decode(words: &[u64]) -> Result<Vec<Job>, String> {
    struct Cursor<'a> {
        words: &'a [u64],
        at: usize,
    }
    impl Cursor<'_> {
        fn next(&mut self, what: &str) -> Result<u64, String> {
            let w = *self.words.get(self.at).ok_or_else(|| format!("the words end inside {what}"))?;
            self.at += 1;
            Ok(w)
        }
    }
    let mut c = Cursor { words, at: 0 };
    let mut jobs = Vec::new();
    let slot = |code: u64| Slot::of_code((code & 0x1f) as u8).ok_or_else(|| format!("no slot {}", code & 0x1f));
    while c.at < words.len() {
        let mut next = |what: &str| c.next(what);
        let h = next("a header")?;
        let (unit, heads_first, layer) = (h & 0xf, h >> 4 & 1 == 1, (h >> 5 & 0xff) as usize);
        let (fresh, loads, puts, program) = ((h >> 13 & 0xff) as usize, (h >> 21 & 0xff) as usize, (h >> 29 & 0xff) as usize, (h >> 37 & 0xffff) as usize);
        jobs.push(match unit {
            0 => Job::Embed { tensor: next("an embedding")? as usize },
            1 => {
                let x = next("a matrix")?;
                let fold = Fold::of_code(x >> 30 & 3).ok_or_else(|| format!("no fold {}", x >> 30 & 3))?;
                Job::Matvec { tensor: (x & 0xf_ffff) as usize, from: slot(x >> 20)?, to: slot(x >> 25)?, fold }
            }
            3 => {
                let x = next("a recurrence")?;
                let sl = |k: u32| slot(x >> (5 * k));
                Job::Delta {
                    layer,
                    query: sl(0)?,
                    key: sl(1)?,
                    value: sl(2)?,
                    log_decay: sl(3)?,
                    decay: sl(4)?,
                    gate: sl(5)?,
                    to: sl(6)?,
                    value_at: (x >> 35 & 0xf_ffff) as usize,
                    norm: if x >> 55 & 1 == 1 {
                        let n = next("a recurrence's norm")?;
                        Some(DeltaNorm { weight: (n & 0xf_ffff) as usize, z: slot(x >> 56)?, eps: f32::from_bits((n >> 20) as u32) })
                    } else {
                        None
                    },
                }
            }
            4 => {
                let x = next("an attention")?;
                Job::Attend {
                    layer,
                    query: slot(x)?,
                    to: slot(x >> 5)?,
                    heads: (x >> 10 & 0xffff) as usize,
                    scale: f32::from_bits((x >> 26) as u32),
                }
            }
            5 => Job::Advance,
            2 => {
                let heads = next("a vector job's heads")? as u32 as usize;
                let fresh = (0..fresh)
                    .map(|_| {
                        let x = next("a fresh slot")?;
                        Ok((slot(x)?, (x >> 5) as u32 as usize))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                let (loads, flags): (Vec<Load>, Vec<Option<usize>>) = (0..loads)
                    .map(|_| {
                        let (a, b) = (next("a load")?, next("a load")?);
                        let (what, extra, at, stride) = ((a >> 8 & 0xffff) as usize, (a >> 24 & 0xff) as usize, b as u32 as usize, (b >> 32) as usize);
                        let (from, kept) = match a >> 4 & 0xf {
                            0 => (Source::Slot { slot: slot(what as u64)?, at, stride }, None),
                            1 => (Source::Tensor { tensor: what, at, stride }, Some(extra)),
                            2 => (Source::Rotary { table: what, sines: extra == 1 }, None),
                            3 => (Source::Ring { layer: what, age: extra }, None),
                            k => return Err(format!("no load source {k}")),
                        };
                        Ok((Load { reg: (a & 0xf) as VReg, from, len: (a >> 32) as usize }, kept))
                    })
                    .collect::<Result<Vec<_>, String>>()?
                    .into_iter()
                    .unzip();
                let puts = (0..puts)
                    .map(|_| {
                        let (a, b) = (next("a store")?, next("a store")?);
                        let reg = if a >> 4 & 1 == 1 { Reg::S((a & 0xf) as SReg) } else { Reg::V((a & 0xf) as VReg) };
                        let what = (a >> 8 & 0xffff) as usize;
                        let to = match a >> 5 & 0x7 {
                            0 => Sink::Slot { slot: slot(what as u64)?, at: b as u32 as usize, stride: (b >> 32) as usize },
                            1 => Sink::Ring { layer: what },
                            2 => Sink::Cache {
                                layer: what,
                                values: a >> 24 & 1 == 1,
                                part: match a >> 25 & 0x3 {
                                    0 => CachePart::Halves,
                                    1 => CachePart::Codes,
                                    2 => CachePart::Scale,
                                    p => return Err(format!("no cache part {p}")),
                                },
                            },
                            k => return Err(format!("no store sink {k}")),
                        };
                        Ok(Put { reg, to, len: (a >> 32) as usize })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                let program = (0..program)
                    .map(|_| {
                        let x = next("an instruction")?;
                        super::rtl::simd::decode(x).ok_or_else(|| format!("no instruction {x:#018x}"))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                let job = VectorJob { heads, order: if heads_first { Order::Heads } else { Order::Rows }, fresh, loads, program, puts };
                for (i, flag) in flags.iter().enumerate() {
                    if let Some(flag) = *flag {
                        if flag != usize::from(job.kept(i)) {
                            return Err(format!("load {i}'s kept flag {flag}, the job keeping it {}", job.kept(i)));
                        }
                    }
                }
                if (h >> 53 & 1 == 1) != job.chunked() {
                    return Err(format!("a vector job's chunk flag {}, the job running a chunk at a time {}", h >> 53 & 1, job.chunked()));
                }
                if (h >> 54 & 1 == 1) != job.grouped() {
                    return Err(format!("a vector job's group flag {}, the job's passes standing alone {}", h >> 54 & 1, job.grouped()));
                }
                Job::Vector(job)
            }
            u => return Err(format!("no unit {u}")),
        });
    }
    Ok(jobs)
}
