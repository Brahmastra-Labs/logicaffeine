//! The decode engine's units in Verilog, each bit for bit its numerics function.
//!
//! `spatial::decode_engine::rtl` prints every unit. Under iverilog each computes the vectors here exactly as
//! `decode_engine::numerics` does:
//!   • `logos_fmul`/`logos_fadd` over seeded bits of every class, ties to even, near cancellations, the overflow
//!     edge, the boundary where a result below the normals rounds up to the least normal, denormals, signed zeros,
//!     infinities and NaNs; their pipelined copies `logos_fmul_p`/`logos_fadd_p` over the same vectors a pair a clock;
//!   • `logos_act_block` over seeded activation blocks, its closed-form scale held to `block_scale` over every
//!     significand; `logos_act_wide` over seeded weight blocks of 32 and of 256, its closed form held to
//!     `wide_scale` over every significand; `logos_act_stream`, the engine's one quantizer, over a stream of blocks of
//!     every length it serves arriving a beat at a time;
//!   • every `logos_dec_<format>` over seeded and patterned blocks, each weight's code, unit multipliers and scales
//!     giving the coded row's coefficient and offset term;
//!   • the array's processing element: the two-row lane packing split exactly over every operand of one product and
//!     over seeded and extreme chains, and `logos_subdot32` giving both rows' 32-element partial dots;
//!   • `logos_lane` over every format's blocks in both word counts, from the array's excess partial sums to the row's
//!     next f32, as the lane model computes it; `logos_lane_value_p`, the lane to its rounding pipelined, over the
//!     same blocks a block a clock;
//!   • `logos_matvec_<format>` — the decoders, the processing elements a digit at a time and the lanes — taking two
//!     rows of every format a block at a time against token-rows in both word counts to `numerics::dot`'s values;
//!     the streaming column `logos_column_<format>` taking a matrix block-major within groups of row pairs to the
//!     same values, at groups and token-rows that bring a sum round from every clock to every eighth, and, fed
//!     whenever it is ready, a token-row a clock to the clock `column_clocks` gives; the column array
//!     `logos_column_array_<format>` taking a matrix laid in stripes over four channels, its cores in lockstep on one
//!     activation read, to the same values, stripes whole and cut short, and to the clock `column_array_clocks` gives;
//!     each channel's `logos_column_feed_<format>` dealing its tiles' blocks from the image stream's beats to its cores
//!     in pairs; and the whole streaming matvec, a striped image's matrix through the feeds into the array, to `dot`,
//!     one built for every format the 27B holds its matrices in taking a matrix of each;
//!   • the block RAM with a registered read, `logos_bram_w<W>`, a word out the clock after its address, the old word
//!     on a read of the address being written, and each byte lane written under its mask;
//!   • the vector units' `exp`, `recip`, `rsqrt`, `ln`, `ln1p`, `softplus`, `sigmoid` and `silu` over every class of
//!     operand and the ranges each is used on;
//!   • the state codec over every binary16 value and seeded f32s, the Gated-DeltaNet row engine streaming seeded
//!     rows through `gdn_row`, and the lazily flushed row's read-out and flush through `gdn_read` and `gdn_flush`, the
//!     state in f32 and in f16;
//!   • the attention unit's vector path over seeded heads of 1 to 64 positions, as `numerics::attend_vector`, and its
//!     int8 path on the array over int8 caches in both word counts, as `numerics::attend_int8`;
//!   • a lane's function unit computing every function of the instruction set over the same operands, one core of
//!     each function in it;
//!   • the vector unit running every micro-program over seeded registers of lengths whole and ragged in its sixteen
//!     lanes — the norms, the softmax, the gates, the rotary turn, the int8 cache row, the convolution, the greedy
//!     token — each register it leaves the instruction set's interpreter's, and its int8 code unit over every class of
//!     operand and inverse scale; each of its lanes holding one set of four multiplies and two adds that all its
//!     instructions share; its lanes sharing 1, 2, 4 and 8 function units, the engine's `FUNCTION_UNITS` among them.

#![cfg(feature = "codegen")]

mod common;
#[path = "common/rtl_sim.rs"]
mod rtl_sim;

use std::fmt::Write as _;
use std::process::Command;

use logicaffeine_compile::model::quant::{f16_to_f32, GgmlType};
use logicaffeine_compile::spatial::decode_engine::lane::{deliver, lane, LaneBlock, UnitSums};
use logicaffeine_compile::spatial::decode_engine::numerics::{
    attend_vector, block_scale, dot, exp, gdn_flush, gdn_fold, gdn_fold_trailing, gdn_read, Logged, f32_to_f16, fadd, fmul, gdn_row, int8_codes, ln, ln1p, quantize, quantize_blocks, reads, recip, rsqrt,
    rmsnorm, sigmoid, silu, softplus, tree, wide_scale, ActScale, CodedRow, FpUnit, Scale, Words, ACT_BLOCK,
};
use logicaffeine_compile::spatial::decode_engine::rtl::act_quant::{act_quant_verilog, closed_form_scale, closed_form_wide_scale};
use logicaffeine_compile::spatial::decode_engine::rtl::array::{array_verilog, pack, split, CHAIN_ELEMENTS, HIGH_OFFSET, LANE};
use logicaffeine_compile::spatial::decode_engine::rtl::fp32::{fp32_verilog, TREE16_LATENCY};
use logicaffeine_compile::spatial::decode_engine::rtl::gdn::{
    delta_normed_lanes, flush_m_verilog, gate_verilog, gdn_stream_verilog, gdn_verilog, lanes_trailing_verilog, lanes_verilog,
    FOLD_M_READ_LATENCY, LANE_CREDIT,
};
use logicaffeine_compile::spatial::decode_engine::rtl::lane::lane_verilog;
use logicaffeine_compile::spatial::decode_engine::rtl::attend::attend_verilog;
use logicaffeine_compile::spatial::decode_engine::rtl::engine::{census, FUNCTION_UNITS};
use logicaffeine_compile::spatial::decode_engine::rtl::matvec::matvec_verilog;
use logicaffeine_compile::spatial::decode_engine::rtl::simd::{
    encode, simd_p_verilog, simd_verilog, LANE_FUNC_P_LATENCY, LANE_FUNC_STAGES, SIMD_P_READ_LATENCY,
};
use logicaffeine_compile::spatial::decode_engine::rtl::vector::vector_verilog;
use logicaffeine_compile::spatial::decode_engine::vector::{self, Func, Op, Regs};
use logicaffeine_compile::spatial::decode_engine::rtl::weights::{decoder_name, units, weights_verilog, DECODERS, UNIT};
use logicaffeine_compile::spatial::serving::array_passes;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

/// Pairs that stress each path: random bits, near-equal magnitudes of either sign (cancellation), exponents far
/// apart (alignment past the sticky), products at the bottom and the top of the range, and the special values.
fn vectors(n: usize) -> Vec<(u32, u32)> {
    let mut r = Rng(0xf32);
    let specials = [
        0u32, 0x8000_0000, 1, 0x8000_0001, 0x007f_ffff, 0x0080_0000, 0x8080_0000, 0x3f80_0000, 0xbf80_0000, 0x7f7f_ffff,
        0xff7f_ffff, 0x7f80_0000, 0xff80_0000, 0x7fc0_0000, 0x7fa0_0001, 0xffc0_0000, 0x3400_0000, 0x0040_0000,
    ];
    let mut out = Vec::with_capacity(n);
    for &a in &specials {
        for &b in &specials {
            out.push((a, b));
        }
    }
    while out.len() < n {
        let a = r.next() as u32;
        let b = match out.len() % 6 {
            0 => r.next() as u32,
            // Near cancellation: the same exponent, a nearby significand, either sign.
            1 => (a ^ 0x8000_0000).wrapping_add((r.next() % 64) as u32),
            2 => a.wrapping_add((r.next() % 8) as u32),
            // Far apart in exponent.
            3 => (a & 0x807f_ffff) | (((((a >> 23) & 0xff) + 20 + (r.next() % 10) as u32) & 0xff) << 23),
            // Products near the least normal: exponents summing to the bottom of the range.
            4 => {
                let ea = (a >> 23) & 0xff;
                let eb = (127u32 + 1).wrapping_sub(ea).wrapping_add((r.next() % 5) as u32) & 0xff;
                (b_bits(&mut r) & 0x807f_ffff) | (eb << 23)
            }
            // Products near the top: overflow and its edge.
            _ => {
                let ea = (a >> 23) & 0xff;
                let eb = (254u32 + 127).wrapping_sub(ea).wrapping_sub((r.next() % 3) as u32) & 0xff;
                (b_bits(&mut r) & 0x807f_ffff) | (eb << 23)
            }
        };
        out.push((a, b));
    }
    out
}

fn b_bits(r: &mut Rng) -> u32 {
    r.next() as u32
}

/// Pairs at the least normal's edge, where random bits almost never land: products whose significands multiply to just
/// under 2^47 or 2^48 at exponents putting them one place below the normals (where a product rounds up to the least
/// normal or is flushed), in the least normal's binade, and two places below; and sums of operands from the three
/// lowest binades, near cancellation or alike in sign, so their sums fall below the normals or just above.
fn edges() -> Vec<(u32, u32)> {
    let mut r = Rng(0xed6e);
    let mut out = Vec::new();
    while out.len() < 40_000 {
        let mb = r.next() & 0x7f_ffff;
        let top = r.next() & 1 == 1;
        let goal: u64 = if top { 1 << 48 } else { 1 << 47 };
        let ma = (goal / ((1 << 23) + mb)) as i64 - (1 << 23) + (r.next() % 5) as i64 - 3;
        if !(0..(1 << 23)).contains(&ma) {
            continue;
        }
        let e = [0i64, 0, 0, 1, -1][(r.next() % 5) as usize];
        let ea = 1 + (r.next() % 126) as i64;
        let eb = e + 127 - top as i64 - ea;
        if !(1..=254).contains(&eb) {
            continue;
        }
        let a = ((r.next() & 1) as u32) << 31 | (ea as u32) << 23 | ma as u32;
        let b = ((r.next() & 1) as u32) << 31 | (eb as u32) << 23 | mb as u32;
        out.push(if r.next() & 1 == 1 { (a, b) } else { (b, a) });
    }
    let mut r = Rng(0x5ed6);
    while out.len() < 80_000 {
        let a = ((r.next() & 1) as u32) << 31 | (1 + (r.next() % 3) as u32) << 23 | (r.next() & 0x7f_ffff) as u32;
        let b = match r.next() % 3 {
            0 => (a ^ 0x8000_0000).wrapping_add((r.next() % 17) as u32).wrapping_sub(8),
            1 => (a & 0x8000_0000) ^ 0x8000_0000 | (1 + (r.next() % 3) as u32) << 23 | (r.next() & 0x7f_ffff) as u32,
            _ => (a & 0x8000_0000) | (1 + (r.next() % 3) as u32) << 23 | (r.next() & 0x7f_ffff) as u32,
        };
        if (b >> 23) & 0xff != 0 {
            out.push((a, b));
        }
    }
    out
}

/// Every vector through both modules under iverilog: the products and sums, in order.
fn simulate(v: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let dir = tempfile::tempdir().unwrap();
    let mut hex = String::with_capacity(v.len() * 20);
    for (a, b) in v {
        writeln!(hex, "{a:08x}\n{b:08x}").unwrap();
    }
    std::fs::write(dir.path().join("vectors.hex"), hex).unwrap();
    std::fs::write(dir.path().join("fp32.v"), fp32_verilog()).unwrap();
    let n = v.len();
    let tb = format!(
        r#"module tb;
  reg [31:0] mem [0:{last}];
  reg [31:0] a, b;
  wire [31:0] m, s;
  logos_fmul um(a, b, m);
  logos_fadd ua(a, b, s);
  integer i, f;
  initial begin
    $readmemh("vectors.hex", mem);
    f = $fopen("out.hex", "w");
    for (i = 0; i < {n}; i = i + 1) begin
      a = mem[2*i]; b = mem[2*i+1]; #1;
      $fwrite(f, "%h %h\n", m, s);
    end
    $fclose(f);
    $finish;
  end
endmodule
"#,
        last = 2 * n - 1
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-o", "sim", "tb.v", "fp32.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    std::fs::read_to_string(dir.path().join("out.hex"))
        .unwrap()
        .lines()
        .map(|l| {
            let (m, s) = l.split_once(' ').unwrap();
            (u32::from_str_radix(m, 16).unwrap(), u32::from_str_radix(s, 16).unwrap())
        })
        .collect()
}

#[test]
fn the_rtl_multiplies_and_adds_as_the_numerics_do() {
    if !common::require_tool("iverilog") {
        return;
    }
    let v = [vectors(400_000), edges()].concat();
    let got = simulate(&v);
    assert_eq!(got.len(), v.len(), "every vector simulated");
    let mut wrong = Vec::new();
    for (&(a, b), &(m, s)) in v.iter().zip(&got) {
        let (fa, fb) = (f32::from_bits(a), f32::from_bits(b));
        let (wm, ws) = (fmul(fa, fb).to_bits(), fadd(fa, fb).to_bits());
        if m != wm {
            wrong.push(format!("fmul {a:08x} {b:08x}: rtl {m:08x}, numerics {wm:08x}"));
        }
        if s != ws {
            wrong.push(format!("fadd {a:08x} {b:08x}: rtl {s:08x}, numerics {ws:08x}"));
        }
    }
    assert!(wrong.is_empty(), "{} of {} vectors differ:\n{}", wrong.len(), 2 * v.len(), wrong.iter().take(20).cloned().collect::<Vec<_>>().join("\n"));
}

/// The pipelined multiply and add, a pair of operands a clock under iverilog: `logos_fmul_p`'s output each clock the
/// numerics' `fmul` of the pair three clocks before, `logos_fadd_p`'s the `fadd` of the pair four before, over every
/// vector the combinational units are held over, the least normal's edge among them, so every class of operand passes
/// every stage.
#[test]
fn the_pipelined_multiply_and_add_are_the_numerics_a_clock_at_a_time() {
    if !common::require_tool("iverilog") {
        return;
    }
    let v = [vectors(400_000), edges()].concat();
    let dir = tempfile::tempdir().unwrap();
    let mut hex = String::with_capacity(v.len() * 18);
    for (a, b) in &v {
        writeln!(hex, "{a:08x}{b:08x}").unwrap();
    }
    std::fs::write(dir.path().join("pairs.hex"), hex).unwrap();
    std::fs::write(dir.path().join("fp32.v"), fp32_verilog()).unwrap();
    let n = v.len();
    let tb = format!(
        r#"module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  reg [63:0] mem [0:{last}];
  reg [31:0] a = 0, b = 0;
  wire [31:0] m, s;
  logos_fmul_p pm(.clk(clk), .a(a), .b(b), .y(m));
  logos_fadd_p pa(.clk(clk), .a(a), .b(b), .y(s));
  integer i, f;
  initial begin
    $readmemh("pairs.hex", mem);
    f = $fopen("out.hex", "w");
    for (i = 0; i < {n} + 4; i = i + 1) begin
      @(negedge clk);
      if (i >= 4) $fwrite(f, "%h %h\n", m, s);
      if (i < {n}) {{a, b}} = mem[i];
    end
    $fclose(f);
    $finish;
  end
endmodule
"#,
        last = n - 1
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "fp32.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    let out = std::fs::read_to_string(dir.path().join("out.hex")).unwrap();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), n, "every pair's sum came out");
    // Line k, read at the negative edge after clock k + 4, holds the add of pair k and the multiply of pair k + 1.
    let mut wrong = Vec::new();
    for (k, line) in lines.iter().enumerate() {
        let (m, s) = line.split_once(' ').unwrap();
        let (m, s) = (u32::from_str_radix(m, 16).unwrap(), u32::from_str_radix(s, 16).unwrap());
        if let Some(&(a, b)) = v.get(k + 1) {
            let want = fmul(f32::from_bits(a), f32::from_bits(b)).to_bits();
            if m != want {
                wrong.push(format!("fmul {a:08x} {b:08x}: pipelined {m:08x}, numerics {want:08x}"));
            }
        }
        let (a, b) = v[k];
        let want = fadd(f32::from_bits(a), f32::from_bits(b)).to_bits();
        if s != want {
            wrong.push(format!("fadd {a:08x} {b:08x}: pipelined {s:08x}, numerics {want:08x}"));
        }
    }
    assert!(wrong.is_empty(), "{} results differ:\n{}", wrong.len(), wrong.iter().take(20).cloned().collect::<Vec<_>>().join("\n"));
}

/// The card's multiply on its block (`logos_fmul_p` built for `FpUnit::Hard`: the block's own multiply wrapped at the
/// soft unit's ports and latency, nothing corrected), the block a stand-in for the device's computing the card's class, a
/// pair of operands a clock under iverilog: each clock's product [`FpUnit::Hard`]'s of the pair three clocks before —
/// over every vector the soft units are held over, the least normal's edge, and the band where the card flushes what
/// the engine rounds into the least normal, `(1 − k·2^-24)·(1 + j·2^-23)·2^-126` about `k = 2j`.
#[test]
fn the_cards_multiply_on_its_block_is_the_cards_unit_a_clock_at_a_time() {
    use logicaffeine_compile::codegen_sva::fp_hard::Block;
    use logicaffeine_compile::codegen_sva::fp_hypotheses::Semantics;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{hard_float_model, hard_float_verilog, HardFloat};
    use logicaffeine_compile::spatial::decode_engine::rtl::fp32::{FADD_LATENCY, FMUL_LATENCY};
    if !common::require_tool("iverilog") {
        return;
    }
    let hard = HardFloat {
        product: Some(Block { semantics: FpUnit::CARD, full: 4, least: 1 }),
        sum: Block { semantics: Semantics::ENGINE, full: FADD_LATENCY, least: 1 },
        fp_unit: FpUnit::Hard,
    };
    let mut v = [vectors(200_000), edges()].concat();
    for j in 1300u32..2200 {
        for k in [2 * j - 1, 2 * j, 2 * j + 1] {
            let a = (1.0 - k as f64 * 2f64.powi(-24)) as f32;
            for sign in [0u32, 0x8000_0000] {
                v.push((sign | a.to_bits(), 0x0080_0000 | j));
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let mut hex = String::with_capacity(v.len() * 17);
    for (a, b) in &v {
        writeln!(hex, "{a:08x}{b:08x}").unwrap();
    }
    std::fs::write(dir.path().join("pairs.hex"), hex).unwrap();
    std::fs::write(dir.path().join("fp32.v"), format!("{}\n{}", hard_float_verilog(fp32_verilog(), &hard), hard_float_model(&hard))).unwrap();
    let n = v.len();
    let tb = format!(
        r#"module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  reg [63:0] mem [0:{last}];
  reg [31:0] a = 0, b = 0;
  wire [31:0] m;
  logos_fmul_p pm(.clk(clk), .a(a), .b(b), .y(m));
  integer i, f;
  initial begin
    $readmemh("pairs.hex", mem);
    f = $fopen("out.hex", "w");
    for (i = 0; i < {n} + {lat}; i = i + 1) begin
      @(negedge clk);
      if (i >= {lat}) $fwrite(f, "%h\n", m);
      if (i < {n}) {{a, b}} = mem[i];
    end
    $fclose(f);
    $finish;
  end
endmodule
"#,
        last = n - 1,
        lat = FMUL_LATENCY
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "fp32.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    let out = std::fs::read_to_string(dir.path().join("out.hex")).unwrap();
    let got: Vec<u32> = out.lines().map(|l| u32::from_str_radix(l.trim(), 16).unwrap()).collect();
    assert_eq!(got.len(), n, "every pair's product came out");
    let (mut wrong, mut band) = (Vec::new(), 0usize);
    for (k, (&(a, b), &m)) in v.iter().zip(&got).enumerate() {
        let want = FpUnit::Hard.mul(f32::from_bits(a), f32::from_bits(b)).to_bits();
        band += usize::from(want != fmul(f32::from_bits(a), f32::from_bits(b)).to_bits() && !f32::from_bits(want).is_nan());
        if m != want {
            wrong.push(format!("pair {k}: {a:08x} x {b:08x}: block {m:08x}, the card's unit {want:08x}"));
        }
    }
    assert!(band >= 100, "the band reached {band} times");
    assert!(wrong.is_empty(), "{} of {n} products differ:\n{}", wrong.len(), wrong.iter().take(20).cloned().collect::<Vec<_>>().join("\n"));
}

/// The pipelined tree of sixteen lanes, sixteen lanes a clock under iverilog: each clock's sum the numerics' `tree` of
/// the lanes [`TREE16_LATENCY`] clocks before, over lanes drawn from every vector the units are held over and over
/// lanes of one magnitude and either sign, whose sums cancel.
#[test]
fn the_pipelined_tree_folds_sixteen_lanes_as_the_numerics_do_a_clock_at_a_time() {
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0x7e16);
    let flat: Vec<u32> = vectors(160_000).into_iter().flat_map(|(a, b)| [a, b]).collect();
    let mut sets: Vec<[u32; 16]> = flat.chunks_exact(16).map(|c| c.try_into().unwrap()).collect();
    for _ in 0..20_000 {
        let base = 100 + (r.next() % 60) as u32;
        sets.push(std::array::from_fn(|_| ((r.next() as u32) & 0x8000_0000) | ((base + (r.next() % 4) as u32) << 23) | (r.next() as u32 & 0x7f_ffff)));
    }
    let dir = tempfile::tempdir().unwrap();
    let mut hex = String::with_capacity(sets.len() * 130);
    for set in &sets {
        for x in set.iter().rev() {
            write!(hex, "{x:08x}").unwrap();
        }
        hex.push('\n');
    }
    std::fs::write(dir.path().join("lanes.hex"), hex).unwrap();
    std::fs::write(dir.path().join("fp32.v"), fp32_verilog()).unwrap();
    let n = sets.len();
    let tb = format!(
        r#"module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  reg [511:0] mem [0:{last}];
  reg [511:0] lanes = 0;
  wire [31:0] sum;
  logos_tree16_p t(.clk(clk), .lanes(lanes), .sum(sum));
  integer i, f;
  initial begin
    $readmemh("lanes.hex", mem);
    f = $fopen("out.hex", "w");
    for (i = 0; i < {n} + {lat}; i = i + 1) begin
      @(negedge clk);
      if (i >= {lat}) $fwrite(f, "%h\n", sum);
      if (i < {n}) lanes = mem[i];
    end
    $fclose(f);
    $finish;
  end
endmodule
"#,
        last = n - 1,
        lat = TREE16_LATENCY,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "fp32.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    let out = std::fs::read_to_string(dir.path().join("out.hex")).unwrap();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), n, "every set's sum came out");
    let mut wrong = Vec::new();
    for (set, line) in sets.iter().zip(&lines) {
        let got = u32::from_str_radix(line, 16).unwrap();
        let want = tree(set.iter().map(|&x| f32::from_bits(x)).collect()).to_bits();
        if got != want {
            wrong.push(format!("{set:08x?}: pipelined {got:08x}, numerics {want:08x}"));
        }
    }
    assert!(wrong.is_empty(), "{} of {n} sums differ:\n{}", wrong.len(), wrong.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
}

/// The boundary the flush is defined at: a product whose exact value lies just below the least normal rounds, at the
/// denormals' precision, either up to the least normal (kept) or down (flushed).
#[test]
fn a_result_rounding_up_to_the_least_normal_survives_the_flush() {
    if !common::require_tool("iverilog") {
        return;
    }
    // (1 − (t+1)·2^-24)·2^-126·(1 + j·2^-23) ≈ 2^-126·(1 + (2j − t − 1)·2^-24): just below the least normal, on it and
    // just above, at two exponent pairs, in both operand orders. t = 0, j = 0 is the exact tie 2^-126 − 2^-150.
    let mut cases: Vec<(u32, u32)> = Vec::new();
    for (one, least) in [(0x3f7f_ffffu32, 0x0080_0000u32), (0x3eff_ffff, 0x0100_0000)] {
        for t in 0..64 {
            for j in 0..64 {
                cases.push((one - t, least + j));
                cases.push((least + j, one - t));
            }
        }
    }
    let results: Vec<u32> = cases.iter().map(|&(a, b)| fmul(f32::from_bits(a), f32::from_bits(b)).to_bits()).collect();
    let survived = results.iter().filter(|&&r| r == 0x0080_0000).count();
    let flushed = results.iter().filter(|&&r| r == 0).count();
    assert!(survived > 0 && flushed > 0, "the cases straddle the boundary: {survived} kept at the least normal, {flushed} flushed");
    let got = simulate(&cases);
    for (&(a, b), &(m, _)) in cases.iter().zip(&got) {
        assert_eq!(m, fmul(f32::from_bits(a), f32::from_bits(b)).to_bits(), "{a:08x} × {b:08x}");
    }
}

// ── The activation quantizer ───────────────────────────────────────────────────────────────────────

/// The quantizer's closed-form scale is `block_scale`'s least scale for every significand: the form depends on the
/// significand alone, the exponent only shifting it, so every significand at two exponents covers every normal amax.
#[test]
fn the_closed_form_scale_is_the_least_scale_for_every_significand() {
    for e in [0i32, -37] {
        for s in (1u32 << 23)..(1u32 << 24) {
            let amax = f32::from_bits((((e + 127) as u32) << 23) | (s & 0x7f_ffff));
            assert_eq!(closed_form_scale(s, e), block_scale(amax), "significand {s:#x} at 2^{e}");
        }
    }
}

/// Seeded activation blocks through `logos_act_block`: every block's scale and code, one word and two.
#[test]
fn the_rtl_quantizes_a_block_as_the_numerics_do() {
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0xac7);
    let mut blocks: Vec<Vec<f32>> = Vec::new();
    for b in 0..1200 {
        let scale = 2f32.powi((r.next() % 50) as i32 - 30);
        let mut block: Vec<f32> = (0..ACT_BLOCK)
            .map(|_| ((r.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * scale)
            .collect();
        match b % 7 {
            // The scale pinned to 11·2^-20 by a largest element of exactly 127 of it, every other element exactly
            // halfway between two codes: ties to even.
            0 => {
                let unit = 11.0 * 2f32.powi(-20);
                block[0] = 127.0 * unit;
                for (i, v) in block.iter_mut().enumerate().skip(1) {
                    *v = (i as f32 - 15.5) * unit;
                }
            }
            1 => block.iter_mut().for_each(|v| *v = 0.0),
            2 => block[3] = f32::from_bits(0x0000_0042),
            _ => {}
        }
        blocks.push(block);
    }
    let dir = tempfile::tempdir().unwrap();
    let mut hex = String::new();
    for block in &blocks {
        for two in [0u8, 1] {
            for v in block.iter().rev() {
                write!(hex, "{:08x}", v.to_bits()).unwrap();
            }
            writeln!(hex, "\n{two}").unwrap();
        }
    }
    std::fs::write(dir.path().join("vectors.hex"), hex).unwrap();
    std::fs::write(dir.path().join("act.v"), act_quant_verilog()).unwrap();
    let n = blocks.len() * 2;
    let tb = format!(
        r#"module tb;
  reg [1023:0] xs [0:{last}];
  reg [1023:0] x;
  reg two;
  wire [3:0] m;
  wire signed [11:0] e;
  wire [511:0] codes;
  logos_act_block u(.x(x), .two(two), .m(m), .e(e), .codes(codes));
  integer i, f;
  reg [1023:0] word;
  initial begin
    $readmemh("vectors.hex", xs);
    f = $fopen("out.hex", "w");
    for (i = 0; i < {n}; i = i + 1) begin
      x = xs[2*i]; word = xs[2*i+1]; two = word[0]; #1;
      $fwrite(f, "%h %h %h\n", m, e, codes);
    end
    $fclose(f);
    $finish;
  end
endmodule
"#,
        last = 2 * n - 1
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-o", "sim", "tb.v", "act.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    let out = std::fs::read_to_string(dir.path().join("out.hex")).unwrap();
    let mut lines = out.lines();
    let mut wrong = Vec::new();
    for (b, block) in blocks.iter().enumerate() {
        for words in [Words::One, Words::Two] {
            let line = lines.next().expect("a line a vector");
            let mut parts = line.split(' ');
            let m = i64::from_str_radix(parts.next().unwrap(), 16).unwrap();
            let e = i64::from_str_radix(parts.next().unwrap(), 16).unwrap() as i32;
            let e = if e >= 0x800 { e - 0x1000 } else { e };
            let codes_hex = parts.next().unwrap();
            let codes: Vec<i32> = (0..ACT_BLOCK)
                .map(|i| {
                    let at = codes_hex.len() - 4 * (i + 1);
                    i32::from(u16::from_str_radix(&codes_hex[at..at + 4], 16).unwrap() as i16)
                })
                .collect();
            let mut padded = block.clone();
            padded.resize(256, 0.0);
            let want = quantize(&padded, words);
            let (ws, wc) = (want.scales[0], &want.codes[..ACT_BLOCK]);
            if (m, e) != (ws.m, ws.e) || codes != wc {
                wrong.push(format!("block {b} {words:?}: rtl {m}·2^{e} {:?}, numerics {}·2^{} {:?}", &codes[..4], ws.m, ws.e, &wc[..4]));
            }
        }
    }
    assert!(wrong.is_empty(), "{} of {n} blocks differ:\n{}", wrong.len(), wrong.iter().take(10).cloned().collect::<Vec<_>>().join("\n"));
}

/// `logos_wide_scale` under iverilog is `wide_scale` at every rounded significand its two ceilings over 127 see: a
/// significand for each value the nine-place rounding reaches (2^14 to 2^15) and each the ten-place one reaches, at either
/// side of each rounding step, at two exponents, the exponent eight lower for a two-word code; and zero is 128·2^0.
#[test]
fn the_rtl_block_scale_is_wide_scale_at_every_rounded_significand() {
    if !common::require_tool("iverilog") {
        return;
    }
    let mut sig: Vec<u32> = Vec::new();
    for k in 0..=(1u32 << 14) {
        for at in [(1u32 << 23) + k * 512, (1 << 23) + k * 512 + 511] {
            sig.push(at.saturating_sub(511).clamp(1 << 23, (1 << 24) - 1));
            sig.push(at.clamp(1 << 23, (1 << 24) - 1));
        }
    }
    for k in 0..=(1u32 << 13) {
        for at in [(1u32 << 23) + k * 1024, (1 << 23) + k * 1024 + 1023] {
            sig.push(at.saturating_sub(1023).clamp(1 << 23, (1 << 24) - 1));
            sig.push(at.clamp(1 << 23, (1 << 24) - 1));
        }
    }
    sig.sort_unstable();
    sig.dedup();
    let mut amax: Vec<u32> = vec![0];
    for e in [127u32, 90] {
        amax.extend(sig.iter().map(|s| e << 23 | (s & 0x7f_ffff)));
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("amax.hex"), amax.iter().map(|a| format!("{a:08x}\n")).collect::<String>()).unwrap();
    std::fs::write(dir.path().join("aq.v"), act_quant_verilog()).unwrap();
    let n = amax.len();
    let tb = format!(
        r#"module tb;
  reg [31:0] mem [0:{last}];
  reg [30:0] amax = 0;
  reg two = 0;
  wire [7:0] m;
  wire signed [11:0] e;
  logos_wide_scale u(.amax(amax), .two(two), .m(m), .e(e));
  integer i, t;
  initial begin
    $readmemh("amax.hex", mem);
    for (i = 0; i < {n}; i = i + 1)
      for (t = 0; t < 2; t = t + 1) begin
        amax = mem[i][30:0];
        two = t;
        #1 $display("S %0d %0d %0d %0d", i, t, m, e);
      end
    $finish;
  end
endmodule
"#,
        last = n - 1,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "aq.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    let out = String::from_utf8_lossy(&run.stdout).to_string();
    let mut seen = 0;
    let mut wrong = Vec::new();
    for line in out.lines().filter_map(|l| l.strip_prefix("S ")) {
        let f: Vec<i64> = line.split_whitespace().map(|x| x.parse().unwrap()).collect();
        let (i, two, m, e) = (f[0] as usize, f[1] == 1, f[2], f[3]);
        let want = if amax[i] == 0 { ActScale { m: 128, e: 0 } } else { wide_scale(f32::from_bits(amax[i])) };
        let want_e = if amax[i] != 0 && two { want.e - 8 } else { want.e };
        if (m, e as i32) != (want.m, want_e) {
            wrong.push(format!("amax {:#010x} two {two}: rtl {m}·2^{e}, wide_scale {}·2^{want_e}", amax[i], want.m));
        }
        seen += 1;
    }
    assert_eq!(seen, 2 * n, "{out}");
    assert!(wrong.is_empty(), "{} of {} scales differ:\n{}", wrong.len(), 2 * n, wrong.iter().take(10).cloned().collect::<Vec<_>>().join("\n"));
}

/// `logos_wide_scale_p`, an operand a clock under the RTL simulator, beside `logos_wide_scale`: every scale it gives
/// [`WIDE_SCALE_P_LATENCY`] clocks after its operands is the combinational unit's of them, in both word counts — every
/// significand at either side of each step of the two ceilings' roundings at four exponents (the least and greatest
/// normal among them), zero, and seeded magnitudes of every exponent, denormals among them.
#[test]
fn the_pipelined_block_scale_is_the_block_scale_a_clock_at_a_time() {
    use logicaffeine_compile::spatial::decode_engine::rtl::act_quant::WIDE_SCALE_P_LATENCY;
    if !common::require_tool("iverilog") {
        return;
    }
    let mut amax: Vec<u32> = vec![0];
    for (k, step) in [(1u32 << 14, 512u32), (1 << 13, 1024)] {
        for i in 0..=k {
            for s in [(1u32 << 23) + i * step, (1 << 23) + i * step + step - 1] {
                for near in [s.saturating_sub(step - 1), s] {
                    for e in [1u32, 90, 127, 254] {
                        amax.push(e << 23 | (near.clamp(1 << 23, (1 << 24) - 1) & 0x7f_ffff));
                    }
                }
            }
        }
    }
    let mut r = Rng(0x5ca1e);
    for _ in 0..50_000 {
        let x = r.next() as u32;
        amax.push(if (x >> 23) & 0xff == 0xff { x & 0x7f7f_ffff } else { x & 0x7fff_ffff });
        amax.push((r.next() as u32) & 0x7f_ffff);
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("amax.hex"), amax.iter().map(|a| format!("{a:08x}\n")).collect::<String>()).unwrap();
    std::fs::write(dir.path().join("aq.v"), act_quant_verilog()).unwrap();
    let n = amax.len();
    let tb = format!(
        r#"module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  reg [31:0] mem [0:{last}];
  reg [30:0] amax = 0;
  reg two = 0;
  wire [7:0] mw, mp;
  wire signed [11:0] ew, ep;
  reg [19:0] late [0:{lat}];
  logos_wide_scale comb(.amax(amax), .two(two), .m(mw), .e(ew));
  logos_wide_scale_p u(.clk(clk), .amax(amax), .two(two), .m(mp), .e(ep));
  integer i, k, f;
  initial begin
    $readmemh("amax.hex", mem);
    f = $fopen("out.txt", "w");
    for (i = 0; i < 2 * {n} + {lat}; i = i + 1) begin
      @(negedge clk);
      if (i >= {lat}) $fwrite(f, "%0d %0d %0d %0d %0d\n", i - {lat}, mp, ep, late[{lat} - 1][19:12], $signed(late[{lat} - 1][11:0]));
      for (k = {lat} - 1; k > 0; k = k - 1) late[k] = late[k - 1];
      if (i < 2 * {n}) begin amax = mem[i >> 1][30:0]; two = i[0]; end
      #1 late[0] = {{mw, ew}};
    end
    $fclose(f);
    $finish;
  end
endmodule
"#,
        last = n - 1,
        lat = WIDE_SCALE_P_LATENCY,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "aq.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    let out = std::fs::read_to_string(dir.path().join("out.txt")).unwrap();
    let mut seen = 0;
    let mut wrong = Vec::new();
    for line in out.lines() {
        let f: Vec<i64> = line.split_whitespace().map(|x| x.parse().unwrap()).collect();
        let (k, got, want) = (f[0] as usize, (f[1], f[2]), (f[3], f[4]));
        if got != want {
            wrong.push(format!("amax {:#010x} two {}: pipelined {}·2^{}, combinational {}·2^{}", amax[k / 2], k % 2, got.0, got.1, want.0, want.1));
        }
        seen += 1;
    }
    assert_eq!(seen, 2 * n, "a scale a clock for every operand");
    assert!(wrong.is_empty(), "{} of {} scales differ:\n{}", wrong.len(), 2 * n, wrong.iter().take(10).cloned().collect::<Vec<_>>().join("\n"));
}

/// The per-block quantizer's closed form is `wide_scale`'s least 8-bit-significand scale for every significand.
#[test]
fn the_wide_closed_form_scale_is_the_least_scale_for_every_significand() {
    for e in [0i32, -37] {
        for s in (1u32 << 23)..(1u32 << 24) {
            let amax = f32::from_bits((((e + 127) as u32) << 23) | (s & 0x7f_ffff));
            assert_eq!(closed_form_wide_scale(s, e), wide_scale(amax), "significand {s:#x} at 2^{e}");
        }
    }
}

/// Seeded weight blocks of each length through `logos_act_wide`: every block's scale and code, one word and two.
#[test]
fn the_rtl_quantizes_a_weight_block_as_the_numerics_do() {
    if !common::require_tool("iverilog") {
        return;
    }
    for (lanes, count) in [(32usize, 800usize), (256, 120)] {
        let mut r = Rng(0xb10c + lanes as u64);
        let mut blocks: Vec<Vec<f32>> = Vec::new();
        for b in 0..count {
            let scale = 2f32.powi((r.next() % 50) as i32 - 30);
            let mut block: Vec<f32> = (0..lanes).map(|_| ((r.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * scale).collect();
            match b % 7 {
                // The scale pinned to 200·2^-20 by a largest element of exactly 127 of it, the rest exactly halfway
                // between two codes: ties to even.
                0 => {
                    let unit = 200.0 * 2f32.powi(-20);
                    block[0] = 127.0 * unit;
                    for (i, v) in block.iter_mut().enumerate().skip(1) {
                        *v = ((i % 64) as f32 - 31.5) * unit;
                    }
                }
                1 => block.iter_mut().for_each(|v| *v = 0.0),
                2 => block[3] = f32::from_bits(0x0000_0042),
                _ => {}
            }
            blocks.push(block);
        }
        let dir = tempfile::tempdir().unwrap();
        let mut hex = String::new();
        for block in &blocks {
            for two in [0u8, 1] {
                for v in block.iter().rev() {
                    write!(hex, "{:08x}", v.to_bits()).unwrap();
                }
                writeln!(hex, "\n{two}").unwrap();
            }
        }
        std::fs::write(dir.path().join("vectors.hex"), hex).unwrap();
        std::fs::write(dir.path().join("act.v"), act_quant_verilog()).unwrap();
        let n = blocks.len() * 2;
        let tb = format!(
            r#"module tb;
  reg [{xw}:0] xs [0:{last}];
  reg [{xw}:0] x;
  reg two;
  wire [7:0] m;
  wire signed [11:0] e;
  wire [{cw}:0] codes;
  logos_act_wide #(.LANES({lanes})) u(.x(x), .two(two), .m(m), .e(e), .codes(codes));
  integer i, f;
  reg [{xw}:0] word;
  initial begin
    $readmemh("vectors.hex", xs);
    f = $fopen("out.hex", "w");
    for (i = 0; i < {n}; i = i + 1) begin
      x = xs[2*i]; word = xs[2*i+1]; two = word[0]; #1;
      $fwrite(f, "%h %h %h\n", m, e, codes);
    end
    $fclose(f);
    $finish;
  end
endmodule
"#,
            xw = 32 * lanes - 1,
            cw = 16 * lanes - 1,
            last = 2 * n - 1
        );
        std::fs::write(dir.path().join("tb.v"), tb).unwrap();
        let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-o", "sim", "tb.v", "act.v"]).output().expect("iverilog");
        assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
        let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
        assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
        let out = std::fs::read_to_string(dir.path().join("out.hex")).unwrap();
        let mut lines = out.lines();
        let mut wrong = Vec::new();
        for (b, block) in blocks.iter().enumerate() {
            for words in [Words::One, Words::Two] {
                let line = lines.next().expect("a line a vector");
                let mut parts = line.split(' ');
                let m = i64::from_str_radix(parts.next().unwrap(), 16).unwrap();
                let e = i64::from_str_radix(parts.next().unwrap(), 16).unwrap() as i32;
                let e = if e >= 0x800 { e - 0x1000 } else { e };
                let codes_hex = parts.next().unwrap();
                let codes: Vec<i32> = (0..lanes).map(|i| i32::from(hex_field(codes_hex, i, 4) as u16 as i16)).collect();
                let want = quantize_blocks(block, words, lanes);
                let ws = want.scales[0];
                if (m, e) != (ws.m, ws.e) || codes != want.codes {
                    wrong.push(format!("{lanes}-block {b} {words:?}: rtl {m}·2^{e} {:?}, numerics {}·2^{} {:?}", &codes[..4], ws.m, ws.e, &want.codes[..4]));
                }
            }
        }
        assert!(wrong.is_empty(), "{} of {n} blocks differ:\n{}", wrong.len(), wrong.iter().take(10).cloned().collect::<Vec<_>>().join("\n"));
    }
}

/// The pipelined code unit `logos_act_code_p`, a value a clock under the RTL simulator, beside `logos_act_code`: every
/// code it gives `ACT_CODE_LATENCY` clocks after its operands is the combinational unit's of them — seeded values,
/// significands from 2 to 255 (the 4-bit scales' 8 to 15 and the 8-bit scales' 128 to 255 within them) and exponents of
/// every class, values on and beside the halves between codes, past the word's limit and far below the scale, denormals
/// and zeros of both signs, both word counts.
#[test]
fn the_pipelined_code_unit_is_the_code_unit_a_clock_at_a_time() {
    use logicaffeine_compile::spatial::decode_engine::rtl::streaming::{act_stream_p_verilog, ACT_CODE_LATENCY};
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0xc0de);
    let mut ops: Vec<(u32, u8, i16, bool)> = Vec::new();
    for i in 0..200_000u32 {
        let m = match i % 4 {
            0 => 128 + (r.next() % 128) as u8,
            1 => [2u8, 8, 9, 15, 127, 128, 129, 254, 255][(r.next() % 9) as usize],
            _ => 2 + (r.next() % 254) as u8,
        };
        let e = ((r.next() % 300) as i16) - 180;
        let x = match i % 7 {
            // A value a whole number and a half of the scale, or just beside it: ties and their neighbours.
            0 => {
                let q = (r.next() % 300) as f64 + 0.5;
                let v = q * f64::from(m) * 2f64.powi(i32::from(e));
                let bits = (v as f32).to_bits();
                bits.wrapping_add((r.next() % 3) as u32).wrapping_sub(1) | ((r.next() as u32 & 1) << 31)
            }
            1 => (r.next() as u32) & 0x807f_ffff,
            2 => 0x8000_0000 * (r.next() as u32 & 1),
            _ => r.next() as u32,
        };
        let x = if (x >> 23) & 0xff == 0xff { x & 0xff7f_ffff } else { x };
        ops.push((x, m, e, r.next() % 2 == 0));
    }
    let mut hex = String::new();
    for &(x, m, e, two) in &ops {
        writeln!(hex, "{x:08x}{m:02x}{:03x}{}", e as u16 & 0xfff, u8::from(two)).unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ops.hex"), hex).unwrap();
    std::fs::write(dir.path().join("act.v"), act_stream_p_verilog()).unwrap();
    let n = ops.len();
    let tb = format!(
        r#"module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  reg [55:0] mem [0:{last}];
  reg [31:0] x = 0;
  reg [7:0] m = 1;
  reg signed [11:0] e = 0;
  reg two = 0;
  wire signed [15:0] want, got;
  reg signed [15:0] late [0:{lat}];
  logos_act_code comb(.x(x), .m(m), .e(e), .two(two), .code(want));
  logos_act_code_p u(.clk(clk), .x(x), .m(m), .e(e), .two(two), .code(got));
  integer i, k, f;
  initial begin
    $readmemh("ops.hex", mem);
    f = $fopen("out.txt", "w");
    for (i = 0; i < {n} + {lat}; i = i + 1) begin
      @(negedge clk);
      if (i >= {lat} && got != late[{lat} - 1]) $fwrite(f, "%0d %h %h\n", i - {lat}, got, late[{lat} - 1]);
      for (k = {lat} - 1; k > 0; k = k - 1) late[k] = late[k - 1];
      if (i < {n}) begin x = mem[i][55:24]; m = mem[i][23:16]; e = mem[i][15:4]; two = mem[i][0]; end
      #1 late[0] = want;
    end
    $fclose(f);
    $finish;
  end
endmodule
"#,
        last = n - 1,
        lat = ACT_CODE_LATENCY,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let run = rtl_sim::Simulator::build(dir.path(), &["act.v", "tb.v"]).run(dir.path(), &[]);
    assert!(run.status.success(), "simulation: {}", String::from_utf8_lossy(&run.stderr));
    let out = std::fs::read_to_string(dir.path().join("out.txt")).unwrap();
    let wrong: Vec<String> = out
        .lines()
        .take(16)
        .map(|l| {
            let f: Vec<&str> = l.split(' ').collect();
            let k: usize = f[0].parse().unwrap();
            let (x, m, e, two) = ops[k];
            format!("x {x:08x} m {m} e {e} two {two}: pipelined {}, combinational {}", f[1], f[2])
        })
        .collect();
    assert!(wrong.is_empty(), "{} codes differ:\n{}", out.lines().count(), wrong.join("\n"));
}

/// The engine's one quantizer, `logos_act_stream`, and the streaming engine's, `logos_act_stream_p`, over a stream of
/// blocks of every length they serve, back to back: each block's beats arrive with idle cycles between them, `go` comes
/// one to three cycles after the last, and the next block's first beat as soon as `busy` falls. Every block's scale and
/// codes are `quantize_blocks`' in both word counts, whatever block came before it.
#[test]
fn the_streaming_quantizer_quantizes_each_block_as_the_numerics_do() {
    use logicaffeine_compile::spatial::decode_engine::rtl::streaming::act_stream_p_verilog;
    if !common::require_tool("iverilog") {
        return;
    }
    let runs = [("logos_act_stream", 256usize, &[32usize, 64, 256][..], 240usize), ("logos_act_stream", 32, &[16, 32][..], 400), ("logos_act_stream_p", 256, &[32, 64, 256][..], 240), ("logos_act_stream_p", 32, &[16, 32][..], 400)];
    for (module, lanes, lengths, count) in runs {
        let mut r = Rng(0x57ea + lanes as u64);
        let mut blocks: Vec<(Vec<f32>, Words)> = Vec::new();
        for b in 0..count {
            let len = lengths[(r.next() % lengths.len() as u64) as usize];
            let scale = 2f32.powi((r.next() % 50) as i32 - 30);
            let mut block: Vec<f32> = (0..len).map(|_| ((r.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * scale).collect();
            match b % 9 {
                // The scale pinned to 200·2^-20 by a largest element of exactly 127 of it, the rest exactly halfway
                // between two codes: ties to even.
                0 => {
                    let unit = 200.0 * 2f32.powi(-20);
                    block[0] = 127.0 * unit;
                    for (i, v) in block.iter_mut().enumerate().skip(1) {
                        *v = ((i % 64) as f32 - 31.5) * unit;
                    }
                }
                1 => block.iter_mut().for_each(|v| *v = 0.0),
                2 => block[3] = f32::from_bits(0x0000_0042),
                // Only denormals and signed zeros: no scale of its own.
                3 => block.iter_mut().enumerate().for_each(|(i, v)| *v = f32::from_bits(((i as u32 % 2) << 31) | (r.next() as u32 & 0x7f_ffff))),
                // The largest magnitude in the last beat, negative.
                4 => {
                    let last = block.len() - 1;
                    block[last] = -4.0 * scale;
                }
                _ => {}
            }
            let words = if r.next() % 2 == 0 { Words::One } else { Words::Two };
            blocks.push((block, words));
        }
        let mut stim = String::new();
        for (block, words) in &blocks {
            writeln!(stim, "{} {} {}", block.len() / 16, u8::from(*words == Words::Two), 1 + r.next() % 3).unwrap();
            for beat in block.chunks(16) {
                write!(stim, "{} ", r.next() % 3).unwrap();
                for v in beat.iter().rev() {
                    write!(stim, "{:08x}", v.to_bits()).unwrap();
                }
                stim.push('\n');
            }
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("stim.txt"), stim).unwrap();
        std::fs::write(dir.path().join("act.v"), if module == "logos_act_stream" { act_quant_verilog() } else { act_stream_p_verilog() }).unwrap();
        let tb = format!(
            r#"module tb;
  reg clk = 0, rst = 1, clear = 0, in_v = 0, go = 0, two = 0;
  reg [511:0] in_beat = 0;
  wire busy;
  wire [7:0] m;
  wire signed [11:0] e;
  wire [{cw}:0] codes;
  {module} #(.LANES({lanes})) u(.clk(clk), .rst(rst), .clear(clear), .in_v(in_v), .in_beat(in_beat), .go(go),
    .two(two), .busy(busy), .m(m), .e(e), .codes(codes));
  always #5 clk = !clk;
  integer fd, f, b, j, beats, words, gap, wait_go, got;
  reg [511:0] beat;
  initial begin
    fd = $fopen("stim.txt", "r");
    f = $fopen("out.hex", "w");
    @(posedge clk); #1; rst = 0;
    for (b = 0; b < {count}; b = b + 1) begin
      got = $fscanf(fd, "%d %d %d\n", beats, words, wait_go);
      for (j = 0; j < beats; j = j + 1) begin
        got = $fscanf(fd, "%d %h\n", gap, beat);
        repeat (gap) @(posedge clk);
        #1; clear = (j == 0); in_v = 1; in_beat = beat;
        @(posedge clk); #1; clear = 0; in_v = 0; in_beat = {{16{{32'hdeadbeef}}}};
      end
      repeat (wait_go - 1) @(posedge clk);
      #1; go = 1; two = words[0];
      @(posedge clk); #1; go = 0; two = !words[0];
      while (busy) begin @(posedge clk); #1; end
      $fwrite(f, "%h %h %h\n", m, e, codes);
    end
    $fclose(f);
    $finish;
  end
endmodule
"#,
            cw = 16 * lanes - 1,
        );
        std::fs::write(dir.path().join("tb.v"), tb).unwrap();
        let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "act.v"]).output().expect("iverilog");
        assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
        let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
        assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
        let out = std::fs::read_to_string(dir.path().join("out.hex")).unwrap();
        let mut lines = out.lines();
        let mut wrong = Vec::new();
        for (b, (block, words)) in blocks.iter().enumerate() {
            let line = lines.next().expect("a line a block");
            if line.contains(['x', 'z']) {
                wrong.push(format!("{lanes}-lane unit, {}-block {b} {words:?}: rtl left bits unknown", block.len()));
                continue;
            }
            let mut parts = line.split(' ');
            let m = i64::from_str_radix(parts.next().unwrap(), 16).unwrap();
            let e = i64::from_str_radix(parts.next().unwrap(), 16).unwrap() as i32;
            let e = if e >= 0x800 { e - 0x1000 } else { e };
            let codes_hex = parts.next().unwrap();
            let codes: Vec<i32> = (0..block.len()).map(|i| i32::from(hex_field(codes_hex, i, 4) as u16 as i16)).collect();
            let want = quantize_blocks(block, *words, block.len());
            let ws = want.scales[0];
            if (m, e) != (ws.m, ws.e) || codes != want.codes {
                wrong.push(format!(
                    "{lanes}-lane unit, {}-block {b} {words:?}: rtl {m}·2^{e} {:?}, numerics {}·2^{} {:?}",
                    block.len(),
                    &codes[..4],
                    ws.m,
                    ws.e,
                    &want.codes[..4]
                ));
            }
        }
        assert!(wrong.is_empty(), "{} of {count} blocks differ:\n{}", wrong.len(), wrong.iter().take(10).cloned().collect::<Vec<_>>().join("\n"));
    }
}

/// The streaming matvec's quantizer, `logos_act_blocks_p`, over blocks back to back: each block of every length it
/// serves arriving a beat a clock or at gaps, its first marked and its last marked with its word count and its tag,
/// the next block's first as soon as the clock after the last or later; runs of blocks that fill its ring the most
/// (a widest block, then single-beat blocks, none of them apart), and of blocks of one length end to end. Every block
/// comes out once, in order, its scale, codes and tag `quantize_blocks`' and its own, on exactly the clock
/// `act_blocks_out` gives; blocks of one length end to end come out that many clocks apart, a beat a clock.
#[test]
fn the_block_quantizer_quantizes_blocks_back_to_back_as_the_numerics_do() {
    use logicaffeine_compile::spatial::decode_engine::rtl::streaming::{act_blocks_out, act_stream_p_verilog};
    if !common::require_tool("iverilog") {
        return;
    }
    for (lanes, lengths) in [(256usize, &[16usize, 32, 64, 256][..]), (32, &[16, 32][..])] {
        let mut r = Rng(0xb10c + lanes as u64);
        // Each block: its values, its word count, the clocks before each of its beats.
        let mut blocks: Vec<(Vec<f32>, Words, Vec<usize>)> = Vec::new();
        let push = |blocks: &mut Vec<(Vec<f32>, Words, Vec<usize>)>, r: &mut Rng, len: usize, gaps: &dyn Fn(&mut Rng, usize) -> usize, special: usize| {
            let scale = 2f32.powi((r.next() % 50) as i32 - 30);
            let mut block: Vec<f32> = (0..len).map(|_| ((r.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * scale).collect();
            match special % 7 {
                0 => {
                    let unit = 200.0 * 2f32.powi(-20);
                    block[0] = 127.0 * unit;
                    for (i, v) in block.iter_mut().enumerate().skip(1) {
                        *v = ((i % 64) as f32 - 31.5) * unit;
                    }
                }
                1 => block.iter_mut().for_each(|v| *v = 0.0),
                2 => block.iter_mut().enumerate().for_each(|(i, v)| *v = f32::from_bits(((i as u32 % 2) << 31) | (r.next() as u32 & 0x7f_ffff))),
                3 => {
                    let last = block.len() - 1;
                    block[last] = -4.0 * scale;
                }
                _ => {}
            }
            let words = if r.next() % 2 == 0 { Words::One } else { Words::Two };
            let gaps = (0..len / 16).map(|j| gaps(r, j)).collect();
            blocks.push((block, words, gaps));
        };
        // Random lengths at random gaps, none of them apart a third of the time.
        for b in 0..240 {
            let len = lengths[(r.next() % lengths.len() as u64) as usize];
            let tight = r.next() % 3 == 0;
            push(&mut blocks, &mut r, len, &|r: &mut Rng, j| if tight { 0 } else if j == 0 { (r.next() % 4) as usize } else { (r.next() % 3) as usize }, b);
        }
        // The ring at its fullest: a widest block, then single-beat blocks, nothing between them.
        for k in 0..3 {
            push(&mut blocks, &mut r, lanes, &|_: &mut Rng, _| 0, k);
            for b in 0..40 {
                push(&mut blocks, &mut r, 16, &|_: &mut Rng, _| 0, b + 4);
            }
        }
        // Blocks of one length end to end: the throughput run, its out_v a block's beats apart.
        let run_at = blocks.len();
        for b in 0..24 {
            push(&mut blocks, &mut r, lanes, &|_: &mut Rng, _| 0, b + 4);
        }
        let mut stim = String::new();
        for (block, words, gaps) in &blocks {
            writeln!(stim, "{} {}", block.len() / 16, u8::from(*words == Words::Two)).unwrap();
            for (beat, gap) in block.chunks(16).zip(gaps) {
                write!(stim, "{gap} ").unwrap();
                for v in beat.iter().rev() {
                    write!(stim, "{:08x}", v.to_bits()).unwrap();
                }
                stim.push('\n');
            }
        }
        let count = blocks.len();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("stim.txt"), stim).unwrap();
        std::fs::write(dir.path().join("act.v"), act_stream_p_verilog()).unwrap();
        let tb = format!(
            r#"module tb;
  reg clk = 0, rst = 1, in_v = 0, in_first = 0, in_last = 0, in_two = 0;
  reg [511:0] in_beat = 0;
  reg [23:0] in_tag = 0;
  wire busy, out_v;
  wire [7:0] m;
  wire signed [11:0] e;
  wire [23:0] tag;
  wire [{cw}:0] codes;
  logos_act_blocks_p #(.LANES({lanes}), .TW(24)) u(.clk(clk), .rst(rst), .in_v(in_v), .in_beat(in_beat), .in_first(in_first),
    .in_last(in_last), .in_two(in_two), .in_tag(in_tag), .busy(busy), .out_v(out_v), .out_m(m), .out_e(e), .out_tag(tag),
    .codes(codes));
  always #5 clk = !clk;
  integer fd, f, b, j, beats, words, gap, got, clock = 0;
  reg [511:0] beat;
  always @(posedge clk) clock <= clock + 1;
  always @(negedge clk) if (out_v) $fwrite(f, "%0d %h %h %h %h\n", clock, tag, m, e, codes);
  initial begin
    fd = $fopen("stim.txt", "r");
    f = $fopen("out.hex", "w");
    @(posedge clk); #1; rst = 0;
    for (b = 0; b < {count}; b = b + 1) begin
      got = $fscanf(fd, "%d %d\n", beats, words);
      for (j = 0; j < beats; j = j + 1) begin
        got = $fscanf(fd, "%d %h\n", gap, beat);
        repeat (gap) @(posedge clk);
        #1; in_v = 1; in_first = j == 0; in_last = j == beats - 1; in_two = words[0]; in_tag = b; in_beat = beat;
        $fwrite(f, "B %0d %0d\n", b, clock);
        @(posedge clk); #1; in_v = 0; in_first = 0; in_last = 0; in_two = !words[0]; in_tag = 24'hffffff; in_beat = {{16{{32'hdeadbeef}}}};
      end
    end
    while (busy) @(posedge clk);
    repeat (4) @(posedge clk);
    $fclose(f);
    $finish;
  end
endmodule
"#,
            cw = 16 * lanes - 1,
        );
        std::fs::write(dir.path().join("tb.v"), tb).unwrap();
        let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "act.v"]).output().expect("iverilog");
        assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
        let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
        assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
        let out = std::fs::read_to_string(dir.path().join("out.hex")).unwrap();
        // The clock each block's last beat was taken at, from the testbench's own record of each beat.
        let mut last = vec![0usize; count];
        let mut outs = Vec::new();
        for l in out.lines() {
            let f: Vec<&str> = l.split(' ').collect();
            if f[0] == "B" {
                last[f[1].parse::<usize>().unwrap()] = f[2].parse().unwrap();
            } else {
                outs.push(f.iter().map(|s| s.to_string()).collect::<Vec<_>>());
            }
        }
        assert_eq!(outs.len(), count, "{lanes}-lane unit: every block out once");
        let model = act_blocks_out(&blocks.iter().enumerate().map(|(b, (block, _, _))| (last[b], block.len() / 16)).collect::<Vec<_>>());
        let mut wrong = Vec::new();
        for (b, ((block, words, _), o)) in blocks.iter().zip(&outs).enumerate() {
            let line = o.join(" ");
            if line.contains(['x', 'z']) {
                wrong.push(format!("{lanes}-lane unit, {}-block {b}: rtl left bits unknown", block.len()));
                continue;
            }
            let clock: usize = o[0].parse().unwrap();
            let tag = usize::from_str_radix(&o[1], 16).unwrap();
            let m = i64::from_str_radix(&o[2], 16).unwrap();
            let e = i64::from_str_radix(&o[3], 16).unwrap() as i32;
            let e = if e >= 0x800 { e - 0x1000 } else { e };
            let codes: Vec<i32> = (0..block.len()).map(|i| i32::from(hex_field(&o[4], i, 4) as u16 as i16)).collect();
            let want = quantize_blocks(block, *words, block.len());
            let ws = want.scales[0];
            if tag != b || (m, e) != (ws.m, ws.e) || codes != want.codes {
                wrong.push(format!("{lanes}-lane unit, {}-block {b} {words:?}: rtl tag {tag} {m}·2^{e} {:?}, numerics {}·2^{} {:?}", block.len(), &codes[..4], ws.m, ws.e, &want.codes[..4]));
            }
            if clock != model[b] {
                wrong.push(format!("{lanes}-lane unit, {}-block {b}: out at clock {clock}, the model's {}", block.len(), model[b]));
            }
        }
        for b in run_at + 1..count {
            if model[b] - model[b - 1] != lanes / 16 {
                wrong.push(format!("{lanes}-lane unit: block {b} of the run {} clocks after the one before, its {} beats", model[b] - model[b - 1], lanes / 16));
            }
        }
        assert!(wrong.is_empty(), "{} of {count} blocks differ:\n{}", wrong.len(), wrong.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
    }
}

// ── The weight decoders ────────────────────────────────────────────────────────────────────────────

/// The byte offsets of a format's binary16 scales, from ggml's block layouts (`ggml-common.h`).
fn scales_at(ty: GgmlType) -> &'static [usize] {
    match ty {
        GgmlType::Q4_0 | GgmlType::Q5_0 | GgmlType::Q8_0 | GgmlType::Iq4Nl | GgmlType::Iq4Xs => &[0],
        GgmlType::Q4_1 | GgmlType::Q5_1 | GgmlType::Q4K | GgmlType::Q5K => &[0, 2],
        GgmlType::Q2K => &[80, 82],
        GgmlType::Q3K => &[108],
        GgmlType::Q6K => &[208],
        other => panic!("no layout for {}", other.name()),
    }
}

fn finite_half(h: u16) -> bool {
    (h >> 10) & 0x1f != 0x1f
}

/// Blocks of `ty`: every byte pattern, then seeded bytes, each scale kept finite (a non-finite scale is a block the
/// engine refuses, not one it decodes).
fn weight_blocks(ty: GgmlType, n: usize) -> Vec<Vec<u8>> {
    let bb = ty.block_bytes();
    let mut r = Rng(0xdec0 + ty.id() as u64);
    let mut blocks: Vec<Vec<u8>> = [0x00u8, 0xff, 0x55, 0xaa, 0x0f, 0xf0, 0x80, 0x7f, 0x01, 0xfe, 0x33, 0xcc].iter().map(|&p| vec![p; bb]).collect();
    while blocks.len() < n {
        blocks.push((0..bb).map(|_| r.next() as u8).collect());
    }
    for b in &mut blocks {
        for &at in scales_at(ty) {
            let h = u16::from_le_bytes([b[at], b[at + 1]]);
            if !finite_half(h) {
                b[at + 1] &= 0xbf;
            }
        }
    }
    blocks
}

/// What a decoder gives for one block: its codes, unit multipliers and the two scales' bits.
struct Decoded {
    code: Vec<u32>,
    c: Vec<i16>,
    n: Vec<i16>,
    a: u16,
    b: u16,
}

fn hex_field(hex: &str, i: usize, digits: usize) -> u32 {
    let at = hex.len() - digits * (i + 1);
    u32::from_str_radix(&hex[at..at + digits], 16).unwrap()
}

/// Every block through `ty`'s decoder under iverilog.
fn decode_rtl(ty: GgmlType, blocks: &[Vec<u8>]) -> Vec<Decoded> {
    let (bb, elems) = (ty.block_bytes(), ty.block_len());
    let units = elems / UNIT;
    let dir = tempfile::tempdir().unwrap();
    let mut hex = String::with_capacity(blocks.len() * (2 * bb + 1));
    for b in blocks {
        for byte in b.iter().rev() {
            write!(hex, "{byte:02x}").unwrap();
        }
        hex.push('\n');
    }
    std::fs::write(dir.path().join("blocks.hex"), hex).unwrap();
    std::fs::write(dir.path().join("weights.v"), weights_verilog()).unwrap();
    let tb = format!(
        r#"module tb;
  reg [{bw}:0] mem [0:{last}];
  reg [{bw}:0] blk;
  wire [{cw}:0] code;
  wire [{uw}:0] c, n;
  wire [15:0] a, b;
  {module} u(.blk(blk), .code(code), .c(c), .n(n), .a(a), .b(b));
  integer i, f;
  initial begin
    $readmemh("blocks.hex", mem);
    f = $fopen("out.hex", "w");
    for (i = 0; i < {count}; i = i + 1) begin
      blk = mem[i]; #1;
      $fwrite(f, "%h %h %h %h %h\n", code, c, n, a, b);
    end
    $fclose(f);
    $finish;
  end
endmodule
"#,
        bw = 8 * bb - 1,
        cw = 8 * elems - 1,
        uw = 16 * units - 1,
        last = blocks.len() - 1,
        count = blocks.len(),
        module = decoder_name(ty),
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-o", "sim", "tb.v", "weights.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog ({}): {}", ty.name(), String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    assert!(run.status.success(), "vvp ({}): {}", ty.name(), String::from_utf8_lossy(&run.stderr));
    std::fs::read_to_string(dir.path().join("out.hex"))
        .unwrap()
        .lines()
        .map(|l| {
            let f: Vec<&str> = l.split(' ').collect();
            Decoded {
                code: (0..elems).map(|i| hex_field(f[0], i, 2)).collect(),
                c: (0..units).map(|u| hex_field(f[1], u, 4) as u16 as i16).collect(),
                n: (0..units).map(|u| hex_field(f[2], u, 4) as u16 as i16).collect(),
                a: u32::from_str_radix(f[3], 16).unwrap() as u16,
                b: u32::from_str_radix(f[4], 16).unwrap() as u16,
            }
        })
        .collect()
}

/// A decoder's scale bits stand for the coded row's scale: equal, or both zero.
fn same_scale(bits: u16, want: Scale) -> bool {
    match Scale::f16(bits) {
        Some(s) => s == want || (s.sig == 0 && want.sig == 0),
        None => false,
    }
}

/// The decoders cover exactly the formats the engine reads, and a code's width is the array's pass count: one 4-bit
/// digit a pass.
#[test]
fn a_decoder_exists_for_every_format_the_engine_reads_and_spans_its_passes() {
    let read: Vec<GgmlType> = (0..64).filter_map(|id| GgmlType::from_id(id).ok()).filter(|&t| reads(t)).collect();
    let decoded: Vec<GgmlType> = DECODERS.iter().map(|d| d.ty).collect();
    assert_eq!(decoded, read, "the decoders, in id order, are the formats `reads` admits");
    for d in DECODERS {
        let pricer = logicaffeine_compile::spatial::decode::GgmlType::from_id(d.ty.id()).expect("the pricer knows every read format");
        assert_eq!(array_passes(pricer), Some(d.code_bits.div_ceil(4)), "{}: {} code bits", d.ty.name(), d.code_bits);
        assert!(d.offset < 1 << d.code_bits, "{}: the offset {} lies inside its codes", d.ty.name(), d.offset);
    }
}

/// The scale offsets this lock fills are the ones the coded row reads: a non-finite half at any of them is a block
/// the engine refuses, and every other half position leaves the block readable.
#[test]
fn the_scale_layout_is_the_coded_rows() {
    for d in DECODERS {
        let bb = d.ty.block_bytes();
        for at in 0..bb - 1 {
            let mut b = vec![0u8; bb];
            b[at] = 0x00;
            b[at + 1] = 0x7c;
            let refused = matches!(CodedRow::decode(d.ty, &b), Some(Err(_)));
            assert_eq!(refused, scales_at(d.ty).contains(&at), "{}: an infinity at byte {at}", d.ty.name());
        }
    }
}

/// Every format's unit multipliers `c` and offsets `n` are signed bytes — over blocks of every byte value, blocks with
/// each position at the byte's extremes, and seeded blocks — so a lane multiplies them as eight bits; and every format's
/// code offset is zero or a power of two, so a lane takes it times a sum as a shift.
#[test]
fn every_decoders_unit_multipliers_are_signed_bytes_and_its_offset_a_shift() {
    let mut r = Rng(0xb17e);
    for d in DECODERS {
        assert!(d.offset == 0 || d.offset.is_power_of_two(), "{}: an offset of {}", d.ty.name(), d.offset);
        let bb = d.ty.block_bytes();
        let mut blocks: Vec<Vec<u8>> = (0..=255u8).map(|v| vec![v; bb]).collect();
        for at in 0..bb {
            for v in [0x00u8, 0x3f, 0x40, 0x7f, 0x80, 0xc0, 0xff] {
                let mut b = vec![0x55u8; bb];
                b[at] = v;
                blocks.push(b);
            }
        }
        blocks.extend((0..4000).map(|_| (0..bb).map(|_| r.next() as u8).collect::<Vec<u8>>()));
        let (mut lo, mut hi) = (i16::MAX, i16::MIN);
        for b in &blocks {
            let u = units(d.ty, b);
            for &x in u.c.iter().chain(&u.n) {
                lo = lo.min(x);
                hi = hi.max(x);
            }
        }
        assert!(lo >= -128 && hi <= 127, "{}: unit multipliers from {lo} to {hi}", d.ty.name());
    }
}

/// Every format's decoder over its blocks: each weight `c_u·(code − z)` is the coded row's coefficient, `n_u` its
/// offset term, each code inside its width, and the scale bits the row's scales; every field is its software twin's.
#[test]
fn every_decoder_gives_the_coded_row() {
    if !common::require_tool("iverilog") {
        return;
    }
    let mut wrong = Vec::new();
    for d in DECODERS {
        let blocks = weight_blocks(d.ty, 3000);
        let got = decode_rtl(d.ty, &blocks);
        assert_eq!(got.len(), blocks.len(), "{}: every block simulated", d.ty.name());
        for (bi, (block, g)) in blocks.iter().zip(&got).enumerate() {
            let row = CodedRow::decode(d.ty, block).expect("a read format").expect("finite scales");
            let twin = units(d.ty, block);
            if (twin.code.clone(), twin.c.clone(), twin.n.clone(), twin.a, twin.b) != (g.code.clone(), g.c.clone(), g.n.clone(), g.a, g.b) {
                wrong.push(format!("{} block {bi}: the decoder and its software twin differ", d.ty.name()));
            }
            let seg = &row.segments[0];
            if !same_scale(g.a, seg.a) || !same_scale(g.b, seg.b) {
                wrong.push(format!("{} block {bi}: scales {:04x} {:04x}, the row's {:?} {:?}", d.ty.name(), g.a, g.b, seg.a, seg.b));
            }
            for i in 0..d.ty.block_len() {
                let u = i / UNIT;
                let coef = i32::from(g.c[u]) * (g.code[i] as i32 - d.offset as i32);
                if g.code[i] >= 1 << d.code_bits || coef != i32::from(row.coef[i]) || g.n[u] != row.offs[i] {
                    wrong.push(format!(
                        "{} block {bi} element {i}: code {} c {} n {} → {coef}, {}; the row's {}, {}",
                        d.ty.name(), g.code[i], g.c[u], g.n[u], g.n[u], row.coef[i], row.offs[i]
                    ));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{} weights differ:\n{}", wrong.len(), wrong.iter().take(20).cloned().collect::<Vec<_>>().join("\n"));
}

// ── The array's processing element ─────────────────────────────────────────────────────────────────

/// Every activation code against every pair of digits: the packed operand fits the DSP block's signed 19-bit port, the
/// activation its signed 18-bit port, and one product splits back into both rows' products.
#[test]
fn one_product_splits_into_both_rows_for_every_operand() {
    for a in 0i64..256 {
        for q1 in 0u8..16 {
            for q2 in 0u8..16 {
                let w = pack(q1, q2);
                assert!((-(1 << 18)..1 << 18).contains(&w), "the packed operand {w} fits 19 signed bits");
                assert!(a < 1 << 17, "the activation fits 18 signed bits");
                assert_eq!(split(a * w, a), (a * q1 as i64, a * q2 as i64), "a={a} q1={q1} q2={q2}");
            }
        }
    }
}

/// A chain's sum splits into both rows' partials: at every corner of the operand box and over seeded chains. The low
/// lane's sum never reaches the high lane's place.
#[test]
fn a_chains_sum_splits_into_both_rows_partials() {
    assert!(CHAIN_ELEMENTS as i64 * 255 * 15 < 1 << LANE, "the low lane holds a whole chain");
    let mut r = Rng(0xa77);
    let corners: Vec<(Vec<i64>, Vec<u8>, Vec<u8>)> = [(255i64, 15u8, 15u8), (255, 15, 0), (255, 0, 15), (255, 0, 0), (0, 15, 15), (1, 15, 0)]
        .iter()
        .map(|&(a, q1, q2)| (vec![a; CHAIN_ELEMENTS], vec![q1; CHAIN_ELEMENTS], vec![q2; CHAIN_ELEMENTS]))
        .collect();
    let seeded = (0..200_000).map(|_| {
        let a: Vec<i64> = (0..CHAIN_ELEMENTS).map(|_| (r.next() % 256) as i64).collect();
        let q1: Vec<u8> = (0..CHAIN_ELEMENTS).map(|_| (r.next() % 16) as u8).collect();
        let q2: Vec<u8> = (0..CHAIN_ELEMENTS).map(|_| (r.next() % 16) as u8).collect();
        (a, q1, q2)
    });
    for (a, q1, q2) in corners.into_iter().chain(seeded) {
        let sum: i64 = (0..CHAIN_ELEMENTS).map(|i| a[i] * pack(q1[i], q2[i])).sum();
        let act: i64 = a.iter().sum();
        let want = (
            (0..CHAIN_ELEMENTS).map(|i| a[i] * q1[i] as i64).sum::<i64>(),
            (0..CHAIN_ELEMENTS).map(|i| a[i] * q2[i] as i64).sum::<i64>(),
        );
        assert_eq!(split(sum, act), want, "a={a:?} q1={q1:?} q2={q2:?}");
        assert!(sum.abs() < 1 << 32, "the chain's sum is far inside its 64 bits");
    }
    assert_eq!(HIGH_OFFSET, 8, "the high digit's offset centres 0..15 on the signed port");
}

/// `logos_subdot32` under iverilog: a 32-element sub-block's two rows' partial dots, `Σ a′·q₁` and `Σ a′·q₂`, over
/// seeded sub-blocks and the extremes.
#[test]
fn the_processing_element_gives_both_rows_sub_block_partials() {
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0x5ab);
    let mut cases: Vec<(Vec<u8>, Vec<u8>, Vec<u8>)> = vec![
        (vec![255; 32], vec![15; 32], vec![15; 32]),
        (vec![255; 32], vec![15; 32], vec![0; 32]),
        (vec![255; 32], vec![0; 32], vec![0; 32]),
        (vec![0; 32], vec![15; 32], vec![15; 32]),
        ((0..32).map(|i| (i * 8) as u8).collect(), (0..32).map(|i| (i % 16) as u8).collect(), (0..32).map(|i| (15 - i % 16) as u8).collect()),
    ];
    while cases.len() < 4000 {
        cases.push((
            (0..32).map(|_| r.next() as u8).collect(),
            (0..32).map(|_| (r.next() % 16) as u8).collect(),
            (0..32).map(|_| (r.next() % 16) as u8).collect(),
        ));
    }
    let dir = tempfile::tempdir().unwrap();
    let mut hex = String::new();
    for (a, q1, q2) in &cases {
        let s: u32 = a.iter().map(|&v| v as u32).sum();
        for v in a.iter().rev() {
            write!(hex, "{v:02x}").unwrap();
        }
        hex.push('\n');
        for v in q1.iter().rev() {
            write!(hex, "{v:x}").unwrap();
        }
        hex.push('\n');
        for v in q2.iter().rev() {
            write!(hex, "{v:x}").unwrap();
        }
        writeln!(hex, "\n{s:x}").unwrap();
    }
    std::fs::write(dir.path().join("vectors.hex"), hex).unwrap();
    std::fs::write(dir.path().join("array.v"), array_verilog()).unwrap();
    let n = cases.len();
    let tb = format!(
        r#"module tb;
  reg [255:0] mem [0:{last}];
  reg [255:0] a;
  reg [127:0] q1, q2;
  reg [12:0] s;
  wire [16:0] p1;
  wire signed [18:0] p2;
  logos_subdot32 u(.a(a), .q1(q1), .q2(q2), .s(s), .p1(p1), .p2(p2));
  integer i, f;
  initial begin
    $readmemh("vectors.hex", mem);
    f = $fopen("out.txt", "w");
    for (i = 0; i < {n}; i = i + 1) begin
      a = mem[4*i]; q1 = mem[4*i+1][127:0]; q2 = mem[4*i+2][127:0]; s = mem[4*i+3][12:0]; #1;
      $fwrite(f, "%0d %0d\n", p1, p2);
    end
    $fclose(f);
    $finish;
  end
endmodule
"#,
        last = 4 * n - 1
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-o", "sim", "tb.v", "array.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    let out = std::fs::read_to_string(dir.path().join("out.txt")).unwrap();
    let got: Vec<(i64, i64)> = out
        .lines()
        .map(|l| {
            let (x, y) = l.split_once(' ').unwrap();
            (x.parse().unwrap(), y.parse().unwrap())
        })
        .collect();
    assert_eq!(got.len(), n, "every sub-block simulated");
    for (c, ((a, q1, q2), g)) in cases.iter().zip(&got).enumerate() {
        let want = (
            a.iter().zip(q1).map(|(&a, &q)| a as i64 * q as i64).sum::<i64>(),
            a.iter().zip(q2).map(|(&a, &q)| a as i64 * q as i64).sum::<i64>(),
        );
        assert_eq!(*g, want, "sub-block {c}");
    }
}

// ── The block-scale lane ───────────────────────────────────────────────────────────────────────────

/// Random blocks of `ty`, each scale finite and small as checkpoints carry them.
fn small_scaled_blocks(r: &mut Rng, ty: GgmlType, n: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(n * ty.block_bytes());
    while out.len() < n * ty.block_bytes() {
        let mut b: Vec<u8> = (0..ty.block_bytes()).map(|_| r.next() as u8).collect();
        for &at in scales_at(ty) {
            let mant = (r.next() % 1024) as u16;
            let exp = 5 + (r.next() % 12) as u16;
            let sign = if r.next() & 1 == 0 { 0 } else { 0x8000 };
            b[at..at + 2].copy_from_slice(&(sign | exp << 10 | mant).to_le_bytes());
        }
        out.extend(b);
    }
    out
}

/// A lane input as one hex line per field group, in the testbench's read order.
fn lane_vector(blk: &LaneBlock, two: bool, acc: f32, units: usize) -> String {
    let mut p_hi = String::new();
    let mut s_hi = String::new();
    let mut p_lo = String::new();
    let mut s_lo = String::new();
    let mut q = String::new();
    let mut c = String::new();
    let mut n = String::new();
    for u in (0..units).rev() {
        let hi = blk.words[0].units[u];
        let lo = if two { blk.words[1].units[u] } else { UnitSums { products: 0, act: 0 } };
        write!(p_hi, "{:06x}", hi.products).unwrap();
        write!(s_hi, "{:03x}", hi.act).unwrap();
        write!(p_lo, "{:06x}", lo.products).unwrap();
        write!(s_lo, "{:03x}", lo.act).unwrap();
        write!(q, "{:03x}", blk.q[u]).unwrap();
        write!(c, "{:04x}", blk.c[u] as i16 as u16).unwrap();
        write!(n, "{:04x}", blk.n[u] as i16 as u16).unwrap();
    }
    let half = |s: Scale| -> u16 {
        // The binary16 bits of an exact scale: sig·2^exp with a denormal at -24.
        let neg = s.sig < 0;
        let mag = s.sig.unsigned_abs() as u16;
        let bits = if mag == 0 { 0 } else if s.exp == -24 && mag < 1024 { mag } else { (((s.exp + 25) as u16) << 10) | (mag - 1024) };
        bits | if neg { 0x8000 } else { 0 }
    };
    format!(
        "{p_hi}\n{s_hi}\n{p_lo}\n{s_lo}\n{q}\n{c}\n{n}\n{:02x}\n{:04x}\n{:04x}\n{:02x}\n{:03x}\n{}\n{:08x}\n",
        blk.offset,
        half(blk.a),
        half(blk.b),
        blk.act.m,
        blk.act.e as i16 as u16 & 0xfff,
        two as u8,
        acc.to_bits()
    )
}

/// Every format's blocks in both word counts through `logos_lane` under iverilog: the row's next f32 is the lane
/// model's, bit for bit, from accumulators of every sign and size.
#[test]
fn the_lane_rtl_adds_each_block_as_the_lane_model_does() {
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0x1a7e);
    let mut wrong = Vec::new();
    for (len, unit_count) in [(32usize, 2usize), (256, 16)] {
        let mut cases: Vec<(LaneBlock, bool, f32, f32)> = Vec::new();
        for d in DECODERS.iter().filter(|d| d.ty.block_len() == len) {
            for words in [Words::One, Words::Two] {
                let count = 4096 / len;
                let raw = small_scaled_blocks(&mut r, d.ty, count);
                let row = CodedRow::decode(d.ty, &raw).unwrap().unwrap();
                let x: Vec<f32> = (0..count * len).map(|_| ((r.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * 2f32.powi((r.next() % 20) as i32 - 10)).collect();
                let act = logicaffeine_compile::spatial::decode_engine::numerics::quantize_blocks(&x, words, len);
                let mut acc = 0f32;
                for (b, blk) in raw.chunks_exact(d.ty.block_bytes()).enumerate() {
                    let seg = &row.segments[b];
                    let delivered = deliver(&units(d.ty, blk), (seg.a, seg.b), d.offset, &act, words, b);
                    let start = if b % 5 == 4 { -acc } else { acc };
                    let next = lane(&delivered, start);
                    cases.push((delivered, words == Words::Two, start, next));
                    acc = next;
                }
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let hex: String = cases.iter().map(|(blk, two, acc, _)| lane_vector(blk, *two, *acc, unit_count)).collect();
        std::fs::write(dir.path().join("vectors.hex"), hex).unwrap();
        std::fs::write(dir.path().join("lane.v"), lane_verilog()).unwrap();
        let fields = 14;
        let tb = format!(
            r#"module tb;
  reg [{pw}:0] mem [0:{last}];
  reg [{pw}:0] p_hi, p_lo;
  reg [{sw}:0] s_hi, s_lo, q;
  reg [{cw}:0] c, n;
  reg [7:0] z, m;
  reg [15:0] a, b;
  reg [11:0] e;
  reg two;
  reg [31:0] acc;
  wire [31:0] out;
  logos_lane #(.UNITS({units})) u(.p_hi(p_hi), .s_hi(s_hi), .p_lo(p_lo), .s_lo(s_lo), .q(q), .c(c), .n(n), .z(z), .a(a), .b(b),
                                  .m(m), .e($signed(e)), .two(two), .acc(acc), .out(out));
  integer i, f;
  initial begin
    $readmemh("vectors.hex", mem);
    f = $fopen("out.hex", "w");
    for (i = 0; i < {count}; i = i + 1) begin
      p_hi = mem[{fields}*i]; s_hi = mem[{fields}*i+1]; p_lo = mem[{fields}*i+2]; s_lo = mem[{fields}*i+3]; q = mem[{fields}*i+4];
      c = mem[{fields}*i+5]; n = mem[{fields}*i+6]; z = mem[{fields}*i+7]; a = mem[{fields}*i+8]; b = mem[{fields}*i+9];
      m = mem[{fields}*i+10]; e = mem[{fields}*i+11]; two = mem[{fields}*i+12]; acc = mem[{fields}*i+13]; #1;
      $fwrite(f, "%h\n", out);
    end
    $fclose(f);
    $finish;
  end
endmodule
"#,
            units = unit_count,
            pw = 24 * unit_count - 1,
            sw = 12 * unit_count - 1,
            cw = 16 * unit_count - 1,
            last = fields * cases.len() - 1,
            count = cases.len(),
        );
        std::fs::write(dir.path().join("tb.v"), tb).unwrap();
        let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-o", "sim", "tb.v", "lane.v"]).output().expect("iverilog");
        assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
        let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
        assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
        let out = std::fs::read_to_string(dir.path().join("out.hex")).unwrap();
        let got: Vec<u32> = out.lines().map(|l| u32::from_str_radix(l, 16).unwrap()).collect();
        assert_eq!(got.len(), cases.len(), "every block simulated");
        for (k, ((_, two, acc, want), g)) in cases.iter().zip(&got).enumerate() {
            if *g != want.to_bits() {
                wrong.push(format!("{len}-element block {k} (two words {two}) from {acc:e}: rtl {g:08x}, model {:08x}", want.to_bits()));
            }
        }
    }
    assert!(wrong.is_empty(), "{} blocks differ:\n{}", wrong.len(), wrong.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
}

/// The pipelined rounding of an exact sum, an operand a clock under iverilog: each clock's f32 `logos_round88`'s of the
/// sum and exponent [`ROUND88_P_LATENCY`] clocks before, over every power of two and its neighbours at every magnitude
/// and both signs, the significands either side of every halfway point at every shift, random sums of every width, the
/// extremes and zero, at exponents from past the f32 top to below its least normal.
#[test]
fn the_pipelined_rounding_of_an_exact_sum_is_the_rounding_a_clock_at_a_time() {
    use logicaffeine_compile::spatial::decode_engine::rtl::lane::ROUND88_P_LATENCY;
    if !common::require_tool("iverilog") {
        return;
    }
    let mask = (1u128 << 88) - 1;
    let mut ops: Vec<u128> = Vec::new();
    let mut add = |v: i128, least: i32| ops.push(((least as u128) & 0xfff) << 88 | (v as u128) & mask);
    let mut r = Rng(0x88);
    for least in [-300, -200, -150, -127, -126, -125, -100, -60, -30, -1, 0, 1, 30, 60, 100, 103, 104, 105, 127, 128, 200] {
        for k in 0..87 {
            for d in [-1i128, 0, 1] {
                add((1i128 << k) + d, least);
                add(-((1i128 << k) + d), least);
            }
        }
        for s in 1..64 {
            for kept in [0x80_0000i128, 0x80_0001, 0xff_ffff, 0xff_fffe] {
                for low in [0, 1i128 << (s - 1), (1i128 << (s - 1)) + 1, (1i128 << (s - 1)) - 1] {
                    add((kept << s) + low, least);
                    add(-((kept << s) + low), least);
                }
            }
        }
    }
    for _ in 0..4000 {
        let w = 1 + (r.next() % 87) as u32;
        let v = ((r.next() as u128) << 64 | r.next() as u128) as i128 & ((1i128 << w) - 1);
        let least = (r.next() % 801) as i32 - 400;
        add(if r.next() & 1 == 0 { v } else { -v }, least);
    }
    add(0, 0);
    add(-(1i128 << 87), 0);
    add((1i128 << 87) - 1, -50);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ops.hex"), ops.iter().map(|x| format!("{x:025x}\n")).collect::<String>()).unwrap();
    std::fs::write(dir.path().join("lane.v"), lane_verilog()).unwrap();
    let n = ops.len();
    let tb = format!(
        r#"module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  reg [99:0] mem [0:{last}];
  reg [99:0] hist [0:{lat}];
  reg signed [87:0] v = 0;
  reg signed [11:0] least = 0;
  wire [31:0] y, want;
  logos_round88_p u(.clk(clk), .v(v), .least(least), .f(y));
  logos_round88 c(.v(hist[{held}][87:0]), .least(hist[{held}][99:88]), .f(want));
  integer i, k, bad = 0;
  initial begin
    $readmemh("ops.hex", mem);
    for (k = 0; k <= {lat}; k = k + 1) hist[k] = 0;
    for (i = 0; i < {n} + {lat}; i = i + 1) begin
      @(negedge clk);
      if (i >= {lat}) begin
        #1;
        if (y !== want) begin
          bad = bad + 1;
          if (bad < 20) $display("logos_round88_p(%h, %0d): pipelined %h, logos_round88 %h", hist[{held}][87:0], $signed(hist[{held}][99:88]), y, want);
        end
      end
      for (k = {lat}; k > 0; k = k - 1) hist[k] = hist[k-1];
      hist[0] = (i < {n}) ? mem[i] : 100'd0;
      v = hist[0][87:0];
      least = hist[0][99:88];
    end
    $display("OPERANDS %0d BAD %0d", {n}, bad);
    $finish;
  end
endmodule
"#,
        last = n - 1,
        lat = ROUND88_P_LATENCY,
        held = ROUND88_P_LATENCY - 1,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "lane.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    let out = String::from_utf8_lossy(&run.stdout).to_string();
    assert!(run.status.success() && out.contains(&format!("OPERANDS {n} BAD 0")), "{out}");
}

/// `logos_lane_value_p #(UNITS, WORDS)` under iverilog, a block a clock: each case's value `LANE_VALUE_LATENCY` clocks
/// after its inputs, in order.
fn pipelined_lane_values(cases: &[(LaneBlock, bool)], unit_count: usize, words: usize) -> Vec<u32> {
    use logicaffeine_compile::spatial::decode_engine::rtl::lane::LANE_VALUE_LATENCY;
    let dir = tempfile::tempdir().unwrap();
    let hex: String = cases.iter().map(|(blk, two)| lane_vector(blk, *two, 0.0, unit_count)).collect();
    std::fs::write(dir.path().join("vectors.hex"), hex).unwrap();
    std::fs::write(dir.path().join("lane.v"), lane_verilog()).unwrap();
    let fields = 14;
    let tb = format!(
        r#"module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  reg [{pw}:0] mem [0:{last}];
  reg [{pw}:0] p_hi = 0, p_lo = 0;
  reg [{sw}:0] s_hi = 0, s_lo = 0, q = 0;
  reg [{cw}:0] c = 0, n = 0;
  reg [7:0] z = 0, m = 0;
  reg [15:0] a = 0, b = 0;
  reg [11:0] e = 0;
  reg two = 0;
  wire [31:0] value;
  // The pipelined lane takes each unit's multiplier and offset term as its signed byte.
  wire [{bw}:0] cb, nb;
  genvar k;
  generate for (k = 0; k < {units}; k = k + 1) begin : bytes
    assign cb[8*k +: 8] = c[16*k +: 8];
    assign nb[8*k +: 8] = n[16*k +: 8];
  end endgenerate
  logos_lane_value_p #(.UNITS({units}), .WORDS({words})) u(.clk(clk), .p_hi(p_hi), .s_hi(s_hi), .p_lo(p_lo), .s_lo(s_lo), .q(q), .c(cb), .n(nb),
                                          .z(z), .a(a), .b(b), .m(m), .e($signed(e)), .two(two), .value(value));
  integer i, f;
  initial begin
    $readmemh("vectors.hex", mem);
    f = $fopen("out.hex", "w");
    for (i = 0; i < {count} + {lat}; i = i + 1) begin
      @(negedge clk);
      if (i >= {lat}) $fwrite(f, "%h\n", value);
      if (i < {count}) begin
        p_hi = mem[{fields}*i]; s_hi = mem[{fields}*i+1]; p_lo = mem[{fields}*i+2]; s_lo = mem[{fields}*i+3]; q = mem[{fields}*i+4];
        c = mem[{fields}*i+5]; n = mem[{fields}*i+6]; z = mem[{fields}*i+7]; a = mem[{fields}*i+8]; b = mem[{fields}*i+9];
        m = mem[{fields}*i+10]; e = mem[{fields}*i+11]; two = mem[{fields}*i+12];
      end
    end
    $fclose(f);
    $finish;
  end
endmodule
"#,
        units = unit_count,
        pw = 24 * unit_count - 1,
        sw = 12 * unit_count - 1,
        cw = 16 * unit_count - 1,
        bw = 8 * unit_count - 1,
        last = fields * cases.len() - 1,
        count = cases.len(),
        lat = LANE_VALUE_LATENCY,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "lane.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    let out = std::fs::read_to_string(dir.path().join("out.hex")).unwrap();
    let got: Vec<u32> = out.lines().map(|l| u32::from_str_radix(l, 16).unwrap()).collect();
    assert_eq!(got.len(), cases.len(), "every block's value came out");
    got
}

/// `logos_lane_value_p` under iverilog, a block a clock: every format's blocks in both word counts, the value
/// `LANE_VALUE_LATENCY` clocks later the lane model's from −0 (the block's own f32, which −0 leaves as it is), bit for
/// bit.
#[test]
fn the_pipelined_lane_gives_each_blocks_value_a_clock_at_a_time() {
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0x1a7f);
    let mut wrong = Vec::new();
    for (len, unit_count) in [(32usize, 2usize), (256, 16)] {
        let mut cases: Vec<(LaneBlock, bool, f32)> = Vec::new();
        for d in DECODERS.iter().filter(|d| d.ty.block_len() == len) {
            for words in [Words::One, Words::Two] {
                let count = 4096 / len;
                let raw = small_scaled_blocks(&mut r, d.ty, count);
                let row = CodedRow::decode(d.ty, &raw).unwrap().unwrap();
                let x: Vec<f32> = (0..count * len).map(|_| ((r.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * 2f32.powi((r.next() % 20) as i32 - 10)).collect();
                let act = logicaffeine_compile::spatial::decode_engine::numerics::quantize_blocks(&x, words, len);
                for (b, blk) in raw.chunks_exact(d.ty.block_bytes()).enumerate() {
                    let seg = &row.segments[b];
                    let delivered = deliver(&units(d.ty, blk), (seg.a, seg.b), d.offset, &act, words, b);
                    let value = lane(&delivered, -0.0);
                    cases.push((delivered, words == Words::Two, value));
                }
            }
        }
        let inputs: Vec<(LaneBlock, bool)> = cases.iter().map(|(blk, two, _)| (blk.clone(), *two)).collect();
        let got = pipelined_lane_values(&inputs, unit_count, 2);
        for (k, ((_, two, want), g)) in cases.iter().zip(&got).enumerate() {
            if *g != want.to_bits() {
                wrong.push(format!("{len}-element block {k} (two words {two}): pipelined {g:08x}, lane {:08x}", want.to_bits()));
            }
        }
    }
    assert!(wrong.is_empty(), "{} values differ:\n{}", wrong.len(), wrong.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
}

/// `logos_lane_value_p` built for one activation word, which sums a pair of units' products in one DSP block's two
/// 18×19 multipliers, its unit sums taken in 19 bits: every format's blocks of every byte pattern (each code at an end of
/// its range: Q8_0's −128 and 127, IQ4's table ends, Q6_K's −32 under a scale of −128) under activations all 127, all
/// −127, a sign a unit and seeded, and seeded blocks besides — each unit's sum up to 16·128·127 in magnitude — each value
/// the lane model's from −0, bit for bit.
#[test]
fn the_one_word_lane_takes_every_units_widest_sums_exactly() {
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0x1a80);
    let mut wrong = Vec::new();
    for (len, unit_count) in [(32usize, 2usize), (256, 16)] {
        let mut cases: Vec<(LaneBlock, bool)> = Vec::new();
        let mut wants: Vec<(String, f32)> = Vec::new();
        for d in DECODERS.iter().filter(|d| d.ty.block_len() == len) {
            for (k, blk) in weight_blocks(d.ty, 20).iter().enumerate() {
                let row = CodedRow::decode(d.ty, blk).unwrap().unwrap();
                let seg = &row.segments[0];
                for (shape, x) in [
                    ("all 127", vec![127.0f32; len]),
                    ("all -127", vec![-127.0f32; len]),
                    ("a sign a unit", (0..len).map(|i| if (i / 16) % 2 == 0 { 127.0 } else { -127.0 }).collect()),
                    ("seeded", (0..len).map(|_| ((r.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * 127.0).collect::<Vec<f32>>()),
                ] {
                    let act = logicaffeine_compile::spatial::decode_engine::numerics::quantize_blocks(&x, Words::One, len);
                    let delivered = deliver(&units(d.ty, blk), (seg.a, seg.b), d.offset, &act, Words::One, 0);
                    wants.push((format!("{} block {k} under {shape}", d.ty.name()), lane(&delivered, -0.0)));
                    cases.push((delivered, false));
                }
            }
        }
        let got = pipelined_lane_values(&cases, unit_count, 1);
        for ((what, want), g) in wants.iter().zip(&got) {
            if *g != want.to_bits() {
                wrong.push(format!("{what}: one-word lane {g:08x}, lane {:08x}", want.to_bits()));
            }
        }
    }
    assert!(wrong.is_empty(), "{} values differ:\n{}", wrong.len(), wrong.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
}

/// `logos_lane_value_p` built for one activation word at the corners of what it takes: sixteen units each of a one-word
/// unit's widest signed sum, 16·128·127 of either sign (codes at `z + 128` under `z` 0, or `z − 128` under 128, against
/// activations all 127 or all −127), and each unit's activation sum at ±16·127 with it; each multiplier and offset
/// multiplier −128 or 127, all one way, alternating or seeded — so its terms reach 16·128·16·128·127 and 16·128·16·127,
/// all of a 30-bit and a 23-bit sum — under scales of either sign and every exponent gap, each term alone and both: each
/// value the lane model's from −0, bit for bit.
#[test]
fn the_one_word_lane_sums_its_terms_at_the_widest_its_units_and_multipliers_make() {
    use logicaffeine_compile::spatial::decode_engine::lane::WordSums;
    use logicaffeine_compile::spatial::decode_engine::numerics::{ActScale, Scale};
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0x1a82);
    let pattern = |r: &mut Rng, k: usize| -> Vec<bool> {
        match k {
            0 => vec![true; 16],
            1 => vec![false; 16],
            2 => (0..16).map(|u| u % 2 == 0).collect(),
            _ => (0..16).map(|_| r.next() & 1 == 1).collect(),
        }
    };
    let byte = |up: bool| if up { 127i64 } else { -128 };
    let scale = |r: &mut Rng| loop {
        let h = r.next() as u16;
        if let Some(s) = Scale::f16(h) {
            if s.sig != 0 {
                return s;
            }
        }
    };
    let (mut cases, mut wants) = (Vec::new(), Vec::new());
    for case in 0..1024 {
        let (signs, cs, ns) = (pattern(&mut r, case % 4), pattern(&mut r, case / 4 % 4), pattern(&mut r, case / 16 % 4));
        let offset = if case / 64 % 2 == 0 { 0 } else { 128 };
        // A unit's code and excess activation: its sum 16·(q − z)·(a′ − 128) = ±16·128·127.
        let (q, units): (Vec<i64>, Vec<UnitSums>) = signs
            .iter()
            .map(|&up| {
                let (code, act) = match (offset, up) {
                    (0, true) => (128, 255),
                    (0, false) => (128, 1),
                    (_, true) => (0, 1),
                    (_, false) => (0, 255),
                };
                (16 * code, UnitSums { products: 16 * code * act, act: 16 * act })
            })
            .unzip();
        // The two scales: of one exponent, a's alone, both, b's alone.
        let first = scale(&mut r);
        let (a, b) = match case / 128 % 4 {
            0 => (first, Scale { sig: -first.sig, exp: first.exp }),
            1 => (first, Scale::ZERO),
            2 => (first, scale(&mut r)),
            _ => (Scale::ZERO, first),
        };
        let blk = LaneBlock {
            c: cs.iter().map(|&u| byte(u)).collect(),
            n: ns.iter().map(|&u| byte(u)).collect(),
            q,
            offset,
            a,
            b,
            act: ActScale { m: 128 + (r.next() % 128) as i64, e: (r.next() % 40) as i32 - 20 },
            words: vec![WordSums { units, excess: true, weight: 1 }],
        };
        wants.push((format!("case {case}: {blk:?}"), lane(&blk, -0.0)));
        cases.push((blk, false));
    }
    let got = pipelined_lane_values(&cases, 16, 1);
    let wrong: Vec<String> =
        wants.iter().zip(&got).filter(|((_, want), g)| **g != want.to_bits()).map(|((what, want), g)| format!("{what}: lane {g:08x}, model {:08x}", want.to_bits())).collect();
    assert!(wrong.is_empty(), "{} values differ:\n{}", wrong.len(), wrong.iter().take(6).cloned().collect::<Vec<_>>().join("\n"));
}

/// `logos_lane_slim_p` under iverilog, a block a clock: every 32-element format whose blocks are of one scale — each
/// unit's multiplier 1 and its offset multiplier 0, Q8_0's and Q4_0's among them — in both word counts, each block's
/// value `LANE_VALUE_LATENCY` clocks later the lane model's from −0, bit for bit, as the full lane's is.
#[test]
fn the_slim_lane_gives_each_one_scale_blocks_value_as_the_lane_does() {
    use logicaffeine_compile::spatial::decode_engine::rtl::lane::LANE_VALUE_LATENCY;
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0x51a7);
    let mut cases: Vec<(LaneBlock, bool, f32)> = Vec::new();
    let mut formats: Vec<GgmlType> = Vec::new();
    for d in DECODERS.iter().filter(|d| d.ty.block_len() == 32) {
        for words in [Words::One, Words::Two] {
            let count = 4096 / 32;
            let raw = small_scaled_blocks(&mut r, d.ty, count);
            let row = CodedRow::decode(d.ty, &raw).unwrap().unwrap();
            let x: Vec<f32> = (0..count * 32).map(|_| ((r.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * 2f32.powi((r.next() % 40) as i32 - 20)).collect();
            let act = logicaffeine_compile::spatial::decode_engine::numerics::quantize_blocks(&x, words, 32);
            let mut blocks = Vec::new();
            for (b, blk) in raw.chunks_exact(d.ty.block_bytes()).enumerate() {
                let seg = &row.segments[b];
                blocks.push(deliver(&units(d.ty, blk), (seg.a, seg.b), d.offset, &act, words, b));
            }
            if !blocks.iter().all(|blk| blk.c.iter().all(|&c| c == 1) && blk.n.iter().all(|&n| n == 0)) {
                continue;
            }
            if !formats.contains(&d.ty) {
                formats.push(d.ty);
            }
            for blk in blocks {
                let value = lane(&blk, -0.0);
                cases.push((blk, words == Words::Two, value));
            }
        }
    }
    assert!(formats.contains(&GgmlType::Q8_0) && formats.contains(&GgmlType::Q4_0), "the one-scale formats: {formats:?}");
    let dir = tempfile::tempdir().unwrap();
    let hex: String = cases.iter().map(|(blk, two, _)| lane_vector(blk, *two, 0.0, 2)).collect();
    std::fs::write(dir.path().join("vectors.hex"), hex).unwrap();
    std::fs::write(dir.path().join("lane.v"), lane_verilog()).unwrap();
    let fields = 14;
    let tb = format!(
        r#"module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  reg [47:0] mem [0:{last}];
  reg [47:0] p_hi = 0, p_lo = 0;
  reg [23:0] s_hi = 0, s_lo = 0, q = 0;
  reg [7:0] z = 0, m = 0;
  reg [15:0] a = 0;
  reg [11:0] e = 0;
  reg two = 0;
  wire [31:0] value;
  logos_lane_slim_p #(.UNITS(2)) u(.clk(clk), .p_hi(p_hi), .s_hi(s_hi), .p_lo(p_lo), .s_lo(s_lo), .q(q), .z(z), .a(a), .m(m),
                                   .e($signed(e)), .two(two), .value(value));
  integer i, f;
  initial begin
    $readmemh("vectors.hex", mem);
    f = $fopen("out.hex", "w");
    for (i = 0; i < {count} + {lat}; i = i + 1) begin
      @(negedge clk);
      if (i >= {lat}) $fwrite(f, "%h\n", value);
      if (i < {count}) begin
        p_hi = mem[{fields}*i]; s_hi = mem[{fields}*i+1]; p_lo = mem[{fields}*i+2]; s_lo = mem[{fields}*i+3]; q = mem[{fields}*i+4];
        z = mem[{fields}*i+7]; a = mem[{fields}*i+8]; m = mem[{fields}*i+10]; e = mem[{fields}*i+11]; two = mem[{fields}*i+12];
      end
    end
    $fclose(f);
    $finish;
  end
endmodule
"#,
        last = fields * cases.len() - 1,
        count = cases.len(),
        lat = LANE_VALUE_LATENCY,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "lane.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    let out = std::fs::read_to_string(dir.path().join("out.hex")).unwrap();
    let got: Vec<u32> = out.lines().map(|l| u32::from_str_radix(l, 16).unwrap()).collect();
    assert_eq!(got.len(), cases.len(), "every block's value came out");
    let wrong: Vec<String> = cases
        .iter()
        .zip(&got)
        .enumerate()
        .filter(|(_, ((_, _, want), g))| **g != want.to_bits())
        .map(|(k, ((_, two, want), g))| format!("block {k} (two words {two}): slim {g:08x}, lane {:08x}", want.to_bits()))
        .collect();
    assert!(wrong.is_empty(), "{} values differ:\n{}", wrong.len(), wrong.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
}

// ── The vector units ───────────────────────────────────────────────────────────────────────────────

/// The operands every vector-unit function is held over: the special values and the edges of each function's
/// branches, dense sweeps of the ranges the engine uses the functions on, random bits of every class, and positive
/// magnitudes of every exponent.
fn vector_operands() -> Vec<u32> {
    let mut r = Rng(0xfec7);
    let mut xs: Vec<u32> = vec![
        0, 0x8000_0000, 1, 0x8000_0001, 0x007f_ffff, 0x0080_0000, 0x3f80_0000, 0xbf80_0000, 0x3f00_0000, 0x4000_0000,
        0x7f7f_ffff, 0xff7f_ffff, 0x7f80_0000, 0xff80_0000, 0x7fc0_0000, 0x3fc0_0000, 0x3fbf_ffff, 0x3f40_0000,
        0x3f3f_ffff, 0x42b1_999a, 0x42b1_9999, 0x42b1_999b, 0xc2d0_0000, 0xc2cf_ffff, 0xc2d0_0001, 0x4180_0000,
        0x417f_ffff, 0x4180_0001, 0x3380_0000, 0x3400_0000,
    ];
    let sweep = |lo: f32, hi: f32, n: usize| (0..n).map(move |i| (lo + (hi - lo) * i as f32 / n as f32).to_bits());
    xs.extend(sweep(-110.0, 95.0, 4000));
    xs.extend(sweep(-30.0, 30.0, 4000));
    xs.extend(sweep(0.0, 4.0, 2000));
    xs.extend((0..3000).map(|_| r.next() as u32));
    xs.extend((0..3000).map(|_| {
        let e = 1 + (r.next() % 253) as u32;
        (e << 23) | (r.next() as u32 & 0x7f_ffff)
    }));
    xs
}

/// `module` (its operand port `port`) under iverilog over every operand: each result `f`'s, bit for bit.
fn vector_function_is_its_numerics(module: &str, port: &str, f: fn(f32) -> f32) {
    if !common::require_tool("iverilog") {
        return;
    }
    let xs = vector_operands();
    let dir = tempfile::tempdir().unwrap();
    let hex: String = xs.iter().map(|x| format!("{x:08x}\n")).collect();
    std::fs::write(dir.path().join("vectors.hex"), hex).unwrap();
    std::fs::write(dir.path().join("vector.v"), vector_verilog()).unwrap();
    let n = xs.len();
    let tb = format!(
        r#"module tb;
  reg [31:0] mem [0:{last}];
  reg [31:0] x;
  wire [31:0] y;
  {module} u(.{port}(x), .y(y));
  integer i, f;
  initial begin
    $readmemh("vectors.hex", mem);
    f = $fopen("out.hex", "w");
    for (i = 0; i < {n}; i = i + 1) begin
      x = mem[i]; #1;
      $fwrite(f, "%h\n", y);
    end
    $fclose(f);
    $finish;
  end
endmodule
"#,
        last = n - 1
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-o", "sim", "tb.v", "vector.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    let out = std::fs::read_to_string(dir.path().join("out.hex")).unwrap();
    let got: Vec<&str> = out.lines().collect();
    assert_eq!(got.len(), n, "every operand simulated");
    let wrong: Vec<String> = xs
        .iter()
        .zip(&got)
        .filter_map(|(&x, field)| {
            let g = u32::from_str_radix(field, 16).unwrap_or_else(|_| panic!("{module}({x:08x}): the rtl gave `{field}`"));
            let want = f(f32::from_bits(x)).to_bits();
            (g != want).then(|| format!("{module}({x:08x}): rtl {g:08x}, numerics {want:08x}"))
        })
        .collect();
    assert!(wrong.is_empty(), "{} of {n} results differ:\n{}", wrong.len(), wrong.iter().take(20).cloned().collect::<Vec<_>>().join("\n"));
}

#[test]
fn the_exp_unit_is_exp() {
    vector_function_is_its_numerics("logos_exp", "x_in", |x| exp(FpUnit::Soft, x));
}

/// The pipelined exponential, an operand a clock under iverilog over every operand the exponential is held over: each
/// clock's result the numerics' `exp` of the operand [`EXP_LATENCY`] clocks before.
#[test]
fn the_pipelined_exp_unit_is_exp_a_clock_at_a_time() {
    use logicaffeine_compile::spatial::decode_engine::rtl::vector::EXP_LATENCY;
    pipelined_function_is_its_numerics("logos_exp_p", EXP_LATENCY, |x| exp(FpUnit::Soft, x));
}

/// The pipelined reciprocal, an operand a clock: each clock's result the numerics' `recip` of the operand
/// [`RECIP_LATENCY`] clocks before, over every operand the reciprocal is held over.
#[test]
fn the_pipelined_reciprocal_unit_is_recip_a_clock_at_a_time() {
    use logicaffeine_compile::spatial::decode_engine::rtl::vector::RECIP_LATENCY;
    pipelined_function_is_its_numerics("logos_recip_p", RECIP_LATENCY, |x| recip(FpUnit::Soft, x));
}

/// The pipelined reciprocal square root, an operand a clock: each clock's result the numerics' `rsqrt` of the operand
/// [`RSQRT_LATENCY`] clocks before, over every operand the reciprocal square root is held over.
#[test]
fn the_pipelined_reciprocal_square_root_unit_is_rsqrt_a_clock_at_a_time() {
    use logicaffeine_compile::spatial::decode_engine::rtl::vector::RSQRT_LATENCY;
    pipelined_function_is_its_numerics("logos_rsqrt_p", RSQRT_LATENCY, |x| rsqrt(FpUnit::Soft, x));
}

/// The pipelined logarithm, an operand a clock: each clock's result the numerics' `ln` of the operand [`LN_LATENCY`]
/// clocks before, over every operand the logarithm is held over.
#[test]
fn the_pipelined_logarithm_unit_is_ln_a_clock_at_a_time() {
    use logicaffeine_compile::spatial::decode_engine::rtl::vector::LN_LATENCY;
    pipelined_function_is_its_numerics("logos_ln_p", LN_LATENCY, |x| ln(FpUnit::Soft, x));
}

/// The pipelined binary16 rounding, an f32 a clock under iverilog: each clock's result `logos_f32_to_f16`'s of the f32
/// [`F32_TO_F16_LATENCY`] clocks before, over every operand the vector functions are held over and every exponent from
/// past the binary16 top to below its least denormal, each with the significands either side of a halfway point.
#[test]
fn the_pipelined_binary16_rounding_is_the_rounding_a_clock_at_a_time() {
    use logicaffeine_compile::spatial::decode_engine::rtl::fp32::F32_TO_F16_LATENCY;
    if !common::require_tool("iverilog") {
        return;
    }
    let mut xs = vector_operands();
    for e in 90u32..=150 {
        for m in [0u32, 1, 0x1000, 0x0fff, 0x1001, 0x2000, 0x3000, 0x7f_e000, 0x7f_f000, 0x7f_ffff, 0x40_0000, 0x40_1000] {
            xs.push(e << 23 | m);
            xs.push(1 << 31 | e << 23 | m);
        }
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("xs.hex"), xs.iter().map(|x| format!("{x:08x}\n")).collect::<String>()).unwrap();
    std::fs::write(dir.path().join("fp32.v"), fp32_verilog()).unwrap();
    let n = xs.len();
    let tb = format!(
        r#"module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  reg [31:0] mem [0:{last}];
  reg [31:0] x;
  reg [31:0] hist [0:{lat}];
  wire [15:0] y, want;
  logos_f32_to_f16_p u(.clk(clk), .x(x), .h(y));
  logos_f32_to_f16 c(.x(hist[{held}]), .h(want));
  integer i, k, bad = 0;
  initial begin
    $readmemh("xs.hex", mem);
    for (k = 0; k <= {lat}; k = k + 1) hist[k] = 0;
    for (i = 0; i < {n} + {lat}; i = i + 1) begin
      @(negedge clk);
      if (i >= {lat}) begin
        #1;
        if (y !== want) begin
          bad = bad + 1;
          if (bad < 20) $display("logos_f32_to_f16_p(%h): pipelined %h, logos_f32_to_f16 %h", hist[{held}], y, want);
        end
      end
      for (k = {lat}; k > 0; k = k - 1) hist[k] = hist[k-1];
      hist[0] = (i < {n}) ? mem[i] : 32'd0;
      x = hist[0];
    end
    $display("OPERANDS %0d BAD %0d", {n}, bad);
    $finish;
  end
endmodule
"#,
        last = n - 1,
        lat = F32_TO_F16_LATENCY,
        held = F32_TO_F16_LATENCY - 1,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "fp32.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    let out = String::from_utf8_lossy(&run.stdout).to_string();
    assert!(run.status.success() && out.contains(&format!("OPERANDS {n} BAD 0")), "{out}");
}

/// The pipelined integer-to-f32 conversion, an integer a clock under iverilog: each clock's result `logos_i2f`'s of the
/// integer [`I2F_LATENCY`] clocks before, over every power of two and its neighbours of both signs, the extremes, zero
/// and random integers of every width.
#[test]
fn the_pipelined_integer_conversion_is_the_conversion_a_clock_at_a_time() {
    use logicaffeine_compile::spatial::decode_engine::rtl::vector::I2F_LATENCY;
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0x12f);
    let mut ns: Vec<u32> = vec![0, 1, u32::MAX, 0x8000_0000, 0x7fff_ffff, 0x8000_0001];
    for k in 0..32 {
        for d in [-1i64, 0, 1] {
            let v = (1i64 << k) + d;
            ns.push(v as u32);
            ns.push((-v) as u32);
        }
    }
    for w in 1..=32 {
        for _ in 0..40 {
            let v = r.next() as u32 & (u32::MAX >> (32 - w));
            ns.push(v);
            ns.push(v.wrapping_neg());
        }
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ints.hex"), ns.iter().map(|n| format!("{n:08x}\n")).collect::<String>()).unwrap();
    std::fs::write(dir.path().join("vector.v"), vector_verilog()).unwrap();
    let n = ns.len();
    let tb = format!(
        r#"module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  reg [31:0] mem [0:{last}];
  reg [31:0] x = 0;
  reg [31:0] hist [0:{lat}];
  wire [31:0] y, want;
  logos_i2f_p u(.clk(clk), .n(x), .y(y));
  logos_i2f c(.n(hist[{held}]), .y(want));
  integer i, k, bad = 0;
  initial begin
    $readmemh("ints.hex", mem);
    for (k = 0; k <= {lat}; k = k + 1) hist[k] = 0;
    for (i = 0; i < {n} + {lat}; i = i + 1) begin
      @(negedge clk);
      if (i >= {lat}) begin
        #1;
        if (y !== want) begin
          bad = bad + 1;
          if (bad < 20) $display("logos_i2f_p(%h): pipelined %h, logos_i2f %h", hist[{held}], y, want);
        end
      end
      for (k = {lat}; k > 0; k = k - 1) hist[k] = hist[k-1];
      hist[0] = (i < {n}) ? mem[i] : 32'd0;
      x = hist[0];
    end
    $display("INTEGERS %0d BAD %0d", {n}, bad);
    $finish;
  end
endmodule
"#,
        last = n - 1,
        lat = I2F_LATENCY,
        held = I2F_LATENCY - 1,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "vector.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    let out = String::from_utf8_lossy(&run.stdout).to_string();
    assert!(run.status.success() && out.contains(&format!("INTEGERS {n} BAD 0")), "{out}");
}

/// `module`, a pipelined vector function of `latency` clocks, an operand a clock under iverilog over every operand the
/// vector functions are held over: each clock's result `f` of the operand `latency` clocks before.
fn pipelined_function_is_its_numerics(module: &str, latency: usize, f: fn(f32) -> f32) {
    if !common::require_tool("iverilog") {
        return;
    }
    let xs = vector_operands();
    let dir = tempfile::tempdir().unwrap();
    let hex: String = xs.iter().map(|x| format!("{x:08x}\n")).collect();
    std::fs::write(dir.path().join("vectors.hex"), hex).unwrap();
    std::fs::write(dir.path().join("vector.v"), vector_verilog()).unwrap();
    let n = xs.len();
    let tb = format!(
        r#"module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  reg [31:0] mem [0:{last}];
  reg [31:0] x = 0;
  wire [31:0] y;
  {module} u(.clk(clk), .x_in(x), .y(y));
  integer i, f;
  initial begin
    $readmemh("vectors.hex", mem);
    f = $fopen("out.hex", "w");
    for (i = 0; i < {n} + {latency}; i = i + 1) begin
      @(negedge clk);
      if (i >= {latency}) $fwrite(f, "%h\n", y);
      if (i < {n}) x = mem[i];
    end
    $fclose(f);
    $finish;
  end
endmodule
"#,
        last = n - 1,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "vector.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    let out = std::fs::read_to_string(dir.path().join("out.hex")).unwrap();
    let got: Vec<&str> = out.lines().collect();
    assert_eq!(got.len(), n, "every operand's value came out of {module}");
    let wrong: Vec<String> = xs
        .iter()
        .zip(&got)
        .filter_map(|(&x, field)| {
            let g = u32::from_str_radix(field, 16).unwrap_or_else(|_| panic!("{module}({x:08x}): the rtl gave `{field}`"));
            let want = f(f32::from_bits(x)).to_bits();
            (g != want).then(|| format!("{module}({x:08x}): pipelined {g:08x}, numerics {want:08x}"))
        })
        .collect();
    assert!(wrong.is_empty(), "{} of {n} results differ:\n{}", wrong.len(), wrong.iter().take(20).cloned().collect::<Vec<_>>().join("\n"));
}

#[test]
fn the_reciprocal_unit_is_recip() {
    vector_function_is_its_numerics("logos_recip", "x_in", |x| recip(FpUnit::Soft, x));
}

#[test]
fn the_reciprocal_square_root_unit_is_rsqrt() {
    vector_function_is_its_numerics("logos_rsqrt", "x_in", |x| rsqrt(FpUnit::Soft, x));
}

#[test]
fn the_ln_unit_is_ln() {
    vector_function_is_its_numerics("logos_ln", "x_in", |x| ln(FpUnit::Soft, x));
}

#[test]
fn the_ln1p_unit_is_ln1p() {
    vector_function_is_its_numerics("logos_ln1p", "t_in", |x| ln1p(FpUnit::Soft, x));
}

#[test]
fn the_softplus_unit_is_softplus() {
    vector_function_is_its_numerics("logos_softplus", "u_in", |x| softplus(FpUnit::Soft, x));
}

#[test]
fn the_sigmoid_unit_is_sigmoid() {
    vector_function_is_its_numerics("logos_sigmoid", "x_in", |x| sigmoid(FpUnit::Soft, x));
}

#[test]
fn the_silu_unit_is_silu() {
    vector_function_is_its_numerics("logos_silu", "x", |x| silu(FpUnit::Soft, x));
}

/// Every function of the instruction set through one lane's function unit, chosen by its instruction code: each
/// result `Func::apply`'s over every operand the function units are held over, bit for bit.
#[test]
fn the_lane_function_unit_computes_every_function_of_the_instruction_set() {
    lane_function_unit_computes_every_function(LaneUnit::AtOnce);
}

/// The function unit in its stages, an operand a cycle: each result `Func::apply`'s [`LANE_FUNC_STAGES`] cycles after
/// its operand, over every operand the function units are held over, bit for bit.
#[test]
fn the_staged_lane_function_unit_computes_every_function_its_stages_later() {
    lane_function_unit_computes_every_function(LaneUnit::Staged);
}

/// The function unit for the user clock (`logos_lane_func_p`), an operand a clock on the pipelined units: each result
/// `Func::apply`'s [`LANE_FUNC_P_LATENCY`] clocks after its operand, over every operand the function units are held
/// over, bit for bit.
#[test]
fn the_pipelined_lane_function_unit_computes_every_function_a_clock_at_a_time() {
    lane_function_unit_computes_every_function(LaneUnit::Pipelined);
}

/// Which lane function unit a lock runs: the engine's at once or in its stages, or the user clock's.
#[derive(Clone, Copy)]
enum LaneUnit {
    AtOnce,
    Staged,
    Pipelined,
}

/// [`the_lane_function_unit_computes_every_function_of_the_instruction_set`] on `unit`.
fn lane_function_unit_computes_every_function(unit: LaneUnit) {
    if !common::require_tool("iverilog") {
        return;
    }
    let funcs = [Func::Exp, Func::Recip, Func::Rsqrt, Func::Sigmoid, Func::Silu, Func::Softplus, Func::Ln1p, Func::Neg, Func::F16];
    let xs = vector_operands();
    let mut cases: Vec<(Func, u64, u32)> = Vec::new();
    for f in funcs {
        let code = encode(&Op::Map { f, a: 0, d: 0 }) >> 4 & 0xf;
        cases.extend(xs.iter().map(|&x| (f, code, x)));
    }
    let dir = tempfile::tempdir().unwrap();
    let hex: String = cases.iter().map(|(_, code, x)| format!("{code:x}{x:08x}\n")).collect();
    std::fs::write(dir.path().join("vectors.hex"), hex).unwrap();
    let (source, instance, lag) = match unit {
        LaneUnit::AtOnce => (simd_verilog(), "logos_lane_func #(.STAGED(0)) u(.clk(clk), .rst(1'b0), .f(f), .x(x), .y(y));", 0),
        LaneUnit::Staged => (simd_verilog(), "logos_lane_func #(.STAGED(1)) u(.clk(clk), .rst(1'b0), .f(f), .x(x), .y(y));", LANE_FUNC_STAGES),
        LaneUnit::Pipelined => (simd_p_verilog(), "logos_lane_func_p u(.clk(clk), .f(f), .x(x), .y(y));", LANE_FUNC_P_LATENCY),
    };
    std::fs::write(dir.path().join("simd.v"), source).unwrap();
    let n = cases.len();
    let tb = format!(
        r#"module tb;
  reg [35:0] mem [0:{last}];
  reg clk = 0;
  always #5 clk = ~clk;
  reg [3:0] f = 0;
  reg [31:0] x = 0;
  wire [31:0] y;
  {instance}
  integer i, fd;
  initial begin
    $readmemh("vectors.hex", mem);
    fd = $fopen("out.hex", "w");
    for (i = 0; i < {n} + {lag}; i = i + 1) begin
      @(negedge clk);
      if (i < {n}) begin {{f, x}} = mem[i]; #1; end
      if (i >= {lag}) $fwrite(fd, "%h\n", y);
    end
    $fclose(fd);
    $finish;
  end
endmodule
"#,
        last = n - 1,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "simd.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    let out = std::fs::read_to_string(dir.path().join("out.hex")).unwrap();
    let got: Vec<&str> = out.lines().collect();
    assert_eq!(got.len(), n, "every operand simulated");
    let wrong: Vec<String> = cases
        .iter()
        .zip(&got)
        .filter_map(|(&(f, _, x), field)| {
            let want = f.apply(FpUnit::Soft, f32::from_bits(x)).to_bits();
            match u32::from_str_radix(field, 16) {
                Ok(g) if g == want => None,
                Ok(g) => Some(format!("{f:?}({x:08x}): rtl {g:08x}, numerics {want:08x}")),
                Err(_) => Some(format!("{f:?}({x:08x}): the rtl gave `{field}`")),
            }
        })
        .collect();
    assert!(wrong.is_empty(), "{} of {n} results differ:\n{}", wrong.len(), wrong.iter().take(20).cloned().collect::<Vec<_>>().join("\n"));
}

/// The fold lane gives each sum's element as the machine's fold does ([`Fold::apply`]): a pair a clock, each out its
/// latency on, the sum added to the element its slot held and the SiLU of that element times the sum, over the vector
/// unit's operands as the elements against sums of every kind — random words, signed numbers near one, the specials
/// and the operands themselves — so its SiLU is the function unit's and its product and add the engine's.
#[test]
fn the_fold_lane_folds_each_sum_into_its_element_as_the_machine_does() {
    use logicaffeine_compile::spatial::decode_engine::machine::Fold;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::compose;
    use logicaffeine_compile::spatial::decode_engine::rtl::ram::delay_verilog;
    use logicaffeine_compile::spatial::decode_engine::rtl::streaming::{fold_lane_verilog, FOLD_LATENCY};
    if !common::require_tool("iverilog") {
        return;
    }
    let olds = vector_operands();
    let mut r = Rng(0xf01d);
    let specials = [0u32, 0x8000_0000, 1, 0x0080_0000, 0x7f80_0000, 0xff80_0000, 0x7fc0_0000, 0x7f7f_ffff, 0x3f80_0000];
    let sums: Vec<u32> = olds
        .iter()
        .map(|_| match r.next() % 4 {
            0 => r.next() as u32,
            1 => (0x3f00_0000 + (r.next() as u32 & 0x00ff_ffff)) | (r.next() as u32 & 0x8000_0000),
            2 => specials[(r.next() % specials.len() as u64) as usize],
            _ => olds[(r.next() % olds.len() as u64) as usize],
        })
        .collect();
    let cases: Vec<(Fold, u32, u32)> = [Fold::Add, Fold::SwiGlu].iter().flat_map(|&f| olds.iter().zip(&sums).map(move |(&o, &s)| (f, o, s))).collect();
    let dir = tempfile::tempdir().unwrap();
    let hex: String = cases.iter().map(|&(f, o, s)| format!("{:x}{o:08x}{s:08x}\n", f.code())).collect();
    std::fs::write(dir.path().join("vectors.hex"), hex).unwrap();
    std::fs::write(dir.path().join("fold.v"), compose(&[&simd_p_verilog(), delay_verilog(), &fold_lane_verilog()])).unwrap();
    let n = cases.len();
    let tb = format!(
        r#"module tb;
  reg [67:0] mem [0:{last}];
  reg clk = 0;
  always #5 clk = ~clk;
  reg [1:0] fc = 0;
  reg [31:0] old = 0, sum = 0;
  wire [31:0] y;
  logos_fold_lane_p u(.clk(clk), .fc(fc), .old(old), .sum(sum), .y(y));
  integer i, fd;
  reg [3:0] code;
  initial begin
    $readmemh("vectors.hex", mem);
    fd = $fopen("out.hex", "w");
    for (i = 0; i < {n} + {lag}; i = i + 1) begin
      @(negedge clk);
      if (i < {n}) begin {{code, old, sum}} = mem[i]; fc = code[1:0]; #1; end
      if (i >= {lag}) $fwrite(fd, "%h\n", y);
    end
    $fclose(fd);
    $finish;
  end
endmodule
"#,
        last = n - 1,
        lag = FOLD_LATENCY,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let run = rtl_sim::Simulator::icarus(dir.path(), &["tb.v", "fold.v"]).run(dir.path(), &[]);
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    let out = std::fs::read_to_string(dir.path().join("out.hex")).unwrap();
    let got: Vec<&str> = out.lines().collect();
    assert_eq!(got.len(), n, "every pair simulated");
    let wrong: Vec<String> = cases
        .iter()
        .zip(&got)
        .filter_map(|(&(f, o, s), field)| {
            let want = f.apply(FpUnit::Soft, f32::from_bits(o), f32::from_bits(s)).to_bits();
            match u32::from_str_radix(field, 16) {
                Ok(g) if g == want => None,
                Ok(g) => Some(format!("{f:?}({o:08x}, {s:08x}): rtl {g:08x}, the machine {want:08x}")),
                Err(_) => Some(format!("{f:?}({o:08x}, {s:08x}): the rtl gave `{field}`")),
            }
        })
        .collect();
    assert!(wrong.is_empty(), "{} of {n} folds differ:\n{}", wrong.len(), wrong.iter().take(20).cloned().collect::<Vec<_>>().join("\n"));
}

/// A lane's function unit holds one core of each function — one exp, one reciprocal, one ln, one reciprocal square
/// root — and routes sigmoid, silu, softplus and ln1p through them, holding none of its own.
#[test]
fn each_lane_function_unit_holds_one_core_of_each_function() {
    let held = census(&simd_verilog(), "logos_lane_func");
    for core in ["logos_exp", "logos_recip", "logos_ln", "logos_rsqrt"] {
        assert_eq!(held.get(core).copied().unwrap_or(0), 1, "{core} in a lane's function unit: {held:?}");
    }
    for composite in ["logos_sigmoid", "logos_silu", "logos_softplus", "logos_ln1p"] {
        assert!(!held.contains_key(composite), "a lane's function unit holds its own {composite}: {held:?}");
    }
}

/// The statements in `module`'s own body that instantiate `child`, one inside a generate loop counted once.
fn own_instances(verilog: &str, module: &str, child: &str) -> usize {
    let start = verilog
        .match_indices(&format!("module {module}"))
        .map(|(i, _)| i)
        .find(|&i| (i == 0 || verilog.as_bytes()[i - 1] == b'\n') && verilog[i + 7 + module.len()..].starts_with([' ', '(', '#']))
        .unwrap_or_else(|| panic!("module {module}"));
    let body = &verilog[start..start + verilog[start..].find("endmodule").expect("its end")];
    body.lines()
        .filter(|l| l.trim_start().strip_prefix(child).is_some_and(|rest| rest.starts_with([' ', '#'])))
        .count()
}

/// A vector lane's own arithmetic, beside its function unit, is one set of four multiplies and two adds that every
/// instruction shares — a product or a sum, a dot's product and its running sum, the int8 code's product, a Hadamard
/// pair's sum and difference, a rotary pair's four products, its difference and its sum — and the scalar operations'
/// one binary unit: no instruction holds a multiply or an add of its own.
#[test]
fn each_vector_lane_shares_four_multiplies_and_two_adds_across_its_instructions() {
    let v = simd_verilog();
    let own = |child: &str| own_instances(&v, "logos_simd", child);
    assert!(own("logos_lane_func") >= 1, "the census reads the vector unit's body: {} function units", own("logos_lane_func"));
    assert!(own("logos_fmul") <= 4, "{} multiplies in a vector lane", own("logos_fmul"));
    assert!(own("logos_fadd") <= 2, "{} adds in a vector lane", own("logos_fadd"));
    assert_eq!(own("logos_fsub"), 0, "a vector lane subtracts through its own adds");
    assert!(own("logos_lane_bin") <= 1, "{} binary units in the vector unit: only the scalar operations hold one", own("logos_lane_bin"));
    assert_eq!(own("logos_code8"), 0, "a lane's int8 code multiplies through the shared multiply");
}

// ── The Gated-DeltaNet row engine ──────────────────────────────────────────────────────────────────

/// The state codec under iverilog: every binary16 value to f32 and back, and seeded f32s of every class to binary16.
#[test]
fn the_state_codec_rounds_as_the_numerics_do() {
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0xc0de);
    let halves: Vec<u16> = (0..=u16::MAX).collect();
    let mut floats: Vec<u32> = (0..60_000).map(|_| r.next() as u32).collect();
    // Near the binary16 range's edges and its denormals.
    floats.extend((0..20_000u32).map(|i| (((96 + (i % 48)) << 23) | (r.next() as u32 & 0x7f_ffff)) ^ ((i & 1) << 31)));
    let dir = tempfile::tempdir().unwrap();
    let n = halves.len().max(floats.len());
    let hex: String = (0..n).map(|i| format!("{:04x} {:08x}\n", halves.get(i).copied().unwrap_or(0), floats.get(i).copied().unwrap_or(0))).collect();
    std::fs::write(dir.path().join("vectors.txt"), hex).unwrap();
    std::fs::write(dir.path().join("gdn.v"), gdn_verilog()).unwrap();
    let tb = format!(
        r#"module tb;
  reg [15:0] h;
  reg [31:0] x;
  wire [31:0] hx;
  wire [15:0] xh;
  logos_f16_to_f32 d(.h(h), .x(hx));
  logos_f32_to_f16 e(.x(x), .h(xh));
  integer i, f, g, c;
  initial begin
    f = $fopen("vectors.txt", "r");
    g = $fopen("out.txt", "w");
    for (i = 0; i < {n}; i = i + 1) begin
      c = $fscanf(f, "%h %h\n", h, x); #1;
      $fwrite(g, "%h %h\n", hx, xh);
    end
    $fclose(g);
    $finish;
  end
endmodule
"#
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-o", "sim", "tb.v", "gdn.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    let out = std::fs::read_to_string(dir.path().join("out.txt")).unwrap();
    let mut wrong = Vec::new();
    for (i, line) in out.lines().enumerate() {
        let (a, b) = line.split_once(' ').unwrap();
        let (hx, xh) = (u32::from_str_radix(a, 16).unwrap(), u16::from_str_radix(b, 16).unwrap() as u16);
        if let Some(&h) = halves.get(i) {
            let want = logicaffeine_compile::model::quant::f16_to_f32(h).to_bits();
            if hx != want {
                wrong.push(format!("f16 {h:04x} to f32: rtl {hx:08x}, numerics {want:08x}"));
            }
        }
        if let Some(&x) = floats.get(i) {
            let want = f32_to_f16(f32::from_bits(x));
            if xh != want {
                wrong.push(format!("f32 {x:08x} to f16: rtl {xh:04x}, numerics {want:04x}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{} conversions differ:\n{}", wrong.len(), wrong.iter().take(20).cloned().collect::<Vec<_>>().join("\n"));
}

/// Seeded rows of a 128-wide head streamed through `logos_gdn_row_engine`: every updated element and every read-out
/// `gdn_row`'s, the state stored in f32 and in f16.
#[test]
fn the_gdn_row_engine_updates_and_reads_a_row_as_the_numerics_do() {
    if !common::require_tool("iverilog") {
        return;
    }
    const N: usize = 128;
    struct Case {
        row: Vec<f32>,
        k: Vec<f32>,
        q: Vec<f32>,
        decay: f32,
        beta: f32,
        v: f32,
        kq: f32,
        scale: f32,
        f16: bool,
    }
    let mut r = Rng(0x6d4);
    let mut unit = || ((r.next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0;
    let mut cases = Vec::new();
    for c in 0..160 {
        let f16 = c % 2 == 1;
        let store = |x: f32| if f16 { logicaffeine_compile::model::quant::f16_to_f32(f32_to_f16(x)) } else { x };
        let row: Vec<f32> = (0..N).map(|_| store(unit() * 0.3)).collect();
        let k: Vec<f32> = (0..N).map(|_| unit() * 0.09).collect();
        let q: Vec<f32> = (0..N).map(|_| unit() * 0.09).collect();
        let kq = logicaffeine_compile::spatial::decode_engine::numerics::fdot(FpUnit::Soft, &k, &q);
        cases.push(Case { row, k, q, decay: 0.5 + 0.49 * (unit() + 1.0) / 2.0, beta: (unit() + 1.0) / 2.0, v: unit(), kq, scale: 0.088388, f16 });
    }
    let dir = tempfile::tempdir().unwrap();
    let mut hex = String::new();
    let vec_hex = |v: &[f32]| -> String { v.iter().rev().map(|x| format!("{:08x}", x.to_bits())).collect() };
    for c in &cases {
        writeln!(hex, "{}\n{}\n{:08x}\n{:08x}\n{:08x}\n{:08x}\n{:08x}\n{}", vec_hex(&c.k), vec_hex(&c.q), c.decay.to_bits(), c.beta.to_bits(), c.kq.to_bits(), c.scale.to_bits(), c.v.to_bits(), c.f16 as u8).unwrap();
        for b in 0..N / 16 {
            writeln!(hex, "{}", vec_hex(&c.row[16 * b..16 * b + 16])).unwrap();
        }
    }
    std::fs::write(dir.path().join("vectors.hex"), hex).unwrap();
    std::fs::write(dir.path().join("gdn.v"), gdn_verilog()).unwrap();
    let per = 8 + N / 16;
    let tb = format!(
        r#"module tb;
  reg [4095:0] mem [0:{last}];
  reg clk = 0, rst = 1;
  reg [4095:0] k, q;
  reg [31:0] decay, beta, kq, scale, v;
  reg f16, in_valid, in_first;
  reg [511:0] in_data;
  wire out_valid, out_last;
  wire [511:0] out_data;
  wire [31:0] out_read;
  logos_gdn_row_engine #(.N({N})) u(.clk(clk), .rst(rst), .k(k), .q(q), .decay(decay), .beta(beta), .kq(kq), .scale(scale),
    .f16(f16), .in_valid(in_valid), .in_first(in_first), .in_data(in_data), .in_v(v),
    .out_valid(out_valid), .out_last(out_last), .out_data(out_data), .out_read(out_read));
  always #5 clk = ~clk;
  integer c, b, f;
  reg [4095:0] word;
  initial begin
    $readmemh("vectors.hex", mem);
    f = $fopen("out.hex", "w");
    in_valid = 0; in_first = 0;
    @(posedge clk); @(posedge clk); rst <= 0;
    for (c = 0; c < {count}; c = c + 1) begin
      k = mem[{per}*c]; q = mem[{per}*c+1]; decay = mem[{per}*c+2]; beta = mem[{per}*c+3]; kq = mem[{per}*c+4];
      scale = mem[{per}*c+5]; v = mem[{per}*c+6]; word = mem[{per}*c+7]; f16 = word[0];
      for (b = 0; b < {beats}; b = b + 1) begin
        @(negedge clk); in_valid = 1; in_first = (b == 0); in_data = mem[{per}*c+8+b];
      end
      @(negedge clk); in_valid = 0; in_first = 0;
      begin : wait_out
        while (1) begin
          @(posedge clk); #1;
          if (out_valid) $fwrite(f, "%h\n", out_data);
          if (out_valid && out_last) begin $fwrite(f, "%h\n", out_read); disable wait_out; end
        end
      end
    end
    $fclose(f);
    $finish;
  end
endmodule
"#,
        last = per * cases.len() - 1,
        count = cases.len(),
        beats = N / 16,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-o", "sim", "tb.v", "gdn.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    let out = std::fs::read_to_string(dir.path().join("out.hex")).unwrap();
    let mut lines = out.lines();
    let mut wrong = Vec::new();
    for (ci, c) in cases.iter().enumerate() {
        let mut want_row = c.row.clone();
        let f16 = c.f16;
        let want_read = gdn_row(FpUnit::Soft, &mut want_row, &c.k, &c.q, c.decay, c.beta, c.v, c.kq, c.scale, |x| {
            if f16 { logicaffeine_compile::model::quant::f16_to_f32(f32_to_f16(x)) } else { x }
        });
        for b in 0..N / 16 {
            let line = lines.next().unwrap_or_else(|| panic!("case {ci}: beat {b} missing"));
            for l in 0..16 {
                let got = hex_field(line, l, 8);
                let want = want_row[16 * b + l].to_bits();
                if got != want {
                    wrong.push(format!("case {ci} (f16 {f16}) element {}: rtl {got:08x}, numerics {want:08x}", 16 * b + l));
                }
            }
        }
        let read = u32::from_str_radix(lines.next().unwrap_or_else(|| panic!("case {ci}: no read-out")), 16).unwrap();
        if read != want_read.to_bits() {
            wrong.push(format!("case {ci} (f16 {f16}) read-out: rtl {read:08x}, numerics {:08x}", want_read.to_bits()));
        }
    }
    assert!(wrong.is_empty(), "{} values differ:\n{}", wrong.len(), wrong.iter().take(20).cloned().collect::<Vec<_>>().join("\n"));
}

// ── The vector unit ────────────────────────────────────────────────────────────────────────────────

/// One micro-program over loaded registers: the vectors `inputs` into registers 0.., the scalars `scalars` into
/// scalar registers 0..; afterwards `check` names the vector registers to compare.
struct SimdCase {
    program: Vec<Op>,
    inputs: Vec<Vec<f32>>,
    scalars: Vec<f32>,
    check: Vec<u8>,
}

/// Every micro-program under iverilog on `logos_simd`: each checked vector register and every scalar register the
/// interpreter's, bit for bit.
#[test]
fn the_vector_unit_runs_each_micro_program_as_the_interpreter_does() {
    vector_unit_runs_as_the_interpreter(&micro_program_cases(), &[16], VectorUnit::Engine);
}

/// Every micro-program under iverilog on the user clock's vector unit (`logos_simd_p`): each checked vector register
/// and every scalar register the interpreter's, bit for bit.
#[test]
fn the_user_clock_vector_unit_runs_each_micro_program_as_the_interpreter_does() {
    vector_unit_runs_as_the_interpreter(&micro_program_cases(), &[16], VectorUnit::UserClock);
}

/// The cases [`the_vector_unit_runs_each_micro_program_as_the_interpreter_does`] runs.
fn micro_program_cases() -> Vec<SimdCase> {
    let mut r = Rng(0x51d);
    let mut f = || (((r.next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0) * 4.0;
    let mut cases = Vec::new();
    for n in [16usize, 37, 128, 1000, 5120] {
        let (x, w): (Vec<f32>, Vec<f32>) = ((0..n).map(|_| f()).collect(), (0..n).map(|_| f()).collect());
        cases.push(SimdCase { program: vector::rmsnorm(FpUnit::Soft, 0, 1, 2, 3, n, 1e-6), inputs: vec![x.clone(), w.clone()], scalars: vec![], check: vec![2, 3] });
        cases.push(SimdCase { program: vector::l2norm(0, 2, 1e-6), inputs: vec![x.clone()], scalars: vec![], check: vec![2] });
        cases.push(SimdCase { program: vector::swiglu(0, 1, 2), inputs: vec![x.clone(), w.clone()], scalars: vec![], check: vec![2] });
        cases.push(SimdCase { program: vector::sigmoid_gate(0, 1, 2, 3), inputs: vec![x.clone(), w.clone()], scalars: vec![], check: vec![2, 3] });
        let scores: Vec<f32> = x.iter().map(|v| v * 10.0).collect();
        cases.push(SimdCase { program: vector::softmax(0, 1), inputs: vec![scores], scalars: vec![], check: vec![1] });
    }
    for n in [1usize, 2, 8, 16, 64, 256] {
        let x: Vec<f32> = (0..n).map(|_| f()).collect();
        cases.push(SimdCase { program: vec![Op::Hadamard { a: 0, d: 1 }], inputs: vec![x], scalars: vec![], check: vec![1] });
    }
    for _ in 0..20 {
        let scalars = vec![f() * 8.0, f(), -f().abs() * 2.0, f() * 5.0];
        cases.push(SimdCase { program: vector::gdn_gates(0, 1, 2, 3, 4, 5, 6), inputs: vec![], scalars, check: vec![] });
    }
    cases
}

/// The vector unit with its sixteen lanes sharing 1, 2, 4 and 8 function units: every function mapped over operands
/// of every class in every lane of a slice, and the micro-programs that map a function over a vector or a scalar, each
/// register the interpreter's, bit for bit.
#[test]
fn the_vector_unit_shares_its_function_units_across_its_lanes() {
    vector_unit_runs_as_the_interpreter(&shared_function_unit_cases(), &[1, 2, 4, 8], VectorUnit::Engine);
}

/// The user clock's vector unit with its sixteen lanes sharing 1, 2, 4 and 8 pipelined function units: every function
/// over operands of every class in every lane of a slice, and the micro-programs that map a function over a vector or
/// a scalar, each register the interpreter's, bit for bit.
#[test]
fn the_user_clock_vector_unit_shares_its_function_units_across_its_lanes() {
    vector_unit_runs_as_the_interpreter(&shared_function_unit_cases(), &[1, 2, 4, 8], VectorUnit::UserClock);
}

/// The cases [`the_vector_unit_shares_its_function_units_across_its_lanes`] runs.
fn shared_function_unit_cases() -> Vec<SimdCase> {
    let mut r = Rng(0xf0f0);
    let mut f = || (((r.next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0) * 4.0;
    let operands: Vec<f32> = vector_operands().iter().step_by(7).take(245).map(|&b| f32::from_bits(b)).collect();
    let mut cases = Vec::new();
    for func in [Func::Exp, Func::Recip, Func::Rsqrt, Func::Sigmoid, Func::Silu, Func::Softplus, Func::Ln1p, Func::Neg, Func::F16] {
        cases.push(SimdCase { program: vec![Op::Map { f: func, a: 0, d: 1 }], inputs: vec![operands.clone()], scalars: vec![], check: vec![1] });
    }
    for n in [16usize, 37, 300] {
        let mut v = || (0..n).map(|_| f()).collect::<Vec<f32>>();
        let (x, w, z) = (v(), v(), v());
        cases.push(SimdCase { program: vector::swiglu(0, 1, 2), inputs: vec![x.clone(), w.clone()], scalars: vec![], check: vec![2] });
        cases.push(SimdCase { program: vector::sigmoid_gate(0, 1, 2, 3), inputs: vec![x.clone(), w.clone()], scalars: vec![], check: vec![2, 3] });
        let scores: Vec<f32> = x.iter().map(|v| v * 10.0).collect();
        cases.push(SimdCase { program: vector::softmax(0, 1), inputs: vec![scores], scalars: vec![], check: vec![1] });
        cases.push(SimdCase { program: vector::gdn_out(FpUnit::Soft, 0, 1, 2, 3, 4, n, 1e-6), inputs: vec![x.clone(), w.clone(), z.clone()], scalars: vec![], check: vec![3] });
        let a: Vec<f32> = z.iter().map(|v| -v.abs() * 2.0).collect();
        let beta: Vec<f32> = w.iter().map(|v| v * 5.0).collect();
        cases.push(SimdCase { program: vector::delta_gates(0, 1, 2, 3, 4, 5, 6), inputs: vec![x, z, a, beta], scalars: vec![], check: vec![4, 5, 6] });
    }
    for _ in 0..8 {
        let scalars = vec![f() * 8.0, f(), -f().abs() * 2.0, f() * 5.0];
        cases.push(SimdCase { program: vector::gdn_gates(0, 1, 2, 3, 4, 5, 6), inputs: vec![], scalars, check: vec![] });
    }
    cases
}

/// The vector unit's functions run on one row of shared function units and no other unit of their own: a scalar
/// function on the first. The engine's vector unit shares `FUNCTION_UNITS` of them among its sixteen lanes, a
/// divisor of sixteen below it.
#[test]
fn the_vector_unit_holds_one_row_of_shared_function_units() {
    let v = simd_verilog();
    assert_eq!(own_instances(&v, "logos_simd", "logos_lane_func"), 1, "the vector unit's own function units");
    assert!(16 % FUNCTION_UNITS == 0 && FUNCTION_UNITS < 16, "the engine's vector unit shares {FUNCTION_UNITS} function units");
}

/// The user clock's vector unit beside the engine's, driven alike under iverilog with 16, 8, 4 and 1 function units:
/// random programs over every instruction — elementwise, Map and ScalarMap of every function, Dot, Sum, Max, AbsMax and
/// ArgMax, scalar operations, int8 codes, power-of-two scales, multiply-adds of both addends, Hadamard transforms in place and apart over every power
/// of two to 256, rotations of both pairings over tables that straddle rows, in place and apart — over registers of
/// every length from empty, every row of every register defined before the first. After every instruction every row of
/// every bank and every scalar register is the engine unit's, and so is every length.
#[test]
fn the_user_clock_vector_unit_runs_every_instruction_as_the_engines_does() {
    if !common::require_tool("iverilog") {
        return;
    }
    const ROWS: usize = 16;
    let mut wrong = Vec::new();
    for (units, seed) in [(16usize, 0x7e57u64), (8, 0x7e5a), (4, 0x7e58), (1, 0x7e59)] {
        let mut r = Rng(seed);
        let mut script = String::new();
        let word = |r: &mut Rng| -> String { (0..16).map(|_| format!("{:08x}", r.next() as u32)).collect() };
        for reg in 0..8 {
            for row in 0..ROWS {
                writeln!(script, "W {reg:x} {row:x} {}", word(&mut r)).unwrap();
            }
        }
        let special = [0.0f32, -0.0, f32::INFINITY, f32::NEG_INFINITY, f32::NAN, 1.0, -1.0, 16.0, 15.999999, 16.000002, 88.7, -88.7, -103.0, 1e-40, -1e-40, 3.4e38, 1e-7, 0.5, -0.5, 2.5, -127.5];
        let value = |r: &mut Rng| -> u32 {
            match r.next() % 20 {
                0..=2 => special[(r.next() % special.len() as u64) as usize].to_bits(),
                3 => r.next() as u32,
                4..=6 => ((((r.next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0) * 100.0).to_bits(),
                _ => ((((r.next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0) * 6.0).to_bits(),
            }
        };
        let load = |r: &mut Rng, script: &mut String, reg: u64, n: u64| {
            writeln!(script, "L {reg:x} {n:x}").unwrap();
            for row in 0..n.div_ceil(16).max(1) {
                let w: String = (0..16).rev().map(|i| format!("{:08x}", if 16 * row + i < n { value(r) } else { r.next() as u32 })).collect();
                writeln!(script, "W {reg:x} {row:x} {w}").unwrap();
            }
        };
        let pick = |r: &mut Rng, of: &[u64]| of[(r.next() % of.len() as u64) as usize];
        let ins = |op: u64, sub: u64, a: u64, b: u64, d: u64, s: u64, k: u64| op | sub << 4 | a << 8 | b << 12 | d << 16 | s << 20 | (k & 0xffff_ffff) << 32;
        for _ in 0..40 {
            let mut regs: Vec<u64> = (0..8).collect();
            for i in 0..4 {
                let j = i + (r.next() % (8 - i as u64)) as usize;
                regs.swap(i, j);
            }
            let regs = regs[..4].to_vec();
            for &reg in &regs {
                let n = pick(&mut r, &[0, 1, 2, 3, 15, 16, 17, 31, 32, 48, 64, 100, 128, 256]);
                load(&mut r, &mut script, reg, n);
            }
            for s_reg in 0..8 {
                let v = value(&mut r) as u64;
                writeln!(script, "I {:016x}", ins(8, 0, 0, 0, s_reg, 0, v)).unwrap();
            }
            for _ in 0..3 + r.next() % 10 {
                let (a, b, d) = (pick(&mut r, &regs), pick(&mut r, &regs), r.next() % 8);
                let funcs = [0u64, 1, 2, 3, 4, 5, 6, 7, 8];
                let w = match r.next() % 55 {
                    50..=52 => ins(15, 0, a, b, d, pick(&mut r, &regs), 0),
                    53..=54 => ins(15, 1, a, b, d, r.next() % 8, 0),
                    0..=9 => ins(5, pick(&mut r, &funcs), a, 0, d, 0, 0),
                    10..=13 => ins(7, pick(&mut r, &funcs), r.next() % 8, 0, r.next() % 8, 0, 0),
                    14..=17 => ins(3, r.next() % 3, a, b, d, 0, 0),
                    18..=20 => ins(4, r.next() % 3, a, 0, d, r.next() % 8, 0),
                    21..=23 => ins(0, 0, a, b, r.next() % 8, 0, 0),
                    24..=25 => ins(1, 0, a, 0, r.next() % 8, 0, 0),
                    26..=27 => ins(2, 0, a, 0, r.next() % 8, 0, 0),
                    28..=29 => ins(11, 0, a, 0, r.next() % 8, 0, 0),
                    30..=31 => ins(12, 0, a, 0, r.next() % 8, 0, 0),
                    32..=33 => ins(6, r.next() % 3, r.next() % 8, r.next() % 8, r.next() % 8, 0, 0),
                    34..=35 => ins(13, 0, a, 0, d, r.next() % 8, 0),
                    36..=37 => ins(14, 0, a, 0, d, 0, pick(&mut r, &[0, 1, (-1i64) as u64, 3, (-7i64) as u64, 20, (-20i64) as u64, 127, (-126i64) as u64, 200, (-200i64) as u64])),
                    38..=43 => {
                        let h = pick(&mut r, &regs);
                        let n = pick(&mut r, &[1, 2, 4, 8, 16, 32, 64, 128, 256]);
                        load(&mut r, &mut script, h, n);
                        ins(9, 0, h, 0, if r.next() % 2 == 0 { h } else { r.next() % 8 }, 0, 0)
                    }
                    _ => {
                        let pairs = pick(&mut r, &[0, 1, 7, 16, 17, 32, 40, 64]);
                        let mut three: Vec<u64> = (0..8).collect();
                        for i in 0..3 {
                            let j = i + (r.next() % (8 - i as u64)) as usize;
                            three.swap(i, j);
                        }
                        let (av, cv, sv) = (three[0], three[1], three[2]);
                        let n = pick(&mut r, &[2 * pairs, 2 * pairs + 1, 2 * pairs + 16, 2 * pairs + 33]);
                        load(&mut r, &mut script, av, n);
                        load(&mut r, &mut script, cv, pairs);
                        load(&mut r, &mut script, sv, pairs);
                        let apart: Vec<u64> = (0..8).filter(|&x| x != cv && x != sv).collect();
                        let dv = if r.next() % 2 == 0 { av } else { pick(&mut r, &apart) };
                        ins(10, r.next() % 2, av, cv, dv, sv, 0)
                    }
                };
                writeln!(script, "I {w:016x}").unwrap();
            }
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("script.txt"), &script).unwrap();
        std::fs::write(dir.path().join("simd.v"), simd_p_verilog()).unwrap();
        let banks: String = (0..8)
            .map(|b| {
                format!(
                    "            if (u0.bank[{b}].ca.mem[rw] !== u1.bank[{b}].ca.mem[rw] || u1.bank[{b}].cb.mem[rw] !== u1.bank[{b}].ca.mem[rw] || u1.bank[{b}].cc.mem[rw] !== u1.bank[{b}].ca.mem[rw]) begin bad = bad + 1; if (bad < 20) $display(\"instruction %0d %h: bank {b} row %0d differs\", instrs, w64, rw); end\n"
                )
            })
            .collect();
        let tb = format!(
            r#"module tb;
  reg clk = 0, rst = 1;
  reg issue = 0, wr = 0, len_wr = 0;
  reg [63:0] instr;
  reg [3:0] wr_reg, rd_reg = 0, rd_s = 0;
  reg [15:0] wr_row, rd_row = 0;
  reg [511:0] wr_data;
  reg [19:0] len_val;
  wire busy0, busy1;
  wire [511:0] rd0, rd1;
  wire [31:0] rs0, rs1;
  logos_simd #(.ROWS({ROWS}), .VREGS(8), .FU({units})) u0(.clk(clk), .rst(rst), .issue(issue), .instr(instr), .busy(busy0), .wr(wr),
    .wr_reg(wr_reg), .wr_row(wr_row), .wr_data(wr_data), .len_wr(len_wr), .len_val(len_val), .rd_reg(rd_reg), .rd_row(rd_row),
    .rd_data(rd0), .rd_s(rd_s), .rd_s_val(rs0));
  logos_simd_p #(.ROWS({ROWS}), .VREGS(8), .FU({units})) u1(.clk(clk), .rst(rst), .issue(issue), .instr(instr), .busy(busy1),
    .group(8'd1), .group_rows(16'd0), .wr(wr), .wr_reg(wr_reg), .wr_row(wr_row), .wr_data(wr_data), .len_wr(len_wr),
    .len_val(len_val), .rd_reg(rd_reg), .rd_row(rd_row), .rd_data(rd1), .rd_s(rd_s), .rd_s_pass(8'd0), .rd_s_val(rs1));
  always #5 clk = ~clk;
  integer fi, c, rg, rw, bad = 0, instrs = 0;
  reg [7:0] tag;
  reg [31:0] a, b;
  reg [511:0] word;
  reg [63:0] w64;
  initial begin
    fi = $fopen("script.txt", "r");
    @(posedge clk); @(posedge clk); rst <= 0;
    while (!$feof(fi)) begin
      c = $fscanf(fi, "%c", tag);
      if (tag == "L") begin
        c = $fscanf(fi, " %h %h\n", a, b);
        @(negedge clk); len_wr = 1; wr_reg = a; len_val = b; @(negedge clk); len_wr = 0;
      end else if (tag == "W") begin
        c = $fscanf(fi, " %h %h %h\n", a, b, word);
        @(negedge clk); wr = 1; wr_reg = a; wr_row = b; wr_data = word; @(negedge clk); wr = 0;
      end else if (tag == "I") begin
        c = $fscanf(fi, " %h\n", w64);
        instrs = instrs + 1;
        @(negedge clk); issue = 1; instr = w64; @(negedge clk); issue = 0;
        @(negedge clk); while (busy0 || busy1) @(negedge clk);
        @(negedge clk);
        for (rg = 0; rg < 8; rg = rg + 1) begin
          rd_s = rg; #1;
          if (rs0 !== rs1) begin bad = bad + 1; if (bad < 20) $display("instruction %0d %h: scalar %0d %h, engine's %h", instrs, w64, rg, rs1, rs0); end
          if (u0.len[rg] !== u1.len[rg]) begin bad = bad + 1; if (bad < 20) $display("instruction %0d %h: length %0d", instrs, w64, rg); end
        end
        for (rw = 0; rw < {ROWS}; rw = rw + 1) begin
{banks}        end
      end
    end
    $display("INSTRUCTIONS %0d BAD %0d", instrs, bad);
    $finish;
  end
endmodule
"#
        );
        std::fs::write(dir.path().join("tb.v"), tb).unwrap();
        let run = rtl_sim::Simulator::build(dir.path(), &["tb.v", "simd.v"]).run(dir.path(), &[]);
        let out = String::from_utf8_lossy(&run.stdout).to_string();
        assert!(run.status.success(), "the simulation: {}", String::from_utf8_lossy(&run.stderr));
        let instructions: usize = out.split("INSTRUCTIONS ").nth(1).and_then(|t| t.split_whitespace().next()).and_then(|n| n.parse().ok()).unwrap_or(0);
        assert!(instructions > 200, "{units} function units: the programs ran: {out}");
        if !out.contains(" BAD 0") {
            wrong.push(format!("{units} function units:\n{out}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The user clock's vector unit runs an instruction over a group of passes as each pass alone: a unit of ADD passes
/// (`PASSES`), given groups of one to ADD passes of sixteen rows each, beside one-pass units each holding one pass's
/// registers at rows 0..16 and run alike — random programs over every instruction a group runs (elementwise of every
/// operation and with a scalar, Map and ScalarMap of every function, Dot, Sum, Max, AbsMax and ArgMax, scalar operations,
/// constants, int8 codes and power-of-two scales) over registers of every length from empty, every pass's values its
/// own, under 16, 8, 4 and 1 function units. After every instruction every row of every pass's registers, every pass's
/// scalar registers and every length are the one-pass unit's.
#[test]
fn the_user_clock_vector_unit_runs_a_group_of_passes_as_each_pass_alone() {
    use logicaffeine_compile::spatial::decode_engine::rtl::fp32::FADD_LATENCY;
    let most = FADD_LATENCY + 1;
    vector_unit_groups_run_as_each_pass_alone(most, &[(16, most, 0x9a55), (8, most, 0x9a5a), (4, 3, 0x9a56), (1, 2, 0x9a57), (16, 1, 0x9a58), (4, most, 0x9a59)]);
}

/// [`the_user_clock_vector_unit_runs_a_group_of_passes_as_each_pass_alone`] past the add's latency and one: a unit of
/// twelve passes given groups of seven and of eleven, a Dot's and a Sum's passes in subgroups of the add's latency and
/// one, the last subgroup short.
#[test]
fn the_user_clock_vector_unit_runs_a_group_past_the_adds_latency_as_each_pass_alone() {
    vector_unit_groups_run_as_each_pass_alone(12, &[(4, 11, 0x9a5b), (16, 7, 0x9a5c)]);
}

/// For each `(units, group, seed)` of `cases`: a user-clock vector unit of `passes` passes and `units` function units
/// given a group of `group` passes of sixteen rows each, beside one-pass units each holding one pass's registers, run
/// alike over random programs seeded `seed`; every pass's registers, scalars and lengths the one-pass unit's after
/// every instruction.
fn vector_unit_groups_run_as_each_pass_alone(passes: usize, cases: &[(usize, usize, u64)]) {
    if !common::require_tool("iverilog") {
        return;
    }
    const RP: usize = 16;
    let mut wrong = Vec::new();
    for &(units, group, seed) in cases {
        let mut r = Rng(seed);
        let mut script = String::new();
        let word = |r: &mut Rng| -> String { (0..16).map(|_| format!("{:08x}", r.next() as u32)).collect() };
        for p in 0..group {
            for reg in 0..8 {
                for row in 0..RP {
                    writeln!(script, "W {p:x} {reg:x} {row:x} {}", word(&mut r)).unwrap();
                }
            }
        }
        let special = [0.0f32, -0.0, f32::INFINITY, f32::NEG_INFINITY, f32::NAN, 1.0, -1.0, 16.0, 15.999999, 16.000002, 88.7, -88.7, -103.0, 1e-40, -1e-40, 3.4e38, 1e-7, 0.5, -0.5, 2.5, -127.5];
        let value = |r: &mut Rng| -> u32 {
            match r.next() % 20 {
                0..=2 => special[(r.next() % special.len() as u64) as usize].to_bits(),
                3 => r.next() as u32,
                4..=6 => ((((r.next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0) * 100.0).to_bits(),
                _ => ((((r.next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0) * 6.0).to_bits(),
            }
        };
        // A register's length, the same in every pass, and each pass's own values in its rows.
        let load = |r: &mut Rng, script: &mut String, reg: u64, n: u64| {
            writeln!(script, "L {reg:x} {n:x}").unwrap();
            for p in 0..group {
                for row in 0..n.div_ceil(16).max(1) {
                    let w: String = (0..16).rev().map(|i| format!("{:08x}", if 16 * row + i < n { value(r) } else { r.next() as u32 })).collect();
                    writeln!(script, "W {p:x} {reg:x} {row:x} {w}").unwrap();
                }
            }
        };
        let pick = |r: &mut Rng, of: &[u64]| of[(r.next() % of.len() as u64) as usize];
        let ins = |op: u64, sub: u64, a: u64, b: u64, d: u64, s: u64, k: u64| op | sub << 4 | a << 8 | b << 12 | d << 16 | s << 20 | (k & 0xffff_ffff) << 32;
        for _ in 0..30 {
            let mut regs: Vec<u64> = (0..8).collect();
            for i in 0..4 {
                let j = i + (r.next() % (8 - i as u64)) as usize;
                regs.swap(i, j);
            }
            let regs = regs[..4].to_vec();
            for &reg in &regs {
                let n = pick(&mut r, &[0, 1, 2, 3, 15, 16, 17, 31, 32, 48, 64, 100, 128, 256]);
                load(&mut r, &mut script, reg, n);
            }
            for s_reg in 0..8 {
                let v = value(&mut r) as u64;
                writeln!(script, "I {:016x}", ins(8, 0, 0, 0, s_reg, 0, v)).unwrap();
            }
            for _ in 0..3 + r.next() % 10 {
                let (a, b, d) = (pick(&mut r, &regs), pick(&mut r, &regs), r.next() % 8);
                let funcs = [0u64, 1, 2, 3, 4, 5, 6, 7, 8];
                let w = match r.next() % 49 {
                    44..=46 => ins(15, 0, a, b, d, pick(&mut r, &regs), 0),
                    47..=48 => ins(15, 1, a, b, d, r.next() % 8, 0),
                    0..=9 => ins(5, pick(&mut r, &funcs), a, 0, d, 0, 0),
                    10..=13 => ins(7, pick(&mut r, &funcs), r.next() % 8, 0, r.next() % 8, 0, 0),
                    14..=17 => ins(3, r.next() % 3, a, b, d, 0, 0),
                    18..=20 => ins(4, r.next() % 3, a, 0, d, r.next() % 8, 0),
                    21..=24 => ins(0, 0, a, b, r.next() % 8, 0, 0),
                    25..=27 => ins(1, 0, a, 0, r.next() % 8, 0, 0),
                    28..=29 => ins(2, 0, a, 0, r.next() % 8, 0, 0),
                    30..=31 => ins(11, 0, a, 0, r.next() % 8, 0, 0),
                    32..=33 => ins(12, 0, a, 0, r.next() % 8, 0, 0),
                    34..=37 => ins(6, r.next() % 3, r.next() % 8, r.next() % 8, r.next() % 8, 0, 0),
                    38..=40 => ins(13, 0, a, 0, d, r.next() % 8, 0),
                    _ => ins(14, 0, a, 0, d, 0, pick(&mut r, &[0, 1, (-1i64) as u64, 3, (-7i64) as u64, 20, (-20i64) as u64, 127, (-126i64) as u64, 200, (-200i64) as u64])),
                };
                writeln!(script, "I {w:016x}").unwrap();
            }
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("script.txt"), &script).unwrap();
        std::fs::write(dir.path().join("simd.v"), simd_p_verilog()).unwrap();
        let alone: String = (0..group)
            .map(|p| {
                format!(
                    "  wire busy_{p};\n  logos_simd_p #(.ROWS({RP}), .VREGS(8), .FU({units})) u{p}(.clk(clk), .rst(rst), .issue(issue), .instr(instr), .busy(busy_{p}),\n    \
                     .group(8'd1), .group_rows(16'd0), .wr(wr && wp == {p}), .wr_reg(wr_reg), .wr_row(wr_row), .wr_data(wr_data), .len_wr(len_wr),\n    \
                     .len_val(len_val), .rd_reg(4'd0), .rd_row(16'd0), .rd_data(), .rd_s(4'd0), .rd_s_pass(8'd0), .rd_s_val());\n"
                )
            })
            .collect();
        let busy: String = (0..group).map(|p| format!(" || busy_{p}")).collect();
        let checks: String = (0..group)
            .map(|p| {
                let banks: String = (0..8)
                    .map(|b| format!("          if (g.bank[{b}].ca.mem[{p} * {RP} + rw] !== u{p}.bank[{b}].ca.mem[rw]) begin bad = bad + 1; if (bad < 20) $display(\"instruction %0d %h: pass {p} bank {b} row %0d differs\", instrs, w64, rw); end\n"))
                    .collect();
                format!(
                    "        for (rw = 0; rw < {RP}; rw = rw + 1) begin\n{banks}        end\n        for (rg = 0; rg < 8; rg = rg + 1) begin\n          \
                     if (g.sr[{p} * 8 + rg] !== u{p}.sr[rg]) begin bad = bad + 1; if (bad < 20) $display(\"instruction %0d %h: pass {p} scalar %0d %h, alone %h\", instrs, w64, rg, g.sr[{p} * 8 + rg], u{p}.sr[rg]); end\n          \
                     if (g.len[rg] !== u{p}.len[rg]) begin bad = bad + 1; if (bad < 20) $display(\"instruction %0d %h: length %0d\", instrs, w64, rg); end\n        end\n"
                )
            })
            .collect();
        let tb = format!(
            r#"module tb;
  reg clk = 0, rst = 1;
  reg issue = 0, wr = 0, len_wr = 0;
  reg [63:0] instr;
  reg [3:0] wr_reg;
  reg [7:0] wp;
  reg [15:0] wr_row;
  reg [511:0] wr_data;
  reg [19:0] len_val;
  wire busy_g;
  logos_simd_p #(.ROWS({rows}), .VREGS(8), .FU({units}), .PASSES({passes})) g(.clk(clk), .rst(rst), .issue(issue), .instr(instr), .busy(busy_g),
    .group(8'd{group}), .group_rows(16'd{RP}), .wr(wr), .wr_reg(wr_reg), .wr_row(wp * {RP} + wr_row), .wr_data(wr_data), .len_wr(len_wr),
    .len_val(len_val), .rd_reg(4'd0), .rd_row(16'd0), .rd_data(), .rd_s(4'd0), .rd_s_pass(8'd0), .rd_s_val());
{alone}  always #5 clk = ~clk;
  integer fi, c, rg, rw, bad = 0, instrs = 0;
  reg [7:0] tag;
  reg [31:0] a, b;
  reg [511:0] word;
  reg [63:0] w64;
  initial begin
    fi = $fopen("script.txt", "r");
    @(posedge clk); @(posedge clk); rst <= 0;
    while (!$feof(fi)) begin
      c = $fscanf(fi, "%c", tag);
      if (tag == "L") begin
        c = $fscanf(fi, " %h %h\n", a, b);
        @(negedge clk); len_wr = 1; wr_reg = a; len_val = b; @(negedge clk); len_wr = 0;
      end else if (tag == "W") begin
        c = $fscanf(fi, " %h %h %h %h\n", wp, a, b, word);
        @(negedge clk); wr = 1; wr_reg = a; wr_row = b; wr_data = word; @(negedge clk); wr = 0;
      end else if (tag == "I") begin
        c = $fscanf(fi, " %h\n", w64);
        instrs = instrs + 1;
        @(negedge clk); issue = 1; instr = w64; @(negedge clk); issue = 0;
        @(negedge clk); while (busy_g{busy}) @(negedge clk);
        @(negedge clk);
{checks}      end
    end
    $display("INSTRUCTIONS %0d BAD %0d", instrs, bad);
    $finish;
  end
endmodule
"#,
            rows = passes * RP
        );
        std::fs::write(dir.path().join("tb.v"), tb).unwrap();
        let run = rtl_sim::Simulator::build(dir.path(), &["tb.v", "simd.v"]).run(dir.path(), &[]);
        let out = String::from_utf8_lossy(&run.stdout).to_string();
        assert!(run.status.success(), "the simulation: {}", String::from_utf8_lossy(&run.stderr));
        let instructions: usize = out.split("INSTRUCTIONS ").nth(1).and_then(|t| t.split_whitespace().next()).and_then(|n| n.parse().ok()).unwrap_or(0);
        assert!(instructions > 150, "{units} function units, a group of {group}: the programs ran: {out}");
        if !out.contains(" BAD 0") {
            wrong.push(format!("{units} function units, a group of {group}:\n{out}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Which vector unit a lock runs: the engine's (`logos_simd`), or the user clock's (`logos_simd_p`), its pair copies
/// as deep as its registers or the rows given.
#[derive(Clone, Copy, PartialEq, Eq)]
enum VectorUnit {
    Engine,
    UserClock,
    UserClockPairs(usize),
}

/// `cases` on `unit` with each of `function_units` function units: every register the interpreter's.
fn vector_unit_runs_as_the_interpreter(cases: &[SimdCase], function_units: &[usize], unit: VectorUnit) {
    if !common::require_tool("iverilog") {
        return;
    }
    for &units in function_units {
        let wrong = simd_mismatches(cases, units, unit);
        assert!(wrong.is_empty(), "{units} function units: {} registers differ:\n{}", wrong.len(), wrong.iter().take(20).cloned().collect::<Vec<_>>().join("\n"));
    }
}

/// `cases` under iverilog on `unit` with `function_units` function units, one after another: every scalar register and
/// each checked vector register the instruction set's interpreter leaves, the ones that differ named.
fn simd_mismatches(cases: &[SimdCase], function_units: usize, unit: VectorUnit) -> Vec<String> {
    const VREGS: usize = 16;
    const ROWS: usize = 320;
    let dir = tempfile::tempdir().unwrap();
    let mut script = String::new();
    for c in cases {
        // One case: its loads, its instructions, its reads — a line each, tagged.
        for (reg, v) in c.inputs.iter().enumerate() {
            writeln!(script, "L {reg:x} {:x}", v.len()).unwrap();
            for (row, chunk) in v.chunks(16).enumerate() {
                let mut word = String::new();
                for i in (0..16).rev() {
                    write!(word, "{:08x}", chunk.get(i).map_or(0, |x| x.to_bits())).unwrap();
                }
                writeln!(script, "W {reg:x} {row:x} {word}").unwrap();
            }
        }
        // Every scalar register starts at +0, as the interpreter's do, then takes the case's scalars.
        for s_reg in 0..8u8 {
            let value = c.scalars.get(s_reg as usize).copied().unwrap_or(0.0);
            writeln!(script, "I {:016x}", encode(&Op::Const { value, d: s_reg })).unwrap();
        }
        for op in &c.program {
            writeln!(script, "I {:016x}", encode(op)).unwrap();
        }
        writeln!(script, "E").unwrap();
    }
    std::fs::write(dir.path().join("script.txt"), script).unwrap();
    // The user clock's unit reads its registers on the rising edge, a row's word out SIMD_P_READ_LATENCY rising edges
    // after it is asked; the engine's on the falling edge.
    let user_read = format!("{}@(negedge clk); #1;", "@(posedge clk); ".repeat(SIMD_P_READ_LATENCY));
    let (source, module, read) = match unit {
        VectorUnit::Engine => (simd_verilog(), "logos_simd", "@(negedge clk); #1;"),
        VectorUnit::UserClock | VectorUnit::UserClockPairs(_) => (simd_p_verilog(), "logos_simd_p", user_read.as_str()),
    };
    let pairs = match unit {
        VectorUnit::UserClockPairs(rows) => format!(", .PAIRS({rows})"),
        _ => String::new(),
    };
    let group = match unit {
        VectorUnit::Engine => "",
        VectorUnit::UserClock | VectorUnit::UserClockPairs(_) => ".group(8'd1), .group_rows(16'd0), .rd_s_pass(8'd0), ",
    };
    // With pair copies of their own depth, each case ends with every register's pair copies held against copy A below it.
    let held = match unit {
        VectorUnit::UserClockPairs(rows) => (0..VREGS)
            .map(|b| {
                format!(
                    "        for (rw = 0; rw < {rows}; rw = rw + 1)\n          if (u.bank[{b}].cb.mem[rw] !== u.bank[{b}].ca.mem[rw] || u.bank[{b}].cc.mem[rw] !== u.bank[{b}].ca.mem[rw]) $fwrite(fo, \"P {b} %0d\\n\", rw);\n"
                )
            })
            .collect::<String>(),
        _ => String::new(),
    };
    std::fs::write(dir.path().join("simd.v"), source).unwrap();
    let tb = format!(
        r#"module tb;
  reg clk = 0, rst = 1;
  reg issue = 0, wr = 0, len_wr = 0;
  reg [63:0] instr;
  reg [3:0] wr_reg, rd_reg, rd_s;
  reg [15:0] wr_row, rd_row;
  reg [511:0] wr_data;
  reg [19:0] len_val;
  wire busy;
  wire [511:0] rd_data;
  wire [31:0] rd_s_val;
  {module} #(.ROWS({ROWS}), .VREGS({VREGS}), .FU({function_units}){pairs}) u(.clk(clk), .rst(rst), .issue(issue), .instr(instr), .busy(busy), {group}.wr(wr), .wr_reg(wr_reg),
    .wr_row(wr_row), .wr_data(wr_data), .len_wr(len_wr), .len_val(len_val), .rd_reg(rd_reg), .rd_row(rd_row),
    .rd_data(rd_data), .rd_s(rd_s), .rd_s_val(rd_s_val));
  always #5 clk = ~clk;
  integer fi, fo, c, rg, rw, i;
  reg [7:0] tag;
  reg [31:0] a, b;
  reg [511:0] word;
  reg [63:0] w64;
  initial begin
    fi = $fopen("script.txt", "r");
    fo = $fopen("out.txt", "w");
    @(posedge clk); @(posedge clk); rst <= 0;
    while (!$feof(fi)) begin
      c = $fscanf(fi, "%c", tag);
      if (tag == "L") begin
        c = $fscanf(fi, " %h %h\n", a, b);
        @(negedge clk); len_wr = 1; wr_reg = a; len_val = b; @(negedge clk); len_wr = 0;
      end else if (tag == "W") begin
        c = $fscanf(fi, " %h %h %h\n", a, b, word);
        @(negedge clk); wr = 1; wr_reg = a; wr_row = b; wr_data = word; @(negedge clk); wr = 0;
      end else if (tag == "I") begin
        c = $fscanf(fi, " %h\n", w64);
        @(negedge clk); issue = 1; instr = w64; @(negedge clk); issue = 0;
        @(negedge clk); while (busy) @(negedge clk);
      end else if (tag == "E") begin
        for (rg = 0; rg < 8; rg = rg + 1) begin
          rd_s = rg; #1; $fwrite(fo, "S %h\n", rd_s_val);
        end
        for (rg = 0; rg < {VREGS}; rg = rg + 1)
          for (rw = 0; rw < (u.len[rg] + 15) / 16; rw = rw + 1) begin
            rd_reg = rg; rd_row = rw; {read}
            $fwrite(fo, "V %0d %0d %h\n", rg, rw, rd_data);
          end
{held}        $fwrite(fo, "E\n");
      end
    end
    $fclose(fo);
    $finish;
  end
endmodule
"#
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    // The engine's unit under iverilog, which reads a register no write reached as X; the user clock's, deep in its
    // pipelines, on the shared simulator (Verilator when the box has it).
    let run = match unit {
        VectorUnit::Engine => {
            let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-o", "sim", "tb.v", "simd.v"]).output().expect("iverilog");
            assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
            Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp")
        }
        VectorUnit::UserClock | VectorUnit::UserClockPairs(_) => rtl_sim::Simulator::build(dir.path(), &["tb.v", "simd.v"]).run(dir.path(), &[]),
    };
    assert!(run.status.success(), "the simulation: {}", String::from_utf8_lossy(&run.stderr));
    let out = std::fs::read_to_string(dir.path().join("out.txt")).unwrap();
    let mut blocks = out.split("E\n");
    let mut wrong = Vec::new();
    for (ci, c) in cases.iter().enumerate() {
        let block = blocks.next().unwrap_or_else(|| panic!("case {ci}: no registers read"));
        let mut regs = Regs::new(VREGS, 8);
        for (reg, v) in c.inputs.iter().enumerate() {
            regs.v[reg] = v.clone();
        }
        regs.s[..c.scalars.len()].copy_from_slice(&c.scalars);
        vector::run(FpUnit::Soft, &c.program, &mut regs);
        // A register the RTL left with unknown bits reads as `None`, which no interpreter value equals.
        let word = |w: &str| u32::from_str_radix(w, 16).ok();
        let mut scalars: Vec<Option<u32>> = Vec::new();
        let mut vectors: Vec<Vec<Option<u32>>> = vec![Vec::new(); VREGS];
        for line in block.lines() {
            let parts: Vec<&str> = line.split(' ').collect();
            match parts[0] {
                "S" => scalars.push(word(parts[1])),
                "P" => wrong.push(format!("case {ci} register {} row {}: a pair copy is not copy A", parts[1], parts[2])),
                "V" => {
                    let reg: usize = parts[1].parse().unwrap();
                    for l in 0..16 {
                        let at = parts[3].len() - 8 * (l + 1);
                        vectors[reg].push(word(&parts[3][at..at + 8]));
                    }
                }
                _ => {}
            }
        }
        for s_reg in 0..8 {
            let want = regs.s[s_reg].to_bits();
            if scalars[s_reg] != Some(want) {
                wrong.push(format!("case {ci} scalar {s_reg}: rtl {:08x?}, interpreter {want:08x}", scalars[s_reg]));
            }
        }
        for &reg in &c.check {
            let want: Vec<Option<u32>> = regs.v[reg as usize].iter().map(|x| Some(x.to_bits())).collect();
            let got = &vectors[reg as usize][..want.len().min(vectors[reg as usize].len())];
            if got != &want[..] {
                let at = got.iter().zip(&want).position(|(a, b)| a != b).unwrap_or(got.len());
                wrong.push(format!("case {ci} vector {reg} element {at}: rtl {:08x?}, interpreter {:08x?}", got.get(at), want.get(at)));
            }
        }
    }
    wrong
}

/// The rotary, cache-row, int8-code, magnitude, convolution, read-out, f16 and greedy programs under iverilog on
/// `logos_simd`: tables whose pairs straddle rows at other lanes, rows whose scale underflows and overflows binary16,
/// codes on their ties and past ±127, maxima repeated in several lanes, NaNs, both zeros and an empty table.
#[test]
fn the_vector_unit_runs_the_rotary_cache_conv_and_greedy_programs_as_the_interpreter_does() {
    vector_unit_runs_as_the_interpreter(&rotary_cache_conv_greedy_cases(), &[16], VectorUnit::Engine);
}

/// The rotary, cache-row, int8-code, magnitude, convolution, read-out, f16 and greedy programs on the user clock's
/// vector unit, each register the interpreter's, bit for bit.
#[test]
fn the_user_clock_vector_unit_runs_the_rotary_cache_conv_and_greedy_programs_as_the_interpreter_does() {
    vector_unit_runs_as_the_interpreter(&rotary_cache_conv_greedy_cases(), &[16], VectorUnit::UserClock);
}

/// The user clock's vector unit with its pair copies (B and C, read only at a far Hadamard stage's partner rows and a
/// rotary block's rows) as deep as the longest operand a Hadamard or rotary program of the cases loads, far short of its
/// registers' 320 rows: every micro-program, rotary, cache-row, convolution and greedy program, each register the
/// interpreter's, bit for bit, the pairs at the copies' last rows among them.
#[test]
fn the_user_clock_vector_unit_pairs_rows_within_copies_as_deep_as_its_pair_operands() {
    let mut cases = micro_program_cases();
    cases.extend(rotary_cache_conv_greedy_cases());
    let pairs = cases
        .iter()
        .filter(|c| c.program.iter().any(|op| matches!(op, Op::Hadamard { .. } | Op::Rotate { .. })))
        .flat_map(|c| c.inputs.iter().map(|v| v.len().div_ceil(16)))
        .max()
        .expect("a Hadamard or rotary case");
    assert!(pairs > 1 && pairs < 320, "{pairs}");
    vector_unit_runs_as_the_interpreter(&cases, &[16], VectorUnit::UserClockPairs(pairs));
}

/// The cases [`the_vector_unit_runs_the_rotary_cache_conv_and_greedy_programs_as_the_interpreter_does`] runs.
fn rotary_cache_conv_greedy_cases() -> Vec<SimdCase> {
    let mut r = Rng(0x2075);
    let mut bits = Rng(0x2076);
    let mut f = || (((r.next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0) * 4.0;
    let mut cases = Vec::new();
    for &(n, half) in &[(2usize, 1usize), (40, 12), (64, 32), (96, 40), (256, 32), (256, 128), (16, 0)] {
        for neox in [true, false] {
            let x: Vec<f32> = (0..n).map(|_| f()).collect();
            let (cos, sin): (Vec<f32>, Vec<f32>) = (0..half).map(|i| ((i as f32 * 0.37).cos(), (i as f32 * 0.37).sin())).unzip();
            cases.push(SimdCase { program: vector::rotary(0, 1, 2, 3, neox), inputs: vec![x.clone(), cos.clone(), sin.clone()], scalars: vec![], check: vec![3] });
            cases.push(SimdCase { program: vector::rotary(0, 1, 2, 0, neox), inputs: vec![x, cos, sin], scalars: vec![], check: vec![0] });
        }
    }
    for k in [-30i32, -18, -12, -6, 0, 14, 122] {
        for n in [4usize, 37, 256] {
            let x: Vec<f32> = (0..n).map(|_| f() * 2f32.powi(k)).collect();
            cases.push(SimdCase { program: vector::cache_row(0, 1, 0), inputs: vec![x], scalars: vec![], check: vec![1] });
        }
    }
    cases.push(SimdCase { program: vector::cache_row(0, 1, 0), inputs: vec![vec![0.0; 20]], scalars: vec![], check: vec![1] });
    cases.push(SimdCase { program: vector::cache_row(0, 1, 0), inputs: vec![vec![2.5e-6, -1e-6, 0.0, 3.7e-6]], scalars: vec![], check: vec![1] });
    let ties: Vec<f32> = vec![0.5, 1.5, 2.5, -0.5, -1.5, -2.5, 126.5, 127.4, 127.5, 128.0, -127.6, -1e30, 3e38, f32::NAN, -0.0, f32::from_bits(1), 7.0];
    for s in [1.0f32, 0.5, 3.0, f32::INFINITY, f32::NAN, 0.0, 2f32.powi(-100)] {
        cases.push(SimdCase { program: vec![Op::Codes { a: 0, s: 0, d: 1 }], inputs: vec![ties.clone()], scalars: vec![s], check: vec![1] });
    }
    let magnitudes: Vec<f32> = vec![-3.0, 2.0, f32::NAN, f32::from_bits(0x8000_0005), -0.0, 2.9999998, -3.0000002];
    cases.push(SimdCase { program: vec![Op::AbsMax { a: 0, d: 0 }], inputs: vec![magnitudes], scalars: vec![], check: vec![] });
    cases.push(SimdCase { program: vec![Op::AbsMax { a: 0, d: 0 }], inputs: vec![vec![f32::NAN, f32::from_bits(3), -0.0]], scalars: vec![], check: vec![] });
    for ch in [16usize, 37, 160] {
        let mut inputs: Vec<Vec<f32>> = (0..8).map(|_| (0..ch).map(|_| f()).collect()).collect();
        inputs[3] = (0..ch).map(|_| f()).collect();
        cases.push(SimdCase { program: vector::conv(&[0, 1, 2], 3, &[4, 5, 6, 7], 8, 9), inputs, scalars: vec![], check: vec![8, 9] });
    }
    for n in [16usize, 128] {
        let inputs: Vec<Vec<f32>> = (0..3).map(|_| (0..n).map(|_| f()).collect()).collect();
        cases.push(SimdCase { program: vector::gdn_out(FpUnit::Soft, 0, 1, 2, 3, 4, n, 1e-6), inputs, scalars: vec![], check: vec![3, 4] });
    }
    let halves: Vec<f32> = [65504.0f32, 65519.99, 65520.0, -65520.0, 1e-8, 2f32.powi(-24), 2f32.powi(-25), 3.0 * 2f32.powi(-26), 1.0 + 2f32.powi(-11), 1.0 + 3.0 * 2f32.powi(-11), f32::from_bits(5), f32::NAN, -0.0, f32::INFINITY]
        .into_iter()
        .chain((0..146).map(|_| f32::from_bits(bits.next() as u32)))
        .collect();
    cases.push(SimdCase { program: vec![Op::Map { f: Func::F16, a: 0, d: 1 }, Op::ScalarMap { f: Func::F16, a: 0, d: 1 }], inputs: vec![halves], scalars: vec![65519.99], check: vec![1] });
    let mut greedy_rows: Vec<Vec<f32>> = Vec::new();
    for n in [1usize, 17, 300, 5120] {
        let mut x: Vec<f32> = (0..n).map(|_| f()).collect();
        if n > 40 {
            // The largest value in several lanes, the lowest index of it not in lane 0.
            for &i in &[37usize, 21, 40] {
                x[i] = 9.5;
            }
            x[5] = f32::NAN;
        }
        greedy_rows.push(x);
    }
    greedy_rows.push(vec![f32::NEG_INFINITY; 20]);
    greedy_rows.push(vec![f32::NAN; 20]);
    greedy_rows.push((0..40).map(|i| if i % 3 == 0 { -0.0 } else if i % 3 == 1 { 0.0 } else { -1.0 }).collect());
    greedy_rows.push((0..40).map(|i| if i == 0 { f32::NAN } else if i == 33 { 2.0 } else { 1.0 }).collect());
    for x in greedy_rows {
        cases.push(SimdCase { program: vector::greedy(0, 0), inputs: vec![x], scalars: vec![], check: vec![] });
    }
    cases
}

/// The multiply-adds under iverilog on `logos_simd`, each register the interpreter's, bit for bit
/// ([`multiply_add_cases`]).
#[test]
fn the_vector_unit_multiplies_and_adds_as_the_interpreter_does() {
    vector_unit_runs_as_the_interpreter(&multiply_add_cases(), &[16], VectorUnit::Engine);
}

/// The multiply-adds on the user clock's vector unit, each register the interpreter's, bit for bit
/// ([`multiply_add_cases`]).
#[test]
fn the_user_clock_vector_unit_multiplies_and_adds_as_the_interpreter_does() {
    vector_unit_runs_as_the_interpreter(&multiply_add_cases(), &[16], VectorUnit::UserClock);
}

/// A vector addend and a scalar one, apart from every operand, in place over the addend and over a factor, over the
/// products every pairing of special values makes — signed zeros, the least normal and the values about it, denormals,
/// the largest finite, infinities, NaNs — and random ones, at lengths from one element to ten rows and a part.
fn multiply_add_cases() -> Vec<SimdCase> {
    let special = [
        0.0f32,
        -0.0,
        f32::MIN_POSITIVE,
        -f32::MIN_POSITIVE,
        f32::from_bits(0x0080_0001),
        f32::from_bits(0x007f_ffff),
        f32::MAX,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
        1.0,
        -1.0,
        1.0000001,
        2.0f32.powi(-63),
    ];
    let mut r = Rng(0x6d61);
    let mut f = || (((r.next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0) * 2.0f32.powi((r.next() % 120) as i32 - 60);
    let (mut x, mut y, mut z) = (Vec::new(), Vec::new(), Vec::new());
    for &p in &special {
        for &q in &special {
            for &s in &special[..6] {
                x.push(p);
                y.push(q);
                z.push(s);
            }
        }
    }
    for _ in 0..400 {
        x.push(f());
        y.push(f());
        z.push(f());
    }
    let mut cases = Vec::new();
    for n in [1usize, 15, 16, 17, 37, 160, x.len()] {
        let inputs = vec![x[..n].to_vec(), y[..n].to_vec(), z[..n].to_vec()];
        cases.push(SimdCase { program: vec![Op::MulAdd { a: 0, b: 1, c: 2, d: 3 }], inputs: inputs.clone(), scalars: vec![], check: vec![3] });
        cases.push(SimdCase { program: vec![Op::MulAdd { a: 0, b: 1, c: 2, d: 2 }], inputs: inputs.clone(), scalars: vec![], check: vec![2] });
        cases.push(SimdCase { program: vec![Op::MulAdd { a: 0, b: 1, c: 2, d: 0 }], inputs: inputs.clone(), scalars: vec![], check: vec![0] });
        for &k in &[0.0f32, -0.0, f32::NAN, f32::INFINITY, -1.5, f32::from_bits(0x8080_0000)] {
            cases.push(SimdCase { program: vec![Op::MulAddScalar { a: 0, b: 1, s: 0, d: 3 }], inputs: inputs.clone(), scalars: vec![k], check: vec![3] });
        }
    }
    cases
}

/// The exact power-of-two scale under iverilog on `logos_simd`: every exponent at every shift the rotations back take,
/// the band just below the normals that a product would round up into them, the zeros, the infinities and a NaN.
#[test]
fn the_vector_unit_scales_by_a_power_of_two_as_the_interpreter_does() {
    vector_unit_runs_as_the_interpreter(&power_of_two_scale_cases(), &[16], VectorUnit::Engine);
}

/// The exact power-of-two scale on the user clock's vector unit, over every exponent at every shift the rotations back
/// take, each register the interpreter's, bit for bit.
#[test]
fn the_user_clock_vector_unit_scales_by_a_power_of_two_as_the_interpreter_does() {
    vector_unit_runs_as_the_interpreter(&power_of_two_scale_cases(), &[16], VectorUnit::UserClock);
}

/// The cases [`the_vector_unit_scales_by_a_power_of_two_as_the_interpreter_does`] runs.
fn power_of_two_scale_cases() -> Vec<SimdCase> {
    let mut x: Vec<f32> = Vec::new();
    for e in 0u32..255 {
        for m in [0u32, 1, 0x40_0000, 0x7f_ffff] {
            x.push(f32::from_bits(e << 23 | m));
            x.push(f32::from_bits(1 << 31 | e << 23 | m));
        }
    }
    x.extend([f32::INFINITY, f32::NEG_INFINITY, f32::NAN, -0.0]);
    [-24i8, -9, -8, -7, -1, 0, 3]
        .into_iter()
        .map(|n| SimdCase { program: vec![Op::Scale { a: 0, n, d: 1 }], inputs: vec![x.clone()], scalars: vec![], check: vec![1] })
        .collect()
}

/// `logos_code8` under iverilog over every vector operand times inverse scales of every class: each code
/// `int8_codes`'s, bit for bit as the f32 the register holds.
#[test]
fn the_int8_code_unit_is_int8_codes() {
    if !common::require_tool("iverilog") {
        return;
    }
    let xs = vector_operands();
    let scales: [u32; 7] = [0x3f80_0000, 0x4300_0000, 0x3c01_0204, 0x0080_0000, 0x7f80_0000, 0x7fc0_0000, 0x0000_0000];
    let pairs: Vec<(u32, u32)> = scales.iter().flat_map(|&s| xs.iter().map(move |&x| (x, s))).collect();
    let dir = tempfile::tempdir().unwrap();
    let hex: String = pairs.iter().map(|(x, s)| format!("{x:08x} {s:08x}\n")).collect();
    std::fs::write(dir.path().join("vectors.txt"), hex).unwrap();
    std::fs::write(dir.path().join("simd.v"), simd_verilog()).unwrap();
    let n = pairs.len();
    let tb = format!(
        r#"module tb;
  reg [31:0] x, s;
  wire [31:0] y;
  logos_code8 u(.x(x), .s(s), .y(y));
  integer i, f, g, c;
  initial begin
    f = $fopen("vectors.txt", "r");
    g = $fopen("out.txt", "w");
    for (i = 0; i < {n}; i = i + 1) begin
      c = $fscanf(f, "%h %h\n", x, s); #1;
      $fwrite(g, "%h\n", y);
    end
    $fclose(g);
    $finish;
  end
endmodule
"#
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-o", "sim", "tb.v", "simd.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    let out = std::fs::read_to_string(dir.path().join("out.txt")).unwrap();
    let got: Vec<&str> = out.lines().collect();
    assert_eq!(got.len(), n, "every pair simulated");
    let wrong: Vec<String> = pairs
        .iter()
        .zip(&got)
        .filter_map(|(&(x, s), field)| {
            let g = u32::from_str_radix(field, 16).unwrap_or_else(|_| panic!("code8({x:08x}, {s:08x}): the rtl gave `{field}`"));
            let want = (int8_codes(FpUnit::Soft, &[f32::from_bits(x)], f32::from_bits(s))[0] as f32).to_bits();
            (g != want).then(|| format!("code8({x:08x}, {s:08x}): rtl {g:08x}, numerics {want:08x}"))
        })
        .collect();
    assert!(wrong.is_empty(), "{} of {n} codes differ:\n{}", wrong.len(), wrong.iter().take(20).cloned().collect::<Vec<_>>().join("\n"));
}

// ── The matrix-vector step ─────────────────────────────────────────────────────────────────────────

/// Two rows of every format the array reads, 1,024 elements each, against token-rows in both word counts, through
/// `logos_matvec_<format>` under iverilog a block at a time: every running f32 between blocks is the per-block dot's,
/// and each row's last is `numerics::dot`'s, bit for bit.
#[test]
fn the_matvec_step_adds_each_block_as_dot_does() {
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0x3a7f);
    let mut wrong = Vec::new();
    for d in DECODERS {
        let (len, bb) = (d.ty.block_len(), d.ty.block_bytes());
        let blocks = 1024 / len;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("matvec.v"), matvec_verilog()).unwrap();
        let mut hex = String::new();
        let mut want: Vec<(u32, u32)> = Vec::new();
        let mut steps = 0usize;
        for words in [Words::One, Words::Two] {
            for _ in 0..3 {
                let raw: Vec<Vec<u8>> = (0..2).map(|_| small_scaled_blocks(&mut r, d.ty, blocks)).collect();
                let rows: Vec<CodedRow> = raw.iter().map(|b| CodedRow::decode(d.ty, b).unwrap().unwrap()).collect();
                let x: Vec<f32> = (0..1024).map(|_| ((r.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * 2f32.powi((r.next() % 20) as i32 - 10)).collect();
                let act = quantize_blocks(&x, words, len);
                let (mut acc1, mut acc2) = (0f32, 0f32);
                for b in 0..blocks {
                    for row in &raw {
                        for byte in row[b * bb..(b + 1) * bb].iter().rev() {
                            write!(hex, "{byte:02x}").unwrap();
                        }
                        hex.push('\n');
                    }
                    for i in (0..len).rev() {
                        write!(hex, "{:04x}", act.codes[b * len + i] as i16 as u16).unwrap();
                    }
                    hex.push('\n');
                    let s = act.scales[b];
                    writeln!(hex, "{}\n{:02x}\n{:03x}\n{:08x}\n{:08x}", (words == Words::Two) as u8, s.m, s.e as i16 as u16 & 0xfff, acc1.to_bits(), acc2.to_bits()).unwrap();
                    // The running value a block at a time: the lane's step, the per-block dot's.
                    let step = |k: usize, acc: f32| {
                        let seg = &rows[k].segments[b];
                        lane(&deliver(&units(d.ty, &raw[k][b * bb..(b + 1) * bb]), (seg.a, seg.b), d.offset, &act, words, b), acc)
                    };
                    acc1 = step(0, acc1);
                    acc2 = step(1, acc2);
                    want.push((acc1.to_bits(), acc2.to_bits()));
                    steps += 1;
                }
                let (f1, f2) = (dot(&rows[0], &act), dot(&rows[1], &act));
                assert_eq!((acc1.to_bits(), acc2.to_bits()), (f1.to_bits(), f2.to_bits()), "{}: the blocks' chain is the dot", d.ty.name());
            }
        }
        std::fs::write(dir.path().join("vectors.hex"), hex).unwrap();
        let fields = 8;
        let tb = format!(
            r#"module tb;
  reg [{aw}:0] mem [0:{last}];
  reg [{bw}:0] blk1, blk2;
  reg [{aw}:0] act;
  reg two;
  reg [7:0] m;
  reg [11:0] e;
  reg [31:0] acc1, acc2;
  wire [31:0] out1, out2;
  logos_matvec_{name} u(.blk1(blk1), .blk2(blk2), .act(act), .two(two), .m(m), .e($signed(e)), .acc1(acc1), .acc2(acc2),
                      .out1(out1), .out2(out2));
  integer i, f;
  initial begin
    $readmemh("vectors.hex", mem);
    f = $fopen("out.hex", "w");
    for (i = 0; i < {steps}; i = i + 1) begin
      blk1 = mem[{fields}*i]; blk2 = mem[{fields}*i+1]; act = mem[{fields}*i+2]; two = mem[{fields}*i+3]; m = mem[{fields}*i+4];
      e = mem[{fields}*i+5]; acc1 = mem[{fields}*i+6]; acc2 = mem[{fields}*i+7]; #1;
      $fwrite(f, "%h %h\n", out1, out2);
    end
    $fclose(f);
    $finish;
  end
endmodule
"#,
            aw = 16 * len - 1,
            bw = 8 * bb - 1,
            last = fields * steps - 1,
            name = d.ty.name().to_lowercase(),
        );
        std::fs::write(dir.path().join("tb.v"), tb).unwrap();
        let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-o", "sim", "tb.v", "matvec.v"]).output().expect("iverilog");
        assert!(build.status.success(), "iverilog ({}): {}", d.ty.name(), String::from_utf8_lossy(&build.stderr));
        let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
        assert!(run.status.success(), "vvp ({}): {}", d.ty.name(), String::from_utf8_lossy(&run.stderr));
        let out = std::fs::read_to_string(dir.path().join("out.hex")).unwrap();
        let got: Vec<(u32, u32)> = out
            .lines()
            .map(|l| {
                let (a, b) = l.split_once(' ').unwrap();
                (u32::from_str_radix(a, 16).unwrap(), u32::from_str_radix(b, 16).unwrap())
            })
            .collect();
        assert_eq!(got.len(), steps, "{}: every block simulated", d.ty.name());
        for (k, (g, w)) in got.iter().zip(&want).enumerate() {
            if g != w {
                wrong.push(format!("{} block step {k}: rtl {:08x} {:08x}, dot {:08x} {:08x}", d.ty.name(), g.0, g.1, w.0, w.1));
            }
        }
    }
    assert!(wrong.is_empty(), "{} steps differ:\n{}", wrong.len(), wrong.iter().take(16).cloned().collect::<Vec<_>>().join("\n"));
}

/// One column of the array, `logos_matvec_engine_<format>`, under iverilog: a four-row matrix of 512 elements a row
/// streamed two rows a beat against three token-rows in both word counts, every emitted pair of rows each token-row's
/// `numerics::dot`, bit for bit, for the K-quants, the legacy formats and IQ4.
#[test]
fn the_matvec_engine_multiplies_a_matrix_as_dot_does() {
    if !common::require_tool("iverilog") {
        return;
    }
    const ROWS: usize = 4;
    const ELEMS: usize = 512;
    const TOKENS: usize = 3;
    let mut r = Rng(0x3a80);
    let mut wrong = Vec::new();
    for d in DECODERS.iter().filter(|d| matches!(d.ty, GgmlType::Q4K | GgmlType::Q6K | GgmlType::Q8_0 | GgmlType::Q4_0 | GgmlType::Iq4Xs)) {
        let (len, bb) = (d.ty.block_len(), d.ty.block_bytes());
        let blocks = ELEMS / len;
        for words in [Words::One, Words::Two] {
            let raw: Vec<Vec<u8>> = (0..ROWS).map(|_| small_scaled_blocks(&mut r, d.ty, blocks)).collect();
            let rows: Vec<CodedRow> = raw.iter().map(|b| CodedRow::decode(d.ty, b).unwrap().unwrap()).collect();
            let acts: Vec<_> = (0..TOKENS)
                .map(|_| {
                    let x: Vec<f32> = (0..ELEMS).map(|_| ((r.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * 2f32.powi((r.next() % 16) as i32 - 8)).collect();
                    quantize_blocks(&x, words, len)
                })
                .collect();
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("matvec.v"), matvec_verilog()).unwrap();
            let mut act_hex = String::new();
            for (t, act) in acts.iter().enumerate() {
                for b in 0..blocks {
                    for i in (0..len).rev() {
                        write!(act_hex, "{:04x}", act.codes[b * len + i] as i16 as u16).unwrap();
                    }
                    writeln!(act_hex, "\n{t:02x}\n{b:04x}\n{:02x}\n{:03x}", act.scales[b].m, act.scales[b].e as i16 as u16 & 0xfff).unwrap();
                }
            }
            let mut w_hex = String::new();
            for p in 0..ROWS / 2 {
                for b in 0..blocks {
                    for k in [2 * p, 2 * p + 1] {
                        for byte in raw[k][b * bb..(b + 1) * bb].iter().rev() {
                            write!(w_hex, "{byte:02x}").unwrap();
                        }
                        w_hex.push('\n');
                    }
                }
            }
            std::fs::write(dir.path().join("acts.hex"), act_hex).unwrap();
            std::fs::write(dir.path().join("weights.hex"), w_hex).unwrap();
            let tb = format!(
                r#"module tb;
  reg clk = 0, rst = 1;
  always #5 clk = ~clk;
  reg start = 0, act_wr = 0, w_valid = 0;
  reg [7:0] act_t, act_m;
  reg [15:0] act_b;
  reg [11:0] act_e;
  reg [{aw}:0] act_codes;
  reg [{bw}:0] w_blk1, w_blk2;
  wire busy, w_ready, o_valid;
  wire [15:0] o_pair;
  wire [7:0] o_t;
  wire [31:0] o_val1, o_val2;
  logos_matvec_engine_{name} #(.T_MAX(4), .NB_MAX(16)) u(.clk(clk), .rst(rst), .start(start), .pairs(16'd{pairs}), .blocks(16'd{blocks}),
    .tokens(8'd{tokens}), .two(1'b{two}), .busy(busy), .act_wr(act_wr), .act_t(act_t), .act_b(act_b), .act_codes(act_codes),
    .act_m(act_m), .act_e(act_e), .w_valid(w_valid), .w_ready(w_ready), .w_blk1(w_blk1), .w_blk2(w_blk2), .o_valid(o_valid),
    .o_pair(o_pair), .o_t(o_t), .o_val1(o_val1), .o_val2(o_val2));
  reg [{aw}:0] am [0:{alast}];
  reg [{bw}:0] wm [0:{wlast}];
  integer i, f;
  always @(negedge clk) if (o_valid) $fwrite(f, "%h %h %h %h\n", o_pair, o_t, o_val1, o_val2);
  initial begin
    $readmemh("acts.hex", am);
    $readmemh("weights.hex", wm);
    f = $fopen("out.txt", "w");
    @(negedge clk); rst = 0;
    for (i = 0; i < {acts}; i = i + 1) begin
      @(negedge clk); act_wr = 1; act_codes = am[5*i]; act_t = am[5*i+1]; act_b = am[5*i+2]; act_m = am[5*i+3]; act_e = am[5*i+4];
    end
    @(negedge clk); act_wr = 0; start = 1;
    @(negedge clk); start = 0;
    for (i = 0; i < {beats}; i = i + 1) begin
      w_blk1 = wm[2*i]; w_blk2 = wm[2*i+1]; w_valid = 1;
      while (!w_ready) @(negedge clk);
      @(negedge clk); w_valid = 0;
    end
    while (busy) @(negedge clk);
    @(negedge clk); @(negedge clk);
    $fclose(f);
    $finish;
  end
endmodule
"#,
                aw = 16 * len - 1,
                bw = 8 * bb - 1,
                name = d.ty.name().to_lowercase(),
                pairs = ROWS / 2,
                tokens = TOKENS,
                two = (words == Words::Two) as u8,
                alast = 5 * TOKENS * blocks - 1,
                wlast = ROWS * blocks - 1,
                acts = TOKENS * blocks,
                beats = ROWS / 2 * blocks,
            );
            std::fs::write(dir.path().join("tb.v"), tb).unwrap();
            let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-o", "sim", "tb.v", "matvec.v"]).output().expect("iverilog");
            assert!(build.status.success(), "iverilog ({}): {}", d.ty.name(), String::from_utf8_lossy(&build.stderr));
            let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
            assert!(run.status.success(), "vvp ({}): {}", d.ty.name(), String::from_utf8_lossy(&run.stderr));
            let out = std::fs::read_to_string(dir.path().join("out.txt")).unwrap();
            let lines: Vec<&str> = out.lines().collect();
            assert_eq!(lines.len(), ROWS / 2 * TOKENS, "{} {words:?}: every token-row's every row pair emitted", d.ty.name());
            for l in lines {
                let f: Vec<u32> = l.split(' ').map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
                let (p, t) = (f[0] as usize, f[1] as usize);
                for (k, got) in [(2 * p, f[2]), (2 * p + 1, f[3])] {
                    let want = dot(&rows[k], &acts[t]).to_bits();
                    if got != want {
                        wrong.push(format!("{} {words:?} row {k} token-row {t}: rtl {got:08x}, dot {want:08x}", d.ty.name()));
                    }
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{} dots differ:\n{}", wrong.len(), wrong.iter().take(16).cloned().collect::<Vec<_>>().join("\n"));
}

/// `logos_bram_w<W>` under iverilog against a model of the memory, at a width of one 32-bit word and one of eleven
/// bytes: a seeded run of reads and byte-masked writes, a quarter of the writes to the address read that clock, every
/// clock's word the one the model gives the address of the clock before — the word as it stood before that edge's
/// write.
#[test]
fn the_registered_memory_reads_a_clock_after_its_address() {
    use logicaffeine_compile::spatial::decode_engine::rtl::ram::{bram_name, bram_verilog};
    if !common::require_tool("iverilog") {
        return;
    }
    const DEPTH: usize = 16;
    const OPS: usize = 4000;
    for w in [32usize, 88] {
        let mut r = Rng(0xb4a3 ^ w as u64);
        let full = (1u128 << w) - 1;
        let lanes = w / 8;
        // Each clock's (read address, write enable, write address, byte mask, data); the first DEPTH clocks write every
        // word whole.
        let ops: Vec<(usize, bool, usize, u32, u128)> = (0..OPS)
            .map(|i| {
                let ra = (r.next() % DEPTH as u64) as usize;
                let data = ((r.next() as u128) << 64 | r.next() as u128) & full;
                if i < DEPTH {
                    (ra, true, i, (1 << lanes) - 1, data)
                } else {
                    let we = r.next() % 2 == 0;
                    let wa = if r.next() % 4 == 0 { ra } else { (r.next() % DEPTH as u64) as usize };
                    (ra, we, wa, (r.next() % (1 << lanes)) as u32, data)
                }
            })
            .collect();
        let mut hex = String::new();
        for &(ra, we, wa, wm, wd) in &ops {
            writeln!(hex, "{ra:x}\n{:x}\n{wa:x}\n{wm:x}\n{wd:x}", we as u8).unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ops.hex"), hex).unwrap();
        std::fs::write(dir.path().join("ram.v"), bram_verilog(&[w])).unwrap();
        let tb = format!(
            r#"module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  reg [3:0] ra = 0, wa = 0;
  reg we = 0;
  reg [{top}:0] wd = 0;
  reg [{mtop}:0] wm = 0;
  wire [{top}:0] rd;
  {name} #(.D({DEPTH}), .A(4)) m(.clk(clk), .ra(ra), .rd(rd), .we(we), .wa(wa), .wd(wd), .wm(wm));
  reg [{top}:0] ops [0:{last}];
  integer i, f;
  initial begin
    $readmemh("ops.hex", ops);
    f = $fopen("out.hex", "w");
    for (i = 0; i <= {OPS}; i = i + 1) begin
      @(negedge clk);
      if (i > 0) $fwrite(f, "%h\n", rd);
      if (i < {OPS}) begin ra = ops[5*i]; we = ops[5*i+1]; wa = ops[5*i+2]; wm = ops[5*i+3]; wd = ops[5*i+4]; end
      else we = 0;
    end
    $fclose(f);
    $finish;
  end
endmodule
"#,
            top = w - 1,
            mtop = lanes - 1,
            name = bram_name(w),
            last = 5 * OPS - 1,
        );
        std::fs::write(dir.path().join("tb.v"), tb).unwrap();
        let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "ram.v"]).output().expect("iverilog");
        assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
        let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
        assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
        let out = std::fs::read_to_string(dir.path().join("out.hex")).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), OPS, "a word for every clock");
        let mut mem: Vec<Option<u128>> = vec![None; DEPTH];
        let (mut wrong, mut checked, mut same) = (Vec::new(), 0, 0);
        for (i, &(ra, we, wa, wm, wd)) in ops.iter().enumerate() {
            if let Some(want) = mem[ra] {
                checked += 1;
                same += (we && wa == ra) as usize;
                let got = u128::from_str_radix(lines[i], 16).unwrap_or_else(|_| panic!("width {w} clock {i}: {}", lines[i]));
                if got != want {
                    wrong.push(format!("width {w} clock {i} address {ra}: rtl {got:x}, model {want:x}"));
                }
            }
            if we {
                let mut word = mem[wa].unwrap_or(0);
                for b in 0..lanes {
                    if wm >> b & 1 == 1 {
                        word = word & !(0xffu128 << (8 * b)) | wd & (0xffu128 << (8 * b));
                    }
                }
                mem[wa] = if mem[wa].is_some() || wm == (1 << lanes) - 1 { Some(word) } else { None };
            }
        }
        assert!(checked > OPS - 2 * DEPTH && same > OPS / 20, "width {w}: {checked} clocks checked, {same} of them a read of the word written");
        assert!(wrong.is_empty(), "{} words differ:\n{}", wrong.len(), wrong.iter().take(16).cloned().collect::<Vec<_>>().join("\n"));
    }
}

/// The depths and widths `logos_delay` is held at: every depth a chain or a ring takes up to past a power of two (the
/// ring's words), the pipelined units' latencies among them, and widths of one bit to the column core's lane fields.
const DELAY_DEPTHS: [usize; 15] = [0, 1, 2, 3, 4, 5, 7, 8, 9, 16, 17, 42, 43, 64, 65];
const DELAY_WIDTHS: [usize; 5] = [1, 13, 40, 512, 1536];

/// Seeded words of `w` bits, as hex.
fn seeded_words(r: &mut Rng, w: usize, n: usize) -> Vec<String> {
    (0..n)
        .map(|_| {
            let digits = w.div_ceil(4);
            let mut hex: String = (0..digits.div_ceil(16)).map(|_| format!("{:016x}", r.next())).collect();
            hex = hex[hex.len() - digits..].to_string();
            let top = w - 4 * (digits - 1);
            let lead = u8::from_str_radix(&hex[..1], 16).unwrap() & ((1u8 << top) - 1);
            format!("{lead:x}{}", &hex[1..])
        })
        .collect()
}

/// `logos_delay` under iverilog at every depth and width of the lists, each instance fed a seeded word a clock: from its
/// depth's clock on, each clock's word out is the word that went in that many clocks before (at depth 0, the word going
/// in), and every word out is defined.
#[test]
fn the_delay_gives_back_each_word_its_depth_in_clocks_later() {
    use logicaffeine_compile::spatial::decode_engine::rtl::ram::delay_verilog;
    if !common::require_tool("iverilog") {
        return;
    }
    const CLOCKS: usize = 300;
    let dir = tempfile::tempdir().unwrap();
    let mut r = Rng(0xde1a);
    let cases: Vec<(usize, usize)> = DELAY_WIDTHS.iter().flat_map(|&w| DELAY_DEPTHS.iter().map(move |&d| (w, d))).collect();
    let mut words = Vec::new();
    let (mut decls, mut drive, mut sample) = (String::new(), String::new(), String::new());
    for (c, &(w, d)) in cases.iter().enumerate() {
        let ws = seeded_words(&mut r, w, CLOCKS);
        std::fs::write(dir.path().join(format!("in{c}.hex")), ws.join("\n") + "\n").unwrap();
        words.push(ws);
        writeln!(decls, "  reg [{t}:0] d{c} = 0;\n  wire [{t}:0] q{c};\n  reg [{t}:0] in{c} [0:{last}];", t = w - 1, last = CLOCKS - 1).unwrap();
        writeln!(decls, "  logos_delay #(.W({w}), .D({d})) u{c}(.clk(clk), .d(d{c}), .q(q{c}));\n  integer f{c};").unwrap();
        writeln!(drive, "      d{c} = in{c}[i];").unwrap();
        writeln!(sample, "      $fwrite(f{c}, \"%h\\n\", q{c});").unwrap();
    }
    let opens: String = (0..cases.len()).map(|c| format!("    $readmemh(\"in{c}.hex\", in{c});\n    f{c} = $fopen(\"out{c}.hex\", \"w\");\n")).collect();
    let closes: String = (0..cases.len()).map(|c| format!("    $fclose(f{c});\n")).collect();
    // A word is set at a falling edge and read at each later falling edge, so the word out at falling edge k is the
    // one set at k - depth.
    let tb = format!(
        r#"module tb;
  reg clk = 0;
  always #5 clk = ~clk;
{decls}  integer i;
  initial begin
{opens}    for (i = 0; i < {CLOCKS}; i = i + 1) begin
      @(negedge clk);
      #1;
{drive}      #1;
{sample}    end
{closes}    $finish;
  end
endmodule
"#
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    std::fs::write(dir.path().join("delay.v"), delay_verilog()).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "delay.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    let mut wrong = Vec::new();
    for (c, &(w, d)) in cases.iter().enumerate() {
        let out = std::fs::read_to_string(dir.path().join(format!("out{c}.hex"))).unwrap();
        let got: Vec<&str> = out.lines().collect();
        assert_eq!(got.len(), CLOCKS, "width {w} depth {d}: a word out every clock");
        let digits = w.div_ceil(4);
        for k in d..CLOCKS {
            let want = &words[c][k - d];
            let want = format!("{want:0>digits$}");
            if got[k] != want {
                wrong.push(format!("width {w} depth {d} clock {k}: out {}, in at clock {} {want}", got[k], k - d));
            }
        }
    }
    assert!(wrong.is_empty(), "{} words differ:\n{}", wrong.len(), wrong.iter().take(20).cloned().collect::<Vec<_>>().join("\n"));
}

/// The flip-flop bits of `top` elaborated by yosys at `params`, and its memories as (width, words, every read clocked,
/// the block RAM its `ramstyle` asks for), each register a read is taken into merged into the memory's read port, as a
/// block RAM holds it.
fn elaborated_storage(verilog: &str, top: &str, params: &[(&str, usize)]) -> (usize, Vec<(usize, usize, bool, String)>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("d.v"), verilog).unwrap();
    let set: String = params.iter().map(|(k, v)| format!(" -set {k} {v}")).collect();
    let script = format!(
        "read_verilog -sv d.v; chparam{set} {top}; hierarchy -top {top}; proc; flatten; opt_clean; memory -nomap; opt_clean; write_json s.json"
    );
    let o = Command::new("yosys").current_dir(dir.path()).args(["-q", "-p", &script]).output().expect("yosys");
    assert!(o.status.success(), "yosys on {top} {params:?}: {}", String::from_utf8_lossy(&o.stderr));
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("s.json")).unwrap()).unwrap();
    let number = |v: &serde_json::Value| match v {
        serde_json::Value::String(s) => usize::from_str_radix(s, 2).unwrap_or_else(|_| panic!("a parameter {s}")),
        v => v.as_u64().unwrap_or_else(|| panic!("a parameter {v}")) as usize,
    };
    let modules = json["modules"].as_object().unwrap();
    let (_, module) = modules
        .iter()
        .find(|(_, m)| m["attributes"].get("top").is_some_and(|t| number(t) == 1))
        .unwrap_or_else(|| panic!("no top among {:?}", modules.keys().collect::<Vec<_>>()));
    let (mut flops, mut memories) = (0, Vec::new());
    for cell in module["cells"].as_object().unwrap().values() {
        let ty = cell["type"].as_str().unwrap();
        let p = &cell["parameters"];
        if ty.starts_with('$') && (ty.contains("dff") || ty.contains("dlatch")) {
            flops += number(&p["WIDTH"]);
        } else if ty == "$mem_v2" {
            let ports = number(&p["RD_PORTS"]);
            let clocked = number(&p["RD_CLK_ENABLE"]);
            let style = cell["attributes"].get("ramstyle").and_then(|s| s.as_str()).unwrap_or("").to_string();
            memories.push((number(&p["WIDTH"]), number(&p["SIZE"]), ports > 0 && clocked == (1 << ports) - 1, style));
        }
    }
    (flops, memories)
}

/// `logos_delay` keeps one or two clocks of words in registers and three or more in a memory read on the clock, the
/// form Quartus builds in block RAM: a ring of the next power of two of its depth in words, each read into its port's
/// register, so the only flip-flops left are the ring's write address. A ring of an MLAB's depth (32 words) or fewer
/// asks for MLABs, whatever its width; a deeper one asks for M20K blocks from a block's widest word (40 bits) and for
/// MLABs below it. A chain of D registers W wide is D·W flip-flops, which Quartus Prime Pro left in registers in every
/// build (hybrid8: a column core's lane fields, 1,536 bits five clocks, in 7,680 flip-flops each); a ring of eight
/// words in M20K blocks holds each 40 bits of its width in a block of 20,480 (hybrid9c2: a column core's lane fields,
/// 1,184 bits eight clocks, in 30 blocks each).
#[test]
fn a_delay_of_three_clocks_or_more_keeps_its_words_in_block_ram() {
    use logicaffeine_compile::spatial::decode_engine::rtl::ram::{delay_verilog, DELAY_M20K_LEAST, DELAY_RING_LEAST, MLAB_WORDS};
    if !common::require_tool("yosys") {
        return;
    }
    assert_eq!((DELAY_RING_LEAST, DELAY_M20K_LEAST, MLAB_WORDS), (3, 40, 32));
    let mut wrong = Vec::new();
    for &w in &DELAY_WIDTHS {
        for &d in &DELAY_DEPTHS {
            let (flops, memories) = elaborated_storage(delay_verilog(), "logos_delay", &[("W", w), ("D", d)]);
            if d < DELAY_RING_LEAST {
                if !memories.is_empty() || flops != w * d {
                    wrong.push(format!("width {w} depth {d}: {flops} flip-flops and memories {memories:?}, want {} flip-flops", w * d));
                }
            } else {
                let words = d.next_power_of_two();
                let address = words.trailing_zeros() as usize;
                let style = if words > MLAB_WORDS && w >= DELAY_M20K_LEAST { "M20K" } else { "MLAB" };
                if memories != [(w, words, true, style.to_string())] || flops > address {
                    wrong.push(format!(
                        "width {w} depth {d}: {flops} flip-flops and memories {memories:?}, want one {style} memory of {words} words {w} wide read on the clock and at most {address} flip-flops"
                    ));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Every memory `rtl::ram` builds asks for the block RAM its depth fills: MLABs to an MLAB's depth (32 words), M20K
/// blocks beyond, whatever its width and whichever read it has (on the falling edge, on the rising
/// edge, held, at the engine's read mark), each stack of a deep memory in M20K blocks. Quartus Prime Pro builds a memory
/// that asks for nothing in an M20K block for every forty bits of its width at any depth (hybrid9c2: a vector unit's pair
/// copies of sixteen words, 13 blocks each for 8 Kbit).
#[test]
fn every_memory_rtl_ram_builds_asks_for_the_block_ram_its_depth_fills() {
    use logicaffeine_compile::spatial::decode_engine::rtl::ram::{
        bram_name, bram_re_name, bram_re_verilog, bram_verilog, ram_ce_name, ram_ce_verilog, ram_name, ram_verilog, MLAB_WORDS,
    };
    if !common::require_tool("yosys") {
        return;
    }
    assert_eq!(MLAB_WORDS, 32);
    let style = |words: usize| if words <= MLAB_WORDS { "MLAB" } else { "M20K" };
    let widths = [8usize, 40, 512];
    let kinds: [(&str, fn(&[usize]) -> String, fn(usize) -> String); 4] =
        [("falling", ram_verilog, ram_name), ("rising", bram_verilog, bram_name), ("held", bram_re_verilog, bram_re_name), ("enabled", ram_ce_verilog, ram_ce_name)];
    let mut wrong = Vec::new();
    for (what, verilog, name) in kinds {
        let text = verilog(&widths);
        for &w in &widths {
            for words in [2usize, 3, 16, 32, 33, 64, 512, 1088] {
                let a = words.next_power_of_two().trailing_zeros().max(1) as usize;
                let (_, memories) = elaborated_storage(&text, &name(w), &[("D", words), ("A", a)]);
                let got: Vec<(usize, usize, String)> = memories.iter().map(|(mw, mwords, _, s)| (*mw, *mwords, s.clone())).collect();
                let stacked = w >= 40 && words > 512 && matches!(what, "rising" | "held");
                let want: Vec<(usize, usize, String)> = if stacked {
                    vec![(w, 1024, "M20K".to_string()), (w, 512, "M20K".to_string())]
                } else {
                    vec![(w, words, style(words).to_string())]
                };
                let (mut got_sorted, mut want_sorted) = (got.clone(), want.clone());
                got_sorted.sort();
                want_sorted.sort();
                if got_sorted != want_sorted {
                    wrong.push(format!("{what} {w} bits, {words} words: memories {got:?}, want {want:?}"));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// `logos_ram_ce_w<W>`, the memory of an engine stepping in the user clock, under iverilog with its marks from the
/// card's divider (`ce` the user cycle that ends with the engine clock's rising edge, `cr` the cycle after its falling
/// edge), at a width of one 32-bit word and one of eleven bytes and at six and eight user cycles an engine cycle: a
/// seeded run of engine cycles, each holding a read address and a byte-masked write from its step to the next, a
/// quarter of the writes to the word read. The word read is the one the model gives the address as it stood before the
/// engine cycle's write, from the read mark to the step, as a memory read at the falling edge of the engine clock and
/// written at its rising edge gives it.
#[test]
fn the_enabled_memory_reads_at_its_read_mark_and_writes_at_its_step() {
    use logicaffeine_compile::spatial::decode_engine::ofs::COMPUTE_CLOCK_VERILOG;
    use logicaffeine_compile::spatial::decode_engine::rtl::ram::{ram_ce_name, ram_ce_verilog};
    if !common::require_tool("iverilog") {
        return;
    }
    const DEPTH: usize = 16;
    const OPS: usize = 1500;
    for (w, divide) in [(32usize, 6u32), (88, 8), (32, 8), (88, 6)] {
        let mut r = Rng(0x6e5c ^ (w as u64) << 8 ^ divide as u64);
        let full = (1u128 << w) - 1;
        let lanes = w / 8;
        // Each engine cycle's (read address, write enable, write address, byte mask, data); the first DEPTH write every
        // word whole.
        let ops: Vec<(usize, bool, usize, u32, u128)> = (0..OPS)
            .map(|i| {
                let ra = (r.next() % DEPTH as u64) as usize;
                let data = ((r.next() as u128) << 64 | r.next() as u128) & full;
                if i < DEPTH {
                    (ra, true, i, (1 << lanes) - 1, data)
                } else {
                    let we = r.next() % 2 == 0;
                    let wa = if r.next() % 4 == 0 { ra } else { (r.next() % DEPTH as u64) as usize };
                    (ra, we, wa, (r.next() % (1 << lanes)) as u32, data)
                }
            })
            .collect();
        let mut hex = String::new();
        for &(ra, we, wa, wm, wd) in &ops {
            writeln!(hex, "{ra:x}\n{:x}\n{wa:x}\n{wm:x}\n{wd:x}", we as u8).unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ops.hex"), hex).unwrap();
        std::fs::write(dir.path().join("ram.v"), ram_ce_verilog(&[w])).unwrap();
        std::fs::write(dir.path().join("clock.v"), COMPUTE_CLOCK_VERILOG).unwrap();
        let tb = format!(
            r#"module tb;
  reg fclk = 0, rst_n = 0, cr = 0;
  always #5 fclk = ~fclk;
  wire cclk, crst_n, fedge, fmid;
  logos_compute_clock #(.DIVIDE({divide})) cc(.clk(fclk), .rst_n(rst_n), .cclk(cclk), .cclk_rst_n(crst_n), .fedge(fedge), .fmid(fmid));
  always @(posedge fclk) cr <= fmid;
  reg [3:0] ra = 0, wa = 0;
  reg we = 0;
  reg [{top}:0] wd = 0, mid = 0;
  reg [{mtop}:0] wm = 0;
  wire [{top}:0] rd;
  {name} #(.D({DEPTH}), .A(4)) m(.clk(fclk), .ce(fedge), .cr(cr), .ra(ra), .rd(rd), .we(we), .wa(wa), .wd(wd), .wm(wm));
  reg [{top}:0] ops [0:{last}];
  integer i, f;
  initial begin
    $readmemh("ops.hex", ops);
    f = $fopen("out.hex", "w");
    #23 rst_n = 1;
    @(negedge fclk); while (!fedge) @(negedge fclk);
    @(posedge fclk);
    for (i = 0; i < {OPS}; i = i + 1) begin
      #1 ra = ops[5*i]; we = ops[5*i+1]; wa = ops[5*i+2]; wm = ops[5*i+3]; wd = ops[5*i+4];
      @(negedge fclk); while (!cr) @(negedge fclk);
      @(posedge fclk); #1 mid = rd;
      @(negedge fclk); while (!fedge) @(negedge fclk);
      $fwrite(f, "%h %h\n", mid, rd);
      @(posedge fclk);
    end
    $fclose(f);
    $finish;
  end
endmodule
"#,
            top = w - 1,
            mtop = lanes - 1,
            name = ram_ce_name(w),
            last = 5 * OPS - 1,
        );
        std::fs::write(dir.path().join("tb.v"), tb).unwrap();
        let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "ram.v", "clock.v"]).output().expect("iverilog");
        assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
        let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
        assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
        let out = std::fs::read_to_string(dir.path().join("out.hex")).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), OPS, "width {w}, divide {divide}: a word for every engine cycle");
        let mut mem: Vec<Option<u128>> = vec![None; DEPTH];
        let (mut wrong, mut checked, mut same) = (Vec::new(), 0, 0);
        for (i, &(ra, we, wa, wm, wd)) in ops.iter().enumerate() {
            if let Some(want) = mem[ra] {
                checked += 1;
                same += (we && wa == ra) as usize;
                let words: Vec<u128> = lines[i]
                    .split_whitespace()
                    .map(|x| u128::from_str_radix(x, 16).unwrap_or_else(|_| panic!("width {w} divide {divide} cycle {i}: {}", lines[i])))
                    .collect();
                if words != [want, want] {
                    wrong.push(format!("width {w} divide {divide} cycle {i} address {ra}: read {:x}, at the step {:x}, model {want:x}", words[0], words[1]));
                }
            }
            if we {
                let mut word = mem[wa].unwrap_or(0);
                for b in 0..lanes {
                    if wm >> b & 1 == 1 {
                        word = word & !(0xffu128 << (8 * b)) | wd & (0xffu128 << (8 * b));
                    }
                }
                mem[wa] = if mem[wa].is_some() || wm == (1 << lanes) - 1 { Some(word) } else { None };
            }
        }
        assert!(checked > OPS - 2 * DEPTH && same > OPS / 20, "width {w} divide {divide}: {checked} engine cycles checked, {same} of them a read of the word written");
        assert!(wrong.is_empty(), "{} words differ:\n{}", wrong.len(), wrong.iter().take(16).cloned().collect::<Vec<_>>().join("\n"));
    }
}

/// The four memory forms of `rtl::ram`, each with its generator, its module name for a width, the ports it takes besides a
/// plain memory's, and whether it is built in stacks (the registered and held forms).
fn memory_forms() -> [(&'static str, fn(&[usize]) -> String, fn(usize) -> String, &'static str, bool); 4] {
    use logicaffeine_compile::spatial::decode_engine::rtl::ram::{
        bram_name, bram_re_name, bram_re_verilog, bram_verilog, ram_ce_name, ram_ce_verilog, ram_name, ram_verilog,
    };
    [
        ("registered", bram_verilog, bram_name, "", true),
        ("falling", ram_verilog, ram_name, "", false),
        ("held", bram_re_verilog, bram_re_name, ".re(ctl[0]), ", true),
        ("enabled", ram_ce_verilog, ram_ce_name, ".ce(ctl[1]), .cr(ctl[2]), ", false),
    ]
}

/// The pipelined token-row store (`logos_store_w512`, which a build whose core reads the store for the host alone reads
/// through) reads what the store of one clock (`logos_bram_re_w512`) reads, STORE_READ_LATENCY clocks after its address
/// instead of one: both banks under the same clocks of random reads held between the clocks `re` is high on, writes under
/// random byte masks, a write and a read of one address in one clock (the read the word before the write), reads of the
/// words just written and of every layer's first and last word, against a model of the memory — at a depth of less than
/// a layer, of a layer, of nine and ten layers (two groups of its gathering) and of eighteen (three).
#[test]
fn the_pipelined_store_reads_what_the_store_of_one_clock_reads_its_latency_later() {
    use logicaffeine_compile::spatial::decode_engine::rtl::ram::{store_verilog, STORE_READ_LATENCY};
    if !common::require_tool("iverilog") {
        return;
    }
    const OPS: usize = 6000;
    let l = STORE_READ_LATENCY;
    for (depth, a) in [(300usize, 9usize), (512, 9), (4608, 13), (4708, 13), (8705, 14)] {
        let mut r = Rng(0x5701e ^ depth as u64);
        let layers = depth.div_ceil(512);
        let mut hot = 0usize;
        // Each clock's (re, ra, we, wa, wm, wd): every word written whole first, then the random clocks.
        let mut ops: Vec<(bool, usize, bool, usize, u64, [u64; 8])> = Vec::new();
        for i in 0..depth {
            let wd: [u64; 8] = std::array::from_fn(|_| r.next());
            ops.push((false, 0, true, i, u64::MAX, wd));
        }
        for _ in 0..OPS {
            if r.next() % 64 == 0 {
                hot = (r.next() % depth as u64) as usize;
            }
            let pick = |r: &mut Rng| -> usize {
                match r.next() % 4 {
                    0 | 1 => (hot + (r.next() % 24) as usize) % depth,
                    2 => (r.next() % depth as u64) as usize,
                    _ => {
                        let layer = (r.next() % layers as u64) as usize;
                        (layer * 512 + if r.next() % 2 == 0 { 0 } else { 511 }).min(depth - 1)
                    }
                }
            };
            let ra = pick(&mut r);
            let wa = if r.next() % 4 == 0 { ra } else { pick(&mut r) };
            let wd: [u64; 8] = std::array::from_fn(|_| r.next());
            ops.push((r.next() % 8 != 0, ra, r.next() % 4 != 0, wa, r.next(), wd));
        }
        let word = |w: &[u64; 8]| -> String { w.iter().rev().map(|x| format!("{x:016x}")).collect() };
        let mut hex = String::new();
        for (re, ra, we, wa, wm, wd) in &ops {
            writeln!(hex, "{:x}\n{ra:x}\n{:x}\n{wa:x}\n{wm:x}\n{}", *re as u8, *we as u8, word(wd)).unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ops.hex"), hex).unwrap();
        std::fs::write(dir.path().join("store.v"), store_verilog()).unwrap();
        let tb = format!(
            r#"`define LOGOS_RAMSTYLE(words) (* ramstyle = (words) <= 32 ? "MLAB" : "M20K" *)
module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  reg re = 0, we = 0;
  reg [{at}:0] ra = 0, wa = 0;
  reg [511:0] wd = 0;
  reg [63:0] wm = 0;
  wire [511:0] one, piped;
  logos_bram_re_w512 #(.D({depth}), .A({a})) m1(.clk(clk), .re(re), .ra(ra), .rd(one), .we(we), .wa(wa), .wd(wd), .wm(wm));
  logos_store_w512 #(.D({depth}), .A({a})) m4(.clk(clk), .re(re), .ra(ra), .rd(piped), .we(we), .wa(wa), .wd(wd), .wm(wm));
  reg [511:0] ops [0:{last}];
  integer i, f;
  initial begin
    $readmemh("ops.hex", ops);
    f = $fopen("out.hex", "w");
    for (i = 0; i < {n}; i = i + 1) begin
      @(negedge clk);
      re = ops[6*i][0]; ra = ops[6*i+1][{at}:0]; we = ops[6*i+2][0]; wa = ops[6*i+3][{at}:0]; wm = ops[6*i+4][63:0]; wd = ops[6*i+5];
      @(posedge clk); #1;
      $fwrite(f, "%h %h\n", one, piped);
    end
    $fclose(f);
    $finish;
  end
endmodule
"#,
            at = a - 1,
            last = 6 * ops.len() - 1,
            n = ops.len(),
        );
        std::fs::write(dir.path().join("tb.v"), tb).unwrap();
        let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "store.v"]).output().expect("iverilog");
        assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
        let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
        assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
        let out = std::fs::read_to_string(dir.path().join("out.hex")).unwrap();
        let lines: Vec<(&str, &str)> = out.lines().map(|x| x.split_once(' ').expect("two words a clock")).collect();
        assert_eq!(lines.len(), ops.len(), "depth {depth}: a word of each bank a clock");
        // The model: each clock's read sees the memory as the clocks before it left it, and is held while re is low.
        let mut mem: Vec<[u64; 8]> = vec![[0; 8]; depth];
        let mut read: Vec<Option<String>> = Vec::with_capacity(ops.len());
        for (t, (re, ra, we, wa, wm, wd)) in ops.iter().enumerate() {
            let held = if t > 0 { read[t - 1].clone() } else { None };
            read.push(if *re { Some(word(&mem[*ra])) } else { held });
            if *we {
                for b in 0..64 {
                    if wm >> b & 1 == 1 {
                        let (lane, shift) = (b / 8, 8 * (b % 8));
                        mem[*wa][lane] = mem[*wa][lane] & !(0xffu64 << shift) | wd[lane] & (0xffu64 << shift);
                    }
                }
            }
        }
        let (mut wrong, mut checked) = (Vec::new(), 0);
        for t in depth + l..ops.len() {
            let (Some(want), Some(was)) = (&read[t], &read[t + 1 - l]) else { continue };
            checked += 1;
            if lines[t].0 != want {
                wrong.push(format!("depth {depth} clock {t}: the bank of one clock read {}, the model {want}", &lines[t].0[..16]));
            }
            if lines[t].1 != was {
                wrong.push(format!("depth {depth} clock {t}: the pipelined bank read {}, the model {l} clocks before {was}", &lines[t].1[..16]));
            }
        }
        assert!(checked > OPS - 3 * l, "depth {depth}: {checked} clocks checked");
        assert!(wrong.is_empty(), "{} words differ:\n{}", wrong.len(), wrong.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
    }
}

/// Every layer of the pipelined token-row store reads every clock as its block RAM reads: yosys taking each register a
/// read is taken into into its memory's read port (`memory_dff`), every layer's read port is clocked, enabled on every
/// clock and reads the word a write of its address in the same clock leaves (not transparent), so the block takes the
/// read register whole. A layer read under the bank's read enable instead takes the address register the bank sets into
/// its port and reads through a write of it; Quartus then builds each layer's read-during-write as pass-through logic in
/// the fabric (hybrid9g: 517 ALUTs and 1,555 registers a layer, 71 layers a bank, two banks). At a layer, at ten layers
/// (two groups of its gathering) and at eighteen (three).
#[test]
fn every_layer_of_the_pipelined_store_reads_every_clock_as_its_block_ram_reads() {
    use logicaffeine_compile::spatial::decode_engine::rtl::ram::store_verilog;
    if !common::require_tool("yosys") {
        return;
    }
    let number = |v: &serde_json::Value| match v {
        serde_json::Value::String(s) => u128::from_str_radix(s, 2).unwrap_or_else(|_| panic!("a parameter {s}")),
        v => v.as_u64().unwrap_or_else(|| panic!("a parameter {v}")) as u128,
    };
    for (depth, a) in [(512usize, 9usize), (4708, 13), (8705, 14)] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("store.v"), store_verilog()).unwrap();
        let script = format!(
            "read_verilog -sv store.v; hierarchy -top logos_store_w512 -chparam D {depth} -chparam A {a}; proc; flatten; opt_clean; memory_dff; write_json s.json"
        );
        let o = Command::new("yosys").current_dir(dir.path()).args(["-q", "-p", &script]).output().expect("yosys");
        assert!(o.status.success(), "yosys on the store at {depth}: {}", String::from_utf8_lossy(&o.stderr));
        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("s.json")).unwrap()).unwrap();
        let module = &json["modules"]["logos_store_w512"];
        let reads: Vec<(&String, &serde_json::Value)> =
            module["cells"].as_object().unwrap().iter().filter(|(_, c)| c["type"].as_str().is_some_and(|t| t.starts_with("$memrd"))).collect();
        assert_eq!(reads.len(), depth.div_ceil(512), "depth {depth}: a read port a layer");
        for (name, cell) in reads {
            let p = &cell["parameters"];
            assert_eq!(number(&p["CLK_ENABLE"]), 1, "depth {depth}: {name} reads into a register of its own");
            assert_eq!(number(&p["TRANSPARENCY_MASK"]), 0, "depth {depth}: {name} reads the word before a write of its address, not through it");
            let en = cell["connections"]["EN"].as_array().unwrap();
            assert!(en.iter().all(|b| b == "1"), "depth {depth}: {name} reads every clock, its enable {en:?}");
        }
    }
}

/// A registered or held memory (the vector units' registers, the token-row store) of a word as wide as an M20K block's
/// widest (40 bits) or wider, more than one layer of 512 words deep in a number of layers that is no power of two, is
/// stacks of powers of two layers: its deepest the memory's own from address 0 and each other a `<name>_part` beside it,
/// every one at its module's own scope (so `ram_check.py` passes it and Quartus builds each in block RAM). The fitter
/// builds a memory's depth as a stack of a power of two layers, so the vector unit's 1,088 rows take three layers' blocks
/// where they took four, and the store's 57,344 words 112 where they took 128. A memory of one layer, or of a power of two
/// layers, or narrower than 40 bits, is one memory as it was, and a falling-edge or enabled memory (the reference
/// engine's, the core's) is one memory at every depth.
#[test]
fn a_memory_whose_layers_are_no_power_of_two_is_stacks_of_powers_of_two() {
    if !common::require_tool("yosys") {
        return;
    }
    let cases: [(usize, usize, &[usize]); 14] = [
        (512, 320, &[320]),
        (512, 512, &[512]),
        (512, 576, &[576]),
        (512, 1024, &[1024]),
        (512, 1088, &[512, 1024]),
        (512, 1536, &[512, 1024]),
        (512, 2560, &[512, 2048]),
        (512, 3584, &[512, 1024, 2048]),
        (512, 57_344, &[8192, 16_384, 32_768]),
        (512, 65_536, &[65_536]),
        (88, 1088, &[512, 1024]),
        (40, 1536, &[512, 1024]),
        (32, 1088, &[1088]),
        (8, 3584, &[3584]),
    ];
    let mut wrong = Vec::new();
    for (form, verilog, name, extra, stacked) in memory_forms() {
        for &(w, depth, stacks) in &cases {
            let whole = [depth];
            let want: &[usize] = if stacked { stacks } else { &whole };
            let a = depth.next_power_of_two().trailing_zeros().max(1) as usize;
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("ram.v"), verilog(&[w])).unwrap();
            std::fs::write(
                dir.path().join("top.v"),
                format!(
                    "module top(input clk, input [2:0] ctl, input [{at}:0] ra, output [{wt}:0] rd, input we, input [{at}:0] wa, input [{wt}:0] wd, input [{mt}:0] wm);\n  \
                     {m} #(.D({depth}), .A({a})) m(.clk(clk), {extra}.ra(ra), .rd(rd), .we(we), .wa(wa), .wd(wd), .wm(wm));\nendmodule\n",
                    at = a - 1,
                    wt = w - 1,
                    mt = w / 8 - 1,
                    m = name(w),
                ),
            )
            .unwrap();
            let o = Command::new("yosys")
                .current_dir(dir.path())
                .args(["-q", "-p", "read_verilog -sv top.v ram.v; hierarchy -top top; tee -q -o dump.txt dump m:*"])
                .output()
                .expect("yosys");
            assert!(o.status.success(), "yosys on the {form} memory {w} x {depth}: {}", String::from_utf8_lossy(&o.stderr));
            let dump = std::fs::read_to_string(dir.path().join("dump.txt")).unwrap();
            let mut sizes: Vec<usize> = dump
                .lines()
                .filter_map(|l| {
                    let t = l.trim();
                    let rest = t.strip_prefix("memory ")?;
                    let width = rest.strip_prefix("width ").and_then(|r| r.split(' ').next()).map_or(Some(1), |x| x.parse().ok())?;
                    let size: usize = rest.split("size ").nth(1)?.split(' ').next()?.parse().ok()?;
                    (width == w).then_some(size)
                })
                .collect();
            sizes.sort();
            if sizes != want {
                wrong.push(format!("the {form} memory {w} x {depth}: memories of {sizes:?} words, want {want:?}"));
            }
            let check = Command::new("python3")
                .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/d5005/ram_check.py"))
                .args(["top", "65536", "top.v", "ram.v"])
                .current_dir(dir.path())
                .output()
                .expect("python3");
            if !check.status.success() {
                wrong.push(format!("the {form} memory {w} x {depth}: ram_check.py refused it: {}", String::from_utf8_lossy(&check.stderr)));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A memory built as stacks of powers of two reads and writes as the one memory of its depth (its `_part` at that
/// depth, the single memory every narrower or power-of-two memory still is), in both forms built in stacks: a seeded run
/// of reads and byte-masked writes, a third of them at the stacks' first and last words, a quarter of the writes to the
/// word read, the held form's read enable seeded too, every clock's word the same.
#[test]
fn a_memory_of_stacks_reads_and_writes_as_one_memory_of_its_depth() {
    if !common::require_tool("iverilog") {
        return;
    }
    let mut wrong = Vec::new();
    for (form, verilog, name, extra, _) in memory_forms().into_iter().filter(|f| f.4) {
        for (w, depth, ops) in [(512usize, 1088usize, 12_000usize), (88, 1088, 12_000), (512, 3584, 16_000), (88, 2560, 12_000), (512, 57_344, 60_000)] {
            let a = depth.next_power_of_two().trailing_zeros() as usize;
            let mut r = Rng(0x5a1c ^ (w as u64) << 20 ^ depth as u64);
            let edges: Vec<usize> = [0usize, 511, 512, 1023, 1024, 1535, 1536, 2047, 2048, 2559, 3071, 3072, 3583, 32_767, 32_768, 49_151, 49_152, depth - 1]
                .into_iter()
                .filter(|&x| x < depth)
                .collect();
            let addr = |r: &mut Rng| if r.next() % 3 == 0 { edges[(r.next() % edges.len() as u64) as usize] } else { (r.next() % depth as u64) as usize };
            let lanes = w / 8;
            let mut hex = String::new();
            for i in 0..ops {
                let ra = addr(&mut r);
                let wa = if r.next() % 4 == 0 { ra } else { addr(&mut r) };
                let we = i < 64 || r.next() % 2 == 0;
                let wm: u128 = if r.next() % 3 == 0 { (1u128 << lanes) - 1 } else { (r.next() as u128) & ((1u128 << lanes) - 1) };
                let wd: String = (0..w / 4).map(|_| format!("{:x}", r.next() % 16)).collect();
                writeln!(hex, "{ra:x}\n{:x}\n{wa:x}\n{wm:x}\n{wd}\n{:x}", we as u8, r.next() % 8).unwrap();
            }
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("ops.hex"), hex).unwrap();
            std::fs::write(dir.path().join("ram.v"), verilog(&[w])).unwrap();
            let m = name(w);
            let tb = format!(
                r#"module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  reg [{wt}:0] mem_ops [0:{last}];
  reg [{at}:0] ra = 0, wa = 0;
  reg we = 0;
  reg [{wt}:0] wd = 0;
  reg [{mt}:0] wm = 0;
  reg [2:0] ctl = 0;
  wire [{wt}:0] stacked, whole;
  {m} #(.D({depth}), .A({a})) s(.clk(clk), {extra}.ra(ra), .rd(stacked), .we(we), .wa(wa), .wd(wd), .wm(wm));
  {m}_part #(.D({depth}), .A({a})) one(.clk(clk), {extra}.ra(ra), .rd(whole), .we(we), .wa(wa), .wd(wd), .wm(wm));
  integer i, bad = 0, known = 0;
  initial begin
    $readmemh("ops.hex", mem_ops);
    for (i = 0; i < {ops}; i = i + 1) begin
      @(posedge clk); #2;
      if (stacked !== whole) begin bad = bad + 1; if (bad < 6) $display("clock %0d: stacked %h, one memory %h", i, stacked, whole); end
      if (^whole !== 1'bx) known = known + 1;
      ra = mem_ops[6*i]; we = mem_ops[6*i+1]; wa = mem_ops[6*i+2]; wm = mem_ops[6*i+3]; wd = mem_ops[6*i+4]; ctl = mem_ops[6*i+5];
    end
    $display("CLOCKS {ops} BAD %0d KNOWN %0d", bad, known);
    $finish;
  end
endmodule
"#,
                wt = w - 1,
                at = a - 1,
                mt = lanes - 1,
                last = 6 * ops - 1,
            );
            std::fs::write(dir.path().join("tb.v"), tb).unwrap();
            let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "ram.v"]).output().expect("iverilog");
            assert!(build.status.success(), "iverilog on the {form} memory {w} x {depth}: {}", String::from_utf8_lossy(&build.stderr));
            let run = Command::new("vvp").current_dir(dir.path()).args(["-n", "sim"]).output().expect("vvp");
            let out = String::from_utf8_lossy(&run.stdout).to_string();
            let known: usize = out.split("KNOWN ").nth(1).and_then(|k| k.split_whitespace().next()).and_then(|n| n.parse().ok()).unwrap_or(0);
            if !out.contains(&format!("CLOCKS {ops} BAD 0 ")) || known < ops / 3 {
                wrong.push(format!("the {form} memory {w} x {depth} ({known} known reads): {out}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}


/// One run of the streaming column: the activations' word count, `groups` groups of `pairs` row pairs against `tokens`
/// token-rows, the stream offering its next beat at random gaps or, `steady`, whenever the column is ready, and,
/// `aborted`, a run started, fed forty clocks and cut short by a reset before the run that counts.
#[derive(Clone, Copy)]
struct ColumnRun {
    words: Words,
    pairs: usize,
    tokens: usize,
    groups: usize,
    steady: bool,
    aborted: bool,
}

/// The streaming column `logos_column_<format>` under the RTL simulator, built once for the format of `d` and run for
/// each of `runs`: rows of `ELEMS` elements streamed block-major within each group (a group's every row pair's block 0,
/// then block 1, …). Every emission is checked against `numerics::dot` (each (group, pair, token-row) once, both rows
/// bit for bit); returned for each run are the differences and the clocks from the edge that took `start` to the edge
/// that raised the last sum.
fn column_runs(d: &logicaffeine_compile::spatial::decode_engine::rtl::weights::Decoder, runs: &[ColumnRun], r: &mut Rng) -> Vec<(Vec<String>, usize)> {
    use logicaffeine_compile::spatial::decode_engine::rtl::column::column_verilog;
    const ELEMS: usize = 512;
    let (len, bb) = (d.ty.block_len(), d.ty.block_bytes());
    let blocks = ELEMS / len;
    let rows_max = runs.iter().map(|c| 2 * c.pairs * c.groups).max().unwrap();
    let tokens_max = runs.iter().map(|c| c.tokens).max().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("column.v"), column_verilog()).unwrap();
    // The run: groups, pairs, blocks, token-rows, word count, steady, aborted, beats, activation blocks.
    let tb = format!(
        r#"module tb;
  reg clk = 0, rst = 1;
  always #5 clk = ~clk;
  reg [31:0] cfg [0:8];
  reg start = 0, act_wr = 0, feeding = 0, gap_valid = 0, aborting = 0, steady = 0;
  reg [7:0] act_t, act_m;
  reg [15:0] act_b;
  reg [11:0] act_e;
  reg [{aw}:0] act_codes;
  reg [{bw}:0] gap_blk1, gap_blk2;
  wire busy, w_ready, o_valid;
  wire [15:0] o_group;
  wire [7:0] o_pair, o_t;
  wire [31:0] o_val1, o_val2;
  reg [{aw}:0] am [0:{alast}];
  reg [{bw}:0] wm [0:{wlast}];
  integer i, f, n = 0, edges = 0, started = 0, emitted = 0;
  wire w_valid = (steady || aborting) ? (feeding && n < cfg[7]) : gap_valid;
  wire [{bw}:0] w_blk1 = (steady || aborting) ? wm[2*n] : gap_blk1, w_blk2 = (steady || aborting) ? wm[2*n+1] : gap_blk2;
  logos_column_{name} #(.T_MAX(8), .NB_MAX(16), .P_MAX(4)) u(.clk(clk), .rst(rst), .start(start), .groups(cfg[0][15:0]),
    .pairs(cfg[1][7:0]), .blocks(cfg[2][15:0]), .tokens(cfg[3][7:0]), .two(cfg[4][0]), .busy(busy), .act_wr(act_wr),
    .act_t(act_t), .act_b(act_b), .act_codes(act_codes), .act_m(act_m), .act_e(act_e), .w_valid(w_valid),
    .w_ready(w_ready), .w_blk1(w_blk1), .w_blk2(w_blk2), .o_valid(o_valid), .o_group(o_group), .o_pair(o_pair),
    .o_t(o_t), .o_val1(o_val1), .o_val2(o_val2));
  always @(posedge clk) begin
    edges <= edges + 1;
    if (rst) n <= 0;
    else if (w_valid && w_ready) n <= n + 1;
  end
  always @(negedge clk) if (o_valid && !aborting) begin
    $fwrite(f, "%h %h %h %h %h\n", o_group, o_pair, o_t, o_val1, o_val2);
    emitted = edges;
  end
  initial begin
    $readmemh("config.hex", cfg);
    $readmemh("acts.hex", am);
    $readmemh("weights.hex", wm);
    steady = cfg[5][0];
    aborting = cfg[6][0];
    f = $fopen("out.txt", "w");
    @(negedge clk); rst = 0;
    for (i = 0; i < cfg[8]; i = i + 1) begin
      @(negedge clk); act_wr = 1; act_codes = am[5*i]; act_t = am[5*i+1]; act_b = am[5*i+2]; act_m = am[5*i+3]; act_e = am[5*i+4];
    end
    @(negedge clk); act_wr = 0;
    if (aborting) begin
      start = 1; @(negedge clk); start = 0; feeding = 1;
      repeat (40) @(negedge clk);
      feeding = 0; rst = 1; @(negedge clk); rst = 0; @(negedge clk); aborting = 0;
    end
    start = 1;
    @(negedge clk); start = 0; started = edges;
    // A steady stream presents beat `n` until the edge that takes it; a gapped one presents each beat alone, waits
    // for the column to take it, and lets a quarter of the clocks after a beat go by empty.
    if (steady) begin
      feeding = 1;
      while (n < cfg[7]) @(negedge clk);
      feeding = 0;
    end else
      for (i = 0; i < cfg[7]; i = i + 1) begin
        gap_blk1 = wm[2*i]; gap_blk2 = wm[2*i+1]; gap_valid = 1;
        while (!w_ready) @(negedge clk);
        @(negedge clk); gap_valid = 0;
        if (($random & 3) == 0) @(negedge clk);
      end
    while (busy) @(negedge clk);
    @(negedge clk); @(negedge clk);
    $fwrite(f, "clocks %0d\n", emitted - started);
    $fclose(f);
    $finish;
  end
endmodule
"#,
        aw = 16 * len - 1,
        bw = 8 * bb - 1,
        name = d.ty.name().to_lowercase(),
        alast = 5 * tokens_max * blocks - 1,
        wlast = rows_max * blocks - 1,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let sim = rtl_sim::Simulator::build(dir.path(), &["column.v", "tb.v"]);
    let mut out = Vec::new();
    for (k, c) in runs.iter().enumerate() {
        let ColumnRun { words, pairs, tokens, groups, steady, aborted } = *c;
        let rows_n = 2 * pairs * groups;
        let raw: Vec<Vec<u8>> = (0..rows_n).map(|_| small_scaled_blocks(r, d.ty, blocks)).collect();
        let rows: Vec<CodedRow> = raw.iter().map(|b| CodedRow::decode(d.ty, b).unwrap().unwrap()).collect();
        let acts: Vec<_> = (0..tokens)
            .map(|_| {
                let x: Vec<f32> = (0..ELEMS).map(|_| ((r.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * 2f32.powi((r.next() % 16) as i32 - 8)).collect();
                quantize_blocks(&x, words, len)
            })
            .collect();
        let at = dir.path().join(format!("run{k}"));
        std::fs::create_dir_all(&at).unwrap();
        let mut act_hex = String::new();
        for (t, act) in acts.iter().enumerate() {
            for b in 0..blocks {
                for i in (0..len).rev() {
                    write!(act_hex, "{:04x}", act.codes[b * len + i] as i16 as u16).unwrap();
                }
                writeln!(act_hex, "\n{t:02x}\n{b:04x}\n{:02x}\n{:03x}", act.scales[b].m, act.scales[b].e as i16 as u16 & 0xfff).unwrap();
            }
        }
        // Block-major within each group: for every group, for every block, every row pair's two blocks.
        let mut w_hex = String::new();
        for g in 0..groups {
            for b in 0..blocks {
                for p in 0..pairs {
                    for k in [2 * (g * pairs + p), 2 * (g * pairs + p) + 1] {
                        for byte in raw[k][b * bb..(b + 1) * bb].iter().rev() {
                            write!(w_hex, "{byte:02x}").unwrap();
                        }
                        w_hex.push('\n');
                    }
                }
            }
        }
        let config = [groups, pairs, blocks, tokens, (words == Words::Two) as usize, steady as usize, aborted as usize, rows_n / 2 * blocks, tokens * blocks];
        std::fs::write(at.join("config.hex"), config.iter().map(|c| format!("{c:x}\n")).collect::<String>()).unwrap();
        std::fs::write(at.join("acts.hex"), act_hex).unwrap();
        std::fs::write(at.join("weights.hex"), w_hex).unwrap();
        let run = sim.run(&at, &[]);
        assert!(run.status.success(), "simulation ({}): {}", d.ty.name(), String::from_utf8_lossy(&run.stderr));
        let text = std::fs::read_to_string(at.join("out.txt")).unwrap();
        let (sums, clocks) = text.trim_end().rsplit_once('\n').map(|(s, c)| (s.to_string(), c)).unwrap_or((String::new(), text.trim_end()));
        let clocks: usize = clocks.strip_prefix("clocks ").expect("the run's clock count").parse().unwrap();
        let lines: Vec<&str> = sums.lines().collect();
        let what = format!("{} {words:?} at {groups} groups of {pairs} pairs and {tokens} token-rows", d.ty.name());
        assert_eq!(lines.len(), rows_n / 2 * tokens, "{what}: every token-row's every row pair emitted once");
        let mut wrong = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for l in lines {
            let f: Vec<u32> = l.split(' ').map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let (g, p, t) = (f[0] as usize, f[1] as usize, f[2] as usize);
            assert!(seen.insert((g, p, t)), "{what}: group {g} pair {p} token-row {t} emitted twice");
            for (k, got) in [(2 * (g * pairs + p), f[3]), (2 * (g * pairs + p) + 1, f[4])] {
                let want = dot(&rows[k], &acts[t]).to_bits();
                if got != want {
                    wrong.push(format!("{what}: row {k} token-row {t}: rtl {got:08x}, dot {want:08x}"));
                }
            }
        }
        out.push((wrong, clocks));
    }
    out
}

/// The streaming column, `logos_column_<format>`, under iverilog: a matrix of eight rows of 512 elements streamed
/// block-major within groups of R rows, at random gaps, against T token-rows in both word counts, every emitted (row,
/// token-row) `numerics::dot`, bit for bit — for the K-quants, the legacy formats and IQ4, at groups of one, two and
/// four row pairs and one to three token-rows, so a sum comes round again from every clock (waiting on the add) to
/// every eighth.
#[test]
fn the_streaming_column_multiplies_a_matrix_block_major_as_dot_does() {
    if !common::require_tool("iverilog") {
        return;
    }
    const ROWS: usize = 8;
    let shapes = [(1usize, 1usize), (2, 3), (4, 2), (1, 3), (2, 1)];
    let mut r = Rng(0xc01d);
    let mut wrong = Vec::new();
    let formats: Vec<_> = DECODERS.iter().filter(|d| matches!(d.ty, GgmlType::Q4K | GgmlType::Q6K | GgmlType::Q8_0 | GgmlType::Q4_0 | GgmlType::Iq4Xs)).collect();
    for (fi, d) in formats.iter().enumerate() {
        let runs: Vec<ColumnRun> = [Words::One, Words::Two]
            .into_iter()
            .enumerate()
            .map(|(wi, words)| {
                let (pairs, tokens) = shapes[(2 * fi + wi) % shapes.len()];
                ColumnRun { words, pairs, tokens, groups: ROWS / (2 * pairs), steady: false, aborted: false }
            })
            .collect();
        wrong.extend(column_runs(d, &runs, &mut r).into_iter().flat_map(|(w, _)| w));
    }
    assert!(wrong.is_empty(), "{} dots differ:\n{}", wrong.len(), wrong.iter().take(16).cloned().collect::<Vec<_>>().join("\n"));
}

/// The streaming column fed a beat whenever it is ready takes exactly `column_clocks` from `start` to its last sum: a
/// pass of a (pair, token-row) every clock once a block step has more passes than the add has stages — across passes,
/// beats, blocks and groups, no clock lost between them — and a step of fewer waiting only for its sums to leave the
/// add; every sum still `numerics::dot`'s, a run after one cut short by a reset too. One to eight issues a step of one to
/// four passes (one and two code digits, one and two activation words), one to four groups.
#[test]
fn the_streaming_column_runs_a_pass_a_clock() {
    use logicaffeine_compile::spatial::decode_engine::rtl::column::{column_clocks, column_passes};
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0xc10c);
    let mut wrong = Vec::new();
    let q4k = DECODERS.iter().find(|d| d.ty == GgmlType::Q4K).unwrap();
    let q8 = DECODERS.iter().find(|d| d.ty == GgmlType::Q8_0).unwrap();
    // (words, pairs, tokens, groups, aborted first) a format: steps of 1, 3, 4, 5, 6 and 8 issues.
    let shapes = [
        (q4k, vec![(Words::One, 1usize, 1usize, 2usize, false), (Words::Two, 2, 2, 2, true), (Words::One, 1, 5, 4, false), (Words::One, 4, 2, 1, true)]),
        (q8, vec![(Words::Two, 1, 3, 1, true), (Words::One, 2, 3, 2, false)]),
    ];
    for (d, shapes) in shapes {
        let blocks = 512 / d.ty.block_len();
        let runs: Vec<ColumnRun> = shapes.iter().map(|&(words, pairs, tokens, groups, aborted)| ColumnRun { words, pairs, tokens, groups, steady: true, aborted }).collect();
        for (c, (diffs, clocks)) in runs.iter().zip(column_runs(d, &runs, &mut r)) {
            wrong.extend(diffs);
            let want = column_clocks(c.groups, c.pairs, blocks, c.tokens, column_passes(d.ty, c.words == Words::Two));
            assert_eq!(clocks, want, "{} {:?}: {} groups of {} pairs, {blocks} blocks, {} token-rows", d.ty.name(), c.words, c.groups, c.pairs, c.tokens);
        }
    }
    assert!(wrong.is_empty(), "{} dots differ:\n{}", wrong.len(), wrong.iter().take(16).cloned().collect::<Vec<_>>().join("\n"));
}

/// One run of the column array: the activations' word count, the matrix's rows, the token-rows, the channels' turns in
/// the stripes' deal, and whether every core's stream offers its beat whenever the core is ready (`steady`) or at
/// random gaps with an eighth of the array's clocks held.
#[derive(Clone, Copy)]
struct ArrayRun {
    words: Words,
    rows: usize,
    tokens: usize,
    turns: [usize; 4],
    steady: bool,
}

/// The column array `logos_column_array_<format>` under the RTL simulator, built once for the format of `d` at four
/// channels of `n` cores and run for each of `runs`: a matrix of 512-element rows laid in stripes of `n` pairs a channel,
/// each core fed its beats (its pair's block of every stripe its channel's tile holds the pair in, the second block
/// zeros past the tensor's last row). Every emission is checked against `numerics::dot` — every row and token-row once,
/// both rows bit for bit, no core emitting for a pair past the tensor — and returned for each run are the differences
/// and the clocks from the edge that took `start` to the edge that raised the last sums.
fn column_array_runs(d: &logicaffeine_compile::spatial::decode_engine::rtl::weights::Decoder, n: usize, runs: &[ArrayRun], r: &mut Rng) -> Vec<(Vec<String>, usize)> {
    use logicaffeine_compile::spatial::decode_engine::rtl::column::{column_verilog, ColumnSet};
    const ELEMS: usize = 512;
    const CH: usize = 4;
    let cores = CH * n;
    let (len, bb) = (d.ty.block_len(), d.ty.block_bytes());
    let blocks = ELEMS / len;
    let rows_max = runs.iter().map(|c| c.rows).max().unwrap();
    let tokens_max = runs.iter().map(|c| c.tokens).max().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("column.v"), column_verilog()).unwrap();
    // The run: stripes, token-rows, word count, rows, turns, steady, activation blocks; each core's first beat and beats.
    let tb = format!(
        r#"module tb;
  localparam C = {cores}, BITS = {bits}, BEAT = {beat}, QUEUED = {queued};
  reg clk = 0, rst = 1;
  always #5 clk = ~clk;
  reg start = 0, act_wr = 0, hold = 0;
  reg [7:0] act_t, act_m;
  reg [15:0] act_b;
  reg [11:0] act_e;
  reg [{aw}:0] act_codes;
  wire busy;
  wire [C-1:0] w_valid, w_ready, w_put, o_valid;
  wire [C*BEAT-1:0] w_beat;
  wire [C*32-1:0] o_val1, o_val2;
  wire [15:0] o_stripe;
  wire [7:0] o_t;
  reg [{aw}:0] am [0:{alast}];
  reg [BITS-1:0] wm [0:{wlast}];
  reg [31:0] cfg [0:6];
  reg [31:0] base [0:C-1];
  reg [31:0] count [0:C-1];
  integer i, j, f, edges = 0, started = 0, emitted = 0;
  logos_column_array_{name} #(.CH(4), .N({n}), .T_MAX(8), .NB_MAX(16)) u(.clk(clk), .rst(rst), .start(start),
    .stripes(cfg[0][15:0]), .blocks(16'd{blocks}), .tokens(cfg[1][7:0]), .two(cfg[2][0]), .rows(cfg[3]),
    .turns(cfg[4][7:0]), .hold(hold), .busy(busy), .act_wr(act_wr), .act_t(act_t), .act_b(act_b), .act_codes(act_codes),
    .act_m(act_m), .act_e(act_e), .w_valid(w_valid), .w_ready(w_ready), .w_put(w_put), .w_beat(w_beat),
    .o_valid(o_valid), .o_val1(o_val1), .o_val2(o_val2), .o_stripe(o_stripe), .o_t(o_t));
  genvar g;
  generate
    for (g = 0; g < C; g = g + 1) begin : feed
      // The core's beats put into its ring, each the clock the ring has room for it (QUEUED beats queued behind the one
      // it holds), at random gaps unless steady; the ring's queued beats the core's to take.
      integer k = 0, q = 0;
      reg gap = 0;
      assign w_put[g] = !rst && k < count[g] && q < QUEUED && !gap;
      assign w_valid[g] = q != 0;
      logos_column_decode_{name} dec(.blk1(wm[2*(base[g] + k)]), .blk2(wm[2*(base[g] + k) + 1]), .beat(w_beat[g*BEAT +: BEAT]));
      always @(posedge clk) begin
        if (w_put[g]) k <= k + 1;
        q <= q + w_put[g] - (w_valid[g] && w_ready[g]);
        gap <= !cfg[5][0] && (($random & 3) == 0);
      end
    end
  endgenerate
  // At random gaps an eighth of the clocks held.
  always @(posedge clk) begin
    edges <= edges + 1;
    hold <= !cfg[5][0] && (($random & 7) == 0);
  end
  always @(negedge clk) if (o_valid != 0) begin
    for (j = 0; j < C; j = j + 1) if (o_valid[j]) $fwrite(f, "%0d %h %h %h %h\n", j, o_stripe, o_t, o_val1[j*32 +: 32], o_val2[j*32 +: 32]);
    emitted = edges;
  end
  initial begin
    $readmemh("config.hex", cfg);
    $readmemh("bases.hex", base);
    $readmemh("counts.hex", count);
    $readmemh("acts.hex", am);
    $readmemh("weights.hex", wm);
    f = $fopen("out.txt", "w");
    @(negedge clk); rst = 0;
    for (i = 0; i < cfg[6]; i = i + 1) begin
      @(negedge clk); act_wr = 1; act_codes = am[5*i]; act_t = am[5*i+1]; act_b = am[5*i+2]; act_m = am[5*i+3]; act_e = am[5*i+4];
    end
    @(negedge clk); act_wr = 0; start = 1;
    @(negedge clk); start = 0; started = edges;
    while (busy) @(negedge clk);
    @(negedge clk); @(negedge clk);
    $fwrite(f, "clocks %0d\n", emitted - started);
    $fclose(f);
    $finish;
  end
endmodule
"#,
        aw = 16 * len - 1,
        bits = 8 * bb,
        beat = 2 * ColumnSet::new("", &[d.ty], Words::Two).decoded_bits(),
        queued = if n == 1 { 2 } else { 1 },
        name = d.ty.name().to_lowercase(),
        alast = 5 * tokens_max * blocks - 1,
        wlast = 2 * rows_max.div_ceil(2) * blocks + 2 * cores * blocks - 1,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let sim = rtl_sim::Simulator::build(dir.path(), &["column.v", "tb.v"]);
    let mut out = Vec::new();
    for (run_k, c) in runs.iter().enumerate() {
        let ArrayRun { words, rows: rows_n, tokens, turns, steady } = *c;
        let stripes = rows_n.div_ceil(CH * 2 * n);
        let raw: Vec<Vec<u8>> = (0..rows_n).map(|_| small_scaled_blocks(r, d.ty, blocks)).collect();
        let rows: Vec<CodedRow> = raw.iter().map(|b| CodedRow::decode(d.ty, b).unwrap().unwrap()).collect();
        let acts: Vec<_> = (0..tokens)
            .map(|_| {
                let x: Vec<f32> = (0..ELEMS).map(|_| ((r.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * 2f32.powi((r.next() % 16) as i32 - 8)).collect();
                quantize_blocks(&x, words, len)
            })
            .collect();
        let first_row = |g: usize, s: usize| (CH * s + turns[g / n]) * 2 * n + 2 * (g % n);
        let mut act_hex = String::new();
        for (t, act) in acts.iter().enumerate() {
            for b in 0..blocks {
                for i in (0..len).rev() {
                    write!(act_hex, "{:04x}", act.codes[b * len + i] as i16 as u16).unwrap();
                }
                writeln!(act_hex, "\n{t:02x}\n{b:04x}\n{:02x}\n{:03x}", act.scales[b].m, act.scales[b].e as i16 as u16 & 0xfff).unwrap();
            }
        }
        // Each core's beats in order, its run of the beat memory starting where the core before's ends.
        let block_hex = |k: usize, b: usize| -> String {
            if k >= rows_n {
                return "00".repeat(bb);
            }
            raw[k][b * bb..(b + 1) * bb].iter().rev().map(|byte| format!("{byte:02x}")).collect()
        };
        let (mut w_hex, mut base, mut count, mut at) = (String::new(), Vec::new(), Vec::new(), 0usize);
        for g in 0..cores {
            base.push(at);
            for s in 0..stripes {
                let k = first_row(g, s);
                if k >= rows_n {
                    continue;
                }
                for b in 0..blocks {
                    writeln!(w_hex, "{}\n{}", block_hex(k, b), block_hex(k + 1, b)).unwrap();
                    at += 1;
                }
            }
            count.push(at - base[g]);
        }
        let turn_bits: usize = turns.iter().enumerate().map(|(ch, &t)| t << (2 * ch)).sum();
        let at_dir = dir.path().join(format!("run{run_k}"));
        std::fs::create_dir_all(&at_dir).unwrap();
        let words_hex = |v: &[usize]| v.iter().map(|c| format!("{c:x}\n")).collect::<String>();
        std::fs::write(at_dir.join("config.hex"), words_hex(&[stripes, tokens, (words == Words::Two) as usize, rows_n, turn_bits, steady as usize, tokens * blocks])).unwrap();
        std::fs::write(at_dir.join("bases.hex"), words_hex(&base)).unwrap();
        std::fs::write(at_dir.join("counts.hex"), words_hex(&count)).unwrap();
        std::fs::write(at_dir.join("acts.hex"), act_hex).unwrap();
        std::fs::write(at_dir.join("weights.hex"), w_hex).unwrap();
        let run = sim.run(&at_dir, &[]);
        assert!(run.status.success(), "simulation ({}): {}", d.ty.name(), String::from_utf8_lossy(&run.stderr));
        let text = std::fs::read_to_string(at_dir.join("out.txt")).unwrap();
        let (sums, clocks) = text.trim_end().rsplit_once('\n').map(|(s, c)| (s.to_string(), c)).unwrap_or((String::new(), text.trim_end()));
        let clocks: usize = clocks.strip_prefix("clocks ").expect("the run's clock count").parse().unwrap();
        let what = format!("{} {words:?}: {rows_n} rows in stripes of {n} pairs a channel, turns {turns:?}, {tokens} token-rows", d.ty.name());
        let mut wrong = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for l in sums.lines() {
            let f: Vec<&str> = l.split(' ').collect();
            let g: usize = f[0].parse().unwrap();
            let (s, t) = (usize::from_str_radix(f[1], 16).unwrap(), usize::from_str_radix(f[2], 16).unwrap());
            let k = first_row(g, s);
            assert!(k < rows_n && t < tokens, "{what}: core {g} emitted stripe {s} token-row {t}, row {k}");
            for (row, got) in [(k, f[3]), (k + 1, f[4])] {
                if row >= rows_n {
                    continue;
                }
                assert!(seen.insert((row, t)), "{what}: row {row} token-row {t} emitted twice");
                let got = u32::from_str_radix(got, 16).unwrap();
                let want = dot(&rows[row], &acts[t]).to_bits();
                if got != want {
                    wrong.push(format!("{what}: row {row} token-row {t}: rtl {got:08x}, dot {want:08x}"));
                }
            }
        }
        assert_eq!(seen.len(), rows_n * tokens, "{what}: every row and token-row emitted");
        out.push((wrong, clocks));
    }
    out
}

/// The column array under the RTL simulator, its cores' streams at random gaps and an eighth of its clocks held: a matrix
/// of 512-element rows laid in stripes over four channels dealt in a turn order other than the channels', every row's
/// every token-row `numerics::dot`,
/// bit for bit — stripes whole and cut short, a last tile of an odd row, one to five token-rows (a step waiting on the
/// add and not), both word counts, a K-quant of one digit and of two, and a legacy format of 32-element blocks.
#[test]
fn the_column_array_multiplies_a_striped_matrix_as_dot_does() {
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0xa77a);
    let mut wrong = Vec::new();
    let fmt = |ty| DECODERS.iter().find(|d| d.ty == ty).unwrap();
    // (format, words, pairs a channel, rows, token-rows, the channels' turns)
    let runs = [
        (fmt(GgmlType::Q4K), Words::One, 2usize, 38usize, 3usize, [2usize, 0, 3, 1]),
        (fmt(GgmlType::Q6K), Words::Two, 2, 37, 5, [1, 3, 0, 2]),
        (fmt(GgmlType::Q8_0), Words::One, 1, 13, 1, [3, 2, 1, 0]),
    ];
    for (d, words, n, rows, tokens, turns) in runs {
        let run = ArrayRun { words, rows, tokens, turns, steady: false };
        wrong.extend(column_array_runs(d, n, &[run], &mut r).into_iter().flat_map(|(w, _)| w));
    }
    assert!(wrong.is_empty(), "{} dots differ:\n{}", wrong.len(), wrong.iter().take(16).cloned().collect::<Vec<_>>().join("\n"));
}

/// [`column_array_runs`] for format `ty` of `set`, whose array may take a format's blocks in octets: the set's own build
/// (`columns_verilog`), its format chosen by `fmt` when it holds more than one, each beat a block of the set's widest
/// bytes, and an octet job's activations written a 32-element block at a time into its octet's word at the block's slot
/// (`act_s`).
fn set_array_runs(set: &logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet, ty: GgmlType, n: usize, runs: &[ArrayRun], r: &mut Rng) -> Vec<(Vec<String>, usize)> {
    use logicaffeine_compile::spatial::decode_engine::rtl::column::columns_verilog;
    const ELEMS: usize = 512;
    const CH: usize = 4;
    let cores = CH * n;
    let (len, bb, qlen) = (set.block_len(ty), set.block_bytes(ty), ty.block_len());
    let octet = set.octet(ty);
    let width = set.formats.iter().map(|&t| set.block_bytes(t)).max().unwrap();
    let (blocks, qblocks) = (ELEMS / len, ELEMS / qlen);
    let rows_max = runs.iter().map(|c| c.rows).max().unwrap();
    let tokens_max = runs.iter().map(|c| c.tokens).max().unwrap();
    let t_max = tokens_max.next_power_of_two().max(8);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("column.v"), columns_verilog(std::slice::from_ref(set))).unwrap();
    let place = set.formats.iter().position(|&t| t == ty).unwrap();
    let fmt = if set.formats.len() > 1 { format!(".fmt(4'd{place}), ") } else { String::new() };
    let slot = if set.octets { ".act_s(act_s), " } else { "" };
    // A set of token pairs emits each core's sums a token pair at a time, the second token-row's (o_val1_b, o_val2_b)
    // the one after o_t.
    let (pair_wires, pair_ports, pair_fmt, pair_vals) = if set.token_pairs {
        (", o_val1_b, o_val2_b", ".o_val1_b(o_val1_b), .o_val2_b(o_val2_b), ", " %h %h", ", o_val1_b[j*32 +: 32], o_val2_b[j*32 +: 32]")
    } else {
        ("", "", "", "")
    };
    // A set of paired cores runs its processing elements in clk2, twice clk, the two clocks' rising edges together.
    let (clocks, clk2) = if set.pe_pairs {
        ("  reg clk = 0, clk2 = 0, rst = 1;\n  always #5 clk2 = ~clk2;\n  initial begin #5 clk = 1; forever #10 clk = ~clk; end", ".clk2(clk2), ")
    } else {
        ("  reg clk = 0, rst = 1;\n  always #5 clk = ~clk;", "")
    };
    let tb = format!(
        r#"module tb;
  localparam C = {cores}, BITS = {bits}, BEAT = {beat}, QUEUED = {queued};
{clocks}
  reg start = 0, act_wr = 0, hold = 0;
  reg [7:0] act_t, act_m;
  reg [15:0] act_b;
  reg [11:0] act_e;
  reg [2:0] act_s;
  reg [{aw}:0] act_codes;
  wire busy;
  wire [C-1:0] w_valid, w_ready, w_put, o_valid;
  wire [C*BEAT-1:0] w_beat;
  wire [C*32-1:0] o_val1, o_val2{pair_wires};
  wire [15:0] o_stripe;
  wire [7:0] o_t;
  reg [{aw}:0] am [0:{alast}];
  reg [BITS-1:0] wm [0:{wlast}];
  reg [31:0] cfg [0:6];
  reg [31:0] base [0:C-1];
  reg [31:0] count [0:C-1];
  integer i, j, f, edges = 0, started = 0, emitted = 0;
  logos_column_array_{name} #(.CH(4), .N({n}), .T_MAX({t_max}), .NB_MAX(16)) u(.clk(clk), {clk2}.rst(rst), .start(start),
    .stripes(cfg[0][15:0]), .blocks(16'd{blocks}), .tokens(cfg[1][7:0]), .two(cfg[2][0]), {fmt}.rows(cfg[3]),
    .turns(cfg[4][7:0]), .hold(hold), .busy(busy), .act_wr(act_wr), .act_t(act_t), .act_b(act_b), {slot}.act_codes(act_codes),
    .act_m(act_m), .act_e(act_e), .w_valid(w_valid), .w_ready(w_ready), .w_put(w_put), .w_beat(w_beat),
    .o_valid(o_valid), .o_val1(o_val1), .o_val2(o_val2), {pair_ports}.o_stripe(o_stripe), .o_t(o_t));
  genvar g;
  generate
    for (g = 0; g < C; g = g + 1) begin : feed
      // The core's beats put into its ring, each the clock the ring has room for it (QUEUED beats queued behind the one
      // it holds), at random gaps unless steady; the ring's queued beats the core's to take.
      integer k = 0, q = 0;
      reg gap = 0;
      assign w_put[g] = !rst && k < count[g] && q < QUEUED && !gap;
      assign w_valid[g] = q != 0;
      logos_column_decode_{name} dec({fmt}.blk1(wm[2*(base[g] + k)]), .blk2(wm[2*(base[g] + k) + 1]), .beat(w_beat[g*BEAT +: BEAT]));
      always @(posedge clk) begin
        if (w_put[g]) k <= k + 1;
        q <= q + w_put[g] - (w_valid[g] && w_ready[g]);
        gap <= !cfg[5][0] && (($random & 3) == 0);
      end
    end
  endgenerate
  // At random gaps an eighth of the clocks held.
  always @(posedge clk) begin
    edges <= edges + 1;
    hold <= !cfg[5][0] && (($random & 7) == 0);
  end
  always @(negedge clk) if (o_valid != 0) begin
    for (j = 0; j < C; j = j + 1) if (o_valid[j]) $fwrite(f, "%0d %h %h %h %h{pair_fmt}\n", j, o_stripe, o_t, o_val1[j*32 +: 32], o_val2[j*32 +: 32]{pair_vals});
    emitted = edges;
  end
  initial begin
    $readmemh("config.hex", cfg);
    $readmemh("bases.hex", base);
    $readmemh("counts.hex", count);
    $readmemh("acts.hex", am);
    $readmemh("weights.hex", wm);
    f = $fopen("out.txt", "w");
    @(negedge clk); rst = 0;
    for (i = 0; i < cfg[6]; i = i + 1) begin
      @(negedge clk); act_wr = 1; act_codes = am[6*i]; act_t = am[6*i+1]; act_b = am[6*i+2]; act_m = am[6*i+3]; act_e = am[6*i+4];
      act_s = am[6*i+5];
    end
    @(negedge clk); act_wr = 0; start = 1;
    @(negedge clk); start = 0; started = edges;
    while (busy) @(negedge clk);
    @(negedge clk); @(negedge clk);
    $fwrite(f, "clocks %0d\n", emitted - started);
    $fclose(f);
    $finish;
  end
endmodule
"#,
        aw = 16 * set.elems() - 1,
        bits = 8 * width,
        beat = 2 * set.decoded_bits(),
        queued = if n == 1 { 2 } else { 1 },
        name = set.name,
        alast = 6 * tokens_max * qblocks - 1,
        wlast = 2 * rows_max.div_ceil(2) * blocks + 2 * cores * blocks - 1,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let sim = rtl_sim::Simulator::build(dir.path(), &["column.v", "tb.v"]);
    let mut out = Vec::new();
    for (run_k, c) in runs.iter().enumerate() {
        let ArrayRun { words, rows: rows_n, tokens, turns, steady } = *c;
        let stripes = rows_n.div_ceil(CH * 2 * n);
        let raw: Vec<Vec<u8>> = (0..rows_n).map(|_| small_scaled_blocks(r, ty, qblocks)).collect();
        let rows: Vec<CodedRow> = raw.iter().map(|b| CodedRow::decode(ty, b).unwrap().unwrap()).collect();
        let acts: Vec<_> = (0..tokens)
            .map(|_| {
                let x: Vec<f32> = (0..ELEMS).map(|_| ((r.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * 2f32.powi((r.next() % 16) as i32 - 8)).collect();
                quantize_blocks(&x, words, qlen)
            })
            .collect();
        let first_row = |g: usize, s: usize| (CH * s + turns[g / n]) * 2 * n + 2 * (g % n);
        let mut act_hex = String::new();
        for (t, act) in acts.iter().enumerate() {
            for k in 0..qblocks {
                // A block's codes in its slot's lanes of the word: an octet's block k in lanes 32·(k mod 8) on.
                let lane0 = if octet { qlen * (k % 8) } else { 0 };
                for i in (0..set.elems()).rev() {
                    let code = if i >= lane0 && i < lane0 + qlen { act.codes[k * qlen + i - lane0] } else { 0 };
                    write!(act_hex, "{:04x}", code as i16 as u16).unwrap();
                }
                let (b, slot) = if octet { (k / 8, k % 8) } else { (k, 0) };
                writeln!(act_hex, "\n{t:02x}\n{b:04x}\n{:02x}\n{:03x}\n{slot:x}", act.scales[k].m, act.scales[k].e as i16 as u16 & 0xfff).unwrap();
            }
        }
        let block_hex = |k: usize, b: usize| -> String {
            if k >= rows_n {
                return "00".repeat(width);
            }
            let block: String = raw[k][b * bb..(b + 1) * bb].iter().rev().map(|byte| format!("{byte:02x}")).collect();
            format!("{}{block}", "00".repeat(width - bb))
        };
        let (mut w_hex, mut base, mut count, mut at) = (String::new(), Vec::new(), Vec::new(), 0usize);
        for g in 0..cores {
            base.push(at);
            for s in 0..stripes {
                let k = first_row(g, s);
                if k >= rows_n {
                    continue;
                }
                for b in 0..blocks {
                    writeln!(w_hex, "{}\n{}", block_hex(k, b), block_hex(k + 1, b)).unwrap();
                    at += 1;
                }
            }
            count.push(at - base[g]);
        }
        let turn_bits: usize = turns.iter().enumerate().map(|(ch, &t)| t << (2 * ch)).sum();
        let at_dir = dir.path().join(format!("run{run_k}"));
        std::fs::create_dir_all(&at_dir).unwrap();
        let words_hex = |v: &[usize]| v.iter().map(|c| format!("{c:x}\n")).collect::<String>();
        std::fs::write(at_dir.join("config.hex"), words_hex(&[stripes, tokens, (words == Words::Two) as usize, rows_n, turn_bits, steady as usize, tokens * qblocks])).unwrap();
        std::fs::write(at_dir.join("bases.hex"), words_hex(&base)).unwrap();
        std::fs::write(at_dir.join("counts.hex"), words_hex(&count)).unwrap();
        std::fs::write(at_dir.join("acts.hex"), act_hex).unwrap();
        std::fs::write(at_dir.join("weights.hex"), w_hex).unwrap();
        let run = sim.run(&at_dir, &[]);
        let what = format!("{} of {} {words:?}: {rows_n} rows in stripes of {n} pairs a channel, turns {turns:?}, {tokens} token-rows", ty.name(), set.signature());
        assert!(run.status.success(), "simulation ({what}): {}", String::from_utf8_lossy(&run.stderr));
        let text = std::fs::read_to_string(at_dir.join("out.txt")).unwrap();
        let (sums, clocks) = text.trim_end().rsplit_once('\n').map(|(s, c)| (s.to_string(), c)).unwrap_or((String::new(), text.trim_end()));
        let clocks: usize = clocks.strip_prefix("clocks ").expect("the run's clock count").parse().unwrap();
        let mut wrong = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for l in sums.lines() {
            let f: Vec<&str> = l.split(' ').collect();
            let g: usize = f[0].parse().unwrap();
            let (s, t0) = (usize::from_str_radix(f[1], 16).unwrap(), usize::from_str_radix(f[2], 16).unwrap());
            let k = first_row(g, s);
            assert!(k < rows_n && t0 < tokens, "{what}: core {g} emitted stripe {s} token-row {t0}, row {k}");
            assert!(!set.token_pairs || t0 % 2 == 0, "{what}: core {g} emitted a token pair from odd token-row {t0}");
            let mut got_rows = vec![(k, t0, f[3]), (k + 1, t0, f[4])];
            if set.token_pairs && t0 + 1 < tokens {
                got_rows.extend([(k, t0 + 1, f[5]), (k + 1, t0 + 1, f[6])]);
            }
            for (row, t, got) in got_rows {
                if row >= rows_n {
                    continue;
                }
                assert!(seen.insert((row, t)), "{what}: row {row} token-row {t} emitted twice");
                let got = u32::from_str_radix(got, 16).unwrap();
                let want = dot(&rows[row], &acts[t]).to_bits();
                if got != want {
                    wrong.push(format!("{what}: row {row} token-row {t}: rtl {got:08x}, dot {want:08x}"));
                }
            }
        }
        assert_eq!(seen.len(), rows_n * tokens, "{what}: every row and token-row emitted");
        out.push((wrong, clocks));
    }
    out
}

/// The column array of a set taking Q8_0 in octets multiplies a striped matrix as `numerics::dot` does, bit for bit: a
/// Q8_0 row's eight blocks an issue, each into the lane a clock as a block of its own, its sum's adds in block order —
/// one to eight token-rows (a sum's next block closer than the add is deep, and not), both word counts, stripes whole
/// and cut short, held and fed at random gaps — the set alone and beside a K-quant, whose jobs run as they did.
#[test]
fn the_column_array_takes_q8_0_in_octets_and_multiplies_as_dot_does() {
    use logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet;
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0x0c7e);
    let mut wrong = Vec::new();
    let alone = ColumnSet::new("q8x8", &[GgmlType::Q8_0], Words::Two).with_octets();
    let beside = ColumnSet::new("q8x8k", &[GgmlType::Q8_0, GgmlType::Q4K], Words::Two).with_octets();
    let run = |words, rows, tokens, turns| ArrayRun { words, rows, tokens, turns, steady: false };
    for (set, ty, n, runs) in [
        (&alone, GgmlType::Q8_0, 2usize, vec![run(Words::One, 38usize, 3usize, [2usize, 0, 3, 1]), run(Words::Two, 13, 1, [3, 2, 1, 0])]),
        (&alone, GgmlType::Q8_0, 1, vec![run(Words::One, 21, 8, [1, 3, 0, 2]), run(Words::Two, 9, 5, [0, 1, 2, 3])]),
        (&beside, GgmlType::Q8_0, 1, vec![run(Words::One, 17, 6, [2, 3, 1, 0])]),
        (&beside, GgmlType::Q4K, 1, vec![run(Words::Two, 19, 4, [1, 0, 3, 2])]),
    ] {
        wrong.extend(set_array_runs(set, ty, n, &runs, &mut r).into_iter().flat_map(|(w, _)| w));
    }
    assert!(wrong.is_empty(), "{} dots differ:\n{}", wrong.len(), wrong.iter().take(16).cloned().collect::<Vec<_>>().join("\n"));
}

/// The column array fed every core's beat whenever it is ready takes an octet job in exactly
/// `column_array_octet_clocks` from `start` to its last sums — its blocks a clock each, a sum's next block waiting on
/// the add when there are fewer token-rows than the add has stages, a step's issues on the buffer its blocks left —
/// every sum still `numerics::dot`'s: one to eight token-rows, both word counts.
#[test]
fn the_column_array_takes_an_octets_blocks_a_clock_each() {
    use logicaffeine_compile::spatial::decode_engine::rtl::column::{column_array_octet_clocks, column_passes, ColumnSet};
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0x0c7c);
    let mut wrong = Vec::new();
    let set = ColumnSet::new("q8x8", &[GgmlType::Q8_0], Words::Two).with_octets();
    let run = |words, rows, tokens, turns| ArrayRun { words, rows, tokens, turns, steady: true };
    for (n, runs) in [
        (2usize, vec![run(Words::One, 32usize, 6usize, [0usize, 1, 2, 3]), run(Words::Two, 37, 2, [2, 0, 3, 1])]),
        (1, vec![run(Words::One, 20, 1, [1, 0, 3, 2]), run(Words::One, 12, 8, [3, 1, 0, 2]), run(Words::Two, 19, 4, [0, 2, 1, 3])]),
    ] {
        for (c, (diffs, clocks)) in runs.iter().zip(set_array_runs(&set, GgmlType::Q8_0, n, &runs, &mut r)) {
            wrong.extend(diffs);
            let stripes = c.rows.div_ceil(8 * n);
            let passes = column_passes(GgmlType::Q8_0, c.words == Words::Two);
            assert_eq!(clocks, column_array_octet_clocks(stripes, 512 / 256, c.tokens, passes), "{:?}: {} rows in stripes of {n} pairs a channel, {} token-rows", c.words, c.rows, c.tokens);
        }
    }
    assert!(wrong.is_empty(), "{} dots differ:\n{}", wrong.len(), wrong.iter().take(16).cloned().collect::<Vec<_>>().join("\n"));
}

/// The column array of a set taking Q8_0 in octets on two and four lanes a row multiplies a striped matrix as
/// `numerics::dot` does, bit for bit: a token-row's next blocks of its octet's group into the row's full lane and its
/// lanes of one scale at once, their values added into the sum in block order through the chain of adds — one to eight
/// token-rows (a sum's next group closer than the chain is deep, and not), both word counts, stripes whole and cut
/// short, held and fed at random gaps — the set beside a K-quant too, whose jobs run as they did.
#[test]
fn the_column_array_takes_q8_0_in_octets_on_several_lanes_and_multiplies_as_dot_does() {
    use logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet;
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0x0c71);
    let mut wrong = Vec::new();
    let run = |words, rows, tokens, turns| ArrayRun { words, rows, tokens, turns, steady: false };
    for lanes in [2usize, 4] {
        let alone = ColumnSet::new(&format!("q8x8l{lanes}"), &[GgmlType::Q8_0], Words::Two).with_octets().with_octet_lanes(lanes);
        let beside = ColumnSet::new(&format!("q8x8l{lanes}k"), &[GgmlType::Q8_0, GgmlType::Q4K], Words::Two).with_octets().with_octet_lanes(lanes);
        for (set, ty, n, runs) in [
            (&alone, GgmlType::Q8_0, 2usize, vec![run(Words::One, 38usize, 3usize, [2usize, 0, 3, 1]), run(Words::Two, 13, 1, [3, 2, 1, 0])]),
            (&alone, GgmlType::Q8_0, 1, vec![run(Words::One, 21, 8, [1, 3, 0, 2]), run(Words::Two, 9, 5, [0, 1, 2, 3])]),
            (&beside, GgmlType::Q8_0, 1, vec![run(Words::One, 17, 6, [2, 3, 1, 0])]),
            (&beside, GgmlType::Q4K, 1, vec![run(Words::Two, 19, 4, [1, 0, 3, 2])]),
        ] {
            wrong.extend(set_array_runs(set, ty, n, &runs, &mut r).into_iter().flat_map(|(w, _)| w.into_iter().map(move |d| format!("{lanes} lanes: {d}"))));
        }
    }
    assert!(wrong.is_empty(), "{} dots differ:\n{}", wrong.len(), wrong.iter().take(16).cloned().collect::<Vec<_>>().join("\n"));
}

/// The column array of a set taking Q8_0 in octets on several lanes a row, fed every core's beat whenever it is ready,
/// takes an octet job in exactly `column_array_octet_lane_clocks` from `start` to its last sums — a group of a
/// token-row's blocks a clock, a sum's next group waiting on the chain when there are fewer token-rows than the chain
/// has stages — every sum still `numerics::dot`'s: two and four lanes, one to sixteen token-rows, both word counts.
/// Two cores' passes on one set of processing elements in a clock of twice theirs (`logos_matvec_pass_x2`), the clocks'
/// rising edges together, under iverilog beside two `logos_matvec_pass_p`s taking the same word through the column's
/// stagger: seeded codes, words, unit sums and digit every clock, each core's partials every clock the reference
/// pass's, bit for bit, at one, two and sixteen units.
#[test]
fn two_cores_passes_on_elements_at_twice_their_clock_are_each_cores_own() {
    use logicaffeine_compile::spatial::decode_engine::rtl::column::{columns_verilog, matvec_pass_x2_verilog};
    if !common::require_tool("iverilog") {
        return;
    }
    for units in [1usize, 2, 16] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pass.v"), columns_verilog(&[]) + &matvec_pass_x2_verilog()).unwrap();
        let tb = format!(
            r#"module tb;
  localparam UNITS = {units}, N = 3000;
  reg clk = 0, clk2 = 0;
  initial begin #5 clk = 1; forever #10 clk = ~clk; end
  always #5 clk2 = ~clk2;
  reg [128*UNITS-1:0] c1a, c2a, c1b, c2b, w;
  reg [12*UNITS-1:0] s;
  reg digit;
  wire [128*UNITS-1:0] st;
  logos_column_act_stagger #(.ELEMS(16*UNITS)) sg(.clk(clk), .codes(w), .st(st));
  wire [24*UNITS-1:0] ra1, ra2, rb1, rb2, xa1, xa2, xb1, xb2;
  logos_matvec_pass_p #(.UNITS(UNITS)) refa(.clk(clk), .code1(c1a), .code2(c2a), .w(st), .s(s), .digit(digit), .p1(ra1), .p2(ra2));
  logos_matvec_pass_p #(.UNITS(UNITS)) refb(.clk(clk), .code1(c1b), .code2(c2b), .w(st), .s(s), .digit(digit), .p1(rb1), .p2(rb2));
  logos_matvec_pass_x2 #(.UNITS(UNITS)) pair(.clk(clk), .clk2(clk2), .code1a(c1a), .code2a(c2a), .code1b(c1b), .code2b(c2b),
    .w(w), .s(s), .digit(digit), .p1a(xa1), .p2a(xa2), .p1b(xb1), .p2b(xb2));
  integer i, j, bad = 0, seen = 0, moved = 0;
  reg [24*UNITS-1:0] last;
  task draw; begin
    for (j = 0; j < 4*UNITS; j = j + 1) begin
      c1a[32*j +: 32] = $random; c2a[32*j +: 32] = $random; c1b[32*j +: 32] = $random; c2b[32*j +: 32] = $random;
      w[32*j +: 32] = $random;
    end
    for (j = 0; j < UNITS; j = j + 1) s[12*j +: 12] = $random;
    digit = $random;
  end endtask
  initial begin
    draw;
    for (i = 0; i < N; i = i + 1) begin
      @(posedge clk); #2;
      if (i > 12) begin
        seen = seen + 1;
        if (xa1 !== ra1 || xa2 !== ra2 || xb1 !== rb1 || xb2 !== rb2) begin
          bad = bad + 1;
          if (bad < 6) $display("clock %0d: a %h %h, want %h %h; b %h %h, want %h %h", i, xa1, xa2, ra1, ra2, xb1, xb2, rb1, rb2);
        end
        if (xa1 !== last) moved = moved + 1;
      end
      last = xa1;
      draw;
    end
    $display("CLOCKS %0d BAD %0d MOVED %0d", seen, bad, moved);
    $finish;
  end
endmodule
"#
        );
        std::fs::write(dir.path().join("tb.v"), tb).unwrap();
        let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "pass.v"]).output().expect("iverilog");
        assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
        let run = Command::new("vvp").current_dir(dir.path()).args(["-n", "sim"]).output().expect("vvp");
        let out = String::from_utf8_lossy(&run.stdout).to_string();
        let moved: usize = out.split("MOVED ").nth(1).and_then(|m| m.split_whitespace().next()).and_then(|n| n.parse().ok()).unwrap_or(0);
        assert!(out.contains("CLOCKS 2987 BAD 0 ") && moved > 2900, "{units} units: {out}");
    }
}

#[test]
fn the_column_array_takes_an_octets_groups_a_clock_each_on_several_lanes() {
    use logicaffeine_compile::spatial::decode_engine::rtl::column::{column_array_octet_lane_clocks, column_passes, ColumnSet};
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0x0c72);
    let mut wrong = Vec::new();
    let run = |words, rows, tokens, turns| ArrayRun { words, rows, tokens, turns, steady: true };
    for lanes in [2usize, 4] {
        let set = ColumnSet::new(&format!("q8x8c{lanes}"), &[GgmlType::Q8_0], Words::Two).with_octets().with_octet_lanes(lanes);
        for (n, runs) in [
            (2usize, vec![run(Words::One, 32usize, 6usize, [0usize, 1, 2, 3]), run(Words::Two, 37, 2, [2, 0, 3, 1])]),
            (1, vec![run(Words::One, 20, 1, [1, 0, 3, 2]), run(Words::One, 12, 16, [3, 1, 0, 2]), run(Words::Two, 19, 4, [0, 2, 1, 3])]),
        ] {
            for (c, (diffs, clocks)) in runs.iter().zip(set_array_runs(&set, GgmlType::Q8_0, n, &runs, &mut r)) {
                wrong.extend(diffs);
                let stripes = c.rows.div_ceil(8 * n);
                let passes = column_passes(GgmlType::Q8_0, c.words == Words::Two);
                assert_eq!(
                    clocks,
                    column_array_octet_lane_clocks(stripes, 512 / 256, c.tokens, passes, lanes),
                    "{lanes} lanes, {:?}: {} rows in stripes of {n} pairs a channel, {} token-rows",
                    c.words,
                    c.rows,
                    c.tokens
                );
            }
        }
    }
    assert!(wrong.is_empty(), "{} dots differ:\n{}", wrong.len(), wrong.iter().take(16).cloned().collect::<Vec<_>>().join("\n"));
}

/// The column array with its cores paired on processing elements in a clock of twice theirs
/// ([`ColumnSet::with_pe_pairs`]), under the RTL simulator, its cores' streams at random gaps and an eighth of its clocks
/// held: every row's every token-row `numerics::dot`'s, bit for bit, the pairs within a channel (two and four pairs a
/// channel) and across two (one): a K-quant of one code digit and of two in both word counts, a legacy format of
/// 32-element blocks, and Q8_0 in octets on four lanes beside a K-quant.
#[test]
fn the_column_array_of_paired_cores_multiplies_as_dot_does() {
    use logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet;
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0xa7a2);
    let mut wrong = Vec::new();
    let run = |words, rows, tokens, turns| ArrayRun { words, rows, tokens, turns, steady: false };
    let k = ColumnSet::new("pairk", &[GgmlType::Q4K, GgmlType::Q6K, GgmlType::Q4_0], Words::Two).with_pe_pairs();
    let oct = ColumnSet::new("pairq8", &[GgmlType::Q8_0, GgmlType::Q4K], Words::Two).with_octets().with_octet_lanes(4).with_pe_pairs();
    for (set, ty, n, runs) in [
        (&k, GgmlType::Q4K, 2usize, vec![run(Words::One, 38usize, 3usize, [2usize, 0, 3, 1]), run(Words::Two, 21, 2, [0, 1, 2, 3])]),
        (&k, GgmlType::Q6K, 4, vec![run(Words::Two, 37, 5, [1, 3, 0, 2])]),
        (&k, GgmlType::Q4_0, 1, vec![run(Words::One, 13, 4, [3, 2, 1, 0])]),
        (&oct, GgmlType::Q8_0, 2, vec![run(Words::One, 38, 3, [2, 0, 3, 1]), run(Words::Two, 13, 1, [3, 2, 1, 0])]),
        (&oct, GgmlType::Q4K, 1, vec![run(Words::Two, 19, 4, [1, 0, 3, 2])]),
    ] {
        wrong.extend(set_array_runs(set, ty, n, &runs, &mut r).into_iter().flat_map(|(w, _)| w));
    }
    assert!(wrong.is_empty(), "{} dots differ:\n{}", wrong.len(), wrong.iter().take(16).cloned().collect::<Vec<_>>().join("\n"));
}

/// The column array of paired cores takes the clocks the array of lone cores does, `column_array_clocks` and
/// `column_array_octet_lane_clocks` unchanged: each core still runs a pass a clock, its pair's elements taking both cores'
/// passes in that clock.
#[test]
fn the_column_array_of_paired_cores_runs_a_pass_a_clock() {
    use logicaffeine_compile::spatial::decode_engine::rtl::column::{column_array_clocks, column_array_octet_lane_clocks, column_passes, ColumnSet};
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0xa7a3);
    let mut wrong = Vec::new();
    let run = |words, rows, tokens, turns| ArrayRun { words, rows, tokens, turns, steady: true };
    let k = ColumnSet::new("pairck", &[GgmlType::Q4K, GgmlType::Q6K], Words::Two).with_pe_pairs();
    for (ty, n, runs) in [
        (GgmlType::Q4K, 2usize, vec![run(Words::One, 32usize, 6usize, [0usize, 1, 2, 3]), run(Words::Two, 37, 2, [2, 0, 3, 1])]),
        (GgmlType::Q6K, 1, vec![run(Words::One, 12, 1, [3, 1, 0, 2]), run(Words::Two, 19, 3, [0, 2, 1, 3])]),
    ] {
        for (c, (diffs, clocks)) in runs.iter().zip(set_array_runs(&k, ty, n, &runs, &mut r)) {
            wrong.extend(diffs);
            let (stripes, blocks) = (c.rows.div_ceil(8 * n), 512 / ty.block_len());
            let passes = column_passes(ty, c.words == Words::Two);
            assert_eq!(clocks, column_array_clocks(stripes, blocks, c.tokens, passes), "{} {:?}: {} rows, {n} pairs a channel, {} token-rows", ty.name(), c.words, c.rows, c.tokens);
        }
    }
    let oct = ColumnSet::new("pairc8", &[GgmlType::Q8_0], Words::Two).with_octets().with_octet_lanes(4).with_pe_pairs();
    let runs = vec![run(Words::One, 32, 6, [0, 1, 2, 3]), run(Words::Two, 37, 2, [2, 0, 3, 1])];
    for (c, (diffs, clocks)) in runs.iter().zip(set_array_runs(&oct, GgmlType::Q8_0, 2, &runs, &mut r)) {
        wrong.extend(diffs);
        let passes = column_passes(GgmlType::Q8_0, c.words == Words::Two);
        assert_eq!(clocks, column_array_octet_lane_clocks(c.rows.div_ceil(16), 512 / 256, c.tokens, passes, 4), "octets {:?}: {} rows, {} token-rows", c.words, c.rows, c.tokens);
    }
    assert!(wrong.is_empty(), "{} dots differ:\n{}", wrong.len(), wrong.iter().take(16).cloned().collect::<Vec<_>>().join("\n"));
}

/// The column array of a set of token pairs multiplies a striped matrix as `numerics::dot` does, bit for bit: each issue
/// runs the beat held against a token pair, the second token-row through elements and lanes of its own, its sums beside
/// the first's — one to nine token-rows (an odd count's last pair one short), both word counts, formats of one code digit
/// and of two, one to three cores a channel, stripes whole and cut short, held and fed at random gaps — and with octets
/// a Q8_0 job's slots a token pair's, emitted a pair a clock, the set's K-quant beside them running as it does; with paired
/// processing elements each core's two words on one set of its elements in clk2, and with octet lanes past the first
/// each token-row's next blocks on lanes of its own.
#[test]
fn the_column_array_of_token_pairs_multiplies_as_dot_does() {
    use logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet;
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0x7a12);
    let mut wrong = Vec::new();
    let run = |words, rows, tokens, turns| ArrayRun { words, rows, tokens, turns, steady: false };
    let k = ColumnSet::new("tpk", &[GgmlType::Q4K, GgmlType::Q6K, GgmlType::Q4_0], Words::Two).with_token_pairs();
    let oct = ColumnSet::new("tpq8", &[GgmlType::Q8_0, GgmlType::Q4K], Words::Two).with_octets().with_token_pairs();
    let k2 = ColumnSet::new("tpk2", &[GgmlType::Q4K, GgmlType::Q6K], Words::Two).with_token_pairs().with_pe_pairs();
    let oct2 = ColumnSet::new("tpq82", &[GgmlType::Q8_0, GgmlType::Q4K], Words::Two).with_octets().with_token_pairs().with_pe_pairs();
    let oct4 = ColumnSet::new("tpq84", &[GgmlType::Q8_0, GgmlType::Q4K], Words::Two).with_octets().with_octet_lanes(4).with_token_pairs();
    let oct22 = ColumnSet::new("tpq822", &[GgmlType::Q8_0], Words::Two).with_octets().with_octet_lanes(2).with_token_pairs().with_pe_pairs();
    for (set, ty, n, runs) in [
        (&k, GgmlType::Q4K, 2usize, vec![run(Words::One, 38usize, 3usize, [2usize, 0, 3, 1]), run(Words::Two, 21, 2, [0, 1, 2, 3])]),
        (&k, GgmlType::Q6K, 3, vec![run(Words::Two, 37, 5, [1, 3, 0, 2]), run(Words::One, 29, 8, [0, 2, 1, 3])]),
        (&k, GgmlType::Q4_0, 1, vec![run(Words::One, 13, 9, [3, 2, 1, 0]), run(Words::Two, 7, 1, [2, 3, 0, 1])]),
        (&oct, GgmlType::Q8_0, 2, vec![run(Words::One, 38, 3, [2, 0, 3, 1]), run(Words::Two, 13, 1, [3, 2, 1, 0])]),
        (&oct, GgmlType::Q8_0, 1, vec![run(Words::One, 21, 8, [1, 3, 0, 2]), run(Words::Two, 9, 7, [0, 1, 2, 3])]),
        (&oct, GgmlType::Q4K, 3, vec![run(Words::Two, 19, 4, [1, 0, 3, 2]), run(Words::One, 26, 9, [3, 1, 2, 0])]),
        (&k2, GgmlType::Q4K, 3, vec![run(Words::One, 37, 5, [2, 0, 3, 1]), run(Words::Two, 24, 2, [1, 3, 0, 2])]),
        (&k2, GgmlType::Q6K, 1, vec![run(Words::Two, 13, 9, [0, 1, 2, 3])]),
        (&oct2, GgmlType::Q8_0, 2, vec![run(Words::One, 38, 7, [3, 2, 1, 0]), run(Words::Two, 17, 4, [2, 3, 0, 1])]),
        (&oct4, GgmlType::Q8_0, 2, vec![run(Words::One, 38, 7, [1, 0, 3, 2]), run(Words::Two, 13, 4, [0, 3, 2, 1])]),
        (&oct4, GgmlType::Q4K, 1, vec![run(Words::Two, 19, 5, [2, 1, 0, 3])]),
        (&oct22, GgmlType::Q8_0, 3, vec![run(Words::One, 26, 9, [3, 0, 1, 2]), run(Words::Two, 11, 2, [1, 2, 3, 0])]),
    ] {
        wrong.extend(set_array_runs(set, ty, n, &runs, &mut r).into_iter().flat_map(|(w, _)| w));
    }
    assert!(wrong.is_empty(), "{} dots differ:\n{}", wrong.len(), wrong.iter().take(16).cloned().collect::<Vec<_>>().join("\n"));
}

/// The column array of a set of token pairs, fed every core's beat whenever it is ready, takes exactly the clocks of an
/// array of one token-row an issue running ⌈tokens / 2⌉ token-rows (`ColumnSet::issues`): `column_array_clocks`, and with
/// octets `column_array_octet_clocks`, a pass a clock an issue — every sum still `numerics::dot`'s, for odd and even
/// counts, both word counts and formats of one and two code digits, its elements in the cores' clock and in clk2, and
/// `column_array_octet_lane_clocks` on four octet lanes.
#[test]
fn the_column_array_of_token_pairs_runs_a_pair_a_clock() {
    use logicaffeine_compile::spatial::decode_engine::rtl::column::{column_array_clocks, column_array_octet_clocks, column_array_octet_lane_clocks, column_passes, ColumnSet};
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0x7a13);
    let mut wrong = Vec::new();
    let run = |words, rows, tokens, turns| ArrayRun { words, rows, tokens, turns, steady: true };
    let k = ColumnSet::new("tpck", &[GgmlType::Q4K, GgmlType::Q6K], Words::Two).with_token_pairs();
    for (ty, n, runs) in [
        (GgmlType::Q4K, 2usize, vec![run(Words::One, 32usize, 6usize, [0usize, 1, 2, 3]), run(Words::Two, 37, 3, [2, 0, 3, 1]), run(Words::One, 24, 16, [1, 2, 3, 0])]),
        (GgmlType::Q6K, 1, vec![run(Words::One, 12, 1, [3, 1, 0, 2]), run(Words::Two, 19, 9, [0, 2, 1, 3])]),
    ] {
        for (c, (diffs, clocks)) in runs.iter().zip(set_array_runs(&k, ty, n, &runs, &mut r)) {
            wrong.extend(diffs);
            let (stripes, blocks) = (c.rows.div_ceil(8 * n), 512 / ty.block_len());
            let passes = column_passes(ty, c.words == Words::Two);
            assert_eq!(clocks, column_array_clocks(stripes, blocks, k.issues(c.tokens), passes), "{} {:?}: {} rows, {n} pairs a channel, {} token-rows", ty.name(), c.words, c.rows, c.tokens);
        }
    }
    let k2 = ColumnSet::new("tpck2", &[GgmlType::Q4K, GgmlType::Q6K], Words::Two).with_token_pairs().with_pe_pairs();
    let runs = vec![run(Words::One, 37, 7, [1, 0, 3, 2]), run(Words::Two, 19, 3, [0, 2, 1, 3])];
    for (c, (diffs, clocks)) in runs.iter().zip(set_array_runs(&k2, GgmlType::Q6K, 1, &runs, &mut r)) {
        wrong.extend(diffs);
        let passes = column_passes(GgmlType::Q6K, c.words == Words::Two);
        assert_eq!(clocks, column_array_clocks(c.rows.div_ceil(8), 512 / 256, k2.issues(c.tokens), passes), "q6_K in clk2 {:?}: {} rows, {} token-rows", c.words, c.rows, c.tokens);
    }
    let oct4 = ColumnSet::new("tpc84", &[GgmlType::Q8_0], Words::Two).with_octets().with_octet_lanes(4).with_token_pairs();
    let runs = vec![run(Words::One, 32, 9, [0, 1, 2, 3]), run(Words::Two, 21, 16, [2, 0, 3, 1])];
    for (c, (diffs, clocks)) in runs.iter().zip(set_array_runs(&oct4, GgmlType::Q8_0, 2, &runs, &mut r)) {
        wrong.extend(diffs);
        let passes = column_passes(GgmlType::Q8_0, c.words == Words::Two);
        assert_eq!(clocks, column_array_octet_lane_clocks(c.rows.div_ceil(16), 512 / 256, oct4.issues(c.tokens), passes, 4), "octets on four lanes {:?}: {} rows, {} token-rows", c.words, c.rows, c.tokens);
    }
    let oct = ColumnSet::new("tpc8", &[GgmlType::Q8_0], Words::Two).with_octets().with_token_pairs();
    let runs = vec![run(Words::One, 32, 6, [0, 1, 2, 3]), run(Words::Two, 37, 3, [2, 0, 3, 1]), run(Words::One, 20, 16, [3, 2, 1, 0])];
    for (c, (diffs, clocks)) in runs.iter().zip(set_array_runs(&oct, GgmlType::Q8_0, 2, &runs, &mut r)) {
        wrong.extend(diffs);
        let passes = column_passes(GgmlType::Q8_0, c.words == Words::Two);
        assert_eq!(clocks, column_array_octet_clocks(c.rows.div_ceil(16), 512 / 256, oct.issues(c.tokens), passes), "octets {:?}: {} rows, {} token-rows", c.words, c.rows, c.tokens);
    }
    assert!(wrong.is_empty(), "{} dots differ:\n{}", wrong.len(), wrong.iter().take(16).cloned().collect::<Vec<_>>().join("\n"));
}

/// The column array fed every core's beat whenever it is ready takes exactly `column_array_clocks` from `start` to its
/// last sums — a pass a clock across token-rows, blocks and stripes once a step has more passes than the add has stages,
/// a core without a pair in a stripe never holding the others back — every sum still `numerics::dot`'s: one to four
/// passes an issue, a format of one code digit and of two in both word counts.
#[test]
fn the_column_array_runs_a_pass_a_clock() {
    use logicaffeine_compile::spatial::decode_engine::rtl::column::{column_array_clocks, column_passes};
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0xa1c0);
    let mut wrong = Vec::new();
    let fmt = |ty| DECODERS.iter().find(|d| d.ty == ty).unwrap();
    let builds = [
        (fmt(GgmlType::Q4K), 2usize, vec![(Words::One, 32usize, 6usize, [0usize, 1, 2, 3]), (Words::Two, 37, 2, [2, 0, 3, 1])]),
        (fmt(GgmlType::Q4K), 1, vec![(Words::One, 20, 5, [1, 0, 3, 2])]),
        (fmt(GgmlType::Q6K), 1, vec![(Words::One, 12, 1, [3, 1, 0, 2]), (Words::Two, 19, 3, [0, 2, 1, 3])]),
    ];
    for (d, n, shapes) in builds {
        let runs: Vec<ArrayRun> = shapes.iter().map(|&(words, rows, tokens, turns)| ArrayRun { words, rows, tokens, turns, steady: true }).collect();
        for (c, (diffs, clocks)) in runs.iter().zip(column_array_runs(d, n, &runs, &mut r)) {
            wrong.extend(diffs);
            let (stripes, blocks) = (c.rows.div_ceil(8 * n), 512 / d.ty.block_len());
            let passes = column_passes(d.ty, c.words == Words::Two);
            assert_eq!(clocks, column_array_clocks(stripes, blocks, c.tokens, passes), "{} {:?}: {} rows in stripes of {n} pairs a channel, {} token-rows", d.ty.name(), c.words, c.rows, c.tokens);
        }
    }
    assert!(wrong.is_empty(), "{} dots differ:\n{}", wrong.len(), wrong.iter().take(16).cloned().collect::<Vec<_>>().join("\n"));
}

/// One matrix held in memory: a weight source of its one tensor.
struct HeldMatrix {
    ty: GgmlType,
    dims: Vec<u64>,
    bytes: Vec<u8>,
}

impl logicaffeine_compile::model::reference::WeightSource for HeldMatrix {
    fn describe(&self, _: &str) -> std::io::Result<(GgmlType, Vec<u64>)> {
        Ok((self.ty, self.dims.clone()))
    }
    fn row_bytes(&mut self, _: &str, first: usize, count: usize) -> std::io::Result<Vec<u8>> {
        let row = self.bytes.len() / self.dims[1] as usize;
        Ok(self.bytes[first * row..(first + count) * row].to_vec())
    }
    fn has(&self, name: &str) -> bool {
        name == "w"
    }
}

/// A matrix of `rows` rows of `elems` elements in `ty`, streamed by a matvec job, laid in stripes of `n` pairs a channel
/// over the card's four channels at 64-byte lines: its rows' bytes, its image, and each channel's beats of its tiles as
/// the image stream engine sends them (`rtl::stream::beats`).
fn striped_matrix(ty: GgmlType, elems: usize, rows: usize, n: u64, r: &mut Rng) -> (Vec<Vec<u8>>, logicaffeine_compile::spatial::decode_engine::image::Image, Vec<Vec<logicaffeine_compile::spatial::decode_engine::rtl::stream::Beat>>) {
    striped_matrix_in(ty, elems, rows, n, 1, r)
}

/// [`striped_matrix`] with every block in a slot of a multiple of `slot` bytes (`ImageLayout::slot`).
fn striped_matrix_in(ty: GgmlType, elems: usize, rows: usize, n: u64, slot: u64, r: &mut Rng) -> (Vec<Vec<u8>>, logicaffeine_compile::spatial::decode_engine::image::Image, Vec<Vec<logicaffeine_compile::spatial::decode_engine::rtl::stream::Beat>>) {
    use logicaffeine_compile::spatial::d5005::Beat as LineBeat;
    use logicaffeine_compile::spatial::decode_engine::image::{weight_image, ImageLayout, TensorTable};
    use logicaffeine_compile::spatial::decode_engine::rtl::stream::{beats, descriptors};
    let blocks = elems / ty.block_len();
    let raw: Vec<Vec<u8>> = (0..rows).map(|_| small_scaled_blocks(r, ty, blocks)).collect();
    let mut table = TensorTable::from_rows(0, vec![("w".to_string(), vec![elems as u64, rows as u64], ty.id())]).unwrap();
    table.tensors[0].streamed = true;
    let layout = ImageLayout { stripe: Some(n), slot, ..ImageLayout::d5005(LineBeat::Data64) };
    let image = weight_image(&table, &[0], &layout).unwrap();
    let mut source = HeldMatrix { ty, dims: vec![elems as u64, rows as u64], bytes: raw.concat() };
    let channels = image.fill(&mut source).unwrap();
    let streams = (0..4u32).map(|ch| descriptors(&image, ch).iter().flat_map(|d| beats(&channels[ch as usize], d, &layout)).collect()).collect();
    (raw, image, streams)
}

/// A channel's beats as testbench words: each beat's 64 bytes (byte 0 lowest), its kept bytes, its tile's first row and
/// its offset in the tile.
fn beat_hex(stream: &[logicaffeine_compile::spatial::decode_engine::rtl::stream::Beat], out: &mut String) {
    for beat in stream {
        let data: String = beat.data.iter().rev().map(|b| format!("{b:02x}")).collect();
        writeln!(out, "{data}\n{:x}\n{:x}\n{:x}", beat.keep, beat.row, beat.offset).unwrap();
    }
}

/// The beat `logos_column_decode_<set>` gives for two blocks of `ty`, as hex, most significant digit first: each
/// block's codes, its units' multipliers and offset terms as bytes, its scales and its units' sums of codes, at the
/// set's fields (`ColumnSet::decoded_fields`), past a narrower format's units zero.
fn decoded_beat_hex(set: &logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet, ty: GgmlType, blk1: &[u8], blk2: &[u8]) -> String {
    let (db, f) = (set.decoded_bits(), set.decoded_fields());
    let mut bits = vec![false; 2 * db];
    let mut put = |at: usize, width: usize, value: u64| (0..width).for_each(|i| bits[at + i] = value >> i & 1 == 1);
    for (k, blk) in [blk1, blk2].into_iter().enumerate() {
        let base = k * db;
        let u = units(ty, blk);
        for (i, &code) in u.code.iter().enumerate() {
            put(base + f.code + 8 * i, 8, u64::from(code));
        }
        for (j, (&c, &n)) in u.c.iter().zip(&u.n).enumerate() {
            put(base + f.c + 8 * j, 8, u64::from(c as u8));
            put(base + f.n + 8 * j, 8, u64::from(n as u8));
            put(base + f.q + 12 * j, 12, u.code[UNIT * j..UNIT * (j + 1)].iter().map(|&q| u64::from(q)).sum());
        }
        put(base + f.a, 16, u64::from(u.a));
        put(base + f.b, 16, u64::from(u.b));
    }
    (0..bits.len().div_ceil(4))
        .rev()
        .map(|d| {
            let nibble = (0..4).fold(0u32, |v, b| v | u32::from(bits.get(4 * d + b).copied().unwrap_or(false)) << b);
            char::from_digit(nibble, 16).unwrap()
        })
        .collect()
}

/// The modules yosys leaves of `top`, one module of `verilog`, at its parameters `params`: the modules of `keep`
/// elaborated and put through `passes`, every other module a black box of its ports.
fn yosys_modules(verilog: &str, top: &str, keep: &[&str], params: &[(&str, usize)], passes: &str) -> serde_json::Value {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("d.v"), common::stubbed(verilog, keep)).unwrap();
    let set: String = params.iter().map(|(k, v)| format!(" -set {k} {v}")).collect();
    let chparam = if set.is_empty() { String::new() } else { format!("chparam{set} {top}; ") };
    let script = format!("read_verilog -sv d.v; {chparam}hierarchy -top {top}; {passes}; write_json s.json");
    let o = Command::new("yosys").current_dir(dir.path()).args(["-q", "-p", &script]).output().expect("yosys");
    assert!(o.status.success(), "yosys on {top} {params:?}: {}", String::from_utf8_lossy(&o.stderr));
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("s.json")).unwrap()).unwrap();
    json["modules"].clone()
}

/// The module of `modules` yosys elaborated as the top.
fn top_of(modules: &serde_json::Value) -> &serde_json::Value {
    modules.as_object().unwrap().values().find(|m| m["attributes"].get("top").is_some()).unwrap()
}

/// A yosys number parameter, as its JSON gives it: a string of binary digits or a number.
fn yosys_number(v: &serde_json::Value) -> usize {
    match v {
        serde_json::Value::String(s) => usize::from_str_radix(s, 2).unwrap(),
        serde_json::Value::Null => 1,
        v => v.as_u64().unwrap() as usize,
    }
}

/// The places `top`'s queue `q` holds, elaborated by yosys at `params` as it is declared (no port sharing, which pads a
/// memory read at constant addresses to a power of two): its memory's words, or the registers the frontend made of it.
fn queue_places(verilog: &str, top: &str, params: &[(&str, usize)]) -> usize {
    let modules = yosys_modules(verilog, top, &[top], params, "proc; memory_collect");
    let module = top_of(&modules);
    let words: usize = module["cells"]
        .as_object()
        .unwrap()
        .values()
        .filter(|c| c["type"] == "$mem_v2" && c["parameters"]["MEMID"] == "\\q")
        .map(|c| yosys_number(&c["parameters"]["SIZE"]))
        .sum();
    let registers = module["netnames"]
        .as_object()
        .unwrap()
        .keys()
        .filter(|k| k.strip_prefix("q[").and_then(|r| r.strip_suffix(']')).is_some_and(|i| i.bytes().all(|b| b.is_ascii_digit())))
        .count();
    words + registers
}

/// `module` of `verilog` at its parameters `params` as yosys leaves it after `proc` and `opt`: its own cells, every
/// module below it a black box of its ports.
fn module_json(verilog: &str, module: &str, params: &[(&str, usize)]) -> serde_json::Value {
    top_of(&yosys_modules(verilog, module, &[module], params, "proc; opt")).clone()
}

/// The bits `signal` of `module` (as [`module_json`] gives it) enables or selects: every cell input bit it drives, a
/// word-level cell's select or enable counted once for each bit of the word it chooses (a register's enable holds that
/// many bits, a multiplexer's select steers that many).
fn enabled_bits(module: &serde_json::Value, signal: &str) -> usize {
    let bit = module["ports"][signal]["bits"][0].clone();
    let mut bits = 0;
    for c in module["cells"].as_object().unwrap().values() {
        for (port, conns) in c["connections"].as_object().unwrap() {
            if c["port_directions"][port] != "input" {
                continue;
            }
            let steers = matches!(port.as_str(), "S" | "EN" | "SRST" | "ARST" | "CLR" | "SET") && !c["type"].as_str().unwrap().starts_with("$mem");
            let each = if steers { yosys_number(&c["parameters"]["WIDTH"]) } else { 1 };
            bits += conns.as_array().unwrap().iter().filter(|b| **b == bit).count() * each;
        }
    }
    bits
}

/// The most cell inputs any bit of `port` of `module` (as [`module_json`] gives it) reaches, a module below it counted
/// once for each of its inputs the bit reaches.
fn port_sinks(module: &serde_json::Value, port: &str) -> usize {
    let bits = module["ports"][port]["bits"].as_array().unwrap_or_else(|| panic!("a port {port}")).clone();
    let reaches = |bit: &serde_json::Value| -> usize {
        let mut n = 0;
        for c in module["cells"].as_object().unwrap().values() {
            for (p, conns) in c["connections"].as_object().unwrap() {
                if c["port_directions"][p.as_str()] == "input" {
                    n += conns.as_array().unwrap().iter().filter(|b| *b == bit).count();
                }
            }
        }
        n
    };
    bits.iter().map(reaches).max().unwrap()
}

/// Whether `module`'s copy of the job's format (`fmt_q`, as [`module_json`] gives the module) is marked to be kept apart
/// from every other register like it (`dont_merge`), which a synthesis would otherwise merge into one register again.
fn keeps_its_copy(module: &serde_json::Value) -> bool {
    module["netnames"]["fmt_q"]["attributes"].get("dont_merge").is_some()
}

/// A job's format reaches every module of the matvec through a copy of the module's own: the streaming unit, the
/// array, each channel's feed and each core register the job's format a clock behind it (a job holds its format from its
/// start to its end, none reads it in the job's first clock) and read the copy alone, handing the format itself only to
/// the modules below, which copy it themselves. So a bit of the job's format reaches the unit's copy, its array and its
/// four feeds, an array's copy and its cores, and a feed's or a core's copy alone — never a select of their datapaths,
/// 13,258 of which one bit of it reached in hybrid7b's fit — and each copy is its module's own, kept apart from the
/// others (`dont_merge`), which a synthesis would otherwise merge back into the one register.
#[test]
fn a_jobs_format_reaches_each_module_of_the_matvec_through_a_copy_of_its_own() {
    use logicaffeine_compile::spatial::d5005::Beat as LineBeat;
    use logicaffeine_compile::spatial::decode_engine::image::ImageLayout;
    use logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet;
    use logicaffeine_compile::spatial::decode_engine::rtl::streaming::streaming_verilog;
    if !common::require_tool("yosys") {
        return;
    }
    let set = ColumnSet::hybrid(&[GgmlType::Q8_0, GgmlType::Q4K, GgmlType::Q5K, GgmlType::Q6K], Words::One).with_octets().with_slots(16);
    let v = streaming_verilog(std::slice::from_ref(&set), &ImageLayout { slot: 16, ..ImageLayout::d5005(LineBeat::Data64) });
    let name = &set.name;
    for n in [1usize, 3] {
        for (module, params, sinks) in [
            (format!("logos_column_feed_{name}"), vec![("N", n)], 1),
            (format!("logos_column_array_{name}"), vec![("CH", 4), ("N", n)], 1 + 4 * n),
            (format!("logos_streaming_matvec_{name}"), vec![("N", n)], 6),
            (format!("logos_column_core_{name}"), vec![("RING", n.min(2) + 1)], 1),
        ] {
            let m = module_json(&v, &module, &params);
            assert_eq!(port_sinks(&m, "fmt"), sinks, "{module} {params:?}: a bit of the job's format reaches {} inputs", port_sinks(&m, "fmt"));
            assert!(keeps_its_copy(&m), "{module} {params:?} keeps its copy of the job's format apart");
        }
    }
}

/// Each channel's cores read the activations from a copy of their own: the array keeps a block RAM of every token-row's
/// activation words beside each channel (`ach[k].act_ram`), every copy written alike a clock after the word is made, and
/// a channel's cores — and its pairs' processing elements, which take the word in the clock of twice theirs, half a core's
/// clock after its read — read their channel's copy alone. So no bit of a read reaches more than its channel's cores and
/// their elements, where one memory's read reached every core of the array.
#[test]
fn each_channels_cores_read_the_activations_from_a_copy_of_their_own() {
    use logicaffeine_compile::spatial::decode_engine::rtl::column::{columns_verilog, ColumnSet};
    if !common::require_tool("yosys") {
        return;
    }
    let plain = ColumnSet::new("q4_k", &[GgmlType::Q4K], Words::One);
    let paired = ColumnSet::new("pairck", &[GgmlType::Q4K, GgmlType::Q6K], Words::Two).with_pe_pairs();
    for set in [plain, paired] {
        let v = columns_verilog(std::slice::from_ref(&set));
        let array = format!("logos_column_array_{}", set.name);
        for n in [2usize, 4] {
            let m = module_json(&v, &array, &[("CH", 4), ("N", n)]);
            let rams: Vec<&serde_json::Value> = m["cells"].as_object().unwrap().iter().filter(|(k, _)| k.ends_with("act_ram")).map(|(_, c)| c).collect();
            assert_eq!(rams.len(), 4, "{array}, {n} cores a channel: a copy of the activations a channel");
            for ram in rams {
                let read = &ram["connections"]["rd"];
                let reach = |bit: &serde_json::Value| -> usize {
                    let mut k = 0;
                    for c in m["cells"].as_object().unwrap().values() {
                        for (port, conns) in c["connections"].as_object().unwrap() {
                            if c["port_directions"][port.as_str()] == "input" {
                                k += conns.as_array().unwrap().iter().filter(|b| *b == bit).count();
                            }
                        }
                    }
                    k
                };
                let most = read.as_array().unwrap().iter().map(reach).max().unwrap();
                assert!(most <= 2 * n, "{array}, {n} cores a channel: a bit of a channel's read reaches {most} inputs, its channel's cores and elements {}", 2 * n);
            }
        }
    }
}

/// A step moves each core's ring and never its beats: a core takes its next beat by moving its ring's head, so its
/// `load` — the array's take, a broadcast of the step's last issue to every core — enables the head's place and no bit
/// of a beat, at any width of beat and either ring; its beats are written only by its `put`, its feed's register. At
/// hybrid8's set a beat is 5,280 bits, and a step that copied it would reach every one of them in every core.
#[test]
fn a_step_moves_each_cores_ring_and_never_its_beats() {
    use logicaffeine_compile::spatial::decode_engine::rtl::column::{columns_verilog, ColumnSet};
    if !common::require_tool("yosys") {
        return;
    }
    let narrow = ColumnSet::new("q4_k", &[GgmlType::Q4K], Words::One);
    let wide = ColumnSet::hybrid(&[GgmlType::Q8_0, GgmlType::Q4K, GgmlType::Q5K, GgmlType::Q6K], Words::One).with_octets().with_slots(16);
    for set in [narrow, wide] {
        let v = columns_verilog(std::slice::from_ref(&set));
        let core = format!("logos_column_core_{}", set.name);
        let beat = 2 * set.decoded_bits();
        for ring in [2usize, 3] {
            let m = module_json(&v, &core, &[("RING", ring)]);
            let (load, put) = (enabled_bits(&m, "load"), enabled_bits(&m, "put"));
            assert!(load <= 8, "{core}, a ring of {ring}: a step's load enables {load} bits, its beat is {beat}");
            assert!(put >= beat, "{core}, a ring of {ring}: a put writes its beat's {beat} bits, it enables {put}");
        }
    }
}

/// The places of the memories named `ring` in the cores (modules `core`) that `top` (one module of `verilog`, its
/// parameters `params`) instantiates, each instance's.
fn ring_places(verilog: &str, top: &str, core: &str, params: &[(&str, usize)]) -> usize {
    let modules = yosys_modules(verilog, top, &[top, core], params, "proc; memory_collect");
    let ring = |m: &serde_json::Value| -> usize {
        m["cells"]
            .as_object()
            .unwrap()
            .values()
            .filter(|c| c["type"] == "$mem_v2" && c["parameters"]["MEMID"] == "\\ring")
            .map(|c| yosys_number(&c["parameters"]["SIZE"]))
            .sum()
    };
    top_of(&modules)["cells"].as_object().unwrap().values().filter_map(|c| modules.get(c["type"].as_str().unwrap())).map(ring).sum()
}

/// A channel queues one decoded beat a core when it deals to two cores or more, and two when it deals to one, each in
/// its core's ring behind the beat the core holds, none in the feed: a pair's beat lands on its core no sooner than 2N
/// blocks after the core's last, with one beat at most decoded on its way, so for two or more cores a second queued beat
/// never fills while the core takes a beat a clock (the stream rate lock's law holds at two and three pairs); a lone
/// core's beats can come two blocks apart. A queued beat is 5,280 bits at hybrid8's set.
#[test]
fn a_channel_queues_one_decoded_beat_a_core_when_it_deals_to_two_cores_or_more() {
    use logicaffeine_compile::spatial::decode_engine::rtl::column::{columns_verilog, ColumnSet};
    if !common::require_tool("yosys") {
        return;
    }
    let v = columns_verilog(&[ColumnSet::new("q4_k", &[GgmlType::Q4K], Words::One)]);
    for n in 1..=3usize {
        assert_eq!(queue_places(&v, "logos_column_feed_q4_k", &[("N", n)]), 0, "{n} cores: the feed queues no beat of its own");
        let places = ring_places(&v, "logos_column_array_q4_k", "logos_column_core_q4_k", &[("CH", 1), ("N", n)]);
        let want = if n == 1 { 2 } else { n };
        assert_eq!(places - n, want, "{n} cores: {} beats queued behind the {n} held, want {want}", places - n);
    }
}

/// `logos_column_feed_<format>` under iverilog on each channel of a striped image: the image stream's beats of the
/// channel's tiles at random gaps, every core taking its beats at random, and every core's beats, in order, its pair's
/// blocks of each of its channel's tiles, block by block — the second zeros where a tile's last row has no partner —
/// with nothing left over: tiles whole and cut short, odd, of one row, of fewer bytes than a beat (several tiles in the
/// gather register at once), and blocks of 144, 210, 34 and 18 bytes.
#[test]
fn the_column_feed_deals_each_tiles_blocks_to_its_cores_in_pairs() {
    use logicaffeine_compile::spatial::decode_engine::rtl::column::{column_verilog, ColumnSet};
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0xfeed);
    // (format, elements a row, pairs a channel, rows)
    let runs = [(GgmlType::Q4K, 512usize, 2u64, 37usize), (GgmlType::Q6K, 512, 3, 50), (GgmlType::Q8_0, 512, 2, 9), (GgmlType::Q4K, 512, 1, 1), (GgmlType::Q4_0, 32, 1, 23), (GgmlType::Q8_0, 64, 1, 15)];
    for (ty, elems, n, rows) in runs {
        let (raw, image, streams) = striped_matrix(ty, elems, rows, n, &mut r);
        let p = &image.tensors[0];
        let set = ColumnSet::new(&ty.name().to_lowercase(), &[ty], Words::Two);
        let bb = ty.block_bytes();
        let blocks = elems / ty.block_len();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("column.v"), column_verilog()).unwrap();
        let tb = format!(
            r#"module tb;
  localparam N = {n}, BEAT = {beat};
  reg clk = 0, rst = 1;
  always #5 clk = ~clk;
  reg [31:0] BEATS [0:0];
  reg [511:0] bm [0:{last}];
  integer i = 0, f, quiet = 0;
  reg gap = 0;
  wire in_valid = !rst && i < BEATS[0] && !gap;
  wire in_ready;
  reg [N-1:0] w_ready = 0;
  wire [N-1:0] w_valid, w_put;
  wire [N*BEAT-1:0] w_beat;
  logos_column_feed_{name} #(.N(N)) u(.clk(clk), .rst(rst), .blocks(16'd{blocks}), .rows(32'd{rows}), .in_valid(in_valid),
    .in_ready(in_ready), .in_data(bm[4*i]), .in_keep(bm[4*i+1][6:0]), .in_row(bm[4*i+2][31:0]), .in_offset(bm[4*i+3][23:0]),
    .w_valid(w_valid), .w_ready(w_ready), .w_put(w_put), .w_beat(w_beat));
  integer k;
  always @(posedge clk) begin
    if (in_valid && in_ready) i <= i + 1;
    for (k = 0; k < N; k = k + 1) if (w_put[k]) $fwrite(f, "%0d %h\n", k, w_beat[k*BEAT +: BEAT]);
  end
  always @(negedge clk) begin
    gap = ($random & 3) == 0;
    w_ready = $random;
    quiet = (w_valid == 0 && i == BEATS[0]) ? quiet + 1 : 0;
  end
  initial begin
    $readmemh("count.hex", BEATS);
    $readmemh("beats.hex", bm);
    f = $fopen("out.txt", "w");
    @(negedge clk); rst = 0;
    while (quiet < 64) @(negedge clk);
    $fwrite(f, "left %0d %0d\n", u.fill, u.held);
    $fclose(f);
    $finish;
  end
endmodule
"#,
            beat = 2 * set.decoded_bits(),
            last = 4 * streams.iter().map(Vec::len).max().unwrap().max(1) - 1,
            name = ty.name().to_lowercase(),
        );
        std::fs::write(dir.path().join("tb.v"), tb).unwrap();
        let sim = rtl_sim::Simulator::build(dir.path(), &["column.v", "tb.v"]);
        for (ch, stream) in streams.iter().enumerate() {
            let at = dir.path().join(format!("ch{ch}"));
            std::fs::create_dir_all(&at).unwrap();
            let mut hex = String::new();
            beat_hex(stream, &mut hex);
            std::fs::write(at.join("beats.hex"), &hex).unwrap();
            std::fs::write(at.join("count.hex"), format!("{:x}\n", stream.len())).unwrap();
            let run = sim.run(&at, &[]);
            assert!(run.status.success(), "simulation ({}): {}", ty.name(), String::from_utf8_lossy(&run.stderr));
            let out = std::fs::read_to_string(at.join("out.txt")).unwrap();
            let what = format!("{} in stripes of {n} pairs, {rows} rows, channel {ch}", ty.name());
            // What each core takes: its pair of every tile on this channel, block by block, decoded.
            let zeros = vec![0u8; bb];
            let block = |k: usize, b: usize| -> &[u8] { if k < rows { &raw[k][b * bb..(b + 1) * bb] } else { &zeros } };
            let mut want: Vec<Vec<String>> = vec![Vec::new(); n as usize];
            for t in image.tiles.iter().filter(|t| t.channel as usize == ch) {
                let held = (t.end_row - t.first_row) as usize;
                for b in 0..blocks {
                    for q in 0..held.div_ceil(2) {
                        let k = t.first_row as usize + 2 * q;
                        let second = if 2 * q + 1 < held { block(k + 1, b) } else { &zeros[..] };
                        want[q].push(decoded_beat_hex(&set, ty, block(k, b), second));
                    }
                }
            }
            let mut got: Vec<Vec<String>> = vec![Vec::new(); n as usize];
            let mut left = "";
            for l in out.lines() {
                if let Some(rest) = l.strip_prefix("left ") {
                    left = rest;
                    continue;
                }
                let (k, pair) = l.split_once(' ').unwrap();
                got[k.parse::<usize>().unwrap()].push(pair.to_string());
            }
            for q in 0..n as usize {
                assert_eq!(got[q].len(), want[q].len(), "{what}: core {q}'s beats");
                if let Some(i) = (0..got[q].len()).find(|&i| got[q][i] != want[q][i]) {
                    panic!("{what}: core {q}'s beat {i}: feed {}, image {}", got[q][i], want[q][i]);
                }
            }
            assert_eq!(left, "0 0", "{what}: the gather register empty and no tile open at the end");
            assert!(p.block_major && p.rows_per_tile == (2 * n).min(rows as u64), "{what}");
        }
    }
}

/// The streaming matvec from the image under the RTL simulator, built once for the format set `formats` (named `set`)
/// at `n` pairs a channel and run for each job of `runs` — (the format's place in the set, word count, rows, token-rows)
/// — a matrix of 512-element rows laid in stripes over the four channels, each channel's beats as the image stream
/// sends them through its `logos_column_feed_<set>` at random gaps into `logos_column_array_<set>`, whose cores' turns
/// are the image's deal. Every row's every token-row is checked against `numerics::dot` once; returned are the
/// differences.
fn streaming_matvec_runs(set: &str, formats: &[GgmlType], n: u64, runs: &[(usize, Words, usize, usize)], r: &mut Rng) -> Vec<String> {
    streaming_matvec_runs_in(set, formats, n, 1, runs, r)
}

/// [`streaming_matvec_runs`] with every block of the image in a slot of a multiple of `slot` bytes and the set's feeds
/// gathering `slot` bytes at a time (`ColumnSet::with_slots`).
fn streaming_matvec_runs_in(set: &str, formats: &[GgmlType], n: u64, slot: usize, runs: &[(usize, Words, usize, usize)], r: &mut Rng) -> Vec<String> {
    use logicaffeine_compile::spatial::decode_engine::rtl::column::{columns_verilog, ColumnSet};
    const ELEMS: usize = 512;
    let elems_max = formats.iter().map(|t| t.block_len()).max().unwrap();
    let column_set = ColumnSet::new(set, formats, Words::Two);
    let column_set = if slot > 1 { column_set.with_slots(slot) } else { column_set };
    let beat = 2 * column_set.decoded_bits();
    let multi = formats.len() > 1;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("column.v"), columns_verilog(&[column_set])).unwrap();
    let fmt_port = if multi { ".fmt(cfg[0][3:0]), " } else { "" };
    let tb = format!(
        r#"module tb;
  localparam N = {n}, C = 4 * N, BEAT = {beat};
  reg clk = 0, rst = 1;
  always #5 clk = ~clk;
  reg start = 0, act_wr = 0, feeding = 0, hold = 0;
  reg [7:0] act_t, act_m;
  reg [15:0] act_b;
  reg [11:0] act_e;
  reg [{aw}:0] act_codes;
  reg [{aw}:0] am [0:8191];
  reg [511:0] bm [0:65535];
  // The job: format, blocks a row, rows, stripes, token-rows, word count, the channels' turns, each channel's first
  // beat and the end of the last, the activation blocks.
  reg [31:0] cfg [0:12];
  wire busy;
  wire [C-1:0] w_valid, w_ready, w_put, o_valid;
  wire [C*BEAT-1:0] w_beat;
  wire [C*32-1:0] o_val1, o_val2;
  wire [15:0] o_stripe;
  wire [7:0] o_t;
  genvar ch;
  generate
    for (ch = 0; ch < 4; ch = ch + 1) begin : chan
      integer i = 0;
      reg gap = 0;
      wire in_valid = feeding && cfg[7 + ch] + i < cfg[8 + ch] && !gap;
      wire in_ready;
      wire [31:0] at = 4 * (cfg[7 + ch] + i);
      logos_column_feed_{set} #(.N(N)) feed(.clk(clk), .rst(rst), {fmt_port}.blocks(cfg[1][15:0]), .rows(cfg[2]),
        .in_valid(in_valid), .in_ready(in_ready), .in_data(bm[at]), .in_keep(bm[at+1][6:0]), .in_row(bm[at+2][31:0]),
        .in_offset(bm[at+3][23:0]), .w_valid(w_valid[ch*N +: N]), .w_ready(w_ready[ch*N +: N]), .w_put(w_put[ch*N +: N]),
        .w_beat(w_beat[ch*N*BEAT +: N*BEAT]));
      always @(posedge clk) begin
        if (in_valid && in_ready) i <= i + 1;
        gap <= ($random & 3) == 0;
      end
    end
  endgenerate
  logos_column_array_{set} #(.CH(4), .N(N), .T_MAX(4), .NB_MAX(16)) u(.clk(clk), .rst(rst), .start(start),
    .stripes(cfg[3][15:0]), .blocks(cfg[1][15:0]), .tokens(cfg[4][7:0]), .two(cfg[5][0]), {fmt_port}.rows(cfg[2]),
    .turns(cfg[6][7:0]), .hold(hold), .busy(busy), .act_wr(act_wr), .act_t(act_t), .act_b(act_b), .act_codes(act_codes),
    .act_m(act_m), .act_e(act_e), .w_valid(w_valid), .w_ready(w_ready), .w_put(w_put), .w_beat(w_beat),
    .o_valid(o_valid), .o_val1(o_val1), .o_val2(o_val2), .o_stripe(o_stripe), .o_t(o_t));
  integer i, j, f;
  always @(posedge clk) hold <= ($random & 7) == 0;
  always @(negedge clk) if (o_valid != 0)
    for (j = 0; j < C; j = j + 1) if (o_valid[j]) $fwrite(f, "%0d %h %h %h %h\n", j, o_stripe, o_t, o_val1[j*32 +: 32], o_val2[j*32 +: 32]);
  initial begin
    $readmemh("config.hex", cfg);
    $readmemh("acts.hex", am);
    $readmemh("beats.hex", bm);
    f = $fopen("out.txt", "w");
    @(negedge clk); rst = 0;
    for (i = 0; i < cfg[12]; i = i + 1) begin
      @(negedge clk); act_wr = 1; act_codes = am[5*i]; act_t = am[5*i+1]; act_b = am[5*i+2]; act_m = am[5*i+3]; act_e = am[5*i+4];
    end
    @(negedge clk); act_wr = 0; start = 1;
    @(negedge clk); start = 0; feeding = 1;
    while (busy) @(negedge clk);
    @(negedge clk); @(negedge clk);
    $fclose(f);
    $finish;
  end
endmodule
"#,
        aw = 16 * elems_max - 1,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let sim = rtl_sim::Simulator::build(dir.path(), &["column.v", "tb.v"]);
    let mut wrong = Vec::new();
    for &(fi, words, rows_n, tokens) in runs {
        let ty = formats[fi];
        let (raw, image, streams) = striped_matrix_in(ty, ELEMS, rows_n, n, slot as u64, r);
        let p = &image.tensors[0];
        let len = ty.block_len();
        let blocks = ELEMS / len;
        let rows: Vec<CodedRow> = raw.iter().map(|b| CodedRow::decode(ty, b).unwrap().unwrap()).collect();
        let acts: Vec<_> = (0..tokens)
            .map(|_| {
                let x: Vec<f32> = (0..ELEMS).map(|_| ((r.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * 2f32.powi((r.next() % 16) as i32 - 8)).collect();
                quantize_blocks(&x, words, len)
            })
            .collect();
        let mut turns = [0usize; 4];
        for (k, &ch) in p.channels.iter().enumerate() {
            turns[ch as usize] = k;
        }
        let turn_bits: usize = turns.iter().enumerate().map(|(ch, &t)| t << (2 * ch)).sum();
        let stripes = rows_n.div_ceil(8 * n as usize);
        let mut act_hex = String::new();
        for (t, act) in acts.iter().enumerate() {
            for b in 0..blocks {
                for i in (0..len).rev() {
                    write!(act_hex, "{:04x}", act.codes[b * len + i] as i16 as u16).unwrap();
                }
                writeln!(act_hex, "\n{t:02x}\n{b:04x}\n{:02x}\n{:03x}", act.scales[b].m, act.scales[b].e as i16 as u16 & 0xfff).unwrap();
            }
        }
        let (mut beats, mut bases) = (String::new(), vec![0usize]);
        for stream in &streams {
            beat_hex(stream, &mut beats);
            bases.push(bases.last().unwrap() + stream.len());
        }
        let config = [fi, blocks, rows_n, stripes, tokens, (words == Words::Two) as usize, turn_bits, bases[0], bases[1], bases[2], bases[3], bases[4], tokens * blocks];
        std::fs::write(dir.path().join("config.hex"), config.iter().map(|c| format!("{c:x}\n")).collect::<String>()).unwrap();
        std::fs::write(dir.path().join("acts.hex"), act_hex).unwrap();
        std::fs::write(dir.path().join("beats.hex"), beats).unwrap();
        let run = sim.run(dir.path(), &[]);
        assert!(run.status.success(), "simulation ({}): {}", ty.name(), String::from_utf8_lossy(&run.stderr));
        let out = std::fs::read_to_string(dir.path().join("out.txt")).unwrap();
        let what = format!("{set}: {} {words:?}, {rows_n} rows in stripes of {n} pairs, {tokens} token-rows", ty.name());
        let mut seen = std::collections::BTreeSet::new();
        for l in out.lines() {
            let f: Vec<&str> = l.split(' ').collect();
            let g: usize = f[0].parse().unwrap();
            let (s, t) = (usize::from_str_radix(f[1], 16).unwrap(), usize::from_str_radix(f[2], 16).unwrap());
            let (ch, q) = (g / n as usize, g % n as usize);
            let k = (4 * s + turns[ch]) * 2 * n as usize + 2 * q;
            assert!(k < rows_n && t < tokens, "{what}: core {g} emitted stripe {s} token-row {t}, row {k}");
            for (row, got) in [(k, f[3]), (k + 1, f[4])] {
                if row >= rows_n {
                    continue;
                }
                assert!(seen.insert((row, t)), "{what}: row {row} token-row {t} emitted twice");
                let got = u32::from_str_radix(got, 16).unwrap();
                let want = dot(&rows[row], &acts[t]).to_bits();
                if got != want {
                    wrong.push(format!("{what}: row {row} token-row {t}: rtl {got:08x}, dot {want:08x}"));
                }
            }
        }
        assert_eq!(seen.len(), rows_n * tokens, "{what}: every row and token-row emitted; {stripes} stripes");
    }
    wrong
}

/// The streaming matvec from the image under the RTL simulator: a matrix laid in stripes over the four channels, each
/// channel's beats as the image stream sends them through its feed at random gaps into the column array, an eighth of
/// its clocks held, every row's every token-row `numerics::dot`, bit for bit, once — a stripe cut short to an odd last
/// row, both word counts.
#[test]
fn the_streaming_matvec_takes_a_striped_image_matrix_as_dot_does() {
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0x57a7);
    let mut wrong = streaming_matvec_runs("q4_k", &[GgmlType::Q4K], 2, &[(0, Words::One, 37, 3)], &mut r);
    wrong.extend(streaming_matvec_runs("q6_k", &[GgmlType::Q6K], 1, &[(0, Words::Two, 13, 2)], &mut r));
    assert!(wrong.is_empty(), "{} dots differ:\n{}", wrong.len(), wrong.iter().take(16).cloned().collect::<Vec<_>>().join("\n"));
}

/// One streaming matvec built for every format Qwen3.8-27B's Q4_K_M checkpoint holds its matrices in — Q4_K, Q5_K, Q6_K
/// and Q8_0 for its step, Q4_0 for its drafting block — one core holding a block of any of them, the job's format chosen,
/// runs a striped matrix of each, in both word counts, every row's every token-row `numerics::dot`, bit for bit, once:
/// K-quant blocks of 256 elements of one and two digits, and legacy blocks of 32 with the units past them zero.
#[test]
fn one_streaming_matvec_takes_every_format_the_27b_holds() {
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0x27b);
    let set = [GgmlType::Q4K, GgmlType::Q5K, GgmlType::Q6K, GgmlType::Q8_0, GgmlType::Q4_0];
    let runs = [(0usize, Words::One, 13usize, 3usize), (1, Words::Two, 11, 2), (2, Words::One, 9, 1), (3, Words::Two, 13, 2), (4, Words::One, 7, 3), (0, Words::Two, 16, 4)];
    let wrong = streaming_matvec_runs("q4km", &set, 1, &runs, &mut r);
    assert!(wrong.is_empty(), "{} dots differ:\n{}", wrong.len(), wrong.iter().take(16).cloned().collect::<Vec<_>>().join("\n"));
}

/// [`one_streaming_matvec_takes_every_format_the_27b_holds`] with every block in a slot: the image's blocks padded with
/// zeros to a multiple of 16 bytes (Q4_K's 144 and Q5_K's 176 none, Q6_K's 210, Q8_0's 34 and Q4_0's 18 each to the next)
/// and of 64 (every block padded), each feed gathering its beats a slot at a time — every row's every token-row
/// `numerics::dot`, bit for bit, once.
#[test]
fn the_streaming_matvec_takes_a_matrix_of_blocks_in_slots_as_dot_does() {
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Rng(0x5107);
    let set = [GgmlType::Q4K, GgmlType::Q5K, GgmlType::Q6K, GgmlType::Q8_0, GgmlType::Q4_0];
    let runs = [(0usize, Words::One, 13usize, 3usize), (1, Words::Two, 11, 2), (2, Words::One, 9, 1), (3, Words::Two, 13, 2), (4, Words::One, 7, 3), (2, Words::Two, 16, 4)];
    let mut wrong = streaming_matvec_runs_in("q4km", &set, 1, 16, &runs, &mut r);
    wrong.extend(streaming_matvec_runs_in("k64", &[GgmlType::Q4K, GgmlType::Q6K], 2, 64, &[(0, Words::One, 37, 3), (1, Words::Two, 13, 2)], &mut r));
    assert!(wrong.is_empty(), "{} dots differ:\n{}", wrong.len(), wrong.iter().take(16).cloned().collect::<Vec<_>>().join("\n"));
}

// ── The attention unit ─────────────────────────────────────────────────────────────────────────────

/// `logos_attend_vector` under iverilog: seeded 64-element heads over 1 to 64 positions — either side of the sixteen
/// position-lanes, scores spread from a flat softmax to a sharp one, a largest score of zero — each output element
/// `numerics::attend_vector`'s, bit for bit.
#[test]
fn the_attention_unit_attends_as_the_numerics_do() {
    if !common::require_tool("iverilog") {
        return;
    }
    const HD: usize = 64;
    let mut r = Rng(0xa77e);
    let mut f = || ((r.next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0;
    struct Case {
        q: Vec<f32>,
        keys: Vec<Vec<f32>>,
        values: Vec<Vec<f32>>,
        scale: f32,
    }
    let mut cases = Vec::new();
    for &p in &[1usize, 2, 15, 16, 17, 33, 40, 64] {
        for &spread in &[0.01f32, 0.25, 1.0, 6.0] {
            let q: Vec<f32> = (0..HD).map(|_| f()).collect();
            let keys: Vec<Vec<f32>> = (0..p).map(|_| (0..HD).map(|_| f() * spread).collect()).collect();
            let values: Vec<Vec<f32>> = (0..p).map(|_| (0..HD).map(|_| f() * 3.0).collect()).collect();
            cases.push(Case { q, keys, values, scale: 0.125 });
        }
    }
    // Every score zero but one negative: the largest score a zero.
    let zeros: Vec<Vec<f32>> = (0..5).map(|j| if j == 2 { vec![-1.0; HD] } else { vec![0.0; HD] }).collect();
    cases.push(Case { q: vec![1.0; HD], keys: zeros, values: (0..5).map(|_| (0..HD).map(|_| f()).collect()).collect(), scale: 0.125 });
    let beats = |v: &[f32]| -> Vec<String> {
        v.chunks(16)
            .map(|c| {
                let mut s = String::new();
                for x in c.iter().rev() {
                    write!(s, "{:08x}", x.to_bits()).unwrap();
                }
                s
            })
            .collect()
    };
    let mut script = String::new();
    for c in &cases {
        let mut qhex = String::new();
        for x in c.q.iter().rev() {
            write!(qhex, "{:08x}", x.to_bits()).unwrap();
        }
        writeln!(script, "C {:x} {:08x} {qhex}", c.keys.len(), c.scale.to_bits()).unwrap();
        for k in &c.keys {
            for b in beats(k) {
                writeln!(script, "K {b}").unwrap();
            }
        }
        for v in &c.values {
            for b in beats(v) {
                writeln!(script, "V {b}").unwrap();
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("script.txt"), script).unwrap();
    std::fs::write(dir.path().join("attend.v"), attend_verilog()).unwrap();
    let tb = format!(
        r#"module tb;
  reg clk = 0, rst = 1;
  always #5 clk = ~clk;
  reg start = 0, k_valid = 0, v_valid = 0;
  reg [15:0] positions;
  reg [{qw}:0] q;
  reg [31:0] scale;
  reg [511:0] k_beat, v_beat;
  wire busy, k_ready, v_ready, o_valid, o_last;
  wire [511:0] o_beat;
  logos_attend_vector #(.HD({HD}), .VD({HD}), .P_MAX(64)) u(.clk(clk), .rst(rst), .start(start), .positions(positions), .q(q),
    .scale(scale), .busy(busy), .k_valid(k_valid), .k_beat(k_beat), .k_ready(k_ready), .v_valid(v_valid), .v_beat(v_beat),
    .v_ready(v_ready), .o_valid(o_valid), .o_beat(o_beat), .o_last(o_last));
  integer fi, fo, c;
  reg [7:0] tag;
  reg [511:0] word;
  always @(negedge clk) if (o_valid) $fwrite(fo, "%h\n", o_beat);
  initial begin
    fi = $fopen("script.txt", "r");
    fo = $fopen("out.txt", "w");
    @(negedge clk); rst = 0;
    while (!$feof(fi)) begin
      c = $fscanf(fi, "%c", tag);
      if (tag == "C") begin
        while (busy) @(negedge clk);
        c = $fscanf(fi, " %h %h %h\n", positions, scale, q);
        start = 1; @(negedge clk); start = 0;
      end else if (tag == "K") begin
        c = $fscanf(fi, " %h\n", word);
        k_beat = word; k_valid = 1;
        while (!k_ready) @(negedge clk);
        @(negedge clk); k_valid = 0;
      end else if (tag == "V") begin
        c = $fscanf(fi, " %h\n", word);
        v_beat = word; v_valid = 1;
        while (!v_ready) @(negedge clk);
        @(negedge clk); v_valid = 0;
      end
    end
    while (busy) @(negedge clk);
    @(negedge clk); @(negedge clk);
    $fclose(fo);
    $finish;
  end
endmodule
"#,
        qw = 32 * HD - 1,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-o", "sim", "tb.v", "attend.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    let out = std::fs::read_to_string(dir.path().join("out.txt")).unwrap();
    let mut lines = out.lines();
    let mut wrong = Vec::new();
    for (ci, c) in cases.iter().enumerate() {
        let keys: Vec<&[f32]> = c.keys.iter().map(|k| k.as_slice()).collect();
        let values: Vec<&[f32]> = c.values.iter().map(|v| v.as_slice()).collect();
        let want = attend_vector(FpUnit::Soft, &c.q, &keys, &values, c.scale);
        for b in 0..HD / 16 {
            let line = lines.next().unwrap_or_else(|| panic!("case {ci}: beat {b} missing"));
            for l in 0..16 {
                let got = hex_field(line, l, 8);
                if got != want[16 * b + l].to_bits() {
                    wrong.push(format!("case {ci} ({} positions) element {}: rtl {got:08x}, numerics {:08x}", c.keys.len(), 16 * b + l, want[16 * b + l].to_bits()));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{} elements differ:\n{}", wrong.len(), wrong.iter().take(20).cloned().collect::<Vec<_>>().join("\n"));
}

/// Heads of lazily flushed rows streamed back to back through `logos_gdn_read_p`, a beat a clock at most under
/// iverilog, the beats held back at random and the unit holding them off when its buffers are full: every row's read-out,
/// in the rows' order, `gdn_read`'s of the row, its head's folded query and scale and its carried value — 128, 32 and
/// 16 elements a row, the last a row of one beat, over values of one magnitude and the special values among them.
#[test]
fn the_streaming_read_out_reads_every_row_as_the_numerics_do() {
    if !common::require_tool("iverilog") {
        return;
    }
    let specials = [0u32, 0x8000_0000, 1, 0x8000_0001, 0x0080_0000, 0x7f80_0000, 0xff80_0000, 0x7fc0_0000, 0x7f7f_ffff, 0xff7f_ffff];
    for (n, heads) in [(128usize, 3usize), (32, 5), (16, 6)] {
        let mut r = Rng(0x9d1 ^ n as u64);
        let value = |r: &mut Rng, magnitude: f32| -> f32 {
            if r.next() % 97 == 0 {
                f32::from_bits(specials[(r.next() % specials.len() as u64) as usize])
            } else {
                (((r.next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0) * magnitude
            }
        };
        let ys: Vec<Vec<f32>> = (0..heads).map(|_| (0..n).map(|_| value(&mut r, 0.1)).collect()).collect();
        let scales: Vec<f32> = (0..heads).map(|h| if h % 2 == 0 { (1.0 / (n as f64).sqrt()) as f32 } else { value(&mut r, 2.0) }).collect();
        // Each row: its head, its elements and its carried value.
        let rows: Vec<(usize, Vec<f32>, f32)> =
            (0..heads * n).map(|i| (i / n, (0..n).map(|_| value(&mut r, 0.5)).collect(), value(&mut r, 0.2))).collect();
        let beats = n / 16;
        let hex = |v: &[f32]| -> String { v.iter().rev().map(|x| format!("{:08x}", x.to_bits())).collect() };
        let (mut beat_hex, mut tag_hex) = (String::new(), String::new());
        for (h, row, carried) in &rows {
            for b in 0..beats {
                writeln!(beat_hex, "{}", hex(&row[16 * b..16 * b + 16])).unwrap();
                // A beat's head, its carried value and the clocks held back before it: an odd head's beats as fast as
                // the unit takes them, so its buffers fill and it holds them off.
                let gap = if h % 2 == 0 && r.next() % 3 == 0 { r.next() % 4 } else { 0 };
                writeln!(tag_hex, "{:02x}{:08x}{h:04x}", gap, carried.to_bits()).unwrap();
            }
        }
        let y_hex: String = ys.iter().map(|y| format!("{}\n", hex(y))).collect();
        let scale_hex: String = scales.iter().map(|s| format!("{:08x}\n", s.to_bits())).collect();
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in [("beats.hex", beat_hex), ("tags.hex", tag_hex), ("ys.hex", y_hex), ("scales.hex", scale_hex)] {
            std::fs::write(dir.path().join(name), text).unwrap();
        }
        std::fs::write(dir.path().join("gdn.v"), gdn_stream_verilog()).unwrap();
        let total = rows.len() * beats;
        let tb = format!(
            r#"module tb;
  reg clk = 0, rst = 1;
  always #5 clk = ~clk;
  reg [511:0] beats [0:{last}];
  reg [55:0] tags [0:{last}];
  reg [{yw}:0] ys [0:{hl}];
  reg [31:0] scales [0:{hl}];
  reg [{yw}:0] y = 0;
  reg [31:0] scale = 0, in_carried = 0;
  reg [511:0] in_beat = 0;
  reg in_valid = 0;
  wire in_ready, out_valid;
  wire [31:0] out_read;
  logos_gdn_read_p #(.N({n})) u(.clk(clk), .rst(rst), .y(y), .scale(scale), .in_valid(in_valid), .in_ready(in_ready),
    .in_beat(in_beat), .in_carried(in_carried), .out_valid(out_valid), .out_read(out_read));
  integer i, g, f, outs = 0, stalls = 0;
  always @(posedge clk) if (out_valid) begin $fwrite(f, "%h\n", out_read); outs = outs + 1; end
  initial begin
    $readmemh("beats.hex", beats);
    $readmemh("tags.hex", tags);
    $readmemh("ys.hex", ys);
    $readmemh("scales.hex", scales);
    f = $fopen("out.hex", "w");
    repeat (3) @(negedge clk);
    rst = 0;
    for (i = 0; i < {total}; i = i + 1) begin
      for (g = 0; g < tags[i][55:48]; g = g + 1) begin @(negedge clk); in_valid = 0; end
      @(negedge clk);
      in_valid = 1; in_beat = beats[i]; in_carried = tags[i][47:16]; y = ys[tags[i][15:0]]; scale = scales[tags[i][15:0]];
      while (!in_ready) begin stalls = stalls + 1; @(negedge clk); end
      @(posedge clk);
    end
    @(negedge clk);
    in_valid = 0;
    repeat (200) @(negedge clk);
    $display("STALLS %0d", stalls);
    $fclose(f);
    $finish;
  end
endmodule
"#,
            last = total - 1,
            yw = 32 * n - 1,
            hl = heads - 1,
        );
        std::fs::write(dir.path().join("tb.v"), tb).unwrap();
        let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "gdn.v"]).output().expect("iverilog");
        assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
        let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
        assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
        let stalls: usize = String::from_utf8_lossy(&run.stdout).split("STALLS ").nth(1).and_then(|s| s.split_whitespace().next()?.parse().ok()).unwrap_or(0);
        let out = std::fs::read_to_string(dir.path().join("out.hex")).unwrap();
        let got: Vec<u32> = out.lines().map(|l| u32::from_str_radix(l, 16).unwrap()).collect();
        assert_eq!(got.len(), rows.len(), "{n} a row: a read-out for every row");
        let mut wrong = Vec::new();
        for (i, ((h, row, carried), &g)) in rows.iter().zip(&got).enumerate() {
            let want = gdn_read(FpUnit::Soft, row, &ys[*h], *carried, scales[*h]).to_bits();
            if g != want {
                wrong.push(format!("{n} a row, row {i} of head {h}: pipelined {g:08x}, numerics {want:08x}"));
            }
        }
        assert!(wrong.is_empty(), "{} of {} rows differ:\n{}", wrong.len(), rows.len(), wrong.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
        assert!(stalls > 0, "{n} a row: the unit never held the beats off");
    }
}

/// Heads of lazily flushed rows through `logos_gdn_flush_p` under iverilog, each head's log written and then its rows
/// streamed in fours, the beats held back at random and the flushed rows taken at random: every element of every row
/// `gdn_flush`'s — every logged update replayed in order and the row stored once — at 128, 32 and 16 elements a row,
/// logs of sixteen, seven, three, one and no tokens, the state stored in f32 and in f16.
#[test]
fn the_streaming_flush_replays_every_row_as_the_numerics_do() {
    if !common::require_tool("iverilog") {
        return;
    }
    for (n, logs) in [(128usize, vec![16usize, 1, 7]), (32, vec![16, 3, 0]), (16, vec![2, 16])] {
        for f16 in [false, true] {
            narrow_flush_case(n, &logs, f16, 0, true);
        }
    }
}

/// The flush reset for one clock with its passes in flight — a head's log written, its rows in, its run under way —
/// gives none of it out, and with the head's rows sent again at once over the log it still holds flushes that head and
/// every head after as the numerics do: at 128 and 32 elements, in f32 and in binary16, reset in a dot pass, as its
/// coefficients come and as its updates are written.
#[test]
fn a_flush_reset_with_its_passes_in_flight_flushes_every_head_after_as_the_numerics_do() {
    if !common::require_tool("iverilog") {
        return;
    }
    for (n, logs, f16, aborted) in [(128usize, vec![16usize, 1, 7], false, 11usize), (128, vec![16, 1, 7], false, 95), (128, vec![16, 3], true, 130), (32, vec![16, 3, 0], true, 58)] {
        narrow_flush_case(n, &logs, f16, aborted, false);
    }
}

/// The clocks a flush testbench lets pass with nothing moved — no log beat written, no row beat taken or given — before
/// it stops; every case's longest such stretch stays under a quarter of it.
const FLUSH_STALL: u64 = 200_000;

/// [`the_streaming_flush_replays_every_row_as_the_numerics_do`]'s heads of `n` elements with `logs` tokens each, the state
/// stored in binary16 given `f16`; with `aborted` clocks, the first head's log written and its rows sent, the unit reset
/// for one clock that many clocks into its run, and the head's rows sent again at once over the log it still holds,
/// then every other head, only what it gives out after taken; on iverilog's four states given `four_state`, else on the
/// box's fastest simulator ([`rtl_sim::Simulator::build`]).
fn narrow_flush_case(n: usize, logs: &[usize], f16: bool, aborted: usize, four_state: bool) {
    const T: usize = 16;
    let specials = [0u32, 0x8000_0000, 1, 0x0080_0000, 0x7f80_0000, 0x7fc0_0000];
    {
        {
            let mut r = Rng(0xf1u64 ^ (n as u64) << 4 ^ f16 as u64 ^ (aborted as u64) << 16);
            let value = |r: &mut Rng, magnitude: f32| -> f32 {
                if r.next() % 211 == 0 {
                    f32::from_bits(specials[(r.next() % specials.len() as u64) as usize])
                } else {
                    (((r.next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0) * magnitude
                }
            };
            struct Head {
                keys: Vec<Vec<f32>>,
                values: Vec<Vec<f32>>,
                log_decays: Vec<f32>,
                betas: Vec<f32>,
                rows: Vec<Vec<f32>>,
            }
            let heads: Vec<Head> = logs
                .iter()
                .map(|&entries| Head {
                    keys: (0..entries).map(|_| (0..n).map(|_| value(&mut r, 0.09)).collect()).collect(),
                    values: (0..entries).map(|_| (0..n).map(|_| value(&mut r, 1.0)).collect()).collect(),
                    log_decays: (0..entries).map(|_| -value(&mut r, 0.3).abs()).collect(),
                    betas: (0..entries).map(|_| value(&mut r, 1.0).abs()).collect(),
                    rows: (0..n).map(|_| (0..n).map(|_| value(&mut r, 0.5)).collect()).collect(),
                })
                .collect();
            let beats = n / 16;
            let hex = |v: &[f32]| -> String { v.iter().rev().map(|x| format!("{:08x}", x.to_bits())).collect() };
            let (mut key_hex, mut vlog_hex, mut tok_hex, mut beat_hex, mut head_hex) = (String::new(), String::new(), String::new(), String::new(), String::new());
            let (mut keys_at, mut toks_at) = (0usize, 0usize);
            for (h, head) in heads.iter().enumerate() {
                writeln!(head_hex, "{:04x}{:04x}{:02x}", keys_at, toks_at, head.keys.len()).unwrap();
                for (e, k) in head.keys.iter().enumerate() {
                    for b in 0..beats {
                        writeln!(key_hex, "{}", hex(&k[16 * b..16 * b + 16])).unwrap();
                        writeln!(vlog_hex, "{}", hex(&head.values[e][16 * b..16 * b + 16])).unwrap();
                    }
                    writeln!(tok_hex, "{:08x}{:08x}", head.log_decays[e].to_bits(), head.betas[e].to_bits()).unwrap();
                }
                keys_at += head.keys.len() * beats;
                toks_at += head.keys.len();
                for (i, row) in head.rows.iter().enumerate() {
                    for b in 0..beats {
                        let gap = if (h + i) % 3 == 0 && r.next() % 2 == 0 { r.next() % 3 } else { 0 };
                        writeln!(beat_hex, "{:02x}{}", gap, hex(&row[16 * b..16 * b + 16])).unwrap();
                    }
                }
            }
            let dir = tempfile::tempdir().unwrap();
            let pad = |t: String| if t.is_empty() { "0\n".to_string() } else { t };
            for (name, text) in [("keys.hex", pad(key_hex)), ("vlogs.hex", pad(vlog_hex)), ("toks.hex", pad(tok_hex)), ("beats.hex", beat_hex), ("heads.hex", head_hex)] {
                std::fs::write(dir.path().join(name), text).unwrap();
            }
            std::fs::write(dir.path().join("gdn.v"), gdn_stream_verilog()).unwrap();
            let rows_total = heads.len() * n;
            let tb = format!(
                r#"module tb;
  reg clk = 0, rst = 1;
  always #5 clk = ~clk;
  reg [511:0] keys [0:{kl}];
  reg [511:0] vlogs [0:{kl}];
  reg [63:0] toks [0:{tl}];
  reg [519:0] beats [0:{bl}];
  reg [39:0] hs [0:{hl}];
  reg log_wr = 0, in_valid = 0, out_ready = 0;
  reg [7:0] log_at = 0, entries = 0;
  reg [15:0] log_beat = 0;
  reg [511:0] log_k = 0, log_v = 0, in_beat = 0;
  reg [31:0] log_decay = 0, log_beta = 0;
  wire in_ready, out_valid;
  wire [511:0] out_beat;
  logos_gdn_flush_p #(.N({n}), .T_MAX({T})) u(.clk(clk), .rst(rst), .log_wr(log_wr), .log_at(log_at), .log_beat(log_beat),
    .log_k(log_k), .log_v(log_v), .log_decay(log_decay), .log_beta(log_beta), .entries(entries), .f16(1'b{f16}), .in_valid(in_valid),
    .in_ready(in_ready), .in_beat(in_beat), .out_valid(out_valid), .out_ready(out_ready), .out_beat(out_beat));
  integer h, e, b, row, g, f, outs = 0, seed = 7, held = 0;
  reg logging = {logging};
  integer quiet = 0, quietest = 0;
  always @(posedge clk) begin
    quiet = (log_wr || (in_valid && in_ready) || (out_valid && out_ready)) ? 0 : quiet + 1;
    if (quiet > quietest) quietest = quiet;
    if (quiet == {stall}) begin $display("STALL after %0d quiet clocks, %0d beats out", quiet, outs); $finish; end
  end
  always @(posedge clk) if (out_valid && logging) begin $fwrite(f, "%h\n", out_beat); outs = outs + 1; end
  always @(negedge clk) begin out_ready = ($random(seed) & 3) != 0; if (!out_ready) held = held + 1; end
  initial begin
    $readmemh("keys.hex", keys);
    $readmemh("vlogs.hex", vlogs);
    $readmemh("toks.hex", toks);
    $readmemh("beats.hex", beats);
    $readmemh("heads.hex", hs);
    f = $fopen("out.hex", "w");
    repeat (3) @(negedge clk);
    rst = 0;
    row = 0;
{abort}    for (h = 0; h <= {hl}; h = h + 1) begin
      entries = hs[h][7:0];
      for (e = 0; e < ((h == 0 && {held}) ? 0 : hs[h][7:0]); e = e + 1)
        for (b = 0; b < {beats}; b = b + 1) begin
          @(negedge clk);
          log_wr = 1; log_at = e; log_beat = b; log_k = keys[hs[h][39:24] + e * {beats} + b]; log_v = vlogs[hs[h][39:24] + e * {beats} + b];
          log_decay = toks[hs[h][23:8] + e][63:32]; log_beta = toks[hs[h][23:8] + e][31:0];
        end
      @(negedge clk);
      log_wr = 0;
      for (g = 0; g < {n}; g = g + 1) begin
        for (b = 0; b < {beats}; b = b + 1) begin
          repeat (beats[row * {beats} + b][519:512]) begin @(negedge clk); in_valid = 0; end
          @(negedge clk);
          in_valid = 1; in_beat = beats[row * {beats} + b][511:0];
          while (!in_ready) @(negedge clk);
          @(posedge clk);
        end
        row = row + 1;
      end
      @(negedge clk);
      in_valid = 0;
      while (outs < row * {beats}) @(negedge clk);
    end
    $display("HELD %0d", held);
    $display("QUIET %0d", quietest);
    $fclose(f);
    $finish;
  end
endmodule
"#,
                logging = u8::from(aborted == 0),
                held = u8::from(aborted > 0),
                abort = if aborted == 0 {
                    String::new()
                } else {
                    format!(
                        "    entries = hs[0][7:0];\n    for (e = 0; e < hs[0][7:0]; e = e + 1)\n      for (b = 0; b < {beats}; b = b + 1) begin\n        @(negedge clk);\n        log_wr = 1; log_at = e; log_beat = b; log_k = keys[hs[0][39:24] + e * {beats} + b]; log_v = vlogs[hs[0][39:24] + e * {beats} + b];\n        log_decay = toks[hs[0][23:8] + e][63:32]; log_beta = toks[hs[0][23:8] + e][31:0];\n      end\n    @(negedge clk);\n    log_wr = 0;\n    for (g = 0; g < {n} * {beats}; g = g + 1) begin\n      @(negedge clk);\n      in_valid = 1; in_beat = beats[g][511:0];\n      while (!in_ready) @(negedge clk);\n      @(posedge clk);\n    end\n    @(negedge clk);\n    in_valid = 0;\n    repeat ({aborted}) @(negedge clk);\n    rst = 1;\n    @(negedge clk);\n    rst = 0; logging = 1;\n"
                    )
                },
                kl = (keys_at.max(1)) - 1,
                tl = (toks_at.max(1)) - 1,
                bl = rows_total * beats - 1,
                hl = heads.len() - 1,
                f16 = f16 as u8,
                stall = FLUSH_STALL,
            );
            std::fs::write(dir.path().join("tb.v"), tb).unwrap();
            let sim = if four_state { rtl_sim::Simulator::icarus(dir.path(), &["tb.v", "gdn.v"]) } else { rtl_sim::Simulator::build(dir.path(), &["tb.v", "gdn.v"]) };
            let run = sim.run(dir.path(), &[]);
            let said = format!("{}{}", String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr));
            assert!(run.status.success() && !said.contains("STALL"), "{n} a row, f16 {f16}: {said}");
            let quiet: u64 = said.split("QUIET ").nth(1).and_then(|q| q.split_whitespace().next()).and_then(|q| q.parse().ok()).expect("the testbench's QUIET line");
            assert!(4 * quiet < FLUSH_STALL, "{n} a row, f16 {f16}: {quiet} quiet clocks, a quarter of the stall bound {FLUSH_STALL} or more");
            let out = std::fs::read_to_string(dir.path().join("out.hex")).unwrap();
            let got: Vec<Vec<u32>> = out
                .lines()
                .map(|l| (0..16).rev().map(|k| u32::from_str_radix(&l[8 * k..8 * k + 8], 16).unwrap()).collect())
                .collect();
            assert_eq!(got.len(), rows_total * beats, "{n} a row, f16 {f16}: every row's beats out");
            let store = |x: f32| if f16 { logicaffeine_compile::model::quant::f16_to_f32(f32_to_f16(x)) } else { x };
            let mut wrong = Vec::new();
            for (h, head) in heads.iter().enumerate() {
                let log: Vec<Logged> = (0..head.keys.len())
                    .map(|e| Logged { k: &head.keys[e], v: &head.values[e], log_decay: head.log_decays[e], beta: head.betas[e] })
                    .collect();
                for (i, row) in head.rows.iter().enumerate() {
                    let mut want = row.clone();
                    gdn_flush(FpUnit::Soft, &mut want, &log, i, store);
                    let at = (h * n + i) * beats;
                    for (x, w) in want.iter().enumerate() {
                        let g = got[at + x / 16][x % 16];
                        if g != w.to_bits() {
                            wrong.push(format!("{n} a row, f16 {f16}, head {h} ({} tokens), row {i} element {x}: pipelined {g:08x}, numerics {:08x}", head.keys.len(), w.to_bits()));
                        }
                    }
                }
            }
            assert!(wrong.is_empty(), "{} elements differ:\n{}", wrong.len(), wrong.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
            let held: usize = String::from_utf8_lossy(&run.stdout).split("HELD ").nth(1).and_then(|s| s.split_whitespace().next()?.parse().ok()).unwrap_or(0);
            assert!(held > 0, "{n} a row: the rows were never held back on their way out");
        }
    }
}

/// Heads' queries folded back through their logs by `logos_gdn_fold_p` under iverilog, one head after another: every
/// element of the folded query and of the collected values `gdn_fold`'s — the tokens newest first, each token's decay
/// through the pipelined exponential — and every token's decay `exp`'s of its log decay, at 128, 32 and 16 elements,
/// logs of sixteen, five, one and no tokens.
#[test]
fn the_streaming_fold_folds_every_head_as_the_numerics_do() {
    if !common::require_tool("iverilog") {
        return;
    }
    for (n, logs) in [(128usize, vec![16usize, 5, 1]), (32, vec![16, 0, 3]), (16, vec![2, 16])] {
        fold_case(n, &logs, 0);
    }
}

/// The fold reset for one clock with a fold in flight — a head's log and query written, its fold started — and started
/// again the clock after over what it holds folds that head, and every head sent after, as the numerics do: at 128 and
/// 32 elements, reset early and late in the fold.
#[test]
fn a_fold_reset_with_a_fold_in_flight_folds_every_head_after_as_the_numerics_do() {
    if !common::require_tool("iverilog") {
        return;
    }
    for (n, logs, aborted) in [(128usize, vec![16usize, 5, 1], 7usize), (128, vec![16, 3], 90), (32, vec![16, 0, 3], 31)] {
        fold_case(n, &logs, aborted);
    }
}

/// [`the_streaming_fold_folds_every_head_as_the_numerics_do`]'s heads of `n` elements with `logs` tokens each; with
/// `aborted` clocks, the first head's log and query written and its fold started, the unit reset for one clock that many
/// clocks in, and the head's fold started again the clock after over the log and query the unit still holds, before
/// the other heads are sent.
fn fold_case(n: usize, logs: &[usize], aborted: usize) {
    const T: usize = 16;
    let specials = [0u32, 0x8000_0000, 1, 0x0080_0000, 0x7f80_0000, 0x7fc0_0000];
    {
        let mut r = Rng(0xf0dd ^ n as u64 ^ (aborted as u64) << 16);
        let value = |r: &mut Rng, magnitude: f32| -> f32 {
            if r.next() % 307 == 0 {
                f32::from_bits(specials[(r.next() % specials.len() as u64) as usize])
            } else {
                (((r.next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0) * magnitude
            }
        };
        struct Head {
            q: Vec<f32>,
            keys: Vec<Vec<f32>>,
            values: Vec<Vec<f32>>,
            log_decays: Vec<f32>,
            betas: Vec<f32>,
        }
        let heads: Vec<Head> = logs
            .iter()
            .map(|&entries| Head {
                q: (0..n).map(|_| value(&mut r, 0.1)).collect(),
                keys: (0..entries).map(|_| (0..n).map(|_| value(&mut r, 0.09)).collect()).collect(),
                values: (0..entries).map(|_| (0..n).map(|_| value(&mut r, 1.0)).collect()).collect(),
                log_decays: (0..entries).map(|_| -value(&mut r, 0.3).abs()).collect(),
                betas: (0..entries).map(|_| value(&mut r, 1.0).abs()).collect(),
            })
            .collect();
        let beats = n / 16;
        let hex = |v: &[f32]| -> String { v.iter().rev().map(|x| format!("{:08x}", x.to_bits())).collect() };
        // The script: per head its log beats ("L token beat key values log-decay gate"), its query's beats ("Q beat
        // data"), and the fold ("F entries").
        let mut script = String::new();
        if aborted > 0 {
            let head = &heads[0];
            for e in 0..head.keys.len() {
                for b in 0..beats {
                    writeln!(script, "L {e:x} {b:x} {} {} {:08x} {:08x}", hex(&head.keys[e][16 * b..16 * b + 16]), hex(&head.values[e][16 * b..16 * b + 16]),
                        head.log_decays[e].to_bits(), head.betas[e].to_bits()).unwrap();
                }
            }
            for b in 0..beats {
                writeln!(script, "Q {b:x} {}", hex(&head.q[16 * b..16 * b + 16])).unwrap();
            }
            writeln!(script, "A {:x} {aborted:x}", head.keys.len()).unwrap();
            writeln!(script, "F {:x}", head.keys.len()).unwrap();
        }
        for head in heads.iter().skip(usize::from(aborted > 0)) {
            for e in 0..head.keys.len() {
                for b in 0..beats {
                    writeln!(script, "L {e:x} {b:x} {} {} {:08x} {:08x}", hex(&head.keys[e][16 * b..16 * b + 16]), hex(&head.values[e][16 * b..16 * b + 16]),
                        head.log_decays[e].to_bits(), head.betas[e].to_bits()).unwrap();
                }
            }
            for b in 0..beats {
                writeln!(script, "Q {b:x} {}", hex(&head.q[16 * b..16 * b + 16])).unwrap();
            }
            writeln!(script, "F {:x}", head.keys.len()).unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("script.txt"), script).unwrap();
        std::fs::write(dir.path().join("gdn.v"), gdn_stream_verilog()).unwrap();
        let tb = format!(
            r#"module tb;
  reg clk = 0, rst = 1;
  always #5 clk = ~clk;
  reg log_wr = 0, q_wr = 0, start = 0;
  reg [7:0] log_at = 0, entries = 0;
  reg [15:0] log_beat = 0, q_beat = 0, rd_beat = 0;
  reg [511:0] log_k = 0, log_v = 0, q_data = 0;
  reg [31:0] log_decay = 0, log_beta = 0;
  reg [7:0] rd_token = 0;
  wire busy;
  wire [511:0] y_beat, carried_beat;
  wire [31:0] decay_out;
  logos_gdn_fold_p #(.N({n}), .T_MAX({T})) u(.clk(clk), .rst(rst), .log_wr(log_wr), .log_at(log_at), .log_beat(log_beat),
    .log_k(log_k), .log_v(log_v), .log_decay(log_decay), .log_beta(log_beta), .q_wr(q_wr), .q_beat(q_beat), .q_data(q_data),
    .start(start), .entries(entries), .busy(busy), .rd_beat(rd_beat), .y_beat(y_beat), .carried_beat(carried_beat),
    .rd_token(rd_token), .decay_out(decay_out));
  integer fi, fo, fd, c, b, waited;
  reg [7:0] tag;
  reg [31:0] a0, a1;
  initial begin
    fi = $fopen("script.txt", "r");
    fo = $fopen("out.txt", "w");
    fd = $fopen("decays.txt", "w");
    repeat (3) @(negedge clk);
    rst = 0;
    while (!$feof(fi)) begin
      c = $fscanf(fi, "%c", tag);
      if (tag == "L") begin
        c = $fscanf(fi, " %h %h %h %h %h %h\n", a0, a1, log_k, log_v, log_decay, log_beta);
        log_at = a0; log_beat = a1; log_wr = 1; @(negedge clk); log_wr = 0;
      end else if (tag == "Q") begin
        c = $fscanf(fi, " %h %h\n", a0, q_data);
        q_beat = a0; q_wr = 1; @(negedge clk); q_wr = 0;
      end else if (tag == "A") begin
        c = $fscanf(fi, " %h %h\n", a0, a1);
        entries = a0; start = 1; @(negedge clk); start = 0;
        repeat (a1) @(negedge clk);
        rst = 1; @(negedge clk); rst = 0;
      end else if (tag == "F") begin
        c = $fscanf(fi, " %h\n", a0);
        entries = a0; start = 1; @(negedge clk); start = 0;
        waited = 0;
        while (busy && waited < 100000) begin @(negedge clk); waited = waited + 1; end
        if (busy) $display("STUCK");
        for (b = 0; b < {beats}; b = b + 1) begin rd_beat = b; #1 $fwrite(fo, "%h %h\n", y_beat, carried_beat); end
        for (b = 0; b < a0; b = b + 1) begin rd_token = b; #1 $fwrite(fd, "%h\n", decay_out); end
        @(negedge clk);
      end
    end
    $fclose(fo);
    $fclose(fd);
    $finish;
  end
endmodule
"#,
        );
        std::fs::write(dir.path().join("tb.v"), tb).unwrap();
        let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "gdn.v"]).output().expect("iverilog");
        assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
        let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
        assert!(run.status.success() && !String::from_utf8_lossy(&run.stdout).contains("STUCK"), "{n} wide: {}", String::from_utf8_lossy(&run.stdout));
        let decays: Vec<u32> = std::fs::read_to_string(dir.path().join("decays.txt")).unwrap().lines().map(|l| u32::from_str_radix(l, 16).unwrap()).collect();
        let want: Vec<u32> = heads.iter().flat_map(|head| head.log_decays.iter().map(|&d| exp(FpUnit::Soft, d).to_bits())).collect();
        assert_eq!(decays, want, "{n} wide: every token's decay exp's of its log decay");
        let out = std::fs::read_to_string(dir.path().join("out.txt")).unwrap();
        let words = |h: &str| -> Vec<u32> { (0..16).rev().map(|k| u32::from_str_radix(&h[8 * k..8 * k + 8], 16).unwrap()).collect() };
        let lines: Vec<(Vec<u32>, Vec<u32>)> = out.lines().map(|l| { let (y, c) = l.split_once(' ').unwrap(); (words(y), words(c)) }).collect();
        assert_eq!(lines.len(), heads.len() * beats, "{n} wide: every head's beats read back");
        let mut wrong = Vec::new();
        for (h, head) in heads.iter().enumerate() {
            let log: Vec<Logged> = (0..head.keys.len())
                .map(|e| Logged { k: &head.keys[e], v: &head.values[e], log_decay: head.log_decays[e], beta: head.betas[e] })
                .collect();
            let (y, carried) = gdn_fold(FpUnit::Soft, &head.q, &log, n);
            for x in 0..n {
                let (gy, gc) = (lines[h * beats + x / 16].0[x % 16], lines[h * beats + x / 16].1[x % 16]);
                if gy != y[x].to_bits() || gc != carried[x].to_bits() {
                    wrong.push(format!("{n} wide, head {h} ({} tokens), element {x}: pipelined y {gy:08x} carried {gc:08x}, numerics {:08x} {:08x}",
                        head.keys.len(), y[x].to_bits(), carried[x].to_bits()));
                }
            }
        }
        assert!(wrong.is_empty(), "{} elements differ:\n{}", wrong.len(), wrong.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
    }
}

/// Batches of heads folded at once by `logos_gdn_fold_m` under iverilog, as many heads a batch as its slots (the add's
/// clocks) or fewer, into alternate banks: every slot's folded query and collected values `gdn_fold`'s of its head and
/// every token's decay `exp`'s, batch after batch over the slots the last left, and each batch's bank as it was after the
/// next batch's fold — at 128, 32 and 16 elements, logs of sixteen, five, one and no tokens, full batches and batches of
/// one and three heads. A bank's beat is read a clock after its address, as block RAM reads.
#[test]
fn the_fold_of_several_heads_at_once_folds_every_head_as_the_numerics_do() {
    if !common::require_tool("iverilog") {
        return;
    }
    use logicaffeine_compile::spatial::decode_engine::rtl::fp32::FADD_LATENCY;
    let h = FADD_LATENCY;
    for (n, batches) in [(128usize, vec![(16usize, h), (5, h), (1, 3)]), (32, vec![(16, h), (0, 2), (3, 1)]), (16, vec![(2, h), (16, h)])] {
        fold_m_case(n, &batches);
    }
}

/// [`the_fold_of_several_heads_at_once_folds_every_head_as_the_numerics_do`]'s batches of heads of `n` elements, each
/// batch `(tokens, heads)`.
fn fold_m_case(n: usize, batches: &[(usize, usize)]) {
    fold_m_case_at(n, &batches.iter().map(|&(tokens, heads)| (tokens, heads, 0)).collect::<Vec<_>>(), false);
}

/// The fold of a trailing flush's heads by `logos_gdn_fold_m` built `TRAILING`: beside every slot's fold over its whole
/// log, its fold over the tokens after the log's oldest `older` (`gdn_fold_trailing`'s newer fold), from either bank,
/// batch after batch — logs of one and two windows of eight with the window behind them, a token past one, and the
/// whole log taken as newer — at 128, 32 and 16 elements.
#[test]
fn the_fold_of_several_heads_at_once_keeps_each_heads_fold_over_its_newer_tokens() {
    if !common::require_tool("iverilog") {
        return;
    }
    use logicaffeine_compile::spatial::decode_engine::rtl::fp32::FADD_LATENCY;
    let h = FADD_LATENCY;
    for (n, batches) in [(128usize, vec![(16usize, h, 8usize), (9, h, 8), (12, 3, 8)]), (32, vec![(10, h, 8), (5, 2, 0), (16, 1, 8)]), (16, vec![(9, h, 8), (16, h, 8)])] {
        fold_m_case_at(n, &batches, true);
    }
}

/// [`fold_m_case`] with each batch `(tokens, heads, older)`: on a unit built `TRAILING` when `trailing`, each slot's
/// fold over its tokens after the oldest `older` read back too.
fn fold_m_case_at(n: usize, batches: &[(usize, usize, usize)], trailing: bool) {
    const T: usize = 16;
    let specials = [0u32, 0x8000_0000, 1, 0x0080_0000, 0x7f80_0000, 0x7fc0_0000];
    let mut r = Rng(0xf01d ^ n as u64);
    let value = |r: &mut Rng, magnitude: f32| -> f32 {
        if r.next() % 307 == 0 {
            f32::from_bits(specials[(r.next() % specials.len() as u64) as usize])
        } else {
            (((r.next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0) * magnitude
        }
    };
    struct Head {
        q: Vec<f32>,
        keys: Vec<Vec<f32>>,
        values: Vec<Vec<f32>>,
        log_decays: Vec<f32>,
        betas: Vec<f32>,
    }
    let beats = n / 16;
    let hex = |v: &[f32]| -> String { v.iter().rev().map(|x| format!("{:08x}", x.to_bits())).collect() };
    let mut heads: Vec<Vec<Head>> = Vec::new();
    // The script: per batch each head's log beats into its slot ("L slot token beat key values log-decay gate"), its
    // query's beats ("Q slot beat data"), the fold into the batch's bank, alternate batches alternate banks, and its
    // read-back ("F tokens heads bank"), then the batch before read back again from the other bank ("R tokens heads
    // bank"). The read-backs in order: each batch's slots, then the batch before's.
    let mut script = String::new();
    let mut order: Vec<usize> = Vec::new();
    for (j, &(entries, count, older)) in batches.iter().enumerate() {
        let batch: Vec<Head> = (0..count)
            .map(|_| Head {
                q: (0..n).map(|_| value(&mut r, 0.1)).collect(),
                keys: (0..entries).map(|_| (0..n).map(|_| value(&mut r, 0.09)).collect()).collect(),
                values: (0..entries).map(|_| (0..n).map(|_| value(&mut r, 1.0)).collect()).collect(),
                log_decays: (0..entries).map(|_| -value(&mut r, 0.3).abs()).collect(),
                betas: (0..entries).map(|_| value(&mut r, 1.0).abs()).collect(),
            })
            .collect();
        for (slot, head) in batch.iter().enumerate() {
            for e in 0..entries {
                for b in 0..beats {
                    writeln!(script, "L {slot:x} {e:x} {b:x} {} {} {:08x} {:08x}", hex(&head.keys[e][16 * b..16 * b + 16]),
                        hex(&head.values[e][16 * b..16 * b + 16]), head.log_decays[e].to_bits(), head.betas[e].to_bits()).unwrap();
                }
            }
            for b in 0..beats {
                writeln!(script, "Q {slot:x} {b:x} {}", hex(&head.q[16 * b..16 * b + 16])).unwrap();
            }
        }
        writeln!(script, "F {entries:x} {count:x} {:x} {older:x}", j % 2).unwrap();
        order.push(j);
        if j > 0 {
            writeln!(script, "R {:x} {:x} {:x} 0", batches[j - 1].0, batches[j - 1].1, (j - 1) % 2).unwrap();
            order.push(j - 1);
        }
        heads.push(batch);
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("script.txt"), script).unwrap();
    std::fs::write(dir.path().join("gdn.v"), gdn_stream_verilog()).unwrap();
    let behind = trailing as u8;
    let read = FOLD_M_READ_LATENCY;
    let tb = format!(
        r#"module tb;
  reg clk = 0, rst = 1;
  always #5 clk = ~clk;
  reg log_wr = 0, q_wr = 0, start = 0, bank = 0, rd_bank = 0;
  reg [7:0] log_slot = 0, q_slot = 0, rd_slot = 0, log_at = 0, entries = 0, rd_token = 0, older = 0;
  reg [15:0] log_beat = 0, q_beat = 0, rd_beat = 0;
  reg [511:0] log_k = 0, log_v = 0, q_data = 0;
  reg [31:0] log_decay = 0, log_beta = 0;
  wire busy;
  wire [511:0] y_beat, carried_beat, ynew_beat, cnew_beat;
  wire [31:0] decay_out;
  logos_gdn_fold_m #(.N({n}), .T_MAX({T}), .TRAILING({behind})) u(.clk(clk), .rst(rst), .log_wr(log_wr), .log_slot(log_slot), .log_at(log_at),
    .log_beat(log_beat), .log_k(log_k), .log_v(log_v), .log_decay(log_decay), .log_beta(log_beta), .q_wr(q_wr),
    .q_slot(q_slot), .q_beat(q_beat), .q_data(q_data), .start(start), .bank(bank), .entries(entries), .older(older), .busy(busy),
    .rd_bank(rd_bank), .rd_slot(rd_slot),
    .rd_beat(rd_beat), .y_beat(y_beat), .carried_beat(carried_beat), .ynew_beat(ynew_beat), .cnew_beat(cnew_beat),
    .rd_token(rd_token), .decay_out(decay_out));
  integer fi, fo, fd, c, b, t, sl, waited;
  reg [7:0] tag;
  reg [31:0] a0, a1, a2, a3;
  initial begin
    fi = $fopen("script.txt", "r");
    fo = $fopen("out.txt", "w");
    fd = $fopen("decays.txt", "w");
    repeat (3) @(negedge clk);
    rst = 0;
    while (!$feof(fi)) begin
      c = $fscanf(fi, "%c", tag);
      if (tag == "L") begin
        c = $fscanf(fi, " %h %h %h %h %h %h %h\n", a2, a0, a1, log_k, log_v, log_decay, log_beta);
        log_slot = a2; log_at = a0; log_beat = a1; log_wr = 1; @(negedge clk); log_wr = 0;
      end else if (tag == "Q") begin
        c = $fscanf(fi, " %h %h %h\n", a2, a0, q_data);
        q_slot = a2; q_beat = a0; q_wr = 1; @(negedge clk); q_wr = 0;
      end else if (tag == "F" || tag == "R") begin
        c = $fscanf(fi, " %h %h %h %h\n", a0, a1, a2, a3);
        if (tag == "F") begin
          entries = a0; bank = a2; older = a3; start = 1; @(negedge clk); start = 0;
          waited = 0;
          while (busy && waited < 100000) begin @(negedge clk); waited = waited + 1; end
          if (busy) $display("STUCK");
        end
        rd_bank = a2;
        for (sl = 0; sl < a1; sl = sl + 1) begin
          rd_slot = sl;
          for (b = 0; b < {beats}; b = b + 1) begin rd_beat = b; repeat ({read}) @(posedge clk); #1 $fwrite(fo, "%h %h %h %h\n", y_beat, carried_beat, ynew_beat, cnew_beat); end
          if (tag == "F") for (t = 0; t < a0; t = t + 1) begin rd_token = t; #1 $fwrite(fd, "%h\n", decay_out); end
        end
        @(negedge clk);
      end
    end
    $fclose(fo);
    $fclose(fd);
    $finish;
  end
endmodule
"#,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "gdn.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    assert!(run.status.success() && !String::from_utf8_lossy(&run.stdout).contains("STUCK"), "{n} wide: {}", String::from_utf8_lossy(&run.stdout));
    let decays: Vec<u32> = std::fs::read_to_string(dir.path().join("decays.txt")).unwrap().lines().map(|l| u32::from_str_radix(l, 16).unwrap()).collect();
    let want: Vec<u32> = heads.iter().flatten().flat_map(|head| head.log_decays.iter().map(|&d| exp(FpUnit::Soft, d).to_bits())).collect();
    assert_eq!(decays, want, "{n} wide: every token's decay exp's of its log decay, slot by slot");
    let out = std::fs::read_to_string(dir.path().join("out.txt")).unwrap();
    let words = |h: &str| -> Vec<u32> { (0..16).rev().map(|k| u32::from_str_radix(&h[8 * k..8 * k + 8], 16).unwrap()).collect() };
    let lines: Vec<[Vec<u32>; 4]> = out.lines().map(|l| { let w: Vec<&str> = l.split(' ').collect(); [words(w[0]), words(w[1]), words(w[2]), words(w[3])] }).collect();
    let all: Vec<(&Head, usize)> = order.iter().flat_map(|&j| heads[j].iter().map(move |head| (head, batches[j].2))).collect();
    assert_eq!(lines.len(), all.len() * beats, "{n} wide: every slot's beats read back, and the batch before's again");
    let mut wrong = Vec::new();
    for (h, &(head, older)) in all.iter().enumerate() {
        let log: Vec<Logged> = (0..head.keys.len())
            .map(|e| Logged { k: &head.keys[e], v: &head.values[e], log_decay: head.log_decays[e], beta: head.betas[e] })
            .collect();
        let [newer, whole] = gdn_fold_trailing(FpUnit::Soft, &head.q, &log, n, older);
        let (y, carried) = if trailing { whole } else { gdn_fold(FpUnit::Soft, &head.q, &log, n) };
        for x in 0..n {
            let got = &lines[h * beats + x / 16];
            let (gy, gc) = (got[0][x % 16], got[1][x % 16]);
            if gy != y[x].to_bits() || gc != carried[x].to_bits() {
                wrong.push(format!("{n} wide, head {h} ({} tokens), element {x}: y {gy:08x} carried {gc:08x}, numerics {:08x} {:08x}",
                    head.keys.len(), y[x].to_bits(), carried[x].to_bits()));
            }
            let (ny, nc) = (got[2][x % 16], got[3][x % 16]);
            if trailing && (ny != newer.0[x].to_bits() || nc != newer.1[x].to_bits()) {
                wrong.push(format!("{n} wide, head {h} ({} tokens past {older}), element {x}: newer y {ny:08x} carried {nc:08x}, numerics {:08x} {:08x}",
                    head.keys.len(), newer.0[x].to_bits(), newer.1[x].to_bits()));
            }
        }
    }
    assert!(wrong.is_empty(), "{} elements differ:\n{}", wrong.len(), wrong.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
}

/// A trailing flush's shares of several heads at once through `logos_gdn_flush_m` under iverilog: per bank, each slot's
/// head's log and its share of rows written, the bank started, the shares replayed through the log's first tokens and
/// streamed out — each row `gdn_flush`'s of its head's row with the row's own value of each token (the share's rows from
/// `first`), stored once — a bank filled while the others replay and stream out, a bank started again as soon as it is
/// free, so the banks replay in the order started whichever each is, the three in turn and out of turn, and all three
/// started and waiting at once (three batches of a head and a token, each started before the first's decay is through
/// the exponential); the stream taken at random clocks: shares of sixteen rows of 128 at a line and half a line a clock, of eight rows of 32; four,
/// three, two and one heads; eight, seven, six, five, four, three, one and no tokens; binary16 and f32.
#[test]
fn the_trailing_flush_replays_the_shares_of_several_heads_at_once_as_the_numerics_do() {
    if !common::require_tool("iverilog") {
        return;
    }
    for (n, w, share, f16, batches) in [
        (128usize, 2usize, 16usize, true, vec![(8usize, 4usize, 32usize, 0usize), (8, 4, 48, 1), (5, 3, 0, 2), (8, 1, 112, 0), (8, 2, 96, 1), (3, 4, 16, 2), (8, 4, 64, 2)]),
        (128, 1, 16, false, vec![(8, 4, 16, 0), (1, 2, 64, 2), (6, 3, 32, 1)]),
        (32, 2, 8, false, vec![(8, 4, 24, 1), (0, 4, 0, 1), (8, 3, 8, 2), (7, 2, 16, 0), (8, 4, 0, 2)]),
        (32, 1, 8, true, vec![(8, 4, 16, 0), (4, 2, 8, 0), (8, 4, 24, 2)]),
        (32, 2, 8, false, vec![(1, 1, 0, 0), (1, 1, 8, 1), (1, 1, 16, 2), (2, 2, 0, 0)]),
    ] {
        flush_m_case(n, w, share, f16, &batches);
    }
}

/// [`the_trailing_flush_replays_the_shares_of_several_heads_at_once_as_the_numerics_do`]'s batches of heads of `n`
/// elements at `16·w` a clock, shares of `share` rows, each batch `(tokens, heads, first, bank)`, a bank filled again only
/// once it is free.
fn flush_m_case(n: usize, w: usize, share: usize, f16: bool, batches: &[(usize, usize, usize, usize)]) {
    const T: usize = 8;
    const SLOTS: usize = 4;
    let mut r = Rng(0xf105 ^ (n * 8 + w) as u64);
    let value = |r: &mut Rng, magnitude: f32| -> f32 { (((r.next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0) * magnitude };
    let stored = |x: f32| if f16 { f16_to_f32(f32_to_f16(x)) } else { x };
    struct Head {
        rows: Vec<Vec<f32>>,
        keys: Vec<Vec<f32>>,
        values: Vec<Vec<f32>>,
        log_decays: Vec<f32>,
        betas: Vec<f32>,
    }
    let (beat, beats) = (16 * w, n / (16 * w));
    let hex = |v: &[f32]| -> String { v.iter().rev().map(|x| format!("{:08x}", x.to_bits())).collect() };
    let mut script = String::new();
    let mut heads: Vec<(Vec<Head>, usize)> = Vec::new();
    let mut used = [false; 3];
    for &(entries, count, first, bank) in batches {
        if used[bank] {
            writeln!(script, "F {bank:x}").unwrap();
        }
        used[bank] = true;
        let batch: Vec<Head> = (0..count)
            .map(|_| Head {
                rows: (0..share).map(|_| (0..n).map(|_| stored(value(&mut r, 0.5))).collect()).collect(),
                keys: (0..entries).map(|_| (0..n).map(|_| value(&mut r, 0.09)).collect()).collect(),
                values: (0..entries).map(|_| (0..n).map(|_| value(&mut r, 1.0)).collect()).collect(),
                log_decays: (0..entries).map(|_| -value(&mut r, 0.3).abs()).collect(),
                betas: (0..entries).map(|_| value(&mut r, 1.0).abs()).collect(),
            })
            .collect();
        for (slot, head) in batch.iter().enumerate() {
            for e in 0..entries {
                for b in 0..beats {
                    writeln!(script, "L {bank:x} {slot:x} {e:x} {b:x} {} {} {:08x} {:08x}", hex(&head.keys[e][beat * b..beat * (b + 1)]),
                        hex(&head.values[e][beat * b..beat * (b + 1)]), head.log_decays[e].to_bits(), head.betas[e].to_bits()).unwrap();
                }
            }
            for (k, row) in head.rows.iter().enumerate() {
                for b in 0..beats {
                    writeln!(script, "W {bank:x} {slot:x} {k:x} {b:x} {}", hex(&row[beat * b..beat * (b + 1)])).unwrap();
                }
            }
        }
        writeln!(script, "S {bank:x} {entries:x} {count:x} {first:x}").unwrap();
        heads.push((batch, first));
    }
    let want: Vec<Vec<f32>> = heads
        .iter()
        .flat_map(|(batch, first)| {
            batch.iter().flat_map(move |head| {
                let log: Vec<Logged> = (0..head.keys.len())
                    .map(|e| Logged { k: &head.keys[e], v: &head.values[e], log_decay: head.log_decays[e], beta: head.betas[e] })
                    .collect();
                head.rows
                    .iter()
                    .enumerate()
                    .map(|(k, row)| {
                        let mut m = row.clone();
                        gdn_flush(FpUnit::Soft, &mut m, &log, first + k, stored);
                        m
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("script.txt"), script).unwrap();
    std::fs::write(dir.path().join("gdn.v"), format!("{}\n{}", gdn_stream_verilog(), flush_m_verilog())).unwrap();
    let bw = 512 * w;
    let tb = format!(
        r#"module tb;
  reg clk = 0, rst = 1;
  always #5 clk = ~clk;
  reg log_wr = 0, row_wr = 0, start = 0, out_ready = 0;
  reg [1:0] log_bank = 0, row_bank = 0, bank = 0;
  reg [7:0] log_slot = 0, log_at = 0, row_slot = 0, entries = 0, slots = 0;
  reg [15:0] log_beat = 0, row_j = 0, row_beat = 0, first = 0;
  reg [{top}:0] log_k = 0, log_v = 0, row_data = 0;
  reg [31:0] log_decay = 0, log_beta = 0;
  wire [2:0] free;
  wire out_valid;
  wire [{top}:0] out_beat;
  wire [{half}:0] out_half;
  logos_gdn_flush_m #(.N({n}), .W({w}), .T_MAX({T}), .SHARE({share}), .SLOTS({SLOTS})) u(.clk(clk), .rst(rst),
    .log_wr(log_wr), .log_bank(log_bank), .log_slot(log_slot), .log_at(log_at), .log_beat(log_beat), .log_k(log_k),
    .log_v(log_v), .log_decay(log_decay), .log_beta(log_beta), .row_wr(row_wr), .row_bank(row_bank), .row_slot(row_slot),
    .row_j(row_j), .row_beat(row_beat), .row_data(row_data), .start(start), .bank(bank), .entries(entries), .slots(slots),
    .first(first), .f16(1'b{f16}), .free(free), .out_valid(out_valid), .out_ready(out_ready), .out_beat(out_beat),
    .out_half(out_half));
  integer fi, fo, c, waited, seed = 5, got = 0;
  reg [7:0] tag;
  reg [31:0] a0, a1, a2, a3;
  always @(negedge clk) begin
    out_ready <= ($random(seed) & 3) != 0;
    if (out_valid) begin $fwrite(fo, "%h\n", out_beat); got = got + 1; end
  end
  initial begin
    fi = $fopen("script.txt", "r");
    fo = $fopen("out.txt", "w");
    repeat (3) @(negedge clk);
    rst = 0;
    @(negedge clk);
    while (!$feof(fi)) begin
      c = $fscanf(fi, "%c", tag);
      if (tag == "L") begin
        c = $fscanf(fi, " %h %h %h %h %h %h %h %h\n", log_bank, a0, a1, a2, log_k, log_v, log_decay, log_beta);
        log_slot = a0; log_at = a1; log_beat = a2; log_wr = 1; @(negedge clk); log_wr = 0;
      end else if (tag == "W") begin
        c = $fscanf(fi, " %h %h %h %h %h\n", row_bank, a0, a1, a2, row_data);
        row_slot = a0; row_j = a1; row_beat = a2; row_wr = 1; @(negedge clk); row_wr = 0;
      end else if (tag == "S") begin
        c = $fscanf(fi, " %h %h %h %h\n", bank, a0, a1, a2);
        entries = a0; slots = a1; first = a2; start = 1; @(negedge clk); start = 0;
      end else if (tag == "F") begin
        c = $fscanf(fi, " %h\n", a0);
        @(negedge clk);
        waited = 0;
        while (!free[a0] && waited < 400000) begin @(negedge clk); waited = waited + 1; end
        if (!free[a0]) $display("STUCK on bank %0d", a0);
      end
    end
    waited = 0;
    while (got < {want_beats} && waited < 400000) begin @(negedge clk); waited = waited + 1; end
    if (got < {want_beats}) $display("STUCK at %0d beats", got);
    repeat (40) @(negedge clk);
    $fclose(fo);
    $finish;
  end
endmodule
"#,
        top = bw - 1,
        half = bw / 2 - 1,
        f16 = f16 as u8,
        want_beats = want.len() * beats,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let what = format!("{n} wide at {} a clock, shares of {share}, f16 {f16}", 16 * w);
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "gdn.v"]).output().expect("iverilog");
    assert!(build.status.success(), "{what}: iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    assert!(run.status.success() && !String::from_utf8_lossy(&run.stdout).contains("STUCK"), "{what}: {}", String::from_utf8_lossy(&run.stdout));
    let out = std::fs::read_to_string(dir.path().join("out.txt")).unwrap();
    let words = |h: &str| -> Vec<u32> { (0..beat).rev().map(|k| u32::from_str_radix(&h[8 * k..8 * k + 8], 16).unwrap()).collect() };
    let lines: Vec<Vec<u32>> = out.lines().map(words).collect();
    assert_eq!(lines.len(), want.len() * beats, "{what}: every share's beats streamed out, and no more");
    let mut wrong = Vec::new();
    for (k, row) in want.iter().enumerate() {
        for x in 0..n {
            let got = lines[k * beats + x / beat][x % beat];
            if got != row[x].to_bits() {
                wrong.push(format!("{what}: row {k} element {x}: {got:08x}, numerics {:08x}", row[x].to_bits()));
            }
        }
    }
    assert!(wrong.is_empty(), "{} elements differ:\n{}", wrong.len(), wrong.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
}

/// The words of a `$writememh` dump, `bytes_a_word` bytes each, byte 0 the word's least significant.
fn read_hex(path: &std::path::Path, bytes_a_word: usize) -> Vec<Vec<u8>> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//") && !l.starts_with('@'))
        .map(|l| {
            let l = l.replace('_', "");
            assert_eq!(l.len(), 2 * bytes_a_word, "{}: a word of {} hex digits", path.display(), l.len());
            (0..bytes_a_word).rev().map(|i| u8::from_str_radix(&l[2 * i..2 * i + 2], 16).expect("a hex byte")).collect()
        })
        .collect()
}

/// One token row's lazily flushed Gated-DeltaNet block through `logos_gdn_side` under iverilog, laid out as the machine
/// lays a block in a region (the value heads' rows, then the log's count line and its entries, each entry's key, values,
/// decay exponents and gates beat-padded): the log's count read from its line, the row's entry appended from the store —
/// each field's lanes past its elements holding anything — as the state stores it, for every value head its key group's
/// query folded back through the log with the row's entry (`gdn_fold`), every row's read-out (`gdn_read`) in the store,
/// when the log fills every row flushed (`gdn_flush`) back where it was read, and the count written back, none once
/// flushed — over a memory that answers each read at random later, takes each write at random and reports it written a
/// while after. The log's lines and the count line after are the machine's byte for byte. A case of `hd`-wide heads,
/// `nk` key heads and `nv` value heads with `count` tokens logged with the row's, f16 or f32, its seed `case`; under
/// iverilog, which sees a register no reset reaches as X, or at the card's width (`verilator`) on [`rtl_sim::Simulator`].
fn gdn_side_case(case: u64, hd: usize, nk: usize, nv: usize, count: usize, f16: bool, verilator: bool) {
    gdn_side_case_writing(case, hd, nk, nv, count, f16, verilator, Writes::Quick)
}

/// How a co-simulated memory answers its reads.
#[derive(Clone, Copy, PartialEq)]
enum Reads {
    /// Asks taken three cycles in four at random, each answered at random later, a line every other cycle on average.
    Random,
    /// Every ask taken, each answered `.0` clocks after it was taken, a line a clock: a memory at its full rate behind
    /// its latency.
    Latency(u32),
}

/// How the side's memory takes its writes.
#[derive(Clone, Copy, PartialEq)]
enum Writes {
    /// Three cycles in four at random, each in memory as it is taken.
    Quick,
    /// One cycle in eight at random, so the side's write queue fills and its log entry's beats wait on its room.
    Slow,
    /// Three cycles in four at random, each landing in memory 64 to 127 clocks after it is taken, in order, the memory
    /// reporting itself written only once none waits: a read the side asks before its write has landed reads the line
    /// as it was.
    Late,
    /// One cycle in thirty-two at random: a replay's write-back outlasts the batches streamed after it.
    Crawl,
}

impl Writes {
    /// The testbench's expression for a clock on which the memory takes a write.
    fn ready(self) -> &'static str {
        match self {
            Writes::Slow => "($random(seed) & 7) == 0",
            Writes::Crawl => "($random(seed) & 31) == 0",
            Writes::Quick | Writes::Late => "($random(seed) & 3) != 0",
        }
    }
}

/// [`gdn_side_case`] over a memory that takes its writes as `writes` says.
#[allow(clippy::too_many_arguments)]
fn gdn_side_case_writing(case: u64, hd: usize, nk: usize, nv: usize, count: usize, f16: bool, verilator: bool, writes: Writes) {
    gdn_side_case_at(case, hd, nk, nv, count, f16, verilator, writes, 1)
}

/// [`gdn_side_case_writing`] on the side built to take `w` sixteens of elements a clock.
#[allow(clippy::too_many_arguments)]
fn gdn_side_case_at(case: u64, hd: usize, nk: usize, nv: usize, count: usize, f16: bool, verilator: bool, writes: Writes, w: usize) {
    if !common::require_tool("iverilog") {
        return;
    }
    let c = SideCase::new(case, hd, nk, nv, count, f16);
    let dir = tempfile::tempdir().unwrap();
    c.write(dir.path(), &gdn_stream_verilog());
    let (q_beat, out_beat, k_beat, v_beat, ld_beat, gt_beat) = c.places();
    let tb = format!(
        r#"module tb;
  reg clk = 0, rst = 1, start = 0;
  always #5 clk = ~clk;
  reg [511:0] mem [0:{last_line}];
  reg [511:0] store [0:{store_last}];
  wire busy, st_we, rq_valid, wq_valid;
  wire [9:0] st_ra, st_wa;
  wire [511:0] st_wd, wq_data;
  wire [31:0] rq_line, wq_line;
  reg [511:0] st_rd = 0, rs_data = 0;
  reg rq_ready = 0, wq_ready = 0, rs_valid = 0;
  reg [3:0] settling = 0;
  // Writes taken and waiting to land: each line, its data and the clock it lands at.
  reg [31:0] lq_line [0:255];
  reg [511:0] lq_data [0:255];
  integer lq_at [0:255];
  reg [7:0] lq_wr = 0, lq_rd = 0;
  integer now = 0;
  logos_gdn_side #(.HD({hd}), .TMAX({T}), .SA(10), .W({w})) u(.clk(clk), .rst(rst), .start(start), .busy(busy),
    .region(32'd{region}), .mem_off(32'd{mem_off}), .log_off(32'd{log_off}), .entry(32'd{entry}), .nk(32'd{nk}), .nv(32'd{nv}),
    .fval(32'd{fval}), .fdec(32'd{fdec}), .fgate(32'd{fgate}), .scale(32'h{scale:08x}), .tokens(8'd{T}), .f16(1'b{f16}),
    .q_beat(10'd{q_beat}), .out_beat(10'd{out_beat}), .k_beat(10'd{k_beat}), .v_beat(10'd{v_beat}), .ld_beat(10'd{ld_beat}),
    .gt_beat(10'd{gt_beat}), .st_ra(st_ra), .st_rd(st_rd), .st_we(st_we), .st_wa(st_wa), .st_wd(st_wd), .rq_valid(rq_valid),
    .rq_ready(rq_ready), .rq_line(rq_line), .rs_valid(rs_valid), .rs_data(rs_data), .wq_valid(wq_valid), .wq_ready(wq_ready),
    .wq_line(wq_line), .wq_data(wq_data), .written(settling == 4'd0 && lq_rd == lq_wr));
  // The memory: a read taken at random, answered in order at random later; a write taken at random and reported
  // written a few clocks after the last lands.
  reg [31:0] pend [0:255];
  reg [7:0] p_wr = 0, p_rd = 0;
  integer seed = 11, waited;
  always @(posedge clk) begin
    st_rd <= store[st_ra];
    if (st_we) store[st_wa] <= st_wd;
    rq_ready <= ($random(seed) & 3) != 0;
    wq_ready <= {wready};
    if (rq_valid && rq_ready) begin pend[p_wr] <= rq_line; p_wr <= p_wr + 8'd1; end
    rs_valid <= 1'b0;
    if (p_rd != p_wr && ($random(seed) & 1)) begin rs_valid <= 1'b1; rs_data <= mem[pend[p_rd]]; p_rd <= p_rd + 8'd1; end
    now <= now + 1;
{take}
    else if (settling != 4'd0) settling <= settling - 4'd1;
    if (lq_rd != lq_wr && now >= lq_at[lq_rd]) begin mem[lq_line[lq_rd]] <= lq_data[lq_rd]; lq_rd <= lq_rd + 8'd1; end
  end
  initial begin
    $readmemh("mem.hex", mem);
    $readmemh("store.hex", store);
    repeat (3) @(negedge clk);
    rst = 0;
    @(negedge clk) start = 1;
    @(negedge clk) start = 0;
    waited = 0;
    while ((busy || p_rd != p_wr || settling != 4'd0 || lq_rd != lq_wr) && waited < 4000000) begin @(negedge clk); waited = waited + 1; end
    if (busy) $display("STUCK");
    $writememh("mem.out", mem);
    $writememh("store.out", store);
    $finish;
  end
endmodule
"#,
        last_line = c.mem.len() / 64 - 1,
        store_last = c.store.len() - 1,
        T = SideCase::TOKENS,
        region = c.region,
        mem_off = c.mem_off,
        log_off = c.log_off,
        entry = c.entry,
        fval = c.f.values,
        fdec = c.f.log_decays,
        fgate = c.f.gates,
        scale = c.scale.to_bits(),
        f16 = f16 as u8,
        wready = writes.ready(),
        take = if writes == Writes::Late {
            "    if (wq_valid && wq_ready) begin\n      lq_line[lq_wr] <= wq_line; lq_data[lq_wr] <= wq_data; lq_at[lq_wr] <= now + 64 + ($random(seed) & 63);\n      lq_wr <= lq_wr + 8'd1; settling <= 4'd3 + ($random(seed) & 7);\n    end"
        } else {
            "    if (wq_valid && wq_ready) begin mem[wq_line] <= wq_data; settling <= 4'd3 + ($random(seed) & 7); end"
        },
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let what = format!("{hd} wide, {nk}/{nv} heads, {count} tokens, flush {}, f16 {f16}, {} a clock", c.flush, 16 * w);
    let out = c.simulate(dir.path(), verilator, &what);
    assert!(!out.contains("STUCK"), "{what}: {out}");
    let wrong = c.wrong(dir.path(), &what);
    assert!(wrong.is_empty(), "{} differ:\n{}", wrong.len(), wrong.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
}

/// One token row's lazily flushed Gated-DeltaNet block laid out in a region as the machine lays one, as
/// [`gdn_side_case`] describes, and what the numerics make of it: the memory and the store a recurrence unit starts
/// from, the block's places, and every token's fields, the rows and the query the results are checked against.
struct SideCase {
    hd: usize,
    nk: usize,
    nv: usize,
    count: usize,
    f16: bool,
    flush: bool,
    /// A trailing flush's window and the slot of the log's oldest token in its ring of two windows; `None` for a flush
    /// every [`SideCase::TOKENS`].
    window: Option<usize>,
    oldest: usize,
    f: logicaffeine_compile::spatial::decode_engine::machine::LogFields,
    entry: usize,
    log_off: usize,
    region: usize,
    mem_off: usize,
    base: usize,
    mem: Vec<u8>,
    store: Vec<[f32; 16]>,
    keys: Vec<Vec<f32>>,
    values: Vec<Vec<f32>>,
    log_decays: Vec<Vec<f32>>,
    betas: Vec<Vec<f32>>,
    rows: Vec<Vec<Vec<f32>>>,
    q: Vec<f32>,
    scale: f32,
    /// The gated norm a recurrence job carries, when this case's row is of one ([`SideCase::normed`]).
    norm: Option<CaseNorm>,
}

/// A side case's gated norm: the norm's weight (a head's `hd` elements, at memory line `w_line`), the gate's elements
/// (every value head's, in the store from [`SideCase::z_place`]) and the norm's epsilon.
struct CaseNorm {
    w: Vec<f32>,
    z: Vec<f32>,
    eps: f32,
    w_line: usize,
    /// Whether the job carries the norm: a job without one on lanes built for norms leaves its read-outs as made.
    on: bool,
}

/// A gated norm's weight of `hd` elements drawn from `seed`: around one, of both signs, an exact zero among them.
fn norm_weight(seed: u64, hd: usize) -> Vec<f32> {
    let mut r = Rng(0x7e19 ^ seed);
    (0..hd)
        .map(|i| match i % 9 {
            0 => 0.0,
            1 => -(((r.next() >> 40) as f32) / (1u64 << 24) as f32 + 0.5),
            _ => ((r.next() >> 40) as f32) / (1u64 << 23) as f32 + 0.25,
        })
        .collect()
}

impl SideCase {
    /// The tokens a log holds before it flushes.
    const TOKENS: usize = 16;
    /// Beats the store holds at least.
    const STORE: usize = 256;
    /// Beats the store's addresses reach (`SA` 10).
    const STORE_REACH: usize = 1024;

    /// Where the store holds each of a case's vectors, one after another from beat 0: the query, the read-outs, then the
    /// row's keys, values, decay exponents and gates.
    fn beats(nk: usize, nv: usize, hd: usize) -> (usize, usize, usize, usize, usize, usize) {
        let span = |n: usize| n.div_ceil(16);
        let q = 0;
        let out = q + span(nk * hd);
        let k = out + span(nv * hd);
        let v = k + span(nk * hd);
        let ld = v + span(nv * hd);
        let gt = ld + span(nv);
        assert!(gt + span(nv) <= SideCase::STORE_REACH, "{nk}/{nv} heads of {hd} fit the store's reach of {} beats", SideCase::STORE_REACH);
        (q, out, k, v, ld, gt)
    }

    /// The beats a case's store holds: [`SideCase::STORE`], or the power of two past it its vectors need.
    fn store_beats(nk: usize, nv: usize, hd: usize) -> usize {
        let (_, _, _, _, _, gt) = SideCase::beats(nk, nv, hd);
        (gt + nv.div_ceil(16)).next_power_of_two().max(SideCase::STORE)
    }

    /// Where this case's store holds its vectors.
    fn places(&self) -> (usize, usize, usize, usize, usize, usize) {
        SideCase::beats(self.nk, self.nv, self.hd)
    }

    /// Where a normed case's store holds the gate's elements: past the gates.
    fn z_place(&self) -> usize {
        let (.., gt) = self.places();
        gt + self.nv.div_ceil(16)
    }

    /// This case's row with its read-outs gated and normed where they are made, as a recurrence job's norm asks: the
    /// norm's weight `w` in lines of its own past the case's memory, the gate's elements in the store past the gates —
    /// drawn from `seed`, of both signs, across the SiLU's flat ends and its bend, exact zeros among them — and `eps`.
    fn normed(mut self, w: &[f32], eps: f32, seed: u64) -> SideCase {
        let (nv, hd) = (self.nv, self.hd);
        assert_eq!(w.len(), hd, "a weight element a head element");
        let mut r = Rng(0x9a7e ^ seed);
        let mut draw = |m: f32| -> f32 { (((r.next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0) * m };
        let z: Vec<f32> = (0..nv * hd)
            .map(|i| match i % 11 {
                0 => 0.0,
                1 => -0.0,
                2 => draw(40.0),
                3 => draw(0.01),
                _ => draw(6.0),
            })
            .collect();
        let at = self.z_place();
        let need = at + (nv * hd).div_ceil(16);
        assert!(need <= SideCase::STORE_REACH, "{nv} heads of {hd} and their gate fit the store's reach");
        if self.store.len() < need {
            self.store.resize(need.next_power_of_two(), [0f32; 16]);
        }
        for (i, &x) in z.iter().enumerate() {
            self.store[at + i / 16][i % 16] = x;
        }
        let w_line = self.mem.len() / 64;
        let mut lines: Vec<u8> = w.iter().flat_map(|x| x.to_bits().to_le_bytes()).collect();
        lines.resize(hd.div_ceil(16) * 64, 0);
        self.mem.extend(lines);
        self.norm = Some(CaseNorm { w: w.to_vec(), z, eps, w_line, on: true });
        self
    }

    /// [`SideCase::normed`]'s row on lanes built for norms in a job that carries none: its weight and gate laid out, its
    /// read-outs left as made.
    fn normed_off(self, w: &[f32], eps: f32, seed: u64) -> SideCase {
        let mut c = self.normed(w, eps, seed);
        c.norm.as_mut().unwrap().on = false;
        c
    }

    /// The norm's ports of the lanes' instance for this case's row: none when it has no norm.
    fn norm_ports(&self, sa: usize, z_beat: usize, w_line: usize) -> String {
        match &self.norm {
            None => String::new(),
            Some(n) => format!(" .norm(1'b{}), .eps(32'h{:08x}), .z_beat({sa}'d{z_beat}), .w_line(32'd{w_line}),", u8::from(n.on), n.eps.to_bits()),
        }
    }

    /// A case of `hd`-wide heads, `nk` key heads and `nv` value heads with `count` tokens logged with the row's, f16 or
    /// f32, its seed `case`.
    fn new(case: u64, hd: usize, nk: usize, nv: usize, count: usize, f16: bool) -> SideCase {
        SideCase::new_on(case, hd, nk, nv, count, f16, None, 0)
    }

    /// [`SideCase::new`] under a trailing flush of `window` tokens, the log's oldest token in slot `oldest` of its ring of
    /// two windows, every slot outside the log holding a stale entry; `None`, a flush every [`SideCase::TOKENS`].
    #[allow(clippy::too_many_arguments)]
    fn new_on(case: u64, hd: usize, nk: usize, nv: usize, count: usize, f16: bool, window: Option<usize>, oldest: usize) -> SideCase {
        use logicaffeine_compile::spatial::decode_engine::machine::LogFields;
        let cap: usize = window.map_or(SideCase::TOKENS, |wn| 2 * wn);
        let flush = window.map_or(count >= cap, |wn| count > wn);
        assert!(count <= cap && oldest < cap && (window.is_some() || oldest == 0), "{count} tokens from slot {oldest} of a log of {cap}");
        let mut r = Rng(0x51de ^ case);
        let stored = |x: f32| if f16 { f16_to_f32(f32_to_f16(x)) } else { x };
        let draw = |r: &mut Rng, magnitude: f32| -> f32 { (((r.next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0) * magnitude };
        let eb = if f16 { 2 } else { 4 };
        let f = LogFields::of(nk, nv, hd);
        let entry = (f.elems * eb).div_ceil(64) * 64;
        let (log_off, region) = (0usize, 3usize);
        let mem_off = 64 + cap * entry;
        let base = region * 64;
        let bytes = base + mem_off + nv * hd * hd * eb;
        // Every token's key heads, value heads, decay exponents and gates; the last is the row's, which the store holds.
        let keys: Vec<Vec<f32>> = (0..count).map(|_| (0..nk * hd).map(|_| draw(&mut r, 0.09)).collect()).collect();
        let values: Vec<Vec<f32>> = (0..count).map(|_| (0..nv * hd).map(|_| draw(&mut r, 1.0)).collect()).collect();
        let log_decays: Vec<Vec<f32>> = (0..count).map(|_| (0..nv).map(|_| -draw(&mut r, 0.3).abs()).collect()).collect();
        let betas: Vec<Vec<f32>> = (0..count).map(|_| (0..nv).map(|_| draw(&mut r, 1.0).abs()).collect()).collect();
        let rows: Vec<Vec<Vec<f32>>> = (0..nv).map(|_| (0..hd).map(|_| (0..hd).map(|_| stored(draw(&mut r, 0.5))).collect()).collect()).collect();
        let q: Vec<f32> = (0..nk * hd).map(|_| draw(&mut r, 0.1)).collect();
        let (q_beat, _, k_beat, v_beat, ld_beat, gt_beat) = SideCase::beats(nk, nv, hd);
        let mut store = vec![[0f32; 16]; SideCase::store_beats(nk, nv, hd)];
        for (i, &x) in q.iter().enumerate() {
            store[q_beat + i / 16][i % 16] = x;
        }
        let last = count - 1;
        for (at, field) in [(k_beat, &keys[last]), (v_beat, &values[last]), (ld_beat, &log_decays[last]), (gt_beat, &betas[last])] {
            for b in 0..field.len().div_ceil(16) {
                for l in 0..16 {
                    let i = 16 * b + l;
                    store[at + b][l] = if i < field.len() { field[i] } else { draw(&mut r, 7.0) };
                }
            }
        }
        let mut c = SideCase {
            hd,
            nk,
            nv,
            count,
            f16,
            flush,
            f,
            entry,
            log_off,
            region,
            mem_off,
            base,
            mem: vec![0u8; bytes.div_ceil(64) * 64],
            store,
            keys,
            values,
            log_decays,
            betas,
            rows,
            q,
            scale: (1.0 / (hd as f64).sqrt()) as f32,
            window,
            oldest,
            norm: None,
        };
        let log_at = c.base + c.log_off;
        if window.is_some() {
            for slot in 0..cap {
                let at = log_at + 64 + slot * entry;
                for b in &mut c.mem[at..at + entry] {
                    *b = (r.next() >> 56) as u8;
                }
            }
        }
        for e in 0..count - 1 {
            let at = log_at + 64 + c.slot(e) * entry;
            let bytes = c.entry_bytes(e);
            c.mem[at..at + entry].copy_from_slice(&bytes);
        }
        c.mem[log_at..log_at + 4].copy_from_slice(&((count - 1) as u32).to_le_bytes());
        c.mem[log_at + 4..log_at + 8].copy_from_slice(&(oldest as u32).to_le_bytes());
        for h in 0..nv {
            for rr in 0..hd {
                for i in 0..hd {
                    let at = base + mem_off + ((h * hd + rr) * hd + i) * eb;
                    let x = c.encode(c.rows[h][rr][i]);
                    c.mem[at..at + eb].copy_from_slice(&x);
                }
            }
        }
        c
    }

    /// The slots the log's ring holds: two windows, or the tokens a flush every [`SideCase::TOKENS`] logs.
    fn capacity(&self) -> usize {
        self.window.map_or(SideCase::TOKENS, |wn| 2 * wn)
    }

    /// The slot of the log's token `e`.
    fn slot(&self, e: usize) -> usize {
        (self.oldest + e) % self.capacity()
    }

    /// Bytes of a state element.
    fn eb(&self) -> usize {
        if self.f16 { 2 } else { 4 }
    }

    /// An element as the state stores it.
    fn encode(&self, x: f32) -> Vec<u8> {
        if self.f16 { f32_to_f16(x).to_le_bytes().to_vec() } else { x.to_bits().to_le_bytes().to_vec() }
    }

    /// An element as the state holds it.
    fn stored(&self, x: f32) -> f32 {
        if self.f16 { f16_to_f32(f32_to_f16(x)) } else { x }
    }

    /// Token `e`'s entry as the state stores it: its fields beat-padded with zeros, then zeros to the entry's end.
    fn entry_bytes(&self, e: usize) -> Vec<u8> {
        let (f, nk, nv, hd) = (self.f, self.nk, self.nv, self.hd);
        let mut logged = vec![0f32; f.elems];
        logged[..nk * hd].copy_from_slice(&self.keys[e]);
        logged[f.values..f.values + nv * hd].copy_from_slice(&self.values[e]);
        logged[f.log_decays..f.log_decays + nv].copy_from_slice(&self.log_decays[e]);
        logged[f.gates..f.gates + nv].copy_from_slice(&self.betas[e]);
        let mut out: Vec<u8> = logged.iter().flat_map(|&x| self.encode(x)).collect();
        out.resize(self.entry, 0);
        out
    }

    /// The lines the log holds when it is full: every entry it takes before it flushes.
    fn log_lines(&self) -> usize {
        self.capacity() * self.entry / 64
    }

    /// The memory (`mem.hex`), the store (`store.hex`) and the units (`gdn.v`, `units`) in `dir`.
    fn write(&self, dir: &std::path::Path, units: &str) {
        let hex_line = |l: &[u8]| -> String { l.iter().rev().map(|b| format!("{b:02x}")).collect() };
        let mem_hex: String = self.mem.chunks(64).map(|l| format!("{}\n", hex_line(l))).collect();
        let store_hex: String = self.store.iter().map(|beat| format!("{}\n", beat.iter().rev().map(|x| format!("{:08x}", x.to_bits())).collect::<String>())).collect();
        std::fs::write(dir.join("mem.hex"), mem_hex).unwrap();
        std::fs::write(dir.join("store.hex"), store_hex).unwrap();
        std::fs::write(dir.join("gdn.v"), units).unwrap();
    }

    /// `tb.v` with `gdn.v` in `dir` run under iverilog, or at the card's width (`verilator`) on [`rtl_sim::Simulator`]:
    /// what it printed, once it ran to its end.
    fn simulate(&self, dir: &std::path::Path, verilator: bool, what: &str) -> String {
        let run = if verilator {
            rtl_sim::Simulator::build(dir, &["tb.v", "gdn.v"]).run(dir, &[])
        } else {
            let build = Command::new("iverilog").current_dir(dir).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "gdn.v"]).output().expect("iverilog");
            assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
            Command::new("vvp").current_dir(dir).arg("sim").output().expect("vvp")
        };
        let out = String::from_utf8_lossy(&run.stdout).to_string();
        assert!(run.status.success(), "{what}: {out}");
        out
    }

    /// What the memory and the store the simulation in `dir` left (`mem.out`, `store.out`) hold other than the
    /// numerics' after the row: the log's entries the machine's with the row's appended and its count line the count
    /// kept (none once flushed), every value head's read-outs `gdn_read`'s of its key group's query folded back through
    /// the log, every row as read or, once the log is full, as `gdn_flush` leaves it, and every other store beat and
    /// memory byte as it was.
    fn wrong(&self, dir: &std::path::Path, what: &str) -> Vec<String> {
        let (hd, nk, nv, count, flush, eb) = (self.hd, self.nk, self.nv, self.count, self.flush, self.eb());
        let (_, out_beat, ..) = self.places();
        let store_out = read_hex(&dir.join("store.out"), 64);
        let mem_out: Vec<u8> = read_hex(&dir.join("mem.out"), 64).concat();
        let mut wrong = Vec::new();
        let outs = out_beat..out_beat + (nv * hd).div_ceil(16);
        for (b, beat) in self.store.iter().enumerate().filter(|(b, _)| !outs.contains(b)) {
            let was: Vec<u8> = beat.iter().flat_map(|x| x.to_bits().to_le_bytes()).collect();
            if store_out[b] != was {
                wrong.push(format!("{what}: store beat {b}, outside the read-outs, moved"));
            }
        }
        let block = self.base..self.base + self.mem_off + nv * hd * hd * eb;
        if let Some(at) = (0..self.mem.len()).find(|i| !block.contains(i) && mem_out[*i] != self.mem[*i]) {
            wrong.push(format!("{what}: memory byte {at}, outside the block, moved"));
        }
        let get = |at: usize| -> f32 {
            if self.f16 {
                f16_to_f32(u16::from_le_bytes([mem_out[at], mem_out[at + 1]]))
            } else {
                f32::from_bits(u32::from_le_bytes(mem_out[at..at + 4].try_into().unwrap()))
            }
        };
        let log_at = self.base + self.log_off;
        // The count kept and the slot of the oldest: none once flushed every window; a window on once a trailing log
        // holds two.
        let dropped = self.window.is_some_and(|wn| count == 2 * wn);
        let (kept, oldest): (u32, u32) = match self.window {
            None => (if flush { 0 } else { count as u32 }, 0),
            Some(wn) if dropped => ((count - wn) as u32, ((self.oldest + wn) % (2 * wn)) as u32),
            Some(_) => (count as u32, self.oldest as u32),
        };
        let mut want_count = vec![0u8; 64];
        want_count[..4].copy_from_slice(&kept.to_le_bytes());
        want_count[4..8].copy_from_slice(&oldest.to_le_bytes());
        if mem_out[log_at..log_at + 64] != want_count[..] {
            wrong.push(format!("{what}: the count line {:02x?}, the machine's kept count {kept} and oldest slot {oldest}", &mem_out[log_at..log_at + 8]));
        }
        for slot in 0..self.capacity() {
            let at = log_at + 64 + slot * self.entry;
            let token = (0..count).find(|&e| self.slot(e) == slot);
            let want = match token {
                Some(e) => self.entry_bytes(e),
                None => self.mem[at..at + self.entry].to_vec(),
            };
            if mem_out[at..at + self.entry] != want[..] {
                let first = mem_out[at..at + self.entry].iter().zip(&want).position(|(x, y)| x != y).unwrap_or(0);
                wrong.push(format!("{what}: log slot {slot} ({token:?}) differs from its byte {first}"));
            }
        }
        // A trailing row's replay: the log's oldest window into the share from row `lo`, the rows below it read through
        // the tokens after the window.
        let (older, lo, share) = match self.window {
            Some(wn) if flush => (wn, (count - wn - 1) * (hd / wn), hd / wn),
            _ => (0, 0, if flush { hd } else { 0 }),
        };
        let held: Vec<(Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>)> = (0..count)
            .map(|e| {
                let s = |v: &Vec<f32>| v.iter().map(|&x| self.stored(x)).collect::<Vec<f32>>();
                (s(&self.keys[e]), s(&self.values[e]), s(&self.log_decays[e]), s(&self.betas[e]))
            })
            .collect();
        for h in 0..nv {
            let g = h % nk;
            let log: Vec<Logged> = held
                .iter()
                .map(|(k, v, d, b)| Logged { k: &k[g * hd..(g + 1) * hd], v: &v[h * hd..(h + 1) * hd], log_decay: d[h], beta: b[h] })
                .collect();
            let folds = gdn_fold_trailing(FpUnit::Soft, &self.q[g * hd..(g + 1) * hd], &log, hd, older);
            let read: Vec<f32> = (0..hd)
                .map(|rr| {
                    let (y, carried) = &folds[usize::from(rr >= lo)];
                    gdn_read(FpUnit::Soft, &self.rows[h][rr], y, carried[rr], self.scale)
                })
                .collect();
            // A normed row's read-outs as the machine's gated norm leaves them: the head RMS-normed by the weight, then each
            // element times the SiLU of its gate.
            let wants: Vec<f32> = match &self.norm {
                Some(n) if n.on => rmsnorm(FpUnit::Soft, &read, &n.w, n.eps)
                    .iter()
                    .zip(&n.z[h * hd..(h + 1) * hd])
                    .map(|(&y, &z)| FpUnit::Soft.mul(y, silu(FpUnit::Soft, z)))
                    .collect(),
                _ => read,
            };
            for rr in 0..hd {
                let want = wants[rr].to_bits();
                let at = h * hd + rr;
                let got = u32::from_le_bytes(store_out[out_beat + at / 16][4 * (at % 16)..4 * (at % 16) + 4].try_into().unwrap());
                if got != want {
                    wrong.push(format!("{what}: head {h} row {rr} read-out: rtl {got:08x}, numerics {want:08x}"));
                }
                let mut row = self.rows[h][rr].clone();
                if (lo..lo + share).contains(&rr) {
                    let by = if self.window.is_some() { older } else { log.len() };
                    gdn_flush(FpUnit::Soft, &mut row, &log[..by], rr, |x| self.stored(x));
                }
                for (i, w) in row.iter().enumerate() {
                    let g = get(self.base + self.mem_off + ((h * hd + rr) * hd + i) * eb);
                    if g.to_bits() != w.to_bits() {
                        wrong.push(format!("{what}: head {h} row {rr} element {i}: rtl {:08x}, numerics {:08x}", g.to_bits(), w.to_bits()));
                        break;
                    }
                }
            }
        }
        wrong
    }
}

/// [`gdn_side_case`]'s row on `logos_gdn_lanes`: the coordinator counting, appending and reading the log into `lanes`
/// head lanes' copies of it (each `log_lines` lines, or as many as the full log takes), lane k taking value heads k,
/// k + lanes, … through a memory port of its own — the coordinator's port and every lane's each a memory as the
/// side's, answering at random, over one memory — the read-outs, the rows and the log after as the numerics' and the
/// machine's; at `w` sixteens of elements a clock, its memories taking their writes as `writes` says. A row whose log
/// would take more lines than a copy holds faults and leaves the memory and the store as they were.
#[allow(clippy::too_many_arguments)]
fn gdn_lanes_case(case: u64, hd: usize, nk: usize, nv: usize, count: usize, f16: bool, verilator: bool, writes: Writes, w: usize, lanes: usize, log_lines: Option<usize>) {
    gdn_lanes_case_on(case, hd, nk, nv, count, f16, verilator, writes, w, lanes, log_lines, 1);
}

/// [`gdn_lanes_case`] with each lane's flushing head split over `flushes` flush units: the clocks the run took.
#[allow(clippy::too_many_arguments)]
fn gdn_lanes_case_on(
    case: u64,
    hd: usize,
    nk: usize,
    nv: usize,
    count: usize,
    f16: bool,
    verilator: bool,
    writes: Writes,
    w: usize,
    lanes: usize,
    log_lines: Option<usize>,
    flushes: usize,
) -> u64 {
    if !common::require_tool("iverilog") {
        return 0;
    }
    lanes_run(&SideCase::new(case, hd, nk, nv, count, f16), verilator, writes, Reads::Random, w, lanes, log_lines, flushes, LANE_CREDIT)
}

/// A trailing flush's row on `logos_gdn_lanes` built for a window of `window` tokens, as [`gdn_lanes_case`] runs a row
/// flushed every [`SideCase::TOKENS`]: `count` tokens logged with the row's, the log's oldest in slot `oldest` of its
/// ring of two windows.
#[allow(clippy::too_many_arguments)]
fn gdn_lanes_trailing_case(case: u64, hd: usize, nk: usize, nv: usize, count: usize, oldest: usize, window: usize, f16: bool, writes: Writes, w: usize, lanes: usize) -> u64 {
    if !common::require_tool("iverilog") {
        return 0;
    }
    lanes_run(&SideCase::new_on(case, hd, nk, nv, count, f16, Some(window), oldest), true, writes, Reads::Random, w, lanes, None, 1, LANE_CREDIT)
}

/// [`gdn_lanes_case_on`]'s run of the row `c` lays out, on the lanes its window builds: the clocks the run took.
#[allow(clippy::too_many_arguments)]
fn lanes_run(
    c: &SideCase,
    verilator: bool,
    writes: Writes,
    reads: Reads,
    w: usize,
    lanes: usize,
    log_lines: Option<usize>,
    flushes: usize,
    credit: usize,
) -> u64 {
    assert!(credit < 256, "each port's memory holds 256 asks, and a port has at most {credit} owed");
    let (hd, nk, nv, count, f16) = (c.hd, c.nk, c.nv, c.count, c.f16);
    let holds = log_lines.unwrap_or(c.log_lines());
    let faults = holds < c.log_lines();
    let dir = tempfile::tempdir().unwrap();
    let (units, tmax, tokens) = match c.window {
        None => (lanes_verilog(), SideCase::TOKENS, SideCase::TOKENS),
        Some(wn) => (lanes_trailing_verilog(wn), 2 * wn, wn),
    };
    let units = if c.norm.is_some() { delta_normed_lanes(&units, hd, FpUnit::Soft) } else { units };
    c.write(dir.path(), &format!("{}\n{}", gdn_stream_verilog(), units));
    let norm_ports = c.norm_ports(10, c.z_place(), c.norm.as_ref().map_or(0, |n| n.w_line));
    let (q_beat, out_beat, k_beat, v_beat, ld_beat, gt_beat) = c.places();
    let lba = (usize::BITS - (holds.max(2) - 1).leading_zeros()) as usize;
    let tb = format!(
        r#"module tb;
  localparam L = {lanes}, P = L + 1;
  reg clk = 0, rst = 1, start = 0, row_valid = 0;
  wire row_ready;
  always #5 clk = ~clk;
  reg [511:0] mem [0:{last_line}];
  reg [511:0] store [0:{store_last}];
  wire busy, st_we, fault;
  wire [9:0] st_ra, st_wa;
  wire [511:0] st_wd;
  reg [511:0] st_rd = 0;
  // Port 0 the coordinator's memory port, port 1 + k lane k's.
  wire [P-1:0] q_valid, w_valid, wrote;
  wire [32*P-1:0] q_line, w_line;
  wire [512*P-1:0] w_data;
  reg [P-1:0] q_ready = 0, w_ready = 0, r_valid = 0;
  reg [512*P-1:0] r_data = 0;
  logos_gdn_lanes #(.HD({hd}), .TMAX({tmax}), .SA(10), .OUTSTANDING({credit}), .W({w}), .L(L), .LBA({lba}), .LOG_LINES({holds}), .FLUSHES({flushes})) u(.clk(clk), .rst(rst),
    .start(start), .busy(busy), .rows(16'd1), .row_valid(row_valid), .row_ready(row_ready), .region(32'd{region}), .mem_off(32'd{mem_off}), .log_off(32'd{log_off}), .entry(32'd{entry}),
    .nk(32'd{nk}), .nv(32'd{nv}), .fval(32'd{fval}), .fdec(32'd{fdec}), .fgate(32'd{fgate}), .scale(32'h{scale:08x}),
    .tokens(8'd{tokens}), .f16(1'b{f16}), .q_beat(10'd{q_beat}), .out_beat(10'd{out_beat}), .k_beat(10'd{k_beat}),
    .v_beat(10'd{v_beat}), .ld_beat(10'd{ld_beat}), .gt_beat(10'd{gt_beat}),{norm_ports} .st_ra(st_ra), .st_rd(st_rd), .st_we(st_we),
    .st_wa(st_wa), .st_wd(st_wd), .rq_valid(q_valid[0]), .rq_ready(q_ready[0]), .rq_line(q_line[31:0]),
    .rs_valid(r_valid[0]), .rs_data(r_data[511:0]), .wq_valid(w_valid[0]), .wq_ready(w_ready[0]), .wq_line(w_line[31:0]),
    .wq_data(w_data[511:0]), .written(wrote[0]), .fault(fault), .l_rq_valid(q_valid[P-1:1]), .l_rq_ready(q_ready[P-1:1]),
    .l_rq_line(q_line[32*P-1:32]), .l_rs_valid(r_valid[P-1:1]), .l_rs_data(r_data[512*P-1:512]), .l_wq_valid(w_valid[P-1:1]),
    .l_wq_ready(w_ready[P-1:1]), .l_wq_line(w_line[32*P-1:32]), .l_wq_data(w_data[512*P-1:512]), .l_written(wrote[P-1:1]));
  // Each port's memory, as the side's: a read taken at random, answered in order at random later; a write taken at
  // random and reported written a few clocks after the last lands.
  reg [31:0] pend [0:256*P-1];
  integer pend_at [0:256*P-1];
  reg [7:0] p_wr [0:P-1];
  reg [7:0] p_rd [0:P-1];
  reg [7:0] lq_wr [0:P-1];
  reg [7:0] lq_rd [0:P-1];
  reg [3:0] settling [0:P-1];
  reg [31:0] lq_line [0:256*P-1];
  reg [511:0] lq_data [0:256*P-1];
  integer lq_at [0:256*P-1];
  integer now = 0, seed = 11, waited, p, q, done;
  genvar g;
  generate
    for (g = 0; g < P; g = g + 1) begin : port
      assign wrote[g] = settling[g] == 4'd0 && lq_rd[g] == lq_wr[g];
    end
  endgenerate
  always @(posedge clk) begin
    if (row_ready) row_valid <= 1'b0;
    st_rd <= store[st_ra];
    if (st_we) store[st_wa] <= st_wd;
    now <= now + 1;
    for (p = 0; p < P; p = p + 1) begin
      q_ready[p] <= {qready};
      w_ready[p] <= {wready};
      if (q_valid[p] && q_ready[p]) begin pend[256*p + p_wr[p]] <= q_line[32*p +: 32]; pend_at[256*p + p_wr[p]] <= now; p_wr[p] <= p_wr[p] + 8'd1; end
      r_valid[p] <= 1'b0;
      if (p_rd[p] != p_wr[p] && {answer}) begin
        r_valid[p] <= 1'b1; r_data[512*p +: 512] <= mem[pend[256*p + p_rd[p]]]; p_rd[p] <= p_rd[p] + 8'd1;
      end
{take}
      else if (settling[p] != 4'd0) settling[p] <= settling[p] - 4'd1;
      if (lq_rd[p] != lq_wr[p] && now >= lq_at[256*p + lq_rd[p]]) begin
        mem[lq_line[256*p + lq_rd[p]]] <= lq_data[256*p + lq_rd[p]]; lq_rd[p] <= lq_rd[p] + 8'd1;
      end
    end
    if (rst) for (p = 0; p < P; p = p + 1) begin
      p_wr[p] <= 8'd0; p_rd[p] <= 8'd0; lq_wr[p] <= 8'd0; lq_rd[p] <= 8'd0; settling[p] <= 4'd0;
    end
  end
  initial begin
    $readmemh("mem.hex", mem);
    $readmemh("store.hex", store);
    repeat (3) @(negedge clk);
    rst = 0;
    @(negedge clk) begin start = 1; row_valid = 1; end
    @(negedge clk) start = 0;
    waited = 0;
    done = 0;
    while (!done && waited < 4000000) begin
      @(negedge clk);
      waited = waited + 1;
      done = !busy;
      for (q = 0; q < P; q = q + 1) if (p_rd[q] != p_wr[q] || settling[q] != 4'd0 || lq_rd[q] != lq_wr[q]) done = 0;
    end
    if (!done) $display("STUCK");
    $display("FAULT %0d", fault);
    $display("WAITED %0d", waited);
    $writememh("mem.out", mem);
    $writememh("store.out", store);
    $finish;
  end
endmodule
"#,
        last_line = c.mem.len() / 64 - 1,
        store_last = c.store.len() - 1,
        region = c.region,
        mem_off = c.mem_off,
        log_off = c.log_off,
        entry = c.entry,
        fval = c.f.values,
        fdec = c.f.log_decays,
        fgate = c.f.gates,
        scale = c.scale.to_bits(),
        f16 = f16 as u8,
        wready = writes.ready(),
        qready = match reads { Reads::Random => "($random(seed) & 3) != 0".to_string(), Reads::Latency(_) => "1'b1".to_string() },
        answer = match reads {
            Reads::Random => "($random(seed) & 1)".to_string(),
            Reads::Latency(l) => format!("now >= pend_at[256*p + p_rd[p]] + {l}"),
        },
        take = if writes == Writes::Late {
            "      if (w_valid[p] && w_ready[p]) begin\n        lq_line[256*p + lq_wr[p]] <= w_line[32*p +: 32]; lq_data[256*p + lq_wr[p]] <= w_data[512*p +: 512];\n        lq_at[256*p + lq_wr[p]] <= now + 64 + ($random(seed) & 63); lq_wr[p] <= lq_wr[p] + 8'd1; settling[p] <= 4'd3 + ($random(seed) & 7);\n      end"
        } else {
            "      if (w_valid[p] && w_ready[p]) begin mem[w_line[32*p +: 32]] <= w_data[512*p +: 512]; settling[p] <= 4'd3 + ($random(seed) & 7); end"
        },
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let what = format!(
        "{lanes} lanes, {hd} wide, {nk}/{nv} heads, {count} tokens{}, flush {} over {flushes}, f16 {f16}, {} a clock, copies of {holds} lines",
        c.window.map_or(String::new(), |wn| format!(" from slot {} of a ring of two windows of {wn}", c.oldest)),
        c.flush,
        16 * w
    );
    let out = c.simulate(dir.path(), verilator, &what);
    assert!(!out.contains("STUCK"), "{what}: {out}");
    assert!(out.contains(&format!("FAULT {}", faults as u8)), "{what}: {out}");
    if faults {
        let mem_out: Vec<u8> = read_hex(&dir.path().join("mem.out"), 64).concat();
        let store_out: Vec<Vec<u8>> = read_hex(&dir.path().join("store.out"), 64);
        let store_in: Vec<Vec<u8>> = c.store.iter().map(|b| b.iter().flat_map(|x| x.to_bits().to_le_bytes()).collect()).collect();
        assert!(mem_out == c.mem, "{what}: the memory moved");
        assert!(store_out == store_in, "{what}: the store moved");
        return 0;
    }
    let wrong = c.wrong(dir.path(), &what);
    assert!(wrong.is_empty(), "{} differ:\n{}", wrong.len(), wrong.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
    out.split("WAITED ").nth(1).and_then(|s| s.split_whitespace().next()).and_then(|n| n.parse().ok()).expect("the clocks the run took")
}

/// A job of `cs.len()` token rows on the head lanes (`logos_gdn_lanes`), row k the case `cs[k]` in a region of its own
/// and a stretch of the store of its own, its descriptor offered as the lanes take the rows before it: every row's block,
/// log, count line and read-outs as [`SideCase::wrong`] has them for that case alone, and every byte of memory and beat
/// of the store outside every row's block and read-outs as it was. The rows share their block's shape. The clocks the
/// job took.
fn lanes_rows_run(cs: &[SideCase], writes: Writes, reads: Reads, w: usize, lanes: usize, credit: usize) -> u64 {
    let c0 = &cs[0];
    let shape = |c: &SideCase| (c.hd, c.nk, c.nv, c.f16, c.window, c.entry, c.mem_off, c.log_off);
    assert!(cs.iter().all(|c| shape(c) == shape(c0)), "a job's rows share their block's shape");
    let (hd, nk, nv, f16, rows) = (c0.hd, c0.nk, c0.nv, c0.f16, cs.len());
    let holds = c0.log_lines();
    let lba = (usize::BITS - (holds.max(2) - 1).leading_zeros()) as usize;
    let (units, tmax, tokens) = match c0.window {
        None => (lanes_verilog(), SideCase::TOKENS, SideCase::TOKENS),
        Some(wn) => (lanes_trailing_verilog(wn), 2 * wn, wn),
    };
    let units = if c0.norm.is_some() { delta_normed_lanes(&units, hd, FpUnit::Soft) } else { units };
    // Row k's block from region regions[k] (a line past the row before's), its vectors from beat k · stretch.
    let mut regions = Vec::new();
    let mut at = 3usize;
    for c in cs {
        regions.push(at);
        let own = c.norm.as_ref().map_or(c.mem.len(), |n| n.w_line * 64);
        at += (own - c.base).div_ceil(64) + 1;
    }
    // A normed job's weight, the same for every row, in lines of its own past the last row's block.
    let norm = c0.norm.as_ref().map(|n| (n.w.clone(), n.eps, n.on));
    assert!(cs.iter().all(|c| c.norm.as_ref().map(|n| (n.w.clone(), n.eps, n.on)) == norm), "a job's rows share its norm");
    let w_line = at;
    if norm.is_some() {
        at += hd.div_ceil(16);
    }
    let mut mem = vec![0u8; at * 64];
    for (c, &r) in cs.iter().zip(&regions) {
        let own = c.norm.as_ref().map_or(c.mem.len(), |n| n.w_line * 64);
        mem[r * 64..r * 64 + own - c.base].copy_from_slice(&c.mem[c.base..own]);
    }
    if let Some((w, _, _)) = &norm {
        for (i, x) in w.iter().enumerate() {
            mem[w_line * 64 + 4 * i..w_line * 64 + 4 * i + 4].copy_from_slice(&x.to_bits().to_le_bytes());
        }
    }
    let stretch = cs.iter().map(|c| c.store.len()).max().unwrap().max(SideCase::store_beats(nk, nv, hd));
    let norm_ports = match &norm {
        None => String::new(),
        Some((_, eps, on)) => format!(" .norm(1'b{}), .eps(32'h{:08x}), .z_beat(d_z[di < R ? di : 0]), .w_line(32'd{w_line}),", u8::from(*on), eps.to_bits()),
    };
    let beats = (rows * stretch).next_power_of_two().max(SideCase::STORE_REACH);
    let sa = beats.trailing_zeros() as usize;
    let mut store = vec![[0f32; 16]; beats];
    for (k, c) in cs.iter().enumerate() {
        store[k * stretch..k * stretch + c.store.len()].copy_from_slice(&c.store);
    }
    let dir = tempfile::tempdir().unwrap();
    let hex_line = |l: &[u8]| -> String { l.iter().rev().map(|b| format!("{b:02x}")).collect() };
    let beat_bytes = |beat: &[f32; 16]| -> Vec<u8> { beat.iter().flat_map(|x| x.to_bits().to_le_bytes()).collect() };
    std::fs::write(dir.path().join("mem.hex"), mem.chunks(64).map(|l| format!("{}\n", hex_line(l))).collect::<String>()).unwrap();
    std::fs::write(dir.path().join("store.hex"), store.iter().map(|b| format!("{}\n", hex_line(&beat_bytes(b)))).collect::<String>()).unwrap();
    std::fs::write(dir.path().join("gdn.v"), format!("{}\n{}", gdn_stream_verilog(), units)).unwrap();
    let (q, out, kb, vb, ld, gt) = c0.places();
    let descriptors: String = (0..rows)
        .map(|k| {
            let o = k * stretch;
            format!(
                "    d_region[{k}] = 32'd{}; d_q[{k}] = {sa}'d{}; d_out[{k}] = {sa}'d{}; d_k[{k}] = {sa}'d{}; d_v[{k}] = {sa}'d{}; d_ld[{k}] = {sa}'d{}; d_gt[{k}] = {sa}'d{}; d_z[{k}] = {sa}'d{};\n",
                regions[k], o + q, o + out, o + kb, o + vb, o + ld, o + gt, o + c0.z_place()
            )
        })
        .collect();
    let tb = format!(
        r#"module tb;
  localparam L = {lanes}, P = L + 1, R = {rows};
  reg clk = 0, rst = 1, start = 0, go = 0;
  always #5 clk = ~clk;
  reg [511:0] mem [0:{last_line}];
  reg [511:0] store [0:{store_last}];
  reg [31:0] d_region [0:R-1];
  reg [{st}:0] d_q [0:R-1];
  reg [{st}:0] d_out [0:R-1];
  reg [{st}:0] d_k [0:R-1];
  reg [{st}:0] d_v [0:R-1];
  reg [{st}:0] d_ld [0:R-1];
  reg [{st}:0] d_gt [0:R-1];
  reg [{st}:0] d_z [0:R-1];
  integer di = 0;
  wire row_valid = go && di < R;
  wire row_ready;
  wire busy, st_we, fault;
  wire [{st}:0] st_ra, st_wa;
  wire [511:0] st_wd;
  reg [511:0] st_rd = 0;
  wire [P-1:0] q_valid, w_valid, wrote;
  wire [32*P-1:0] q_line, w_line;
  wire [512*P-1:0] w_data;
  reg [P-1:0] q_ready = 0, w_ready = 0, r_valid = 0;
  reg [512*P-1:0] r_data = 0;
  logos_gdn_lanes #(.HD({hd}), .TMAX({tmax}), .SA({sa}), .OUTSTANDING({credit}), .W({w}), .L(L), .LBA({lba}), .LOG_LINES({holds})) u(.clk(clk), .rst(rst),
    .start(start), .busy(busy), .rows(16'd{rows}), .row_valid(row_valid), .row_ready(row_ready), .region(d_region[di < R ? di : 0]),
    .mem_off(32'd{mem_off}), .log_off(32'd{log_off}), .entry(32'd{entry}), .nk(32'd{nk}), .nv(32'd{nv}), .fval(32'd{fval}),
    .fdec(32'd{fdec}), .fgate(32'd{fgate}), .scale(32'h{scale:08x}), .tokens(8'd{tokens}), .f16(1'b{f16}),
    .q_beat(d_q[di < R ? di : 0]), .out_beat(d_out[di < R ? di : 0]), .k_beat(d_k[di < R ? di : 0]), .v_beat(d_v[di < R ? di : 0]),
    .ld_beat(d_ld[di < R ? di : 0]), .gt_beat(d_gt[di < R ? di : 0]),{norm_ports} .st_ra(st_ra), .st_rd(st_rd), .st_we(st_we),
    .st_wa(st_wa), .st_wd(st_wd), .rq_valid(q_valid[0]), .rq_ready(q_ready[0]), .rq_line(q_line[31:0]),
    .rs_valid(r_valid[0]), .rs_data(r_data[511:0]), .wq_valid(w_valid[0]), .wq_ready(w_ready[0]), .wq_line(w_line[31:0]),
    .wq_data(w_data[511:0]), .written(wrote[0]), .fault(fault), .l_rq_valid(q_valid[P-1:1]), .l_rq_ready(q_ready[P-1:1]),
    .l_rq_line(q_line[32*P-1:32]), .l_rs_valid(r_valid[P-1:1]), .l_rs_data(r_data[512*P-1:512]), .l_wq_valid(w_valid[P-1:1]),
    .l_wq_ready(w_ready[P-1:1]), .l_wq_line(w_line[32*P-1:32]), .l_wq_data(w_data[512*P-1:512]), .l_written(wrote[P-1:1]));
  reg [31:0] pend [0:256*P-1];
  integer pend_at [0:256*P-1];
  reg [7:0] p_wr [0:P-1];
  reg [7:0] p_rd [0:P-1];
  reg [7:0] lq_wr [0:P-1];
  reg [7:0] lq_rd [0:P-1];
  reg [3:0] settling [0:P-1];
  reg [31:0] lq_line [0:256*P-1];
  reg [511:0] lq_data [0:256*P-1];
  integer lq_at [0:256*P-1];
  integer now = 0, seed = 11, waited, p, q, done;
  genvar g;
  generate
    for (g = 0; g < P; g = g + 1) begin : port
      assign wrote[g] = settling[g] == 4'd0 && lq_rd[g] == lq_wr[g];
    end
  endgenerate
  always @(posedge clk) begin
    if (row_ready) di <= di + 1;
    st_rd <= store[st_ra];
    if (st_we) store[st_wa] <= st_wd;
    now <= now + 1;
    for (p = 0; p < P; p = p + 1) begin
      q_ready[p] <= {qready};
      w_ready[p] <= {wready};
      if (q_valid[p] && q_ready[p]) begin pend[256*p + p_wr[p]] <= q_line[32*p +: 32]; pend_at[256*p + p_wr[p]] <= now; p_wr[p] <= p_wr[p] + 8'd1; end
      r_valid[p] <= 1'b0;
      if (p_rd[p] != p_wr[p] && {answer}) begin
        r_valid[p] <= 1'b1; r_data[512*p +: 512] <= mem[pend[256*p + p_rd[p]]]; p_rd[p] <= p_rd[p] + 8'd1;
      end
{take}
      else if (settling[p] != 4'd0) settling[p] <= settling[p] - 4'd1;
      if (lq_rd[p] != lq_wr[p] && now >= lq_at[256*p + lq_rd[p]]) begin
        mem[lq_line[256*p + lq_rd[p]]] <= lq_data[256*p + lq_rd[p]]; lq_rd[p] <= lq_rd[p] + 8'd1;
      end
    end
    if (rst) for (p = 0; p < P; p = p + 1) begin
      p_wr[p] <= 8'd0; p_rd[p] <= 8'd0; lq_wr[p] <= 8'd0; lq_rd[p] <= 8'd0; settling[p] <= 4'd0;
    end
  end
  initial begin
{descriptors}    $readmemh("mem.hex", mem);
    $readmemh("store.hex", store);
    repeat (3) @(negedge clk);
    rst = 0;
    @(negedge clk) begin start = 1; go = 1; end
    @(negedge clk) start = 0;
    waited = 0;
    done = 0;
    while (!done && waited < 8000000) begin
      @(negedge clk);
      waited = waited + 1;
      done = !busy && di == R;
      for (q = 0; q < P; q = q + 1) if (p_rd[q] != p_wr[q] || settling[q] != 4'd0 || lq_rd[q] != lq_wr[q]) done = 0;
    end
    if (!done) $display("STUCK %0d rows taken", di);
    $display("FAULT %0d", fault);
    $display("WAITED %0d", waited);
    $writememh("mem.out", mem);
    $writememh("store.out", store);
    $finish;
  end
endmodule
"#,
        last_line = mem.len() / 64 - 1,
        store_last = beats - 1,
        st = sa - 1,
        mem_off = c0.mem_off,
        log_off = c0.log_off,
        entry = c0.entry,
        fval = c0.f.values,
        fdec = c0.f.log_decays,
        fgate = c0.f.gates,
        scale = c0.scale.to_bits(),
        f16 = f16 as u8,
        wready = writes.ready(),
        qready = match reads { Reads::Random => "($random(seed) & 3) != 0".to_string(), Reads::Latency(_) => "1'b1".to_string() },
        answer = match reads {
            Reads::Random => "($random(seed) & 1)".to_string(),
            Reads::Latency(l) => format!("now >= pend_at[256*p + p_rd[p]] + {l}"),
        },
        take = if writes == Writes::Late {
            "      if (w_valid[p] && w_ready[p]) begin\n        lq_line[256*p + lq_wr[p]] <= w_line[32*p +: 32]; lq_data[256*p + lq_wr[p]] <= w_data[512*p +: 512];\n        lq_at[256*p + lq_wr[p]] <= now + 64 + ($random(seed) & 63); lq_wr[p] <= lq_wr[p] + 8'd1; settling[p] <= 4'd3 + ($random(seed) & 7);\n      end"
        } else {
            "      if (w_valid[p] && w_ready[p]) begin mem[w_line[32*p +: 32]] <= w_data[512*p +: 512]; settling[p] <= 4'd3 + ($random(seed) & 7); end"
        },
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let what = format!(
        "{rows} rows on {lanes} lanes, {hd} wide, {nk}/{nv} heads, counts {:?}{}, f16 {f16}, {} a clock",
        cs.iter().map(|c| c.count).collect::<Vec<_>>(),
        c0.window.map_or(String::new(), |wn| format!(" from slots {:?} of rings of two windows of {wn}", cs.iter().map(|c| c.oldest).collect::<Vec<_>>())),
        16 * w
    );
    let printed = c0.simulate(dir.path(), true, &what);
    assert!(!printed.contains("STUCK"), "{what}: {printed}");
    assert!(printed.contains("FAULT 0"), "{what}: {printed}");
    let mem_out: Vec<u8> = read_hex(&dir.path().join("mem.out"), 64).concat();
    let store_out: Vec<Vec<u8>> = read_hex(&dir.path().join("store.out"), 64);
    let mut wrong = Vec::new();
    let own = |c: &SideCase| c.norm.as_ref().map_or(c.mem.len(), |n| n.w_line * 64);
    let blocks: Vec<std::ops::Range<usize>> = cs.iter().zip(&regions).map(|(c, &r)| r * 64..r * 64 + own(c) - c.base).collect();
    if let Some(i) = (0..mem.len()).find(|i| !blocks.iter().any(|b| b.contains(i)) && mem_out[*i] != mem[*i]) {
        wrong.push(format!("{what}: memory byte {i}, outside every row's block, moved"));
    }
    let outs: Vec<std::ops::Range<usize>> = (0..rows).map(|k| k * stretch + out..k * stretch + out + (nv * hd).div_ceil(16)).collect();
    if let Some(b) = (0..beats).find(|b| !outs.iter().any(|o| o.contains(b)) && store_out[*b] != beat_bytes(&store[*b])) {
        wrong.push(format!("{what}: store beat {b}, outside every row's read-outs, moved"));
    }
    for (k, (c, &r)) in cs.iter().zip(&regions).enumerate() {
        let view = tempfile::tempdir().unwrap();
        let mut m = c.mem.clone();
        m[c.base..own(c)].copy_from_slice(&mem_out[r * 64..r * 64 + own(c) - c.base]);
        std::fs::write(view.path().join("mem.out"), m.chunks(64).map(|l| format!("{}\n", hex_line(l))).collect::<String>()).unwrap();
        let st: String = (0..c.store.len()).map(|b| format!("{}\n", hex_line(&store_out[k * stretch + b]))).collect();
        std::fs::write(view.path().join("store.out"), st).unwrap();
        wrong.extend(c.wrong(view.path(), &format!("row {k} of {what}")));
    }
    assert!(wrong.is_empty(), "{} differ:\n{}", wrong.len(), wrong.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
    printed.split("WAITED ").nth(1).and_then(|s| s.split_whitespace().next()).and_then(|n| n.parse().ok()).expect("the clocks the run took")
}

/// `text` with each module defined once, its first definition kept: generated texts that each carry the modules they are
/// built from.
fn modules_once(text: &str) -> String {
    let starts: Vec<usize> = text.match_indices("\nmodule ").map(|(i, _)| i + 1).collect();
    let mut out = String::from(&text[..starts.first().copied().unwrap_or(text.len())]);
    let mut seen = std::collections::BTreeSet::new();
    for (k, &at) in starts.iter().enumerate() {
        let end = at + text[at..].find("endmodule").expect("the module's end") + "endmodule".len();
        let next = starts.get(k + 1).copied().unwrap_or(text.len());
        let name: String = text[at + "module ".len()..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        if seen.insert(name) {
            out.push_str(&text[at..end]);
        }
        out.push_str(&text[end..next]);
    }
    out
}

/// The convolution path (`logos_conv_path`) alone convolves each row as `numerics::conv` does, bit for bit, its ring
/// rows left as the machine leaves them: the kernel's four taps read once, each row's three ring rows (oldest first, its
/// sequence's ring index on) and its x streamed through the lanes, the output beat written once its last lane is out and
/// x, in the state's format, into the ring's oldest row — 64 to 1024 channels, one to thirteen rows of four sequences
/// (a sequence's next row reading the ring row its last wrote), binary16 and f32 rings, one to sixteen lanes, against a
/// memory that answers late and a quarter of its clocks not ready and a line queue that lands late. Each case's first
/// channels are crafted: every product a negative zero (the sum from +0 makes it +0), x infinite, a NaN in the ring,
/// binary16 subnormals in the ring, products that overflow.
#[test]
fn the_conv_path_convolves_each_row_as_numerics_conv_does() {
    conv_path_convolves_as_numerics_does(1);
}

/// The convolution path behind a store whose reads take two to four clocks (the pipelined token-row store's four,
/// `STORE_READ_LATENCY`) convolves each row as behind a store of one, bit for bit: every case of
/// [`the_conv_path_convolves_each_row_as_numerics_conv_does`], each x beat asked only while the path's queue holds room
/// for every beat on its way from the store and every one queued.
#[test]
fn the_conv_path_convolves_each_row_as_numerics_conv_does_behind_a_store_of_two_to_four_clocks() {
    for srl in [2, 3, 4] {
        conv_path_convolves_as_numerics_does(srl);
    }
}

/// [`the_conv_path_convolves_each_row_as_numerics_conv_does`]'s cases behind a store whose reads take `srl` clocks.
fn conv_path_convolves_as_numerics_does(srl: usize) {
    use logicaffeine_compile::spatial::decode_engine::numerics::conv;
    use logicaffeine_compile::spatial::decode_engine::rtl::{ram, vector_side};
    if !common::require_tool("iverilog") {
        return;
    }
    let units = modules_once(&format!("{}{}\n{}", ram::RAMSTYLE, vector_side::vector_side_verilog(), vector_side::conv_path_verilog()));
    for (case, (c, rows, f16, lanes)) in [(64usize, 1usize, true, 4usize), (96, 3, true, 4), (256, 8, true, 4), (64, 13, false, 4), (128, 8, false, 2), (256, 5, true, 1), (64, 8, true, 8), (128, 6, true, 16), (1024, 3, true, 4), (1024, 3, false, 4), (512, 9, true, 8), (96, 7, false, 16)].into_iter().enumerate() {
        let beats = c / 16;
        let mut r = Rng(0xc0c0 ^ case as u64);
        let mut val = |r: &mut Rng| -> f32 {
            let x = r.next();
            match x % 16 {
                0 => 0.0,
                1 => -0.0,
                _ => (((x >> 8) as f32 / (1u64 << 56) as f32) - 0.5) * 2f32.powi(((x >> 4) % 12) as i32 - 6),
            }
        };
        let sb = if f16 { 2 } else { 4 };
        let (row_bytes, ring_off, kl) = (c * sb, 64 * 5, 16usize);
        let seqs: Vec<usize> = (0..rows).map(|k| [0usize, 0, 1, 2, 1, 3, 0, 2][k % 8]).collect();
        let nseq = seqs.iter().max().unwrap() + 1;
        let region = |s: usize| 1024 + s * ((ring_off + 3 * row_bytes) / 64 + 8);
        let mut mem = vec![[0u8; 64]; 8192];
        let put = |mem: &mut Vec<[u8; 64]>, at: usize, b: &[u8]| mem[at / 64][at % 64..at % 64 + b.len()].copy_from_slice(b);
        // The kernel, tap-major: channel 0's every tap -1 (its products negative zeros), channel 4's 3e38 (overflow).
        let kernel: Vec<Vec<f32>> = (0..4).map(|_| (0..c).map(|ch| match ch { 0 => -1.0, 4 => 3.0e38, 1 => 1.0, _ => val(&mut r) }).collect()).collect();
        for (j, tap) in kernel.iter().enumerate() {
            for (ch, &k) in tap.iter().enumerate() {
                put(&mut mem, (kl + j * beats) * 64 + ch * 4, &k.to_bits().to_le_bytes());
            }
        }
        // The rings in the state's format: channel 0 +0, channel 2 a NaN in the newest row, channel 3 subnormals,
        // channel 4 the format's largest.
        let mut ring: Vec<Vec<Vec<f32>>> = Vec::new();
        for s in 0..nseq {
            let mut rs = Vec::new();
            for idx in 0..3 {
                let mut row = Vec::new();
                for ch in 0..c {
                    let x = match ch {
                        0 => 0.0,
                        2 if idx == 2 => f32::NAN,
                        3 => 2f32.powi(-24) * (1 + idx) as f32,
                        4 => 65504.0,
                        _ => val(&mut r),
                    };
                    let at = region(s) * 64 + ring_off + idx * row_bytes + ch * sb;
                    if f16 {
                        let h = f32_to_f16(x);
                        put(&mut mem, at, &h.to_le_bytes());
                        row.push(f16_to_f32(h));
                    } else {
                        put(&mut mem, at, &x.to_bits().to_le_bytes());
                        row.push(x);
                    }
                }
                rs.push(row);
            }
            ring.push(rs);
        }
        let start_mem = mem.clone();
        let mut next_ring: Vec<usize> = (0..nseq).map(|_| (r.next() % 3) as usize).collect();
        let (x_base, x_stride, o_base, o_stride) = (0usize, beats + 3, 2048usize, beats + 1);
        let mut store = vec![[0u8; 64]; 4096];
        let (mut want_out, mut rowt) = (Vec::new(), String::new());
        for k in 0..rows {
            let s = seqs[k];
            let ri = next_ring[s];
            next_ring[s] = (ri + 1) % 3;
            writeln!(rowt, "{:016x}{:016x}", ri as u64, region(s) as u64).unwrap();
            let x: Vec<f32> = (0..c).map(|ch| match ch { 0 => 0.0, 1 => f32::INFINITY, _ => val(&mut r) }).collect();
            for (ch, xv) in x.iter().enumerate() {
                let at = (x_base + k * x_stride) * 64 + ch * 4;
                store[at / 64][at % 64..at % 64 + 4].copy_from_slice(&xv.to_bits().to_le_bytes());
            }
            let ages: Vec<Vec<f32>> = (0..3).map(|age| ring[s][(ri + age) % 3].clone()).collect();
            let channel_major: Vec<f32> = (0..c).flat_map(|ch| (0..4).map(move |j| (ch, j))).map(|(ch, j)| kernel[j][ch]).collect();
            want_out.push(conv(FpUnit::Soft, &ages, &x, &channel_major, 4));
            for (ch, &xv) in x.iter().enumerate() {
                let at = region(s) * 64 + ring_off + ri * row_bytes + ch * sb;
                if f16 {
                    let h = f32_to_f16(xv);
                    put(&mut mem, at, &h.to_le_bytes());
                    ring[s][ri][ch] = f16_to_f32(h);
                } else {
                    put(&mut mem, at, &xv.to_bits().to_le_bytes());
                    ring[s][ri][ch] = xv;
                }
            }
        }
        let hexl = |l: &[u8; 64]| l.iter().rev().map(|b| format!("{b:02x}")).collect::<String>();
        let lines = |m: &[[u8; 64]]| m.iter().map(|l| format!("{}\n", hexl(l))).collect::<String>();
        let r0 = region(0);
        let r_end = region(nseq - 1) + (ring_off + 3 * row_bytes) / 64 + 1;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("mem.hex"), lines(&start_mem)).unwrap();
        std::fs::write(dir.path().join("want_ring.hex"), lines(&mem[r0..r_end])).unwrap();
        let mut wo = String::new();
        for row in &want_out {
            for b in 0..beats {
                let mut l = [0u8; 64];
                for i in 0..16 {
                    l[4 * i..4 * i + 4].copy_from_slice(&row[16 * b + i].to_bits().to_le_bytes());
                }
                writeln!(wo, "{}", hexl(&l)).unwrap();
            }
        }
        std::fs::write(dir.path().join("want_out.hex"), wo).unwrap();
        std::fs::write(dir.path().join("store.hex"), lines(&store)).unwrap();
        std::fs::write(dir.path().join("rows.hex"), rowt).unwrap();
        let cfg = [rows, f16 as usize, kl, beats, x_base, x_stride, o_base, o_stride, ring_off, 3, row_bytes, r_end - r0, r0, 0, 0, 0];
        std::fs::write(dir.path().join("cfg.hex"), cfg.iter().map(|v| format!("{v:x}\n")).collect::<String>()).unwrap();
        std::fs::write(dir.path().join("conv.v"), &units).unwrap();
        let cvb = beats;
        std::fs::write(dir.path().join("tb.v"), format!(r#"// The conv path alone: a store and a memory (each read a line a request, back in order after a random latency, a
// quarter of the clocks not ready), the side's line queue applied to the memory a few clocks later (settled once none is
// pending), the row table; every output beat and ring row compared with the machine's after the job.
module tb;
  localparam SA = 12, CVB = {cvb};
  reg clk = 0, rst = 1;
  always #5 clk = ~clk;
  reg [31:0] cfg [0:15];
  reg [511:0] mem [0:8191];
  reg [511:0] store [0:4095];
  reg [127:0] rowt [0:15];
  reg [511:0] want_out [0:1023];
  reg [511:0] want_ring [0:4095];
  reg start = 0;
  wire busy, st_we, rq_valid, q_v, flush;
  wire [15:0] row_k;
  wire [SA-1:0] st_ra, st_wa;
  wire [511:0] st_wd, q_data;
  wire [63:0] q_mask;
  wire [31:0] rq_line, q_line;
  // The store's reads, {srl} clocks from the address to the word.
  reg [511:0] st_q [0:{srl}-1];
  wire [511:0] st_rd = st_q[{srl}-1];
  integer sk;
  reg rq_ready = 0, rs_valid = 0;
  reg [511:0] rs_data;
  reg [4:0] q_n = 0;
  // The memory's reads in flight: each asked line, its word as the memory held it when the read was taken, and the clock
  // it comes back.
  reg [31:0] fl_line [0:255];
  reg [511:0] fl_data [0:255];
  integer fl_at [0:255];
  integer fl_w = 0, fl_r = 0, now = 0;
  // The side's queue: each pushed line applied to the memory 20 to 275 clocks on, so a read taken before then sees the
  // line as it was.
  reg [511:0] pq_data [0:255];
  reg [63:0] pq_mask [0:255];
  reg [31:0] pq_line [0:255];
  integer pq_at [0:255];
  integer pq_w = 0, pq_r = 0, i, j, bad = 0, n_rows, n_beats, x_base, x_stride, o_base, o_stride, cycles = 0;
  wire settled = pq_w == pq_r;
  logos_conv_path #(.SA(SA), .CVB(CVB), .LANES({lanes}), .SRL({srl})) u(.clk(clk), .rst(rst), .start(start), .rows(cfg[0][15:0]),
    .f16(cfg[1][0]), .k_line(cfg[2]), .beats(cfg[3]), .x_base(cfg[4]), .x_stride(cfg[5]), .o_base(cfg[6]), .o_stride(cfg[7]),
    .ring_off(cfg[8]), .ring_taps(cfg[9][7:0]), .row_bytes(cfg[10]), .row_k(row_k), .row_e(rowt[row_k]), .settled(settled),
    .busy(busy), .st_ra(st_ra), .st_rd(st_rd), .st_we(st_we), .st_wa(st_wa), .st_wd(st_wd), .rq_valid(rq_valid),
    .rq_ready(rq_ready), .rq_line(rq_line), .rs_valid(rs_valid), .rs_data(rs_data), .q_v(q_v), .q_data(q_data),
    .q_mask(q_mask), .q_line(q_line), .q_n(q_n), .flush(flush));
  always @(posedge clk) begin
    now <= now + 1;
    st_q[0] <= store[st_ra];
    for (sk = 1; sk < {srl}; sk = sk + 1) st_q[sk] <= st_q[sk-1];
    if (st_we) store[st_wa] <= st_wd;
    rq_ready <= ($random & 3) != 0;
    if (rq_valid && rq_ready) begin fl_line[fl_w % 256] <= rq_line; fl_data[fl_w % 256] <= mem[rq_line]; fl_at[fl_w % 256] <= now + 8 + ($random & 31); fl_w <= fl_w + 1; end
    rs_valid <= 1'b0;
    if (fl_r != fl_w && fl_at[fl_r % 256] <= now) begin rs_valid <= 1'b1; rs_data <= fl_data[fl_r % 256]; fl_r <= fl_r + 1; end
    if (q_v) begin pq_data[pq_w % 256] <= q_data; pq_mask[pq_w % 256] <= q_mask; pq_line[pq_w % 256] <= q_line; pq_at[pq_w % 256] <= now + 20 + ($random & 255); pq_w <= pq_w + 1; end
    if (pq_r != pq_w && pq_at[pq_r % 256] <= now) begin
      for (j = 0; j < 64; j = j + 1) if (pq_mask[pq_r % 256][j]) mem[pq_line[pq_r % 256]][8*j +: 8] <= pq_data[pq_r % 256][8*j +: 8];
      pq_r <= pq_r + 1;
    end
    q_n <= (pq_w - pq_r > 3) ? 5'd3 : (pq_w - pq_r);
    if (busy) cycles <= cycles + 1;
  end
  initial begin
    $readmemh("cfg.hex", cfg);
    $readmemh("mem.hex", mem);
    $readmemh("store.hex", store);
    $readmemh("rows.hex", rowt);
    $readmemh("want_out.hex", want_out);
    $readmemh("want_ring.hex", want_ring);
    n_rows = cfg[0]; n_beats = cfg[3]; o_base = cfg[6]; o_stride = cfg[7];
    @(negedge clk); rst = 0;
    @(negedge clk); start = 1;
    @(negedge clk); start = 0;
    while (busy || !settled) begin @(negedge clk); if (cycles > 2000000) begin $display("TIMEOUT"); $finish; end end
    for (i = 0; i < n_rows; i = i + 1)
      for (j = 0; j < n_beats; j = j + 1)
        if (store[o_base + i * o_stride + j] !== want_out[i * n_beats + j]) begin
          bad = bad + 1;
          if (bad < 6) $display("out row %0d beat %0d: %h want %h", i, j, store[o_base + i * o_stride + j], want_out[i * n_beats + j]);
        end
    for (i = 0; i < cfg[11]; i = i + 1)
      if (mem[cfg[12] + i] !== want_ring[i]) begin
        bad = bad + 1;
        if (bad < 10) $display("ring line %0d: %h want %h", i, mem[cfg[12] + i], want_ring[i]);
      end
    $display("RESULT bad %0d cycles %0d", bad, cycles);
    $finish;
  end
endmodule
"#)).unwrap();
        let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "conv.v"]).output().expect("iverilog");
        assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
        let run = Command::new("vvp").current_dir(dir.path()).args(["-n", "sim"]).output().expect("vvp");
        let out = String::from_utf8_lossy(&run.stdout);
        let what = format!("{c} channels, {rows} rows, {} ring, {lanes} lanes, a store of {srl} clocks", if f16 { "binary16" } else { "f32" });
        assert!(out.contains("RESULT bad 0 "), "{what}: {out}");
    }
}

/// The gate (`logos_gdn_gate_p`) norms and gates each head's read-outs as the machine's gated norm does: every head of
/// `hd` elements, 16 to 256, RMS-normed by the weight and each element times the SiLU of its gate — read-outs, gates and
/// weights of both signs, exact zeros, values at the least normal and large enough to overflow a head's sum — its
/// read-outs a clock apart or with gaps, its slots two, four or sixteen heads, a head's gate written only while the gate
/// has its slot free, every head's beats written once at the store beats its first read-out names.
#[test]
fn the_gate_norms_and_gates_each_head_as_the_machines_gated_norm() {
    if !common::require_tool("iverilog") {
        return;
    }
    let units = format!("{}\n{}", gdn_stream_verilog(), gate_verilog());
    for (k, (hd, nz, gap)) in [(16usize, 2usize, 0usize), (16, 16, 20), (32, 4, 3), (64, 2, 20), (64, 16, 0), (128, 4, 0), (128, 16, 3), (256, 2, 3)].into_iter().enumerate() {
        let heads = if hd >= 128 { 10 } else { 24 };
        let mut r = Rng(0x6a7e ^ k as u64);
        let mut draw = |r: &mut Rng, m: f32| -> f32 { (((r.next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0) * m };
        let o: Vec<f32> = (0..heads * hd)
            .map(|i| match (i * 7 + k) % 23 {
                0 => 0.0,
                1 => -0.0,
                2 => f32::from_bits(0x0080_0000 | (r.next() as u32 & 0x7f_ffff)),
                3 if i / hd % 5 == 4 => 3.0e19,
                _ => draw(&mut r, 2.0),
            })
            .collect();
        let z: Vec<f32> = (0..heads * hd).map(|i| if i % 13 == 0 { draw(&mut r, 60.0) } else { draw(&mut r, 6.0) }).collect();
        let w = norm_weight(0x900 + k as u64, hd);
        let eps = [1.0e-6f32, 1.0e-5, 0.0][k % 3];
        let want: Vec<u32> = (0..heads)
            .flat_map(|h| {
                let y = rmsnorm(FpUnit::Soft, &o[h * hd..(h + 1) * hd], &w, eps);
                y.iter().zip(&z[h * hd..(h + 1) * hd]).map(|(&y, &z)| FpUnit::Soft.mul(y, silu(FpUnit::Soft, z)).to_bits()).collect::<Vec<u32>>()
            })
            .collect();
        let dir = tempfile::tempdir().unwrap();
        let words = |v: &[f32]| v.iter().map(|x| format!("{:08x}\n", x.to_bits())).collect::<String>();
        std::fs::write(dir.path().join("o.hex"), words(&o)).unwrap();
        std::fs::write(dir.path().join("z.hex"), words(&z)).unwrap();
        std::fs::write(dir.path().join("w.hex"), words(&w)).unwrap();
        std::fs::write(dir.path().join("want.hex"), want.iter().map(|x| format!("{x:08x}\n")).collect::<String>()).unwrap();
        std::fs::write(dir.path().join("gdn.v"), &units).unwrap();
        let rn = recip(FpUnit::Soft, hd as f32).to_bits();
        let beats = hd / 16;
        let ba = if beats > 1 { beats.next_power_of_two().trailing_zeros().max(1) } else { 1 };
        let tb = format!(
            r#"module tb;
  localparam HD = {hd}, NZ = {nz}, HEADS = {heads}, BEATS = {beats}, GAP = {gap};
  reg clk = 0, rst = 1;
  always #5 clk = ~clk;
  reg w_we = 0, z_we = 0, e_v = 0;
  reg [7:0] w_wa = 0;
  reg [15:0] z_wa = 0;
  reg [511:0] w_wd = 0, z_wd = 0;
  reg [31:0] e_o = 0;
  reg [11:0] e_at = 0;
  wire st_we, busy;
  wire [11:0] st_wa;
  wire [511:0] st_wd;
  wire [15:0] gated;
  logos_gdn_gate_p #(.HD(HD), .SA(12), .NZ(NZ), .RN(32'h{rn:08x})) u(.clk(clk), .rst(rst), .eps(32'h{eps:08x}), .w_we(w_we),
    .w_wa(w_wa), .w_wd(w_wd), .z_we(z_we), .z_wa(z_wa), .z_wd(z_wd), .e_v(e_v), .e_o(e_o), .e_at(e_at), .st_we(st_we),
    .st_wa(st_wa), .st_wd(st_wd), .gated(gated), .busy(busy));
  reg [31:0] o [0:HEADS*HD-1];
  reg [31:0] z [0:HEADS*HD-1];
  reg [31:0] w [0:HD-1];
  reg [31:0] want [0:HEADS*HD-1];
  reg [31:0] got [0:HEADS*HD-1];
  reg [HEADS*BEATS-1:0] seen = 0;
  integer h, j, b, l, k, errs = 0, writes = 0, seed = {k};
  always @(posedge clk) if (st_we) begin
    writes = writes + 1;
    if (st_wa < 100 || st_wa >= 100 + HEADS * BEATS || seen[st_wa - 100]) errs = errs + 1;
    else begin
      seen[st_wa - 100] = 1'b1;
      for (l = 0; l < 16; l = l + 1) got[(st_wa - 100) * 16 + l] = st_wd[32*l +: 32];
    end
  end
  initial begin
    $readmemh("o.hex", o);
    $readmemh("z.hex", z);
    $readmemh("w.hex", w);
    $readmemh("want.hex", want);
    repeat (4) @(negedge clk);
    rst = 0;
    for (b = 0; b < BEATS; b = b + 1) begin
      @(negedge clk) begin w_we = 1; w_wa = b; for (l = 0; l < 16; l = l + 1) w_wd[32*l +: 32] = w[16*b + l]; end
    end
    @(negedge clk) w_we = 0;
    for (h = 0; h < HEADS; h = h + 1) begin
      while (h - gated >= NZ) @(negedge clk);
      for (b = 0; b < BEATS; b = b + 1) begin
        z_we = 1; z_wa = h % NZ * (1 << {ba}) + b; for (l = 0; l < 16; l = l + 1) z_wd[32*l +: 32] = z[h*HD + 16*b + l];
        @(negedge clk);
      end
      z_we = 0;
      for (j = 0; j < HD; j = j + 1) begin
        e_v = 1; e_o = o[h*HD + j]; e_at = 100 + h * BEATS;
        @(negedge clk);
        e_v = 0;
        if (GAP > 0) repeat ($unsigned($random(seed)) % (GAP + 1)) @(negedge clk);
      end
    end
    k = 0;
    while ((busy || gated != HEADS) && k < 200000) begin @(negedge clk); k = k + 1; end
    for (j = 0; j < HEADS * HD; j = j + 1) if (got[j] !== want[j]) begin
      if (errs < 8) $display("MISMATCH head %0d element %0d: rtl %h numerics %h", j / HD, j % HD, got[j], want[j]);
      errs = errs + 1;
    end
    $display("RESULT writes=%0d of %0d errors=%0d hung=%0d", writes, HEADS * BEATS, errs, k >= 200000);
    $finish;
  end
endmodule
"#,
            eps = eps.to_bits(),
        );
        std::fs::write(dir.path().join("tb.v"), tb).unwrap();
        let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "gdn.v"]).output().expect("iverilog");
        assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
        let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
        let out = String::from_utf8_lossy(&run.stdout);
        assert!(out.contains(&format!("RESULT writes={} of {} errors=0 hung=0", heads * beats, heads * beats)), "{hd} wide, {nz} slots, gaps to {gap}: {out}");
    }
}

/// Head lanes running a job whose read-outs a gated norm takes (`delta_normed_lanes`) leave every head's read-outs as
/// the machine's gated norm leaves them, the weight fetched once at the job's start and each head's gate read with its
/// query, and the memory and the log as the numerics' after the row: flushed every sixteen tokens on two, three and four
/// lanes — a row logging, one flushing — in binary16 and f32, writes taken quickly, late and slowly, at one and two
/// sixteens a clock; a window behind on four lanes, replaying and not, a window dropped and one wrapping the ring; and at
/// the card's width on two lanes.
#[test]
fn head_lanes_norm_and_gate_every_heads_read_outs_as_the_machines_gated_norm() {
    if !common::require_tool("iverilog") || !common::require_tool("verilator") {
        return;
    }
    let eps = 1.0e-6f32;
    for (k, (count, f16, writes, w, lanes)) in [(5, true, Writes::Quick, 1, 2), (16, false, Writes::Late, 2, 4), (9, true, Writes::Slow, 2, 3)].into_iter().enumerate() {
        let c = SideCase::new(300 + k as u64, 32, 2, 8, count, f16).normed(&norm_weight(310 + k as u64, 32), eps, 320 + k as u64);
        lanes_run(&c, false, writes, Reads::Random, w, lanes, None, 1, LANE_CREDIT);
    }
    for (k, (count, oldest, f16, writes)) in [(5, 0, true, Writes::Quick), (8, 4, false, Writes::Late), (3, 2, true, Writes::Quick), (6, 7, false, Writes::Quick)].into_iter().enumerate() {
        let c = SideCase::new_on(330 + k as u64, 32, 2, 8, count, f16, Some(4), oldest).normed(&norm_weight(340 + k as u64, 32), eps, 350 + k as u64);
        lanes_run(&c, true, writes, Reads::Random, 2, 4, None, 1, LANE_CREDIT);
    }
    let c = SideCase::new_on(360, 128, 2, 4, 9, true, Some(8), 3).normed(&norm_weight(361, 128), 1.0e-5, 362);
    lanes_run(&c, true, Writes::Quick, Reads::Random, 2, 2, None, 1, LANE_CREDIT);
}

/// A job's rows on head lanes whose read-outs a gated norm takes, one after another, each as the machine has it alone:
/// rows a window behind that replay their share and one that replays nothing, on four lanes of narrow heads in binary16
/// and f32, writes taken quickly and late; rows flushed every sixteen tokens, a row logging beside a row flushing; every
/// row's gate its own and the job's weight the same — a lane's gate takes the heads of one row after another's.
#[test]
fn head_lanes_norm_a_jobs_rows_one_after_another_as_the_machines_gated_norm() {
    if !common::require_tool("verilator") {
        return;
    }
    let w = norm_weight(400, 32);
    for (f16, writes, seed) in [(true, Writes::Quick, 410u64), (false, Writes::Late, 420)] {
        let rows = [
            SideCase::new_on(seed, 32, 2, 8, 5, f16, Some(4), 0).normed(&w, 1.0e-6, seed + 100),
            SideCase::new_on(seed + 1, 32, 2, 8, 3, f16, Some(4), 2).normed(&w, 1.0e-6, seed + 101),
            SideCase::new_on(seed + 2, 32, 2, 8, 8, f16, Some(4), 4).normed(&w, 1.0e-6, seed + 102),
        ];
        lanes_rows_run(&rows, writes, Reads::Random, 2, 4, LANE_CREDIT);
    }
    let rows = [SideCase::new(430, 32, 2, 8, 5, true).normed(&w, 1.0e-6, 431), SideCase::new(432, 32, 2, 8, 16, true).normed(&w, 1.0e-6, 433)];
    lanes_rows_run(&rows, Writes::Quick, Reads::Random, 2, 4, LANE_CREDIT);
}

/// Head lanes built for norms run a job that carries none as the lanes built without do: every read-out as made, the
/// gate and its weight unused, one row flushed every sixteen tokens and a job of rows a window behind.
#[test]
fn head_lanes_built_for_norms_run_a_job_without_one_as_made() {
    if !common::require_tool("verilator") {
        return;
    }
    let w = norm_weight(500, 32);
    let c = SideCase::new(501, 32, 2, 8, 16, true).normed_off(&w, 1.0e-6, 502);
    lanes_run(&c, false, Writes::Quick, Reads::Random, 2, 4, None, 1, LANE_CREDIT);
    let rows = [
        SideCase::new_on(510, 32, 2, 8, 5, false, Some(4), 0).normed_off(&w, 1.0e-6, 511),
        SideCase::new_on(512, 32, 2, 8, 8, false, Some(4), 4).normed_off(&w, 1.0e-6, 513),
    ];
    lanes_rows_run(&rows, Writes::Late, Reads::Random, 2, 4, LANE_CREDIT);
}

/// A row's gated norm on its head lanes costs the row only its tail: behind a memory answering 40 clocks after an ask,
/// the row normed takes at most the weight's fetch and the gate's last head more clocks than the same row without, a
/// window behind and every sixteen tokens — the gate keeps up with the read-outs it takes a clock at a time.
#[test]
fn a_rows_gated_norm_costs_its_head_lanes_only_the_gates_tail() {
    if !common::require_tool("verilator") {
        return;
    }
    let w = norm_weight(600, 32);
    for (normed, plain) in [
        (SideCase::new_on(601, 32, 2, 8, 6, true, Some(4), 0).normed(&w, 1.0e-6, 602), SideCase::new_on(601, 32, 2, 8, 6, true, Some(4), 0)),
        (SideCase::new(603, 32, 2, 8, 12, false).normed(&w, 1.0e-6, 604), SideCase::new(603, 32, 2, 8, 12, false)),
    ] {
        let lat = Reads::Latency(40);
        let with = lanes_run(&normed, true, Writes::Quick, lat, 2, 4, None, 1, LANE_CREDIT);
        let without = lanes_run(&plain, true, Writes::Quick, lat, 2, 4, None, 1, LANE_CREDIT);
        assert!(with <= without + 600, "the norm took the row from {without} to {with} clocks");
    }
}

/// A job's rows on four head lanes one after another, each as the numerics have it alone: rows logging their fifth,
/// sixteenth (the log full, so the row flushes every head) and ninth tokens, in binary16 and in f32, writes taken quickly
/// and late; the coordinator reading a row's log while the lanes run the row before.
#[test]
fn head_lanes_run_a_jobs_rows_one_after_another_as_the_numerics_do() {
    if !common::require_tool("verilator") {
        return;
    }
    for (f16, writes, seed) in [(true, Writes::Quick, 110u64), (false, Writes::Late, 120)] {
        let rows = [SideCase::new(seed, 32, 2, 8, 5, f16), SideCase::new(seed + 1, 32, 2, 8, 16, f16), SideCase::new(seed + 2, 32, 2, 8, 9, f16)];
        lanes_rows_run(&rows, writes, Reads::Random, 2, 4, LANE_CREDIT);
    }
}

/// A job's rows on head lanes under a trailing flush, one after another, each as the numerics have it alone: rows that
/// replay their share (the first window past, the log full and a window dropped, a window wrapping the ring) beside a row
/// that replays nothing, on four lanes of narrow heads in binary16 and f32, writes taken quickly and late, and on two
/// lanes at the card's width.
#[test]
fn head_lanes_replay_a_jobs_rows_a_window_behind_one_after_another_as_the_numerics_do() {
    if !common::require_tool("verilator") {
        return;
    }
    for (f16, writes, seed) in [(true, Writes::Quick, 130u64), (false, Writes::Late, 140)] {
        let rows = [
            SideCase::new_on(seed, 32, 2, 8, 5, f16, Some(4), 0),
            SideCase::new_on(seed + 1, 32, 2, 8, 3, f16, Some(4), 2),
            SideCase::new_on(seed + 2, 32, 2, 8, 8, f16, Some(4), 4),
            SideCase::new_on(seed + 3, 32, 2, 8, 6, f16, Some(4), 7),
        ];
        lanes_rows_run(&rows, writes, Reads::Random, 2, 4, LANE_CREDIT);
    }
    let card = [
        SideCase::new_on(150, 128, 2, 4, 12, true, Some(8), 8),
        SideCase::new_on(151, 128, 2, 4, 9, true, Some(8), 3),
        SideCase::new_on(152, 128, 2, 4, 5, true, Some(8), 0),
    ];
    lanes_rows_run(&card, Writes::Quick, Reads::Random, 2, 2, LANE_CREDIT);
}

/// A job's rows through the coordinator faster than through the lanes, each as the numerics have it alone: six rows of
/// one to three logged tokens (a short log read each) on two lanes of eight heads, the lanes' writes taken slowly, so the
/// coordinator comes round to a copy of the log while the lanes may still read the row it holds; every sixteen tokens,
/// in binary16 and in f32.
#[test]
fn head_lanes_reuse_a_copy_of_the_log_only_once_the_row_it_holds_is_done() {
    if !common::require_tool("verilator") {
        return;
    }
    for (f16, seed) in [(true, 300u64), (false, 310)] {
        let rows: Vec<SideCase> = [1usize, 3, 2, 1, 2, 3].iter().enumerate().map(|(k, &count)| SideCase::new(seed + k as u64, 32, 2, 16, count, f16)).collect();
        lanes_rows_run(&rows, Writes::Slow, Reads::Random, 2, 2, LANE_CREDIT);
    }
}

/// A job's rows under a trailing flush on two lanes of eight heads, every row replaying its share, the replays' writes
/// taken slowly: each lane loads a head's folds into its flush banks while the heads before replay, three banks round,
/// and a bank is loaded only once its replay is done; each row as the numerics have it alone.
#[test]
fn head_lanes_load_a_flush_bank_only_once_its_replay_is_done() {
    if !common::require_tool("verilator") {
        return;
    }
    let rows = [
        SideCase::new_on(320, 32, 2, 16, 5, true, Some(4), 0),
        SideCase::new_on(321, 32, 2, 16, 6, true, Some(4), 2),
        SideCase::new_on(322, 32, 2, 16, 8, true, Some(4), 4),
        SideCase::new_on(323, 32, 2, 16, 7, true, Some(4), 6),
    ];
    lanes_rows_run(&rows, Writes::Slow, Reads::Random, 2, 2, LANE_CREDIT);
}

/// A job's rows of different shares under a trailing flush, behind a memory answering 200 clocks after each ask, the
/// lanes asking 128 lines ahead: the lines of one row are still on their way while the lanes stream the next row's, and
/// each line takes its place in its own row's share; each row as the numerics have it alone.
#[test]
fn head_lanes_place_each_line_in_its_own_rows_share_behind_a_far_memory() {
    if !common::require_tool("verilator") {
        return;
    }
    let rows = [
        SideCase::new_on(330, 32, 2, 8, 5, true, Some(4), 0),
        SideCase::new_on(331, 32, 2, 8, 6, true, Some(4), 7),
        SideCase::new_on(332, 32, 2, 8, 8, true, Some(4), 2),
        SideCase::new_on(333, 32, 2, 8, 3, true, Some(4), 5),
        SideCase::new_on(334, 32, 2, 8, 7, true, Some(4), 4),
    ];
    lanes_rows_run(&rows, Writes::Quick, Reads::Latency(200), 2, 4, 128);
}

/// A job's rows under a trailing flush on two lanes of twenty-four heads, as the 27B's: each lane loads a row's six batches
/// one after another while it streams the batches before, and takes the next row while it still loads this one, so the
/// coordinator, through its short log reads well ahead, comes round to the copy of the log a lane still loads its last
/// batches from; it reuses a copy only once every lane is done with the row that wrote it. Each row as the numerics have
/// it alone.
#[test]
fn head_lanes_of_many_batches_reuse_a_copy_of_the_log_only_once_the_row_it_holds_is_done() {
    if !common::require_tool("verilator") {
        return;
    }
    let rows: Vec<SideCase> = [2usize, 1, 3, 2, 1, 3]
        .iter()
        .enumerate()
        .map(|(k, &count)| SideCase::new_on(340 + k as u64, 32, 4, 48, count, true, Some(4), (3 * k) % 8))
        .collect();
    lanes_rows_run(&rows, Writes::Slow, Reads::Random, 2, 2, LANE_CREDIT);
}

/// A job's rows under a trailing flush, every row replaying its share, on two lanes of sixteen heads (four batches a row),
/// the memory taking a write one clock in thirty-two: each batch's replay writes its share back long after the next two
/// batches have streamed, so a lane comes round to a flush bank still replaying and loads it only once its replay is
/// done. Each row as the numerics have it alone.
#[test]
fn head_lanes_load_a_flush_bank_only_once_its_replay_has_written_back() {
    if !common::require_tool("verilator") {
        return;
    }
    let rows = [
        SideCase::new_on(350, 32, 2, 32, 6, true, Some(4), 0),
        SideCase::new_on(351, 32, 2, 32, 8, true, Some(4), 3),
        SideCase::new_on(352, 32, 2, 32, 5, true, Some(4), 6),
    ];
    lanes_rows_run(&rows, Writes::Crawl, Reads::Random, 2, 2, LANE_CREDIT);
}

/// A job's rows under a trailing flush, a batch a row, behind a memory answering 200 clocks after each ask with the lanes
/// asking 128 lines ahead and taking a write one clock in thirty-two, rows replaying in twos around a row that replays
/// nothing: that row's lines are still on their way when its lane moves to the replaying row after it, whose share
/// differs, and the flush bank that row takes, three batches round, is still replaying the row two before; each line
/// goes into a share only if its own row replays it. Each row as the numerics have it alone.
#[test]
fn head_lanes_place_only_a_replaying_rows_lines_in_a_share_behind_a_far_memory() {
    if !common::require_tool("verilator") {
        return;
    }
    let rows = [
        SideCase::new_on(360, 32, 2, 8, 6, true, Some(4), 0),
        SideCase::new_on(361, 32, 2, 8, 7, true, Some(4), 5),
        SideCase::new_on(362, 32, 2, 8, 2, true, Some(4), 2),
        SideCase::new_on(363, 32, 2, 8, 8, true, Some(4), 7),
        SideCase::new_on(364, 32, 2, 8, 6, true, Some(4), 4),
        SideCase::new_on(365, 32, 2, 8, 3, true, Some(4), 1),
        SideCase::new_on(366, 32, 2, 8, 7, true, Some(4), 3),
    ];
    lanes_rows_run(&rows, Writes::Crawl, Reads::Latency(200), 2, 2, 128);
}

/// Head lanes under a trailing flush replay each row's share of every head as the numerics do: four lanes of two heads
/// at 32 wide over windows of four, the first share of the log's second window from its first slot, and the last share
/// with the log full and the window dropped, its writes landing late; in binary16.
#[test]
fn four_head_lanes_replay_each_rows_share_a_window_behind_as_the_numerics_do() {
    gdn_lanes_trailing_case(80, 32, 2, 8, 5, 0, 4, true, Writes::Quick, 2, 4);
    gdn_lanes_trailing_case(81, 32, 2, 8, 8, 4, 4, true, Writes::Late, 2, 4);
}

/// [`four_head_lanes_replay_each_rows_share_a_window_behind_as_the_numerics_do`] over a share whose window wraps the
/// ring, in f32, and a row that replays nothing yet, its log a window long but for the row's.
#[test]
fn four_head_lanes_replay_a_share_whose_window_wraps_the_ring_and_a_row_short_of_a_window() {
    gdn_lanes_trailing_case(82, 32, 2, 8, 6, 4, 4, false, Writes::Quick, 2, 4);
    gdn_lanes_trailing_case(83, 32, 2, 8, 3, 4, 4, true, Writes::Quick, 2, 4);
}

/// At the card's width: two lanes of 128-wide heads over windows of eight, two batches a lane (each lane's two flush
/// banks in turn), and a lane at half a line a clock over shares of four rows.
#[test]
fn head_lanes_replay_each_rows_share_a_window_behind_at_the_cards_width_and_half_a_line_a_clock() {
    gdn_lanes_trailing_case(84, 128, 2, 4, 12, 8, 8, true, Writes::Quick, 2, 2);
    gdn_lanes_trailing_case(85, 32, 2, 12, 7, 0, 4, true, Writes::Slow, 2, 2);
    gdn_lanes_trailing_case(86, 32, 2, 6, 13, 8, 8, true, Writes::Quick, 1, 4);
}

/// Head lanes and their coordinator asking up to 128 lines ahead read out and flush every head as the numerics do, over
/// memories answering at random: each lane's queue of 256 lines wraps, each head's rows are asked while the heads before
/// are on their way through the read-out, its folds in the buffer the head before did not take and its lines tagged with
/// that buffer (and, flushed a window behind, with its head's slot in the batch), and a lane copies a head's folds only
/// once every line of the head two back is through. Narrow heads (a head's rows a quarter of the credit) put the copy
/// against the buffer's last line: flushed every sixteen tokens and not, in binary16 and in f32, its writes taken quickly,
/// late and slowly; a window behind, from a share whose window wraps the ring and at the card's width. Behind a memory
/// that answers 200 clocks after an ask, slower than the credit, each lane and the coordinator hold all 128 lines owed,
/// every place of a queue twice the credit deep in use.
#[test]
fn head_lanes_reading_far_ahead_read_and_flush_every_head_as_the_numerics_do() {
    if !common::require_tool("verilator") {
        return;
    }
    let every = |case, hd, nk, nv, count, f16, writes, w, credit| {
        lanes_run(&SideCase::new(case, hd, nk, nv, count, f16), true, writes, Reads::Random, w, 2, None, 1, credit)
    };
    every(91, 32, 2, 12, SideCase::TOKENS, true, Writes::Quick, 2, 128);
    every(92, 32, 2, 12, 5, true, Writes::Late, 2, 128);
    every(93, 64, 2, 8, 9, false, Writes::Slow, 2, 64);
    every(94, 32, 2, 10, 7, true, Writes::Quick, 1, 128);
    let behind = |case, hd, nk, nv, count, oldest, window, writes| {
        lanes_run(&SideCase::new_on(case, hd, nk, nv, count, true, Some(window), oldest), true, writes, Reads::Random, 2, 2, None, 1, 128)
    };
    behind(95, 32, 2, 12, 7, 6, 4, Writes::Late);
    behind(96, 32, 2, 12, 8, 0, 4, Writes::Quick);
    behind(97, 128, 2, 4, 12, 8, 8, Writes::Quick);
    lanes_run(&SideCase::new(99, 32, 2, 12, 9, true), true, Writes::Quick, Reads::Latency(200), 2, 2, None, 1, 128);
    lanes_run(&SideCase::new_on(100, 32, 2, 12, 7, true, Some(4), 6), true, Writes::Quick, Reads::Latency(200), 2, 2, None, 1, 128);
}

/// A job's rows overlap on head lanes: past the first, a row takes the lanes at most a quarter more than its rows'
/// stream (each lane's lines of its heads' rows, a line a clock), where a row alone also pays the coordinator's count,
/// append and log read and its first batch's load and fold before its rows, and its last batch's replay after them. At
/// the 27B's shape — 48 value heads 128 wide on four key heads, binary16, two lanes, behind a memory answering a clock
/// after each ask — three rows against one, under a trailing flush of eight with a log of twelve and flushed every
/// sixteen with a log of eight.
#[test]
fn head_lanes_take_a_row_past_the_first_in_little_more_than_its_rows_stream() {
    if !common::require_tool("verilator") {
        return;
    }
    let (hd, nk, nv, lanes) = (128usize, 4, 48, 2);
    let stream = (nv / lanes * hd * (hd * 2).div_ceil(64)) as u64;
    for (what, cases) in [
        ("a window behind", [160, 161, 162].map(|k| SideCase::new_on(k, hd, nk, nv, 12, true, Some(8), 8))),
        ("every sixteen", [163, 164, 165].map(|k| SideCase::new(k, hd, nk, nv, 8, true))),
    ] {
        let alone = lanes_run(&cases[0], true, Writes::Quick, Reads::Latency(1), 2, lanes, None, 1, 128);
        let three = lanes_rows_run(&cases, Writes::Quick, Reads::Latency(1), 2, lanes, 128);
        let past = (three - alone) / 2;
        assert!(past >= stream, "{what}: a row past the first in {past} clocks, under its rows' {stream}");
        assert!(4 * past <= 5 * stream, "{what}: a row past the first in {past} clocks, past a quarter more than its rows' {stream} (alone {alone})");
    }
}

/// A head lane under a trailing flush takes fewer clocks a token than one flushed every sixteen tokens, a window's tokens
/// averaged, at the 27B's shape: 48 value heads 128 wide on two lanes, 24 a lane (six batches), binary16 at a line a
/// clock. A batch's shares replay while the lane streams the next batch's rows, so a trailing row costs about a flush
/// of a batch past its rows, where a row of a log flushed every sixteen reads alone fifteen times and then flushes every
/// row of every head; the trailing row (any row past the first window costs as much, its log half again as long as the
/// mean of a log flushed every sixteen) against a row that reads alone and one that flushes, weighed as sixteen tokens
/// have them. A lane of a single batch has nothing to replay a batch beside, and gains nothing.
#[test]
fn a_head_lane_replaying_a_window_behind_takes_fewer_clocks_a_token_than_one_flushing_every_sixteen() {
    let trailing = gdn_lanes_trailing_case(87, 128, 4, 48, 12, 8, 8, true, Writes::Quick, 2, 2);
    let reading = gdn_lanes_case_on(88, 128, 4, 48, 8, true, true, Writes::Quick, 2, 2, None, 1);
    let flushing = gdn_lanes_case_on(89, 128, 4, 48, 16, true, true, Writes::Quick, 2, 2, None, 1);
    let every = (15 * reading + flushing) as f64 / 16.0;
    eprintln!("clocks a token: trailing {trailing}; flushed every sixteen {every:.0} (reading {reading}, flushing {flushing})");
    assert!((trailing as f64) < every, "a trailing token takes {trailing} clocks, a token flushed every sixteen {every:.0} on average");
}

/// A row of `c` on two head lanes reading 128 lines ahead, at a line a clock, behind a memory that takes every ask and
/// answers each a line a clock one clock after it and behind one that answers 96 clocks after it: the far memory's
/// latency costs the row at most four latencies (the count, the log, the first batch's rows, the last read-outs) and one
/// for each of the `drains` batch ends a lane waits for its rows at, not one a head nor a line every latency / credit
/// clocks; and the near row takes at most the lines a lane reads (each head's rows) and the coordinator's (the count,
/// the entry, the log into the copies) at a line a clock, past `allowance` clocks for the entry's append and the first
/// batch's load and fold. Every read-out and row is the numerics'.
fn lanes_at_their_line_rate(c: &SideCase, what: &str, allowance: u64, drains: u64) {
    let (latency, credit) = (96u32, 128usize);
    let near = lanes_run(c, true, Writes::Quick, Reads::Latency(1), 2, 2, None, 1, credit);
    let far = lanes_run(c, true, Writes::Quick, Reads::Latency(latency), 2, 2, None, 1, credit);
    let lane_lines = (c.nv / 2 * c.hd * c.hd * if c.f16 { 2 } else { 4 } / 64) as u64;
    let coordinator_lines = (1 + (c.count + 1) * c.entry / 64) as u64;
    let (bound, cost) = (lane_lines + coordinator_lines + allowance, (4 + drains) * latency as u64);
    eprintln!("{what}: {near} clocks behind the near memory (bound {bound}), {far} behind the far one ({} more, at most {cost})", far - near);
    assert!(near <= bound, "{what} takes {near} clocks behind a memory answering the next clock, past {bound}");
    assert!(far <= near + cost, "{what} takes {far} clocks behind a memory answering {latency} clocks on, {} more than the next clock's", far - near);
}

/// Head lanes read their heads' rows at a line a clock behind their memory's latency
/// ([`lanes_at_their_line_rate`]): at the 27B's shape (48 value heads 128 wide on two lanes, 24 a lane, binary16), a row of
/// a log of eight tokens flushed every sixteen.
#[test]
fn head_lanes_read_their_heads_at_a_line_a_clock_behind_their_memorys_latency() {
    if !common::require_tool("verilator") {
        return;
    }
    lanes_at_their_line_rate(&SideCase::new(90, 128, 4, 48, 8, true), "a row read on two lanes", 2_500, 0);
}

/// [`head_lanes_read_their_heads_at_a_line_a_clock_behind_their_memorys_latency`] flushed a window of eight behind: a row
/// of a log of twelve tokens replaying its share, each batch's rows through the read-out before the batch's share
/// replays (a drain at each of a lane's six batches), the next batch's log loaded and folded while the batch before
/// streams and the one before that replays. A batch's walk and fold of a log of twelve tokens take longer than its
/// rows, so the row's allowance holds them past its rows, beside the first batch's load and fold and the last batch's
/// replay.
#[test]
fn head_lanes_flushed_a_window_behind_read_at_a_line_a_clock_behind_their_memorys_latency() {
    if !common::require_tool("verilator") {
        return;
    }
    lanes_at_their_line_rate(&SideCase::new_on(98, 128, 4, 48, 12, true, Some(8), 8), "a row replaying a window behind on two lanes", 7_000, 6);
}

/// A lane's flushing head split over two flush units by rows flushes every row as one unit does: four lanes, two heads
/// each, every head read and flushed as the numerics read and flush it, in binary16 and in f32, its writes taken
/// quickly and slowly.
#[test]
fn four_head_lanes_flush_every_head_over_two_flush_units_as_the_numerics_do() {
    gdn_lanes_case_on(61, 32, 2, 8, 16, true, true, Writes::Quick, 2, 4, None, 2);
    gdn_lanes_case_on(62, 32, 2, 8, 16, false, true, Writes::Quick, 2, 4, None, 2);
    gdn_lanes_case_on(63, 32, 2, 8, 16, true, true, Writes::Slow, 2, 4, None, 2);
}

/// Four flush units, a group of rows each at 32 wide, and two and four at the card's width of 128 over two lanes.
#[test]
fn head_lanes_flush_every_head_over_four_flush_units_and_at_the_cards_width() {
    gdn_lanes_case_on(64, 32, 2, 8, 16, true, true, Writes::Quick, 2, 4, None, 4);
    gdn_lanes_case_on(65, 128, 2, 4, 16, true, true, Writes::Quick, 2, 2, None, 2);
    gdn_lanes_case_on(66, 128, 2, 4, 16, true, true, Writes::Late, 2, 2, None, 4);
}

/// A head lane flushes in fewer clocks over more flush units: two lanes flushing a head each at the card's width over
/// one, two and four units, each run every row as the numerics flush it, two units fewer clocks than one and four fewer
/// than two; lanes only reading take the same clocks over any number.
#[test]
fn a_head_lane_flushes_in_fewer_clocks_over_more_flush_units() {
    let flushing: Vec<u64> = [1, 2, 4].iter().map(|&f| gdn_lanes_case_on(67, 128, 1, 2, 16, true, true, Writes::Quick, 2, 2, None, f)).collect();
    eprintln!("a flushing lane's clocks over 1, 2 and 4 flush units: {flushing:?}");
    assert!(flushing[1] < flushing[0] && flushing[2] < flushing[1], "a flushing lane's clocks over 1, 2 and 4 flush units: {flushing:?}");
    let reading: Vec<u64> = [1, 4].iter().map(|&f| gdn_lanes_case_on(68, 128, 1, 2, 5, true, true, Writes::Quick, 2, 2, None, f)).collect();
    assert_eq!(reading[0], reading[1], "a reading lane's clocks over 1 and 4 flush units");
}

#[test]
fn four_head_lanes_read_every_head_as_the_numerics_do_two_heads_a_lane_logging_a_fifth_token_in_f16() {
    gdn_lanes_case(40, 32, 2, 8, 5, true, true, Writes::Quick, 2, 4, None);
}

#[test]
fn four_head_lanes_read_and_flush_every_head_as_the_numerics_do_two_heads_a_lane_in_f16() {
    gdn_lanes_case(41, 32, 2, 8, 16, true, true, Writes::Quick, 2, 4, None);
}

#[test]
fn four_head_lanes_flush_three_heads_one_lane_idle_in_f16() {
    gdn_lanes_case(42, 32, 1, 3, 16, true, true, Writes::Quick, 2, 4, None);
}

#[test]
fn four_head_lanes_log_the_first_token_of_six_heads_of_two_groups() {
    gdn_lanes_case(43, 32, 2, 6, 1, true, true, Writes::Quick, 2, 4, None);
}

#[test]
fn four_head_lanes_at_half_a_line_a_clock_log_a_third_token_of_six_heads_in_f32() {
    gdn_lanes_case(44, 16, 2, 6, 3, false, false, Writes::Quick, 1, 4, None);
}

#[test]
fn four_head_lanes_at_half_a_line_a_clock_flush_five_heads_in_f16() {
    gdn_lanes_case(45, 32, 1, 5, 16, true, false, Writes::Quick, 1, 4, None);
}

#[test]
fn two_head_lanes_flush_four_heads_of_two_groups_in_f32() {
    gdn_lanes_case(46, 32, 2, 4, 16, false, true, Writes::Quick, 2, 2, None);
}

#[test]
fn three_head_lanes_flush_seven_heads_of_three_groups_in_f16() {
    gdn_lanes_case(47, 32, 3, 7, 16, true, true, Writes::Quick, 2, 3, None);
}

#[test]
fn four_head_lanes_flush_through_slow_write_ports() {
    gdn_lanes_case(48, 32, 2, 8, 16, true, true, Writes::Slow, 2, 4, None);
}

#[test]
fn four_head_lanes_flush_two_lines_an_ask_through_slow_write_ports_in_f32() {
    gdn_lanes_case(60, 32, 2, 8, 16, false, true, Writes::Slow, 2, 4, None);
}

#[test]
fn four_head_lanes_log_their_first_token_over_late_landing_writes() {
    gdn_lanes_case(49, 32, 1, 4, 1, true, true, Writes::Late, 2, 4, None);
}

#[test]
fn four_head_lanes_flush_over_late_landing_writes() {
    gdn_lanes_case(50, 32, 2, 8, 16, true, true, Writes::Late, 2, 4, None);
}

#[test]
fn four_head_lanes_flush_eight_heads_at_the_cards_width() {
    gdn_lanes_case(51, 128, 2, 8, 16, true, true, Writes::Quick, 2, 4, None);
}

#[test]
fn two_head_lanes_read_twelve_heads_of_three_groups_two_batches_a_lane_logging_a_fifth_token() {
    gdn_lanes_case(54, 32, 3, 12, 5, true, true, Writes::Quick, 2, 2, None);
}

#[test]
fn four_head_lanes_read_twenty_heads_of_four_groups_two_batches_a_lane_logging_a_third_token() {
    gdn_lanes_case(55, 32, 4, 20, 3, true, true, Writes::Slow, 2, 4, None);
}

#[test]
fn four_head_lanes_flush_twenty_heads_of_four_groups_five_heads_a_lane() {
    gdn_lanes_case(56, 32, 4, 20, 16, true, true, Writes::Quick, 2, 4, None);
}

#[test]
fn two_head_lanes_at_half_a_line_a_clock_read_ten_heads_in_f32_over_late_landing_writes() {
    gdn_lanes_case(57, 16, 2, 10, 4, false, false, Writes::Late, 1, 2, None);
}

#[test]
fn two_head_lanes_read_eighteen_heads_of_three_groups_three_batches_a_lane_logging_a_fifth_token() {
    gdn_lanes_case(58, 32, 3, 18, 5, true, true, Writes::Quick, 2, 2, None);
}

/// The 27B's heads: sixteen key heads and forty-eight value heads, twelve a lane on four lanes — three batches a lane,
/// as the card runs them — at a narrower width, through slow write ports.
#[test]
fn four_head_lanes_read_forty_eight_heads_of_sixteen_groups_three_batches_a_lane_as_the_27b_lays_them() {
    gdn_lanes_case(59, 32, 16, 48, 7, true, true, Writes::Slow, 2, 4, None);
}

#[test]
fn head_lanes_whose_copies_are_a_line_short_of_the_full_log_fault_and_touch_nothing() {
    let short = SideCase::new(52, 32, 2, 8, 5, true).log_lines() - 1;
    gdn_lanes_case(52, 32, 2, 8, 5, true, true, Writes::Quick, 2, 4, Some(short));
}

#[test]
fn head_lanes_whose_copies_hold_the_full_log_exactly_run_it() {
    let exact = SideCase::new(53, 32, 2, 8, 16, true).log_lines();
    gdn_lanes_case(53, 32, 2, 8, 16, true, true, Writes::Quick, 2, 4, Some(exact));
}

#[test]
fn the_gdn_side_reads_and_flushes_every_head_as_the_numerics_do_logging_a_fifth_token_in_f16() {
    gdn_side_case(0, 32, 1, 2, 5, true, false);
}

#[test]
fn the_gdn_side_reads_and_flushes_every_head_as_the_numerics_do_flushing_four_heads_of_two_groups_in_f16() {
    gdn_side_case(1, 32, 2, 4, 16, true, false);
}

/// A row's log entry appended over a memory that lands each write long after taking it, the log holding only the row's
/// entry: the side reads the entry back only once every beat of it is out of the encoder and written.
#[test]
fn the_gdn_side_reads_and_flushes_every_head_as_the_numerics_do_logging_its_first_token_over_late_landing_writes() {
    gdn_side_case_writing(7, 32, 1, 2, 1, true, false, Writes::Late);
}

#[test]
fn the_gdn_side_reads_and_flushes_every_head_as_the_numerics_do_through_a_slow_write_port() {
    gdn_side_case_writing(6, 32, 2, 4, 16, true, false, Writes::Slow);
}

#[test]
fn the_gdn_side_reads_and_flushes_every_head_as_the_numerics_do_logging_a_third_token_in_f32() {
    gdn_side_case(2, 16, 1, 2, 3, false, false);
}

#[test]
fn the_gdn_side_reads_and_flushes_every_head_as_the_numerics_do_flushing_two_groups_in_f32() {
    gdn_side_case(3, 16, 2, 2, 16, false, false);
}

#[test]
fn the_gdn_side_reads_and_flushes_every_head_as_the_numerics_do_flushing_at_the_cards_width() {
    gdn_side_case(4, 128, 1, 2, 16, true, true);
}

#[test]
fn the_gdn_side_reads_and_flushes_every_head_as_the_numerics_do_logging_the_first_token_of_three_heads() {
    gdn_side_case(5, 32, 1, 3, 1, true, false);
}

/// Seeded rows of a 128-wide head under a lazily flushed memory through `logos_gdn_lazy_row`: every read-out
/// `gdn_read`'s, and when the log is full — one to sixteen tokens — every flushed element `gdn_flush`'s, the state
/// stored in f32 and in f16.
#[test]
fn the_lazy_row_engine_reads_and_flushes_a_row_as_the_numerics_do() {
    if !common::require_tool("iverilog") {
        return;
    }
    const N: usize = 128;
    const T: usize = 16;
    let mut r = Rng(0x1a27);
    let mut f = || ((r.next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0;
    struct Case {
        row: Vec<f32>,
        y: Vec<f32>,
        carried: f32,
        scale: f32,
        f16: bool,
        flush: bool,
        log: Vec<(Vec<f32>, f32, f32, f32)>,
    }
    let mut cases = Vec::new();
    for entries in [1usize, 2, 7, 16] {
        for (f16, flush) in [(false, false), (false, true), (true, true)] {
            let row: Vec<f32> = (0..N).map(|_| f() * 0.5).collect();
            let y: Vec<f32> = (0..N).map(|_| f() * 0.1).collect();
            let log = (0..entries)
                .map(|_| {
                    let k: Vec<f32> = (0..N).map(|_| f() * 0.09).collect();
                    (k, -(f().abs() * 0.3), f().abs(), f())
                })
                .collect();
            cases.push(Case { row, y, carried: f() * 0.2, scale: 0.088388, f16, flush, log });
        }
    }
    let hex = |v: &[f32]| -> String {
        let mut s = String::new();
        for x in v.iter().rev() {
            write!(s, "{:08x}", x.to_bits()).unwrap();
        }
        s
    };
    let mut script = String::new();
    for c in &cases {
        // The engine takes each token's decay, the exponent's exp as the vector unit makes it.
        for (i, (k, log_decay, beta, _)) in c.log.iter().enumerate() {
            writeln!(script, "L {i:x} {:08x} {:08x} {}", exp(FpUnit::Soft, *log_decay).to_bits(), beta.to_bits(), hex(k)).unwrap();
        }
        let mut values: Vec<f32> = c.log.iter().map(|e| e.3).collect();
        values.resize(T, 0.0);
        writeln!(script, "C {:x} {:x} {:x} {:08x} {:08x} {} {}", c.log.len(), c.flush as u8, c.f16 as u8, c.scale.to_bits(), c.carried.to_bits(), hex(&c.y), hex(&values)).unwrap();
        for b in 0..N / 16 {
            writeln!(script, "R {}", hex(&c.row[16 * b..16 * b + 16])).unwrap();
        }
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("script.txt"), script).unwrap();
    std::fs::write(dir.path().join("gdn.v"), gdn_verilog()).unwrap();
    let tb = format!(
        r#"module tb;
  reg clk = 0, rst = 1;
  always #5 clk = ~clk;
  reg [{nw}:0] y, log_k;
  reg [31:0] scale, log_decay, log_beta, in_carried;
  reg f16 = 0, flush = 0, log_wr = 0, in_valid = 0, in_first = 0;
  reg [7:0] entries, log_at;
  reg [511:0] in_data;
  reg [{tw}:0] in_values;
  wire out_valid, out_last, busy;
  wire [511:0] out_data;
  wire [31:0] out_read;
  logos_gdn_lazy_row #(.N({N}), .T_MAX({T})) u(.clk(clk), .rst(rst), .y(y), .scale(scale), .f16(f16), .entries(entries), .flush(flush),
    .log_wr(log_wr), .log_at(log_at), .log_k(log_k), .log_decay(log_decay), .log_beta(log_beta), .in_valid(in_valid),
    .in_first(in_first), .in_data(in_data), .in_carried(in_carried), .in_values(in_values), .out_valid(out_valid),
    .out_last(out_last), .out_data(out_data), .out_read(out_read), .busy(busy));
  integer fi, fo, c, b;
  reg [7:0] tag;
  reg [31:0] a0, a1, a2;
  reg [511:0] word;
  always @(negedge clk) if (out_valid) begin
    if (flush) $fwrite(fo, "D %h\n", out_data);
    if (out_last) $fwrite(fo, "E %h\n", out_read);
  end
  initial begin
    fi = $fopen("script.txt", "r");
    fo = $fopen("out.txt", "w");
    @(negedge clk); rst = 0;
    while (!$feof(fi)) begin
      c = $fscanf(fi, "%c", tag);
      if (tag == "L") begin
        c = $fscanf(fi, " %h %h %h %h\n", a0, a1, a2, log_k);
        log_at = a0; log_decay = a1; log_beta = a2; log_wr = 1; @(negedge clk); log_wr = 0;
      end else if (tag == "C") begin
        c = $fscanf(fi, " %h %h %h %h %h %h %h\n", entries, a0, a1, scale, in_carried, y, in_values);
        flush = a0; f16 = a1; b = 0;
      end else if (tag == "R") begin
        c = $fscanf(fi, " %h\n", word);
        in_data = word; in_first = (b == 0); in_valid = 1; @(negedge clk); in_valid = 0; in_first = 0;
        b = b + 1;
        if (b == {beats}) begin @(negedge clk); while (busy) @(negedge clk); @(negedge clk); end
      end
    end
    @(negedge clk); @(negedge clk);
    $fclose(fo);
    $finish;
  end
endmodule
"#,
        nw = 32 * N - 1,
        tw = 32 * T - 1,
        beats = N / 16,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-o", "sim", "tb.v", "gdn.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    let out = std::fs::read_to_string(dir.path().join("out.txt")).unwrap();
    let mut lines = out.lines();
    let mut wrong = Vec::new();
    for (ci, c) in cases.iter().enumerate() {
        let log: Vec<Logged> = c.log.iter().map(|(k, log_decay, beta, v)| Logged { k, v: std::slice::from_ref(v), log_decay: *log_decay, beta: *beta }).collect();
        let want_read = gdn_read(FpUnit::Soft, &c.row, &c.y, c.carried, c.scale);
        let mut want_row = c.row.clone();
        if c.flush {
            let f16 = c.f16;
            gdn_flush(FpUnit::Soft, &mut want_row, &log, 0, |x| if f16 { logicaffeine_compile::model::quant::f16_to_f32(f32_to_f16(x)) } else { x });
            for b in 0..N / 16 {
                let line = lines.next().unwrap_or_else(|| panic!("case {ci}: beat {b} missing"));
                let data = line.strip_prefix("D ").unwrap_or_else(|| panic!("case {ci}: `{line}` where a row beat belongs"));
                for l in 0..16 {
                    let got = hex_field(data, l, 8);
                    if got != want_row[16 * b + l].to_bits() {
                        wrong.push(format!("case {ci} ({} tokens, f16 {}) element {}: rtl {got:08x}, numerics {:08x}", c.log.len(), c.f16, 16 * b + l, want_row[16 * b + l].to_bits()));
                    }
                }
            }
        }
        let line = lines.next().unwrap_or_else(|| panic!("case {ci}: no read-out"));
        let got = u32::from_str_radix(line.strip_prefix("E ").unwrap(), 16).unwrap();
        if got != want_read.to_bits() {
            wrong.push(format!("case {ci} read-out: rtl {got:08x}, numerics {:08x}", want_read.to_bits()));
        }
    }
    assert!(wrong.is_empty(), "{} values differ:\n{}", wrong.len(), wrong.iter().take(20).cloned().collect::<Vec<_>>().join("\n"));
}

/// `logos_attend_int8` under iverilog: seeded 64-element heads over an int8 cache of 1 to 64 positions, the query in
/// one and two words scaled as one block, and a cache whose later blocks of positions weigh nothing — each output
/// element `numerics::attend_int8`'s, bit for bit.
#[test]
fn the_int8_attention_unit_attends_as_the_numerics_do() {
    if !common::require_tool("iverilog") {
        return;
    }
    use logicaffeine_compile::spatial::decode_engine::numerics::{attend_int8, cache_row, quantize_head, ActScaling};
    const HD: usize = 64;
    let mut r = Rng(0x1a8e);
    let mut f = || ((r.next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0;
    struct Case {
        q: Vec<f32>,
        keys: Vec<(u16, Vec<i8>)>,
        values: Vec<(u16, Vec<i8>)>,
        scale: f32,
        words: Words,
    }
    let mut cases = Vec::new();
    for &p in &[1usize, 5, 31, 32, 33, 64] {
        for words in [Words::One, Words::Two] {
            let q: Vec<f32> = (0..HD).map(|_| f()).collect();
            let keys = (0..p).map(|_| cache_row(FpUnit::Soft, &(0..HD).map(|_| f() * 2.0).collect::<Vec<_>>())).collect();
            let values = (0..p).map(|_| cache_row(FpUnit::Soft, &(0..HD).map(|_| f() * 3.0).collect::<Vec<_>>())).collect();
            cases.push(Case { q, keys, values, scale: 0.125, words });
        }
    }
    // One key along the query, the rest against it: every block after the first weighs nothing.
    let keys: Vec<(u16, Vec<i8>)> = (0..40).map(|j| cache_row(FpUnit::Soft, &vec![if j == 0 { 1.0 } else { -1.0 }; HD])).collect();
    let values = (0..40).map(|_| cache_row(FpUnit::Soft, &(0..HD).map(|_| f()).collect::<Vec<_>>())).collect();
    cases.push(Case { q: vec![1.0; HD], keys, values, scale: 4.0, words: Words::One });
    let hex8 = |v: &[i8]| -> String {
        let mut s = String::new();
        for &x in v.iter().rev() {
            write!(s, "{:02x}", x as u8).unwrap();
        }
        s
    };
    let mut script = String::new();
    for c in &cases {
        let qa = quantize_head(&c.q, c.words, ActScaling::PerBlock);
        let mut qhex = String::new();
        for &x in qa.codes.iter().rev() {
            write!(qhex, "{:04x}", x as i16 as u16).unwrap();
        }
        writeln!(script, "C {:x} {:02x} {:03x} {:08x} {qhex}", c.keys.len(), qa.scales[0].m, qa.scales[0].e as i16 as u16 & 0xfff, c.scale.to_bits()).unwrap();
        for (s, k) in &c.keys {
            writeln!(script, "K {s:04x} {}", hex8(k)).unwrap();
        }
        for block in c.values.chunks(32) {
            for (s, _) in block {
                writeln!(script, "S {s:04x}").unwrap();
            }
            for (_, v) in block {
                writeln!(script, "V {}", hex8(v)).unwrap();
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("script.txt"), script).unwrap();
    std::fs::write(dir.path().join("attend.v"), attend_verilog()).unwrap();
    let tb = format!(
        r#"module tb;
  reg clk = 0, rst = 1;
  always #5 clk = ~clk;
  reg start = 0, k_valid = 0, s_valid = 0, v_valid = 0;
  reg [15:0] positions, k_scale, v_scale;
  reg [{qw}:0] q_codes;
  reg [7:0] q_m;
  reg [11:0] q_e;
  reg [31:0] scale;
  reg [{cw}:0] k_codes, v_codes;
  wire busy, k_ready, s_ready, v_ready, o_valid, o_last;
  wire [511:0] o_beat;
  logos_attend_int8 #(.HD({HD}), .VD({HD}), .P_MAX(64)) u(.clk(clk), .rst(rst), .start(start), .positions(positions),
    .q_codes(q_codes), .q_m(q_m), .q_e($signed(q_e)), .scale(scale), .busy(busy), .k_valid(k_valid), .k_codes(k_codes),
    .k_scale(k_scale), .k_ready(k_ready), .s_valid(s_valid), .v_scale(v_scale), .s_ready(s_ready), .v_valid(v_valid),
    .v_codes(v_codes), .v_ready(v_ready), .o_valid(o_valid), .o_beat(o_beat), .o_last(o_last));
  integer fi, fo, c;
  reg [7:0] tag;
  reg [31:0] a0, a1, a2, a3;
  always @(negedge clk) if (o_valid) $fwrite(fo, "%h\n", o_beat);
  initial begin
    fi = $fopen("script.txt", "r");
    fo = $fopen("out.txt", "w");
    @(negedge clk); rst = 0;
    while (!$feof(fi)) begin
      c = $fscanf(fi, "%c", tag);
      if (tag == "C") begin
        while (busy) @(negedge clk);
        c = $fscanf(fi, " %h %h %h %h %h\n", a0, a1, a2, a3, q_codes);
        positions = a0; q_m = a1; q_e = a2; scale = a3;
        start = 1; @(negedge clk); start = 0;
      end else if (tag == "K") begin
        c = $fscanf(fi, " %h %h\n", k_scale, k_codes);
        k_valid = 1; while (!k_ready) @(negedge clk); @(negedge clk); k_valid = 0;
      end else if (tag == "S") begin
        c = $fscanf(fi, " %h\n", v_scale);
        s_valid = 1; while (!s_ready) @(negedge clk); @(negedge clk); s_valid = 0;
      end else if (tag == "V") begin
        c = $fscanf(fi, " %h\n", v_codes);
        v_valid = 1; while (!v_ready) @(negedge clk); @(negedge clk); v_valid = 0;
      end
    end
    while (busy) @(negedge clk);
    @(negedge clk); @(negedge clk);
    $fclose(fo);
    $finish;
  end
endmodule
"#,
        qw = 16 * HD - 1,
        cw = 8 * HD - 1,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-o", "sim", "tb.v", "attend.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    let out = std::fs::read_to_string(dir.path().join("out.txt")).unwrap();
    let mut lines = out.lines();
    let mut wrong = Vec::new();
    for (ci, c) in cases.iter().enumerate() {
        let keys: Vec<(u16, &[i8])> = c.keys.iter().map(|(s, k)| (*s, k.as_slice())).collect();
        let values: Vec<(u16, &[i8])> = c.values.iter().map(|(s, v)| (*s, v.as_slice())).collect();
        let want = attend_int8(FpUnit::Soft, &c.q, &keys, &values, c.scale, c.words, ActScaling::PerBlock);
        for b in 0..HD / 16 {
            let line = lines.next().unwrap_or_else(|| panic!("case {ci}: beat {b} missing"));
            for l in 0..16 {
                let got = hex_field(line, l, 8);
                if got != want[16 * b + l].to_bits() {
                    wrong.push(format!("case {ci} ({} positions, {:?}) element {}: rtl {got:08x}, numerics {:08x}", c.keys.len(), c.words, 16 * b + l, want[16 * b + l].to_bits()));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{} elements differ:\n{}", wrong.len(), wrong.iter().take(20).cloned().collect::<Vec<_>>().join("\n"));
}

/// The greedy head's reduction (`logos_greedy`) leaves each token-row's `numerics::argmax` of the rows it is given and
/// that row's value, over beats as the streaming matvec's writer gives them: each token-row's rows split into stripes,
/// each stripe's beats a contiguous run of present lanes (a beat met twice where two stripes share it, the lanes not
/// present holding other numbers), the token-rows' stripes interleaved, idle clocks between beats. The values are ties
/// at the largest, NaNs of either sign and payload, both zeros, both infinities, subnormals and random bits, with rows
/// holding no number among them: 600 cases of four token-rows, cleared between, every token-row's row and value.
#[test]
fn the_greedy_head_keeps_each_token_rows_first_largest_number_as_numerics_argmax_does() {
    use logicaffeine_compile::spatial::decode_engine::numerics::argmax;
    use logicaffeine_compile::spatial::decode_engine::rtl::streaming::greedy_verilog;
    let dir = tempfile::tempdir().unwrap();
    let mut r = Rng(0x9eed);
    let specials = [0u32, 0x8000_0000, 0x7f80_0000, 0xff80_0000, 0x7fc0_0000, 0xffc0_0001, 0x7f80_0001, 1, 0x8000_0001, 0x007f_ffff, 0x7f7f_ffff, 0xff7f_ffff, 0x3f80_0000, 0xbf80_0000];
    let nans = [0x7fc0_0000u32, 0xffc0_0001, 0x7f80_0001, 0xff80_00ff];
    let mut cmds: Vec<String> = Vec::new();
    let beat = |kind: u32, t: u32, at: u32, present: u32, lanes: &[u32; 16]| {
        let mut h = format!("{kind:02x}{t:02x}{at:08x}{present:04x}");
        for l in lanes.iter().rev() {
            write!(h, "{l:08x}").unwrap();
        }
        h
    };
    let mut want: Vec<(u32, u32)> = Vec::new();
    for _ in 0..600 {
        cmds.push(beat(2, 0, 0, 0, &[0; 16]));
        let ties: Vec<u32> = (0..3).map(|_| r.next() as u32 & 0x7fff_ffff).collect();
        let rows: Vec<Vec<u32>> = (0..4)
            .map(|_| {
                let n = 1 + (r.next() % 70) as usize;
                let nan_only = r.next() % 9 == 0;
                (0..n)
                    .map(|_| match (nan_only, r.next() % 10) {
                        (true, _) => nans[(r.next() % nans.len() as u64) as usize],
                        (_, 0..=2) => specials[(r.next() % specials.len() as u64) as usize],
                        (_, 3..=5) => ties[(r.next() % 3) as usize] ^ ((r.next() as u32 & 1) << 31),
                        _ => r.next() as u32,
                    })
                    .collect()
            })
            .collect();
        let stripe = [3usize, 4, 8, 16, 24, 32][(r.next() % 6) as usize];
        let stripes = rows.iter().map(|x| x.len().div_ceil(stripe)).max().unwrap();
        for s in 0..stripes {
            for (t, x) in rows.iter().enumerate() {
                let (from, to) = (s * stripe, ((s + 1) * stripe).min(x.len()));
                if from >= to {
                    continue;
                }
                for b in from / 16..=(to - 1) / 16 {
                    let mut lanes = [0u32; 16];
                    let mut present = 0u32;
                    for (l, lane) in lanes.iter_mut().enumerate() {
                        let row = 16 * b + l;
                        if row >= from && row < to {
                            *lane = x[row];
                            present |= 1 << l;
                        } else {
                            *lane = r.next() as u32;
                        }
                    }
                    cmds.push(beat(1, t as u32, b as u32, present, &lanes));
                    for _ in 0..r.next() % 3 {
                        cmds.push(beat(0, 0, 0, 0, &[0; 16]));
                    }
                }
            }
        }
        for (t, x) in rows.iter().enumerate() {
            cmds.push(beat(3, t as u32, 0, 0, &[0; 16]));
            let v: Vec<f32> = x.iter().map(|&b| f32::from_bits(b)).collect();
            let best = argmax(&v);
            want.push((best as u32, x[best]));
        }
    }
    std::fs::write(dir.path().join("cmds.hex"), cmds.join("\n") + "\n").unwrap();
    std::fs::write(dir.path().join("greedy.v"), greedy_verilog()).unwrap();
    let tb = format!(
        r#"module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  reg rst = 1, clear = 0, in_valid = 0;
  reg [7:0] in_t = 0, rd_t = 0;
  reg [31:0] in_at = 0;
  reg [15:0] in_present = 0;
  reg [511:0] in_lanes = 0;
  wire busy;
  wire [31:0] rd_row, rd_val;
  logos_greedy #(.T_MAX(4)) u(.clk(clk), .rst(rst), .clear(clear), .in_valid(in_valid), .in_t(in_t), .in_at(in_at),
    .in_lanes(in_lanes), .in_present(in_present), .busy(busy), .rd_t(rd_t), .rd_row(rd_row), .rd_val(rd_val));
  reg [575:0] cmd [0:{last}];
  integer i, f;
  initial begin
    $readmemh("cmds.hex", cmd);
    f = $fopen("out.txt", "w");
    @(negedge clk); rst = 0;
    for (i = 0; i <= {last}; i = i + 1) begin
      case (cmd[i][575:568])
        8'd1: begin
          in_valid = 1; in_t = cmd[i][567:560]; in_at = cmd[i][559:528]; in_present = cmd[i][527:512]; in_lanes = cmd[i][511:0];
          @(negedge clk); in_valid = 0;
        end
        8'd2: begin clear = 1; @(negedge clk); clear = 0; end
        8'd3: begin
          while (busy) @(negedge clk);
          rd_t = cmd[i][567:560]; #1;
          $fwrite(f, "%h %h\n", rd_row, rd_val);
        end
        default: @(negedge clk);
      endcase
    end
    $fclose(f);
    $finish;
  end
endmodule
"#,
        last = cmds.len() - 1
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-o", "sim", "tb.v", "greedy.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    let got: Vec<(u32, u32)> = std::fs::read_to_string(dir.path().join("out.txt"))
        .unwrap()
        .lines()
        .map(|l| {
            let (a, b) = l.split_once(' ').unwrap();
            (u32::from_str_radix(a, 16).unwrap(), u32::from_str_radix(b, 16).unwrap())
        })
        .collect();
    assert_eq!(got.len(), want.len(), "a read-out per token-row of every case");
    let wrong: Vec<String> = got
        .iter()
        .zip(&want)
        .enumerate()
        .filter(|(_, (g, w))| g != w)
        .map(|(k, (g, w))| format!("case {} token-row {}: row {} value {:#010x}, argmax row {} value {:#010x}", k / 4, k % 4, g.0, g.1, w.0, w.1))
        .collect();
    assert!(wrong.is_empty(), "{} of {} differ:\n{}", wrong.len(), want.len(), wrong.iter().take(20).cloned().collect::<Vec<_>>().join("\n"));
}

/// Heads of lazily flushed rows streamed back to back through `logos_gdn_read_w`, `16·w` elements a clock, the beats
/// held back at random and the unit holding them off when its buffers are full: every row's read-out `gdn_read`'s.
fn wide_read_case(n: usize, w: usize, heads: usize) -> Vec<String> {
    read_case(n, w, heads, false, 0, 0)
}

/// [`wide_read_case`] on the wide read-out, or with `narrow` on `logos_gdn_read_p` (`w` 1); with `aborted` beats, the
/// stream's first beats sent, `hold` clocks waited and the unit reset for one clock with them in flight before the
/// stream is sent whole, and only what it reads out after the reset taken.
fn read_case(n: usize, w: usize, heads: usize, narrow: bool, aborted: usize, hold: usize) -> Vec<String> {
    let specials = [0u32, 0x8000_0000, 1, 0x8000_0001, 0x0080_0000, 0x7f80_0000, 0xff80_0000, 0x7fc0_0000, 0x7f7f_ffff, 0xff7f_ffff];
    let mut r = Rng(0x91de ^ (n as u64) << 8 ^ w as u64);
    let value = |r: &mut Rng, magnitude: f32| -> f32 {
        if r.next() % 97 == 0 {
            f32::from_bits(specials[(r.next() % specials.len() as u64) as usize])
        } else {
            (((r.next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0) * magnitude
        }
    };
    let ys: Vec<Vec<f32>> = (0..heads).map(|_| (0..n).map(|_| value(&mut r, 0.1)).collect()).collect();
    let scales: Vec<f32> = (0..heads).map(|h| if h % 2 == 0 { (1.0 / (n as f64).sqrt()) as f32 } else { value(&mut r, 2.0) }).collect();
    let rows: Vec<(usize, Vec<f32>, f32)> = (0..heads * n).map(|i| (i / n, (0..n).map(|_| value(&mut r, 0.5)).collect(), value(&mut r, 0.2))).collect();
    let (lanes, beats) = (16 * w, n / (16 * w));
    let hex = |v: &[f32]| -> String { v.iter().rev().map(|x| format!("{:08x}", x.to_bits())).collect() };
    let (mut beat_hex, mut tag_hex) = (String::new(), String::new());
    for (h, row, carried) in &rows {
        for b in 0..beats {
            writeln!(beat_hex, "{}", hex(&row[lanes * b..lanes * b + lanes])).unwrap();
            let gap = if h % 2 == 0 && r.next() % 3 == 0 { r.next() % 4 } else { 0 };
            writeln!(tag_hex, "{:02x}{:08x}{h:04x}", gap, carried.to_bits()).unwrap();
        }
    }
    let y_hex: String = ys.iter().map(|y| format!("{}\n", hex(y))).collect();
    let scale_hex: String = scales.iter().map(|s| format!("{:08x}\n", s.to_bits())).collect();
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in [("beats.hex", beat_hex), ("tags.hex", tag_hex), ("ys.hex", y_hex), ("scales.hex", scale_hex)] {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    std::fs::write(dir.path().join("gdn.v"), gdn_stream_verilog()).unwrap();
    let total = rows.len() * beats;
    let tb = format!(
        r#"module tb;
  reg clk = 0, rst = 1;
  always #5 clk = ~clk;
  reg [{bw}:0] beats [0:{last}];
  reg [55:0] tags [0:{last}];
  reg [{yw}:0] ys [0:{hl}];
  reg [31:0] scales [0:{hl}];
  reg [{yw}:0] y = 0;
  reg [31:0] scale = 0, in_carried = 0;
  reg [{bw}:0] in_beat = 0;
  reg in_valid = 0;
  wire in_ready, out_valid;
  wire [31:0] out_read;
  {module} #(.N({n}){wparam}) u(.clk(clk), .rst(rst), .y(y), .scale(scale), .in_valid(in_valid), .in_ready(in_ready),
    .in_beat(in_beat), .in_carried(in_carried), .out_valid(out_valid), .out_read(out_read));
  integer i, g, f, stalls = 0;
  reg logging = {logging};
  always @(posedge clk) if (out_valid && logging) $fwrite(f, "%h\n", out_read);
  initial begin
    $readmemh("beats.hex", beats);
    $readmemh("tags.hex", tags);
    $readmemh("ys.hex", ys);
    $readmemh("scales.hex", scales);
    f = $fopen("out.hex", "w");
    repeat (3) @(negedge clk);
    rst = 0;
{abort}    for (i = 0; i < {total}; i = i + 1) begin
      for (g = 0; g < tags[i][55:48]; g = g + 1) begin @(negedge clk); in_valid = 0; end
      @(negedge clk);
      in_valid = 1; in_beat = beats[i]; in_carried = tags[i][47:16]; y = ys[tags[i][15:0]]; scale = scales[tags[i][15:0]];
      while (!in_ready) begin stalls = stalls + 1; @(negedge clk); end
      @(posedge clk);
    end
    @(negedge clk);
    in_valid = 0;
    repeat (300) @(negedge clk);
    $display("STALLS %0d", stalls);
    $fclose(f);
    $finish;
  end
endmodule
"#,
        bw = 512 * w - 1,
        module = if narrow { "logos_gdn_read_p" } else { "logos_gdn_read_w" },
        wparam = if narrow { String::new() } else { format!(", .W({w})") },
        logging = u8::from(aborted == 0),
        abort = if aborted == 0 {
            String::new()
        } else {
            format!(
                "    for (i = 0; i < {aborted}; i = i + 1) begin\n      @(negedge clk);\n      in_valid = 1; in_beat = beats[i]; in_carried = tags[i][47:16]; y = ys[tags[i][15:0]]; scale = scales[tags[i][15:0]];\n      while (!in_ready) @(negedge clk);\n      @(posedge clk);\n    end\n    @(negedge clk); in_valid = 0;\n    repeat ({hold}) @(negedge clk);\n    rst = 1;\n    @(negedge clk); rst = 0; logging = 1;\n"
            )
        },
        last = total - 1,
        yw = 32 * n - 1,
        hl = heads - 1,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let run = rtl_sim::Simulator::build(dir.path(), &["tb.v", "gdn.v"]).run(dir.path(), &[]);
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let stalls: usize = String::from_utf8_lossy(&run.stdout).split("STALLS ").nth(1).and_then(|s| s.split_whitespace().next()?.parse().ok()).unwrap_or(0);
    let got: Vec<u32> = std::fs::read_to_string(dir.path().join("out.hex")).unwrap().lines().map(|l| u32::from_str_radix(l, 16).unwrap()).collect();
    let what = format!("{n} a row at {lanes} a clock{}", if aborted > 0 { format!(", reset {hold} clocks after {aborted} beats") } else { String::new() });
    let mut wrong = Vec::new();
    if got.len() != rows.len() {
        wrong.push(format!("{what}: {} read-outs for {} rows", got.len(), rows.len()));
    }
    for (i, ((h, row, carried), &g)) in rows.iter().zip(&got).enumerate() {
        let want = gdn_read(FpUnit::Soft, row, &ys[*h], *carried, scales[*h]).to_bits();
        if g != want {
            wrong.push(format!("{what}, row {i} of head {h}: pipelined {g:08x}, numerics {want:08x}"));
        }
    }
    if stalls == 0 {
        wrong.push(format!("{what}: the unit never held the beats off"));
    }
    wrong
}

/// The read-outs reset for one clock with rows in flight — a group of rows sent whole, its rows' last beats on their
/// way through the units — read none of them out, and read every row streamed after as the numerics do:
/// `logos_gdn_read_p` at 128 and 32 elements a row, and `logos_gdn_read_w` at two and four sixteens a clock, the reset
/// as the group's last beats enter and as they near the end.
#[test]
fn a_read_out_reset_with_rows_in_flight_reads_none_of_them_out() {
    if !common::require_tool("verilator") {
        return;
    }
    let mut wrong = Vec::new();
    for (n, w, narrow, aborted, hold) in [
        (128usize, 1usize, true, 32usize, 34usize),
        (128, 1, true, 32, 52),
        (32, 1, true, 8, 14),
        (128, 2, false, 32, 36),
        (128, 2, false, 32, 58),
        (64, 4, false, 16, 24),
    ] {
        wrong.extend(read_case(n, w, 2, narrow, aborted, hold));
    }
    assert!(wrong.is_empty(), "{} differ:\n{}", wrong.len(), wrong.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
}

/// Heads of lazily flushed rows streamed back to back through `logos_gdn_read_w` at thirty-two elements a clock (a
/// binary16 line) and at sixty-four: every row's read-out, in the rows' order, `gdn_read`'s of the row, its head's
/// folded query and scale and its carried value — each element still meeting lane `i mod 16` in index order through a
/// lane's chain of adds — at 128 and 32 elements a row (a row of one beat), and 128 and 64 at sixty-four, over values
/// of one magnitude with the special values among them, the beats held back at random.
#[test]
fn the_wide_streaming_read_out_reads_every_row_as_the_numerics_do() {
    if !common::require_tool("verilator") {
        return;
    }
    let mut wrong = Vec::new();
    for (n, w, heads) in [(128usize, 2usize, 3usize), (32, 2, 5), (128, 4, 2), (64, 4, 3)] {
        wrong.extend(wide_read_case(n, w, heads));
    }
    assert!(wrong.is_empty(), "{} differ:\n{}", wrong.len(), wrong.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
}

/// Heads of lazily flushed rows through `logos_gdn_flush_w`, `16·w` elements a clock, each head's log written and then
/// its rows streamed in, the beats held back at random and the flushed rows taken at random: every element of every
/// row `gdn_flush`'s. Returned are the elements that differ, and the clocks each head took from its first row in to its
/// last row out.
fn wide_flush_case(n: usize, w: usize, logs: &[usize], f16: bool) -> (Vec<String>, Vec<u64>) {
    wide_flush_case_after(n, w, logs, f16, 0)
}

/// [`wide_flush_case`] with the unit reset for one clock `aborted` clocks into its first head's run — its log written and
/// every row in, its passes in flight — and the head's rows sent again at once over the log the unit still holds, then
/// every other head, only what it gives out after the reset taken.
fn wide_flush_case_after(n: usize, w: usize, logs: &[usize], f16: bool, aborted: usize) -> (Vec<String>, Vec<u64>) {
    const T: usize = 16;
    let specials = [0u32, 0x8000_0000, 1, 0x0080_0000, 0x7f80_0000, 0x7fc0_0000];
    let mut r = Rng(0xf1de ^ (n as u64) << 8 ^ (w as u64) << 4 ^ f16 as u64);
    let value = |r: &mut Rng, magnitude: f32| -> f32 {
        if r.next() % 211 == 0 {
            f32::from_bits(specials[(r.next() % specials.len() as u64) as usize])
        } else {
            (((r.next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0) * magnitude
        }
    };
    struct Head {
        keys: Vec<Vec<f32>>,
        values: Vec<Vec<f32>>,
        log_decays: Vec<f32>,
        betas: Vec<f32>,
        rows: Vec<Vec<f32>>,
    }
    let heads: Vec<Head> = logs
        .iter()
        .map(|&entries| Head {
            keys: (0..entries).map(|_| (0..n).map(|_| value(&mut r, 0.09)).collect()).collect(),
            values: (0..entries).map(|_| (0..n).map(|_| value(&mut r, 1.0)).collect()).collect(),
            log_decays: (0..entries).map(|_| -value(&mut r, 0.3).abs()).collect(),
            betas: (0..entries).map(|_| value(&mut r, 1.0).abs()).collect(),
            rows: (0..n).map(|_| (0..n).map(|_| value(&mut r, 0.5)).collect()).collect(),
        })
        .collect();
    let (lanes, beats) = (16 * w, n / (16 * w));
    let hex = |v: &[f32]| -> String { v.iter().rev().map(|x| format!("{:08x}", x.to_bits())).collect() };
    let (mut key_hex, mut vlog_hex, mut tok_hex, mut beat_hex, mut head_hex) = (String::new(), String::new(), String::new(), String::new(), String::new());
    let (mut keys_at, mut toks_at) = (0usize, 0usize);
    for (h, head) in heads.iter().enumerate() {
        writeln!(head_hex, "{:04x}{:04x}{:02x}", keys_at, toks_at, head.keys.len()).unwrap();
        for (e, k) in head.keys.iter().enumerate() {
            for b in 0..beats {
                writeln!(key_hex, "{}", hex(&k[lanes * b..lanes * b + lanes])).unwrap();
                writeln!(vlog_hex, "{}", hex(&head.values[e][lanes * b..lanes * b + lanes])).unwrap();
            }
            writeln!(tok_hex, "{:08x}{:08x}", head.log_decays[e].to_bits(), head.betas[e].to_bits()).unwrap();
        }
        keys_at += head.keys.len() * beats;
        toks_at += head.keys.len();
        for (i, row) in head.rows.iter().enumerate() {
            for b in 0..beats {
                let gap = if (h + i) % 3 == 0 && r.next() % 2 == 0 { r.next() % 3 } else { 0 };
                writeln!(beat_hex, "{:02x}{}", gap, hex(&row[lanes * b..lanes * b + lanes])).unwrap();
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let pad = |t: String| if t.is_empty() { "0\n".to_string() } else { t };
    for (name, text) in [("keys.hex", pad(key_hex)), ("vlogs.hex", pad(vlog_hex)), ("toks.hex", pad(tok_hex)), ("beats.hex", beat_hex), ("heads.hex", head_hex)] {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    std::fs::write(dir.path().join("gdn.v"), gdn_stream_verilog()).unwrap();
    let rows_total = heads.len() * n;
    let tb = format!(
        r#"module tb;
  reg clk = 0, rst = 1;
  always #5 clk = ~clk;
  reg [{bw}:0] keys [0:{kl}];
  reg [{bw}:0] vlogs [0:{kl}];
  reg [63:0] toks [0:{tl}];
  reg [{bw} + 8:0] beats [0:{bl}];
  reg [39:0] hs [0:{hl}];
  reg log_wr = 0, in_valid = 0, out_ready = 0;
  reg [7:0] log_at = 0, entries = 0;
  reg [15:0] log_beat = 0;
  reg [{bw}:0] log_k = 0, log_v = 0, in_beat = 0;
  reg [31:0] log_decay = 0, log_beta = 0;
  wire in_ready, out_valid;
  wire [{bw}:0] out_beat;
  logos_gdn_flush_w #(.N({n}), .W({w}), .T_MAX({T})) u(.clk(clk), .rst(rst), .log_wr(log_wr), .log_at(log_at), .log_beat(log_beat),
    .log_k(log_k), .log_v(log_v), .log_decay(log_decay), .log_beta(log_beta), .entries(entries), .f16(1'b{f16}), .in_valid(in_valid),
    .in_ready(in_ready), .in_beat(in_beat), .out_valid(out_valid), .out_ready(out_ready), .out_beat(out_beat));
  integer h, e, b, row, g, f, outs = 0, seed = 7, held = 0, began = 0, now = 0;
  reg logging = {logging};
  always @(posedge clk) begin now = now + 1; if (out_valid && logging) begin $fwrite(f, "%h\n", out_beat); outs = outs + 1; end end
  always @(negedge clk) begin out_ready = ($random(seed) & 3) != 0; if (!out_ready) held = held + 1; end
  initial begin
    $readmemh("keys.hex", keys);
    $readmemh("vlogs.hex", vlogs);
    $readmemh("toks.hex", toks);
    $readmemh("beats.hex", beats);
    $readmemh("heads.hex", hs);
    f = $fopen("out.hex", "w");
    repeat (3) @(negedge clk);
    rst = 0;
    row = 0;
{abort}    for (h = 0; h <= {hl}; h = h + 1) begin
      entries = hs[h][7:0];
      for (e = 0; e < ((h == 0 && {held}) ? 0 : hs[h][7:0]); e = e + 1)
        for (b = 0; b < {beats}; b = b + 1) begin
          @(negedge clk);
          log_wr = 1; log_at = e; log_beat = b; log_k = keys[hs[h][39:24] + e * {beats} + b]; log_v = vlogs[hs[h][39:24] + e * {beats} + b];
          log_decay = toks[hs[h][23:8] + e][63:32]; log_beta = toks[hs[h][23:8] + e][31:0];
        end
      @(negedge clk);
      log_wr = 0;
      began = now;
      for (g = 0; g < {n}; g = g + 1) begin
        for (b = 0; b < {beats}; b = b + 1) begin
          repeat (beats[row * {beats} + b][{bw} + 8:{bw} + 1]) begin @(negedge clk); in_valid = 0; end
          @(negedge clk);
          in_valid = 1; in_beat = beats[row * {beats} + b][{bw}:0];
          while (!in_ready) @(negedge clk);
          @(posedge clk);
        end
        row = row + 1;
      end
      @(negedge clk);
      in_valid = 0;
      while (outs < row * {beats}) @(negedge clk);
      $display("HEAD %0d CLOCKS %0d", h, now - began);
    end
    $display("HELD %0d", held);
    $fclose(f);
    $finish;
  end
endmodule
"#,
        bw = 512 * w - 1,
        logging = u8::from(aborted == 0),
        held = u8::from(aborted > 0),
        abort = if aborted == 0 {
            String::new()
        } else {
            format!(
                "    entries = hs[0][7:0];\n    for (e = 0; e < hs[0][7:0]; e = e + 1)\n      for (b = 0; b < {beats}; b = b + 1) begin\n        @(negedge clk);\n        log_wr = 1; log_at = e; log_beat = b; log_k = keys[hs[0][39:24] + e * {beats} + b]; log_v = vlogs[hs[0][39:24] + e * {beats} + b];\n        log_decay = toks[hs[0][23:8] + e][63:32]; log_beta = toks[hs[0][23:8] + e][31:0];\n      end\n    @(negedge clk);\n    log_wr = 0;\n    for (g = 0; g < {n} * {beats}; g = g + 1) begin\n      @(negedge clk);\n      in_valid = 1; in_beat = beats[g][{bw}:0];\n      while (!in_ready) @(negedge clk);\n      @(posedge clk);\n    end\n    @(negedge clk);\n    in_valid = 0;\n    repeat ({aborted}) @(negedge clk);\n    rst = 1;\n    @(negedge clk);\n    rst = 0; logging = 1;\n",
                bw = 512 * w - 1
            )
        },
        kl = (keys_at.max(1)) - 1,
        tl = (toks_at.max(1)) - 1,
        bl = rows_total * beats - 1,
        hl = heads.len() - 1,
        f16 = f16 as u8,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let run = rtl_sim::Simulator::build(dir.path(), &["tb.v", "gdn.v"]).run(dir.path(), &[]);
    let what = format!("{n} a row at {lanes} a clock, f16 {f16}{}", if aborted > 0 { format!(", reset {aborted} clocks into its first head's run") } else { String::new() });
    assert!(run.status.success(), "{what}: vvp: {}{}", String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr));
    let stdout = String::from_utf8_lossy(&run.stdout).to_string();
    let clocks: Vec<u64> = stdout.lines().filter_map(|l| l.split("CLOCKS ").nth(1)?.trim().parse().ok()).collect();
    let got: Vec<Vec<u32>> = std::fs::read_to_string(dir.path().join("out.hex"))
        .unwrap()
        .lines()
        .map(|l| (0..lanes).rev().map(|k| u32::from_str_radix(&l[8 * k..8 * k + 8], 16).unwrap()).collect())
        .collect();
    let mut wrong = Vec::new();
    if got.len() != rows_total * beats {
        wrong.push(format!("{what}: {} beats out for {} rows of {beats}", got.len(), rows_total));
        return (wrong, clocks);
    }
    let store = |x: f32| if f16 { logicaffeine_compile::model::quant::f16_to_f32(f32_to_f16(x)) } else { x };
    for (h, head) in heads.iter().enumerate() {
        let log: Vec<Logged> = (0..head.keys.len())
            .map(|e| Logged { k: &head.keys[e], v: &head.values[e], log_decay: head.log_decays[e], beta: head.betas[e] })
            .collect();
        for (i, row) in head.rows.iter().enumerate() {
            let mut want = row.clone();
            gdn_flush(FpUnit::Soft, &mut want, &log, i, store);
            let at = (h * n + i) * beats;
            for (x, wv) in want.iter().enumerate() {
                let g = got[at + x / lanes][x % lanes];
                if g != wv.to_bits() {
                    wrong.push(format!("{what}, head {h} ({} tokens), row {i} element {x}: pipelined {g:08x}, numerics {:08x}", head.keys.len(), wv.to_bits()));
                }
            }
        }
    }
    let held: usize = stdout.split("HELD ").nth(1).and_then(|s| s.split_whitespace().next()?.parse().ok()).unwrap_or(0);
    if held == 0 {
        wrong.push(format!("{what}: the rows were never held back on their way out"));
    }
    (wrong, clocks)
}

/// The wide flush reset for one clock with its passes in flight — a head's log written, its rows in, its run under way
/// — gives none of it out, and with the head's rows sent again at once over the log it still holds flushes that head
/// and every head after as the numerics do: at two sixteens a clock in binary16 and at four in f32, reset in its dot
/// passes, as its updates are written and late in the run.
#[test]
fn a_wide_flush_reset_with_its_passes_in_flight_flushes_every_head_after_as_the_numerics_do() {
    if !common::require_tool("verilator") {
        return;
    }
    let mut wrong = Vec::new();
    for (n, w, f16, aborted) in [(128usize, 2usize, true, 9usize), (128, 2, true, 61), (128, 2, true, 110), (64, 4, false, 23), (64, 4, false, 70)] {
        wrong.extend(wide_flush_case_after(n, w, &[16, 3, 1], f16, aborted).0);
    }
    assert!(wrong.is_empty(), "{} differ:\n{}", wrong.len(), wrong.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
}

/// Heads of lazily flushed rows through `logos_gdn_flush_w` at thirty-two elements a clock and at sixty-four: every
/// element of every row `gdn_flush`'s — every logged update replayed in order and the row stored once, the dot passes
/// and the update passes of a head's groups running beside each other — at 128 and 32 elements a row (a row of one
/// beat), and 128 and 64 at sixty-four, logs of sixteen, seven, three, one and no tokens, the state stored in f32 and
/// in f16. A full log of 128-element rows at thirty-two a clock takes within a quarter more than its passes' clocks —
/// the rows in, sixteen tokens of every group's pass of `N/4` clocks, the rows out — so the dot and update passes run
/// beside each other with no clock between them.
#[test]
fn the_wide_streaming_flush_replays_every_row_as_the_numerics_do() {
    if !common::require_tool("verilator") {
        return;
    }
    let mut wrong = Vec::new();
    for (n, w, logs) in [(128usize, 2usize, vec![16usize, 1, 7]), (32, 2, vec![16, 3, 0]), (128, 4, vec![16, 3]), (64, 4, vec![7, 16])] {
        for f16 in [false, true] {
            let (bad, clocks) = wide_flush_case(n, w, &logs, f16);
            wrong.extend(bad);
            if n == 128 && w == 2 {
                let (rows_in, groups) = ((n * n / (16 * w)) as u64, (n / (4 * w)) as u64);
                let passes = 2 * rows_in + 16 * groups * (n as u64 / 4);
                assert!(clocks[0] * 4 < passes * 5, "a full log of {n}-element rows at {} a clock: {} clocks, its passes {passes}", 16 * w, clocks[0]);
            }
        }
    }
    assert!(wrong.is_empty(), "{} differ:\n{}", wrong.len(), wrong.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
}

/// The side taking a binary16 line a clock (`W` 2: `logos_gdn_read_w`, `logos_gdn_flush_w`, the flush's log two of the
/// fold's beats at a time, f32 rows two lines a beat in and out) over the cases the side at half a line runs: a fifth
/// token logged in f16, four heads of two groups flushed in f16, the first token logged over late-landing writes, a
/// flush through a slow write port, a third token logged and two groups flushed in f32, a flush at the card's width
/// and the first token of three heads, all under Verilator (Icarus Verilog runs the wide units a few clocks a second).
#[test]
fn the_gdn_side_at_a_line_a_clock_logs_and_flushes_a_fifth_token_in_f16() {
    gdn_side_case_at(20, 32, 1, 2, 5, true, true, Writes::Quick, 2);
}

#[test]
fn the_gdn_side_at_a_line_a_clock_flushes_four_heads_of_two_groups_in_f16() {
    gdn_side_case_at(21, 32, 2, 4, 16, true, true, Writes::Quick, 2);
}

#[test]
fn the_gdn_side_at_a_line_a_clock_logs_its_first_token_over_late_landing_writes() {
    gdn_side_case_at(22, 32, 1, 2, 1, true, true, Writes::Late, 2);
}

#[test]
fn the_gdn_side_at_a_line_a_clock_flushes_through_a_slow_write_port() {
    gdn_side_case_at(23, 32, 2, 4, 16, true, true, Writes::Slow, 2);
}

#[test]
fn the_gdn_side_at_a_line_a_clock_logs_a_third_token_in_f32() {
    gdn_side_case_at(24, 32, 1, 2, 3, false, true, Writes::Quick, 2);
}

#[test]
fn the_gdn_side_at_a_line_a_clock_flushes_two_groups_in_f32() {
    gdn_side_case_at(25, 32, 2, 2, 16, false, true, Writes::Quick, 2);
}

#[test]
fn the_gdn_side_at_a_line_a_clock_flushes_at_the_cards_width() {
    gdn_side_case_at(26, 128, 1, 2, 16, true, true, Writes::Quick, 2);
}

#[test]
fn the_gdn_side_at_a_line_a_clock_flushes_eight_heads_of_two_groups_at_the_cards_width() {
    gdn_side_case_at(28, 128, 2, 8, 16, true, true, Writes::Quick, 2);
}

#[test]
fn the_gdn_side_at_a_line_a_clock_logs_the_first_token_of_three_heads() {
    gdn_side_case_at(27, 32, 1, 3, 1, true, true, Writes::Quick, 2);
}

/// One case of `logos_attend_int8_p`: a query head of `hd` elements over `positions` cached rows of an int8 cache, `vd`
/// value elements, under Verilator; the key lines, the value scales and the value lines offered at random, each only
/// when the unit asks. Returned are the output elements that differ from `numerics::attend_int8`'s, named.
fn attend_int8_p_case(case: u64, hd: usize, vd: usize, positions: usize, sim: &rtl_sim::Simulator, dir: &std::path::Path) -> Vec<String> {
    use logicaffeine_compile::spatial::decode_engine::numerics::{attend_int8, quantize_head, ActScaling};
    let mut r = Rng(0xa77e ^ case << 8 ^ positions as u64);
    let f16s = [0x0000u16, 0x0001, 0x03ff, 0x0400, 0x2e66, 0x3c00, 0x2000, 0x1800, 0x0200];
    let q: Vec<f32> = (0..hd).map(|_| (((r.next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0) * if case % 3 == 0 { 8.0 } else { 0.7 }).collect();
    let rows = |n: usize, w: usize, r: &mut Rng| -> Vec<(u16, Vec<i8>)> {
        (0..n)
            .map(|p| {
                let scale = if r.next() % 7 == 0 { f16s[(r.next() % f16s.len() as u64) as usize] } else { 0x2000 + (r.next() % 0x1800) as u16 };
                let codes = if p % 13 == 5 { vec![0i8; w] } else { (0..w).map(|_| (r.next() % 255) as i32 as i8).map(|c| c.max(-127)).collect() };
                (scale, codes)
            })
            .collect()
    };
    let keys = rows(positions, hd, &mut r);
    let values = rows(positions, vd, &mut r);
    let scale = if case % 2 == 0 { (1.0 / (hd as f64).sqrt()) as f32 } else { 0.25 };
    let qa = quantize_head(&q, Words::One, ActScaling::PerBlock);
    let kref: Vec<(u16, &[i8])> = keys.iter().map(|(s, c)| (*s, c.as_slice())).collect();
    let vref: Vec<(u16, &[i8])> = values.iter().map(|(s, c)| (*s, c.as_slice())).collect();
    let want = attend_int8(FpUnit::Soft, &q, &kref, &vref, scale, Words::One, ActScaling::PerBlock);
    let bytes = |c: &[i8]| -> Vec<String> { c.chunks(64).map(|l| l.iter().rev().map(|&b| format!("{:02x}", b as u8)).collect()).collect() };
    let mut klines = Vec::new();
    let mut vlines = Vec::new();
    for (s, c) in &keys {
        for l in bytes(c) {
            klines.push(format!("{s:04x}{l}"));
        }
    }
    for (_, c) in &values {
        vlines.extend(bytes(c));
    }
    let qhex: String = qa.codes.iter().rev().map(|&c| format!("{:04x}", c as i16 as u16)).collect();
    let at = dir.join(format!("case{case}-{positions}"));
    std::fs::create_dir_all(&at).unwrap();
    std::fs::write(at.join("k.hex"), klines.join("\n") + "\n").unwrap();
    std::fs::write(at.join("v.hex"), vlines.join("\n") + "\n").unwrap();
    std::fs::write(at.join("s.hex"), values.iter().map(|(s, _)| format!("{s:04x}\n")).collect::<String>()).unwrap();
    std::fs::write(at.join("q.hex"), format!("{qhex}\n")).unwrap();
    std::fs::write(
        at.join("cfg.hex"),
        format!("{:02x}\n{:03x}\n{:08x}\n{positions:04x}\n{:08x}\n", qa.scales[0].m as u8, (qa.scales[0].e as i16 as u16) & 0xfff, scale.to_bits(), case as u32),
    )
    .unwrap();
    let run = sim.run(&at, &[]);
    let what = format!("{hd}/{vd} wide over {positions} positions (case {case})");
    if !run.status.success() || String::from_utf8_lossy(&run.stdout).contains("STUCK") {
        return vec![format!("{what}: {}{}", String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr))];
    }
    let got: Vec<u32> = std::fs::read_to_string(at.join("out.hex"))
        .unwrap()
        .lines()
        .flat_map(|l| (0..16).rev().map(move |k| u32::from_str_radix(&l[8 * k..8 * k + 8], 16).unwrap()).collect::<Vec<_>>())
        .collect();
    if got.len() != vd {
        return vec![format!("{what}: {} output elements", got.len())];
    }
    got.iter().zip(&want).enumerate().filter(|(_, (g, w))| **g != w.to_bits()).map(|(e, (g, w))| format!("{what}: element {e}: rtl {g:08x}, numerics {:08x}", w.to_bits())).collect()
}

/// `logos_attend_int8_p`, the int8 attention unit for the user clock, attends as `numerics::attend_int8` does, bit for
/// bit: query heads of 64 and 256 elements quantized as one block, over 1, 5, 31, 32, 33 and 70 cached positions (one
/// block of 32, a block's edge either side, three blocks), int8 rows with all-zero rows among them and f16 scales of
/// every kind (zero, subnormal, normal), two score scales, every line and scale offered at random and only when the
/// unit asks, under Verilator.
#[test]
fn the_user_clocks_int8_attention_unit_attends_as_the_numerics_do() {
    use logicaffeine_compile::spatial::decode_engine::rtl::attend::attend_int8_p_verilog;
    if !common::require_tool("verilator") {
        return;
    }
    let mut wrong = Vec::new();
    for (hd, vd) in [(64usize, 64usize), (256, 256)] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("attend.v"), attend_int8_p_verilog()).unwrap();
        let tb = format!(
            r#"module tb;
  localparam HD = {hd}, VD = {vd}, KL = HD / 64, VL = VD / 64;
  reg clk = 0, rst = 1, start = 0;
  always #5 clk = ~clk;
  reg [16*HD-1:0] cfg_q [0:0];
  reg [31:0] cfg [0:4];
  reg [16*HD-1:0] q;
  reg [527:0] klines [0:8191];
  reg [511:0] vlines [0:8191];
  reg [15:0] vscales [0:2047];
  reg k_valid = 0, s_valid = 0, v_valid = 0;
  reg [511:0] k_line = 0, v_line = 0;
  reg [15:0] k_scale = 0, v_scale = 0;
  wire busy, k_ready, s_ready, v_ready, o_valid, o_last;
  wire [511:0] o_beat;
  logos_attend_int8_p #(.HD(HD), .VD(VD), .P_MAX(128)) u(.clk(clk), .rst(rst), .start(start), .positions(cfg[3][15:0]),
    .q_codes(q), .q_m(cfg[0][7:0]), .q_e(cfg[1][11:0]), .scale(cfg[2]), .busy(busy), .k_valid(k_valid), .k_line(k_line),
    .k_scale(k_scale), .k_ready(k_ready), .s_valid(s_valid), .v_scale(v_scale), .s_ready(s_ready), .v_valid(v_valid),
    .v_line(v_line), .v_ready(v_ready), .o_valid(o_valid), .o_beat(o_beat), .o_last(o_last));
  integer ki = 0, si = 0, vi = 0, f, cycles = 0, seed;
  always @(posedge clk) if (o_valid) $fwrite(f, "%h\n", o_beat);
  // Each stream offers its next item at random, and moves on the edge the unit takes it.
  always @(negedge clk) begin
    k_valid = ki < cfg[3] * KL && ($random(seed) & 3) != 0;
    k_line = klines[ki][511:0]; k_scale = klines[ki][527:512];
    s_valid = si < cfg[3] && ($random(seed) & 3) != 0;
    v_scale = vscales[si];
    v_valid = vi < cfg[3] * VL && ($random(seed) & 3) != 0;
    v_line = vlines[vi];
  end
  always @(posedge clk) begin
    if (k_valid && k_ready) ki <= ki + 1;
    if (s_valid && s_ready) si <= si + 1;
    if (v_valid && v_ready) vi <= vi + 1;
  end
  initial begin
    $readmemh("q.hex", cfg_q);
    $readmemh("cfg.hex", cfg);
    seed = cfg[4];
    $readmemh("k.hex", klines);
    $readmemh("v.hex", vlines);
    $readmemh("s.hex", vscales);
    q = cfg_q[0];
    f = $fopen("out.hex", "w");
    repeat (3) @(negedge clk);
    rst = 0;
    @(negedge clk) start = 1;
    @(negedge clk) start = 0;
    while (busy && cycles < 2000000) begin @(negedge clk); cycles = cycles + 1; end
    if (busy) $display("STUCK");
    $fclose(f);
    $finish;
  end
endmodule
"#
        );
        std::fs::write(dir.path().join("tb.v"), tb).unwrap();
        let sim = rtl_sim::Simulator::build(dir.path(), &["attend.v", "tb.v"]);
        for (case, positions) in [(0u64, 1usize), (1, 5), (2, 31), (3, 32), (4, 33), (5, 70)] {
            wrong.extend(attend_int8_p_case(case + 10 * hd as u64, hd, vd, positions, &sim, dir.path()));
        }
    }
    assert!(wrong.is_empty(), "{} differ:\n{}", wrong.len(), wrong.iter().take(16).cloned().collect::<Vec<_>>().join("\n"));
}
