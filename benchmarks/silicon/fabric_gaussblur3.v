// fabric_gaussblur3 — a compiled dispatch-free residual as a fixed dataflow circuit.
// No program counter, no fetch, no opcode dispatch: the residual's ops are WIRED
// (SSA), not interpreted. Futamura P1: the specialized residual IS the circuit.
// Lowered from the REAL compiler's residual; 64-bit so it wraps identically to the
// VM's Int; proven == the running VM by iverilog.
module fabric_gaussblur3(input [63:0] in0, input [63:0] in1, input [63:0] in2, input [63:0] in3, input [63:0] in4, input [63:0] in5, input [63:0] in6, input [63:0] in7, input [63:0] in8, output [63:0] result);
  wire [63:0] v0 = $signed(64'sd2) * $signed(in1);
  wire [63:0] v1 = $signed(in0) + $signed(v0);
  wire [63:0] v2 = $signed(v1) + $signed(in2);
  wire [63:0] v3 = $signed(64'sd2) * $signed(in3);
  wire [63:0] v4 = $signed(v2) + $signed(v3);
  wire [63:0] v5 = $signed(64'sd4) * $signed(in4);
  wire [63:0] v6 = $signed(v4) + $signed(v5);
  wire [63:0] v7 = $signed(64'sd2) * $signed(in5);
  wire [63:0] v8 = $signed(v6) + $signed(v7);
  wire [63:0] v9 = $signed(v8) + $signed(in6);
  wire [63:0] v10 = $signed(64'sd2) * $signed(in7);
  wire [63:0] v11 = $signed(v9) + $signed(v10);
  wire [63:0] v12 = $signed(v11) + $signed(in8);
  wire [63:0] v13 = ($signed(v12) >>> ((64'sd4) & 64'd63));
  assign result = v13;
endmodule
