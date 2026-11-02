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
    dut.imem[0] = 32'h05010000;
    dut.imem[1] = 32'h05020007;
    dut.imem[2] = 32'h0A000102;
    dut.imem[3] = 32'h06030000;
    dut.imem[4] = 32'hD8050004;
    dut.imem[5] = 32'h0000000A;
    dut.imem[6] = 32'h40060300;
    dut.imem[7] = 32'h20040506;
    dut.imem[8] = 32'hD9000400;
    dut.imem[9] = 32'h2B000000;
    dut.imem[10] = 32'h0000006E;
    dut.imem[11] = 32'h00000020;
    dut.imem[12] = 32'h0000003D;
    dut.imem[13] = 32'h00000020;
    #3 rst = 0;
    wait (halted);
    #10;
    fail = 0;
    if (n != 7) begin fail = 1; $display("FAIL: text_interp_negative — got %0d capture entries, want 7", n); end
    if (!fail && (skind[0] !== 2'd2 || sval[0] !== 32'h00000006)) begin fail = 1; $display("FAIL: text_interp_negative — entry 0 was kind=%0d val=%0d, want kind=2 val=%0d", skind[0], sval[0], 32'h00000006); end
    if (!fail && (skind[1] !== 2'd1 || sval[1] !== 32'h0000006E)) begin fail = 1; $display("FAIL: text_interp_negative — entry 1 was kind=%0d val=%0d, want kind=1 val=%0d", skind[1], sval[1], 32'h0000006E); end
    if (!fail && (skind[2] !== 2'd1 || sval[2] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_interp_negative — entry 2 was kind=%0d val=%0d, want kind=1 val=%0d", skind[2], sval[2], 32'h00000020); end
    if (!fail && (skind[3] !== 2'd1 || sval[3] !== 32'h0000003D)) begin fail = 1; $display("FAIL: text_interp_negative — entry 3 was kind=%0d val=%0d, want kind=1 val=%0d", skind[3], sval[3], 32'h0000003D); end
    if (!fail && (skind[4] !== 2'd1 || sval[4] !== 32'h00000020)) begin fail = 1; $display("FAIL: text_interp_negative — entry 4 was kind=%0d val=%0d, want kind=1 val=%0d", skind[4], sval[4], 32'h00000020); end
    if (!fail && (skind[5] !== 2'd1 || sval[5] !== 32'h0000002D)) begin fail = 1; $display("FAIL: text_interp_negative — entry 5 was kind=%0d val=%0d, want kind=1 val=%0d", skind[5], sval[5], 32'h0000002D); end
    if (!fail && (skind[6] !== 2'd1 || sval[6] !== 32'h00000037)) begin fail = 1; $display("FAIL: text_interp_negative — entry 6 was kind=%0d val=%0d, want kind=1 val=%0d", skind[6], sval[6], 32'h00000037); end
    if (!fail) $display("PASS: text_interp_negative");
    $finish;
  end
  initial begin #2000000000 $display("FAIL: text_interp_negative timeout"); $finish; end
endmodule
