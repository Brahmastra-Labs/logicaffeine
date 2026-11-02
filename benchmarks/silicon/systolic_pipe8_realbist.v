module systolic_pipe8_realbist(input clk, input rst, output sig);
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
  wire signed [63:0] o0;
  wire signed [63:0] o1;
  wire signed [63:0] o2;
  wire signed [63:0] o3;
  wire signed [63:0] o4;
  wire signed [63:0] o5;
  wire signed [63:0] o6;
  wire signed [63:0] o7;
  systolic_pipe8 arr(.clk(clk), .rst(rst), .a0(a0), .a1(a1), .a2(a2), .a3(a3), .a4(a4), .a5(a5), .a6(a6), .a7(a7), .o0(o0), .o1(o1), .o2(o2), .o3(o3), .o4(o4), .o5(o5), .o6(o6), .o7(o7));
  always @(posedge clk) begin
    if (rst) begin
      lfsr <= 64'hD1CEF00D12345678;
      misr <= 64'd0;
    end else begin
      lfsr <= {lfsr[62:0], lfsr[63]^lfsr[62]^lfsr[60]^lfsr[59]};
      misr <= {misr[62:0], misr[63]} ^ (o0 ^ o1 ^ o2 ^ o3 ^ o4 ^ o5 ^ o6 ^ o7);
    end
  end
  assign sig = ^misr;
endmodule
