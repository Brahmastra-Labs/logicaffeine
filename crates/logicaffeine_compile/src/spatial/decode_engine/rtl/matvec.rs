//! The array's matrix-vector step as Verilog: one weight block of two rows against one token-row's activations, from
//! the GGUF bytes to each row's next f32 ([`crate::spatial::decode_engine::numerics::dot`] under
//! [`ActScaling::PerBlock`](crate::spatial::decode_engine::numerics::ActScaling::PerBlock), a block at a time).
//!
//! The two rows' blocks pass their decoders; their codes meet the token-row's activation words in the processing
//! elements a 4-bit digit at a time — one digit for a code of four bits or fewer, two for a wider one, the high digit's
//! partials weighted by 16 — each 16-element unit's partials summed from its two elements; the units' `Σ q·a′`, the
//! words' `Σ a′`, the decoders' `Σ q` and multipliers feed each row's lane, which adds the block into the row.
//!
//! An activation code is 16 bits: a one-word code in `−127..127` crosses as `a + 128`; a two-word code as its signed
//! high byte plus 128 and its unsigned low byte. `logos_matvec_block #(UNITS, DIGITS)` is the format-free step;
//! `logos_matvec_<format>` puts the format's decoders in front of it; `logos_matvec_engine_<format>` is one column of
//! the array: the weight stream's two rows a beat, every token-row's codes held, a token-row a cycle, every
//! token-row's two rows emitted once their last block is in.

use std::fmt::Write as _;

use super::weights::{decoder_name, DECODERS, UNIT};

