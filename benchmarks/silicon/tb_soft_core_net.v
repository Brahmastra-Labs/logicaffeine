`timescale 1ns/1ps
// Wave 4 networking coprocessor (offline): Connect/Listen are single-node no-ops; program computes -> 42.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h69000000;  // NetConnect (no-op)              (op 105=0x69)
    dut.imem[1] = 32'h6A000000;  // NetListen  (no-op)              (op 106=0x6A)
    dut.imem[2] = 32'h0500002A;  // r0 = 42
    dut.imem[3] = 32'h59000000;  // Show r0
    dut.imem[4] = 32'h2B000000;  // Halt
    #3 rst = 0; wait (halted); #10;
    if (got && shown == 32'd42) $display("PASS: soft-core offline net (Connect/Listen no-op, compute = %0d)", shown);
    else $display("FAIL: got=%0b shown=%0d (expected 42)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
