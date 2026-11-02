`timescale 1ns/1ps
// Closure: doubler = (n) -> n*2; doubler(5) = 10.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h28000600;  // MakeClosure doubler(r0), entry=6 (op 40=0x28)
    dut.imem[1] = 32'h05010005;  // n = 5
    dut.imem[2] = 32'h27020000;  // CallValue result=r2, callee=r0    (op 39=0x27)
    dut.imem[3] = 32'h59000200;  // Show r2
    dut.imem[4] = 32'h2B000000;  // Halt
    dut.imem[5] = 32'h2B000000;  // pad
    dut.imem[6] = 32'h05040002;  // [doubler] r4=2
    dut.imem[7] = 32'h0B050104;  // Mul r5 = r1 * r4 (op 11=0x0B)
    dut.imem[8] = 32'h29000500;  // Return r5 (op 41=0x29)
    #3 rst = 0; wait (halted); #10;
    if (got && shown == 32'd10) $display("PASS: soft-core Closure (doubler(5) = %0d)", shown);
    else $display("FAIL: got=%0b shown=%0d (expected 10)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
