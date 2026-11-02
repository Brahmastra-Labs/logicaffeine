// decision_hft — the HFT decision if-CONVERTED to a combinational select-DAG.
// Every `If` is a predicated Select (mux) tile; the answer settles in ONE clock — no PC,
// no fetch, no dispatch, no loop counter. Futamura P1: the decision IS the circuit.
// Lowered from the REAL compiler; 64-bit, so it wraps identically to the VM's Int; proven
// == the running VM by iverilog.
module decision_hft(input [63:0] in0, input [63:0] in1, input [63:0] in2, input [63:0] in3, input [63:0] in4, input [63:0] in5, input [63:0] in6, input [63:0] in7, output [63:0] result);
  wire [63:0] v0 = $signed(in1) + $signed(in2);
  wire [63:0] v1 = $signed(in1) - $signed(in2);
  wire [63:0] v2 = ($signed(in0) < $signed(v1)) ? 64'sd1 : 64'sd0;
  wire [63:0] v3 = ($signed(in0) < $signed(v1)) ? in3 : 64'sd0;
  wire [63:0] v4 = ($signed(in0) > $signed(v0)) ? 64'sd2 : v2;
  wire [63:0] v5 = ($signed(in0) > $signed(v0)) ? in3 : v3;
  wire [63:0] v6 = ($signed(in7) == $signed(64'sd1)) ? v4 : 64'sd0;
  wire [63:0] v7 = ($signed(in7) == $signed(64'sd1)) ? v5 : 64'sd0;
  wire [63:0] v8 = ($signed(v7) > $signed(in4)) ? in4 : v7;
  wire [63:0] v9 = ($signed(v6) == $signed(64'sd1)) ? v8 : 64'sd0;
  wire [63:0] v10 = $signed(64'sd0) - $signed(v8);
  wire [63:0] v11 = ($signed(v6) == $signed(64'sd2)) ? v10 : v9;
  wire [63:0] v12 = $signed(in5) + $signed(v11);
  wire [63:0] v13 = ($signed(v12) > $signed(in6)) ? 64'sd0 : 64'sd1;
  wire [63:0] v14 = $signed(64'sd0) - $signed(in6);
  wire [63:0] v15 = ($signed(v12) < $signed(v14)) ? 64'sd0 : v13;
  wire [63:0] v16 = ($signed(v15) == $signed(64'sd0)) ? 64'sd0 : v11;
  assign result = v16;
endmodule
