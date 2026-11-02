`timescale 1ns/1ps
// CallBuiltin (op 38=0x26) — kernel-builtin coprocessor dispatching integer builtins by ordinal:
// Max(8), Min(7), Abs(5). max(3,7)=7, min(8,2)=2, abs(0-5)=5.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] seen [0:3]; integer n = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin seen[n] = out; n = n + 1; end
  initial begin
    #1;
    dut.imem[0]  = 32'h05000003;  // r0 = 3
    dut.imem[1]  = 32'h05010007;  // r1 = 7
    dut.imem[2]  = 32'h26020008;  // CallBuiltin r2 = Max(args@r0) = max(3,7)
    dut.imem[3]  = 32'h59000200;  // Show r2
    dut.imem[4]  = 32'h05030008;  // r3 = 8
    dut.imem[5]  = 32'h05040002;  // r4 = 2
    dut.imem[6]  = 32'h26050307;  // CallBuiltin r5 = Min(args@r3) = min(8,2)
    dut.imem[7]  = 32'h59000500;  // Show r5
    dut.imem[8]  = 32'h05060000;  // r6 = 0
    dut.imem[9]  = 32'h05070005;  // r7 = 5
    dut.imem[10] = 32'h0A080607;  // Sub r8 = r6 - r7 = -5
    dut.imem[11] = 32'h26090805;  // CallBuiltin r9 = Abs(args@r8) = abs(-5)
    dut.imem[12] = 32'h59000900;  // Show r9
    dut.imem[13] = 32'h2B000000;  // Halt
    #3 rst = 0; wait (halted); #10;
    if (n == 3 && seen[0] == 32'd7 && seen[1] == 32'd2 && seen[2] == 32'd5)
      $display("PASS: soft-core CallBuiltin (max=%0d min=%0d abs=%0d)", seen[0], seen[1], seen[2]);
    else
      $display("FAIL: n=%0d seen=%0d,%0d,%0d (expected 7,2,5)", n, seen[0], seen[1], seen[2]);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
