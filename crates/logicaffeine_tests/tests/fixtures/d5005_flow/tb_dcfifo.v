// One dual-clock FIFO (`FIFO`, a module name) under two unrelated clocks and random traffic: every word out in order,
// neither side ever claiming more than the FIFO holds. Each side steps at its enable, high one cycle in WCE (RCE) of its
// clock, its producer (consumer) acting only then, as a side clocked that much slower does; neither side's outputs move
// at an edge its enable is low for.
`timescale 1ps/1ps
module tb;
  parameter W = 32, AW = 3;
  parameter WHALF = 1667, RHALF = 1111, SEED = 1, N = 20000, WP = 70, RP = 60, WCE = 1, RCE = 1;
  reg wclk = 0, rclk = 0, wrst_n = 0, rrst_n = 0;
  always #(WHALF) wclk = ~wclk;
  always #(RHALF) rclk = ~rclk;
  reg wce = 1, rce = 1;
  integer wphase = 0, rphase = 0;
  always @(posedge wclk) begin wphase = (wphase + 1) % WCE; wce <= wphase == 0; end
  always @(posedge rclk) begin rphase = (rphase + 1) % RCE; rce <= rphase == 0; end
  reg wvalid = 0, rready = 0;
  reg [W-1:0] wdata = 0;
  wire wready, rvalid;
  wire [W-1:0] rdata;
  wire [AW:0] wfree, rused;
  `FIFO #(.W(W), .AW(AW)) dut(.wclk(wclk), .wce(wce), .wrst_n(wrst_n), .wvalid(wvalid), .wready(wready), .wdata(wdata),
                                     .wfree(wfree), .rclk(rclk), .rce(rce), .rrst_n(rrst_n), .rvalid(rvalid), .rready(rready),
                                     .rdata(rdata), .rused(rused));
  integer seed = SEED, sent = 0, got = 0, errors = 0, inflight;
  initial begin
    #10000 wrst_n = 1; rrst_n = 1;
  end
  always @(posedge wclk) if (wrst_n && wce) begin
    if (wvalid && wready) begin sent = sent + 1; wdata <= wdata + 1; end
    if (wfree > (1 << AW)) begin errors = errors + 1; $display("wfree %0d past depth", wfree); end
    wvalid <= (sent < N) && (($random(seed) % 100 + 100) % 100 < WP);
  end
  always @(posedge rclk) if (rrst_n && rce) begin
    if (rvalid && rready) begin
      if (rdata !== got) begin errors = errors + 1; if (errors < 5) $display("read %0d, expected %0d", rdata, got); end
      got = got + 1;
    end
    rready <= (($random(seed) % 100 + 100) % 100 < RP);
  end
  // The two views never claim more than the FIFO holds: what the write side has pushed minus what the read side
  // has taken, plus one word in the output register.
  always @(posedge rclk) if (rrst_n) begin
    inflight = sent - got;
    if (rused > inflight + 1) begin errors = errors + 1; $display("rused %0d past %0d in flight", rused, inflight); end
  end
  // A side's outputs as they stood before each edge of its clock, and after an edge it did not step at.
  reg [AW+1:0]  wview;
  reg [W+AW+1:0] rview;
  always @(negedge wclk) wview = {wready, wfree};
  always @(negedge rclk) rview = {rvalid, rdata, rused};
  always @(posedge wclk) if (wrst_n && !wce) #1 if ({wready, wfree} !== wview) begin
    errors = errors + 1; if (errors < 5) $display("the write side moved at %0t, an edge it did not step at", $time);
  end
  always @(posedge rclk) if (rrst_n && !rce) #1 if ({rvalid, rdata, rused} !== rview) begin
    errors = errors + 1; if (errors < 5) $display("the read side moved at %0t, an edge it did not step at", $time);
  end
  initial begin
    wait (got == N);
    #100000;
    if (got != N || sent != N) errors = errors + 1;
    $display("RESULT sent=%0d got=%0d errors=%0d", sent, got, errors);
    $finish;
  end
  initial begin #2000000000; $display("RESULT timeout sent=%0d got=%0d", sent, got); $finish; end
endmodule
