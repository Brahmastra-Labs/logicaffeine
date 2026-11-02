// tb_nested_nsum — the hierarchical (outer+inner counter) CGRA vs the RUNNING VM.
`timescale 1ns/1ps
module tb_nested_nsum;
  reg clk = 0, rst;
  reg  [63:0] in0;
  reg  [63:0] oin [0:23];
  reg  [63:0] oout [0:23];
  wire [63:0] result;
  wire done;
  integer i, c, errors = 0;
  nested_nsum dut(.clk(clk), .rst(rst), .in0(in0), .result(result), .done(done));
  always #5 clk = ~clk;
  initial begin
    $readmemh("nested_nsum_in.hex", oin);
    $readmemh("nested_nsum_out.hex", oout);
    for (i = 0; i < 24; i = i + 1) begin
      @(negedge clk); in0 = oin[i*1+0]; rst = 1;
      @(negedge clk); rst = 0;
      c = 0;
      while (!done && c < 4000) begin @(negedge clk); c = c + 1; end
      if (result !== oout[i]) begin errors = errors + 1; $display("FAIL case %0d: got %0d exp %0d", i, result, oout[i]); end
    end
    if (errors == 0) $display("PASS: 24 nested-CGRA runs match the VM (outer counter streams the inner loop)");
    else $display("FAIL: %0d nested-CGRA mismatches", errors);
    $finish;
  end
endmodule
