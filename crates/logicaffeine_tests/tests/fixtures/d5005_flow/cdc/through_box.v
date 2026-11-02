// A value from one clock to the other through a dual-clock FIFO and on through `inner`, a module of two levels all on
// the read clock: clean with `inner` a single-clock box.
module xtop(input a_clk, input b_clk, input a_rst_n, input b_rst_n, input [7:0] a_in, output [7:0] b_out);
  reg [7:0] a_q;
  wire rvalid, wready;
  wire [7:0] rdata;
  wire [4:0] wfree, rused;
  always @(posedge a_clk or negedge a_rst_n) if (!a_rst_n) a_q <= 0; else a_q <= a_in;
  logos_dcfifo_mlab #(.W(8), .AW(4)) f(.wclk(a_clk), .wce(1'b1), .wrst_n(a_rst_n), .wvalid(1'b1), .wready(wready), .wdata(a_q),
    .wfree(wfree), .rclk(b_clk), .rce(1'b1), .rrst_n(b_rst_n), .rvalid(rvalid), .rready(1'b1), .rdata(rdata), .rused(rused));
  inner u(.clk(b_clk), .valid(rvalid), .d(rdata), .q(b_out));
endmodule
module inner #(parameter W = 8) (input clk, input valid, input [W-1:0] d, output [W-1:0] q);
  reg [W-1:0] held;
  always @(posedge clk) if (valid) held <= d;
  leaf l(.c(clk), .x(held), .y(q));
endmodule
module leaf(input c, input [7:0] x, output reg [7:0] y);
  always @(negedge c) y <= x + 8'd1;
endmodule
