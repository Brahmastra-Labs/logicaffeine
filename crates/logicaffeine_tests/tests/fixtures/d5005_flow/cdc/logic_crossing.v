// An output of one clock computed from a register of the other, through logic.
module xtop(input a_clk, input b_clk, input a_rst_n, input b_rst_n, input [7:0] a_in, output [7:0] b_out);
  reg [7:0] a_q, b_q;
  always @(posedge a_clk or negedge a_rst_n) if (!a_rst_n) a_q <= 0; else a_q <= a_in;
  always @(posedge b_clk or negedge b_rst_n) if (!b_rst_n) b_q <= 0; else b_q <= b_q + 1'b1;
  assign b_out = a_q ^ b_q;
endmodule
