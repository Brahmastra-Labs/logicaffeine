//! infinichip_rtl — the integrated **Infinichip** memory subsystem, taken all the way to a
//! signoff-clean Sky130 GDSII.
//!
//! Stage 1 tapes out the command player alone (`codegen_sva::dram::dram_executor`). Stage 2 —
//! here — composes the *product*: one `infinichip` top-level that wires three INDEPENDENTLY
//! CERTIFIED sub-blocks into a single manufacturable die:
//!   • the **command player** — the deterministic DRAM command-stream executor (`dram_executor`);
//!     its plan-word decode and countdown issue-timing are proven by our OWN SAT in
//!     `codegen_sva::dram`;
//!   • the **SECDED(72,64) ECC** on the data path (`secded_enc`/`secded_dec`, `codegen_sva::
//!     scratchpad_rtl`) — single-error-correct, double-error-detect, exhaustively verified and
//!     Hamming-SAT-proven;
//!   • the **double-buffer** (`double_buffer`) — ping-pong bank select whose safety invariant
//!     `read_bank != fill_bank` holds by construction.
//!
//! Because each sub-block carries its own proof, the integration's remaining obligation is that
//! the WIRING composes correctly — discharged by the self-checking `tb_infinichip` iverilog
//! testbench (command stream issues, ECC corrects a single fault, banks stay disjoint), and the
//! data-integrity of the composed ECC path is re-checked here against the certified SECDED oracle.
//!
//! The emitted `infinichip.v` is SELF-CONTAINED (every sub-module definition is inlined), so the
//! OpenLane/Sky130 flow tapes it out from a single PDK-agnostic source, exactly like the other
//! blocks: `scripts/openlane-run.sh infinichip clk <period_ns> infinichip.v`.

use super::dram::emit_dram_executor_verilog;
use super::scratchpad_rtl::{emit_double_buffer_verilog, emit_secded_verilog};

/// The `infinichip` top-level module string: the structural wrapper that instantiates the four
/// certified sub-blocks and exposes their composed interface (command stream, ECC write/read
/// paths, double-buffer). It adds no datapath logic, only a register at each codec's die-side
/// boundary: the write path registers the data word before the encoder, the read path registers
/// the decoder's verdict, so each codec's logic sits between one port and one register and
/// carries one external delay, never both, and a word crosses either path in one edge.
fn infinichip_top() -> &'static str {
    r#"// infinichip — the integrated deterministic memory subsystem (Stage 2 tape-out top).
// A STRUCTURAL composition of four independently-certified sub-blocks, registered at the
// ECC boundaries:
//   dram_executor  — the command player (in-order countdown FSM; decode + issue timing
//                    certified by our SAT in codegen_sva::dram),
//   secded_enc/dec — SECDED(72,64) ECC on the data path (single-error-correct, double-error-
//                    detect; exhaustively verified + Hamming-SAT-proven),
//   double_buffer  — ping-pong bank select (compute never reads the DMA-filled bank;
//                    read_bank != fill_bank by construction).
// Each sub-block carries its own proof; tb_infinichip certifies the wiring composes correctly.
module infinichip(
  input clk,
  input rst,
  // command player — compile-time DRAM command stream in, issued commands out
  input [63:0] cmd_word,
  input cmd_valid,
  output cmd_ack,
  output issue,
  output [2:0] op,
  output [4:0] bank,
  output [31:0] row,
  // ECC write path — 64-bit data in, its 72-bit protected codeword out one edge later
  input [63:0] wr_data,
  output [71:0] wr_cw,
  // ECC read path — 72-bit codeword in, corrected 64-bit data + status out one edge later
  input [71:0] rd_cw,
  output reg [63:0] rd_data,
  output reg rd_corrected,
  output reg rd_double_err,
  // scratchpad double buffer
  input swap,
  output read_bank,
  output fill_bank
);
  reg  [63:0] wr_word;
  wire [63:0] rd_word;
  wire        rd_fix, rd_bad;
  dram_executor u_cmd (.clk(clk), .rst(rst), .cmd_word(cmd_word), .cmd_valid(cmd_valid),
                       .cmd_ack(cmd_ack), .issue(issue), .op(op), .bank(bank), .row(row));
  secded_enc    u_enc (.din(wr_word), .cw(wr_cw));
  secded_dec    u_dec (.cw_in(rd_cw), .dout(rd_word), .corrected(rd_fix),
                       .double_err(rd_bad));
  double_buffer u_dbuf(.clk(clk), .rst(rst), .swap(swap),
                       .read_bank(read_bank), .fill_bank(fill_bank));
  always @(posedge clk) begin
    wr_word       <= wr_data;
    rd_data       <= rd_word;
    rd_corrected  <= rd_fix;
    rd_double_err <= rd_bad;
  end
