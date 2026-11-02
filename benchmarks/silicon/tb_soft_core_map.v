`timescale 1ns/1ps
// Map: Set "a"->3, update "a"->9, Set "b"->4; item "a" = 9, length = 2.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown [0:3]; integer nsh = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown[nsh] = out; nsh = nsh + 1; end
  initial begin
    #1;
    dut.imem[0]  = 32'h05000061;  // r0='a'(97)
    dut.imem[1]  = 32'h30010001;  // NewList a_key(r1)=['a']         (op 48=0x30)
    dut.imem[2]  = 32'h05020062;  // r2='b'(98)
    dut.imem[3]  = 32'h30030201;  // NewList b_key(r3)=['b']
    dut.imem[4]  = 32'h34040000;  // NewEmptyMap m(r4)               (op 52=0x34)
    dut.imem[5]  = 32'h05050003;  // r5=3
    dut.imem[6]  = 32'h39040105;  // SetIndex m,key=r1,val=r5(3)     (op 57=0x39)
    dut.imem[7]  = 32'h05060009;  // r6=9
    dut.imem[8]  = 32'h39040106;  // SetIndex m,key=r1,val=r6(9) upd
    dut.imem[9]  = 32'h05070004;  // r7=4
    dut.imem[10] = 32'h39040307;  // SetIndex m,key=r3,val=r7(4)
    dut.imem[11] = 32'h3B080401;  // Index r8=item a(r1) of m        (op 59=0x3B)
    dut.imem[12] = 32'h59000800;  // Show r8 = 9
    dut.imem[13] = 32'h3E090400;  // Length r9 = length of m         (op 62=0x3E)
    dut.imem[14] = 32'h59000900;  // Show r9 = 2
    dut.imem[15] = 32'h2B000000;  // Halt
    #3 rst = 0; wait (halted); #10;
    if (nsh == 2 && shown[0] == 32'd9 && shown[1] == 32'd2) $display("PASS: soft-core Map (item a=%0d, len=%0d)", shown[0], shown[1]);
    else $display("FAIL: nsh=%0d shown=%0d,%0d (expected 9,2)", nsh, shown[0], shown[1]);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
