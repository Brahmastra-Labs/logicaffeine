// while_accw — a compiled WHILE loop as a CGRA: a fixed body datapath streamed by a
// controller whose CONTINUE condition is FABRIC-COMPUTED (a predicate over the state), not
// a counter — the data-dependent trip count is decided by the datapath. Futamura P1 for a
// data-dependent loop; 64-bit so it wraps like the VM's Int; body-step and predicate are
// each certified == the VM by our own SAT.
module while_accw(input clk, input rst, input [63:0] in0, output [63:0] result, output done);
  reg [63:0] s0, s1;
  reg running;
  wire [63:0] inv0 = in0;
  wire [63:0] b0 = $signed(s1) + $signed(s0);
  wire [63:0] b1 = $signed(s0) + $signed(64'sd1);
  wire [63:0] ns0 = b1;
  wire [63:0] ns1 = b0;
  always @(posedge clk) begin
    if (rst) begin
      s0 <= 64'sd0;
      s1 <= 64'sd0;
      running <= ($signed(64'sd0) < $signed(inv0));
    end else if (running) begin
      s0 <= ns0;
      s1 <= ns1;
      running <= ($signed(ns0) < $signed(inv0));
    end
  end
  assign done = !running;
  assign result = s1;
endmodule
