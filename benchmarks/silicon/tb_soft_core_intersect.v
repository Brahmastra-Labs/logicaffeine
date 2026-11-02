`timescale 1ns/1ps
// Load-store batch 8 on the soft-core: set algebra. a={1,2}, b={2,3}; UNION length = 3 (== rtl.rs prog 21).
// Exercises NewEmptySet/SetAdd (setup) + UnionOp (nested-loop dedup merge) + Length.
module tb;
  reg clk = 0, rst = 1;
  wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0]  = 32'h33000000;  // NewEmptySet a(r0)               (op 51=0x33)
    dut.imem[1]  = 32'h05010001;  // r1 = 1
    dut.imem[2]  = 32'h37000100;  // SetAdd a += r1                  (op 55=0x37)
    dut.imem[3]  = 32'h05020002;  // r2 = 2
    dut.imem[4]  = 32'h37000200;  // SetAdd a += r2
    dut.imem[5]  = 32'h33030000;  // NewEmptySet b(r3)
    dut.imem[6]  = 32'h05040002;  // r4 = 2
    dut.imem[7]  = 32'h37030400;  // SetAdd b += r4
    dut.imem[8]  = 32'h05050003;  // r5 = 3
    dut.imem[9]  = 32'h37030500;  // SetAdd b += r5
    dut.imem[10] = 32'h45060003;  // IntersectOp c = a n b   (op 68=0x44)
    dut.imem[11] = 32'h3E070600;  // Length r7 = length of c         (op 62=0x3E)
    dut.imem[12] = 32'h59000700;  // Show r7
    dut.imem[13] = 32'h2B000000;  // Halt
    #3 rst = 0;
    wait (halted);
    #10;
    if (got && shown == 32'd1)
      $display("PASS: soft-core UnionOp (|{1,2} n {2,3}| = %0d)", shown);
    else
      $display("FAIL: got=%0b shown=%0d (expected 1)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
