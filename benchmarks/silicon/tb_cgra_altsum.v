// tb_cgra_altsum — the CGRA (clocked body datapath) vs the RUNNING VM over the loop program.
`timescale 1ns/1ps
module tb_cgra_altsum;
  reg clk = 0, rst;
  reg  [63:0] in0;
  reg  [63:0] oin [0:47];
  reg  [63:0] oout [0:47];
  wire [63:0] result;
  wire done;
  integer i, c, errors = 0;
  cgra_altsum dut(.clk(clk), .rst(rst), .in0(in0), .result(result), .done(done));
  always #5 clk = ~clk;
  initial begin
    $readmemh("cgra_altsum_in.hex", oin);
    $readmemh("cgra_altsum_out.hex", oout);
    for (i = 0; i < 48; i = i + 1) begin
      @(negedge clk); in0 = oin[i*1+0]; rst = 1;
      @(negedge clk); rst = 0;
      c = 0;
      while (!done && c < 256) begin @(negedge clk); c = c + 1; end
      if (result !== oout[i]) begin errors = errors + 1; $display("FAIL case %0d: got %0d exp %0d", i, result, oout[i]); end
    end
    if (errors == 0) $display("PASS: 48 CGRA loop runs match the VM (body = circuit, loop = counter)");
    else $display("FAIL: %0d CGRA mismatches", errors);
    $finish;
  end
endmodule
