`timescale 1ns/1ps
// AUTO-GENERATED from the run_soft_core ≡VM differential (codegen_sva/rtl.rs). Do not edit by hand.
// Regenerate: LOGOS_EMIT_SC_TB=1 cargo test -p logicaffeine-compile --lib the_soft_core_runs_logos_programs_identically_to_the_vm
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  integer n = 0; reg [31:0] seen [0:31];
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin seen[n] = out; n = n + 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h0500000A;
    dut.imem[1] = 32'h06010000;
    dut.imem[2] = 32'h31020000;
    dut.imem[3] = 32'h0503002A;
    dut.imem[4] = 32'h06040300;
    dut.imem[5] = 32'h05050000;
    dut.imem[6] = 32'h06060500;
    dut.imem[7] = 32'h19070601;
    dut.imem[8] = 32'h231A0700;
    dut.imem[9] = 32'h050B6255;
    dut.imem[10] = 32'h0B0A040B;
    dut.imem[11] = 32'h050C3619;
    dut.imem[12] = 32'h08090A0C;
    dut.imem[13] = 32'hC80D0000;
    dut.imem[14] = 32'h00010000;
    dut.imem[15] = 32'h0F08090D;
    dut.imem[16] = 32'h06040800;
    dut.imem[17] = 32'h05100002;
    dut.imem[18] = 32'h0C0F0410;
    dut.imem[19] = 32'hC8110000;
    dut.imem[20] = 32'h00008000;
    dut.imem[21] = 32'h0F0E0F11;
    dut.imem[22] = 32'h36020E00;
    dut.imem[23] = 32'h05120001;
    dut.imem[24] = 32'h08060612;
    dut.imem[25] = 32'h22070000;
    dut.imem[26] = 32'h05080000;
    dut.imem[27] = 32'h06060800;
    dut.imem[28] = 32'h050B0001;
    dut.imem[29] = 32'h0A0A010B;
    dut.imem[30] = 32'h1909060A;
    dut.imem[31] = 32'h23390900;
    dut.imem[32] = 32'h050C0001;
    dut.imem[33] = 32'h060D0C00;
    dut.imem[34] = 32'h05110001;
    dut.imem[35] = 32'h0A100111;
    dut.imem[36] = 32'h0A0F1006;
    dut.imem[37] = 32'h1B0E0D0F;
    dut.imem[38] = 32'h23360E00;
    dut.imem[39] = 32'h3C12020D;
    dut.imem[40] = 32'h06131200;
    dut.imem[41] = 32'h05160001;
    dut.imem[42] = 32'h08150D16;
    dut.imem[43] = 32'h3C140215;
    dut.imem[44] = 32'h06171400;
    dut.imem[45] = 32'h1A181317;
    dut.imem[46] = 32'h23331800;
    dut.imem[47] = 32'h3A020D17;
    dut.imem[48] = 32'h051A0001;
    dut.imem[49] = 32'h08190D1A;
    dut.imem[50] = 32'h3A021913;
    dut.imem[51] = 32'h05190001;
    dut.imem[52] = 32'h080D0D19;
    dut.imem[53] = 32'h22220000;
    dut.imem[54] = 32'h05120001;
    dut.imem[55] = 32'h08060612;
    dut.imem[56] = 32'h221C0000;
    dut.imem[57] = 32'h050D0001;
    dut.imem[58] = 32'h3B0C020D;
    dut.imem[59] = 32'h59000C00;
    dut.imem[60] = 32'h2B000000;
    #3 rst = 0;
    wait (halted);
    #10;
    if (n == 1 && seen[0] == 32'h00000C13)
      $display("PASS: full_bubble");
    else begin
      $display("FAIL: full_bubble (got n=%0d)", n);
      $display("  seen[0] = %0d", $signed(seen[0]));
    end
    $finish;
  end
  initial begin #1000000000 $display("FAIL: full_bubble timeout"); $finish; end
endmodule
