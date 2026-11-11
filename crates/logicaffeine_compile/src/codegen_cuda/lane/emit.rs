//! The lane's forms printed as one CUDA program, and the sidecar of values it reads.
//!
//! A lane is the decode step and its prefill chunks (the same layers over a block of 2, 4, … rows), compiled as
//! separate forms that share the model's constants and its carried state. The program is run as
//! `<program> <checkpoint.gguf> <sidecar> <n_gen> [--logits <file>] <token> …`. It maps the checkpoint (refusing a
//! file other than the one it was compiled against: the size and the digest of everything before the data section
//! are compiled in), uploads each quantized matrix's rows straight from it, uploads the computed constants from the
//! sidecar, and zeroes the carried state. Each form's step is captured once as a CUDA graph (the `cuda_graphs` knob).
//! The prompt runs as the largest chunks that fit, then one decode step per generated token: the token and the
//! position live on the device, and each step's arg-max moves both on, so generation needs no round trip to the host.
//! The program prints the generated token ids, the prefill time and the decode rate; `--logits` also writes every
//! generated position's logits as little-endian `f32`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::codegen_cuda::knobs::CudaKnobs;
use crate::codegen_cuda::provenance::{header, Provenance};
use crate::codegen_cuda::qwen_emit::Q8MatKind;
use crate::model::quant::GgmlType;
use crate::tensor_recognize::qwen::{Activation, MoeAct};
use crate::tensor_recognize::ElementwiseOp;

use super::arena;
use super::branches;
use super::kernels::{self, Kernel};
use super::plan::{kind, quant_in_place, Buffer, DType, Dst, Fill, Launch, Part, RopeSide, Router, RowSel, ScaleBy, Epilogue, StepIo, Touch, GEMV_ROWS, QBLOCK};
use super::residence::{self, shared_names, Plan, POOL_ALIGN};
use super::weights::Placement;
use super::graph::Role;
use super::{Form, FormKind, LaneError};

/// The checkpoint a program is compiled against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    pub size: u64,
    pub data_start: u64,
    pub digest: u64,
}

/// A compiled lane: the CUDA program and the sidecar it reads its computed constants from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub cu: String,
    pub sidecar: Vec<u8>,
}

/// A tensor's C name: the program's own name, which the language keeps to letters, digits and underscores.
fn c(name: &str) -> String {
    name.chars().map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' }).collect()
}

fn cdiv(a: usize, b: usize) -> usize {
    a.div_ceil(b)
}

/// A float as a C literal that reads back to the same `f32`.
fn flit(v: f32) -> String {
    format!("{v:e}f")
}

/// How one form names its buffers in C: the constants and the carried state once for every form, the position once
/// for every form (`d_pos`) — a batch's sequence `b`'s at `d_bpos + b` — everything else under the form's tag.
struct Names<'a> {
    tag: String,
    rows: usize,
    batch: bool,
    shared: &'a BTreeSet<String>,
    positions: Vec<&'a str>,
    two: bool,
    sms: usize,
    buffers: &'a BTreeMap<String, Buffer>,
}

impl<'a> Names<'a> {
    fn of(f: &'a Form, shared: &'a BTreeSet<String>, two: bool, sms: usize) -> Names<'a> {
        Names {
            tag: f.tag(),
            rows: f.rows,
            batch: f.kind == FormKind::Batch,
            shared,
            positions: f.schedule.io.positions(),
            two,
            sms,
            buffers: &f.schedule.buffers,
        }
    }

    /// The element types of the arrays a kernel reads or writes in place, as its last template arguments (`elems`).
    fn elems(&self, arrays: &[Option<&str>]) -> String {
        elems(self.buffers, arrays)
    }

    /// The element type of array `t`.
    fn elem(&self, t: &str) -> &'static str {
        self.buffers.get(t).map_or("float", |b| b.dtype.c_type())
    }

    fn plain(&self, t: &str) -> String {
        match self.positions.iter().position(|p| *p == t) {
            Some(b) if self.batch => format!("(d_bpos + {b})"),
            Some(_) => "d_pos".into(),
            None if self.shared.contains(t) => format!("t_{}", c(t)),
            None => format!("t{}_{}", self.tag, c(t)),
        }
    }

    /// The position of a causal convolution over `ring` (`width` values a sequence's ring), the ring's slot of each row:
    /// the form's, or a batch's — its sequences' from the first where one launch takes `seqs` of them, else the position
    /// of the sequence whose ring it is (`X_s<b>`, or its part of the sequences' array).
    fn conv_pos(&self, ring: &str, width: usize, seqs: usize) -> String {
        if !self.batch {
            return "d_pos".into();
        }
        if seqs > 1 {
            return "d_bpos".into();
        }
        let named = ring.rsplit_once("_s").and_then(|(_, b)| b.parse::<usize>().ok());
        let b = named.or_else(|| match self.buffers.get(ring).map(|b| &b.fill) {
            Some(Fill::Alias { offset, .. }) if width > 0 && offset % width == 0 => Some(offset / width),
            _ => None,
        });
        format!("(d_bpos + {})", b.unwrap_or(0))
    }

    fn opt(&self, t: &Option<String>) -> String {
        t.as_deref().map_or_else(|| "nullptr".to_string(), |t| self.plain(t))
    }

    /// A product's epilogue argument (`lk_epi`): its bias alone where its epilogue does nothing past it; otherwise its
    /// bias, its scale, what it adds after — a constant of the product's shape read from row `row` on, a slab's first, of
    /// `cols` outputs a row — and whether it squares the ReLU of that.
    fn epi(&self, bias: &Option<String>, epi: &Option<Epilogue>, row: usize, cols: usize) -> String {
        let Some(e) = epi else { return self.opt(bias) };
        let from = |t: &str, own: bool| match row * cols {
            at if own && at > 0 => format!("{} + {at}", self.plain(t)),
            _ => self.plain(t),
        };
        let (factor, scales, each) = match &e.scale {
            None => (1.0, "nullptr".to_string(), false),
            Some(ScaleBy::Factor(c)) => (*c as f32, "nullptr".to_string(), false),
            Some(ScaleBy::Row(t)) => (1.0, self.plain(t), false),
            Some(ScaleBy::Each(t)) => (1.0, from(t, true), true),
        };
        let (after, own) = e.after.as_ref().map_or(("nullptr".to_string(), false), |(t, own)| (from(t, *own), *own));
        format!("lk_epi({}, {}, {scales}, {each}, {after}, {own}, {})", self.opt(bias), flit(factor), e.relu_sq)
    }

    /// The suffix of a tensor's quantized buffers (`q`, `l` and `s` before it), shared or the form's as its `f32`
    /// buffer is.
    fn qname(&self, t: &str) -> String {
        if self.shared.contains(t) {
            format!("_{}", c(t))
        } else {
            format!("{}_{}", self.tag, c(t))
        }
    }

    fn quant(&self, t: &str, on: bool) -> String {
        if !on {
            return "nullptr, nullptr, nullptr".into();
        }
        let n = self.qname(t);
        let lo = if self.two { format!("l{n}") } else { "nullptr".into() };
        format!("q{n}, {lo}, s{n}")
    }

    /// The quantized buffers of `t` from element `at` (a whole number of blocks) on.
    fn quant_at(&self, t: &str, at: usize) -> String {
        let n = self.qname(t);
        let lo = if self.two { format!("l{n} + {at}") } else { "nullptr".into() };
        format!("q{n} + {at}, {lo}, s{n} + {}", at / QBLOCK)
    }

    fn scratch(&self, kind: &str, t: &str) -> String {
        format!("{kind}{}_{}", self.tag, c(t))
    }

    /// The destination arguments `(buffer, row, pitch)` of a result.
    fn dst(&self, d: &Dst) -> String {
        self.dst_at(d, 0)
    }

    /// The destination arguments of the part of a result `off` elements in.
    fn dst_at(&self, d: &Dst, off: usize) -> String {
        let (buf, row, pitch) = self.dst_split(d, off);
        format!("{buf}, {row}, {pitch}")
    }

    /// [`Self::dst_at`]'s three arguments apart, for a kernel that takes others between them.
    fn dst_split(&self, d: &Dst, off: usize) -> (String, String, usize) {
        let at = |o: usize| if o == 0 { self.plain(&d.buf) } else { format!("{} + {o}", self.plain(&d.buf)) };
        match &d.row {
            Some((RowSel::Param(p), pitch)) => (at(off), self.plain(p), *pitch),
            Some((RowSel::Fixed(r), pitch)) => (at((r - 1) * pitch + off), "nullptr".into(), 0),
            None => (at(off), "nullptr".into(), 0),
        }
    }

    /// The destination of a kernel that writes a plain pointer.
    fn dst_ptr(&self, d: &Dst) -> String {
        match &d.row {
            Some((RowSel::Fixed(r), pitch)) => format!("{} + {}", self.plain(&d.buf), (r - 1) * pitch),
            _ => self.plain(&d.buf),
        }
    }
}

const QWORD_ROWS: &str = "w_";
const QWORD_SCALES: &str = "ws_";

fn rows_q(t: &str) -> String {
    format!("{QWORD_ROWS}{}", c(t))
}

fn rows_s(t: &str) -> String {
    format!("{QWORD_SCALES}{}", c(t))
}

/// The device blocks of a matrix kept in its checkpoint format.
fn rows_b(t: &str) -> String {
    format!("wb_{}", c(t))
}

/// The rows a group of 16-bit matrix `t` (`lk_w::g`), as its upload laid them out.
fn rows_g(t: &str) -> String {
    format!("G_{}", c(t))
}

/// The kernels' handle on matrix `t` of format `ty`, rows `cols` wide: its split Q8_0 arrays, or its blocks and the
/// bytes a row takes — a 16-bit matrix's rows a group too.
fn handle(t: &str, ty: GgmlType, cols: usize) -> String {
    if ty == GgmlType::Q8_0 {
        format!("lk_w{{{}, {}, nullptr, 0}}", rows_q(t), rows_s(t))
    } else if kernels::sixteen(ty) {
        format!("lk_w{{nullptr, nullptr, {}, {}L, {}}}", rows_b(t), kernels::row_bytes(ty, cols), rows_g(t))
    } else {
        format!("lk_w{{nullptr, nullptr, {}, {}L}}", rows_b(t), kernels::row_bytes(ty, cols))
    }
}

/// The block-format tensor-core GEMM of format `f` under numerics `m` over tiles of `bm` activation rows, grouped by
/// expert under `group` 1 and 2 and paired (gate and up, through the gated activation `act`) under 3 — the rows and the
/// activation written only where they are not the defaults (128 rows dense, 64 grouped; SwiGLU).
fn gemm_blk_name(f: u32, m: u32, group: u32, bm: usize, act: i32, plain: bool) -> String {
    let default = if matches!(group, 1..=3) { 64 } else { kernels::GEMM_BLK_TILE_M };
    if plain {
        return format!("lk_gemm_blk<{f}, {m}, {group}, {bm}, {act}, true>");
    }
    match (group, bm == default, act) {
        (0, true, _) => format!("lk_gemm_blk<{f}, {m}>"),
        (_, true, 0) => format!("lk_gemm_blk<{f}, {m}, {group}>"),
        (_, _, 0) => format!("lk_gemm_blk<{f}, {m}, {group}, {bm}>"),
        _ => format!("lk_gemm_blk<{f}, {m}, {group}, {bm}, {act}>"),
    }
}

/// Whether a block-format GEMM's every output is its sum as it is into an `f32` array (`lk_gemm_blk`'s `PLAIN`): a dense
/// product's or every part's, with no bias and no epilogue step — so its outputs store at once, and the epilogue's
/// general steps take no registers from the tile's multiply (Qwen3.5-0.8B's 8192-output in-projections at 512 rows on the
/// 3060, in parts: 607 µs → 380).
fn blk_plain(l: &Launch, buffers: &BTreeMap<String, Buffer>) -> bool {
    let f32 = |d: &Dst| d.unit.is_none() && buffers.get(&d.buf).is_some_and(|b| b.dtype == DType::F32);
    match l {
        Launch::Gemv { bias, epi, y, .. } => bias.is_none() && epi.is_none() && f32(y),
        Launch::GemmParts { parts, .. } => parts.iter().all(|p| p.bias.is_none() && f32(&p.y)),
        _ => false,
    }
}

/// The 16-bit tensor-core GEMM of format `f` under numerics `m` (one activation word, two), dense (`group` 0), in parts
/// (4) or paired through the gated activation `act` (5), in tiles of `tn` outputs.
fn gemm_h16_name(f: u32, m: u32, group: u32, act: i32, tn: usize) -> String {
    match (group, act, tn) {
        (0, _, 128) => format!("lk_gemm_h16<{f}, {m}>"),
        (_, 0, 128) => format!("lk_gemm_h16<{f}, {m}, {group}>"),
        (_, _, 128) => format!("lk_gemm_h16<{f}, {m}, {group}, {act}>"),
        _ => format!("lk_gemm_h16<{f}, {m}, {group}, {act}, {tn}>"),
    }
}

/// The kernels the forms launch.
fn kernels_used(forms: &[Form]) -> Vec<Kernel> {
    let mut ks: Vec<Kernel> = forms.iter().flat_map(|f| f.schedule.launches.iter().flat_map(move |l| launch_kernels(l, f.schedule.numerics))).collect();
    ks.sort();
    ks.dedup();
    ks
}

/// The library kernels launch `l` runs under `numerics`.
fn launch_kernels(l: &Launch, numerics: Q8MatKind) -> Vec<Kernel> {
    match l {
        Launch::Gather { format: Some(GgmlType::Q8_0), .. } => vec![Kernel::GatherQ8],
        Launch::Gather { format: Some(_), .. } => vec![Kernel::Blocks, Kernel::GatherBlk],
        Launch::Gather { .. } => vec![Kernel::GatherF32],
        Launch::View { .. } => vec![],
        Launch::Norm { .. } => vec![Kernel::RmsNorm],
        Launch::NormParts { .. } => vec![Kernel::NormParts],
        Launch::Quant { .. } => vec![Kernel::Quant],
        Launch::Moe { router, .. } if l.on_tensor_cores(numerics) => {
            let mut k = vec![Kernel::Blocks, Kernel::Mma, Kernel::GemmBlk, Kernel::Moe, Kernel::Quant];
            k.extend((*router == Router::F32).then_some(Kernel::GemmF32));
            k
        }
        l if l.on_tensor_cores(numerics) && l.reads_sixteen() => vec![Kernel::Blocks, Kernel::Mma, Kernel::GemmH16],
        l if l.on_tensor_cores(numerics) => vec![Kernel::Blocks, Kernel::Mma, Kernel::GemmBlk],
        Launch::GemvGlu { .. } if l.batch_blk_format(numerics).is_some() => vec![Kernel::Blocks, Kernel::GemvMmaBlk, Kernel::SwiGlu, Kernel::Binary],
        l if l.batch_blk_format(numerics).is_some() => vec![Kernel::Blocks, Kernel::GemvMmaBlk],
        Launch::GemvGlu { .. } if l.batch_on_tensor_cores(numerics) => {
            vec![Kernel::Blocks, Kernel::Mma, Kernel::GemvMma, Kernel::SwiGlu, Kernel::Binary]
        }
        l if l.batch_on_tensor_cores(numerics) => vec![Kernel::Blocks, Kernel::Mma, Kernel::GemvMma],
        Launch::GemmParts { .. } => vec![Kernel::Blocks, Kernel::GemvParts],
        Launch::Gemv { format: GgmlType::Q8_0, .. } => vec![Kernel::GemvQ8],
        Launch::Gemv { .. } => vec![Kernel::Blocks, Kernel::GemvBlk],
        l if l.multiplies_in_tiles(numerics) => vec![Kernel::GemmF32],
        Launch::GemvF32 { .. } => vec![Kernel::GemvF32],
        Launch::Binary { .. } => vec![Kernel::Binary],
        Launch::Scale { .. } => vec![Kernel::Scale],
        Launch::Softcap { .. } => vec![Kernel::Softcap],
        Launch::SwiGlu { .. } => vec![Kernel::SwiGlu],
        Launch::Unary { .. } => vec![Kernel::Unary],
        Launch::Relu { .. } => vec![Kernel::Relu],
        Launch::Rope { .. } | Launch::RopeParts { .. } => vec![Kernel::Rope],
        Launch::SetRow { .. } => vec![Kernel::SetRow],
        Launch::Attention { .. } if l.attends_on_tensor_cores(numerics) => vec![Kernel::Mma, Kernel::AttentionMma],
        Launch::Attention { .. } if l.attends_in_two_words(numerics) => vec![Kernel::Mma, Kernel::AttentionI8],
        Launch::Attention { .. } if l.attends_flash(numerics) => vec![Kernel::Mma, Kernel::AttentionFlash],
        Launch::Attention { .. } if l.attends_in_tiles(numerics) => vec![Kernel::AttentionTiled],
        Launch::Attention { shared: Some(_), .. } => vec![Kernel::Mma, Kernel::AttentionShared],
        Launch::Attention { .. } => vec![Kernel::Attention],
        Launch::ArgMax { .. } => vec![Kernel::ArgMax],
        Launch::Conv { .. } | Launch::ConvParts { .. } => vec![Kernel::Conv],
        Launch::L2Norm { .. } => vec![Kernel::L2Norm],
        Launch::Decay { .. } => vec![Kernel::Decay],
        Launch::Gates { .. } | Launch::GateProj { .. } => vec![Kernel::Decay, Kernel::Gates],
        Launch::Delta { .. } if l.delta_in_chunks(numerics) => vec![Kernel::DeltaChunks, Kernel::DeltaNorm],
        Launch::Delta { .. } => vec![Kernel::DeltaStep, Kernel::DeltaNorm, Kernel::DeltaStepNorm],
        Launch::Moe { .. } if numerics != Q8MatKind::Dequant => vec![Kernel::Blocks, Kernel::Moe, Kernel::Quant],
        Launch::Moe { .. } => vec![Kernel::Blocks, Kernel::Moe],
        Launch::Slice { .. } => vec![Kernel::Slice],
        Launch::Join { .. } => vec![Kernel::Join],
        Launch::HeadGemv { .. } if l.heads_in_tiles(numerics) => vec![Kernel::Blocks, Kernel::HeadTiled],
        Launch::HeadGemv { .. } => vec![Kernel::Blocks, Kernel::Heads],
        Launch::GemvGlu { .. } => vec![Kernel::Blocks, Kernel::Glu],
        l if l.gdn_inputs().is_some() => vec![Kernel::Blocks, Kernel::Conv, Kernel::Decay, Kernel::Gates, Kernel::GemvGdn],
        Launch::Fold { first, .. } if matches!(first.as_ref(), Launch::Gather { .. }) => {
            vec![Kernel::Blocks, Kernel::GatherQ8, Kernel::GatherBlk, Kernel::GatherF32, Kernel::RmsNorm, Kernel::GatherNorm]
        }
        Launch::Fold { then, .. } if matches!(then.as_ref(), Launch::RopeParts { .. }) => vec![Kernel::Blocks, Kernel::Rope, Kernel::GemvPartsRope],
        // A mixture whose last step adds and norms: the mixture's kernels, its last step's with the norm's row.
        Launch::Fold { first, .. } if l.finished_mixture().is_some() => {
            let mut k = launch_kernels(first.parts()[0], numerics);
            k.extend([Kernel::RmsNorm, Kernel::MoeNorm]);
            k
        }
        Launch::Fold { first, .. } if matches!(first.as_ref(), Launch::Gemv { format: GgmlType::Q8_0, .. }) => vec![Kernel::RmsNorm, Kernel::GemvQ8Norm],
        Launch::Fold { .. } => vec![Kernel::Blocks, Kernel::GemvBlk, Kernel::RmsNorm, Kernel::GemvBlkNorm],
    }
}

/// Where a launch reads its input's units of `width` values: a part of every unit of a wider tensor, or the whole
/// tensor — `(pitch, offset)`.
fn reading(part: &Option<Part>, width: usize) -> (usize, usize) {
    part.map_or((width, 0), |q| (q.pitch, q.offset))
}

/// Where a launch writes its result's units of `width` values in its destination, `(pitch, offset)`.
fn unit(d: &Dst, width: usize) -> (usize, usize) {
    reading(&d.unit, width)
}

/// The output rows a warp a launch of a product by `format` over `rows` outputs takes, its plan giving `per_warp`: a
/// 16-bit matrix's on the tensor cores as many as still fill the chip (`kernels::sixteen_launch_rows`), every other
/// format's its plan's.
fn launch_rows(format: GgmlType, per_warp: usize, rows: usize, numerics: Q8MatKind, n: &Names) -> usize {
    if kernels::sixteen(format) && numerics != Q8MatKind::Dequant {
        kernels::sixteen_launch_rows(per_warp, rows, n.sms)
    } else {
        per_warp
    }
}

/// The outputs a tile of the 16-bit GEMM takes under `numerics` (`kernels::gemm_h16_tile_n`).
fn h16_tile(numerics: Q8MatKind) -> usize {
    kernels::gemm_h16_tile_n(numerics == Q8MatKind::TwoPassInt8)
}

/// The outputs a tile of the 16-bit GEMM takes in launch `l` on a chip of `sms` multiprocessors
/// (`kernels::gemm_h16_launch_tile`): a launch in parts `h16_tile`'s, whole tiles a part.
fn h16_launch_tile(l: &Launch, numerics: Q8MatKind, sms: usize) -> usize {
    let two = numerics == Q8MatKind::TwoPassInt8;
    match l {
        Launch::Gemv { n, m, .. } => kernels::gemm_h16_launch_tile(two, *n, *m, sms),
        Launch::GemvGlu { n, m, .. } => kernels::gemm_h16_launch_tile(two, 2 * *n, *m, sms),
        _ => h16_tile(numerics),
    }
}

/// Whether a 16-bit GEMM in parts runs as one launch over its parts side by side (`lk_gemm_h16`'s `G` 4): each part
/// whole tiles of the GEMM's; otherwise a launch a part.
fn h16_parted(parts: &[super::plan::GemmPart], numerics: Q8MatKind) -> bool {
    parts.iter().all(|p| p.n % h16_tile(numerics) == 0)
}

/// The template argument that names a kernel's output rows a warp, left to its default of one.
fn per_warp_arg(rows: usize) -> String {
    if rows == 1 {
        String::new()
    } else {
        format!(", {rows}")
    }
}

/// The threads of a rotation's block: a norm inside takes the norm kernel's, so it sums a head's squares as
/// `lk_rmsnorm` sums them.
fn rope_threads(normed: bool, hd: usize) -> usize {
    if normed {
        kernels::norm_threads(hd)
    } else {
        128
    }
}

/// The `lk_rope_side` of one rotation of a launch of rotations.
fn rope_side(r: &RopeSide, hd: usize, n: &Names) -> String {
    let ((xp, xo), (yp, yo)) = (reading(&r.part, hd), unit(&r.y, hd));
    format!(
        "lk_rope_side{{{}, {}, {}, {}, {xp}, {xo}, {yp}, {yo}, {}, {}, {}}}",
        n.plain(&r.x),
        r.norm.as_ref().map_or_else(|| "nullptr".to_string(), |(w, _)| n.plain(w)),
        n.dst(&r.y),
        flit(r.norm.as_ref().map_or(0.0, |(_, e)| e.get() as f32)),
        r.heads,
        r.per_row,
        r.yz
    )
}

/// `lk_attention`'s blocks a multiprocessor as its last template argument: none for the default two.
/// The element types of `arrays` — a memory, a cache's keys, values and split keys' rest — as a kernel's last template
/// arguments, each array's own (`f32` for none): none where every one is `f32`, so a launch over `f32` arrays names the
/// kernel it always named.
fn elems(buffers: &BTreeMap<String, Buffer>, arrays: &[Option<&str>]) -> String {
    let types: Vec<&str> = arrays.iter().map(|a| a.and_then(|a| buffers.get(a)).map_or("float", |b| b.dtype.c_type())).collect();
    if types.iter().all(|t| *t == "float") {
        String::new()
    } else {
        types.iter().map(|t| format!(", {t}")).collect()
    }
}

