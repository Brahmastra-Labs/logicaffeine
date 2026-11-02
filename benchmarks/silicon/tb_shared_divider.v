// tb_shared_divider — drive the sequential shared_divider and check its quotient+remainder against
// the combinational `$signed(a)/$signed(b)` / `$signed(a)%$signed(b)` oracle (the exact ALU semantics,
// b==0 → 0) over every sign/edge case plus a 4000-pair pseudo-random sweep. PASS iff all match.
`timescale 1ns/1ps
module tb_shared_divider;
  localparam W = 32;
  reg              clk = 0, rst = 1, start = 0;
  reg  [W-1:0]     a, b;
  wire [W-1:0]     q, r;
  wire             done;
  integer          errors = 0, i;
  reg  [W-1:0]     eq, er;
  reg  [63:0]      seed;
  reg  [W-1:0]     ra, rb;

  shared_divider #(.W(W)) dut(.clk(clk), .rst(rst), .start(start), .a(a), .b(b), .q(q), .r(r), .done(done));
  always #5 clk = ~clk;

  // One division: pulse start, wait for done, compare q/r to the combinational oracle.
  task check(input [W-1:0] ta, input [W-1:0] tb_);
    begin
      a = ta; b = tb_;
      @(negedge clk); start = 1'b1;
      @(posedge clk);                       // IDLE samples start → clears done, enters RUN
      @(negedge clk); start = 1'b0;
      while (!done) @(negedge clk);          // stall until the divider finishes
      // Signed oracle. An if/else (not a ?: with an unsigned zero branch) keeps the division in a
      // SIGNED context — a `{W{1'b0}}` branch would poison it to an unsigned divide (Verilog's
      // context-determined signedness), the exact trap the ALU avoids with its unsized signed `0`.
      if (tb_ == 0) begin
        eq = {W{1'b0}}; er = {W{1'b0}};
      end else begin
        eq = $signed(ta) / $signed(tb_);
        er = $signed(ta) % $signed(tb_);
      end
      if (q !== eq || r !== er) begin
        errors = errors + 1;
        $display("FAIL a=%0d b=%0d : got q=%0d r=%0d  exp q=%0d r=%0d",
                 $signed(ta), $signed(tb_), $signed(q), $signed(r), $signed(eq), $signed(er));
      end
    end
  endtask

  initial begin
    #12 rst = 1'b0;
    // ── sign / edge cases ──
    check(32'd0,        32'd0);           // 0 / 0            → 0, 0 (guard)
    check(32'd7,        32'd0);           // x / 0            → 0, 0 (guard)
    check(32'd42,       32'd1);
    check(32'd42,       32'd5);
    check(-32'sd42,     32'sd5);
    check(32'sd42,      -32'sd5);
    check(-32'sd42,     -32'sd5);
    check(32'h80000000, -32'sd1);         // INT_MIN / -1     → INT_MIN, 0 (overflow wrap)
    check(32'h80000000, 32'sd1);
    check(32'h80000000, 32'sd2);
    check(32'h80000000, 32'sd3);
    check(32'h7FFFFFFF, 32'sd2);
    check(32'sd5,       32'sd7);           // |a| < |b|        → 0, a
    check(32'sd7,       32'sd7);           // a == b           → 1, 0
    check(-32'sd7,      32'sd7);
    check(32'sd1000000, 32'sd3);
    check(-32'sd1000000, 32'sd3);
    // ── pseudo-random sweep (SplitMix-ish LCG) ──
    seed = 64'hDEAD_BEEF_1234_5678;
    for (i = 0; i < 4000; i = i + 1) begin
      seed = seed * 64'd6364136223846793005 + 64'd1442695040888963407;
      ra   = seed[63:32];
      seed = seed * 64'd6364136223846793005 + 64'd1442695040888963407;
      rb   = seed[63:32];
      check(ra, rb);
    end
    if (errors == 0)
      $display("PASS: shared_divider == combinational $signed div/mod over all edge cases + 4000 random pairs");
    else
      $display("FAIL: %0d mismatches", errors);
    $finish;
  end
endmodule
