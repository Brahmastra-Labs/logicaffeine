// cgra_sumto — a compiled COUNTED LOOP as a CGRA: a fixed spatial body datapath
// (wired SSA, no fetch/dispatch) streamed by a sequential controller once per induction
// value in lo..=hi. Futamura P1 for control flow: the body IS the circuit; the loop is a
// counter. 64-bit, so it wraps identically to the VM's Int; the body's one-step next-state
// is certified == the VM's per-iteration transition by our own SAT (the induction step).
module cgra_sumto(input clk, input rst, input [63:0] in0, output [63:0] result, output done);
  reg [63:0] s0;
  reg [63:0] iv;
  reg running;
  wire [63:0] lo = 64'sd1;
  wire [63:0] hi = in0;
  wire [63:0] b0 = $signed(s0) + $signed(iv);
  wire [63:0] ns0 = b0;
  always @(posedge clk) begin
    if (rst) begin
      s0 <= 64'sd0;
      iv <= lo;
      running <= ($signed(lo) <= $signed(hi));
    end else if (running) begin
      s0 <= ns0;
      if ($signed(iv) < $signed(hi)) iv <= iv + 64'sd1;
      else running <= 1'b0;
    end
  end
  assign done = !running;
  assign result = s0;
endmodule
