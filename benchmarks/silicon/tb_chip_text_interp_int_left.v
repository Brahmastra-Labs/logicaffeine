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
    dut.imem[0] = 32'h0501007B;
    dut.imem[1] = 32'hD8020001;
    dut.imem[2] = 32'h00000007;
    dut.imem[3] = 32'h40030100;
    dut.imem[4] = 32'h20000302;
    dut.imem[5] = 32'hD9000000;
    dut.imem[6] = 32'h2B000000;
    dut.imem[7] = 32'h00000021;
    #3 rst = 0;
    wait (halted);
    #10;
    fail = 0;
    if (n != 5) begin fail = 1; $display("FAIL: text_interp_int_left — got %0d capture entries, want 5", n); end
    if (!fail && (skind[0] !== 2'd2 || sval[0] !== 32'h00000004)) begin fail = 1; $display("FAIL: text_interp_int_left — entry 0 was kind=%0d val=%0d, want kind=2 val=%0d", skind[0], sval[0], 32'h00000004); end
    if (!fail && (skind[1] !== 2'd1 || sval[1] !== 32'h00000031)) begin fail = 1; $display("FAIL: text_interp_int_left — entry 1 was kind=%0d val=%0d, want kind=1 val=%0d", skind[1], sval[1], 32'h00000031); end
    if (!fail && (skind[2] !== 2'd1 || sval[2] !== 32'h00000032)) begin fail = 1; $display("FAIL: text_interp_int_left — entry 2 was kind=%0d val=%0d, want kind=1 val=%0d", skind[2], sval[2], 32'h00000032); end
    if (!fail && (skind[3] !== 2'd1 || sval[3] !== 32'h00000033)) begin fail = 1; $display("FAIL: text_interp_int_left — entry 3 was kind=%0d val=%0d, want kind=1 val=%0d", skind[3], sval[3], 32'h00000033); end
    if (!fail && (skind[4] !== 2'd1 || sval[4] !== 32'h00000021)) begin fail = 1; $display("FAIL: text_interp_int_left — entry 4 was kind=%0d val=%0d, want kind=1 val=%0d", skind[4], sval[4], 32'h00000021); end
    if (!fail) $display("PASS: text_interp_int_left");
    $finish;
  end
  initial begin #2000000000 $display("FAIL: text_interp_int_left timeout"); $finish; end
endmodule
