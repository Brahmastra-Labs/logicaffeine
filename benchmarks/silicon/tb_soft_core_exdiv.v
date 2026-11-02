`timescale 1ns/1ps
// ExactDiv (op 13=0x0D) — over the integer sub-domain (evenly-divisible), the exact quotient IS the
// truncating quotient, so it routes to the proven signed divider. 6/2=3, 100/4=25.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] seen [0:3]; integer n = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin seen[n] = out; n = n + 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h05000006;  // r0 = 6
    dut.imem[1] = 32'h05010002;  // r1 = 2
    dut.imem[2] = 32'h0D020001;  // ExactDiv r2 = 6/2      (op 13=0x0D)
    dut.imem[3] = 32'h59000200;  // Show r2 -> 3
    dut.imem[4] = 32'h05030064;  // r3 = 100
    dut.imem[5] = 32'h05040004;  // r4 = 4
    dut.imem[6] = 32'h0D050304;  // ExactDiv r5 = 100/4
    dut.imem[7] = 32'h59000500;  // Show r5 -> 25
    dut.imem[8] = 32'h2B000000;  // Halt
    #3 rst = 0; wait (halted); #10;
    if (n == 2 && seen[0] == 32'd3 && seen[1] == 32'd25)
      $display("PASS: soft-core ExactDiv (6/2=%0d, 100/4=%0d)", seen[0], seen[1]);
    else
      $display("FAIL: n=%0d seen=%0d,%0d (expected 3,25)", n, seen[0], seen[1]);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
