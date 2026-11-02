`timescale 1ns/1ps
// Wave 4 CRDT coprocessor: convergent counter. a.n = 1 (+5, -2 => 4), b.n = 10; Merge b into a => 14.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0]  = 32'h05000001;  // r0 = 1
    dut.imem[1]  = 32'h4A010001;  // NewStruct a(r1), 1 field        (op 74=0x4A)
    dut.imem[2]  = 32'h4B010000;  // StructInsert a.f0 = r0 (=1)     (op 75=0x4B)
    dut.imem[3]  = 32'h0502000A;  // r2 = 10
    dut.imem[4]  = 32'h4A030001;  // NewStruct b(r3), 1 field
    dut.imem[5]  = 32'h4B030200;  // StructInsert b.f0 = r2 (=10)
    dut.imem[6]  = 32'h05040005;  // r4 = 5
    dut.imem[7]  = 32'h50010400;  // CrdtBump a, amt=r4, field0, incr  (op 80=0x50) -> 6
    dut.imem[8]  = 32'h05050002;  // r5 = 2
    dut.imem[9]  = 32'h50010510;  // CrdtBump a, amt=r5, field0, negate (rb=0x10) -> 4
    dut.imem[10] = 32'h51010300;  // CrdtMerge target=a(r1), source=b(r3) (op 81=0x51) -> 4+10=14
    dut.imem[11] = 32'h4C060100;  // GetField r6 = a.f0               (op 76=0x4C)
    dut.imem[12] = 32'h59000600;  // Show r6
    dut.imem[13] = 32'h2B000000;  // Halt
    #3 rst = 0; wait (halted); #10;
    if (got && shown == 32'd14) $display("PASS: soft-core CRDT counter merge (1+5-2 merge 10 = %0d)", shown);
    else $display("FAIL: got=%0b shown=%0d (expected 14)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
