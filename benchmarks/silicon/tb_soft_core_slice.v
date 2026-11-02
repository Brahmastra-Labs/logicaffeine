`timescale 1ns/1ps
// Load-store batch 9 on the soft-core: SliceOp. `items 2 through 4 of [10,20,30,40,50]` = [20,30,40]
// (1-based inclusive); item 1 = 20 (== rtl.rs program 24). start/end packed in the rb byte (2<<4|4 = 0x24).
module tb;
  reg clk = 0, rst = 1;
  wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h0500000A;  // r0 = 10
    dut.imem[1] = 32'h05010014;  // r1 = 20
    dut.imem[2] = 32'h0502001E;  // r2 = 30
    dut.imem[3] = 32'h05030028;  // r3 = 40
    dut.imem[4] = 32'h05040032;  // r4 = 50
    dut.imem[5] = 32'h30050005;  // NewList s(r5) = [r0..], count 5     (op 48=0x30)
    dut.imem[6] = 32'h41060524;  // SliceOp r(r6) = items 2..=4 of s    (op 65=0x41, b=0x24)
    dut.imem[7] = 32'h05070001;  // r7 = 1
    dut.imem[8] = 32'h3B080607;  // Index r8 = item r7 of r(r6)         (op 59=0x3B)
    dut.imem[9] = 32'h59000800;  // Show r8
    dut.imem[10] = 32'h2B000000; // Halt
    #3 rst = 0;
    wait (halted);
    #10;
    if (got && shown == 32'd20)
      $display("PASS: soft-core SliceOp (item 1 of [20,30,40] = %0d)", shown);
    else
      $display("FAIL: got=%0b shown=%0d (expected 20)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
