// `inner` instantiated with a parameter its single-clock proof was not made at.
module xtop(input a_clk, input b_clk, input a_rst_n, input b_rst_n, input [7:0] a_in, output [7:0] b_out);
  reg [7:0] b_q;
  always @(posedge b_clk) b_q <= 8'd3;
  inner #(.W(8)) u(.clk(b_clk), .valid(1'b1), .d(b_q), .q(b_out));
endmodule
module inner #(parameter W = 4) (input clk, input valid, input [W-1:0] d, output [W-1:0] q);
  reg [W-1:0] held;
  always @(posedge clk) if (valid) held <= d;
  assign q = held;
endmodule
