// keccak_round — one Keccak-f[1600] round (theta, rho+pi, chi, iota).
// GENERATED from the same constants as logicaffeine_system::keccak::keccak_f1600.
module keccak_round(input [1599:0] s, input [63:0] rc, output [1599:0] o);
  wire [63:0] a0 = s[0*64 +: 64];
  wire [63:0] a1 = s[1*64 +: 64];
  wire [63:0] a2 = s[2*64 +: 64];
  wire [63:0] a3 = s[3*64 +: 64];
  wire [63:0] a4 = s[4*64 +: 64];
  wire [63:0] a5 = s[5*64 +: 64];
  wire [63:0] a6 = s[6*64 +: 64];
  wire [63:0] a7 = s[7*64 +: 64];
  wire [63:0] a8 = s[8*64 +: 64];
  wire [63:0] a9 = s[9*64 +: 64];
  wire [63:0] a10 = s[10*64 +: 64];
  wire [63:0] a11 = s[11*64 +: 64];
  wire [63:0] a12 = s[12*64 +: 64];
  wire [63:0] a13 = s[13*64 +: 64];
  wire [63:0] a14 = s[14*64 +: 64];
  wire [63:0] a15 = s[15*64 +: 64];
  wire [63:0] a16 = s[16*64 +: 64];
  wire [63:0] a17 = s[17*64 +: 64];
  wire [63:0] a18 = s[18*64 +: 64];
  wire [63:0] a19 = s[19*64 +: 64];
  wire [63:0] a20 = s[20*64 +: 64];
  wire [63:0] a21 = s[21*64 +: 64];
  wire [63:0] a22 = s[22*64 +: 64];
  wire [63:0] a23 = s[23*64 +: 64];
  wire [63:0] a24 = s[24*64 +: 64];
  wire [63:0] c0 = a0^a5^a10^a15^a20;
  wire [63:0] c1 = a1^a6^a11^a16^a21;
  wire [63:0] c2 = a2^a7^a12^a17^a22;
  wire [63:0] c3 = a3^a8^a13^a18^a23;
  wire [63:0] c4 = a4^a9^a14^a19^a24;
  wire [63:0] d0 = c4 ^ ((c1 << 1) | (c1 >> 63));
  wire [63:0] d1 = c0 ^ ((c2 << 1) | (c2 >> 63));
  wire [63:0] d2 = c1 ^ ((c3 << 1) | (c3 >> 63));
  wire [63:0] d3 = c2 ^ ((c4 << 1) | (c4 >> 63));
  wire [63:0] d4 = c3 ^ ((c0 << 1) | (c0 >> 63));
  wire [63:0] t0 = a0 ^ d0;
  wire [63:0] t1 = a1 ^ d1;
  wire [63:0] t2 = a2 ^ d2;
  wire [63:0] t3 = a3 ^ d3;
  wire [63:0] t4 = a4 ^ d4;
  wire [63:0] t5 = a5 ^ d0;
  wire [63:0] t6 = a6 ^ d1;
  wire [63:0] t7 = a7 ^ d2;
  wire [63:0] t8 = a8 ^ d3;
  wire [63:0] t9 = a9 ^ d4;
  wire [63:0] t10 = a10 ^ d0;
  wire [63:0] t11 = a11 ^ d1;
  wire [63:0] t12 = a12 ^ d2;
  wire [63:0] t13 = a13 ^ d3;
  wire [63:0] t14 = a14 ^ d4;
  wire [63:0] t15 = a15 ^ d0;
  wire [63:0] t16 = a16 ^ d1;
  wire [63:0] t17 = a17 ^ d2;
  wire [63:0] t18 = a18 ^ d3;
  wire [63:0] t19 = a19 ^ d4;
  wire [63:0] t20 = a20 ^ d0;
  wire [63:0] t21 = a21 ^ d1;
  wire [63:0] t22 = a22 ^ d2;
  wire [63:0] t23 = a23 ^ d3;
  wire [63:0] t24 = a24 ^ d4;
  wire [63:0] b0 = t0;
  wire [63:0] b1 = ((t6 << 44) | (t6 >> 20));
  wire [63:0] b2 = ((t12 << 43) | (t12 >> 21));
  wire [63:0] b3 = ((t18 << 21) | (t18 >> 43));
  wire [63:0] b4 = ((t24 << 14) | (t24 >> 50));
  wire [63:0] b5 = ((t3 << 28) | (t3 >> 36));
  wire [63:0] b6 = ((t9 << 20) | (t9 >> 44));
  wire [63:0] b7 = ((t10 << 3) | (t10 >> 61));
  wire [63:0] b8 = ((t16 << 45) | (t16 >> 19));
  wire [63:0] b9 = ((t22 << 61) | (t22 >> 3));
  wire [63:0] b10 = ((t1 << 1) | (t1 >> 63));
  wire [63:0] b11 = ((t7 << 6) | (t7 >> 58));
  wire [63:0] b12 = ((t13 << 25) | (t13 >> 39));
  wire [63:0] b13 = ((t19 << 8) | (t19 >> 56));
  wire [63:0] b14 = ((t20 << 18) | (t20 >> 46));
  wire [63:0] b15 = ((t4 << 27) | (t4 >> 37));
  wire [63:0] b16 = ((t5 << 36) | (t5 >> 28));
  wire [63:0] b17 = ((t11 << 10) | (t11 >> 54));
  wire [63:0] b18 = ((t17 << 15) | (t17 >> 49));
  wire [63:0] b19 = ((t23 << 56) | (t23 >> 8));
  wire [63:0] b20 = ((t2 << 62) | (t2 >> 2));
  wire [63:0] b21 = ((t8 << 55) | (t8 >> 9));
  wire [63:0] b22 = ((t14 << 39) | (t14 >> 25));
  wire [63:0] b23 = ((t15 << 41) | (t15 >> 23));
  wire [63:0] b24 = ((t21 << 2) | (t21 >> 62));
  wire [63:0] o0 = (b0 ^ (~b1 & b2)) ^ rc;
  wire [63:0] o1 = b1 ^ (~b2 & b3);
  wire [63:0] o2 = b2 ^ (~b3 & b4);
  wire [63:0] o3 = b3 ^ (~b4 & b0);
  wire [63:0] o4 = b4 ^ (~b0 & b1);
  wire [63:0] o5 = b5 ^ (~b6 & b7);
  wire [63:0] o6 = b6 ^ (~b7 & b8);
  wire [63:0] o7 = b7 ^ (~b8 & b9);
  wire [63:0] o8 = b8 ^ (~b9 & b5);
  wire [63:0] o9 = b9 ^ (~b5 & b6);
  wire [63:0] o10 = b10 ^ (~b11 & b12);
  wire [63:0] o11 = b11 ^ (~b12 & b13);
  wire [63:0] o12 = b12 ^ (~b13 & b14);
  wire [63:0] o13 = b13 ^ (~b14 & b10);
  wire [63:0] o14 = b14 ^ (~b10 & b11);
  wire [63:0] o15 = b15 ^ (~b16 & b17);
  wire [63:0] o16 = b16 ^ (~b17 & b18);
  wire [63:0] o17 = b17 ^ (~b18 & b19);
  wire [63:0] o18 = b18 ^ (~b19 & b15);
  wire [63:0] o19 = b19 ^ (~b15 & b16);
  wire [63:0] o20 = b20 ^ (~b21 & b22);
  wire [63:0] o21 = b21 ^ (~b22 & b23);
  wire [63:0] o22 = b22 ^ (~b23 & b24);
  wire [63:0] o23 = b23 ^ (~b24 & b20);
  wire [63:0] o24 = b24 ^ (~b20 & b21);
  assign o = {o24, o23, o22, o21, o20, o19, o18, o17, o16, o15, o14, o13, o12, o11, o10, o9, o8, o7, o6, o5, o4, o3, o2, o1, o0};
