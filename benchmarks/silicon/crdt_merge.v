// crdt_merge — the CRDT (G-Counter) join-semilattice merge in silicon.
// merged[i] = max(a[i], b[i]) per replica lane; value = sum(merged).
// Byte-identical to logicaffeine_data::crdt::GCounter::merge (proven by
// crdt_merge_unit_matches_the_gcounter_oracle). 4 replicas x 32-bit.
module crdt_merge(
  input  [127:0] a,        // {a3,a2,a1,a0}, one 32-bit count per replica
  input  [127:0] b,
  output [127:0] merged,   // per-replica max (the lattice join)
  output [33:0]  value     // sum of the four merged lanes
);
  wire [31:0] a0 = a[31:0],  a1 = a[63:32],  a2 = a[95:64],  a3 = a[127:96];
  wire [31:0] b0 = b[31:0],  b1 = b[63:32],  b2 = b[95:64],  b3 = b[127:96];
  wire [31:0] m0 = (a0 >= b0) ? a0 : b0;
  wire [31:0] m1 = (a1 >= b1) ? a1 : b1;
  wire [31:0] m2 = (a2 >= b2) ? a2 : b2;
  wire [31:0] m3 = (a3 >= b3) ? a3 : b3;
  assign merged = {m3, m2, m1, m0};
  assign value  = m0 + m1 + m2 + m3;   // 34-bit LHS ⇒ no truncation
endmodule
