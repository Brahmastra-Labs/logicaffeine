//! keccak_rtl — the flagship silicon FACT.
//!
//! The handout's Keccak throughput was a "design point." This module makes it real: an
//! iterative `Keccak-f[1600]` core (one round/cycle, 24-cycle permutation) emitted as
//! synthesizable Verilog whose round function is generated FROM THE SAME θ/ρ+π/χ/ι
//! constants the proven-fast native kernel uses (`logicaffeine_system::keccak::
//! keccak_f1600`), so the two cannot diverge by transcription. The RTL is checked
//! **bit-exact against the real 24-round oracle under iverilog**, synthesized to real
//! ECP5 numbers, and two of its layers are proven by our own certified SAT (θ is
//! GF(2)-linear; χ is a bijection on a row). Plus the ML-KEM NTT butterfly (Montgomery
//! `fqmul`) emitted, oracle-checked, and its Montgomery invariant SAT-proven.

/// The ρ+π step as (source lane, rotate-left amount) for each destination lane b0..b24 —
/// IDENTICAL to the constants in `logicaffeine_system::keccak::keccak_f1600`.
const RHO_PI: [(usize, u32); 25] = [
    (0, 0),
    (6, 44),
    (12, 43),
    (18, 21),
    (24, 14),
    (3, 28),
    (9, 20),
    (10, 3),
    (16, 45),
    (22, 61),
    (1, 1),
    (7, 6),
    (13, 25),
    (19, 8),
    (20, 18),
    (4, 27),
    (5, 36),
    (11, 10),
    (17, 15),
    (23, 56),
    (2, 62),
    (8, 55),
    (14, 39),
    (15, 41),
    (21, 2),
];

/// The 24 Keccak-f round constants (FIPS-202) — same table as the native kernel.
pub const RC: [u64; 24] = [
    0x0000000000000001,
    0x0000000000008082,
    0x800000000000808a,
    0x8000000080008000,
    0x000000000000808b,
    0x0000000080000001,
    0x8000000080008081,
    0x8000000000008009,
    0x000000000000008a,
    0x0000000000000088,
    0x0000000080008009,
    0x000000008000000a,
    0x000000008000808b,
    0x800000000000008b,
    0x8000000000008089,
    0x8000000000008003,
    0x8000000000008002,
    0x8000000000000080,
    0x000000000000800a,
    0x800000008000000a,
    0x8000000080008081,
    0x8000000000008080,
    0x0000000080000001,
    0x8000000080008008,
];

/// `rotl(expr, n)` on a 64-bit lane, Verilog form (n == 0 is identity).
fn rotl(expr: &str, n: u32) -> String {
    if n == 0 {
        expr.to_string()
    } else {
        format!("(({expr} << {n}) | ({expr} >> {}))", 64 - n)
    }
}

/// Emit the combinational one-round `Keccak-f[1600]` module: state `s` + round constant
/// `rc` → `o`. Lane i occupies bits `[64*i +: 64]` (little-endian lane packing).
pub fn emit_keccak_round_verilog() -> String {
    let mut w = String::new();
    w.push_str(
        "// keccak_round — one Keccak-f[1600] round (theta, rho+pi, chi, iota).\n\
         // GENERATED from the same constants as logicaffeine_system::keccak::keccak_f1600.\n\
         module keccak_round(input [1599:0] s, input [63:0] rc, output [1599:0] o);\n",
    );
    w.push_str(&keccak_theta_wires("s"));
    w.push_str(&keccak_rest_wires());
    w.push_str(&keccak_pack("o"));
    w.push_str("endmodule\n");
    w
}

/// Theta on the 1600-bit `state`: its lanes `a0..a24`, the column parities `c*` and their mix `d*`, and the lanes
/// after theta, `t0..t24`.
fn keccak_theta_wires(state: &str) -> String {
    let mut w = String::new();
    for i in 0..25 {
        w.push_str(&format!("  wire [63:0] a{i} = {state}[{}*64 +: 64];\n", i));
    }
    for col in 0..5 {
        w.push_str(&format!(
            "  wire [63:0] c{col} = a{}^a{}^a{}^a{}^a{};\n",
            col,
            col + 5,
            col + 10,
            col + 15,
            col + 20
        ));
    }
    for col in 0..5 {
        let left = (col + 4) % 5;
        let right = (col + 1) % 5;
        w.push_str(&format!(
            "  wire [63:0] d{col} = c{left} ^ {};\n",
            rotl(&format!("c{right}"), 1)
        ));
    }
    for i in 0..25 {
        let col = i % 5;
        w.push_str(&format!("  wire [63:0] t{i} = a{i} ^ d{col};\n"));
    }
    w
}

