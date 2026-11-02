`timescale 1ns/1ps
// Load-store batch 5 on the soft-core: the for-each engine. `Repeat for x in xs` sums [10,20,30] = 60.
// Exercises IterPrepare (push cursor), IterNext (dmem[ptr+cursor] read + advance + exit branch), IterPop.
// Hand-assembled (== rtl.rs program (17)):
//   0..2 LoadConst 10/20/30   3 NewList xs(r3)   4 s=0(r4)   5 IterPrepare xs
//   6 IterNext r5 exit->9   7 Add s=s+x   8 Jump->6   9 IterPop   10 Show s   11 Halt
module tb;
  reg clk = 0, rst = 1;
  wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0]  = 32'h0500000A;  // r0 = 10
    dut.imem[1]  = 32'h05010014;  // r1 = 20
    dut.imem[2]  = 32'h0502001E;  // r2 = 30
    dut.imem[3]  = 32'h30030003;  // NewList xs(r3) = [r0..], count 3
    dut.imem[4]  = 32'h05040000;  // s = 0 (r4)
    dut.imem[5]  = 32'h55000300;  // IterPrepare iterable=r3            (op 85=0x55)
    dut.imem[6]  = 32'h56050900;  // IterNext r5, exit=9                (op 86=0x56)
    dut.imem[7]  = 32'h08040405;  // Add r4 = r4 + r5                   (op 8)
    dut.imem[8]  = 32'h22060000;  // Jump -> 6                          (op 34=0x22)
    dut.imem[9]  = 32'h57000000;  // IterPop                            (op 87=0x57)
    dut.imem[10] = 32'h59000400;  // Show r4
    dut.imem[11] = 32'h2B000000;  // Halt
    #3 rst = 0;
    wait (halted);
    #10;
    if (got && shown == 32'd60)
      $display("PASS: soft-core for-each (sum [10,20,30] = %0d)", shown);
    else
      $display("FAIL: got=%0b shown=%0d (expected 60)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
