// A dual-clock FIFO filled faster than it is read: a writer offering a counter's words a clock each that, when HOLD,
// holds a refused word until the FIFO takes it, and otherwise moves on to the next; without HOLDS the FIFO is one
// whose writer may let a word fall. Built HOLDS, a FIFO whose writer lets a refused word fall stops the simulation;
// otherwise the run ends with the words taken and read.
//   RESULT sent=<n> got=<n>
`timescale 1ps/1ps
module tb;
  parameter integer HOLD = 1, HOLDS = 1, N = 400;
  localparam W = 16, AW = 2;
  reg wclk = 0, rclk = 0, rst_n = 0;
  always #1000 wclk = ~wclk;
  initial begin #313 forever #3100 rclk = ~rclk; end
  reg          wvalid = 0;
  reg  [W-1:0] wdata = 0;
  wire         wready, rvalid;
  wire [W-1:0] rdata;
  wire [AW:0]  wfree, rused;
  generate if (HOLDS) begin : held
    logos_dcfifo_mlab #(.W(W), .AW(AW), .HOLDS(1)) dut(.wclk(wclk), .wce(1'b1), .wrst_n(rst_n), .wvalid(wvalid),
      .wready(wready), .wdata(wdata), .wfree(wfree), .rclk(rclk), .rce(1'b1), .rrst_n(rst_n), .rvalid(rvalid),
      .rready(1'b1), .rdata(rdata), .rused(rused));
  end else begin : unheld
    logos_dcfifo_mlab #(.W(W), .AW(AW)) dut(.wclk(wclk), .wce(1'b1), .wrst_n(rst_n), .wvalid(wvalid),
      .wready(wready), .wdata(wdata), .wfree(wfree), .rclk(rclk), .rce(1'b1), .rrst_n(rst_n), .rvalid(rvalid),
      .rready(1'b1), .rdata(rdata), .rused(rused));
  end endgenerate
  integer sent = 0, got = 0;
  always @(posedge wclk) if (rst_n) begin
    if (wvalid && wready) sent = sent + 1;
    if (!wvalid || wready || !HOLD) begin
      wvalid <= sent < N;
      wdata <= wdata + 1'b1;
    end
  end
  always @(posedge rclk) if (rst_n && rvalid) got = got + 1;
  initial begin
    #20000 rst_n = 1;
    wait (sent == N || (!HOLD && wdata >= N + 64));
    #200000;
    $display("RESULT sent=%0d got=%0d", sent, got);
    $finish;
  end
endmodule
