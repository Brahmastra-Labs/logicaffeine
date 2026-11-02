//! The attention unit's vector path as Verilog: one query head over its sequence's cached rows, exactly as
//! [`crate::spatial::decode_engine::numerics::attend_vector`] computes it.
//!
//! Three passes over the positions. The keys stream in, `HD/16` beats of sixteen elements a position: each beat's
//! products with the head's queries accumulate in sixteen lanes (element `i` in lane `i mod 16`), the lanes fold by the
//! pairwise tree, the dot times the scale is the position's score, kept, and the scores' maximum is taken as the
//! softmax takes it (a NaN never larger, a largest zero `+0`). Then each `exp(s − max)` is kept and summed in sixteen
//! lanes by position, the sum folded by the tree and its reciprocal taken. Then the values stream in, `VD/16` beats a
//! position: each position's probability `e·(1/Σ)` times its row adds into lane `position mod 16` of every element,
//! and the lanes fold by the tree into the head's output, sixteen elements a beat.

/// The unit's two paths, with the f32 units, the functions, the lane's one rounding and the block RAM they are built
/// from.
pub fn attend_verilog() -> String {
    format!(
        "{}\n{}\n{}\n{}\n{}",
        super::ram::ram_verilog(&[32]),
        super::vector::vector_verilog(),
        ROUND_VERILOG,
        ATTEND_VERILOG,
        ATTEND_INT8_VERILOG
    )
}

/// The lane's rounding, `v·2^least` to f32 once, without the lane around it.
const ROUND_VERILOG: &str = r#"
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
"#;

