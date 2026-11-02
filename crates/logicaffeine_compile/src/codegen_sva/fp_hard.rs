//! The engine's f32 multiply and add on the card's floating-point DSP block (`fourteennm_fp_mac`), as the oracle
//! ([`super::fp_oracle`]) measured it: a wrapper with the pipelined unit's ports (`logos_fmul_p`'s and
//! `logos_fadd_p`'s: `clk`, `a`, `b`, `y`) and exactly its latency, computing the engine's own arithmetic
//! ([`Semantics::ENGINE`]) on a block whose semantics are another member of the family, wherever a thin layer of
//! logic around the block can make them one.
//!
//! The block is registered fully, as the IP registers each mode, when that is no deeper than the latency, and with its
//! output register alone otherwise; registers after it make up the rest ([`plan`]). Around it: a denormal operand read
//! as a zero of its sign when the block keeps denormal operands, a denormal result written as a zero of its sign when
//! the block keeps denormal results (a result rounded at the denormals' precision and flushed unless it reaches the
//! least normal, as the engine's is), and every NaN result the engine's `0x7fc00000` when the block returns another; a
//! multiply's denormal operand is decided beside the block rather than flushed before it, so the block multiplies the
//! significands the engine's multiply does. A
//! block that flushes its results otherwise (tininess decided another way, or the flushed zero always positive) loses
//! what no logic after it can restore, and is refused by name. [`hard_verilog`] writes the wrapper free-running;
//! [`hard_verilog_ce`] writes it stepping at a clock enable, the block's registers enabled with the rest.

use std::fmt::{self, Write};

use super::fp_hypotheses::{Denormals, FlushSign, Nan, Semantics, Tininess};
use super::fp_oracle::{fp_mac, fma_vectors, Oracle, ADD_REGISTERS, LEAST_REGISTERS, MULTIPLY_REGISTERS};

/// The two operations a block computes for the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Multiply,
    Add,
}

impl Op {
    fn mode(self) -> &'static str {
        match self {
            Op::Multiply => "sp_mult",
            Op::Add => "sp_add",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Op::Multiply => "multiply",
            Op::Add => "add",
        }
    }
}

/// What the oracle measured of one mode of the block: the semantics it computes and its latency fully registered and
/// with its output register alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    pub semantics: Semantics,
    pub full: usize,
    pub least: usize,
}

/// How the block is registered in a wrapper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Registering {
    Full,
    Least,
}

/// A wrapper's build: the block's registering, the registers after it, and what the logic around it corrects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub registering: Registering,
    pub pads: usize,
    pub flush_operands: bool,
    pub flush_results: bool,
    pub canonical_nan: bool,
}

/// Why a block cannot be the engine's unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HardError {
    /// Even with its output register alone the block is deeper than the unit's latency.
    TooDeep { op: &'static str, latency: usize, least: usize },
    /// The block computes something no logic around it makes the arithmetic it stands in for.
    Semantics { op: &'static str, semantics: String, why: String },
}

impl fmt::Display for HardError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            HardError::TooDeep { op, latency, least } => {
                write!(f, "the block's {op} is {least} clocks deep with its output register alone, past the unit's {latency}")
            }
            HardError::Semantics { op, semantics, why } => write!(f, "the block's {op} ({semantics}) {why}"),
        }
    }
}

/// The wrapper's build for the block's `op` at `latency` clocks, standing in for the engine's own arithmetic
/// ([`plan_for`] of [`Semantics::ENGINE`]).
pub fn plan(op: Op, block: &Block, latency: usize) -> Result<Plan, HardError> {
    plan_for(op, block, latency, &Semantics::ENGINE)
}

/// How many results `t` decides tiny, in order: after rounding at the denormals' place fewest, then after rounding to
/// 24 bits, then on the exact result every one below the least normal.
fn tinier(t: Tininess) -> u8 {
    match t {
        Tininess::AfterRoundingAtDenormals => 0,
        Tininess::AfterRoundingUnbounded => 1,
        Tininess::BeforeRounding => 2,
    }
}

