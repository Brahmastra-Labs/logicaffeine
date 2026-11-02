//! The hard floating-point oracle: a D5005 persona that measures which binary32 semantics the card's floating-point DSP
//! block computes (`fourteennm_fp_mac`, whose simulation model is encrypted), by running the block beside every class
//! of the hypothesis family ([`super::fp_hypotheses`]) as soft units on one stream of operands and counting, class by
//! class, the results that differ from the block's.
//!
//! The persona is an engine build whose engine is the oracle. [`Oracle::engine_verilog`] writes a `logos_engine` with
//! the engine's ports, the engine's own crossing ([`bridge_verilog`]) and the oracle's core as its `core` instance, so
//! the shell, its commands and the timing constraints an engine build carries hold for it unchanged: the core steps at
//! the compute clock's rising edges, every path inside it given half a compute cycle.
//!
//! Each step the core takes one operand triple (A, B, C) from its [`Source`] and hands it to three hard blocks: the
//! multiply A·B (`sp_mult`, ay·az), the add A+B (`sp_add`, ax+ay) and the multiply-add C+A·B (`sp_mult_add`,
//! ax+ay·az), every register of each on the compute step. The triple also enters a delay line the soft units read
//! [`Run::tap`] + 1 steps late, and each block's results wait [`Run::delays`] steps more, so a block of latency `L`
//! given `tap + 1 - L` meets the soft units on its own operands, whatever the three blocks' latencies. The host
//! measures them first: the multiply's and the add's as the taps at which their classes come nearest every vector
//! ([`Source::Vectors`]; a block no class names still has its tap), the multiply-add's as the shift at which its results with C zero are the products. Every step
//! the core adds one to the counter of each class whose result differs from the block's, and counts the steps no
//! product class or no sum class explains; a recording run ([`Run::record`]) stores every step ([`Record`]), a
//! counting run the steps nothing explains, up to [`Oracle::record_cap`]. Beside them a multiply and an add with only
//! their output registered take the same triples, their results recorded as they leave them: the shift at which they
//! are the aligned blocks' is their latency, the fewest steps a block of each mode can be.
//!
//! The host's commands: `ENGINE_TABLE` into table 0 at entry 0, the stream's seed in bits 63:0 of the line;
//! `ENGINE_RUN`, ARG0 the [`Run`] word and ARG1 the triples to take; `STORE_READ`, the store of [`STORE_BEATS`]
//! beats ([`Store::read`]): beat 0 the header, then the counters eight to a beat (the product classes', then the sum
//! classes'), then the records. [`HOST_PROGRAM`] runs the whole measurement and prints it as one JSON object.

use std::collections::HashMap;
use std::fmt::{self, Write};

use super::fp_hypotheses::{fadd_verilog, family, fmul_verilog, Semantics};
use crate::crypto_accel::sha256;
use crate::spatial::decode_engine::ofs::{afu_sources, AfuConfig, AfuFile, LocalMemoryLine};
use crate::spatial::decode_engine::rtl::engine::{bridge_verilog, EngineParams};
use crate::spatial::decode_engine::rtl::ram::ram_ce_verilog;

/// The header's first word: `LOGOSFPO`.
pub const MAGIC: u64 = 0x4f50_4653_4f47_4f4c;

/// The latencies the soft side can be delayed to: a tap of 0 to `TAPS - 1`, a block latency of 1 to `TAPS` steps.
pub const TAPS: usize = 16;

/// The store's beats, 512 bits each.
pub const STORE_BEATS: usize = 4096;

/// Steps a run takes past its last triple, so every result of every triple leaves the delay line.
pub const DRAIN: usize = TAPS + 1;

/// Where a run's operands come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The discriminator vectors and the multiply-add's, in the oracle's ROM, taken in turn from the first.
    Vectors,
    /// Three words of a xorshift stream from the seed.
    Random,
    /// The stream drawn where the semantics part: a's exponent zero, one, two, the top two or any; then, half the
    /// time, b's placing the product within a few binades of the least normal; a quarter, b's a's or within a few of
    /// it, its significand a's but for the low byte and its sign the other, so the sum cancels deep; else b's exponent
    /// any or an end.
    Near,
}

/// A run's word (`ENGINE_RUN`'s ARG0): its source in bits 1:0, whether it records every step in bit 2, the soft
/// units' tap in bits 7:4, and the steps each hard block's results wait before they meet the soft units: the
/// multiply's in bits 11:8, the add's in 15:12, the multiply-add's in 19:16.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub source: Source,
    pub record: bool,
    pub tap: u8,
    pub delays: [u8; 3],
}

impl Run {
    pub fn word(&self) -> u32 {
        let source = match self.source {
            Source::Vectors => 0,
            Source::Random => 1,
            Source::Near => 2,
        };
        let [m, a, f] = self.delays.map(|d| u32::from(d) & 0xf);
        source | (self.record as u32) << 2 | (u32::from(self.tap) & 0xf) << 4 | m << 8 | a << 12 | f << 16
    }

    /// The run of a soft tap that meets blocks of `latencies` (the multiply's, the add's, the multiply-add's):
    /// the least tap all three reach, each block's results delayed the rest of the way. `None` for a latency
    /// outside 1 to [`TAPS`].
    pub fn aligned(source: Source, record: bool, latencies: [usize; 3]) -> Option<Run> {
        if latencies.iter().any(|l| !(1..=TAPS).contains(l)) {
            return None;
        }
        let tap = latencies.iter().max().expect("three") - 1;
        Some(Run { source, record, tap: tap as u8, delays: latencies.map(|l| (tap + 1 - l) as u8) })
    }

    pub fn of(word: u32) -> Option<Run> {
        let source = match word & 3 {
            0 => Source::Vectors,
            1 => Source::Random,
            2 => Source::Near,
            _ => return None,
        };
        let nibble = |at: u32| (word >> at & 0xf) as u8;
        (word >> 20 == 0 && word & 8 == 0).then_some(Run { source, record: word & 4 != 0, tap: nibble(4), delays: [nibble(8), nibble(12), nibble(16)] })
    }
}

/// A class of the family: its members' names (all one function, proven), and the semantics of the first, which the
/// oracle builds.
#[derive(Clone, Debug, PartialEq)]
pub struct Class {
    pub members: Vec<String>,
    pub semantics: Semantics,
}

/// What the oracle is built from: the classes of the multiply and of the add, and the vectors its ROM holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Oracle {
    pub products: Vec<Class>,
    pub sums: Vec<Class>,
    pub vectors: Vec<[u32; 3]>,
}

/// Why a discriminator ledger cannot make an oracle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OracleError {
    Json(String),
    /// The ledger lacks the multiply's or the add's table.
    Missing(&'static str),
    /// A member name no semantics of the family has.
    Unknown(String),
    /// A word that is not eight hex digits.
    Word(String),
    /// More classes than the records' agreement bits hold, or more vectors than the ROM's index reaches.
    TooMany(&'static str, usize),
}

impl fmt::Display for OracleError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            OracleError::Json(e) => write!(f, "the discriminator ledger: {e}"),
            OracleError::Missing(op) => write!(f, "the discriminator ledger has no {op} table"),
            OracleError::Unknown(m) => write!(f, "no member of the family is named {m}"),
            OracleError::Word(w) => write!(f, "not a binary32 word: {w}"),
            OracleError::TooMany(what, n) => write!(f, "{n} {what}, more than the oracle holds"),
        }
    }
}

#[derive(serde::Deserialize)]
struct LedgerClass {
    members: Vec<String>,
}

