//! qor_harness — the measurement harness of the comparative-synthesis bench.
//!
//! Every candidate implementation of a kernel (LOGOS's own RTL, an HLS tool's output, a third-party
//! design) is measured inside the same generated wrapper, `qorbist_<candidate>(clk, rst, sig)`:
//!
//! * **Stimulus.** A 64-bit xorshift state (13, 7, 17) seeded with [`SEED`] advances once per
//!   operation the candidate *accepts*. Each input field reads its own lane word
//!   ([`lane_word`]: a rotation of the state XOR a per-lane constant) and shapes it into the
//!   kernel's declared domain ([`Field`]). Values never depend on the candidate's timing, so every
//!   candidate of a kernel sees the same operand sequence.
//! * **Signature.** Each result the candidate *produces* folds into a 64-bit MISR
//!   ([`misr_step`] over [`fold`]). The MISR advances on result-valid only, so the signature is
//!   independent of latency, initiation interval and stalls: two candidates of one kernel are
//!   functionally equal on the stimulus exactly when their signatures are, and both must equal the
//!   golden [`signature`] the LOGOS VM (or the oracle the RTL generator is locked to) predicts.
//! * **Boundary.** The candidate is instantiated under `(* keep_hierarchy *)` as `u_dut`, so
//!   synthesis cannot fold the shaped stimulus into it and its area and power are reported for that
//!   instance alone. An input its binding does not name is held at 0, never left floating. The wrapper exposes only `clk`, `rst` and a 16-bit `sig`, which is what the
//!   gate-level power flow drives.
//!
//! A kernel is a [`Kernel`] contract (ports, widths, domains, golden); a candidate is a
//! [`Candidate`] (top module, clock, reset, [`Protocol`], and a port binding). [`check_ports`]
//! refuses a binding that does not fit the DUT's real ports before any harness is emitted.

use crate::codegen_sva::decision_workload::{DECIDE_FN, DECIDE_NAME};
use crate::codegen_sva::fabric_pipe::{
    carry_save_lower_with, decision_ops, emit_fabric_pipelined_verilog, horner_seq_pipeline, schedule_pipeline, FinalAdd,
    Inputs, PipelineSchedule, SumSharing, DECISION_CELLS_BUDGET, DECISION_CHAIN_BUDGET,
};
use crate::codegen_sva::select_hoist::hoist_selects;
use crate::codegen_sva::fabric_rtl::{
    decision_fabric_from_source, emit_decision_fabric_verilog, emit_fabric_verilog, emit_seq_datapath_verilog,
    residual_from_source, DecisionFabric, FabricOp, POLY_BODY,
};
use crate::codegen_sva::fabric_rtl::AluOp;
use crate::codegen_sva::gpu_simt_rtl::{emit_gpu_simt_verilog, load_slots, LaneArith, SimtEngine, Slot};
use crate::codegen_sva::mul_tree::mul_tree_levels;
use crate::codegen_sva::simt_ports::emit_simt_ports_verilog;
use crate::codegen_sva::keccak_rtl::{
    emit_keccak_core_half_verilog, emit_keccak_core_verilog, emit_ntt_butterfly_pipelined_verilog, emit_ntt_butterfly_verilog,
    fqmul, NttCuts,
};
use crate::codegen_sva::systolic::{
    emit_systolic_os_streaming_verilog, emit_systolic_pipelined_loadable_verilog, os_streaming_result_offset,
    pipelined_latency, SystolicOs,
};

/// The stimulus state after reset.
pub const SEED: u64 = 0x1234_5678_9ABC_DEF1;

/// The per-lane constant multiplier: lane `k` XORs `k · LANE_MIX` into its rotated state.
pub const LANE_MIX: u64 = 0x9E37_79B9_7F4A_7C15;

/// One step of the stimulus generator: xorshift64 with shifts 13, 7, 17.
pub fn next_state(s: u64) -> u64 {
    let mut x = s;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

/// The rotation lane `k` applies to the state.
fn lane_rotation(k: usize) -> u32 {
    ((11 * k) % 64) as u32
}

/// The stimulus state `n` steps after `s`.
pub fn advance(s: u64, n: u32) -> u64 {
    (0..n).fold(s, |s, _| next_state(s))
}

/// The 64×64 GF(2) matrix of `n` stimulus steps as rows: bit `i` of [`advance`]`(s, n)` is the parity
/// of `s & rows[i]` (xorshift is linear, so `n` steps are one matrix).
pub fn jump_rows(n: u32) -> [u64; 64] {
    let cols: Vec<u64> = (0..64).map(|k| advance(1u64 << k, n)).collect();
    let mut rows = [0u64; 64];
    for (i, row) in rows.iter_mut().enumerate() {
        for (k, col) in cols.iter().enumerate() {
            if (col >> i) & 1 == 1 {
                *row |= 1u64 << k;
            }
        }
    }
    rows
}

/// A contract reference `port` or `port@j`: the port and the batch slot (operation `j` of the batch
/// a batched candidate takes per call).
pub fn contract_ref(r: &str) -> (&str, u32) {
    match r.split_once('@') {
        Some((p, j)) => (p, j.parse().unwrap_or(u32::MAX)),
        None => (r, 0),
    }
}

/// The 64-bit word input lane `k` reads from state `s`.
pub fn lane_word(s: u64, k: usize) -> u64 {
    s.rotate_left(lane_rotation(k)) ^ (k as u64).wrapping_mul(LANE_MIX)
}

/// Which values a field draws: uniformly over its domain, or only its two ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Every value of the domain (the slice of the lane word, offset by `base`).
    Shaped,
    /// Only the domain's minimum and maximum, chosen per sub-lane by one bit of the lane word.
    Edge,
}

/// How one 64-bit chunk of an input port is drawn from its lane word: `lanes` packed sub-lanes of
/// `width` bits each. A single-lane field is `base + slice` (sign-extended when `signed`) as a
/// 64-bit value; a packed field (`lanes > 1`) carries no offset and packs each sub-lane's raw slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    pub base: i64,
    pub width: u32,
    pub signed: bool,
    pub lanes: u32,
}

impl Field {
    pub const fn unsigned(base: i64, width: u32) -> Field {
        Field { base, width, signed: false, lanes: 1 }
    }

    pub const fn signed(width: u32) -> Field {
        Field { base: 0, width, signed: true, lanes: 1 }
    }

    pub const fn full() -> Field {
        Field { base: 0, width: 64, signed: false, lanes: 1 }
    }

    pub const fn packed(width: u32, lanes: u32, signed: bool) -> Field {
        Field { base: 0, width, signed, lanes }
    }

    fn slice_bounds(&self) -> (i128, i128) {
        let w = self.width;
        if self.signed {
            (-(1i128 << (w - 1)), (1i128 << (w - 1)) - 1)
        } else {
            (0, (1i128 << w) - 1)
        }
    }

    /// The smallest and largest value one sub-lane takes.
    pub fn range(&self) -> (i128, i128) {
        let (lo, hi) = self.slice_bounds();
        (self.base as i128 + lo, self.base as i128 + hi)
    }

    fn mask(&self) -> u64 {
        if self.width >= 64 {
            u64::MAX
        } else {
            (1u64 << self.width) - 1
        }
    }

    fn slice(&self, word: u64, j: u32, mode: Mode) -> i128 {
        let (lo, hi) = self.slice_bounds();
        match mode {
            Mode::Edge => {
                if (word >> j) & 1 == 1 {
                    hi
                } else {
                    lo
                }
            }
            Mode::Shaped => {
                let raw = if self.width >= 64 {
                    word
                } else {
                    (word >> (j * self.width)) & self.mask()
                };
                if self.signed && self.width < 64 && (raw >> (self.width - 1)) & 1 == 1 {
                    raw as i128 - (1i128 << self.width)
                } else if self.signed && self.width == 64 {
                    raw as i64 as i128
                } else {
                    raw as i128
                }
            }
        }
    }

    /// The 64-bit chunk this field draws from `word`.
    pub fn value(&self, word: u64, mode: Mode) -> u64 {
        if self.lanes == 1 {
            return (self.base as i128 + self.slice(word, 0, mode)) as u64;
        }
        (0..self.lanes).fold(0u64, |acc, j| {
            acc | (((self.slice(word, j, mode) as u64) & self.mask()) << (j * self.width))
        })
    }
}

/// An input port of a kernel contract: `bits` wide, one [`Field`] per 64-bit chunk (least
/// significant first). A port with `hold > 1` (a power of two) keeps the value drawn at its first
/// operation for `hold` operations — a weight tile reused across rows.
#[derive(Clone, Debug, PartialEq)]
pub struct InPort {
    pub name: &'static str,
    pub bits: u32,
    pub fields: Vec<Field>,
    pub hold: u32,
}

/// An output port of a kernel contract.
#[derive(Clone, Debug, PartialEq)]
pub struct OutPort {
    pub name: &'static str,
    pub bits: u32,
}

/// Where a kernel's LOGOS candidate comes from: compiled from a LOGOS program by the compiler, or
/// written by one of the RTL generators in this module tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    CompiledFromLg,
    RtlGenerator,
}

impl SourceKind {
    pub fn label(&self) -> &'static str {
        match self {
            SourceKind::CompiledFromLg => "compiled-from-lg",
            SourceKind::RtlGenerator => "rtl-generator",
        }
    }
}

/// One operation's values: per port, per 64-bit chunk.
pub type Words = Vec<Vec<u64>>;

/// The reference a kernel's results are checked against.
#[derive(Clone, Copy)]
pub enum Golden {
    /// The LOGOS function `func` declared by `program()`, run on the VM: every input port is one
    /// `Int` argument and the single output port is its result.
    Vm { program: fn() -> String, func: &'static str },
    /// The VM computes each lane of a SIMT step with `func`; the engine's mask and cross-lane sum
    /// are composed over those lane results.
    VmLanes { program: fn() -> String, func: &'static str, lanes: usize },
    /// The Rust oracle the kernel's RTL generator is locked against.
    Oracle(fn(&Words) -> Words),
}

impl std::fmt::Debug for Golden {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Golden::Vm { func, .. } => write!(f, "Vm({func})"),
            Golden::VmLanes { func, lanes, .. } => write!(f, "VmLanes({func} × {lanes})"),
            Golden::Oracle(_) => write!(f, "Oracle"),
        }
    }
}

/// A kernel of the bench: its contract and its golden.
#[derive(Clone, Debug)]
pub struct Kernel {
    pub id: &'static str,
    pub name: &'static str,
    pub summary: &'static str,
    pub source_kind: SourceKind,
    pub inputs: Vec<InPort>,
    pub outputs: Vec<OutPort>,
    pub golden: Golden,
}

fn port(name: &'static str, bits: u32, field: Field) -> InPort {
    InPort { name, bits, fields: vec![field], hold: 1 }
}

fn out(name: &'static str, bits: u32) -> OutPort {
    OutPort { name, bits }
}

fn decide_program() -> String {
    DECIDE_FN.to_string()
}

fn horner_program() -> String {
    format!("## To poly (x: Int) -> Int:\n    Return {POLY_BODY}.\n")
}

fn shade_program() -> String {
    "## To shade (a: Int, x: Int, b: Int) -> Int:\n    Return a * x + b.\n".to_string()
}

fn keccak_oracle(op: &Words) -> Words {
    let mut st = [0u64; 25];
    st.copy_from_slice(&op[0][..25]);
    logicaffeine_system::keccak::keccak_f1600(&mut st);
    vec![st.to_vec()]
}

fn butterfly_oracle(op: &Words) -> Words {
    let (a, b, zeta) = (op[0][0] as i16, op[1][0] as i16, op[2][0] as i16);
    let t = fqmul(zeta, b);
    vec![
        vec![a.wrapping_add(t) as u16 as u64],
        vec![a.wrapping_sub(t) as u16 as u64],
    ]
}

/// GEMM tile rows and columns, and the operand width.
pub const GEMM_DIM: usize = 8;
pub const GEMM_BITS: u32 = 8;
/// The accumulator width every GEMM candidate's output carries.
pub const GEMM_ACC_BITS: u32 = 32;
/// The narrower accumulator Gemmini's default configuration gives its mesh (`spatialArrayOutputType`
/// SInt(20)): wide enough for every sum of eight int8 products, so the matched rows compute the same
/// results with less state.
pub const GEMM_MATCHED_ACC_BITS: u32 = 20;

fn acc_name(base: &str, acc_bits: u32) -> String {
    if acc_bits == GEMM_ACC_BITS {
        base.to_string()
    } else {
        format!("{base}_acc{acc_bits}")
    }
}

fn gemm_oracle(op: &Words) -> Words {
    let byte = |w: &[u64], i: usize| ((w[i / 8] >> (8 * (i % 8))) & 0xff) as u8 as i8 as i64;
    let mut o = vec![0u64; GEMM_DIM * GEMM_ACC_BITS as usize / 64];
    for c in 0..GEMM_DIM {
        let acc: i64 = (0..GEMM_DIM)
            .map(|r| byte(&op[0], r) * byte(&op[1], r * GEMM_DIM + c))
            .sum();
        o[c / 2] |= (acc as i32 as u32 as u64) << (32 * (c % 2));
    }
    vec![o]
}

/// SIMT lanes of the K6 engine.
pub const SIMT_LANES: usize = 4;

/// The kernels of the bench, in id order.
pub fn kernels() -> Vec<Kernel> {
    vec![
        Kernel {
            id: "K1",
            name: "decide",
            summary: "HFT decision: microprice market-making, fat-finger clamp, position risk gate; 8 Int in, 1 Int out",
            source_kind: SourceKind::CompiledFromLg,
            inputs: vec![
                port("micro100", 64, Field::unsigned(10_000, 7)),
                port("fair100", 64, Field::unsigned(10_000, 7)),
                port("edge100", 64, Field::unsigned(0, 4)),
                port("base_size", 64, Field::unsigned(1, 6)),
                port("max_order", 64, Field::unsigned(10, 5)),
                port("inventory", 64, Field::signed(8)),
                port("pos_cap", 64, Field::unsigned(64, 7)),
                port("book_ok", 64, Field::unsigned(0, 1)),
            ],
            outputs: vec![out("order", 64)],
            golden: Golden::Vm { program: decide_program, func: DECIDE_NAME },
        },
        Kernel {
            id: "K2",
            name: "horner",
            summary: "Degree-3 polynomial in Horner form, ((2x+5)x-3)x+100, over Int",
            source_kind: SourceKind::CompiledFromLg,
            inputs: vec![port("x", 64, Field::signed(21))],
            outputs: vec![out("y", 64)],
            golden: Golden::Vm { program: horner_program, func: "poly" },
        },
        Kernel {
            id: "K3",
            name: "gemm8",
            summary: "8x8 int8 weight-stationary GEMM tile, int32 accumulation, weights reloaded every 8 rows",
            source_kind: SourceKind::RtlGenerator,
            inputs: vec![
                port("a", 64, Field::packed(8, 8, true)),
                InPort {
                    name: "w",
                    bits: (GEMM_DIM * GEMM_DIM) as u32 * GEMM_BITS,
                    fields: vec![Field::packed(8, 8, true); GEMM_DIM],
                    hold: GEMM_DIM as u32,
                },
            ],
            outputs: vec![out("o", GEMM_DIM as u32 * GEMM_ACC_BITS)],
            golden: Golden::Oracle(gemm_oracle),
        },
        Kernel {
            id: "K4",
            name: "keccak_f1600",
            summary: "Keccak-f[1600] permutation (FIPS 202), 1600-bit state in and out",
            source_kind: SourceKind::RtlGenerator,
            inputs: vec![InPort {
                name: "s_in",
                bits: 1600,
                fields: vec![Field::full(); 25],
                hold: 1,
            }],
            outputs: vec![out("s_out", 1600)],
            golden: Golden::Oracle(keccak_oracle),
        },
        Kernel {
            id: "K5",
            name: "ntt_bfly",
            summary: "ML-KEM NTT butterfly: Montgomery fqmul (q = 3329) and Cooley-Tukey add/sub, 16-bit",
            source_kind: SourceKind::RtlGenerator,
            inputs: vec![
                port("a", 16, Field::signed(16)),
                port("b", 16, Field::signed(16)),
                port("zeta", 16, Field::signed(16)),
            ],
            outputs: vec![out("hi", 16), out("lo", 16)],
            golden: Golden::Oracle(butterfly_oracle),
        },
        Kernel {
            id: "K6",
            name: "simt_axpb_sum",
            summary: "4-lane SIMT step: masked a*x+b per lane (uniform a, b; varying x) and the cross-lane sum",
            source_kind: SourceKind::CompiledFromLg,
            inputs: vec![
                port("a", 64, Field::signed(31)),
                port("b", 64, Field::signed(31)),
                port("mask", SIMT_LANES as u32, Field::unsigned(0, SIMT_LANES as u32)),
                port("x0", 64, Field::signed(31)),
                port("x1", 64, Field::signed(31)),
                port("x2", 64, Field::signed(31)),
                port("x3", 64, Field::signed(31)),
            ],
            outputs: vec![out("y0", 64), out("y1", 64), out("y2", 64), out("y3", 64), out("sum", 64)],
            golden: Golden::VmLanes { program: shade_program, func: "shade", lanes: SIMT_LANES },
        },
    ]
}

/// The kernel with id `id`.
pub fn kernel(id: &str) -> Option<Kernel> {
    kernels().into_iter().find(|k| k.id == id)
}

fn chunk_bits(bits: u32, c: usize) -> u32 {
    bits.saturating_sub(64 * c as u32).min(64)
}

fn mask_bits(v: u64, bits: u32) -> u64 {
    if bits >= 64 {
        v
    } else {
        v & ((1u64 << bits) - 1)
    }
}

/// The operand values of the first `ops` operations of `k` in `mode`.
pub fn stimulus(k: &Kernel, mode: Mode, ops: usize) -> Vec<Words> {
    let mut s = SEED;
    let mut held: Vec<Vec<u64>> = vec![Vec::new(); k.inputs.len()];
    let mut all = Vec::with_capacity(ops);
    for op in 0..ops {
        let mut lane = 0usize;
        let mut words = Vec::with_capacity(k.inputs.len());
        for (p, port) in k.inputs.iter().enumerate() {
            let fresh: Vec<u64> = port
                .fields
                .iter()
                .enumerate()
                .map(|(c, f)| mask_bits(f.value(lane_word(s, lane + c), mode), chunk_bits(port.bits, c)))
                .collect();
            lane += port.fields.len();
            if port.hold > 1 {
                if op % port.hold as usize == 0 {
                    held[p] = fresh;
                }
                words.push(held[p].clone());
            } else {
                words.push(fresh);
            }
        }
        all.push(words);
        s = next_state(s);
    }
    all
}

/// Why a harness, golden, or run was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum QorError {
    /// The binding names a DUT port the module does not have.
    PortMissing { candidate: String, port: String },
    /// A DUT port is narrower than the contract bits bound to it.
    PortWidth { candidate: String, port: String, need: u32, have: u32 },
    /// A DUT port runs the other way from its binding.
    PortDirection { candidate: String, port: String },
    /// Contract bits no binding drives or reads.
    Unmapped { candidate: String, contract: String },
    /// The protocol's control ports do not fit the DUT.
    Protocol(String),
    /// The golden could not be computed.
    Golden(String),
    /// A simulated signature went unknown (X/Z) at this operation.
    Unknown { op: usize },
    /// The testbench output carries no verdict.
    Testbench(String),
    /// The candidate cannot be built.
    Build(String),
}

