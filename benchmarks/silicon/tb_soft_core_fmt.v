`timescale 1ns/1ps
// FormatValue: "{42}" -> "42", length 2.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h0500002A;  // x = 42
    dut.imem[1] = 32'h40010000;  // FormatValue s(r1) = "{x}"       (op 64=0x40)
    dut.imem[2] = 32'h3E020100;  // Length r2 = length of s         (op 62=0x3E)
    dut.imem[3] = 32'h59000200;  // Show r2 = 2
    dut.imem[4] = 32'h2B000000;  // Halt
    #3 rst = 0; wait (halted); #10;
    if (got && shown == 32'd2) $display("PASS: soft-core FormatValue (len of \"42\" = %0d)", shown);
    else $display("FAIL: got=%0b shown=%0d (expected 2)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