/// The wrapper's build for the block's `op` at `latency` clocks that makes it compute `target`: the logic around it
/// flushes the denormal operands the block reads as themselves where `target` reads zeros, flushes the denormal results
/// the block keeps where `target` flushes them at the denormals' place keeping their sign, and writes `0x7fc00000`
/// for every NaN where that is `target`'s one NaN and the block's is another. Refused by name is a block that reads a
/// zero or flushes a result where `target` keeps a denormal, decides tininess elsewhere than `target`, flushes to
/// another sign, or returns NaNs `target` does not and the wrapper cannot write.
pub fn plan_for(op: Op, block: &Block, latency: usize, target: &Semantics) -> Result<Plan, HardError> {
    let s = &block.semantics;
    let who = if *target == Semantics::ENGINE { "the engine".to_string() } else { target.name() };
    let refuse = |why: String| Err(HardError::Semantics { op: op.name(), semantics: s.name(), why });
    if s.denormal_inputs == Denormals::Flush && target.denormal_inputs == Denormals::Keep {
        return refuse(format!("reads a denormal operand as a zero, which {who} reads as itself"));
    }
    match (s.denormal_results, target.denormal_results) {
        (Denormals::Flush, Denormals::Keep) => return refuse(format!("flushes a result {who} keeps as a denormal")),
        (Denormals::Flush, Denormals::Flush) => {
            if tinier(s.tininess) > tinier(target.tininess) {
                return refuse(format!("decides a result tiny before {who} does, and flushes what {who} would round into the least normal"));
            }
            if tinier(s.tininess) < tinier(target.tininess) {
                return refuse(format!("rounds into the least normal a result {who} decides tiny and flushes"));
            }
            if s.flushed_sign != target.flushed_sign {
                return refuse(match target.flushed_sign {
                    FlushSign::Kept => format!("flushes a tiny result to +0 whatever its sign, which {who} keeps"),
                    FlushSign::Positive => format!("flushes a tiny result with its sign, which {who} drops"),
                });
            }
        }
        (Denormals::Keep, Denormals::Flush) => {
            if target.tininess != Tininess::AfterRoundingAtDenormals || target.flushed_sign != FlushSign::Kept {
                return refuse(format!("keeps a denormal result, which flushed after it is no result {who} decides tiny"));
            }
        }
        (Denormals::Keep, Denormals::Keep) => {}
    }
    if s.nan != target.nan && target.nan != Nan::Canonical(0x7fc0_0000) {
        return refuse(format!("returns NaNs {who} does not, and the logic after it writes only 0x7fc00000"));
    }
    let (registering, depth) = if block.full <= latency {
        (Registering::Full, block.full)
    } else if block.least <= latency {
        (Registering::Least, block.least)
    } else {
        return Err(HardError::TooDeep { op: op.name(), latency, least: block.least });
    };
    Ok(Plan {
        registering,
        pads: latency - depth,
        flush_operands: s.denormal_inputs == Denormals::Keep && target.denormal_inputs == Denormals::Flush,
        flush_results: s.denormal_results == Denormals::Keep && target.denormal_results == Denormals::Flush,
        canonical_nan: s.nan != target.nan,
    })
}

/// The wrapper `module` for the block's `op` at `latency` clocks, free-running.
pub fn hard_verilog(op: Op, module: &str, block: &Block, latency: usize) -> Result<String, HardError> {
    wrapper(op, module, block, latency, false, &Semantics::ENGINE)
}

/// The wrapper `module` for the block's `op` at `latency` steps, every register (the block's and the pads) stepping
/// where `step_en`, as the engine's enabled cores step.
pub fn hard_verilog_ce(op: Op, module: &str, block: &Block, latency: usize) -> Result<String, HardError> {
    wrapper(op, module, block, latency, true, &Semantics::ENGINE)
}

/// [`hard_verilog`] computing `target` ([`plan_for`]): `hard_verilog` is this of [`Semantics::ENGINE`], byte for byte.
pub fn hard_verilog_for(op: Op, module: &str, block: &Block, latency: usize, target: &Semantics) -> Result<String, HardError> {
    wrapper(op, module, block, latency, false, target)
}

