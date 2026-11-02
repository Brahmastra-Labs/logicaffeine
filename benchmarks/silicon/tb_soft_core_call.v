`timescale 1ns/1ps
// Recursion on the soft-core: sumdown(n) = if n<=0 then 0 else n + sumdown(n-1); sumdown(3) = 6.
// Exercises the Call/Return frame stack (register-window save/restore). Hand-assembled chip words:
//   0 LoadConst r0=3   1 Call entry=4 res=r5   2 Show r5   3 Halt
//   4 LoadConst r6=0   5 LtEq r1=(r0<=r6)   6 JumpIfFalse r1->9   7 LoadConst r2=0   8 Return r2
//   9 Move r4=r0   10 LoadConst r7=1   11 Sub r0=r0-r7   12 Call entry=4 res=r3   13 Add r5=r4+r3   14 Return r5
module tb;
  reg clk = 0, rst = 1;
  wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0]  = 32'h05000003;
    dut.imem[1]  = 32'h25040500;
    dut.imem[2]  = 32'h59000500;
    dut.imem[3]  = 32'h2B000000;
    dut.imem[4]  = 32'h05060000;
    dut.imem[5]  = 32'h1B010006;
    dut.imem[6]  = 32'h23090100;
    dut.imem[7]  = 32'h05020000;
    dut.imem[8]  = 32'h29000200;
    dut.imem[9]  = 32'h06040000;
    dut.imem[10] = 32'h05070001;
    dut.imem[11] = 32'h0A000007;
    dut.imem[12] = 32'h25040300;
    dut.imem[13] = 32'h08050403;
    dut.imem[14] = 32'h29000500;
    #3 rst = 0;
    wait (halted);
    #10;
    if (got && shown == 32'd6)
      $display("PASS: soft-core executed recursion (sumdown(3) = %0d)", shown);
    else
      $display("FAIL: got=%0b shown=%0d (expected 6)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
