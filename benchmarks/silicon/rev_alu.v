module rev_alu(input clk, output reg [3:0] q);
  initial q = 0;
  always @(posedge clk) q <= q + 4'd1;
endmodule
