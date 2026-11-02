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
    dut.imem[0] = 32'h0501007F;
    dut.imem[1] = 32'h05020080;
    dut.imem[2] = 32'h050307FF;
    dut.imem[3] = 32'h05040800;
    dut.imem[4] = 32'hC8050000;
    dut.imem[5] = 32'h0000D7FF;
    dut.imem[6] = 32'hC8060000;
    dut.imem[7] = 32'h0000E000;
    dut.imem[8] = 32'hC8070000;
    dut.imem[9] = 32'h0000FFFF;
    dut.imem[10] = 32'hC8080000;
    dut.imem[11] = 32'h00010000;
    dut.imem[12] = 32'hC8090000;
    dut.imem[13] = 32'h000E0041;
    dut.imem[14] = 32'hC80A0000;
    dut.imem[15] = 32'h0010FFFF;
    dut.imem[16] = 32'h3000010A;
    dut.imem[17] = 32'h060B0000;
    dut.imem[18] = 32'h3E0C0B00;
    dut.imem[19] = 32'h050D0001;
    dut.imem[20] = 32'h050E0001;
    dut.imem[21] = 32'h1B100D0C;
    dut.imem[22] = 32'h23251000;
    dut.imem[23] = 32'h3C0F0B0D;
    dut.imem[24] = 32'h06130F00;
    dut.imem[25] = 32'hE9121314;
    dut.imem[26] = 32'hED111213;
    dut.imem[27] = 32'h06141100;
    dut.imem[28] = 32'h3E151400;
    dut.imem[29] = 32'h59001500;
    dut.imem[30] = 32'h55001400;
    dut.imem[31] = 32'h56152200;
    dut.imem[32] = 32'h59001500;
    dut.imem[33] = 32'h221F0000;
    dut.imem[34] = 32'h57000000;
    dut.imem[35] = 32'h080D0D0E;
    dut.imem[36] = 32'h22150000;
    dut.imem[37] = 32'h2B000000;
    #3 rst = 0;
    wait (halted);
    #10;
    fail = 0;
    if (n != 39) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — got %0d capture entries, want 39", n); end
    if (!fail && (skind[0] !== 2'd0 || sval[0] !== 32'h00000001)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 0 was kind=%0d val=%0d, want kind=0 val=%0d", skind[0], sval[0], 32'h00000001); end
    if (!fail && (skind[1] !== 2'd0 || sval[1] !== 32'h0000007F)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 1 was kind=%0d val=%0d, want kind=0 val=%0d", skind[1], sval[1], 32'h0000007F); end
    if (!fail && (skind[2] !== 2'd0 || sval[2] !== 32'h00000002)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 2 was kind=%0d val=%0d, want kind=0 val=%0d", skind[2], sval[2], 32'h00000002); end
    if (!fail && (skind[3] !== 2'd0 || sval[3] !== 32'h000000C2)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 3 was kind=%0d val=%0d, want kind=0 val=%0d", skind[3], sval[3], 32'h000000C2); end
    if (!fail && (skind[4] !== 2'd0 || sval[4] !== 32'h00000080)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 4 was kind=%0d val=%0d, want kind=0 val=%0d", skind[4], sval[4], 32'h00000080); end
    if (!fail && (skind[5] !== 2'd0 || sval[5] !== 32'h00000002)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 5 was kind=%0d val=%0d, want kind=0 val=%0d", skind[5], sval[5], 32'h00000002); end
    if (!fail && (skind[6] !== 2'd0 || sval[6] !== 32'h000000DF)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 6 was kind=%0d val=%0d, want kind=0 val=%0d", skind[6], sval[6], 32'h000000DF); end
    if (!fail && (skind[7] !== 2'd0 || sval[7] !== 32'h000000BF)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 7 was kind=%0d val=%0d, want kind=0 val=%0d", skind[7], sval[7], 32'h000000BF); end
    if (!fail && (skind[8] !== 2'd0 || sval[8] !== 32'h00000003)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 8 was kind=%0d val=%0d, want kind=0 val=%0d", skind[8], sval[8], 32'h00000003); end
    if (!fail && (skind[9] !== 2'd0 || sval[9] !== 32'h000000E0)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 9 was kind=%0d val=%0d, want kind=0 val=%0d", skind[9], sval[9], 32'h000000E0); end
    if (!fail && (skind[10] !== 2'd0 || sval[10] !== 32'h000000A0)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 10 was kind=%0d val=%0d, want kind=0 val=%0d", skind[10], sval[10], 32'h000000A0); end
    if (!fail && (skind[11] !== 2'd0 || sval[11] !== 32'h00000080)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 11 was kind=%0d val=%0d, want kind=0 val=%0d", skind[11], sval[11], 32'h00000080); end
    if (!fail && (skind[12] !== 2'd0 || sval[12] !== 32'h00000003)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 12 was kind=%0d val=%0d, want kind=0 val=%0d", skind[12], sval[12], 32'h00000003); end
    if (!fail && (skind[13] !== 2'd0 || sval[13] !== 32'h000000ED)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 13 was kind=%0d val=%0d, want kind=0 val=%0d", skind[13], sval[13], 32'h000000ED); end
    if (!fail && (skind[14] !== 2'd0 || sval[14] !== 32'h0000009F)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 14 was kind=%0d val=%0d, want kind=0 val=%0d", skind[14], sval[14], 32'h0000009F); end
    if (!fail && (skind[15] !== 2'd0 || sval[15] !== 32'h000000BF)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 15 was kind=%0d val=%0d, want kind=0 val=%0d", skind[15], sval[15], 32'h000000BF); end
    if (!fail && (skind[16] !== 2'd0 || sval[16] !== 32'h00000003)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 16 was kind=%0d val=%0d, want kind=0 val=%0d", skind[16], sval[16], 32'h00000003); end
    if (!fail && (skind[17] !== 2'd0 || sval[17] !== 32'h000000EE)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 17 was kind=%0d val=%0d, want kind=0 val=%0d", skind[17], sval[17], 32'h000000EE); end
    if (!fail && (skind[18] !== 2'd0 || sval[18] !== 32'h00000080)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 18 was kind=%0d val=%0d, want kind=0 val=%0d", skind[18], sval[18], 32'h00000080); end
    if (!fail && (skind[19] !== 2'd0 || sval[19] !== 32'h00000080)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 19 was kind=%0d val=%0d, want kind=0 val=%0d", skind[19], sval[19], 32'h00000080); end
    if (!fail && (skind[20] !== 2'd0 || sval[20] !== 32'h00000003)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 20 was kind=%0d val=%0d, want kind=0 val=%0d", skind[20], sval[20], 32'h00000003); end
    if (!fail && (skind[21] !== 2'd0 || sval[21] !== 32'h000000EF)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 21 was kind=%0d val=%0d, want kind=0 val=%0d", skind[21], sval[21], 32'h000000EF); end
    if (!fail && (skind[22] !== 2'd0 || sval[22] !== 32'h000000BF)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 22 was kind=%0d val=%0d, want kind=0 val=%0d", skind[22], sval[22], 32'h000000BF); end
    if (!fail && (skind[23] !== 2'd0 || sval[23] !== 32'h000000BF)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 23 was kind=%0d val=%0d, want kind=0 val=%0d", skind[23], sval[23], 32'h000000BF); end
    if (!fail && (skind[24] !== 2'd0 || sval[24] !== 32'h00000004)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 24 was kind=%0d val=%0d, want kind=0 val=%0d", skind[24], sval[24], 32'h00000004); end
    if (!fail && (skind[25] !== 2'd0 || sval[25] !== 32'h000000F0)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 25 was kind=%0d val=%0d, want kind=0 val=%0d", skind[25], sval[25], 32'h000000F0); end
    if (!fail && (skind[26] !== 2'd0 || sval[26] !== 32'h00000090)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 26 was kind=%0d val=%0d, want kind=0 val=%0d", skind[26], sval[26], 32'h00000090); end
    if (!fail && (skind[27] !== 2'd0 || sval[27] !== 32'h00000080)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 27 was kind=%0d val=%0d, want kind=0 val=%0d", skind[27], sval[27], 32'h00000080); end
    if (!fail && (skind[28] !== 2'd0 || sval[28] !== 32'h00000080)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 28 was kind=%0d val=%0d, want kind=0 val=%0d", skind[28], sval[28], 32'h00000080); end
    if (!fail && (skind[29] !== 2'd0 || sval[29] !== 32'h00000004)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 29 was kind=%0d val=%0d, want kind=0 val=%0d", skind[29], sval[29], 32'h00000004); end
    if (!fail && (skind[30] !== 2'd0 || sval[30] !== 32'h000000F3)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 30 was kind=%0d val=%0d, want kind=0 val=%0d", skind[30], sval[30], 32'h000000F3); end
    if (!fail && (skind[31] !== 2'd0 || sval[31] !== 32'h000000A0)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 31 was kind=%0d val=%0d, want kind=0 val=%0d", skind[31], sval[31], 32'h000000A0); end
    if (!fail && (skind[32] !== 2'd0 || sval[32] !== 32'h00000081)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 32 was kind=%0d val=%0d, want kind=0 val=%0d", skind[32], sval[32], 32'h00000081); end
    if (!fail && (skind[33] !== 2'd0 || sval[33] !== 32'h00000081)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 33 was kind=%0d val=%0d, want kind=0 val=%0d", skind[33], sval[33], 32'h00000081); end
    if (!fail && (skind[34] !== 2'd0 || sval[34] !== 32'h00000004)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 34 was kind=%0d val=%0d, want kind=0 val=%0d", skind[34], sval[34], 32'h00000004); end
    if (!fail && (skind[35] !== 2'd0 || sval[35] !== 32'h000000F4)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 35 was kind=%0d val=%0d, want kind=0 val=%0d", skind[35], sval[35], 32'h000000F4); end
    if (!fail && (skind[36] !== 2'd0 || sval[36] !== 32'h0000008F)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 36 was kind=%0d val=%0d, want kind=0 val=%0d", skind[36], sval[36], 32'h0000008F); end
    if (!fail && (skind[37] !== 2'd0 || sval[37] !== 32'h000000BF)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 37 was kind=%0d val=%0d, want kind=0 val=%0d", skind[37], sval[37], 32'h000000BF); end
    if (!fail && (skind[38] !== 2'd0 || sval[38] !== 32'h000000BF)) begin fail = 1; $display("FAIL: tsb_text_bytes_boundaries — entry 38 was kind=%0d val=%0d, want kind=0 val=%0d", skind[38], sval[38], 32'h000000BF); end
    if (!fail) $display("PASS: tsb_text_bytes_boundaries");
    $finish;
  end
  initial begin #2000000000 $display("FAIL: tsb_text_bytes_boundaries timeout"); $finish; end
endmodule
