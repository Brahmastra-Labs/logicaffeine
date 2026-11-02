// A value through a dual-clock FIFO and on through `inner`, whose register is reset asynchronously by an input of
// `inner` in the read clock's domain: clean with `inner` a single-clock box.
module xtop(input a_clk, input b_clk, input a_rst_n, input b_rst_n, input [7:0] a_in, output [7:0] b_out);
  reg [7:0] a_q;
  wire rvalid, wready;
  wire [7:0] rdata;
  wire [4:0] wfree, rused;
  always @(posedge a_clk or negedge a_rst_n) if (!a_rst_n) a_q <= 0; else a_q <= a_in;
  logos_dcfifo_mlab #(.W(8), .AW(4)) f(.wclk(a_clk), .wce(1'b1), .wrst_n(a_rst_n), .wvalid(1'b1), .wready(wready), .wdata(a_q),
    .wfree(wfree), .rclk(b_clk), .rce(1'b1), .rrst_n(b_rst_n), .rvalid(rvalid), .rready(1'b1), .rdata(rdata), .rused(rused));
  inner u(.clk(b_clk), .rst_n(b_rst_n), .valid(rvalid), .d(rdata), .q(b_out));
endmodule
module inner(input clk, input rst_n, input valid, input [7:0] d, output reg [7:0] q);
  always @(posedge clk or negedge rst_n) if (!rst_n) q <= 8'd0; else if (valid) q <= d;
endmodule
