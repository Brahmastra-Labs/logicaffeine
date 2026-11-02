`timescale 1ns/1ps
// AUTO-GENERATED from the run_soft_core_typed ≡VM float differential (codegen_sva/rtl.rs). Do not edit.
// Regenerate: LOGOS_EMIT_FPU_TB=1 cargo test -p logicaffeine-compile --lib float_chip_benchmarks_match_vm_and_emit_rtl_tb
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire [63:0] outf; wire ov, ofv, halted;
  soft_core #(.FRAMES(64), .REGS(256), .IMEM_WORDS(1024), .HEAP(2048))
    dut(.clk(clk), .rst(rst), .out(out), .out_valid(ov), .outf(outf), .outf_valid(ofv), .halted(halted));
  integer n = 0; reg [63:0] seen [0:63];
  always #5 clk = ~clk;
  always @(posedge clk) begin
    if (ov)  begin seen[n] = {32'd0, out}; n = n + 1; end
    if (ofv) begin seen[n] = outf;          n = n + 1; end
  end
  function is_nan(input [63:0] v); is_nan = (v[62:52] == 11'h7FF) && (v[51:0] != 52'd0); endfunction
  initial begin
    #1;
    dut.imem[0] = 32'h05000014;
    dut.imem[1] = 32'h06010000;
    dut.imem[2] = 32'h05020000;
    dut.imem[3] = 32'h06030200;
    dut.imem[4] = 32'h05040000;
    dut.imem[5] = 32'h06050400;
    dut.imem[6] = 32'h19060501;
    dut.imem[7] = 32'h235D0600;
    dut.imem[8] = 32'h05070000;
    dut.imem[9] = 32'h06080700;
    dut.imem[10] = 32'h19090801;
    dut.imem[11] = 32'h235A0900;
    dut.imem[12] = 32'hC90D0000;
    dut.imem[13] = 32'h00000000;
    dut.imem[14] = 32'h40000000;
    dut.imem[15] = 32'hD1080800;
    dut.imem[16] = 32'hCC0C0D08;
    dut.imem[17] = 32'hD1010100;
    dut.imem[18] = 32'hCD0B0C01;
    dut.imem[19] = 32'hC90E0000;
    dut.imem[20] = 32'h00000000;
    dut.imem[21] = 32'h3FF80000;
    dut.imem[22] = 32'hCB0A0B0E;
    dut.imem[23] = 32'hD30F0A00;
    dut.imem[24] = 32'hC9130000;
    dut.imem[25] = 32'h00000000;
    dut.imem[26] = 32'h40000000;
    dut.imem[27] = 32'hD1050500;
    dut.imem[28] = 32'hCC121305;
    dut.imem[29] = 32'hD1010100;
    dut.imem[30] = 32'hCD111201;
    dut.imem[31] = 32'hC9140000;
    dut.imem[32] = 32'h00000000;
    dut.imem[33] = 32'h3FF00000;
    dut.imem[34] = 32'hCB101114;
    dut.imem[35] = 32'hD3151000;
    dut.imem[36] = 32'hC9160000;
    dut.imem[37] = 32'h00000000;
    dut.imem[38] = 32'h00000000;
    dut.imem[39] = 32'hD3171600;
    dut.imem[40] = 32'hC9180000;
    dut.imem[41] = 32'h00000000;
    dut.imem[42] = 32'h00000000;
    dut.imem[43] = 32'hD3191800;
    dut.imem[44] = 32'h051A0001;
    dut.imem[45] = 32'h061B1A00;
    dut.imem[46] = 32'h051C0000;
    dut.imem[47] = 32'h061D1C00;
    dut.imem[48] = 32'h051F0032;
    dut.imem[49] = 32'h191E1D1F;
    dut.imem[50] = 32'h23521E00;
    dut.imem[51] = 32'hCC221717;
    dut.imem[52] = 32'hCC231919;
    dut.imem[53] = 32'hCB212223;
    dut.imem[54] = 32'hCA20210F;
    dut.imem[55] = 32'hD3242000;
    dut.imem[56] = 32'hC9280000;
    dut.imem[57] = 32'h00000000;
    dut.imem[58] = 32'h40000000;
    dut.imem[59] = 32'hCC272817;
    dut.imem[60] = 32'hCC262719;
    dut.imem[61] = 32'hCA252615;
    dut.imem[62] = 32'hD3292500;
    dut.imem[63] = 32'hD32A2400;
    dut.imem[64] = 32'hD3172A00;
    dut.imem[65] = 32'hD32B2900;
    dut.imem[66] = 32'hD3192B00;
    dut.imem[67] = 32'hCC2E1717;
    dut.imem[68] = 32'hCC2F1919;
    dut.imem[69] = 32'hCA2D2E2F;
    dut.imem[70] = 32'hC9300000;
    dut.imem[71] = 32'h00000000;
    dut.imem[72] = 32'h40100000;
    dut.imem[73] = 32'hCF2C302D;
    dut.imem[74] = 32'h234F2C00;
    dut.imem[75] = 32'h05310000;
    dut.imem[76] = 32'h061B3100;
    dut.imem[77] = 32'h05320032;
    dut.imem[78] = 32'h061D3200;
    dut.imem[79] = 32'h05310001;
    dut.imem[80] = 32'h081D1D31;
    dut.imem[81] = 32'h22300000;
    dut.imem[82] = 32'h05210001;
    dut.imem[83] = 32'h1D201B21;
    dut.imem[84] = 32'h23572000;
    dut.imem[85] = 32'h05220001;
    dut.imem[86] = 32'h08030322;
    dut.imem[87] = 32'h05220001;
    dut.imem[88] = 32'h08080822;
    dut.imem[89] = 32'h220A0000;
    dut.imem[90] = 32'h050A0001;
    dut.imem[91] = 32'h0805050A;
    dut.imem[92] = 32'h22060000;
    dut.imem[93] = 32'h59000300;
    dut.imem[94] = 32'h2B000000;
    #3 rst = 0;
    wait (halted);
    #10;
    if (n !== 1) begin $display("FAIL: mandelbrot (got n=%0d, want 1)", n); $finish; end
    if (seen[0][31:0] !== 32'h0000009D) begin $display("FAIL: mandelbrot seen[0]=%h (int)", seen[0]); $finish; end
    $display("PASS: mandelbrot (1 shows, float bit-exact)");
    $finish;
  end
  initial begin #2000000000 $display("FAIL: mandelbrot timeout"); $finish; end
endmodule