impl std::fmt::Display for QorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QorError::PortMissing { candidate, port } => {
                write!(f, "{candidate}: the DUT has no port `{port}`")
            }
            QorError::PortWidth { candidate, port, need, have } => {
                write!(f, "{candidate}: port `{port}` is {have} bits, the binding needs {need}")
            }
            QorError::PortDirection { candidate, port } => {
                write!(f, "{candidate}: port `{port}` runs the wrong way for its binding")
            }
            QorError::Unmapped { candidate, contract } => {
                write!(f, "{candidate}: contract port `{contract}` is not fully bound")
            }
            QorError::Protocol(m) | QorError::Golden(m) | QorError::Testbench(m) | QorError::Build(m) => {
                write!(f, "{m}")
            }
            QorError::Unknown { op } => write!(f, "the signature went unknown at operation {op}"),
        }
    }
}

impl std::error::Error for QorError {}

/// The VM's printed integer, reduced to its 64-bit two's-complement pattern.
fn wrap_decimal(s: &str) -> Option<u64> {
    let (neg, digits) = match s.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, s),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let v = digits
        .bytes()
        .fold(0u64, |acc, b| acc.wrapping_mul(10).wrapping_add((b - b'0') as u64));
    Some(if neg { v.wrapping_neg() } else { v })
}

/// A LOGOS integer literal argument; a negative one is written `(0 - n)`.
fn logos_arg(v: i64) -> String {
    if v < 0 {
        format!("(0 - {})", (v as i128).abs())
    } else {
        v.to_string()
    }
}

/// Run `func` of `program` on the VM over `calls` (each the argument list of one call), in
/// batches of one program per 512 calls.
fn vm_calls(program: &str, func: &str, calls: &[Vec<i64>]) -> Result<Vec<u64>, QorError> {
    let mut out = Vec::with_capacity(calls.len());
    for batch in calls.chunks(512) {
        let mut src = format!("{program}\n## Main\n");
        for args in batch {
            let list: Vec<String> = args.iter().map(|a| logos_arg(*a)).collect();
            src.push_str(&format!("    Show {func}({}).\n", list.join(", ")));
        }
        let run = crate::compile::vm_outcome(&src);
        if let Some(e) = run.error {
            return Err(QorError::Golden(format!("the VM refused `{func}`: {e}")));
        }
        let values: Vec<u64> = run
            .output
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|l| wrap_decimal(l).ok_or_else(|| QorError::Golden(format!("`{l}` is not an Int"))))
            .collect::<Result<_, _>>()?;
        if values.len() != batch.len() {
            return Err(QorError::Golden(format!(
                "the VM printed {} results for {} calls of `{func}`",
                values.len(),
                batch.len()
            )));
        }
        out.extend(values);
    }
    Ok(out)
}

/// The golden results of `k` over `stim`.
pub fn golden(k: &Kernel, stim: &[Words]) -> Result<Vec<Words>, QorError> {
    match k.golden {
        Golden::Oracle(f) => Ok(stim.iter().map(f).collect()),
        Golden::Vm { program, func } => {
            let calls: Vec<Vec<i64>> = stim
                .iter()
                .map(|op| op.iter().map(|p| p[0] as i64).collect())
                .collect();
            let results = vm_calls(&program(), func, &calls)?;
            Ok(results.into_iter().map(|r| vec![vec![r]]).collect())
        }
        Golden::VmLanes { program, func, lanes } => {
            let mut calls = Vec::with_capacity(stim.len() * lanes);
            for op in stim {
                let (a, b) = (op[0][0] as i64, op[1][0] as i64);
                for lane in 0..lanes {
                    calls.push(vec![a, op[3 + lane][0] as i64, b]);
                }
            }
            let results = vm_calls(&program(), func, &calls)?;
            Ok(stim
                .iter()
                .zip(results.chunks(lanes))
                .map(|(op, lane_out)| {
                    let mask = op[2][0];
                    let mut words: Words = Vec::with_capacity(lanes + 1);
                    let mut sum = 0u64;
                    for (lane, y) in lane_out.iter().enumerate() {
                        let y = if (mask >> lane) & 1 == 1 { *y } else { 0 };
                        sum = sum.wrapping_add(y);
                        words.push(vec![y]);
                    }
                    words.push(vec![sum]);
                    words
                })
                .collect())
        }
    }
}

/// One result folded to 64 bits: chunk `c` (counted across ports in order) rotated left by `c`.
pub fn fold(k: &Kernel, result: &Words) -> u64 {
    let mut c = 0usize;
    let mut acc = 0u64;
    for (p, port) in k.outputs.iter().enumerate() {
        for (i, w) in result[p].iter().enumerate() {
            acc ^= mask_bits(*w, chunk_bits(port.bits, i)).rotate_left((c % 64) as u32);
            c += 1;
        }
    }
    acc
}

/// The MISR update: shift left with the parity of the old state fed back, XOR the folded result.
pub fn misr_step(misr: u64, folded: u64) -> u64 {
    ((misr << 1) | (misr.count_ones() as u64 & 1)) ^ folded
}

/// The signature of a result sequence.
pub fn signature(k: &Kernel, results: &[Words]) -> u64 {
    results.iter().fold(0u64, |m, r| misr_step(m, fold(k, r)))
}

/// A DUT port's direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Dir {
    In,
    Out,
}

/// A port of the DUT's top module, as its netlist declares it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DutPort {
    pub name: String,
    pub dir: Dir,
    pub bits: u32,
}

/// A DUT reset input.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Reset {
    pub port: String,
    pub active_high: bool,
}

/// One cycle of a register-file transfer: contract bits `[lsb, lsb + width)` of port `contract`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RegSlot {
    pub contract: String,
    pub lsb: u32,
    pub width: u32,
}

/// What a beat schedule drives onto one DUT input port in one beat.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum DriveValue {
    Const(u64),
    /// Contract bits `[lsb, lsb + width)` of input reference `contract` (`port` or `port@j`).
    Slice { contract: String, lsb: u32, width: u32 },
    /// The same bits of the previous call's operands (zero before the first call): a design that
    /// overlaps calls — loading one call's weights while it computes on the call before — takes part
    /// of a call's operands a call later than the rest.
    Previous { contract: String, lsb: u32, width: u32 },
}

/// DUT input `dut` takes `value` in beat `beat` of every call; a port is 0 in the beats that do not
/// drive it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Drive {
    pub beat: u32,
    pub dut: String,
    pub value: DriveValue,
}

/// A device's configuration-register port: `valid` with `rw` high writes `data` to register `addr`;
/// with `rw` low it reads, and the device answers on `rsp_valid`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DcrBus {
    pub valid: String,
    pub rw: String,
    pub addr: String,
    pub data: String,
    pub rsp_valid: String,
}

/// One configuration-register write.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DcrWrite {
    pub addr: u32,
    pub data: u64,
}

/// One constant line of a device's memory (a program image): its line address and its bytes in hex,
/// the byte at the highest address first.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ImageLine {
    pub line: u64,
    pub hex: String,
}

/// Lines of a device's memory holding contract ports from line `base_line` on: the listed ports in
/// order, each as `words_per_port` words, packed from the region's first byte. An input region answers
/// reads with the call's operands; an output region keeps the device's writes.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LineRegion {
    pub base_line: u64,
    pub contract: Vec<String>,
    pub words_per_port: u32,
    /// The bits each port occupies, as a structure of machine words lays them out (0: its own
    /// width): an operand narrower than its stride is extended the way a bind extends one (by sign
    /// for a signed field), a result is read from the low bits of its stride.
    #[serde(default)]
    pub stride_bits: u32,
}

impl LineRegion {
    /// The bits port `bits` wide occupies in this region.
    fn span(&self, bits: u32) -> u32 {
        bits.max(self.stride_bits)
    }
}

/// The memory a device reads and writes whole lines of, served by the harness over `ports` channels:
/// each a request (`<prefix>_req_valid`, `_rw`, `_byteen`, `_addr` — a line address — `_data`, `_tag`,
/// `_ready`) and a read response (`<prefix>_rsp_valid`, `_data`, `_tag`, `_ready`); a write has no
/// response. Each signal carries every port's value side by side, port 0 in the most significant
/// bits when `msb_first`. The harness accepts every request and answers each read in order on its
/// port from the next cycle on, with the line as it is when answered: an ideal memory, never a
/// candidate's bottleneck.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LineBus {
    pub ports: u32,
    pub line_bytes: u32,
    pub addr_bits: u32,
    pub tag_bits: u32,
    pub msb_first: bool,
    pub prefix: String,
    pub image: Vec<ImageLine>,
    pub regions: Vec<LineRegion>,
}

impl LineBus {
    fn line_bits(&self) -> u32 {
        self.line_bytes * 8
    }

    fn port(&self, suffix: &str) -> String {
        format!("{}_{suffix}", self.prefix)
    }

    /// Port `p`'s field of a signal carrying `width` bits per port.
    fn slot(&self, signal: &str, p: u32, width: u32) -> String {
        let at = if self.msb_first { self.ports - 1 - p } else { p };
        format!("{signal}[{}:{}]", at * width + width - 1, at * width)
    }

    /// Every bus signal, its direction from the device, and its width.
    fn signals(&self) -> Vec<(String, Dir, u32)> {
        let (n, lb) = (self.ports, self.line_bits());
        [
            ("req_valid", Dir::Out, n),
            ("req_rw", Dir::Out, n),
            ("req_byteen", Dir::Out, n * self.line_bytes),
            ("req_addr", Dir::Out, n * self.addr_bits),
            ("req_data", Dir::Out, n * lb),
            ("req_tag", Dir::Out, n * self.tag_bits),
            ("req_ready", Dir::In, n),
            ("rsp_valid", Dir::In, n),
            ("rsp_data", Dir::In, n * lb),
            ("rsp_tag", Dir::In, n * self.tag_bits),
            ("rsp_ready", Dir::Out, n),
        ]
        .into_iter()
        .map(|(s, d, w)| (self.port(s), d, w))
        .collect()
    }
}

/// How the harness hands operands to a candidate and collects its results.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Protocol {
    /// A new operation every cycle; its result is valid `latency` cycles later (0: combinational).
    Registered { latency: u32 },
    /// Pulse `start` with the operands; hold them until `done`, which marks the result valid.
    /// One idle cycle separates operations.
    StartDone { start: String, done: String },
    /// Pulse `start` every `ii` cycles with a new operation; each `done` marks the next result.
    Pipelined { start: String, done: String, ii: u32 },
    /// Hold `go` high; each one-cycle `done` marks a result. A component restarts in the very cycle it
    /// raises `done`, so from that cycle on the next operation's operands are presented (a read issued
    /// then answers with them).
    GoDone { go: String, done: String },
    /// Hold `enable` high with the operands until `done`, which stays up once raised; then lower `enable` and
    /// raise `clear` for one cycle, which takes `done` down, before the next operation — how Spatial's Fringe
    /// drives an accelerator (its command and status registers, the accelerator's own reset between runs).
    EnableDone { enable: String, done: String, clear: String },
    /// A register-file engine: one `load` cycle per load slot puts the slot on `din`, a `go` pulse
    /// starts it, `done` marks the results, and one cycle per read slot puts the slot's index on
    /// `raddr` and takes it from `dout`. With `reset_each_op` the DUT's reset is pulsed before each
    /// operation's loads (an engine whose load counter counts from reset).
    LoadGoRead {
        load: String,
        din: String,
        go: String,
        done: String,
        raddr: String,
        dout: String,
        reset_each_op: bool,
        load_slots: Vec<RegSlot>,
        read_slots: Vec<RegSlot>,
    },
    /// A fixed beat schedule: every `period` cycles a call begins; beat `t` of a call drives the
    /// `drives` of beat `t`, and the result of the call's operation `j` is on the output binds of
    /// slot `j` exactly `captures[j]` cycles after its beat 0. A call takes the candidate's whole batch.
    Beats { period: u32, drives: Vec<Drive>, captures: Vec<u32> },
    /// A device launched like a GPU kernel: after reset the `setup` writes go out on its DCR port, one
    /// a cycle; then each call raises `start` for a cycle, waits for `busy` to rise and to fall, issues
    /// the `flush` DCR read (if any) and waits for its answer, and takes the call's results from the
    /// output regions of `bus`, which the device reads its operands from and writes its results to.
    Launch { start: String, busy: String, dcr: DcrBus, setup: Vec<DcrWrite>, flush: Option<u32>, bus: LineBus },
}

impl Protocol {
    pub fn label(&self) -> String {
        match self {
            Protocol::Registered { latency } => format!("registered, latency {latency}"),
            Protocol::StartDone { .. } => "start/done".to_string(),
            Protocol::Pipelined { ii, .. } => format!("pipelined start/done, II {ii}"),
            Protocol::GoDone { .. } => "go/done".to_string(),
            Protocol::EnableDone { .. } => "enable held until a sticky done, then cleared".to_string(),
            Protocol::Beats { period, captures, .. } => {
                format!("beat schedule: a call every {period} cycles, {} results", captures.len())
            }
            Protocol::LoadGoRead { load_slots, read_slots, reset_each_op, .. } => format!(
                "register file: {} loads, go/done, {} reads{}",
                load_slots.len(),
                read_slots.len(),
                if *reset_each_op { ", reset before each operation" } else { "" }
            ),
            Protocol::Launch { bus, flush, .. } => format!(
                "kernel launch over a {}-port {}-byte-line memory bus{}",
                bus.ports,
                bus.line_bytes,
                if flush.is_some() { ", flushed before the results are read" } else { "" }
            ),
        }
    }
}

/// The memory regions of a launched device's bus (none for any other protocol).
fn launch_regions(c: &Candidate) -> &[LineRegion] {
    match &c.protocol {
        Protocol::Launch { bus, .. } => &bus.regions,
        _ => &[],
    }
}

/// Contract bits `[lsb, lsb + width)` of port `contract` connect to DUT port `dut`. Several binds
/// to one input DUT port concatenate, the first in the least significant bits; an input DUT port
/// wider than what is bound to it is sign-extended when the topmost bound contract field is signed,
/// else zero-extended. An output DUT port contributes its low `width` bits.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Bind {
    pub contract: String,
    pub lsb: u32,
    pub width: u32,
    pub dut: String,
    /// For an output: the DUT port bit the contract bits start at.
    #[serde(default)]
    pub dut_lsb: u32,
    /// For an output: the DUT port whose one-cycle pulse marks a new value on `dut`. Values arrive in
    /// operation order; the harness queues each port and folds a result once every port has one.
    #[serde(default)]
    pub valid: Option<String>,
    /// For an output the DUT carries narrower than the contract field: the DUT supplies this many
    /// bits from `dut_lsb`, a signed value sign-extended to the bind's width (0: the whole width).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub dut_width: u32,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

impl Bind {
    /// The whole contract port `contract` to DUT port `dut`; the width is the contract's.
    pub fn whole(contract: &str, dut: &str) -> Bind {
        Bind { contract: contract.to_string(), lsb: 0, width: 0, dut: dut.to_string(), dut_lsb: 0, valid: None, dut_width: 0 }
    }

    pub fn slice(contract: &str, lsb: u32, width: u32, dut: &str) -> Bind {
        Bind { contract: contract.to_string(), lsb, width, dut: dut.to_string(), dut_lsb: 0, valid: None, dut_width: 0 }
    }

    /// This output bind taking only `dut_width` signed bits from the DUT, sign-extended to its width.
    pub fn sign_extended(mut self, dut_width: u32) -> Bind {
        self.dut_width = dut_width;
        self
    }

    /// The DUT port bits this bind reads or drives.
    fn dut_bits(&self, contract_bits: u32) -> u32 {
        if self.dut_width == 0 {
            self.resolved_width(contract_bits)
        } else {
            self.dut_width
        }
    }

    /// This output bind with its values marked by DUT pulse `valid`.
    pub fn valid_on(mut self, valid: &str) -> Bind {
        self.valid = Some(valid.to_string());
        self
    }

    /// The whole output contract port `contract` from bits `[dut_lsb, dut_lsb + width)` of DUT port
    /// `dut`.
    pub fn from_dut(contract: &str, dut: &str, dut_lsb: u32, width: u32) -> Bind {
        Bind { contract: contract.to_string(), lsb: 0, width, dut: dut.to_string(), dut_lsb, valid: None, dut_width: 0 }
    }

    fn resolved_width(&self, contract_bits: u32) -> u32 {
        if self.width == 0 {
            contract_bits - self.lsb
        } else {
            self.width
        }
    }
}

/// The memory protocol a [`MemBind`] speaks.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum MemKind {
    /// Calyx's `seq_mem`: `<p>_addr0`, `_content_en`, `_write_en`, `_write_data`, `_read_data`, `_done`.
    /// A read or a write issued with `content_en` completes one cycle later, when `done` rises.
    SeqMem,
    /// A block-RAM interface with `ports` ports `i`: `<p>_address<i>`, `_ce<i>`, and `_q<i>` for a
    /// read (one cycle later) or `_we<i>` with `_d<i>` for a write.
    Bram { ports: u32 },
}

/// Contract ports carried through a memory interface instead of scalar ports. The memory's words
/// are the listed contract ports in order, each split into `words_per_port` words (least
/// significant first), so one wide port can become an array and several ports one array.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MemBind {
    pub prefix: String,
    pub kind: MemKind,
    pub contract: Vec<String>,
    pub words_per_port: u32,
    /// The memory's shape when its interface takes one address per dimension, as Calyx gives a
    /// Dahlia `decl x: bit<N>[R][C]` (`addr0` the row, `addr1` the column): the words row-major.
    /// Empty: one flat address.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dims: Vec<u32>,
}

/// The address bits that index `n` words.
fn index_bits(n: u32) -> u32 {
    (32 - (n.max(2) - 1).leading_zeros()).max(1)
}

impl MemBind {
    /// A Calyx `seq_mem` holding contract port `contract` as `words` words.
    pub fn seq_mem(contract: &str, prefix: &str, words: u32) -> MemBind {
        MemBind::seq_mem_rows(&[contract.to_string()], prefix, words)
    }

    /// A Calyx `seq_mem` whose words are the contract ports `contract` in order, each as
    /// `words_per_port` words.
    pub fn seq_mem_rows(contract: &[String], prefix: &str, words_per_port: u32) -> MemBind {
        MemBind {
            prefix: prefix.to_string(),
            kind: MemKind::SeqMem,
            contract: contract.to_vec(),
            words_per_port,
            dims: Vec::new(),
        }
    }

    /// This memory addressed by one port per dimension of `dims`, row-major.
    pub fn shaped(mut self, dims: &[u32]) -> MemBind {
        self.dims = dims.to_vec();
        self
    }

    /// A `ports`-port block RAM whose words are the contract ports `contract`, one each.
    pub fn bram(contract: &[&str], prefix: &str, ports: u32) -> MemBind {
        MemBind {
            prefix: prefix.to_string(),
            kind: MemKind::Bram { ports },
            contract: contract.iter().map(|c| c.to_string()).collect(),
            words_per_port: 1,
            dims: Vec::new(),
        }
    }

    fn words(&self) -> u32 {
        self.contract.len() as u32 * self.words_per_port
    }

    fn addr_bits(&self) -> u32 {
        index_bits(self.words())
    }

    /// The address ports and the width each needs: `addr0` for a flat memory, `addr<k>` for
    /// dimension `k` of a shaped one.
    fn address_ports(&self) -> Vec<(String, u32)> {
        if self.dims.is_empty() {
            vec![(self.port("addr0"), self.addr_bits())]
        } else {
            self.dims.iter().enumerate().map(|(k, d)| (self.port(&format!("addr{k}")), index_bits(*d))).collect()
        }
    }

    /// The flat word index the address wires `q_<prefix>_addr<k>` select, row-major.
    fn word_index(&self) -> String {
        if self.dims.is_empty() {
            return format!("q_{}_addr0", self.prefix);
        }
        let mut e = format!("q_{}_addr0", self.prefix);
        for (k, d) in self.dims.iter().enumerate().skip(1) {
            e = format!("({e} * {d} + q_{}_addr{k})", self.prefix);
        }
        e
    }

