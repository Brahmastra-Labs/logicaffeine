// dpu_ingest — the DPU near-data compute tile: sequential CRDT merge-on-ingest.
// On each posedge clk with in_valid, acc <= per-replica-max(acc, in_state) — the same
// join-semilattice merge as crdt_merge (proven == GCounter::merge), now folded over a
// stream. Order-invariant by the certified lattice laws, so it is correct even when the
// deterministic controller delivers ingest beats out of logical order. 4 replicas x 32-bit.
module dpu_ingest(
  input          clk,
  input          rst,        // synchronous clear of the accumulator
  input          in_valid,   // a beat of near-data arrived this cycle
  input  [127:0] in_state,   // {s3,s2,s1,s0}, one 32-bit count per replica
  output [127:0] state,      // the accumulated join so far
  output [33:0]  value       // sum of the four accumulated lanes
);
  reg  [127:0] acc;
  wire [31:0] a0 = acc[31:0],      a1 = acc[63:32],      a2 = acc[95:64],      a3 = acc[127:96];
  wire [31:0] b0 = in_state[31:0], b1 = in_state[63:32], b2 = in_state[95:64], b3 = in_state[127:96];
  wire [31:0] m0 = (a0 >= b0) ? a0 : b0;
  wire [31:0] m1 = (a1 >= b1) ? a1 : b1;
  wire [31:0] m2 = (a2 >= b2) ? a2 : b2;
  wire [31:0] m3 = (a3 >= b3) ? a3 : b3;
  always @(posedge clk) begin
    if (rst)           acc <= 128'd0;
    else if (in_valid) acc <= {m3, m2, m1, m0};
  end
  assign state = acc;
  assign value = acc[31:0] + acc[63:32] + acc[95:64] + acc[127:96];  // 34-bit LHS ⇒ no truncation
endmodule
