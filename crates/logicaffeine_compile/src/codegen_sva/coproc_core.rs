//! coproc_core — the coprocessors, ISSUED as CPU instructions.
//!
//! The `crdt_merge` / `bigint_alu` / `bigint_mul` blocks are real synthesized silicon, but on
//! their own they are standalone datapaths. This module wires them into a genuine (if minimal)
//! core: a program counter, a combinational instruction ROM, an 8×256-bit register file, and a
//! decode → issue → retire pipeline. Each cycle it fetches `rom[pc]`, decodes `{op,rd,rs,rt}`,
//! ROUTES `R[rs]`/`R[rt]` into the addressed coprocessor, and RETIRES the result to `R[rd]` —
//! exactly a CPU issuing a coprocessor instruction. Proven == the software oracle under
//! iverilog, tied to the shipped `BigInt`/`GCounter` at the model level, and synthesized.
//!
//! This is a DEMONSTRATOR core (five ops); the full 113-op LOGOS soft-core with these
//! coprocessors on its issue port is the in-progress integration (SILICON_CAPABILITY_MAP).
//!
//! ISA: `op` ∈ {0=HALT, 1=BADD, 2=BSUB, 3=BMUL, 4=CMERGE}; `rd/rs/rt` are 3-bit reg indices.

use super::bignum_rtl::{limb_add, limb_mul, limb_sub};
use super::crdt_rtl::merge_lane;

/// One decoded instruction: `(op, rd, rs, rt)`.
pub type Instr = (u8, u8, u8, u8);

/// Initial register-file operands (R0, R1); the rest reset to zero. Chosen with A > B (so the
/// subtract has no underflow) and full-width carry/borrow activity.
pub const COPROC_A: [u64; 4] = [0xFFFF_FFFF_FFFF_FFFF, 0x1111_1111_1111_1111, 0x2, 0x3];
pub const COPROC_B: [u64; 4] = [1, 1, 1, 1];

/// The demo program: issue each coprocessor op once, retire to a distinct register, then halt.
pub const COPROC_PROGRAM: &[Instr] = &[
    (1, 2, 0, 1), // BADD   R2 = R0 + R1
    (2, 3, 0, 1), // BSUB   R3 = R0 - R1
    (3, 4, 0, 1), // BMUL   R4 = R0.lo64 * R1.lo64  (128-bit product in low limbs)
    (4, 5, 0, 1), // CMERGE R5 = per-lane max of R0.lo128, R1.lo128
    (0, 0, 0, 0), // HALT
];

/// The per-lane-max merge the RTL applies to a register's low 128 bits (four 32-bit lanes).
fn merge_low128(a: &[u64; 4], b: &[u64; 4]) -> [u64; 2] {
    let lanes = |x: &[u64; 4]| {
        [
            x[0] as u32,
            (x[0] >> 32) as u32,
            x[1] as u32,
            (x[1] >> 32) as u32,
        ]
    };
    let (la, lb) = (lanes(a), lanes(b));
    let m: Vec<u32> = (0..4).map(|i| merge_lane(la[i], lb[i])).collect();
    [
        (m[0] as u64) | ((m[1] as u64) << 32),
        (m[2] as u64) | ((m[3] as u64) << 32),
    ]
}

/// A cycle-faithful Rust MODEL of the core: fetch → decode → issue → retire, over the same
/// coprocessor semantics the RTL instantiates. Returns the final 8×256-bit register file. The
/// iverilog testbench proves the RTL reproduces exactly this.
pub fn run_core(a: &[u64; 4], b: &[u64; 4], program: &[Instr]) -> [[u64; 4]; 8] {
    let mut r = [[0u64; 4]; 8];
    r[0] = *a;
    r[1] = *b;
    for &(op, rd, rs, rt) in program {
        let (rd, rs, rt) = (rd as usize, rs as usize, rt as usize);
        match op {
            0 => break,                              // HALT
            1 => r[rd] = limb_add(&r[rs], &r[rt]).0, // BADD (sum; carry-out dropped, 256-bit reg)
            2 => r[rd] = limb_sub(&r[rs], &r[rt]).0, // BSUB (difference)
            3 => {
                let p = limb_mul(r[rs][0], r[rt][0]); // BMUL (64×64→128 in low two limbs)
                r[rd] = [p as u64, (p >> 64) as u64, 0, 0];
            }
            4 => {
                let m = merge_low128(&r[rs], &r[rt]); // CMERGE (128-bit merged into low limbs)
                r[rd] = [m[0], m[1], 0, 0];
            }
            _ => panic!("coproc_core: unknown op {op}"),
        }
    }
    r
}

fn hex256(x: &[u64; 4]) -> String {
    format!("256'h{:016x}{:016x}{:016x}{:016x}", x[3], x[2], x[1], x[0])
}

