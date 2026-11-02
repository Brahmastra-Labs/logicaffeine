`timescale 1ns/1ps
// Offline networking — NetConnect(105=0x69)/NetListen(106=0x6A)/NetSend(107=0x6B)/NetSync(111=0x6F) are
// single-node no-ops (Sync leaves its variable unchanged). Interleaved with Shows: 1, then state still 5.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] seen [0:3]; integer n = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin seen[n] = out; n = n + 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h05000005;  // state(r0) = 5
    dut.imem[1] = 32'h69000000;  // NetConnect -> no-op
    dut.imem[2] = 32'h6A000000;  // NetListen  -> no-op
    dut.imem[3] = 32'h05010001;  // r1 = 1
    dut.imem[4] = 32'h59000100;  // Show 1
    dut.imem[5] = 32'h05020009;  // r2 = 9
    dut.imem[6] = 32'h6B000200;  // NetSend    -> no-op
    dut.imem[7] = 32'h6F000000;  // NetSync state(r0) -> no-op
    dut.imem[8] = 32'h59000000;  // Show state -> 5
    dut.imem[9] = 32'h2B000000;  // Halt
    #3 rst = 0; wait (halted); #10;
    if (n == 2 && seen[0] == 32'd1 && seen[1] == 32'd5)
      $display("PASS: soft-core offline Net (Show=%0d, state=%0d)", seen[0], seen[1]);
    else
      $display("FAIL: n=%0d seen=%0d,%0d (expected 1,5)", n, seen[0], seen[1]);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
