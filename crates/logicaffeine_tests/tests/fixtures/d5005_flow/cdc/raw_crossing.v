// A register in one clock sampling a register of the other directly.
module xtop(input a_clk, input b_clk, input a_rst_n, input b_rst_n, input [7:0] a_in, output reg [7:0] b_out);
  reg [7:0] a_q;
  always @(posedge a_clk or negedge a_rst_n) if (!a_rst_n) a_q <= 0; else a_q <= a_in;
  always @(posedge b_clk or negedge b_rst_n) if (!b_rst_n) b_out <= 0; else b_out <= a_q;
endmodule
