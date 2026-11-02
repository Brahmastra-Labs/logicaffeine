`timescale 1ns/1ps
// ApproxEq (op 30=0x1E) — over the integer sub-domain, `is approximately` collapses to exact equality,
// routing to the proven Eq datapath. (5 ~= 5) = 1, (5 ~= 7) = 0.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] seen [0:3]; integer n = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin seen[n] = out; n = n + 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h05000005;  // r0 = 5
    dut.imem[1] = 32'h05010005;  // r1 = 5
    dut.imem[2] = 32'h1E020001;  // ApproxEq r2 = (5 ~= 5)   (op 30=0x1E)
    dut.imem[3] = 32'h59000200;  // Show r2 -> 1
    dut.imem[4] = 32'h05030005;  // r3 = 5
    dut.imem[5] = 32'h05040007;  // r4 = 7
    dut.imem[6] = 32'h1E050304;  // ApproxEq r5 = (5 ~= 7)
    dut.imem[7] = 32'h59000500;  // Show r5 -> 0
    dut.imem[8] = 32'h2B000000;  // Halt
    #3 rst = 0; wait (halted); #10;
    if (n == 2 && seen[0] == 32'd1 && seen[1] == 32'd0)
      $display("PASS: soft-core ApproxEq (5~=5=%0d, 5~=7=%0d)", seen[0], seen[1]);
    else
      $display("FAIL: n=%0d seen=%0d,%0d (expected 1,0)", n, seen[0], seen[1]);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
