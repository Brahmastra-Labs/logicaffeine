module rev_alu32(input clk, input [31:0] delta, output reg [31:0] q);
  initial q = 0;
  always @(posedge clk) q <= q + delta;
endmodule
