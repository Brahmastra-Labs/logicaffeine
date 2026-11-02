// fabric_horner — a compiled dispatch-free residual as a fixed dataflow circuit.
// No program counter, no fetch, no opcode dispatch: the residual's ops are WIRED
// (SSA), not interpreted. Futamura P1: the specialized residual IS the circuit.
// Lowered from the REAL compiler's residual; 64-bit so it wraps identically to the
// VM's Int; proven == the running VM by iverilog.
module fabric_horner(input [63:0] in0, output [63:0] result);
  wire [63:0] v0 = $signed(64'sd2) * $signed(in0);
  wire [63:0] v1 = $signed(v0) + $signed(64'sd5);
  wire [63:0] v2 = $signed(v1) * $signed(in0);
  wire [63:0] v3 = $signed(v2) - $signed(64'sd3);
  wire [63:0] v4 = $signed(v3) * $signed(in0);
  wire [63:0] v5 = $signed(v4) + $signed(64'sd100);
  assign result = v5;
endmodule
