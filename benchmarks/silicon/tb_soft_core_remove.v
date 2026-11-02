`timescale 1ns/1ps
// Load-store batch 8: RemoveFrom. {10,20,30} minus 20 = {10,30}, length 2 (== rtl.rs program 23).
module tb;
  reg clk = 0, rst = 1;
  wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h33000000;  // NewEmptySet s(r0)
    dut.imem[1] = 32'h0501000A;  // r1 = 10
    dut.imem[2] = 32'h37000100;  // SetAdd s += 10
    dut.imem[3] = 32'h05020014;  // r2 = 20
    dut.imem[4] = 32'h37000200;  // SetAdd s += 20
    dut.imem[5] = 32'h0503001E;  // r3 = 30
    dut.imem[6] = 32'h37000300;  // SetAdd s += 30
    dut.imem[7] = 32'h38000200;  // RemoveFrom s, value=r2 (20)      (op 56=0x38)
    dut.imem[8] = 32'h3E040000;  // Length r4 = length of s
    dut.imem[9] = 32'h59000400;  // Show r4
    dut.imem[10] = 32'h2B000000; // Halt
    #3 rst = 0;
    wait (halted);
    #10;
    if (got && shown == 32'd2)
      $display("PASS: soft-core RemoveFrom (|{10,20,30}\\{20}| = %0d)", shown);
    else
      $display("FAIL: got=%0b shown=%0d (expected 2)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
