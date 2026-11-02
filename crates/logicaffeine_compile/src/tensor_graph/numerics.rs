//! Quantize insertion: every product that reads its activation quantized, as the chip's shim asks, gets that activation's
//! quantized form — from its producer's epilogue where the chip runs that whole (a `*_quant` class: the producer and the
//! quantize as one op), or from a quantize op of its own right after the producer.
//!
//! A product reading a row of a tensor in place at a whole block reads the tensor's quantized form in place too, so the
//! tensor is quantized once, at the root of its views, for every row a product reads of it. A tensor no op computes whole
//! — a table assembled a row at a time — is quantized right before the first product that reads it, once its last row
//! has landed; or, where each of its rows is set, once and in whole blocks, from a result its producer can quantize in
//! its epilogue — a batch's sequences' rows — a row at a time by those producers, each set carrying its row's quantized
//! form into the table's (PARITY-BATCH-ROW-QUANT).

use std::collections::BTreeSet;

use super::facts;
use super::fuse::{binary, candidate, class, gemv, norm, Rewriter};
use super::front::TensorOpSpec;
use super::graph::{Access, Dst, Op, QuantFormat, Sym};
use super::symbols::ClassId;
use crate::tensor_recognize::qwen::RowIndex;

/// The values of an activation's quantized form that share one scale.
pub const BLOCK: usize = 32;

/// A quantize of `x` in `format`, in place.
fn quantize_op(x: &str, format: QuantFormat) -> Op {
    Op { sym: Sym::Quantize(format), ins: vec![x.to_string()], access: vec![Access::Whole], out: x.to_string(), dst: Dst::to(x), keeps: Vec::new(), covers: Vec::new() }
}

/// The ops that quantize `x` a row at a time: where every row of the table `x` is set — once, in whole blocks, by a
/// literal index — from a result read by nothing but its set and unseen by the host, whose producer the chip runs with
/// that result's quantize in its epilogue, and the chip runs each set carrying its row's quantized form into the
/// table's (`row_write_quant`): each producer and each set, with the class and the quantize it takes in; nothing
/// otherwise.
fn quantized_rows(r: &mut Rewriter, x: &str, format: QuantFormat) -> Option<Vec<(usize, ClassId, Op)>> {
    let t = r.g.tensors.get(x)?;
    let (rows_of, cols_of) = (t.rows, t.cols);
    let sets: Vec<usize> = r.s.live().filter(|(_, op)| facts::writes(op).contains(&x)).map(|(k, _)| k).collect();
    let (mut rows, mut hosts) = (BTreeSet::new(), Vec::new());
    for k in sets {
        let set = r.s.get(k);
        let Some(TensorOpSpec::SetRow(sr)) = set.prim() else { return None };
        let RowIndex::Literal(row) = sr.index else { return None };
        if (sr.rows, sr.cols, sr.count) != (rows_of, cols_of, 1) || sr.cols % BLOCK != 0 || set.ins[0] != x || !rows.insert(row) {
            return None;
        }
        let src = set.ins[1].clone();
        let p = r.s.producer(&src)?;
        if r.s.readers(&src) != [k] || r.observed(&src) {
            return None;
        }
        let host = r.s.get(p).clone();
        let class = folded_class(&host, r.g)?;
        let quant = quantize_op(&src, format);
        let cand = candidate(class, vec![host.clone(), quant.clone()], host.ins.clone(), &host.out, host.dst.clone());
        let set = r.s.get(k).clone();
        let carried = quantize_op(x, format);
        let carry = candidate(ClassId::RowWriteQuant, vec![set.clone(), carried.clone()], set.ins.clone(), &set.out, set.dst.clone());
        if !(r.admits(&cand) && r.admits(&carry)) {
            return None;
        }
        hosts.push((p, class, quant));
        hosts.push((k, ClassId::RowWriteQuant, carried));
    }
    (rows.len() == rows_of && rows.iter().copied().eq(1..=rows_of as i64)).then_some(hosts)
}

/// Fold `quant` into the epilogue of the op at `p` as one op of `class`: one firing of `fused_quant`.
fn fold_into(r: &mut Rewriter, p: usize, class: ClassId, quant: Op) {
    let host = r.s.take(p).expect("a live slot");
    let mut covers = host.covers.clone();
    covers.sort_unstable();
    let (ins, access, out, dst, keeps) = (host.ins.clone(), host.access.clone(), host.out.clone(), host.dst.clone(), host.keeps.clone());
    let parts = vec![host, quant];
    r.s.put(p, Op { sym: Sym::Fused(super::graph::Fused { class, parts }), ins, access, out, dst, keeps, covers });
    r.record("fused_quant", class, p);
}