/// [`hard_verilog_ce`] computing `target` ([`plan_for`]): `hard_verilog_ce` is this of [`Semantics::ENGINE`], byte for
/// byte.
pub fn hard_verilog_ce_for(op: Op, module: &str, block: &Block, latency: usize, target: &Semantics) -> Result<String, HardError> {
    wrapper(op, module, block, latency, true, target)
}

/// The NaN `target` returns, as a Verilog expression over the operands `a` and `b` (and the operand classes `na`, `nb`).
fn target_nan(target: &Semantics) -> String {
    let word = |v: u32| format!("32'h{:04x}_{:04x}", v >> 16, v & 0xffff);
    match target.nan {
        Nan::Canonical(v) => word(v),
        Nan::XorSigned(v) => format!("{{a[31] ^ b[31], 31'h{:08x}}}", v & 0x7fff_ffff),
        Nan::Propagate { invalid } => format!("(na ? {{a[31], 8'hff, 1'b1, a[21:0]}} : nb ? {{b[31], 8'hff, 1'b1, b[21:0]}} : {})", word(invalid)),
    }
}

fn wrapper(op: Op, module: &str, block: &Block, latency: usize, ce: bool, target: &Semantics) -> Result<String, HardError> {
    let p = plan_for(op, block, latency, target)?;
    let (ports, enable, step) = if ce {
        ("input clk, input step_en, input [31:0] a, input [31:0] b, output [31:0] y", "step_en", "always @(posedge clk) if (step_en) begin")
    } else {
        ("input clk, input [31:0] a, input [31:0] b, output [31:0] y", "1'b1", "always @(posedge clk) begin")
    };
    let (registered, depth) = match p.registering {
        Registering::Full => ("registered as the IP registers it", block.full),
        Registering::Least => ("with its output register alone", block.least),
    };
    let (what, is) = if *target == Semantics::ENGINE {
        (format!("The engine's f32 {}", op.name()), format!("the engine's {}", op.name()))
    } else {
        (format!("The f32 {} of {}", op.name(), target.name()), format!("that {}", op.name()))
    };
    let mut doc = format!(
        "// {what} on the card's floating-point DSP block, {latency} {}: y is {is} of the operands\n\
         // {latency} {} before. The block {registered}, {depth} deep{}",
        if ce { "steps" } else { "clocks" },
        if ce { "steps" } else { "clocks" },
        match p.pads {
            0 => String::new(),
            1 => ", then a register".to_string(),
            n => format!(", then {n} registers"),
        }
    );
    let fixes: Vec<&str> = [
        (p.flush_operands, "a denormal operand read as a zero of its sign"),
        (p.flush_results, "a denormal result written as a zero of its sign"),
        (p.canonical_nan, "every NaN result 0x7fc00000"),
    ]
    .iter()
    .filter(|(on, _)| *on)
    .map(|(_, what)| *what)
    .collect();
    if fixes.is_empty() {
        doc.push_str(".\n");
    } else {
        write!(doc, ".\n// Around it: {}.\n", fixes.join(", ")).unwrap();
    }
    let mut v = format!("{doc}module {module}({ports});\n");
    // An add's denormal operand is flushed before the block. A multiply's is decided beside it, the block taking the
    // operands as they come, so it multiplies what the engine multiplies: to the engine the operand is a zero of its
    // sign, the product a signed zero, or NaN against an infinity or a NaN.
    let beside = p.flush_operands && op == Op::Multiply;
    let operand = |x: &str| if p.flush_operands && !beside { format!("(({x}[30:23] == 8'd0) ? {{{x}[31], 31'd0}} : {x})") } else { x.to_string() };
    writeln!(v, "  wire [31:0] ia = {}, ib = {};", operand("a"), operand("b")).unwrap();
    if beside {
        v.push_str(
            "  wire        da = (a[30:23] == 8'd0) && (a[22:0] != 23'd0), db = (b[30:23] == 8'd0) && (b[22:0] != 23'd0);\n\
             \x20 wire        za = (a[30:23] == 8'd0), zb = (b[30:23] == 8'd0);\n\
             \x20 wire        na = (a[30:23] == 8'hff) && (a[22:0] != 23'd0), nb = (b[30:23] == 8'hff) && (b[22:0] != 23'd0);\n\
             \x20 wire        fa = (a[30:23] == 8'hff) && (a[22:0] == 23'd0), fb = (b[30:23] == 8'hff) && (b[22:0] == 23'd0);\n\
             \x20 wire [32:0] side_c = {da || db, (na || nb || (fa && zb) || (fb && za)) ? NAN : {a[31] ^ b[31], 31'd0}};\n"
                .replace("NAN", &target_nan(target))
                .as_str(),
        );
        writeln!(
            v,
            "  // The decision as deep as the block, beside its result.\n  reg [32:0] side [0:{last}];\n  integer j;\n  {step}\n    side[0] <= side_c;\n    for (j = 1; j < {depth}; j = j + 1) side[j] <= side[j-1];\n  end",
            last = depth - 1
        )
        .unwrap();
    }
    v.push_str("  wire [31:0] h;\n  wire [7:0] flags;\n");
    let (used, operands) = match (op, p.registering) {
        (Op::Multiply, Registering::Full) => (MULTIPLY_REGISTERS, ["32'd0", "ia", "ib"]),
        (Op::Add, Registering::Full) => (ADD_REGISTERS, ["ia", "ib", "32'd0"]),
        (Op::Multiply, Registering::Least) => (LEAST_REGISTERS, ["32'd0", "ia", "ib"]),
        (Op::Add, Registering::Least) => (LEAST_REGISTERS, ["ia", "ib", "32'd0"]),
    };
    v.push_str(&fp_mac(op.mode(), "u", used, enable, operands, "h", "flags"));
    v.push_str("  wire nan = (h[30:23] == 8'hff) && (h[22:0] != 23'd0);\n");
    let fixed = match (p.canonical_nan, p.flush_results) {
        (false, false) => "h".to_string(),
        (true, false) => "nan ? 32'h7fc0_0000 : h".to_string(),
        (false, true) => "(h[30:23] == 8'd0) ? {h[31], 31'd0} : h".to_string(),
        (true, true) => "nan ? 32'h7fc0_0000 : (h[30:23] == 8'd0) ? {h[31], 31'd0} : h".to_string(),
    };
    if beside {
        writeln!(v, "  wire [31:0] c = side[{}][32] ? side[{}][31:0] : {fixed};", depth - 1, depth - 1).unwrap();
    } else {
        writeln!(v, "  wire [31:0] c = {fixed};").unwrap();
    }
    if p.pads == 0 {
        v.push_str("  assign y = c;\n");
    } else {
        writeln!(v, "  reg [31:0] pad [0:{}];\n  integer i;\n  {step}\n    pad[0] <= c;\n    for (i = 1; i < {}; i = i + 1) pad[i] <= pad[i-1];\n  end\n  assign y = pad[{}];", p.pads - 1, p.pads, p.pads - 1).unwrap();
    }
    v.push_str("endmodule\n");
    Ok(v)
}

