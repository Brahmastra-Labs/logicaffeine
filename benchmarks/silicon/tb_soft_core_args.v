`timescale 1ns/1ps
// Args (op 90=0x5A) — `args()` on the deterministic on-chip node = the empty list; length = 0.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h5A000000;  // Args a(r0) = []
    dut.imem[1] = 32'h3E010000;  // Length r1 = length of r0  (op 62=0x3E)
    dut.imem[2] = 32'h59000100;  // Show r1
    dut.imem[3] = 32'h2B000000;  // Halt
    #3 rst = 0; wait (halted); #10;
    if (got && shown == 32'd0) $display("PASS: soft-core Args (length of args() = %0d)", shown);
    else $display("FAIL: got=%0b shown=%0d (expected 0)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