/// An attention's kernel (`lk_attention`), every template argument named: its heads a block, its value pairs a thread,
/// its accumulator, its blocks a multiprocessor, its cache's element types (`kv`, `f32` for none), its store's `QO`
/// (`attention_qo`, 0 for none), its path past a row of one chunk (`kernels::attn_path`) and its run width — the one
/// name its launch and its shared memory's limit both take.
#[allow(clippy::too_many_arguments)]
fn attention_kernel(per_block: usize, vd: usize, ty: &str, mb: usize, kv: &str, qo: &str, path: &str, hd: usize) -> String {
    let kv = if kv.is_empty() { ", float, float" } else { kv };
    let qo = if qo.is_empty() { ", 0" } else { qo };
    format!("lk_attention<{per_block}, {}, {ty}, {mb}{kv}{qo}, {path}, {}>", kernels::attn_values_per_thread(vd), kernels::attn_run_width(hd))
}

/// The f16 tensor-core attention's kernel (`lk_attention_mma`), the one name its launch and its shared memory's limit
/// both take: its heads a block and widths, and its cache's element types (`kv`, as `elems` names them) — or under the
/// stream-K schedule (`stream` blocks) every element type named (`kte`: keys, values, the split keys' rest), its blocks
/// a multiprocessor and the schedule.
fn attention_mma_kernel(hg: usize, hd: usize, vd: usize, kv: &str, kte: [&str; 3], stream: usize) -> String {
    if stream == 0 {
        format!("lk_attention_mma<{hg}, {hd}, {vd}{kv}>")
    } else {
        let [ke, ve, te] = kte;
        format!("lk_attention_mma<{hg}, {hd}, {vd}, {ke}, {ve}, {te}, {}, 1>", kernels::attn_mma_blocks(hd, vd))
    }
}

/// A latent group's attention kernel (`lk_attention_shared`): its heads, value pairs a thread, head width in fours,
/// whether its values are its keys' rows, its cache's element types and its store's `QO` — named alike by its launch
/// and its shared memory's limit.
fn attention_shared_kernel(heads: usize, vd: usize, hd: usize, vk: bool, kv: &str, qo: &str) -> String {
    let kv = if !qo.is_empty() && kv.is_empty() { ", float, float" } else { kv };
    format!("lk_attention_shared<{heads}, {}, {}, {vk}{kv}{qo}>", kernels::attn_values_per_thread(vd), hd / 4)
}

/// An attention's `QO` template argument (`Launch::Attention::quant`): its store's words where it quantizes its output,
/// 0 where it only gates it, none where it writes only `f32`.
fn attention_qo(quant: bool, gate: &Option<(String, Activation)>, m: u32) -> String {
    if quant || gate.is_some() {
        format!(", {}", if quant { m } else { 0 })
    } else {
        String::new()
    }
}

/// An attention's quantized output (`Launch::Attention::quant`): the `QO` template argument (`attention_qo`) and the
/// `lk_qout` its kernel takes last — nothing when it writes only `f32`.
fn attention_quant(n: &Names, out: &str, quant: bool, gate: &Option<(String, Activation)>, goff: usize, m: u32) -> (String, String) {
    let words = if quant { n.quant(out, true) } else { "nullptr, nullptr, nullptr".into() };
    let (g, ga) = gate.as_ref().map_or(("nullptr".to_string(), -1), |(g, act)| match goff {
        0 => (n.plain(g), act_arg(*act)),
        off => (format!("({} + {off})", n.plain(g)), act_arg(*act)),
    });
    let qo = attention_qo(quant, gate, m);
    let args = if qo.is_empty() { String::new() } else { format!(", lk_qout{{{words}, {g}, {ga}}}") };
    (qo, args)
}

/// An attention block's shared memory (`lk_attention`) and the chunks it weighs side by side: a block that takes every
/// chunk of its row (not `parallel`) holds their partials where they fit (`kernels::attn_holds`) and weighs each chunk
/// once, `kernels::attn_side_chunks` side by side; otherwise none.
fn attention_smem(per_block: usize, hd: usize, vd: usize, splits: usize, parallel: bool, acc: usize) -> (usize, usize) {
    let side = if !parallel && kernels::attn_holds(per_block, vd, splits, acc) { kernels::attn_side_chunks(vd, splits) } else { 0 };
    let held = if side > 0 { kernels::attn_held_bytes(per_block, vd, splits, acc) + kernels::attn_side_bytes(per_block, side, acc) } else { 0 };
    (kernels::attn_smem_bytes(per_block, hd, splits, acc) + held, side)
}

/// A product's activation arguments `(x, xq, xl, xs)`, from `at` values on where it names an offset: its `f32` values
/// under the precise numerics and for a 16-bit float matrix (`kernels::sixteen`), which reads them `f32` under every
/// numerics; its quantized form otherwise.
fn activation_args(n: &Names, x: &str, at: Option<usize>, m: u32, sixteen: bool) -> String {
    match at {
        _ if m > 0 && !sixteen => format!("nullptr, {}", n.quant_at(x, at.unwrap_or(0))),
        Some(at) => format!("{} + {at}, nullptr, nullptr, nullptr", n.plain(x)),
        None => format!("{}, nullptr, nullptr, nullptr", n.plain(x)),
    }
}

/// One part of a launch over matrices side by side (`lk_parts`): its matrix, its destination `(array, row, pitch)`, its
/// bias, and its outputs.
struct PartOut {
    w: String,
    dst: (String, String, usize),
    bias: String,
    n: usize,
}

/// The `lk_parts` of up to `GEMM_PARTS` parts, those past the last ending at the launch's last output.
fn lk_parts(parts: &[PartOut]) -> String {
    let pad = |v: Vec<String>, with: &str| (0..kernels::GEMM_PARTS).map(|i| v.get(i).cloned().unwrap_or_else(|| with.to_string())).collect::<Vec<_>>().join(", ");
    let cols: usize = parts.iter().map(|p| p.n).sum();
    let ends: Vec<String> = parts.iter().scan(0, |end, p| {
        *end += p.n;
        Some(end.to_string())
    }).collect();
    let groups = [
        pad(parts.iter().map(|p| p.w.clone()).collect(), "lk_w{}"),
        pad(parts.iter().map(|p| p.dst.0.clone()).collect(), "nullptr"),
        pad(parts.iter().map(|p| p.bias.clone()).collect(), "nullptr"),
        pad(ends, &cols.to_string()),
        pad(parts.iter().map(|p| p.dst.1.clone()).collect(), "nullptr"),
        pad(parts.iter().map(|p| p.dst.2.to_string()).collect(), "0"),
    ];
    format!("lk_parts{{{}}}", groups.iter().map(|g| format!("{{{g}}}")).collect::<Vec<_>>().join(", "))
}

/// The template argument that names an activation in the library (`lk_act`).
/// A batch's projection by up to `GEMM_PARTS` Q8_0 matrices side by side on the tensor cores (`lk_gemv_mma`): its rows in
/// launches of `GEMV_MMA_ROWS`, each launch's activation and destination rows the launch's — and a lone product's
/// epilogue (`epi`, from a launch's first row on) its own.
fn gemv_mma(n: &Names, x: &str, k: usize, rows: usize, parts: &[PartOut], m: u32, epi: Option<&dyn Fn(usize) -> String>) -> String {
    let cols: usize = parts.iter().map(|p| p.n).sum();
    (0..rows.div_ceil(kernels::GEMV_MMA_ROWS))
        .map(|slab| {
            let r = kernels::GEMV_MMA_ROWS.min(rows - slab * kernels::GEMV_MMA_ROWS);
            let at = slab * kernels::GEMV_MMA_ROWS;
            let (mt, nt, mw, nw) = kernels::gemv_mma_launch(r, cols, k, n.sms);
            let slab_parts: Vec<PartOut> = parts
                .iter()
                .map(|p| {
                    let (buf, row, pitch) = &p.dst;
                    let base = if at == 0 { buf.clone() } else { format!("{buf} + {}", at * p.n) };
                    let bias = epi.filter(|_| parts.len() == 1).map_or_else(|| p.bias.clone(), |e| e(at));
                    PartOut { w: p.w.clone(), dst: (base, row.clone(), *pitch), bias, n: p.n }
                })
                .collect();
            format!(
                "lk_gemv_mma<{m}, {mt}, {nt}, {mw}, {nw}><<<{}, {}, {}, st>>>({}, {k}, {cols}, {r}, {});",
                cdiv(cols, 8 * nt),
                kernels::gemv_mma_threads(mt, nt, mw, nw),
                kernels::gemv_mma_smem_bytes(n.two, mt, nt),
                n.quant_at(x, at * k),
                lk_parts(&slab_parts)
            )
        })
        .collect::<Vec<_>>()
        .join("\n  CKL();\n  ")
}

/// A batch's gated pair's gate over its gate's and up's products in scratch (`hg`, `hu`, `len` values): SwiGLU, or the
/// activation `act` of the gate times the up, written to `out` (`f32` and, quantized under numerics `m`, its codes) as
/// the pair's own epilogue writes it.
#[allow(clippy::too_many_arguments)]
fn glu_gate(n: &Names, m: u32, hg: &str, hu: &str, act: &Option<Activation>, out: &str, f32: bool, quant: bool, len: usize) -> String {
    let (b, qm) = (if f32 { "true" } else { "false" }, if quant { m } else { 0 });
    let y = if f32 { n.plain(out) } else { "nullptr".into() };
    match act {
        None => format!("lk_swiglu<{b}, {qm}><<<{}, 256, 0, st>>>({hg}, {hu}, {y}, {}, {len});", cdiv(len, 256), n.quant(out, quant)),
        Some(a) => format!(
            "lk_binary<1, {}, {b}, {qm}><<<{}, 256, 0, st>>>({hu}, {hg}, {y}, {}, {len}, {len});",
            act_arg(*a),
            cdiv(len, 256),
            n.quant(out, quant)
        ),
    }
}

/// A batch's projection by up to `GEMM_PARTS` Q4_K or Q6_K matrices side by side on the tensor cores in its rows' own
/// order (`lk_gemv_mma_blk`): one launch of its rows, `GEMV_MMA_BLK_OUTPUTS` outputs a block.
fn gemv_mma_blk(n: &Names, x: &str, k: usize, rows: usize, parts: &[PartOut], format: GgmlType) -> String {
    let cols: usize = parts.iter().map(|p| p.n).sum();
    let f = kernels::format_arg(format);
    format!(
        "lk_gemv_mma_blk<{f}><<<{}, {}, {}, st>>>({}, {k}, {cols}, {rows}, {});",
        cols / kernels::GEMV_MMA_BLK_OUTPUTS,
        kernels::GEMV_MMA_BLK_THREADS,
        kernels::gemv_mma_blk_smem_bytes(f),
        n.quant_at(x, 0),
        lk_parts(parts)
    )
}

/// The arguments a soft-capped attention passes after those its launch gives anyway (`given`): the defaults of the
/// arguments between where it gives none, then its cap (`lk_cap`); nothing where its scores are not capped.
fn capped(cap: Option<f64>, given: &str, defaults: &str) -> String {
    cap.map_or(String::new(), |c| format!("{}, {}", if given.is_empty() { defaults } else { "" }, flit(c as f32)))
}

/// An attention's window as its kernels take it (`lk_window_lo`): its span, negative for a chunked-local one, 0 for none.
fn win(window: Option<usize>, chunked: bool) -> i64 {
    window.map_or(0, |w| if chunked { -(w as i64) } else { w as i64 })
}

fn act_arg(act: Activation) -> i32 {
    match act {
        Activation::Silu => 0,
        Activation::Sigmoid => 1,
        Activation::Gelu => 2,
    }
}