/// How the block's multiply-add rounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rounding {
    /// Once, the exact `C + A·B` rounded: a fused multiply-add.
    Once,
    /// The product rounded, then the sum.
    Twice,
    /// Neither, on the oracle's multiply-add vectors.
    Other,
}

/// What the card said of its block: each mode's semantics and depths, the multiply-add's latency and rounding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Measurement {
    pub product: Block,
    pub sum: Block,
    pub multiply_add_latency: usize,
    pub multiply_add: Rounding,
}

/// One mode's counts as the measurement found them: each class's steps differing from the block over every run, in the
/// oracle's order, and the steps no class explains.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tally {
    pub differing: Vec<u64>,
    pub unexplained: u64,
}

/// Why a measurement names no block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MeasureError {
    Json(String),
    /// The measurement did not finish, or a field it writes is missing.
    Incomplete(&'static str),
    /// No class, or more than one, agreed with the block on every step of every run.
    Class { op: &'static str, agreeing: usize, unexplained: u64 },
}

impl fmt::Display for MeasureError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            MeasureError::Json(e) => write!(f, "the measurement: {e}"),
            MeasureError::Incomplete(what) => write!(f, "the measurement has no {what}"),
            MeasureError::Class { op, agreeing: 0, unexplained } => {
                write!(f, "the block's {op} is no member of the family: every class differs somewhere ({unexplained} steps no class explains)")
            }
            MeasureError::Class { op, agreeing, .. } => write!(f, "{agreeing} classes agree with the block's {op} everywhere, not one"),
        }
    }
}

