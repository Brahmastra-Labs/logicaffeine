// A dual-clock FIFO of tokens: a writer pulsing a token at random, a step at a time in bursts, never looking at wready,
// as a stream engine marks its runs done; a reader taking them at random; each side stepping at its enable. With TOKENS
// the FIFO counts the tokens it has not taken; without, it is a plain FIFO given the pulses. Once the writer has given
// every token and both sides have been quiet a while, the counts.
//   RESULT given=<n> got=<n>
`timescale 1ps/1ps
module tb;
  parameter integer TOKENS = 1, WHALF = 1667, RHALF = 5000, WCE = 1, RCE = 1, SEED = 1, N = 2000, BURST = 90, RP = 60;
  reg wclk = 0, rclk = 0, wrst_n = 0, rrst_n = 0;
  always #(WHALF) wclk = ~wclk;
  initial begin #(RHALF / 3) forever #(RHALF) rclk = ~rclk; end
  reg wce = 0, rce = 0;
  integer wphase = 0, rphase = 0;
  always @(posedge wclk) begin wphase = (wphase + 1) % WCE; wce <= wphase == 0; end
  always @(posedge rclk) begin rphase = (rphase + 1) % RCE; rce <= rphase == 0; end
  reg  wvalid = 0, rready = 0;
  wire wready, rvalid, rdata;
  wire [2:0] wfree, rused;
  generate if (TOKENS) begin : counted
    logos_dcfifo_mlab #(.W(1), .AW(2), .TOKENS(1)) dut(.wclk(wclk), .wce(wce), .wrst_n(wrst_n), .wvalid(wvalid),
      .wready(wready), .wdata(1'b1), .wfree(wfree), .rclk(rclk), .rce(rce), .rrst_n(rrst_n), .rvalid(rvalid),
      .rready(rready), .rdata(rdata), .rused(rused));
  end else begin : plain
    logos_dcfifo_mlab #(.W(1), .AW(2)) dut(.wclk(wclk), .wce(wce), .wrst_n(wrst_n), .wvalid(wvalid),
      .wready(wready), .wdata(1'b1), .wfree(wfree), .rclk(rclk), .rce(rce), .rrst_n(rrst_n), .rvalid(rvalid),
      .rready(rready), .rdata(rdata), .rused(rused));
  end endgenerate
  integer seed = SEED, given = 0, got = 0, quiet = 0;
  // A burst: a token every step while it lasts, a burst starting a step in ten.
  reg bursting = 0;
  always @(posedge wclk) if (wrst_n && wce) begin
    if (wvalid) given = given + 1;
    bursting <= bursting ? (($random(seed) % 100 + 100) % 100 < BURST) : (($random(seed) % 100 + 100) % 100 < 10);
    wvalid <= given + wvalid < N && bursting;
  end
  always @(posedge rclk) if (rrst_n && rce) begin
    if (rvalid && rready) got = got + 1;
    rready <= (($random(seed) % 100 + 100) % 100) < RP;
  end
  always @(posedge rclk) quiet <= (rvalid || wvalid) ? 0 : quiet + 1;
  initial begin
    #(WHALF * 20) wrst_n = 1; rrst_n = 1;
    wait (given == N);
    wait (quiet > 200 * RCE);
    $display("RESULT given=%0d got=%0d", given, got);
    $finish;
  end
endmodule
