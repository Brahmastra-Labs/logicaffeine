// decision_hft_bist — an on-chip BIST harness that characterizes the decision datapath's ROUTED
// power / area / timing WITHOUT the pad-limited 577-pin I/O explosion of the raw 8x64-bit interface.
// A 64-bit xorshift LFSR drives the eight decision inputs with high-activity (conservative) stimulus;
// the 64-bit result folds into a signature register. Only clk + rst + an 8-bit signature leave the
// block (~10 pins), so it places at high utilization and signs off DRC/LVS/XOR-clean. The decision
// logic is preserved intact (its full output feeds the signature, so synthesis can't fold it away);
// the Combinational power group in the report IS the decision select-DAG (the LFSR/MISR are a small
// sequential overhead). decision_hft is proven ==VM by hft_decision_fabric_lock + tb_decision_hft.
module decision_hft_bist(input clk, input rst, output reg [7:0] sig);
  reg  [63:0] lfsr;
  reg  [63:0] misr;
  wire [63:0] y;

  // eight decorrelated stimulus streams from the single LFSR state
  wire [63:0] i0 = lfsr;
  wire [63:0] i1 = {lfsr[31:0], lfsr[63:32]};
  wire [63:0] i2 = lfsr ^ 64'h9E3779B97F4A7C15;
  wire [63:0] i3 = {lfsr[15:0], lfsr[63:16]};
  wire [63:0] i4 = ~lfsr;
  wire [63:0] i5 = lfsr + 64'd10050;
  wire [63:0] i6 = {lfsr[47:0], lfsr[63:48]};
  wire [63:0] i7 = lfsr ^ {lfsr[0], lfsr[63:1]};

  decision_hft u_dec(.in0(i0), .in1(i1), .in2(i2), .in3(i3),
                     .in4(i4), .in5(i5), .in6(i6), .in7(i7), .result(y));

  function [63:0] nxt(input [63:0] s);
    reg [63:0] x;
    begin
      x = s;
      x = x ^ (x << 13);
      x = x ^ (x >> 7);
      x = x ^ (x << 17);
      nxt = x;
    end
  endfunction

  always @(posedge clk) begin
    if (rst) begin
      lfsr <= 64'h1234_5678_9ABC_DEF1;
      misr <= 64'd0;
      sig  <= 8'd0;
    end else begin
      lfsr <= nxt(lfsr);
      misr <= {misr[62:0], ^misr} ^ y;
      sig  <= misr[7:0];
    end
  end
endmodule
