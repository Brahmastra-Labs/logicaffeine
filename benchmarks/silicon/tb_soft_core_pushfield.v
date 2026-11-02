`timescale 1ns/1ps
// Batch 10: ListPushField. Struct b with an empty list field; push 10, push 20; length of b's items = 2.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h31000000;  // NewEmptyList items(r0)              (op 49=0x31)
    dut.imem[1] = 32'h4A010001;  // NewStruct b(r1), 1 field            (op 74=0x4A)
    dut.imem[2] = 32'h4B010000;  // StructInsert b.f0 = r0              (op 75=0x4B)
    dut.imem[3] = 32'h0502000A;  // r2 = 10
    dut.imem[4] = 32'h2F010200;  // ListPushField b, src=r2, field=0    (op 47=0x2F)
    dut.imem[5] = 32'h05030014;  // r3 = 20
    dut.imem[6] = 32'h2F010300;  // ListPushField b, src=r3, field=0
    dut.imem[7] = 32'h4C040100;  // GetField r4 = b.f0                  (op 76=0x4C)
    dut.imem[8] = 32'h3E050400;  // Length r5 = length of the list      (op 62=0x3E)
    dut.imem[9] = 32'h59000500;  // Show r5
    dut.imem[10] = 32'h2B000000; // Halt
    #3 rst = 0; wait (halted); #10;
    if (got && shown == 32'd2) $display("PASS: soft-core ListPushField (length after 2 pushes = %0d)", shown);
    else $display("FAIL: got=%0b shown=%0d (expected 2)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
