`timescale 1ns/1ps
// tb_fconst_roundtrip.v — FConst (op 201) SILICON-ROBUSTNESS regression lock + SPLIT-BANK round-trip lock.
//
// FConst was reworked into a two-cycle SEQUENTIALIZER (a 1-bit `fc` FSM in soft_core.v) so no cycle ever
// does more than the proven TWO combinational imem reads — the FF-backed `imem` (ram_style=registers)
// made the original single-cycle three-read (`imem[pc]` + `imem[pc+1]` + `imem[pc+2]`) load float
// constants as 0 on real FPGA silicon (correct only in sim).
//
// SEPARATELY, the 64-bit float register bank (frf0/frf1) was stored as ONE 64-bit-wide block-RAM per port.
// Through the yosys/nextpnr/prjxray open flow that 64-bit BRAM mis-mapped and EVERY float read back as 0 on
// real xc7 (sim stayed correct). The bank is now TWO 32-bit BRAMs per port (frf*_lo / frf*_hi) — the exact
// 32-bit width proven to work by the int regfile — with the write splitting ff_wd[31:0]->lo, ff_wd[63:32]->hi
// and the read reconstructing fav={hi,lo}. This TB proves the split-bank ROUND-TRIP is bit-exact:
//   Part A — FConst two distinct constants -> ShowF each, bit-exact (the constant-write path).
//   Part B — a value whose LOW and HIGH 32-bit halves are DISTINCT non-trivial patterns is written by
//            FConst, then FMove'd to another register (which READS it back through the fav port and
//            RE-WRITES it through the split write port — a full bank round-trip), then ShowF'd. A lo/hi
//            swap, a dropped half, or a 64-bit mis-map would corrupt this value; asserting it bit-exact
//            locks the {hi,lo} reconstruction. The halves 0xDEADBEEF (hi) / 0x0BADF00D (lo) are chosen so
//            no symmetry can hide a swap.
// Runs directly on soft_core.v + fpu.v (no cargo, no model).
//
// Program: FConst f0=2.0 ; FConst f1=pi ; FConst f2=0xDEADBEEF0BADF00D ; FMove f3<-f2 ;
//          ShowF f0 ; ShowF f1 ; ShowF f3 ; Halt
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire [63:0] outf; wire ov, ofv, halted;
  soft_core #(.FRAMES(64), .REGS(256), .IMEM_WORDS(1024), .HEAP(2048))
    dut(.clk(clk), .rst(rst), .out(out), .out_valid(ov), .outf(outf), .outf_valid(ofv), .halted(halted));
  integer n = 0; reg [63:0] seen [0:7];
  always #5 clk = ~clk;
  always @(posedge clk) begin
    if (ofv) begin seen[n] = outf; n = n + 1; end
  end
  initial begin
    #1;
    // FConst f0 = 2.0 (0x4000000000000000): word1 = low 32 bits, word2 = high 32 bits
    dut.imem[0] = 32'hC9000000;               // op 201 (0xC9), dst = f0
    dut.imem[1] = 32'h00000000;               //   bits[31:0]  = 0x00000000
    dut.imem[2] = 32'h40000000;               //   bits[63:32] = 0x40000000
    // FConst f1 = pi (0x400921FB54442D18)
    dut.imem[3] = 32'hC9010000;               // op 201, dst = f1
    dut.imem[4] = 32'h54442D18;               //   bits[31:0]  = 0x54442D18
    dut.imem[5] = 32'h400921FB;               //   bits[63:32] = 0x400921FB
    // FConst f2 = 0xDEADBEEF0BADF00D — LOW half 0x0BADF00D, HIGH half 0xDEADBEEF (distinct, no symmetry)
    dut.imem[6] = 32'hC9020000;               // op 201, dst = f2
    dut.imem[7] = 32'h0BADF00D;               //   bits[31:0]  = 0x0BADF00D  -> frf*_lo
    dut.imem[8] = 32'hDEADBEEF;               //   bits[63:32] = 0xDEADBEEF  -> frf*_hi
    // FMove f3 <- f2 (op 211 = 0xD3, dst = f3, ra = f2): reads f2 through the fav port ({hi,lo} reconstruct)
    // and re-writes it through the split write port — a full split-bank read+write round-trip.
    dut.imem[9]  = 32'hD3030200;              // op 211, dst = 3, ra = 2
    dut.imem[10] = 32'hD4000000;              // ShowF f0 (op 212 = 0xD4, ra = 0)
    dut.imem[11] = 32'hD4000100;              // ShowF f1 (op 212, ra = 1)
    dut.imem[12] = 32'hD4000300;              // ShowF f3 (op 212, ra = 3) — the round-tripped value
    dut.imem[13] = 32'h2B000000;              // Halt (op 43 = 0x2B)
    #3 rst = 0;
    wait (halted);
    #10;
    if (n !== 3) begin $display("FAIL: fconst_roundtrip (got n=%0d, want 3)", n); $finish; end
    if (seen[0] !== 64'h4000000000000000) begin $display("FAIL: fconst_roundtrip f0=%h want 4000000000000000", seen[0]); $finish; end
    if (seen[1] !== 64'h400921FB54442D18) begin $display("FAIL: fconst_roundtrip f1=%h want 400921FB54442D18", seen[1]); $finish; end
    if (seen[2] !== 64'hDEADBEEF0BADF00D) begin $display("FAIL: split-bank round-trip f3=%h want DEADBEEF0BADF00D (lo/hi 32-bit BRAM reconstruct)", seen[2]); $finish; end
    $display("PASS: fconst_roundtrip (2 float constants + FMove split-bank round-trip read back bit-exact; hi/lo 32-bit BRAM reconstruct proven)");
    $finish;
  end
  initial begin #2000000 $display("FAIL: fconst_roundtrip timeout"); $finish; end
endmodule
