// tb_tmux_linsq — ONE tile array, two fabrics: ctx=1 runs lin (3x+7), ctx=0 runs sq (x²+1).
`timescale 1ns/1ps
module tb_tmux_linsq;
  reg [63:0] in0; reg ctx;
  reg [63:0] oin [0:47]; reg [63:0] olin [0:47]; reg [63:0] osq [0:47];
  wire [63:0] result;
  integer i, errors = 0;
  tmux_linsq dut(.in0(in0), .ctx(ctx), .result(result));
  initial begin
    $readmemh("tmux_in.hex", oin); $readmemh("tmux_lin.hex", olin); $readmemh("tmux_sq.hex", osq);
    for (i = 0; i < 48; i = i + 1) begin
      in0 = oin[i]; ctx = 1'b1; #1;
      if (result !== olin[i]) begin errors = errors + 1; $display("FAIL lin %0d", i); end
      ctx = 1'b0; #1;
      if (result !== osq[i]) begin errors = errors + 1; $display("FAIL sq %0d", i); end
    end
    if (errors == 0) $display("PASS: 48 time-mux runs match (one array runs two fabrics by ctx)");
    else $display("FAIL: %0d tmux mismatches", errors);
    $finish;
  end
endmodule