endmodule
"#
}

/// Emit the SELF-CONTAINED `infinichip.v`: every certified sub-block definition inlined, followed
/// by the structural top. One PDK-agnostic file → tapes out on Sky130 with no cross-file deps.
pub fn emit_infinichip_verilog() -> String {
    let mut v = String::new();
    v.push_str(&emit_dram_executor_verilog());
    v.push('\n');
    v.push_str(&emit_secded_verilog());
    v.push('\n');
    v.push_str(&emit_double_buffer_verilog());
    v.push('\n');
    v.push_str(infinichip_top());
    v
}

/// Emit a SELF-CHECKING integration testbench: it drives all three datapaths through the composed
/// `infinichip` top and asserts (1) the ECC read path corrects a single-bit fault and recovers the
/// data one edge after the codeword arrives, (2) the double-buffer banks stay disjoint across a
/// swap, and (3) the command player issues a loaded command with the exact decoded payload. Prints
/// PASS/FAIL — no external vectors needed.
pub fn emit_infinichip_testbench() -> String {
    r#"// tb_infinichip — self-checking integration of the composed memory subsystem.
`timescale 1ns/1ps
module tb_infinichip;
  reg clk = 0; always #5 clk = ~clk;
  reg rst = 1;
  reg [63:0] cmd_word = 0; reg cmd_valid = 0;
  wire cmd_ack, issue; wire [2:0] op; wire [4:0] bank; wire [31:0] row;
  reg [63:0] wr_data = 64'hDEAD_BEEF_1234_5678;
  wire [71:0] wr_cw;
  reg [71:0] rd_cw = 0;
  wire [63:0] rd_data; wire rd_corrected, rd_double_err;
  reg swap = 0; wire read_bank, fill_bank;
  integer errors = 0;
  reg saw_issue = 0;

  infinichip dut(.clk(clk), .rst(rst), .cmd_word(cmd_word), .cmd_valid(cmd_valid),
                 .cmd_ack(cmd_ack), .issue(issue), .op(op), .bank(bank), .row(row),
                 .wr_data(wr_data), .wr_cw(wr_cw),
                 .rd_cw(rd_cw), .rd_data(rd_data), .rd_corrected(rd_corrected),
                 .rd_double_err(rd_double_err),
                 .swap(swap), .read_bank(read_bank), .fill_bank(fill_bank));

  // A delta-0 command issues the cycle after it loads; latch and check its payload.
  always @(posedge clk) if (issue) begin
    saw_issue <= 1'b1;
    if (op !== 3'd1 || bank !== 5'd3 || row !== 32'd7) errors = errors + 1;
  end

  initial begin
    repeat (3) @(posedge clk);
    rst = 0;
    @(posedge clk);

    // (1) ECC read path corrects a single-bit fault (position 7 = a Hamming data bit), its
    //     verdict registered one edge after the codeword arrives.
    #1 rd_cw = wr_cw ^ (72'b1 << 7);
    @(posedge clk); #1;
    if (rd_data !== wr_data)    begin errors = errors + 1; $display("FAIL: ECC did not recover data"); end
    if (rd_corrected !== 1'b1)  begin errors = errors + 1; $display("FAIL: ECC did not flag corrected"); end
    if (rd_double_err !== 1'b0) begin errors = errors + 1; $display("FAIL: ECC false double-error"); end

    // (2) Double-buffer disjointness holds now and across a swap.
    if (fill_bank !== ~read_bank) begin errors = errors + 1; $display("FAIL: banks not disjoint"); end
    swap = 1; @(posedge clk); swap = 0; @(posedge clk);
    if (fill_bank !== ~read_bank) begin errors = errors + 1; $display("FAIL: banks not disjoint after swap"); end

    // (3) Command player issues a loaded (delta-0) command with the exact payload.
    cmd_word = {24'd0, 3'd1, 5'd3, 32'd7}; cmd_valid = 1;
    repeat (6) @(posedge clk);
    cmd_valid = 0;
    repeat (2) @(posedge clk);
    if (!saw_issue) begin errors = errors + 1; $display("FAIL: command player never issued"); end

    if (errors == 0)
      $display("PASS: infinichip integration - command player + SECDED ECC + double buffer");
    else
      $display("FAIL: %0d integration errors", errors);
    $finish;
  end
endmodule
"#
    .to_string()
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/infinichip_rtl/tests.rs"]
mod tests;
