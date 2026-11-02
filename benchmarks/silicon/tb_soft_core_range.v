`timescale 1ns/1ps
// Load-store batch 7 on the soft-core: `1 through 5` materializes [1,2,3,4,5]; for-each sum = 15.
// Exercises NewRange (inclusive materialization) feeding the iterator engine. (== rtl.rs program (20)):
//   0 r0=1  1 r1=5  2 NewRange r(r2)=1..=5  3 s=0(r3)  4 IterPrepare  5 IterNext r4 exit->8
//   6 Add s=s+x  7 Jump->5  8 IterPop  9 Show s  10 Halt
module tb;
  reg clk = 0, rst = 1;
  wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0]  = 32'h05000001;  // r0 = 1
    dut.imem[1]  = 32'h05010005;  // r1 = 5
    dut.imem[2]  = 32'h35020001;  // NewRange r(r2) = r0 through r1     (op 53=0x35)
    dut.imem[3]  = 32'h05030000;  // s = 0 (r3)
    dut.imem[4]  = 32'h55000200;  // IterPrepare iterable=r2            (op 85=0x55)
    dut.imem[5]  = 32'h56040800;  // IterNext r4, exit=8                (op 86=0x56)
    dut.imem[6]  = 32'h08030304;  // Add r3 = r3 + r4
    dut.imem[7]  = 32'h22050000;  // Jump -> 5                          (op 34=0x22)
    dut.imem[8]  = 32'h57000000;  // IterPop                            (op 87=0x57)
    dut.imem[9]  = 32'h59000300;  // Show r3
    dut.imem[10] = 32'h2B000000;  // Halt
    #3 rst = 0;
    wait (halted);
    #10;
    if (got && shown == 32'd15)
      $display("PASS: soft-core NewRange (sum 1..=5 = %0d)", shown);
    else
      $display("FAIL: got=%0b shown=%0d (expected 15)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
