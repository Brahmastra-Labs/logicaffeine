//! The activation quantizer ([`crate::spatial::decode_engine::numerics::quantize`]) as Verilog: a block of 32 f32
//! values to its 4-bit-significand scale and its codes.
//!
//! The scale has a closed form in the block's largest magnitude `S·2^(E−23)` (`S` its 24-bit significand): with
//! `m₁ = ⌈⌈S/2¹³⌉/127⌉`, the scale is `m₁·2^(E−10)` when `m₁ ≤ 15`, else `⌈⌈S/2¹⁴⌉/127⌉·2^(E−9)`, a significand of 8
//! or 9 ([`closed_form_scale`], held to `block_scale` over every significand). A code is the exact quotient of its
//! value by the scale, carried with 32 fraction bits and a remainder, rounded to nearest with ties to even.

use crate::spatial::decode_engine::numerics::ActScale;

/// The per-block scale's closed form ([`crate::spatial::decode_engine::numerics::wide_scale`], an 8-bit significand),
/// for a normal `amax` with significand `s` (24 bits) and unbiased exponent `e`: with `m₁ = ⌈⌈S/2⁹⌉/127⌉`, the scale is
/// `m₁·2^(e−14)` when `m₁ ≤ 255`, else `⌈⌈S/2¹⁰⌉/127⌉·2^(e−13)`, a significand of 128 to 130.
pub fn closed_form_wide_scale(s: u32, e: i32) -> ActScale {
    let m1 = ((s + 511) >> 9).div_ceil(127) as i64;
    if m1 <= 255 {
        ActScale { m: m1, e: e - 14 }
    } else {
        ActScale { m: ((s + 1023) >> 10).div_ceil(127) as i64, e: e - 13 }
    }
}

/// Clocks from `logos_wide_scale_p`'s operands to their scale.
pub const WIDE_SCALE_P_LATENCY: usize = 3;

/// The block scale's closed form, for a normal `amax` with significand `s` (24 bits) and unbiased exponent `e`.
pub fn closed_form_scale(s: u32, e: i32) -> ActScale {
    let m1 = ((s + 8191) >> 13).div_ceil(127) as i64;
    if m1 <= 15 {
        ActScale { m: m1, e: e - 10 }
    } else {
        ActScale { m: ((s + 16383) >> 14).div_ceil(127) as i64, e: e - 9 }
    }
}

/// `logos_act_block(x, two) -> (m, e, codes)`: 32 f32 values packed little-end first in `x`; `two` selects the two-word
/// codes; `codes` packs 32 signed 16-bit codes; `e` is the scale's exponent, signed. `logos_act_wide #(LANES)` is the
/// per-block quantizer over a block of `LANES` values: the same ports, its `m` eight bits. `logos_act_stream #(LANES)`
/// is `logos_act_wide` over a block arriving sixteen values a beat, with sixteen code units: one quantizer for every
/// block length up to `LANES`, its beats held in block RAM. `logos_wide_scale_p` is `logos_wide_scale` for the user
/// clock, its scale [`WIDE_SCALE_P_LATENCY`] clocks after its operands.
pub fn act_quant_verilog() -> String {
    format!("{}\n{}", super::ram::ram_verilog(&[512]), ACT_QUANT_VERILOG)
}

