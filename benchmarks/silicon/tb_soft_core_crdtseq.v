`timescale 1ns/1ps
// NewCrdt (op 82=0x52) + CrdtAppend (op 83=0x53) — a single-replica SharedSequence is an ordered log;
// two appends → length 2.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h52000001;  // NewCrdt lines(r0), kind=1 (SharedSequence)
    dut.imem[1] = 32'h05010007;  // r1 = 7 (an element)
    dut.imem[2] = 32'h53000100;  // CrdtAppend seq=r0, value=r1
    dut.imem[3] = 32'h53000100;  // CrdtAppend seq=r0, value=r1
    dut.imem[4] = 32'h3E020000;  // Length r2 = length of r0
    dut.imem[5] = 32'h59000200;  // Show r2
    dut.imem[6] = 32'h2B000000;  // Halt
    #3 rst = 0; wait (halted); #10;
    if (got && shown == 32'd2) $display("PASS: soft-core CRDT sequence (length = %0d)", shown);
    else $display("FAIL: got=%0b shown=%0d (expected 2)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