#[derive(serde::Deserialize)]
struct LedgerOp {
    op: String,
    vectors: Vec<[String; 2]>,
    classes: Vec<LedgerClass>,
}

/// The most product classes a record's agreement bits hold: the two words it gives the multiply's agreement, the
/// least-registered multiply's result beside the flags.
pub const MAX_PRODUCT_CLASSES: usize = 128;

/// The most sum classes a record's agreement bits hold: three halves of the two words it gives the add, the fourth the
/// least-registered add's result.
pub const MAX_SUM_CLASSES: usize = 96;

/// The multiply-add's own vectors: A and B in [1, 2) whose product is inexact, C the product rounded and negated, so a
/// fused multiply-add gives the product's rounding error and one rounding after another gives zero; then C a power of
/// two beside the product.
pub fn fma_vectors() -> Vec<[u32; 3]> {
    let mut state = 0x243f_6a88_85a3_08d3u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut out = Vec::new();
    while out.len() < 64 {
        let r = next();
        let (a, b) = (0x3f80_0000 | (r as u32 & 0x7f_ffff), 0x3f80_0000 | ((r >> 32) as u32 & 0x7f_ffff));
        let p = f32::from_bits(a) * f32::from_bits(b);
        if f64::from(p) == f64::from(f32::from_bits(a)) * f64::from(f32::from_bits(b)) {
            continue;
        }
        let c = if out.len() % 4 == 3 { (f32::from_bits(0x4000_0000) * if r & 1 == 0 { 1.0 } else { -1.0 }).to_bits() } else { (-p).to_bits() };
        out.push([a, b, c]);
    }
    out
}

impl Oracle {
    /// The oracle `benchmarks/silicon/d5005/fp_discriminators.json` describes: each class's first member, the
    /// multiply's and the add's vectors with C zero, then [`fma_vectors`].
    pub fn from_ledger(json: &str) -> Result<Oracle, OracleError> {
        let ops: Vec<LedgerOp> = serde_json::from_str(json).map_err(|e| OracleError::Json(e.to_string()))?;
        let named: HashMap<String, Semantics> = family().into_iter().map(|s| (s.name(), s)).collect();
        let op = |name: &'static str| ops.iter().find(|o| o.op == name).ok_or(OracleError::Missing(name));
        let classes = |o: &LedgerOp| -> Result<Vec<Class>, OracleError> {
            o.classes
                .iter()
                .map(|c| {
                    let first = c.members.first().ok_or(OracleError::Missing("member"))?;
                    let semantics = *named.get(first).ok_or_else(|| OracleError::Unknown(first.clone()))?;
                    for m in &c.members {
                        named.get(m).ok_or_else(|| OracleError::Unknown(m.clone()))?;
                    }
                    Ok(Class { members: c.members.clone(), semantics })
                })
                .collect()
        };
        let word = |w: &String| u32::from_str_radix(w, 16).ok().filter(|_| w.len() == 8).ok_or_else(|| OracleError::Word(w.clone()));
        let (mul, add) = (op("fmul")?, op("fadd")?);
        let mut vectors = Vec::new();
        for [a, b] in mul.vectors.iter().chain(&add.vectors) {
            vectors.push([word(a)?, word(b)?, 0]);
        }
        vectors.extend(fma_vectors());
        let oracle = Oracle { products: classes(mul)?, sums: classes(add)?, vectors };
        for (what, n, most) in [("product classes", oracle.products.len(), MAX_PRODUCT_CLASSES), ("sum classes", oracle.sums.len(), MAX_SUM_CLASSES)] {
            if n == 0 || n > most {
                return Err(OracleError::TooMany(what, n));
            }
        }
        if oracle.vectors.len() > 1 << 16 {
            return Err(OracleError::TooMany("vectors", oracle.vectors.len()));
        }
        Ok(oracle)
    }

    /// The store's beats of counters, eight to a beat.
    pub fn counter_beats(&self) -> usize {
        (self.products.len() + self.sums.len()).div_ceil(8)
    }

    /// The store's first record beat.
    pub fn records_at(&self) -> usize {
        1 + self.counter_beats()
    }

    /// The most records a run stores.
    pub fn record_cap(&self) -> usize {
        STORE_BEATS - self.records_at()
    }

    /// Bits of the vector ROM's index.
    fn index_bits(&self) -> usize {
        (usize::BITS - (self.vectors.len().max(2) - 1).leading_zeros()) as usize
    }

    /// The oracle's core (`logos_fp_oracle`) with its soft classes, its vector ROM and its store: every module the
    /// engine top instantiates but the crossing and the hard block.
    pub fn core_verilog(&self) -> String {
        let mut v = self.units_verilog();
        v.push_str(&self.rom_verilog());
        v.push_str(&ram_ce_verilog(&[512]));
        v.push_str(&self.core_text());
        v
    }

    /// The modules the oracle writes itself, which say what the card measures and how its record reads: its classes'
    /// units, its vector ROM and delay line, its core. The rest of [`Oracle::engine_verilog`] is the library's it
    /// embeds: the engine's crossing, the store's memory, and the persona's engine top over the engine's ports.
    pub fn own_verilog(&self) -> String {
        let mut v = self.units_verilog();
        v.push_str(&self.rom_verilog());
        v.push_str(&self.core_text());
        v
    }

    /// The soft classes' units: each product class's multiply, each sum class's add.
    fn units_verilog(&self) -> String {
        let mut v = String::new();
        for (k, c) in self.products.iter().enumerate() {
            v.push_str(&fmul_verilog(&format!("logos_fpo_mul{k}"), &c.semantics));
        }
        for (k, c) in self.sums.iter().enumerate() {
            v.push_str(&fadd_verilog(&format!("logos_fpo_add{k}"), &c.semantics));
        }
        v
    }

    /// The persona's `logos_engine`: the core behind the engine's crossing, with every module it is built of but the
    /// hard block, which Quartus places.
    pub fn engine_verilog(&self) -> String {
        format!("{}\n{}\n{}", bridge_verilog(), self.core_verilog(), top_text())
    }

    fn rom_verilog(&self) -> String {
        let ib = self.index_bits();
        let mut v = format!(
            "// The oracle's vectors, (C, B, A) a word: the multiply's and the add's discriminators, then the multiply-add's.\n\
             module logos_fpo_vectors(input [{}:0] i, output reg [95:0] v);\n  always @* case (i)\n",
            ib - 1
        );
        for (k, [a, b, c]) in self.vectors.iter().enumerate() {
            writeln!(v, "    {ib}'d{k}: v = 96'h{c:08x}_{b:08x}_{a:08x};").unwrap();
        }
        v.push_str("    default: v = 96'd0;\n  endcase\nendmodule\n\n");
        v.push_str(LATE);
        v
    }

