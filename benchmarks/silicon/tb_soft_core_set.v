`timescale 1ns/1ps
// Load-store batch 7 on the soft-core: sets + ranges.
//   SET (program 19): a new Set of Int; Add 2/4/2 (dedup); length = 2.
//   RANGE (program 20): 1 through 5 -> [1,2,3,4,5]; for-each sum = 15.
// This TB runs the SET program (NewEmptySet + SetAdd dedup + Length). A twin RANGE TB follows.
module tb;
  reg clk = 0, rst = 1;
  wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h33000000;  // NewEmptySet evens(r0)          (op 51=0x33)
    dut.imem[1] = 32'h05010002;  // r1 = 2
    dut.imem[2] = 32'h37000100;  // SetAdd evens += r1             (op 55=0x37)
    dut.imem[3] = 32'h05020004;  // r2 = 4
    dut.imem[4] = 32'h37000200;  // SetAdd evens += r2
    dut.imem[5] = 32'h37000100;  // SetAdd evens += r1 (dup, no-op)
    dut.imem[6] = 32'h3E030000;  // Length r3 = length of evens    (op 62=0x3E)
    dut.imem[7] = 32'h59000300;  // Show r3
    dut.imem[8] = 32'h2B000000;  // Halt
    #3 rst = 0;
    wait (halted);
    #10;
    if (got && shown == 32'd2)
      $display("PASS: soft-core Set dedup (|{2,4,2}| = %0d)", shown);
    else
      $display("FAIL: got=%0b shown=%0d (expected 2)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