/// The class a producer's epilogue quantize builds, by what the producer is: a norm, an elementwise op, a gated pair or
/// SwiGLU, a delta rule, an attention, a product — or, for an op a rule built, the member that computes its result.
fn folded_class(op: &Op, g: &super::graph::TensorGraph) -> Option<ClassId> {
    if let Some(c) = direct_class(op).or_else(|| gemv(op, g).map(|_| ClassId::GemvQuant)) {
        return Some(c);
    }
    match &op.sym {
        Sym::Fused(f) => f.parts.iter().rev().find(|p| p.out == op.out).and_then(|p| folded_class(p, g)),
        _ => None,
    }
}

fn direct_class(op: &Op) -> Option<ClassId> {
    if let Some(n) = norm(op) {
        return Some(if n.add.is_some() { ClassId::AddNormQuant } else { ClassId::NormQuant });
    }
    if let Some(b) = binary(op) {
        return Some(if b.gate.is_some() { ClassId::GluActMulQuant } else { ClassId::ElementwiseQuant });
    }
    match (op.prim(), class(op)) {
        (Some(TensorOpSpec::SwiGlu(_)), _) | (_, Some(ClassId::GemvGateUpGlu)) => Some(ClassId::GluActMulQuant),
        (Some(TensorOpSpec::GatedDelta(_)), _) => Some(ClassId::GdnOutputQuant),
        (Some(TensorOpSpec::Attention(_)), _) => Some(ClassId::AttnOutputQuant),
        (Some(TensorOpSpec::FusedRow(_)), _) => Some(ClassId::NormQuant),
        _ => None,
    }
}

/// Give every product that reads its activation quantized that activation's quantized form: in its producer's epilogue
/// where the chip runs the two as one op and `fold` allows it, otherwise as a quantize of its own.
pub fn quantize(r: &mut Rewriter, fold: bool) {
    // The products' quantized activations, each at the root of the in-place views a product reads it through.
    let views: std::collections::BTreeMap<String, (String, usize)> = r
        .s
        .live()
        .filter_map(|(_, op)| match op.sym {
            Sym::View { offset } => Some((op.out.clone(), (op.ins[0].clone(), offset))),
            _ => None,
        })
        .collect();
    let mut read: BTreeSet<(String, super::graph::QuantFormat)> = BTreeSet::new();
    for (_, op) in r.s.live() {
        for k in 0..op.ins.len() {
            if let Some(f) = r.shim.activation(op, k, r.g) {
                read.insert((op.ins[k].clone(), f));
            }
        }
    }
    let mut needed: Vec<(String, super::graph::QuantFormat)> = Vec::new();
    for (x, f) in read {
        let mut root = x;
        while let Some((src, _)) = views.get(&root).filter(|(_, offset)| offset % BLOCK == 0) {
            root = src.clone();
        }
        if !needed.iter().any(|(n, _)| *n == root) {
            needed.push((root, f));
        }
    }
    needed.sort();
    for (x, format) in needed {
        let Some(n) = r.g.tensors.get(&x).map(|t| t.len()) else { continue };
        if n % BLOCK != 0 {
            continue;
        }
        // The ops that read `x`, each with the name it reads it by: `x`, or a view reading its blocks in place.
        let mut readers: Vec<(usize, String)> = Vec::new();
        let mut names = vec![x.clone()];
        while let Some(t) = names.pop() {
            for at in r.s.readers(&t) {
                match r.s.get(at).sym {
                    Sym::View { offset } if offset % BLOCK == 0 => names.push(r.s.get(at).out.clone()),
                    _ => readers.push((at, t.clone())),
                }
            }
        }
        readers.sort();
        let quantized = |r: &Rewriter, at: usize, t: &str| {
            let op = r.s.get(at);
            op.ins.iter().enumerate().any(|(k, n)| n == t && r.shim.activation(op, k, r.g).is_some())
        };
        let producer = r.s.producer(&x);
        let quant = quantize_op(&x, format);
        // The producer's epilogue, where the chip runs the producer and the quantize as one op.
        let folded = producer.filter(|_| fold).and_then(|p| {
            let host = r.s.get(p);
            let class = folded_class(host, r.g)?;
            // The shim reads the producer as it stands, then the quantize it would take into its epilogue.
            let cand = candidate(class, vec![host.clone(), quant.clone()], host.ins.clone(), &host.out, host.dst.clone());
            r.admits(&cand).then_some((p, class))
        });
        // A table set a row at a time, its rows quantized by the producers that store them.
        let rows = if folded.is_none() && fold { quantized_rows(r, &x, format) } else { None };
        match (folded, rows) {
            (Some((p, class)), _) => fold_into(r, p, class, quant),
            (None, Some(hosts)) => {
                for (p, class, quant) in hosts {
                    fold_into(r, p, class, quant);
                }
            }
            (None, None) => {
                let first_product = readers.iter().find(|(at, t)| quantized(r, *at, t)).map_or(readers[0].0, |(at, _)| *at);
                let at = producer.map_or(first_product, |p| p + 1);
                r.s.insert(at, quant);
            }
        }
    }
}

