//! The block-scale lane as Verilog ([`crate::spatial::decode_engine::lane::lane`]): a weight block's excess partial
//! sums to its signed sums, the two scaled terms, their exact sum rounded once to f32, and the f32 add into the row.
//!
//! `logos_lane #(UNITS)` takes, per 16-element unit, the high word's `Σ q·a′` and `Σ a′` (the only word of a one-word
//! code), the low byte's alike, the weight image's `Σ q`, and the unit's multipliers `c` and `n`; per block the
//! format's offset `z`, the two binary16 scales, the activation scale `m·2^e`, whether the code has two words, and the
//! row's running f32. `logos_round88` is the one rounding: an exact signed integer times a power of two to f32,
//! nearest even, a result below the normals flushed to a zero of its sign, past the top infinite.
//!
//! `logos_lane_value_p #(UNITS, WORDS)` is the lane up to its rounding, pipelined for the streaming engine: a block's
//! inputs a clock, and [`LANE_VALUE_LATENCY`] clocks later the block's f32 value, the f32 the lane adds into its row
//! (the lane from −0, which every value leaves as it is), its stages each unit's signed sums and their products, the
//! units' sums in fours, their sum, the two scaled terms, their exact sum, and the rounding in [`ROUND88_P_LATENCY`] stages of its own
//! (`logos_round88_p`: the magnitude; each byte's leading one; the leading one; the significand kept with its round and
//! sticky bits; the rounding and the f32). Its multipliers are the bytes every decoder's
//! `c` and `n` are, its offset term a shift (every format's offset is zero or a power of two). Built for one activation
//! word (`WORDS` 1), a unit's signed sum `Σ (q − z)·a` over its 16 elements is within 19 bits (every format's offset
//! code within ±128, every one-word activation code within ±127: at most 16·128·127 = 260,096), so a pair of units'
//! products is one DSP block's two 18×19 multipliers summed; built for two, each unit's product is one wide multiplier.
//!
//! `logos_lane_slim_p #(UNITS, WORDS)` is `logos_lane_value_p` for a block of one scale — every unit's multiplier 1
//! and its offset multiplier 0, Q8_0's and Q4_0's blocks — at the same latency: the units' signed sums summed, the one
//! scaled term and the rounding, no unit multiplies and one product a block.

/// Clocks from `logos_round88_p`'s operands to their f32.
pub const ROUND88_P_LATENCY: usize = 5;

/// Clocks from a block's inputs to its value in `logos_lane_value_p`: four to the exact sum, then the rounding.
pub const LANE_VALUE_LATENCY: usize = 5 + ROUND88_P_LATENCY;

/// Every module of the lane, with the f32 adder it accumulates through.
pub fn lane_verilog() -> String {
    format!("{}\n{}", super::fp32::fp32_verilog(), LANE_VERILOG)
}

const LANE_VERILOG: &str = r#"// The block-scale lane: a block's integer sums to its f32 value, added into its row.

