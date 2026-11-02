// tb_fabric_horner — the dataflow circuit vs the RUNNING VM's output for the program.
`timescale 1ns/1ps
module tb_fabric_horner;
  reg  [63:0] in0;
  reg  [63:0] oin [0:63];
  reg  [63:0] oout [0:63];
  wire [63:0] result;
  integer i, errors = 0;
  fabric_horner dut(.in0(in0), .result(result));
  initial begin
    $readmemh("fabric_in.hex", oin);
    $readmemh("fabric_out.hex", oout);
    for (i = 0; i < 64; i = i + 1) begin
      in0 = oin[i]; #1;
      if (result !== oout[i]) begin errors = errors + 1; $display("FAIL eval %0d", i); end
    end
    if (errors == 0) $display("PASS: 64 circuit evaluations match the VM (residual = circuit, no interpreter)");
    else $display("FAIL: %0d fabric mismatches", errors);
    $finish;
  end
endmodule
