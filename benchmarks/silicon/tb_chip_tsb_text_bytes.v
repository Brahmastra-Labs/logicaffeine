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
    dut.imem[0] = 32'hD8010004;
    dut.imem[1] = 32'h0000000C;
    dut.imem[2] = 32'hED000102;
    dut.imem[3] = 32'h06020000;
    dut.imem[4] = 32'h3E030200;
    dut.imem[5] = 32'h59000300;
    dut.imem[6] = 32'h55000200;
    dut.imem[7] = 32'h56030A00;
    dut.imem[8] = 32'h59000300;
    dut.imem[9] = 32'h22070000;
    dut.imem[10] = 32'h57000000;
    dut.imem[11] = 32'h2B000000;
    dut.imem[12] = 32'h00000061;
    dut.imem[13] = 32'h000000E9;
    dut.imem[14] = 32'h000020AC;
    dut.imem[15] = 32'h0001F600;
    #3 rst = 0;
    wait (halted);
    #10;
    fail = 0;
    if (n != 11) begin fail = 1; $display("FAIL: tsb_text_bytes — got %0d capture entries, want 11", n); end
    if (!fail && (skind[0] !== 2'd0 || sval[0] !== 32'h0000000A)) begin fail = 1; $display("FAIL: tsb_text_bytes — entry 0 was kind=%0d val=%0d, want kind=0 val=%0d", skind[0], sval[0], 32'h0000000A); end
    if (!fail && (skind[1] !== 2'd0 || sval[1] !== 32'h00000061)) begin fail = 1; $display("FAIL: tsb_text_bytes — entry 1 was kind=%0d val=%0d, want kind=0 val=%0d", skind[1], sval[1], 32'h00000061); end
    if (!fail && (skind[2] !== 2'd0 || sval[2] !== 32'h000000C3)) begin fail = 1; $display("FAIL: tsb_text_bytes — entry 2 was kind=%0d val=%0d, want kind=0 val=%0d", skind[2], sval[2], 32'h000000C3); end
    if (!fail && (skind[3] !== 2'd0 || sval[3] !== 32'h000000A9)) begin fail = 1; $display("FAIL: tsb_text_bytes — entry 3 was kind=%0d val=%0d, want kind=0 val=%0d", skind[3], sval[3], 32'h000000A9); end
    if (!fail && (skind[4] !== 2'd0 || sval[4] !== 32'h000000E2)) begin fail = 1; $display("FAIL: tsb_text_bytes — entry 4 was kind=%0d val=%0d, want kind=0 val=%0d", skind[4], sval[4], 32'h000000E2); end
    if (!fail && (skind[5] !== 2'd0 || sval[5] !== 32'h00000082)) begin fail = 1; $display("FAIL: tsb_text_bytes — entry 5 was kind=%0d val=%0d, want kind=0 val=%0d", skind[5], sval[5], 32'h00000082); end
    if (!fail && (skind[6] !== 2'd0 || sval[6] !== 32'h000000AC)) begin fail = 1; $display("FAIL: tsb_text_bytes — entry 6 was kind=%0d val=%0d, want kind=0 val=%0d", skind[6], sval[6], 32'h000000AC); end
    if (!fail && (skind[7] !== 2'd0 || sval[7] !== 32'h000000F0)) begin fail = 1; $display("FAIL: tsb_text_bytes — entry 7 was kind=%0d val=%0d, want kind=0 val=%0d", skind[7], sval[7], 32'h000000F0); end
    if (!fail && (skind[8] !== 2'd0 || sval[8] !== 32'h0000009F)) begin fail = 1; $display("FAIL: tsb_text_bytes — entry 8 was kind=%0d val=%0d, want kind=0 val=%0d", skind[8], sval[8], 32'h0000009F); end
    if (!fail && (skind[9] !== 2'd0 || sval[9] !== 32'h00000098)) begin fail = 1; $display("FAIL: tsb_text_bytes — entry 9 was kind=%0d val=%0d, want kind=0 val=%0d", skind[9], sval[9], 32'h00000098); end
    if (!fail && (skind[10] !== 2'd0 || sval[10] !== 32'h00000080)) begin fail = 1; $display("FAIL: tsb_text_bytes — entry 10 was kind=%0d val=%0d, want kind=0 val=%0d", skind[10], sval[10], 32'h00000080); end
    if (!fail) $display("PASS: tsb_text_bytes");
    $finish;
  end
  initial begin #2000000000 $display("FAIL: tsb_text_bytes timeout"); $finish; end
endmodule
