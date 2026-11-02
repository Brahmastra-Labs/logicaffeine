// bigint_mul — the 64x64 -> 128 limb multiplier mag_mul iterates. The full 256-bit
// product is this primitive composed with the proven bigint_alu carry-adds (schoolbook).
// prod = a * b, byte-identical to (a as u128) * (b as u128)
// (proven by bigint_mul_matches_the_limb_mul_oracle; distributivity by our certified SAT).
//
// BUILT FROM FOUR 32x32 SUB-PRODUCTS RATHER THAN ONE 64x64 ARRAY, and that is a
// MANUFACTURABILITY choice, not an arithmetic one. `assign prod = a * b` lets synthesis build a
// flat 64x64 array whose partial-product nets are up to 128 bits wide and run the width of the
// block. On sky130 those long wide runs are an antenna source, and an antenna violation on a net
// that ALREADY carries a diode cannot be repaired by adding another one — measured on
// coproc_core, where all 7 surviving violators had a diode on the violating pin and the jog
// router could not bridge any of them (TritonRoute had patched them with via-enclosure RECTs,
// whose geometry odb cannot round-trip from Python). Every intermediate below is <= 65 bits,
// matching the width of every antenna-clean block in the suite.
//
// EQUIVALENCE IS NOT ASSUMED. Verified against `a * b` under iverilog over 16 corner cases
// (0 / 1 / 2^64-1 / the 2^32 boundary, in both operands) and 200,000 random vectors, plus the
// 64 oracle vectors of tb_bigint_mul.
module bigint_mul(
  input  [63:0]  a,
  input  [63:0]  b,
  output [127:0] prod
);
  wire [31:0] a0 = a[31:0], a1 = a[63:32];
  wire [31:0] b0 = b[31:0], b1 = b[63:32];
  wire [63:0] t0 = a0 * b0;                              // 32x32 -> 64, four of them
  wire [63:0] t1 = a0 * b1;
  wire [63:0] t2 = a1 * b0;
  wire [63:0] t3 = a1 * b1;
  wire [64:0] mid    = {1'b0, t1} + {1'b0, t2};          // 65-bit, cannot overflow
  wire [64:0] lo_sum = {1'b0, t0} + {mid[31:0], 32'b0};  // t0 + (mid_lo << 32)
  wire [63:0] plo = lo_sum[63:0];
  wire [63:0] phi = t3 + mid[64:32] + {63'b0, lo_sum[64]};
  assign prod = {phi, plo};
endmodule
