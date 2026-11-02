// bigint_mul32 — 64x64 -> 128 multiply built from four 32x32 sub-products, every intermediate
// <= 65 bits (no 128-bit array, no 128-bit accumulator), output as two 64-bit words {phi, plo}.
// P = a0*b0 + (a0*b1 + a1*b0)*2^32 + a1*b1*2^64, assembled word-by-word with an explicit carry.
// Byte-identical to a*b (proven by tb_bigint_mul32). This is the narrow-datapath multiply: its
// widest net is 65-bit, matching every antenna-clean block (all <=72-bit).
module bigint_mul32(input [63:0] a, input [63:0] b, output [63:0] plo, output [63:0] phi);
  wire [31:0] a0 = a[31:0], a1 = a[63:32];
  wire [31:0] b0 = b[31:0], b1 = b[63:32];
  wire [63:0] t0 = a0 * b0;                 // 32x32 -> 64
  wire [63:0] t1 = a0 * b1;
  wire [63:0] t2 = a1 * b0;
  wire [63:0] t3 = a1 * b1;
  wire [64:0] mid    = {1'b0, t1} + {1'b0, t2};        // 65-bit
  wire [64:0] lo_sum = {1'b0, t0} + {mid[31:0], 32'b0}; // t0 + (mid_lo << 32), 65-bit
  assign plo = lo_sum[63:0];
  assign phi = t3 + mid[64:32] + {63'b0, lo_sum[64]};   // t3 + (mid >> 32) + carry (fits: P<2^128)
endmodule