    fn core_text(&self) -> String {
        let (nm, na) = (self.products.len(), self.sums.len());
        let ib = self.index_bits();
        let mut units = String::new();
        for k in 0..nm {
            writeln!(units, "  wire [31:0] pm{k};\n  logos_fpo_mul{k} um{k}(.a(t[31:0]), .b(t[63:32]), .y(pm{k}));").unwrap();
        }
        for k in 0..na {
            writeln!(units, "  wire [31:0] pa{k};\n  logos_fpo_add{k} ua{k}(.a(t[31:0]), .b(t[63:32]), .y(pa{k}));").unwrap();
        }
        let agree = |n: usize, w: &str, hard: &str| (0..n).rev().map(|k| format!("{w}{k} == {hard}")).collect::<Vec<_>>().join(", ");
        let mut counters = String::new();
        let mut clear = String::new();
        let mut bump = String::new();
        for k in 0..nm {
            writeln!(counters, "  reg [47:0] cm{k};").unwrap();
            write!(clear, " cm{k} <= 48'd0;").unwrap();
            writeln!(bump, "          if (!agree_m[{k}]) cm{k} <= cm{k} + 48'd1;").unwrap();
        }
        for k in 0..na {
            writeln!(counters, "  reg [47:0] ca{k};").unwrap();
            write!(clear, " ca{k} <= 48'd0;").unwrap();
            writeln!(bump, "          if (!agree_a[{k}]) ca{k} <= ca{k} + 48'd1;").unwrap();
        }
        let all: Vec<String> = (0..nm).map(|k| format!("cm{k}")).chain((0..na).map(|k| format!("ca{k}"))).collect();
        let mut dump = String::new();
        for (b, chunk) in all.chunks(8).enumerate() {
            let words: Vec<String> = (0..8).rev().map(|i| chunk.get(i).map_or("64'd0".to_string(), |c| format!("{{16'd0, {c}}}"))).collect();
            writeln!(dump, "      {}: dump_word = {{{}}};", b + 1, words.join(", ")).unwrap();
        }
        CORE.replace("@NM@", &nm.to_string())
            .replace("@NA@", &na.to_string())
            .replace("@NV@", &self.vectors.len().to_string())
            .replace("@IB@", &ib.to_string())
            .replace("@IBM@", &(ib - 1).to_string())
            .replace("@BEATS@", &STORE_BEATS.to_string())
            .replace("@AB@", &(STORE_BEATS.trailing_zeros()).to_string())
            .replace("@RECORDS@", &self.records_at().to_string())
            .replace("@RECORDS_M1@", &(self.records_at() - 1).to_string())
            .replace("@NVM1@", &(self.vectors.len() - 1).to_string())
            .replace("@CAP@", &self.record_cap().to_string())
            .replace("@TAPS@", &TAPS.to_string())
            .replace("@DRAIN@", &DRAIN.to_string())
            .replace("@MAGIC@", &format!("64'h{MAGIC:016x}"))
            .replace("@HARD@", &hard_blocks())
            .replace("@UNITS@", &units)
            .replace("@AGREE_M@", &agree(nm, "pm", "hm"))
            .replace("@AGREE_A@", &agree(na, "pa", "ha"))
            .replace("@COUNTERS@", &counters)
            .replace("@CLEAR@", &clear)
            .replace("@BUMP@", &bump)
            .replace("@DUMP@", &dump)
    }
}

/// The block's clock parameters, in the order the FP DSP IP (`altera_fpdsp_block`) sets them.
const CLOCKS: [&str; 13] = [
    "accumulate_clock",
    "ax_clock",
    "ay_clock",
    "az_clock",
    "accum_pipeline_clock",
    "ax_chainin_pl_clock",
    "mult_pipeline_clock",
    "accum_2nd_pipeline_clock",
    "ax_chainin_2nd_pl_clock",
    "mult_2nd_pipeline_clock",
    "accum_adder_clock",
    "adder_input_clock",
    "output_clock",
];

/// The registers each mode uses fully registered, every operand the mode reads crossing as many to the adder: the
/// multiply's and the multiply-add's their input register and two pipeline registers (AX the chain-in pipeline's, AY
/// and AZ the multiplier's), the add's its input register alone, Quartus allowing an add no multiplier pipeline.
pub const MULTIPLY_REGISTERS: &[&str] = &["ay_clock", "az_clock", "mult_pipeline_clock", "mult_2nd_pipeline_clock", "output_clock"];
pub const ADD_REGISTERS: &[&str] = &["ax_clock", "ay_clock", "adder_input_clock", "output_clock"];
pub const MULTIPLY_ADD_REGISTERS: &[&str] = &[
    "ax_clock",
    "ay_clock",
    "az_clock",
    "ax_chainin_pl_clock",
    "mult_pipeline_clock",
    "ax_chainin_2nd_pl_clock",
    "mult_2nd_pipeline_clock",
    "adder_input_clock",
    "output_clock",
];

/// The register a least-registered block keeps: its output's.
pub const LEAST_REGISTERS: &[&str] = &["output_clock"];

/// One `fourteennm_fp_mac` named `name` in `mode` (`sp_mult`, `sp_add`, `sp_mult_add`), the registers of `used` on
/// clock 0 enabled by `enable` and every other `NONE`, the operands `[ax, ay, az]` (only those its mode reads connected,
/// [`MODE_OPERANDS`], and no internal-use port), the result `y` and the eight flags
/// `flags` (the multiply's then the add's {inexact, invalid, overflow, underflow}); with the wires its chain-out and its
/// test output drive, `{name}_chain` and `{name}_dft`, which nothing reads.
pub(crate) fn fp_mac(mode: &str, name: &str, used: &[&str], enable: &str, operands: [&str; 3], y: &str, flags: &str) -> String {
    let params: Vec<String> = CLOCKS.iter().map(|c| format!(".{c}(\"{}\")", if used.contains(c) { "0" } else { "NONE" })).collect();
    let read = MODE_OPERANDS.iter().find(|(m, _)| *m == mode).unwrap_or_else(|| panic!("no block mode {mode}")).1;
    let ports: String = ["ax", "ay", "az"].iter().zip(operands).zip(read).filter(|(_, r)| *r).map(|((p, o), _)| format!(".{p}({o}), ")).collect();
    format!(
        "  wire [31:0] {name}_chain;\n  wire {name}_dft;\n  fourteennm_fp_mac #({}, .clear_type(\"NONE\"), .use_chainin(\"false\"), \
         .operation_mode(\"{mode}\"), .adder_subtract(\"false\")) {name}(\n    .clk({{2'b00, clk}}), .ena({{2'b00, {enable}}}), .clr(2'b00), \
         {ports}.accumulate(1'b0), .chainin(32'd0), .resulta({y}), .chainout({name}_chain),\n    .mult_inexact({flags}[0]), \
         .mult_invalid({flags}[1]), .mult_overflow({flags}[2]), .mult_underflow({flags}[3]),\n    .adder_inexact({flags}[4]), \
         .adder_invalid({flags}[5]), .adder_overflow({flags}[6]), .adder_underflow({flags}[7]), .dftout({name}_dft));\n",
        params.join(", ")
    )
}

/// The operands each mode of the block reads, of `ax`, `ay` and `az`: the multiply ay·az, the add ax+ay, the multiply-add
/// ax+ay·az. Quartus refuses a block with a port its mode does not read connected.
const MODE_OPERANDS: [(&str, [bool; 3]); 3] = [("sp_mult", [false, true, true]), ("sp_add", [true, true, false]), ("sp_mult_add", [true, true, true])];

/// The oracle's hard blocks: the multiply, the add and the multiply-add fully registered, then the multiply and the
/// add with their output register alone, each stepping at the compute step.
fn hard_blocks() -> String {
    let blocks: [(&str, &str, &[&str], [&str; 3], [&str; 2]); 5] = [
        ("sp_mult", "hmul", MULTIPLY_REGISTERS, ["32'd0", "opr[31:0]", "opr[63:32]"], ["hm_now", "fm_now"]),
        ("sp_add", "hadd", ADD_REGISTERS, ["opr[31:0]", "opr[63:32]", "32'd0"], ["ha_now", "fa_now"]),
        ("sp_mult_add", "hfma", MULTIPLY_ADD_REGISTERS, ["opr[95:64]", "opr[31:0]", "opr[63:32]"], ["hf_now", "ff_now"]),
        ("sp_mult", "hmul1", LEAST_REGISTERS, ["32'd0", "opr[31:0]", "opr[63:32]"], ["hm1_now", "fm1_now"]),
        ("sp_add", "hadd1", LEAST_REGISTERS, ["opr[31:0]", "opr[63:32]", "32'd0"], ["ha1_now", "fa1_now"]),
    ];
    blocks.iter().map(|&(mode, name, used, operands, [y, flags])| fp_mac(mode, name, used, "step_en", operands, y, flags)).collect()
}