impl Measurement {
    /// The block the oracle's measurement (the host program's JSON, [`super::fp_oracle::HOST_PROGRAM`]) names for
    /// `oracle`: each mode's one class with no difference over the vectors and both streams, its first member's
    /// semantics, the latencies measured; and the multiply-add's rounding from its vectors whose C is the product
    /// rounded and negated, read once rounded as the product's error and twice rounded as zero.
    pub fn read(oracle: &Oracle, json: &str) -> Result<Measurement, MeasureError> {
        let v = finished(json)?;
        let product = block(&v, oracle, Op::Multiply)?;
        let sum = block(&v, oracle, Op::Add)?;
        let (multiply_add_latency, multiply_add) = multiply_add(&v)?;
        Ok(Measurement { product, sum, multiply_add_latency, multiply_add })
    }

    /// The multiply-add alone: its depth and how it rounds, whatever either block named.
    pub fn read_multiply_add(json: &str) -> Result<(usize, Rounding), MeasureError> {
        multiply_add(&finished(json)?)
    }

    /// The multiply's block alone: its one agreeing class and its depths, whatever the add named.
    pub fn read_product(oracle: &Oracle, json: &str) -> Result<Block, MeasureError> {
        block(&finished(json)?, oracle, Op::Multiply)
    }

    /// The add's block alone: its one agreeing class and its depths, whatever the multiply named.
    pub fn read_sum(oracle: &Oracle, json: &str) -> Result<Block, MeasureError> {
        block(&finished(json)?, oracle, Op::Add)
    }

    /// One mode's counts alone, whatever its classes name.
    pub fn read_tally(oracle: &Oracle, json: &str, op: Op) -> Result<Tally, MeasureError> {
        tally(&finished(json)?, oracle, op)
    }

    /// One mode's depths alone, fully registered and with its output register alone.
    pub fn read_depths(json: &str, op: Op) -> Result<(usize, usize), MeasureError> {
        let v = finished(json)?;
        let keys = Keys::of(op);
        Ok((latency(&v, keys.full)?, latency(&v, keys.least)?))
    }

    /// The measurement's seed and the steps of its runs: the vectors, the random stream and the near stream.
    pub fn read_runs(json: &str) -> Result<(u64, [u64; 3]), MeasureError> {
        let v = finished(json)?;
        let seed = v["seed"].as_u64().ok_or(MeasureError::Incomplete("seed"))?;
        let mut steps = [0; 3];
        for (n, run) in steps.iter_mut().zip(RUNS) {
            *n = v["runs"][run]["steps"].as_u64().ok_or(MeasureError::Incomplete("steps"))?;
        }
        Ok((seed, steps))
    }
}

/// A finished measurement's JSON.
fn finished(json: &str) -> Result<serde_json::Value, MeasureError> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| MeasureError::Json(e.to_string()))?;
    if v["complete"] != true {
        return Err(MeasureError::Incomplete("finished run"));
    }
    Ok(v)
}

