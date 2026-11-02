// cgra_arr8 — a compiled COUNTED LOOP as a CGRA, pipelined: the body's terms compute from the induction
// counter and the invariants through 2 register stage(s); each accumulator holds sum and carry words
// that absorb them through 3:2 compressors, so neither recurrence carries a carry chain. 64-bit, so it wraps
// identically to the VM's Int; a loop whose body runs finishes 2 clock(s) after the counter does.
module cgra_arr8(input clk, input rst, input [63:0] in0, output [63:0] result, output done);
  reg [63:0] s0_sum, s0_carry;
  reg [63:0] t0_0;
  reg [63:0] t0_1;
  reg [63:0] t0_2;
  reg [63:0] t0_3;
  reg [63:0] t0_4;
  reg [63:0] t0_5;
  reg [63:0] t0_6;
  reg [63:0] t0_7;
  reg [63:0] iv;
  reg running;
  reg [1:0] live;
  reg [63:0] iv_d1;
  reg [63:0] b0_d1;
  reg [63:0] b3_d1;
  reg [63:0] b5_d1;
  reg [63:0] b8_d1;
  reg [63:0] b10_d1;
  reg [63:0] b13_d1;
  reg [63:0] b15_d1;
  reg [63:0] b18_d1;
  reg [63:0] b20_d1;
  reg [63:0] b23_d1;
  reg [63:0] b25_d1;
  reg [63:0] b28_d1;
  reg [63:0] b30_d1;
  reg [63:0] b33_d1;
  reg [63:0] b35_d1;
  wire [63:0] lo = 64'sd1;
  wire [63:0] hi = in0;
  wire [63:0] inv0 = in0;
  wire [63:0] b0 = $signed(inv0) - $signed(iv);
  wire signed [7:0] ma1 = iv_d1;
  wire signed [7:0] mb1 = b0_d1;
  wire signed [15:0] mp1 = ma1 * mb1;
  wire [63:0] b1 = mp1;
  wire [63:0] b3 = $signed(iv) + $signed(64'sd1);
  wire [63:0] b4 = $signed(inv0) - $signed(iv);
  wire [63:0] b5 = $signed(b4) - $signed(64'sd1);
  wire signed [7:0] ma6 = b3_d1;
  wire signed [7:0] mb6 = b5_d1;
  wire signed [15:0] mp6 = ma6 * mb6;
  wire [63:0] b6 = mp6;
  wire [63:0] b8 = $signed(iv) + $signed(64'sd2);
  wire [63:0] b9 = $signed(inv0) - $signed(iv);
  wire [63:0] b10 = $signed(b9) - $signed(64'sd2);
  wire signed [7:0] ma11 = b8_d1;
  wire signed [7:0] mb11 = b10_d1;
  wire signed [15:0] mp11 = ma11 * mb11;
  wire [63:0] b11 = mp11;
  wire [63:0] b13 = $signed(iv) + $signed(64'sd3);
  wire [63:0] b14 = $signed(inv0) - $signed(iv);
  wire [63:0] b15 = $signed(b14) - $signed(64'sd3);
  wire signed [7:0] ma16 = b13_d1;
  wire signed [7:0] mb16 = b15_d1;
  wire signed [15:0] mp16 = ma16 * mb16;
  wire [63:0] b16 = mp16;
  wire [63:0] b18 = $signed(iv) + $signed(64'sd4);
  wire [63:0] b19 = $signed(inv0) - $signed(iv);
  wire [63:0] b20 = $signed(b19) - $signed(64'sd4);
  wire signed [7:0] ma21 = b18_d1;
  wire signed [7:0] mb21 = b20_d1;
  wire signed [15:0] mp21 = ma21 * mb21;
  wire [63:0] b21 = mp21;
  wire [63:0] b23 = $signed(iv) + $signed(64'sd5);
  wire [63:0] b24 = $signed(inv0) - $signed(iv);
  wire [63:0] b25 = $signed(b24) - $signed(64'sd5);
  wire signed [7:0] ma26 = b23_d1;
  wire signed [7:0] mb26 = b25_d1;
  wire signed [15:0] mp26 = ma26 * mb26;
  wire [63:0] b26 = mp26;
  wire [63:0] b28 = $signed(iv) + $signed(64'sd6);
  wire [63:0] b29 = $signed(inv0) - $signed(iv);
  wire [63:0] b30 = $signed(b29) - $signed(64'sd6);
  wire signed [7:0] ma31 = b28_d1;
  wire signed [7:0] mb31 = b30_d1;
  wire signed [15:0] mp31 = ma31 * mb31;
  wire [63:0] b31 = mp31;
  wire [63:0] b33 = $signed(iv) + $signed(64'sd7);
  wire [63:0] b34 = $signed(inv0) - $signed(iv);
  wire [63:0] b35 = $signed(b34) - $signed(64'sd7);
  wire signed [7:0] ma36 = b33_d1;
  wire signed [7:0] mb36 = b35_d1;
  wire signed [15:0] mp36 = ma36 * mb36;
  wire [63:0] b36 = mp36;
  wire [63:0] s0_l0_0_x = s0_sum ^ s0_carry;
  wire [63:0] s0_l0_0_s = s0_l0_0_x ^ t0_0;
  wire [63:0] s0_l0_0_c = ((s0_sum & s0_carry) | (t0_0 & s0_l0_0_x)) << 1;
  wire [63:0] s0_l0_1_x = t0_1 ^ t0_2;
  wire [63:0] s0_l0_1_s = s0_l0_1_x ^ t0_3;
  wire [63:0] s0_l0_1_c = ((t0_1 & t0_2) | (t0_3 & s0_l0_1_x)) << 1;
  wire [63:0] s0_l0_2_x = t0_4 ^ t0_5;
  wire [63:0] s0_l0_2_s = s0_l0_2_x ^ t0_6;
  wire [63:0] s0_l0_2_c = ((t0_4 & t0_5) | (t0_6 & s0_l0_2_x)) << 1;
  wire [63:0] s0_l1_0_x = s0_l0_0_s ^ s0_l0_0_c;
  wire [63:0] s0_l1_0_s = s0_l1_0_x ^ s0_l0_1_s;
  wire [63:0] s0_l1_0_c = ((s0_l0_0_s & s0_l0_0_c) | (s0_l0_1_s & s0_l1_0_x)) << 1;
  wire [63:0] s0_l1_1_x = s0_l0_1_c ^ s0_l0_2_s;
  wire [63:0] s0_l1_1_s = s0_l1_1_x ^ s0_l0_2_c;
  wire [63:0] s0_l1_1_c = ((s0_l0_1_c & s0_l0_2_s) | (s0_l0_2_c & s0_l1_1_x)) << 1;
  wire [63:0] s0_l2_0_x = s0_l1_0_s ^ s0_l1_0_c;
  wire [63:0] s0_l2_0_s = s0_l2_0_x ^ s0_l1_1_s;
  wire [63:0] s0_l2_0_c = ((s0_l1_0_s & s0_l1_0_c) | (s0_l1_1_s & s0_l2_0_x)) << 1;
  wire [63:0] s0_l3_0_x = s0_l2_0_s ^ s0_l2_0_c;
  wire [63:0] s0_l3_0_s = s0_l3_0_x ^ s0_l1_1_c;
  wire [63:0] s0_l3_0_c = ((s0_l2_0_s & s0_l2_0_c) | (s0_l1_1_c & s0_l3_0_x)) << 1;
  wire [63:0] s0_l4_0_x = s0_l3_0_s ^ s0_l3_0_c;
  wire [63:0] s0_l4_0_s = s0_l4_0_x ^ t0_7;
  wire [63:0] s0_l4_0_c = ((s0_l3_0_s & s0_l3_0_c) | (t0_7 & s0_l4_0_x)) << 1;
  wire [63:0] s0_sum_next = s0_l4_0_s;
  wire [63:0] s0_carry_next = s0_l4_0_c;
  wire [63:0] ivp0 = iv;
  wire [63:0] ivp1 = ivp0 & {ivp0[62:0], 1'b1};
  wire [63:0] ivp2 = ivp1 & {ivp1[61:0], 2'b11};
  wire [63:0] ivp3 = ivp2 & {ivp2[59:0], 4'b1111};
  wire [63:0] ivp4 = ivp3 & {ivp3[55:0], 8'b11111111};
  wire [63:0] ivp5 = ivp4 & {ivp4[47:0], 16'b1111111111111111};
  wire [63:0] ivp6 = ivp5 & {ivp5[31:0], 32'b11111111111111111111111111111111};
  wire [63:0] iv_next = iv ^ {ivp6[62:0], 1'b1};
  wire [63:0] s0 = s0_sum + s0_carry;
  always @(posedge clk) begin
    if (rst) begin
      s0_sum <= 64'sd0; s0_carry <= 64'd0;
      iv <= lo;
      running <= ($signed(lo) <= $signed(hi));
      live <= {2{1'b0}};
    end else begin
      live <= {live[0:0], running};
      iv_d1 <= iv;
      b0_d1 <= b0;
      b3_d1 <= b3;
      b5_d1 <= b5;
      b8_d1 <= b8;
      b10_d1 <= b10;
      b13_d1 <= b13;
      b15_d1 <= b15;
      b18_d1 <= b18;
      b20_d1 <= b20;
      b23_d1 <= b23;
      b25_d1 <= b25;
      b28_d1 <= b28;
      b30_d1 <= b30;
      b33_d1 <= b33;
      b35_d1 <= b35;
      t0_0 <= b1;
      t0_1 <= b6;
      t0_2 <= b11;
      t0_3 <= b16;
      t0_4 <= b21;
      t0_5 <= b26;
      t0_6 <= b31;
      t0_7 <= b36;
      if (live[1]) begin
        s0_sum <= s0_sum_next; s0_carry <= s0_carry_next;
      end
      if (running) begin
        if ($signed(iv) < $signed(hi)) iv <= iv_next;
        else running <= 1'b0;
      end
    end
  end
  assign done = !running && !(|live);
  assign result = s0;
endmodule

module matmul_array_top(input clk, input rst, input serin, output reg serout);
  reg [63:0] in0 = 64'd0;
  wire [63:0] result; wire done;
  reg [63:0] acc = 64'd0;
  cgra_arr8 dut(.clk(clk), .rst(rst), .in0(in0), .result(result), .done(done));
  always @(posedge clk) begin
    in0 <= {in0[62:0], serin};
    if (done) acc <= result;
    serout <= ^acc;
  end
endmodule
