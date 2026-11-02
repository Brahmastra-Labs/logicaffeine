`timescale 1ns/1ps
// AUTO-GENERATED from the scalar-encoder ≡VM differential (codegen_sva/rtl.rs). Do not edit by hand.
// Regenerate: LOGOS_EMIT_SC_TB=1 cargo test -p logicaffeine-compile --lib scalar_encoder_tbs_match_vm
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core_scalar dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  integer n = 0; reg [31:0] seen [0:31];
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin seen[n] = out; n = n + 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h05000005;
    dut.imem[1] = 32'h06010000;
    dut.imem[2] = 32'h05030005;
    dut.imem[3] = 32'h1C020103;
    dut.imem[4] = 32'h23080200;
    dut.imem[5] = 32'h05040001;
    dut.imem[6] = 32'h59000400;
    dut.imem[7] = 32'h220A0000;
    dut.imem[8] = 32'h05040000;
    dut.imem[9] = 32'h59000400;
    dut.imem[10] = 32'h2B000000;
    #3 rst = 0;
    wait (halted);
    #10;
    if (n == 1 && seen[0] == 32'h00000001)
      $display("PASS: cmp_gte");
    else begin
      $display("FAIL: cmp_gte (got n=%0d)", n);
      $display("  seen[0] = %0d", $signed(seen[0]));
    end
    $finish;
  end
  initial begin #200000 $display("FAIL: cmp_gte timeout"); $finish; end
endmodule
