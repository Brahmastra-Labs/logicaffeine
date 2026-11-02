`timescale 1ns/1ps
// String Concat (char lists): "ab" + "cde" = "abcde", length 5.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0]  = 32'h05000061;  // r0='a'(97)
    dut.imem[1]  = 32'h05010062;  // r1='b'(98)
    dut.imem[2]  = 32'h30020002;  // NewList a(r2)=[a,b]
    dut.imem[3]  = 32'h05030063;  // r3='c'(99)
    dut.imem[4]  = 32'h05040064;  // r4='d'(100)
    dut.imem[5]  = 32'h05050065;  // r5='e'(101)
    dut.imem[6]  = 32'h30060303;  // NewList b(r6)=[c,d,e]
    dut.imem[7]  = 32'h20070206;  // Concat c(r7)=a+b            (op 32=0x20)
    dut.imem[8]  = 32'h3E080700;  // Length r8
    dut.imem[9]  = 32'h59000800;  // Show r8 = 5
    dut.imem[10] = 32'h2B000000;  // Halt
    #3 rst = 0; wait (halted); #10;
    if (got && shown == 32'd5) $display("PASS: soft-core string Concat (len abcde = %0d)", shown);
    else $display("FAIL: got=%0b shown=%0d (expected 5)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
