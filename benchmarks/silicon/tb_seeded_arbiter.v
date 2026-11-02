// tb_seeded_arbiter — HW SplitMix64 draws vs the software scheduler oracle.
`timescale 1ns/1ps
module tb_seeded_arbiter;
  reg clk = 0; always #5 clk = ~clk;
  reg rst = 1, load = 0, step = 0;
  reg [63:0] seed;
  reg [63:0] oracle [0:63];
  wire [63:0] draw; wire draw_valid;
  integer got = 0, errors = 0;
  seeded_arbiter_prng dut(.clk(clk), .rst(rst), .seed(seed), .load(load), .step(step),
                          .draw(draw), .draw_valid(draw_valid));
  always @(posedge clk) if (draw_valid) begin
    if (draw !== oracle[got]) begin errors = errors + 1; $display("FAIL draw %0d", got); end
    got = got + 1;
  end
  initial begin
    $readmemh("arbiter_seq.hex", oracle);
    seed = 64'h123456789abcdef0;
    rst = 1; repeat (3) @(posedge clk); #1 rst = 0;
    @(posedge clk); #1 load = 1;
    @(posedge clk); #1 load = 0;
    repeat (64) begin @(posedge clk); #1 step = 1; @(posedge clk); #1 step = 0; end
    repeat (4) @(posedge clk);
    if (got !== 64) $display("FAIL: got %0d of 64 draws", got);
    else if (errors == 0) $display("PASS: 64 HW SplitMix64 draws match the software scheduler bit-exact");
    $finish;
  end
endmodule