/// The multiply-add's depth, and how it rounds from its records of the multiply-add vectors: once when every result is
/// the fused one, twice when every result is the product rounded and then the sum (each vector of a quadruple but the
/// last cancelling to zero), neither otherwise.
fn multiply_add(v: &serde_json::Value) -> Result<(usize, Rounding), MeasureError> {
    let records = v["runs"]["records"].as_array().ok_or(MeasureError::Incomplete("records"))?;
    let word = |r: &serde_json::Value, i: usize| r[i].as_str().and_then(|w| u32::from_str_radix(w, 16).ok());
    let mut once = true;
    let mut twice = true;
    for (k, &[a, b, c]) in fma_vectors().iter().enumerate() {
        let r = records.iter().find(|r| word(r, 0) == Some(a) && word(r, 1) == Some(b) && word(r, 2) == Some(c)).ok_or(MeasureError::Incomplete("multiply-add records"))?;
        let got = word(r, 5).ok_or(MeasureError::Incomplete("multiply-add result"))?;
        let (x, y, z) = (f32::from_bits(a), f32::from_bits(b), f32::from_bits(c));
        once &= got == ((f64::from(x) * f64::from(y) + f64::from(z)) as f32).to_bits();
        twice &= got == (x * y + z).to_bits();
        if k % 4 != 3 {
            twice &= got == 0;
        }
    }
    let rounding = match (once, twice) {
        (true, false) => Rounding::Once,
        (false, true) => Rounding::Twice,
        _ => Rounding::Other,
    };
    Ok((latency(v, "multiply_add")?, rounding))
}

/// A depth the measurement found, by its key.
fn latency(v: &serde_json::Value, key: &'static str) -> Result<usize, MeasureError> {
    v["latency"][key].as_i64().filter(|l| *l > 0).map(|l| l as usize).ok_or(MeasureError::Incomplete(key))
}

/// The runs a measurement makes, in the order it makes them.
const RUNS: [&str; 3] = ["vectors", "random", "near"];

/// Where a mode's numbers sit in the measurement: its name, its counters' and unexplained steps' keys in each run, and
/// its depths' keys.
struct Keys {
    name: &'static str,
    counters: &'static str,
    lone: &'static str,
    full: &'static str,
    least: &'static str,
}

impl Keys {
    fn of(op: Op) -> Keys {
        match op {
            Op::Multiply => Keys { name: "multiply", counters: "products", lone: "lone_products", full: "product", least: "product_least" },
            Op::Add => Keys { name: "add", counters: "sums", lone: "lone_sums", full: "sum", least: "sum_least" },
        }
    }
}

/// One mode's counts: each class's counters summed over the runs, one counter a class in every run, and the steps no
/// class explains summed.
fn tally(v: &serde_json::Value, oracle: &Oracle, op: Op) -> Result<Tally, MeasureError> {
    let keys = Keys::of(op);
    let classes = match op {
        Op::Multiply => oracle.products.len(),
        Op::Add => oracle.sums.len(),
    };
    let mut tally = Tally { differing: vec![0; classes], unexplained: 0 };
    for run in RUNS {
        let counters = v["runs"][run][keys.counters].as_array().filter(|c| c.len() == classes).ok_or(MeasureError::Incomplete("counters"))?;
        for (sum, c) in tally.differing.iter_mut().zip(counters) {
            *sum += c.as_u64().ok_or(MeasureError::Incomplete("counters"))?;
        }
        tally.unexplained += v["runs"][run][keys.lone].as_u64().ok_or(MeasureError::Incomplete("unexplained steps"))?;
    }
    Ok(tally)
}

/// One block as the measurement found it: the one class that differed on no step of any run (the steps no class
/// explains when none or several did) and the block's depths, fully registered and least.
fn block(v: &serde_json::Value, oracle: &Oracle, op: Op) -> Result<Block, MeasureError> {
    let keys = Keys::of(op);
    let classes = match op {
        Op::Multiply => &oracle.products,
        Op::Add => &oracle.sums,
    };
    let tally = tally(v, oracle, op)?;
    let agreeing: Vec<usize> = (0..classes.len()).filter(|&k| tally.differing[k] == 0).collect();
    let [one] = agreeing[..] else {
        return Err(MeasureError::Class { op: keys.name, agreeing: agreeing.len(), unexplained: tally.unexplained });
    };
    Ok(Block { semantics: classes[one].semantics, full: latency(v, keys.full)?, least: latency(v, keys.least)? })
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/fp_hard/tests.rs"]
mod tests;
