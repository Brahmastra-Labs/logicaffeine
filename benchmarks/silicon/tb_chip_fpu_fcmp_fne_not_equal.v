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
    dut.imem[0] = 32'hC9000000;
    dut.imem[1] = 32'h00000000;
    dut.imem[2] = 32'h3FF80000;
    dut.imem[3] = 32'hD3010000;
    dut.imem[4] = 32'hC9020000;
    dut.imem[5] = 32'h00000000;
    dut.imem[6] = 32'h40040000;
    dut.imem[7] = 32'hD3030200;
    dut.imem[8] = 32'hE6040103;
    dut.imem[9] = 32'h230D0400;
    dut.imem[10] = 32'h05050001;
    dut.imem[11] = 32'h59000500;
    dut.imem[12] = 32'h220F0000;
    dut.imem[13] = 32'h05050000;
    dut.imem[14] = 32'h59000500;
    dut.imem[15] = 32'hE6050303;
    dut.imem[16] = 32'h23140500;
    dut.imem[17] = 32'h05060001;
    dut.imem[18] = 32'h59000600;
    dut.imem[19] = 32'h22160000;
    dut.imem[20] = 32'h05060000;
    dut.imem[21] = 32'h59000600;
    dut.imem[22] = 32'h2B000000;
    #3 rst = 0;
    wait (halted);
    #10;
    if (n !== 2) begin $display("FAIL: fcmp_fne_not_equal (got n=%0d, want 2)", n); $finish; end
    if (seen[0][31:0] !== 32'h00000001) begin $display("FAIL: fcmp_fne_not_equal seen[0]=%h (int)", seen[0]); $finish; end
    if (seen[1][31:0] !== 32'h00000000) begin $display("FAIL: fcmp_fne_not_equal seen[1]=%h (int)", seen[1]); $finish; end
    $display("PASS: fcmp_fne_not_equal (2 shows, float bit-exact)");
    $finish;
  end
  initial begin #2000000000 $display("FAIL: fcmp_fne_not_equal timeout"); $finish; end
endmodule
