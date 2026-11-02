// ntt_butterfly — Montgomery fqmul + one CT butterfly (ML-KEM, q=3329).
// t = montgomery_reduce(zeta*b); hi = a + t; lo = a - t  (16-bit wrapping).
module ntt_butterfly(input signed [15:0] a, input signed [15:0] b,
                     input signed [15:0] zeta, output signed [15:0] hi,
                     output signed [15:0] lo);
  wire signed [31:0] prod = zeta * b;                  // full 32-bit product
  wire signed [15:0] m    = prod[15:0] * (-16'sd3327); // (prod as i16) * qinv (=-3327)
  wire signed [31:0] sub  = prod - m * 32'sd3329;       // prod - m*q
  wire signed [15:0] t    = sub[31:16];                // arithmetic >> 16
  assign hi = a + t;
  assign lo = a - t;
endmodule
