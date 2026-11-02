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
    dut.imem[0] = 32'h05010014;
    dut.imem[1] = 32'h25000101;
    dut.imem[2] = 32'h00000005;
    dut.imem[3] = 32'h59000000;
    dut.imem[4] = 32'h2B000000;
    dut.imem[5] = 32'hD8010000;
    dut.imem[6] = 32'h0000002A;
    dut.imem[7] = 32'h06020100;
    dut.imem[8] = 32'h05030000;
    dut.imem[9] = 32'h06040300;
    dut.imem[10] = 32'h19050400;
    dut.imem[11] = 32'h23150500;
    dut.imem[12] = 32'h06060400;
    dut.imem[13] = 32'h40110600;
    dut.imem[14] = 32'h20020211;
    dut.imem[15] = 32'hD8070001;
    dut.imem[16] = 32'h0000002A;
    dut.imem[17] = 32'h20020207;
    dut.imem[18] = 32'h05060001;
    dut.imem[19] = 32'h08040406;
    dut.imem[20] = 32'h220A0000;
    dut.imem[21] = 32'h05050000;
    dut.imem[22] = 32'h06060500;
    dut.imem[23] = 32'h05070000;
    dut.imem[24] = 32'h06080700;
    dut.imem[25] = 32'h3E0A0200;
    dut.imem[26] = 32'h1909080A;
    dut.imem[27] = 32'h23280900;
    dut.imem[28] = 32'h050E0001;
    dut.imem[29] = 32'h080D080E;
    dut.imem[30] = 32'hDA0C020D;
    dut.imem[31] = 32'hD80F0001;
    dut.imem[32] = 32'h0000002A;
    dut.imem[33] = 32'hDB0B0C0F;
    dut.imem[34] = 32'h23250B00;
    dut.imem[35] = 32'h05100001;
    dut.imem[36] = 32'h08060610;
    dut.imem[37] = 32'h050B0001;
    dut.imem[38] = 32'h0808080B;
    dut.imem[39] = 32'h22190000;
    dut.imem[40] = 32'h29000600;
    dut.imem[41] = 32'h2A000000;
    dut.imem[42] = 32'h00000020;
    #3 rst = 0;
    wait (halted);
    #10;
    fail = 0;
    if (n != 1) begin fail = 1; $display("FAIL: text_bench_strings — got %0d capture entries, want 1", n); end
    if (!fail && (skind[0] !== 2'd0 || sval[0] !== 32'h00000014)) begin fail = 1; $display("FAIL: text_bench_strings — entry 0 was kind=%0d val=%0d, want kind=0 val=%0d", skind[0], sval[0], 32'h00000014); end
    if (!fail) $display("PASS: text_bench_strings");
    $finish;
  end
  initial begin #2000000000 $display("FAIL: text_bench_strings timeout"); $finish; end
endmodule
