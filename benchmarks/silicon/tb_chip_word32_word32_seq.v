`timescale 1ns/1ps
// AUTO-GENERATED from the run_soft_core_typed ≡VM word32 differential (codegen_sva/rtl.rs). Do not edit.
// Regenerate: LOGOS_EMIT_WORD32_TB=1 cargo test -p logicaffeine-compile --lib emit_word32_tbs
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire [63:0] outf; wire [31:0] outs; wire [31:0] outw;
  wire ov, ofv, osv, osf, owv, halted;
  soft_core #(.FRAMES(64), .REGS(256), .IMEM_WORDS(1024), .HEAP(4096))
    dut(.clk(clk), .rst(rst), .out(out), .out_valid(ov), .outf(outf), .outf_valid(ofv),
        .outs(outs), .outs_valid(osv), .outs_first(osf), .outw(outw), .outw_valid(owv), .halted(halted));
  integer n = 0; reg [1:0] skind [0:1023]; reg [31:0] sval [0:1023];
  always #5 clk = ~clk;
  always @(posedge clk) begin
    if (ov)  begin skind[n] = 2'd0; sval[n] = out;  n = n + 1; end
    if (owv) begin skind[n] = 2'd3; sval[n] = outw; n = n + 1; end
  end
  integer e; reg fail;
  initial begin
    #1;
    dut.imem[0] = 32'h31000000;
    dut.imem[1] = 32'h05010001;
    dut.imem[2] = 32'h06020100;
    dut.imem[3] = 32'h05040004;
    dut.imem[4] = 32'h1B030204;
    dut.imem[5] = 32'h230C0300;
    dut.imem[6] = 32'h0B060202;
    dut.imem[7] = 32'h06050600;
    dut.imem[8] = 32'h36000500;
    dut.imem[9] = 32'h05070001;
    dut.imem[10] = 32'h08020207;
    dut.imem[11] = 32'h22030000;
    dut.imem[12] = 32'h05070003;
    dut.imem[13] = 32'h3C060007;
    dut.imem[14] = 32'h06050600;
    dut.imem[15] = 32'hDD000500;
    dut.imem[16] = 32'h2B000000;
    #3 rst = 0;
    wait (halted);
    #10;
    fail = 0;
    if (n != 1) begin fail = 1; $display("FAIL: word32_word32_seq — got %0d capture entries, want 1", n); end
    if (!fail && (skind[0] !== 2'd3 || sval[0] !== 32'h00000009)) begin fail = 1; $display("FAIL: word32_word32_seq — entry 0 was kind=%0d val=%0d, want kind=3 val=%0d", skind[0], sval[0], 32'h00000009); end
    if (!fail) $display("PASS: word32_word32_seq");
    $finish;
  end
  initial begin #2000000000 $display("FAIL: word32_word32_seq timeout"); $finish; end
endmodule
