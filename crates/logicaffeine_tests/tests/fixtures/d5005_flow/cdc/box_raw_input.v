// `inner`, single-clock on the read clock, fed straight from a register of the write clock.
module xtop(input a_clk, input b_clk, input a_rst_n, input b_rst_n, input [7:0] a_in, output [7:0] b_out);
  reg [7:0] a_q;
  always @(posedge a_clk or negedge a_rst_n) if (!a_rst_n) a_q <= 0; else a_q <= a_in;
  inner u(.clk(b_clk), .valid(1'b1), .d(a_q), .q(b_out));
endmodule
module inner #(parameter W = 8) (input clk, input valid, input [W-1:0] d, output [W-1:0] q);
  reg [W-1:0] held;
  always @(posedge clk) if (valid) held <= d;
  assign q = held;
endmodule
