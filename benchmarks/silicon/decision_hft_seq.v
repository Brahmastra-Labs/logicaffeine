// decision_hft_seq — the combinational decision (decision_hft) in input/output registers so
// the RTL-to-GDSII flow can TIME it. The reg-to-reg path is the decision's single-cycle latency:
// no PC, no fetch, no dispatch, no loop counter — the whole branchy decision settles in ONE clock.
module decision_hft_seq(input clk, input [63:0] in0, input [63:0] in1, input [63:0] in2, input [63:0] in3, input [63:0] in4, input [63:0] in5, input [63:0] in6, input [63:0] in7, output reg [63:0] result);
  reg [63:0] in0_r;
  reg [63:0] in1_r;
  reg [63:0] in2_r;
  reg [63:0] in3_r;
  reg [63:0] in4_r;
  reg [63:0] in5_r;
  reg [63:0] in6_r;
  reg [63:0] in7_r;
  wire [63:0] y;
  decision_hft u_dec(.in0(in0_r), .in1(in1_r), .in2(in2_r), .in3(in3_r), .in4(in4_r), .in5(in5_r), .in6(in6_r), .in7(in7_r), .result(y));
  always @(posedge clk) begin
    in0_r <= in0;
    in1_r <= in1;
    in2_r <= in2;
    in3_r <= in3;
    in4_r <= in4;
    in5_r <= in5;
    in6_r <= in6;
    in7_r <= in7;
    result <= y;
  end
endmodule
