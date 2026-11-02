`timescale 1ns/1ps
// ChanClose (op 97=0x61) — closing a channel sets a flag no receive path reads; output-transparent.
// Enqueue 42, dequeue it (42), close — the dequeued value is unaffected. Show 42.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h5C000000;  // ChanNew ch(r0)                    (op 92=0x5C)
    dut.imem[1] = 32'h0501002A;  // r1 = 42
    dut.imem[2] = 32'h5F020001;  // ChanTrySend ch, val=r1 -> r2      (op 95=0x5F)
    dut.imem[3] = 32'h60030000;  // ChanTryRecv r3 = ch -> 42         (op 96=0x60)
    dut.imem[4] = 32'h61000000;  // ChanClose ch -> no-op             (op 97=0x61)
    dut.imem[5] = 32'h59000300;  // Show r3
    dut.imem[6] = 32'h2B000000;  // Halt
    #3 rst = 0; wait (halted); #10;
    if (got && shown == 32'd42) $display("PASS: soft-core ChanClose (dequeued value unaffected = %0d)", shown);
    else $display("FAIL: got=%0b shown=%0d (expected 42)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
