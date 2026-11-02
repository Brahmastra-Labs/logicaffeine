//! f32 multiply and add as the engine computes them ([`crate::spatial::decode_engine::numerics::fmul`],
//! [`fadd`](crate::spatial::decode_engine::numerics::fadd)): IEEE binary32 rounded to nearest with ties to even, a
//! denormal operand read as a zero of its sign, a result below the normals rounded at the denormals' precision and
//! then flushed to a zero of its sign (so only a result that rounds up to the least normal survives), every NaN the one
//! quiet NaN `0x7fc00000`. Beside them, the binary16 codec between those f32s and the f16 a stored state, a cache
//! scale or the vector unit's f16 rounding keeps.
//!
//! The modules are combinational; a pipelined copy for timing keeps their function and the lock holds both.

/// `logos_fmul(a, b) -> y` and `logos_fadd(a, b) -> y`, and their pipelined copies `logos_fmul_p` (three registered
/// stages) and `logos_fadd_p` (four), a pair of operands a clock, each the same function of the operands its latency
/// before; `logos_tree16`, the pairwise tree of sixteen lanes' sums
/// ([`crate::spatial::decode_engine::numerics::fdot`]'s fold), and its pipelined copy `logos_tree16_p` on
/// `logos_fadd_p`, sixteen lanes a clock; and the binary16 codec `logos_f32_to_f16` and
/// `logos_f16_to_f32` ([`crate::spatial::decode_engine::numerics::f32_to_f16`], [`crate::model::quant::f16_to_f32`]):
/// nearest even, binary16 denormals kept, an f32 denormal a zero of its sign, past the top infinite; and the rounding's
/// pipelined copy `logos_f32_to_f16_p` ([`F32_TO_F16_LATENCY`]).
pub fn fp32_verilog() -> &'static str {
    FP32_VERILOG
}

/// Clocks from a pair of operands to their product in `logos_fmul_p`.
pub const FMUL_LATENCY: usize = 3;

/// Clocks from a pair of operands to their sum in `logos_fadd_p`.
pub const FADD_LATENCY: usize = 4;

/// Clocks from sixteen lanes to their tree's sum in `logos_tree16_p`.
pub const TREE16_LATENCY: usize = 4 * FADD_LATENCY;

/// Clocks from an f32 entering `logos_f32_to_f16_p` to its binary16 leaving: its candidates, round bits and class
/// registered, then the rounding and the choice.
pub const F32_TO_F16_LATENCY: usize = 3;

