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
    dut.imem[0] = 32'hD800000B;
    dut.imem[1] = 32'h00000016;
    dut.imem[2] = 32'h06010000;
    dut.imem[3] = 32'hD8030005;
    dut.imem[4] = 32'h00000021;
    dut.imem[5] = 32'hEA020103;
    dut.imem[6] = 32'h230A0200;
    dut.imem[7] = 32'h05040001;
    dut.imem[8] = 32'h59000400;
    dut.imem[9] = 32'h220C0000;
    dut.imem[10] = 32'h05040000;
    dut.imem[11] = 32'h59000400;
    dut.imem[12] = 32'hD8030003;
    dut.imem[13] = 32'h00000026;
    dut.imem[14] = 32'hEA020103;
    dut.imem[15] = 32'h23130200;
    dut.imem[16] = 32'h0504006F;
    dut.imem[17] = 32'h59000400;
    dut.imem[18] = 32'h22150000;
    dut.imem[19] = 32'h050400DE;
    dut.imem[20] = 32'h59000400;
    dut.imem[21] = 32'h2B000000;
    dut.imem[22] = 32'h00000068;
    dut.imem[23] = 32'h00000065;
    dut.imem[24] = 32'h0000006C;
    dut.imem[25] = 32'h0000006C;
    dut.imem[26] = 32'h0000006F;
    dut.imem[27] = 32'h00000020;
    dut.imem[28] = 32'h00000077;
    dut.imem[29] = 32'h0000006F;
    dut.imem[30] = 32'h00000072;
    dut.imem[31] = 32'h0000006C;
    dut.imem[32] = 32'h00000064;
    dut.imem[33] = 32'h00000077;
    dut.imem[34] = 32'h0000006F;
    dut.imem[35] = 32'h00000072;
    dut.imem[36] = 32'h0000006C;
    dut.imem[37] = 32'h00000064;
    dut.imem[38] = 32'h0000007A;
    dut.imem[39] = 32'h0000007A;
    dut.imem[40] = 32'h0000007A;
    #3 rst = 0;
    wait (halted);
    #10;
    fail = 0;
    if (n != 2) begin fail = 1; $display("FAIL: tsb_contains_op — got %0d capture entries, want 2", n); end
    if (!fail && (skind[0] !== 2'd0 || sval[0] !== 32'h00000001)) begin fail = 1; $display("FAIL: tsb_contains_op — entry 0 was kind=%0d val=%0d, want kind=0 val=%0d", skind[0], sval[0], 32'h00000001); end
    if (!fail && (skind[1] !== 2'd0 || sval[1] !== 32'h000000DE)) begin fail = 1; $display("FAIL: tsb_contains_op — entry 1 was kind=%0d val=%0d, want kind=0 val=%0d", skind[1], sval[1], 32'h000000DE); end
    if (!fail) $display("PASS: tsb_contains_op");
    $finish;
  end
  initial begin #2000000000 $display("FAIL: tsb_contains_op timeout"); $finish; end
endmodule
