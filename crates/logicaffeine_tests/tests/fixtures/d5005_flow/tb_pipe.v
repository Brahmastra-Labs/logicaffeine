// logos_pipe between a source offering words IP% of its clocks and a sink taking them OP% of its clocks: every word out
// in order, none lost, none twice, none taken while in_ready is low; then, both sides ready every clock, a word out
// every clock a clock after it went in.
`timescale 1ps/1ps
module tb;
  parameter SEED = 1, IP = 70, OP = 60, N = 4000, FULL = 1000;
  reg clk = 0, rst_n = 0;
  always #500 clk = ~clk;
  reg in_valid = 0, out_ready = 0;
  reg [15:0] in_data = 0;
  wire in_ready, out_valid;
  wire [15:0] out_data;
  logos_pipe #(.W(16)) p(.clk(clk), .rst_n(rst_n), .in_valid(in_valid), .in_ready(in_ready), .in_data(in_data),
    .out_valid(out_valid), .out_ready(out_ready), .out_data(out_data));
  integer seed, sent, got, errors, i, run, clocks;
  initial begin
    seed = SEED; sent = 0; got = 0; errors = 0; clocks = 0;
    repeat (3) @(posedge clk);
    #1 rst_n = 1;
    // Each falling edge sets the source's and the sink's signals; what the next rising edge moves is counted from them
    // and the pipe's registers, which only that edge changes. A word lost leaves the count short at a hundred clocks a word.
    while (got < N && clocks < 100 * N) begin
      @(negedge clk);
      clocks = clocks + 1;
      in_valid = (sent < N) && ($urandom(seed) % 100 < IP);
      in_data = sent[15:0];
      out_ready = $urandom(seed) % 100 < OP;
      if (in_valid && in_ready) sent = sent + 1;
      if (out_valid && out_ready) begin
        if (out_data !== got[15:0]) errors = errors + 1;
        got = got + 1;
      end
    end
    // Both sides ready every clock: a word out every clock, each the clock after it went in.
    @(negedge clk);
    in_valid = 0; out_ready = 1;
    repeat (4) @(negedge clk);
    if (out_valid || !in_ready) errors = errors + 1;
    run = 0;
    in_valid = 1; in_data = 0;
    for (i = 1; i <= FULL; i = i + 1) begin
      @(negedge clk);
      if (!(out_valid && out_data === i[15:0] - 16'd1 && in_ready)) errors = errors + 1;
      else run = run + 1;
      in_valid = i < FULL; in_data = i[15:0];
    end
    $display("RESULT sent=%0d got=%0d errors=%0d run=%0d", sent, got, errors, run);
    $finish;
  end
endmodule
