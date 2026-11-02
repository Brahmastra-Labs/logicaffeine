`timescale 1ns/1ps
// AUTO-GENERATED from the run_soft_core ≡VM differential (codegen_sva/rtl.rs). Do not edit by hand.
// Regenerate: LOGOS_EMIT_SC_TB=1 cargo test -p logicaffeine-compile --lib the_soft_core_runs_logos_programs_identically_to_the_vm
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  integer n = 0; reg [31:0] seen [0:31];
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin seen[n] = out; n = n + 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h34000000;
    dut.imem[1] = 32'h05010005;
    dut.imem[2] = 32'h0502002A;
    dut.imem[3] = 32'hE0000102;
    dut.imem[4] = 32'h05030007;
    dut.imem[5] = 32'h0504000D;
    dut.imem[6] = 32'hE0000304;
    dut.imem[7] = 32'h05050005;
    dut.imem[8] = 32'h05060063;
    dut.imem[9] = 32'hE0000506;
    dut.imem[10] = 32'h05080005;
    dut.imem[11] = 32'hDF070008;
    dut.imem[12] = 32'h59000700;
    dut.imem[13] = 32'h050A0007;
    dut.imem[14] = 32'hDF09000A;
    dut.imem[15] = 32'h59000900;
    dut.imem[16] = 32'h2B000000;
    #3 rst = 0;
    wait (halted);
    #10;
    if (n == 2 && seen[0] == 32'h00000063 && seen[1] == 32'h0000000D)
      $display("PASS: full_mapint_update");
    else begin
      $display("FAIL: full_mapint_update (got n=%0d)", n);
      $display("  seen[0] = %0d", $signed(seen[0]));
      $display("  seen[1] = %0d", $signed(seen[1]));
    end
    $finish;
  end
  initial begin #1000000000 $display("FAIL: full_mapint_update timeout"); $finish; end
endmodule
