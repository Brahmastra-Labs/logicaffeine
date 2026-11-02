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
    dut.imem[0] = 32'hD8010000;
    dut.imem[1] = 32'h0000001C;
    dut.imem[2] = 32'hED000102;
    dut.imem[3] = 32'h06020000;
    dut.imem[4] = 32'h3E030200;
    dut.imem[5] = 32'h59000300;
    dut.imem[6] = 32'hD8040002;
    dut.imem[7] = 32'h0000001C;
    dut.imem[8] = 32'hED030405;
    dut.imem[9] = 32'h06050300;
    dut.imem[10] = 32'h05070005;
    dut.imem[11] = 32'h05080003;
    dut.imem[12] = 32'hEB060708;
    dut.imem[13] = 32'h06090600;
    dut.imem[14] = 32'h3E0A0500;
    dut.imem[15] = 32'h59000A00;
    dut.imem[16] = 32'h050B0001;
    dut.imem[17] = 32'h3B0A050B;
    dut.imem[18] = 32'h59000A00;
    dut.imem[19] = 32'h050B0002;
    dut.imem[20] = 32'h3B0A050B;
    dut.imem[21] = 32'h59000A00;
    dut.imem[22] = 32'h050B0003;
    dut.imem[23] = 32'h3B0A090B;
    dut.imem[24] = 32'h59000A00;
    dut.imem[25] = 32'h3E0A0200;
    dut.imem[26] = 32'h59000A00;
    dut.imem[27] = 32'h2B000000;
    dut.imem[28] = 32'h00000068;
    dut.imem[29] = 32'h00000069;
    #3 rst = 0;
    wait (halted);
    #10;
    fail = 0;
    if (n != 6) begin fail = 1; $display("FAIL: tsb_text_bytes_edge — got %0d capture entries, want 6", n); end
    if (!fail && (skind[0] !== 2'd0 || sval[0] !== 32'h00000000)) begin fail = 1; $display("FAIL: tsb_text_bytes_edge — entry 0 was kind=%0d val=%0d, want kind=0 val=%0d", skind[0], sval[0], 32'h00000000); end
    if (!fail && (skind[1] !== 2'd0 || sval[1] !== 32'h00000002)) begin fail = 1; $display("FAIL: tsb_text_bytes_edge — entry 1 was kind=%0d val=%0d, want kind=0 val=%0d", skind[1], sval[1], 32'h00000002); end
    if (!fail && (skind[2] !== 2'd0 || sval[2] !== 32'h00000068)) begin fail = 1; $display("FAIL: tsb_text_bytes_edge — entry 2 was kind=%0d val=%0d, want kind=0 val=%0d", skind[2], sval[2], 32'h00000068); end
    if (!fail && (skind[3] !== 2'd0 || sval[3] !== 32'h00000069)) begin fail = 1; $display("FAIL: tsb_text_bytes_edge — entry 3 was kind=%0d val=%0d, want kind=0 val=%0d", skind[3], sval[3], 32'h00000069); end
    if (!fail && (skind[4] !== 2'd0 || sval[4] !== 32'h00000005)) begin fail = 1; $display("FAIL: tsb_text_bytes_edge — entry 4 was kind=%0d val=%0d, want kind=0 val=%0d", skind[4], sval[4], 32'h00000005); end
    if (!fail && (skind[5] !== 2'd0 || sval[5] !== 32'h00000000)) begin fail = 1; $display("FAIL: tsb_text_bytes_edge — entry 5 was kind=%0d val=%0d, want kind=0 val=%0d", skind[5], sval[5], 32'h00000000); end
    if (!fail) $display("PASS: tsb_text_bytes_edge");
    $finish;
  end
  initial begin #2000000000 $display("FAIL: tsb_text_bytes_edge timeout"); $finish; end
endmodule
