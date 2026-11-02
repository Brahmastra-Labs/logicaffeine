`timescale 1ns/1ps
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h32000000;  // NewEmptyListI32 xs(r0)          (op 50=0x32)
    dut.imem[1] = 32'h05010005;  // r1 = 5
    dut.imem[2] = 32'h36000100;  // ListPush xs, r1                 (op 54=0x36)
    dut.imem[3] = 32'h3E020000;  // Length r2                       (op 62=0x3E)
    dut.imem[4] = 32'h59000200;  // Show r2 = 1
    dut.imem[5] = 32'h2B000000;  // Halt
    #3 rst = 0; wait (halted); #10;
    if (got && shown == 32'd1) $display("PASS: soft-core NewEmptyListI32 (len after push = %0d)", shown);
    else $display("FAIL: got=%0b shown=%0d (expected 1)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
