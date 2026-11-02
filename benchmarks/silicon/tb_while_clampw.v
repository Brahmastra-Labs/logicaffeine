// tb_while_clampw — the while-CGRA (fabric-predicate controller) vs the RUNNING VM.
`timescale 1ns/1ps
module tb_while_clampw;
  reg clk = 0, rst;
  reg  [63:0] in0;
  reg  [63:0] oin [0:47];
  reg  [63:0] oout [0:47];
  wire [63:0] result;
  wire done;
  integer i, c, errors = 0;
  while_clampw dut(.clk(clk), .rst(rst), .in0(in0), .result(result), .done(done));
  always #5 clk = ~clk;
  initial begin
    $readmemh("while_clampw_in.hex", oin);
    $readmemh("while_clampw_out.hex", oout);
    for (i = 0; i < 48; i = i + 1) begin
      @(negedge clk); in0 = oin[i*1+0]; rst = 1;
      @(negedge clk); rst = 0;
      c = 0;
      while (!done && c < 256) begin @(negedge clk); c = c + 1; end
      if (result !== oout[i]) begin errors = errors + 1; $display("FAIL case %0d: got %0d exp %0d", i, result, oout[i]); end
    end
    if (errors == 0) $display("PASS: 48 while-CGRA runs match the VM (fabric predicate = trip count)");
    else $display("FAIL: %0d while-CGRA mismatches", errors);
    $finish;
  end
endmodule
