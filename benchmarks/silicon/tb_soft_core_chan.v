`timescale 1ns/1ps
// Wave 4 concurrency coprocessor: FIFO channel. Send 7 then 8; receive comes back 7 then 8 (FIFO).
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown [0:3]; integer nsh = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown[nsh] = out; nsh = nsh + 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h5C000000;  // ChanNew ch(r0)                 (op 92=0x5C)
    dut.imem[1] = 32'h05010007;  // r1 = 7
    dut.imem[2] = 32'h5D000100;  // ChanSend ch, r1                (op 93=0x5D)
    dut.imem[3] = 32'h05020008;  // r2 = 8
    dut.imem[4] = 32'h5D000200;  // ChanSend ch, r2
    dut.imem[5] = 32'h5E030000;  // ChanRecv r3, ch                (op 94=0x5E)
    dut.imem[6] = 32'h59000300;  // Show r3 (=7)
    dut.imem[7] = 32'h5E040000;  // ChanRecv r4, ch
    dut.imem[8] = 32'h59000400;  // Show r4 (=8)
    dut.imem[9] = 32'h2B000000;  // Halt
    #3 rst = 0; wait (halted); #10;
    if (nsh == 2 && shown[0] == 32'd7 && shown[1] == 32'd8)
      $display("PASS: soft-core FIFO channel (recv 7,8 in order: %0d,%0d)", shown[0], shown[1]);
    else $display("FAIL: nsh=%0d shown=%0d,%0d (expected 7,8)", nsh, shown[0], shown[1]);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