/// Rho and pi (`b0..b24` from `t0..t24`), chi row by row, and iota on lane 0 (`rc`): the round after theta, `o0..o24`.
fn keccak_rest_wires() -> String {
    let mut w = String::new();
    for (dst, &(src, rot)) in RHO_PI.iter().enumerate() {
        w.push_str(&format!(
            "  wire [63:0] b{dst} = {};\n",
            rotl(&format!("t{src}"), rot)
        ));
    }
    for r in 0..5 {
        for c in 0..5 {
            let i = 5 * r + c;
            let n1 = 5 * r + (c + 1) % 5;
            let n2 = 5 * r + (c + 2) % 5;
            let chi = format!("b{i} ^ (~b{n1} & b{n2})");
            if i == 0 {
                w.push_str(&format!("  wire [63:0] o{i} = ({chi}) ^ rc;\n"));
            } else {
                w.push_str(&format!("  wire [63:0] o{i} = {chi};\n"));
            }
        }
    }
    w
}

/// `assign <out> = {o24, ..., o0};` — lane 0 at the least significant bits.
fn keccak_pack(out: &str) -> String {
    let mut w = format!("  assign {out} = {{");
    for i in (0..25).rev() {
        w.push_str(&format!("o{i}"));
        if i != 0 {
            w.push_str(", ");
        }
    }
    w.push_str("};\n");
    w
}

/// Emit the clocked iterative core: `start` latches `s_in`, then one round/cycle for 24
/// cycles applying `RC[round]`; `done` pulses with `s_out` = the permutation result.
pub fn emit_keccak_core_verilog() -> String {
    let mut w = String::new();
    w.push_str(&emit_keccak_round_verilog());
    w.push('\n');
    w.push_str(
        "// keccak_core — 24-cycle iterative Keccak-f[1600] (one keccak_round per clock).\n\
         module keccak_core(input clk, input rst, input start, input [1599:0] s_in,\n\
         \x20                 output reg done, output reg [1599:0] s_out);\n\
         \x20 reg [4:0] round;\n\
         \x20 reg running;\n\
         \x20 reg [1599:0] st;\n\
         \x20 reg [63:0] rc;\n\
         \x20 wire [1599:0] nxt;\n\
         \x20 keccak_round rnd(.s(st), .rc(rc), .o(nxt));\n\
         \x20 always @(*) begin\n\
         \x20   case (round)\n",
    );
    for (i, rc) in RC.iter().enumerate() {
        w.push_str(&format!("      5'd{i}: rc = 64'h{rc:016x};\n"));
    }
    w.push_str(
        "      default: rc = 64'h0;\n\
         \x20   endcase\n\
         \x20 end\n\
         \x20 always @(posedge clk) begin\n\
         \x20   if (rst) begin running <= 1'b0; done <= 1'b0; round <= 5'd0; end\n\
         \x20   else begin\n\
         \x20     done <= 1'b0;\n\
         \x20     if (start) begin st <= s_in; round <= 5'd0; running <= 1'b1; end\n\
         \x20     else if (running) begin\n\
         \x20       st <= nxt;\n\
         \x20       if (round == 5'd23) begin running <= 1'b0; done <= 1'b1; s_out <= nxt; end\n\
         \x20       else round <= round + 5'd1;\n\
         \x20     end\n\
         \x20   end\n\
         \x20 end\n\
         endmodule\n",
    );
    w
}

