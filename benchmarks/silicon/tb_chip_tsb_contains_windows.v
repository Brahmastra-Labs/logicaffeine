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
    dut.imem[0] = 32'hD8000006;
    dut.imem[1] = 32'h00000028;
    dut.imem[2] = 32'h06010000;
    dut.imem[3] = 32'hD8030003;
    dut.imem[4] = 32'h0000002E;
    dut.imem[5] = 32'hEA020103;
    dut.imem[6] = 32'h230A0200;
    dut.imem[7] = 32'h05040001;
    dut.imem[8] = 32'h59000400;
    dut.imem[9] = 32'h220C0000;
    dut.imem[10] = 32'h05040000;
    dut.imem[11] = 32'h59000400;
    dut.imem[12] = 32'hD8030003;
    dut.imem[13] = 32'h00000031;
    dut.imem[14] = 32'hEA020103;
    dut.imem[15] = 32'h23130200;
    dut.imem[16] = 32'h05040002;
    dut.imem[17] = 32'h59000400;
    dut.imem[18] = 32'h22150000;
    dut.imem[19] = 32'h05040000;
    dut.imem[20] = 32'h59000400;
    dut.imem[21] = 32'hD8030006;
    dut.imem[22] = 32'h00000028;
    dut.imem[23] = 32'hEA020103;
    dut.imem[24] = 32'h231C0200;
    dut.imem[25] = 32'h05040003;
    dut.imem[26] = 32'h59000400;
    dut.imem[27] = 32'h221E0000;
    dut.imem[28] = 32'h05040000;
    dut.imem[29] = 32'h59000400;
    dut.imem[30] = 32'hD8030006;
    dut.imem[31] = 32'h00000034;
    dut.imem[32] = 32'hEA020103;
    dut.imem[33] = 32'h23250200;
    dut.imem[34] = 32'h05040009;
    dut.imem[35] = 32'h59000400;
    dut.imem[36] = 32'h22270000;
    dut.imem[37] = 32'h05040004;
    dut.imem[38] = 32'h59000400;
    dut.imem[39] = 32'h2B000000;
    dut.imem[40] = 32'h00000061;
    dut.imem[41] = 32'h00000062;
    dut.imem[42] = 32'h00000063;
    dut.imem[43] = 32'h00000064;
    dut.imem[44] = 32'h00000065;
    dut.imem[45] = 32'h00000066;
    dut.imem[46] = 32'h00000061;
    dut.imem[47] = 32'h00000062;
    dut.imem[48] = 32'h00000063;
    dut.imem[49] = 32'h00000064;
    dut.imem[50] = 32'h00000065;
    dut.imem[51] = 32'h00000066;
    dut.imem[52] = 32'h00000061;
    dut.imem[53] = 32'h00000062;
    dut.imem[54] = 32'h00000063;
    dut.imem[55] = 32'h00000064;
    dut.imem[56] = 32'h00000065;
    dut.imem[57] = 32'h00000058;
    #3 rst = 0;
    wait (halted);
    #10;
    fail = 0;
    if (n != 4) begin fail = 1; $display("FAIL: tsb_contains_windows — got %0d capture entries, want 4", n); end
    if (!fail && (skind[0] !== 2'd0 || sval[0] !== 32'h00000001)) begin fail = 1; $display("FAIL: tsb_contains_windows — entry 0 was kind=%0d val=%0d, want kind=0 val=%0d", skind[0], sval[0], 32'h00000001); end
    if (!fail && (skind[1] !== 2'd0 || sval[1] !== 32'h00000002)) begin fail = 1; $display("FAIL: tsb_contains_windows — entry 1 was kind=%0d val=%0d, want kind=0 val=%0d", skind[1], sval[1], 32'h00000002); end
    if (!fail && (skind[2] !== 2'd0 || sval[2] !== 32'h00000003)) begin fail = 1; $display("FAIL: tsb_contains_windows — entry 2 was kind=%0d val=%0d, want kind=0 val=%0d", skind[2], sval[2], 32'h00000003); end
    if (!fail && (skind[3] !== 2'd0 || sval[3] !== 32'h00000004)) begin fail = 1; $display("FAIL: tsb_contains_windows — entry 3 was kind=%0d val=%0d, want kind=0 val=%0d", skind[3], sval[3], 32'h00000004); end
    if (!fail) $display("PASS: tsb_contains_windows");
    $finish;
  end
  initial begin #2000000000 $display("FAIL: tsb_contains_windows timeout"); $finish; end
endmodule
