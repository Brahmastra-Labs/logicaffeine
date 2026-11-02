// tb_coproc_core — the issuing core vs the software oracle's retired register file.
`timescale 1ns/1ps
module tb_coproc_core;
  reg clk = 0, rst = 1;
  reg [2:0] rsel = 0;
  wire [255:0] rout;
  wire halted;
  reg [255:0] exp [0:7];
  integer errors = 0, i;
  coproc_core dut(.clk(clk), .rst(rst), .rsel(rsel), .rout(rout), .halted(halted));
  always #5 clk = ~clk;
  initial begin
    $readmemh("coproc_exp.hex", exp);
    #12 rst = 0;
    for (i = 0; i < 64 && !halted; i = i + 1) @(posedge clk);
    if (!halted) begin $display("FAIL: core never halted"); $finish; end
    for (i = 2; i <= 5; i = i + 1) begin
      rsel = i[2:0]; #1;
      if (rout !== exp[i]) begin errors = errors + 1; $display("FAIL R%0d", i); end
    end
    if (errors == 0)
      $display("PASS: coproc_core issued BADD/BSUB/BMUL/CMERGE and retired results matching the oracle");
    else
      $display("FAIL: %0d retired-register mismatches", errors);
    $finish;
  end
endmodule
