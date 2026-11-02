`timescale 1ns/1ps
// Load-store batch 3 on the soft-core: build a Point record `[nfields, x, y]` on the heap, write both
// fields, read `p's y` (=20). Exercises NewStruct (alloc + zero), StructInsert (field-slot write), GetField
// (field-slot read). Hand-assembled (== rtl.rs program (13)):
//   0 LoadConst r1=10   1 LoadConst r2=20   2 NewStruct p(r0),2   3 StructInsert p.f0=r1
//   4 StructInsert p.f1=r2   5 GetField r3=p.f1   6 Show r3   7 Halt
module tb;
  reg clk = 0, rst = 1;
  wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h0501000A;  // LoadConst r1 = 10 (x)
    dut.imem[1] = 32'h05020014;  // LoadConst r2 = 20 (y)
    dut.imem[2] = 32'h4A000002;  // NewStruct r0, 2 fields          (op 74=0x4A)
    dut.imem[3] = 32'h4B000100;  // StructInsert obj=r0 val=r1 f=0   (op 75=0x4B)
    dut.imem[4] = 32'h4B000201;  // StructInsert obj=r0 val=r2 f=1
    dut.imem[5] = 32'h4C030001;  // GetField r3 = obj r0 field 1     (op 76=0x4C)
    dut.imem[6] = 32'h59000300;  // Show r3
    dut.imem[7] = 32'h2B000000;  // Halt
    #3 rst = 0;
    wait (halted);
    #10;
    if (got && shown == 32'd20)
      $display("PASS: soft-core struct (p's y = %0d)", shown);
    else
      $display("FAIL: got=%0b shown=%0d (expected 20)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
