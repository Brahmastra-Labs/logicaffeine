//! arbiter_rtl — the deterministic NoC arbiter's entropy core in silicon.
//!
//! The handout claims a "deterministic (seeded, reproducible) interconnect" — the same
//! seeded scheduler our concurrency runtime already ships
//! (`logicaffeine_system::concurrency::seeded_pick`, a SplitMix64 draw). This module
//! emits that draw source as synthesizable RTL and proves, under iverilog, that the
//! hardware draw sequence is **byte-identical** to the software scheduler's for the same
//! seed — so a message-passing schedule replays identically whether it ran on the CPU
//! runtime or the chip's NoC.
//!
//! The `below(n)` reduction (`draw % n`, with the runtime's "n<=1 ⇒ 0 without drawing"
//! rule) is a thin wrapper over this core; the core IS the reproducible entropy, so it
//! is what the differential pins.

/// The SplitMix64 golden constants — IDENTICAL to
/// `logicaffeine_system::concurrency::SplitMix64`.
pub const SM64_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
pub const SM64_MIX1: u64 = 0xBF58_476D_1CE4_E5B9;
pub const SM64_MIX2: u64 = 0x94D0_49BB_1331_11EB;

/// The software SplitMix64 draw — mirrors the runtime, the oracle the RTL is checked
/// against.
pub fn sm64_next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(SM64_GAMMA);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(SM64_MIX1);
    z = (z ^ (z >> 27)).wrapping_mul(SM64_MIX2);
    z ^ (z >> 31)
}

/// Emit the clocked SplitMix64 draw core: on each `step`, advance the state and present
/// the mixed 64-bit draw on `draw` with `draw_valid`. Combinational mixing, registered
/// state — one draw per requested step, deterministic from the loaded `seed`.
pub fn emit_seeded_arbiter_verilog() -> String {
    format!(
        r#"// seeded_arbiter_prng — the NoC arbiter's deterministic entropy core.
// SplitMix64, bit-identical to logicaffeine_system::concurrency::SplitMix64.
module seeded_arbiter_prng(
  input clk,
  input rst,
  input [63:0] seed,
  input load,
  input step,
  output reg [63:0] draw,
  output reg draw_valid
);
  reg [63:0] state;
  // Combinational mix of the NEXT state (state + gamma).
  wire [63:0] s1 = state + 64'h{gamma:016x};
  wire [63:0] z1 = (s1 ^ (s1 >> 30)) * 64'h{mix1:016x};
  wire [63:0] z2 = (z1 ^ (z1 >> 27)) * 64'h{mix2:016x};
  wire [63:0] mixed = z2 ^ (z2 >> 31);
  always @(posedge clk) begin
    if (rst) begin
      state <= 64'd0; draw <= 64'd0; draw_valid <= 1'b0;
    end else begin
      draw_valid <= 1'b0;
      if (load) begin
        state <= seed;
      end else if (step) begin
        state <= s1;
        draw <= mixed;
        draw_valid <= 1'b1;
      end
    end
  end
endmodule
"#,
        gamma = SM64_GAMMA,
        mix1 = SM64_MIX1,
        mix2 = SM64_MIX2,
    )
}

/// A self-checking testbench: load a seed, step N times, and compare each hardware draw
/// against the software oracle sequence from `arbiter_seq.hex`. Prints PASS/FAIL.
pub fn emit_seeded_arbiter_testbench(n: usize) -> String {
    format!(
        r#"// tb_seeded_arbiter — HW SplitMix64 draws vs the software scheduler oracle.
`timescale 1ns/1ps
module tb_seeded_arbiter;
  reg clk = 0; always #5 clk = ~clk;
  reg rst = 1, load = 0, step = 0;
  reg [63:0] seed;
  reg [63:0] oracle [0:{last}];
  wire [63:0] draw; wire draw_valid;
  integer got = 0, errors = 0;
  seeded_arbiter_prng dut(.clk(clk), .rst(rst), .seed(seed), .load(load), .step(step),
                          .draw(draw), .draw_valid(draw_valid));
  always @(posedge clk) if (draw_valid) begin
    if (draw !== oracle[got]) begin errors = errors + 1; $display("FAIL draw %0d", got); end
    got = got + 1;
  end
  initial begin
    $readmemh("arbiter_seq.hex", oracle);
    seed = 64'h{seed:016x};
    rst = 1; repeat (3) @(posedge clk); #1 rst = 0;
    @(posedge clk); #1 load = 1;
    @(posedge clk); #1 load = 0;
    repeat ({n}) begin @(posedge clk); #1 step = 1; @(posedge clk); #1 step = 0; end
    repeat (4) @(posedge clk);
    if (got !== {n}) $display("FAIL: got %0d of {n} draws", got);
    else if (errors == 0) $display("PASS: {n} HW SplitMix64 draws match the software scheduler bit-exact");
    $finish;
  end
endmodule
"#,
        last = n - 1,
        n = n,
        seed = ARBITER_TEST_SEED,
    )
}

/// The fixed seed the arbiter differential uses (a stand-in for `LOGOS_SEED`).
pub const ARBITER_TEST_SEED: u64 = 0x1234_5678_9ABC_DEF0;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/arbiter_rtl/tests.rs"]
mod tests;
