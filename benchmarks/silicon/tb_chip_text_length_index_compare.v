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
    dut.imem[0] = 32'hD8000005;
    dut.imem[1] = 32'h00000016;
    dut.imem[2] = 32'h06010000;
    dut.imem[3] = 32'h05020000;
    dut.imem[4] = 32'h06030200;
    dut.imem[5] = 32'h05040001;
    dut.imem[6] = 32'h06050400;
    dut.imem[7] = 32'h3E070100;
    dut.imem[8] = 32'h1B060507;
    dut.imem[9] = 32'h23140600;
    dut.imem[10] = 32'hDA090105;
    dut.imem[11] = 32'hD80A0001;
    dut.imem[12] = 32'h0000001B;
    dut.imem[13] = 32'hDB08090A;
    dut.imem[14] = 32'h23110800;
    dut.imem[15] = 32'h050B0001;
    dut.imem[16] = 32'h0803030B;
    dut.imem[17] = 32'h05080001;
    dut.imem[18] = 32'h08050508;
    dut.imem[19] = 32'h22070000;
    dut.imem[20] = 32'h59000300;
    dut.imem[21] = 32'h2B000000;
    dut.imem[22] = 32'h00000061;
    dut.imem[23] = 32'h00000020;
    dut.imem[24] = 32'h00000062;
    dut.imem[25] = 32'h00000020;
    dut.imem[26] = 32'h00000063;
    dut.imem[27] = 32'h00000020;
    #3 rst = 0;
    wait (halted);
    #10;
    fail = 0;
    if (n != 1) begin fail = 1; $display("FAIL: text_length_index_compare — got %0d capture entries, want 1", n); end
    if (!fail && (skind[0] !== 2'd0 || sval[0] !== 32'h00000002)) begin fail = 1; $display("FAIL: text_length_index_compare — entry 0 was kind=%0d val=%0d, want kind=0 val=%0d", skind[0], sval[0], 32'h00000002); end
    if (!fail) $display("PASS: text_length_index_compare");
    $finish;
  end
  initial begin #2000000000 $display("FAIL: text_length_index_compare timeout"); $finish; end
endmodule
