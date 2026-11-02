// nested_nsum — a compiled NESTED LOOP as a HIERARCHICAL CGRA: an outer counter that
// runs the INNER loop (its own body datapath + counter) to completion each iteration; the
// inner range may depend on the outer induction variable. Futamura P1 for nested control
// flow; 64-bit so it wraps like the VM's Int; the inner body-step is certified == the VM by
// our own SAT, and the nested loop is sound by double induction.
module nested_nsum(input clk, input rst, input [63:0] in0, output [63:0] result, output done);
  localparam SETUP = 2'd0, RUN = 2'd1, DONE = 2'd2;
  reg [1:0] st;
  reg [63:0] s0;
  reg [63:0] oi, ij;
  wire [63:0] olo = 64'sd1;
  wire [63:0] ohi = in0;
  wire [63:0] ilo = 64'sd1;
  wire [63:0] ihi = in0;
  wire [63:0] b0 = $signed(s0) + $signed(64'sd1);
  wire [63:0] ns0 = b0;
  always @(posedge clk) begin
    if (rst) begin
      s0 <= 64'sd0;
      oi <= olo;
      st <= ($signed(olo) <= $signed(ohi)) ? SETUP : DONE;
    end else if (st == SETUP) begin
      ij <= ilo;
      if ($signed(ilo) <= $signed(ihi)) st <= RUN;
      else if ($signed(oi) < $signed(ohi)) oi <= oi + 64'sd1;
      else st <= DONE;
    end else if (st == RUN) begin
        s0 <= ns0;
      if ($signed(ij) < $signed(ihi)) ij <= ij + 64'sd1;
      else if ($signed(oi) < $signed(ohi)) begin oi <= oi + 64'sd1; st <= SETUP; end
      else st <= DONE;
    end
  end
  assign done = (st == DONE);
  assign result = s0;
endmodule
