// bigint_alu — a 256-bit (4 x 64-bit limb) BigInt datapath: add, subtract, unsigned
// compare, over one shared carry/borrow chain. Byte-identical to the software limb ops
// (proven by bigint_alu_matches_the_numeric_oracle) and correct by our certified SAT
// (the_bigint_alu_is_a_certified_group_and_order). a[63:0] is limb 0 (little-endian).
module bigint_alu(
  input  [255:0] a,
  input  [255:0] b,
  output [255:0] sum,    output cout,      // a + b  (carry out of the top limb kept)
  output [255:0] diff,   output borrow,    // a - b  (borrow = 1 iff a < b; magnitude when a>=b)
  output         eq,     output lt         // unsigned compare: a == b, a < b
);
  assign {cout, sum}    = a + b;           // 257-bit LHS ⇒ carry-propagate, carry retained
  assign {borrow, diff} = {1'b0, a} - {1'b0, b}; // borrow (bit 256) = 1 ⇔ a < b
  assign eq = (a == b);
  assign lt = borrow;                      // a < b  ⇔  the subtract underflowed
endmodule
