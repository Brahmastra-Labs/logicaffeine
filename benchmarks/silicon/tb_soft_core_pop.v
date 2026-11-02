`timescale 1ns/1ps
// Load-store batch 2 on the soft-core: build [10,20,30], Pop the last element into r4, Show it (=30).
// Exercises ListPop (dmem[ptr+len] read + length decrement). Hand-assembled (== rtl.rs program (10)):
//   0..2 LoadConst 10/20/30   3 NewList xs(r3)=[..] count=3   4 ListPop r4=pop(xs)   5 Show r4   6 Halt
module tb;
  reg clk = 0, rst = 1;
  wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h0500000A;  // LoadConst r0 = 10
    dut.imem[1] = 32'h05010014;  // LoadConst r1 = 20
    dut.imem[2] = 32'h0502001E;  // LoadConst r2 = 30
    dut.imem[3] = 32'h30030003;  // NewList r3 = [r0..], count=3   (op 48=0x30)
    dut.imem[4] = 32'h46040300;  // ListPop r4 = pop(r3)           (op 70=0x46)
    dut.imem[5] = 32'h59000400;  // Show r4                        (op 89=0x59)
    dut.imem[6] = 32'h2B000000;  // Halt                           (op 43=0x2B)
    #3 rst = 0;
    wait (halted);
    #10;
    if (got && shown == 32'd30)
      $display("PASS: soft-core ListPop (pop [10,20,30] = %0d)", shown);
    else
      $display("FAIL: got=%0b shown=%0d (expected 30)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
