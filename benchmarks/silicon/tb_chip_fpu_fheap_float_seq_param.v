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
    dut.imem[0] = 32'h31000000;
    dut.imem[1] = 32'hC9010000;
    dut.imem[2] = 32'h00000000;
    dut.imem[3] = 32'h3FF40000;
    dut.imem[4] = 32'hD5000100;
    dut.imem[5] = 32'hC9020000;
    dut.imem[6] = 32'h00000000;
    dut.imem[7] = 32'h3FFC0000;
    dut.imem[8] = 32'hD5000200;
    dut.imem[9] = 32'h06040000;
    dut.imem[10] = 32'h25030401;
    dut.imem[11] = 32'h0000000E;
    dut.imem[12] = 32'hD4000300;
    dut.imem[13] = 32'h2B000000;
    dut.imem[14] = 32'h05030001;
    dut.imem[15] = 32'hD6020003;
    dut.imem[16] = 32'h05050002;
    dut.imem[17] = 32'hD6040005;
    dut.imem[18] = 32'hCA010204;
    dut.imem[19] = 32'h29000100;
    dut.imem[20] = 32'h2A000000;
    #3 rst = 0;
    wait (halted);
    #10;
    if (n !== 1) begin $display("FAIL: fheap_float_seq_param (got n=%0d, want 1)", n); $finish; end
    if (seen[0] !== 64'h4008000000000000) begin $display("FAIL: fheap_float_seq_param seen[0]=%h want 4008000000000000", seen[0]); $finish; end
    $display("PASS: fheap_float_seq_param (1 shows, float bit-exact)");
    $finish;
  end
  initial begin #2000000000 $display("FAIL: fheap_float_seq_param timeout"); $finish; end
endmodule
