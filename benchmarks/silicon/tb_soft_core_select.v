`timescale 1ns/1ps
// Select: ch holds 7; `Await the first of: Receive x from ch` fires the recv arm -> Show 7.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h5C000000;  // ChanNew ch(r0)                  (op 92=0x5C)
    dut.imem[1] = 32'h05010007;  // r1 = 7
    dut.imem[2] = 32'h5D000100;  // ChanSend ch, r1                 (op 93=0x5D)
    dut.imem[3] = 32'h66000200;  // SelectArmRecv chan=r0, var=r2   (op 102=0x66)
    dut.imem[4] = 32'h68030000;  // SelectWait dst_arm=r3           (op 104=0x68)
    dut.imem[5] = 32'h59000200;  // Show r2 (=x=7)
    dut.imem[6] = 32'h2B000000;  // Halt
    #3 rst = 0; wait (halted); #10;
    if (got && shown == 32'd7) $display("PASS: soft-core Select (first-ready recv = %0d)", shown);
    else $display("FAIL: got=%0b shown=%0d (expected 7)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
