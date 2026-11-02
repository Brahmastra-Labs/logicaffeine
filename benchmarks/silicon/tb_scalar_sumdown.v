`timescale 1ns/1ps
// AUTO-GENERATED from the run_soft_core ≡VM differential (codegen_sva/rtl.rs). Do not edit by hand.
// Regenerate: LOGOS_EMIT_SC_TB=1 cargo test -p logicaffeine-compile --lib the_soft_core_runs_logos_programs_identically_to_the_vm
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core_scalar dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  integer n = 0; reg [31:0] seen [0:31];
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin seen[n] = out; n = n + 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h05000003;
    dut.imem[1] = 32'h25040500;
    dut.imem[2] = 32'h59000500;
    dut.imem[3] = 32'h2B000000;
    dut.imem[4] = 32'h05060000;
    dut.imem[5] = 32'h1B010006;
    dut.imem[6] = 32'h23090100;
    dut.imem[7] = 32'h05020000;
    dut.imem[8] = 32'h29000200;
    dut.imem[9] = 32'h06040000;
    dut.imem[10] = 32'h05070001;
    dut.imem[11] = 32'h0A000007;
    dut.imem[12] = 32'h25040300;
    dut.imem[13] = 32'h08050403;
    dut.imem[14] = 32'h29000500;
    #3 rst = 0;
    wait (halted);
    #10;
    if (n == 1 && seen[0] == 32'h00000006)
      $display("PASS: sumdown");
    else begin
      $display("FAIL: sumdown (got n=%0d)", n);
      $display("  seen[0] = %0d", $signed(seen[0]));
    end
    $finish;
  end
  initial begin #200000 $display("FAIL: sumdown timeout"); $finish; end
endmodule
