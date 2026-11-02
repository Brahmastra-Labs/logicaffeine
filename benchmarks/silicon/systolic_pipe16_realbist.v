module systolic_pipe16_realbist(input clk, input rst, output sig);
  reg [63:0] lfsr;
  reg [63:0] misr;
  wire signed [63:0] a0 = $signed(lfsr[7:0]);
  wire signed [63:0] a1 = $signed(lfsr[15:8]);
  wire signed [63:0] a2 = $signed(lfsr[23:16]);
  wire signed [63:0] a3 = $signed(lfsr[31:24]);
  wire signed [63:0] a4 = $signed(lfsr[39:32]);
  wire signed [63:0] a5 = $signed(lfsr[47:40]);
  wire signed [63:0] a6 = $signed(lfsr[55:48]);
  wire signed [63:0] a7 = $signed(lfsr[63:56]);
  wire signed [63:0] a8 = $signed(lfsr[14:7]);
  wire signed [63:0] a9 = $signed(lfsr[22:15]);
  wire signed [63:0] a10 = $signed(lfsr[30:23]);
  wire signed [63:0] a11 = $signed(lfsr[38:31]);
  wire signed [63:0] a12 = $signed(lfsr[46:39]);
  wire signed [63:0] a13 = $signed(lfsr[54:47]);
  wire signed [63:0] a14 = $signed(lfsr[62:55]);
  wire signed [63:0] a15 = $signed(lfsr[13:6]);
  wire signed [63:0] o0;
  wire signed [63:0] o1;
  wire signed [63:0] o2;
  wire signed [63:0] o3;
  wire signed [63:0] o4;
  wire signed [63:0] o5;
  wire signed [63:0] o6;
  wire signed [63:0] o7;
  wire signed [63:0] o8;
  wire signed [63:0] o9;
  wire signed [63:0] o10;
  wire signed [63:0] o11;
  wire signed [63:0] o12;
  wire signed [63:0] o13;
  wire signed [63:0] o14;
  wire signed [63:0] o15;
  systolic_pipe16 arr(.clk(clk), .rst(rst), .a0(a0), .a1(a1), .a2(a2), .a3(a3), .a4(a4), .a5(a5), .a6(a6), .a7(a7), .a8(a8), .a9(a9), .a10(a10), .a11(a11), .a12(a12), .a13(a13), .a14(a14), .a15(a15), .o0(o0), .o1(o1), .o2(o2), .o3(o3), .o4(o4), .o5(o5), .o6(o6), .o7(o7), .o8(o8), .o9(o9), .o10(o10), .o11(o11), .o12(o12), .o13(o13), .o14(o14), .o15(o15));
  always @(posedge clk) begin
    if (rst) begin
      lfsr <= 64'hD1CEF00D12345678;
      misr <= 64'd0;
    end else begin
      lfsr <= {lfsr[62:0], lfsr[63]^lfsr[62]^lfsr[60]^lfsr[59]};
      misr <= {misr[62:0], misr[63]} ^ (o0 ^ o1 ^ o2 ^ o3 ^ o4 ^ o5 ^ o6 ^ o7 ^ o8 ^ o9 ^ o10 ^ o11 ^ o12 ^ o13 ^ o14 ^ o15);
    end
  end
  assign sig = ^misr;
endmodule