/// Emit `keccak_core_half`: the iterative core with a register between theta and the rest of each round, two clocks a
/// round (48 for the permutation), each half carrying only its own wiring — theta's column parities, then rho and
/// pi's lane moves into chi. The same ports and handshake as `keccak_core`.
pub fn emit_keccak_core_half_verilog() -> String {
    let mut w = String::new();
    w.push_str("// keccak_theta — theta of one Keccak-f[1600] round.\nmodule keccak_theta(input [1599:0] s, output [1599:0] t);\n");
    w.push_str(&keccak_theta_wires("s"));
    w.push_str("  assign t = {");
    for i in (0..25).rev() {
        w.push_str(&format!("t{i}"));
        if i != 0 {
            w.push_str(", ");
        }
    }
    w.push_str("};\nendmodule\n\n");
    w.push_str("// keccak_rest — rho, pi, chi and iota of one Keccak-f[1600] round.\nmodule keccak_rest(input [1599:0] tt, input [63:0] rc, output [1599:0] o);\n");
    for i in 0..25 {
        w.push_str(&format!("  wire [63:0] t{i} = tt[{}*64 +: 64];\n", i));
    }
    w.push_str(&keccak_rest_wires());
    w.push_str(&keccak_pack("o"));
    w.push_str("endmodule\n\n");
    w.push_str(
        "// keccak_core_half — 48-cycle iterative Keccak-f[1600]: theta, then the rest of the round, a clock each.\n\
         module keccak_core_half(input clk, input rst, input start, input [1599:0] s_in,\n\
         \x20                      output reg done, output reg [1599:0] s_out);\n\
         \x20 reg [4:0] round;\n\
         \x20 reg running, half;\n\
         \x20 reg [1599:0] st, tt;\n\
         \x20 reg [63:0] rc;\n\
         \x20 wire [1599:0] th, nxt;\n\
         \x20 keccak_theta theta(.s(st), .t(th));\n\
         \x20 keccak_rest rest(.tt(tt), .rc(rc), .o(nxt));\n\
         \x20 always @(*) begin\n\
         \x20   case (round)\n",
    );
    for (i, rc) in RC.iter().enumerate() {
        w.push_str(&format!("      5'd{i}: rc = 64'h{rc:016x};\n"));
    }
    w.push_str(
        "      default: rc = 64'h0;\n\
         \x20   endcase\n\
         \x20 end\n\
         \x20 always @(posedge clk) begin\n\
         \x20   if (rst) begin running <= 1'b0; done <= 1'b0; round <= 5'd0; half <= 1'b0; end\n\
         \x20   else begin\n\
         \x20     done <= 1'b0;\n\
         \x20     if (start) begin st <= s_in; round <= 5'd0; half <= 1'b0; running <= 1'b1; end\n\
         \x20     else if (running) begin\n\
         \x20       if (!half) begin tt <= th; half <= 1'b1; end\n\
         \x20       else begin\n\
         \x20         st <= nxt; half <= 1'b0;\n\
         \x20         if (round == 5'd23) begin running <= 1'b0; done <= 1'b1; s_out <= nxt; end\n\
         \x20         else round <= round + 5'd1;\n\
         \x20       end\n\
         \x20     end\n\
         \x20   end\n\
         \x20 end\n\
         endmodule\n",
    );
    w
}

/// Emit a NARROW-BUS wrapper around `keccak_core` so it place-and-routes on a real FPGA:
/// the bare core exposes a 1600-bit parallel state port (fine for simulation, but 1600
/// pins exceed any FPGA's IO), so `keccak_top` streams the 25 lanes in and out over a
/// 64-bit bus addressed by a 5-bit lane index. `we` writes `din` to lane `idx`; `go`
/// launches the permutation; `done` marks completion; `dout` reads lane `idx` of the
/// result. ~140 IOs — routes on the ECP5.
pub fn emit_keccak_top_verilog() -> String {
    let mut w = String::new();
    w.push_str(&emit_keccak_core_verilog());
    w.push('\n');
    w.push_str(
        "// keccak_top — narrow-bus (64-bit lane) wrapper so Keccak-f[1600] place-and-routes.\n\
         module keccak_top(input clk, input rst, input we, input [4:0] idx,\n\
         \x20                input [63:0] din, input go, output done, output [63:0] dout);\n\
         \x20 reg [1599:0] state;\n\
         \x20 reg launch;\n\
         \x20 wire core_done;\n\
         \x20 wire [1599:0] result;\n\
         \x20 keccak_core core(.clk(clk), .rst(rst), .start(launch), .s_in(state),\n\
         \x20                  .done(core_done), .s_out(result));\n\
         \x20 assign done = core_done;\n\
         \x20 assign dout = state[64*idx +: 64];\n\
         \x20 always @(posedge clk) begin\n\
         \x20   if (rst) begin launch <= 1'b0; end\n\
         \x20   else begin\n\
         \x20     launch <= go;\n\
         \x20     if (we) state[64*idx +: 64] <= din;\n\
         \x20     else if (core_done) state <= result;\n\
         \x20   end\n\
         \x20 end\n\
         endmodule\n",
    );
    w
}

