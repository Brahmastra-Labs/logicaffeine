`timescale 1ns/1ps
// NetMakePeer(110=0x6E) + NetStream(108=0x6C) — a PeerAgent handle is an offline stub and streaming a
// batch to it is a no-op; the trailing Show survives. Make peer, stream [1,2,3], Show 6.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h6E000000;  // NetMakePeer p(r0) = stub
    dut.imem[1] = 32'h05010001;  // r1 = 1
    dut.imem[2] = 32'h05020002;  // r2 = 2
    dut.imem[3] = 32'h05030003;  // r3 = 3
    dut.imem[4] = 32'h30040103;  // NewList batch(r4) = [1,2,3]  (op 48=0x30, start=r1, count=3)
    dut.imem[5] = 32'h6C000400;  // NetStream batch(r4) to p -> no-op
    dut.imem[6] = 32'h05050006;  // r5 = 6
    dut.imem[7] = 32'h59000500;  // Show 6
    dut.imem[8] = 32'h2B000000;  // Halt
    #3 rst = 0; wait (halted); #10;
    if (got && shown == 32'd6) $display("PASS: soft-core NetMakePeer+NetStream (Show=%0d)", shown);
    else $display("FAIL: got=%0b shown=%0d (expected 6)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
