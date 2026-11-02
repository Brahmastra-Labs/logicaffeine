module systolic8(
  input clk,
  input rst,
  input signed [63:0] a0,
  input signed [63:0] a1,
  input signed [63:0] a2,
  input signed [63:0] a3,
  input signed [63:0] a4,
  input signed [63:0] a5,
  input signed [63:0] a6,
  input signed [63:0] a7,
  output reg signed [63:0] o0,
  output reg signed [63:0] o1,
  output reg signed [63:0] o2,
  output reg signed [63:0] o3,
  output reg signed [63:0] o4,
  output reg signed [63:0] o5,
  output reg signed [63:0] o6,
  output reg signed [63:0] o7
);
  wire signed [63:0] sa0 = $signed(a0[7:0]);
  wire signed [63:0] sa1 = $signed(a1[7:0]);
  wire signed [63:0] sa2 = $signed(a2[7:0]);
  wire signed [63:0] sa3 = $signed(a3[7:0]);
  wire signed [63:0] sa4 = $signed(a4[7:0]);
  wire signed [63:0] sa5 = $signed(a5[7:0]);
  wire signed [63:0] sa6 = $signed(a6[7:0]);
  wire signed [63:0] sa7 = $signed(a7[7:0]);
  always @(posedge clk) begin
    if (rst) begin
      o0 <= 64'sd0;
      o1 <= 64'sd0;
      o2 <= 64'sd0;
      o3 <= 64'sd0;
      o4 <= 64'sd0;
      o5 <= 64'sd0;
      o6 <= 64'sd0;
      o7 <= 64'sd0;
    end else begin
      o0 <= sa0 * 64'shFFFFFFFFFFFFFF82 + sa1 * 64'sh0000000000000053 + sa2 * 64'sh000000000000001F + sa3 * 64'shFFFFFFFFFFFFFFD2 + sa4 * 64'shFFFFFFFFFFFFFFC7 + sa5 * 64'shFFFFFFFFFFFFFFE5 + sa6 * 64'sh000000000000001D + sa7 * 64'sh0000000000000036;
      o1 <= sa0 * 64'shFFFFFFFFFFFFFF9E + sa1 * 64'shFFFFFFFFFFFFFFCD + sa2 * 64'shFFFFFFFFFFFFFFAB + sa3 * 64'sh0000000000000053 + sa4 * 64'sh000000000000000D + sa5 * 64'shFFFFFFFFFFFFFFE5 + sa6 * 64'shFFFFFFFFFFFFFFB5 + sa7 * 64'sh000000000000007D;
      o2 <= sa0 * 64'shFFFFFFFFFFFFFFB4 + sa1 * 64'sh0000000000000018 + sa2 * 64'sh0000000000000022 + sa3 * 64'sh000000000000005B + sa4 * 64'shFFFFFFFFFFFFFFCE + sa5 * 64'shFFFFFFFFFFFFFFD1 + sa6 * 64'sh0000000000000029 + sa7 * 64'shFFFFFFFFFFFFFFB1;
      o3 <= sa0 * 64'sh000000000000006F + sa1 * 64'shFFFFFFFFFFFFFFD5 + sa2 * 64'sh0000000000000048 + sa3 * 64'shFFFFFFFFFFFFFFFC + sa4 * 64'sh000000000000007C + sa5 * 64'sh000000000000001B + sa6 * 64'sh0000000000000050 + sa7 * 64'shFFFFFFFFFFFFFFA7;
      o4 <= sa0 * 64'shFFFFFFFFFFFFFFE9 + sa1 * 64'sh0000000000000028 + sa2 * 64'shFFFFFFFFFFFFFFB2 + sa3 * 64'shFFFFFFFFFFFFFFDA + sa4 * 64'shFFFFFFFFFFFFFFC6 + sa5 * 64'shFFFFFFFFFFFFFF9D + sa6 * 64'sh0000000000000039 + sa7 * 64'shFFFFFFFFFFFFFFD8;
      o5 <= sa0 * 64'sh000000000000004B + sa1 * 64'shFFFFFFFFFFFFFFED + sa2 * 64'sh0000000000000024 + sa3 * 64'shFFFFFFFFFFFFFF9C + sa4 * 64'sh0000000000000005 + sa5 * 64'shFFFFFFFFFFFFFF94 + sa6 * 64'shFFFFFFFFFFFFFFBB + sa7 * 64'sh0000000000000022;
      o6 <= sa0 * 64'shFFFFFFFFFFFFFFD8 + sa1 * 64'shFFFFFFFFFFFFFFA9 + sa2 * 64'sh0000000000000012 + sa3 * 64'sh0000000000000027 + sa4 * 64'sh0000000000000009 + sa5 * 64'shFFFFFFFFFFFFFFBE + sa6 * 64'sh0000000000000061 + sa7 * 64'shFFFFFFFFFFFFFFC8;
      o7 <= sa0 * 64'shFFFFFFFFFFFFFF9C + sa1 * 64'shFFFFFFFFFFFFFFDB + sa2 * 64'shFFFFFFFFFFFFFFA6 + sa3 * 64'shFFFFFFFFFFFFFFAB + sa4 * 64'sh000000000000003D + sa5 * 64'shFFFFFFFFFFFFFFF7 + sa6 * 64'sh0000000000000073 + sa7 * 64'shFFFFFFFFFFFFFF8A;
    end
  end
endmodule