/// The engine's ports as the crossing names them: whether the engine takes it, its width's brackets, its name.
fn engine_ports() -> Vec<(bool, String, String)> {
    let bridge = bridge_verilog();
    let start = bridge.find("input fmid,").expect("the crossing's ports") + "input fmid,".len();
    let end = start + bridge[start..].find(");").expect("their end");
    bridge[start..end]
        .split(",\n")
        .map(|pair| {
            let first = pair.split(", ").next().expect("a port pair").trim();
            let input = first.starts_with("input");
            let last = first.split_whitespace().last().expect("a name");
            let name = last.strip_prefix("s_").expect("the shell's side of a port").to_string();
            let width = first.find('[').map_or(String::new(), |a| first[a..=first.find(']').expect("a width's end")].to_string());
            (input, width, name)
        })
        .collect()
}

/// `logos_engine`: the oracle's core stepping in the user clock at the compute clock's rising edges, reading its store
/// the user cycle after the falling edges, its ports through the engine's crossing.
fn top_text() -> String {
    let ports = engine_ports();
    let decls: Vec<String> = ports
        .iter()
        .map(|(input, width, name)| format!("{} {}{name}", if *input { "input" } else { "output" }, if width.is_empty() { String::new() } else { format!("{width} ") }))
        .collect();
    let mut wires = String::new();
    let mut bridge = vec![".sclk(clk), .fclk(fclk), .frst(frst), .fedge(fedge), .fmid(fmid)".to_string()];
    let mut core = vec![".clk(fclk), .step_en(fedge), .read_en(cr)".to_string()];
    for (_, width, name) in &ports {
        writeln!(wires, "  wire {}f_{name};", if width.is_empty() { String::new() } else { format!("{width} ") }).unwrap();
        bridge.push(format!(".s_{name}({name}), .f_{name}(f_{name})"));
        core.push(format!(".{name}(f_{name})"));
    }
    format!(
        "// The oracle as the shell's engine: its core stepping in the user clock (fclk) at the compute clock's rising edges\n\
         // (fedge), its store read the user cycle after the falling edges, the shell's values and its own through the\n\
         // engine's crossing.\n\
         module logos_engine\n  (input clk,\n   {},\n   input fclk, input frst, input fedge, input fmid);\n  \
         reg cr;\n  always @(posedge fclk) cr <= fmid;\n{wires}  logos_engine_bridge bridge({});\n  logos_fp_oracle core({});\nendmodule\n",
        decls.join(",\n   "),
        bridge.join(",\n    "),
        core.join(",\n    ")
    )
}

/// A hard block's result and flags delayed 0 to 15 steps.
const LATE: &str = r#"// x delayed d steps, 0 to 15, at the compute step; every register clear from reset.
module logos_fpo_late(input clk, input step_en, input rst, input [3:0] d, input [39:0] x, output [39:0] y);
  reg [39:0] line [1:15];
  integer i;
  always @(posedge clk) if (step_en) begin
    if (rst) for (i = 1; i < 16; i = i + 1) line[i] <= 40'd0;
    else begin
      line[1] <= x;
      for (i = 2; i < 16; i = i + 1) line[i] <= line[i-1];
    end
  end
  assign y = d == 4'd0 ? x : line[d];
endmodule

"#;

