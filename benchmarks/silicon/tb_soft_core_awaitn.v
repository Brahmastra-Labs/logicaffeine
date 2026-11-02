`timescale 1ns/1ps
// NetAwait (op 109=0x6D) — the offline loopback receive. NetMakePeer mints a peer key; NetSend(107=0x6B)
// PUBLISHES (peer,7) to the inbox; NetAwait DRAINS the entry keyed by that peer, delivering 7.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h6A000000;  // NetListen (no-op)                (op 106=0x6A)
    dut.imem[1] = 32'h6E000100;  // me_peer(r0) = NetMakePeer(r1)    (op 110=0x6E)
    dut.imem[2] = 32'h05020007;  // r2 = 7
    dut.imem[3] = 32'h6B000002;  // NetSend to=r0, msg=r2 -> publish (op 107=0x6B)
    dut.imem[4] = 32'h6D030000;  // NetAwait got(r3) from r0 -> drain (op 109=0x6D)
    dut.imem[5] = 32'h59000300;  // Show r3
    dut.imem[6] = 32'h2B000000;  // Halt
    #3 rst = 0; wait (halted); #10;
    if (got && shown == 32'd7) $display("PASS: soft-core NetAwait (loopback delivered = %0d)", shown);
    else $display("FAIL: got=%0b shown=%0d (expected 7)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