    fn port(&self, suffix: &str) -> String {
        format!("{}_{suffix}", self.prefix)
    }
}

/// One implementation of a kernel, as the harness drives it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Candidate {
    pub kernel: String,
    pub name: String,
    pub top: String,
    pub clock: String,
    pub reset: Option<Reset>,
    pub protocol: Protocol,
    pub inputs: Vec<Bind>,
    pub outputs: Vec<Bind>,
    /// Contract ports carried through memory interfaces instead of scalar ports.
    #[serde(default)]
    pub mems: Vec<MemBind>,
    /// Contract operations per call: binds name slot `j` as `port@j`.
    #[serde(default = "one")]
    pub batch: u32,
    /// The DUT top module's ports, as its netlist declares them.
    pub ports: Vec<DutPort>,
}

fn one() -> u32 {
    1
}

/// The harness module's name for `c`.
pub fn harness_module(c: &Candidate) -> String {
    format!("qorbist_{}", c.name)
}

fn find<'a>(c: &'a Candidate, name: &str) -> Result<&'a DutPort, QorError> {
    c.ports.iter().find(|p| p.name == name).ok_or_else(|| QorError::PortMissing {
        candidate: c.name.clone(),
        port: name.to_string(),
    })
}

fn expect_dir(c: &Candidate, p: &DutPort, dir: Dir) -> Result<(), QorError> {
    if p.dir == dir {
        Ok(())
    } else {
        Err(QorError::PortDirection { candidate: c.name.clone(), port: p.name.clone() })
    }
}

/// The canonical `port@slot` key of a contract reference.
fn slot_key(r: &str) -> String {
    let (p, j) = contract_ref(r);
    format!("{p}@{j}")
}

