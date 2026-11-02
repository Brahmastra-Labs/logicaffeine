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
    dut.imem[0] = 32'h05010014;
    dut.imem[1] = 32'h25000101;
    dut.imem[2] = 32'h00000005;
    dut.imem[3] = 32'h59000000;
    dut.imem[4] = 32'h2B000000;
    dut.imem[5] = 32'h34010000;
    dut.imem[6] = 32'h05020001;
    dut.imem[7] = 32'h06030200;
    dut.imem[8] = 32'h05060001;
    dut.imem[9] = 32'h08050006;
    dut.imem[10] = 32'h19040305;
    dut.imem[11] = 32'h23120400;
    dut.imem[12] = 32'h05080002;
    dut.imem[13] = 32'h0B070308;
    dut.imem[14] = 32'hE0010307;
    dut.imem[15] = 32'h05090001;
    dut.imem[16] = 32'h08030309;
    dut.imem[17] = 32'h22080000;
    dut.imem[18] = 32'h05070000;
    dut.imem[19] = 32'h06080700;
    dut.imem[20] = 32'h05090001;
    dut.imem[21] = 32'h06030900;
    dut.imem[22] = 32'h050C0001;
    dut.imem[23] = 32'h080B000C;
    dut.imem[24] = 32'h190A030B;
    dut.imem[25] = 32'h23240A00;
    dut.imem[26] = 32'hDF0E0103;
    dut.imem[27] = 32'h05100002;
    dut.imem[28] = 32'h0B0F0310;
    dut.imem[29] = 32'h1D0D0E0F;
    dut.imem[30] = 32'h23210D00;
    dut.imem[31] = 32'h05110001;
    dut.imem[32] = 32'h08080811;
    dut.imem[33] = 32'h05110001;
    dut.imem[34] = 32'h08030311;
    dut.imem[35] = 32'h22160000;
    dut.imem[36] = 32'h29000800;
    dut.imem[37] = 32'h2A000000;
    #3 rst = 0;
    wait (halted);
    #10;
    if (n == 1 && seen[0] == 32'h00000014)
      $display("PASS: full_mapint");
    else begin
      $display("FAIL: full_mapint (got n=%0d)", n);
      $display("  seen[0] = %0d", $signed(seen[0]));
    end
    $finish;
  end
  initial begin #1000000000 $display("FAIL: full_mapint timeout"); $finish; end
endmodule
