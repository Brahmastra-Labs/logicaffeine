// tb_decision_hft — the emitted decision circuit vs the fabric oracle (== the running VM,
// proven by hft_decision_fabric_lock). 256 market states.
`timescale 1ns/1ps
module tb_decision_hft;
  reg  [63:0] mem [0:2047];
  reg  [63:0] exp [0:255];
  reg  [63:0] iw [0:7];
  wire [63:0] result;
  integer i, j, errors = 0;
  decision_hft dut(.in0(iw[0]), .in1(iw[1]), .in2(iw[2]), .in3(iw[3]), .in4(iw[4]), .in5(iw[5]), .in6(iw[6]), .in7(iw[7]), .result(result));
  initial begin
    $readmemh("decision_in.hex", mem);
    $readmemh("decision_exp.hex", exp);
    for (i = 0; i < 256; i = i + 1) begin
      for (j = 0; j < 8; j = j + 1) iw[j] = mem[i*8+j];
      #1;
      if (result !== exp[i]) begin errors = errors + 1; $display("FAIL vec %0d: got %0d exp %0d", i, $signed(result), $signed(exp[i])); end
    end
    if (errors == 0) $display("PASS: 256 decision evaluations match (RTL == fabric == VM)");
    else $display("FAIL: %0d mismatches", errors);
    $finish;
  end
endmodule
