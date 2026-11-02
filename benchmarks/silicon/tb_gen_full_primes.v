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
    dut.imem[0] = 32'h05000000;
    dut.imem[1] = 32'h06010000;
    dut.imem[2] = 32'h05020002;
    dut.imem[3] = 32'h06030200;
    dut.imem[4] = 32'h05050028;
    dut.imem[5] = 32'h1B040305;
    dut.imem[6] = 32'h231B0400;
    dut.imem[7] = 32'h05060002;
    dut.imem[8] = 32'h06070600;
    dut.imem[9] = 32'h05080001;
    dut.imem[10] = 32'h06090800;
    dut.imem[11] = 32'h190A0703;
    dut.imem[12] = 32'h23160A00;
    dut.imem[13] = 32'h0F0C0307;
    dut.imem[14] = 32'h050D0000;
    dut.imem[15] = 32'h1D0B0C0D;
    dut.imem[16] = 32'h23130B00;
    dut.imem[17] = 32'h050E0000;
    dut.imem[18] = 32'h06090E00;
    dut.imem[19] = 32'h050E0001;
    dut.imem[20] = 32'h0807070E;
    dut.imem[21] = 32'h220B0000;
    dut.imem[22] = 32'h060B0900;
    dut.imem[23] = 32'h0801010B;
    dut.imem[24] = 32'h050C0001;
    dut.imem[25] = 32'h0803030C;
    dut.imem[26] = 32'h22040000;
    dut.imem[27] = 32'h59000100;
    dut.imem[28] = 32'h2B000000;
    #3 rst = 0;
    wait (halted);
    #10;
    if (n == 1 && seen[0] == 32'h0000000C)
      $display("PASS: full_primes");
    else begin
      $display("FAIL: full_primes (got n=%0d)", n);
      $display("  seen[0] = %0d", $signed(seen[0]));
    end
    $finish;
  end
  initial begin #1000000000 $display("FAIL: full_primes timeout"); $finish; end
endmodule