const CORE: &str = r#"// The oracle's core: each step an operand triple (C, B, A) from the run's source into the three hard blocks and a delay
// line, the soft classes reading the line at the run's tap, so each meets the hard results of its own operands; a counter
// a class, one more for each result of it the block's differs from; the steps no class explains counted and their
// records stored, or every step's in a recording run; then the header and the counters stored, and the core idle.
module logos_fp_oracle
  (input clk, input step_en, input read_en, input rst,
   input start, input [31:0] job_at, input [31:0] job_end, input [15:0] rows, input state_f16, input cache_int8, input two_words, input per_block,
   output busy, output reg fault, output reg [31:0] pc,
   input host_wr, input [2:0] host_table, input [31:0] host_addr, input [511:0] host_wdata,
   input [31:0] host_raddr, output [511:0] host_rdata,
   output mem_rd, output [31:0] mem_rd_line, input mem_rd_valid, input [511:0] mem_rd_data,
   output mem_wr, output [31:0] mem_wr_line, output [511:0] mem_wr_data, output [63:0] mem_wr_mask, input mem_wr_ready);
  localparam NM = @NM@, NA = @NA@, NV = @NV@, TAPS = @TAPS@, RECORDS = @RECORDS@, CAP = @CAP@;
  assign mem_rd = 1'b0;
  assign mem_rd_line = 32'd0;
  assign mem_wr = 1'b0;
  assign mem_wr_line = 32'd0;
  assign mem_wr_data = 512'd0;
  assign mem_wr_mask = 64'd0;

  localparam IDLE = 2'd0, RUN = 2'd1, DUMP = 2'd2;
  reg [1:0] state;
  assign busy = state != IDLE;
  reg [31:0] word, left;
  reg [5:0] drain;
  reg [63:0] seed, s;
  reg [@IBM@:0] at;
  reg [63:0] steps, lone_m, lone_a, records;
  reg [11:0] beat;

  // The stream: two xorshift words a step.
  function [63:0] xs(input [63:0] x);
    reg [63:0] y;
    begin
      y = x ^ (x << 13);
      y = y ^ (y >> 7);
      xs = y ^ (y << 17);
    end
  endfunction
  wire [63:0] r1 = xs(s), r2 = xs(r1);
  wire [95:0] rom_v;
  logos_fpo_vectors rom(.i(at), .v(rom_v));
  // Near: a's exponent zero, one, two, 255, 254 or any. Half the time b's places the product within -4 to 3 binades of
  // the least normal (127 - ea + d), a's then from the lower half when any, which b's can meet; a quarter, b's is a's
  // on a coin or within as many binades of it (ea + d), its significand a's but for the low byte and its sign the
  // other, so the sum cancels deep; else b's is any, or one of the ends.
  wire [1:0] kb = r2[7:6];
  wire [2:0] ka = r2[2:0];
  wire [7:0] ea = ka == 3'd0 ? 8'd0 : ka == 3'd1 ? 8'd1 : ka == 3'd2 ? 8'd2 : ka == 3'd3 ? 8'd255 : ka == 3'd4 ? 8'd254
                : kb[1] ? r1[30:23] : {1'b0, r1[29:23]};
  wire signed [10:0] d = $signed({{8{r2[5]}}, r2[5:3]});
  wire cancel = kb == 2'd2;
  wire [7:0] eend = r2[9:8] == 2'd0 ? 8'd0 : r2[9:8] == 2'd1 ? 8'd1 : r2[9:8] == 2'd2 ? 8'd254 : 8'd255;
  wire signed [10:0] eb_w = !kb[1] ? 11'sd127 - $signed({3'd0, ea}) + d : cancel ? $signed({3'd0, ea}) + (r2[11] ? 11'sd0 : d)
                          : r2[10] ? $signed({3'd0, r1[62:55]}) : $signed({3'd0, eend});
  wire [7:0] eb = eb_w < 11'sd0 ? 8'd0 : eb_w > 11'sd255 ? 8'd255 : eb_w[7:0];
  wire [22:0] mb = cancel ? r1[22:0] ^ {15'd0, r1[39:32]} : r1[54:32];
  wire [95:0] near_v = {r2[63:32], cancel ? !r1[31] : r1[63], eb, mb, r1[31], ea, r1[22:0]};
  wire [95:0] next_v = word[1:0] == 2'd0 ? rom_v : word[1:0] == 2'd1 ? {r2[31:0], r1} : near_v;

  // The triple the hard blocks take this step, and the delay line the soft classes read at the tap.
  reg [95:0] opr;
  reg ov;
  reg [95:0] dl [0:TAPS-1];
  reg [TAPS-1:0] dv;
  wire [3:0] tap = word[7:4];
  wire [95:0] t = dl[tap];
  wire tv = dv[tap];

  // The hard blocks' results and flags as they leave them, then each delayed the run's steps for its block.
  wire [31:0] hm_now, ha_now, hf_now, hm1_now, ha1_now;
  wire [7:0] fm_now, fa_now, ff_now, fm1_now, fa1_now;
@HARD@
  wire [31:0] hm, ha, hf;
  wire [7:0] fm, fa, ff;
  logos_fpo_late lm(.clk(clk), .step_en(step_en), .rst(rst), .d(word[11:8]), .x({fm_now, hm_now}), .y({fm, hm}));
  logos_fpo_late la(.clk(clk), .step_en(step_en), .rst(rst), .d(word[15:12]), .x({fa_now, ha_now}), .y({fa, ha}));
  logos_fpo_late lf(.clk(clk), .step_en(step_en), .rst(rst), .d(word[19:16]), .x({ff_now, hf_now}), .y({ff, hf}));
@UNITS@
  wire [NM-1:0] agree_m = {@AGREE_M@};
  wire [NA-1:0] agree_a = {@AGREE_A@};
  wire lonely = !(|agree_m) || !(|agree_a);
  wire keep = tv && records < 64'd@CAP@ && (word[2] || lonely);
@COUNTERS@
  // A record: (B, A), (the hard product, C), (the hard multiply-add, the hard sum), the three blocks' flags {underflow,
  // overflow, invalid, inexact} of the multiply then the add with the least-registered multiply's result as it leaves
  // it, the product classes' agreement, the sum classes' agreement and the least-registered add's result.
  wire [127:0] am = agree_m;
  wire [95:0] aa = agree_a;
  wire [511:0] record = {ha1_now, aa, am, 8'd0, hm1_now, ff, fa, fm, hf, ha, hm, t[95:64], t[63:0]};
  reg [511:0] dump_word;
  always @* case (beat)
      0: dump_word = {64'd@NA@, 64'd@NM@, {32'd0, word}, records, lone_a, lone_m, steps, @MAGIC@};
@DUMP@      default: dump_word = 512'd0;
  endcase
  wire st_we = (state == RUN && keep) || state == DUMP;
  wire [@AB@-1:0] st_wa = state == DUMP ? beat[@AB@-1:0] : RECORDS + records[@AB@-1:0];
  wire [511:0] st_wd = state == DUMP ? dump_word : record;
  logos_ram_ce_w512 #(.D(@BEATS@), .A(@AB@)) store(.clk(clk), .ce(step_en), .cr(read_en), .ra(host_raddr[@AB@-1:0]),
    .rd(host_rdata), .we(st_we), .wa(st_wa), .wd(st_wd), .wm({64{1'b1}}));

  integer i;
  always @(posedge clk) if (step_en) begin
    if (rst) begin
      state <= IDLE; fault <= 1'b0; pc <= 32'd0; seed <= 64'd0; opr <= 96'd0; ov <= 1'b0; dv <= {TAPS{1'b0}};
    end else begin
      if (host_wr && host_table == 3'd0 && host_addr == 32'd0) seed <= host_wdata[63:0];
      case (state)
        IDLE: if (start) begin
          state <= RUN; word <= job_at; left <= job_end; drain <= @DRAIN@;
          s <= seed == 64'd0 ? 64'h9e37_79b9_7f4a_7c15 : seed; at <= 0;
          steps <= 0; lone_m <= 0; lone_a <= 0; records <= 0; pc <= 32'd0; ov <= 1'b0; dv <= {TAPS{1'b0}};
         @CLEAR@
        end
        RUN: begin
          if (left != 32'd0) begin
            opr <= next_v; ov <= 1'b1; left <= left - 32'd1;
            if (word[1:0] == 2'd0) at <= at == @IB@'d@NVM1@ ? @IB@'d0 : at + 1'b1;
            else s <= r2;
          end else begin
            ov <= 1'b0;
            if (drain == 6'd0) begin state <= DUMP; beat <= 12'd0; end else drain <= drain - 6'd1;
          end
          dl[0] <= opr;
          for (i = 1; i < TAPS; i = i + 1) dl[i] <= dl[i-1];
          dv <= {dv[TAPS-2:0], ov};
          if (tv) begin
            steps <= steps + 64'd1;
            pc <= pc + 32'd1;
            if (!(|agree_m)) lone_m <= lone_m + 64'd1;
            if (!(|agree_a)) lone_a <= lone_a + 64'd1;
@BUMP@          end
          if (keep) records <= records + 64'd1;
        end
        default: if (beat == 12'd@RECORDS_M1@) state <= IDLE; else beat <= beat + 12'd1;
      endcase
    end
  end
endmodule
"#;

/// The floating-point DSP block's interface as the device declares it: its parameters and its ports.
const FP_MAC_HEADER: &str = "module fourteennm_fp_mac #(parameter accumulate_clock = \"none\", ax_clock = \"none\", ay_clock = \"none\",\n  \
     az_clock = \"none\", accum_pipeline_clock = \"none\", ax_chainin_pl_clock = \"none\", mult_pipeline_clock = \"none\",\n  \
     accum_2nd_pipeline_clock = \"none\", ax_chainin_2nd_pl_clock = \"none\", mult_2nd_pipeline_clock = \"none\",\n  \
     accum_adder_clock = \"none\", adder_input_clock = \"none\", output_clock = \"none\", clear_type = \"NONE\",\n  \
     use_chainin = \"false\", operation_mode = \"sp_mult\", adder_subtract = \"false\")\n  \
     (input [2:0] clk, input [2:0] ena, input [1:0] clr, input [31:0] ax, input [31:0] ay, input [31:0] az,\n   \
     input accumulate, input [31:0] chainin, input dfxlfsrena, input dfxmisrena, output [31:0] resulta,\n   \
     output [31:0] chainout, output adder_inexact, output adder_invalid, output adder_overflow, output adder_underflow,\n   \
     output mult_inexact, output mult_invalid, output mult_overflow, output mult_underflow, output dftout);\n";

/// The floating-point DSP block as the timing model takes it, a box only Quartus elaborates: its interface, one DSP
/// block large (`logos_box_area`, two halves), its results registers inside it.
pub fn fp_mac_blackbox() -> String {
    format!("(* blackbox, logos_box_area = \"0,0,2,0,0\" *)\n{FP_MAC_HEADER}endmodule\n")
}

/// How many steps deep a stand-in block is: fully registered, the multiply, the add and the multiply-add; with its
/// output register alone, the multiply and the add.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Depths {
    pub full: [usize; 3],
    pub least: [usize; 2],
}

/// `fourteennm_fp_mac` as a simulator runs the oracle: `product` and `sum` the semantics a block's multiply and add
/// compute (the multiply-add one after the other), `depths` the steps deep a block is in each mode and registering,
/// each register zero from configuration.
/// The device's own model is encrypted; this one stands in for it so a simulation of the persona can be held to what
/// the oracle must say of a block whose semantics are known.
pub fn hard_model_verilog(product: &Semantics, sum: &Semantics, depths: Depths) -> String {
    for latency in depths.full.iter().chain(&depths.least) {
        assert!((1..=TAPS).contains(latency), "a block {latency} steps deep");
    }
    let ([lm, la, lf], [lm1, la1]) = (depths.full, depths.least);
    format!(
        "{}{}// A floating-point DSP block for simulation: the multiply ay·az ({lm} steps deep, {lm1} with its pipeline unregistered),\n\
         // the add ax+ay ({la}, {la1}), the multiply-add ax+ay·az ({lf}), at clk[0] where ena[0].\n\
         {FP_MAC_HEADER}  wire [31:0] p, q, r;\n  \
         logos_fpo_hard_mul m(.a(ay), .b(az), .y(p));\n  \
         logos_fpo_hard_add s(.a(ax), .b(ay), .y(q));\n  \
         logos_fpo_hard_add f(.a(ax), .b(p), .y(r));\n  \
         wire [31:0] y = operation_mode == \"sp_mult\" ? p : operation_mode == \"sp_add\" ? q : r;\n  \
         localparam integer L = operation_mode == \"sp_mult\" ? (mult_pipeline_clock == \"NONE\" ? {lm1} : {lm})\n  \
         \x20 : operation_mode == \"sp_add\" ? (adder_input_clock == \"NONE\" ? {la1} : {la}) : {lf};\n  \
         // Every register zero from configuration, as the device's are.\n  \
         reg [31:0] line [0:{last}];\n  \
         integer i;\n  \
         initial for (i = 0; i <= {last}; i = i + 1) line[i] = 32'd0;\n  \
         always @(posedge clk[0]) if (ena[0]) begin\n    line[0] <= y;\n    for (i = 1; i <= {last}; i = i + 1) line[i] <= line[i-1];\n  end\n  \
         assign resulta = line[L-1];\n  \
         assign chainout = 32'd0;\n  \
         assign {{adder_inexact, adder_invalid, adder_overflow, adder_underflow}} = 4'd0;\n  \
         assign {{mult_inexact, mult_invalid, mult_overflow, mult_underflow}} = 4'd0;\n  \
         assign dftout = 1'b0;\nendmodule\n",
        fmul_verilog("logos_fpo_hard_mul", product),
        fadd_verilog("logos_fpo_hard_add", sum),
        last = TAPS - 1,
    )
}

/// The smallest engine the shell is built for: the oracle stands in its place, so nothing of it is the oracle's but
/// the store's depth.
fn shell_engine() -> EngineParams {
    EngineParams {
        rows: 1,
        vector_rows: 1,
        side_rows: 1,
        pair_rows: 1,
        store_beats: STORE_BEATS,
        job_words: 1,
        tensors: 1,
        layers: 1,
        ropes: 1,
        formats: Vec::new(),
        delta: None,
        attend: None,
        function_units: 16,
        folds: false,
        streaming: None,
    }
}

/// The persona's accelerator configuration: an engine build on plain 512-bit lines, its compute clock the user clock
/// divided by `divide`, its build identity (the BUILD register) the first eight bytes of the oracle engine's SHA-256.
pub fn persona_config(oracle: &Oracle, divide: u32) -> AfuConfig {
    let digest = sha256(oracle.engine_verilog().as_bytes());
    AfuConfig {
        name: "logos_d5005_fp_oracle",
        build: u64::from_be_bytes(digest[..8].try_into().expect("eight bytes")),
        compute_divide: divide,
        ..AfuConfig::engine(shell_engine(), LocalMemoryLine::Data512)
    }
}

/// Every file of the persona's tree for `scripts/d5005/build-afu.sh`: the engine build's files
/// ([`afu_sources`] of [`persona_config`]) with the oracle for its engine (the card's floating-point DSP blocks left to
/// Quartus), its simulation twin the oracle beside a stand-in block computing `model`, `depths` deep (what
/// `scripts/d5005/run-cosim.sh` runs), and the measurement the card runs, [`HOST_PROGRAM`] with its class table
/// ([`host_header`]).
pub fn persona_sources(oracle: &Oracle, divide: u32, model: &Semantics, depths: Depths) -> Vec<AfuFile> {
    let engine = oracle.engine_verilog();
    let mut files: Vec<AfuFile> = afu_sources(&persona_config(oracle, divide))
        .into_iter()
        .map(|f| match f.path {
            "hw/rtl/logos_engine.v" => AfuFile { path: f.path, text: engine.clone() },
            "sim/logos_engine_gated.v" => AfuFile { path: f.path, text: format!("{engine}\n{}", hard_model_verilog(model, model, depths)) },
            // The core proven one clock on its own hierarchy and boxed in the shell's check is the oracle's.
            p if p.ends_with("logos_d5005_domains.json") => {
                assert_eq!(f.text.matches("\"logos_engine_core_ce\"").count(), 1, "the engine's core in the clock-domain map");
                AfuFile { path: f.path, text: f.text.replace("\"logos_engine_core_ce\"", "\"logos_fp_oracle\"") }
            }
            _ => f,
        })
        .collect();
    files.push(AfuFile { path: "sw/logos_fp_oracle.c", text: HOST_PROGRAM.to_string() });
    files.push(AfuFile { path: "sw/logos_fp_oracle.h", text: host_header(oracle) });
    files
}

/// The store's header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub magic: u64,
    /// The triples compared.
    pub steps: u64,
    /// Steps no product class explains, and no sum class.
    pub lone_products: u64,
    pub lone_sums: u64,
    /// Records stored.
    pub records: u64,
    pub run: u32,
    pub product_classes: u64,
    pub sum_classes: u64,
}

/// A record: the operands, the three hard results and their flags (bits 3:0 the multiply's {inexact, invalid,
/// overflow, underflow} and 7:4 the add's, of each block), which classes agree with the block, and the
/// least-registered blocks' results.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub a: u32,
    pub b: u32,
    pub c: u32,
    pub product: u32,
    pub sum: u32,
    pub fma: u32,
    pub flags: [u8; 3],
    pub products_agree: Vec<bool>,
    pub sums_agree: Vec<bool>,
    /// The least-registered multiply's and add's results as they leave them, on the triple their latency before.
    pub least: [u32; 2],
}

/// The store read back: the header, each class's counter, the records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Store {
    pub header: Header,
    pub products: Vec<u64>,
    pub sums: Vec<u64>,
    pub records: Vec<Record>,
}

impl Store {
    /// The store from its beats (each eight 64-bit words, the lowest first), as far as the header's record count
    /// reaches; `None` for a header that is not the oracle's or does not name `oracle`'s classes.
    pub fn read(oracle: &Oracle, beats: &[[u64; 8]]) -> Option<Store> {
        let h = beats.first()?;
        let header = Header {
            magic: h[0],
            steps: h[1],
            lone_products: h[2],
            lone_sums: h[3],
            records: h[4],
            run: h[5] as u32,
            product_classes: h[6],
            sum_classes: h[7],
        };
        let (nm, na) = (oracle.products.len(), oracle.sums.len());
        if header.magic != MAGIC || header.product_classes != nm as u64 || header.sum_classes != na as u64 {
            return None;
        }
        let counters: Vec<u64> = beats.get(1..oracle.records_at())?.iter().flatten().copied().collect();
        let records = beats
            .get(oracle.records_at()..oracle.records_at() + header.records as usize)?
            .iter()
            .map(|w| {
                let bit = |base: usize, k: usize| w[base + k / 64] >> (k % 64) & 1 == 1;
                Record {
                    a: w[0] as u32,
                    b: (w[0] >> 32) as u32,
                    c: w[1] as u32,
                    product: (w[1] >> 32) as u32,
                    sum: w[2] as u32,
                    fma: (w[2] >> 32) as u32,
                    flags: [w[3] as u8, (w[3] >> 8) as u8, (w[3] >> 16) as u8],
                    products_agree: (0..nm).map(|k| bit(4, k)).collect(),
                    sums_agree: (0..na).map(|k| bit(6, k)).collect(),
                    least: [(w[3] >> 24) as u32, (w[7] >> 32) as u32],
                }
            })
            .collect();
        Some(Store { header, products: counters[..nm].to_vec(), sums: counters[nm..nm + na].to_vec(), records })
    }
}

/// The host program's table of the oracle's classes (`sw/logos_fp_oracle.h`): their counts, the store's layout, and
/// each class's members by name.
pub fn host_header(oracle: &Oracle) -> String {
    let mut h = String::from("/* The oracle the persona was built with: its classes, by their members' names, and its store's layout. */\n");
    writeln!(h, "#define LOGOS_FPO_MAGIC 0x{MAGIC:016x}ull").unwrap();
    writeln!(h, "#define LOGOS_FPO_PRODUCTS {}", oracle.products.len()).unwrap();
    writeln!(h, "#define LOGOS_FPO_SUMS {}", oracle.sums.len()).unwrap();
    writeln!(h, "#define LOGOS_FPO_VECTORS {}", oracle.vectors.len()).unwrap();
    writeln!(h, "#define LOGOS_FPO_TAPS {TAPS}").unwrap();
    writeln!(h, "#define LOGOS_FPO_STORE_BEATS {STORE_BEATS}").unwrap();
    writeln!(h, "#define LOGOS_FPO_RECORDS_AT {}", oracle.records_at()).unwrap();
    writeln!(h, "#define LOGOS_FPO_RECORD_CAP {}", oracle.record_cap()).unwrap();
    for (name, classes) in [("logos_fpo_products", &oracle.products), ("logos_fpo_sums", &oracle.sums)] {
        writeln!(h, "static const char *const {name}[] = {{").unwrap();
        for c in classes {
            writeln!(h, "    \"{}\",", c.members.join(" ")).unwrap();
        }
        h.push_str("};\n");
    }
    h
}

/// The measurement on the card (`sw/logos_fp_oracle.c`): the multiply's and the add's latencies as the taps at which
/// their classes come nearest every vector (counting runs over the vectors at each tap, no delay: the first tap of the
/// fewest disagreements, so a block no class names still has its depth); the multiply-add's as
/// the shift at which its results on the vectors whose C is zero are the products (a recording run at the last tap);
/// then, the three aligned ([`Run::aligned`]), a recording run over the vectors and counting runs of `<steps>` triples
/// from the random and the near streams; one JSON object on stdout. The exit status is 0 when every latency was found
/// and every run completed, 1 when a latency was not, 2 when the card cannot be had or a command failed.
pub const HOST_PROGRAM: &str = r##"/* Measure the card's floating-point DSP block with the oracle persona: find the multiply's and the add's latencies
   as the taps at which their classes come nearest every vector, the multiply-add's as the shift at which its results with
   C zero are the products; then, the three aligned, record every vector's results and count each class's
   disagreements over <steps> triples of the random stream and of the near stream. One JSON object on stdout.

     logos_fp_oracle [<steps>] [--seed <n>] */
#include "logos_core.h"
#include "logos_fp_oracle.h"

#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifndef LOGOS_PROGRAM_MAIN
#define LOGOS_PROGRAM_MAIN main
#endif

static uint64_t tag = 1;

static int run(logos_dev *dev, uint8_t op, uint64_t a0, uint64_t a1, uint64_t a2) {
    uint64_t t = tag++;
    logos_record r = logos_core_run(dev, op, t, a0, a1, a2);
    return (r.status >> 8) == t && (r.status & 0xff) == LOGOS_D5005_STATUS_OK;
}

/* A run's word: its source, whether it records every step, the soft tap, each block's delay. */
static uint64_t run_word(unsigned source, int record, unsigned tap, const unsigned d[3]) {
    return source | (uint64_t)(record != 0) << 2 | (uint64_t)(tap & 15) << 4 | (uint64_t)(d[0] & 15) << 8 |
           (uint64_t)(d[1] & 15) << 12 | (uint64_t)(d[2] & 15) << 16;
}

/* One run of `steps` triples, its store read into `buf`: the header and the counters, then the records the header
   counts. */
static int oracle_run(logos_dev *dev, uint64_t *buf, uint64_t io, uint64_t word, uint64_t steps) {
    if (!run(dev, LOGOS_D5005_OP_ENGINE_RUN, word, steps, 0)) return 0;
    if (!run(dev, LOGOS_D5005_OP_STORE_READ, io >> 6, 0, LOGOS_FPO_RECORDS_AT)) return 0;
    if (buf[0] != LOGOS_FPO_MAGIC || buf[6] != LOGOS_FPO_PRODUCTS || buf[7] != LOGOS_FPO_SUMS || buf[4] > LOGOS_FPO_RECORD_CAP) return 0;
    return buf[4] == 0 || run(dev, LOGOS_D5005_OP_STORE_READ, (io >> 6) + LOGOS_FPO_RECORDS_AT, LOGOS_FPO_RECORDS_AT, buf[4]);
}

static uint64_t counter(const uint64_t *buf, unsigned k) { return buf[8 + k]; }

static uint64_t least(const uint64_t *buf, unsigned from, unsigned n) {
    uint64_t m = UINT64_MAX;
    for (unsigned k = 0; k < n; ++k) if (counter(buf, from + k) < m) m = counter(buf, from + k);
    return m;
}

/* Record r's words: (B, A), (the product, C), (the multiply-add, the sum), the flags with the least-registered
   multiply's result above them, the product classes' agreement in two words, the sum classes' with the
   least-registered add's result above it. */
static const uint64_t *record(const uint64_t *buf, uint64_t r) { return buf + 8 * (LOGOS_FPO_RECORDS_AT + r); }

/* The 32 bits of word `word` of record `r` from bit `at`. */
static uint32_t field(const uint64_t *buf, uint64_t r, unsigned word, unsigned at) { return (uint32_t)(record(buf, r)[word] >> at); }

/* The shift s at which field `seen` of record k is, most often, field `want` of record k + s, over the records whose C
   is zero when `c_zero` (over all otherwise); -1 when no shift matches more than half of them. A result recorded as it
   leaves a block of latency L, the run at tap t, is that of the record t + 1 - L on. */
static int best_shift(const uint64_t *buf, const unsigned seen[2], const unsigned want[2], int c_zero, uint64_t *best, uint64_t *n) {
    int shift = -1;
    *best = 0;
    *n = 0;
    for (unsigned s = 0; s < LOGOS_FPO_TAPS; ++s) {
        uint64_t matches = 0, of = 0;
        for (uint64_t k = 0; k + s < buf[4]; ++k) {
            if (c_zero && field(buf, k + s, 1, 0) != 0) continue;
            ++of;
            if (field(buf, k, seen[0], seen[1]) == field(buf, k + s, want[0], want[1])) ++matches;
        }
        if (matches > *best) *best = matches, *n = of, shift = (int)s;
    }
    return shift >= 0 && 2 * *best > *n ? shift : -1;
}

static void counters(const uint64_t *buf, const char *name) {
    printf("    \"%s\": {\"steps\": %" PRIu64 ", \"lone_products\": %" PRIu64 ", \"lone_sums\": %" PRIu64 ", \"records\": %" PRIu64 ",\n",
           name, buf[1], buf[2], buf[3], buf[4]);
    printf("      \"products\": [");
    for (unsigned k = 0; k < LOGOS_FPO_PRODUCTS; ++k) printf("%s%" PRIu64, k ? ", " : "", counter(buf, k));
    printf("],\n      \"sums\": [");
    for (unsigned k = 0; k < LOGOS_FPO_SUMS; ++k) printf("%s%" PRIu64, k ? ", " : "", counter(buf, LOGOS_FPO_PRODUCTS + k));
    printf("]}");
}

static void records(const uint64_t *buf) {
    printf("    \"records\": [");
    for (uint64_t r = 0; r < buf[4]; ++r) {
        const uint64_t *w = record(buf, r);
        printf("%s\n      [\"%08" PRIx32 "\", \"%08" PRIx32 "\", \"%08" PRIx32 "\", \"%08" PRIx32 "\", \"%08" PRIx32 "\", \"%08" PRIx32 "\", \"%06" PRIx32 "\", \"%08" PRIx32 "\", \"%08" PRIx32 "\"]",
               r ? "," : "", (uint32_t)w[0], (uint32_t)(w[0] >> 32), (uint32_t)w[1], (uint32_t)(w[1] >> 32), (uint32_t)w[2],
               (uint32_t)(w[2] >> 32), (uint32_t)(w[3] & 0xffffff), (uint32_t)(w[3] >> 24), (uint32_t)(w[7] >> 32));
    }
    printf("]");
}

int LOGOS_PROGRAM_MAIN(int argc, char **argv) {
    uint64_t steps = 1ull << 24, seed = 0x5eedf00du;
    for (int i = 1; i < argc; ++i) {
        if (strcmp(argv[i], "--seed") == 0 && i + 1 < argc) seed = strtoull(argv[++i], NULL, 0);
        else steps = strtoull(argv[i], NULL, 0);
    }
    logos_dev *dev = logos_open(LOGOS_D5005_AFU_ID);
    if (!dev) {
        fprintf(stderr, "logos_fp_oracle: no card\n");
        return 2;
    }
    uint64_t io = 0;
    uint64_t *buf = logos_buffer(dev, (size_t)LOGOS_FPO_STORE_BEATS * 64, &io);
    memset(buf, 0, (size_t)LOGOS_FPO_STORE_BEATS * 64);
    buf[0] = seed;
    int ok = run(dev, LOGOS_D5005_OP_ENGINE_TABLE, io >> 6, 0, 1);
    const unsigned none[3] = {0, 0, 0};
    int tm = -1, ta = -1, lf = -1;
    uint64_t pmin = UINT64_MAX, smin = UINT64_MAX;
    printf("{\n  \"seed\": %" PRIu64 ",\n  \"taps\": [", seed);
    for (unsigned t = 0; ok && t < LOGOS_FPO_TAPS; ++t) {
        ok = oracle_run(dev, buf, io, run_word(0, 0, t, none), LOGOS_FPO_VECTORS);
        uint64_t p = ok ? least(buf, 0, LOGOS_FPO_PRODUCTS) : UINT64_MAX, s = ok ? least(buf, LOGOS_FPO_PRODUCTS, LOGOS_FPO_SUMS) : UINT64_MAX;
        printf("%s{\"tap\": %u, \"least_product\": %" PRIu64 ", \"least_sum\": %" PRIu64 "}", t ? ", " : "", t, p, s);
        if (ok && p < pmin) pmin = p, tm = (int)t;
        if (ok && s < smin) smin = s, ta = (int)t;
    }
    printf("],\n");
    /* The multiply-add's results at the last tap undelayed: the shift at which they are the products of the triples
       whose C is zero. */
    uint64_t fbest = 0, fn = 0, mbest = 0, mn = 0, abest = 0, an = 0;
    const unsigned fma_at[2] = {2, 32}, product_at[2] = {1, 32}, sum_at[2] = {2, 0}, least_product[2] = {3, 24}, least_sum[2] = {7, 32};
    if (ok && tm >= 0 && ta >= 0) {
        const unsigned d[3] = {15u - (unsigned)tm, 15u - (unsigned)ta, 0};
        ok = oracle_run(dev, buf, io, run_word(0, 1, 15, d), LOGOS_FPO_VECTORS);
        int s = ok ? best_shift(buf, fma_at, product_at, 1, &fbest, &fn) : -1;
        if (s >= 0) lf = 16 - s;
    }
    int found = tm >= 0 && ta >= 0 && lf > 0, lm1 = -1, la1 = -1;
    unsigned tap = 0, d[3] = {0, 0, 0};
    if (ok && found) {
        int l[3] = {tm + 1, ta + 1, lf};
        int top = l[0] > l[1] ? l[0] : l[1];
        top = top > l[2] ? top : l[2];
        tap = (unsigned)(top - 1);
        for (int k = 0; k < 3; ++k) d[k] = tap + 1 - (unsigned)l[k];
        ok = oracle_run(dev, buf, io, run_word(0, 1, tap, d), LOGOS_FPO_VECTORS);
        /* The least-registered blocks' results as they leave them: the shift at which they are the aligned ones'. */
        int sm = ok ? best_shift(buf, least_product, product_at, 0, &mbest, &mn) : -1, sa = ok ? best_shift(buf, least_sum, sum_at, 0, &abest, &an) : -1;
        if (sm >= 0) lm1 = (int)tap + 1 - sm;
        if (sa >= 0) la1 = (int)tap + 1 - sa;
    }
    printf("  \"latency\": {\"product\": %d, \"sum\": %d, \"multiply_add\": %d, \"product_least\": %d, \"sum_least\": %d,\n", tm + 1, ta + 1, lf, lm1, la1);
    printf("    \"matches\": {\"multiply_add\": [%" PRIu64 ", %" PRIu64 "], \"product_least\": [%" PRIu64 ", %" PRIu64 "], \"sum_least\": [%" PRIu64 ", %" PRIu64 "]}},\n",
           fbest, fn, mbest, mn, abest, an);
    printf("  \"runs\": {\n");
    if (ok && found) {
        counters(buf, "vectors");
        printf(",\n");
        records(buf);
        const char *names[2] = {"random", "near"};
        for (unsigned source = 1; ok && source <= 2; ++source) {
            ok = oracle_run(dev, buf, io, run_word(source, 0, tap, d), steps);
            if (ok) {
                printf(",\n");
                counters(buf, names[source - 1]);
            }
        }
        printf("\n  },\n  \"run_word\": %" PRIu64 ",\n", run_word(0, 0, tap, d));
    } else {
        printf("\n  },\n");
    }
    printf("  \"complete\": %s\n}\n", ok ? "true" : "false");
    logos_buffer_free(dev, buf);
    logos_close(dev);
    return !ok ? 2 : !found ? 1 : 0;
}
"##;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/fp_oracle/tests.rs"]
mod tests;
