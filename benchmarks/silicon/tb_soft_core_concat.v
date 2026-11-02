`timescale 1ns/1ps
// Load-store batch 4 on the soft-core: `a followed by b` (SeqConcat) builds a fresh [1,2,3,4] on the heap;
// read item 3 (=3). Exercises SeqConcat's two unrolled element copies + fresh allocation. Hand-assembled
// (== rtl.rs program (15)):
//   0 LoadConst r0=1  1 r1=2  2 NewList a(r2)=[1,2]  3 r3=3  4 r4=4  5 NewList b(r5)=[3,4]
//   6 SeqConcat c(r6)=a++b  7 r7=3  8 Index r8=item 3 of c  9 Show r8  10 Halt
module tb;
  reg clk = 0, rst = 1;
  wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0]  = 32'h05000001;  // r0 = 1
    dut.imem[1]  = 32'h05010002;  // r1 = 2
    dut.imem[2]  = 32'h30020002;  // NewList a(r2) = [r0..], count 2   (op 48=0x30)
    dut.imem[3]  = 32'h05030003;  // r3 = 3
    dut.imem[4]  = 32'h05040004;  // r4 = 4
    dut.imem[5]  = 32'h30050302;  // NewList b(r5) = [r3..], count 2
    dut.imem[6]  = 32'h21060205;  // SeqConcat c(r6) = a(r2) ++ b(r5)  (op 33=0x21)
    dut.imem[7]  = 32'h05070003;  // r7 = 3
    dut.imem[8]  = 32'h3B080607;  // Index r8 = item r7 of c(r6)       (op 59=0x3B)
    dut.imem[9]  = 32'h59000800;  // Show r8
    dut.imem[10] = 32'h2B000000;  // Halt
    #3 rst = 0;
    wait (halted);
    #10;
    if (got && shown == 32'd3)
      $display("PASS: soft-core SeqConcat (item 3 of [1,2]++[3,4] = %0d)", shown);
    else
      $display("FAIL: got=%0b shown=%0d (expected 3)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
