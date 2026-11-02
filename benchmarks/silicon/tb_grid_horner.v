// tb_grid_horner — the placed+routed CGRA tile array vs the RUNNING VM's output.
`timescale 1ns/1ps
module tb_grid_horner;
  reg  [63:0] in0;
  reg  [63:0] oin [0:47];
  reg  [63:0] oout [0:47];
  wire [63:0] result;
  integer i, errors = 0;
  grid_horner dut(.in0(in0), .result(result));
  initial begin
    $readmemh("grid_horner_in.hex", oin);
    $readmemh("grid_horner_out.hex", oout);
    for (i = 0; i < 48; i = i + 1) begin
      in0 = oin[i*1+0]; #1;
      if (result !== oout[i]) begin errors = errors + 1; $display("FAIL case %0d: got %0d exp %0d", i, result, oout[i]); end
    end
    if (errors == 0) $display("PASS: 48 tile-array evaluations match the VM (placed+routed CGRA)");
    else $display("FAIL: %0d grid mismatches", errors);
    $finish;
  end
endmodule