/// The matrix-vector step for every format, with the lane, the processing element, the decoders, the f32 units and the
/// block RAM they are built from.
pub fn matvec_verilog() -> String {
    let mut v = format!(
        "{}\n{}\n{}\n{}\n{}",
        super::ram::ram_verilog(&act_ram_widths()),
        super::lane::lane_verilog(),
        super::array::array_verilog(),
        super::weights::weights_verilog(),
        MATVEC_VERILOG
    );
    for d in DECODERS {
        let elems = d.ty.block_len();
        let units = elems / UNIT;
        let digits = if d.code_bits > 4 { 2 } else { 1 };
        let name = d.ty.name().to_lowercase();
        let bits = 8 * d.ty.block_bytes();
        writeln!(
            v,
            r#"
// Two rows' {fmt} blocks against one token-row.
module logos_matvec_{name}(input [{top}:0] blk1, input [{top}:0] blk2, input [{act_top}:0] act, input two, input [7:0] m,
                    input signed [11:0] e, input [31:0] acc1, input [31:0] acc2, output [31:0] out1, output [31:0] out2);
  wire [{code_top}:0] code1, code2;
  wire [{unit_top}:0] c1, n1, c2, n2;
  wire [15:0] a1, b1, a2, b2;
  {dec} d1(.blk(blk1), .code(code1), .c(c1), .n(n1), .a(a1), .b(b1));
  {dec} d2(.blk(blk2), .code(code2), .c(c2), .n(n2), .a(a2), .b(b2));
  logos_matvec_block #(.UNITS({units}), .DIGITS({digits})) u(.code1(code1), .code2(code2), .c1(c1), .n1(n1), .c2(c2), .n2(n2),
    .a1(a1), .b1(b1), .a2(a2), .b2(b2), .z(8'd{offset}), .act(act), .two(two), .m(m), .e(e), .acc1(acc1), .acc2(acc2),
    .out1(out1), .out2(out2));
endmodule"#,
            fmt = d.ty.name(),
            top = bits - 1,
            act_top = 16 * elems - 1,
            code_top = 8 * elems - 1,
            unit_top = 16 * units - 1,
            dec = decoder_name(d.ty),
            offset = d.offset,
        )
        .unwrap();
        writeln!(
            v,
            r#"
// A {fmt} matrix against up to T_MAX token-rows: two rows' blocks a beat from the weight stream, each applied to every
// token-row a cycle, every token-row's two rows emitted after their last block, then the next two rows.
module logos_matvec_engine_{name} #(parameter T_MAX = 4, NB_MAX = 8)
  (input clk, input rst,
   input start, input [15:0] pairs, input [15:0] blocks, input [7:0] tokens, input two, output busy,
   input act_wr, input [7:0] act_t, input [15:0] act_b, input [{act_top}:0] act_codes, input [7:0] act_m, input [11:0] act_e,
   input w_valid, output w_ready, input [{top}:0] w_blk1, input [{top}:0] w_blk2,
   output reg o_valid, output reg [15:0] o_pair, output reg [7:0] o_t, output reg [31:0] o_val1, output reg [31:0] o_val2);
  reg [15:0] pair, b;
  reg [7:0] t;
  // Each token-row's blocks of activations in block RAM: the codes, the scale's m and its exponent.
  localparam AN = T_MAX * NB_MAX, AA = (AN > 1) ? $clog2(AN) : 1;
  wire [{act_top} + 24:0] act_word;
  wire [31:0] act_ra = t * NB_MAX + b, act_wa = act_t * NB_MAX + act_b;
  {act_ram} #(.D(AN), .A(AA)) act_ram(.clk(clk), .ra(act_ra[AA-1:0]), .rd(act_word), .we(act_wr),
    .wa(act_wa[AA-1:0]), .wd({{4'd0, act_e, act_m, act_codes}}), .wm({{(({act_top} + 25) / 8){{1'b1}}}}));
  (* ramstyle = "logic" *) reg [31:0] acc1 [0:T_MAX-1];
  (* ramstyle = "logic" *) reg [31:0] acc2 [0:T_MAX-1];
  reg [{top}:0] blk1, blk2;
  localparam IDLE = 2'd0, WAIT = 2'd1, APPLY = 2'd2, EMIT = 2'd3;
  reg [1:0] state;
  assign busy = (state != IDLE);
  assign w_ready = (state == WAIT);
  wire [31:0] out1, out2;
  logos_matvec_{name} u(.blk1(blk1), .blk2(blk2), .act(act_word[{act_top}:0]), .two(two), .m(act_word[{act_top} + 8:{act_top} + 1]),
                    .e($signed(act_word[{act_top} + 20:{act_top} + 9])), .acc1(acc1[t]), .acc2(acc2[t]), .out1(out1), .out2(out2));
  integer k;
  always @(posedge clk) begin
    o_valid <= 1'b0;
    if (rst) state <= IDLE;
    else case (state)
      IDLE: if (start) begin
        pair <= 16'd0; b <= 16'd0; t <= 8'd0; state <= WAIT;
        for (k = 0; k < T_MAX; k = k + 1) begin acc1[k] <= 32'd0; acc2[k] <= 32'd0; end
      end
      WAIT: if (w_valid) begin blk1 <= w_blk1; blk2 <= w_blk2; t <= 8'd0; state <= APPLY; end
      APPLY: begin
        acc1[t] <= out1; acc2[t] <= out2;
        if (t + 8'd1 < tokens) t <= t + 8'd1;
        else if (b + 16'd1 < blocks) begin b <= b + 16'd1; state <= WAIT; end
        else begin t <= 8'd0; state <= EMIT; end
      end
      EMIT: begin
        o_valid <= 1'b1; o_pair <= pair; o_t <= t; o_val1 <= acc1[t]; o_val2 <= acc2[t];
        if (t + 8'd1 < tokens) t <= t + 8'd1;
        else begin
          for (k = 0; k < T_MAX; k = k + 1) begin acc1[k] <= 32'd0; acc2[k] <= 32'd0; end
          b <= 16'd0;
          if (pair + 16'd1 < pairs) begin pair <= pair + 16'd1; state <= WAIT; end
          else state <= IDLE;
        end
      end
    endcase
  end
endmodule"#,
            fmt = d.ty.name(),
            top = bits - 1,
            act_top = 16 * elems - 1,
            act_ram = super::ram::ram_name(16 * elems + 24),
        )
        .unwrap();
    }
    v
}

/// The widths of the columns' activation memories: a block's codes, its scale's m and its exponent.
pub(super) fn act_ram_widths() -> Vec<usize> {
    let mut w: Vec<usize> = DECODERS.iter().map(|d| 16 * d.ty.block_len() + 24).collect();
    w.sort();
    w.dedup();
    w
}

const MATVEC_VERILOG: &str = r#"
// One weight block of two rows against one token-row: the rows' unit sums from the processing elements, a 4-bit digit
// at a time, and each row's lane.
module logos_matvec_block #(parameter UNITS = 16, DIGITS = 2)
  (input [128*UNITS-1:0] code1, input [128*UNITS-1:0] code2,
   input [16*UNITS-1:0] c1, input [16*UNITS-1:0] n1, input [16*UNITS-1:0] c2, input [16*UNITS-1:0] n2,
   input [15:0] a1, input [15:0] b1, input [15:0] a2, input [15:0] b2, input [7:0] z,
   input [256*UNITS-1:0] act, input two, input [7:0] m, input signed [11:0] e,
   input [31:0] acc1, input [31:0] acc2, output [31:0] out1, output [31:0] out2);
  wire [24*UNITS-1:0] bp1_hi, bp2_hi, bp1_lo, bp2_lo;
  wire [12*UNITS-1:0] bs_hi, bs_lo, bq1, bq2;
  logos_matvec_partials #(.UNITS(UNITS), .DIGITS(DIGITS)) parts(.code1(code1), .code2(code2), .act(act), .two(two),
    .p1_hi(bp1_hi), .p2_hi(bp2_hi), .p1_lo(bp1_lo), .p2_lo(bp2_lo), .s_hi(bs_hi), .s_lo(bs_lo), .q1(bq1), .q2(bq2));
  logos_lane #(.UNITS(UNITS)) l1(.p_hi(bp1_hi), .s_hi(bs_hi), .p_lo(bp1_lo), .s_lo(bs_lo), .q(bq1), .c(c1), .n(n1), .z(z),
    .a(a1), .b(b1), .m(m), .e(e), .two(two), .acc(acc1), .out(out1));
  logos_lane #(.UNITS(UNITS)) l2(.p_hi(bp2_hi), .s_hi(bs_hi), .p_lo(bp2_lo), .s_lo(bs_lo), .q(bq2), .c(c2), .n(n2), .z(z),
    .a(a2), .b(b2), .m(m), .e(e), .two(two), .acc(acc2), .out(out2));
