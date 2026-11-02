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
    dut.imem[0] = 32'h05000006;
    dut.imem[1] = 32'h06010000;
    dut.imem[2] = 32'h05020004;
    dut.imem[3] = 32'h06030200;
    dut.imem[4] = 32'h06040100;
    dut.imem[5] = 32'h06050400;
    dut.imem[6] = 32'h05080002;
    dut.imem[7] = 32'h08070308;
    dut.imem[8] = 32'h1A060705;
    dut.imem[9] = 32'h230D0600;
    dut.imem[10] = 32'h050A0002;
    dut.imem[11] = 32'h0809030A;
    dut.imem[12] = 32'h06050900;
    dut.imem[13] = 32'h05080001;
    dut.imem[14] = 32'h08070508;
    dut.imem[15] = 32'h25060701;
    dut.imem[16] = 32'h0000005D;
    dut.imem[17] = 32'h06090600;
    dut.imem[18] = 32'hD80D0016;
    dut.imem[19] = 32'h0000006F;
    dut.imem[20] = 32'h050F0001;
    dut.imem[21] = 32'h080E050F;
    dut.imem[22] = 32'h401D0E00;
    dut.imem[23] = 32'h200C0D1D;
    dut.imem[24] = 32'hD8100008;
    dut.imem[25] = 32'h00000085;
    dut.imem[26] = 32'h200B0C10;
    dut.imem[27] = 32'h401D0900;
    dut.imem[28] = 32'h200A0B1D;
    dut.imem[29] = 32'hD9000A00;
    dut.imem[30] = 32'h060A0300;
    dut.imem[31] = 32'h060B0A00;
    dut.imem[32] = 32'h1B0C0B05;
    dut.imem[33] = 32'h234F0C00;
    dut.imem[34] = 32'h050D0001;
    dut.imem[35] = 32'h060E0D00;
    dut.imem[36] = 32'h050F0000;
    dut.imem[37] = 32'h06100F00;
    dut.imem[38] = 32'h0A13050B;
    dut.imem[39] = 32'h08121303;
    dut.imem[40] = 32'h19111012;
    dut.imem[41] = 32'h23301100;
    dut.imem[42] = 32'h05150002;
    dut.imem[43] = 32'h0B140E15;
    dut.imem[44] = 32'h060E1400;
    dut.imem[45] = 32'h05140001;
    dut.imem[46] = 32'h08101014;
    dut.imem[47] = 32'h22260000;
    dut.imem[48] = 32'h05110000;
    dut.imem[49] = 32'h06121100;
    dut.imem[50] = 32'h05130000;
    dut.imem[51] = 32'h06141300;
    dut.imem[52] = 32'h1915140E;
    dut.imem[53] = 32'h233D1500;
    dut.imem[54] = 32'h06170B00;
    dut.imem[55] = 32'h25161701;
    dut.imem[56] = 32'h0000005D;
    dut.imem[57] = 32'h08121216;
    dut.imem[58] = 32'h05160001;
    dut.imem[59] = 32'h08141416;
    dut.imem[60] = 32'h22340000;
    dut.imem[61] = 32'hD81A0000;
    dut.imem[62] = 32'h0000008D;
    dut.imem[63] = 32'h401D0E00;
    dut.imem[64] = 32'h20191A1D;
    dut.imem[65] = 32'hD81B0010;
    dut.imem[66] = 32'h0000008D;
    dut.imem[67] = 32'h2018191B;
    dut.imem[68] = 32'h401D0B00;
    dut.imem[69] = 32'h2017181D;
    dut.imem[70] = 32'hD81C0008;
    dut.imem[71] = 32'h00000085;
    dut.imem[72] = 32'h2016171C;
    dut.imem[73] = 32'h401D1200;
    dut.imem[74] = 32'h2015161D;
    dut.imem[75] = 32'hD9001500;
    dut.imem[76] = 32'h05150002;
    dut.imem[77] = 32'h080B0B15;
    dut.imem[78] = 32'h22200000;
    dut.imem[79] = 32'hD80F0019;
    dut.imem[80] = 32'h0000009D;
    dut.imem[81] = 32'h401D0500;
    dut.imem[82] = 32'h200E0F1D;
    dut.imem[83] = 32'hD8100008;
    dut.imem[84] = 32'h00000085;
    dut.imem[85] = 32'h200D0E10;
    dut.imem[86] = 32'h06120500;
    dut.imem[87] = 32'h25111201;
    dut.imem[88] = 32'h0000005D;
    dut.imem[89] = 32'h401D1100;
    dut.imem[90] = 32'h200C0D1D;
    dut.imem[91] = 32'hD9000C00;
    dut.imem[92] = 32'h2B000000;
    dut.imem[93] = 32'h05020000;
    dut.imem[94] = 32'h1D010002;
    dut.imem[95] = 32'h23620100;
    dut.imem[96] = 32'h05030001;
    dut.imem[97] = 32'h29000300;
    dut.imem[98] = 32'h05030001;
    dut.imem[99] = 32'h05060001;
    dut.imem[100] = 32'h0A050006;
    dut.imem[101] = 32'h25040501;
    dut.imem[102] = 32'h0000005D;
    dut.imem[103] = 32'h08020304;
    dut.imem[104] = 32'h05090001;
    dut.imem[105] = 32'h0A080009;
    dut.imem[106] = 32'h25070801;
    dut.imem[107] = 32'h0000005D;
    dut.imem[108] = 32'h08010207;
    dut.imem[109] = 32'h29000100;
    dut.imem[110] = 32'h2A000000;
    dut.imem[111] = 32'h00000073;
    dut.imem[112] = 32'h00000074;
    dut.imem[113] = 32'h00000072;
    dut.imem[114] = 32'h00000065;
    dut.imem[115] = 32'h00000074;
    dut.imem[116] = 32'h00000063;
    dut.imem[117] = 32'h00000068;
    dut.imem[118] = 32'h00000020;
    dut.imem[119] = 32'h00000074;
    dut.imem[120] = 32'h00000072;
    dut.imem[121] = 32'h00000065;
    dut.imem[122] = 32'h00000065;
    dut.imem[123] = 32'h00000020;
    dut.imem[124] = 32'h0000006F;
    dut.imem[125] = 32'h00000066;
    dut.imem[126] = 32'h00000020;
    dut.imem[127] = 32'h00000064;
    dut.imem[128] = 32'h00000065;
    dut.imem[129] = 32'h00000070;
    dut.imem[130] = 32'h00000074;
    dut.imem[131] = 32'h00000068;
    dut.imem[132] = 32'h00000020;
    dut.imem[133] = 32'h00000020;
    dut.imem[134] = 32'h00000063;
    dut.imem[135] = 32'h00000068;
    dut.imem[136] = 32'h00000065;
    dut.imem[137] = 32'h00000063;
    dut.imem[138] = 32'h0000006B;
    dut.imem[139] = 32'h0000003A;
    dut.imem[140] = 32'h00000020;
    dut.imem[141] = 32'h00000020;
    dut.imem[142] = 32'h00000074;
    dut.imem[143] = 32'h00000072;
    dut.imem[144] = 32'h00000065;
    dut.imem[145] = 32'h00000065;
    dut.imem[146] = 32'h00000073;
    dut.imem[147] = 32'h00000020;
    dut.imem[148] = 32'h0000006F;
    dut.imem[149] = 32'h00000066;
    dut.imem[150] = 32'h00000020;
    dut.imem[151] = 32'h00000064;
    dut.imem[152] = 32'h00000065;
    dut.imem[153] = 32'h00000070;
    dut.imem[154] = 32'h00000074;
    dut.imem[155] = 32'h00000068;
    dut.imem[156] = 32'h00000020;
    dut.imem[157] = 32'h0000006C;
    dut.imem[158] = 32'h0000006F;
    dut.imem[159] = 32'h0000006E;
    dut.imem[160] = 32'h00000067;
    dut.imem[161] = 32'h00000020;
    dut.imem[162] = 32'h0000006C;
    dut.imem[163] = 32'h00000069;
    dut.imem[164] = 32'h00000076;
    dut.imem[165] = 32'h00000065;
    dut.imem[166] = 32'h00000064;
    dut.imem[167] = 32'h00000020;
    dut.imem[168] = 32'h00000074;
    dut.imem[169] = 32'h00000072;
    dut.imem[170] = 32'h00000065;
    dut.imem[171] = 32'h00000065;
    dut.imem[172] = 32'h00000020;
    dut.imem[173] = 32'h0000006F;
    dut.imem[174] = 32'h00000066;
    dut.imem[175] = 32'h00000020;
    dut.imem[176] = 32'h00000064;
    dut.imem[177] = 32'h00000065;
    dut.imem[178] = 32'h00000070;
    dut.imem[179] = 32'h00000074;
    dut.imem[180] = 32'h00000068;
    dut.imem[181] = 32'h00000020;
    #3 rst = 0;
    wait (halted);
    #10;
    fail = 0;
    if (n != 137) begin fail = 1; $display("FAIL: text_bench_binary_trees — got %0d capture entries, want 137", n); end
    if (!fail && (skind[0] !== 2'd2 || sval[0] !== 32'h00000022)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 0 was kind=%0d val=%0d, want kind=2 val=%0d", skind[0], sval[0], 32'h00000022); end
    if (!fail && (skind[1] !== 2'd1 || sval[1] !== 32'h00000073)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 1 was kind=%0d val=%0d, want kind=1 val=%0d", skind[1], sval[1], 32'h00000073); end
    if (!fail && (skind[2] !== 2'd1 || sval[2] !== 32'h00000074)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 2 was kind=%0d val=%0d, want kind=1 val=%0d", skind[2], sval[2], 32'h00000074); end
    if (!fail && (skind[3] !== 2'd1 || sval[3] !== 32'h00000072)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 3 was kind=%0d val=%0d, want kind=1 val=%0d", skind[3], sval[3], 32'h00000072); end
    if (!fail && (skind[4] !== 2'd1 || sval[4] !== 32'h00000065)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 4 was kind=%0d val=%0d, want kind=1 val=%0d", skind[4], sval[4], 32'h00000065); end
    if (!fail && (skind[5] !== 2'd1 || sval[5] !== 32'h00000074)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 5 was kind=%0d val=%0d, want kind=1 val=%0d", skind[5], sval[5], 32'h00000074); end
    if (!fail && (skind[6] !== 2'd1 || sval[6] !== 32'h00000063)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 6 was kind=%0d val=%0d, want kind=1 val=%0d", skind[6], sval[6], 32'h00000063); end
    if (!fail && (skind[7] !== 2'd1 || sval[7] !== 32'h00000068)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 7 was kind=%0d val=%0d, want kind=1 val=%0d", skind[7], sval[7], 32'h00000068); end
    if (!fail && (skind[8] !== 2'd1 || sval[8] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 8 was kind=%0d val=%0d, want kind=1 val=%0d", skind[8], sval[8], 32'h00000020); end
    if (!fail && (skind[9] !== 2'd1 || sval[9] !== 32'h00000074)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 9 was kind=%0d val=%0d, want kind=1 val=%0d", skind[9], sval[9], 32'h00000074); end
    if (!fail && (skind[10] !== 2'd1 || sval[10] !== 32'h00000072)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 10 was kind=%0d val=%0d, want kind=1 val=%0d", skind[10], sval[10], 32'h00000072); end
    if (!fail && (skind[11] !== 2'd1 || sval[11] !== 32'h00000065)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 11 was kind=%0d val=%0d, want kind=1 val=%0d", skind[11], sval[11], 32'h00000065); end
    if (!fail && (skind[12] !== 2'd1 || sval[12] !== 32'h00000065)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 12 was kind=%0d val=%0d, want kind=1 val=%0d", skind[12], sval[12], 32'h00000065); end
    if (!fail && (skind[13] !== 2'd1 || sval[13] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 13 was kind=%0d val=%0d, want kind=1 val=%0d", skind[13], sval[13], 32'h00000020); end
    if (!fail && (skind[14] !== 2'd1 || sval[14] !== 32'h0000006F)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 14 was kind=%0d val=%0d, want kind=1 val=%0d", skind[14], sval[14], 32'h0000006F); end
    if (!fail && (skind[15] !== 2'd1 || sval[15] !== 32'h00000066)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 15 was kind=%0d val=%0d, want kind=1 val=%0d", skind[15], sval[15], 32'h00000066); end
    if (!fail && (skind[16] !== 2'd1 || sval[16] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 16 was kind=%0d val=%0d, want kind=1 val=%0d", skind[16], sval[16], 32'h00000020); end
    if (!fail && (skind[17] !== 2'd1 || sval[17] !== 32'h00000064)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 17 was kind=%0d val=%0d, want kind=1 val=%0d", skind[17], sval[17], 32'h00000064); end
    if (!fail && (skind[18] !== 2'd1 || sval[18] !== 32'h00000065)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 18 was kind=%0d val=%0d, want kind=1 val=%0d", skind[18], sval[18], 32'h00000065); end
    if (!fail && (skind[19] !== 2'd1 || sval[19] !== 32'h00000070)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 19 was kind=%0d val=%0d, want kind=1 val=%0d", skind[19], sval[19], 32'h00000070); end
    if (!fail && (skind[20] !== 2'd1 || sval[20] !== 32'h00000074)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 20 was kind=%0d val=%0d, want kind=1 val=%0d", skind[20], sval[20], 32'h00000074); end
    if (!fail && (skind[21] !== 2'd1 || sval[21] !== 32'h00000068)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 21 was kind=%0d val=%0d, want kind=1 val=%0d", skind[21], sval[21], 32'h00000068); end
    if (!fail && (skind[22] !== 2'd1 || sval[22] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 22 was kind=%0d val=%0d, want kind=1 val=%0d", skind[22], sval[22], 32'h00000020); end
    if (!fail && (skind[23] !== 2'd1 || sval[23] !== 32'h00000037)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 23 was kind=%0d val=%0d, want kind=1 val=%0d", skind[23], sval[23], 32'h00000037); end
    if (!fail && (skind[24] !== 2'd1 || sval[24] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 24 was kind=%0d val=%0d, want kind=1 val=%0d", skind[24], sval[24], 32'h00000020); end
    if (!fail && (skind[25] !== 2'd1 || sval[25] !== 32'h00000063)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 25 was kind=%0d val=%0d, want kind=1 val=%0d", skind[25], sval[25], 32'h00000063); end
    if (!fail && (skind[26] !== 2'd1 || sval[26] !== 32'h00000068)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 26 was kind=%0d val=%0d, want kind=1 val=%0d", skind[26], sval[26], 32'h00000068); end
    if (!fail && (skind[27] !== 2'd1 || sval[27] !== 32'h00000065)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 27 was kind=%0d val=%0d, want kind=1 val=%0d", skind[27], sval[27], 32'h00000065); end
    if (!fail && (skind[28] !== 2'd1 || sval[28] !== 32'h00000063)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 28 was kind=%0d val=%0d, want kind=1 val=%0d", skind[28], sval[28], 32'h00000063); end
    if (!fail && (skind[29] !== 2'd1 || sval[29] !== 32'h0000006B)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 29 was kind=%0d val=%0d, want kind=1 val=%0d", skind[29], sval[29], 32'h0000006B); end
    if (!fail && (skind[30] !== 2'd1 || sval[30] !== 32'h0000003A)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 30 was kind=%0d val=%0d, want kind=1 val=%0d", skind[30], sval[30], 32'h0000003A); end
    if (!fail && (skind[31] !== 2'd1 || sval[31] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 31 was kind=%0d val=%0d, want kind=1 val=%0d", skind[31], sval[31], 32'h00000020); end
    if (!fail && (skind[32] !== 2'd1 || sval[32] !== 32'h00000032)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 32 was kind=%0d val=%0d, want kind=1 val=%0d", skind[32], sval[32], 32'h00000032); end
    if (!fail && (skind[33] !== 2'd1 || sval[33] !== 32'h00000035)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 33 was kind=%0d val=%0d, want kind=1 val=%0d", skind[33], sval[33], 32'h00000035); end
    if (!fail && (skind[34] !== 2'd1 || sval[34] !== 32'h00000035)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 34 was kind=%0d val=%0d, want kind=1 val=%0d", skind[34], sval[34], 32'h00000035); end
    if (!fail && (skind[35] !== 2'd2 || sval[35] !== 32'h0000001F)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 35 was kind=%0d val=%0d, want kind=2 val=%0d", skind[35], sval[35], 32'h0000001F); end
    if (!fail && (skind[36] !== 2'd1 || sval[36] !== 32'h00000036)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 36 was kind=%0d val=%0d, want kind=1 val=%0d", skind[36], sval[36], 32'h00000036); end
    if (!fail && (skind[37] !== 2'd1 || sval[37] !== 32'h00000034)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 37 was kind=%0d val=%0d, want kind=1 val=%0d", skind[37], sval[37], 32'h00000034); end
    if (!fail && (skind[38] !== 2'd1 || sval[38] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 38 was kind=%0d val=%0d, want kind=1 val=%0d", skind[38], sval[38], 32'h00000020); end
    if (!fail && (skind[39] !== 2'd1 || sval[39] !== 32'h00000074)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 39 was kind=%0d val=%0d, want kind=1 val=%0d", skind[39], sval[39], 32'h00000074); end
    if (!fail && (skind[40] !== 2'd1 || sval[40] !== 32'h00000072)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 40 was kind=%0d val=%0d, want kind=1 val=%0d", skind[40], sval[40], 32'h00000072); end
    if (!fail && (skind[41] !== 2'd1 || sval[41] !== 32'h00000065)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 41 was kind=%0d val=%0d, want kind=1 val=%0d", skind[41], sval[41], 32'h00000065); end
    if (!fail && (skind[42] !== 2'd1 || sval[42] !== 32'h00000065)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 42 was kind=%0d val=%0d, want kind=1 val=%0d", skind[42], sval[42], 32'h00000065); end
    if (!fail && (skind[43] !== 2'd1 || sval[43] !== 32'h00000073)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 43 was kind=%0d val=%0d, want kind=1 val=%0d", skind[43], sval[43], 32'h00000073); end
    if (!fail && (skind[44] !== 2'd1 || sval[44] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 44 was kind=%0d val=%0d, want kind=1 val=%0d", skind[44], sval[44], 32'h00000020); end
    if (!fail && (skind[45] !== 2'd1 || sval[45] !== 32'h0000006F)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 45 was kind=%0d val=%0d, want kind=1 val=%0d", skind[45], sval[45], 32'h0000006F); end
    if (!fail && (skind[46] !== 2'd1 || sval[46] !== 32'h00000066)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 46 was kind=%0d val=%0d, want kind=1 val=%0d", skind[46], sval[46], 32'h00000066); end
    if (!fail && (skind[47] !== 2'd1 || sval[47] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 47 was kind=%0d val=%0d, want kind=1 val=%0d", skind[47], sval[47], 32'h00000020); end
    if (!fail && (skind[48] !== 2'd1 || sval[48] !== 32'h00000064)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 48 was kind=%0d val=%0d, want kind=1 val=%0d", skind[48], sval[48], 32'h00000064); end
    if (!fail && (skind[49] !== 2'd1 || sval[49] !== 32'h00000065)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 49 was kind=%0d val=%0d, want kind=1 val=%0d", skind[49], sval[49], 32'h00000065); end
    if (!fail && (skind[50] !== 2'd1 || sval[50] !== 32'h00000070)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 50 was kind=%0d val=%0d, want kind=1 val=%0d", skind[50], sval[50], 32'h00000070); end
    if (!fail && (skind[51] !== 2'd1 || sval[51] !== 32'h00000074)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 51 was kind=%0d val=%0d, want kind=1 val=%0d", skind[51], sval[51], 32'h00000074); end
    if (!fail && (skind[52] !== 2'd1 || sval[52] !== 32'h00000068)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 52 was kind=%0d val=%0d, want kind=1 val=%0d", skind[52], sval[52], 32'h00000068); end
    if (!fail && (skind[53] !== 2'd1 || sval[53] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 53 was kind=%0d val=%0d, want kind=1 val=%0d", skind[53], sval[53], 32'h00000020); end
    if (!fail && (skind[54] !== 2'd1 || sval[54] !== 32'h00000034)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 54 was kind=%0d val=%0d, want kind=1 val=%0d", skind[54], sval[54], 32'h00000034); end
    if (!fail && (skind[55] !== 2'd1 || sval[55] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 55 was kind=%0d val=%0d, want kind=1 val=%0d", skind[55], sval[55], 32'h00000020); end
    if (!fail && (skind[56] !== 2'd1 || sval[56] !== 32'h00000063)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 56 was kind=%0d val=%0d, want kind=1 val=%0d", skind[56], sval[56], 32'h00000063); end
    if (!fail && (skind[57] !== 2'd1 || sval[57] !== 32'h00000068)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 57 was kind=%0d val=%0d, want kind=1 val=%0d", skind[57], sval[57], 32'h00000068); end
    if (!fail && (skind[58] !== 2'd1 || sval[58] !== 32'h00000065)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 58 was kind=%0d val=%0d, want kind=1 val=%0d", skind[58], sval[58], 32'h00000065); end
    if (!fail && (skind[59] !== 2'd1 || sval[59] !== 32'h00000063)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 59 was kind=%0d val=%0d, want kind=1 val=%0d", skind[59], sval[59], 32'h00000063); end
    if (!fail && (skind[60] !== 2'd1 || sval[60] !== 32'h0000006B)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 60 was kind=%0d val=%0d, want kind=1 val=%0d", skind[60], sval[60], 32'h0000006B); end
    if (!fail && (skind[61] !== 2'd1 || sval[61] !== 32'h0000003A)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 61 was kind=%0d val=%0d, want kind=1 val=%0d", skind[61], sval[61], 32'h0000003A); end
    if (!fail && (skind[62] !== 2'd1 || sval[62] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 62 was kind=%0d val=%0d, want kind=1 val=%0d", skind[62], sval[62], 32'h00000020); end
    if (!fail && (skind[63] !== 2'd1 || sval[63] !== 32'h00000031)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 63 was kind=%0d val=%0d, want kind=1 val=%0d", skind[63], sval[63], 32'h00000031); end
    if (!fail && (skind[64] !== 2'd1 || sval[64] !== 32'h00000039)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 64 was kind=%0d val=%0d, want kind=1 val=%0d", skind[64], sval[64], 32'h00000039); end
    if (!fail && (skind[65] !== 2'd1 || sval[65] !== 32'h00000038)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 65 was kind=%0d val=%0d, want kind=1 val=%0d", skind[65], sval[65], 32'h00000038); end
    if (!fail && (skind[66] !== 2'd1 || sval[66] !== 32'h00000034)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 66 was kind=%0d val=%0d, want kind=1 val=%0d", skind[66], sval[66], 32'h00000034); end
    if (!fail && (skind[67] !== 2'd2 || sval[67] !== 32'h0000001F)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 67 was kind=%0d val=%0d, want kind=2 val=%0d", skind[67], sval[67], 32'h0000001F); end
    if (!fail && (skind[68] !== 2'd1 || sval[68] !== 32'h00000031)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 68 was kind=%0d val=%0d, want kind=1 val=%0d", skind[68], sval[68], 32'h00000031); end
    if (!fail && (skind[69] !== 2'd1 || sval[69] !== 32'h00000036)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 69 was kind=%0d val=%0d, want kind=1 val=%0d", skind[69], sval[69], 32'h00000036); end
    if (!fail && (skind[70] !== 2'd1 || sval[70] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 70 was kind=%0d val=%0d, want kind=1 val=%0d", skind[70], sval[70], 32'h00000020); end
    if (!fail && (skind[71] !== 2'd1 || sval[71] !== 32'h00000074)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 71 was kind=%0d val=%0d, want kind=1 val=%0d", skind[71], sval[71], 32'h00000074); end
    if (!fail && (skind[72] !== 2'd1 || sval[72] !== 32'h00000072)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 72 was kind=%0d val=%0d, want kind=1 val=%0d", skind[72], sval[72], 32'h00000072); end
    if (!fail && (skind[73] !== 2'd1 || sval[73] !== 32'h00000065)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 73 was kind=%0d val=%0d, want kind=1 val=%0d", skind[73], sval[73], 32'h00000065); end
    if (!fail && (skind[74] !== 2'd1 || sval[74] !== 32'h00000065)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 74 was kind=%0d val=%0d, want kind=1 val=%0d", skind[74], sval[74], 32'h00000065); end
    if (!fail && (skind[75] !== 2'd1 || sval[75] !== 32'h00000073)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 75 was kind=%0d val=%0d, want kind=1 val=%0d", skind[75], sval[75], 32'h00000073); end
    if (!fail && (skind[76] !== 2'd1 || sval[76] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 76 was kind=%0d val=%0d, want kind=1 val=%0d", skind[76], sval[76], 32'h00000020); end
    if (!fail && (skind[77] !== 2'd1 || sval[77] !== 32'h0000006F)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 77 was kind=%0d val=%0d, want kind=1 val=%0d", skind[77], sval[77], 32'h0000006F); end
    if (!fail && (skind[78] !== 2'd1 || sval[78] !== 32'h00000066)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 78 was kind=%0d val=%0d, want kind=1 val=%0d", skind[78], sval[78], 32'h00000066); end
    if (!fail && (skind[79] !== 2'd1 || sval[79] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 79 was kind=%0d val=%0d, want kind=1 val=%0d", skind[79], sval[79], 32'h00000020); end
    if (!fail && (skind[80] !== 2'd1 || sval[80] !== 32'h00000064)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 80 was kind=%0d val=%0d, want kind=1 val=%0d", skind[80], sval[80], 32'h00000064); end
    if (!fail && (skind[81] !== 2'd1 || sval[81] !== 32'h00000065)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 81 was kind=%0d val=%0d, want kind=1 val=%0d", skind[81], sval[81], 32'h00000065); end
    if (!fail && (skind[82] !== 2'd1 || sval[82] !== 32'h00000070)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 82 was kind=%0d val=%0d, want kind=1 val=%0d", skind[82], sval[82], 32'h00000070); end
    if (!fail && (skind[83] !== 2'd1 || sval[83] !== 32'h00000074)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 83 was kind=%0d val=%0d, want kind=1 val=%0d", skind[83], sval[83], 32'h00000074); end
    if (!fail && (skind[84] !== 2'd1 || sval[84] !== 32'h00000068)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 84 was kind=%0d val=%0d, want kind=1 val=%0d", skind[84], sval[84], 32'h00000068); end
    if (!fail && (skind[85] !== 2'd1 || sval[85] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 85 was kind=%0d val=%0d, want kind=1 val=%0d", skind[85], sval[85], 32'h00000020); end
    if (!fail && (skind[86] !== 2'd1 || sval[86] !== 32'h00000036)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 86 was kind=%0d val=%0d, want kind=1 val=%0d", skind[86], sval[86], 32'h00000036); end
    if (!fail && (skind[87] !== 2'd1 || sval[87] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 87 was kind=%0d val=%0d, want kind=1 val=%0d", skind[87], sval[87], 32'h00000020); end
    if (!fail && (skind[88] !== 2'd1 || sval[88] !== 32'h00000063)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 88 was kind=%0d val=%0d, want kind=1 val=%0d", skind[88], sval[88], 32'h00000063); end
    if (!fail && (skind[89] !== 2'd1 || sval[89] !== 32'h00000068)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 89 was kind=%0d val=%0d, want kind=1 val=%0d", skind[89], sval[89], 32'h00000068); end
    if (!fail && (skind[90] !== 2'd1 || sval[90] !== 32'h00000065)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 90 was kind=%0d val=%0d, want kind=1 val=%0d", skind[90], sval[90], 32'h00000065); end
    if (!fail && (skind[91] !== 2'd1 || sval[91] !== 32'h00000063)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 91 was kind=%0d val=%0d, want kind=1 val=%0d", skind[91], sval[91], 32'h00000063); end
    if (!fail && (skind[92] !== 2'd1 || sval[92] !== 32'h0000006B)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 92 was kind=%0d val=%0d, want kind=1 val=%0d", skind[92], sval[92], 32'h0000006B); end
    if (!fail && (skind[93] !== 2'd1 || sval[93] !== 32'h0000003A)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 93 was kind=%0d val=%0d, want kind=1 val=%0d", skind[93], sval[93], 32'h0000003A); end
    if (!fail && (skind[94] !== 2'd1 || sval[94] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 94 was kind=%0d val=%0d, want kind=1 val=%0d", skind[94], sval[94], 32'h00000020); end
    if (!fail && (skind[95] !== 2'd1 || sval[95] !== 32'h00000032)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 95 was kind=%0d val=%0d, want kind=1 val=%0d", skind[95], sval[95], 32'h00000032); end
    if (!fail && (skind[96] !== 2'd1 || sval[96] !== 32'h00000030)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 96 was kind=%0d val=%0d, want kind=1 val=%0d", skind[96], sval[96], 32'h00000030); end
    if (!fail && (skind[97] !== 2'd1 || sval[97] !== 32'h00000033)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 97 was kind=%0d val=%0d, want kind=1 val=%0d", skind[97], sval[97], 32'h00000033); end
    if (!fail && (skind[98] !== 2'd1 || sval[98] !== 32'h00000032)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 98 was kind=%0d val=%0d, want kind=1 val=%0d", skind[98], sval[98], 32'h00000032); end
    if (!fail && (skind[99] !== 2'd2 || sval[99] !== 32'h00000025)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 99 was kind=%0d val=%0d, want kind=2 val=%0d", skind[99], sval[99], 32'h00000025); end
    if (!fail && (skind[100] !== 2'd1 || sval[100] !== 32'h0000006C)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 100 was kind=%0d val=%0d, want kind=1 val=%0d", skind[100], sval[100], 32'h0000006C); end
    if (!fail && (skind[101] !== 2'd1 || sval[101] !== 32'h0000006F)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 101 was kind=%0d val=%0d, want kind=1 val=%0d", skind[101], sval[101], 32'h0000006F); end
    if (!fail && (skind[102] !== 2'd1 || sval[102] !== 32'h0000006E)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 102 was kind=%0d val=%0d, want kind=1 val=%0d", skind[102], sval[102], 32'h0000006E); end
    if (!fail && (skind[103] !== 2'd1 || sval[103] !== 32'h00000067)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 103 was kind=%0d val=%0d, want kind=1 val=%0d", skind[103], sval[103], 32'h00000067); end
    if (!fail && (skind[104] !== 2'd1 || sval[104] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 104 was kind=%0d val=%0d, want kind=1 val=%0d", skind[104], sval[104], 32'h00000020); end
    if (!fail && (skind[105] !== 2'd1 || sval[105] !== 32'h0000006C)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 105 was kind=%0d val=%0d, want kind=1 val=%0d", skind[105], sval[105], 32'h0000006C); end
    if (!fail && (skind[106] !== 2'd1 || sval[106] !== 32'h00000069)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 106 was kind=%0d val=%0d, want kind=1 val=%0d", skind[106], sval[106], 32'h00000069); end
    if (!fail && (skind[107] !== 2'd1 || sval[107] !== 32'h00000076)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 107 was kind=%0d val=%0d, want kind=1 val=%0d", skind[107], sval[107], 32'h00000076); end
    if (!fail && (skind[108] !== 2'd1 || sval[108] !== 32'h00000065)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 108 was kind=%0d val=%0d, want kind=1 val=%0d", skind[108], sval[108], 32'h00000065); end
    if (!fail && (skind[109] !== 2'd1 || sval[109] !== 32'h00000064)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 109 was kind=%0d val=%0d, want kind=1 val=%0d", skind[109], sval[109], 32'h00000064); end
    if (!fail && (skind[110] !== 2'd1 || sval[110] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 110 was kind=%0d val=%0d, want kind=1 val=%0d", skind[110], sval[110], 32'h00000020); end
    if (!fail && (skind[111] !== 2'd1 || sval[111] !== 32'h00000074)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 111 was kind=%0d val=%0d, want kind=1 val=%0d", skind[111], sval[111], 32'h00000074); end
    if (!fail && (skind[112] !== 2'd1 || sval[112] !== 32'h00000072)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 112 was kind=%0d val=%0d, want kind=1 val=%0d", skind[112], sval[112], 32'h00000072); end
    if (!fail && (skind[113] !== 2'd1 || sval[113] !== 32'h00000065)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 113 was kind=%0d val=%0d, want kind=1 val=%0d", skind[113], sval[113], 32'h00000065); end
    if (!fail && (skind[114] !== 2'd1 || sval[114] !== 32'h00000065)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 114 was kind=%0d val=%0d, want kind=1 val=%0d", skind[114], sval[114], 32'h00000065); end
    if (!fail && (skind[115] !== 2'd1 || sval[115] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 115 was kind=%0d val=%0d, want kind=1 val=%0d", skind[115], sval[115], 32'h00000020); end
    if (!fail && (skind[116] !== 2'd1 || sval[116] !== 32'h0000006F)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 116 was kind=%0d val=%0d, want kind=1 val=%0d", skind[116], sval[116], 32'h0000006F); end
    if (!fail && (skind[117] !== 2'd1 || sval[117] !== 32'h00000066)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 117 was kind=%0d val=%0d, want kind=1 val=%0d", skind[117], sval[117], 32'h00000066); end
    if (!fail && (skind[118] !== 2'd1 || sval[118] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 118 was kind=%0d val=%0d, want kind=1 val=%0d", skind[118], sval[118], 32'h00000020); end
    if (!fail && (skind[119] !== 2'd1 || sval[119] !== 32'h00000064)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 119 was kind=%0d val=%0d, want kind=1 val=%0d", skind[119], sval[119], 32'h00000064); end
    if (!fail && (skind[120] !== 2'd1 || sval[120] !== 32'h00000065)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 120 was kind=%0d val=%0d, want kind=1 val=%0d", skind[120], sval[120], 32'h00000065); end
    if (!fail && (skind[121] !== 2'd1 || sval[121] !== 32'h00000070)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 121 was kind=%0d val=%0d, want kind=1 val=%0d", skind[121], sval[121], 32'h00000070); end
    if (!fail && (skind[122] !== 2'd1 || sval[122] !== 32'h00000074)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 122 was kind=%0d val=%0d, want kind=1 val=%0d", skind[122], sval[122], 32'h00000074); end
    if (!fail && (skind[123] !== 2'd1 || sval[123] !== 32'h00000068)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 123 was kind=%0d val=%0d, want kind=1 val=%0d", skind[123], sval[123], 32'h00000068); end
    if (!fail && (skind[124] !== 2'd1 || sval[124] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 124 was kind=%0d val=%0d, want kind=1 val=%0d", skind[124], sval[124], 32'h00000020); end
    if (!fail && (skind[125] !== 2'd1 || sval[125] !== 32'h00000036)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 125 was kind=%0d val=%0d, want kind=1 val=%0d", skind[125], sval[125], 32'h00000036); end
    if (!fail && (skind[126] !== 2'd1 || sval[126] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 126 was kind=%0d val=%0d, want kind=1 val=%0d", skind[126], sval[126], 32'h00000020); end
    if (!fail && (skind[127] !== 2'd1 || sval[127] !== 32'h00000063)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 127 was kind=%0d val=%0d, want kind=1 val=%0d", skind[127], sval[127], 32'h00000063); end
    if (!fail && (skind[128] !== 2'd1 || sval[128] !== 32'h00000068)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 128 was kind=%0d val=%0d, want kind=1 val=%0d", skind[128], sval[128], 32'h00000068); end
    if (!fail && (skind[129] !== 2'd1 || sval[129] !== 32'h00000065)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 129 was kind=%0d val=%0d, want kind=1 val=%0d", skind[129], sval[129], 32'h00000065); end
    if (!fail && (skind[130] !== 2'd1 || sval[130] !== 32'h00000063)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 130 was kind=%0d val=%0d, want kind=1 val=%0d", skind[130], sval[130], 32'h00000063); end
    if (!fail && (skind[131] !== 2'd1 || sval[131] !== 32'h0000006B)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 131 was kind=%0d val=%0d, want kind=1 val=%0d", skind[131], sval[131], 32'h0000006B); end
    if (!fail && (skind[132] !== 2'd1 || sval[132] !== 32'h0000003A)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 132 was kind=%0d val=%0d, want kind=1 val=%0d", skind[132], sval[132], 32'h0000003A); end
    if (!fail && (skind[133] !== 2'd1 || sval[133] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 133 was kind=%0d val=%0d, want kind=1 val=%0d", skind[133], sval[133], 32'h00000020); end
    if (!fail && (skind[134] !== 2'd1 || sval[134] !== 32'h00000031)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 134 was kind=%0d val=%0d, want kind=1 val=%0d", skind[134], sval[134], 32'h00000031); end
    if (!fail && (skind[135] !== 2'd1 || sval[135] !== 32'h00000032)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 135 was kind=%0d val=%0d, want kind=1 val=%0d", skind[135], sval[135], 32'h00000032); end
    if (!fail && (skind[136] !== 2'd1 || sval[136] !== 32'h00000037)) begin fail = 1; $display("FAIL: text_bench_binary_trees — entry 136 was kind=%0d val=%0d, want kind=1 val=%0d", skind[136], sval[136], 32'h00000037); end
    if (!fail) $display("PASS: text_bench_binary_trees");
    $finish;
  end
  initial begin #2000000000 $display("FAIL: text_bench_binary_trees timeout"); $finish; end
endmodule
