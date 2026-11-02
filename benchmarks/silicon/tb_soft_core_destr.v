`timescale 1ns/1ps
// DestructureTuple: `for (a,b) in [(1,2),(3,4)]: Show a` -> 1, 3.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown [0:3]; integer nsh = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown[nsh] = out; nsh = nsh + 1; end
  initial begin
    #1;
    dut.imem[0]  = 32'h05000001;  // r0=1
    dut.imem[1]  = 32'h05010002;  // r1=2
    dut.imem[2]  = 32'h43020002;  // NewTuple t1(r2)=(1,2)   (op 67=0x43)
    dut.imem[3]  = 32'h05030003;  // r3=3
    dut.imem[4]  = 32'h05040004;  // r4=4
    dut.imem[5]  = 32'h43050302;  // NewTuple t2(r5)=(3,4)
    dut.imem[6]  = 32'h06070200;  // Move r7=r2               (op 6)
    dut.imem[7]  = 32'h06080500;  // Move r8=r5
    dut.imem[8]  = 32'h30090702;  // NewList xs(r9)=[r7,r8]   (op 48=0x30)
    dut.imem[9]  = 32'h55000900;  // IterPrepare xs           (op 85=0x55)
    dut.imem[10] = 32'h560A0E00;  // IterNext r10, exit=14    (op 86=0x56)
    dut.imem[11] = 32'h470B0A02;  // DestructureTuple start=11,src=r10,count=2 (op 71=0x47)
    dut.imem[12] = 32'h59000B00;  // Show r11 (=a)
    dut.imem[13] = 32'h220A0000;  // Jump -> 10               (op 34=0x22)
    dut.imem[14] = 32'h57000000;  // IterPop                  (op 87=0x57)
    dut.imem[15] = 32'h2B000000;  // Halt
    #3 rst = 0; wait (halted); #10;
    if (nsh == 2 && shown[0] == 32'd1 && shown[1] == 32'd3) $display("PASS: soft-core DestructureTuple (a of each = %0d,%0d)", shown[0], shown[1]);
    else $display("FAIL: nsh=%0d shown=%0d,%0d (expected 1,3)", nsh, shown[0], shown[1]);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