const ACT_QUANT_VERILOG: &str = r#"// One activation block: its 4-bit-significand scale and its exact codes.
module logos_act_code(input [31:0] x, input [7:0] m, input signed [11:0] e, input two, output reg signed [15:0] code);
  wire        neg = x[31];
  wire [7:0]  ex = x[30:23];
  wire [23:0] sx = (ex == 8'd0) ? 24'd0 : {1'b1, x[22:0]};
  // The value's place against the scale's: x / (m·2^e) = sx·2^k / m, k = (ex − 150) − e; carried 32 places finer.
  wire signed [12:0] k = $signed({5'd0, ex}) - 13'sd150 - e;
  wire signed [12:0] up32 = k + 13'sd32;
  wire [63:0] num = (up32 < 13'sd0) ? 64'd0 : (up32 > 13'sd39) ? 64'hffff_ffff_ffff_ffff : ({40'd0, sx} << up32[5:0]);
  wire [63:0] q = num / {56'd0, m};
  wire [63:0] rem = num % {56'd0, m};
  wire [31:0] whole = q[63:32];
  wire [31:0] frac = q[31:0];
  wire        up = (frac > 32'h8000_0000) || (frac == 32'h8000_0000 && (rem != 64'd0 || whole[0]));
  wire [31:0] n = whole + {31'd0, up};
  wire [31:0] lim = two ? 32'd32767 : 32'd127;
  wire [31:0] c = (n > lim) ? lim : n;
  always @* code = neg ? -$signed({1'b0, c[14:0]}) : $signed({1'b0, c[14:0]});
endmodule

module logos_act_block(input [1023:0] x, input two, output reg [3:0] m, output reg signed [11:0] e, output [511:0] codes);
  // The largest magnitude, a denormal read as zero.
  reg [30:0] amax;
  integer i;
  always @* begin
    amax = 31'd0;
    for (i = 0; i < 32; i = i + 1)
      if (x[32*i+30 -: 8] != 8'd0 && x[32*i+30 -: 31] > amax) amax = x[32*i+30 -: 31];
  end
  wire [23:0] s = {1'b1, amax[22:0]};
  wire signed [11:0] eu = $signed({4'd0, amax[30:23]}) - 12'sd127;
  wire [24:0] r13 = ({1'b0, s} + 25'd8191) >> 13;
  wire [11:0] c13 = r13[11:0];
  wire [11:0] m1 = (c13 + 12'd126) / 12'd127;
  wire [24:0] r14 = ({1'b0, s} + 25'd16383) >> 14;
  wire [11:0] c14 = r14[11:0];
  wire [11:0] m2 = (c14 + 12'd126) / 12'd127;
  always @* begin
    if (amax == 31'd0) begin m = 4'd8; e = 12'sd0; end
    else if (m1 <= 12'd15) begin m = m1[3:0]; e = eu - 12'sd10 - (two ? 12'sd8 : 12'sd0); end
    else begin m = m2[3:0]; e = eu - 12'sd9 - (two ? 12'sd8 : 12'sd0); end
  end
  genvar g;
  generate
    for (g = 0; g < 32; g = g + 1) begin : lane
      wire signed [15:0] c;
      logos_act_code u(.x(x[32*g +: 32]), .m({4'd0, m}), .e(e), .two(two), .code(c));
      assign codes[16*g +: 16] = (amax == 31'd0) ? 16'd0 : c;
    end
  endgenerate
endmodule

// The per-block scale of a block whose largest magnitude is `amax` (the bits below the sign, a denormal read as zero):
// wide_scale's 8-bit significand, its exponent eight lower for two-word codes.
module logos_wide_scale(input [30:0] amax, input two, output reg [7:0] m, output reg signed [11:0] e);
  wire [23:0] s = {1'b1, amax[22:0]};
  wire signed [11:0] eu = $signed({4'd0, amax[30:23]}) - 12'sd127;
  wire [24:0] r9 = ({1'b0, s} + 25'd511) >> 9;
  wire [15:0] c9 = r9[15:0];
  wire [24:0] r10 = ({1'b0, s} + 25'd1023) >> 10;
  wire [15:0] c10 = r10[15:0];
  // Each ceiling over 127 as a product and a shift, no divider: (x·33027) >> 22 is ⌊x/127⌋ for every x = c + 126, c at
  // most 2^15.
  wire [31:0] p1 = ({16'd0, c9} + 32'd126) * 32'd33027;
  wire [31:0] p2 = ({16'd0, c10} + 32'd126) * 32'd33027;
  wire [15:0] m1 = {6'd0, p1[31:22]};
  wire [15:0] m2 = {6'd0, p2[31:22]};
  always @* begin
    if (amax == 31'd0) begin m = 8'd128; e = 12'sd0; end
    else if (m1 <= 16'd255) begin m = m1[7:0]; e = eu - 12'sd14 - (two ? 12'sd8 : 12'sd0); end
    else begin m = m2[7:0]; e = eu - 12'sd13 - (two ? 12'sd8 : 12'sd0); end
  end
endmodule

// logos_wide_scale for the user clock, an operand a clock: m and e are logos_wide_scale's of the amax and two three
// clocks before. Each ceiling's operand ⌈S/2^k⌉ + 126 by one addition (126·2^k added before the shift); its product by
// 33027, whose top ten bits are the quotient by 127; the choice between the two.
module logos_wide_scale_p(input clk, input [30:0] amax, input two, output reg [7:0] m, output reg signed [11:0] e);
  wire [24:0] s = {2'b01, amax[22:0]};
  wire [24:0] r9 = (s + 25'd65023) >> 9;
  wire [24:0] r10 = (s + 25'd130047) >> 10;
  reg [15:0] x1, x2;
  reg signed [11:0] eu1, eu2;
  reg z1, z2;
  always @(posedge clk) begin
    x1 <= r9[15:0];
    x2 <= r10[15:0];
    eu1 <= $signed({4'd0, amax[30:23]}) - (two ? 12'sd135 : 12'sd127);
    z1 <= amax == 31'd0;
  end
  reg [31:0] p1, p2;
  always @(posedge clk) begin
    p1 <= {16'd0, x1} * 32'd33027;
    p2 <= {16'd0, x2} * 32'd33027;
    eu2 <= eu1;
    z2 <= z1;
  end
  always @(posedge clk) begin
    if (z2) begin m <= 8'd128; e <= 12'sd0; end
    else if (p1[31:30] == 2'd0) begin m <= p1[29:22]; e <= eu2 - 12'sd14; end
    else begin m <= p2[29:22]; e <= eu2 - 12'sd13; end
  end
endmodule

// One weight block's activations under one 8-bit-significand scale.
module logos_act_wide #(parameter LANES = 32)
  (input [32*LANES-1:0] x, input two, output [7:0] m, output signed [11:0] e, output [16*LANES-1:0] codes);
  reg [30:0] amax;
  integer i;
  always @* begin
    amax = 31'd0;
    for (i = 0; i < LANES; i = i + 1)
      if (x[32*i+30 -: 8] != 8'd0 && x[32*i+30 -: 31] > amax) amax = x[32*i+30 -: 31];
  end
  logos_wide_scale sc(.amax(amax), .two(two), .m(m), .e(e));
  genvar g;
  generate
    for (g = 0; g < LANES; g = g + 1) begin : lane
      wire signed [15:0] c;
      logos_act_code u(.x(x[32*g +: 32]), .m(m), .e(e), .two(two), .code(c));
      assign codes[16*g +: 16] = (amax == 31'd0) ? 16'd0 : c;
    end
  endgenerate
endmodule

// logos_act_wide over a block that arrives sixteen values a beat, LANES a multiple of sixteen. `in_v` comes with each
// beat and `clear` with a block's first: the block's largest magnitude is kept as its beats arrive and the beats are
// held in block RAM. `go`, a cycle after the last beat, latches the scale; the sixteen code units then make a beat's
// codes a cycle into `codes`, `busy` until the last beat's. A block of fewer beats leaves the codes above its own as
// they were; the next block's beats come once `busy` falls.
module logos_act_stream #(parameter LANES = 256)
  (input clk, input rst, input clear, input in_v, input [511:0] in_beat, input go, input two,
   output reg busy, output reg [7:0] m, output reg signed [11:0] e, output [16*LANES-1:0] codes);
  localparam BEATS = LANES / 16;
  localparam BA = (BEATS > 1) ? $clog2(BEATS) : 1;
  // The beat's largest magnitude, a denormal read as zero, by a balanced tree.
  wire [30:0] mag [0:15];
  wire [30:0] t1 [0:7];
  wire [30:0] t2 [0:3];
  wire [30:0] t3 [0:1];
  genvar g;
  generate
    for (g = 0; g < 16; g = g + 1) begin : magnitude
      assign mag[g] = (in_beat[32*g+30 -: 8] == 8'd0) ? 31'd0 : in_beat[32*g+30 -: 31];
    end
    for (g = 0; g < 8; g = g + 1) begin : max8
      assign t1[g] = (mag[2*g] > mag[2*g+1]) ? mag[2*g] : mag[2*g+1];
    end
    for (g = 0; g < 4; g = g + 1) begin : max4
      assign t2[g] = (t1[2*g] > t1[2*g+1]) ? t1[2*g] : t1[2*g+1];
    end
    for (g = 0; g < 2; g = g + 1) begin : max2
      assign t3[g] = (t2[2*g] > t2[2*g+1]) ? t2[2*g] : t2[2*g+1];
    end
  endgenerate
  wire [30:0] beat_max = (t3[0] > t3[1]) ? t3[0] : t3[1];
  reg [30:0] amax;
  wire [7:0] m_c;
  wire signed [11:0] e_c;
  logos_wide_scale sc(.amax(amax), .two(two), .m(m_c), .e(e_c));
  reg [BA:0] n, c;
  reg two_r;
  wire [511:0] xb;
  logos_ram_w512 #(.D(BEATS), .A(BA)) beats(.clk(clk), .ra(c[BA-1:0]), .rd(xb), .we(in_v),
    .wa(clear ? {BA{1'b0}} : n[BA-1:0]), .wd(in_beat), .wm({64{1'b1}}));
  reg [256*BEATS-1:0] cr;
  assign codes = cr[16*LANES-1:0];
  wire [255:0] cb;
  generate
    for (g = 0; g < 16; g = g + 1) begin : lane
      wire signed [15:0] cg;
      logos_act_code u(.x(xb[32*g +: 32]), .m(m), .e(e), .two(two_r), .code(cg));
      assign cb[16*g +: 16] = cg;
    end
  endgenerate
  always @(posedge clk) begin
    if (rst) begin
      busy <= 1'b0; n <= {(BA+1){1'b0}};
    end else begin
      if (in_v) begin
        n <= (clear ? {(BA+1){1'b0}} : n) + 1'b1;
        amax <= (clear || beat_max > amax) ? beat_max : amax;
      end
      if (go) begin
        busy <= n != {(BA+1){1'b0}}; c <= {(BA+1){1'b0}}; m <= m_c; e <= e_c; two_r <= two;
      end else if (busy) begin
        cr[256*c +: 256] <= cb;
        if (c + 1'b1 == n) busy <= 1'b0; else c <= c + 1'b1;
      end
    end
  end
endmodule
"#;
