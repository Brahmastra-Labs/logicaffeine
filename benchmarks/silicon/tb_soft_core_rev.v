`timescale 1ns/1ps
// Reversible core (opcodes 0..4) integrated into the CPU: RevAdd/RevSub/RevSwap/RevXor/RevNeg.
// r0=100,r1=7 -> RevAdd=107, RevSub=100, RevSwap->r0=7, RevXor(7^100)=99, RevNeg=-99.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] seen [0:7]; integer n = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin seen[n] = out; n = n + 1; end
  initial begin
    #1;
    dut.imem[0]  = 32'h05000064;  // r0 = 100
    dut.imem[1]  = 32'h05010007;  // r1 = 7
    dut.imem[2]  = 32'h00000100;  // RevAdd place=0, delta=1
    dut.imem[3]  = 32'h59000000;  // Show r0 -> 107
    dut.imem[4]  = 32'h01000100;  // RevSub place=0, delta=1
    dut.imem[5]  = 32'h59000000;  // Show r0 -> 100
    dut.imem[6]  = 32'h04000100;  // RevSwap a=0, b=1
    dut.imem[7]  = 32'h59000000;  // Show r0 -> 7
    dut.imem[8]  = 32'h02000100;  // RevXor place=0, delta=1
    dut.imem[9]  = 32'h59000000;  // Show r0 -> 99
    dut.imem[10] = 32'h03000000;  // RevNeg place=0
    dut.imem[11] = 32'h59000000;  // Show r0 -> -99
    dut.imem[12] = 32'h2B000000;  // Halt
    #3 rst = 0; wait (halted); #10;
    if (n == 5 && seen[0]==32'd107 && seen[1]==32'd100 && seen[2]==32'd7 && seen[3]==32'd99 && seen[4]==32'hFFFFFF9D)
      $display("PASS: soft-core reversible core (RevAdd/Sub/Swap/Xor/Neg = %0d,%0d,%0d,%0d,%0d)",
               seen[0], seen[1], seen[2], seen[3], $signed(seen[4]));
    else $display("FAIL: n=%0d seen=%0d,%0d,%0d,%0d,%0d", n, seen[0], seen[1], seen[2], seen[3], seen[4]);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
