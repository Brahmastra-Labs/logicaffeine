// cgra_dot — a compiled COUNTED LOOP as a CGRA: a fixed spatial body datapath
// (wired SSA, no fetch/dispatch) streamed by a sequential controller once per induction
// value in lo..=hi. Futamura P1 for control flow: the body IS the circuit; the loop is a
// counter. 64-bit, so it wraps identically to the VM's Int; the body's one-step next-state
// is certified == the VM's per-iteration transition by our own SAT (the induction step).
module cgra_dot(input clk, input rst, input [63:0] seq_len, input mem_we, input [8:0] mem_waddr, input signed [63:0] mem_wdata, output [63:0] result, output done);
  reg [63:0] s0, s1, s2;
  reg [63:0] iv;
  reg running;
  reg signed [63:0] mem [0:511];
  always @(posedge clk) if (mem_we) mem[mem_waddr] <= mem_wdata;
  wire [63:0] lo = 64'sd0;
  wire [63:0] hi = $signed(seq_len) - 64'sd1;
  wire [63:0] amr0 = iv;
  wire [63:0] b0 = mem[amr0[8:0]];
  wire signed [7:0] ma1 = s1;
  wire signed [7:0] mb1 = b0;
  wire signed [15:0] mp1 = ma1 * mb1;
  wire [63:0] b1 = mp1;
  wire [63:0] b2 = $signed(s0) + $signed(b1);
  wire [63:0] b3 = ($signed(s2) == $signed(64'sd1)) ? b2 : s0;
  wire [63:0] b4 = $signed(64'sd1) - $signed(s2);
  wire [63:0] ns0 = b3;
  wire [63:0] ns1 = b0;
  wire [63:0] ns2 = b4;
  always @(posedge clk) begin
    if (rst) begin
      s0 <= 64'sd0;
      s1 <= 64'sd0;
      s2 <= 64'sd0;
      iv <= lo;
      running <= ($signed(lo) <= $signed(hi));
    end else if (running) begin
      s0 <= ns0;
      s1 <= ns1;
      s2 <= ns2;
      if ($signed(iv) < $signed(hi)) iv <= iv + 64'sd1;
      else running <= 1'b0;
    end
  end
  assign done = !running;
  assign result = s0;
endmodule