const FP32_VERILOG: &str = r#"// The engine's f32 multiply and add: round to nearest even, denormals flushed, one NaN.
module logos_fmul(input [31:0] a, input [31:0] b, output reg [31:0] y);
  wire        sa = a[31], sb = b[31], s = sa ^ sb;
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire        top = p[47];
  // The product normalized to a leading one at bit 47, and the biased exponent of that one.
  wire [47:0] pn = top ? p : {p[46:0], 1'b0};
  wire signed [11:0] e = $signed({4'd0, ea}) + $signed({4'd0, eb}) - 12'sd127 + (top ? 12'sd1 : 12'sd0);
  // Below the normals the significand is rounded at the denormals' place, and survives only by rounding up to the
  // least normal: one place below (e = 0) it is shifted a place first, and lower (e < 0) its 24 bits are under 2^22, so
  // no rounding reaches 2^23 and it is flushed as a zero significand is.
  wire        below = (e == 12'sd0), gone = (e < 12'sd0);
  wire [23:0] keep = gone ? 24'd0 : below ? {1'b0, pn[47:25]} : pn[47:24];
  wire        guard = !gone && (below ? pn[24] : pn[23]);
  wire        sticky = !gone && (below ? |pn[23:0] : |pn[22:0]);
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  // A carry out of 24 bits renormalizes; below the normals a carry into bit 23 reaches the least normal.
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  always @* begin
    if (na || nb || (ia && zb) || (ib && za)) y = 32'h7fc00000;
    else if (ia || ib) y = {s, 8'hff, 23'd0};
    else if (za || zb) y = {s, 31'd0};
    else if (ef >= 12'sd255) y = {s, 8'hff, 23'd0};
    else if (ef <= 12'sd0) y = {s, 31'd0};
    else y = {s, ef[7:0], mf};
  end
endmodule

module logos_fadd(input [31:0] a, input [31:0] b, output reg [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  // The larger magnitude first; a flushed operand is a zero.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  e1 = swap ? eb : ea, e2 = swap ? ea : eb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = {1'b1, swap ? mb : ma};
  wire [23:0] f2 = z2 ? 24'd0 : {1'b1, swap ? ma : mb};
  wire [7:0]  d = e1 - e2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  // The leading one of the sum: at bit 27 after a carry, else at 26 or below after a cancellation.
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [10:0] en = c ? $signed({3'd0, e1}) + 11'sd1 : $signed({3'd0, e1}) - $signed({6'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] norm = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  // Two normals are whole multiples of the least denormal, and so is their sum: one below the normals is exact at the
  // denormals' precision, rounds to nothing above them, and is flushed.
  wire [23:0] keep = norm[26:3];
  wire        guard = norm[2];
  wire        sticky = |norm[1:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [10:0] ef = (en < 11'sd1) ? 11'sd0 : (carry ? en + 11'sd1 : en);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        zero = (sum == 28'd0);
  always @* begin
    if (na || nb || (ia && ib && (sa != sb))) y = 32'h7fc00000;
    else if (ia) y = {sa, 8'hff, 23'd0};
    else if (ib) y = {sb, 8'hff, 23'd0};
    else if (za && zb) y = {sa & sb, 31'd0};
    else if (za) y = b;
    else if (zb) y = a;
    else if (zero) y = 32'd0;
    else if (ef >= 11'sd255) y = {s1, 8'hff, 23'd0};
    else if (ef <= 11'sd0) y = {s1, 31'd0};
    else y = {s1, ef[7:0], mf};
  end
endmodule

// The pairwise tree over sixteen lanes: adjacent pairs, then pairs of pairs.
// The engine's f32 multiply, three registered stages: y is logos_fmul(a, b) of the operands three clocks before.
// Stage 1 classifies the operands and multiplies their significands, stage 2 normalizes the product and places it at
// the denormals' precision when it is below the normals, stage 3 rounds and assembles; each stage's logic is
// logos_fmul's, in its order.
module logos_fmul_p(input clk, input [31:0] a, input [31:0] b, output reg [31:0] y);
  // Stage 1.
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  reg         s1, nan1, inf1, zero1;
  reg  [47:0] p1;
  reg signed [11:0] e1;
  always @(posedge clk) begin
    s1 <= sa ^ sb;
    nan1 <= ((ea == 8'hff) && (ma != 23'd0)) || ((eb == 8'hff) && (mb != 23'd0))
            || ((ea == 8'hff) && (ma == 23'd0) && (eb == 8'd0)) || ((eb == 8'hff) && (mb == 23'd0) && (ea == 8'd0));
    inf1 <= ((ea == 8'hff) && (ma == 23'd0)) || ((eb == 8'hff) && (mb == 23'd0));
    zero1 <= (ea == 8'd0) || (eb == 8'd0);
    p1 <= {1'b1, ma} * {1'b1, mb};
    e1 <= $signed({4'd0, ea}) + $signed({4'd0, eb}) - 12'sd127;
  end
  // Stage 2.
  wire        top = p1[47];
  wire [47:0] pn = top ? p1 : {p1[46:0], 1'b0};
  wire signed [11:0] e = e1 + (top ? 12'sd1 : 12'sd0);
  wire        below = (e == 12'sd0), gone = (e < 12'sd0);
  reg         s2, nan2, inf2, zero2, guard2, sticky2;
  reg  [23:0] keep2;
  reg signed [11:0] e2;
  always @(posedge clk) begin
    s2 <= s1; nan2 <= nan1; inf2 <= inf1; zero2 <= zero1;
    keep2 <= gone ? 24'd0 : below ? {1'b0, pn[47:25]} : pn[47:24];
    guard2 <= !gone && (below ? pn[24] : pn[23]);
    sticky2 <= !gone && (below ? |pn[23:0] : |pn[22:0]);
    e2 <= e;
  end
  // Stage 3.
  wire        up = guard2 && (sticky2 || keep2[0]);
  wire [24:0] r = {1'b0, keep2} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e2 < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e2 + 12'sd1 : e2);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  always @(posedge clk) begin
    if (nan2) y <= 32'h7fc00000;
    else if (inf2) y <= {s2, 8'hff, 23'd0};
    else if (zero2) y <= {s2, 31'd0};
    else if (ef >= 12'sd255) y <= {s2, 8'hff, 23'd0};
    else if (ef <= 12'sd0) y <= {s2, 31'd0};
    else y <= {s2, ef[7:0], mf};
  end
endmodule

// The engine's f32 add, four registered stages: y is logos_fadd(a, b) of the operands four clocks before. Stage 1
// orders the operands by magnitude, stage 2 aligns the smaller and adds, stage 3 finds the sum's leading one and
// normalizes, stage 4 rounds and assembles, a sum below the normals flushed; each stage's logic is logos_fadd's.
module logos_fadd_p(input clk, input [31:0] a, input [31:0] b, output reg [31:0] y);
  // Stage 1.
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        swap = {ea, ma} < {eb, mb};
  wire [7:0]  e1w = swap ? eb : ea, e2w = swap ? ea : eb;
  wire        z2 = swap ? za : zb;
  wire [7:0]  d = e1w - e2w;
  // The special cases decided here and carried: a NaN, an infinity's sign, a zero operand's result.
  reg  [2:0]  kind1;          // 0 the sum, 1 NaN, 2 +-inf of sa, 3 +-inf of sb, 4 both zero, 5 b, 6 a
  reg         sa1, sb1, s11;
  reg  [31:0] a1, b1;
  reg  [7:0]  e11;
  reg  [23:0] f11, f21;
  reg  [5:0]  d1;
  reg         sub1;
  always @(posedge clk) begin
    kind1 <= (na || nb || (ia && ib && (sa != sb))) ? 3'd1 : ia ? 3'd2 : ib ? 3'd3 : (za && zb) ? 3'd4 : za ? 3'd5 : zb ? 3'd6 : 3'd0;
    sa1 <= sa; sb1 <= sb; a1 <= a; b1 <= b;
    s11 <= swap ? sb : sa;
    e11 <= e1w;
    f11 <= {1'b1, swap ? mb : ma};
    f21 <= z2 ? 24'd0 : {1'b1, swap ? ma : mb};
    d1 <= (d > 8'd50) ? 6'd50 : d[5:0];
    sub1 <= sa ^ sb;
  end
  // Stage 2.
  wire [50:0] aligned = {f21, 27'd0} >> d1;
  wire [26:0] g1 = {f11, 3'd0}, g2 = {aligned[50:25], |aligned[24:0]};
  wire [27:0] sum = sub1 ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [2:0]  kind2;
  reg         sa2, sb2, s12;
  reg  [31:0] a2, b2;
  reg  [7:0]  e12;
  reg  [27:0] sum2;
  always @(posedge clk) begin
    kind2 <= kind1; sa2 <= sa1; sb2 <= sb1; a2 <= a1; b2 <= b1; s12 <= s11; e12 <= e11;
    sum2 <= sum;
  end
  // Stage 3.
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum2[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum2[27];
  wire signed [10:0] en = c ? $signed({3'd0, e12}) + 11'sd1 : $signed({3'd0, e12}) - $signed({6'd0, lz});
  wire [26:0] norm = c ? {sum2[27:2], sum2[1] | sum2[0]} : (sum2[26:0] << lz);
  reg  [2:0]  kind3;
  reg         sa3, sb3, s13, zero3, guard3, sticky3;
  reg  [31:0] a3, b3;
  reg  [23:0] keep3;
  reg signed [10:0] en3;
  always @(posedge clk) begin
    kind3 <= kind2; sa3 <= sa2; sb3 <= sb2; a3 <= a2; b3 <= b2; s13 <= s12;
    zero3 <= (sum2 == 28'd0);
    keep3 <= norm[26:3];
    guard3 <= norm[2];
    sticky3 <= |norm[1:0];
    en3 <= en;
  end
  // Stage 4.
  wire        up = guard3 && (sticky3 || keep3[0]);
  wire [24:0] r = {1'b0, keep3} + {24'd0, up};
  wire        carry = r[24];
  wire signed [10:0] ef = (en3 < 11'sd1) ? 11'sd0 : (carry ? en3 + 11'sd1 : en3);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  always @(posedge clk) begin
    case (kind3)
      3'd1: y <= 32'h7fc00000;
      3'd2: y <= {sa3, 8'hff, 23'd0};
      3'd3: y <= {sb3, 8'hff, 23'd0};
      3'd4: y <= {sa3 & sb3, 31'd0};
      3'd5: y <= b3;
      3'd6: y <= a3;
      default:
        if (zero3) y <= 32'd0;
        else if (ef >= 11'sd255) y <= {s13, 8'hff, 23'd0};
        else if (ef <= 11'sd0) y <= {s13, 31'd0};
        else y <= {s13, ef[7:0], mf};
    endcase
  end
endmodule

module logos_tree16(input [511:0] lanes, output [31:0] sum);
  wire [31:0] l1 [0:7];
  wire [31:0] l2 [0:3];
  wire [31:0] l3 [0:1];
  genvar i;
  generate
    for (i = 0; i < 8; i = i + 1) begin : t1
      logos_fadd a(.a(lanes[64*i +: 32]), .b(lanes[64*i + 32 +: 32]), .y(l1[i]));
    end
    for (i = 0; i < 4; i = i + 1) begin : t2
      logos_fadd a(.a(l1[2*i]), .b(l1[2*i + 1]), .y(l2[i]));
    end
    for (i = 0; i < 2; i = i + 1) begin : t3
      logos_fadd a(.a(l2[2*i]), .b(l2[2*i + 1]), .y(l3[i]));
    end
  endgenerate
  logos_fadd t4(.a(l3[0]), .b(l3[1]), .y(sum));
endmodule

// The pairwise tree of sixteen lanes on the pipelined add: sum is logos_tree16 of the lanes sixteen clocks before, four
// levels of logos_fadd_p, sixteen lanes a clock.
module logos_tree16_p(input clk, input [511:0] lanes, output [31:0] sum);
  wire [31:0] l1 [0:7];
  wire [31:0] l2 [0:3];
  wire [31:0] l3 [0:1];
  genvar i;
  generate
    for (i = 0; i < 8; i = i + 1) begin : t1
      logos_fadd_p a(.clk(clk), .a(lanes[64*i +: 32]), .b(lanes[64*i + 32 +: 32]), .y(l1[i]));
    end
    for (i = 0; i < 4; i = i + 1) begin : t2
      logos_fadd_p a(.clk(clk), .a(l1[2*i]), .b(l1[2*i + 1]), .y(l2[i]));
    end
    for (i = 0; i < 2; i = i + 1) begin : t3
      logos_fadd_p a(.clk(clk), .a(l2[2*i]), .b(l2[2*i + 1]), .y(l3[i]));
    end
  endgenerate
  logos_fadd_p t4(.clk(clk), .a(l3[0]), .b(l3[1]), .y(sum));
endmodule

// The binary16 codec.
module logos_f32_to_f16(input [31:0] x, output reg [15:0] h);
  wire        s = x[31];
  wire [7:0]  ex = x[30:23];
  wire [22:0] mant = x[22:0];
  wire signed [9:0] e = $signed({2'd0, ex}) - 10'sd127;
  wire [9:0]  m = mant[22:13];
  wire [12:0] rem = mant[12:0];
  wire [15:0] normal = {1'b0, e[4:0] + 5'd15, m};
  wire        up = (rem > 13'h1000) || (rem == 13'h1000 && m[0]);
  // A binary16 denormal counts units of 2^-24: the 24-bit significand shifted right by -(e + 1).
  wire [23:0] sig = {1'b1, mant};
  wire signed [9:0] sh = -(e + 10'sd1);
  wire [23:0] n = sig >> sh;
  wire [23:0] dr = sig & ((24'd1 << sh) - 24'd1);
  wire [23:0] dh = 24'd1 << (sh - 10'sd1);
  wire        dup = (dr > dh) || (dr == dh && n[0]);
  always @* begin
    if (ex == 8'hff) h = {s, 5'h1f, (mant != 23'd0) ? 10'h200 : 10'd0};
    else if (ex == 8'd0) h = {s, 15'd0};
    else if (e > 10'sd15) h = {s, 15'h7c00};
    else if (e >= -10'sd14) h = {s, 15'd0} | (normal + {15'd0, up});
    else if (sh > 10'sd24) h = {s, 15'd0};
    else h = {s, 15'd0} | (n[15:0] + {15'd0, dup});
  end
endmodule

// logos_f32_to_f16 for the user clock, an operand a clock: h is logos_f32_to_f16 of the operand two clocks before, its
// normal and denormal candidates with their round bits and its class registered, then the rounding and the choice.
module logos_f32_to_f16_p(input clk, input [31:0] x, output reg [15:0] h);
  // Stage 1: the class, the normal's rounding, and the denormal's shift.
  wire        s = x[31];
  wire [7:0]  ex = x[30:23];
  wire [22:0] mant = x[22:0];
  wire signed [9:0] e = $signed({2'd0, ex}) - 10'sd127;
  wire [9:0]  m = mant[22:13];
  wire [12:0] rem = mant[12:0];
  wire [15:0] normal = {1'b0, e[4:0] + 5'd15, m};
  wire        up = (rem > 13'h1000) || (rem == 13'h1000 && m[0]);
  wire signed [9:0] sh = -(e + 10'sd1);
  // The class, in logos_f32_to_f16's order: NaN or infinity, zero or denormal, past the top, normal, below the
  // least denormal, denormal.
  wire [2:0] cls_c = (ex == 8'hff) ? 3'd0 : (ex == 8'd0) ? 3'd1 : (e > 10'sd15) ? 3'd2 : (e >= -10'sd14) ? 3'd3
                   : (sh > 10'sd24) ? 3'd4 : 3'd5;
  reg s1, up1, nan1;
  reg [15:0] normal1;
  reg [23:0] sig1;
  reg [4:0] sh1;
  reg [2:0] cls1;
  always @(posedge clk) begin
    s1 <= s; up1 <= up; nan1 <= mant != 23'd0; normal1 <= normal; sig1 <= {1'b1, mant}; sh1 <= sh[4:0]; cls1 <= cls_c;
  end
  // Stage 2: the denormal's significand and its rounding, nearest even.
  wire [23:0] n = sig1 >> sh1;
  wire [23:0] dr = sig1 & ((24'd1 << sh1) - 24'd1);
  wire [23:0] dh = 24'd1 << (sh1 - 5'd1);
  wire        dup = (dr > dh) || (dr == dh && n[0]);
  reg s2, up2, dup2, nan2;
  reg [15:0] normal2, n2;
  reg [2:0] cls;
  always @(posedge clk) begin
    s2 <= s1; up2 <= up1; dup2 <= dup; nan2 <= nan1; normal2 <= normal1; n2 <= n[15:0]; cls <= cls1;
  end
  // Stage 3: the binary16.
  always @(posedge clk)
    case (cls)
      3'd0: h <= {s2, 5'h1f, nan2 ? 10'h200 : 10'd0};
      3'd1: h <= {s2, 15'd0};
      3'd2: h <= {s2, 15'h7c00};
      3'd3: h <= {s2, 15'd0} | (normal2 + {15'd0, up2});
      3'd4: h <= {s2, 15'd0};
      default: h <= {s2, 15'd0} | (n2 + {15'd0, dup2});
    endcase
endmodule

// A subnormal's leading one at bit p gives its significand m << (10 - p) and exponent 103 + p, each candidate chosen by
// the one-hot of p (lead), so the normalization is an and-or of ten candidates rather than a search and a shift.
module logos_f16_to_f32(input [15:0] h, output [31:0] x);
  wire s = h[15];
  wire [4:0] e = h[14:10];
  wire [9:0] m = h[9:0];
  wire [9:0] lead;
  genvar p;
  generate
    for (p = 0; p < 10; p = p + 1) begin : one
      assign lead[p] = m[p] && (m >> (p + 1)) == 10'd0;
    end
  endgenerate
  reg [9:0] sub_m;
  reg [7:0] sub_e;
  integer q;
  always @* begin
    sub_m = 10'd0;
    sub_e = 8'd0;
    for (q = 0; q < 10; q = q + 1) begin
      sub_m = sub_m | ({10{lead[q]}} & (m << (10 - q)));
      sub_e = sub_e | ({8{lead[q]}} & (8'd103 + q[7:0]));
    end
  end
  assign x = e == 5'd0 ? (m == 10'd0 ? {s, 31'd0} : {s, sub_e, sub_m, 13'd0})
           : e == 5'h1f ? (m == 10'd0 ? {s, 8'hff, 23'd0} : {s, 8'hff, 1'b1, m[8:0], 13'd0})
           : {s, {3'd0, e} + 8'd112, m, 13'd0};
endmodule
"#;