/// The launch statements of one launch of a form; `token` is the decode step's token buffer, which every step's
/// arg-max advances.
fn launch(l: &Launch, n: &Names, numerics: Q8MatKind, token: &str, out: &mut String) {
    let m = kernels::mode(numerics);
    let qm = |on: bool| if on { m } else { 0 };
    // Every kernel's sums: `f64` under the precise numerics.
    let sum_ty = kernels::accumulator(m).1;
    let b = |v: bool| if v { "true" } else { "false" };
    let p = |t: &str| n.plain(t);
    // A mixture that adds and norms in its last step is the mixture's launch, that step finished by the norm: its count
    // of blocks arrived, and the array the norm adds the mixture's sum with, if any.
    let (l, finish) = match l.finished_mixture() {
        Some((mixture, norm, sum)) => (mixture, Some((norm, sum, n.scratch("gc", l.writes()[0])))),
        None => (l, None),
    };
    let line = match l {
        Launch::View { .. } => return,
        Launch::Gather { table, index, out, cols, rows, format: Some(GgmlType::Q8_0), scale } => format!(
            "lk_gather_q8<<<dim3({}, {rows}), 256, 0, st>>>({}, {}, {}, {}, {cols}, {});",
            cdiv(*cols, 256),
            rows_q(table),
            rows_s(table),
            p(index),
            p(out),
            flit(scale.unwrap_or(1.0) as f32)
        ),
        Launch::Gather { table, index, out, cols, rows, format: Some(ty), scale } => format!(
            "lk_gather_blk<{}><<<dim3({}, {rows}), 256, 0, st>>>({}, {}, {}, {cols}, {});",
            kernels::format_arg(*ty),
            cdiv(cols / 32, 256),
            handle(table, *ty, *cols),
            p(index),
            p(out),
            flit(scale.unwrap_or(1.0) as f32)
        ),
        Launch::Gather { table, index, out, cols, rows, format: None, scale } => format!(
            "lk_gather_f32<<<dim3({}, {rows}), 256, 0, st>>>({}, {}, {}, {cols}, {});",
            cdiv(*cols, 256),
            p(table),
            p(index),
            p(out),
            flit(scale.unwrap_or(1.0) as f32)
        ),
        Launch::Scale { x, y, n: len, factor } => format!(
            "lk_scale<<<{}, 256, 0, st>>>({}, {}, {len}, {});",
            cdiv(*len, 256),
            p(x),
            p(y),
            flit(*factor as f32)
        ),
        Launch::Softcap { x, y, n: len, cap } => format!("lk_softcap<<<{}, 256, 0, st>>>({}, {}, {len}, {});", cdiv(*len, 256), p(x), p(y), flit(*cap as f32)),
        Launch::Norm { x, part, add, pre, sum, w, out, into, f32, quant, rows, cols, eps, scale, fold } => {
            let own = Dst::to(out);
            let d = into.as_ref().unwrap_or(&own);
            let (y, yrow, ypitch) = if *f32 { n.dst_split(d, 0) } else { ("nullptr".into(), "nullptr".into(), 0) };
            let ((xp, xo), (yp, yo)) = (reading(part, *cols), unit(d, *cols));
            // A chunk's rows in blocks that each play `vw` of the norm's warps a warp.
            let (threads, vw) = kernels::norm_block(*cols, *rows);
            // A norm of its added operand first (`sandwich_norm`): its weight and epsilon after the rest; then the factor
            // its result is scaled by (`norm_scale`).
            let (pre_arg, mut pre_tail) = match pre {
                Some(pn) => (", true", format!(", {}, {}", n.opt(&pn.w), flit(pn.eps.get() as f32))),
                None => ("", String::new()),
            };
            // Then the partials whose fold it adds in place of `add` (`allreduce_norm`).
            if scale.is_some() || !fold.is_empty() {
                if pre.is_none() {
                    pre_tail = format!(", nullptr, {}", flit(0.0));
                }
                pre_tail.push_str(&format!(", {}", flit(scale.unwrap_or(1.0) as f32)));
            }
            // The partials' count a template argument after `PRE`, so their pointers stay registers.
            let pre_arg = if fold.is_empty() && vw == 1 { pre_arg.to_string() } else { format!(", {}, {}", b(pre.is_some()), fold.len()) };
            let pre_arg = if vw == 1 { pre_arg } else { format!("{pre_arg}, {vw}") };
            if !fold.is_empty() {
                let parts: Vec<String> = fold.iter().map(|t| p(t)).chain(std::iter::repeat_n("nullptr".to_string(), kernels::FOLD_PARTIALS - fold.len())).collect();
                pre_tail.push_str(&format!(", lk_fold{{{{{}}}}}", parts.join(", ")));
            }
            format!(
                "lk_rmsnorm<{}, {}, {}, {}, {}, {sum_ty}{pre_arg}><<<{rows}, {threads}, 0, st>>>({}, {}, {}, {}, {y}, {}, {cols}, {}, {xp}, {xo}, {yrow}, {ypitch}, {yp}, {yo}{pre_tail});",
                b(add.is_some() || !fold.is_empty()),
                b(sum.is_some()),
                b(*f32),
                qm(*quant),
                cols.div_ceil(threads * vw),
                p(x),
                n.opt(add),
                n.opt(sum),
                n.opt(w),
                n.quant(out, *quant),
                flit(eps.get() as f32)
            )
        }
        // Every part's sums of squares, then every part it writes normed by their sum; both a block a row and part, of
        // the widest part's norm's threads.
        Launch::NormParts { parts, normed, w, rows, cols, eps } => {
            let widest = parts.iter().map(|q| q.cols).max().unwrap_or(1);
            let threads = kernels::norm_threads(widest);
            let per = widest.div_ceil(threads);
            let list = |v: Vec<String>, none: &str| {
                let mut v = v;
                v.resize(kernels::NORM_PARTS_MAX, none.to_string());
                v.join(", ")
            };
            let at: Vec<(usize, usize)> = parts.iter().map(|q| reading(&q.part, q.cols)).collect();
            let ins = format!(
                "lk_row_parts{{{{{}}}, {{{}}}, {{{}}}, {{{}}}, {{{}}}}}",
                list(parts.iter().map(|q| p(&q.x)).collect(), "nullptr"),
                list(at.iter().map(|(pitch, _)| pitch.to_string()).collect(), "0"),
                list(at.iter().map(|(_, offset)| offset.to_string()).collect(), "0"),
                list(parts.iter().map(|q| q.cols.to_string()).collect(), "0"),
                list(parts.iter().scan(0, |o, q| Some(std::mem::replace(o, *o + q.cols).to_string())).collect(), "0")
            );
            let dsts: Vec<(String, String, usize)> = normed.iter().map(|o| n.dst_split(&o.y, 0)).collect();
            let units: Vec<(usize, usize)> = normed.iter().map(|o| unit(&o.y, parts[o.part].cols)).collect();
            let outs = format!(
                "lk_normed_parts{{{{{}}}, {{{}}}, {{{}}}, {{{}}}, {{{}}}, {{{}}}}}",
                list(dsts.iter().map(|d| d.0.clone()).collect(), "nullptr"),
                list(dsts.iter().map(|d| d.1.clone()).collect(), "nullptr"),
                list(dsts.iter().map(|d| d.2.to_string()).collect(), "0"),
                list(units.iter().map(|(pitch, _)| pitch.to_string()).collect(), "0"),
                list(units.iter().map(|(_, offset)| offset.to_string()).collect(), "0"),
                list(normed.iter().map(|o| o.part.to_string()).collect(), "0")
            );
            let sums = n.scratch("ss", l.writes()[0]);
            format!(
                "lk_sumsq<{per}, {sum_ty}><<<dim3({rows}, {}), {threads}, 0, st>>>({ins}, {sums}, {});\n  CKL();\n  \
                 lk_norm_by_sums<{per}, {sum_ty}><<<dim3({rows}, {}), {threads}, 0, st>>>({ins}, {outs}, {sums}, {}, {}, {cols}, {});",
                parts.len(),
                parts.len(),
                normed.len(),
                n.opt(w),
                parts.len(),
                flit(eps.get() as f32)
            )
        }
        Launch::Quant { x, n: len } => {
            format!("lk_quant<{m}><<<{}, 256, 0, st>>>({}, {}, {len});", cdiv(*len, 256), p(x), n.quant(x, true))
        }
        // A gated delta layer's inputs in one launch: the GEMV in parts in blocks of eight warps, or of its normed heads'
        // width where that is more, `per_warp` rows a warp; then, where its gates project the activation in `f32`, the
        // blocks of `lk_gate_proj` after them.
        gdn @ Launch::Fold { .. } if gdn.gdn_inputs().is_some() => {
            let Some((Launch::GemmParts { x, format, k, parts, per_warp, .. }, Launch::ConvParts { parts: convs, taps, block, eps, .. }, gates)) = gdn.gdn_inputs() else {
                unreachable!("a gated delta layer's inputs")
            };
            let threads = (*block).max(256);
            let per_warp = launch_rows(*format, *per_warp, parts.iter().map(|q| q.n).sum(), numerics, n);
            // A 16-bit matrix's launch names its threads, the registers they may take (`lk_gemv_gdn`'s `NT`).
            let nt = if kernels::sixteen(*format) { format!(", {threads}") } else { String::new() };
            let cols: usize = parts.iter().map(|q| q.n).sum();
            let outs: Vec<PartOut> = parts.iter().map(|q| PartOut { w: handle(&q.w, *format, *k), dst: n.dst_split(&q.y, 0), bias: n.opt(&q.bias), n: q.n }).collect();
            let part_of = |t: &str| parts.iter().position(|q| q.y.own() == Some(t)).expect("a part's output") as i64;
            let conv = |c: Option<&super::plan::ConvPart>| match c {
                Some(c) => format!("lk_gdn_conv{{{}, {}, {}, {}, {}}}", part_of(&c.x), p(&c.w), p(&c.ring), p(&c.y), u8::from(c.l2)),
                None => "lk_gdn_conv{-1, nullptr, nullptr, nullptr, 0}".to_string(),
            };
            let none = || "nullptr".to_string();
            let (pb, pa, gb, gk, gparts, heads, wb, wa, xf, dt, al, beta, decay) = match gates {
                Launch::Gates { br, ar, dt, al, beta, decay, heads, .. } => (part_of(br), part_of(ar), 0, 0, 1, *heads, none(), none(), none(), p(dt), p(al), p(beta), p(decay)),
                Launch::GateProj { x: gx, wb, wa, dt, al, beta, decay, k: gk, heads, parts } => {
                    (-1, -1, cdiv(*heads, 8 / parts), *gk, *parts, *heads, p(wb), p(wa), p(gx), p(dt), p(al), p(beta), p(decay))
                }
                _ => unreachable!("the gates close a gated delta layer's inputs"),
            };
            let xin = activation_args(n, x, None, m, kernels::sixteen(*format));
            format!(
                "lk_gemv_gdn<{}, {m}, {per_warp}, {sum_ty}{nt}><<<{}, {threads}, 0, st>>>({xin}, {k}, {cols}, {}, lk_gdn{{{{{}, {}, {}}}, {taps}, {block}, {}, {pb}, {pa}, {gb}, {gk}, {gparts}, {heads}, {wb}, {wa}, {xf}, {dt}, {al}, {beta}, {decay}}}, {}, {});",
                kernels::format_arg(*format),
                cdiv(cols, threads / 32 * per_warp) + gb,
                lk_parts(&outs),
                conv(convs.first()),
                conv(convs.get(1)),
                conv(convs.get(2)),
                flit(eps.map_or(0.0, |e| e.get() as f32)),
                n.conv_pos(&convs[0].ring, taps * convs[0].chan, 1),
                n.scratch("gc", gdn.writes()[0])
            )
        }
        // A gather whose block norms the row it gathered, a block a row of the norm's threads or playing them.
        Launch::Fold { first, then } if matches!(first.as_ref(), Launch::Gather { .. }) => {
            let (Launch::Gather { table, index, out: raw, cols, rows, format, scale }, Launch::Norm { w, out, f32, quant, eps, scale: None, .. }) = (first.as_ref(), then.as_ref()) else {
                unreachable!("the lane norms a gathered row, unscaled, in its gather alone")
            };
            let (y, yrow, ypitch) = if *f32 { n.dst_split(&Dst::to(out), 0) } else { ("nullptr".into(), "nullptr".into(), 0) };
            let (threads, vw) = kernels::norm_block(*cols, *rows);
            let (w8, t) = match format {
                Some(ty) => (handle(table, *ty, *cols), "nullptr".to_string()),
                None => ("lk_w{}".to_string(), p(table)),
            };
            format!(
                "lk_gather_norm<{}, {}, {}, {}, {sum_ty}{}><<<{rows}, {threads}, 0, st>>>({w8}, {t}, {}, {}, {cols}, {}, {}, {y}, {}, {}, {yrow}, {ypitch});",
                format.map_or(0, kernels::format_arg),
                b(*f32),
                qm(*quant),
                cols.div_ceil(threads * vw),
                if vw == 1 { String::new() } else { format!(", {vw}") },
                p(index),
                p(raw),
                flit(scale.unwrap_or(1.0) as f32),
                n.opt(w),
                n.quant(out, *quant),
                flit(eps.get() as f32)
            )
        }
        // A projection whose last-arriving block norms the rows it wrote, a block of the norm's threads, a warp a row.
        // A projection in a block format whose last-arriving block norms the row it wrote, its warps playing the norm's.
        Launch::Fold { first, then } if matches!((first.as_ref(), then.as_ref()), (Launch::Gemv { format, .. }, Launch::Norm { .. }) if *format != GgmlType::Q8_0) => {
            let Launch::Gemv { x: ax, w, format, bias, epi: None, y: gy, k, n: cols, per_warp, .. } = first.as_ref() else { unreachable!("the lane folds a norm into a projection of no epilogue only") };
            let per_warp = &launch_rows(*format, *per_warp, *cols, numerics, n);
            let warps = kernels::gemv_blk_norm_threads(*per_warp) / 32;
            let (norm_t, nm) = norm_of(then, n, m, *cols, sum_ty);
            let xin = activation_args(n, ax, None, m, kernels::sixteen(*format));
            format!(
                "lk_gemv_blk_norm<{}, {m}, {per_warp}, {norm_t}, {}><<<{}, {}, 0, st>>>({}, {xin}, {}, {}, {k}, {cols}, {nm}, {}, {});",
                kernels::format_arg(*format),
                (kernels::norm_threads(*cols) / 32).div_ceil(warps),
                cdiv(*cols, warps * per_warp),
                32 * warps,
                handle(w, *format, *k),
                n.opt(bias),
                p(&gy.buf),
                kernels::norm_threads(*cols),
                n.scratch("gc", l.writes()[0])
            )
        }
        Launch::Fold { first, then } if matches!(then.as_ref(), Launch::Norm { .. }) => {
            let (Launch::Gemv { x: ax, w, bias, epi: None, y: gy, k, n: cols, m: rows, .. }, Launch::Norm { x, add, pre, sum, w: nw, out, into, f32, quant, eps, scale: None, .. }) =
                (first.as_ref(), then.as_ref())
            else {
                unreachable!("the lane folds an unscaled norm into a one-row projection only")
            };
            let own = Dst::to(out);
            let (y, yrow, ypitch) = if *f32 { n.dst_split(into.as_ref().unwrap_or(&own), 0) } else { ("nullptr".into(), "nullptr".into(), 0) };
            let threads = kernels::norm_threads(*cols);
            let (pre_arg, pre_tail) = match pre {
                Some(pn) => ("true", format!(", {}, {}", n.opt(&pn.w), flit(pn.eps.get() as f32))),
                None => ("false", String::new()),
            };
            format!(
                "lk_gemv_q8_norm<{m}, {rows}, {}, {}, {}, {}, {}, {sum_ty}, {pre_arg}><<<{}, {threads}, 0, st>>>({}, {}, {}, {}, {}, {}, {k}, {cols}, {}, {}, {}, {}, {y}, {yrow}, {ypitch}, {}, {}, {}{pre_tail});",
                b(add.is_some()),
                b(sum.is_some()),
                b(*f32),
                qm(*quant),
                cols.div_ceil(threads),
                cdiv(*cols, threads / 32),
                rows_q(w),
                rows_s(w),
                if m == 0 { p(ax) } else { "nullptr".into() },
                n.quant(ax, m > 0),
                n.opt(bias),
                p(&gy.buf),
                p(x),
                n.opt(add),
                n.opt(sum),
                n.opt(nw),
                n.quant(out, *quant),
                flit(eps.get() as f32),
                n.scratch("gc", l.writes()[0])
            )
        }
        // A batch's projections over Q4_K and Q6_K rows on the tensor cores in their rows' own order (`lk_gemv_mma_blk`);
        // a gated pair as its gate and up apart, each in its own format, then gated as the pair's own epilogue gates them.
        Launch::Gemv { x, w, format, bias, epi, y, k, n: cols, m: rows, .. } if l.batch_blk_format(numerics).is_some() => {
            let part = [PartOut { w: handle(w, *format, *k), dst: n.dst_split(y, 0), bias: n.epi(bias, epi, 0, *cols), n: *cols }];
            gemv_mma_blk(n, x, *k, *rows, &part, *format)
        }
        Launch::GemmParts { x, format, k, m: rows, parts, .. } if l.batch_blk_format(numerics).is_some() => {
            let parts: Vec<PartOut> = parts
                .iter()
                .map(|p| PartOut { w: handle(&p.w, *format, *k), dst: n.dst_split(&p.y, 0), bias: n.opt(&p.bias), n: p.n })
                .collect();
            gemv_mma_blk(n, x, *k, *rows, &parts, *format)
        }
        Launch::GemvGlu { x, wg, wu, formats: [fg, fu], biases: [bg, bu], act, out, f32, quant, k, n: cols, m: rows, .. } if l.batch_blk_format(numerics).is_some() => {
            let (hg, hu) = (n.scratch("mg", out), n.scratch("mu", out));
            let scratch = |w: &str, f: GgmlType, s: &str, bias: &Option<String>| PartOut {
                w: handle(w, f, *k),
                dst: (s.to_string(), "nullptr".to_string(), 0),
                bias: n.opt(bias),
                n: *cols,
            };
            let products = if fg == fu {
                gemv_mma_blk(n, x, *k, *rows, &[scratch(wg, *fg, &hg, bg), scratch(wu, *fu, &hu, bu)], *fg)
            } else {
                format!(
                    "{}\n  CKL();\n  {}",
                    gemv_mma_blk(n, x, *k, *rows, &[scratch(wg, *fg, &hg, bg)], *fg),
                    gemv_mma_blk(n, x, *k, *rows, &[scratch(wu, *fu, &hu, bu)], *fu)
                )
            };
            format!("{products}\n  CKL();\n  {}", glu_gate(n, m, &hg, &hu, act, out, *f32, *quant, rows * cols))
        }
        // A batch's projections on the tensor cores in their rows' own order (`lk_gemv_mma`), up to `GEMV_MMA_ROWS` rows a
        // launch; a gated pair as its gate and up apart, then gated as the pair's own epilogue gates them.
        Launch::Gemv { x, w, bias, epi, y, k, n: cols, m: rows, .. } if l.batch_on_tensor_cores(numerics) => {
            let part = [PartOut { w: handle(w, GgmlType::Q8_0, *k), dst: n.dst_split(y, 0), bias: n.opt(bias), n: *cols }];
            gemv_mma(n, x, *k, *rows, &part, m, Some(&|at| n.epi(bias, epi, at, *cols)))
        }
        Launch::GemmParts { x, k, m: rows, parts, .. } if l.batch_on_tensor_cores(numerics) => {
            let parts: Vec<PartOut> = parts
                .iter()
                .map(|p| PartOut { w: handle(&p.w, GgmlType::Q8_0, *k), dst: n.dst_split(&p.y, 0), bias: n.opt(&p.bias), n: p.n })
                .collect();
            gemv_mma(n, x, *k, *rows, &parts, m, None)
        }
        Launch::GemvGlu { x, wg, wu, biases: [bg, bu], act, out, f32, quant, k, n: cols, m: rows, .. } if l.batch_on_tensor_cores(numerics) => {
            let (hg, hu) = (n.scratch("mg", out), n.scratch("mu", out));
            let scratch = |w: &str, s: &str, bias: &Option<String>| PartOut {
                w: handle(w, GgmlType::Q8_0, *k),
                dst: (s.to_string(), "nullptr".to_string(), 0),
                bias: n.opt(bias),
                n: *cols,
            };
            let parts = [scratch(wg, &hg, bg), scratch(wu, &hu, bu)];
            format!("{}\n  CKL();\n  {}", gemv_mma(n, x, *k, *rows, &parts, m, None), glu_gate(n, m, &hg, &hu, act, out, *f32, *quant, rows * cols))
        }
        Launch::Gemv { x, w, format, bias, epi, y, k, n: cols, m: rows, .. } if l.on_tensor_cores(numerics) && l.reads_sixteen() => format!(
            "{}<<<dim3({}, {}), 256, {}, st>>>({}, {}, {}, {}, {k}, {cols}, {rows});",
            gemm_h16_name(kernels::format_arg(*format), m, 0, 0, h16_launch_tile(l, numerics, n.sms)),
            cdiv(*cols, h16_launch_tile(l, numerics, n.sms)),
            cdiv(*rows, kernels::GEMM_H16_ROWS),
            kernels::gemm_h16_smem_bytes(numerics == Q8MatKind::TwoPassInt8, h16_launch_tile(l, numerics, n.sms)),
            handle(w, *format, *k),
            p(x),
            n.epi(bias, epi, 0, *cols),
            n.dst(y)
        ),
        Launch::GemmParts { x, format, k, m: rows, parts, .. } if l.on_tensor_cores(numerics) && l.reads_sixteen() => {
            let (f, tn) = (kernels::format_arg(*format), h16_tile(numerics));
            let launch = |group: u32, cols: usize, args: String| {
                format!(
                    "{}<<<dim3({}, {}), 256, {}, st>>>({args});",
                    gemm_h16_name(f, m, group, 0, tn),
                    cdiv(cols, tn),
                    cdiv(*rows, kernels::GEMM_H16_ROWS),
                    kernels::gemm_h16_smem_bytes(numerics == Q8MatKind::TwoPassInt8, h16_tile(numerics))
                )
            };
            if h16_parted(parts, numerics) {
                // Whole tiles a part: one launch over the parts side by side.
                let outs: Vec<PartOut> = parts.iter().map(|p| PartOut { w: handle(&p.w, *format, *k), dst: n.dst_split(&p.y, 0), bias: n.opt(&p.bias), n: p.n }).collect();
                let cols = l.gemm_cols();
                launch(4, cols, format!("{{}}, {}, {{}}, nullptr, nullptr, 0, {k}, {cols}, {rows}, {{}}, nullptr, {}", p(x), lk_parts(&outs)))
            } else {
                // A part a launch, each summed as the parts' one launch sums it.
                parts
                    .iter()
                    .map(|q| launch(0, q.n, format!("{}, {}, {}, {}, {k}, {}, {rows}", handle(&q.w, *format, *k), p(x), n.opt(&q.bias), n.dst(&q.y), q.n)))
                    .collect::<Vec<_>>()
                    .join("\n  CKL();\n  ")
            }
        }
        Launch::GemvGlu { x, wg, wu, formats: [fg, _], biases: [bg, bu], act, out, k, n: cols, m: rows, .. } if l.on_tensor_cores(numerics) && l.reads_sixteen() => format!(
            "{}<<<dim3({}, {}), 256, {}, st>>>({}, {}, {}, {}, nullptr, 0, {k}, {cols}, {rows}, {}, {});",
            gemm_h16_name(kernels::format_arg(*fg), m, 5, act.map_or(-1, act_arg), h16_launch_tile(l, numerics, n.sms)),
            cdiv(*cols, h16_launch_tile(l, numerics, n.sms) / 2),
            cdiv(*rows, kernels::GEMM_H16_ROWS),
            kernels::gemm_h16_smem_bytes(numerics == Q8MatKind::TwoPassInt8, h16_launch_tile(l, numerics, n.sms)),
            handle(wg, *fg, *k),
            p(x),
            n.opt(bg),
            p(out),
            handle(wu, *fg, *k),
            n.opt(bu)
        ),
        Launch::Gemv { x, w, format, bias, epi, y, k, n: cols, m: rows, tail, .. } if l.on_tensor_cores(numerics) => {
            let bm = l.gemm_rows(numerics);
            let tiles = cdiv(*cols, kernels::GEMM_TILE_N) * cdiv(*rows, bm);
            // A line of blocks when the last wave cuts its tiles along `k`, the tiles' grid otherwise.
            let (grid, line) = match tail {
                Some((whole, split)) => (
                    format!("{}", whole + (tiles - whole) * split),
                    format!(
                        ", nullptr, nullptr, nullptr, nullptr, {{}}, nullptr, nullptr, nullptr, nullptr, 0.f, 0.f, {whole}, {split}, {}, {}",
                        n.scratch("tp", &y.buf),
                        n.scratch("tc", &y.buf)
                    ),
                ),
                None => (format!("dim3({}, {})", cdiv(*cols, kernels::GEMM_TILE_N), cdiv(*rows, bm)), String::new()),
            };
            format!(
                "{}<<<{grid}, {}, {}, st>>>({}, {}, {}, {}, {k}, {cols}, {rows}{line});",
                gemm_blk_name(kernels::format_arg(*format), m, 0, bm, 0, blk_plain(l, n.buffers)),
                kernels::gemm_blk_launch_threads(kernels::format_arg(*format), bm, *k),
                kernels::gemm_blk_launch_smem(kernels::format_arg(*format), m == 2, bm, *k),
                handle(w, *format, *k),
                n.quant(x, true),
                n.epi(bias, epi, 0, *cols),
                n.dst(y)
            )
        }
        Launch::GemmParts { x, format, k, m: rows, parts, tail, .. } if l.on_tensor_cores(numerics) => {
            let bm = l.gemm_rows(numerics);
            let cols = l.gemm_cols();
            let tiles = cdiv(cols, kernels::GEMM_TILE_N) * cdiv(*rows, bm);
            let key = &parts[0].y.buf;
            let (grid, line) = match tail {
                Some((whole, split)) => (format!("{}", whole + (tiles - whole) * split), format!("{whole}, {split}, {}, {}", n.scratch("tp", key), n.scratch("tc", key))),
                None => (format!("dim3({}, {})", cdiv(cols, kernels::GEMM_TILE_N), cdiv(*rows, bm)), "0, 1, nullptr, nullptr".to_string()),
            };
            let outs: Vec<PartOut> = parts
                .iter()
                .map(|p| PartOut { w: handle(&p.w, *format, *k), dst: n.dst_split(&p.y, 0), bias: n.opt(&p.bias), n: p.n })
                .collect();
            format!(
                "{}<<<{grid}, {}, {}, st>>>({{}}, {}, nullptr, nullptr, nullptr, 0, {k}, {cols}, {rows}, nullptr, nullptr, nullptr, nullptr, {{}}, nullptr, nullptr, nullptr, nullptr, 0.f, 0.f, {line}, {});",
                gemm_blk_name(kernels::format_arg(*format), m, 4, bm, 0, blk_plain(l, n.buffers)),
                kernels::gemm_blk_launch_threads(kernels::format_arg(*format), bm, *k),
                kernels::gemm_blk_launch_smem(kernels::format_arg(*format), m == 2, bm, *k),
                n.quant(x, true),
                lk_parts(&outs)
            )
        }
        Launch::GemmParts { x, format, k, m: rows, parts, per_warp, .. } => {
            let cols = l.gemm_cols();
            let per_warp = &launch_rows(*format, *per_warp, cols, numerics, n);
            // A batch's rows past the GEMV's in slabs of `GEMV_ROWS`, each launch's activation and output rows the slab's.
            (0..rows.div_ceil(GEMV_ROWS))
                .map(|slab| {
                    let r = GEMV_ROWS.min(rows - slab * GEMV_ROWS);
                    let at = slab * GEMV_ROWS * k;
                    let outs: Vec<PartOut> = parts
                        .iter()
                        .map(|p| PartOut {
                            w: handle(&p.w, *format, *k),
                            dst: n.dst_split(&p.y, slab * GEMV_ROWS * p.n),
                            bias: n.opt(&p.bias),
                            n: p.n,
                        })
                        .collect();
                    let xin = activation_args(n, x, Some(at), m, kernels::sixteen(*format));
                    // Eight warps a block, as a part's own GEMV takes its rows.
                    format!(
                        "lk_gemv_parts<{}, {m}, {r}{}><<<{}, 256, 0, st>>>({xin}, {k}, {cols}, {});",
                        kernels::format_arg(*format),
                        per_warp_arg(*per_warp),
                        cdiv(cols, 8 * *per_warp),
                        lk_parts(&outs)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n  CKL();\n  ")
        }
        Launch::Gemv { x, w, format, bias, epi, y, k, n: cols, m: rows, per_warp, f32, quant, .. } if *format != GgmlType::Q8_0 => (0..rows.div_ceil(GEMV_ROWS))
            .map(|slab| {
                let r = GEMV_ROWS.min(rows - slab * GEMV_ROWS);
                let (at, oat) = (slab * GEMV_ROWS * k, slab * GEMV_ROWS * cols);
                let xin = activation_args(n, x, Some(at), m, kernels::sixteen(*format));
                let rows_a_warp = launch_rows(*format, *per_warp, *cols, numerics, n);
                // A quantized output takes a block of 32 outputs; a plain one, four warps of four rows or eight of one, a
                // 16-bit matrix's on the tensor cores as few as still keep every multiprocessor its blocks.
                let warps = match (*quant, rows_a_warp) {
                    (true, w) => 32 / w,
                    (false, _) if kernels::sixteen(*format) && numerics != Q8MatKind::Dequant => kernels::SIXTEEN_BLOCK_WARPS,
                    (false, 1) => 8,
                    (false, _) => 4,
                };
                let dst = if *f32 { n.dst_at(y, oat) } else { "nullptr, nullptr, 0".into() };
                let yq = if *quant { n.quant_at(&y.buf, oat) } else { "nullptr, nullptr, nullptr".into() };
                format!(
                    "lk_gemv_blk<{}, {m}, {r}{}><<<{}, {}, 0, st>>>({}, {xin}, {}, {dst}, {k}, {cols}, {yq});",
                    kernels::format_arg(*format),
                    match (rows_a_warp, *quant) {
                        (1, false) => String::new(),
                        (w, q) => format!(", {w}, {}", qm(q)),
                    },
                    cdiv(*cols, warps * rows_a_warp),
                    32 * warps,
                    handle(w, *format, *k),
                    n.epi(bias, epi, slab * GEMV_ROWS, *cols)
                )
            })
            .collect::<Vec<_>>()
            .join("\n  CKL();\n  "),
        Launch::Gemv { x, w, bias, epi, y, k, n: cols, m: rows, .. } if *rows > GEMV_ROWS => (0..rows.div_ceil(GEMV_ROWS))
            .map(|slab| {
                let r = GEMV_ROWS.min(rows - slab * GEMV_ROWS);
                format!(
                    "lk_gemv_q8<0, {r}><<<{}, 256, 0, st>>>({}, {}, {} + {}, nullptr, nullptr, nullptr, {}, {}, {k}, {cols});",
                    cdiv(*cols, 8),
                    rows_q(w),
                    rows_s(w),
                    p(x),
                    slab * GEMV_ROWS * k,
                    n.epi(bias, epi, slab * GEMV_ROWS, *cols),
                    n.dst_at(y, slab * GEMV_ROWS * cols)
                )
            })
            .collect::<Vec<_>>()
            .join("\n  CKL();\n  "),
        Launch::Gemv { x, w, bias, epi, y, k, n: cols, m: rows, .. } => format!(
            "lk_gemv_q8<{m}, {rows}><<<{}, 256, 0, st>>>({}, {}, {}, {}, {}, {}, {k}, {cols});",
            cdiv(*cols, 8),
            rows_q(w),
            rows_s(w),
            if m == 0 { p(x) } else { "nullptr".into() },
            n.quant(x, m > 0),
            n.epi(bias, epi, 0, *cols),
            n.dst(y)
        ),
        Launch::GemvF32 { x, w, bias, epi, y, k, n: cols, m: rows, split, .. } if l.multiplies_in_tiles(numerics) => format!(
            "lk_gemm_f32<<<dim3({}, {}, {split}), 256, 0, st>>>({}, {}, {}, {}, {rows}, {cols}, {k}{});",
            cdiv(*cols, 64),
            cdiv(*rows, 64),
            p(x),
            p(w),
            n.epi(bias, epi, 0, *cols),
            n.dst_ptr(y),
            if *split > 1 { format!(", {split}, {}, {}", n.scratch("fp", &y.buf), n.scratch("fq", &y.buf)) } else { String::new() }
        ),
        Launch::GemvF32 { x, w, bias, epi, y, k, n: cols, m: rows, split, .. } => format!(
            "lk_gemv_f32<<<dim3({}, {}), 256, 0, st>>>({}, {}, {}, {}, {k}, {cols}, {rows}{});",
            cdiv(*cols, 8 / split),
            cdiv(*rows, kernels::GEMV_F32_ROWS),
            p(w),
            p(x),
            n.epi(bias, epi, 0, *cols),
            n.dst(y),
            if *split > 1 { format!(", {split}") } else { String::new() }
        ),
        Launch::Relu { x, out, n: len, square, f32, quant } => format!(
            "lk_relu<{}, {}, {}><<<{}, 256, 0, st>>>({}, {}, {}, {len});",
            b(*square),
            b(*f32),
            qm(*quant),
            cdiv(*len, 256),
            p(x),
            if *f32 { p(out) } else { "nullptr".into() },
            n.quant(out, *quant)
        ),
        Launch::Binary { op, a, b: bb, gate, row, out, f32, quant, n: len } => format!(
            "lk_binary<{}, {}, {}, {}><<<{}, 256, 0, st>>>({}, {}, {}, {}, {len}, {});",
            match op {
                ElementwiseOp::Add => 0,
                ElementwiseOp::Mul => 1,
            },
            gate.map_or(-1, act_arg),
            b(*f32),
            qm(*quant),
            cdiv(*len, 256),
            p(a),
            p(bb),
            if *f32 { p(out) } else { "nullptr".into() },
            n.quant(out, *quant),
            row.unwrap_or(*len)
        ),
        Launch::SwiGlu { g, u, out, f32, quant, n: len } => format!(
            "lk_swiglu<{}, {}><<<{}, 256, 0, st>>>({}, {}, {}, {}, {len});",
            b(*f32),
            qm(*quant),
            cdiv(*len, 256),
            p(g),
            p(u),
            if *f32 { p(out) } else { "nullptr".into() },
            n.quant(out, *quant)
        ),
        // A block of activation rows in slabs of `GEMV_ROWS`, each launch's activation and output rows the slab's.
        Launch::GemvGlu { x, wg, wu, formats: [fg, _], biases: [bg, bu], act, out, f32, quant, k, n: cols, m: rows, tail, .. } if l.on_tensor_cores(numerics) => {
            let bm = l.gemm_rows(numerics);
            let tw = kernels::GEMM_TILE_N / 2;
            let tiles = cdiv(*cols, tw) * cdiv(*rows, bm);
            let (grid, line) = match tail {
                Some((whole, split)) => (format!("{}", whole + (tiles - whole) * split), format!("{whole}, {split}, {}, {}", n.scratch("tp", out), n.scratch("tc", out))),
                None => (format!("dim3({}, {})", cdiv(*cols, tw), cdiv(*rows, bm)), "0, 1, nullptr, nullptr".to_string()),
            };
            format!(
                "{}<<<{grid}, {}, {}, st>>>({}, {}, {}, {}, nullptr, 0, {k}, {cols}, {rows}, nullptr, nullptr, nullptr, nullptr, {}, {}, {}, 0.f, 0.f, {line});",
                gemm_blk_name(kernels::format_arg(*fg), m, 5, bm, act.map_or(-1, act_arg), false),
                kernels::gemm_blk_launch_threads(kernels::format_arg(*fg), bm, *k),
                kernels::gemm_blk_launch_smem(kernels::format_arg(*fg), m == 2, bm, *k),
                handle(wg, *fg, *k),
                n.quant(x, true),
                n.opt(bg),
                if *f32 { p(out) } else { "nullptr".into() },
                handle(wu, *fg, *k),
                n.opt(bu),
                if *quant { n.quant(out, true) } else { "nullptr, nullptr, nullptr".into() }
            )
        }
        Launch::GemvGlu { x, wg, wu, formats: [fg, fu], biases: [bg, bu], act, out, f32, quant, k, n: cols, m: rows, per_warp, .. } => (0..rows.div_ceil(GEMV_ROWS))
            .map(|slab| {
                let r = GEMV_ROWS.min(rows - slab * GEMV_ROWS);
                let (at, oat) = (slab * GEMV_ROWS * k, slab * GEMV_ROWS * cols);
                let xin = activation_args(n, x, Some(at), m, kernels::sixteen(*fg) || kernels::sixteen(*fu));
                let y = if *f32 { format!("{} + {oat}", p(out)) } else { "nullptr".into() };
                let yq = if *quant { n.quant_at(out, oat) } else { "nullptr, nullptr, nullptr".into() };
                let rows_a_warp = launch_rows(if kernels::sixteen(*fg) { *fg } else { *fu }, *per_warp, *cols, numerics, n);
                // A quantized output takes a block of 32 outputs; a plain one, eight warps, a 16-bit pair's on the tensor
                // cores one (`kernels::SIXTEEN_BLOCK_WARPS`).
                let warps = match *quant {
                    true => 32 / rows_a_warp,
                    false if kernels::sixteen(*fg) && numerics != Q8MatKind::Dequant => kernels::SIXTEEN_BLOCK_WARPS,
                    false => 8,
                };
                format!(
                    "lk_gemv_glu<{}, {}, {m}, {r}, {}, {}, {}{}><<<{}, {}, 0, st>>>({}, {}, {xin}, {}, {}, {y}, {yq}, {k}, {cols});",
                    kernels::format_arg(*fg),
                    kernels::format_arg(*fu),
                    act.map_or(-1, act_arg),
                    b(*f32),
                    qm(*quant),
                    per_warp_arg(rows_a_warp),
                    cdiv(*cols, warps * rows_a_warp),
                    32 * warps,
                    handle(wg, *fg, *k),
                    handle(wu, *fu, *k),
                    n.opt(bg),
                    n.opt(bu)
                )
            })
            .collect::<Vec<_>>()
            .join("\n  CKL();\n  "),
        Launch::Unary { act, x, y, n: len } => format!(
            "lk_unary<{}><<<{}, 256, 0, st>>>({}, {}, {len});",
            act_arg(*act),
            cdiv(*len, 256),
            p(x),
            p(y)
        ),
        Launch::Rope { x, part, norm: None, cos, sin, pos, y, heads, per_row, hd, half } => {
            let ((xp, xo), (yp, yo)) = (reading(part, *hd), unit(y, *hd));
            format!(
                "lk_rope_pairs<<<{}, 256, 0, st>>>({}, {}, {}, {}, {}, {hd}, {half}, {per_row}, {heads}, {xp}, {xo}, {yp}, {yo});",
                cdiv(heads * (hd - half), 256),
                p(x),
                p(cos),
                p(sin),
                n.opt(pos),
                n.dst(y)
            )
        }
        Launch::Rope { x, part, norm, cos, sin, pos, y, heads, per_row, hd, half } => {
            let ((xp, xo), (yp, yo)) = (reading(part, *hd), unit(y, *hd));
            let threads = rope_threads(norm.is_some(), *hd);
            format!(
                "lk_rope<{}, {sum_ty}><<<{heads}, {threads}, 0, st>>>({}, {}, {}, {}, {}, {}, {hd}, {half}, {per_row}, {}, {xp}, {xo}, {yp}, {yo});",
                b(norm.is_some()),
                p(x),
                norm.as_ref().map_or_else(|| "nullptr".to_string(), |(w, _)| p(w)),
                p(cos),
                p(sin),
                n.opt(pos),
                n.dst(y),
                flit(norm.as_ref().map_or(0.0, |(_, e)| e.get() as f32))
            )
        }
        Launch::RopeParts { sides, cos, sin, pos, hd, half, seqs } => {
            let normed = sides[0].norm.is_some();
            format!(
                "lk_rope_parts<{}, {sum_ty}><<<{}, {}, 0, st>>>({}, {}, {}, {}, {}, {hd}, {half}, {});",
                b(normed),
                sides.iter().map(|r| r.heads).sum::<usize>(),
                rope_threads(normed, *hd),
                rope_side(&sides[0], *hd, n),
                rope_side(&sides[1], *hd, n),
                p(cos),
                p(sin),
                n.opt(pos),
                u8::from(*seqs)
            )
        }
        // Projections in parts whose blocks turn the heads their rows finish: a block of the rotation's threads or eight
        // warps, whichever is more — at least a normed head wide, as the rotation's own — `per_warp` rows a warp; each side
        // reads the part whose output is its input.
        Launch::Fold { first, then } => {
            let (Launch::GemmParts { x, format, k, parts, per_warp, .. }, Launch::RopeParts { sides, cos, sin, pos, hd, half, .. }) = (first.as_ref(), then.as_ref()) else {
                unreachable!("the lane turns heads only in a GEMV in parts")
            };
            let normed = sides[0].norm.is_some();
            let threads = rope_threads(normed, *hd).max(256);
            let nt = if kernels::sixteen(*format) { format!(", {threads}") } else { String::new() };
            let per_warp = &launch_rows(*format, *per_warp, first.gemm_cols(), numerics, n);
            let outs: Vec<PartOut> = parts.iter().map(|q| PartOut { w: handle(&q.w, *format, *k), dst: n.dst_split(&q.y, 0), bias: n.opt(&q.bias), n: q.n }).collect();
            let read = |side: Option<&RopeSide>| side.map_or(-1, |r| parts.iter().position(|q| q.y.own() == Some(r.x.as_str())).expect("a side reads a part") as i64);
            let none = RopeSide { x: sides[0].x.clone(), part: None, norm: sides[0].norm.clone(), y: sides[0].y.clone(), heads: 0, per_row: 1, yz: 0 };
            let xin = activation_args(n, x, None, m, kernels::sixteen(*format));
            format!(
                "lk_gemv_parts_rope<{}, {m}, 1, {per_warp}, {}, {sum_ty}{nt}><<<{}, {threads}, 0, st>>>({xin}, {k}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {hd}, {half}, {});",
                kernels::format_arg(*format),
                b(normed),
                cdiv(first.gemm_cols(), threads / 32 * per_warp),
                first.gemm_cols(),
                lk_parts(&outs),
                rope_side(&sides[0], *hd, n),
                rope_side(sides.get(1).unwrap_or(&none), *hd, n),
                read(sides.first()),
                read(sides.get(1)),
                p(cos),
                p(sin),
                n.opt(pos),
                n.scratch("gc", l.writes()[0])
            )
        }
        Launch::SetRow { table, index, src, cols, count, seq, f32, quant } => {
            let (idx, fixed) = match index {
                RowSel::Param(i) => (p(i), 0),
                RowSel::Fixed(r) => ("nullptr".to_string(), *r),
            };
            let (blocks, n_set, tz) = (cdiv(count * cols, 256), count * cols, seq.unwrap_or(0));
            let mut sets = Vec::new();
            if *f32 {
                sets.push(format!("lk_set_row<<<{blocks}, 256, 0, st>>>({}, {idx}, {fixed}, {}, {n_set}, {cols}, {tz});", p(table), p(src)));
            }
            if *quant {
                sets.push(format!(
                    "lk_set_row_q<<<{blocks}, 256, 0, st>>>({}, {idx}, {fixed}, {}, {n_set}, {cols}, {tz});",
                    n.quant(table, true),
                    n.quant(src, true)
                ));
            }
            sets.join("\n  CKL();\n  ")
        }
        Launch::Attention { q, kc, vc, pos, sinks, out, heads, group, hd, vd, len, rows, window, chunked, vstride, voff, scale, cap, split, stream, .. } if l.attends_in_tiles(numerics) => {
            let hg = kernels::attn_tile_heads(*group);
            let (mma, flash) = (l.attends_on_tensor_cores(numerics), l.attends_flash(numerics));
            let kv = n.elems(&[Some(kc), Some(vc), split.as_ref().map(|s| s.k_tail.0.as_str())]);
            // A split query and key: the query's rest and the key row's rest, and where each splits.
            let parts = split.as_ref().map_or(String::new(), |s| {
                format!(
                    ", {}, {}, {}, {}, {}, {}, {}",
                    p(&s.q_tail.0),
                    s.q_pitch,
                    s.q_tail.1.pitch,
                    p(&s.k_tail.0),
                    s.k_tail.1.pitch,
                    s.k_tail.1.offset,
                    s.own
                )
            });
            // `kt` is a `const TE*`: its default typed as `TE`'s own default, `float`, since a bare `nullptr` deduces no `TE`.
            let tail = format!("{parts}{}", capped(*cap, &parts, ", nullptr, 0, 0, (const float*)nullptr, 0, 0, 0"));
            if l.attends_in_two_words(numerics) {
                // Every key and value row the chunk attends into two words, then the attention over them.
                let (lpad, kvh) = (kernels::attn_i8_rows(*len), heads / group);
                let (k8, w8, s8) = (n.scratch("ak", out), n.scratch("aw", out), n.scratch("as", out));
                let (kparts, qparts) = split.as_ref().map_or((String::new(), String::new()), |s| {
                    (
                        format!(", {}, {}, {}, {}", p(&s.k_tail.0), s.k_tail.1.pitch, s.k_tail.1.offset, s.own),
                        format!(", {}, {}, {}, {}", p(&s.q_tail.0), s.q_pitch, s.q_tail.1.pitch, s.own),
                    )
                });
                let qparts = format!("{qparts}{}", capped(*cap, &qparts, ", nullptr, 0, 0, 0"));
                format!(
                    "lk_kv_two_words<{hd}, {vd}{kv}><<<dim3({}, {kvh}), 256, 0, st>>>({}, {}, {}, {rows}, {len}, {}, {vstride}, {voff}, {lpad}, \
                     {k8}, {k8} + {kb}, {s8}, {w8}, {w8} + {vb}, {s8} + {sb}{kparts});\n  CKL();\n  \
                     lk_attention_i8<{hg}, {hd}, {vd}><<<dim3({}, {}), 128, {}, st>>>({}, {k8}, {k8} + {kb}, {s8}, {w8}, {w8} + {vb}, {s8} + {sb}, {lpad}, \
                     {}, {}, {}, {len}, {}, {group}, {heads}, {}, {rows}{qparts});",
                    lpad / 32,
                    p(kc),
                    p(vc),
                    n.opt(pos),
                    heads / group * split.as_ref().map_or(*hd, |s| s.own),
                    rows.div_ceil(kernels::ATTN_I8_QUERIES / hg),
                    heads / hg,
                    kernels::attn_i8_smem_bytes(*hd, *vd),
                    p(q),
                    n.opt(pos),
                    n.opt(sinks),
                    p(out),
                    win(*window, *chunked),
                    flit(*scale as f32),
                    kb = kvh * lpad * hd,
                    vb = kvh * lpad * vd,
                    sb = kvh * lpad,
                )
            } else if mma {
                let te = split.as_ref().map_or(kc.as_str(), |s| s.k_tail.0.as_str());
                let kernel = attention_mma_kernel(hg, *hd, *vd, &kv, [n.elem(kc), n.elem(vc), n.elem(te)], *stream);
                // Under the stream-K schedule: a grid of the items past the whole waves' blocks behind those split over
                // the chip, past the rows its partials and their counts (where it splits) and the items a block each,
                // then the split's parts and the cap as one block an item takes them, the split keys' rest typed as
                // the kernel's.
                let (grid, tail) = if *stream > 0 {
                    let (ndp, blocks) = kernels::attn_mma_schedule(kernels::attn_mma_items(*rows, *heads, hg), *stream);
                    let (part, cnt) = if blocks > ndp { (n.scratch("ap", out), n.scratch("ac", out)) } else { ("nullptr".into(), "nullptr".into()) };
                    let typed = format!(", nullptr, 0, 0, (const {}*)nullptr, 0, 0, 0", n.elem(te));
                    (blocks.to_string(), format!(", lk_amma_split{{{part}, {cnt}, {ndp}}}{parts}{}", capped(*cap, &parts, &typed)))
                } else {
                    (format!("dim3({}, {})", rows.div_ceil(kernels::ATTN_MMA_QUERIES / hg), heads / hg), tail)
                };
                format!(
                    "{kernel}<<<{grid}, 128, {}, st>>>({}, {}, {}, {}, {}, {}, {len}, {}, {group}, {heads}, {}, {vstride}, {voff}, {}, {rows}{tail});",
                    kernels::attn_mma_smem_bytes(*hd, *vd),
                    p(q),
                    p(kc),
                    p(vc),
                    n.opt(pos),
                    n.opt(sinks),
                    p(out),
                    win(*window, *chunked),
                    heads / group * split.as_ref().map_or(*hd, |s| s.own),
                    flit(*scale as f32)
                )
            } else if flash {
                let (bq, nt) = (kernels::attn_flash_queries(*hd, *vd), kernels::attn_flash_threads(*hd, *vd));
                format!(
                    "lk_attention_flash<{hg}, {hd}, {vd}, {bq}, {nt}{kv}><<<dim3({}, {}), {nt}, {}, st>>>({}, {}, {}, {}, {}, {}, {len}, {}, {group}, {heads}, {}, {vstride}, {voff}, {}, {rows}{tail});",
                    rows.div_ceil(bq / hg),
                    heads / hg,
                    kernels::attn_flash_smem_bytes(*hd, *vd),
                    p(q),
                    p(kc),
                    p(vc),
                    n.opt(pos),
                    n.opt(sinks),
                    p(out),
                    win(*window, *chunked),
                    heads / group * split.as_ref().map_or(*hd, |s| s.own),
                    flit(*scale as f32)
                )
            } else {
                format!(
                    "lk_attention_tiled<{hg}, {}{kv}><<<dim3({}, {}), 256, {}, st>>>({}, {}, {}, {}, {}, {}, {len}, {}, {hd}, {vd}, {group}, {heads}, {}, {vstride}, {voff}, {}, {rows}{tail});",
                    kernels::attn_tile_values(*vd),
                    rows.div_ceil(kernels::ATTN_TILE_Q / hg),
                    heads / hg,
                    kernels::attn_tile_smem_bytes(*vd),
                    p(q),
                    p(kc),
                    p(vc),
                    n.opt(pos),
                    n.opt(sinks),
                    p(out),
                    win(*window, *chunked),
                    heads / group * split.as_ref().map_or(*hd, |s| s.own),
                    flit(*scale as f32)
                )
            }
        }
        // Each chunk a block, every head of the group in it; more than one chunk, and a launch folds them.
        Launch::Attention { q, kc, vc, pos, sinks, out, heads, group, hd, vd, len, rows, window, chunked, vstride, voff, scale, cap, shared: Some(splits), seq, f32, quant, gate, goff, .. } => {
            let part = if *splits > 1 { n.scratch("ap", out) } else { "nullptr".into() };
            let (win, minc) = (win(*window, *chunked), kernels::ATTN_MIN_SPLIT);
            let (qo, qargs) = attention_quant(n, out, *quant, gate, *goff, m);
            let kernel = attention_shared_kernel(*heads, *vd, *hd, kc == vc, &n.elems(&[Some(kc), Some(vc)]), &qo);
            let dst = if *f32 { p(out) } else { "nullptr".into() };
            let capq = capped(*cap, &qargs, ", lk_qout{}");
            let attend = format!(
                "{kernel}<<<dim3({}, {rows}, {splits}), 256, {}, st>>>({}, {}, {}, {}, {}, {dst}, {part}, {len}, {win}, {vd}, {heads}, {}, {vstride}, {voff}, {}, {minc}, {splits}, {}{qargs}{capq});",
                heads / group,
                kernels::attn_shared_smem_bytes(*heads, *hd),
                p(q),
                p(kc),
                p(vc),
                n.opt(pos),
                n.opt(sinks),
                heads / group * hd,
                flit(*scale as f32),
                seq.unwrap_or(0)
            );
            if *splits == 1 {
                attend
            } else {
                let fold = if qo.is_empty() { "lk_attention_fold".to_string() } else { format!("lk_attention_fold<{}>", if *quant { m } else { 0 }) };
                format!(
                    "{attend}\n  CKL();\n  {fold}<<<dim3({heads}, {rows}, {}), {}, {}, st>>>({part}, {}, {}, {dst}, {len}, {win}, {vd}, {heads}, {minc}, {splits}, {}{qargs});",
                    cdiv(*vd, 2 * kernels::ATTN_FOLD_PAIRS),
                    kernels::ATTN_FOLD_PAIRS,
                    splits * 4,
                    n.opt(pos),
                    n.opt(sinks),
                    u8::from(seq.is_some())
                )
            }
        }
        Launch::Attention { q, kc, vc, pos, sinks, out, heads, group, hd, vd, len, rows, window, chunked, vstride, voff, scale, cap, per_block, splits, parallel, split, shared: None, seq, f32, quant, gate, goff, stream: _ } => {
            assert!(split.is_none(), "a split attention attends in tiles");
            let (part, cnt) = if *parallel { (n.scratch("ap", out), n.scratch("ac", out)) } else { ("nullptr".into(), "nullptr".into()) };
            let (acc, ty) = kernels::accumulator(m);
            let (smem, side) = attention_smem(*per_block, *hd, *vd, *splits, *parallel, acc);
            let z = if *parallel { *splits } else { 1 };
            let grid = heads / per_block * rows * z;
            let (qo, qargs) = attention_quant(n, out, *quant, gate, *goff, m);
            let mb = kernels::attn_blocks_a_multiprocessor(grid, n.sms, acc, *per_block, *hd, *vd);
            let kernel = attention_kernel(*per_block, *vd, ty, mb, &n.elems(&[Some(kc), Some(vc)]), &qo, kernels::attn_path(z, side, *splits), *hd);
            let dst = if *f32 { p(out) } else { "nullptr".into() };
            let capq = capped(*cap, &qargs, ", lk_qout{}");
            format!(
                "{kernel}<<<dim3({}, {rows}, {z}), 256, {smem}, st>>>({}, {}, {}, {}, {}, {dst}, {part}, {cnt}, {len}, {}, {hd}, {vd}, {group}, {heads}, {}, {vstride}, {voff}, {}, {}, {splits}, {}, {}{qargs}{capq});",
                heads / per_block,
                p(q),
                p(kc),
                p(vc),
                n.opt(pos),
                n.opt(sinks),
                win(*window, *chunked),
                heads / group * hd,
                flit(*scale as f32),
                kernels::ATTN_MIN_SPLIT,
                seq.unwrap_or(0),
                side
            )
        }
        Launch::Slice { x, y, groups, every, from, width } => format!(
            "lk_slice<<<{}, 256, 0, st>>>({}, {}, {}, {every}, {from}, {width});",
            cdiv(groups * width, 256),
            p(x),
            p(y),
            groups * width
        ),
        Launch::Join { a, b: bb, y, rows, heads, ha, hb, yz } => format!(
            "lk_join<<<{}, 256, 0, st>>>({}, {}, {}, {}, {ha}, {hb}{});",
            cdiv(rows * heads * (ha + hb), 256),
            p(a),
            p(bb),
            n.dst(y),
            rows * heads * (ha + hb),
            if *yz > 0 { format!(", {heads}, {yz}") } else { String::new() }
        ),
        // The per-head matmuls read their activation `f32` under every numerics (`kernels::head_mode`).
        Launch::HeadGemv { x, part, w, format, y, m: rows, heads, a, b: bw, rows_out, shared, upto, .. } if l.heads_in_tiles(numerics) => {
            let ((xp, xo), (yp, yo)) = (reading(part, *a), unit(y, *bw));
            // Every head reading its row's one unit: no pitch between heads, the units' pitch between rows.
            let (xp, xr) = if *shared { (0, xp) } else { (xp, 0) };
            let bound = upto.as_ref().map_or(String::new(), |(at, after)| format!(", {}, {after}", p(at)));
            let (yat, yrow, ypitch) = n.dst_split(y, 0);
            format!(
                "lk_head_tiled<{}, {}, {rows_out}><<<dim3({}, {}, {heads}), 256, 0, st>>>({}, nullptr, nullptr, nullptr, {yat}, {heads}, {a}, {bw}, {xp}, {xo}, {yrow}, {ypitch}, {yp}, {yo}, {rows}, {}, {xr}{bound});",
                kernels::format_arg(*format),
                kernels::head_mode(numerics),
                cdiv(*bw, 64),
                cdiv(*rows, kernels::HEAD_TILE_M),
                handle(w, *format, if *rows_out { *a } else { *bw }),
                p(x)
            )
        }
        // A block of rows in slabs of `GEMV_ROWS`, each launch's activation rows and results the slab's.
        Launch::HeadGemv { shared: true, .. } | Launch::HeadGemv { upto: Some(_), .. } => {
            unreachable!("a per-head matmul over every head's one unit or bounded rows runs in tiles")
        }
        Launch::HeadGemv { x, part, w, format, y, m: rows, heads, a, b: bw, rows_out, .. } => (0..rows.div_ceil(GEMV_ROWS))
            .map(|slab| {
                let (r, m) = (GEMV_ROWS.min(rows - slab * GEMV_ROWS), kernels::head_mode(numerics));
                let ((xp, xo), (yp, yo)) = (reading(part, *a), unit(y, *bw));
                let at = slab * GEMV_ROWS * heads * xp;
                let xin = format!("{} + {at}, nullptr, nullptr, nullptr", p(x));
                let (yat, yrow, ypitch) = n.dst_split(y, slab * GEMV_ROWS * heads * yp);
                let f = kernels::format_arg(*format);
                let shape = format!("{heads}, {a}, {bw}, {xp}, {xo}, {yrow}, {ypitch}, {yp}, {yo}");
                if *rows_out {
                    format!(
                        "lk_head_gemv<{f}, {m}, {r}><<<{}, 256, 0, st>>>({}, {xin}, {yat}, {shape});",
                        cdiv(heads * bw, 8),
                        handle(w, *format, *a)
                    )
                } else if kernels::axpy_staged(*format, m) {
                    // The head's rows a tile at a time, as many as 48 KB of shared memory holds beside the kernel's
                    // own sums.
                    let pitch = kernels::axpy_pitch(*format);
                    let tile = (*a).min((48 * 1024 - kernels::AXPY_SUMS) / pitch);
                    format!(
                        "lk_head_axpy_staged<{f}, {m}, {r}, {}, {}><<<dim3({}, {heads}), {}, {}, st>>>({}, {xin}, {yat}, {shape}, {tile});",
                        kernels::AXPY_W,
                        kernels::AXPY_R,
                        cdiv(bw / 32, kernels::AXPY_W),
                        32 * kernels::AXPY_W * kernels::AXPY_R,
                        tile * pitch,
                        handle(w, *format, *bw)
                    )
                } else {
                    format!(
                        "lk_head_axpy<{f}, {m}, {r}><<<dim3({}, {heads}), 256, 0, st>>>({}, {xin}, {yat}, {shape});",
                        bw / 32,
                        handle(w, *format, *bw)
                    )
                }
            })
            .collect::<Vec<_>>()
            .join("\n  CKL();\n  "),
        Launch::ArgMax { x, out, n: len, advance, rows, cap, y } => format!(
            "lk_argmax<{}><<<dim3({}, {rows}), 256, 0, st>>>({}, {len}, {}, {}, {}, {}, {}, {}, {}{});",
            b(advance.is_some()),
            kernels::argmax_blocks(*len),
            p(x),
            p(out),
            match advance {
                // A batch's sequence advances its own token, position and run.
                Some(a) if n.batch => format!("{} + {1}, d_bpos + {1}, 1, d_bgen + {1} * CTX, d_bcount + {1}", n.plain(&a.token), a.item),
                Some(_) => format!("{token}, d_pos, {}, d_gen, d_count", n.rows),
                None => "nullptr, nullptr, 0, nullptr, nullptr".into(),
            },
            n.scratch("av", out),
            n.scratch("ai", out),
            n.scratch("ac", out),
            u8::from(*rows > 1),
            if *rows > 1 { "CTX" } else { "0" },
            match (cap, y) {
                (Some(c), Some(y)) => format!(", {}, {}", flit(*c as f32), p(y)),
                (Some(c), None) => format!(", {}", flit(*c as f32)),
                (None, _) => String::new(),
            }
        ),
        // A chunk's rows at once, then the ring advanced past them.
        Launch::Conv { x, w, ring, y, chan, taps, rows, silu, l2 } if *rows > 1 => {
            let (grid, block, nl2, eps) = match l2 {
                Some((d, e)) => (chan / d, *d, chan / d, e.get() as f32),
                None => (cdiv(*chan, 256), 256, 0, 0.0),
            };
            format!(
                "lk_conv_rows<{}, {}, {sum_ty}><<<dim3({grid}, {rows}), {block}, 0, st>>>({}, {}, {}, {}, {chan}, {taps}, {rows}, {nl2}, {}, {pos});\n  CKL();\n  \
                 lk_conv_carry<<<{}, 256, 0, st>>>({}, {}, {chan}, {taps}, {rows}, {pos});",
                b(*silu),
                b(l2.is_some()),
                p(x),
                p(w),
                p(ring),
                p(y),
                flit(eps),
                cdiv(*chan, 256),
                p(x),
                p(ring),
                pos = n.conv_pos(ring, taps * chan, 1)
            )
        }
        Launch::Conv { x, w, ring, y, chan, taps, rows, silu, l2 } => {
            let (grid, block, nl2, eps) = match l2 {
                Some((d, e)) => (chan / d, *d, chan / d, e.get() as f32),
                None => (cdiv(*chan, 256), 256, 0, 0.0),
            };
            format!(
                "lk_conv<{}, {}, {sum_ty}><<<{grid}, {block}, 0, st>>>({}, {}, {}, {}, {chan}, {taps}, {rows}, {nl2}, {}, {});",
                b(*silu),
                b(l2.is_some()),
                p(x),
                p(w),
                p(ring),
                p(y),
                flit(eps),
                n.conv_pos(ring, taps * chan, 1)
            )
        }
        Launch::ConvParts { parts, taps, block, eps, seqs } => {
            let entries: Vec<String> = parts
                .iter()
                .map(|q| format!("{{{}, {}, {}, {}, {}, {}, {}}}", p(&q.x), p(&q.w), p(&q.ring), p(&q.y), q.chan, cdiv(q.chan, *block), u8::from(q.l2)))
                .collect();
            let blocks: usize = parts.iter().map(|q| cdiv(q.chan, *block)).sum();
            format!(
                "lk_conv_parts<{sum_ty}><<<dim3({blocks}, {seqs}), {block}, 0, st>>>(lk_convs{{{{{}}}}}, {}, {taps}, {}, {}, {});",
                entries.join(", "),
                parts.len(),
                flit(eps.map_or(0.0, |e| e.get() as f32)),
                n.conv_pos(&parts[0].ring, taps * parts[0].chan, *seqs),
                u8::from(*seqs > 1)
            )
        }
        Launch::L2Norm { x, y, rows, cols, eps } => {
            format!("lk_l2norm<{sum_ty}><<<{rows}, 256, 0, st>>>({}, {}, {cols}, {});", p(x), p(y), flit(eps.get() as f32))
        }
        Launch::Decay { ar, dt, al, y, n: len, heads } => format!(
            "lk_decay<<<{}, 256, 0, st>>>({}, {}, {}, {}, {len}, {heads});",
            cdiv(*len, 256),
            p(ar),
            p(dt),
            p(al),
            p(y)
        ),
        Launch::GateProj { x, wb, wa, dt, al, beta, decay, k, heads, parts } => format!(
            "lk_gate_proj<<<{}, 256, 0, st>>>({}, {}, {}, {}, {}, {}, {}, {k}, {heads}, {parts});",
            cdiv(*heads, 8 / parts),
            p(wb),
            p(wa),
            p(x),
            p(dt),
            p(al),
            p(beta),
            p(decay)
        ),
        Launch::Gates { br, ar, dt, al, beta, decay, n: len, heads } => format!(
            "lk_gates<<<{}, 256, 0, st>>>({}, {}, {}, {}, {}, {}, {len}, {heads});",
            cdiv(*len, 256),
            p(br),
            p(ar),
            p(dt),
            p(al),
            p(beta),
            p(decay)
        ),
        // A chunk of rows at a time: every chunk's products the memory does not enter, then the memory's walk.
        Launch::Delta { q, k, v, z, b: bb, a, st, gn, out, f32, quant, hv, hk, d, rows, eps, .. } if l.delta_in_chunks(numerics) => {
            let chunks = rows.div_ceil(kernels::DELTA_CHUNK);
            let (dk, dq, dt, dp, dg) = (n.scratch("dk", out), n.scratch("dq", out), n.scratch("dt", out), n.scratch("dp", out), n.scratch("dg", out));
            format!(
                "lk_delta_prep<{d}><<<dim3({hv}, {chunks}), 256, {}, st>>>({}, {}, {}, {}, {dk}, {dq}, {dt}, {dp}, {dg}, {hk}, {hv}, {rows});\n  CKL();\n  \
                 lk_delta_state<{d}, {}{}><<<dim3({hv}, {}), {}, 0, st>>>({}, {}, {dk}, {dq}, {dt}, {dp}, {dg}, {}, {}, {hk}, {hv}, {rows});\n  CKL();\n  \
                 lk_delta_norm<{}, {}, {sum_ty}><<<{}, {d}, 0, st>>>({}, {}, {}, {}, {}, {d}, {});",
                kernels::DELTA_PREP_SMEM,
                p(q),
                p(k),
                p(bb),
                p(a),
                kernels::DELTA_STATE_ROWS,
                n.elems(&[Some(st)]),
                d / kernels::DELTA_STATE_ROWS,
                kernels::DELTA_STATE_THREADS,
                p(k),
                p(v),
                p(st),
                n.scratch("o", out),
                b(*f32),
                qm(*quant),
                rows * hv,
                p(z),
                p(gn),
                n.scratch("o", out),
                if *f32 { p(out) } else { "nullptr".into() },
                n.quant(out, *quant),
                flit(eps.get() as f32)
            )
        }
        // A row at a time, each head's last block norming the head: the norm's own block of `d` threads played by the
        // step's four warps.
        Launch::Delta { q, k, v, z, b: bb, a, st, gn, out, f32, quant, hv, hk, d, rows, eps, seqs, qk } => format!(
            "lk_delta_step<{sum_ty}, {}, {}, {}, {}, {}><<<dim3({hv}, {}, {seqs}), 128, 0, st>>>({}, {}, {}, {}, {}, {}, {}, {d}, {hk}, {hv}, {rows}, {}, {}, {}, {}, {}, {}, {}{});",
            d.div_ceil(32),
            b(*f32),
            qm(*quant),
            d.div_ceil(32).div_ceil(4),
            n.elem(st),
            cdiv(*d, 4),
            p(q),
            p(k),
            p(v),
            p(bb),
            p(a),
            p(st),
            n.scratch("o", out),
            u8::from(*seqs > 1),
            p(z),
            p(gn),
            if *f32 { p(out) } else { "nullptr".into() },
            n.quant(out, *quant),
            flit(eps.get() as f32),
            n.scratch("dc", out),
            qk.map_or(String::new(), |e| format!(", {}", flit(e.get() as f32)))
        ),
        Launch::Moe { x, wr, wg, wu, wd, formats, y, d, dff, experts, k, router, rb, biases, renormalize, act, shared, per_warp, rows, router_split, .. } => {
            let key = &y.buf;
            let xin = format!("{}, {}", if m == 0 { p(x) } else { "nullptr".into() }, n.quant(x, m > 0));
            // An f32 router reads the input f32 under every numerics; a quantized one reads it as the experts do.
            let (rf, fr, route_in, router_rows) = match router {
                Router::F32 => {
                    (1, 8, format!("{}, {}", p(x), n.quant(x, m > 0)), format!("lk_w{{nullptr, nullptr, nullptr, 0}}, {}", p(wr)))
                }
                Router::Rows(ty) => (0, kernels::format_arg(*ty), xin.clone(), format!("{}, nullptr", handle(wr, *ty, *d))),
            };
            let [fg, fu, fd] = formats.map(kernels::format_arg);
            // Without shared experts their formats are the kept experts' and their rows none.
            let none = "lk_w{nullptr, nullptr, nullptr, 0}".to_string();
            let (sdff, [fsg, fsu, fsd], [sg, su, sd]) = match shared {
                Some(e) => (
                    e.dff,
                    e.formats.map(kernels::format_arg),
                    [handle(&e.wg, e.formats[0], *d), handle(&e.wu, e.formats[1], *d), handle(&e.wd, e.formats[2], e.dff)],
                ),
                None => (0, [fg, fu, fd], [none.clone(), none.clone(), none]),
            };
            let (hidden, rows_a_warp) = (k * dff + sdff, *per_warp);
            let wpe = kernels::route_warps(*router == Router::F32, *d);
            let (act_arg, limit, slope) = match act {
                MoeAct::Swiglu => (0, 0.0, 0.0),
                MoeAct::ClampedSwiglu { limit, slope } => (1, f64::from_bits(*limit), f64::from_bits(*slope)),
            };
            let (hid, i, g) = (n.scratch("h", key), n.scratch("i", key), n.scratch("g", key));
            let (lg, cnt) = (n.scratch("rl", key), n.scratch("rc", key));
            let hq = if m > 0 {
                let lo = if n.two { n.scratch("hl", key) } else { "nullptr".into() };
                format!("{}, {lo}, {}", n.scratch("hq", key), n.scratch("hs", key))
            } else {
                "nullptr, nullptr, nullptr".into()
            };
            if *rows > 1 {
                // The rows routed each on its own, their choices grouped by expert, each expert over its choices' rows.
                let (rows, choices) = (*rows, rows * k);
                let names = ["ec", "eo", "el", "ep", "gx", "gq", "gl", "gs", "pa"].map(|t| n.scratch(t, key));
                let [ec, eo, el, ep, gx, gq, gl, gs, pa] = &names;
                let gathered = if m == 0 {
                    format!("{gx}, nullptr, nullptr, nullptr")
                } else {
                    format!("nullptr, {gq}, {}, {gs}", if n.two { gl.as_str() } else { "nullptr" })
                };
                let hidden_rows = choices * dff;
                let quant = if m > 0 {
                    format!("lk_quant<{m}><<<{}, 256, 0, st>>>({hid}, {hq}, {hidden_rows});\n  CKL();\n  ", cdiv(hidden_rows, 256))
                } else {
                    String::new()
                };
                let experts_pass = if l.pairs_gate_and_up(numerics) {
                    // Each expert's gate and up GEMMs paired over its choices' rows on the tensor cores, its hidden rows
                    // through the gated activation and quantized in their epilogue; then down weighing each choice's row
                    // by its weight.
                    let words = format!("{gq}, {}, {gs}", if n.two { gl.as_str() } else { "nullptr" });
                    let bm = kernels::gemm_blk_expert_rows(choices, *experts);
                    let shape = |f: u32, k: usize| format!("{}, {}", kernels::gemm_blk_launch_threads(f, bm, k), kernels::gemm_blk_launch_smem(f, n.two, bm, k));
                    format!(
                        "{}<<<dim3({}, {}, {experts}), {}, st>>>({}, {words}, {}, nullptr, nullptr, 0, {d}, {dff}, 0, {ec}, {eo}, nullptr, nullptr, \
                         {}, {}, {hq}, {}, {});\n  CKL();\n  \
                         {}<<<dim3({}, {}, {experts}), {}, st>>>({}, {hq}, {}, {pa}, nullptr, 0, {dff}, {d}, 0, {ec}, {eo}, {el}, {g});\n  CKL();\n  ",
                        gemm_blk_name(fg, m, 3, bm, i32::from(act_arg), false),
                        cdiv(*dff, kernels::GEMM_TILE_N / 2),
                        cdiv(rows, bm),
                        shape(fg, *d),
                        handle(wg, formats[0], *d),
                        n.opt(&biases[0]),
                        handle(wu, formats[1], *d),
                        n.opt(&biases[1]),
                        flit(limit as f32),
                        flit(slope as f32),
                        gemm_blk_name(fd, m, 2, bm, 0, false),
                        cdiv(*d, kernels::GEMM_TILE_N),
                        cdiv(rows, bm),
                        shape(fd, *dff),
                        handle(wd, formats[2], *dff),
                        n.opt(&biases[2])
                    )
                } else if l.on_tensor_cores(numerics) {
                    // Each expert's GEMMs over its choices' rows on the tensor cores: gate and up, the gated activation, and
                    // down weighing each choice's row by its weight.
                    let (hg, hu) = (n.scratch("hg", key), n.scratch("hu", key));
                    let words = format!("{gq}, {}, {gs}", if n.two { gl.as_str() } else { "nullptr" });
                    let bm = kernels::gemm_blk_expert_rows(choices, *experts);
                    let grid = |cols: usize| format!("dim3({}, {}, {experts})", cdiv(cols, kernels::GEMM_TILE_N), cdiv(rows, bm));
                    let shape = |f: u32, k: usize| format!("{}, {}", kernels::gemm_blk_launch_threads(f, bm, k), kernels::gemm_blk_launch_smem(f, n.two, bm, k));
                    format!(
                        "{}<<<{}, {}, st>>>({}, {words}, nullptr, {hg}, nullptr, 0, {d}, {dff}, 0, {ec}, {eo});\n  CKL();\n  \
                         {}<<<{}, {}, st>>>({}, {words}, nullptr, {hu}, nullptr, 0, {d}, {dff}, 0, {ec}, {eo});\n  CKL();\n  \
                         lk_moe_glu<{act_arg}><<<{}, 256, 0, st>>>({hg}, {hu}, {}, {}, {i}, {el}, {hid}, {hidden_rows}L, {dff}, {}, {});\n  CKL();\n  \
                         {quant}{}<<<{}, {}, st>>>({}, {hq}, {}, {pa}, nullptr, 0, {dff}, {d}, 0, {ec}, {eo}, {el}, {g});\n  CKL();\n  ",
                        gemm_blk_name(fg, m, 1, bm, 0, false),
                        grid(*dff),
                        shape(fg, *d),
                        handle(wg, formats[0], *d),
                        gemm_blk_name(fu, m, 1, bm, 0, false),
                        grid(*dff),
                        shape(fu, *d),
                        handle(wu, formats[1], *d),
                        cdiv(hidden_rows, 256),
                        n.opt(&biases[0]),
                        n.opt(&biases[1]),
                        flit(limit as f32),
                        flit(slope as f32),
                        gemm_blk_name(fd, m, 2, bm, 0, false),
                        grid(*d),
                        shape(fd, *dff),
                        handle(wd, formats[2], *dff),
                        n.opt(&biases[2])
                    )
                } else {
                    // Each expert's rows over its choices' rows in slabs.
                    format!(
                        "lk_moe_up_rows<{m}, {act_arg}, {fg}, {fu}><<<dim3({}, {experts}), 256, 0, st>>>({gathered}, {}, {}, {}, {}, {ec}, {eo}, {hid}, {d}, {dff}, {}, {});\n  CKL();\n  \
                         {quant}lk_moe_down_rows<{m}, {fd}><<<dim3({}, {experts}), 256, 0, st>>>({hid}, {hq}, {}, {}, {ec}, {eo}, {el}, {g}, {pa}, {d}, {dff});\n  CKL();\n  ",
                        cdiv(*dff, 8),
                        handle(wg, formats[0], *d),
                        handle(wu, formats[1], *d),
                        n.opt(&biases[0]),
                        n.opt(&biases[1]),
                        flit(limit as f32),
                        flit(slope as f32),
                        cdiv(*d, 8),
                        handle(wd, formats[2], *dff),
                        n.opt(&biases[2])
                    )
                };
                let route = if l.on_tensor_cores(numerics) {
                    // The rows' router logits as one GEMM, then each row's choices from its logits.
                    let logits = match router {
                        Router::F32 => format!(
                            "lk_gemm_f32<<<dim3({}, {}, {router_split}), 256, 0, st>>>({}, {}, {}, {lg}, {rows}, {experts}, {d}{});",
                            cdiv(*experts, 64),
                            cdiv(rows, 64),
                            p(x),
                            p(wr),
                            n.opt(rb),
                            if *router_split > 1 {
                                format!(", {router_split}, {}, {}", n.scratch("rp", key), n.scratch("rq", key))
                            } else {
                                String::new()
                            }
                        ),
                        Router::Rows(ty) => format!(
                            "{}<<<dim3({}, {}), {}, {}, st>>>({}, {}, {}, {lg}, nullptr, 0, {d}, {experts}, {rows});",
                            gemm_blk_name(kernels::format_arg(*ty), m, 0, kernels::GEMM_BLK_TILE_M, 0, false),
                            cdiv(*experts, kernels::GEMM_TILE_N),
                            cdiv(rows, kernels::GEMM_BLK_TILE_M),
                            kernels::gemm_blk_launch_threads(kernels::format_arg(*ty), kernels::GEMM_BLK_TILE_M, *d),
                            kernels::gemm_blk_launch_smem(kernels::format_arg(*ty), n.two, kernels::GEMM_BLK_TILE_M, *d),
                            handle(wr, *ty, *d),
                            n.quant(x, true),
                            n.opt(rb)
                        ),
                    };
                    format!(
                        "{logits}\n  CKL();\n  lk_moe_pick<{m}><<<{rows}, 256, {}, st>>>({lg}, {experts}, {k}, {}, {i}, {g});",
                        experts * 4,
                        u8::from(*renormalize)
                    )
                } else {
                    format!(
                        "lk_moe_route<{m}, {rf}, {fr}{}><<<dim3({}, {rows}), 256, {}, st>>>({route_in}, {router_rows}, {}, {d}, {experts}, {k}, {}, {i}, {g}, {lg}, {cnt});",
                        per_warp_arg(wpe),
                        cdiv(*experts * wpe, 8),
                        experts * 4,
                        n.opt(rb),
                        u8::from(*renormalize)
                    )
                };
                // Each row combined, or with the next norm a block of the norm's threads a row.
                let combine = match &finish {
                    None => format!("lk_moe_combine<{m}><<<dim3({}, {rows}), 256, 0, st>>>({pa}, {ep}, {k}, {d}, {});", cdiv(*d, 256), n.dst_ptr(y)),
                    Some((norm, sum, _)) => {
                        let (norm_t, nm) = norm_of(norm, n, m, *d, sum_ty);
                        let (plus, dst) = sum.map_or(("nullptr".into(), n.dst_ptr(y)), |(plus, to)| (p(plus), p(to)));
                        format!("lk_moe_combine_norm<{m}, {norm_t}><<<{rows}, {}, 0, st>>>({pa}, {ep}, {k}, {d}, {plus}, {dst}, {nm});", kernels::norm_threads(*d))
                    }
                };
                let _ = write!(
                    out,
                    "  {route}\n  CKL();\n  \
                     lk_moe_group<<<1, {}, {}, st>>>({i}, {choices}, {experts}, {ec}, {eo}, {el}, {ep});\n  CKL();\n  \
                     lk_moe_gather<{m}><<<{choices}, 256, 0, st>>>({xin}, {el}, {k}, {d}, {gathered});\n  CKL();\n  \
                     {experts_pass}{combine}\n  CKL();\n",
                    kernels::moe_group_threads(*experts),
                    kernels::moe_group_smem_bytes(*experts, kernels::moe_group_threads(*experts)),
                );
                return;
            }
            let quant = if m > 0 {
                format!("lk_quant<{m}><<<{}, 256, 0, st>>>({hid}, {hq}, {hidden});\n  CKL();\n  ", cdiv(hidden, 256))
            } else {
                String::new()
            };
            let (down_rows, shared_warps) = kernels::moe_down_rows(*d, *dff, sdff);
            let (down_blocks, down_threads) = kernels::moe_down_launch(*d, *dff, *k, sdff);
            // The down projection, or with the next norm in its last block, the block's warps playing the norm's.
            let down = match &finish {
                None => format!("lk_moe_down<{m}, {fd}, {fsd}, {down_rows}, {shared_warps}><<<{down_blocks}, {down_threads}, 0, st>>>"),
                Some((norm, ..)) => {
                    let vt = kernels::norm_threads(*d);
                    let norm_t = norm_of(norm, n, m, *d, sum_ty).0;
                    format!(
                        "lk_moe_down_norm<{m}, {fd}, {fsd}, {down_rows}, {shared_warps}, {norm_t}, {}><<<{down_blocks}, {down_threads}, 0, st>>>",
                        (vt / 32).div_ceil(down_threads / 32)
                    )
                }
            };
            let tail = match &finish {
                None => String::new(),
                Some((norm, sum, gc)) => format!(", {}, {}, {}, {gc}", sum.map_or("nullptr".into(), |(plus, _)| p(plus)), norm_of(norm, n, m, *d, sum_ty).1, kernels::norm_threads(*d)),
            };
            let dst = finish.as_ref().and_then(|(_, sum, _)| sum.map(|(_, to)| p(to))).unwrap_or_else(|| n.dst_ptr(y));
            format!(
                "lk_moe_route<{m}, {rf}, {fr}{}><<<{}, 256, {}, st>>>({route_in}, {router_rows}, {}, {d}, {experts}, {k}, {}, {i}, {g}, {lg}, {cnt});\n  CKL();\n  \
                 lk_moe_up<{m}, {act_arg}, {fg}, {fu}, {fsg}, {fsu}{}><<<{}, 256, 0, st>>>({xin}, {}, {}, {sg}, {su}, {}, {}, {i}, {hid}, {d}, {dff}, {k}, {sdff}, {}, {});\n  CKL();\n  \
                 {quant}{down}({hid}, {hq}, {}, {sd}, {}, {i}, {g}, {dst}, {d}, {dff}, {k}, {sdff}{tail});",
                per_warp_arg(wpe),
                cdiv(*experts * wpe, 8),
                experts * 4,
                n.opt(rb),
                u8::from(*renormalize),
                per_warp_arg(rows_a_warp),
                cdiv(hidden, 8 * rows_a_warp),
                handle(wg, formats[0], *d),
                handle(wu, formats[1], *d),
                n.opt(&biases[0]),
                n.opt(&biases[1]),
                flit(limit as f32),
                flit(slope as f32),
                handle(wd, formats[2], *dff),
                n.opt(&biases[2])
            )
        }
    };
    let _ = write!(out, "  {line}\n  CKL();\n");
}

/// The norm a fold runs over the rows of `cols` it finished: its template arguments after the producer's (`ADD, SUM,
/// OUTF, QO, PER, A, PRE`, `PER` the values a thread of the norm's own block holds) and its operands, outputs and
/// epsilons as an `lk_norm_of`.
fn norm_of(norm: &Launch, n: &Names, m: u32, cols: usize, sum_ty: &str) -> (String, String) {
    let Launch::Norm { x, add, pre, sum, w, out, into, f32, quant, eps, scale: None, .. } = norm else { unreachable!("a fold's norm is an unscaled norm") };
    let own = Dst::to(out);
    let (y, yrow, ypitch) = if *f32 { n.dst_split(into.as_ref().unwrap_or(&own), 0) } else { ("nullptr".into(), "nullptr".into(), 0) };
    let b = |v: bool| if v { "true" } else { "false" };
    let (pw, peps) = match pre {
        Some(pn) => (n.opt(&pn.w), flit(pn.eps.get() as f32)),
        None => ("nullptr".into(), flit(0.0)),
    };
    (
        format!(
            "{}, {}, {}, {}, {}, {sum_ty}, {}",
            b(add.is_some()),
            b(sum.is_some()),
            b(*f32),
            if *quant { m } else { 0 },
            cols.div_ceil(kernels::norm_threads(cols)),
            b(pre.is_some())
        ),
        format!(
            "lk_norm_of{{{}, {}, {}, {}, {y}, {yrow}, {ypitch}, {}, {}, {pw}, {peps}}}",
            n.plain(x),
            n.opt(add),
            n.opt(sum),
            n.opt(w),
            n.quant(out, *quant),
            flit(eps.get() as f32)
        ),
    )
}


/// The nodes the program's graphs capture, every form's step and a prefill chunk's lean step (`output_only`): each
/// kernel its launches print and each memory set or copy — the events a step records across its streams
/// (`branches::branch`) are the graph's edges, not nodes — what `residence::plan`'s spare is counted by.
pub fn captured_nodes(forms: &[Form], knobs: &CudaKnobs) -> usize {
    let two = forms[0].schedule.numerics == Q8MatKind::TwoPassInt8;
    let shared = shared_names(forms);
    let decode = forms.iter().find(|f| f.rows == 1).expect("a lane has a decode step");
    let token = format!("t1_{}", c(&decode.schedule.io.token));
    let nodes = |stmts: &str| ["<<<", "cudaMemsetAsync(", "cudaMemcpyAsync("].iter().map(|op| stmts.matches(op).count()).sum::<usize>();
    forms
        .iter()
        .map(|f| {
            let n = Names::of(f, &shared, two, knobs.chip.sms);
            let lean = output_only(f, forms);
            let (mut whole, mut kept) = (0, 0);
            for (l, &out) in f.schedule.launches.iter().zip(&lean) {
                let mut stmts = String::new();
                launch(l, &n, f.schedule.numerics, &token, &mut stmts);
                whole += nodes(&stmts);
                kept += if out { usize::from(matches!(l, Launch::ArgMax { advance: Some(_), .. })) } else { nodes(&stmts) };
            }
            whole + if lean.iter().any(|&x| x) { kept } else { 0 }
        })
        .sum()
}

/// The tensor whose bytes `t`'s buffer is: `t`, or the tensor its alias (and that alias's, on) lies in.
pub fn alias_root<'a>(buffers: &'a BTreeMap<String, Buffer>, mut t: &'a str) -> &'a str {
    while let Some(Buffer { fill: Fill::Alias { of, .. }, .. }) = buffers.get(t) {
        t = of;
    }
    t
}

/// Whether a state tensor's value passes from one step to the next: some form of the lane first touches it (its buffer's
/// root, `alias_root`) by other than a launch that writes it whole without reading it — a cache's rows, a recurrent
/// memory, the position; not a result the step makes anew each time in a buffer the program keeps (a mixture's sum).
pub fn carried(forms: &[Form], t: &str) -> bool {
    forms.iter().any(|g| {
        let t = alias_root(&g.schedule.buffers, t);
        let Some(first) = g.schedule.launches.iter().find(|l| l.reads().into_iter().chain(l.writes()).any(|x| alias_root(&g.schedule.buffers, x) == t)) else {
            return false;
        };
        let len = g.schedule.buffers.get(t).map_or(0, |b| b.len);
        let anew = !first.reads().into_iter().any(|x| alias_root(&g.schedule.buffers, x) == t) && first.writes_whole(t, Touch::Plain, len);
        g.graph.tensors.get(t).is_some_and(|x| x.role == Role::State) && !anew
    })
}

/// The launches of a prefill chunk that compute only what its output reads — the last row's logits and the token —
/// found by liveness from what the step carries: a launch is kept where it writes a tensor the steps carry (`carried`:
/// a cache, a recurrent memory, the position; through any alias of one) or a tensor a kept launch reads, or writes
/// nothing it names; the arg-max is the output's, its advance of the position and the run kept apart (`lk_advance`). A
/// chunk before the prompt's last runs none of them. The decode step's and a batch's: none, every row of theirs an
/// output.
pub fn output_only(f: &Form, forms: &[Form]) -> Vec<bool> {
    let n = f.schedule.launches.len();
    if f.kind != FormKind::Prefill || f.rows < 2 {
        return vec![false; n];
    }
    let root = |t: &str| alias_root(&f.schedule.buffers, t).to_string();
    let carried = |t: &str| carried(forms, t);
    let mut needed: BTreeSet<String> = BTreeSet::new();
    let mut out = vec![true; n];
    for (i, l) in f.schedule.launches.iter().enumerate().rev() {
        if matches!(l, Launch::ArgMax { .. }) {
            continue;
        }
        let writes = l.writes();
        if writes.is_empty() || writes.iter().any(|&t| carried(t) || needed.contains(&root(t))) {
            out[i] = false;
            needed.extend(l.reads().into_iter().map(root));
        }
    }
    out
}

/// Print `forms` as one CUDA program over the checkpoint `ckpt`, its caches `ctx` positions long.
pub fn emit(forms: &[Form], knobs: &CudaKnobs, ctx: usize, ckpt: Checkpoint, plan: &Plan) -> Result<Program, LaneError> {
    let numerics = forms[0].schedule.numerics;
    let m = kernels::mode(numerics);
    let two = numerics == Q8MatKind::TwoPassInt8;
    let shared = shared_names(forms);
    let decode = forms.iter().find(|f| f.rows == 1).expect("a lane has a decode step");
    let token = format!("t1_{}", c(&decode.schedule.io.token));

    let mut cu = header(Provenance::Compiled);
    for f in forms {
        let _ = writeln!(cu, "// {}", f.identity.line());
        for line in f.source.lines() {
            let _ = writeln!(cu, "//   {line}");
        }
    }
    cu.push_str(
        "#include <cstdio>\n#include <cstdlib>\n#include <cstdint>\n#include <cstring>\n#include <cmath>\n\
         #include <vector>\n#include <string>\n#include <fcntl.h>\n#include <sys/mman.h>\n#include <sys/stat.h>\n\
         #include <unistd.h>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n#include <cuda_bf16.h>\n\
         #define CK(x) do { cudaError_t e_ = (x); if(e_ != cudaSuccess){ fprintf(stderr, \"cuda %s:%d %s\\n\", __FILE__, __LINE__, cudaGetErrorString(e_)); exit(1); } } while(0)\n\
         #define CKL() CK(cudaGetLastError())\n",
    );
    cu.push_str(kernels::PRELUDE);
    cu.push_str(kernels::STALL);
    for k in kernels_used(forms) {
        cu.push_str(k.source());
    }
    let _ = write!(
        cu,
        "\nstatic const uint64_t CKPT_SIZE = {}ULL, CKPT_DATA = {}ULL, CKPT_DIGEST = {}ULL;\nstatic const int CTX = {ctx};\n",
        ckpt.size, ckpt.data_start, ckpt.digest
    );
    let _ = write!(
        cu,
        "static const size_t LK_ALIGN = {POOL_ALIGN}UL, LK_ARENA = {}UL, LK_FIXED = {}UL, LK_SPARE = {}UL;\n\
         static const int LK_MOVABLE = {};\nstatic const size_t LK_ORDER[] = {{{}}};\n",
        plan.arena,
        plan.fixed,
        plan.spare,
        plan.order.len(),
        if plan.order.is_empty() { "0".to_string() } else { plan.order.iter().map(|(_, b)| format!("{b}UL")).collect::<Vec<_>>().join(", ") }
    );
    let _ = write!(
        cu,
        "static const unsigned char LK_STREAMS[] = {{{}}};\nstatic const size_t LK_SLOT = {}UL, LK_STAGE_SPARE = {}UL;\nstatic const int LK_STAGE_ARRAYS = {};\n{STAGE}",
        if plan.order.is_empty() { "0".to_string() } else { plan.order.iter().map(|(m, _)| if plan.streamed.contains_key(m) { "1" } else { "0" }).collect::<Vec<_>>().join(", ") },
        plan.slot,
        plan.copies * residence::SPARE_A_LAUNCH,
        forms
            .iter()
            .flat_map(|f| f.schedule.launches.iter().map(|l| stage(l, &f.schedule.buffers, plan).iter().map(|(_, a)| a.len()).sum::<usize>()))
            .max()
            .unwrap_or(0)
            .max(1)
    );
    let rank: BTreeMap<&str, usize> = plan.order.iter().enumerate().map(|(i, (m, _))| (m.as_str(), i)).collect();
    let moved = |name: &str| format!("lk_moved > {}", rank[name]);

    let mut sidecar: Vec<u8> = Vec::new();
    let mut mxfp4 = false;
    let mut q6k = false;
    let mut sixteen = false;
    let mut decls = String::new();
    let mut loads = String::new();
    let mut aliases = String::new();
    let mut resets = String::new();
    // The step-local arrays' pointers into the one arena, the largest form's size, filled with `0xff` so a read of a
    // byte no launch of the step wrote is a NaN.
    let arena_bytes = forms.iter().map(|f| f.arena.bytes).max().unwrap_or(0);
    let mut arena = format!(
        "  unsigned char* lk_arena = nullptr; CK(cudaMalloc(&lk_arena, {arena_bytes}UL)); CK(cudaMemset(lk_arena, 0xff, {arena_bytes}UL)); lk_device_bytes += {arena_bytes}UL;\n"
    );
    let mut seen: BTreeMap<String, &Buffer> = BTreeMap::new();
    for f in forms {
        let n = Names::of(f, &shared, two, knobs.chip.sms);
        for buf in f.schedule.buffers.values() {
            if f.schedule.io.positions().contains(&buf.name.as_str()) {
                continue;
            }
            if shared.contains(&buf.name) {
                if let Some(prev) = seen.get(&buf.name) {
                    if prev.len != buf.len || prev.fill != buf.fill {
                        return Err(LaneError::Shape { var: buf.name.clone(), declared: prev.len, bound: buf.len });
                    }
                    continue;
                }
                seen.insert(buf.name.clone(), buf);
            }
            let len = buf.len;
            match &buf.fill {
                Fill::Constant(Placement::Rows(r)) if plan.shares.contains_key(&buf.name) => {
                    let of = &plan.shares[&buf.name];
                    if r.ty == GgmlType::Q8_0 {
                        let _ = writeln!(decls, "  signed char* {} = nullptr; __half* {} = nullptr;", rows_q(&buf.name), rows_s(&buf.name));
                        let _ = writeln!(aliases, "  {} = {}; {} = {};", rows_q(&buf.name), rows_q(of), rows_s(&buf.name), rows_s(of));
                    } else {
                        let _ = writeln!(decls, "  unsigned char* {} = nullptr;", rows_b(&buf.name));
                        let _ = writeln!(aliases, "  {} = {};", rows_b(&buf.name), rows_b(of));
                        if kernels::sixteen(r.ty) {
                            let _ = writeln!(decls, "  int {} = 1;", rows_g(&buf.name));
                            let _ = writeln!(aliases, "  {} = {};", rows_g(&buf.name), rows_g(of));
                        }
                    }
                }
                Fill::Constant(Placement::Rows(r)) if r.ty == GgmlType::Mxfp4 => {
                    let runs: Vec<String> = r.runs.iter().map(|run| format!("{}ULL, {}ULL", run.offset, run.rows)).collect();
                    let _ = writeln!(cu, "static const uint64_t R_{}[] = {{{}}};", c(&buf.name), runs.join(", "));
                    let _ = writeln!(decls, "  unsigned char* {} = nullptr;", rows_b(&buf.name));
                    let _ = writeln!(
                        loads,
                        "  upload_mxfp4(file, R_{}, {}, {}L, {}, &{}, {});",
                        c(&buf.name),
                        r.runs.len(),
                        r.rows,
                        r.cols,
                        rows_b(&buf.name),
                        moved(&buf.name)
                    );
                    mxfp4 = true;
                }
                // A 16-bit matrix on the tensor cores: its rows interleaved as they read them.
                Fill::Constant(Placement::Rows(r)) if kernels::sixteen(r.ty) && numerics != Q8MatKind::Dequant => {
                    let runs: Vec<String> = r.runs.iter().map(|run| format!("{}ULL, {}ULL", run.offset, run.rows)).collect();
                    let _ = writeln!(cu, "static const uint64_t R_{}[] = {{{}}};", c(&buf.name), runs.join(", "));
                    let _ = writeln!(decls, "  unsigned char* {} = nullptr; int {} = 1;", rows_b(&buf.name), rows_g(&buf.name));
                    let _ = writeln!(
                        loads,
                        "  upload_h16(file, R_{}, {}, {}L, {}L, &{}, &{}, {});",
                        c(&buf.name),
                        r.runs.len(),
                        r.rows,
                        r.cols,
                        rows_b(&buf.name),
                        rows_g(&buf.name),
                        moved(&buf.name)
                    );
                    sixteen = true;
                }
                // A Q6_K matrix: its blocks grouped eight at a time as the kernels read them.
                Fill::Constant(Placement::Rows(r)) if r.ty == GgmlType::Q6K => {
                    let runs: Vec<String> = r.runs.iter().map(|run| format!("{}ULL, {}ULL", run.offset, run.rows)).collect();
                    let _ = writeln!(cu, "static const uint64_t R_{}[] = {{{}}};", c(&buf.name), runs.join(", "));
                    let _ = writeln!(decls, "  unsigned char* {} = nullptr;", rows_b(&buf.name));
                    let _ = writeln!(
                        loads,
                        "  upload_q6k(file, R_{}, {}, {}L, {}, &{}, {});",
                        c(&buf.name),
                        r.runs.len(),
                        r.rows,
                        r.cols,
                        rows_b(&buf.name),
                        moved(&buf.name)
                    );
                    q6k = true;
                }
                Fill::Constant(Placement::Rows(r)) if r.ty != GgmlType::Q8_0 => {
                    let runs: Vec<String> = r.runs.iter().map(|run| format!("{}ULL, {}ULL", run.offset, run.rows)).collect();
                    let _ = writeln!(cu, "static const uint64_t R_{}[] = {{{}}};", c(&buf.name), runs.join(", "));
                    let _ = writeln!(decls, "  unsigned char* {} = nullptr;", rows_b(&buf.name));
                    if kernels::sixteen(r.ty) {
                        let _ = writeln!(decls, "  int {} = 1;", rows_g(&buf.name));
                    }
                    let _ = writeln!(
                        loads,
                        "  upload_raw(file, R_{}, {}, {}L, {}L, &{}, {});",
                        c(&buf.name),
                        r.runs.len(),
                        r.rows,
                        r.ty.bytes_for(r.cols),
                        rows_b(&buf.name),
                        moved(&buf.name)
                    );
                }
                Fill::Constant(Placement::Rows(r)) => {
                    let runs: Vec<String> = r.runs.iter().map(|run| format!("{}ULL, {}ULL", run.offset, run.rows)).collect();
                    let _ = writeln!(cu, "static const uint64_t R_{}[] = {{{}}};", c(&buf.name), runs.join(", "));
                    let _ = writeln!(decls, "  signed char* {} = nullptr; __half* {} = nullptr;", rows_q(&buf.name), rows_s(&buf.name));
                    let _ = writeln!(
                        loads,
                        "  upload_rows(file, R_{}, {}, {}L, {}, &{}, &{}, {});",
                        c(&buf.name),
                        r.runs.len(),
                        r.rows,
                        r.cols,
                        rows_q(&buf.name),
                        rows_s(&buf.name),
                        moved(&buf.name)
                    );
                }
                Fill::Constant(Placement::Values(v)) => {
                    let off = sidecar.len();
                    sidecar.extend(v.iter().flat_map(|x| x.to_le_bytes()));
                    let _ = writeln!(decls, "  float* {} = nullptr;", n.plain(&buf.name));
                    let _ = writeln!(
                        loads,
                        "  lk_malloc(&{0}, {1}UL * 4); CK(cudaMemcpy({0}, side.data() + {off}, {1}UL * 4, cudaMemcpyHostToDevice));",
                        n.plain(&buf.name),
                        v.len()
                    );
                }
                Fill::Alias { of, offset } => {
                    if buf.plain {
                        let _ = writeln!(aliases, "  {}* {} = {} + {offset};", buf.dtype.c_type(), n.plain(&buf.name), n.plain(of));
                    }
                    if buf.quantized && quant_in_place(*offset) {
                        let (v, t) = (n.qname(&buf.name), n.qname(of));
                        let lo = if two { format!("l{t} + {offset}") } else { "nullptr".into() };
                        let _ = writeln!(
                            aliases,
                            "  signed char* q{v} = q{t} + {offset}; signed char* l{v} = {lo}; float* s{v} = s{t} + {};",
                            offset / QBLOCK
                        );
                        continue;
                    }
                }
                Fill::Param | Fill::Zero | Fill::Computed => {
                    if buf.plain {
                        let ty = buf.dtype.c_type();
                        let _ = writeln!(decls, "  {ty}* {} = nullptr;", n.plain(&buf.name));
                        match f.arena.at(&buf.name, Touch::Plain) {
                            // A state in the arena is written whole before its step reads it: nothing to zero.
                            Some(p) => {
                                let _ = writeln!(arena, "  {} = ({ty}*)(lk_arena + {});", n.plain(&buf.name), p.offset);
                            }
                            None => {
                                let _ = writeln!(loads, "  lk_malloc(&{0}, {len}UL * {1}); CK(cudaMemset({0}, 0, {len}UL * {1}));", n.plain(&buf.name), buf.dtype.bytes());
                                if buf.fill == Fill::Zero {
                                    let _ = writeln!(resets, "    CK(cudaMemsetAsync({}, 0, {len}UL * {}, st));", n.plain(&buf.name), buf.dtype.bytes());
                                }
                            }
                        }
                    }
                }
            }
            if buf.quantized {
                let q = n.qname(&buf.name);
                let _ = writeln!(decls, "  signed char* q{q} = nullptr; signed char* l{q} = nullptr; float* s{q} = nullptr;");
                match f.arena.at(&buf.name, Touch::Quant) {
                    Some(p) => {
                        let (words, low, scales) = arena::quant_parts(p.offset, len, two);
                        let _ = write!(arena, "  q{q} = (signed char*)(lk_arena + {words}); s{q} = (float*)(lk_arena + {scales});");
                        if let Some(low) = low {
                            let _ = write!(arena, " l{q} = (signed char*)(lk_arena + {low});");
                        }
                        arena.push('\n');
                    }
                    None => {
                        let _ = writeln!(loads, "  lk_malloc(&q{q}, {len}); lk_malloc(&s{q}, {}UL * 4);", len / QBLOCK);
                        if two {
                            let _ = writeln!(loads, "  lk_malloc(&l{q}, {len});");
                        }
                    }
                }
            }
        }
    }

    let mut scratch = String::new();
    let mut attrs = String::new();
    // Whether a step runs its head (`lk_head`): every step reads it, a chunk before the prompt's last runs without.
    let mut steps = String::from("  bool lk_head = true;\n");
    let mut graphs = String::new();
    let mut runs = String::new();
    // The batch step's run, its sequences' logits and tokens, and the swap of a sequence's state in for a prefill.
    let mut batch = String::new();
    let mut batch_rows = 0usize;
    let mut attn_smem: BTreeMap<String, usize> = BTreeMap::new();
    let mut shared_smem: BTreeMap<String, usize> = BTreeMap::new();
    let mut flash: BTreeSet<(usize, usize, usize, String)> = BTreeSet::new();
    let mut mma: BTreeSet<(String, usize)> = BTreeSet::new();
    let mut gemm_blk: BTreeSet<(u32, u32, usize, i32, bool)> = BTreeSet::new();
    let mut gemm_h16: BTreeSet<(u32, u32, i32, usize)> = BTreeSet::new();
    // A batch's tensor-core GEMVs, by tile: `(mt, nt, mw, nw)`; and of Q4_K and Q6_K rows, by format.
    let mut gemv_mma: BTreeSet<(usize, usize, usize, usize)> = BTreeSet::new();
    let mut gemv_mma_blk: BTreeSet<u32> = BTreeSet::new();
    // Every step's launches, timed under `--profile`: the form's rows, the launch's kind and output, and the event
    // recorded before it (the next event closes it; each form's last is its own).
    let mut prof: Vec<(i64, &str, String, usize, usize)> = Vec::new();
    let mut prof_events = 0usize;
    let mut route_smem: BTreeMap<(u32, u32, usize), usize> = BTreeMap::new();
    for f in forms {
        let n = Names::of(f, &shared, two, knobs.chip.sms);
        for (at, l) in f.schedule.launches.iter().enumerate() {
            // The launch's scratch: its counters zeroed buffers of their own, the rest in the arena while it runs.
            for sc in arena::scratch(l, f.schedule.numerics) {
                let name = n.scratch(sc.tag, l.writes()[0]);
                let _ = write!(scratch, "  {}* {name} = nullptr;", sc.ty);
                match f.arena.scratch_at(at, sc.tag) {
                    Some(p) if !sc.counter => {
                        let _ = writeln!(scratch, " {name} = ({}*)(lk_arena + {});", sc.ty, p.offset);
                    }
                    _ => {
                        let _ = writeln!(scratch, " lk_malloc(&{name}, {0}UL); CK(cudaMemset({name}, 0, {0}UL));", sc.bytes);
                    }
                }
            }
            for l in l.parts() {
                match l {
                    Launch::Moe { d, experts, router, formats, rows, k, act, .. } => {
                        let (rf, fr) = match router {
                            Router::F32 => (1, 8),
                            Router::Rows(ty) => (0, kernels::format_arg(*ty)),
                        };
                        let wpe = kernels::route_warps(*router == Router::F32, *d);
                        let bytes = route_smem.entry((rf, fr, wpe)).or_insert(0);
                        *bytes = (*bytes).max(experts * 4);
                        if l.on_tensor_cores(f.schedule.numerics) {
                            let [fg, fu, fd] = formats.map(kernels::format_arg);
                            let bm = kernels::gemm_blk_expert_rows(rows * k, *experts);
                            if l.pairs_gate_and_up(f.schedule.numerics) {
                                gemm_blk.insert((fg, 3, bm, i32::from(*act != MoeAct::Swiglu), false));
                            } else {
                                gemm_blk.extend([(fg, 1, bm, 0, false), (fu, 1, bm, 0, false)]);
                            }
                            gemm_blk.insert((fd, 2, bm, 0, false));
                            if let Router::Rows(ty) = router {
                                gemm_blk.insert((kernels::format_arg(*ty), 0, kernels::GEMM_BLK_TILE_M, 0, false));
                            }
                        }
                    }
                    Launch::Attention { kc, vc, group, hd, vd, split, stream, .. } if l.attends_on_tensor_cores(f.schedule.numerics) => {
                        let kv = elems(&f.schedule.buffers, &[Some(kc), Some(vc), split.as_ref().map(|s| s.k_tail.0.as_str())]);
                        let te = split.as_ref().map_or(kc, |s| &s.k_tail.0);
                        let elem = |t: &str| f.schedule.buffers.get(t).map_or("float", |b| b.dtype.c_type());
                        let kernel = attention_mma_kernel(kernels::attn_tile_heads(*group), *hd, *vd, &kv, [elem(kc), elem(vc), elem(te)], *stream);
                        mma.insert((kernel, kernels::attn_mma_smem_bytes(*hd, *vd)));
                    }
                    Launch::Attention { kc, vc, group, hd, vd, split, .. } if l.attends_flash(f.schedule.numerics) => {
                        let kv = elems(&f.schedule.buffers, &[Some(kc), Some(vc), split.as_ref().map(|s| s.k_tail.0.as_str())]);
                        flash.insert((kernels::attn_tile_heads(*group), *hd, *vd, kv));
                    }
                    Launch::Attention { kc, vc, heads, hd, vd, shared: Some(_), quant, gate, .. } => {
                        let kv = elems(&f.schedule.buffers, &[Some(kc), Some(vc)]);
                        let kernel = attention_shared_kernel(*heads, *vd, *hd, kc == vc, &kv, &attention_qo(*quant, gate, m));
                        shared_smem.insert(kernel, kernels::attn_shared_smem_bytes(*heads, *hd));
                    }
                    Launch::Attention { kc, vc, hd, vd, heads, rows, per_block, splits, parallel, quant, gate, .. } if !l.attends_in_tiles(f.schedule.numerics) => {
                        let (acc, ty) = kernels::accumulator(m);
                        let z = if *parallel { *splits } else { 1 };
                        let mb = kernels::attn_blocks_a_multiprocessor(heads / per_block * rows * z, knobs.chip.sms, acc, *per_block, *hd, *vd);
                        let (smem, side) = attention_smem(*per_block, *hd, *vd, *splits, *parallel, acc);
                        let kv = elems(&f.schedule.buffers, &[Some(kc), Some(vc)]);
                        let kernel = attention_kernel(*per_block, *vd, ty, mb, &kv, &attention_qo(*quant, gate, m), kernels::attn_path(z, side, *splits), *hd);
                        let bytes = attn_smem.entry(kernel).or_insert(0);
                        *bytes = (*bytes).max(smem);
                    }
                    Launch::Gemv { format, .. } | Launch::GemmParts { format, .. } if l.on_tensor_cores(f.schedule.numerics) && l.reads_sixteen() => {
                        let parted = matches!(l, Launch::GemmParts { parts, .. } if h16_parted(parts, f.schedule.numerics));
                        gemm_h16.insert((kernels::format_arg(*format), if parted { 4 } else { 0 }, 0, h16_launch_tile(l, f.schedule.numerics, knobs.chip.sms)));
                    }
                    Launch::GemvGlu { formats: [fg, _], act, .. } if l.on_tensor_cores(f.schedule.numerics) && l.reads_sixteen() => {
                        gemm_h16.insert((kernels::format_arg(*fg), 5, act.map_or(-1, act_arg), h16_launch_tile(l, f.schedule.numerics, knobs.chip.sms)));
                    }
                    Launch::Gemv { format, .. } if l.on_tensor_cores(f.schedule.numerics) => {
                        gemm_blk.insert((kernels::format_arg(*format), 0, l.gemm_rows(f.schedule.numerics), 0, blk_plain(l, &f.schedule.buffers)));
                    }
                    Launch::GemvGlu { formats, .. } if l.batch_blk_format(f.schedule.numerics).is_some() => {
                        gemv_mma_blk.extend(formats.iter().map(|f| kernels::format_arg(*f)));
                    }
                    Launch::Gemv { .. } | Launch::GemmParts { .. } if l.batch_blk_format(f.schedule.numerics).is_some() => {
                        gemv_mma_blk.extend(l.batch_blk_format(f.schedule.numerics).map(kernels::format_arg));
                    }
                    Launch::Gemv { m: rows, .. } | Launch::GemmParts { m: rows, .. } | Launch::GemvGlu { m: rows, .. }
                        if l.batch_on_tensor_cores(f.schedule.numerics) =>
                    {
                        let (cols, k) = match l {
                            Launch::Gemv { n, k, .. } | Launch::GemvGlu { n, k, .. } => (*n, *k),
                            Launch::GemmParts { parts, k, .. } => (parts.iter().map(|p| p.n).sum(), *k),
                            _ => unreachable!(),
                        };
                        gemv_mma.extend((0..rows.div_ceil(kernels::GEMV_MMA_ROWS)).map(|s| {
                            kernels::gemv_mma_launch(kernels::GEMV_MMA_ROWS.min(rows - s * kernels::GEMV_MMA_ROWS), cols, k, knobs.chip.sms)
                        }));
                    }
                    Launch::GemmParts { format, .. } if l.on_tensor_cores(f.schedule.numerics) => {
                        gemm_blk.insert((kernels::format_arg(*format), 4, l.gemm_rows(f.schedule.numerics), 0, blk_plain(l, &f.schedule.buffers)));
                    }
                    Launch::GemvGlu { formats: [fg, _], act, .. } if l.on_tensor_cores(f.schedule.numerics) => {
                        gemm_blk.insert((kernels::format_arg(*fg), 5, l.gemm_rows(f.schedule.numerics), act.map_or(-1, act_arg), false));
                    }
                    _ => {}
                }
            }
        }
        // The launches over the form's streams (`branches`): the side streams fork from the step's own and join it at
        // the end; a launch waits on the events of the launches it follows on other streams, and records its own where
        // one waits on it. Under `--profile` every stream is the step's own, so each launch is timed alone; under
        // `--stagger` each launch follows a seeded stall on its stream.
        let b = branches::branch(&f.schedule, &f.arena);
        // The form's name in its step's identifiers, and its number for the profile and the stagger's seed: its rows, a
        // batch's negated.
        let r = f.tag();
        let key = if f.kind == FormKind::Batch { -(f.rows as i64) } else { f.rows as i64 };
        // A chunk before the prompt's last runs none of the launches only its output reads (`output_only`): each under
        // `lk_head`, the arg-max's advance apart — captured as a second graph, `exec…_lean`.
        let lean = output_only(f, forms);
        let leaned = lean.iter().any(|&x| x);
        let _ = writeln!(
            steps,
            "  cudaEvent_t ev{r}[{}], fork{r}; for(auto& e : ev{r}) CK(cudaEventCreateWithFlags(&e, cudaEventDisableTiming)); CK(cudaEventCreateWithFlags(&fork{r}, cudaEventDisableTiming));",
            b.events.max(1)
        );
        let mut body = format!(
            "  cudaStream_t S[{}] = {{st{}}};\n  CK(cudaEventRecord(fork{r}, st));\n",
            branches::STREAMS,
            (1..branches::STREAMS).map(|t| format!(", prof ? st : lk_side[{}]", t - 1)).collect::<String>()
        );
        for t in &b.sides {
            let _ = writeln!(body, "  CK(cudaStreamWaitEvent(S[{t}], fork{r}, 0));");
        }
        let staged = |l: &Launch| -> Vec<(String, Vec<(String, usize)>)> { stage(l, &f.schedule.buffers, plan) };
        let streams = f.schedule.launches.iter().any(|l| !staged(l).is_empty());
        if streams {
            let _ = writeln!(body, "  lk_stage_fork(st);");
        }
        for (i, l) in f.schedule.launches.iter().enumerate() {
            // Every step's launches are timed under `--profile`: an event before each, one after the last.
            if !matches!(l, Launch::View { .. }) {
                let _ = writeln!(body, "  PROF({prof_events});");
                prof.push((key, kind(l), l.writes()[0].to_string(), prof_events, l.weight_bytes()));
                prof_events += 1;
            }
            let mut stmts = String::new();
            launch(l, &n, f.schedule.numerics, &token, &mut stmts);
            if stmts.is_empty() {
                continue;
            }
            let t = b.stream[i];
            for e in &b.waits[i] {
                let _ = writeln!(body, "  CK(cudaStreamWaitEvent(S[{t}], ev{r}[{e}], 0));");
            }
            let _ = writeln!(body, "  if(stagger){{ lk_stall<<<1, 1, 0, S[{t}]>>>(lk_stall_ns(stagger_seed, {key}, {i})); CKL(); }}");
            // A matrix in host memory that streams is copied into a slot before the launch, which reads the slot.
            let staging = staged(l);
            if !staging.is_empty() {
                let mut ins = String::new();
                for (m, arrays) in &staging {
                    let rank = rank[m.as_str()];
                    let copies: String = arrays.iter().map(|(ptr, bytes)| format!(" lk_stage_in((void**)&{ptr}, {bytes}UL);")).collect();
                    let _ = writeln!(ins, "  if(lk_streaming && lk_moved > {rank}){{{copies} }}");
                }
                stmts = format!("{ins}  lk_stage_go(st);\n{stmts}  lk_stage_out(st);\n");
            }
            if lean[i] {
                let other = match l {
                    Launch::ArgMax { advance: Some(_), .. } => format!("  else lk_advance<<<1, 1, 0, S[{t}]>>>(d_pos, {}, d_gen, d_count);\n", f.rows),
                    _ => String::new(),
                };
                let _ = write!(body, "  if(lk_head){{ cudaStream_t st = S[{t}];\n{stmts}  }}\n{other}");
            } else {
                let _ = write!(body, "  {{ cudaStream_t st = S[{t}];\n{stmts}  }}\n");
            }
            if let Some(e) = b.records[i] {
                let _ = writeln!(body, "  CK(cudaEventRecord(ev{r}[{e}], S[{t}]));");
            }
        }
        for e in &b.joins {
            let _ = writeln!(body, "  CK(cudaStreamWaitEvent(st, ev{r}[{e}], 0));");
        }
        if streams {
            let _ = writeln!(body, "  lk_stage_join(st);");
        }
        let _ = writeln!(body, "  PROF({prof_events});");
        prof_events += 1;
        let _ = write!(steps, "  auto step{r} = [&](cudaStream_t st){{\n{body}  }};\n");
        for (exec, head) in std::iter::once(("", true)).chain(leaned.then_some(("_lean", false))) {
            let _ = writeln!(
                graphs,
                "  cudaGraphExec_t exec{r}{exec} = nullptr;\n  if(captured){{ cudaGraph_t g; CK(cudaStreamBeginCapture(st, cudaStreamCaptureModeGlobal)); lk_head = {head}; step{r}(st); \
                 CK(cudaStreamEndCapture(st, &g)); {{ size_t nodes = 0; CK(cudaGraphGetNodes(g, nullptr, &nodes)); lk_graph_nodes += nodes; }} \
                 CK(cudaGraphInstantiate(&exec{r}{exec}, g, 0)); }}"
            );
        }
        if f.kind == FormKind::Batch {
            // The batch step, and each sequence's logits in order.
            let logits: Vec<String> = std::iter::once(&f.schedule.io.logits).chain(f.schedule.io.more.iter().map(|q| &q.logits)).map(|t| host_logits(&f.schedule.io, t, &n)).collect();
            let _ = writeln!(batch, "  auto run_batch = [&]{{ if(captured) CK(cudaGraphLaunch(exec{r}, st)); else {{ lk_head = true; step{r}(st); }} }};");
            let _ = writeln!(batch, "  float* blogits[BATCH] = {{{}}};", logits.join(", "));
            let _ = writeln!(batch, "  int* d_btoks = {};", n.plain(&f.schedule.io.token));
            // Sequence `b`'s state in place of the decode step's and the chunks', so a prompt prefills into it.
            let state: Vec<String> = f.graph.of_role(Role::State).map(|t| t.name.clone()).collect();
            let mut cases = String::new();
            for seq in 0..f.rows {
                let suffix = format!("_s{seq}");
                let mut body = String::new();
                for name in state.iter().filter(|t| t.ends_with(&suffix)) {
                    let single = &name[..name.len() - suffix.len()];
                    if shared.contains(single) {
                        let _ = write!(body, " t_{} = {};", c(single), n.plain(name));
                        let buf = &f.schedule.buffers[name];
                        if buf.quantized {
                            let (to, from) = (format!("_{}", c(single)), n.qname(name));
                            let _ = write!(body, " q{to} = q{from}; l{to} = l{from}; s{to} = s{from};");
                        }
                    }
                }
                let _ = writeln!(cases, "      case {seq}:{body} break;");
            }
            let _ = writeln!(batch, "  auto slot = [&](int b){{\n    switch(b){{\n{cases}    }}\n  }};");
            batch_rows = f.rows;
        } else {
            let exec = if leaned { format!("head ? exec{r} : exec{r}_lean") } else { format!("exec{r}") };
            let _ = writeln!(
                runs,
                "    if(rows == {0}){{ if(captured && !lk_slot) CK(cudaGraphLaunch({exec}, st)); else {{ lk_head = head; step{r}(st); }} logits = {1}; return; }}",
                f.rows,
                host_logits(&f.schedule.io, &f.schedule.io.logits, &n)
            );
        }
    }
    if batch_rows == 0 {
        batch.push_str("  auto run_batch = [&]{};\n  float* blogits[1] = {nullptr};\n  int* d_btoks = nullptr;\n  auto slot = [&](int){};\n");
    }
    for (kernel, bytes) in attn_smem.into_iter().chain(shared_smem).filter(|(_, b)| *b > 48 * 1024) {
        let _ = writeln!(attrs, "  CK(cudaFuncSetAttribute({kernel}, cudaFuncAttributeMaxDynamicSharedMemorySize, {bytes}));");
    }
    for (kernel, bytes) in mma.into_iter().filter(|(_, b)| *b > 48 * 1024) {
        let _ = writeln!(attrs, "  CK(cudaFuncSetAttribute({kernel}, cudaFuncAttributeMaxDynamicSharedMemorySize, {bytes}));");
    }
    for (hg, hd, vd, kv) in flash {
        let (bq, nt) = (kernels::attn_flash_queries(hd, vd), kernels::attn_flash_threads(hd, vd));
        let _ = writeln!(
            attrs,
            "  CK(cudaFuncSetAttribute(lk_attention_flash<{hg}, {hd}, {vd}, {bq}, {nt}{kv}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {}));",
            kernels::attn_flash_smem_bytes(hd, vd)
        );
    }
    for (f, group, act, tn) in gemm_h16 {
        let _ = writeln!(
            attrs,
            "  CK(cudaFuncSetAttribute({}, cudaFuncAttributeMaxDynamicSharedMemorySize, {}));",
            gemm_h16_name(f, m, group, act, tn),
            kernels::gemm_h16_smem_bytes(numerics == Q8MatKind::TwoPassInt8, tn)
        );
    }
    for (f, group, bm, act, plain) in gemm_blk {
        let _ = writeln!(
            attrs,
            "  CK(cudaFuncSetAttribute({}, cudaFuncAttributeMaxDynamicSharedMemorySize, {}));",
            gemm_blk_name(f, m, group, bm, act, plain),
            kernels::gemm_blk_smem_bytes(f, two, bm)
        );
    }
    for f in gemv_mma_blk {
        let _ = writeln!(
            attrs,
            "  CK(cudaFuncSetAttribute(lk_gemv_mma_blk<{f}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {}));",
            kernels::gemv_mma_blk_smem_bytes(f)
        );
    }
    for (mt, nt, mw, nw) in gemv_mma {
        if kernels::gemv_mma_smem_bytes(two, mt, nt) > 48 * 1024 {
            let _ = writeln!(
                attrs,
                "  CK(cudaFuncSetAttribute(lk_gemv_mma<{m}, {mt}, {nt}, {mw}, {nw}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {}));",
                kernels::gemv_mma_smem_bytes(two, mt, nt)
            );
        }
    }
    for ((rf, fr, wpe), bytes) in route_smem.into_iter().filter(|(_, b)| *b > 48 * 1024) {
        let route = format!("lk_moe_route<{m}, {rf}, {fr}{}>", per_warp_arg(wpe));
        let _ = writeln!(attrs, "  CK(cudaFuncSetAttribute({route}, cudaFuncAttributeMaxDynamicSharedMemorySize, {bytes}));");
    }
    let list = |f: &dyn Fn(&(i64, &str, String, usize, usize)) -> String| prof.iter().map(f).collect::<Vec<_>>().join(", ");
    let profile = format!(
        "  static const int PROF_ROWS[] = {{{}}};\n  static const char* PROF_KIND[] = {{{}}};\n  static const char* PROF_OUT[] = {{{}}};\n\
         \x20 static const int PROF_EV[] = {{{}}};\n  static const unsigned long long PROF_BYTES[] = {{{}}};\n  const int PROF_N = {};\n\
         \x20 std::vector<cudaEvent_t> pev({prof_events});\n  if(prof) for(auto& e : pev) CK(cudaEventCreate(&e));\n\
         \x20 std::vector<double> pms(PROF_N, 0.0);\n  std::vector<int> pcount(PROF_N, 0);\n\
         #define PROF(k) do{{ if(prof) CK(cudaEventRecord(pev[k], st)); }}while(0)\n",
        list(&|p| p.0.to_string()),
        list(&|p| format!("\"{}\"", p.1)),
        list(&|p| format!("\"{}\"", p.2)),
        list(&|p| p.3.to_string()),
        list(&|p| format!("{}ULL", p.4)),
        prof.len()
    );
    let mut chunks: Vec<usize> = forms.iter().filter(|f| f.kind == FormKind::Prefill).map(|f| f.rows).collect();
    chunks.sort_unstable_by(|a, b| b.cmp(a));
    let chunk_tokens: String = forms
        .iter()
        .filter(|f| f.kind == FormKind::Prefill)
        .map(|f| format!("    if(T == {0}) CK(cudaMemcpyAsync(t{0}_{1}, d_prompt + at, {0} * 4, cudaMemcpyDeviceToDevice, st));\n", f.rows, c(&f.schedule.io.token)))
        .collect();
    // The vocabulary the decode step's arg-max picks from: its logits' length, read or not.
    let vocab = decode.schedule.launches.iter().find_map(|l| if let Launch::ArgMax { n, .. } = l { Some(*n) } else { None }).expect("the decode step picks its token");
    let _ = write!(
        cu,
        "static const size_t SIDECAR_BYTES = {}UL;\nstatic const int CHUNKS[] = {{{}}};\nstatic const int BATCH = {batch_rows};\nstatic const bool LOGITS = {};\n{HOST}{}\nint main(int argc, char** argv){{\n{ARGS}{PLACE}{SLOTS}{decls}{loads}{arena}{aliases}{scratch}{attrs}\
         \x20 int* d_pos = nullptr; lk_malloc(&d_pos, 4);\n\
         \x20 int* d_gen = nullptr; int* d_count = nullptr; int* d_prompt = nullptr;\n\
         \x20 lk_malloc(&d_gen, (size_t)CTX * 4); lk_malloc(&d_count, 4); CK(cudaMemset(d_count, 0, 4));\n\
         \x20 lk_malloc(&d_prompt, (size_t)CTX * 4);\n\
         \x20 int *d_bpos = nullptr, *d_bgen = nullptr, *d_bcount = nullptr;\n\
         \x20 if(BATCH > 0){{ lk_malloc(&d_bpos, (size_t)BATCH * 4); lk_malloc(&d_bgen, (size_t)BATCH * CTX * 4); lk_malloc(&d_bcount, (size_t)BATCH * 4); }}\n\
         \x20 if(lk_pool_at != lk_pool_bytes || lk_hpool_at != lk_hpool_bytes){{\n\
         \x20   fprintf(stderr, \"the program allocates %zu of its %zu pooled device bytes and %zu of its %zu host bytes\\n\", lk_pool_at, lk_pool_bytes, lk_hpool_at, lk_hpool_bytes);\n\
         \x20   return 5;\n\
         \x20 }}\n\
         \x20 printf(\"DEVICE_BYTES: %zu\\nHOST_BYTES: %zu\\nMOVED: %d\\nSTREAMED: %d\\n\", lk_device_bytes, lk_host_bytes, lk_moved, lk_streamed);\n\
         \x20 cudaStream_t st; CK(cudaStreamCreate(&st));\n\
         \x20 cudaStream_t lk_side[{}]; for(auto& s : lk_side) CK(cudaStreamCreateWithFlags(&s, cudaStreamNonBlocking));\n{profile}{steps}\
         \x20 const bool graphs = {};\n  const bool captured = graphs && !prof;\n  size_t lk_graph_nodes = 0;\n\
         \x20 size_t lk_graph_free = 0, lk_graph_total = 0; CK(cudaDeviceSynchronize()); CK(cudaMemGetInfo(&lk_graph_free, &lk_graph_total));\n{graphs}\
         \x20 if(captured){{ size_t f = 0, t = 0; CK(cudaDeviceSynchronize()); CK(cudaMemGetInfo(&f, &t)); printf(\"GRAPH_NODES: %zu\\nGRAPH_BYTES: %zu\\n\", lk_graph_nodes, lk_graph_free - f); }}\n\
         \x20 float* logits = nullptr;\n\
         \x20 bool lk_slot = false;\n\
         \x20 auto run = [&](int rows, bool head){{\n{runs}  }};\n{batch}\
         \x20 auto reset = [&]{{\n{resets}    CK(cudaMemsetAsync(d_count, 0, 4, st));\n    CK(cudaStreamSynchronize(st));\n\
         \x20   int one = 1; CK(cudaMemcpy(d_pos, &one, 4, cudaMemcpyHostToDevice));\n  }};\n\
         \x20 const int vocab = {vocab};\n{}}}\n",
        sidecar.len(),
        if chunks.is_empty() { "0".to_string() } else { chunks.iter().map(usize::to_string).collect::<Vec<_>>().join(", ") },
        decode.schedule.io.logits_read,
        format!(
            "{}{}{}",
            if mxfp4 { HOST_MXFP4 } else { "" },
            if q6k { HOST_Q6K } else { "" },
            if sixteen { HOST_H16 } else { "" }
        ),
        branches::STREAMS - 1,
        if knobs.cuda_graphs { "true" } else { "false" },
        DRIVE.replace("t_tok_", &token).replace("/*CHUNK_TOKENS*/\n", &chunk_tokens),
        HOST = HOST,
        ARGS = ARGS,
        PLACE = PLACE,
        SLOTS = SLOTS,
    );
    Ok(Program { cu, sidecar })
}

/// The streamed matrices launch `l` reads (`residence::Plan::streamed`), each with the arrays a staging copies — the
/// pointer the launch reads it through and its bytes: a Q8_0 matrix's levels and scales, any other's rows.
fn stage(l: &Launch, buffers: &BTreeMap<String, Buffer>, plan: &Plan) -> Vec<(String, Vec<(String, usize)>)> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for t in l.parts().into_iter().flat_map(|p| p.reads()) {
        let m = plan.shares.get(t).map_or(t, String::as_str);
        let Some(bytes) = plan.streamed.get(m) else { continue };
        if !seen.insert(t) {
            continue;
        }
        let q8 = matches!(buffers.get(t).map(|b| &b.fill), Some(Fill::Constant(Placement::Rows(r))) if r.ty == GgmlType::Q8_0);
        let ptrs = if q8 { vec![rows_q(t), rows_s(t)] } else { vec![rows_b(t)] };
        out.push((m.to_string(), ptrs.into_iter().zip(bytes.iter().copied()).collect()));
    }
    out
}

/// The copies the program's graphs capture where every streamed matrix streams (`residence::Plan::copies`): each array
/// a staging copies, for every form's step and a chunk's lean step, as `captured_nodes` counts their launches.
pub fn captured_copies(forms: &[Form], plan: &Plan) -> usize {
    forms
        .iter()
        .map(|f| {
            let lean = output_only(f, forms);
            let copies = |l: &Launch| stage(l, &f.schedule.buffers, plan).iter().map(|(_, a)| a.len()).sum::<usize>();
            let whole: usize = f.schedule.launches.iter().map(copies).sum();
            let kept: usize = f.schedule.launches.iter().zip(&lean).filter(|(_, &out)| !out).map(|(l, _)| copies(l)).sum();
            whole + if lean.iter().any(|&x| x) { kept } else { 0 }
        })
        .sum()
}

/// The device array the host takes a sequence's logits `t` from after its step: none where the host reads no logits.
fn host_logits(io: &StepIo, t: &str, n: &Names) -> String {
    if io.logits_read {
        n.plain(t)
    } else {
        "nullptr".to_string()
    }
}

/// The host helpers: the checkpoint's digest and the upload of a quantized matrix's rows.
const HOST: &str = r#"
static uint64_t fnv1a64(const uint8_t* p, size_t n){
  uint64_t h = 0xcbf29ce484222325ULL;
  for(size_t i = 0; i < n; ++i){ h ^= p[i]; h *= 0x100000001b3ULL; }
  return h;
}
// Every standing allocation the program makes, in bytes: on the device (the arena and the pool), and in pinned host
// memory the device maps.
static size_t lk_device_bytes = 0, lk_host_bytes = 0;
// The device pool every array but the arena is carved from, and the mapped host pool every moved matrix is, each at
// `LK_ALIGN`: their bytes, and how far each is carved.
static unsigned char *lk_pool = nullptr, *lk_hpool = nullptr;
static size_t lk_pool_bytes = 0, lk_pool_at = 0, lk_hpool_bytes = 0, lk_hpool_at = 0;
static void* lk_carve(unsigned char* pool, size_t bytes, size_t& at, size_t& counted, size_t n, const char* what){
  n = (n + LK_ALIGN - 1) / LK_ALIGN * LK_ALIGN;
  if(at + n > bytes){ fprintf(stderr, "the program allocates past its %zu %s bytes\n", bytes, what); exit(5); }
  void* p = pool + at;
  at += n;
  counted += n;
  return p;
}
template<class T> static void lk_malloc(T** p, size_t n){
  *p = (T*)lk_carve(lk_pool, lk_pool_bytes, lk_pool_at, lk_device_bytes, n, "pooled device");
}
// A matrix past the device's memory, in the mapped host pool, by the pointer a kernel reads it through.
template<class T> static void lk_mapped(T** p, size_t n){
  *p = (T*)lk_carve(lk_hpool, lk_hpool_bytes, lk_hpool_at, lk_host_bytes, n, "host");
}
template<class T> static void lk_alloc(T** p, size_t n, bool host){ if(host) lk_mapped(p, n); else lk_malloc(p, n); }
// Copy a Q8_0 matrix's rows out of the checkpoint (34-byte blocks: an f16 scale, 32 int8) into int8 rows and their
// scales, the layout the GEMV reads, and upload both — to the device, or to mapped host memory when `host`.
static void upload_rows(const uint8_t* file, const uint64_t* runs, int nruns, long rows, int cols,
                        signed char** q, __half** s, bool host){
  int nb = cols / 32;
  std::vector<signed char> hq((size_t)rows * cols);
  std::vector<__half> hs((size_t)rows * nb);
  long r = 0;
  for(int k = 0; k < nruns; ++k){
    const uint8_t* src = file + runs[2 * k];
    for(uint64_t i = 0; i < runs[2 * k + 1]; ++i, ++r)
      for(int b = 0; b < nb; ++b){
        const uint8_t* blk = src + (i * nb + b) * 34;
        memcpy(&hs[(size_t)r * nb + b], blk, 2);
        memcpy(&hq[(size_t)r * cols + b * 32], blk + 2, 32);
      }
  }
  lk_alloc(q, (size_t)rows * cols, host); lk_alloc(s, (size_t)rows * nb * 2, host);
  CK(cudaMemcpy(*q, hq.data(), (size_t)rows * cols, cudaMemcpyDefault));
  CK(cudaMemcpy(*s, hs.data(), (size_t)rows * nb * 2, cudaMemcpyDefault));
}
// The rows of a matrix kept in its checkpoint format, `row_bytes` a row, as the runs of the file lay them out, then 32
// zero bytes: a staged read of a row's units runs up to 30 bytes past them. On the device, or in mapped host memory
// when `host`.
static void upload_raw(const uint8_t* file, const uint64_t* runs, int nruns, long rows, long row_bytes, unsigned char** b,
                       bool host){
  lk_alloc(b, (size_t)rows * row_bytes + 32, host);
  static const unsigned char zeros[32] = {0};
  CK(cudaMemcpy(*b + (size_t)rows * row_bytes, zeros, 32, cudaMemcpyDefault));
  long r = 0;
  for(int k = 0; k < nruns; ++k){
    CK(cudaMemcpy(*b + (size_t)r * row_bytes, file + runs[2 * k], (size_t)runs[2 * k + 1] * row_bytes, cudaMemcpyDefault));
    r += runs[2 * k + 1];
  }
}
"#;

/// The upload of a 16-bit matrix the tensor cores read, in a program that keeps one.
const HOST_H16: &str = r#"
// A 16-bit float matrix's rows out of the checkpoint interleaved as the tensor cores read them, `lk_h16_group(rows)` to a
// group (`lk_h16_interleave`), its group in `g`, then 32 zero bytes; on the device, or in mapped host memory when `host`.
static void upload_h16(const uint8_t* file, const uint64_t* runs, int nruns, long rows, long cols, unsigned char** b, int* g,
                       bool host){
  const long rb = cols * 2;
  std::vector<unsigned char> kept((size_t)rows * rb), h((size_t)rows * rb + 32, 0);
  long r = 0;
  for(int k = 0; k < nruns; ++k){
    memcpy(kept.data() + (size_t)r * rb, file + runs[2 * k], (size_t)runs[2 * k + 1] * rb);
    r += (long)runs[2 * k + 1];
  }
  *g = lk_h16_group(rows);
  lk_h16_interleave(kept.data(), rows, cols, *g, h.data());
  lk_alloc(b, h.size(), host);
  CK(cudaMemcpy(*b, h.data(), h.size(), cudaMemcpyDefault));
}
"#;

/// The upload of an MXFP4 matrix, in a program that keeps one.
const HOST_MXFP4: &str = r#"
// An MXFP4 matrix's rows out of the checkpoint regrouped as the kernels read them (`lk_mx_regroup`), then the 256 zero
// bytes a staged read of a row's last group runs past it; on the device, or in mapped host memory when `host`.
static void upload_mxfp4(const uint8_t* file, const uint64_t* runs, int nruns, long rows, int cols, unsigned char** b,
                         bool host){
  long nb = cols / 32, rb = lk_mx_row_bytes(nb);
  std::vector<unsigned char> h((size_t)rows * rb + 256, 0);
  long r = 0;
  for(int k = 0; k < nruns; ++k){
    lk_mx_regroup(file + runs[2 * k], (long)runs[2 * k + 1], nb, h.data() + r * rb);
    r += (long)runs[2 * k + 1];
  }
  lk_alloc(b, h.size(), host);
  CK(cudaMemcpy(*b, h.data(), h.size(), cudaMemcpyDefault));
}
"#;

/// The upload of a Q6_K matrix, in a program that keeps one.
const HOST_Q6K: &str = r#"
// A Q6_K matrix's rows out of the checkpoint grouped as the kernels read them (`lk_q6_regroup`), then the 32 zero bytes
// every block-format matrix keeps past its rows; on the device, or in mapped host memory when `host`.
static void upload_q6k(const uint8_t* file, const uint64_t* runs, int nruns, long rows, int cols, unsigned char** b,
                       bool host){
  long nb = cols / 256, rb = lk_q6_row_bytes(nb);
  std::vector<unsigned char> h((size_t)rows * rb + 32, 0);
  long r = 0;
  for(int k = 0; k < nruns; ++k){
    lk_q6_regroup(file + runs[2 * k], (long)runs[2 * k + 1], nb, h.data() + r * rb);
    r += (long)runs[2 * k + 1];
  }
  lk_alloc(b, h.size(), host);
  CK(cudaMemcpy(*b, h.data(), h.size(), cudaMemcpyDefault));
}
"#;

/// The command line — one prompt, or a file of them — the checkpoint and the sidecar.
const ARGS: &str = r#"  setenv("CUDA_MODULE_LOADING", "EAGER", 1);
  bool prof = false, stagger = false, whole = false;
  unsigned long long stagger_seed = 0;
  size_t device_bytes = SIZE_MAX;
  std::vector<char*> av;
  for(int i = 0; i < argc; ++i){
    std::string s = argv[i];
    if(s == "--profile") prof = true;
    else if(s == "--whole-chunks") whole = true;
    else if(s == "--stagger" && i + 1 < argc){ stagger = true; stagger_seed = strtoull(argv[++i], nullptr, 10); }
    else if(s == "--device-bytes" && i + 1 < argc) device_bytes = strtoull(argv[++i], nullptr, 10);
    else av.push_back(argv[i]);
  }
  argc = (int)av.size(); argv = av.data();
  if(argc < 5){
    fprintf(stderr, "usage: %s <checkpoint.gguf> <sidecar> <n_gen> [--logits <file>] <token> ...\n"
                    "       %s <checkpoint.gguf> <sidecar> --prompts <file> [--logits <file>]  (a line a prompt: <n_gen> <token> ...)\n"
                    "       %s <checkpoint.gguf> <sidecar> --batch <file> [--logits <file>]  (the batch step's prompts, decoded together)\n"
                    "       --profile anywhere times every step, the decode step and each prefill chunk, launch by launch\n"
                    "       --whole-chunks anywhere runs every prefill chunk whole, the output of a chunk before the prompt's last too\n"
                    "       --stagger <seed> anywhere holds each launch of a step back by a stall drawn from the seed\n"
                    "       --device-bytes <n> anywhere keeps the arrays to n bytes of the device, the matrices past them in host memory\n",
            argv[0], argv[0], argv[0]);
    return 2;
  }
  std::vector<std::pair<int, std::vector<int>>> jobs;
  FILE* logits_out = nullptr;
  int a = 4;
  const bool batch_mode = std::string(argv[3]) == "--batch";
  if(std::string(argv[3]) == "--prompts" || batch_mode){
    FILE* pf = fopen(argv[4], "r");
    if(!pf){ perror("prompts"); return 2; }
    std::vector<char> line(1 << 20);
    while(fgets(line.data(), (int)line.size(), pf)){
      std::vector<int> v;
      char* p = line.data(); char* e = nullptr;
      for(long x = strtol(p, &e, 10); e != p; x = strtol(p, &e, 10)){ v.push_back((int)x); p = e; }
      if(v.empty()) continue;
      std::vector<int> prompt;
      for(size_t i = 1; i < v.size(); ++i) prompt.push_back(v[i] + 1);
      jobs.push_back({v[0], prompt});
    }
    fclose(pf);
    a = 5;
    if(a + 1 < argc && std::string(argv[a]) == "--logits"){ logits_out = fopen(argv[a + 1], "wb"); a += 2; }
  } else {
    const int n_gen = atoi(argv[3]);
    if(a + 1 < argc && std::string(argv[a]) == "--logits"){ logits_out = fopen(argv[a + 1], "wb"); a += 2; }
    std::vector<int> prompt;
    for(; a < argc; ++a) prompt.push_back(atoi(argv[a]) + 1);
    jobs.push_back({n_gen, prompt});
  }
  if(logits_out && !LOGITS){ fprintf(stderr, "this program writes no logits: its host takes the next tokens alone\n"); return 2; }
  if(batch_mode){
    if(BATCH == 0){ fprintf(stderr, "this program has no batch step\n"); return 2; }
    if((int)jobs.size() != BATCH){ fprintf(stderr, "the batch step decodes %d sequences: %zu prompts given\n", BATCH, jobs.size()); return 2; }
    for(auto& job : jobs) if(job.first != jobs[0].first){ fprintf(stderr, "a batch's sequences generate as many tokens each\n"); return 2; }
  }
  for(auto& job : jobs){
    const std::vector<int>& prompt = job.second;
    int n_gen = job.first;
    if(prompt.empty() || n_gen < 1 || (int)prompt.size() + n_gen - 1 > CTX){
      fprintf(stderr, "a prompt of %zu tokens and %d generated need %zu positions; the caches hold %d\n",
              prompt.size(), n_gen, prompt.size() + n_gen - 1, CTX);
      return 2;
    }
  }
  int fd = open(argv[1], O_RDONLY); if(fd < 0){ perror("checkpoint"); return 3; }
  struct stat sb; fstat(fd, &sb);
  if((uint64_t)sb.st_size != CKPT_SIZE){ fprintf(stderr, "%s is not the checkpoint this program was compiled against\n", argv[1]); return 3; }
  const uint8_t* file = (const uint8_t*)mmap(nullptr, sb.st_size, PROT_READ, MAP_SHARED, fd, 0);
  if(file == MAP_FAILED){ perror("mmap"); return 3; }
  if(fnv1a64(file, CKPT_DATA) != CKPT_DIGEST){ fprintf(stderr, "%s is not the checkpoint this program was compiled against\n", argv[1]); return 3; }
  std::vector<uint8_t> side(SIDECAR_BYTES);
  FILE* sf = fopen(argv[2], "rb");
  if(!sf || fread(side.data(), 1, SIDECAR_BYTES, sf) != SIDECAR_BYTES || fgetc(sf) != EOF){ fprintf(stderr, "%s is not this program's sidecar\n", argv[2]); return 3; }
  fclose(sf);
"#;

/// The staging of the matrices in host memory that stream (`residence`): two device slots, the copy stream that fills
/// them and its events. Before a launch, each of its streamed matrices is copied into the slot whose last reader is
/// done, and the launch's pointer to it names the copy (`lk_stage_in`); the launch's stream waits for the copies
/// (`lk_stage_go`); after it the slot is free once the launch is, the pointers name host memory again and the next
/// launch takes the other slot (`lk_stage_out`). A step's copies fork from its stream with both slots free and join it at the end.
const STAGE: &str = r#"static bool lk_streaming = false;
static int lk_streamed = 0;
static unsigned char* lk_stage_slot[2] = {nullptr, nullptr};
static cudaStream_t lk_stage_cs = nullptr;
static cudaEvent_t lk_stage_full[2], lk_stage_free[2], lk_stage_cfork, lk_stage_cjoin;
static int lk_stage_k = 0, lk_stage_n = 0;
static size_t lk_stage_at = 0;
static void** lk_stage_was_p[LK_STAGE_ARRAYS];
static void* lk_stage_was[LK_STAGE_ARRAYS];
static void lk_stage_in(void** p, size_t n){
  if(lk_stage_n == 0) CK(cudaStreamWaitEvent(lk_stage_cs, lk_stage_free[lk_stage_k], 0));
  unsigned char* d = lk_stage_slot[lk_stage_k] + lk_stage_at;
  lk_stage_at += n;
  CK(cudaMemcpyAsync(d, *p, n, cudaMemcpyDefault, lk_stage_cs));
  lk_stage_was_p[lk_stage_n] = p; lk_stage_was[lk_stage_n] = *p; ++lk_stage_n;
  *p = d;
}
static void lk_stage_go(cudaStream_t st){
  if(lk_stage_n){ CK(cudaEventRecord(lk_stage_full[lk_stage_k], lk_stage_cs)); CK(cudaStreamWaitEvent(st, lk_stage_full[lk_stage_k], 0)); }
}
static void lk_stage_out(cudaStream_t st){
  if(!lk_stage_n) return;
  CK(cudaEventRecord(lk_stage_free[lk_stage_k], st));
  for(int i = 0; i < lk_stage_n; ++i) *lk_stage_was_p[i] = lk_stage_was[i];
  lk_stage_n = 0; lk_stage_at = 0; lk_stage_k ^= 1;
}
static void lk_stage_fork(cudaStream_t st){
  if(!lk_streaming) return;
  CK(cudaEventRecord(lk_stage_cfork, st)); CK(cudaStreamWaitEvent(lk_stage_cs, lk_stage_cfork, 0));
  CK(cudaEventRecord(lk_stage_free[0], st)); CK(cudaEventRecord(lk_stage_free[1], st));
}
static void lk_stage_join(cudaStream_t st){
  if(!lk_streaming) return;
  CK(cudaEventRecord(lk_stage_cjoin, lk_stage_cs)); CK(cudaStreamWaitEvent(st, lk_stage_cjoin, 0));
}
"#;

/// The staging slots and the copy stream, where the moved matrices stream: the slots an allocation beside the arrays.
const SLOTS: &str = r#"  if(lk_streaming){
    CK(cudaMalloc(&lk_stage_slot[0], LK_SLOT)); CK(cudaMalloc(&lk_stage_slot[1], LK_SLOT));
    CK(cudaStreamCreateWithFlags(&lk_stage_cs, cudaStreamNonBlocking));
    for(int i = 0; i < 2; ++i){ CK(cudaEventCreateWithFlags(&lk_stage_full[i], cudaEventDisableTiming)); CK(cudaEventCreateWithFlags(&lk_stage_free[i], cudaEventDisableTiming)); }
    CK(cudaEventCreateWithFlags(&lk_stage_cfork, cudaEventDisableTiming)); CK(cudaEventCreateWithFlags(&lk_stage_cjoin, cudaEventDisableTiming));
  }
"#;

/// Where the matrices live, decided as the program starts: on the device memory free once the context is up and its
/// modules loaded (or the `--device-bytes` given), less what its graphs take, as many of the matrices as must move to
/// mapped host memory in the compiled order (`residence::place`); then the device pool and the host pool, carved by
/// every allocation after.
const PLACE: &str = r#"  CK(cudaFree(0));
  size_t lk_free = 0, lk_total = 0;
  CK(cudaMemGetInfo(&lk_free, &lk_total));
  const size_t lk_budget = device_bytes != SIZE_MAX ? device_bytes : (lk_free > LK_SPARE ? lk_free - LK_SPARE : 0);
  if(LK_ARENA + LK_FIXED > lk_budget){
    fprintf(stderr, "the caches, arena and buffers take %zu bytes of the device's %zu, every matrix moved to the host\n", LK_ARENA + LK_FIXED, lk_budget);
    return 4;
  }
  size_t lk_device = 0, lk_host = 0;
  int lk_moved = 0;
  auto lk_down = [&](size_t budget){
    lk_device = LK_ARENA + LK_FIXED; lk_host = 0; lk_moved = 0;
    for(int i = 0; i < LK_MOVABLE; ++i) lk_device += LK_ORDER[i];
    while(lk_device > budget){ lk_device -= LK_ORDER[lk_moved]; lk_host += LK_ORDER[lk_moved]; ++lk_moved; }
  };
  lk_down(lk_budget);
  // A moved matrix that streams: its slots and its copies' graph memory out of the memory found free before the
  // arrays are placed again, beside a budget given (`residence::staging`).
  for(int i = 0; i < lk_moved; ++i) lk_streaming |= LK_STREAMS[i] != 0;
  if(lk_streaming && device_bytes == SIZE_MAX){
    const size_t staging = 2 * LK_SLOT + LK_STAGE_SPARE;
    if(LK_ARENA + LK_FIXED + staging <= lk_budget) lk_down(lk_budget - staging);
    else lk_streaming = false;
  }
  if(lk_streaming) for(int i = 0; i < lk_moved; ++i) lk_streamed += LK_STREAMS[i];
  lk_pool_bytes = lk_device - LK_ARENA;
  CK(cudaMalloc(&lk_pool, lk_pool_bytes));
  if(lk_host){
    void* h = nullptr;
    CK(cudaHostAlloc(&h, lk_host, cudaHostAllocMapped));
    CK(cudaHostGetDevicePointer((void**)&lk_hpool, h, 0));
    lk_hpool_bytes = lk_host;
  }
"#;

/// The generation, prompt after prompt: the carried state and the generated run reset, the prompt as the largest chunks
/// that fit, then one decode step per generated token.
const DRIVE: &str = r#"  cudaEvent_t e0, e1, e2; CK(cudaEventCreate(&e0)); CK(cudaEventCreate(&e1)); CK(cudaEventCreate(&e2));
  if(batch_mode){
    // Each sequence's prompt prefilled into its own state, as a prompt alone runs; then every sequence a token a step
    // in the batch step. Each sequence's logits are written in its turn, as `--prompts` writes a prompt's.
    const int n_gen = jobs[0].first, one = 1;
    std::vector<std::vector<float>> bl(BATCH);
    auto bdump = [&](int b, const float* p){
      if(!logits_out) return;
      size_t at = bl[b].size();
      bl[b].resize(at + vocab);
      CK(cudaMemcpyAsync(bl[b].data() + at, p, (size_t)vocab * 4, cudaMemcpyDeviceToHost, st));
      CK(cudaStreamSynchronize(st));
    };
    reset();
    lk_slot = true;
    CK(cudaEventRecord(e0, st));
    for(int b = 0; b < BATCH; ++b){
      const std::vector<int>& prompt = jobs[b].second;
      slot(b);
      CK(cudaMemsetAsync(d_count, 0, 4, st));
      CK(cudaMemcpyAsync(d_pos, &one, 4, cudaMemcpyHostToDevice, st));
      CK(cudaMemcpyAsync(d_prompt, prompt.data(), prompt.size() * 4, cudaMemcpyHostToDevice, st));
      for(int at = 0; at < (int)prompt.size(); ){
        int T = 1;
        for(int c : CHUNKS) if(c > 1 && c <= (int)prompt.size() - at){ T = c; break; }
        if(T == 1) CK(cudaMemcpyAsync(t_tok_, d_prompt + at, 4, cudaMemcpyDeviceToDevice, st));
/*CHUNK_TOKENS*/
        run(T, whole || at + T >= (int)prompt.size());
        at += T;
      }
      // The sequence's first generated token, its next position and its run into the batch's slot.
      CK(cudaMemcpyAsync(d_btoks + b, t_tok_, 4, cudaMemcpyDeviceToDevice, st));
      CK(cudaMemcpyAsync(d_bpos + b, d_pos, 4, cudaMemcpyDeviceToDevice, st));
      CK(cudaMemcpyAsync(d_bgen + (size_t)b * CTX, t_tok_, 4, cudaMemcpyDeviceToDevice, st));
      CK(cudaMemcpyAsync(d_bcount + b, &one, 4, cudaMemcpyHostToDevice, st));
      bdump(b, logits);
    }
    lk_slot = false;
    CK(cudaEventRecord(e1, st));
    for(int g = 1; g < n_gen; ++g){
      run_batch();
      for(int b = 0; b < BATCH; ++b) bdump(b, blogits[b]);
    }
    CK(cudaEventRecord(e2, st));
    CK(cudaStreamSynchronize(st));
    CK(cudaGetLastError());
    float prefill_ms = 0.f, decode_ms = 0.f;
    CK(cudaEventElapsedTime(&prefill_ms, e0, e1)); CK(cudaEventElapsedTime(&decode_ms, e1, e2));
    std::vector<int> gen((size_t)BATCH * CTX);
    CK(cudaMemcpy(gen.data(), d_bgen, gen.size() * 4, cudaMemcpyDeviceToHost));
    for(int b = 0; b < BATCH; ++b){
      printf("TOKENS:");
      for(int g = 0; g < n_gen; ++g) printf(" %d", gen[(size_t)b * CTX + g] - 1);
      printf("\n");
      if(logits_out) fwrite(bl[b].data(), 4, bl[b].size(), logits_out);
    }
    printf("PREFILL_MS: %.3f\n", prefill_ms);
    if(n_gen > 1) printf("BATCH: %d\nDECODE_MS_PER_STEP: %.4f\nDECODE_TOK_PER_S: %.2f\n", BATCH, decode_ms / (n_gen - 1), 1000.0 * BATCH * (n_gen - 1) / decode_ms);
    if(logits_out) fclose(logits_out);
    return 0;
  }
  std::vector<float> host_logits(logits_out ? vocab : 0);
  auto dump = [&]{
    if(!logits_out) return;
    CK(cudaMemcpyAsync(host_logits.data(), logits, (size_t)vocab * 4, cudaMemcpyDeviceToHost, st));
    CK(cudaStreamSynchronize(st));
    fwrite(host_logits.data(), 4, vocab, logits_out);
  };
  // A step of `rows`, its launches timed one by one under `--profile`.
  auto timed = [&](int rows, bool head){
    run(rows, head);
    if(!prof) return;
    CK(cudaStreamSynchronize(st));
    for(int k = 0; k < PROF_N; ++k) if(PROF_ROWS[k] == rows){
      float ms = 0.f; CK(cudaEventElapsedTime(&ms, pev[PROF_EV[k]], pev[PROF_EV[k] + 1])); pms[k] += ms; ++pcount[k];
    }
  };
  for(auto& job : jobs){
    const std::vector<int>& prompt = job.second;
    const int n_gen = job.first;
    reset();
    CK(cudaMemcpy(d_prompt, prompt.data(), prompt.size() * 4, cudaMemcpyHostToDevice));
    CK(cudaEventRecord(e0, st));
    int steps = 0, lean_chunks = 0;
    for(int at = 0; at < (int)prompt.size(); ){
      int T = 1;
      for(int c : CHUNKS) if(c > 1 && c <= (int)prompt.size() - at){ T = c; break; }
      if(T == 1) CK(cudaMemcpyAsync(t_tok_, d_prompt + at, 4, cudaMemcpyDeviceToDevice, st));
/*CHUNK_TOKENS*/
      const bool head = whole || at + T >= (int)prompt.size();
      lean_chunks += !head;
      timed(T, head);
      at += T; ++steps;
    }
    CK(cudaEventRecord(e1, st));
    dump();
    for(int g = 1; g < n_gen; ++g){
      timed(1, true);
      dump(); ++steps;
    }
    CK(cudaEventRecord(e2, st));
    CK(cudaStreamSynchronize(st));
    CK(cudaGetLastError());
    float prefill_ms = 0.f, decode_ms = 0.f;
    CK(cudaEventElapsedTime(&prefill_ms, e0, e1)); CK(cudaEventElapsedTime(&decode_ms, e1, e2));
    std::vector<int> gen(steps);
    CK(cudaMemcpy(gen.data(), d_gen, (size_t)steps * 4, cudaMemcpyDeviceToHost));
    printf("TOKENS:");
    for(int g = steps - n_gen; g < steps; ++g) printf(" %d", gen[g] - 1);
    printf("\nPREFILL_MS: %.3f\nLEAN_CHUNKS: %d\n", prefill_ms, lean_chunks);
    if(n_gen > 1) printf("DECODE_MS_PER_TOKEN: %.4f\nDECODE_TOK_PER_S: %.2f\n", decode_ms / (n_gen - 1), 1000.0 * (n_gen - 1) / decode_ms);
    fflush(stdout);
  }
  if(prof){
    // The decode step's launches, then each chunk's that ran: a launch's mean time over its step's runs, the matrix bytes
    // it streams and their rate.
    double total = 0.0;
    for(int k = 0; k < PROF_N; ++k) if(PROF_ROWS[k] == 1 && pcount[k]){
      double us = 1000.0 * pms[k] / pcount[k];
      printf("PROFILE: %s %s %.3f %llu %.1f\n", PROF_KIND[k], PROF_OUT[k], us, PROF_BYTES[k], PROF_BYTES[k] / us / 1e3); total += us;
    }
    if(total > 0.0) printf("PROFILE_TOTAL_US: %.3f\n", total);
    for(int c : CHUNKS){
      if(c <= 1) continue;
      double chunk = 0.0;
      for(int k = 0; k < PROF_N; ++k) if(PROF_ROWS[k] == c && pcount[k]){
        double us = 1000.0 * pms[k] / pcount[k];
        printf("PROFILE_CHUNK: %d %s %s %.3f %llu %.1f\n", c, PROF_KIND[k], PROF_OUT[k], us, PROF_BYTES[k], PROF_BYTES[k] / us / 1e3); chunk += us;
      }
      if(chunk > 0.0) printf("PROFILE_CHUNK_TOTAL_US: %d %.3f\n", c, chunk);
    }
  }
  if(logits_out) fclose(logits_out);
  return 0;
"#;