/// Self-checking testbench: load the input state + the oracle's 24-round output from
/// `keccak_vec.hex` (50 × 64-bit words: 25 input lanes then 25 expected lanes), run the
/// core, compare bit-exact. Prints PASS/FAIL.
pub fn emit_keccak_testbench() -> String {
    r#"// tb_keccak_core — the RTL Keccak-f[1600] vs the native oracle (keccak_vec.hex).
`timescale 1ns/1ps
module tb_keccak_core;
  reg clk = 0; always #5 clk = ~clk;
  reg rst = 1, start = 0;
  reg [63:0] mem [0:49];
  reg [1599:0] s_in, expect_out;
  wire done; wire [1599:0] s_out;
  integer i;
  reg seen_done = 0;
  keccak_core dut(.clk(clk), .rst(rst), .start(start), .s_in(s_in), .done(done), .s_out(s_out));
  always @(posedge clk) if (done) seen_done <= 1'b1;
  initial begin
    $readmemh("keccak_vec.hex", mem);
    for (i = 0; i < 25; i = i + 1) s_in[64*i +: 64] = mem[i];
    for (i = 0; i < 25; i = i + 1) expect_out[64*i +: 64] = mem[25 + i];
    rst = 1;
    repeat (3) @(posedge clk);
    #1 rst = 0;                    // deassert OFF the edge — no reset/sample race
    @(posedge clk); #1 start = 1;
    @(posedge clk); #1 start = 0;
    repeat (40) @(posedge clk);   // bounded — 24-round permutation + margin, never hangs
    if (!seen_done)
      $display("FAIL: Keccak core never asserted done");
    else if (s_out === expect_out)
      $display("PASS: Keccak-f[1600] RTL core matches the native oracle bit-exact");
    else
      $display("FAIL: Keccak mismatch");
    $finish;
  end
endmodule
"#
    .to_string()
}

// ── ML-KEM NTT butterfly (Montgomery fqmul) ──────────────────────────────────────────

/// q and Montgomery constant — identical to `logicaffeine_system::ntt`.
pub const KYBER_Q: i32 = 3329;
pub const KYBER_QINV_I16: i32 = -3327; // q^-1 mod 2^16, as a signed i16

/// The reference Montgomery reduction (mirrors `ntt::montgomery_reduce`) — the oracle
/// the emitted RTL is checked against.
pub fn montgomery_reduce(a: i32) -> i16 {
    let t = (a as i16).wrapping_mul(KYBER_QINV_I16 as i16);
    ((a - (t as i32) * KYBER_Q) >> 16) as i16
}

pub fn fqmul(a: i16, b: i16) -> i16 {
    montgomery_reduce(a as i32 * b as i32)
}

/// Emit the combinational Montgomery `fqmul` + one Cooley–Tukey butterfly: inputs
/// `a`, `b` (the two coefficients) and `zeta`; outputs `hi = a + t`, `lo = a - t` where
/// `t = fqmul(zeta, b)` — all wrapping 16-bit, matching `ntt_scalar`'s inner step.
pub fn emit_ntt_butterfly_verilog() -> String {
    format!(
        r#"// ntt_butterfly — Montgomery fqmul + one CT butterfly (ML-KEM, q={q}).
// t = montgomery_reduce(zeta*b); hi = a + t; lo = a - t  (16-bit wrapping).
module ntt_butterfly(input signed [15:0] a, input signed [15:0] b,
                     input signed [15:0] zeta, output signed [15:0] hi,
                     output signed [15:0] lo);
  wire signed [31:0] prod = zeta * b;                  // full 32-bit product
  wire signed [15:0] m    = prod[15:0] * (-16'sd{qmag}); // (prod as i16) * qinv (=-{qmag})
  wire signed [31:0] sub  = prod - m * 32'sd{q};       // prod - m*q
  wire signed [15:0] t    = sub[31:16];                // arithmetic >> 16
  assign hi = a + t;
  assign lo = a - t;
endmodule
"#,
        q = KYBER_Q,
        qmag = -KYBER_QINV_I16, // qinv = -3327; emit the magnitude, negate in Verilog
    )
}

/// The steps of [`emit_ntt_butterfly_pipelined_verilog`]'s datapath a register may follow: the four levels of the
/// product's adder tree; `m`'s high-byte correction; `m`; the two partial sums of `13·m + (m >>> 8)`; that sum; `t`.
pub const NTT_STEPS: usize = 9;

/// Where the pipelined butterfly's registers go: `self.0[i]` puts one after step `i` ([`NTT_STEPS`]). The operands
/// and the results are always registered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NttCuts(pub [bool; NTT_STEPS]);

impl NttCuts {
    /// One multiply to a stage: registers after the product, after `m` and after `t`.
    pub const PER_MULTIPLY: NttCuts = NttCuts([false, false, false, true, false, true, false, false, true]);
    /// A register after every step: no stage holds more than one adder.
    pub const EVERY_STEP: NttCuts = NttCuts([true; NTT_STEPS]);

    /// Edges from a butterfly's operands to its result: the operand registers, one per cut, the result registers.
    pub fn latency(self) -> u32 {
        2 + self.0.iter().filter(|&&c| c).count() as u32
    }
}

/// A partial sum of the product's adder tree: its value, `v · 2^offset` of the product, held in `bits` signed bits,
/// and the least and greatest `v` it can take.
struct NttTerm {
    name: String,
    offset: u32,
    bits: u32,
    lo: i64,
    hi: i64,
}

/// The fewest two's-complement bits that hold every integer from `lo` to `hi`.
fn signed_bits(lo: i64, hi: i64) -> u32 {
    (1..64).find(|&n| -(1i64 << (n - 1)) <= lo && hi < 1i64 << (n - 1)).expect("a range that fits 64 bits")
}

/// A datapath value: its width, the stage whose wire computes it, and the last stage that reads it.
struct NttVal {
    bits: u32,
    stage: u32,
    need: u32,
}

/// The butterfly's values by name as the emitter places them; `at` names a value as a stage sees it, recording the
/// registers that carry it there.
struct NttDatapath {
    vals: std::collections::BTreeMap<String, NttVal>,
    order: Vec<String>,
    wires: String,
}

impl NttDatapath {
    fn define(&mut self, name: &str, bits: u32, stage: u32, expr: String) {
        self.wires.push_str(&format!("  wire signed [{}:0] {name}_s{stage} = {expr};\n", bits - 1));
        self.vals.insert(name.to_string(), NttVal { bits, stage, need: stage });
        self.order.push(name.to_string());
    }

    fn at(&mut self, name: &str, stage: u32) -> String {
        let v = self.vals.get_mut(name).unwrap_or_else(|| panic!("{name} is read before it is defined"));
        assert!(stage >= v.stage, "{name} is read at stage {stage}, before stage {} computes it", v.stage);
        v.need = v.need.max(stage);
        format!("{name}_s{stage}")
    }
}

/// Emit module `name`: [`emit_ntt_butterfly_verilog`]'s butterfly as a pipeline taking one butterfly a cycle, its
/// result [`NttCuts::latency`] edges after its operands, registered after the steps `cuts` names. Each value a later
/// stage reads reaches it through one register a stage, and only those values are carried.
///
/// The product `zeta·b` is an aligned tree of adds over its sixteen partial products (b's bit 15 weighs −2¹⁵), each
/// add as wide as the range of its sum needs. `m = p[15:0]·q⁻¹ mod 2¹⁶` keeps the product's low byte and corrects the
/// high one: q⁻¹ = 1 − 2⁸ − 2¹⁰ − 2¹¹ mod 2¹⁶. The reduction subtracts only high halves: the low halves of `p` and
/// `m·q` agree (Montgomery's invariant), so `t = p[31:16] − (m·q)[31:16]`, and with q = 13·2⁸ + 1 that high half is
/// `(13·m + (m >>> 8)) >>> 8` — the low byte of `m` cannot carry into it.
pub fn emit_ntt_butterfly_pipelined_verilog(name: &str, cuts: NttCuts) -> String {
    let stage_of = |step: usize| 1 + cuts.0[..step].iter().filter(|&&c| c).count() as u32;
    let last = stage_of(NTT_STEPS);
    let mut dp = NttDatapath { vals: Default::default(), order: Vec::new(), wires: String::new() };
    for operand in ["a", "x", "y"] {
        dp.vals.insert(operand.to_string(), NttVal { bits: 16, stage: 1, need: 1 });
        dp.order.push(operand.to_string());
    }

    let s0 = stage_of(0);
    let (x, y) = (dp.at("x", s0), dp.at("y", s0));
    let mut terms: Vec<NttTerm> = (0..16u32)
        .map(|j| {
            let name = format!("r{j}");
            let pick = format!("({y}[{j}] ? {{{x}[15], {x}}} : 17'sd0)");
            dp.define(&name, 17, s0, if j == 15 { format!("-{pick}") } else { pick });
            let (lo, hi) = if j == 15 { (-32767, 32768) } else { (-32768, 32767) };
            NttTerm { name, offset: j, bits: 17, lo, hi }
        })
        .collect();
    for level in 0..4 {
        let st = stage_of(level);
        terms = terms
            .chunks(2)
            .enumerate()
            .map(|(i, pair)| {
                let (a, b) = (&pair[0], &pair[1]);
                let d = b.offset - a.offset;
                let top = signed_bits((a.lo >> d) + b.lo, (a.hi >> d) + b.hi).min(32 - b.offset);
                let name = format!("s{level}_{i}");
                let (av, bv) = (dp.at(&a.name, st), dp.at(&b.name, st));
                dp.define(&format!("{name}h"), top, st, format!("$signed({av}[{}:{d}]) + {bv}", a.bits - 1));
                let hv = dp.at(&format!("{name}h"), st);
                dp.define(&name, d + top, st, format!("{{{hv}, {av}[{}:0]}}", d - 1));
                NttTerm { name, offset: a.offset, bits: d + top, lo: a.lo + (b.lo << d), hi: a.hi + (b.hi << d) }
            })
            .collect();
    }
    let p = terms[0].name.clone();

    let st = stage_of(4);
    let pv = dp.at(&p, st);
    dp.define("pl", 16, st, format!("{pv}[15:0]"));
    dp.define("ph", 16, st, format!("{pv}[31:16]"));
    let pl = dp.at("pl", st);
    dp.define("u", 8, st, format!("{pl}[7:0] + {{{pl}[5:0], 2'b00}} + {{{pl}[4:0], 3'b000}}"));

    let st = stage_of(5);
    let (pl, u) = (dp.at("pl", st), dp.at("u", st));
    dp.define("mh", 8, st, format!("{pl}[15:8] - {u}"));
    let mh = dp.at("mh", st);
    dp.define("m", 16, st, format!("{{{mh}, {pl}[7:0]}}"));

    let st = stage_of(6);
    let m = dp.at("m", st);
    dp.define("u1", 20, st, format!("{{{{2{{{m}[15]}}}}, {m}, 2'b00}} + {{{{4{{{m}[15]}}}}, {m}}}"));
    dp.define("u2", 20, st, format!("{{{m}[15], {m}, 3'b000}} + {{{{12{{{m}[15]}}}}, {m}[15:8]}}"));

    let st = stage_of(7);
    let (u1, u2) = (dp.at("u1", st), dp.at("u2", st));
    dp.define("mq", 20, st, format!("{u1} + {u2}"));

    let st = stage_of(8);
    let (ph, mq) = (dp.at("ph", st), dp.at("mq", st));
    dp.define("t", 16, st, format!("{ph} - {{{{4{{{mq}[19]}}}}, {mq}[19:8]}}"));

    let (a, t) = (dp.at("a", last), dp.at("t", last));
    dp.define("hi", 16, last, format!("{a} + {t}"));
    dp.define("lo", 16, last, format!("{a} - {t}"));

    let mut regs = String::new();
    let mut moves = String::from("    a_s1 <= a; x_s1 <= zeta; y_s1 <= b;\n");
    for v in &dp.order {
        let val = &dp.vals[v];
        let first = if matches!(v.as_str(), "a" | "x" | "y") { 1 } else { val.stage + 1 };
        for st in first.min(val.need + 1)..=val.need {
            regs.push_str(&format!("  reg signed [{}:0] {v}_s{st};\n", val.bits - 1));
            if st > val.stage || matches!(v.as_str(), "a" | "x" | "y") && st > 1 {
                moves.push_str(&format!("    {v}_s{st} <= {v}_s{};\n", st - 1));
            }
        }
    }
    let (hi, lo) = (dp.at("hi", last), dp.at("lo", last));
    format!(
        "// {name} — Montgomery fqmul + one CT butterfly (ML-KEM, q={q}), one a cycle, latency {lat}.\n\
         module {name}(input clk, input signed [15:0] a, input signed [15:0] b,\n\
         {pad}input signed [15:0] zeta, output reg signed [15:0] hi,\n\
         {pad}output reg signed [15:0] lo);\n\
         {regs}{wires}  always @(posedge clk) begin\n{moves}    hi <= {hi}; lo <= {lo};\n  end\nendmodule\n",
        q = KYBER_Q,
        lat = cuts.latency(),
        pad = " ".repeat(name.len() + 8),
        wires = dp.wires,
    )
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/keccak_rtl/tests.rs"]
mod tests;
