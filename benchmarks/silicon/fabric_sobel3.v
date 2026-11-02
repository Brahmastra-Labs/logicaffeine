// fabric_sobel3 — a fixed-point SOBEL EDGE DETECTOR as a dispatch-free dataflow circuit.
// Semantically IDENTICAL to the LOGOS `sobel3` residual (fabric_sobel_lock.rs): the two Sobel
// gradients plus the L1 edge magnitude |Gx|+|Gy| via the branch-free absolute value
// |x| = (x ^ (x>>>63)) - (x>>>63). Hand-authored here only because the lib rebuild that would
// EMIT this (residual_from_source) is momentarily blocked by a co-agent's in-progress
// soft_core_model.rs refactor; the arithmetic matches sobel3 op-for-op and is iverilog-verified
// against the VM's answer. 64-bit signed wrapping, so it equals the VM's Int exactly.
// Window is row-major p0..p8 (in0..in8); in4 (center) never contributes to a Sobel kernel.
module fabric_sobel3(
  input [63:0] in0, input [63:0] in1, input [63:0] in2,
  input [63:0] in3, input [63:0] in4, input [63:0] in5,
  input [63:0] in6, input [63:0] in7, input [63:0] in8,
  output [63:0] result
);
  // Sobel-X kernel [-1,0,1; -2,0,2; -1,0,1]
  wire signed [63:0] gx = -$signed(in0) + $signed(in2)
                          - (64'sd2)*$signed(in3) + (64'sd2)*$signed(in5)
                          - $signed(in6) + $signed(in8);
  // Sobel-Y kernel [-1,-2,-1; 0,0,0; 1,2,1]
  wire signed [63:0] gy = -$signed(in0) - (64'sd2)*$signed(in1) - $signed(in2)
                          + $signed(in6) + (64'sd2)*$signed(in7) + $signed(in8);
  // |gx| and |gy| — branch-free, exactly the LOGOS bit-trick.
  wire signed [63:0] mx = $signed(gx) >>> 63;
  wire signed [63:0] ax = ($signed(gx) ^ mx) - mx;
  wire signed [63:0] my = $signed(gy) >>> 63;
  wire signed [63:0] ay = ($signed(gy) ^ my) - my;
  assign result = ax + ay;
endmodule
