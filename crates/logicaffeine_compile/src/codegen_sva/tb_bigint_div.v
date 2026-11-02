// tb_bigint_div — drive the sequential 256-bit bigint_div and check quotient+remainder against the
// reference UNSIGNED magnitude division (`a / b`, `a % b`, b==0 -> 0) — the same operation as
// logicaffeine_base::numeric::mag_div — over edge cases plus a 2000-pair random sweep.
`timescale 1ns/1ps
module tb_bigint_div;
  localparam W = 256;
  reg              clk = 0, rst = 1, start = 0;
  reg  [W-1:0]     a, b;
  wire [W-1:0]     q, r;
  wire             done;
  integer          errors = 0, i;
  reg  [W-1:0]     eq, er;
  reg  [63:0]      seed;

  bigint_div #(.W(W)) dut(.clk(clk), .rst(rst), .start(start), .a(a), .b(b), .q(q), .r(r), .done(done));
  always #5 clk = ~clk;

  task check(input [W-1:0] ta, input [W-1:0] tb_);
    begin
      a = ta; b = tb_;
      @(negedge clk); start = 1'b1;
      @(posedge clk);                        // IDLE samples start -> clears done, enters RUN
      @(negedge clk); start = 1'b0;
      while (!done) @(negedge clk);           // stall until the 256-cycle division finishes
      if (tb_ == {W{1'b0}}) begin eq = {W{1'b0}}; er = {W{1'b0}}; end
      else begin eq = ta / tb_; er = ta % tb_; end   // native unsigned 256-bit magnitude div/mod
      if (q !== eq || r !== er) begin
        errors = errors + 1;
        $display("FAIL a=%h b=%h : q=%h r=%h exp q=%h r=%h", ta, tb_, q, r, eq, er);
      end
    end
  endtask

  // 256-bit pseudo-random from four 64-bit LCG draws.
  function [W-1:0] rnd;
    integer j;
    begin
      rnd = {W{1'b0}};
      for (j = 0; j < 4; j = j + 1) begin
        seed = seed * 64'd6364136223846793005 + 64'd1442695040888963407;
        rnd = (rnd << 64) | seed;
      end
    end
  endfunction

  initial begin
    #12 rst = 0;
    // ── edge cases ──
    check(256'd0,       256'd0);              // 0 / 0             -> 0, 0
    check(256'd100,     256'd0);              // x / 0             -> 0, 0 (guard)
    check(256'd100,     256'd7);
    check(256'd7,       256'd100);            // a < b             -> 0, a
    check(256'd42,      256'd42);             // a == b            -> 1, 0
    check(256'd1000000, 256'd3);
    check({W{1'b1}},    256'd1);              // (2^256 - 1) / 1
    check({W{1'b1}},    256'd2);
    check({1'b1, 255'd0}, 256'd3);            // 2^255 / 3
    check(256'hDEADBEEFCAFEBABE0000000000000000111122223333444455556666_77778888,
          256'h0000000000000000000000000000000000000000000000000000000012345678);
    // ── random sweep ──
    seed = 64'hB16D_1234_5678_9ABC;
    for (i = 0; i < 2000; i = i + 1) check(rnd(), rnd());
    if (errors == 0)
      $display("PASS: bigint_div == reference 256-bit magnitude div/mod over edge cases + 2000 random pairs");
    else
      $display("FAIL: %0d mismatches", errors);
    $finish;
  end
endmodule
