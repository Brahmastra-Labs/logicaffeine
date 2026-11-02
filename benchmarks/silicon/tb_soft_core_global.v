`timescale 1ns/1ps
// Batch 11: globals. addbase(5) reads global `base`=100 (survives Call/Return) -> 105.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h05000064;  // r0 = 100
    dut.imem[1] = 32'h2E000000;  // GlobalSet globals[0] = r0        (op 46=0x2E)
    dut.imem[2] = 32'h05010005;  // r1 = 5 (arg n)
    dut.imem[3] = 32'h25060200;  // Call addbase(entry=6), result r2 (op 37=0x25)
    dut.imem[4] = 32'h59000200;  // Show r2
    dut.imem[5] = 32'h2B000000;  // Halt
    dut.imem[6] = 32'h2D030000;  // [addbase] GlobalGet r3 = globals[0] (op 45=0x2D)
    dut.imem[7] = 32'h08040103;  // Add r4 = r1 + r3
    dut.imem[8] = 32'h29000400;  // Return r4 (value reg in ra field)                         (op 41=0x29)
    #3 rst = 0; wait (halted); #10;
    if (got && shown == 32'd105) $display("PASS: soft-core globals (addbase(5) with base=100 = %0d)", shown);
    else $display("FAIL: got=%0b shown=%0d (expected 105)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
