// `inner` claimed single-clock with a child clocked by a register it divides its clock into.
module xtop(input a_clk, input b_clk, input a_rst_n, input b_rst_n, input [7:0] a_in, output [7:0] b_out);
  reg [7:0] b_q;
  always @(posedge b_clk) b_q <= 8'd3;
  inner u(.clk(b_clk), .d(b_q), .q(b_out));
endmodule
module inner(input clk, input [7:0] d, output [7:0] q);
  reg half = 1'b0;
  always @(posedge clk) half <= ~half;
  leaf l(.c(half), .x(d), .y(q));
endmodule
module leaf(input c, input [7:0] x, output reg [7:0] y);
  always @(posedge c) y <= x;
endmodule