endmodule

// A block of two rows' codes against one token-row's activation words: each 16-element unit's partial sums for both
// rows from the processing elements, a 4-bit digit at a time, and the unit's sums of excess codes and of each row's
// codes, the buses the lanes take.
module logos_matvec_partials #(parameter UNITS = 16, DIGITS = 2)
  (input [128*UNITS-1:0] code1, input [128*UNITS-1:0] code2, input [256*UNITS-1:0] act, input two,
   output [24*UNITS-1:0] p1_hi, output [24*UNITS-1:0] p2_hi, output [24*UNITS-1:0] p1_lo, output [24*UNITS-1:0] p2_lo,
   output [12*UNITS-1:0] s_hi, output [12*UNITS-1:0] s_lo, output [12*UNITS-1:0] q1, output [12*UNITS-1:0] q2);
  // Each element's words: the excess high (or only) word, and the low byte of a two-word code.
  wire [128*UNITS-1:0] w_hi, w_lo;
  genvar i, u, d;
  generate
    for (i = 0; i < 16*UNITS; i = i + 1) begin : word
      wire signed [15:0] code = act[16*i +: 16];
      assign w_hi[8*i +: 8] = two ? (code[15:8] + 8'd128) : (code[7:0] + 8'd128);
      assign w_lo[8*i +: 8] = two ? code[7:0] : 8'd0;
    end
    for (u = 0; u < UNITS; u = u + 1) begin : unit
      // The unit's sums: the excess codes', each row's codes'.
      reg [11:0] s_hi, s_lo, q1s, q2s;
      integer k;
      always @* begin
        s_hi = 12'd0; s_lo = 12'd0; q1s = 12'd0; q2s = 12'd0;
        for (k = 0; k < 16; k = k + 1) begin
          s_hi = s_hi + w_hi[8*(16*u + k) +: 8];
          s_lo = s_lo + w_lo[8*(16*u + k) +: 8];
          q1s = q1s + code1[8*(16*u + k) +: 8];
          q2s = q2s + code2[8*(16*u + k) +: 8];
        end
      end
      // Per word and digit, the unit's two elements of eight: row one's partial in the low lanes, row two's in the
      // high lanes with its offset restored.
      wire [23:0] p1_hi, p2_hi, p1_lo, p2_lo;
      wire [23:0] dig1_hi [0:1], dig2_hi [0:1], dig1_lo [0:1], dig2_lo [0:1];
      for (d = 0; d < 2; d = d + 1) begin : digit
        wire [31:0] q1a, q2a, q1b, q2b;
        for (i = 0; i < 8; i = i + 1) begin : nib
          assign q1a[4*i +: 4] = (d < DIGITS) ? code1[8*(16*u + i) + 4*d +: 4] : 4'd0;
          assign q2a[4*i +: 4] = (d < DIGITS) ? code2[8*(16*u + i) + 4*d +: 4] : 4'd0;
          assign q1b[4*i +: 4] = (d < DIGITS) ? code1[8*(16*u + 8 + i) + 4*d +: 4] : 4'd0;
          assign q2b[4*i +: 4] = (d < DIGITS) ? code2[8*(16*u + 8 + i) + 4*d +: 4] : 4'd0;
        end
        wire [14:0] lo_ha, lo_hb, lo_la, lo_lb;
        wire signed [16:0] hi_ha, hi_hb, hi_la, hi_lb;
        logos_pe8 ha(.a(w_hi[8*16*u +: 64]), .q1(q1a), .q2(q2a), .lo(lo_ha), .hi(hi_ha));
        logos_pe8 hb(.a(w_hi[8*(16*u + 8) +: 64]), .q1(q1b), .q2(q2b), .lo(lo_hb), .hi(hi_hb));
        logos_pe8 la(.a(w_lo[8*16*u +: 64]), .q1(q1a), .q2(q2a), .lo(lo_la), .hi(hi_la));
        logos_pe8 lb(.a(w_lo[8*(16*u + 8) +: 64]), .q1(q1b), .q2(q2b), .lo(lo_lb), .hi(hi_lb));
        assign dig1_hi[d] = {9'd0, lo_ha} + {9'd0, lo_hb};
        assign dig2_hi[d] = $signed(hi_ha) + $signed(hi_hb) + $signed({9'd0, s_hi, 3'd0});
        assign dig1_lo[d] = {9'd0, lo_la} + {9'd0, lo_lb};
        assign dig2_lo[d] = $signed(hi_la) + $signed(hi_lb) + $signed({9'd0, s_lo, 3'd0});
      end
      assign p1_hi = dig1_hi[0] + (dig1_hi[1] << 4);
      assign p2_hi = dig2_hi[0] + (dig2_hi[1] << 4);
      assign p1_lo = dig1_lo[0] + (dig1_lo[1] << 4);
      assign p2_lo = dig2_lo[0] + (dig2_lo[1] << 4);
    end
  endgenerate
  // The lanes' unit buses.
  generate
    for (u = 0; u < UNITS; u = u + 1) begin : bus
      assign p1_hi[24*u +: 24] = unit[u].p1_hi;
      assign p2_hi[24*u +: 24] = unit[u].p2_hi;
      assign p1_lo[24*u +: 24] = unit[u].p1_lo;
      assign p2_lo[24*u +: 24] = unit[u].p2_lo;
      assign s_hi[12*u +: 12] = unit[u].s_hi;
      assign s_lo[12*u +: 12] = unit[u].s_lo;
      assign q1[12*u +: 12] = unit[u].q1s;
      assign q2[12*u +: 12] = unit[u].q2s;
    end
  endgenerate
endmodule
"#;