const ATTEND_VERILOG: &str = r#"
// One query head of HD elements over up to P_MAX positions of VD-element values.
module logos_attend_vector #(parameter HD = 64, VD = 64, P_MAX = 64)
  (input clk, input rst,
   input start, input [15:0] positions, input [32*HD-1:0] q, input [31:0] scale, output busy,
   input k_valid, input [511:0] k_beat, output k_ready,
   input v_valid, input [511:0] v_beat, output v_ready,
   output reg o_valid, output reg [511:0] o_beat, output reg o_last);
  localparam [15:0] KB = HD / 16, VB = VD / 16;
  localparam IDLE = 3'd0, KEY = 3'd1, FOLD = 3'd2, EXP = 3'd3, INV = 3'd4, VALUE = 3'd5, OUT = 3'd6;
  reg [2:0] state;
  reg [15:0] j, b, n;
  reg [31:0] acc [0:16*VD-1];
  reg [511:0] lanes;
  reg [31:0] m, sc, inv;
  reg [32*HD-1:0] qh;
  assign busy = (state != IDLE);
  assign k_ready = (state == KEY);
  assign v_ready = (state == VALUE);

  // A key beat's products into its sixteen lanes.
  wire [511:0] q_beat = qh[512*b +: 512];
  wire [511:0] k_next;
  genvar l, t;
  generate
    for (l = 0; l < 16; l = l + 1) begin : kl
      wire [31:0] p;
      logos_fmul mk(.a(q_beat[32*l +: 32]), .b(k_beat[32*l +: 32]), .y(p));
      logos_fadd ak(.a(b == 16'd0 ? 32'd0 : lanes[32*l +: 32]), .b(p), .y(k_next[32*l +: 32]));
    end
  endgenerate
  // The fold of a key's lanes: its dot, its score, whether it is the largest yet.
  wire [31:0] dot, score;
  logos_tree16 tk(.lanes(lanes), .sum(dot));
  logos_fmul ms(.a(dot), .b(sc), .y(score));
  wire score_nan = (score[30:23] == 8'hff) && (score[22:0] != 23'd0);
  wire m_lt;
  logos_flt cmp(.a(m), .b(score), .y(m_lt));
  // The largest score, a largest zero +0.
  wire [31:0] mx = (m[30:0] == 31'd0) ? 32'd0 : m;
  // One position's exp(s - max), its score and exp read back from block RAM.
  wire [31:0] score_j, ex_j;
  wire [31:0] diff, e_j;
  logos_fsub sd(.a(score_j), .b(mx), .y(diff));
  logos_exp xe(.x_in(diff), .y(e_j));
  wire [31:0] e_lane, e_next;
  assign e_lane = lanes[32*j[3:0] +: 32];
  logos_fadd ae(.a(e_lane), .b(e_j), .y(e_next));
  // The sum of the exps and its reciprocal.
  wire [31:0] total, total_inv;
  logos_tree16 te(.lanes(lanes), .sum(total));
  logos_recip rr(.x_in(total), .y(total_inv));
  // A value beat: the position's probability times each element into lane j mod 16 of its accumulator.
  wire [31:0] prob;
  logos_fmul mp(.a(ex_j), .b(inv), .y(prob));
  wire [511:0] v_next;
  generate
    for (l = 0; l < 16; l = l + 1) begin : vl
      wire [31:0] pv;
      logos_fmul mv(.a(prob), .b(v_beat[32*l +: 32]), .y(pv));
      logos_fadd av(.a(acc[VD*j[3:0] + 16*b + l]), .b(pv), .y(v_next[32*l +: 32]));
    end
  endgenerate
  // An output beat: each of its sixteen elements' sixteen position-lanes folded by the tree.
  wire [511:0] out_beat;
  generate
    for (l = 0; l < 16; l = l + 1) begin : ol
      wire [511:0] col;
      for (t = 0; t < 16; t = t + 1) begin : ot
        assign col[32*t +: 32] = acc[VD*t + 16*b + l];
      end
      logos_tree16 to(.lanes(col), .sum(out_beat[32*l +: 32]));
    end
  endgenerate

  integer k;
  // The scores and their exponentials in block RAM, each read and written at the position at hand.
  localparam PA = (P_MAX > 1) ? $clog2(P_MAX) : 1;
  logos_ram_w32 #(.D(P_MAX), .A(PA)) score_ram(.clk(clk), .ra(j[PA-1:0]), .rd(score_j), .we(!rst && state == FOLD),
    .wa(j[PA-1:0]), .wd(score), .wm(4'hf));
  logos_ram_w32 #(.D(P_MAX), .A(PA)) ex_ram(.clk(clk), .ra(j[PA-1:0]), .rd(ex_j), .we(!rst && state == EXP),
    .wa(j[PA-1:0]), .wd(e_j), .wm(4'hf));
  always @(posedge clk) begin
    o_valid <= 1'b0;
    o_last <= 1'b0;
    if (rst) state <= IDLE;
    else case (state)
      IDLE: if (start) begin
        qh <= q; sc <= scale; n <= positions; j <= 16'd0; b <= 16'd0; m <= 32'hff80_0000;
        for (k = 0; k < 16*VD; k = k + 1) acc[k] <= 32'd0;
        state <= (positions == 16'd0) ? OUT : KEY;
      end
      KEY: if (k_valid) begin
        lanes <= k_next;
        if (b + 16'd1 == KB) begin b <= 16'd0; state <= FOLD; end else b <= b + 16'd1;
      end
      FOLD: begin
        if (!score_nan && m_lt) m <= score;
        if (j + 16'd1 == n) begin j <= 16'd0; lanes <= 512'd0; state <= EXP; end
        else begin j <= j + 16'd1; state <= KEY; end
      end
      EXP: begin
        lanes[32*j[3:0] +: 32] <= e_next;
        if (j + 16'd1 == n) begin j <= 16'd0; state <= INV; end else j <= j + 16'd1;
      end
      INV: begin inv <= total_inv; b <= 16'd0; state <= VALUE; end
      VALUE: if (v_valid) begin
        for (k = 0; k < 16; k = k + 1) acc[VD*j[3:0] + 16*b + k] <= v_next[32*k +: 32];
        if (b + 16'd1 == VB) begin
          b <= 16'd0;
          if (j + 16'd1 == n) state <= OUT; else j <= j + 16'd1;
        end else b <= b + 16'd1;
      end
      OUT: begin
        o_valid <= 1'b1;
        o_beat <= out_beat;
        if (b + 16'd1 == VB) begin o_last <= 1'b1; b <= 16'd0; state <= IDLE; end else b <= b + 16'd1;
      end
    endcase
  end
endmodule
"#;

const ATTEND_INT8_VERILOG: &str = r#"
// One query head of HD elements over up to P_MAX positions of an int8 cache on the array, the query one block (its
// codes and its scale m·2^e): each key row's codes against the query's, exact, times the key's f16 significand and m,
// rounded once and times the score scale; the softmax; then per block of 32 positions the probabilities times the
// value rows' scales, their maximum's exponent, each an 8-bit code to nearest even, every element's exact integer sum
// against the value codes, rounded once a block and added in order.
module logos_attend_int8 #(parameter HD = 64, VD = 64, P_MAX = 64)
  (input clk, input rst,
   input start, input [15:0] positions, input [16*HD-1:0] q_codes, input [7:0] q_m, input signed [11:0] q_e,
   input [31:0] scale, output busy,
   input k_valid, input [8*HD-1:0] k_codes, input [15:0] k_scale, output k_ready,
   input s_valid, input [15:0] v_scale, output s_ready,
   input v_valid, input [8*VD-1:0] v_codes, output v_ready,
   output reg o_valid, output reg [511:0] o_beat, output reg o_last);
  localparam [15:0] OB = VD / 16, ELEMS = VD;
  localparam IDLE = 4'd0, KEY = 4'd1, EXP = 4'd2, INV = 4'd3, WSCALE = 4'd4, WCODE = 4'd5, WVAL = 4'd6, WROUND = 4'd7, OUT = 4'd8;
  reg [3:0] state;
  reg [15:0] j, n, blk, b, el;
  `LOGOS_RAMSTYLE_ASYNC(32) reg [31:0] w [0:31];
  reg signed [31:0] sums [0:VD-1];
  reg [31:0] out [0:VD-1];
  reg [511:0] lanes;
  reg [31:0] m, sc, inv, wmax;
  reg [16*HD-1:0] qc;
  reg [7:0] qm;
  reg signed [11:0] qe;
  reg signed [11:0] we;
  assign busy = (state != IDLE);
  assign k_ready = (state == KEY);
  assign s_ready = (state == WSCALE);
  assign v_ready = (state == WVAL);
  // The key row's exact dot with the query's codes.
  reg signed [47:0] kdot;
  integer i;
  always @* begin
    kdot = 48'sd0;
    for (i = 0; i < HD; i = i + 1) kdot = kdot + $signed(qc[16*i +: 16]) * $signed(k_codes[8*i +: 8]);
  end
  // The key's f16 scale as a signed significand and an exponent, a denormal's the least.
  wire signed [12:0] k_sig = (k_scale[14:10] == 5'd0) ? (k_scale[15] ? -$signed({3'd0, k_scale[9:0]}) : $signed({3'd0, k_scale[9:0]}))
                                                    : (k_scale[15] ? -$signed({2'd0, 1'b1, k_scale[9:0]}) : $signed({2'd0, 1'b1, k_scale[9:0]}));
  wire signed [11:0] k_exp = (k_scale[14:10] == 5'd0) ? -12'sd24 : $signed({7'd0, k_scale[14:10]}) - 12'sd25;
  wire signed [87:0] term = $signed(k_sig) * $signed({1'b0, qm}) * kdot;
  wire [31:0] rounded, score;
  logos_round88 rs(.v(term), .least(k_exp + qe), .f(rounded));
  logos_fmul ms(.a(rounded), .b(sc), .y(score));
  wire score_nan = (score[30:23] == 8'hff) && (score[22:0] != 23'd0);
  wire m_lt;
  logos_flt cmp(.a(m), .b(score), .y(m_lt));
  wire [31:0] mx = (m[30:0] == 31'd0) ? 32'd0 : m;
  wire [31:0] score_j, ex_j;
  wire [31:0] diff, e_j, e_next;
  logos_fsub sd(.a(score_j), .b(mx), .y(diff));
  logos_exp xe(.x_in(diff), .y(e_j));
  logos_fadd ae(.a(lanes[32*j[3:0] +: 32]), .b(e_j), .y(e_next));
  wire [31:0] total, total_inv;
  logos_tree16 te(.lanes(lanes), .sum(total));
  logos_recip rr(.x_in(total), .y(total_inv));
  // A position's weight: its probability times its value row's scale.
  wire [31:0] prob, vs, wj;
  logos_fmul mp(.a(ex_j), .b(inv), .y(prob));
  logos_f16_to_f32 cv(.h(v_scale), .x(vs));
  logos_fmul mw(.a(prob), .b(vs), .y(wj));
  // The block's exponent: the least e with wmax <= 255·2^e.
  wire [23:0] wm = {1'b1, wmax[22:0]};
  wire signed [11:0] w_exp = $signed({4'd0, wmax[30:23]}) - 12'sd150 + 12'sd16 + ((wm > 24'd16711680) ? 12'sd1 : 12'sd0);
  // A weight's 8-bit code: its significand shifted down to the block's exponent, nearest even.
  wire [31:0] wv = w[j[4:0]];
  wire [23:0] vm = {1'b1, wv[22:0]};
  wire signed [11:0] shift = we - ($signed({4'd0, wv[30:23]}) - 12'sd150);
  wire [23:0] kept = (shift > 12'sd24) ? 24'd0 : (vm >> shift);
  wire [23:0] rem = (shift > 12'sd24) ? vm : (vm & ((24'd1 << shift) - 24'd1));
  wire [24:0] halfw = (shift > 12'sd24) ? 25'h1000000 : (25'd1 << (shift - 1));
  wire up = ({1'b0, rem} > halfw) || ({1'b0, rem} == halfw && kept[0]);
  wire [8:0] code = (wv[30:0] == 31'd0) ? 9'd0 : (kept[8:0] + {8'd0, up});
  // An element's block sum rounded once and added into its output.
  // A block whose weights all vanish adds +0 to every element.
  wire [31:0] block_value, added, zeroed;
  logos_round88 rb(.v({{56{sums[el][31]}}, sums[el]}), .least(we), .f(block_value));
  logos_fadd ao(.a(out[el]), .b(block_value), .y(added));
  logos_fadd az(.a(out[el]), .b(32'd0), .y(zeroed));
  // The output beat: sixteen elements.
  wire [511:0] out_beat;
  genvar l;
  generate
    for (l = 0; l < 16; l = l + 1) begin : ol
      assign out_beat[32*l +: 32] = out[16*b + l];
    end
  endgenerate
  integer k;
  // The scores and their exponentials in block RAM, each read and written at the position at hand.
  localparam PA = (P_MAX > 1) ? $clog2(P_MAX) : 1;
  logos_ram_w32 #(.D(P_MAX), .A(PA)) score_ram(.clk(clk), .ra(j[PA-1:0]), .rd(score_j), .we(!rst && state == KEY && k_valid),
    .wa(j[PA-1:0]), .wd(score), .wm(4'hf));
  logos_ram_w32 #(.D(P_MAX), .A(PA)) ex_ram(.clk(clk), .ra(j[PA-1:0]), .rd(ex_j), .we(!rst && state == EXP),
    .wa(j[PA-1:0]), .wd(e_j), .wm(4'hf));
  always @(posedge clk) begin
    o_valid <= 1'b0;
    o_last <= 1'b0;
    if (rst) state <= IDLE;
    else case (state)
      IDLE: if (start) begin
        qc <= q_codes; qm <= q_m; qe <= q_e; sc <= scale; n <= positions; j <= 16'd0; m <= 32'hff80_0000;
        for (k = 0; k < VD; k = k + 1) out[k] <= 32'd0;
        state <= KEY;
      end
      KEY: if (k_valid) begin
        if (!score_nan && m_lt) m <= score;
        if (j + 16'd1 == n) begin j <= 16'd0; lanes <= 512'd0; state <= EXP; end else j <= j + 16'd1;
      end
      EXP: begin
        lanes[32*j[3:0] +: 32] <= e_next;
        if (j + 16'd1 == n) begin j <= 16'd0; state <= INV; end else j <= j + 16'd1;
      end
      INV: begin inv <= total_inv; blk <= 16'd0; wmax <= 32'd0; state <= WSCALE; end
      WSCALE: if (s_valid) begin
        w[j[4:0]] <= wj;
        if (wj[30:0] > wmax[30:0]) wmax <= wj;
        if (j[4:0] == 5'd31 || j + 16'd1 == n) begin
          j <= {blk[10:0], 5'd0};
          state <= WCODE;
        end else j <= j + 16'd1;
      end
      WCODE: begin
        we <= w_exp;
        for (k = 0; k < VD; k = k + 1) sums[k] <= 32'sd0;
        state <= WVAL;
      end
      WVAL: if (v_valid) begin
        for (k = 0; k < VD; k = k + 1) sums[k] <= sums[k] + $signed({1'b0, code}) * $signed(v_codes[8*k +: 8]);
        if (j[4:0] == 5'd31 || j + 16'd1 == n) begin el <= 16'd0; state <= WROUND; end else j <= j + 16'd1;
      end
      WROUND: begin
        out[el] <= (wmax[30:0] == 31'd0) ? zeroed : added;
        if (el + 16'd1 == ELEMS) begin
          el <= 16'd0;
          if ({blk[10:0], 5'd0} + 16'd32 >= n) begin b <= 16'd0; state <= OUT; end
          else begin blk <= blk + 16'd1; j <= {blk[10:0], 5'd0} + 16'd32; wmax <= 32'd0; state <= WSCALE; end
        end else el <= el + 16'd1;
      end
      OUT: begin
        o_valid <= 1'b1;
        o_beat <= out_beat;
        if (b + 16'd1 == OB) begin o_last <= 1'b1; state <= IDLE; end else b <= b + 16'd1;
      end
    endcase
  end
endmodule
"#;

/// `logos_attend_int8_p`, `ATTEND_INT8_VERILOG`'s unit for the user clock, with the units it is built from.
pub fn attend_int8_p_verilog() -> String {
    use super::fp32::{FADD_LATENCY, FMUL_LATENCY, TREE16_LATENCY};
    use super::vector::{EXP_LATENCY, RECIP_LATENCY};
    let units = [
        ("@FMUL@", FMUL_LATENCY),
        ("@FADD@", FADD_LATENCY),
        ("@TREE@", TREE16_LATENCY),
        ("@EXP@", EXP_LATENCY),
        ("@RECIP@", RECIP_LATENCY),
        ("@ROUND@", super::lane::ROUND88_P_LATENCY),
    ]
    .iter()
    .fold(ATTEND_INT8_P_VERILOG.to_string(), |t, (k, v)| t.replace(k, &v.to_string()));
    format!("{}\n{}\n{}\n{units}", super::vector::vector_verilog(), super::lane::lane_verilog(), super::ram::bram_verilog(&[32]))
}

const ATTEND_INT8_P_VERILOG: &str = r#"
// logos_attend_int8 for the user clock: one query head of HD elements over up to P_MAX positions of an int8 cache, a
// line of 64 codes a clock. The query is one block (its codes, int8 or two-word, and its scale m·2^e). Each key row's
// lines — the row's f16 scale with every line — make the row's exact dot with the query's codes, a line's 64 products
// summed in a tree and the row's lines added; times the scale's significand and m, rounded once at their exponents,
// times the score scale: the position's score, kept, and their maximum as the softmax takes it. Then each
// exp(score − max), kept and summed in sixteen lanes by position, the lanes folded and the sum's reciprocal taken. Then
// per block of 32 positions: each position's probability times its value row's f16 scale (a scale a clock), the block's
// exponent from their maximum, each weight's 8-bit code to nearest even at it; the value rows' lines, each element's
// exact integer sum against the codes; every element's sum rounded once at the exponent and added into its output, the
// blocks in order (a block of no weight adds +0). The output VD/16 beats, busy until the last is out.
module logos_attend_int8_p #(parameter HD = 256, VD = 256, P_MAX = 4096)
  (input clk, input rst,
   input start, input [15:0] positions, input [16*HD-1:0] q_codes, input [7:0] q_m, input signed [11:0] q_e,
   input [31:0] scale, output busy,
   input k_valid, input [511:0] k_line, input [15:0] k_scale, output k_ready,
   input s_valid, input [15:0] v_scale, output s_ready,
   input v_valid, input [511:0] v_line, output v_ready,
   output reg o_valid, output reg [511:0] o_beat, output reg o_last);
  localparam [15:0] KL = HD / 64, VL = VD / 64, OB = VD / 16, VN = VD;
  localparam PA = (P_MAX > 1) ? $clog2(P_MAX) : 1;
  localparam IDLE = 4'd0, KEY = 4'd1, KDRAIN = 4'd2, EXP = 4'd3, EDRAIN = 4'd4, INV = 4'd5, WSCALE = 4'd6, WCODE = 4'd7,
             WVAL = 4'd8, WROUND = 4'd9, WDRAIN = 4'd10, OUT = 4'd11, WCODES = 4'd12, WVD = 4'd13;
  // Clocks after an exponential's position is asked to its value (EX) and its lane's sum (EL); after a scale is taken
  // to its weight (WL); after an element's sum is taken to its output's sum (RL).
  localparam integer EX = 1 + @FADD@ + @EXP@, EL = EX + @FADD@, WL = 1 + 2 * @FMUL@, RL = @ROUND@ + @FADD@ + 1;
  reg [3:0] state;
  reg [15:0] n, j, li, blk, b, landed;
  reg [16*HD-1:0] qc;
  reg [7:0] qm;
  reg signed [11:0] qe;
  reg [31:0] sc;
  assign busy = state != IDLE || o_valid;
  assign k_ready = state == KEY;
  assign s_ready = state == WSCALE && j != blk * 16'd32 + 16'd32 && j != n;
  assign v_ready = state == WVAL;
  // ── Keys: a line's products registered, the tree's two levels, the row's sum; the row's scale products; the
  // rounding; the score scale.
  wire k_take = k_valid && k_ready;
  reg signed [23:0] kp [0:63];
  reg kp_v, kp_first, kp_last;
  reg [15:0] kp_scale;
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 64; i = i + 1) kp[i] <= $signed(qc[16*(64*li + i) +: 16]) * $signed(k_line[8*i +: 8]);
    kp_v <= !rst && k_take;
    kp_first <= li == 16'd0;
    kp_last <= li == KL - 1;
    kp_scale <= k_scale;
  end
  // Eight products a partial and eight partials a line, each in a balanced tree of three adds.
  function signed [29:0] tree8(input signed [23:0] a0, input signed [23:0] a1, input signed [23:0] a2,
                               input signed [23:0] a3, input signed [23:0] a4, input signed [23:0] a5,
                               input signed [23:0] a6, input signed [23:0] a7);
    reg signed [24:0] b0, b1, b2, b3;
    reg signed [25:0] c0, c1;
    begin
      b0 = a0 + a1; b1 = a2 + a3; b2 = a4 + a5; b3 = a6 + a7;
      c0 = b0 + b1; c1 = b2 + b3;
      tree8 = c0 + c1;
    end
  endfunction
  reg signed [29:0] kt [0:7];
  reg kt_v, kt_first, kt_last;
  reg [15:0] kt_scale;
  integer t;
  always @(posedge clk) begin
    for (t = 0; t < 8; t = t + 1)
      kt[t] <= tree8(kp[8*t], kp[8*t+1], kp[8*t+2], kp[8*t+3], kp[8*t+4], kp[8*t+5], kp[8*t+6], kp[8*t+7]);
    kt_v <= !rst && kp_v; kt_first <= kp_first; kt_last <= kp_last; kt_scale <= kp_scale;
  end
  reg signed [47:0] kdot;
  reg kd_v;
  reg [15:0] kd_scale;
  wire signed [32:0] l0 = kt[0] + kt[1], l1 = kt[2] + kt[3], l2 = kt[4] + kt[5], l3 = kt[6] + kt[7];
  wire signed [33:0] l4 = l0 + l1, l5 = l2 + l3;
  wire signed [47:0] line_sum = l4 + l5;
  always @(posedge clk) begin
    if (kt_v) kdot <= (kt_first ? 48'sd0 : kdot) + line_sum;
    kd_v <= !rst && kt_v && kt_last;
    if (kt_v && kt_last) kd_scale <= kt_scale;
  end
  // The row's scale as a signed significand and an exponent, a denormal's the least; times m, times the dot.
  wire signed [12:0] k_sig = (kd_scale[14:10] == 5'd0) ? (kd_scale[15] ? -$signed({3'd0, kd_scale[9:0]}) : $signed({3'd0, kd_scale[9:0]}))
                                                      : (kd_scale[15] ? -$signed({2'd0, 1'b1, kd_scale[9:0]}) : $signed({2'd0, 1'b1, kd_scale[9:0]}));
  wire signed [11:0] k_exp = (kd_scale[14:10] == 5'd0) ? -12'sd24 : $signed({7'd0, kd_scale[14:10]}) - 12'sd25;
  reg signed [21:0] sm;
  reg signed [47:0] kdot2;
  reg signed [11:0] least2;
  reg sm_v;
  always @(posedge clk) begin
    sm <= $signed(k_sig) * $signed({1'b0, qm});
    kdot2 <= kdot;
    least2 <= k_exp + qe;
    sm_v <= !rst && kd_v;
  end
  reg signed [87:0] term;
  reg signed [11:0] least3;
  reg tm_v;
  always @(posedge clk) begin
    term <= $signed(sm) * $signed(kdot2);
    least3 <= least2;
    tm_v <= !rst && sm_v;
  end
  wire [31:0] rounded, score;
  logos_round88_p rs(.clk(clk), .v(term), .least(least3), .f(rounded));
  logos_fmul_p ms(.clk(clk), .a(rounded), .b(sc), .y(score));
  reg [@ROUND@ + @FMUL@:1] sv_d;
  always @(posedge clk) sv_d <= rst ? {(@ROUND@ + @FMUL@){1'b0}} : {sv_d[@ROUND@ + @FMUL@ - 1:1], tm_v};
  wire score_in = sv_d[@ROUND@ + @FMUL@];
  reg [15:0] sj;
  // The scores' maximum: a NaN never larger, a largest zero +0.
  reg [31:0] m;
  wire score_nan = (score[30:23] == 8'hff) && (score[22:0] != 23'd0);
  wire m_lt;
  logos_flt cmp(.a(m), .b(score), .y(m_lt));
  wire [31:0] mx = (m[30:0] == 31'd0) ? 32'd0 : m;
  // ── Exponentials: a score read, less the maximum, through the exponential into its lane by position and kept.
  wire [31:0] score_j, ex_j;
  wire [31:0] diff, e_j;
  reg [15:0] er;
  logos_fadd_p ed(.clk(clk), .a(score_j), .b({~mx[31], mx[30:0]}), .y(diff));
  logos_exp_p xe(.clk(clk), .x_in(diff), .y(e_j));
  reg [EL:1] ev_d;
  reg [15:0] ej_d [1:EL];
  always @(posedge clk) begin
    ev_d <= rst ? {EL{1'b0}} : {ev_d[EL-1:1], state == EXP && er != n};
    ej_d[1] <= er;
    for (i = 2; i <= EL; i = i + 1) ej_d[i] <= ej_d[i-1];
  end
  wire e_in = ev_d[EX];
  wire [15:0] e_at = ej_d[EX];
  reg [511:0] lanes;
  wire [31:0] lane_sum;
  logos_fadd_p el_add(.clk(clk), .a(lanes[32*e_at[3:0] +: 32]), .b(e_j), .y(lane_sum));
  wire lane_in = ev_d[EL];
  wire [15:0] lane_at = ej_d[EL];
  wire [31:0] total, total_inv;
  logos_tree16_p te(.clk(clk), .lanes(lanes), .sum(total));
  logos_recip_p rr(.clk(clk), .x_in(total), .y(total_inv));
  reg [15:0] wait_c;
  reg [31:0] inv;
  logos_bram_w32 #(.D(P_MAX), .A(PA)) score_ram(.clk(clk), .ra(er[PA-1:0]), .rd(score_j),
    .we(score_in), .wa(sj[PA-1:0]), .wd(score), .wm(4'hf));
  logos_bram_w32 #(.D(P_MAX), .A(PA)) ex_ram(.clk(clk), .ra(j[PA-1:0]), .rd(ex_j), .we(e_in), .wa(e_at[PA-1:0]), .wd(e_j),
    .wm(4'hf));
  // ── Weights: a position's probability times its value row's scale, into the block's weights.
  wire [31:0] prob, vs_c, wj;
  reg [31:0] vs;
  reg [15:0] vs_q;
  logos_fmul_p mp(.clk(clk), .a(ex_j), .b(inv), .y(prob));
  reg [15:0] vs_d [1:@FMUL@];
  always @(posedge clk) begin
    vs_d[1] <= vs_q;
    for (i = 2; i <= @FMUL@; i = i + 1) vs_d[i] <= vs_d[i-1];
  end
  // The row's scale widened from binary16 and registered, meeting its probability out of the multiply.
  logos_f16_to_f32 cv(.h(vs_d[@FMUL@ - 1]), .x(vs_c));
  always @(posedge clk) vs <= vs_c;
  logos_fmul_p mw(.clk(clk), .a(prob), .b(vs), .y(wj));
  reg [WL:1] wv_d;
  reg [4:0] wj_d [1:WL];
  always @(posedge clk) begin
    wv_d <= rst ? {WL{1'b0}} : {wv_d[WL-1:1], s_valid && s_ready};
    wj_d[1] <= j[4:0];
    for (i = 2; i <= WL; i = i + 1) wj_d[i] <= wj_d[i-1];
    vs_q <= v_scale;
  end
  wire w_in = wv_d[WL];
  (* ramstyle = "logic" *) reg [31:0] w [0:31];
  reg [31:0] wmax;
  reg [5:0] w_got;
  // The block's exponent: the least e with wmax <= 255·2^e.
  wire [23:0] wm = {1'b1, wmax[22:0]};
  wire signed [11:0] w_exp = $signed({4'd0, wmax[30:23]}) - 12'sd150 + 12'sd16 + ((wm > 24'd16711680) ? 12'sd1 : 12'sd0);
  reg signed [11:0] we;
  // ── Values: each position's code from its weight at the block's exponent, nearest even, for every position of the
  // block once its exponent is set — four a clock, the weights registered, then each cut at the exponent (its kept
  // bits, its remainder and the remainder's half), then rounded into its code (codes); then each value line's 64
  // products with its position's code registered, and added into their sums the clock after, each sum by its own
  // enable, so a block's last sum is in the clock after its last line.
  function [58:0] code_cut(input [31:0] wv, input signed [11:0] e);
    reg [23:0] vm, kept, rem;
    reg [24:0] halfw;
    reg signed [11:0] shift;
    begin
      vm = {1'b1, wv[22:0]};
      shift = e - ($signed({4'd0, wv[30:23]}) - 12'sd150);
      kept = (shift > 12'sd24) ? 24'd0 : (vm >> shift);
      rem = (shift > 12'sd24) ? vm : (vm & ((24'd1 << shift) - 24'd1));
      halfw = (shift > 12'sd24) ? 25'h1000000 : (25'd1 << (shift - 1));
      code_cut = {wv[30:0] == 31'd0, kept[8:0], rem, halfw};
    end
  endfunction
  function [8:0] code_round(input [58:0] c);
    reg zero, up;
    reg [8:0] kept;
    reg [23:0] rem;
    reg [24:0] halfw;
    begin
      {zero, kept, rem, halfw} = c;
      up = ({1'b0, rem} > halfw) || ({1'b0, rem} == halfw && kept[0]);
      code_round = zero ? 9'd0 : (kept + {8'd0, up});
    end
  endfunction
  reg [8:0] codes [0:31];
  reg [31:0] cw [0:3];
  reg [58:0] cut [0:3];
  reg [3:0] cj;
  reg cw_v, cut_v;
  reg [2:0] cw_at, cut_at;
  reg signed [17:0] vp [0:63];
  reg vp_v;
  reg [15:0] vp_li;
  wire v_take = v_valid && v_ready;
  integer e;
  always @(posedge clk) begin
    for (e = 0; e < 4; e = e + 1) cut[e] <= code_cut(cw[e], we);
    cut_v <= !rst && cw_v;
    cut_at <= cw_at;
    for (e = 0; e < 32; e = e + 1) if (cut_v && cut_at == e[4:2]) codes[e] <= code_round(cut[e % 4]);
    vp_v <= !rst && v_take;
    vp_li <= li;
    for (e = 0; e < 64; e = e + 1) vp[e] <= $signed({1'b0, codes[j[4:0]]}) * $signed(v_line[8*e +: 8]);
  end
  reg signed [31:0] sums [0:VD-1];
  // ── Rounding: an element's sum rounded once at the block's exponent, added into its output.
  wire [31:0] block_value, added;
  reg signed [31:0] rsum;
  reg [15:0] rel;
  reg r_v, r_zero;
  logos_round88_p rb(.clk(clk), .v({{56{rsum[31]}}, rsum}), .least(we), .f(block_value));
  reg [31:0] out [0:VD-1];
  reg [@ROUND@:1] rv_d, rz_d;
  reg [15:0] rel_d [1:RL];
  always @(posedge clk) begin
    rv_d <= rst ? {@ROUND@{1'b0}} : {rv_d[@ROUND@-1:1], r_v};
    rz_d <= {rz_d[@ROUND@-1:1], r_zero};
    rel_d[1] <= rel;
    for (i = 2; i <= RL; i = i + 1) rel_d[i] <= rel_d[i-1];
  end
  reg [31:0] add_a;
  logos_fadd_p ao(.clk(clk), .a(add_a), .b(rz_d[@ROUND@] ? 32'd0 : block_value), .y(added));
  reg [@FADD@:1] av_d;
  always @(posedge clk) begin
    add_a <= out[rel_d[@ROUND@]];
    av_d <= rst ? {@FADD@{1'b0}} : {av_d[@FADD@-1:1], rv_d[@ROUND@]};
  end
  wire out_in = av_d[@FADD@];
  wire [15:0] out_at = rel_d[RL];
  // The output beat: sixteen elements.
  wire [511:0] out_beat;
  genvar l;
  generate
    for (l = 0; l < 16; l = l + 1) begin : ol
      assign out_beat[32*l +: 32] = out[16*b + l];
    end
  endgenerate
  integer k;
  always @(posedge clk) begin
    o_valid <= 1'b0;
    o_last <= 1'b0;
    r_v <= 1'b0;
    if (score_in) begin
      sj <= sj + 16'd1;
      if (!score_nan && m_lt) m <= score;
    end
    if (lane_in) lanes[32*lane_at[3:0] +: 32] <= lane_sum;
    if (w_in) begin
      w[wj_d[WL]] <= wj;
      if (wj[30:0] > wmax[30:0]) wmax <= wj;
      w_got <= w_got + 6'd1;
    end
    if (out_in) begin out[out_at] <= added; landed <= landed + 16'd1; end
    if (rst) state <= IDLE;
    else case (state)
      IDLE: if (start) begin
        qc <= q_codes; qm <= q_m; qe <= q_e; sc <= scale; n <= positions; j <= 16'd0; li <= 16'd0; sj <= 16'd0;
        m <= 32'hff80_0000;
        for (k = 0; k < VD; k = k + 1) out[k] <= 32'd0;
        state <= KEY;
      end
      KEY: if (k_take) begin
        if (li == KL - 1) begin
          li <= 16'd0;
          if (j + 16'd1 == n) begin j <= 16'd0; state <= KDRAIN; end else j <= j + 16'd1;
        end else li <= li + 16'd1;
      end
      // Every score in; then each one's exponential, a position a clock.
      KDRAIN: if (sj == n) begin er <= 16'd0; lanes <= 512'd0; state <= EXP; end
      EXP: begin
        if (er != n) er <= er + 16'd1;
        else begin wait_c <= 16'd0; state <= EDRAIN; end
      end
      EDRAIN: begin
        wait_c <= wait_c + 16'd1;
        if (wait_c == EL + @TREE@ + @RECIP@ + 16'd2) state <= INV;
      end
      INV: begin inv <= total_inv; blk <= 16'd0; j <= 16'd0; wmax <= 32'd0; w_got <= 6'd0; state <= WSCALE; end
      // A block's scales a clock, their weights in; then its exponent; then its value rows.
      WSCALE: begin
        if (s_valid && s_ready) j <= j + 16'd1;
        if ((j == blk * 16'd32 + 16'd32 || j == n) && {10'd0, w_got} == j - blk * 16'd32) begin
          j <= blk * 16'd32; state <= WCODE;
        end
      end
      WCODE: begin
        we <= w_exp;
        for (k = 0; k < VD; k = k + 1) sums[k] <= 32'sd0;
        li <= 16'd0;
        cj <= 4'd0;
        state <= WCODES;
      end
      WCODES: begin
        if (cj != 4'd8) begin
          for (k = 0; k < 4; k = k + 1) cw[k] <= w[4*cj[2:0] + k];
          cw_v <= 1'b1; cw_at <= cj[2:0]; cj <= cj + 4'd1;
        end else begin
          cw_v <= 1'b0;
          if (!cw_v && !cut_v) state <= WVAL;
        end
      end
      WVAL: if (v_take) begin
        if (li == VL - 1) begin
          li <= 16'd0;
          if (j[4:0] == 5'd31 || j + 16'd1 == n) begin rel <= 16'd0; landed <= 16'd0; state <= WVD; end
          else j <= j + 16'd1;
        end else li <= li + 16'd1;
      end
      WVD: state <= WROUND;
      // Each element's sum through the rounding and the add, an element a clock.
      WROUND: begin
        r_v <= 1'b1; r_zero <= wmax[30:0] == 31'd0; rsum <= sums[rel];
        rel <= rel + 16'd1;
        if (rel + 16'd1 == VN) state <= WDRAIN;
      end
      WDRAIN: if (landed == VN) begin
        if (blk * 16'd32 + 16'd32 >= n) begin b <= 16'd0; state <= OUT; end
        else begin blk <= blk + 16'd1; j <= blk * 16'd32 + 16'd32; wmax <= 32'd0; w_got <= 6'd0; state <= WSCALE; end
      end
      OUT: begin
        o_valid <= 1'b1;
        o_beat <= out_beat;
        if (b + 16'd1 == OB) begin o_last <= 1'b1; state <= IDLE; end else b <= b + 16'd1;
      end
      default: state <= IDLE;
    endcase
    for (k = 0; k < VD; k = k + 1) if (vp_v && vp_li == k[21:6]) sums[k] <= sums[k] + vp[k % 64];
    if (rst) cw_v <= 1'b0;
  end
endmodule
"#;

/// `logos_attend_side`, a token-row's attention for a hybrid engine's fast side ([`super::engine::Streaming::attention`]),
/// with the user clock's unit and quantizer it drives.
pub fn attend_side_verilog() -> String {
    format!("{}\n{}\n{}", attend_int8_p_verilog(), super::streaming::act_stream_p_verilog(), ATTEND_SIDE_VERILOG)
}

const ATTEND_SIDE_VERILOG: &str = r#"
// A token-row's attention on a hybrid engine's fast side over an int8 cache: for each query head h, its cached head
// g = h / (heads / kv_heads); the head's query from the store (HD/16 beats from q_beat + h·HD/16) quantized as one block
// (logos_act_stream_p); then logos_attend_int8_p fed from the row's region through the port, the reads asked in the
// order the unit takes them — per position its key row's scale line and its key lines, then per block of 32 positions
// each value row's scale line, then each one's value lines — every line back in order, its tag naming what it is; the
// head's output beats into the store from out_beat + h·VD/16. Positions are the row's own and every one before. A cache
// row's first byte and its scale's are the region's (a line) plus the block's offsets plus (j·kv_heads + g) rows.
module logos_attend_side #(parameter HD = 256, VD = 256, P_MAX = 4096, SA = 12, parameter [6:0] OUTSTANDING = 32, SRL = 1)
  (input clk, input rst, input start, output busy,
   input [31:0] region, input [31:0] keys, input [31:0] values, input [31:0] kscales, input [31:0] vscales,
   input [31:0] kv_heads, input [15:0] heads, input [31:0] positions, input [31:0] scale,
   input [SA-1:0] q_beat, input [SA-1:0] out_beat,
   output [SA-1:0] st_ra, input [511:0] st_rd, output reg st_we, output reg [SA-1:0] st_wa, output reg [511:0] st_wd,
   output rq_valid, input rq_ready, output [31:0] rq_line, input rs_valid, input [511:0] rs_data);
  localparam [15:0] HB = HD / 16, OB = VD / 16, KL = HD / 64, VL = VD / 64;
  localparam IDLE = 3'd0, GROUP = 3'd1, QLOAD = 3'd2, QUANT = 3'd3, RUN = 3'd4;
  localparam [1:0] T_KS = 2'd0, T_K = 2'd1, T_VS = 2'd2, T_V = 2'd3;
  reg [2:0] state;
  // The job, the group size, the head and its cached head.
  reg [31:0] rg, ko, vo, kso, vso, kv, n, sc;
  reg [15:0] nh, grp, rem, h, hg;
  reg [31:0] g;
  reg [SA-1:0] qb, ob;
  assign busy = state != IDLE || st_we;
  // The query's beats into the quantizer, a beat the clock after its address.
  reg [15:0] qj;
  reg arriving, first;
  // A query beat asked (arriving, first, the clock after its address) and out of the store SRL clocks after its address
  // (landed, landed_first).
  wire landed, landed_first, landing;
  logos_landing #(.W(1), .D(SRL - 1)) qland(.clk(clk), .rst(rst), .in_v(arriving), .in_d(first), .out_v(landed),
    .out_d(landed_first), .flight(landing));
  wire aq_busy;
  wire [7:0] aq_m;
  wire signed [11:0] aq_e;
  wire [16*HD-1:0] aq_codes;
  reg aq_go;
  wire [31:0] q_at = {{(32-SA){1'b0}}, qb} + h * HB + qj;
  assign st_ra = q_at[SA-1:0];
  logos_act_stream_p #(.LANES(HD)) aq(.clk(clk), .rst(rst), .clear(landed && landed_first), .in_v(landed), .in_beat(st_rd),
    .go(aq_go), .two(1'b0), .busy(aq_busy), .m(aq_m), .e(aq_e), .codes(aq_codes));
  // The unit.
  reg u_start;
  wire u_busy, k_ready, s_ready, v_ready, o_valid, o_last;
  wire [511:0] o_beat;
  reg k_valid, s_valid, v_valid;
  reg [511:0] k_line, v_line;
  reg [15:0] k_scale, v_scale;
  logos_attend_int8_p #(.HD(HD), .VD(VD), .P_MAX(P_MAX)) u(.clk(clk), .rst(rst), .start(u_start), .positions(n[15:0]),
    .q_codes(aq_codes), .q_m(aq_m), .q_e(aq_e), .scale(sc), .busy(u_busy), .k_valid(k_valid), .k_line(k_line),
    .k_scale(k_scale), .k_ready(k_ready), .s_valid(s_valid), .v_scale(v_scale), .s_ready(s_ready), .v_valid(v_valid),
    .v_line(v_line), .v_ready(v_ready), .o_valid(o_valid), .o_beat(o_beat), .o_last(o_last));
  // The walk of the head's reads: its phase (keys, a block's scales, a block's values), position, line, and the cache row
  // of its position (wj·kv_heads + g, kept as the walk steps) and of its block's first.
  localparam [1:0] W_KEYS = 2'd0, W_SCALES = 2'd1, W_VALUES = 2'd2, W_DONE = 2'd3;
  reg [1:0] wph;
  reg [31:0] wj, wblk, w_row, b_row;
  reg [15:0] wl;
  reg walking;
  wire [47:0] base = {16'd0, rg} * 48'd64;
  wire [47:0] ks_byte = base + kso + {w_row, 1'b0};
  wire [47:0] k_byte = base + ko + w_row * HD + {wl, 6'd0};
  wire [47:0] vs_byte = base + vso + {w_row, 1'b0};
  wire [47:0] v_byte = base + vo + w_row * VD + {wl, 6'd0};
  // The keys' phase asks each position's scale line (wl == KL) then its lines; the values' each position's lines.
  wire [1:0] w_kind = wph == W_KEYS ? (wl == KL ? T_KS : T_K) : wph == W_SCALES ? T_VS : T_V;
  wire [47:0] w_byte = w_kind == T_KS ? ks_byte : w_kind == T_K ? k_byte : w_kind == T_VS ? vs_byte : v_byte;
  reg [6:0] owed;
  assign rq_valid = walking && owed < OUTSTANDING;
  assign rq_line = w_byte[37:6];
  wire rq_take = rq_valid && rq_ready;
  wire [31:0] blk_end = ({wblk[26:0], 5'd0} + 32'd32 < n) ? {wblk[26:0], 5'd0} + 32'd32 : n;
  // Every read's tag — its kind and its scale's byte — and its line, in order.
  reg [7:0] tags [0:63];
  reg [511:0] lines [0:63];
  reg [5:0] t_wr, t_rd, l_wr;
  always @(posedge clk) begin
    if (rq_take) tags[t_wr] <= {w_kind, w_byte[5:0]};
    if (rs_valid) lines[l_wr] <= rs_data;
  end
  wire have = l_wr != t_rd;
  wire [1:0] t_kind = tags[t_rd][7:6];
  wire [5:0] t_off = tags[t_rd][5:0];
  wire [511:0] line = lines[t_rd];
  wire [15:0] line_scale = line[8 * t_off +: 16];
  // The line at the head handed to the unit as it asks: a key scale held for its row's lines.
  reg [15:0] kscale_h;
  wire to_k = have && t_kind == T_K, to_s = have && t_kind == T_VS, to_v = have && t_kind == T_V;
  always @* begin
    k_valid = to_k; k_line = line; k_scale = kscale_h;
    s_valid = to_s; v_scale = line_scale;
    v_valid = to_v; v_line = line;
  end
  wire pop = have && (t_kind == T_KS || (to_k && k_ready) || (to_s && s_ready) || (to_v && v_ready));
  reg [15:0] ob_i;
  wire [31:0] o_at = {{(32-SA){1'b0}}, ob} + h * OB + ob_i;
  always @(posedge clk) begin
    u_start <= 1'b0; aq_go <= 1'b0; arriving <= 1'b0; st_we <= 1'b0;
    if (rq_take) t_wr <= t_wr + 6'd1;
    if (rs_valid) l_wr <= l_wr + 6'd1;
    if (pop) begin t_rd <= t_rd + 6'd1; if (t_kind == T_KS) kscale_h <= line_scale; end
    owed <= owed + (rq_take ? 7'd1 : 7'd0) - (pop ? 7'd1 : 7'd0);
    // The walk: keys, then each block's scales and values.
    if (rq_take) begin
      case (wph)
        W_KEYS: if (wl == KL) wl <= 16'd0;
                else if (wl + 16'd1 == KL) begin
                  wl <= KL;
                  if (wj + 32'd1 == n) begin wj <= 32'd0; w_row <= g; b_row <= g; wblk <= 32'd0; wph <= W_SCALES; end
                  else begin wj <= wj + 32'd1; w_row <= w_row + kv; end
                end else wl <= wl + 16'd1;
        W_SCALES: if (wj + 32'd1 == blk_end) begin wj <= {wblk[26:0], 5'd0}; w_row <= b_row; wl <= 16'd0; wph <= W_VALUES; end
                  else begin wj <= wj + 32'd1; w_row <= w_row + kv; end
        W_VALUES: if (wl + 16'd1 == VL) begin
                    wl <= 16'd0;
                    if (wj + 32'd1 == blk_end) begin
                      if (blk_end == n) begin walking <= 1'b0; wph <= W_DONE; end
                      else begin wblk <= wblk + 32'd1; wj <= blk_end; w_row <= w_row + kv; b_row <= w_row + kv; wph <= W_SCALES; end
                    end else begin wj <= wj + 32'd1; w_row <= w_row + kv; end
                  end else wl <= wl + 16'd1;
        default: ;
      endcase
    end
    // The head's output beats into the store.
    if (o_valid) begin st_we <= 1'b1; st_wa <= o_at[SA-1:0]; st_wd <= o_beat; ob_i <= ob_i + 16'd1; end
    if (rst) begin
      state <= IDLE; t_wr <= 6'd0; t_rd <= 6'd0; l_wr <= 6'd0; owed <= 7'd0; walking <= 1'b0;
    end else case (state)
      IDLE: if (start) begin
        rg <= region; ko <= keys; vo <= values; kso <= kscales; vso <= vscales; kv <= kv_heads; nh <= heads;
        n <= positions; sc <= scale; qb <= q_beat; ob <= out_beat; grp <= 16'd0; rem <= heads; state <= GROUP;
      end
      // heads / kv_heads by subtraction.
      GROUP: if (rem >= kv[15:0]) begin rem <= rem - kv[15:0]; grp <= grp + 16'd1; end
             else begin h <= 16'd0; hg <= 16'd0; g <= 32'd0; qj <= 16'd0; state <= QLOAD; end
      QLOAD: begin
        arriving <= 1'b1; first <= qj == 16'd0;
        if (qj + 16'd1 == HB) begin qj <= 16'd0; state <= QUANT; end else qj <= qj + 16'd1;
      end
      QUANT: if (!arriving && !landing && !aq_go && !aq_busy) begin
        if (qj == 16'd0) begin aq_go <= 1'b1; qj <= 16'd1; end
        else begin
          qj <= 16'd0; u_start <= 1'b1; ob_i <= 16'd0;
          wph <= W_KEYS; wj <= 32'd0; w_row <= g; wl <= KL; walking <= 1'b1; state <= RUN;
        end
      end
      RUN: if (o_valid && o_last) begin
        if (h + 16'd1 == nh) state <= IDLE;
        else begin
          h <= h + 16'd1;
          if (hg + 16'd1 == grp) begin hg <= 16'd0; g <= g + 32'd1; end else hg <= hg + 16'd1;
          qj <= 16'd0; state <= QLOAD;
        end
      end
      default: state <= IDLE;
    endcase
  end
endmodule
"#;
