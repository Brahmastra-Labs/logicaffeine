//! computedram_rtl — Phase 4: the in-DRAM compute path proven end-to-end in simulation.
//!
//! A behavioral device that IMPLEMENTS the ComputeDRAM effect: on the ACT→PRE→ACT cadence (an ACT, an
//! interrupted precharge, then a second ACT with no RD/WR between — the signature `emit_computedram_stream`
//! produces), it performs RowClone (`dst := src`). Driven by the proven `dram_executor` (which issues the
//! delta-encoded raw sub-JEDEC command stream), it lets a `ComputeDramOp` execute as real commands and
//! yields a result checked against the `RowStore`/VM oracle — "compute over RAM," proven in sim.
//!
//! This models the CHIP computing; whether the physical Micron DDR2 exhibits the effect is the Phase-3
//! characterization (which may differ). The harness plays the role of the fabric orchestrating the op:
//! it places the operand row, triggers the stream, and reads the result back.

/// Emit `computedram_model` — the behavioral ComputeDRAM device. On the ACT→PRE→ACT compute cadence it
/// performs RowClone (2 activations: `dst := src`); on ACT→PRE→ACT→PRE→ACT (3 activations) it performs
/// the sense-amp triple-row MAJORITY over the ORIGINAL operand values, writing it to all three rows —
/// so bitwise AND = MAJ(a,b,0) and OR = MAJ(a,b,1). Snapshots the originals so the tentative 2-row
/// clone can be corrected by a 3rd activation. A RD/WR closes the cadence. Preload/read ports for the
/// harness (which plays the fabric's orchestration role).
pub fn emit_computedram_model_verilog() -> String {
    r#"// computedram_model — behavioral ComputeDRAM device: RowClone (2-ACT) + triple-row MAJORITY (3-ACT)
// + AMBIT bulk NOT (op 5, dual-contact cells): mem[bank] := ~mem[row]. NOT completes the boolean
// basis {copy, AND, OR, NOT} so XOR / set-difference / popcount kernels run on this engine.
module computedram_model #(parameter AW=8, DW=64) (
  input clk, input rst,
  input issue, input [2:0] op, input [4:0] bank, input [31:0] row,
  input pre_we, input [AW-1:0] pre_addr, input [DW-1:0] pre_data,
  input [AW-1:0] rd_addr, output [DW-1:0] rd_out
);
  (* ram_style = "registers" *) reg [DW-1:0] mem [0:(1<<AW)-1];
  reg [AW-1:0] r0_i, r1_i;
  reg [DW-1:0] snap0, snap1;
  reg cad_open, pre_int; reg [1:0] ncad;
  assign rd_out = mem[rd_addr];
  function [DW-1:0] maj3(input [DW-1:0] a, input [DW-1:0] b, input [DW-1:0] c);
    maj3 = (a & b) | (a & c) | (b & c);
  endfunction
  always @(posedge clk) begin
    if (pre_we) mem[pre_addr] <= pre_data;   // operand placement (works during reset too)
    if (rst) begin
      cad_open <= 1'b0; pre_int <= 1'b0; ncad <= 2'd0; r0_i <= 0; r1_i <= 0; snap0 <= 0; snap1 <= 0;
    end else if (issue) begin
      case (op)
        3'd0: begin // ACT
          if (cad_open && pre_int && ncad == 2'd1) begin
            // 2nd activation: tentative RowClone (dst := src), snapshot the originals
            r1_i <= row[AW-1:0]; snap1 <= mem[row[AW-1:0]];
            mem[row[AW-1:0]] <= snap0;
            ncad <= 2'd2; pre_int <= 1'b0;
          end else if (cad_open && pre_int && ncad == 2'd2) begin
            // 3rd activation: triple-row majority over the ORIGINAL snapshots → all three rows
            mem[r0_i] <= maj3(snap0, snap1, mem[row[AW-1:0]]);
            mem[r1_i] <= maj3(snap0, snap1, mem[row[AW-1:0]]);
            mem[row[AW-1:0]] <= maj3(snap0, snap1, mem[row[AW-1:0]]);
            ncad <= 2'd3; pre_int <= 1'b0;
          end else begin
            // fresh activation: start a new cadence
            r0_i <= row[AW-1:0]; snap0 <= mem[row[AW-1:0]]; ncad <= 2'd1; cad_open <= 1'b1; pre_int <= 1'b0;
          end
        end
        3'd3: if (cad_open) pre_int <= 1'b1;                               // interrupted precharge
        3'd1, 3'd2: begin cad_open <= 1'b0; pre_int <= 1'b0; ncad <= 2'd0; end // RD/WR closes the cadence
        3'd5: mem[bank[AW-1:0]] <= ~mem[row[AW-1:0]];                      // Ambit bulk NOT: dst(bank) := ~src(row)
        default: ;
      endcase
    end
  end
endmodule
"#
    .to_string()
}

/// Emit `tb_computedram` — the harness (the fabric's role): preload `src` with `pattern`, replay the
/// delta-encoded ComputeDRAM stream (`cdram_plan.hex`) through `dram_executor` into the device, then read
/// `dst` and assert it equals `pattern` (the RowClone oracle). `nwords` = stream length.
pub fn emit_computedram_harness_testbench(
    nwords: usize,
    src: usize,
    dst: usize,
    pattern: u64,
) -> String {
    let last = nwords.saturating_sub(1);
    let runcycles = nwords * 8 + 40;
    format!(
        r#"`timescale 1ns/1ps
module tb_computedram;
  reg clk=0; always #5 clk=~clk;
  reg rst=1;
  reg [63:0] rom [0:{last}];
  integer idx=0;
  wire [63:0] cmd_word = rom[idx];
  wire cmd_valid = (idx <= {last});
  wire cmd_ack, issue; wire [2:0] op; wire [4:0] bank; wire [31:0] row;
  dram_executor exec(.clk(clk), .rst(rst), .cmd_word(cmd_word), .cmd_valid(cmd_valid),
                     .cmd_ack(cmd_ack), .issue(issue), .op(op), .bank(bank), .row(row));
  reg pre_we; reg [7:0] pre_addr; reg [63:0] pre_data; reg [7:0] rd_addr; wire [63:0] rd_out;
  computedram_model #(.AW(8), .DW(64)) dut(.clk(clk), .rst(rst), .issue(issue), .op(op), .bank(bank),
    .row(row), .pre_we(pre_we), .pre_addr(pre_addr), .pre_data(pre_data), .rd_addr(rd_addr), .rd_out(rd_out));
  integer errors;
  always @(posedge clk) if (!rst && cmd_ack) idx <= idx + 1;
  initial begin
    errors=0; pre_we=0; pre_addr=0; pre_data=0; rd_addr=0;
    $readmemh("cdram_plan.hex", rom);
    @(posedge clk); pre_we=1; pre_addr=8'd{src}; pre_data=64'h{pattern:016x}; @(posedge clk); pre_we=0;
    @(posedge clk); rst=0;
    repeat ({runcycles}) @(posedge clk);
    rd_addr=8'd{dst}; @(posedge clk); @(posedge clk);
    if (rd_out !== 64'h{pattern:016x}) begin
      errors=errors+1; $display("FAIL: RowClone dst(%0d)=%h exp=%h", {dst}, rd_out, 64'h{pattern:016x});
    end
    if (errors==0)
      $display("PASS: ComputeDRAM RowClone ran as raw sub-JEDEC commands through the executor; dst == src (== the RowStore/VM oracle)");
    $finish;
  end
endmodule
"#
    )
}

/// Emit `tb_computedram_reduce` — proves in-DRAM bitwise AND and OR via triple-row activation. Preloads
/// operands (`a=aval, b=bval, zero=0` for AND with a 0 control; `a2=aval, b2=bval, ones=~0` for OR with a
/// 1 control), replays the concatenated `Maj` streams from `cdram_reduce.hex`, and asserts the majority
/// result: `mem[a] == aval & bval` (AND = MAJ(a,b,0)) and `mem[a2] == aval | bval` (OR = MAJ(a,b,1)).
#[allow(clippy::too_many_arguments)]
pub fn emit_computedram_reduce_testbench(
    nwords: usize,
    a: usize,
    b: usize,
    zero: usize,
    a2: usize,
    b2: usize,
    ones: usize,
    aval: u64,
    bval: u64,
) -> String {
    let last = nwords.saturating_sub(1);
    let runcycles = nwords * 8 + 60;
    let and_exp = aval & bval;
    let or_exp = aval | bval;
    format!(
        r#"`timescale 1ns/1ps
module tb_computedram_reduce;
  reg clk=0; always #5 clk=~clk;
  reg rst=1;
  reg [63:0] rom [0:{last}];
  integer idx=0;
  wire [63:0] cmd_word = rom[idx];
  wire cmd_valid = (idx <= {last});
  wire cmd_ack, issue; wire [2:0] op; wire [4:0] bank; wire [31:0] row;
  dram_executor exec(.clk(clk), .rst(rst), .cmd_word(cmd_word), .cmd_valid(cmd_valid),
                     .cmd_ack(cmd_ack), .issue(issue), .op(op), .bank(bank), .row(row));
  reg pre_we; reg [7:0] pre_addr; reg [63:0] pre_data; reg [7:0] rd_addr; wire [63:0] rd_out;
  computedram_model #(.AW(8), .DW(64)) dut(.clk(clk), .rst(rst), .issue(issue), .op(op), .bank(bank),
    .row(row), .pre_we(pre_we), .pre_addr(pre_addr), .pre_data(pre_data), .rd_addr(rd_addr), .rd_out(rd_out));
  integer errors;
  always @(posedge clk) if (!rst && cmd_ack) idx <= idx + 1;
  task preload(input [7:0] addr, input [63:0] val); begin
    @(posedge clk); pre_we=1; pre_addr=addr; pre_data=val; @(posedge clk); pre_we=0;
  end endtask
  initial begin
    errors=0; pre_we=0; pre_addr=0; pre_data=0; rd_addr=0;
    $readmemh("cdram_reduce.hex", rom);
    preload(8'd{a},  64'h{aval:016x});   // AND operands + 0 control
    preload(8'd{b},  64'h{bval:016x});
    preload(8'd{zero}, 64'h0000000000000000);
    preload(8'd{a2}, 64'h{aval:016x});   // OR operands + all-ones control
    preload(8'd{b2}, 64'h{bval:016x});
    preload(8'd{ones}, 64'hffffffffffffffff);
    @(posedge clk); rst=0;
    repeat ({runcycles}) @(posedge clk);
    rd_addr=8'd{a}; @(posedge clk); @(posedge clk);
    if (rd_out !== 64'h{and_exp:016x}) begin errors=errors+1; $display("FAIL: in-DRAM AND = %h exp %h", rd_out, 64'h{and_exp:016x}); end
    rd_addr=8'd{a2}; @(posedge clk); @(posedge clk);
    if (rd_out !== 64'h{or_exp:016x}) begin errors=errors+1; $display("FAIL: in-DRAM OR = %h exp %h", rd_out, 64'h{or_exp:016x}); end
    if (errors==0)
      $display("PASS: in-DRAM bitwise AND (MAJ a,b,0) and OR (MAJ a,b,1) via triple-row activation == the oracle");
    $finish;
  end
endmodule
"#
    )
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/computedram_rtl/tests.rs"]
mod tests;
