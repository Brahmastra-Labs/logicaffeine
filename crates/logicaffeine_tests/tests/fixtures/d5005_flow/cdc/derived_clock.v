// A register clocked by logic of a clock rather than by a clock port.
module xtop(input a_clk, input b_clk, input a_rst_n, input b_rst_n, input [7:0] a_in, output reg [7:0] b_out);
  reg gate;
  always @(posedge b_clk or negedge b_rst_n) if (!b_rst_n) gate <= 0; else gate <= ~gate;
  wire slow = b_clk & gate;
  always @(posedge slow or negedge b_rst_n) if (!b_rst_n) b_out <= 0; else b_out <= b_out + 1'b1;
endmodule
