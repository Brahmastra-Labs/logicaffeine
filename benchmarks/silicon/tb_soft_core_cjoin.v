// tb_soft_core_cjoin — the full soft-core issues op 113 (CrdtJoin) to the crdt_merge coprocessor.
// Program: build two 2-lane G-Counters a=[3,1], b=[2,7]; CrdtJoin b into a → per-replica MAX [3,7];
// Show lane 1 = max(1,7) = 7. A field-wise ADD (op 81) would give 1+7 = 8, so this can't pass vacuously.
`timescale 1ns/1ps
module tb;
  reg clk = 0, rst = 1;
  wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  always #5 clk = ~clk;
  integer got = -1;
  always @(posedge clk) if (out_valid) got = out;
  initial begin
    dut.imem[0]=32'h05000003; dut.imem[1]=32'h4A010002; dut.imem[2]=32'h4B010000; // r0=3; a=new 2-lane; a.0=r0
    dut.imem[3]=32'h05020001; dut.imem[4]=32'h4B010201;                            // r2=1; a.1=r2
    dut.imem[5]=32'h05030002; dut.imem[6]=32'h4A040002; dut.imem[7]=32'h4B040300; // r3=2; b=new 2-lane; b.0=r3
    dut.imem[8]=32'h05050007; dut.imem[9]=32'h4B040501;                            // r5=7; b.1=r5
    dut.imem[10]=32'h71010400;                                                     // CrdtJoin b into a → [3,7]
    dut.imem[11]=32'h4C060101;                                                     // GetField r6 = a.lane1
    dut.imem[12]=32'h59000600; dut.imem[13]=32'h2B000000;                          // Show 7; Halt
    #3 rst = 0;
    wait (halted);
    #10;
    if (got == 7) $display("PASS: soft-core CrdtJoin (G-Counter lane1 = max(1,7) = 7, not ADD's 8)");
    else $display("FAIL: soft-core CrdtJoin got %0d expected 7", got);
    $finish;
  end
  initial begin #200000; $display("FAIL: soft-core CrdtJoin timeout"); $finish; end
endmodule