// ── Numerics contracts (benchmarks/FUSIONS_WE_NEED.md, OPT-6) ─────────────────────────────────────────────────────

/// How far a value a chip keeps may lie from the value the meaning computes: `of_block` of the largest magnitude in its
/// block, plus `of_value` of its own magnitude, plus `floor`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bound {
    pub of_block: f64,
    pub of_value: f64,
    pub floor: f64,
}

impl Bound {
    /// Bit for bit.
    pub const EXACT: Bound = Bound { of_block: 0.0, of_value: 0.0, floor: 0.0 };

    /// Whether `kept` lies within the bound of `exact`, in a block whose largest magnitude is `block_max`.
    pub fn holds(&self, exact: f64, kept: f64, block_max: f64) -> bool {
        (exact - kept).abs() <= self.of_block * block_max + self.of_value * exact.abs() + self.floor
    }
}

/// The values of a quantized activation that share one scale: a fixed count, the block of the matrix format that reads
/// it, or the row the op takes it in — a head's queries, a cached key or value row, a query's weights over a tile of
/// keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Block {
    Values(usize),
    OfTheWeights,
    Row,
}

/// How a block's scale is chosen from its largest magnitude: that magnitude over 127 in `f32`; the least `m·2^e` with a
/// `significand_bits`-bit `m` at least that; or, for values never negative, the least `2^e` with the largest at most
/// `255·2^e`, its codes unsigned eight bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scale {
    MaxOver127,
    LeastPowerOfTwo { significand_bits: u32 },
    LeastPowerOfTwoOver255,
}

/// A quantized activation's scheme: its blocks, how each block's scale is chosen, and the int8 words each value takes —
/// one, the nearest whole number of the scale; two, a high word and a low word whose code is the nearest whole number of a
/// finer step (the high word's rest at a 128th of the scale, or a 16-bit code at a 256th); three, a third word at a 128th
/// of the second's step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Scheme {
    pub block: Block,
    pub scale: Scale,
    pub words: u8,
}

impl Scheme {
    /// The scheme of an activation quantized in `format`: blocks of [`BLOCK`] under `max / 127`.
    pub const fn of(format: QuantFormat) -> Scheme {
        let words = match format {
            QuantFormat::OneWord => 1,
            QuantFormat::TwoWords => 2,
        };
        Scheme { block: Block::Values(BLOCK), scale: Scale::MaxOver127, words }
    }

    /// The bound a value read in this scheme keeps: half the finest step, where a scale `max / 127` is that value or, a
    /// least power-of-two scale, up to `1 + 2^(1 − bits)` times it, and an unsigned code's half level under its scale
    /// at most `max / 255`; the slack terms cover the value and the scale read in `f32` (a relative 2⁻²⁴ each, a
    /// sixteenth of a step of the block where that is less than 2⁻²⁰ of it), a denormal flushed to zero, and a scale's
    /// floor.
    pub fn bound(&self) -> Bound {
        let (coarse, step) = match (self.scale, self.words) {
            (Scale::MaxOver127, words) => (1.0, 1.0 / (254.0 * 128f64.powi(i32::from(words) - 1))),
            (Scale::LeastPowerOfTwoOver255, _) => (1.0, 1.0 / 255.0),
            (Scale::LeastPowerOfTwo { significand_bits }, words) => {
                let coarse = 1.0 + 2f64.powi(1 - significand_bits as i32);
                (coarse, if words == 1 { 1.0 / 254.0 } else { 1.0 / (254.0 * 256.0) })
            }
        };
        Bound { of_block: step * coarse + (step * coarse / 16.0).min(2f64.powi(-20)), of_value: 2f64.powi(-24), floor: 2f64.powi(-100) }
    }
}