// v·2^least rounded once to f32.
module logos_round88(input signed [87:0] v, input signed [11:0] least, output reg [31:0] f);
  reg [87:0] mag;
  reg        neg;
  integer    lead, i, s;
  reg [87:0] kept, rem, half;
  reg [24:0] m;
  reg signed [13:0] unb;
  always @* begin
    neg = v[87];
    mag = neg ? -v : v;
    lead = -1;
    for (i = 0; i < 88; i = i + 1) if (mag[i]) lead = i;
    f = 32'd0;
    if (lead >= 0) begin
      if (lead <= 23) begin
        m = mag[24:0] << (23 - lead);
        unb = least + $signed(lead[13:0]);
      end else begin
        s = lead - 23;
        kept = mag >> s;
        rem = mag & ((88'd1 << s) - 88'd1);
        half = 88'd1 << (s - 1);
        m = kept[24:0] + ((rem > half || (rem == half && kept[0])) ? 25'd1 : 25'd0);
        unb = least + $signed(lead[13:0]);
        if (m[24]) begin m = m >> 1; unb = unb + 14'sd1; end
      end
      if (unb > 14'sd127) f = {neg, 8'hff, 23'd0};
      else if (unb < -14'sd126) f = {neg, 31'd0};
      else f = {neg, unb[7:0] + 8'd127, m[22:0]};
    end
  end
endmodule

// logos_round88 for the user clock, an operand a clock: f is logos_round88 of the v and least five clocks before. The
// magnitude; each byte's leading one; the leading one, the top byte that has one at its own; the significand kept, the
// first bit dropped (round) and whether any below it is set (sticky); the rounding to nearest even and the f32.
module logos_round88_p(input clk, input signed [87:0] v, input signed [11:0] least, output reg [31:0] f);
  function [2:0] lead8(input [7:0] x);
    integer i;
    begin
      lead8 = 3'd0;
      for (i = 0; i < 8; i = i + 1) if (x[i]) lead8 = i[2:0];
    end
  endfunction
  // Stage 1: the sign and the magnitude.
  reg [87:0] mag1;
  reg        neg1;
  reg signed [11:0] l1;
  always @(posedge clk) begin neg1 <= v[87]; mag1 <= v[87] ? -v : v; l1 <= least; end
  // Stage 2: each byte's leading one, and whether it has one.
  reg [10:0] any2;
  reg [32:0] at2;
  reg [87:0] mag2;
  reg        neg2;
  reg signed [11:0] l2;
  integer g;
  always @(posedge clk) begin
    for (g = 0; g < 11; g = g + 1) begin
      any2[g] <= |mag1[8*g +: 8];
      at2[3*g +: 3] <= lead8(mag1[8*g +: 8]);
    end
    mag2 <= mag1; neg2 <= neg1; l2 <= l1;
  end
  // Stage 3: the leading one: the top byte that has one, at its own.
  reg [3:0] top;
  integer t;
  always @* begin
    top = 4'd0;
    for (t = 0; t < 11; t = t + 1) if (any2[t]) top = t[3:0];
  end
  reg [6:0] lead3;
  reg       none3;
  reg [87:0] mag3;
  reg        neg3;
  reg signed [11:0] l3;
  always @(posedge clk) begin
    lead3 <= {top, at2[3*top +: 3]};
    none3 <= ~|any2;
    mag3 <= mag2; neg3 <= neg2; l3 <= l2;
  end
  // Stage 4: the significand kept (its leading one at bit 23), the round and sticky bits past it, the exponent.
  wire [6:0] s3 = lead3 - 7'd23;
  wire [87:0] below = ~({88{1'b1}} << (s3 - 7'd1));
  wire [87:0] kept3 = mag3 >> s3;
  reg [24:0] m4;
  reg        rnd4, sticky4, none4, neg4;
  reg signed [13:0] unb4;
  always @(posedge clk) begin
    if (lead3 <= 7'd23) begin
      m4 <= mag3[24:0] << (7'd23 - lead3);
      rnd4 <= 1'b0;
      sticky4 <= 1'b0;
    end else begin
      m4 <= kept3[24:0];
      rnd4 <= mag3[s3 - 7'd1];
      sticky4 <= |(mag3 & below);
    end
    unb4 <= l3 + $signed({7'd0, lead3});
    none4 <= none3; neg4 <= neg3;
  end
  // Stage 5: the rounding, nearest even, and the f32: past the top infinite, below the normals a zero of its sign.
  reg [24:0] m;
  reg signed [13:0] unb;
  always @(posedge clk) begin
    m = m4 + ((rnd4 && (sticky4 || m4[0])) ? 25'd1 : 25'd0);
    unb = unb4;
    if (m[24]) begin m = m >> 1; unb = unb + 14'sd1; end
    if (none4) f <= 32'd0;
    else if (unb > 14'sd127) f <= {neg4, 8'hff, 23'd0};
    else if (unb < -14'sd126) f <= {neg4, 31'd0};
    else f <= {neg4, unb[7:0] + 8'd127, m[22:0]};
  end
endmodule

// One weight block of UNITS 16-element units.
module logos_lane #(parameter UNITS = 16)
  (input [24*UNITS-1:0] p_hi, input [12*UNITS-1:0] s_hi, input [24*UNITS-1:0] p_lo, input [12*UNITS-1:0] s_lo,
   input [12*UNITS-1:0] q, input [16*UNITS-1:0] c, input [16*UNITS-1:0] n,
   input [7:0] z, input [15:0] a, input [15:0] b, input [7:0] m, input signed [11:0] e, input two,
   input [31:0] acc, output [31:0] out);
  // The units' signed sums: the high word in excess form, the low byte as it is, weighted by 256 under two words.
  reg signed [47:0] ia, ib;
  reg signed [31:0] sp_hi, sa_hi, sp_lo, sa_lo, sp, sa;
  integer u;
  always @* begin
    ia = 48'sd0;
    ib = 48'sd0;
    for (u = 0; u < UNITS; u = u + 1) begin
      sp_hi = $signed({8'd0, p_hi[24*u +: 24]}) - ($signed({20'd0, q[12*u +: 12]}) <<< 7)
            - $signed({24'd0, z}) * $signed({20'd0, s_hi[12*u +: 12]}) + ($signed({24'd0, z}) <<< 11);
      sa_hi = $signed({20'd0, s_hi[12*u +: 12]}) - 32'sd2048;
      sp_lo = $signed({8'd0, p_lo[24*u +: 24]}) - $signed({24'd0, z}) * $signed({20'd0, s_lo[12*u +: 12]});
      sa_lo = $signed({20'd0, s_lo[12*u +: 12]});
      sp = two ? (sp_hi <<< 8) + sp_lo : sp_hi;
      sa = two ? (sa_hi <<< 8) + sa_lo : sa_hi;
      ia = ia + $signed(c[16*u +: 16]) * sp;
      ib = ib + $signed(n[16*u +: 16]) * sa;
    end
  end
  // Each binary16 scale as a signed significand and its exponent: sig·2^(exp), a denormal's exp the least.
  function signed [12:0] sig16(input [15:0] h);
    sig16 = (h[14:10] == 5'd0) ? (h[15] ? -$signed({3'd0, h[9:0]}) : $signed({3'd0, h[9:0]}))
                               : (h[15] ? -$signed({2'd0, 1'b1, h[9:0]}) : $signed({2'd0, 1'b1, h[9:0]}));
  endfunction
  function signed [11:0] exp16(input [15:0] h);
    exp16 = (h[14:10] == 5'd0) ? -12'sd24 : $signed({7'd0, h[14:10]}) - 12'sd25;
  endfunction
  wire signed [12:0] sa_sig = sig16(a), sb_sig = sig16(b);
  wire signed [11:0] ea = exp16(a) + e, eb = exp16(b) + e;
  wire signed [87:0] va = $signed(sa_sig) * $signed({1'b0, m}) * ia;
  wire signed [87:0] vb = $signed(sb_sig) * $signed({1'b0, m}) * ib;
  wire has_a = (sa_sig != 13'sd0) && (ia != 48'sd0), has_b = (sb_sig != 13'sd0) && (ib != 48'sd0);
  wire signed [11:0] least = !has_b ? ea : !has_a ? eb : (ea < eb ? ea : eb);
  wire signed [87:0] sum = (has_a ? (va <<< (ea - least)) : 88'sd0) + (has_b ? (vb <<< (eb - least)) : 88'sd0);
  wire [31:0] value;
  logos_round88 r(.v(sum), .least(least), .f(value));
  logos_fadd add(.a(acc), .b(value), .y(out));
endmodule

// The lane up to its rounding, ten registered stages: value is the f32 of the block whose inputs came ten clocks
// before, each stage logos_lane's arithmetic in its order, the rounding logos_round88_p's five. The unit multipliers and offset terms are signed bytes, a byte a
// unit, and the offset zero or a power of two; built for one activation word (WORDS 1), `two` is never set, a unit's
// signed sums take 19 and 13 bits, stage 1 sums a pair of units' products as one DSP block's two 18×19 multipliers, and
// the terms' sums take 30 and 23.
module logos_lane_value_p #(parameter UNITS = 16, WORDS = 2)
  (input clk, input [24*UNITS-1:0] p_hi, input [12*UNITS-1:0] s_hi, input [24*UNITS-1:0] p_lo, input [12*UNITS-1:0] s_lo,
   input [12*UNITS-1:0] q, input [8*UNITS-1:0] c, input [8*UNITS-1:0] n,
   input [7:0] z, input [15:0] a, input [15:0] b, input [7:0] m, input signed [11:0] e, input two,
   output reg [31:0] value);
  localparam WIDE = WORDS > 1;
  // A stage-1 term: a unit's products built for two words, a pair of units' summed built for one.
  localparam TERMS = WIDE ? UNITS : (UNITS + 1) / 2;
  // The terms' widths: two words' 48 bits; one word's 30 and 23, sixteen units' products within 16·128·260,096 and
  // 16·128·2,032.
  localparam AW = WIDE ? 48 : 30, BW = WIDE ? 48 : 23;
  wire zero = z == 8'd0;
  wire [2:0] zlog = z[7] ? 3'd7 : z[6] ? 3'd6 : z[5] ? 3'd5 : z[4] ? 3'd4 : z[3] ? 3'd3 : z[2] ? 3'd2 : z[1] ? 3'd1 : 3'd0;
  // Each unit's signed sums.
  wire signed [31:0] sp [0:UNITS-1];
  wire signed [31:0] sa [0:UNITS-1];
  genvar g;
  generate
    for (g = 0; g < UNITS; g = g + 1) begin : unit
      wire signed [31:0] zs_hi = zero ? 32'sd0 : $signed({20'd0, s_hi[12*g +: 12]}) <<< zlog;
      wire signed [31:0] zs_lo = zero ? 32'sd0 : $signed({20'd0, s_lo[12*g +: 12]}) <<< zlog;
      wire signed [31:0] sp_hi = $signed({8'd0, p_hi[24*g +: 24]}) - ($signed({20'd0, q[12*g +: 12]}) <<< 7) - zs_hi + ($signed({24'd0, z}) <<< 11);
      wire signed [31:0] sa_hi = $signed({20'd0, s_hi[12*g +: 12]}) - 32'sd2048;
      wire signed [31:0] sp_lo = $signed({8'd0, p_lo[24*g +: 24]}) - zs_lo;
      wire signed [31:0] sa_lo = $signed({20'd0, s_lo[12*g +: 12]});
      assign sp[g] = (WIDE && two) ? (sp_hi <<< 8) + sp_lo : sp_hi;
      assign sa[g] = (WIDE && two) ? (sa_hi <<< 8) + sa_lo : sa_hi;
    end
  endgenerate
  // Stage 1: the terms, each unit's products with its multipliers.
  wire [AW*TERMS-1:0] pa1;
  wire [BW*TERMS-1:0] pb1;
  generate
    for (g = 0; g < TERMS; g = g + 1) begin : term
      reg signed [AW-1:0] ta;
      reg signed [BW-1:0] tb;
      if (WIDE) begin : wide
        always @(posedge clk) begin
          ta <= $signed(c[8*g +: 8]) * sp[g];
          tb <= $signed(n[8*g +: 8]) * sa[g];
        end
      end else begin : pair
        wire signed [18:0] p0 = sp[2*g][18:0];
        wire signed [12:0] s0 = sa[2*g][12:0];
        wire signed [7:0] c0 = c[16*g +: 8], n0 = n[16*g +: 8];
        wire signed [18:0] p1;
        wire signed [12:0] s1;
        wire signed [7:0] c1, n1;
        if (2*g + 1 < UNITS) begin : second
          assign p1 = sp[2*g + 1][18:0];
          assign s1 = sa[2*g + 1][12:0];
          assign c1 = c[16*g + 8 +: 8];
          assign n1 = n[16*g + 8 +: 8];
        end else begin : alone
          assign p1 = 19'sd0;
          assign s1 = 13'sd0;
          assign c1 = 8'sd0;
          assign n1 = 8'sd0;
        end
        always @(posedge clk) begin
          ta <= c0 * p0 + c1 * p1;
          tb <= n0 * s0 + n1 * s1;
        end
      end
      assign pa1[AW*g +: AW] = ta;
      assign pb1[BW*g +: BW] = tb;
    end
  endgenerate
  reg [15:0] a1, b1;
  reg [7:0] m1;
  reg signed [11:0] e1;
  always @(posedge clk) begin
    a1 <= a; b1 <= b; m1 <= m; e1 <= e;
  end
  // Stages 2 and 3: the terms' sums in fours, then the fours' sum.
  localparam G = (TERMS + 3) / 4;
  reg signed [AW-1:0] fa2 [0:G-1];
  reg signed [BW-1:0] fb2 [0:G-1];
  reg signed [AW-1:0] ia2, ga, ta;
  reg signed [BW-1:0] ib2, gb, tb;
  reg [15:0] a2f, b2f, a2, b2;
  reg [7:0] m2f, m2;
  reg signed [11:0] e2f, e2;
  integer v, w, x;
  always @(posedge clk) begin
    for (w = 0; w < G; w = w + 1) begin
      ga = {AW{1'b0}};
      gb = {BW{1'b0}};
      for (v = 4 * w; v < 4 * w + 4; v = v + 1) if (v < TERMS) begin
        ga = ga + $signed(pa1[AW*v +: AW]);
        gb = gb + $signed(pb1[BW*v +: BW]);
      end
      fa2[w] <= ga; fb2[w] <= gb;
    end
    a2f <= a1; b2f <= b1; m2f <= m1; e2f <= e1;
  end
  always @(posedge clk) begin
    ta = {AW{1'b0}};
    tb = {BW{1'b0}};
    for (x = 0; x < G; x = x + 1) begin
      ta = ta + fa2[x];
      tb = tb + fb2[x];
    end
    ia2 <= ta; ib2 <= tb; a2 <= a2f; b2 <= b2f; m2 <= m2f; e2 <= e2f;
  end
  // Stage 4: the two scaled terms and their exponents.
  function signed [12:0] sig16(input [15:0] h);
    sig16 = (h[14:10] == 5'd0) ? (h[15] ? -$signed({3'd0, h[9:0]}) : $signed({3'd0, h[9:0]}))
                               : (h[15] ? -$signed({2'd0, 1'b1, h[9:0]}) : $signed({2'd0, 1'b1, h[9:0]}));
  endfunction
  function signed [11:0] exp16(input [15:0] h);
    exp16 = (h[14:10] == 5'd0) ? -12'sd24 : $signed({7'd0, h[14:10]}) - 12'sd25;
  endfunction
  wire signed [12:0] sa_sig = sig16(a2), sb_sig = sig16(b2);
  // Which term stage 5 shifts: the one at the greater exponent (a's when only a's is present), by the exponents' gap
  // (zero when only one is present), the lesser exponent the sum's.
  wire signed [11:0] ea = exp16(a2) + e2, eb = exp16(b2) + e2;
  wire has_a = (sa_sig != 13'sd0) && (|ia2), has_b = (sb_sig != 13'sd0) && (|ib2);
  wire a_up = has_a && (!has_b || ea > eb);
  reg signed [87:0] va3, vb3;
  reg a_up3, up_on3, flat_on3;
  reg [11:0] gap3;
  reg signed [11:0] least3;
  always @(posedge clk) begin
    va3 <= $signed(sa_sig) * $signed({1'b0, m2}) * ia2;
    vb3 <= $signed(sb_sig) * $signed({1'b0, m2}) * ib2;
    a_up3 <= a_up;
    up_on3 <= a_up || has_b;
    flat_on3 <= a_up ? has_b : has_a;
    gap3 <= (has_a && has_b) ? (a_up ? ea - eb : eb - ea) : 12'd0;
    least3 <= !has_b ? ea : !has_a ? eb : (ea < eb ? ea : eb);
  end
  // Stage 5: the terms aligned to the lesser exponent and summed exactly, the term at the greater exponent shifted by
  // the gap, the other as it is.
  wire signed [87:0] up = a_up3 ? va3 : vb3, flat = a_up3 ? vb3 : va3;
  reg signed [87:0] sum4;
  reg signed [11:0] least4;
  always @(posedge clk) begin
    sum4 <= (up_on3 ? (up <<< gap3) : 88'sd0) + (flat_on3 ? flat : 88'sd0);
    least4 <= least3;
  end
  // Stages 6 to 10: the one rounding.
  wire [31:0] rounded;
  logos_round88_p r(.clk(clk), .v(sum4), .least(least4), .f(rounded));
  always @* value = rounded;
endmodule

// logos_lane_value_p for a block of one scale: its units' multipliers 1 and their offset multipliers 0, so the
// block's value is its one scaled term, sig(a)·m·Σ(the units' signed sums) rounded once at exp(a) + e — Q8_0's and
// Q4_0's 32-element blocks. Its stages are logos_lane_value_p's, value the f32 of the block whose inputs came ten
// clocks before: no unit multiplies, one product a block.
module logos_lane_slim_p #(parameter UNITS = 2, WORDS = 2)
  (input clk, input [24*UNITS-1:0] p_hi, input [12*UNITS-1:0] s_hi, input [24*UNITS-1:0] p_lo, input [12*UNITS-1:0] s_lo,
   input [12*UNITS-1:0] q, input [7:0] z, input [15:0] a, input [7:0] m, input signed [11:0] e, input two,
   output reg [31:0] value);
  // Stage 1: each unit's signed sum.
  reg signed [47:0] pa1 [0:UNITS-1];
  reg [15:0] a1;
  reg [7:0] m1;
  reg signed [11:0] e1;
  reg signed [31:0] sp_hi, sp_lo, zs_hi, zs_lo;
  localparam WIDE = WORDS > 1;
  wire zero = z == 8'd0;
  wire [2:0] zlog = z[7] ? 3'd7 : z[6] ? 3'd6 : z[5] ? 3'd5 : z[4] ? 3'd4 : z[3] ? 3'd3 : z[2] ? 3'd2 : z[1] ? 3'd1 : 3'd0;
  integer u;
  always @(posedge clk) begin
    for (u = 0; u < UNITS; u = u + 1) begin
      zs_hi = zero ? 32'sd0 : $signed({20'd0, s_hi[12*u +: 12]}) <<< zlog;
      zs_lo = zero ? 32'sd0 : $signed({20'd0, s_lo[12*u +: 12]}) <<< zlog;
      sp_hi = $signed({8'd0, p_hi[24*u +: 24]}) - ($signed({20'd0, q[12*u +: 12]}) <<< 7) - zs_hi + ($signed({24'd0, z}) <<< 11);
      sp_lo = $signed({8'd0, p_lo[24*u +: 24]}) - zs_lo;
      pa1[u] <= (WIDE && two) ? (sp_hi <<< 8) + sp_lo : sp_hi;
    end
    a1 <= a; m1 <= m; e1 <= e;
  end
  // Stages 2 and 3: the units' sum, carried a stage further as logos_lane_value_p's fours and their sum are.
  reg signed [47:0] ia2f, ia2, ta;
  reg [15:0] a2f, a2;
  reg [7:0] m2f, m2;
  reg signed [11:0] e2f, e2;
  integer x;
  always @(posedge clk) begin
    ta = 48'sd0;
    for (x = 0; x < UNITS; x = x + 1) ta = ta + pa1[x];
    ia2f <= ta; a2f <= a1; m2f <= m1; e2f <= e1;
  end
  always @(posedge clk) begin ia2 <= ia2f; a2 <= a2f; m2 <= m2f; e2 <= e2f; end
  // Stage 4: the scaled term and its exponent.
  function signed [12:0] sig16(input [15:0] h);
    sig16 = (h[14:10] == 5'd0) ? (h[15] ? -$signed({3'd0, h[9:0]}) : $signed({3'd0, h[9:0]}))
                               : (h[15] ? -$signed({2'd0, 1'b1, h[9:0]}) : $signed({2'd0, 1'b1, h[9:0]}));
  endfunction
  function signed [11:0] exp16(input [15:0] h);
    exp16 = (h[14:10] == 5'd0) ? -12'sd24 : $signed({7'd0, h[14:10]}) - 12'sd25;
  endfunction
  wire signed [12:0] sa_sig = sig16(a2);
  reg signed [87:0] va3;
  reg signed [11:0] ea3;
  reg has_a3;
  always @(posedge clk) begin
    va3 <= $signed(sa_sig) * $signed({1'b0, m2}) * ia2;
    ea3 <= exp16(a2) + e2;
    has_a3 <= (sa_sig != 13'sd0) && (ia2 != 48'sd0);
  end
  // Stage 5: the term, or zero.
  reg signed [87:0] sum4;
  reg signed [11:0] least4;
  always @(posedge clk) begin
    sum4 <= has_a3 ? va3 : 88'sd0;
    least4 <= ea3;
  end
  // Stages 6 to 10: the one rounding.
  wire [31:0] rounded;
  logos_round88_p r(.clk(clk), .v(sum4), .least(least4), .f(rounded));
  always @* value = rounded;
endmodule
"#;
