`timescale 1ns/1ps
// AUTO-GENERATED from the run_soft_core_typed ≡VM text differential (codegen_sva/rtl.rs). Do not edit.
// Regenerate: LOGOS_EMIT_TEXT_TB=1 cargo test -p logicaffeine-compile --lib emit_text_tbs
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire [63:0] outf; wire [31:0] outs;
  wire ov, ofv, osv, osf, halted;
  soft_core #(.FRAMES(64), .REGS(256), .IMEM_WORDS(1024), .HEAP(4096))
    dut(.clk(clk), .rst(rst), .out(out), .out_valid(ov), .outf(outf), .outf_valid(ofv),
        .outs(outs), .outs_valid(osv), .outs_first(osf), .halted(halted));
  integer n = 0; reg [1:0] skind [0:255]; reg [31:0] sval [0:255];
  always #5 clk = ~clk;
  always @(posedge clk) begin
    if (ov)  begin skind[n] = 2'd0; sval[n] = out;  n = n + 1; end
    if (osv) begin skind[n] = osf ? 2'd2 : 2'd1; sval[n] = outs; n = n + 1; end
  end
  integer e; reg fail;
  initial begin
    #1;
    dut.imem[0] = 32'h05000005;
    dut.imem[1] = 32'h06010000;
    dut.imem[2] = 32'h0502003F;
    dut.imem[3] = 32'h06030200;
    dut.imem[4] = 32'hD8070016;
    dut.imem[5] = 32'h0000000F;
    dut.imem[6] = 32'h40090100;
    dut.imem[7] = 32'h20060709;
    dut.imem[8] = 32'hD8080008;
    dut.imem[9] = 32'h00000025;
    dut.imem[10] = 32'h20050608;
    dut.imem[11] = 32'h40090300;
    dut.imem[12] = 32'h20040509;
    dut.imem[13] = 32'hD9000400;
    dut.imem[14] = 32'h2B000000;
    dut.imem[15] = 32'h00000073;
    dut.imem[16] = 32'h00000074;
    dut.imem[17] = 32'h00000072;
    dut.imem[18] = 32'h00000065;
    dut.imem[19] = 32'h00000074;
    dut.imem[20] = 32'h00000063;
    dut.imem[21] = 32'h00000068;
    dut.imem[22] = 32'h00000020;
    dut.imem[23] = 32'h00000074;
    dut.imem[24] = 32'h00000072;
    dut.imem[25] = 32'h00000065;
    dut.imem[26] = 32'h00000065;
    dut.imem[27] = 32'h00000020;
    dut.imem[28] = 32'h0000006F;
    dut.imem[29] = 32'h00000066;
    dut.imem[30] = 32'h00000020;
    dut.imem[31] = 32'h00000064;
    dut.imem[32] = 32'h00000065;
    dut.imem[33] = 32'h00000070;
    dut.imem[34] = 32'h00000074;
    dut.imem[35] = 32'h00000068;
    dut.imem[36] = 32'h00000020;
    dut.imem[37] = 32'h00000020;
    dut.imem[38] = 32'h00000063;
    dut.imem[39] = 32'h00000068;
    dut.imem[40] = 32'h00000065;
    dut.imem[41] = 32'h00000063;
    dut.imem[42] = 32'h0000006B;
    dut.imem[43] = 32'h0000003A;
    dut.imem[44] = 32'h00000020;
    #3 rst = 0;
    wait (halted);
    #10;
    fail = 0;
    if (n != 34) begin fail = 1; $display("FAIL: text_binary_trees_line — got %0d capture entries, want 34", n); end
    if (!fail && (skind[0] !== 2'd2 || sval[0] !== 32'h00000021)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 0 was kind=%0d val=%0d, want kind=2 val=%0d", skind[0], sval[0], 32'h00000021); end
    if (!fail && (skind[1] !== 2'd1 || sval[1] !== 32'h00000073)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 1 was kind=%0d val=%0d, want kind=1 val=%0d", skind[1], sval[1], 32'h00000073); end
    if (!fail && (skind[2] !== 2'd1 || sval[2] !== 32'h00000074)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 2 was kind=%0d val=%0d, want kind=1 val=%0d", skind[2], sval[2], 32'h00000074); end
    if (!fail && (skind[3] !== 2'd1 || sval[3] !== 32'h00000072)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 3 was kind=%0d val=%0d, want kind=1 val=%0d", skind[3], sval[3], 32'h00000072); end
    if (!fail && (skind[4] !== 2'd1 || sval[4] !== 32'h00000065)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 4 was kind=%0d val=%0d, want kind=1 val=%0d", skind[4], sval[4], 32'h00000065); end
    if (!fail && (skind[5] !== 2'd1 || sval[5] !== 32'h00000074)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 5 was kind=%0d val=%0d, want kind=1 val=%0d", skind[5], sval[5], 32'h00000074); end
    if (!fail && (skind[6] !== 2'd1 || sval[6] !== 32'h00000063)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 6 was kind=%0d val=%0d, want kind=1 val=%0d", skind[6], sval[6], 32'h00000063); end
    if (!fail && (skind[7] !== 2'd1 || sval[7] !== 32'h00000068)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 7 was kind=%0d val=%0d, want kind=1 val=%0d", skind[7], sval[7], 32'h00000068); end
    if (!fail && (skind[8] !== 2'd1 || sval[8] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 8 was kind=%0d val=%0d, want kind=1 val=%0d", skind[8], sval[8], 32'h00000020); end
    if (!fail && (skind[9] !== 2'd1 || sval[9] !== 32'h00000074)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 9 was kind=%0d val=%0d, want kind=1 val=%0d", skind[9], sval[9], 32'h00000074); end
    if (!fail && (skind[10] !== 2'd1 || sval[10] !== 32'h00000072)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 10 was kind=%0d val=%0d, want kind=1 val=%0d", skind[10], sval[10], 32'h00000072); end
    if (!fail && (skind[11] !== 2'd1 || sval[11] !== 32'h00000065)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 11 was kind=%0d val=%0d, want kind=1 val=%0d", skind[11], sval[11], 32'h00000065); end
    if (!fail && (skind[12] !== 2'd1 || sval[12] !== 32'h00000065)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 12 was kind=%0d val=%0d, want kind=1 val=%0d", skind[12], sval[12], 32'h00000065); end
    if (!fail && (skind[13] !== 2'd1 || sval[13] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 13 was kind=%0d val=%0d, want kind=1 val=%0d", skind[13], sval[13], 32'h00000020); end
    if (!fail && (skind[14] !== 2'd1 || sval[14] !== 32'h0000006F)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 14 was kind=%0d val=%0d, want kind=1 val=%0d", skind[14], sval[14], 32'h0000006F); end
    if (!fail && (skind[15] !== 2'd1 || sval[15] !== 32'h00000066)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 15 was kind=%0d val=%0d, want kind=1 val=%0d", skind[15], sval[15], 32'h00000066); end
    if (!fail && (skind[16] !== 2'd1 || sval[16] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 16 was kind=%0d val=%0d, want kind=1 val=%0d", skind[16], sval[16], 32'h00000020); end
    if (!fail && (skind[17] !== 2'd1 || sval[17] !== 32'h00000064)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 17 was kind=%0d val=%0d, want kind=1 val=%0d", skind[17], sval[17], 32'h00000064); end
    if (!fail && (skind[18] !== 2'd1 || sval[18] !== 32'h00000065)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 18 was kind=%0d val=%0d, want kind=1 val=%0d", skind[18], sval[18], 32'h00000065); end
    if (!fail && (skind[19] !== 2'd1 || sval[19] !== 32'h00000070)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 19 was kind=%0d val=%0d, want kind=1 val=%0d", skind[19], sval[19], 32'h00000070); end
    if (!fail && (skind[20] !== 2'd1 || sval[20] !== 32'h00000074)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 20 was kind=%0d val=%0d, want kind=1 val=%0d", skind[20], sval[20], 32'h00000074); end
    if (!fail && (skind[21] !== 2'd1 || sval[21] !== 32'h00000068)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 21 was kind=%0d val=%0d, want kind=1 val=%0d", skind[21], sval[21], 32'h00000068); end
    if (!fail && (skind[22] !== 2'd1 || sval[22] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 22 was kind=%0d val=%0d, want kind=1 val=%0d", skind[22], sval[22], 32'h00000020); end
    if (!fail && (skind[23] !== 2'd1 || sval[23] !== 32'h00000035)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 23 was kind=%0d val=%0d, want kind=1 val=%0d", skind[23], sval[23], 32'h00000035); end
    if (!fail && (skind[24] !== 2'd1 || sval[24] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 24 was kind=%0d val=%0d, want kind=1 val=%0d", skind[24], sval[24], 32'h00000020); end
    if (!fail && (skind[25] !== 2'd1 || sval[25] !== 32'h00000063)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 25 was kind=%0d val=%0d, want kind=1 val=%0d", skind[25], sval[25], 32'h00000063); end
    if (!fail && (skind[26] !== 2'd1 || sval[26] !== 32'h00000068)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 26 was kind=%0d val=%0d, want kind=1 val=%0d", skind[26], sval[26], 32'h00000068); end
    if (!fail && (skind[27] !== 2'd1 || sval[27] !== 32'h00000065)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 27 was kind=%0d val=%0d, want kind=1 val=%0d", skind[27], sval[27], 32'h00000065); end
    if (!fail && (skind[28] !== 2'd1 || sval[28] !== 32'h00000063)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 28 was kind=%0d val=%0d, want kind=1 val=%0d", skind[28], sval[28], 32'h00000063); end
    if (!fail && (skind[29] !== 2'd1 || sval[29] !== 32'h0000006B)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 29 was kind=%0d val=%0d, want kind=1 val=%0d", skind[29], sval[29], 32'h0000006B); end
    if (!fail && (skind[30] !== 2'd1 || sval[30] !== 32'h0000003A)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 30 was kind=%0d val=%0d, want kind=1 val=%0d", skind[30], sval[30], 32'h0000003A); end
    if (!fail && (skind[31] !== 2'd1 || sval[31] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 31 was kind=%0d val=%0d, want kind=1 val=%0d", skind[31], sval[31], 32'h00000020); end
    if (!fail && (skind[32] !== 2'd1 || sval[32] !== 32'h00000036)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 32 was kind=%0d val=%0d, want kind=1 val=%0d", skind[32], sval[32], 32'h00000036); end
    if (!fail && (skind[33] !== 2'd1 || sval[33] !== 32'h00000033)) begin fail = 1; $display("FAIL: text_binary_trees_line — entry 33 was kind=%0d val=%0d, want kind=1 val=%0d", skind[33], sval[33], 32'h00000033); end
    if (!fail) $display("PASS: text_binary_trees_line");
    $finish;
  end
  initial begin #2000000000 $display("FAIL: text_binary_trees_line timeout"); $finish; end
endmodule
