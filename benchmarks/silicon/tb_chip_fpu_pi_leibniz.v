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
    dut.imem[2] = 32'h00000000;
    dut.imem[3] = 32'hD3010000;
    dut.imem[4] = 32'hC9020000;
    dut.imem[5] = 32'h00000000;
    dut.imem[6] = 32'h3FF00000;
    dut.imem[7] = 32'hD3030200;
    dut.imem[8] = 32'h05040000;
    dut.imem[9] = 32'h06050400;
    dut.imem[10] = 32'h050703E8;
    dut.imem[11] = 32'h19060507;
    dut.imem[12] = 32'h23200600;
    dut.imem[13] = 32'hC90B0000;
    dut.imem[14] = 32'h00000000;
    dut.imem[15] = 32'h40000000;
    dut.imem[16] = 32'hD1050500;
    dut.imem[17] = 32'hCC0A0B05;
    dut.imem[18] = 32'hC90C0000;
    dut.imem[19] = 32'h00000000;
    dut.imem[20] = 32'h3FF00000;
    dut.imem[21] = 32'hCA090A0C;
    dut.imem[22] = 32'hCD080309;
    dut.imem[23] = 32'hCA010108;
    dut.imem[24] = 32'hC90E0000;
    dut.imem[25] = 32'h00000000;
    dut.imem[26] = 32'h00000000;
    dut.imem[27] = 32'hCB0D0E03;
    dut.imem[28] = 32'hD3030D00;
    dut.imem[29] = 32'h050F0001;
    dut.imem[30] = 32'h0805050F;
    dut.imem[31] = 32'h220A0000;
    dut.imem[32] = 32'hC9090000;
    dut.imem[33] = 32'h00000000;
    dut.imem[34] = 32'h40100000;
    dut.imem[35] = 32'hCC080109;
    dut.imem[36] = 32'hD4000800;
    dut.imem[37] = 32'h2B000000;
    #3 rst = 0;
    wait (halted);
    #10;
    if (n !== 1) begin $display("FAIL: pi_leibniz (got n=%0d, want 1)", n); $finish; end
    if (seen[0] !== 64'h40091FEF0A9265EB) begin $display("FAIL: pi_leibniz seen[0]=%h want 40091FEF0A9265EB", seen[0]); $finish; end
    $display("PASS: pi_leibniz (1 shows, float bit-exact)");
    $finish;
  end
  initial begin #2000000000 $display("FAIL: pi_leibniz timeout"); $finish; end
endmodule