endmodule

// keccak_core — 24-cycle iterative Keccak-f[1600] (one keccak_round per clock).
module keccak_core(input clk, input rst, input start, input [1599:0] s_in,
                  output reg done, output reg [1599:0] s_out);
  reg [4:0] round;
  reg running;
  reg [1599:0] st;
  reg [63:0] rc;
  wire [1599:0] nxt;
  keccak_round rnd(.s(st), .rc(rc), .o(nxt));
  always @(*) begin
    case (round)
      5'd0: rc = 64'h0000000000000001;
      5'd1: rc = 64'h0000000000008082;
      5'd2: rc = 64'h800000000000808a;
      5'd3: rc = 64'h8000000080008000;
      5'd4: rc = 64'h000000000000808b;
      5'd5: rc = 64'h0000000080000001;
      5'd6: rc = 64'h8000000080008081;
      5'd7: rc = 64'h8000000000008009;
      5'd8: rc = 64'h000000000000008a;
      5'd9: rc = 64'h0000000000000088;
      5'd10: rc = 64'h0000000080008009;
      5'd11: rc = 64'h000000008000000a;
      5'd12: rc = 64'h000000008000808b;
      5'd13: rc = 64'h800000000000008b;
      5'd14: rc = 64'h8000000000008089;
      5'd15: rc = 64'h8000000000008003;
      5'd16: rc = 64'h8000000000008002;
      5'd17: rc = 64'h8000000000000080;
      5'd18: rc = 64'h000000000000800a;
      5'd19: rc = 64'h800000008000000a;
      5'd20: rc = 64'h8000000080008081;
      5'd21: rc = 64'h8000000000008080;
      5'd22: rc = 64'h0000000080000001;
      5'd23: rc = 64'h8000000080008008;
      default: rc = 64'h0;
    endcase
  end
  always @(posedge clk) begin
    if (rst) begin running <= 1'b0; done <= 1'b0; round <= 5'd0; end
    else begin
      done <= 1'b0;
      if (start) begin st <= s_in; round <= 5'd0; running <= 1'b1; end
      else if (running) begin
        st <= nxt;
        if (round == 5'd23) begin running <= 1'b0; done <= 1'b1; s_out <= nxt; end
        else round <= round + 5'd1;
      end
    end
  end
endmodule

// keccak_top — narrow-bus (64-bit lane) wrapper so Keccak-f[1600] place-and-routes.
module keccak_top(input clk, input rst, input we, input [4:0] idx,
                 input [63:0] din, input go, output done, output [63:0] dout);
  reg [1599:0] state;
  reg launch;
  wire core_done;
  wire [1599:0] result;
  keccak_core core(.clk(clk), .rst(rst), .start(launch), .s_in(state),
                   .done(core_done), .s_out(result));
  assign done = core_done;
  assign dout = state[64*idx +: 64];
  always @(posedge clk) begin
    if (rst) begin launch <= 1'b0; end
    else begin
      launch <= go;
      if (we) state[64*idx +: 64] <= din;
      else if (core_done) state <= result;
    end
  end
endmodule