/// Every `port@slot` key of `ports` over a batch of `batch`, with the port's width.
fn slot_keys(ports: &[(&'static str, u32)], batch: u32) -> Vec<(String, u32)> {
    (0..batch)
        .flat_map(|j| ports.iter().map(move |(n, b)| (format!("{n}@{j}"), *b)))
        .collect()
}

/// Every contract bit of `keys` is bound exactly once — by a bind, a memory, a register-file slot, or
/// a beat — and every bind names a contract port of the batch.
fn check_binds(
    c: &Candidate,
    binds: &[Bind],
    slots: &[RegSlot],
    beats: &[(String, u32, u32)],
    keys: &[(String, u32)],
    dir: Dir,
) -> Result<(), QorError> {
    for (key, bits) in keys {
        let mut covered = vec![false; *bits as usize];
        let mut cover = |lo: u32, hi: u32, twice_ok: bool| -> Result<(), QorError> {
            for bit in lo..hi.min(*bits) {
                if covered[bit as usize] && !twice_ok {
                    return Err(QorError::Unmapped {
                        candidate: c.name.clone(),
                        contract: format!("{key} (bit {bit} bound twice)"),
                    });
                }
                covered[bit as usize] = true;
            }
            Ok(())
        };
        for b in binds.iter().filter(|b| slot_key(&b.contract) == *key) {
            let w = b.resolved_width(*bits);
            let p = find(c, &b.dut)?;
            expect_dir(c, p, dir)?;
            if b.dut_width > 0 && (dir == Dir::In || b.dut_width > w) {
                return Err(QorError::Protocol(format!(
                    "{}: `{}` takes {} bits of {} for a {w}-bit field; only an output may be narrower in the DUT",
                    c.name, b.contract, b.dut_width, b.dut
                )));
            }
            let need = b.dut_lsb + b.dut_bits(*bits);
            if p.bits < need {
                return Err(QorError::PortWidth { candidate: c.name.clone(), port: p.name.clone(), need, have: p.bits });
            }
            cover(b.lsb, b.lsb + w, false)?;
        }
        for m in c.mems.iter().filter(|m| m.contract.iter().any(|n| slot_key(n) == *key)) {
            check_mem(c, m, *bits, dir)?;
            cover(0, *bits, false)?;
        }
        for r in launch_regions(c) {
            for _ in r.contract.iter().filter(|n| slot_key(n) == *key) {
                if r.words_per_port == 0 || bits % r.words_per_port != 0 {
                    return Err(QorError::Protocol(format!(
                        "{}: {} words do not divide the {bits}-bit port {key} in the region at line {:#x}",
                        c.name, r.words_per_port, r.base_line
                    )));
                }
                cover(0, *bits, false)?;
            }
        }
        for s in slots.iter().filter(|s| slot_key(&s.contract) == *key) {
            cover(s.lsb, s.lsb + s.width, false)?;
        }
        for (_, lsb, width) in beats.iter().filter(|(r, _, _)| slot_key(r) == *key) {
            cover(*lsb, lsb + width, true)?;
        }
        if covered.iter().any(|b| !b) {
            return Err(QorError::Unmapped { candidate: c.name.clone(), contract: key.clone() });
        }
    }
    let known = |r: &str| keys.iter().any(|(k, _)| *k == slot_key(r));
    for r in binds
        .iter()
        .map(|b| b.contract.as_str())
        .chain(slots.iter().map(|s| s.contract.as_str()))
        .chain(beats.iter().map(|(r, _, _)| r.as_str()))
    {
        if !known(r) {
            return Err(QorError::Unmapped { candidate: c.name.clone(), contract: format!("{r} (not a contract port)") });
        }
    }
    Ok(())
}

/// The memory interface of `m` fits the DUT: every signal present, running the right way, wide
/// enough for one word of the `bits`-wide contract port it carries in direction `dir`.
fn check_mem(c: &Candidate, m: &MemBind, bits: u32, dir: Dir) -> Result<(), QorError> {
    if m.words_per_port == 0 || bits % m.words_per_port != 0 {
        return Err(QorError::Protocol(format!(
            "{}: {} words do not divide the {bits}-bit port behind `{}`",
            c.name, m.words_per_port, m.prefix
        )));
    }
    if !m.dims.is_empty() {
        if matches!(m.kind, MemKind::Bram { .. }) {
            return Err(QorError::Protocol(format!(
                "{}: the block RAM `{}` has one flat address per port and takes no shape",
                c.name, m.prefix
            )));
        }
        let cells: u64 = m.dims.iter().map(|d| *d as u64).product();
        if cells != m.words() as u64 {
            return Err(QorError::Protocol(format!(
                "{}: the shape {:?} of `{}` holds {cells} words, its contract ports {}",
                c.name,
                m.dims,
                m.prefix,
                m.words()
            )));
        }
    }
    let word = bits / m.words_per_port;
    let mut need: Vec<(String, Dir, u32)> = Vec::new();
    match &m.kind {
        MemKind::SeqMem => {
            let data = if dir == Dir::In { "read_data" } else { "write_data" };
            need.extend(m.address_ports().into_iter().map(|(name, w)| (name, Dir::Out, w)));
            for (suffix, pdir, w) in [
                ("content_en", Dir::Out, 1),
                ("write_en", Dir::Out, 1),
                ("write_data", Dir::Out, 0),
                ("read_data", Dir::In, 0),
                ("done", Dir::In, 1),
            ] {
                need.push((m.port(suffix), pdir, if suffix == data { word } else { w }));
            }
        }
        MemKind::Bram { ports } => {
            for i in 0..*ports {
                need.push((m.port(&format!("address{i}")), Dir::Out, m.addr_bits()));
                need.push((m.port(&format!("ce{i}")), Dir::Out, 1));
                if dir == Dir::In {
                    need.push((m.port(&format!("q{i}")), Dir::In, word));
                } else {
                    need.push((m.port(&format!("we{i}")), Dir::Out, 1));
                    need.push((m.port(&format!("d{i}")), Dir::Out, word));
                }
            }
        }
    }
    for (name, pdir, w) in need {
        let p = find(c, &name)?;
        expect_dir(c, p, pdir)?;
        if p.bits < w {
            return Err(QorError::PortWidth { candidate: c.name.clone(), port: p.name.clone(), need: w, have: p.bits });
        }
    }
    Ok(())
}

/// The results each slot of a beat schedule must be able to hold. The harness folds one result per
/// cycle in operation order, so a slot whose result arrives while an earlier slot of its call is
/// still outstanding waits — and when the call after it delivers that slot again before the fold
/// reaches it (results handed back last first), the slot queues two. Found by running the schedule
/// itself: slot `j` of call `n` is captured at `n * period + captures[j]` and folded once every
/// earlier result has been. A schedule that brings more results per call than a call has cycles
/// outruns the fold and is refused.
pub fn beat_capture_depths(period: u32, captures: &[u32]) -> Result<Vec<u32>, String> {
    let nb = captures.len();
    if nb == 0 || period == 0 || nb as u32 > period {
        return Err(format!("{nb} results every {period} cycles outrun a fold of one result per cycle"));
    }
    let deepest = *captures.iter().max().unwrap();
    let calls = (deepest / period + 1) as usize * 4 + nb + 4;
    let end = calls as u32 * period + deepest + 2 * nb as u32 + 2;
    let mut count = vec![0u32; nb];
    let mut depth = vec![1u32; nb];
    let mut fp = 0usize;
    for t in 0..end {
        let pop = (count[fp] > 0).then_some(fp);
        for (j, off) in captures.iter().enumerate() {
            let push = t >= *off && (t - off) % period == 0 && ((t - off) / period) < calls as u32;
            let popped = pop == Some(j);
            count[j] = count[j] + push as u32 - popped as u32;
            depth[j] = depth[j].max(count[j]);
        }
        if pop.is_some() {
            fp = (fp + 1) % nb;
        }
    }
    Ok(depth)
}

/// Refuse a candidate whose binding, clock, reset or protocol ports do not fit its DUT's ports.
pub fn check_ports(k: &Kernel, c: &Candidate) -> Result<(), QorError> {
    if c.batch == 0 {
        return Err(QorError::Protocol(format!("{}: a batch of 0 operations", c.name)));
    }
    let ins: Vec<(&'static str, u32)> = k.inputs.iter().map(|p| (p.name, p.bits)).collect();
    let outs: Vec<(&'static str, u32)> = k.outputs.iter().map(|p| (p.name, p.bits)).collect();
    let (load_slots, read_slots): (&[RegSlot], &[RegSlot]) = match &c.protocol {
        Protocol::LoadGoRead { load_slots, read_slots, .. } => (load_slots, read_slots),
        _ => (&[], &[]),
    };
    let beat_slices: Vec<(String, u32, u32)> = match &c.protocol {
        Protocol::Beats { drives, .. } => drives
            .iter()
            .filter_map(|d| match &d.value {
                DriveValue::Slice { contract, lsb, width } | DriveValue::Previous { contract, lsb, width } => {
                    Some((contract.clone(), *lsb, *width))
                }
                DriveValue::Const(_) => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    // A held port is one register for the whole batch: only its slot 0 is bound.
    let in_keys: Vec<(String, u32)> = slot_keys(&ins, c.batch)
        .into_iter()
        .filter(|(key, _)| {
            let (p, j) = contract_ref(key);
            j == 0 || k.inputs.iter().any(|i| i.name == p && i.hold <= 1)
        })
        .collect();
    check_binds(c, &c.inputs, load_slots, &beat_slices, &in_keys, Dir::In)?;
    check_binds(c, &c.outputs, read_slots, &[], &slot_keys(&outs, c.batch), Dir::Out)?;
    for p in k.inputs.iter().filter(|p| p.hold > 1) {
        if c.batch > 1 && p.hold % c.batch != 0 {
            return Err(QorError::Protocol(format!(
                "{}: a batch of {} does not tile the {}-operation hold of `{}`",
                c.name, c.batch, p.hold, p.name
            )));
        }
    }
    let strobed = c.outputs.iter().filter(|b| b.valid.is_some()).count();
    if strobed > 0 {
        let output_mems = c.mems.iter().any(|m| m.contract.iter().any(|n| k.outputs.iter().any(|o| o.name == contract_ref(n).0)));
        let scheduled = matches!(c.protocol, Protocol::Beats { .. } | Protocol::LoadGoRead { .. } | Protocol::Launch { .. });
        if strobed != c.outputs.len() || output_mems || scheduled {
            return Err(QorError::Protocol(format!(
                "{}: every output is strobed or none is, through ports, under a handshake ({strobed} of {} binds carry a valid, output memories: {output_mems}, {})",
                c.name,
                c.outputs.len(),
                c.protocol.label()
            )));
        }
        for b in &c.outputs {
            let slots: std::collections::BTreeSet<u32> =
                c.outputs.iter().filter(|o| o.dut == b.dut).map(|o| contract_ref(&o.contract).1).collect();
            if slots.len() != 1 {
                return Err(QorError::Protocol(format!(
                    "{}: the strobed port `{}` carries results of {} operations of a call; a strobe marks one",
                    c.name,
                    b.dut,
                    slots.len()
                )));
            }
        }
        for b in &c.outputs {
            let v = find(c, b.valid.as_deref().unwrap_or_default())?;
            expect_dir(c, v, Dir::Out)?;
            if v.bits != 1 {
                return Err(QorError::Protocol(format!("{}: strobe `{}` is {} bits, not 1", c.name, v.name, v.bits)));
            }
        }
    }
    let clk = find(c, &c.clock)?;
    expect_dir(c, clk, Dir::In)?;
    if let Some(r) = &c.reset {
        let p = find(c, &r.port)?;
        expect_dir(c, p, Dir::In)?;
    }
    let control: Vec<(&String, Dir)> = match &c.protocol {
        Protocol::Registered { .. } => vec![],
        Protocol::StartDone { start, done } | Protocol::Pipelined { start, done, .. } => {
            vec![(start, Dir::In), (done, Dir::Out)]
        }
        Protocol::GoDone { go, done } => vec![(go, Dir::In), (done, Dir::Out)],
        Protocol::EnableDone { enable, done, clear } => vec![(enable, Dir::In), (done, Dir::Out), (clear, Dir::In)],
        Protocol::Beats { period, drives, captures } => {
            if captures.len() != c.batch as usize {
                return Err(QorError::Protocol(format!(
                    "{}: {} capture offsets for a batch of {}",
                    c.name,
                    captures.len(),
                    c.batch
                )));
            }
            beat_capture_depths(*period, captures).map_err(|e| QorError::Protocol(format!("{}: {e}", c.name)))?;
            for d in drives {
                if d.beat >= *period {
                    return Err(QorError::Protocol(format!("{}: beat {} of a {period}-cycle call", c.name, d.beat)));
                }
                let p = find(c, &d.dut)?;
                expect_dir(c, p, Dir::In)?;
                if let DriveValue::Slice { width, .. } | DriveValue::Previous { width, .. } = &d.value {
                    if p.bits < *width {
                        return Err(QorError::PortWidth { candidate: c.name.clone(), port: p.name.clone(), need: *width, have: p.bits });
                    }
                }
            }
            vec![]
        }
        Protocol::Launch { start, busy, dcr, setup, flush, bus } => {
            if bus.ports == 0 || bus.line_bytes == 0 || bus.addr_bits == 0 || bus.addr_bits > 64 {
                return Err(QorError::Protocol(format!("{}: a bus of {} ports of {}-byte lines", c.name, bus.ports, bus.line_bytes)));
            }
            for (name, dir, need) in bus.signals() {
                let p = find(c, &name)?;
                expect_dir(c, p, dir)?;
                if p.bits != need {
                    return Err(QorError::PortWidth { candidate: c.name.clone(), port: p.name.clone(), need, have: p.bits });
                }
            }
            let line_bits = bus.line_bits() as u64;
            let mut spans: Vec<(u64, u64, String)> = Vec::new();
            for img in &bus.image {
                if img.hex.len() != 2 * bus.line_bytes as usize || !img.hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(QorError::Protocol(format!(
                        "{}: the image line {:#x} is not {} bytes of hex",
                        c.name, img.line, bus.line_bytes
                    )));
                }
                spans.push((img.line, img.line + 1, format!("image line {:#x}", img.line)));
            }
            for r in &bus.regions {
                let mut dirs = Vec::new();
                let mut content = 0u64;
                for n in &r.contract {
                    let port = contract_ref(n).0;
                    let (dir, bits) = match (k.inputs.iter().find(|p| p.name == port), k.outputs.iter().find(|p| p.name == port)) {
                        (Some(p), _) => (Dir::In, p.bits),
                        (_, Some(p)) => (Dir::Out, p.bits),
                        _ => {
                            return Err(QorError::Unmapped {
                                candidate: c.name.clone(),
                                contract: format!("{n} (not a contract port)"),
                            })
                        }
                    };
                    if r.stride_bits != 0 && r.stride_bits < bits {
                        return Err(QorError::Protocol(format!(
                            "{}: the {bits}-bit port {n} does not fit the {}-bit stride of the region at line {:#x}",
                            c.name, r.stride_bits, r.base_line
                        )));
                    }
                    dirs.push(dir);
                    content += r.span(bits) as u64;
                }
                if dirs.windows(2).any(|w| w[0] != w[1]) {
                    return Err(QorError::Protocol(format!(
                        "{}: the region at line {:#x} mixes operands and results",
                        c.name, r.base_line
                    )));
                }
                let lines = content.div_ceil(line_bits).max(1);
                if bus.addr_bits < 64 && r.base_line + lines > 1u64 << bus.addr_bits {
                    return Err(QorError::Protocol(format!(
                        "{}: the region at line {:#x} runs past {}-bit line addresses",
                        c.name, r.base_line, bus.addr_bits
                    )));
                }
                spans.push((r.base_line, r.base_line + lines, format!("the region at line {:#x}", r.base_line)));
            }
            spans.sort();
            if let Some(w) = spans.windows(2).find(|w| w[1].0 < w[0].1) {
                return Err(QorError::Protocol(format!("{}: {} and {} share a line", c.name, w[0].2, w[1].2)));
            }
            let reg_bits = |v: u64| 64 - v.leading_zeros();
            let addrs = setup.iter().map(|w| w.addr as u64).chain(flush.map(|f| f as u64));
            for (port, need) in [
                (&dcr.addr, addrs.map(reg_bits).max().unwrap_or(1).max(1)),
                (&dcr.data, setup.iter().map(|w| reg_bits(w.data)).max().unwrap_or(1).max(1)),
            ] {
                let p = find(c, port)?;
                expect_dir(c, p, Dir::In)?;
                if p.bits < need {
                    return Err(QorError::PortWidth { candidate: c.name.clone(), port: p.name.clone(), need, have: p.bits });
                }
            }
            vec![(start, Dir::In), (busy, Dir::Out), (&dcr.valid, Dir::In), (&dcr.rw, Dir::In), (&dcr.rsp_valid, Dir::Out)]
        }
        Protocol::LoadGoRead { load, din, go, done, raddr, dout, reset_each_op, load_slots, read_slots } => {
            if *reset_each_op && c.reset.is_none() {
                return Err(QorError::Protocol(format!("{}: resetting each operation needs the DUT's reset port", c.name)));
            }
            let widest = |s: &[RegSlot]| s.iter().map(|s| s.width).max().unwrap_or(1);
            for (port, dir, need) in [
                (din, Dir::In, widest(load_slots)),
                (dout, Dir::Out, widest(read_slots)),
                (raddr, Dir::In, (32 - (read_slots.len().max(2) as u32 - 1).leading_zeros()).max(1)),
            ] {
                let p = find(c, port)?;
                expect_dir(c, p, dir)?;
                if p.bits < need {
                    return Err(QorError::PortWidth { candidate: c.name.clone(), port: p.name.clone(), need, have: p.bits });
                }
            }
            vec![(load, Dir::In), (go, Dir::In), (done, Dir::Out)]
        }
    };
    for (name, dir) in control {
        let p = find(c, name)?;
        expect_dir(c, p, dir)?;
        if p.bits != 1 {
            return Err(QorError::Protocol(format!(
                "{}: control port `{name}` is {} bits, not 1",
                c.name, p.bits
            )));
        }
    }
    Ok(())
}

fn hex64(v: u64) -> String {
    format!("64'h{v:016x}")
}

fn sized_const(bits: u32, v: i128) -> String {
    let m = if bits >= 64 { v as u64 } else { (v as u64) & ((1u64 << bits) - 1) };
    format!("{bits}'h{m:x}")
}

/// The Verilog expression of a field over lane wire `lane` in `mode` (64 bits wide).
fn field_verilog(f: &Field, lane: &str, mode: Mode) -> String {
    let w = f.width;
    if f.lanes == 1 {
        match mode {
            Mode::Edge => {
                let (lo, hi) = f.range();
                format!("({lane}[0] ? {} : {})", hex64(hi as u64), hex64(lo as u64))
            }
            Mode::Shaped => {
                let slice = if w >= 64 {
                    lane.to_string()
                } else if f.signed {
                    format!("{{{{{}{{{lane}[{}]}}}}, {lane}[{}:0]}}", 64 - w, w - 1, w - 1)
                } else {
                    format!("{{{}'d0, {lane}[{}:0]}}", 64 - w, w - 1)
                };
                if f.base == 0 {
                    slice
                } else {
                    format!("({} + {slice})", hex64(f.base as u64))
                }
            }
        }
    } else {
        let used = f.lanes * w;
        let pad = if used < 64 { format!("{}'d0, ", 64 - used) } else { String::new() };
        match mode {
            Mode::Shaped => format!("{{{pad}{lane}[{}:0]}}", used - 1),
            Mode::Edge => {
                let (lo, hi) = f.slice_bounds();
                let parts: Vec<String> = (0..f.lanes)
                    .rev()
                    .map(|j| format!("({lane}[{j}] ? {} : {})", sized_const(w, hi), sized_const(w, lo)))
                    .collect();
                format!("{{{pad}{}}}", parts.join(", "))
            }
        }
    }
}

fn rotl_verilog(src: &str, r: u32) -> String {
    if r == 0 {
        src.to_string()
    } else {
        format!("{{{src}[{}:0], {src}[63:{}]}}", 63 - r, 64 - r)
    }
}

fn input_bits(k: &Kernel, name: &str) -> u32 {
    k.inputs.iter().find(|p| p.name == name).map(|p| p.bits).unwrap_or(0)
}

fn output_bits(k: &Kernel, name: &str) -> u32 {
    k.outputs.iter().find(|p| p.name == name).map(|p| p.bits).unwrap_or(0)
}

fn field_signed(k: &Kernel, name: &str) -> bool {
    k.inputs
        .iter()
        .find(|p| p.name == name)
        .map(|p| p.fields.iter().any(|f| f.signed || f.base < 0))
        .unwrap_or(false)
}

/// The Verilog of a stimulus jump: `name` is `src` advanced `n` steps, one parity per bit.
fn jump_verilog(name: &str, src: &str, n: u32) -> String {
    let mut v = format!("  wire [63:0] {name};\n");
    for (i, row) in jump_rows(n).iter().enumerate() {
        v.push_str(&format!("  assign {name}[{i}] = ^({src} & {});\n", hex64(*row)));
    }
    v
}

/// The harness side of a launched device's memory bus: every region's lines (an input region's are
/// the call's operands; an output region's are registers the device's writes land in, with a `_next`
/// that has this cycle's writes applied, which is what a capture takes), the constant image lines, and
/// per port an in-order queue of the reads still to answer, each answered with its line as it is then.
#[allow(clippy::too_many_arguments)]
fn emit_line_bus(
    k: &Kernel,
    bus: &LineBus,
    opnd: &dyn Fn(&str) -> String,
    v: &mut String,
    conns: &mut Vec<(String, String)>,
    reset: &mut String,
    step: &mut String,
    outputs: &mut Vec<(String, String)>,
) {
    let (np, lb, ab, tb, bytes) = (bus.ports, bus.line_bits(), bus.addr_bits, bus.tag_bits, bus.line_bytes);
    const DEPTH: u32 = 32;
    for (name, _, w) in bus.signals() {
        let suffix = name.strip_prefix(&format!("{}_", bus.prefix)).unwrap_or(&name).to_string();
        let wire = if suffix == "req_ready" { "lb_ready".to_string() } else { format!("lb_{suffix}") };
        v.push_str(&format!("  wire [{}:0] {wire};\n", w - 1));
        conns.push((name, wire));
    }
    v.push_str(&format!("  assign lb_ready = {{{np}{{1'b1}}}};\n  integer lb_w;\n"));
    for p in 0..np {
        v.push_str(&format!(
            "  wire lb_v{p} = {};\n  wire lb_rw{p} = {};\n  wire [{}:0] lb_a{p} = {};\n  wire [{}:0] lb_d{p} = {};\n  wire [{}:0] lb_be{p} = {};\n  wire [{}:0] lb_t{p} = {};\n  wire lb_rr{p} = {};\n",
            bus.slot("lb_req_valid", p, 1),
            bus.slot("lb_req_rw", p, 1),
            ab - 1,
            bus.slot("lb_req_addr", p, ab),
            lb - 1,
            bus.slot("lb_req_data", p, lb),
            bytes - 1,
            bus.slot("lb_req_byteen", p, bytes),
            tb - 1,
            bus.slot("lb_req_tag", p, tb),
            bus.slot("lb_rsp_ready", p, 1),
        ));
    }
    // The regions, and the lookup every port answers its reads with.
    let mut lookup = format!("  function [{}:0] lb_line(input [{}:0] l);\n    begin\n      case (l)\n", lb - 1, ab - 1);
    let hex_digits = ab.div_ceil(4) as usize;
    for img in &bus.image {
        lookup.push_str(&format!("        {ab}'h{:0hex_digits$x}: lb_line = {lb}'h{};\n", img.line, img.hex));
    }
    lookup.push_str(&format!("        default: lb_line = {lb}'d0;\n      endcase\n"));
    for (ri, r) in bus.regions.iter().enumerate() {
        let first = contract_ref(&r.contract[0]).0;
        let is_input = k.inputs.iter().any(|p| p.name == first);
        let bits_of = |n: &str| {
            let port = contract_ref(n).0;
            if is_input { input_bits(k, port) } else { output_bits(k, port) }
        };
        let content: u32 = r.contract.iter().map(|n| r.span(bits_of(n))).sum();
        let lines = content.div_ceil(lb).max(1);
        let total = lines * lb;
        let (base, end) = (r.base_line, r.base_line + lines as u64);
        let name = format!("lr_{ri}");
        if is_input {
            let parts: Vec<String> = r
                .contract
                .iter()
                .rev()
                .map(|n| {
                    let (bits, o) = (bits_of(n), opnd(n));
                    let ext = r.span(bits) - bits;
                    if ext == 0 {
                        o
                    } else if field_signed(k, contract_ref(n).0) {
                        format!("{{{{{ext}{{{o}[{}]}}}}, {o}}}", bits - 1)
                    } else {
                        format!("{{{ext}'d0, {o}}}")
                    }
                })
                .collect();
            let body = if total > content {
                format!("{{{}'d0, {}}}", total - content, parts.join(", "))
            } else {
                format!("{{{}}}", parts.join(", "))
            };
            v.push_str(&format!("  wire [{}:0] {name} = {body};\n", total - 1));
        } else {
            v.push_str(&format!("  reg  [{t}:0] {name};\n  reg  [{t}:0] {name}_next;\n  integer lb_n{ri};\n", t = total - 1));
            reset.push_str(&format!("      {name} <= {total}'d0;\n"));
            let mut next = format!("  always @(*) begin\n    {name}_next = {name};\n");
            for p in 0..np {
                let hit = format!("lb_v{p} && lb_rw{p} && lb_a{p} >= {ab}'d{base} && lb_a{p} < {ab}'d{end}");
                step.push_str(&format!(
                    "      if ({hit}) for (lb_w = 0; lb_w < {bytes}; lb_w = lb_w + 1) if (lb_be{p}[lb_w]) {name}[(lb_a{p} - {ab}'d{base}) * {lb} + lb_w * 8 +: 8] <= lb_d{p}[lb_w * 8 +: 8];\n"
                ));
                next.push_str(&format!(
                    "    if ({hit}) for (lb_n{ri} = 0; lb_n{ri} < {bytes}; lb_n{ri} = lb_n{ri} + 1) if (lb_be{p}[lb_n{ri}]) {name}_next[(lb_a{p} - {ab}'d{base}) * {lb} + lb_n{ri} * 8 +: 8] = lb_d{p}[lb_n{ri} * 8 +: 8];\n"
                ));
            }
            next.push_str("  end\n");
            v.push_str(&next);
            let mut off = 0;
            for n in &r.contract {
                let b = bits_of(n);
                outputs.push((slot_key(n), format!("{name}_next[{}:{}]", off + b - 1, off)));
                off += r.span(b);
            }
        }
        lookup.push_str(&format!(
            "      if (l >= {ab}'d{base} && l < {ab}'d{end}) lb_line = {name}[(l - {ab}'d{base}) * {lb} +: {lb}];\n"
        ));
    }
    lookup.push_str("    end\n  endfunction\n");
    v.push_str(&lookup);
    // Per port, the reads still to answer, oldest first.
    let (qa, qn) = (index_bits(DEPTH), index_bits(DEPTH + 1));
    for p in 0..np {
        v.push_str(&format!(
            "  reg  [{a1}:0] lq_a{p} [0:{d1}];\n  reg  [{t1}:0] lq_t{p} [0:{d1}];\n  reg  [{q1}:0] lq_h{p}, lq_l{p};\n  reg  [{n1}:0] lq_n{p};\n\
             \x20 wire lb_rd{p} = lb_v{p} && !lb_rw{p};\n  wire lb_pop{p} = (lq_n{p} != {qn}'d0) && lb_rr{p};\n\
             \x20 assign {} = (lq_n{p} != {qn}'d0);\n  assign {} = lb_line(lq_a{p}[lq_h{p}]);\n  assign {} = lq_t{p}[lq_h{p}];\n",
            bus.slot("lb_rsp_valid", p, 1),
            bus.slot("lb_rsp_data", p, lb),
            bus.slot("lb_rsp_tag", p, tb),
            a1 = ab - 1,
            d1 = DEPTH - 1,
            t1 = tb - 1,
            q1 = qa - 1,
            n1 = qn - 1,
        ));
        reset.push_str(&format!("      lq_h{p} <= {qa}'d0;\n      lq_l{p} <= {qa}'d0;\n      lq_n{p} <= {qn}'d0;\n"));
        step.push_str(&format!(
            "      if (lb_rd{p}) begin\n        lq_a{p}[lq_l{p}] <= lb_a{p};\n        lq_t{p}[lq_l{p}] <= lb_t{p};\n        lq_l{p} <= lq_l{p} + {qa}'d1;\n        if (lq_n{p} == {qn}'d{DEPTH} && !lb_pop{p}) ovf <= 1'b1;\n      end\n\
             \x20     if (lb_pop{p}) lq_h{p} <= lq_h{p} + {qa}'d1;\n      lq_n{p} <= lq_n{p} + (lb_rd{p} ? {qn}'d1 : {qn}'d0) - (lb_pop{p} ? {qn}'d1 : {qn}'d0);\n"
        ));
    }
}

/// The wrapper `qorbist_<candidate>(clk, rst, sig)` around `c` for kernel `k` in `mode`. Call
/// [`check_ports`] first: the wrapper assumes the binding fits.
///
/// The operands of the call being presented sit in registers (`p_<port>`, one set per batch slot as
/// `p_<port>_<j>`), computed one call ahead from the stimulus state by a per-port function; a batch
/// of `n` advances the state `n` steps through one GF(2) matrix per slot. Each result is registered
/// when it is captured and folded into the MISR one per cycle in operation order — so no wrapper path
/// is longer than one field adder, one jump network or one fold tree, and the measured timing is the
/// candidate's.
pub fn emit_harness(k: &Kernel, c: &Candidate, mode: Mode) -> String {
    emit_harness_as(k, c, mode, &harness_module(c))
}

/// [`emit_harness`] as the module `name` — the harness hardened as a block of its own, whose top module the flow
/// names after the block.
pub fn emit_harness_as(k: &Kernel, c: &Candidate, mode: Mode, name: &str) -> String {
    let nb = c.batch.max(1);
    let sl = |base: &str, j: u32| if nb == 1 { base.to_string() } else { format!("{base}_{j}") };
    let opnd = |r: &str| {
        let (p, j) = contract_ref(r);
        sl(&format!("o_{p}"), j)
    };
    let mut v = String::new();
    v.push_str(&format!(
        "// {m} — the comparative-synthesis harness around `{top}` ({id} {kname}, {proto}{batch}).\n\
         // Operands: xorshift64(13,7,17) from 64'h{SEED:016x}, advanced per accepted operation, one rotated\n\
         // lane word per field, shaped into the {id} contract's domains ({mode:?}). Results: a 64-bit MISR\n\
         // advanced per valid result, so the signature is independent of latency and stalls.\n\
         module {m}(input clk, input rst, output [15:0] sig);\n\
         \x20 reg  [63:0] s;\n\
         \x20 reg  [63:0] misr;\n\
         \x20 reg  [31:0] ops;\n\
         \x20 reg  [31:0] issued;\n",
        m = name,
        top = c.top,
        id = k.id,
        kname = k.name,
        proto = c.protocol.label(),
        batch = if nb > 1 { format!(", {nb} operations per call") } else { String::new() },
    ));
    if nb == 1 {
        v.push_str(
            "  wire [63:0] x1 = s ^ (s << 13);\n  wire [63:0] x2 = x1 ^ (x1 >> 7);\n  wire [63:0] s_next = x2 ^ (x2 << 17);\n",
        );
    } else {
        v.push_str("  wire [63:0] s_0 = s;\n");
        for j in 1..nb {
            v.push_str(&jump_verilog(&format!("s_{j}"), "s", j));
        }
        v.push_str(&jump_verilog("s_jump", "s", nb));
    }
    let state = |j: u32| if nb == 1 { "s".to_string() } else { format!("s_{j}") };

    // Under go/done the component restarts in the cycle it raises `done`: from that cycle on it is
    // shown the next call's operands (`o_<port>`); every other protocol is shown `p_<port>`.
    let restart_done = match &c.protocol {
        Protocol::GoDone { done, .. } => {
            v.push_str(&format!("  wire q_ctl_{done};\n"));
            Some(format!("q_ctl_{done}"))
        }
        _ => None,
    };

    // One function per input port: its value for the operation whose state is `st`.
    let mut lane = 0usize;
    let mut operand_reset = String::new();
    let mut operand_step = String::new();
    for p in &k.inputs {
        let b = p.bits;
        let n = p.name;
        let locals: Vec<String> = (0..p.fields.len()).map(|i| format!("l{}", lane + i)).collect();
        v.push_str(&format!(
            "  function [{}:0] v_{n}(input [63:0] st);\n    reg [63:0] {};\n    begin\n",
            b - 1,
            locals.join(", ")
        ));
        let mut chunks = Vec::new();
        for f in &p.fields {
            let lw = format!("l{lane}");
            v.push_str(&format!(
                "      {lw} = {} ^ {};\n",
                rotl_verilog("st", lane_rotation(lane)),
                hex64((lane as u64).wrapping_mul(LANE_MIX))
            ));
            chunks.push(field_verilog(f, &lw, mode));
            lane += 1;
        }
        chunks.reverse();
        v.push_str(&format!("      v_{n} = {{{}}};\n    end\n  endfunction\n", chunks.join(", ")));
        if p.hold > 1 {
            // One register for the whole batch: a call never straddles a reload.
            let lg = p.hold.trailing_zeros();
            let reg = sl(&format!("p_{n}"), 0);
            v.push_str(&format!("  reg  [{}:0] {reg};\n", b - 1));
            for j in 1..nb {
                v.push_str(&format!("  wire [{}:0] {} = {reg};\n", b - 1, sl(&format!("p_{n}"), j)));
            }
            operand_reset.push_str(&format!("      {reg} <= v_{n}({});\n", hex64(SEED)));
            let reload = format!("(issued[{}:0] == {lg}'d{})", lg - 1, p.hold - nb);
            operand_step.push_str(&format!("        if {reload} {reg} <= v_{n}(s);\n"));
            for j in 0..nb {
                let pj = sl(&format!("p_{n}"), j);
                let oj = sl(&format!("o_{n}"), j);
                match &restart_done {
                    Some(done) => v.push_str(&format!("  wire [{}:0] {oj} = {done} ? ({reload} ? v_{n}(s) : {pj}) : {pj};\n", b - 1)),
                    None => v.push_str(&format!("  wire [{}:0] {oj} = {pj};\n", b - 1)),
                }
            }
        } else {
            for j in 0..nb {
                let pj = sl(&format!("p_{n}"), j);
                let oj = sl(&format!("o_{n}"), j);
                v.push_str(&format!("  reg  [{}:0] {pj};\n", b - 1));
                operand_reset.push_str(&format!("      {pj} <= v_{n}({});\n", hex64(advance(SEED, j))));
                operand_step.push_str(&format!("        {pj} <= v_{n}({});\n", state(j)));
                match &restart_done {
                    Some(done) => v.push_str(&format!("  wire [{}:0] {oj} = {done} ? v_{n}({}) : {pj};\n", b - 1, state(j))),
                    None => v.push_str(&format!("  wire [{}:0] {oj} = {pj};\n", b - 1)),
                }
            }
        }
    }

    // DUT input expressions, per DUT port: the bound contract slices, extended to the port width.
    let mut dut_conns: Vec<(String, String)> = Vec::new();
    let mut by_dut_in: Vec<(String, Vec<&Bind>)> = Vec::new();
    for b in &c.inputs {
        match by_dut_in.iter_mut().find(|(d, _)| *d == b.dut) {
            Some((_, list)) => list.push(b),
            None => by_dut_in.push((b.dut.clone(), vec![b])),
        }
    }
    for (dut, binds) in &by_dut_in {
        let widths: Vec<u32> = binds
            .iter()
            .map(|b| b.resolved_width(input_bits(k, contract_ref(&b.contract).0)))
            .collect();
        let bound: u32 = widths.iter().sum();
        let port_bits = c.ports.iter().find(|p| &p.name == dut).map(|p| p.bits).unwrap_or(bound);
        let body = binds
            .iter()
            .zip(&widths)
            .rev()
            .map(|(b, w)| format!("{}[{}:{}]", opnd(&b.contract), b.lsb + w - 1, b.lsb))
            .collect::<Vec<_>>()
            .join(", ");
        let ext = port_bits.saturating_sub(bound);
        let top = binds.last().map(|b| (b, *widths.last().unwrap_or(&1)));
        let expr = match top {
            _ if ext == 0 => format!("{{{body}}}"),
            Some((b, w)) if field_signed(k, contract_ref(&b.contract).0) => {
                format!("{{{{{ext}{{{}[{}]}}}}, {body}}}", opnd(&b.contract), b.lsb + w - 1)
            }
            _ => format!("{{{ext}'d0, {body}}}"),
        };
        v.push_str(&format!("  wire [{}:0] d_{dut} = {expr};\n", port_bits - 1));
        dut_conns.push((dut.clone(), format!("d_{dut}")));
    }

    // Memory interfaces: inputs answer reads from the presented operands, outputs keep each write.
    let mut mem_reset = String::new();
    let mut mem_step = String::new();
    let mut mem_outputs: Vec<(String, String)> = Vec::new();
    let mut mem_next = String::new();
    for m in &c.mems {
        let first = m.contract.first().cloned().unwrap_or_default();
        let first_port = contract_ref(&first).0;
        let is_input = k.inputs.iter().any(|p| p.name == first_port);
        let bits = if is_input { input_bits(k, first_port) } else { output_bits(k, first_port) };
        let word = bits / m.words_per_port;
        let total = word * m.words();
        let pre = &m.prefix;
        let width_of = |name: &str, fallback: u32| c.ports.iter().find(|p| p.name == name).map(|p| p.bits).unwrap_or(fallback);
        if is_input {
            let content = m.contract.iter().rev().map(|n| opnd(n)).collect::<Vec<_>>().join(", ");
            v.push_str(&format!("  wire [{}:0] mem_{pre} = {{{content}}};\n", total - 1));
        } else {
            v.push_str(&format!("  reg  [{t}:0] mem_{pre};\n  reg  [{t}:0] mem_{pre}_next;\n", t = total - 1));
            for (j, n) in m.contract.iter().enumerate() {
                mem_outputs.push((slot_key(n), format!("mem_{pre}_next[{}:{}]", (j as u32 + 1) * bits - 1, j as u32 * bits)));
            }
            mem_next.push_str(&format!("  always @(*) begin\n    mem_{pre}_next = mem_{pre};\n"));
        }
        let word_at = |addr: &str| {
            if m.words() == 1 {
                format!("mem_{pre}")
            } else {
                format!("mem_{pre}[{addr} * {word} +: {word}]")
            }
        };
        let next_at = |addr: &str| {
            if m.words() == 1 {
                format!("mem_{pre}_next")
            } else {
                format!("mem_{pre}_next[{addr} * {word} +: {word}]")
            }
        };
        match &m.kind {
            MemKind::SeqMem => {
                let (rd, wd) = (width_of(&m.port("read_data"), word), width_of(&m.port("write_data"), word));
                for (k, (name, bits)) in m.address_ports().into_iter().enumerate() {
                    v.push_str(&format!("  wire [{}:0] q_{pre}_addr{k};\n", width_of(&name, bits) - 1));
                    dut_conns.push((name, format!("q_{pre}_addr{k}")));
                }
                v.push_str(&format!(
                    "  wire q_{pre}_content_en, q_{pre}_write_en;\n  wire [{}:0] q_{pre}_write_data;\n  reg  [{}:0] m_{pre}_read;\n  reg  m_{pre}_done;\n",
                    wd - 1,
                    rd - 1
                ));
                let at = m.word_index();
                for (suffix, wire) in [
                    ("content_en", format!("q_{pre}_content_en")),
                    ("write_en", format!("q_{pre}_write_en")),
                    ("write_data", format!("q_{pre}_write_data")),
                    ("read_data", format!("m_{pre}_read")),
                    ("done", format!("m_{pre}_done")),
                ] {
                    dut_conns.push((m.port(suffix), wire));
                }
                mem_reset.push_str(&format!("      m_{pre}_done <= 1'b0;\n"));
                mem_step.push_str(&format!("      m_{pre}_done <= q_{pre}_content_en;\n"));
                if is_input {
                    mem_step.push_str(&format!(
                        "      if (q_{pre}_content_en && !q_{pre}_write_en) m_{pre}_read <= {};\n",
                        word_at(&at)
                    ));
                } else {
                    mem_step.push_str(&format!(
                        "      if (q_{pre}_content_en && q_{pre}_write_en) {} <= q_{pre}_write_data[{}:0];\n      else if (q_{pre}_content_en) m_{pre}_read <= {};\n",
                        word_at(&at),
                        word - 1,
                        word_at(&at)
                    ));
                    mem_next.push_str(&format!(
                        "    if (q_{pre}_content_en && q_{pre}_write_en) {} = q_{pre}_write_data[{}:0];\n",
                        next_at(&at),
                        word - 1
                    ));
                }
            }
            MemKind::Bram { ports } => {
                for i in 0..*ports {
                    let ad = width_of(&m.port(&format!("address{i}")), m.addr_bits());
                    v.push_str(&format!("  wire [{}:0] q_{pre}_address{i};\n  wire q_{pre}_ce{i};\n", ad - 1));
                    dut_conns.push((m.port(&format!("address{i}")), format!("q_{pre}_address{i}")));
                    dut_conns.push((m.port(&format!("ce{i}")), format!("q_{pre}_ce{i}")));
                    let has = |s: &str| c.ports.iter().any(|p| p.name == m.port(s));
                    if has(&format!("q{i}")) {
                        let qd = width_of(&m.port(&format!("q{i}")), word);
                        v.push_str(&format!("  reg  [{}:0] m_{pre}_q{i};\n", qd - 1));
                        dut_conns.push((m.port(&format!("q{i}")), format!("m_{pre}_q{i}")));
                        let guard = if has(&format!("we{i}")) { format!(" && !q_{pre}_we{i}") } else { String::new() };
                        mem_step.push_str(&format!(
                            "      if (q_{pre}_ce{i}{guard}) m_{pre}_q{i} <= {};\n",
                            word_at(&format!("q_{pre}_address{i}"))
                        ));
                    }
                    if !is_input {
                        let dd = width_of(&m.port(&format!("d{i}")), word);
                        v.push_str(&format!("  wire q_{pre}_we{i};\n  wire [{}:0] q_{pre}_d{i};\n", dd - 1));
                        dut_conns.push((m.port(&format!("we{i}")), format!("q_{pre}_we{i}")));
                        dut_conns.push((m.port(&format!("d{i}")), format!("q_{pre}_d{i}")));
                        mem_step.push_str(&format!(
                            "      if (q_{pre}_ce{i} && q_{pre}_we{i}) {} <= q_{pre}_d{i}[{}:0];\n",
                            word_at(&format!("q_{pre}_address{i}")),
                            word - 1
                        ));
                        mem_next.push_str(&format!(
                            "    if (q_{pre}_ce{i} && q_{pre}_we{i}) {} = q_{pre}_d{i}[{}:0];\n",
                            next_at(&format!("q_{pre}_address{i}")),
                            word - 1
                        ));
                    }
                }
            }
        }
        if !is_input {
            mem_next.push_str("  end\n");
        }
    }
    v.push_str(&mem_next);
    if let Protocol::Launch { bus, .. } = &c.protocol {
        emit_line_bus(k, bus, &opnd, &mut v, &mut dut_conns, &mut mem_reset, &mut mem_step, &mut mem_outputs);
    }

    // DUT outputs: one wire per output port, with an in-order queue when its values are strobed.
    let mut out_wires: Vec<String> = Vec::new();
    for b in &c.outputs {
        if !out_wires.contains(&b.dut) {
            out_wires.push(b.dut.clone());
        }
    }
    let strobed = c.outputs.iter().any(|b| b.valid.is_some());
    // A strobed port's queue belongs to the one operation of the call it carries, and is taken when every queue of
    // that operation holds its result.
    let slot_of = |d: &str| c.outputs.iter().find(|b| b.dut == d).map_or(0, |b| contract_ref(&b.contract).1);
    let strobe_capture = |j: u32| if nb == 1 { "capture".to_string() } else { format!("capture_{j}") };
    let mut fifo_reset = String::new();
    let mut fifo_step = String::new();
    let mut fifo_ready: Vec<Vec<String>> = vec![Vec::new(); nb as usize];
    for d in &out_wires {
        let bound = c
            .outputs
            .iter()
            .filter(|b| &b.dut == d)
            .map(|b| b.resolved_width(output_bits(k, contract_ref(&b.contract).0)))
            .max()
            .unwrap_or(1);
        let w = c.ports.iter().find(|p| &p.name == d).map(|p| p.bits).unwrap_or(bound);
        v.push_str(&format!("  wire [{}:0] q_{d};\n", w.max(1) - 1));
        dut_conns.push((d.clone(), format!("q_{d}")));
        if let Some(valid) = c.outputs.iter().find(|b| &b.dut == d).and_then(|b| b.valid.clone()) {
            v.push_str(&format!(
                "  wire q_{d}_vld;\n  reg  [{wb}:0] f_{d} [0:15];\n  reg  [3:0] wp_{d}, rp_{d};\n  reg  [4:0] n_{d};\n  wire [{wb}:0] h_{d} = f_{d}[rp_{d}];\n",
                wb = w.max(1) - 1
            ));
            dut_conns.push((valid, format!("q_{d}_vld")));
            fifo_reset.push_str(&format!("      wp_{d} <= 4'd0;\n      rp_{d} <= 4'd0;\n      n_{d} <= 5'd0;\n"));
            let slot = slot_of(d);
            let take = strobe_capture(slot);
            fifo_step.push_str(&format!(
                "      if (q_{d}_vld) begin\n        f_{d}[wp_{d}] <= q_{d};\n        wp_{d} <= wp_{d} + 4'd1;\n      end\n\
                 \x20     if (q_{d}_vld && n_{d} == 5'd16 && !{take}) ovf <= 1'b1;\n\
                 \x20     if ({take}) rp_{d} <= rp_{d} + 4'd1;\n\
                 \x20     n_{d} <= n_{d} + (q_{d}_vld ? 5'd1 : 5'd0) - ({take} ? 5'd1 : 5'd0);\n"
            ));
            fifo_ready[slot as usize].push(format!("(n_{d} != 5'd0)"));
        }
    }
    let source = |d: &str| if strobed { format!("h_{d}") } else { format!("q_{d}") };

    // Results: per batch slot, the result wires and the registers they are captured into — a queue
    // where a beat schedule delivers the slot again before the fold reaches it.
    let register_file = matches!(c.protocol, Protocol::LoadGoRead { .. });
    let depths: Vec<u32> = match &c.protocol {
        Protocol::Beats { period, captures, .. } => {
            beat_capture_depths(*period, captures).unwrap_or_else(|_| vec![1; nb as usize])
        }
        _ => vec![1; nb as usize],
    };
    let mut result_capture: Vec<String> = vec![String::new(); nb as usize];
    let mut queue_reset = String::new();
    for j in 0..nb {
        let depth = depths[j as usize];
        if depth > 1 {
            let (ab, cb) = (index_bits(depth), index_bits(depth + 1));
            v.push_str(&format!("  reg  [{}:0] wp_{j}, rp_{j};\n  reg  [{}:0] cn_{j};\n", ab - 1, cb - 1));
            queue_reset.push_str(&format!("      wp_{j} <= {ab}'d0;\n      rp_{j} <= {ab}'d0;\n      cn_{j} <= {cb}'d0;\n"));
        }
        for p in &k.outputs {
            let n = p.name;
            let (rj, rq) = (sl(&format!("r_{n}"), j), sl(&format!("r_{n}"), j) + "_q");
            let key = format!("{n}@{j}");
            let expr = if register_file {
                v.push_str(&format!("  reg  [{}:0] rd_{n};\n", p.bits - 1));
                format!("rd_{n}")
            } else if let Some((_, slice)) = mem_outputs.iter().find(|(kk, _)| *kk == key) {
                slice.clone()
            } else {
                let mut binds: Vec<&Bind> = c.outputs.iter().filter(|b| slot_key(&b.contract) == key).collect();
                binds.sort_by_key(|b| std::cmp::Reverse(b.lsb));
                let parts: Vec<String> = binds
                    .iter()
                    .map(|b| {
                        let w = b.resolved_width(p.bits);
                        let dw = b.dut_bits(p.bits);
                        let bits = format!("{}[{}:{}]", source(&b.dut), b.dut_lsb + dw - 1, b.dut_lsb);
                        if dw < w {
                            format!("{{{{{}{{{}[{}]}}}}, {bits}}}", w - dw, source(&b.dut), b.dut_lsb + dw - 1)
                        } else {
                            bits
                        }
                    })
                    .collect();
                format!("{{{}}}", parts.join(", "))
            };
            if depth > 1 {
                v.push_str(&format!(
                    "  wire [{b}:0] {rj} = {expr};\n  reg  [{b}:0] {rq}_e [0:{}];\n  wire [{b}:0] {rq} = {rq}_e[rp_{j}];\n",
                    depth - 1,
                    b = p.bits - 1
                ));
                result_capture[j as usize].push_str(&format!("        {rq}_e[wp_{j}] <= {rj};\n"));
            } else {
                v.push_str(&format!("  wire [{b}:0] {rj} = {expr};\n  reg  [{b}:0] {rq};\n", b = p.bits - 1));
                result_capture[j as usize].push_str(&format!("        {rq} <= {rj};\n"));
            }
        }
    }

    // The fold of each slot's registered result, and the one the fold pointer selects.
    let fp_bits = (32 - (nb.max(2) - 1).leading_zeros()).max(1);
    for j in 0..nb {
        let mut c_idx = 0u32;
        let mut terms = Vec::new();
        for p in &k.outputs {
            let rq = sl(&format!("r_{}", p.name), j) + "_q";
            for i in 0..p.bits.div_ceil(64) {
                let cb = chunk_bits(p.bits, i as usize);
                let chunk = if cb == 64 {
                    format!("{rq}[{}:{}]", 64 * i + 63, 64 * i)
                } else {
                    format!("{{{}'d0, {rq}[{}:{}]}}", 64 - cb, 64 * i + cb - 1, 64 * i)
                };
                let name = sl(&format!("rc{c_idx}"), j);
                v.push_str(&format!("  wire [63:0] {name} = {chunk};\n"));
                terms.push(rotl_verilog(&name, c_idx % 64));
                c_idx += 1;
            }
        }
        v.push_str(&format!("  wire [63:0] {} = {};\n", sl("fold", j), terms.join(" ^ ")));
    }
    v.push_str(&format!("  reg  [{}:0] full;\n", nb - 1));
    if nb == 1 {
        v.push_str("  wire folding = full[0];\n  wire [63:0] fold_sel = fold;\n");
        for p in &k.outputs {
            v.push_str(&format!("  wire [{}:0] fr_{n} = r_{n}_q;\n", p.bits - 1, n = p.name));
        }
    } else {
        v.push_str(&format!("  reg  [{}:0] fp;\n  wire folding = full[fp];\n", fp_bits - 1));
        let sel = |base: &dyn Fn(u32) -> String| {
            (0..nb).rev().fold(String::from("64'd0"), |acc, j| {
                if acc == "64'd0" && j == nb - 1 {
                    base(j)
                } else {
                    format!("(fp == {fp_bits}'d{j}) ? {} : ({acc})", base(j))
                }
            })
        };
        v.push_str(&format!("  wire [63:0] fold_sel = {};\n", sel(&|j| format!("fold_{j}"))));
        for p in &k.outputs {
            let n = p.name;
            v.push_str(&format!("  wire [{}:0] fr_{n} = {};\n", p.bits - 1, sel(&|j| format!("r_{n}_{j}_q"))));
        }
    }

    // Protocol control: `accept` advances the operands, each slot's capture registers its result.
    let mut control_regs = String::new();
    let mut control_reset = String::new();
    let mut control_step = String::new();
    let mut slot_capture: Vec<String> = vec!["capture".to_string(); nb as usize];
    match &c.protocol {
        Protocol::Registered { latency } => {
            v.push_str("  wire accept = 1'b1;\n");
            if *latency == 0 {
                v.push_str("  wire proto_capture = 1'b1;\n");
            } else {
                let l = *latency;
                control_regs.push_str(&format!("  reg  [{}:0] vld;\n", l - 1));
                control_reset.push_str(&format!("      vld <= {l}'d0;\n"));
                if l == 1 {
                    control_step.push_str("      vld <= 1'b1;\n");
                } else {
                    control_step.push_str(&format!("      vld <= {{vld[{}:0], 1'b1}};\n", l - 2));
                }
                v.push_str(&format!("  wire proto_capture = vld[{}];\n", l - 1));
            }
        }
        Protocol::StartDone { start, done } => {
            control_regs.push_str("  reg  busy;\n");
            control_reset.push_str("      busy <= 1'b0;\n");
            control_step.push_str(&format!(
                "      if (!busy) busy <= 1'b1;\n      else if (q_ctl_{done}) busy <= 1'b0;\n"
            ));
            v.push_str(&format!("  wire q_ctl_{done};\n  wire c_{start} = !rst && !busy;\n"));
            v.push_str(&format!("  wire accept = busy && q_ctl_{done};\n  wire proto_capture = busy && q_ctl_{done};\n"));
            dut_conns.push((start.clone(), format!("c_{start}")));
            dut_conns.push((done.clone(), format!("q_ctl_{done}")));
        }
        Protocol::Pipelined { start, done, ii } => {
            let ii = (*ii).max(1);
            control_regs.push_str("  reg  [31:0] tick;\n");
            control_reset.push_str("      tick <= 32'd0;\n");
            control_step.push_str(&format!(
                "      tick <= (tick == 32'd{}) ? 32'd0 : tick + 32'd1;\n",
                ii - 1
            ));
            v.push_str(&format!(
                "  wire q_ctl_{done};\n  wire c_{start} = !rst && (tick == 32'd0);\n  wire accept = c_{start};\n  wire proto_capture = q_ctl_{done};\n"
            ));
            dut_conns.push((start.clone(), format!("c_{start}")));
            dut_conns.push((done.clone(), format!("q_ctl_{done}")));
        }
        Protocol::GoDone { go, done } => {
            v.push_str(&format!(
                "  wire c_{go} = !rst;\n  wire accept = q_ctl_{done};\n  wire proto_capture = q_ctl_{done};\n"
            ));
            dut_conns.push((go.clone(), format!("c_{go}")));
            dut_conns.push((done.clone(), format!("q_ctl_{done}")));
        }
        Protocol::EnableDone { enable, done, clear } => {
            control_regs.push_str("  reg  clearing;\n");
            control_reset.push_str("      clearing <= 1'b0;\n");
            control_step.push_str(&format!("      clearing <= !clearing && q_ctl_{done};\n"));
            v.push_str(&format!(
                "  wire q_ctl_{done};\n  wire c_{enable} = !rst && !clearing;\n  wire c_{clear} = clearing;\n\
                 \x20 wire accept = !clearing && q_ctl_{done};\n  wire proto_capture = accept;\n"
            ));
            dut_conns.push((enable.clone(), format!("c_{enable}")));
            dut_conns.push((done.clone(), format!("q_ctl_{done}")));
            dut_conns.push((clear.clone(), format!("c_{clear}")));
        }
        Protocol::Beats { period, drives, captures } => {
            let pw = (32 - (period.max(&2) - 1).leading_zeros()).max(1);
            let deepest = captures.iter().copied().max().unwrap_or(0);
            control_regs.push_str(&format!("  reg  [{}:0] beat;\n", pw - 1));
            control_reset.push_str(&format!("      beat <= {pw}'d0;\n"));
            control_step.push_str(&format!(
                "      beat <= (beat == {pw}'d{}) ? {pw}'d0 : beat + {pw}'d1;\n",
                period - 1
            ));
            v.push_str(&format!("  wire accept = (beat == {pw}'d{});\n  wire proto_capture = 1'b0;\n", period - 1));
            if deepest > 0 {
                control_regs.push_str(&format!("  reg  [{}:0] began;\n", deepest - 1));
                control_reset.push_str(&format!("      began <= {deepest}'d0;\n"));
                if deepest == 1 {
                    control_step.push_str(&format!("      began <= (beat == {pw}'d0);\n"));
                } else {
                    control_step.push_str(&format!("      began <= {{began[{}:0], (beat == {pw}'d0)}};\n", deepest - 2));
                }
            }
            for (j, off) in captures.iter().enumerate() {
                slot_capture[j] = if *off == 0 {
                    format!("(!rst && beat == {pw}'d0)")
                } else {
                    format!("began[{}]", off - 1)
                };
            }
            let mut by_port: Vec<(String, Vec<&Drive>)> = Vec::new();
            for d in drives {
                match by_port.iter_mut().find(|(p, _)| *p == d.dut) {
                    Some((_, list)) => list.push(d),
                    None => by_port.push((d.dut.clone(), vec![d])),
                }
            }
            // The previous call's operands, for lagged drives: taken from the call's registers as the
            // next call begins, zero before the first call.
            let mut kept: Vec<String> = Vec::new();
            for d in drives {
                if let DriveValue::Previous { contract, .. } = &d.value {
                    let (p, j) = contract_ref(contract);
                    let prev = sl(&format!("pv_{p}"), j);
                    if !kept.contains(&prev) {
                        let b = input_bits(k, p);
                        v.push_str(&format!("  reg  [{}:0] {prev};\n", b - 1));
                        operand_reset.push_str(&format!("      {prev} <= {b}'d0;\n"));
                        operand_step.push_str(&format!("        {prev} <= {};\n", sl(&format!("p_{p}"), j)));
                        kept.push(prev);
                    }
                }
            }
            for (port, list) in &by_port {
                let w = c.ports.iter().find(|p| &p.name == port).map(|p| p.bits).unwrap_or(64);
                let mut arms = String::new();
                for d in list {
                    let value = match &d.value {
                        DriveValue::Const(x) => format!("{w}'d{x}"),
                        DriveValue::Slice { contract, lsb, width } | DriveValue::Previous { contract, lsb, width } => {
                            let src = match &d.value {
                                DriveValue::Previous { .. } => {
                                    let (p, j) = contract_ref(contract);
                                    sl(&format!("pv_{p}"), j)
                                }
                                _ => opnd(contract),
                            };
                            let slice = format!("{src}[{}:{}]", lsb + width - 1, lsb);
                            let ext = w.saturating_sub(*width);
                            if ext == 0 {
                                slice
                            } else if field_signed(k, contract_ref(contract).0) {
                                format!("{{{{{ext}{{{src}[{}]}}}}, {slice}}}", lsb + width - 1)
                            } else {
                                format!("{{{ext}'d0, {slice}}}")
                            }
                        }
                    };
                    arms.push_str(&format!("      {pw}'d{}: bd_{port} = {value};\n", d.beat));
                }
                v.push_str(&format!(
                    "  reg  [{}:0] bd_{port};\n  always @(*) begin\n    bd_{port} = {w}'d0;\n    case (beat)\n{arms}      default: ;\n    endcase\n  end\n",
                    w - 1
                ));
                dut_conns.push((port.clone(), format!("bd_{port}")));
            }
        }
        Protocol::Launch { start, busy, dcr, setup, flush, .. } => {
            // 0 setup writes, 1 start, 2 until busy rises, 3 until it falls, 4 flush read, 5 until it is
            // answered, 6 capture — once the previous call's results are all folded, since a call no
            // longer than the fold of a batch would otherwise land on results still unfolded.
            let port_bits = |n: &str| c.ports.iter().find(|p| p.name == n).map(|p| p.bits).unwrap_or(1);
            let (aw, dw) = (port_bits(&dcr.addr), port_bits(&dcr.data));
            let sw = index_bits(setup.len().max(1) as u32);
            control_regs.push_str(&format!("  reg  [2:0] lph;\n  reg  [{}:0] lsi;\n", sw - 1));
            let first = if setup.is_empty() { 1 } else { 0 };
            control_reset.push_str(&format!("      lph <= 3'd{first};\n      lsi <= {sw}'d0;\n"));
            let mut arms = String::new();
            for (i, w) in setup.iter().enumerate() {
                arms.push_str(&format!("        {sw}'d{i}: begin c_dcr_addr = {aw}'d{}; c_dcr_data = {dw}'d{}; end\n", w.addr, w.data));
            }
            let flush_arm = match flush {
                Some(f) => format!("    if (lph == 3'd4) begin c_dcr_valid = 1'b1; c_dcr_rw = 1'b0; c_dcr_addr = {aw}'d{f}; end\n"),
                None => String::new(),
            };
            v.push_str(&format!(
                "  wire q_ctl_{busy};\n  wire q_ctl_{rsp};\n  wire c_{start} = (lph == 3'd1);\n\
                 \x20 reg  c_dcr_valid, c_dcr_rw;\n  reg  [{a1}:0] c_dcr_addr;\n  reg  [{d1}:0] c_dcr_data;\n\
                 \x20 always @(*) begin\n    c_dcr_valid = 1'b0;\n    c_dcr_rw = 1'b1;\n    c_dcr_addr = {aw}'d0;\n    c_dcr_data = {dw}'d0;\n\
                 \x20   if (lph == 3'd0) begin\n      c_dcr_valid = 1'b1;\n      case (lsi)\n{arms}        default: ;\n      endcase\n    end\n{flush_arm}  end\n\
                 \x20 wire proto_capture = (lph == 3'd6) && !(|full);\n  wire accept = proto_capture;\n",
                rsp = dcr.rsp_valid,
                a1 = aw - 1,
                d1 = dw - 1,
            ));
            let after_busy = if flush.is_some() { 4 } else { 6 };
            control_step.push_str(&format!(
                "      case (lph)\n\
                 \x20       3'd0: begin if (lsi == {sw}'d{last}) lph <= 3'd1; else lsi <= lsi + {sw}'d1; end\n\
                 \x20       3'd1: lph <= 3'd2;\n\
                 \x20       3'd2: if (q_ctl_{busy}) lph <= 3'd3;\n\
                 \x20       3'd3: if (!q_ctl_{busy}) lph <= 3'd{after_busy};\n\
                 \x20       3'd4: lph <= 3'd5;\n\
                 \x20       3'd5: if (q_ctl_{rsp}) lph <= 3'd6;\n\
                 \x20       3'd6: if (!(|full)) lph <= 3'd1;\n\
                 \x20       default: lph <= 3'd1;\n\
                 \x20     endcase\n",
                last = setup.len().saturating_sub(1),
                rsp = dcr.rsp_valid,
            ));
            dut_conns.push((start.clone(), format!("c_{start}")));
            dut_conns.push((busy.clone(), format!("q_ctl_{busy}")));
            dut_conns.push((dcr.valid.clone(), "c_dcr_valid".into()));
            dut_conns.push((dcr.rw.clone(), "c_dcr_rw".into()));
            dut_conns.push((dcr.addr.clone(), "c_dcr_addr".into()));
            dut_conns.push((dcr.data.clone(), "c_dcr_data".into()));
            dut_conns.push((dcr.rsp_valid.clone(), format!("q_ctl_{}", dcr.rsp_valid)));
        }
        Protocol::LoadGoRead { load, din, go, done, raddr, dout, reset_each_op, load_slots, read_slots } => {
            let port_bits = |n: &str, fallback: u32| c.ports.iter().find(|p| p.name == n).map(|p| p.bits).unwrap_or(fallback);
            let (dw, ow, aw) = (port_bits(din, 64), port_bits(dout, 64), port_bits(raddr, 8));
            let (ns, nr) = (load_slots.len(), read_slots.len());
            control_regs.push_str("  reg  [2:0] ph;\n  reg  [15:0] li;\n  wire dut_rst = (ph == 3'd0);\n");
            let mut din_case = String::new();
            for (i, s) in load_slots.iter().enumerate() {
                let slice = format!("p_{}[{}:{}]", s.contract, s.lsb + s.width - 1, s.lsb);
                let value = if s.width < dw { format!("{{{}'d0, {slice}}}", dw - s.width) } else { slice };
                din_case.push_str(&format!("      16'd{i}: c_din = {value};\n"));
            }
            let mut read_case = String::new();
            for (i, s) in read_slots.iter().enumerate() {
                read_case.push_str(&format!(
                    "          16'd{i}: rd_{}[{}:{}] <= q_dout[{}:0];\n",
                    s.contract,
                    s.lsb + s.width - 1,
                    s.lsb,
                    s.width - 1
                ));
            }
            let start = if *reset_each_op { 0 } else { 1 };
            v.push_str(&format!(
                "  wire q_ctl_{done};\n  wire [{ow1}:0] q_dout;\n  reg  [{dw1}:0] c_din;\n  always @(*) begin\n    c_din = {dw}'d0;\n    case (li)\n{din_case}      default: ;\n    endcase\n  end\n\
                 \x20 wire c_{load} = (ph == 3'd1);\n  wire c_{go} = (ph == 3'd2);\n  wire [{aw1}:0] c_{raddr} = li[{aw1}:0];\n\
                 \x20 wire proto_capture = (ph == 3'd5);\n  wire accept = proto_capture;\n",
                ow1 = ow - 1,
                dw1 = dw - 1,
                aw1 = aw - 1,
            ));
            dut_conns.push((load.clone(), format!("c_{load}")));
            dut_conns.push((din.clone(), "c_din".into()));
            dut_conns.push((go.clone(), format!("c_{go}")));
            dut_conns.push((done.clone(), format!("q_ctl_{done}")));
            dut_conns.push((raddr.clone(), format!("c_{raddr}")));
            dut_conns.push((dout.clone(), "q_dout".into()));
            control_reset.push_str(&format!("      ph <= 3'd{start};\n      li <= 16'd0;\n"));
            control_step.push_str(&format!(
                "      case (ph)\n\
                 \x20       3'd0: begin ph <= 3'd1; li <= 16'd0; end\n\
                 \x20       3'd1: begin if (li == 16'd{last_load}) begin ph <= 3'd2; li <= 16'd0; end else li <= li + 16'd1; end\n\
                 \x20       3'd2: ph <= 3'd3;\n\
                 \x20       3'd3: if (q_ctl_{done}) begin ph <= 3'd4; li <= 16'd0; end\n\
                 \x20       3'd4: begin\n\
                 \x20         case (li)\n{read_case}\x20         default: ;\n\x20         endcase\n\
                 \x20         if (li == 16'd{last_read}) ph <= 3'd5; else li <= li + 16'd1;\n\
                 \x20       end\n\
                 \x20       3'd5: begin ph <= 3'd{start}; li <= 16'd0; end\n\
                 \x20       default: ;\n      endcase\n",
                last_load = ns.saturating_sub(1),
                last_read = nr.saturating_sub(1),
            ));
        }
    }
    v.push_str(&control_regs);
    if strobed {
        for j in 0..nb {
            v.push_str(&format!("  wire {} = {};\n", strobe_capture(j), fifo_ready[j as usize].join(" && ")));
            slot_capture[j as usize] = strobe_capture(j);
        }
    } else {
        v.push_str("  wire capture = proto_capture;\n");
    }
    v.push_str("  reg  ovf;\n");

    // The DUT.
    let mut conns = vec![format!(".{}(clk)", c.clock)];
    let per_op_reset = matches!(c.protocol, Protocol::LoadGoRead { reset_each_op: true, .. });
    if let Some(r) = &c.reset {
        let level = if per_op_reset { "rst || dut_rst" } else { "rst" };
        conns.push(format!(
            ".{}({})",
            r.port,
            if r.active_high { level.to_string() } else { format!("!({level})") }
        ));
    }
    for (port, expr) in &dut_conns {
        conns.push(format!(".{port}({expr})"));
    }
    // An input the binding does not name is held at 0, never left floating.
    for p in c.ports.iter().filter(|p| p.dir == Dir::In) {
        let named = p.name == c.clock
            || c.reset.as_ref().map_or(false, |r| r.port == p.name)
            || dut_conns.iter().any(|(port, _)| *port == p.name);
        if !named {
            conns.push(format!(".{}({}'d0)", p.name, p.bits));
        }
    }
    v.push_str(&format!(
        "  (* keep_hierarchy *) {} u_dut(\n    {}\n  );\n",
        c.top,
        conns.join(",\n    ")
    ));

    // Capture each slot; fold the slot the pointer selects; refuse a capture onto an unfolded result.
    let mut capture_step = String::new();
    for j in 0..nb {
        let cap = &slot_capture[j as usize];
        let folds_j = if nb == 1 { "folding".to_string() } else { format!("(folding && fp == {fp_bits}'d{j})") };
        let depth = depths[j as usize];
        if depth > 1 {
            let (ab, cb) = (index_bits(depth), index_bits(depth + 1));
            let next = |r: &str| format!("({r} == {ab}'d{}) ? {ab}'d0 : {r} + {ab}'d1", depth - 1);
            let count = format!("cn_{j} + ({cap} ? {cb}'d1 : {cb}'d0) - ({folds_j} ? {cb}'d1 : {cb}'d0)");
            capture_step.push_str(&format!(
                "      if ({cap}) begin\n{}        wp_{j} <= {};\n        if (cn_{j} == {cb}'d{depth} && !{folds_j}) ovf <= 1'b1;\n      end\n\
                 \x20     if ({folds_j}) rp_{j} <= {};\n      cn_{j} <= {count};\n      full[{j}] <= ({count}) != {cb}'d0;\n",
                result_capture[j as usize],
                next(&format!("wp_{j}")),
                next(&format!("rp_{j}")),
            ));
        } else {
            capture_step.push_str(&format!(
                "      if ({cap}) begin\n{}        if (full[{j}] && !{folds_j}) ovf <= 1'b1;\n      end\n      full[{j}] <= {cap} || (full[{j}] && !{folds_j});\n",
                result_capture[j as usize]
            ));
        }
    }
    let advance_fp = if nb == 1 {
        String::new()
    } else {
        format!("        fp <= (fp == {fp_bits}'d{}) ? {fp_bits}'d0 : fp + {fp_bits}'d1;\n", nb - 1)
    };
    let reset_fp = if nb == 1 { String::new() } else { format!("      fp <= {fp_bits}'d0;\n") } + &queue_reset;
    let (state_reset, state_step, issued_step) = if nb == 1 {
        (hex64(next_state(SEED)), "s_next".to_string(), "32'd1".to_string())
    } else {
        (hex64(advance(SEED, nb)), "s_jump".to_string(), format!("32'd{nb}"))
    };
    v.push_str(&format!(
        "  assign sig = misr[15:0];\n\
         \x20 always @(posedge clk) begin\n\
         \x20   if (rst) begin\n\
         \x20     s <= {state_reset};\n\
         \x20     misr <= 64'd0;\n\
         \x20     ops <= 32'd0;\n\
         \x20     issued <= 32'd0;\n\
         \x20     full <= {nb}'d0;\n\
         \x20     ovf <= 1'b0;\n\
         {reset_fp}{operand_reset}{control_reset}{mem_reset}{fifo_reset}\
         \x20   end else begin\n\
         {control_step}{mem_step}{fifo_step}\
         \x20     if (accept) begin\n\
         \x20       s <= {state_step};\n\
         \x20       issued <= issued + {issued_step};\n\
         {operand_step}\
         \x20     end\n\
         {capture_step}\
         \x20     if (folding) begin\n\
         \x20       misr <= {{misr[62:0], ^misr}} ^ fold_sel;\n\
         \x20       ops <= ops + 32'd1;\n\
         {advance_fp}\
         \x20     end\n\
         \x20   end\n\
         \x20 end\n\
         endmodule\n",
    ));
    v
}

/// The power block `<name>` of a design measured without a functional harness — a memory controller, driven at
/// gate level by the K7 traffic testbench: its clock and reset renamed `clk` and `rst` (active high), every other port
/// passed straight through under its own name, the design kept whole as `u_dut`, so the flow's cells are told apart
/// from its own. A port named `clk` or `rst` that is not the clock or reset would collide, and is refused.
pub fn emit_pass_wrapper(name: &str, top: &str, clock: &str, reset: Option<&Reset>, ports: &[DutPort]) -> String {
    let mut decls = vec!["input clk".to_string()];
    if reset.is_some() {
        decls.push("input rst".to_string());
    }
    let mut conns = Vec::new();
    for p in ports {
        if p.name == clock {
            conns.push(format!(".{}(clk)", p.name));
            continue;
        }
        if let Some(r) = reset.filter(|r| r.port == p.name) {
            conns.push(format!(".{}({})", p.name, if r.active_high { "rst" } else { "!rst" }));
            continue;
        }
        assert!(p.name != "clk" && p.name != "rst", "{top}: port {} would collide with the block's clock or reset", p.name);
        let range = if p.bits > 1 { format!("[{}:0] ", p.bits - 1) } else { String::new() };
        decls.push(format!("{} {range}{}", if p.dir == Dir::In { "input" } else { "output" }, p.name));
        conns.push(format!(".{0}({0})", p.name));
    }
    format!(
        "// {name} — `{top}` as a block of its own: every port passed through, its clock and reset as clk and rst, the \
         design kept whole.\nmodule {name}(\n  {}\n);\n  (* keep_hierarchy *) {top} u_dut(\n    {}\n  );\nendmodule\n",
        decls.join(",\n  "),
        conns.join(",\n    ")
    )
}

/// The synthesis wrapper `qorsyn_<name>(clk, rst, sig)` of a design measured without a functional
/// contract (a memory controller, whose functional gate is a board memtest): every input of `top`
/// from a register the seeded xorshift64 stimulus refills each cycle, one rotated lane word per 64
/// bits as the harness draws operands; every output bit folded into a registered 64-bit XOR whose low
/// 16 bits are `sig`; the design kept whole as `u_dut`, so its cells and its register-to-register
/// paths are what synthesis and place-and-route measure.
pub fn emit_fold_wrapper(name: &str, top: &str, clock: &str, reset: Option<&Reset>, ports: &[DutPort]) -> String {
    let mut v = format!(
        "// qorsyn_{name} — the synthesis wrapper around `{top}`: registered stimulus in, every output folded out.\n\
         module qorsyn_{name}(input clk, input rst, output [15:0] sig);\n\
         \x20 reg  [63:0] s;\n  reg  [63:0] fold;\n\
         \x20 wire [63:0] x1 = s ^ (s << 13);\n  wire [63:0] x2 = x1 ^ (x1 >> 7);\n  wire [63:0] s_next = x2 ^ (x2 << 17);\n"
    );
    let mut conns = Vec::new();
    let mut refill = String::new();
    let mut terms = Vec::new();
    let mut lane = 0usize;
    for p in ports {
        if p.name == clock {
            conns.push(format!(".{}(clk)", p.name));
            continue;
        }
        if let Some(r) = reset.filter(|r| r.port == p.name) {
            conns.push(format!(".{}({})", p.name, if r.active_high { "rst" } else { "!rst" }));
            continue;
        }
        match p.dir {
            Dir::In => {
                v.push_str(&format!("  reg  [{}:0] in_{};\n", p.bits - 1, p.name));
                let mut chunks = Vec::new();
                for _ in 0..p.bits.div_ceil(64) {
                    chunks.push(format!("({} ^ {})", rotl_verilog("s", lane_rotation(lane)), hex64((lane as u64).wrapping_mul(LANE_MIX))));
                    lane += 1;
                }
                chunks.reverse();
                refill.push_str(&format!("    in_{} <= {{{}}};\n", p.name, chunks.join(", ")));
                conns.push(format!(".{0}(in_{0})", p.name));
            }
            Dir::Out => {
                v.push_str(&format!("  wire [{}:0] out_{};\n", p.bits - 1, p.name));
                for c in 0..p.bits.div_ceil(64) {
                    let (lo, hi) = (64 * c, (64 * c + 64).min(p.bits) - 1);
                    let width = hi - lo + 1;
                    let k = terms.len();
                    let chunk = if width == 64 {
                        format!("out_{}[{hi}:{lo}]", p.name)
                    } else {
                        format!("{{{}'d0, out_{}[{hi}:{lo}]}}", 64 - width, p.name)
                    };
                    v.push_str(&format!("  wire [63:0] fo_{k} = {chunk};\n"));
                    terms.push(rotl_verilog(&format!("fo_{k}"), (k % 64) as u32));
                }
                conns.push(format!(".{0}(out_{0})", p.name));
            }
        }
    }
    let folded = if terms.is_empty() { "64'd0".to_string() } else { terms.join(" ^ ") };
    v.push_str(&format!(
        "  (* keep_hierarchy *) {top} u_dut(\n    {}\n  );\n\
         \x20 always @(posedge clk) begin\n{refill}    if (rst) begin\n      s <= {};\n      fold <= 64'd0;\n    end else begin\n\
         \x20     s <= s_next;\n      fold <= {{fold[62:0], fold[63]}} ^ {folded};\n    end\n  end\n\
         \x20 assign sig = fold[15:0];\nendmodule\n",
        conns.join(",\n    "),
        hex64(SEED),
    ));
    v
}

/// A testbench that resets the harness for five cycles and runs it until `ops` results are folded.
/// It prints `QOR first cycles <n>` when the first result is folded, `QOR mid ops <n> cycles <n>` at
/// half the run, and `QOR ops <n> misr <hex> cycles <n>` at the end; an unknown MISR bit prints
/// `QOR X op <n>` and stops. Two watchdogs end a run that cannot finish: `max_cycles` in all
/// (`QOR TIMEOUT ops <n>`), and `max_idle` cycles without a folded result (`QOR TIMEOUT idle
/// <max_idle> cycles ops <n> cycle <n>`), which refuses a candidate that has stopped answering long
/// before the whole budget would.
pub fn emit_testbench(c: &Candidate, ops: usize, max_cycles: u64, max_idle: u64) -> String {
    emit_testbench_as(&harness_module(c), ops, max_cycles, max_idle)
}

/// [`emit_testbench`] around the harness module `m` — one emitted by [`emit_harness_as`].
pub fn emit_testbench_as(m: &str, ops: usize, max_cycles: u64, max_idle: u64) -> String {
    let mid = (ops / 2).max(1);
    format!(
        "`timescale 1ns/1ps\n\
         module tb_qor;\n\
         \x20 reg clk = 1'b0;\n\
         \x20 reg rst = 1'b1;\n\
         \x20 wire [15:0] sig;\n\
         \x20 integer cycles = 0;\n\
         \x20 integer idle = 0;\n\
         \x20 integer last_ops = 0;\n\
         \x20 reg first_seen = 1'b0;\n\
         \x20 reg mid_seen = 1'b0;\n\
         \x20 {m} u_h(.clk(clk), .rst(rst), .sig(sig));\n\
         \x20 always #5 clk = ~clk;\n\
         \x20 initial begin\n\
         \x20   repeat (5) @(posedge clk);\n\
         \x20   rst <= 1'b0;\n\
         \x20 end\n\
         \x20 always @(posedge clk) begin\n\
         \x20   if (!rst) begin\n\
         \x20     cycles = cycles + 1;\n\
         \x20     if (^u_h.misr === 1'bx || u_h.ovf === 1'b1) begin\n\
         \x20       $display(\"QOR X op %0d\", u_h.ops);\n\
         \x20       $finish;\n\
         \x20     end\n\
         \x20     if (!first_seen && u_h.ops >= 1) begin\n\
         \x20       first_seen = 1'b1;\n\
         \x20       $display(\"QOR first cycles %0d\", cycles);\n\
         \x20     end\n\
         \x20     if (!mid_seen && u_h.ops >= {mid}) begin\n\
         \x20       mid_seen = 1'b1;\n\
         \x20       $display(\"QOR mid ops %0d cycles %0d\", u_h.ops, cycles);\n\
         \x20     end\n\
         \x20     if (u_h.ops >= {ops}) begin\n\
         \x20       $display(\"QOR ops %0d misr %016h cycles %0d\", u_h.ops, u_h.misr, cycles);\n\
         \x20       $finish;\n\
         \x20     end\n\
         \x20     if (cycles > {max_cycles}) begin\n\
         \x20       $display(\"QOR TIMEOUT ops %0d\", u_h.ops);\n\
         \x20       $finish;\n\
         \x20     end\n\
         \x20     if (u_h.ops != last_ops) begin last_ops = u_h.ops; idle = 0; end\n\
         \x20     else idle = idle + 1;\n\
         \x20     if (idle >= {max_idle}) begin\n\
         \x20       $display(\"QOR TIMEOUT idle {max_idle} cycles ops %0d cycle %0d\", u_h.ops, cycles);\n\
         \x20       $finish;\n\
         \x20     end\n\
         \x20   end\n\
         \x20 end\n\
         endmodule\n",
    )
}

/// [`emit_testbench`]'s run with every folded result printed as `QOR r <op> <port hex>...` (kernel output
/// order, each port in hex at its contract width), for locating the first operation a candidate gets wrong.
pub fn emit_trace_testbench(k: &Kernel, c: &Candidate, ops: usize, max_cycles: u64, max_idle: u64) -> String {
    emit_trace_testbench_as(k, &harness_module(c), ops, max_cycles, max_idle)
}

/// [`emit_trace_testbench`] around the harness module `m` — one emitted by [`emit_harness_as`].
pub fn emit_trace_testbench_as(k: &Kernel, m: &str, ops: usize, max_cycles: u64, max_idle: u64) -> String {
    let tb = emit_testbench_as(m, ops, max_cycles, max_idle);
    let fmt: Vec<&str> = k.outputs.iter().map(|_| "%h").collect();
    let args: Vec<String> = k.outputs.iter().map(|p| format!("u_h.fr_{}", p.name)).collect();
    let trace = format!(
        "  always @(posedge clk) if (!rst && u_h.folding) $display(\"QOR r %0d {}\", u_h.ops, {});\nendmodule\n",
        fmt.join(" "),
        args.join(", ")
    );
    tb.trim_end().trim_end_matches("endmodule").to_string() + &trace
}

/// The golden results of `results` as `emit_trace_testbench` prints them: one line per operation,
/// `<op> <port hex>...`, each port in hex at its contract width.
pub fn trace_lines(k: &Kernel, results: &[Words]) -> String {
    let mut s = String::new();
    for (op, r) in results.iter().enumerate() {
        let ports: Vec<String> = k
            .outputs
            .iter()
            .zip(r)
            .map(|(p, chunks)| {
                let digits = p.bits.div_ceil(4) as usize;
                let mut hex = String::new();
                for (i, w) in chunks.iter().enumerate().rev() {
                    let bits = chunk_bits(p.bits, i);
                    hex.push_str(&format!("{:0width$x}", mask_bits(*w, bits), width = bits.div_ceil(4) as usize));
                }
                format!("{:0>digits$}", hex.trim_start_matches('0'), digits = digits)
            })
            .collect();
        s.push_str(&format!("{op} {}\n", ports.join(" ")));
    }
    s
}

/// A finished testbench run: results folded, the signature, total cycles, the cycle the first
/// result was folded on, and the (ops, cycles) mark at half the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub ops: usize,
    pub misr: u64,
    pub cycles: u64,
    pub first_cycles: Option<u64>,
    pub mid: Option<(usize, u64)>,
}

impl Run {
    /// Cycles per result in steady state: the second half of the run, so the pipeline fill is out.
    pub fn steady_cycles_per_op(&self) -> Option<f64> {
        let (mo, mc) = self.mid?;
        (self.ops > mo).then(|| (self.cycles - mc) as f64 / (self.ops - mo) as f64)
    }
}

/// The verdict a testbench printed.
pub fn parse_run(stdout: &str) -> Result<Run, QorError> {
    let num = |s: &str, line: &str| s.parse::<u64>().map_err(|_| QorError::Testbench(line.to_string()));
    let mut first_cycles = None;
    let mut mid = None;
    for line in stdout.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["QOR", "X", "op", n, ..] => {
                return Err(QorError::Unknown { op: n.parse().unwrap_or(usize::MAX) });
            }
            ["QOR", "TIMEOUT", ..] => return Err(QorError::Testbench(line.trim().to_string())),
            ["QOR", "first", "cycles", c, ..] => first_cycles = Some(num(c, line)?),
            ["QOR", "mid", "ops", n, "cycles", c, ..] => mid = Some((num(n, line)? as usize, num(c, line)?)),
            ["QOR", "ops", n, "misr", h, rest @ ..] => {
                let ops = num(n, line)? as usize;
                let misr = u64::from_str_radix(h, 16).map_err(|_| QorError::Testbench(line.to_string()))?;
                let cycles = match rest {
                    ["cycles", c, ..] => num(c, line)?,
                    _ => 0,
                };
                return Ok(Run { ops, misr, cycles, first_cycles, mid });
            }
            _ => {}
        }
    }
    Err(QorError::Testbench(format!("no QOR verdict in {} lines of output", stdout.lines().count())))
}

/// The manifest of third-party candidates.
pub const MANIFEST: &str = "benchmarks/silicon/qor/candidates.json";

/// One generated RTL file of a third-party candidate and the SHA3-256 of its bytes.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ManifestFile {
    pub path: String,
    pub sha3_256: String,
}

/// A third-party candidate: the harness-facing description, the tool (a key of
/// `scripts/qor/qor-tools.toml`), the source it read and the exact command that generated its files.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ManifestEntry {
    pub candidate: Candidate,
    pub tool: String,
    pub source: String,
    pub generate: String,
    pub files: Vec<ManifestFile>,
    pub interface_note: String,
}

/// A candidate a tool would not build — its crash, its error, or its deadline — kept in the manifest
/// so the comparison prints a refusal in that cell rather than a silent gap.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ManifestRefusal {
    pub kernel: String,
    pub name: String,
    pub tool: String,
    pub generate: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Manifest {
    pub schema: String,
    pub candidates: Vec<ManifestEntry>,
    #[serde(default)]
    pub refusals: Vec<ManifestRefusal>,
}

/// The manifest under `root`.
pub fn read_manifest(root: &std::path::Path) -> Result<Manifest, QorError> {
    let path = root.join(MANIFEST);
    let text = std::fs::read_to_string(&path).map_err(|e| QorError::Build(format!("{}: {e}", path.display())))?;
    serde_json::from_str(&text).map_err(|e| QorError::Build(format!("{}: {e}", path.display())))
}

/// The lowercase hex SHA3-256 of `bytes`.
pub fn sha3_hex(bytes: &[u8]) -> String {
    logicaffeine_system::keccak::sha3_256_bytes(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Read manifest entry `name` under `root` and its files, refusing any file whose bytes no longer
/// hash to the recorded SHA3-256.
pub fn manifest_candidate(
    root: &std::path::Path,
    name: &str,
) -> Result<(Candidate, Vec<(String, String)>), QorError> {
    let entry = read_manifest(root)?
        .candidates
        .into_iter()
        .find(|e| e.candidate.name == name)
        .ok_or_else(|| QorError::Build(format!("{name}: not in {MANIFEST}")))?;
    let mut files = Vec::new();
    for f in &entry.files {
        let p = root.join(&f.path);
        let bytes = std::fs::read(&p).map_err(|e| QorError::Build(format!("{}: {e}", p.display())))?;
        let got = sha3_hex(&bytes);
        if got != f.sha3_256 {
            return Err(QorError::Build(format!(
                "{}: SHA3-256 {got} is not the recorded {}; regenerate the candidate",
                f.path, f.sha3_256
            )));
        }
        let file_name = std::path::Path::new(&f.path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| f.path.clone());
        files.push((file_name, String::from_utf8_lossy(&bytes).to_string()));
    }
    Ok((entry.candidate, files))
}

fn dut_port(name: &str, dir: Dir, bits: u32) -> DutPort {
    DutPort { name: name.to_string(), dir, bits }
}

/// A combinational kernel module `inner` (ports `ins` → `outs`) in input and output registers:
/// the operands latch on one edge and the result on the next, a registered latency of 2. Returns
/// the module and its ports.
fn registered(top: &str, inner: &str, ins: &[(&str, u32, bool)], outs: &[(&str, u32)]) -> (String, Vec<DutPort>) {
    let decl_in: Vec<String> = ins
        .iter()
        .map(|(n, b, s)| format!("input {}[{}:0] {n}", if *s { "signed " } else { "" }, b - 1))
        .collect();
    let decl_out: Vec<String> = outs.iter().map(|(n, b)| format!("output reg [{}:0] {n}", b - 1)).collect();
    let mut v = format!(
        "// {top} — `{inner}` with its operands and results in registers (registered latency 2).\n\
         module {top}(input clk, {}, {});\n",
        decl_in.join(", "),
        decl_out.join(", ")
    );
    for (n, b, s) in ins {
        v.push_str(&format!("  reg {}[{}:0] {n}_r;\n", if *s { "signed " } else { "" }, b - 1));
    }
    for (n, b) in outs {
        v.push_str(&format!("  wire [{}:0] {n}_w;\n", b - 1));
    }
    let conns: Vec<String> = ins
        .iter()
        .map(|(n, _, _)| format!(".{n}({n}_r)"))
        .chain(outs.iter().map(|(n, _)| format!(".{n}({n}_w)")))
        .collect();
    v.push_str(&format!("  {inner} u_k({});\n  always @(posedge clk) begin\n", conns.join(", ")));
    for (n, _, _) in ins {
        v.push_str(&format!("    {n}_r <= {n};\n"));
    }
    for (n, _) in outs {
        v.push_str(&format!("    {n} <= {n}_w;\n"));
    }
    v.push_str("  end\nendmodule\n");
    let ports = std::iter::once(dut_port("clk", Dir::In, 1))
        .chain(ins.iter().map(|(n, b, _)| dut_port(n, Dir::In, *b)))
        .chain(outs.iter().map(|(n, b)| dut_port(n, Dir::Out, *b)))
        .collect();
    (v, ports)
}

/// The K6 SIMT engine compiled from the shader `a * x + b` (uniform `a`, `b`; varying `x`), four
/// predicated lanes with a cross-lane sum, under multiply strategy `arith`.
fn simt_engine(arith: LaneArith) -> SimtEngine {
    let src = format!("{}## Main\n    Show shade(2, 3, 4).\n", shade_program());
    let mut eng = SimtEngine::from_lane_source(&src, "shade", SIMT_LANES, vec![true, false, true], true, Some(AluOp::Add));
    eng.arith = arith;
    eng
}

/// The register-file candidate of the SIMT engine `eng`, named `name`: one load per engine slot
/// (uniforms, the mask, then each lane's varying), a reset before each step (the engine's load
/// counter counts from reset), and one read per lane result and the sum.
fn simt_candidate(name: &str, module: &str, eng: &SimtEngine) -> (Candidate, Vec<(String, String)>) {
    let whole = |p: &str, bits: u32| RegSlot { contract: p.to_string(), lsb: 0, width: bits };
    let load_slots = load_slots(eng)
        .into_iter()
        .map(|s| match s {
            Slot::Uniform(0) => whole("a", 64),
            Slot::Uniform(_) => whole("b", 64),
            Slot::Mask => whole("mask", SIMT_LANES as u32),
            Slot::Varying(j, _) => whole(["x0", "x1", "x2", "x3"][j], 64),
        })
        .collect();
    let read_slots = ["y0", "y1", "y2", "y3", "sum"].iter().map(|p| whole(p, 64)).collect();
    let verilog = emit_gpu_simt_verilog(module, eng);
    let top = format!("gpu_{module}");
    let c = Candidate {
        kernel: "K6".into(),
        name: name.into(),
        top: top.clone(),
        clock: "clk".into(),
        reset: Some(Reset { port: "rst".into(), active_high: true }),
        protocol: Protocol::LoadGoRead {
            load: "load".into(),
            din: "din".into(),
            go: "go".into(),
            done: "done".into(),
            raddr: "raddr".into(),
            dout: "dout".into(),
            reset_each_op: true,
            load_slots,
            read_slots,
        },
        inputs: Vec::new(),
        outputs: Vec::new(),
        mems: Vec::new(),
        batch: 1,
        ports: vec![
            dut_port("clk", Dir::In, 1),
            dut_port("rst", Dir::In, 1),
            dut_port("load", Dir::In, 1),
            dut_port("din", Dir::In, 64),
            dut_port("go", Dir::In, 1),
            dut_port("raddr", Dir::In, 8),
            dut_port("dout", Dir::Out, 64),
            dut_port("done", Dir::Out, 1),
        ],
    };
    (c, vec![(format!("{top}.v"), verilog)])
}

/// The period of one weight-stationary call: `GEMM_DIM` weight-row loads, `GEMM_DIM` activation rows,
/// then the drain until the last row has passed every column — the weights live in the PEs, so the
/// next call's loads must wait for it.
pub fn gemm_ws_period() -> u32 {
    let d = GEMM_DIM as u32;
    d + d + (d - 2)
}

/// The LOGOS weight-stationary GEMM tile with run-time weights on the K3 beat schedule of period
/// `period`: load the tile's weight rows, stream its activation rows, drain; row `t`'s result emerges
/// `pipelined_latency` cycles after its activation.
pub fn gemm_ws_candidate(period: u32) -> (Candidate, Vec<(String, String)>) {
    gemm_ws_candidate_acc(period, GEMM_ACC_BITS)
}

/// [`gemm_ws_candidate`] with `acc_bits`-wide accumulators (each sum sign-extended onto its output
/// port), named for the width when it is not the contract's.
pub fn gemm_ws_candidate_acc(period: u32, acc_bits: u32) -> (Candidate, Vec<(String, String)>) {
    let d = GEMM_DIM as u32;
    let lat = pipelined_latency(GEMM_DIM, GEMM_DIM) as u32;
    let mut drives = Vec::new();
    for t in 0..d {
        drives.push(Drive { beat: t, dut: "w_load".into(), value: DriveValue::Const(1) });
        drives.push(Drive { beat: t, dut: "w_row".into(), value: DriveValue::Const(t as u64) });
        drives.push(Drive {
            beat: t,
            dut: "w_data".into(),
            value: DriveValue::Slice { contract: "w@0".into(), lsb: d * GEMM_BITS * t, width: d * GEMM_BITS },
        });
        for r in 0..d {
            drives.push(Drive {
                beat: d + t,
                dut: format!("a{r}"),
                value: DriveValue::Slice { contract: format!("a@{t}"), lsb: GEMM_BITS * r, width: GEMM_BITS },
            });
        }
    }
    let row_bits = (usize::BITS - (GEMM_DIM.max(2) - 1).leading_zeros()).max(1);
    let mut ports = vec![
        dut_port("clk", Dir::In, 1),
        dut_port("rst", Dir::In, 1),
        dut_port("w_load", Dir::In, 1),
        dut_port("w_row", Dir::In, row_bits),
        dut_port("w_data", Dir::In, d * GEMM_BITS),
    ];
    ports.extend((0..d).map(|r| dut_port(&format!("a{r}"), Dir::In, 64)));
    ports.extend((0..d).map(|c| dut_port(&format!("o{c}"), Dir::Out, 64)));
    let outputs = (0..d)
        .flat_map(|t| (0..d).map(move |c| Bind::slice(&format!("o@{t}"), GEMM_ACC_BITS * c, GEMM_ACC_BITS, &format!("o{c}"))))
        .collect();
    let c = Candidate {
        kernel: "K3".into(),
        name: acc_name("k3_logos_ws", acc_bits),
        top: "qor_k3_ws".into(),
        clock: "clk".into(),
        reset: Some(Reset { port: "rst".into(), active_high: true }),
        protocol: Protocol::Beats { period, drives, captures: (0..d).map(|t| d + t + lat).collect() },
        inputs: Vec::new(),
        outputs,
        mems: Vec::new(),
        batch: d,
        ports,
    };
    let rtl = emit_systolic_pipelined_loadable_verilog("qor_k3_ws", GEMM_DIM, GEMM_DIM, GEMM_BITS, acc_bits);
    (c, vec![("qor_k3_ws.v".into(), rtl)])
}

/// The LOGOS output-stationary GEMM array streaming one tile every `GEMM_DIM` beats: beat `t` carries
/// lane `t` of every row's activations and weight row `t`, the first beat raises `first`, and row `i`
/// of a tile is read `os_streaming_result_offset` cycles after its first beat.
pub fn gemm_os_candidate() -> (Candidate, Vec<(String, String)>) {
    gemm_os_candidate_acc(GEMM_ACC_BITS)
}

/// [`gemm_os_candidate`] with `acc_bits`-wide accumulators (each sum sign-extended onto its output
/// port), named for the width when it is not the contract's.
pub fn gemm_os_candidate_acc(acc_bits: u32) -> (Candidate, Vec<(String, String)>) {
    let d = GEMM_DIM as u32;
    let mut drives = vec![Drive { beat: 0, dut: "first".into(), value: DriveValue::Const(1) }];
    for t in 0..d {
        for i in 0..d {
            drives.push(Drive {
                beat: t,
                dut: format!("aw{i}"),
                value: DriveValue::Slice { contract: format!("a@{i}"), lsb: GEMM_BITS * t, width: GEMM_BITS },
            });
        }
        for j in 0..d {
            drives.push(Drive {
                beat: t,
                dut: format!("wn{j}"),
                value: DriveValue::Slice { contract: "w@0".into(), lsb: d * GEMM_BITS * t + GEMM_BITS * j, width: GEMM_BITS },
            });
        }
    }
    let mut ports = vec![dut_port("clk", Dir::In, 1), dut_port("rst", Dir::In, 1), dut_port("first", Dir::In, 1)];
    ports.extend((0..d).map(|i| dut_port(&format!("aw{i}"), Dir::In, 64)));
    ports.extend((0..d).map(|j| dut_port(&format!("wn{j}"), Dir::In, 64)));
    for i in 0..d {
        ports.extend((0..d).map(|j| dut_port(&format!("res{i}_{j}"), Dir::Out, 64)));
    }
    let outputs = (0..d)
        .flat_map(|i| (0..d).map(move |j| Bind::slice(&format!("o@{i}"), GEMM_ACC_BITS * j, GEMM_ACC_BITS, &format!("res{i}_{j}"))))
        .collect();
    let captures = (0..GEMM_DIM).map(|i| os_streaming_result_offset(GEMM_DIM, i, GEMM_DIM) as u32).collect();
    let c = Candidate {
        kernel: "K3".into(),
        name: acc_name("k3_logos_os", acc_bits),
        top: "qor_k3_os".into(),
        clock: "clk".into(),
        reset: Some(Reset { port: "rst".into(), active_high: true }),
        protocol: Protocol::Beats { period: d, drives, captures },
        inputs: Vec::new(),
        outputs,
        mems: Vec::new(),
        batch: d,
        ports,
    };
    let rtl = emit_systolic_os_streaming_verilog("qor_k3_os", &SystolicOs::new(GEMM_DIM, GEMM_DIM, GEMM_DIM), GEMM_BITS, acc_bits);
    (c, vec![("qor_k3_os.v".into(), rtl)])
}

/// The streamed K6 engine's register placement: a register after every `K6_STREAMED_EVERY`-th level of its multiply
/// tree (`LaneArith::Streamed`). MEASURED (2026-10-17, Artix-7 fabric map, yosys 0.33 synth_xilinx + nextpnr-xilinx,
/// median of the seeds that routed): every level = 3421 LUTs at 128.2 MHz and 24 clocks an operation, 1.56 million
/// results a second per thousand LUTs; every second = 3403 at 106.6 MHz and 21 clocks, 1.49; every third = 3394 at
/// 90.7 and 20, 1.34; the whole tree in one stage = 3387 at 63.7 and 19, 0.99. Fewer registers hold half (every
/// second) to a quarter (one stage) of the flip-flops.
pub const K6_STREAMED_EVERY: usize = 1;

/// The K6 ports engine's register placement: a register after every `K6_PORTS_EVERY`-th level of each lane's multiply
/// tree and after its last (`simt_ports`). MEASURED (2026-10-18, Sky130 HD: yosys 0.33 with OpenLane v1's `DELAY 0` ABC
/// script at a 2 ns target, OpenSTA at the typical corner, no wires; a step every clock throughout): every level =
/// 536,394 µm² at 3.35 ns and 8 clocks; every second = 488,462 at 3.86 and 5; every third = 468,797 at 5.10 and 4; the
/// product only = 437,985 at 8.30 and 3; no register in the tree = 423,320 at 9.27 and 2. Every second is within 5% of
/// every level in steps a second per area, with 39% of its flip-flops and three clocks fewer.
pub const K6_PORTS_EVERY: usize = 2;

/// The SIMT engine `eng`'s step on ports (`simt_ports`), named `name`: every operand from the harness's registers at
/// once, each lane's product a tree registered every [`K6_PORTS_EVERY`] levels, every lane's output and the sum
/// registered together, a step every clock.
fn simt_ports_candidate(name: &str, module: &str, eng: &SimtEngine) -> Result<(Candidate, Vec<(String, String)>), QorError> {
    let levels = mul_tree_levels(64);
    let p = emit_simt_ports_verilog(module, eng, |l| (l + 1) % K6_PORTS_EVERY == 0 || l + 1 == levels).map_err(QorError::Build)?;
    let lanes = ["x0", "x1", "x2", "x3"];
    let results = ["y0", "y1", "y2", "y3"];
    let mut ports = vec![
        dut_port("clk", Dir::In, 1),
        dut_port("a", Dir::In, 64),
        dut_port("b", Dir::In, 64),
        dut_port("mask", Dir::In, SIMT_LANES as u32),
    ];
    ports.extend(lanes.iter().map(|x| dut_port(x, Dir::In, 64)));
    ports.extend(results.iter().map(|y| dut_port(y, Dir::Out, 64)));
    ports.push(dut_port("reduced", Dir::Out, 64));
    let mut inputs = vec![Bind::whole("a", "a"), Bind::whole("b", "b"), Bind::whole("mask", "mask")];
    inputs.extend(lanes.iter().map(|x| Bind::whole(x, x)));
    let mut outputs: Vec<Bind> = results.iter().map(|y| Bind::whole(y, y)).collect();
    outputs.push(Bind::whole("sum", "reduced"));
    let c = Candidate {
        kernel: "K6".into(),
        name: name.into(),
        top: module.into(),
        clock: "clk".into(),
        reset: None,
        protocol: Protocol::Registered { latency: p.latency },
        inputs,
        outputs,
        mems: Vec::new(),
        batch: 1,
        ports,
    };
    Ok((c, vec![(format!("{module}.v"), p.verilog)]))
}

/// Every LOGOS candidate of kernel `id`, the default first. K1 has the decision fabric in registers, sequenced one
/// cell a cycle, pipelined one carry chain to a stage for a fabric with carry chains (`fabric_pipe::decision_ops`,
/// `DECISION_CHAIN_BUDGET`), pipelined for standard cells (each compare's sums folded into it, `DECISION_CELLS_BUDGET`,
/// its ports read), and hoisted into one clock (`select_hoist`, its sums folded into the compares that read them, its
/// ports read). K4 has the Keccak core
/// a round a clock and a half round a clock (`keccak_core_half`). K2 has the residual in registers, sequenced one cell
/// a cycle, and pipelined (`fabric_pipe`, the RTL the `fabric_horner_seq` block hardens). K5 has the butterfly in
/// registers and its pipeline registered once a multiply and after every step. K6 has one per lane multiply strategy:
/// the sequential shift-add (the signoff-clean default), the two-stage decomposed FMA, the single-cycle
/// combinational multiply and the streamed shared multiply, and the engine's step on ports (`simt_ports`).
pub fn logos_candidates(id: &str) -> Result<Vec<(Candidate, Vec<(String, String)>)>, QorError> {
    match id {
        "K3" => Ok(vec![
            gemm_ws_candidate(gemm_ws_period()),
            gemm_os_candidate(),
            gemm_ws_candidate_acc(gemm_ws_period(), GEMM_MATCHED_ACC_BITS),
            gemm_os_candidate_acc(GEMM_MATCHED_ACC_BITS),
        ]),
        "K1" => {
            let k = kernel("K1").ok_or_else(|| QorError::Build("K1 is not in the catalog".into()))?;
            let df = decision_fabric_from_source(DECIDE_FN, DECIDE_NAME);
            let inputs: Vec<Bind> = k.inputs.iter().enumerate().map(|(i, p)| Bind::whole(p.name, &format!("in{i}"))).collect();
            let (ops, result) = decision_ops(&df).map_err(QorError::Build)?;
            let sched = schedule_pipeline(&ops, result, DECISION_CHAIN_BUDGET);
            let (folded, fores) = carry_save_lower_with(&ops, result, 64, SumSharing::Fold, FinalAdd::Native);
            let cells = schedule_pipeline(&folded, fores, DECISION_CELLS_BUDGET).reading(Inputs::Ports);
            let (hops, hres) = decision_ops(&hoist_selects(&df, 64).map_err(QorError::Build)?).map_err(QorError::Build)?;
            let (flat, fres) = carry_save_lower_with(&hops, hres, 64, SumSharing::Fold, FinalAdd::PrefixTree);
            let one = schedule_pipeline(&flat, fres, f64::INFINITY).reading(Inputs::Ports);
            Ok(vec![
                logos_candidate("K1")?,
                seq_fabric_candidate("K1", "hft", &df, inputs.clone(), "order"),
                residual_pipe_candidate("K1", "k1_logos_pipe", "hft_pipe", &ops, &sched, inputs.clone(), "order"),
                residual_pipe_candidate("K1", "k1_logos_cells", "hft_cells", &folded, &cells, inputs.clone(), "order"),
                residual_pipe_candidate("K1", "k1_logos_hoisted", "hft_hoisted", &flat, &one, inputs, "order"),
            ])
        }
        "K2" => {
            let df = decision_fabric_from_source(&horner_program(), "poly");
            let (ops, sched) = horner_seq_pipeline();
            Ok(vec![
                logos_candidate("K2")?,
                seq_fabric_candidate("K2", "horner", &df, vec![Bind::whole("x", "in0")], "y"),
                residual_pipe_candidate("K2", "k2_logos_pipe", "horner_seq", &ops, &sched, vec![Bind::whole("x", "in0")], "y"),
            ])
        }
        "K4" => {
            let (core, files) = logos_candidate("K4")?;
            let mut half = core.clone();
            half.name = "k4_logos_half".into();
            half.top = "keccak_core_half".into();
            Ok(vec![(core, files), (half, vec![("keccak_core_half.v".into(), emit_keccak_core_half_verilog())])])
        }
        "K5" => Ok(vec![
            logos_candidate("K5")?,
            ntt_butterfly_pipe_candidate("k5_logos_pipe", "ntt_butterfly_pipe", NttCuts::PER_MULTIPLY),
            ntt_butterfly_pipe_candidate("k5_logos_deep", "ntt_butterfly_deep", NttCuts::EVERY_STEP),
        ]),
        "K6" => Ok(vec![
            simt_candidate("k6_logos_seqfma", "qor_k6_seqfma", &simt_engine(LaneArith::SequentialFma { select_slice: 8 })),
            simt_candidate("k6_logos_fma2", "qor_k6_fma2", &simt_engine(LaneArith::DecomposedFma { stages: 2 })),
            simt_candidate("k6_logos_comb", "qor_k6_comb", &simt_engine(LaneArith::Combinational)),
            simt_candidate("k6_logos_streamed", "qor_k6_streamed", &simt_engine(LaneArith::Streamed { every: K6_STREAMED_EVERY })),
            simt_ports_candidate("k6_logos_ports", "qor_k6_ports", &simt_engine(LaneArith::Combinational))?,
        ]),
        other => Ok(vec![logos_candidate(other)?]),
    }
}

/// A K5 pipelined butterfly (`emit_ntt_butterfly_pipelined_verilog`) registered after the steps `cuts` names: its
/// own operand and result registers, one butterfly a cycle.
fn ntt_butterfly_pipe_candidate(name: &str, top: &str, cuts: NttCuts) -> (Candidate, Vec<(String, String)>) {
    let ports = vec![
        dut_port("clk", Dir::In, 1),
        dut_port("a", Dir::In, 16),
        dut_port("b", Dir::In, 16),
        dut_port("zeta", Dir::In, 16),
        dut_port("hi", Dir::Out, 16),
        dut_port("lo", Dir::Out, 16),
    ];
    let c = Candidate {
        kernel: "K5".to_string(),
        name: name.to_string(),
        top: top.to_string(),
        clock: "clk".to_string(),
        reset: None,
        protocol: Protocol::Registered { latency: cuts.latency() },
        inputs: vec![Bind::whole("a", "a"), Bind::whole("b", "b"), Bind::whole("zeta", "zeta")],
        outputs: vec![Bind::whole("hi", "hi"), Bind::whole("lo", "lo")],
        mems: Vec::new(),
        batch: 1,
        ports,
    };
    (c, vec![(format!("{top}.v"), emit_ntt_butterfly_pipelined_verilog(top, cuts))])
}

/// A compiled residual pipelined by `fabric_pipe` (`emit_fabric_pipelined_verilog`): module `fabric_{module}`, its
/// inputs `in0..` registered into the first stage, one result a cycle on `result`, `sched.latency()` edges after its
/// operands.
fn residual_pipe_candidate(
    id: &str,
    name: &str,
    module: &str,
    ops: &[FabricOp],
    sched: &PipelineSchedule,
    inputs: Vec<Bind>,
    output: &str,
) -> (Candidate, Vec<(String, String)>) {
    let top = format!("fabric_{module}");
    let n_in = inputs.len();
    let mut ports = vec![dut_port("clk", Dir::In, 1)];
    ports.extend((0..n_in).map(|k| dut_port(&format!("in{k}"), Dir::In, 64)));
    ports.push(dut_port("result", Dir::Out, 64));
    let c = Candidate {
        kernel: id.to_string(),
        name: name.to_string(),
        top: top.clone(),
        clock: "clk".to_string(),
        reset: None,
        protocol: Protocol::Registered { latency: sched.latency() as u32 },
        inputs,
        outputs: vec![Bind::whole(output, "result")],
        mems: Vec::new(),
        batch: 1,
        ports,
    };
    (c, vec![(format!("{top}.v"), emit_fabric_pipelined_verilog(module, n_in, ops, sched))])
}

/// The sequenced form of a compiled fabric (`emit_seq_datapath_verilog`): the same cells, one a cycle — its reset
/// starts an operation, `done` ends it, and the harness holds the operands meanwhile.
fn seq_fabric_candidate(
    id: &str,
    short: &str,
    df: &DecisionFabric,
    inputs: Vec<Bind>,
    output: &str,
) -> (Candidate, Vec<(String, String)>) {
    let top = format!("seq_{short}");
    let mut ports = vec![dut_port("clk", Dir::In, 1), dut_port("rst", Dir::In, 1)];
    ports.extend((0..df.n_in).map(|k| dut_port(&format!("in{k}"), Dir::In, 64)));
    ports.push(dut_port("result", Dir::Out, 64));
    ports.push(dut_port("done", Dir::Out, 1));
    let c = Candidate {
        kernel: id.to_string(),
        name: format!("{}_logos_seq", id.to_lowercase()),
        top: top.clone(),
        clock: "clk".to_string(),
        reset: None,
        protocol: Protocol::StartDone { start: "rst".into(), done: "done".into() },
        inputs,
        outputs: vec![Bind::whole(output, "result")],
        mems: Vec::new(),
        batch: 1,
        ports,
    };
    (c, vec![(format!("{top}.v"), emit_seq_datapath_verilog(short, df))])
}

/// The LOGOS candidate of kernel `id`: its harness-facing description and its Verilog files
/// (file name, contents). K1, K2 and K6 are compiled from their LOGOS programs; K4 and K5 come from
/// the RTL generators; the remaining kernels' candidates are built by their own protocols.
pub fn logos_candidate(id: &str) -> Result<(Candidate, Vec<(String, String)>), QorError> {
    let base = |name: &str, top: &str, protocol: Protocol, inputs: Vec<Bind>, outputs: Vec<Bind>, ports: Vec<DutPort>| Candidate {
        kernel: id.to_string(),
        name: name.to_string(),
        top: top.to_string(),
        clock: "clk".to_string(),
        reset: None,
        protocol,
        inputs,
        outputs,
        mems: Vec::new(),
        batch: 1,
        ports,
    };
    match id {
        "K1" => {
            let df = decision_fabric_from_source(DECIDE_FN, DECIDE_NAME);
            let comb = emit_decision_fabric_verilog("hft", &df);
            let ins: Vec<String> = (0..df.n_in).map(|k| format!("in{k}")).collect();
            let decl: Vec<(&str, u32, bool)> = ins.iter().map(|n| (n.as_str(), 64, true)).collect();
            let (wrap, ports) = registered("qor_k1_logos", "decision_hft", &decl, &[("result", 64)]);
            let kernel = kernel("K1").ok_or_else(|| QorError::Build("K1 is not in the catalog".into()))?;
            let inputs = kernel
                .inputs
                .iter()
                .zip(&ins)
                .map(|(p, d)| Bind::whole(p.name, d))
                .collect();
            Ok((
                base("k1_logos", "qor_k1_logos", Protocol::Registered { latency: 2 }, inputs, vec![Bind::whole("order", "result")], ports),
                vec![("decision_hft.v".into(), comb), ("qor_k1_logos.v".into(), wrap)],
            ))
        }
        "K2" => {
            let (ops, result, n_in) = residual_from_source(&horner_program(), "poly");
            if n_in != 1 {
                return Err(QorError::Build(format!("the Horner residual takes {n_in} inputs, not 1")));
            }
            let comb = emit_fabric_verilog("horner", n_in, &ops, result);
            let (wrap, ports) = registered("qor_k2_logos", "fabric_horner", &[("in0", 64, true)], &[("result", 64)]);
            Ok((
                base("k2_logos", "qor_k2_logos", Protocol::Registered { latency: 2 }, vec![Bind::whole("x", "in0")], vec![Bind::whole("y", "result")], ports),
                vec![("fabric_horner.v".into(), comb), ("qor_k2_logos.v".into(), wrap)],
            ))
        }
        "K4" => {
            let mut c = base(
                "k4_logos",
                "keccak_core",
                Protocol::StartDone { start: "start".into(), done: "done".into() },
                vec![Bind::whole("s_in", "s_in")],
                vec![Bind::whole("s_out", "s_out")],
                vec![
                    dut_port("clk", Dir::In, 1),
                    dut_port("rst", Dir::In, 1),
                    dut_port("start", Dir::In, 1),
                    dut_port("s_in", Dir::In, 1600),
                    dut_port("done", Dir::Out, 1),
                    dut_port("s_out", Dir::Out, 1600),
                ],
            );
            c.reset = Some(Reset { port: "rst".into(), active_high: true });
            Ok((c, vec![("keccak_core.v".into(), emit_keccak_core_verilog())]))
        }
        "K5" => {
            let (wrap, ports) = registered(
                "qor_k5_logos",
                "ntt_butterfly",
                &[("a", 16, true), ("b", 16, true), ("zeta", 16, true)],
                &[("hi", 16), ("lo", 16)],
            );
            Ok((
                base(
                    "k5_logos",
                    "qor_k5_logos",
                    Protocol::Registered { latency: 2 },
                    vec![Bind::whole("a", "a"), Bind::whole("b", "b"), Bind::whole("zeta", "zeta")],
                    vec![Bind::whole("hi", "hi"), Bind::whole("lo", "lo")],
                    ports,
                ),
                vec![("ntt_butterfly.v".into(), emit_ntt_butterfly_verilog()), ("qor_k5_logos.v".into(), wrap)],
            ))
        }
        "K3" | "K6" => Ok(logos_candidates(id)?.remove(0)),
        other => Err(QorError::Build(format!("{other}: the LOGOS candidate is built by its own protocol"))),
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/qor_harness/tests.rs"]
mod tests;
