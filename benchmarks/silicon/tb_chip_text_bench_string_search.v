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
    dut.imem[0] = 32'h05000064;
    dut.imem[1] = 32'h06010000;
    dut.imem[2] = 32'hD8020000;
    dut.imem[3] = 32'h00000072;
    dut.imem[4] = 32'h06030200;
    dut.imem[5] = 32'h05040000;
    dut.imem[6] = 32'h06050400;
    dut.imem[7] = 32'h19060501;
    dut.imem[8] = 32'h23440600;
    dut.imem[9] = 32'h05080000;
    dut.imem[10] = 32'h1A070508;
    dut.imem[11] = 32'h231A0700;
    dut.imem[12] = 32'h050B0014;
    dut.imem[13] = 32'h0F0A050B;
    dut.imem[14] = 32'h050C0000;
    dut.imem[15] = 32'h1D090A0C;
    dut.imem[16] = 32'h231A0900;
    dut.imem[17] = 32'h050F0005;
    dut.imem[18] = 32'h080E050F;
    dut.imem[19] = 32'h1B0D0E01;
    dut.imem[20] = 32'h231A0D00;
    dut.imem[21] = 32'hD8100005;
    dut.imem[22] = 32'h00000072;
    dut.imem[23] = 32'h20030310;
    dut.imem[24] = 32'h05100005;
    dut.imem[25] = 32'h08050510;
    dut.imem[26] = 32'h19070501;
    dut.imem[27] = 32'h23430700;
    dut.imem[28] = 32'hD8080001;
    dut.imem[29] = 32'h00000077;
    dut.imem[30] = 32'h06090800;
    dut.imem[31] = 32'h050C0005;
    dut.imem[32] = 32'h0F0B050C;
    dut.imem[33] = 32'h050D0001;
    dut.imem[34] = 32'h1D0A0B0D;
    dut.imem[35] = 32'h23270A00;
    dut.imem[36] = 32'hD80E0001;
    dut.imem[37] = 32'h00000078;
    dut.imem[38] = 32'h06090E00;
    dut.imem[39] = 32'h050C0005;
    dut.imem[40] = 32'h0F0B050C;
    dut.imem[41] = 32'h050D0002;
    dut.imem[42] = 32'h1D0A0B0D;
    dut.imem[43] = 32'h232F0A00;
    dut.imem[44] = 32'hD80E0001;
    dut.imem[45] = 32'h00000079;
    dut.imem[46] = 32'h06090E00;
    dut.imem[47] = 32'h050C0005;
    dut.imem[48] = 32'h0F0B050C;
    dut.imem[49] = 32'h050D0003;
    dut.imem[50] = 32'h1D0A0B0D;
    dut.imem[51] = 32'h23370A00;
    dut.imem[52] = 32'hD80E0001;
    dut.imem[53] = 32'h0000007A;
    dut.imem[54] = 32'h06090E00;
    dut.imem[55] = 32'h050C0005;
    dut.imem[56] = 32'h0F0B050C;
    dut.imem[57] = 32'h050D0004;
    dut.imem[58] = 32'h1D0A0B0D;
    dut.imem[59] = 32'h233F0A00;
    dut.imem[60] = 32'hD80E0001;
    dut.imem[61] = 32'h0000007B;
    dut.imem[62] = 32'h06090E00;
    dut.imem[63] = 32'h060A0900;
    dut.imem[64] = 32'h2003030A;
    dut.imem[65] = 32'h050A0001;
    dut.imem[66] = 32'h0805050A;
    dut.imem[67] = 32'h22070000;
    dut.imem[68] = 32'hD8060005;
    dut.imem[69] = 32'h00000072;
    dut.imem[70] = 32'h06070600;
    dut.imem[71] = 32'h05080005;
    dut.imem[72] = 32'h06090800;
    dut.imem[73] = 32'h3E0A0300;
    dut.imem[74] = 32'h060B0A00;
    dut.imem[75] = 32'h050C0000;
    dut.imem[76] = 32'h060D0C00;
    dut.imem[77] = 32'h050E0001;
    dut.imem[78] = 32'h060F0E00;
    dut.imem[79] = 32'h0A120B09;
    dut.imem[80] = 32'h05130001;
    dut.imem[81] = 32'h08111213;
    dut.imem[82] = 32'h1B100F11;
    dut.imem[83] = 32'h23701000;
    dut.imem[84] = 32'h05140001;
    dut.imem[85] = 32'h06151400;
    dut.imem[86] = 32'h05160000;
    dut.imem[87] = 32'h06171600;
    dut.imem[88] = 32'h19181709;
    dut.imem[89] = 32'h23681800;
    dut.imem[90] = 32'h081B0F17;
    dut.imem[91] = 32'hDA1A031B;
    dut.imem[92] = 32'h051E0001;
    dut.imem[93] = 32'h081D171E;
    dut.imem[94] = 32'hDA1C071D;
    dut.imem[95] = 32'hDC191A1C;
    dut.imem[96] = 32'h23651900;
    dut.imem[97] = 32'h051F0000;
    dut.imem[98] = 32'h06151F00;
    dut.imem[99] = 32'h061F0900;
    dut.imem[100] = 32'h06171F00;
    dut.imem[101] = 32'h05190001;
    dut.imem[102] = 32'h08171719;
    dut.imem[103] = 32'h22580000;
    dut.imem[104] = 32'h05190001;
    dut.imem[105] = 32'h1D181519;
    dut.imem[106] = 32'h236D1800;
    dut.imem[107] = 32'h051A0001;
    dut.imem[108] = 32'h080D0D1A;
    dut.imem[109] = 32'h05180001;
    dut.imem[110] = 32'h080F0F18;
    dut.imem[111] = 32'h224F0000;
    dut.imem[112] = 32'h59000D00;
    dut.imem[113] = 32'h2B000000;
    dut.imem[114] = 32'h00000058;
    dut.imem[115] = 32'h00000058;
    dut.imem[116] = 32'h00000058;
    dut.imem[117] = 32'h00000058;
    dut.imem[118] = 32'h00000058;
    dut.imem[119] = 32'h00000061;
    dut.imem[120] = 32'h00000062;
    dut.imem[121] = 32'h00000063;
    dut.imem[122] = 32'h00000064;
    dut.imem[123] = 32'h00000065;
    #3 rst = 0;
    wait (halted);
    #10;
    fail = 0;
    if (n != 1) begin fail = 1; $display("FAIL: text_bench_string_search — got %0d capture entries, want 1", n); end
    if (!fail && (skind[0] !== 2'd0 || sval[0] !== 32'h00000004)) begin fail = 1; $display("FAIL: text_bench_string_search — entry 0 was kind=%0d val=%0d, want kind=0 val=%0d", skind[0], sval[0], 32'h00000004); end
    if (!fail) $display("PASS: text_bench_string_search");
    $finish;
  end
  initial begin #2000000000 $display("FAIL: text_bench_string_search timeout"); $finish; end
endmodule
