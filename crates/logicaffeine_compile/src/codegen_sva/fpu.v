// fpu.v — a synthesizable IEEE-754 double-precision (binary64 / f64) floating-point unit,
// proven BIT-EXACT to Rust `f64` in iverilog (tb_fpu_*.v, generated FROM Rust f64 vectors).
//
// The four float benchmarks (nbody, mandelbrot, spectral_norm, pi_leibniz) need f64: a constant
// like 9.54791938424326609 has 17 significant figures and only survives in binary64. Every op here
// is round-to-nearest-ties-to-even with correct ±0 / ±Inf / quiet-NaN / subnormal handling, matching
// Rust's `f64` operator semantics exactly (Rust uses the platform's IEEE-754 hardware, which is what
// we reproduce here). NO `real` / `$bitstoreal` appears in this module — those are testbench-only;
// this DUT is pure synthesizable logic. div/sqrt are multi-cycle digit-recurrence; add/sub/mul/convert
// are ALSO multi-cycle (an FSM registers each stage) — a start/done handshake, correctly rounded. No
// stage holds more than one carry chain, barrel shift, leading-one search or multiplier: carries are
// parallel-prefix, leading-one searches halve their word level by level, stickies test the shifted-out
// bits with no (1 << s) - 1 mask. Every register boundary is between IDENTICAL arithmetic expressions,
// so the RESULT stays bit-for-bit what the combinational form produced.
//
// Interface — a single fpu module selects the op by a 3-bit code:
//   OP_ADD=0 OP_SUB=1 OP_MUL=2 OP_DIV=3 OP_SQRT=4 OP_LT=5 OP_EQ=6
// Arithmetic ops (add/sub/mul/div/sqrt) run a bounded FSM: `start` pulses to issue, `done` rises when
// `y` is valid. Comparisons (lt/eq) stay combinational (no shifters). For comparisons the numeric
// result low bit is the boolean; `result[0]` and `cmp` both carry it.

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// fp_addsub — multi-cycle IEEE-754 binary64 add/sub, round-to-nearest-ties-even. start/done handshake.
//   sub=0 -> a+b ; sub=1 -> a-b  (a - b == a + (-b), so we just flip b's sign)
// The datapath is the SAME as the classic single-cycle form, split across the FSM below so each wide
// shifter, the 128-bit add, the leading-one search and the rounding increment is its own small cone instead
// of one 128-bit-wide nest. Bit-exact to the combinational reference.
// ─────────────────────────────────────────────────────────────────────────────────────────────────
module fp_addsub(
  input         clk,
  input         rst,
  input         start,
  input  [63:0] a,
  input  [63:0] b_in,
  input         sub,      // 0: add, 1: subtract
  output reg [63:0] y,
  output reg    done
);
  localparam [63:0] QNAN = 64'h7FF8000000000000;

  // One cone per state, none holding more than one carry chain, barrel shift or leading-one search:
  // DECODE (specials, the magnitude compare), ALIGN (the smaller operand's shift and its sticky), ADD (the 128-bit
  // add or subtract), LZC (the leading one), NORM (the normalizing shift and its sticky), ROUND (round to nearest
  // even and pack a normal result), and for a subnormal result SUBN (canonicalize), SUBR (denormalize) and SUBP
  // (round and pack).
  reg  [3:0] state;
  localparam IDLE = 4'd0, DECODE = 4'd1, ALIGN = 4'd2, ADD = 4'd3, LZC = 4'd4, NORM = 4'd5, ROUND = 4'd6,
             SUBN = 4'd7, SUBR = 4'd8, SUBP = 4'd9;

  // latched inputs (captured at issue; the shared operand ports fav/fbv stay stable while we stall)
  reg  [63:0] a_l, b_l;

  // flip the sign of the second operand for subtraction
  wire [63:0] b = sub ? {~b_l[63], b_l[62:0]} : b_l;

  wire        sa = a_l[63], sb = b[63];
  wire [10:0] ea = a_l[62:52], eb = b[62:52];
  wire [51:0] ma = a_l[51:0],  mb = b[51:0];

  wire a_is_zero  = (ea == 11'd0)    && (ma == 52'd0);
  wire b_is_zero  = (eb == 11'd0)    && (mb == 52'd0);
  wire a_is_inf   = (ea == 11'h7FF)  && (ma == 52'd0);
  wire b_is_inf   = (eb == 11'h7FF)  && (mb == 52'd0);
  wire a_is_nan   = (ea == 11'h7FF)  && (ma != 52'd0);
  wire b_is_nan   = (eb == 11'h7FF)  && (mb != 52'd0);
  wire a_is_sub   = (ea == 11'd0)    && (ma != 52'd0);  // subnormal (nonzero, exp field 0)
  wire b_is_sub   = (eb == 11'd0)    && (mb != 52'd0);

  // effective (unbiased-friendly) exponent: subnormals use exp=1, hidden bit 0; normals hidden bit 1
  wire [11:0] xa = a_is_sub ? 12'd1 : {1'b0, ea};
  wire [11:0] xb = b_is_sub ? 12'd1 : {1'b0, eb};
  wire        ha = (ea != 11'd0);
  wire        hb = (eb != 11'd0);
  wire [52:0] siga = {ha, ma};
  wire [52:0] sigb = {hb, mb};

  reg  [11:0] exp_l;        // exponent of the larger-magnitude operand
  reg  [52:0] sig_big, sig_small;
  reg         sgn_big, sgn_small;
  reg  [11:0] shamt;
  reg  [127:0] small_ext;   // the smaller operand aligned, its sticky folded into bit 0
  reg  [127:0] sum_ext;     // the exact aligned sum
  reg          res_sign;
  reg          op_is_sub;
  reg  [11:0] norm_exp;
  reg  [11:0] lead;         // position of the leading 1
  reg  [127:0] norm_sig_r;  // leading 1 at bit 55: [55:3] the significand, [2:0] guard/round/sticky
  reg  [11:0] rexp_r;       // the rounded exponent, for a subnormal result

  // A word's bits below `s`, as one flag: the bits shifted to the top and tested, so no (1 << s) - 1 mask and no
  // borrow chain. Zero when s is zero.
  function below(input [127:0] x, input [11:0] s);
    begin
      below = (s == 12'd0) ? 1'b0 : ((x << (12'd128 - s)) != 128'd0);
    end
  endfunction

  // The highest set bit of a nonzero 128-bit word, by halving: seven levels, not a 128-long priority chain.
  function [11:0] lead_one(input [127:0] x);
    reg [63:0] h6; reg [31:0] h5; reg [15:0] h4; reg [7:0] h3; reg [3:0] h2; reg [1:0] h1; reg [6:0] z;
    begin
      z[6] = ~|x[127:64]; h6 = z[6] ? x[63:0]   : x[127:64];
      z[5] = ~|h6[63:32]; h5 = z[5] ? h6[31:0]  : h6[63:32];
      z[4] = ~|h5[31:16]; h4 = z[4] ? h5[15:0]  : h5[31:16];
      z[3] = ~|h4[15:8];  h3 = z[3] ? h4[7:0]   : h4[15:8];
      z[2] = ~|h3[7:4];   h2 = z[2] ? h3[3:0]   : h3[7:4];
      z[1] = ~|h2[3:2];   h1 = z[1] ? h2[1:0]   : h2[3:2];
      z[0] = ~h1[1];
      lead_one = {5'd0, ~z};
    end
  endfunction

  // a ± b over 128 bits as a Kogge-Stone prefix add, the subtract's +1 entering as bit 0's carry.
  function [127:0] addsub128(input [127:0] x, input [127:0] yy, input s);
    reg [127:0] yb, g, p, p0; integer d;
    begin
      yb = s ? ~yy : yy;
      p0 = x ^ yb;
      g = (x & yb) | (p0 & {127'd0, s});
      p = p0;
      for (d = 1; d < 128; d = d * 2) begin
        g = g | (p & (g << d));
        p = p & (p << d);
      end
      addsub128 = p0 ^ {g[126:0], s};
    end
  endfunction

  // v + cin through a parallel-prefix AND: log2(64) levels. cin is the scan's bit 0, so bit i flips on cin and
  // every bit below it, and cin reaches the bits through the scan's tree rather than one net to all 64.
  function [63:0] inc64(input [63:0] v, input cin);
    reg [63:0] p; integer d;
    begin
      p = {v[62:0], cin};
      for (d = 1; d < 64; d = d * 2) p = p & ((p << d) | ((64'd1 << d) - 64'd1));
      inc64 = v ^ p;
    end
  endfunction

  integer i;
  reg [127:0] big_ext, small_w, norm_sig, win;
  reg [63:0] rounded, rr;
  reg [52:0] final_sig;
  reg [11:0] rexp, final_exp, shr, rshift;
  reg guard, roundb, sticky, lsb, round_up, g2, r2, s2, l2, up2;

  initial begin state = IDLE; done = 1'b0; y = 64'd0; end

  always @(posedge clk) begin
    if (rst) begin state <= IDLE; done <= 1'b0; y <= 64'd0; end
    else case (state)
      IDLE: if (start) begin
        done <= 1'b0; a_l <= a; b_l <= b_in; state <= DECODE;
      end
      DECODE: begin
        if (a_is_nan || b_is_nan) begin
          y <= QNAN; done <= 1'b1; state <= IDLE;
        end
        else if (a_is_inf && b_is_inf) begin
          if (sa == sb) y <= {sa, 11'h7FF, 52'd0}; else y <= QNAN;
          done <= 1'b1; state <= IDLE;
        end
        else if (a_is_inf) begin y <= {sa, 11'h7FF, 52'd0}; done <= 1'b1; state <= IDLE; end
        else if (b_is_inf) begin y <= {sb, 11'h7FF, 52'd0}; done <= 1'b1; state <= IDLE; end
        else if (a_is_zero && b_is_zero) begin
          if (sa == sb) y <= {sa, 63'd0}; else y <= 64'd0;
          done <= 1'b1; state <= IDLE;
        end
        else if (a_is_zero) begin y <= b; done <= 1'b1; state <= IDLE; end
        else if (b_is_zero) begin y <= a_l; done <= 1'b1; state <= IDLE; end
        else begin
          if (xa > xb || (xa == xb && siga >= sigb)) begin
            exp_l     <= xa;
            sig_big   <= siga;   sgn_big   <= sa;
            sig_small <= sigb;   sgn_small <= sb;
            shamt     <= xa - xb;
          end else begin
            exp_l     <= xb;
            sig_big   <= sigb;   sgn_big   <= sb;
            sig_small <= siga;   sgn_small <= sa;
            shamt     <= xb - xa;
          end
          state <= ALIGN;
        end
      end
      ALIGN: begin
        // The small operand, its LSB at bit 64, shifted right by shamt in a 128-bit lane; with 64 bits of room a
        // shift up to 64 keeps every bit, and what falls off folds into a sticky bit 0.
        small_w = {11'd0, sig_small, 64'd0};
        if (shamt >= 12'd128) begin
          small_ext <= {127'd0, |sig_small};
        end else begin
          small_ext <= (small_w >> shamt) | {127'd0, below(small_w, shamt)};
        end
        op_is_sub <= (sgn_big != sgn_small);
        res_sign  <= sgn_big;
        state <= ADD;
      end
      ADD: begin
        big_ext = {11'd0, sig_big, 64'd0};
        sum_ext <= addsub128(big_ext, small_ext, op_is_sub);
        state <= LZC;
      end
      LZC: begin
        if (sum_ext == 128'd0) begin
          y <= 64'd0; done <= 1'b1; state <= IDLE;      // exact cancellation -> +0 (RNE)
        end else begin
          lead  <= lead_one(sum_ext);
          state <= NORM;
        end
      end
      NORM: begin
        norm_exp <= exp_l + lead - 12'd116;              // biased exponent of the leading 1
        if (lead >= 12'd55)
          norm_sig_r <= (sum_ext >> (lead - 12'd55)) | {127'd0, below(sum_ext, lead - 12'd55)};
        else
          norm_sig_r <= sum_ext << (12'd55 - lead);
        state <= ROUND;
      end
      ROUND: begin
        norm_sig = norm_sig_r;
        guard  = norm_sig[2];
        roundb = norm_sig[1];
        sticky = norm_sig[0];
        lsb    = norm_sig[3];
        round_up = guard & (roundb | sticky | lsb);
        rounded = inc64({11'd0, norm_sig[55:3]}, round_up);
        rexp = norm_exp;
        if (rounded[53]) begin
          final_sig = rounded[53:1];
          rexp      = rexp + 12'd1;
        end else begin
          final_sig = rounded[52:0];
        end
        if ($signed(rexp) >= 12'sd2047) begin
          y <= {res_sign, 11'h7FF, 52'd0};              // overflow -> inf
          done <= 1'b1; state <= IDLE;
        end else if ($signed(rexp) <= 12'sd0) begin
          rexp_r <= rexp;
          state <= SUBN;                                // a subnormal result rounds from the exact sum
        end else begin
          final_exp = rexp;
          y <= {res_sign, final_exp[10:0], final_sig[51:0]};
          done <= 1'b1; state <= IDLE;
        end
      end
      SUBN: begin
        // The exact sum canonicalized so its leading 1 sits at bit 55, bits below the window folded into sticky.
        // The canonical window goes to norm_sig_r, free once ROUND has read it.
        if (lead >= 12'd55) begin
          shr = lead - 12'd55;
          norm_sig_r <= (sum_ext >> shr) | {127'd0, below(sum_ext, shr)};
        end else begin
          norm_sig_r <= sum_ext << (12'd55 - lead);
        end
        state <= SUBR;
      end
      SUBR: begin
        // Shift the window right by 1 - exponent, folding the discarded bits into sticky.
        // The denormalized window goes to sum_ext, free once SUBN has read it.
        rshift = 12'd1 - rexp_r;
        if (rshift >= 12'd128)
          sum_ext <= {127'd0, |norm_sig_r};
        else if (rshift != 12'd0)
          sum_ext <= (norm_sig_r >> rshift) | {127'd0, below(norm_sig_r, rshift)};
        else
          sum_ext <= norm_sig_r;
        state <= SUBP;
      end
      SUBP: begin
        win = sum_ext;
        g2 = win[2]; r2 = win[1]; s2 = win[0]; l2 = win[3];
        up2 = g2 & (r2 | s2 | l2);
        rr = inc64({11'd0, win[55:3]}, up2);
        if (rr[52]) y <= {res_sign, 11'd1, rr[51:0]};
        else        y <= {res_sign, 11'd0, rr[51:0]};
        done <= 1'b1; state <= IDLE;
      end
      default: state <= IDLE;
    endcase
  end
endmodule

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// fp_mul — multi-cycle IEEE-754 binary64 multiply, round-to-nearest-ties-even. start/done handshake.
// Same datapath as the single-cycle form, split across the FSM (DECODE: specials + leading-zero counts;
// PRENORM: the pre-normalizing shifts; MUL and MUL2: the 53×53 product's compressor tree (fp_csa_mul), its first
// four levels and then the rest; PRODUCT: its final add + top-bit normalize; ROUND: normal RNE round + pack, or the
// subnormal grid shift; SUBP: the subnormal round + pack). Bit-exact to the combinational reference.
// ─────────────────────────────────────────────────────────────────────────────────────────────────
module fp_mul(
  input         clk,
  input         rst,
  input         start,
  input  [63:0] a,
  input  [63:0] b,
  output reg [63:0] y,
  output reg    done
);
  localparam [63:0] QNAN = 64'h7FF8000000000000;
  parameter integer MUL_TREE = 0;   // 1: the significand product as a compressor tree (an ASIC build)

  reg  [3:0] state;
  localparam IDLE = 4'd0, DECODE = 4'd1, MUL = 4'd2, ROUND = 4'd3, PRENORM = 4'd4, PRODUCT = 4'd5, SUBP = 4'd6,
             MUL2 = 4'd7;

  // latched inputs (captured at issue; the shared operand ports stay stable while we stall)
  reg  [63:0] a_l, b_l;

  wire        sa = a_l[63], sb = b_l[63];
  wire [10:0] ea = a_l[62:52], eb = b_l[62:52];
  wire [51:0] ma = a_l[51:0],  mb = b_l[51:0];
  wire        rs = sa ^ sb;                 // result sign

  wire a_is_zero = (ea == 11'd0)   && (ma == 52'd0);
  wire b_is_zero = (eb == 11'd0)   && (mb == 52'd0);
  wire a_is_inf  = (ea == 11'h7FF) && (ma == 52'd0);
  wire b_is_inf  = (eb == 11'h7FF) && (mb == 52'd0);
  wire a_is_nan  = (ea == 11'h7FF) && (ma != 52'd0);
  wire b_is_nan  = (eb == 11'h7FF) && (mb != 52'd0);

  // significands with implicit bit
  wire        ha = (ea != 11'd0);
  wire        hb = (eb != 11'd0);
  wire [52:0] siga = {ha, ma};
  wire [52:0] sigb = {hb, mb};

  reg  [105:0] prod;               // 53 x 53 = up to 106 bits
  reg  signed [13:0] exp_sum;      // combined unbiased exponent (wide, signed)
  reg  [13:0] lza, lzb;            // leading-zero adjustments for subnormals
  reg  [52:0] sa_sh, sb_sh;        // pre-normalized significands (leading 1 at bit 52), registered
  reg  [105:0] norm;
  reg  signed [13:0] e;
  reg  [55:0] mant;                // 53-bit + GRS
  reg  g, r, s, l, up;
  reg  [63:0] rnd;
  reg  [52:0] fsig;
  reg  signed [13:0] fe;
  reg  [13:0] rsh;
  reg  [127:0] wide;
  reg  g2,r2,s2,l2,up2;
  reg  [63:0] rr;
  reg  signed [13:0] bexp;
  reg          rs_l;               // result sign, registered for the ROUND stage
  reg  [13:0]  lza_r, lzb_r;       // leading-zero counts, for PRENORM
  reg  [63:0]  wide_r;             // a subnormal product on its grid (at most 56 bits), for SUBP

  // The 53×53 significand product, two clocks deep: sa_sh and sb_sh, written in PRENORM and held through PRODUCT,
  // enter it at MUL and its product is read in PRODUCT. MUL_TREE (an ASIC build) makes it a compressor tree, its
  // first four levels registered at the end of MUL and the rest at the end of MUL2 before a Kogge-Stone add; otherwise
  // (an FPGA build) it is the `*` a DSP block takes, registered twice.
  wire [105:0] tree_p;
  generate
    if (MUL_TREE) begin : tree
      fp_csa_mul #(.AW(53), .BW(53), .CUT(4), .FINAL(1)) product_tree(.clk(clk), .a(sa_sh), .b(sb_sh), .p(tree_p));
    end else begin : dsp
      // One register: the operands hold through MUL2, so the product is stable when PRODUCT reads it.
      reg [105:0] p1;
      always @(posedge clk) p1 <= sa_sh * sb_sh;
      assign tree_p = p1;
    end
  endgenerate

  // The highest set bit of a nonzero word, by halving: six levels, not a priority chain.
  function [5:0] lead_one64(input [63:0] m);
    reg [31:0] h5; reg [15:0] h4; reg [7:0] h3; reg [3:0] h2; reg [1:0] h1; reg [5:0] z;
    begin
      z[5] = ~|m[63:32];  h5 = z[5] ? m[31:0]  : m[63:32];
      z[4] = ~|h5[31:16]; h4 = z[4] ? h5[15:0] : h5[31:16];
      z[3] = ~|h4[15:8];  h3 = z[3] ? h4[7:0]  : h4[15:8];
      z[2] = ~|h3[7:4];   h2 = z[2] ? h3[3:0]  : h3[7:4];
      z[1] = ~|h2[3:2];   h1 = z[1] ? h2[1:0]  : h2[3:2];
      z[0] = ~h1[1];
      lead_one64 = ~z;
    end
  endfunction

  // Leading zeros of a 53-bit significand, 53 when it is zero.
  function [13:0] clz53(input [52:0] v);
    begin
      clz53 = (v == 53'd0) ? 14'd53 : 14'd52 - {8'd0, lead_one64({11'd0, v})};
    end
  endfunction

  // v + cin through a parallel-prefix AND: log2(64) levels. cin is the scan's bit 0, so bit i flips on cin and
  // every bit below it, and cin reaches the bits through the scan's tree rather than one net to all 64.
  function [63:0] inc64(input [63:0] v, input cin);
    reg [63:0] p; integer d;
    begin
      p = {v[62:0], cin};
      for (d = 1; d < 64; d = d * 2) p = p & ((p << d) | ((64'd1 << d) - 64'd1));
      inc64 = v ^ p;
    end
  endfunction

  // A word's bits below `s` as one flag: shifted to the top and tested, no (1 << s) - 1 mask, no borrow chain.
  function below(input [127:0] x, input [13:0] s);
    begin
      below = (s == 14'd0) ? 1'b0 : ((x << (14'd128 - s)) != 128'd0);
    end
  endfunction


  initial begin state = IDLE; done = 1'b0; y = 64'd0; end

  always @(posedge clk) begin
    if (rst) begin state <= IDLE; done <= 1'b0; y <= 64'd0; end
    else case (state)
      IDLE: if (start) begin
        done <= 1'b0; a_l <= a; b_l <= b; state <= DECODE;
      end
      DECODE: begin
        rs_l <= rs;
        if (a_is_nan || b_is_nan) begin y <= QNAN; done <= 1'b1; state <= IDLE; end
        else if ((a_is_inf && b_is_zero) || (b_is_inf && a_is_zero)) begin y <= QNAN; done <= 1'b1; state <= IDLE; end
        else if (a_is_inf || b_is_inf) begin y <= {rs, 11'h7FF, 52'd0}; done <= 1'b1; state <= IDLE; end
        else if (a_is_zero || b_is_zero) begin y <= {rs, 63'd0}; done <= 1'b1; state <= IDLE; end
        else begin
          // unbiased exponents; leading-zero counts (this stage isolates the two clz53 cones + pre-shift).
          lza_r <= clz53(siga);
          lzb_r <= clz53(sigb);
          state <= PRENORM;
        end
      end
      PRENORM: begin
        lza = lza_r;
        lzb = lzb_r;
        exp_sum <= (ea != 0 ? $signed({3'd0, ea}) - 14'sd1023 : -14'sd1022)
                 + (eb != 0 ? $signed({3'd0, eb}) - 14'sd1023 : -14'sd1022)
                 - $signed(lza) - $signed(lzb);
        sa_sh <= (siga << lza);   // leading 1 at bit 52
        sb_sh <= (sigb << lzb);
        state <= MUL;
      end
      MUL: begin
        // ── the 53×53 product's compressor tree, its first four levels (registered inside product_tree) ──
        state <= MUL2;
      end
      MUL2: begin
        // ── its remaining levels (registered inside product_tree) ──
        state <= PRODUCT;
      end
      PRODUCT: begin
        // ── the tree's last two words added + top-bit normalize ──
        prod = tree_p;
        if (prod[105]) begin
          norm <= prod;
          e    <= exp_sum + 14'sd1; // leading at 105 means value in [2,4) relative -> exp+1
        end else begin
          norm <= prod << 1;        // shift so leading 1 sits at bit 105
          e    <= exp_sum;
        end
        state <= ROUND;
      end
      ROUND: begin
        // Now norm[105] == 1 and its (would-be normal) biased exponent is `e`. Decide underflow BEFORE
        // rounding — rounding the exact product exactly once avoids the double-rounding bug.
        if (e < -14'sd1022) begin
          // ── subnormal / underflow: the EXACT product shifted onto the subnormal grid, SUBP rounds it once ──
          rsh = (-14'sd1022) - e;                    // extra right-shift into the subnormal grid (>= 1)
          wide = (norm >> (14'd105 - 14'd55)) | {127'd0, below(norm, 14'd105 - 14'd55)};  // norm[105]->bit55
          if (rsh >= 14'd128)       wide_r <= {63'd0, |wide};
          else if (rsh != 14'd0)    wide_r <= (wide >> rsh) | {127'd0, below(wide, rsh)};
          else                      wide_r <= wide;
          state <= SUBP;
        end else begin
          // ── normal path: round the 53-bit significand ──
          mant = {norm[105:53], (norm[52]), (norm[51]), (|norm[50:0])};  // 53 + g + r + s = 56 bits
          g = mant[2]; r = mant[1]; s = mant[0]; l = mant[3];
          up = g & (r | s | l);
          rnd = inc64({11'd0, mant[55:3]}, up);
          if (rnd[53]) begin fsig = rnd[53:1]; fe = e + 14'sd1; end
          else         begin fsig = rnd[52:0]; fe = e; end
          if (fe > 14'sd1023) begin
            y <= {rs_l, 11'h7FF, 52'd0};                          // overflow -> inf
          end else if (fe < -14'sd1022) begin
            y <= {rs_l, 11'd1, fsig[51:0]};
          end else begin
            bexp = fe + 14'sd1023;
            y <= {rs_l, bexp[10:0], fsig[51:0]};
          end
          done <= 1'b1; state <= IDLE;
        end
      end
      SUBP: begin
        g2 = wide_r[2]; r2 = wide_r[1]; s2 = wide_r[0]; l2 = wide_r[3];
        up2 = g2 & (r2 | s2 | l2);
        rr = inc64({11'd0, wide_r[55:3]}, up2);
        if (rr[52]) y <= {rs_l, 11'd1, rr[51:0]};   // rounded up to the smallest normal
        else        y <= {rs_l, 11'd0, rr[51:0]};
        done <= 1'b1; state <= IDLE;
      end
    endcase
  end
endmodule

// fp_csa_mul — an unsigned AW×BW multiplier as a tree of 3:2 compressors over its BW partial-product rows. The
// compressor outputs register after CUT levels (CUT = 0 or beyond the tree's depth: no register inside it), the last
// two words register again when FINAL is set, and those two words resolve through a Kogge-Stone add. `p` is the
// product of the operands sampled as many clocks earlier as there are registers on its path.
module fp_csa_mul #(parameter integer AW = 53, parameter integer BW = 53, parameter integer CUT = 4,
                 parameter integer FINAL = 1) (
  input                clk,
  input  [AW-1:0]      a,
  input  [BW-1:0]      b,
  output [AW+BW-1:0]   p
);
  localparam integer W = AW + BW;

  // The words entering tree level `level`: each level turns every three words into two and passes the rest.
  function integer words(input integer level);
    integer n, l;
    begin
      n = BW;
      for (l = 0; l < level; l = l + 1) n = 2 * (n / 3) + (n % 3);
      words = n;
    end
  endfunction

  function integer depth(input integer unused);
    integer n, l;
    begin
      n = BW; l = 0;
      while (n > 2) begin n = 2 * (n / 3) + (n % 3); l = l + 1; end
      depth = l;
    end
  endfunction

  localparam integer L = depth(0);

  // Level-major bus: level `l` holds its words at word slots l*BW .. l*BW + words(l) - 1.
  wire [W*BW*(L+1)-1:0] lv;

  genvar i, l, g;
  generate
    for (i = 0; i < BW; i = i + 1) begin : row
      assign lv[i*W +: W] = b[i] ? ({{BW{1'b0}}, a} << i) : {W{1'b0}};
    end
    for (l = 0; l < L; l = l + 1) begin : level
      localparam integer N = words(l);
      localparam integer G = N / 3;
      for (g = 0; g < G; g = g + 1) begin : compress
        wire [W-1:0] x = lv[((l*BW) + 3*g) * W +: W];
        wire [W-1:0] y = lv[((l*BW) + 3*g + 1) * W +: W];
        wire [W-1:0] z = lv[((l*BW) + 3*g + 2) * W +: W];
        wire [W-1:0] xy = x ^ y;
        wire [W-1:0] s = xy ^ z;
        wire [W-1:0] c = ((x & y) | (z & xy)) << 1;
        if (l + 1 == CUT) begin : registered
          reg [W-1:0] sr, cr;
          always @(posedge clk) begin sr <= s; cr <= c; end
          assign lv[(((l+1)*BW) + 2*g) * W +: W] = sr;
          assign lv[(((l+1)*BW) + 2*g + 1) * W +: W] = cr;
        end else begin : wired
          assign lv[(((l+1)*BW) + 2*g) * W +: W] = s;
          assign lv[(((l+1)*BW) + 2*g + 1) * W +: W] = c;
        end
      end
      for (g = 0; g < N % 3; g = g + 1) begin : carry_over
        if (l + 1 == CUT) begin : registered
          reg [W-1:0] r;
          always @(posedge clk) r <= lv[((l*BW) + 3*G + g) * W +: W];
          assign lv[(((l+1)*BW) + 2*G + g) * W +: W] = r;
        end else begin : wired
          assign lv[(((l+1)*BW) + 2*G + g) * W +: W] = lv[((l*BW) + 3*G + g) * W +: W];
        end
      end
    end
  endgenerate

  wire [W-1:0] s_word = lv[(L*BW) * W +: W];
  wire [W-1:0] c_word;
  generate
    if (BW == 1) begin : one_row
      assign c_word = {W{1'b0}};
    end else begin : two_rows
      assign c_word = lv[(L*BW + 1) * W +: W];
    end
  endgenerate
  reg  [W-1:0] s_r, c_r;
  always @(posedge clk) begin s_r <= s_word; c_r <= c_word; end
  wire [W-1:0] s_in = FINAL ? s_r : s_word;
  wire [W-1:0] c_in = FINAL ? c_r : c_word;

  // The Kogge-Stone add of the last two words: group generate and propagate doubled in span each level.
  function [W-1:0] prefix_add(input [W-1:0] u, input [W-1:0] v);
    reg [W-1:0] gg, pp, p0; integer d;
    begin
      gg = u & v; p0 = u ^ v; pp = p0;
      for (d = 1; d < W; d = d * 2) begin
        gg = gg | (pp & (gg << d));
        pp = pp & (pp << d);
      end
      prefix_add = p0 ^ (gg << 1);
    end
  endfunction

  assign p = prefix_add(s_in, c_in);
endmodule

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// fp_cmp — combinational IEEE-754 binary64 less-than / equal (returns 1 bit each).
//   lt: a < b (false if either is NaN, -0 == +0)
//   eq: a == b (false if either is NaN, -0 == +0)
// ─────────────────────────────────────────────────────────────────────────────────────────────────
module fp_cmp(
  input  [63:0] a,
  input  [63:0] b,
  output        lt,
  output        eq
);
  wire [10:0] ea = a[62:52], eb = b[62:52];
  wire [51:0] ma = a[51:0],  mb = b[51:0];
  wire a_is_nan = (ea == 11'h7FF) && (ma != 52'd0);
  wire b_is_nan = (eb == 11'h7FF) && (mb != 52'd0);
  wire any_nan  = a_is_nan || b_is_nan;

  wire a_is_zero = (a[62:0] == 63'd0);   // ±0 : exp and frac all zero
  wire b_is_zero = (b[62:0] == 63'd0);
  wire both_zero = a_is_zero && b_is_zero;   // -0 == +0

  wire sa = a[63], sb = b[63];

  // Map to a monotonic ordering key (the classic "flip" trick): for a two's-complement view of the
  // bit pattern, positives keep order; negatives are reversed. Convert to an unsigned key where the
  // numeric order matches: if sign bit set -> ~bits ; else -> bits | 0x8000...0000.
  wire [63:0] ka = a[63] ? ~a : (a | 64'h8000000000000000);
  wire [63:0] kb = b[63] ? ~b : (b | 64'h8000000000000000);

  assign eq = any_nan ? 1'b0 : (both_zero ? 1'b1 : (a == b));
  assign lt = any_nan ? 1'b0 : (both_zero ? 1'b0 : (ka < kb));
endmodule

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// fp_div — multi-cycle IEEE-754 binary64 divide, digit-recurrence (restoring), correctly rounded.
//   start/done handshake. Computes a/b with a 55-bit quotient (53 + guard/round) plus a sticky
//   from the final nonzero remainder, then RNE-rounds; each recurrence step's subtract is a parallel-prefix
//   subtract, and a subnormal quotient takes its grid shift and its rounding in stages of their own.
// ─────────────────────────────────────────────────────────────────────────────────────────────────
module fp_div(
  input         clk,
  input         rst,
  input         start,
  input  [63:0] a,
  input  [63:0] b,
  output reg [63:0] y,
  output reg    done
);
  localparam [63:0] QNAN = 64'h7FF8000000000000;

  reg  [3:0] state;
  // The exponent datapath is PIPELINED (SETUP→SETUP2→SETUP3 and FIN→FIN2) so no single cycle carries a
  // long combinational exponent tail. On silicon that tail was the last-settling node of a deep cone (clz +
  // barrel-shift + wide signed add in SETUP; 128-bit shifters + round + biased-exponent add in FIN), and it
  // dropped a carry — the mantissa (settling earlier in the same cone) stayed exact while the exponent came
  // out 2^32 small. Registering between the clz/shift/add sub-steps keeps every cone short, matching the
  // already-silicon-correct fp_mul, at the cost of a few extra cycles (the unit already runs ~70).
  localparam IDLE = 4'd0, SETUP = 4'd1, SETUP2 = 4'd2, SETUP3 = 4'd3, RUN = 4'd4, FIN = 4'd5, FIN2 = 4'd6,
             FINS = 4'd7, FINP = 4'd8;

  // latched operand decode
  reg  sa, sb, rs;
  reg  [10:0] ea, eb;
  reg  [51:0] ma, mb;
  reg  special;                 // special-case result already computed
  reg  [63:0] special_y;
  reg  signed [13:0] exp_res;   // unbiased result exponent (of the leading quotient bit)

  // digit-recurrence datapath: divide two normalized 53-bit significands.
  reg  [55:0] num;              // remainder / working numerator, gets shifted left each step
  reg  [52:0] den;              // divisor significand (leading 1 at bit 52)
  reg  [55:0] quo;              // quotient bits accumulate here (56 = 53 + 3 GRS)
  reg  [6:0]  cnt;              // step counter
  reg  [72:0] diff;
  reg  [105:0] shl_a, shl_b;    // normalized significands (leading 1 at bit 52) — temporaries
  reg  signed [13:0] bexp;      // biased exponent temporary

  reg  signed [13:0] lza, lzb;

  // The highest set bit of a nonzero word, by halving: six levels, not a priority chain.
  function [5:0] lead_one64(input [63:0] m);
    reg [31:0] h5; reg [15:0] h4; reg [7:0] h3; reg [3:0] h2; reg [1:0] h1; reg [5:0] z;
    begin
      z[5] = ~|m[63:32];  h5 = z[5] ? m[31:0]  : m[63:32];
      z[4] = ~|h5[31:16]; h4 = z[4] ? h5[15:0] : h5[31:16];
      z[3] = ~|h4[15:8];  h3 = z[3] ? h4[7:0]  : h4[15:8];
      z[2] = ~|h3[7:4];   h2 = z[2] ? h3[3:0]  : h3[7:4];
      z[1] = ~|h2[3:2];   h1 = z[1] ? h2[1:0]  : h2[3:2];
      z[0] = ~h1[1];
      lead_one64 = ~z;
    end
  endfunction

  // Leading zeros of a 53-bit significand, 53 when it is zero.
  function [13:0] clz53(input [52:0] v);
    begin
      clz53 = (v == 53'd0) ? 14'd53 : 14'd52 - {8'd0, lead_one64({11'd0, v})};
    end
  endfunction

  // v + cin through a parallel-prefix AND: log2(64) levels. cin is the scan's bit 0, so bit i flips on cin and
  // every bit below it, and cin reaches the bits through the scan's tree rather than one net to all 64.
  function [63:0] inc64(input [63:0] v, input cin);
    reg [63:0] p; integer d;
    begin
      p = {v[62:0], cin};
      for (d = 1; d < 64; d = d * 2) p = p & ((p << d) | ((64'd1 << d) - 64'd1));
      inc64 = v ^ p;
    end
  endfunction

  // A word's bits below `s` as one flag: shifted to the top and tested, no (1 << s) - 1 mask, no borrow chain.
  function below(input [127:0] x, input [13:0] s);
    begin
      below = (s == 14'd0) ? 1'b0 : ((x << (14'd128 - s)) != 128'd0);
    end
  endfunction

  // x - y over 72 bits as a Kogge-Stone prefix subtract; bit 72 of the result is the borrow (1 when x < y).
  function [72:0] sub72(input [71:0] x, input [71:0] yy);
    reg [71:0] yb, g, p, p0; integer d;
    begin
      yb = ~yy;
      p0 = x ^ yb;
      g = (x & yb) | (p0 & 72'd1);
      p = p0;
      for (d = 1; d < 72; d = d * 2) begin
        g = g | (p & (g << d));
        p = p & (p << d);
      end
      sub72 = {~g[71], p0 ^ {g[70:0], 1'b1}};
    end
  endfunction

  // rounding / packing registers
  reg  g,r,s,l,up;
  reg  [63:0] rnd;
  reg  [52:0] fsig;
  reg  signed [13:0] fe;
  reg  [13:0] rsh;
  reg  [127:0] wide;
  reg  g2,r2,s2,l2,up2;
  reg  [63:0] rr;

  // FIN pipeline: the final biased-exponent add (fe_r + 1023) lives alone in FIN2, off a register, so the
  // exponent output is never the tail of the deep FIN cone. normal_path picks the FIN2 assembly; the
  // subnormal / overflow / special results (constant exponent fields, no add) are pre-packed into y_reg.
  reg  [63:0] y_reg;
  reg          normal_path;
  reg  signed [13:0] fe_r;

  initial begin state = IDLE; done = 1'b0; y = 64'd0; end

  wire a_is_zero = (ea == 11'd0)   && (ma == 52'd0);
  wire b_is_zero = (eb == 11'd0)   && (mb == 52'd0);
  wire a_is_inf  = (ea == 11'h7FF) && (ma == 52'd0);
  wire b_is_inf  = (eb == 11'h7FF) && (mb == 52'd0);
  wire a_is_nan  = (ea == 11'h7FF) && (ma != 52'd0);
  wire b_is_nan  = (eb == 11'h7FF) && (mb != 52'd0);

  reg  [127:0] qwin;   // canonical quotient window for the subnormal path
  reg  [63:0]  qwin_r; // that window (at most 57 bits), held for the subnormal stages
  reg  [63:0]  den_win;// that window on the subnormal grid, its dropped bits folded into sticky
  reg  [13:0]  rsh_r;  // the subnormal grid's extra right shift
  reg          to_subnormal;
  reg  [11:0]  qlead;

  always @(posedge clk) begin
    if (rst) begin state <= IDLE; done <= 1'b0; y <= 64'd0; end
    else case (state)
      IDLE: if (start) begin
        done <= 1'b0;
        sa <= a[63]; sb <= b[63]; rs <= a[63]^b[63];
        ea <= a[62:52]; eb <= b[62:52]; ma <= a[51:0]; mb <= b[51:0];
        state <= SETUP;
      end
      SETUP: begin
        // ── cycle 1: special-case detection + count-leading-zeros only (register lza/lzb) ──
        if (a_is_nan || b_is_nan)        begin special <= 1'b1; special_y <= QNAN; state <= FIN; end
        else if (a_is_inf && b_is_inf)   begin special <= 1'b1; special_y <= QNAN; state <= FIN; end
        else if (a_is_inf)               begin special <= 1'b1; special_y <= {rs, 11'h7FF, 52'd0}; state <= FIN; end
        else if (b_is_inf)               begin special <= 1'b1; special_y <= {rs, 63'd0}; state <= FIN; end  // finite/inf=0
        else if (a_is_zero && b_is_zero) begin special <= 1'b1; special_y <= QNAN; state <= FIN; end          // 0/0=NaN
        else if (b_is_zero)              begin special <= 1'b1; special_y <= {rs, 11'h7FF, 52'd0}; state <= FIN; end // x/0=inf
        else if (a_is_zero)              begin special <= 1'b1; special_y <= {rs, 63'd0}; state <= FIN; end    // 0/x=0
        else begin
          special <= 1'b0;
          lza <= clz53({(ea!=0), ma});
          lzb <= clz53({(eb!=0), mb});
          state <= SETUP2;
        end
      end
      SETUP2: begin
        // ── cycle 2: barrel-shift the significands and form the unbiased exponent — each its own short
        // cone off the registered lza/lzb (no clz→shift→wide-add chained together in one cycle) ──
        shl_a <= ({53'd0, (ea!=0), ma}) << lza;
        shl_b <= ({53'd0, (eb!=0), mb}) << lzb;
        exp_res <= (ea != 0 ? $signed({3'd0, ea}) - 14'sd1023 : -14'sd1022)
                 - (eb != 0 ? $signed({3'd0, eb}) - 14'sd1023 : -14'sd1022)
                 - $signed(lza) + $signed(lzb);
        state <= SETUP3;
      end
      SETUP3: begin
        // ── cycle 3: seed the digit-recurrence from the registered normalized significands ──
        den <= shl_b[52:0];
        num <= {3'd0, shl_a[52:0]};   // dividend significand (leading 1 at bit 52), room above to shift
        quo <= 56'd0;
        cnt <= 7'd56;                 // produce 56 quotient bits (53 + 3 GRS), first bit at position 55
        state <= RUN;
      end
      RUN: begin
        if (cnt != 7'd0) begin
          // ── one restoring-division step: compare THEN shift, so the first emitted bit is the 2^0
          // digit of the significand ratio (both operands in [1,2), so the ratio is in (0.5,2)). ──
          diff = sub72({16'd0, num}, {19'd0, den});
          if (!diff[72]) begin                   // num >= den -> quotient bit 1, subtract
            num <= {diff[54:0], 1'b0};            // (num - den) << 1
            quo <= {quo[54:0], 1'b1};
          end else begin
            num <= {num[54:0], 1'b0};             // num << 1
            quo <= {quo[54:0], 1'b0};
          end
          cnt <= cnt - 7'd1;
        end else begin
          state <= FIN;
        end
      end
      FIN: begin
        // ── stage 1: normalize + round the quotient; register the significand (fsig) and unbiased
        // exponent (fe_r). Overflow and special results carry constant exponent fields and are pre-packed into
        // y_reg; a subnormal result goes through FINS and FINP. The biased-exponent ADD is deferred to FIN2. ──
        if (special) begin
          y_reg <= special_y; normal_path <= 1'b0;
        end else begin
          // quotient of two [1,2) significands is in (0.5,2). Its leading 1 is at bit 55 (if >=1) or
          // bit 54. Normalize so the leading 1 sits at bit 55 (window [55:3]=53-bit sig, [2:0]=GRS),
          // with a non-empty division remainder `num` OR'd into sticky.
          if (quo[55]) begin
            g = quo[2]; r = quo[1]; s = (quo[0] | (num != 56'd0)); l = quo[3];
            up = g & (r | s | l);
            rnd = inc64({11'd0, quo[55:3]}, up);
            fe  = exp_res;
            qwin = {72'd0, quo};                 // leading 1 at bit 55 already
            if (num != 56'd0) qwin[0] = 1'b1;
          end else begin
            g = quo[1]; r = quo[0]; s = (num != 56'd0); l = quo[2];
            up = g & (r | s | l);
            rnd = inc64({11'd0, quo[54:2]}, up);
            fe  = exp_res - 14'sd1;
            qwin = {72'd0, quo} << 1;            // shift leading 1 from bit 54 up to bit 55
            if (num != 56'd0) qwin[0] = 1'b1;
          end
          if (rnd[53]) begin fsig <= rnd[53:1]; fe = fe + 14'sd1; end
          else         begin fsig <= rnd[52:0]; end

          // pre-round exponent (of the leading quotient bit) drives the underflow decision so the
          // subnormal path rounds the exact quotient window exactly ONCE (no double rounding).
          if (quo[55]) bexp = exp_res; else bexp = exp_res - 14'sd1;

          if (bexp < -14'sd1022) begin
            // A subnormal quotient: the un-rounded window (leading 1 at bit 55) goes to FINS and FINP, which shift
            // it onto the subnormal grid and round it once.
            rsh_r  <= (-14'sd1022) - bexp;
            qwin_r <= qwin;
            normal_path <= 1'b0;
            to_subnormal <= 1'b1;
          end else if (fe > 14'sd1023) begin
            y_reg <= {rs, 11'h7FF, 52'd0};                // overflow -> inf
            normal_path <= 1'b0;
            to_subnormal <= 1'b0;
          end else begin
            fe_r <= fe;                                   // registered exponent for FIN2's biased add
            normal_path <= 1'b1;
            to_subnormal <= 1'b0;
          end
        end
        state <= (!special && bexp < -14'sd1022) ? FINS : FIN2;
      end
      FINS: begin
        if (rsh_r >= 14'd128)    den_win <= {63'd0, |qwin_r};
        else if (rsh_r != 14'd0) den_win <= (qwin_r >> rsh_r) | {127'd0, below({64'd0, qwin_r}, rsh_r)};
        else                     den_win <= qwin_r;
        state <= FINP;
      end
      FINP: begin
        g2 = den_win[2]; r2 = den_win[1]; s2 = den_win[0]; l2 = den_win[3];
        up2 = g2 & (r2 | s2 | l2);
        rr = inc64({11'd0, den_win[55:3]}, up2);
        if (rr[52]) y_reg <= {rs, 11'd1, rr[51:0]};       // rounded up to the smallest normal
        else        y_reg <= {rs, 11'd0, rr[51:0]};
        state <= FIN2;
      end
      FIN2: begin
        // ── stage 2: the sole remaining exponent op — a short (fe_r + 1023) biased-exponent add off a
        // register, then pack. This is what keeps the exponent out of the deep FIN cone on silicon. ──
        if (normal_path) begin
          bexp = fe_r + 14'sd1023;
          y <= {rs, bexp[10:0], fsig[51:0]};
        end else begin
          y <= y_reg;
        end
        done <= 1'b1; state <= IDLE;
      end
    endcase
  end
endmodule

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// fp_sqrt — multi-cycle IEEE-754 binary64 square root, digit-recurrence (restoring, non-restoring
//   style bit-at-a-time), correctly rounded (RNE). start/done handshake.
//   sqrt(neg) -> NaN, sqrt(+inf)=+inf, sqrt(±0)=±0, sqrt(NaN)=NaN. Each step's trial subtract is a
//   parallel-prefix subtract; the leading-zero count, the normalizing shift and the round each take a stage.
// ─────────────────────────────────────────────────────────────────────────────────────────────────
module fp_sqrt(
  input         clk,
  input         rst,
  input         start,
  input  [63:0] a,
  output reg [63:0] y,
  output reg    done
);
  localparam [63:0] QNAN = 64'h7FF8000000000000;

  reg  [2:0] state;
  localparam IDLE = 3'd0, SETUP = 3'd1, RUN = 3'd2, FIN = 3'd3, NORM = 3'd4, PACK = 3'd5;

  reg  [10:0] ea;
  reg  [51:0] ma;
  reg  asign;                   // input sign (only matters for -0)
  reg  special;
  reg  [63:0] special_y;

  // Digit-recurrence sqrt. The radicand M (= significand, or 2×significand for an odd input exponent)
  // is in [1,4). We place M in a wide fixed-point lane with its binary point just below the top two
  // integer bits, then peel 2 radicand bits per root bit. `root` accumulates the root; the leading root
  // bit lands at a fixed position so we can extract sig/guard/round/sticky directly.
  reg  signed [13:0] exp_in;    // unbiased exponent of the input's leading significand bit
  reg  signed [13:0] exp_res;   // result exponent (floor(exp_in/2))
  reg  [131:0] rad;             // radicand lane (2 bits consumed per step)
  reg  [66:0]  root;            // accumulated root bits (leading root bit ends at a fixed position)
  reg  [67:0]  rem;             // running remainder
  reg  [7:0]   cnt;

  reg  [67:0] trial, test;
  reg  signed [13:0] lz;
  reg  [13:0]  lz_r;        // leading zeros of a subnormal input, for NORM
  reg  [72:0]  dt;
  reg  [63:0]  rnd_r;       // the rounded significand, for PACK
  // The highest set bit of a nonzero word, by halving: six levels, not a priority chain.
  function [5:0] lead_one64(input [63:0] m);
    reg [31:0] h5; reg [15:0] h4; reg [7:0] h3; reg [3:0] h2; reg [1:0] h1; reg [5:0] z;
    begin
      z[5] = ~|m[63:32];  h5 = z[5] ? m[31:0]  : m[63:32];
      z[4] = ~|h5[31:16]; h4 = z[4] ? h5[15:0] : h5[31:16];
      z[3] = ~|h4[15:8];  h3 = z[3] ? h4[7:0]  : h4[15:8];
      z[2] = ~|h3[7:4];   h2 = z[2] ? h3[3:0]  : h3[7:4];
      z[1] = ~|h2[3:2];   h1 = z[1] ? h2[1:0]  : h2[3:2];
      z[0] = ~h1[1];
      lead_one64 = ~z;
    end
  endfunction

  // Leading zeros of a 53-bit significand, 53 when it is zero.
  function [13:0] clz53(input [52:0] v);
    begin
      clz53 = (v == 53'd0) ? 14'd53 : 14'd52 - {8'd0, lead_one64({11'd0, v})};
    end
  endfunction

  // v + cin through a parallel-prefix AND: log2(64) levels. cin is the scan's bit 0, so bit i flips on cin and
  // every bit below it, and cin reaches the bits through the scan's tree rather than one net to all 64.
  function [63:0] inc64(input [63:0] v, input cin);
    reg [63:0] p; integer d;
    begin
      p = {v[62:0], cin};
      for (d = 1; d < 64; d = d * 2) p = p & ((p << d) | ((64'd1 << d) - 64'd1));
      inc64 = v ^ p;
    end
  endfunction

  // A word's bits below `s` as one flag: shifted to the top and tested, no (1 << s) - 1 mask, no borrow chain.
  function below(input [127:0] x, input [13:0] s);
    begin
      below = (s == 14'd0) ? 1'b0 : ((x << (14'd128 - s)) != 128'd0);
    end
  endfunction

  // x - y over 72 bits as a Kogge-Stone prefix subtract; bit 72 of the result is the borrow (1 when x < y).
  function [72:0] sub72(input [71:0] x, input [71:0] yy);
    reg [71:0] yb, g, p, p0; integer d;
    begin
      yb = ~yy;
      p0 = x ^ yb;
      g = (x & yb) | (p0 & 72'd1);
      p = p0;
      for (d = 1; d < 72; d = d * 2) begin
        g = g | (p & (g << d));
        p = p & (p << d);
      end
      sub72 = {~g[71], p0 ^ {g[70:0], 1'b1}};
    end
  endfunction

  reg  g,r,rr_s,l,up;
  reg  [63:0] rnd;
  reg  [52:0] fsig;
  reg  signed [13:0] fe;
  reg  signed [13:0] bexp;
  reg  [52:0] msig;             // normalized significand (leading 1 at bit 52)

  // Number of root bits we generate. We need 53 significand + guard + round + a sticky, so 56 useful
  // bits; produce a couple extra for the sticky tail. NROOT steps consume 2*NROOT radicand bits.
  localparam integer NROOT = 59;

  initial begin state = IDLE; done = 1'b0; y = 64'd0; end

  wire a_is_zero = (ea == 11'd0)   && (ma == 52'd0);
  wire a_is_inf  = (ea == 11'h7FF) && (ma == 52'd0);
  wire a_is_nan  = (ea == 11'h7FF) && (ma != 52'd0);

  always @(posedge clk) begin
    if (rst) begin state <= IDLE; done <= 1'b0; y <= 64'd0; end
    else case (state)
      IDLE: if (start) begin
        done <= 1'b0; asign <= a[63]; ea <= a[62:52]; ma <= a[51:0]; state <= SETUP;
      end
      SETUP: begin
        if (a_is_nan)                    begin special <= 1'b1; special_y <= QNAN; state <= FIN; end
        else if (asign && !a_is_zero)    begin special <= 1'b1; special_y <= QNAN; state <= FIN; end  // sqrt(neg)=NaN
        else if (a_is_inf)               begin special <= 1'b1; special_y <= 64'h7FF0000000000000; state <= FIN; end
        else if (a_is_zero)              begin special <= 1'b1; special_y <= {asign, 63'd0}; state <= FIN; end // sqrt(±0)=±0
        else begin
          special <= 1'b0;
          lz_r <= clz53({(ea!=0), ma});
          state <= NORM;
        end
      end
      NORM: begin
        begin
          lz = lz_r;
          msig = ({(ea!=0), ma} << lz);      // leading 1 at bit 52, value m in [1,2)
          exp_in = (ea != 0 ? $signed({3'd0, ea}) - 14'sd1023 : -14'sd1022) - $signed(lz);
          // Radicand M = m (even exp) or 2m (odd exp), in [1,4). Place M's binary point so the top two
          // lane bits [131:130] hold the integer part. m has its leading 1 at bit 52; shift it up to
          // bit 130 (for even) or 131 (for odd, = 2m). Then process pairs from the top down.
          if (exp_in[0]) begin
            rad <= {msig, 79'd0};             // 2m : leading 1 at bit 131 (52+79)
            exp_res <= (exp_in - 14'sd1) >>> 1;
          end else begin
            rad <= {1'b0, msig, 78'd0};       // m  : leading 1 at bit 130 (52+78)
            exp_res <= exp_in >>> 1;
          end
          root <= 67'd0; rem <= 68'd0; cnt <= NROOT[7:0];
          state <= RUN;
        end
      end
      RUN: begin
        if (cnt != 8'd0) begin
          // ── one restoring sqrt step ──
          test  = {rem[65:0], rad[131:130]};   // bring down the top 2 radicand bits
          trial = {root[65:0], 2'b01};          // 2*root + 1  at this scale
          dt = sub72({4'd0, test}, {4'd0, trial});
          if (!dt[72]) begin                    // test >= trial
            rem  <= dt[67:0];
            root <= {root[65:0], 1'b1};
          end else begin
            rem  <= test;
            root <= {root[65:0], 1'b0};
          end
          rad <= {rad[129:0], 2'b00};           // consume the two processed bits
          cnt <= cnt - 8'd1;
        end else state <= FIN;
      end
      FIN: begin
        if (special) begin
          y <= special_y; done <= 1'b1; state <= IDLE;
        end else begin
          // After NROOT steps the root is an NROOT-bit integer approximating sqrt(M) scaled up. The root
          // value is in [1,2) (since M in [1,4)), so its leading 1 is at bit (NROOT-1). We want the
          // 53-bit significand at the top: significand = root[NROOT-1 : NROOT-53], guard = root[NROOT-54],
          // round = root[NROOT-55], sticky = OR of the rest and the nonzero remainder.
          // With NROOT=59: leading at bit 58; sig=root[58:6]; guard=root[5]; round=root[4];
          // sticky = |root[3:0] | (rem!=0).
          g    = root[5];
          r    = root[4];
          rr_s = (|root[3:0]) | (rem != 68'd0);
          l    = root[6];
          up   = g & (r | rr_s | l);
          rnd_r <= inc64({11'd0, root[58:6]}, up);
          state <= PACK;
        end
      end
      PACK: begin
        rnd = rnd_r;
        if (rnd[53]) begin fsig = rnd[53:1]; fe = exp_res + 14'sd1; end
        else         begin fsig = rnd[52:0]; fe = exp_res; end
        // sqrt of any finite positive f64 lands in the normal range (the smallest positive subnormal
        // has sqrt ~ 2.2e-162, far above the subnormal threshold), so assemble directly.
        bexp = fe + 14'sd1023;
        y <= {1'b0, bexp[10:0], fsig[51:0]};
        done <= 1'b1; state <= IDLE;
      end
    endcase
  end
endmodule

// fp_conv — the f64 <-> i64 converters, FuncUnit::FFloorF (code 10, f64 -> i64 toward -inf) and FuncUnit::IntToF
// (code 13, i64 -> f64 round-to-nearest-ties-even), as a five-stage pipeline: no 64-bit carry chain, leading-one
// search or 64-bit shift shares a stage with another. `start` samples `x` and the direction into the unit's own
// registers, as every unit samples its operands, so no stage's logic reads a port another unit shares; `y` and `done`
// follow on the fifth clock. Bit-identical to the single-cycle conversions it replaces (== Rust `as` casts == the VM).
module fp_conv(input clk, input rst, input start, input to_float, input [63:0] x, output reg [63:0] y, output reg done);
  // Stage 0: the operand and the direction, as presented.
  reg        v0, f0;
  reg [63:0] x0;
  // Stage 1: the sign and magnitude (IntToF), or the decoded fields (FFloor).
  reg        v1, f1, s1, z1, small1, negsmall1;
  reg [63:0] m1;
  reg [10:0] ue1;
  // Stage 2: the leading one (IntToF), or the significand shifted to the binary point (FFloor).
  reg        v2, f2, s2, z2, small2, negsmall2, frac2;
  reg [63:0] m2;
  reg [5:0]  lead2;
  // Stage 3: the 53-bit significand and its round-up (IntToF), or |floor| before the sign (FFloor).
  reg        v3, f3, s3, z3, small3, negsmall3, up3;
  reg [52:0] sig3;
  reg [10:0] be3;
  reg [63:0] mag3;

  // The highest set bit of a nonzero word, by halving: each level asks whether the upper half is empty and keeps
  // the half that holds the leading one, six levels deep rather than a 64-long priority chain.
  function [5:0] lead_one(input [63:0] m);
    reg [31:0] h5; reg [15:0] h4; reg [7:0] h3; reg [3:0] h2; reg [1:0] h1; reg [5:0] z;
    begin
      z[5] = ~|m[63:32];  h5 = z[5] ? m[31:0]  : m[63:32];
      z[4] = ~|h5[31:16]; h4 = z[4] ? h5[15:0] : h5[31:16];
      z[3] = ~|h4[15:8];  h3 = z[3] ? h4[7:0]  : h4[15:8];
      z[2] = ~|h3[7:4];   h2 = z[2] ? h3[3:0]  : h3[7:4];
      z[1] = ~|h2[3:2];   h1 = z[1] ? h2[1:0]  : h2[3:2];
      z[0] = ~h1[1];
      lead_one = ~z;
    end
  endfunction

  // v + cin through a parallel-prefix AND of cin and v's low bits: log2(64) levels, never a ripple. cin is the
  // scan's bit 0, so it reaches the bits through the scan's tree rather than one net to all 64.
  function [63:0] inc64(input [63:0] v, input cin);
    reg [63:0] p; integer d;
    begin
      p = {v[62:0], cin};
      for (d = 1; d < 64; d = d * 2) p = p & ((p << d) | ((64'd1 << d) - 64'd1));
      inc64 = v ^ p;
    end
  endfunction

  wire [63:0] fsig = {11'd0, 1'b1, m1[51:0]};
  wire        fbig = ue1 >= 11'd52;
  wire [63:0] fmag = fbig ? (fsig << (ue1 - 11'd52)) : (fsig >> (11'd52 - ue1));
  wire        ffrac = !fbig && ((fsig << (ue1 + 11'd12)) != 64'd0);

  wire        wide = lead2 > 6'd52;
  wire [5:0]  sh = lead2 - 6'd52;
  wire [63:0] kept = m2 >> sh;
  wire [63:0] dropped = m2 << (7'd64 - {1'b0, sh});
  wire [63:0] aligned = m2 << (6'd52 - lead2);

  wire [63:0] rsig = inc64({11'd0, sig3}, up3);

  always @(posedge clk) begin
    if (rst) begin
      v0 <= 1'b0; v1 <= 1'b0; v2 <= 1'b0; v3 <= 1'b0; done <= 1'b0;
    end else begin
      v0 <= start;
      if (start) begin
        f0 <= to_float; x0 <= x;
      end

      v1 <= v0;
      if (v0) begin
        f1 <= f0;
        s1 <= x0[63];
        z1 <= x0 == 64'd0;
        m1 <= (f0 && x0[63]) ? inc64(~x0, 1'b1) : x0;
        small1 <= x0[62:52] < 11'd1023;
        negsmall1 <= x0[63] && !(x0[62:52] == 11'd0 && x0[51:0] == 52'd0);
        ue1 <= x0[62:52] - 11'd1023;
      end

      v2 <= v1; f2 <= f1; s2 <= s1; z2 <= z1; small2 <= small1; negsmall2 <= negsmall1;
      if (f1) begin
        m2 <= m1; lead2 <= lead_one(m1);
      end else begin
        m2 <= fmag; frac2 <= ffrac;
      end

      v3 <= v2; f3 <= f2; s3 <= s2; z3 <= z2; small3 <= small2; negsmall3 <= negsmall2;
      if (f2) begin
        be3 <= 11'd1023 + {5'd0, lead2};
        if (wide) begin
          sig3 <= kept[52:0];
          up3 <= dropped[63] && ((|dropped[62:0]) || kept[0]);
        end else begin
          sig3 <= aligned[52:0];
          up3 <= 1'b0;
        end
      end else begin
        mag3 <= inc64(m2, s2 && frac2);
      end

      done <= v3;
      if (f3) begin
        if (z3)            y <= 64'd0;
        else if (rsig[53]) y <= {s3, be3 + 11'd1, 52'd0};
        else               y <= {s3, be3, rsig[51:0]};
      end else begin
        if (small3)        y <= negsmall3 ? 64'hFFFF_FFFF_FFFF_FFFF : 64'd0;
        else if (s3)       y <= inc64(~mag3, 1'b1);
        else               y <= mag3;
      end
    end
  end
endmodule