/// How a chip keeps the values it stores between steps — a cache's rows, a recurrent memory: as the meaning computes
/// them, rounded to the nearest of a narrower float, or a row at a time as int8 codes under one binary16 scale
/// (`max / 127`), the row first turned by a Walsh–Hadamard transform where it is `Int8RowsRotated`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    Exact,
    F32,
    F16,
    Bf16,
    Int8Rows,
    Int8RowsRotated,
}

impl Keep {
    /// The bound a value kept this way keeps. A float: half an ulp of its significand of the value, and its smallest
    /// normal where a chip flushes what lies below it. A row of int8 codes: half a level of the row's scale, the scale
    /// rounded to binary16 (a relative 2⁻¹¹, which can push a code past ±127 to be clamped), and a scale that underflows
    /// binary16; its block is the row as stored — turned, for a rotated row, whose error the inverse turn averages.
    pub fn bound(self) -> Bound {
        match self {
            Keep::Exact => Bound::EXACT,
            Keep::F32 => Bound { of_block: 0.0, of_value: 2f64.powi(-24), floor: 2f64.powi(-126) },
            Keep::Int8Rows | Keep::Int8RowsRotated => Bound { of_block: (1.0 / 254.0) * (1.0 + 2f64.powi(-10)) + 2f64.powi(-11) + 2f64.powi(-19), of_value: 0.0, floor: 2f64.powi(-16) },
            Keep::F16 => Bound { of_block: 0.0, of_value: 2f64.powi(-11) + 2f64.powi(-24), floor: 2f64.powi(-25) },
            Keep::Bf16 => Bound { of_block: 0.0, of_value: 2f64.powi(-8) + 2f64.powi(-24), floor: 2f64.powi(-134) },
        }
    }

    /// `v` as this way keeps it, to the nearest, ties to even; `None` for a format that keeps a row at a time.
    pub fn keep(self, v: f64) -> Option<f64> {
        Some(match self {
            Keep::Int8Rows | Keep::Int8RowsRotated => return None,
            Keep::Exact => v,
            Keep::F32 => f64::from(v as f32),
            Keep::F16 => f64::from(crate::model::quant::f16_to_f32(crate::model::quant::f32_to_f16(v as f32))),
            Keep::Bf16 => {
                let x = v as f32;
                if x.is_nan() {
                    return Some(f64::from(x));
                }
                let bits = x.to_bits();
                f64::from(f32::from_bits(bits.wrapping_add(0x7FFF + ((bits >> 16) & 1)) & 0xFFFF_0000))
            }
        })
    }
}

/// How an op takes a value: kept in a float format, or quantized in a scheme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Taken {
    Kept(Keep),
    Quantized(Scheme),
}

/// How an attention a chip computes on narrower units than its values takes its operands: its queries, its keys and
/// values, and its weights — each weight times its value row's scale where the values are quantized.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AttentionNumerics {
    pub queries: Taken,
    pub keys_values: Taken,
    pub weights: Taken,
}

/// What a chip computes at one knob point, and how far it may lie from the meaning: the scheme a product by a matrix in a
/// block format reads its activation in (`None`, its values), how the chip keeps a cache's rows and a recurrent memory,
/// how an attention takes its operands where it narrows them (`None`, it attends on its values as the chip keeps them),
/// how far a fused op may lie from its parts run apart ([`Bound::EXACT`], bit for bit), and the widest integer a product
/// or a sum of integers accumulates in.
#[derive(Clone, Debug, PartialEq)]
pub struct Contract {
    /// The knob point, by name.
    pub point: String,
    pub activations: Option<Scheme>,
    pub cache: Keep,
    pub state: Keep,
    pub attention: Option<AttentionNumerics>,
    pub fused: Bound,
    pub accumulator_bits: u32,
}

/// Whether a chip whose integers accumulate in `bits` holds every partial result `op` accumulates: `Ok`, or why not —
/// the bits its operands' proven ranges need, or an operand no range bounds on a chip narrower than the language's own
/// 64-bit integers.
pub fn holds_accumulator(op: &Op, g: &super::graph::TensorGraph, bits: u32) -> Result<(), String> {
    use super::facts::{accumulator, Accumulates};
    for leaf in super::fuse::leaves(op.clone()) {
        match accumulator(&leaf, g) {
            Accumulates::Nothing => {}
            Accumulates::Unproven if bits >= 64 => {}
            Accumulates::Unproven => return Err(format!("`{}` sums integers no proven range bounds, in a {bits}-bit accumulator", leaf.out)),
            within => {
                let need = within.bits().expect("a proven interval");
                if need > bits {
                    return Err(format!("`{}`'s partial sums need {need} bits, past the {bits}-bit accumulator", leaf.out));
                }
            }
        }
    }
    Ok(())
}