/// Emit the issuing core: PC + combinational ROM + 8×256-bit register file + decode/issue/retire,
/// instantiating the real `bigint_alu` / `bigint_mul` / `crdt_merge` blocks as functional units.
pub fn emit_coproc_core_verilog(a: &[u64; 4], b: &[u64; 4], program: &[Instr]) -> String {
    let mut rom = String::new();
    for (i, &(op, rd, rs, rt)) in program.iter().enumerate() {
        rom.push_str(&format!(
            "      4'd{i}: instr = {{4'd{op}, 3'd{rd}, 3'd{rs}, 3'd{rt}}};\n"
        ));
    }
    format!(
        r#"// coproc_core — a minimal core that ISSUES coprocessor instructions to the real
// synthesized blocks (bigint_alu / bigint_mul / crdt_merge) and RETIRES results to a register
// file: fetch (pc->rom) -> decode (op/rd/rs/rt) -> route R[rs]/R[rt] to the coprocessor ->
// latch to R[rd] -> advance pc. Proven == the software oracle by iverilog. op: 0=HALT 1=BADD
// 2=BSUB 3=BMUL 4=CMERGE. Read back the register file via rsel/rout after halt.
module coproc_core(input clk, input rst, input [2:0] rsel, output [255:0] rout, output halted);
  reg [255:0] R [0:7];
  reg [3:0]   pc;
  reg         done;
  integer j;

  // Combinational instruction ROM + decode.
  reg [12:0] instr;
  always @(*) begin
    case (pc)
{rom}      default: instr = {{4'd0, 9'd0}}; // HALT
    endcase
  end
  wire [3:0] op = instr[12:9];
  wire [2:0] rd = instr[8:6];
  wire [2:0] rs = instr[5:3];
  wire [2:0] rt = instr[2:0];

  // The coprocessor functional units, fed the decoded source registers.
  wire [255:0] alu_sum, alu_diff;
  wire         alu_cout, alu_borrow, alu_eq, alu_lt;
  bigint_alu u_alu(.a(R[rs]), .b(R[rt]), .sum(alu_sum), .cout(alu_cout),
                   .diff(alu_diff), .borrow(alu_borrow), .eq(alu_eq), .lt(alu_lt));
  wire [127:0] mul_prod;
  bigint_mul u_mul(.a(R[rs][63:0]), .b(R[rt][63:0]), .prod(mul_prod));
  wire [127:0] cm_merged;
  wire [33:0]  cm_value;
  crdt_merge u_cm(.a(R[rs][127:0]), .b(R[rt][127:0]), .merged(cm_merged), .value(cm_value));

  assign rout   = R[rsel];
  assign halted = done;

  always @(posedge clk) begin
    if (rst) begin
      pc <= 4'd0; done <= 1'b0;
      R[0] <= {a}; R[1] <= {b};
      for (j = 2; j < 8; j = j + 1) R[j] <= 256'd0;
    end else if (!done) begin
      case (op)
        4'd0: done <= 1'b1;                    // HALT
        4'd1: R[rd] <= alu_sum;                // BADD
        4'd2: R[rd] <= alu_diff;               // BSUB
        4'd3: R[rd] <= {{128'd0, mul_prod}};   // BMUL
        4'd4: R[rd] <= {{128'd0, cm_merged}};  // CMERGE
        default: ;
      endcase
      if (op != 4'd0) pc <= pc + 4'd1;
    end
  end
endmodule
"#,
        rom = rom,
        a = hex256(a),
        b = hex256(b),
    )
}

/// Self-checking testbench: run the core to halt, read back R2..R5, compare to the oracle from
/// `coproc_exp.hex` (the Rust model's final register file). Prints PASS/FAIL.
pub fn emit_coproc_core_testbench() -> String {
    r#"// tb_coproc_core — the issuing core vs the software oracle's retired register file.
`timescale 1ns/1ps
module tb_coproc_core;
  reg clk = 0, rst = 1;
  reg [2:0] rsel = 0;
  wire [255:0] rout;
  wire halted;
  reg [255:0] exp [0:7];
  integer errors = 0, i;
  coproc_core dut(.clk(clk), .rst(rst), .rsel(rsel), .rout(rout), .halted(halted));
  always #5 clk = ~clk;
  initial begin
    $readmemh("coproc_exp.hex", exp);
    #12 rst = 0;
    for (i = 0; i < 64 && !halted; i = i + 1) @(posedge clk);
    if (!halted) begin $display("FAIL: core never halted"); $finish; end
    for (i = 2; i <= 5; i = i + 1) begin
      rsel = i[2:0]; #1;
      if (rout !== exp[i]) begin errors = errors + 1; $display("FAIL R%0d", i); end
    end
    if (errors == 0)
      $display("PASS: coproc_core issued BADD/BSUB/BMUL/CMERGE and retired results matching the oracle");
    else
      $display("FAIL: %0d retired-register mismatches", errors);
    $finish;
  end
endmodule
"#
    .to_string()
}

/// The oracle hex: the model's final 8×256-bit register file, one 64-hex-digit word per line.
fn coproc_exp_hex(a: &[u64; 4], b: &[u64; 4], program: &[Instr]) -> String {
    let r = run_core(a, b, program);
    let mut s = String::new();
    for reg in &r {
        s.push_str(&format!(
            "{:016x}{:016x}{:016x}{:016x}\n",
            reg[3], reg[2], reg[1], reg[0]
        ));
    }
    s
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/coproc_core/tests.rs"]
mod tests;
