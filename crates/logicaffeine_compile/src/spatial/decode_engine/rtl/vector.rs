//! The vector units' functions as Verilog, each the same f32 operations in the same order as its numerics function
//! ([`crate::spatial::decode_engine::numerics`]): `exp`, `recip`, `rsqrt`, `ln`, `ln1p`, `softplus`, `sigmoid` and
//! `silu`, over the engine's f32 multiply and add, with the integer steps the functions take between them — the
//! nearest integer of an f32, an integer as an f32, a power-of-two scaling — and their tables as ROMs.
//!
//! The modules are combinational: one function of its operand, so a lock holds each to its numerics function over
//! every class of operand. `logos_exp_p`, `logos_recip_p`, `logos_rsqrt_p` and `logos_ln_p` are `exp`, `recip`, `rsqrt`
//! and `ln` for the user clock, an operand a clock on the pipelined units, the same operations in the same order with
//! their integer steps and tables registered between them ([`EXP_LATENCY`], [`RECIP_LATENCY`], [`RSQRT_LATENCY`],
//! [`LN_LATENCY`]).

use std::fmt::Write as _;

use super::fp32::{FADD_LATENCY, FMUL_LATENCY};

use crate::spatial::decode_engine::numerics::{
    EXP2_64, EXP_BOTTOM, EXP_TOP, INV_C, INV_LN2_64, LN2_64_HI, LN2_64_LO, LN2_HI, LN2_LO, LN_C, ONE_SIXTH, ONE_THIRD,
    RECIP_SEED, RSQRT_SEED, SOFTPLUS_TOP,
};

/// Every vector-unit module, with the f32 multiply and add they are built from.
pub fn vector_verilog() -> String {
    let mut v = String::new();
    v.push_str(super::fp32::fp32_verilog());
    v.push('\n');
    for (name, table) in [("exp2_64", &EXP2_64[..]), ("recip_seed", &RECIP_SEED[..]), ("rsqrt_seed", &RSQRT_SEED[..]), ("ln_c", &LN_C[..]), ("inv_c", &INV_C[..])] {
        writeln!(v, "module logos_rom_{name}(input [6:0] i, output reg [31:0] y);").unwrap();
        writeln!(v, "  always @* case (i)").unwrap();
        for (j, &bits) in table.iter().enumerate() {
            writeln!(v, "    7'd{j}: y = 32'h{bits:08x};").unwrap();
        }
        writeln!(v, "    default: y = 32'h00000000;\n  endcase\nendmodule\n").unwrap();
    }
    let constants = [
        ("INV_LN2_64", INV_LN2_64),
        ("LN2_64_HI", LN2_64_HI),
        ("LN2_64_LO", LN2_64_LO),
        ("LN2_HI", LN2_HI),
        ("LN2_LO", LN2_LO),
        ("ONE_SIXTH", ONE_SIXTH),
        ("ONE_THIRD", ONE_THIRD),
        ("EXP_TOP", EXP_TOP),
        ("EXP_BOTTOM", EXP_BOTTOM),
        ("SOFTPLUS_TOP", SOFTPLUS_TOP),
    ];
    let mut defines = String::new();
    for (name, value) in constants {
        writeln!(defines, "`define LOGOS_{name} 32'h{:08x}", value.to_bits()).unwrap();
    }
    v.push_str(&defines);
    v.push_str(VECTOR_VERILOG);
    v.push_str(super::ram::delay_verilog());
    v.push_str(&exp_p_verilog());
    v.push_str(&functions_p_verilog());
    v
}

/// Clocks from an integer entering `logos_i2f_p` to its f32 leaving: its magnitude and leading one registered, then the
/// shift and the f32.
pub const I2F_LATENCY: usize = 2;

/// Clocks from an f32 entering `logos_rne_p` to its nearest integer leaving: the shifted magnitude and its round bit
/// registered, then the add and the sign.
pub const RNE_P_LATENCY: usize = 2;

/// Clocks from an operand entering `logos_exp_p` to its exponential leaving: the range reduction's multiply, the
/// nearest integer ([`RNE_P_LATENCY`]), its f32 ([`I2F_LATENCY`]), the two reducing multiplies and subtractions, the
/// polynomial's three multiply-adds, the table's multiply and the result registered.
pub const EXP_LATENCY: usize =
    2 * FMUL_LATENCY + RNE_P_LATENCY + I2F_LATENCY + 2 * FADD_LATENCY + 3 * (FMUL_LATENCY + FADD_LATENCY) + FMUL_LATENCY + 1;

/// `logos_exp_p` with its delays counted in the pipelined units' latencies.
fn exp_p_verilog() -> String {
    let integer = FMUL_LATENCY + RNE_P_LATENCY + I2F_LATENCY;
    let at_s1 = integer + FMUL_LATENCY;
    let polynomial = at_s1 + 2 * FADD_LATENCY + 3 * (FMUL_LATENCY + FADD_LATENCY);
    [
        ("@I2F@", I2F_LATENCY),
        ("@X_AT_S1@", at_s1),
        ("@FADD@", FADD_LATENCY),
        ("@ROUND@", FMUL_LATENCY + FADD_LATENCY),
        ("@TABLE@", polynomial - integer),
        ("@POWER@", polynomial - integer + FMUL_LATENCY),
        ("@END@", EXP_LATENCY - 1),
        ("@LATENCY@", EXP_LATENCY),
    ]
    .iter()
    .fold(EXP_P_VERILOG.to_string(), |t, (k, v)| t.replace(k, &v.to_string()))
}

/// Clocks from an operand entering `logos_recip_p` to its reciprocal leaving: the seed registered with the operand, the
/// two Newton steps' multiply, subtraction and multiply each, the result registered.
pub const RECIP_LATENCY: usize = 2 * (2 * FMUL_LATENCY + FADD_LATENCY) + 2;

/// Clocks from an operand entering `logos_rsqrt_p` to its reciprocal square root leaving: the seed registered with the
/// operand, the two Newton steps' four multiplies and subtraction each, the result registered.
pub const RSQRT_LATENCY: usize = 2 * (4 * FMUL_LATENCY + FADD_LATENCY) + 2;

/// Clocks from an operand entering `logos_ln_p` to its logarithm leaving: the table index registered, the centre's
/// numerator and the exponent as f32s ([`I2F_LATENCY`]), the table's entries, the centre and the exponent registered,
/// the polynomial's four multiplies and four subtractions in turn, the three sums with the table's logarithm and the
/// exponent's, the result registered.
pub const LN_LATENCY: usize = 2 + I2F_LATENCY + 4 * FMUL_LATENCY + 4 * FADD_LATENCY + 3 * FADD_LATENCY + 1;

/// `logos_recip_p`, `logos_rsqrt_p` and `logos_ln_p` with their delays counted in the pipelined units' latencies.
fn functions_p_verilog() -> String {
    let (recip_newton, rsqrt_newton) = (2 * FMUL_LATENCY + FADD_LATENCY, 4 * FMUL_LATENCY + FADD_LATENCY);
    let poly = 4 * FMUL_LATENCY + 4 * FADD_LATENCY;
    [
        ("@RECIP_STEP@", FMUL_LATENCY + FADD_LATENCY),
        ("@RECIP_NEWTONS@", 2 * recip_newton),
        ("@RECIP_NEWTON@", recip_newton),
        ("@RECIP@", RECIP_LATENCY),
        ("@RSQRT_TO_E@", 2 * FMUL_LATENCY + FADD_LATENCY),
        ("@RSQRT_NEWTONS@", 2 * rsqrt_newton),
        ("@RSQRT_NEWTON@", rsqrt_newton),
        ("@RSQRT@", RSQRT_LATENCY),
        ("@FMUL@", FMUL_LATENCY),
        ("@D_T1@", FMUL_LATENCY + FADD_LATENCY),
        ("@D2_T3@", 2 * FADD_LATENCY + FMUL_LATENCY),
        ("@D_T4@", 2 * FMUL_LATENCY + FADD_LATENCY),
        ("@POLY@", poly),
        ("@A0@", poly + FADD_LATENCY - FMUL_LATENCY),
        ("@A3@", poly + 2 * FADD_LATENCY - FMUL_LATENCY),
        ("@LN_END@", poly + 3 * FADD_LATENCY),
        ("@LN@", LN_LATENCY),
        ("@FADD@", FADD_LATENCY),
        ("@I2F@", I2F_LATENCY),
    ]
    .iter()
    .fold(FUNCTIONS_P_VERILOG.to_string(), |t, (k, v)| t.replace(k, &v.to_string()))
}

const FUNCTIONS_P_VERILOG: &str = r#"
// recip on the pipelined units, an operand a clock: y is logos_recip of the operand @RECIP@ clocks before, its two
// Newton steps' f32 operations in their order, the seed taken from the table and scaled and registered with the
// operand, the special cases held to the end.
module logos_recip_p(input clk, input [31:0] x_in, output reg [31:0] y);
  wire [31:0] xc = `LOGOS_FTZ(x_in);
  wire signed [15:0] ec = $signed({8'd0, xc[30:23]}) - 16'sd127;
  wire [31:0] seed_t, seed_m;
  logos_rom_recip_seed rom(.i(xc[22:16]), .y(seed_t));
  logos_ldexp ld(.x(seed_t), .k(-ec), .y(seed_m));
  reg [31:0] x, seed;
  always @(posedge clk) begin x <= xc; seed <= {xc[31], seed_m[30:0]}; end
  wire nan = (x[30:23] == 8'hff) && (x[22:0] != 23'd0);
  wire inf = (x[30:0] == 31'h7f80_0000);
  wire zero = (x[30:0] == 31'd0);
  wire [31:0] xy0, d0, y1, xy1, d1, y2, seed_d, x_d, y1_d;
  logos_fmul_p a0(.clk(clk), .a(x), .b(seed), .y(xy0));
  logos_fadd_p b0(.clk(clk), .a(32'h4000_0000), .b({~xy0[31], xy0[30:0]}), .y(d0));
  logos_delay #(.W(32), .D(@RECIP_STEP@)) ds(.clk(clk), .d(seed), .q(seed_d));
  logos_fmul_p c0(.clk(clk), .a(seed_d), .b(d0), .y(y1));
  logos_delay #(.W(32), .D(@RECIP_NEWTON@)) dx(.clk(clk), .d(x), .q(x_d));
  logos_fmul_p a1(.clk(clk), .a(x_d), .b(y1), .y(xy1));
  logos_fadd_p b1(.clk(clk), .a(32'h4000_0000), .b({~xy1[31], xy1[30:0]}), .y(d1));
  logos_delay #(.W(32), .D(@RECIP_STEP@)) dy(.clk(clk), .d(y1), .q(y1_d));
  logos_fmul_p c1(.clk(clk), .a(y1_d), .b(d1), .y(y2));
  wire [34:0] flags;
  logos_delay #(.W(35), .D(@RECIP_NEWTONS@)) df(.clk(clk), .d({nan, zero, inf, x}), .q(flags));
  always @(posedge clk) begin
    if (flags[34]) y <= flags[31:0];
    else if (flags[33]) y <= {flags[31], 8'hff, 23'd0};
    else if (flags[32]) y <= {flags[31], 31'd0};
    else y <= y2;
  end
endmodule

// rsqrt on the pipelined units, an operand a clock: y is logos_rsqrt of the operand @RSQRT@ clocks before, its two
// Newton steps' f32 operations in their order, the seed taken from the table and scaled and registered with the
// operand, the special cases held to the end.
module logos_rsqrt_p(input clk, input [31:0] x_in, output reg [31:0] y);
  wire [31:0] xc = `LOGOS_FTZ(x_in);
  wire signed [15:0] ec = $signed({8'd0, xc[30:23]}) - 16'sd127;
  wire parity = ec[0];
  wire signed [15:0] half = -((ec - $signed({15'd0, parity})) >>> 1);
  wire [31:0] seed_t, seed_c;
  logos_rom_rsqrt_seed rom(.i({parity, xc[22:17]}), .y(seed_t));
  logos_ldexp ld(.x(seed_t), .k(half), .y(seed_c));
  reg [31:0] x, seed;
  always @(posedge clk) begin x <= xc; seed <= seed_c; end
  wire nan = (x[30:23] == 8'hff) && (x[22:0] != 23'd0);
  wire inf = (x[30:0] == 31'h7f80_0000);
  wire zero = (x[30:0] == 31'd0);
  wire [31:0] xy0, xyy0, h0, d0, y1, xy1, xyy1, h1, d1, y2, s_a, s_e, x_d, y1_a, y1_e;
  logos_fmul_p a0(.clk(clk), .a(x), .b(seed), .y(xy0));
  logos_delay #(.W(32), .D(@FMUL@)) dsa(.clk(clk), .d(seed), .q(s_a));
  logos_fmul_p b0(.clk(clk), .a(xy0), .b(s_a), .y(xyy0));
  logos_fmul_p c0(.clk(clk), .a(32'h3f00_0000), .b(xyy0), .y(h0));
  logos_fadd_p d0u(.clk(clk), .a(32'h3fc0_0000), .b({~h0[31], h0[30:0]}), .y(d0));
  logos_delay #(.W(32), .D(@RSQRT_TO_E@)) dse(.clk(clk), .d(s_a), .q(s_e));
  logos_fmul_p e0(.clk(clk), .a(s_e), .b(d0), .y(y1));
  logos_delay #(.W(32), .D(@RSQRT_NEWTON@)) dx(.clk(clk), .d(x), .q(x_d));
  logos_fmul_p a1(.clk(clk), .a(x_d), .b(y1), .y(xy1));
  logos_delay #(.W(32), .D(@FMUL@)) dya(.clk(clk), .d(y1), .q(y1_a));
  logos_fmul_p b1(.clk(clk), .a(xy1), .b(y1_a), .y(xyy1));
  logos_fmul_p c1(.clk(clk), .a(32'h3f00_0000), .b(xyy1), .y(h1));
  logos_fadd_p d1u(.clk(clk), .a(32'h3fc0_0000), .b({~h1[31], h1[30:0]}), .y(d1));
  logos_delay #(.W(32), .D(@RSQRT_TO_E@)) dye(.clk(clk), .d(y1_a), .q(y1_e));
  logos_fmul_p e1(.clk(clk), .a(y1_e), .b(d1), .y(y2));
  wire [34:0] flags;
  logos_delay #(.W(35), .D(@RSQRT_NEWTONS@)) df(.clk(clk), .d({nan || (x[31] && !zero), zero, inf, x}), .q(flags));
  always @(posedge clk) begin
    if (flags[34]) y <= 32'h7fc0_0000;
    else if (flags[33]) y <= 32'h7f80_0000;
    else if (flags[32]) y <= 32'd0;
    else y <= y2;
  end
endmodule

// ln on the pipelined units, an operand a clock: y is logos_ln of the operand @LN@ clocks before, its f32 operations
// in their order, the significand's table index registered, then the table's entries, the centre and the exponent as
// an f32 registered, each operand held to the operation that takes it, the special cases held to the end.
module logos_ln_p(input clk, input [31:0] x_in, output reg [31:0] y);
  wire [31:0] xc = `LOGOS_FTZ(x_in);
  wire bigc = xc[22];
  wire [31:0] fixed = bigc ? ({8'd0, 1'b1, xc[22:0]} << 5) : ({8'd0, 1'b1, xc[22:0]} << 6);
  wire [31:0] off = fixed - (32'd48 << 23);
  wire [31:0] jw = (off >> 23) + (((off[22:0] > 23'h40_0000) || (off[22:0] == 23'h40_0000 && off[23])) ? 32'd1 : 32'd0);
  reg [31:0] x1;
  reg [6:0] j;
  reg signed [31:0] be, cj;
  always @(posedge clk) begin
    x1 <= xc; j <= jw[6:0]; cj <= $signed({25'd0, jw[6:0]}) + 32'sd48;
    be <= $signed({24'd0, xc[30:23]}) - 32'sd127 + (bigc ? 32'sd1 : 32'sd0);
  end
  // The centre's numerator and the exponent as f32s on logos_i2f_p, the operand and the table index held beside them;
  // the centre the numerator over 64, an exact exponent's step (the numerator is 48 to 175).
  wire [31:0] cnum, ef_c, x1d;
  wire [6:0] jd;
  logos_i2f_p ci(.clk(clk), .n(cj), .y(cnum));
  logos_i2f_p ei(.clk(clk), .n(be), .y(ef_c));
  logos_delay #(.W(39), .D(@I2F@)) dj(.clk(clk), .d({x1, j}), .q({x1d, jd}));
  wire [31:0] c_c = {cnum[31], cnum[30:23] - 8'd6, cnum[22:0]};
  wire [31:0] invc_c, lnc_c;
  logos_rom_inv_c r0(.i(jd), .y(invc_c));
  logos_rom_ln_c r1(.i(jd), .y(lnc_c));
  reg [31:0] x, m, c, invc, lnc, ef;
  always @(posedge clk) begin
    x <= x1d; m <= {1'b0, x1d[22] ? 8'd126 : 8'd127, x1d[22:0]};
    c <= c_c; invc <= invc_c; lnc <= lnc_c; ef <= ef_c;
  end
  wire nan = (x[30:23] == 8'hff) && (x[22:0] != 23'd0);
  wire inf = (x[30:0] == 31'h7f80_0000);
  wire zero = (x[30:0] == 31'd0);
  wire [31:0] dm, d, d2, t0, t1, t2, t3, t4, poly, a0, a1, a2, a3, z;
  wire [31:0] invc_d, d_t1, d2_t3, d_t4, lnc_d, a0_d, a3_d;
  logos_fadd_p s0(.clk(clk), .a(m), .b({~c[31], c[30:0]}), .y(dm));
  logos_delay #(.W(32), .D(@FADD@)) di(.clk(clk), .d(invc), .q(invc_d));
  logos_fmul_p f0(.clk(clk), .a(dm), .b(invc_d), .y(d));
  logos_fmul_p f1(.clk(clk), .a(d), .b(d), .y(d2));
  logos_fmul_p f2(.clk(clk), .a(d), .b(32'h3e80_0000), .y(t0));
  logos_fadd_p s1(.clk(clk), .a(`LOGOS_ONE_THIRD), .b({~t0[31], t0[30:0]}), .y(t1));
  logos_delay #(.W(32), .D(@D_T1@)) dd1(.clk(clk), .d(d), .q(d_t1));
  logos_fmul_p f3(.clk(clk), .a(d_t1), .b(t1), .y(t2));
  logos_fadd_p s2(.clk(clk), .a(32'h3f00_0000), .b({~t2[31], t2[30:0]}), .y(t3));
  logos_delay #(.W(32), .D(@D2_T3@)) dd2(.clk(clk), .d(d2), .q(d2_t3));
  logos_fmul_p f4(.clk(clk), .a(d2_t3), .b(t3), .y(t4));
  logos_delay #(.W(32), .D(@D_T4@)) dd4(.clk(clk), .d(d_t1), .q(d_t4));
  logos_fadd_p s3(.clk(clk), .a(d_t4), .b({~t4[31], t4[30:0]}), .y(poly));
  logos_fmul_p f5(.clk(clk), .a(ef), .b(`LOGOS_LN2_HI), .y(a0));
  logos_delay #(.W(32), .D(@POLY@)) dl(.clk(clk), .d(lnc), .q(lnc_d));
  logos_fadd_p g0(.clk(clk), .a(lnc_d), .b(poly), .y(a1));
  logos_delay #(.W(32), .D(@A0@)) da0(.clk(clk), .d(a0), .q(a0_d));
  logos_fadd_p g1(.clk(clk), .a(a0_d), .b(a1), .y(a2));
  logos_fmul_p f6(.clk(clk), .a(ef), .b(`LOGOS_LN2_LO), .y(a3));
  logos_delay #(.W(32), .D(@A3@)) da3(.clk(clk), .d(a3), .q(a3_d));
  logos_fadd_p g2(.clk(clk), .a(a2), .b(a3_d), .y(z));
  wire [34:0] flags;
  logos_delay #(.W(35), .D(@LN_END@)) df(.clk(clk), .d({nan || (x[31] && !zero), zero, inf, x}), .q(flags));
  always @(posedge clk) begin
    if (flags[34]) y <= 32'h7fc0_0000;
    else if (flags[33]) y <= 32'hff80_0000;
    else if (flags[32]) y <= flags[31:0];
    else y <= z;
  end
endmodule
"#;

const EXP_P_VERILOG: &str = r#"
// logos_i2f for the user clock, an integer a clock: y is logos_i2f of the integer two clocks before, the integer's
// magnitude and its leading one registered, then the shift and the f32.
module logos_i2f_p(input clk, input signed [31:0] n, output reg [31:0] y);
  // The highest set bit of m, -1 for none.
  function signed [31:0] lead_of(input [31:0] m);
    integer i;
    begin
      lead_of = -1;
      for (i = 0; i < 32; i = i + 1) if (m[i]) lead_of = i;
    end
  endfunction
  wire [31:0] mag_c = n[31] ? -n : n;
  wire signed [31:0] lead_c = lead_of(mag_c);
  reg [31:0] mag;
  reg signed [31:0] lead;
  reg s;
  always @(posedge clk) begin mag <= mag_c; lead <= lead_c; s <= n[31]; end
  wire [31:0] mant = mag << (23 - lead);
  always @(posedge clk)
    if (lead < 0) y <= 32'd0;
    else y <= {s, 8'd127 + lead[7:0], mant[22:0]};
endmodule

// exp on the pipelined units, an operand a clock: y is logos_exp of the operand @LATENCY@ clocks before, its f32
// operations in their order, the nearest integer on logos_rne_p and its f32 on logos_i2f_p, the integer held beside it, the
// table's entry and the power of two taken from the integer and held to the polynomial, the special cases held to the end.
module logos_exp_p(input clk, input [31:0] x_in, output reg [31:0] y);
  wire [31:0] x = `LOGOS_FTZ(x_in);
  wire nan = (x[30:23] == 8'hff) && (x[22:0] != 23'd0);
  wire above, below;
  logos_flt top(.a(`LOGOS_EXP_TOP), .b(x), .y(above));
  logos_flt bottom(.a(x), .b(`LOGOS_EXP_BOTTOM), .y(below));
  wire [31:0] t;
  logos_fmul_p m0(.clk(clk), .a(x), .b(`LOGOS_INV_LN2_64), .y(t));
  wire signed [31:0] n1;
  logos_rne_p r(.clk(clk), .x(t), .n(n1));
  wire [31:0] nf;
  logos_i2f_p c(.clk(clk), .n(n1), .y(nf));
  wire signed [31:0] n;
  logos_delay #(.W(32), .D(@I2F@)) dn(.clk(clk), .d(n1), .q(n));
  wire [31:0] h, l, l_d, x_d, r1, r0;
  logos_fmul_p m1(.clk(clk), .a(nf), .b(`LOGOS_LN2_64_HI), .y(h));
  logos_fmul_p m2(.clk(clk), .a(nf), .b(`LOGOS_LN2_64_LO), .y(l));
  logos_delay #(.W(32), .D(@X_AT_S1@)) dx(.clk(clk), .d(x), .q(x_d));
  logos_fadd_p s1(.clk(clk), .a(x_d), .b({~h[31], h[30:0]}), .y(r1));
  logos_delay #(.W(32), .D(@FADD@)) dl(.clk(clk), .d(l), .q(l_d));
  logos_fadd_p s2(.clk(clk), .a(r1), .b({~l_d[31], l_d[30:0]}), .y(r0));
  // 1 + r·(1 + r·(1/2 + r/6)), r held for each multiply.
  wire [31:0] p3, q2, p2, q1, p1, p, r0_a, r0_b;
  logos_fmul_p m3(.clk(clk), .a(r0), .b(`LOGOS_ONE_SIXTH), .y(p3));
  logos_fadd_p a3(.clk(clk), .a(32'h3f00_0000), .b(p3), .y(q2));
  logos_delay #(.W(32), .D(@ROUND@)) dr0(.clk(clk), .d(r0), .q(r0_a));
  logos_fmul_p m4(.clk(clk), .a(r0_a), .b(q2), .y(p2));
  logos_fadd_p a2(.clk(clk), .a(32'h3f80_0000), .b(p2), .y(q1));
  logos_delay #(.W(32), .D(@ROUND@)) dr1(.clk(clk), .d(r0_a), .q(r0_b));
  logos_fmul_p m5(.clk(clk), .a(r0_b), .b(q1), .y(p1));
  logos_fadd_p a1(.clk(clk), .a(32'h3f80_0000), .b(p1), .y(p));
  wire [5:0] j = n[5:0];
  wire signed [31:0] mexp = (n - $signed({26'd0, j})) >>> 6;
  wire [31:0] tj_c, tj, tp, z;
  wire [15:0] k;
  logos_rom_exp2_64 rom(.i({1'b0, j}), .y(tj_c));
  logos_delay #(.W(32), .D(@TABLE@)) dt(.clk(clk), .d(tj_c), .q(tj));
  logos_fmul_p m6(.clk(clk), .a(tj), .b(p), .y(tp));
  logos_delay #(.W(16), .D(@POWER@)) dk(.clk(clk), .d(mexp[15:0]), .q(k));
  logos_ldexp ld(.x(tp), .k(k), .y(z));
  wire [34:0] flags;
  logos_delay #(.W(35), .D(@END@)) df(.clk(clk), .d({nan, above, below, x}), .q(flags));
  always @(posedge clk) begin
    if (flags[34]) y <= flags[31:0];
    else if (flags[33]) y <= 32'h7f80_0000;
    else if (flags[32]) y <= 32'd0;
    else y <= z;
  end
endmodule
"#;

const VECTOR_VERILOG: &str = r#"
// A denormal read as a zero of its sign.
`define LOGOS_FTZ(x) (((x[30:23]) == 8'd0) ? {x[31], 31'd0} : (x))

module logos_fsub(input [31:0] a, input [31:0] b, output [31:0] y);
  logos_fadd u(.a(a), .b({~b[31], b[30:0]}), .y(y));
endmodule

// a < b over finite or infinite f32s, a zero of either sign equal to the other.
module logos_flt(input [31:0] a, input [31:0] b, output y);
  wire az = (a[30:0] == 31'd0), bz = (b[30:0] == 31'd0);
  wire [31:0] ka = az ? 32'h8000_0000 : (a[31] ? ~a : {1'b1, a[30:0]});
  wire [31:0] kb = bz ? 32'h8000_0000 : (b[31] ? ~b : {1'b1, b[30:0]});
  assign y = ka < kb;
endmodule

// The nearest integer of a finite f32 below 2^30 in magnitude, ties to even.
module logos_rne(input [31:0] x, output reg signed [31:0] n);
  wire [7:0] ex = x[30:23];
  wire [23:0] sig = {1'b1, x[22:0]};
  integer s;
  reg [55:0] wide;
  reg [31:0] whole;
  reg [23:0] frac;
  reg up;
  always @* begin
    if (ex < 8'd126) n = 32'sd0;
    else if (ex >= 8'd150) begin
      whole = {8'd0, sig} << (ex - 8'd150);
      n = x[31] ? -$signed(whole) : $signed(whole);
    end else begin
      s = 150 - ex;
      wide = {32'd0, sig} >> s;
      whole = wide[31:0];
      frac = sig & ((24'd1 << s) - 24'd1);
      up = (frac > (24'd1 << (s - 1))) || (frac == (24'd1 << (s - 1)) && whole[0]);
      whole = whole + {31'd0, up};
      n = x[31] ? -$signed(whole) : $signed(whole);
    end
  end
endmodule

// logos_rne for the user clock in two stages: the shifted magnitude and its round bit, nearest even, registered with
// the sign; then the add and the sign. n is logos_rne of the f32 two clocks before.
module logos_rne_p(input clk, input [31:0] x, output reg signed [31:0] n);
  wire [7:0] ex = x[30:23];
  wire [23:0] sig = {1'b1, x[22:0]};
  integer s;
  reg [55:0] wide;
  reg [31:0] whole;
  reg [23:0] frac;
  reg up;
  always @* begin
    s = 0; wide = 56'd0; frac = 24'd0; up = 1'b0; whole = 32'd0;
    if (ex >= 8'd150) whole = {8'd0, sig} << (ex - 8'd150);
    else if (ex >= 8'd126) begin
      s = 150 - ex;
      wide = {32'd0, sig} >> s;
      whole = wide[31:0];
      frac = sig & ((24'd1 << s) - 24'd1);
      up = (frac > (24'd1 << (s - 1))) || (frac == (24'd1 << (s - 1)) && whole[0]);
    end
  end
  reg [31:0] whole1;
  reg up1, neg1;
  always @(posedge clk) begin whole1 <= whole; up1 <= up; neg1 <= x[31]; end
  wire [31:0] mag = whole1 + {31'd0, up1};
  always @(posedge clk) n <= neg1 ? -$signed(mag) : $signed(mag);
endmodule

// A signed integer below 2^24 in magnitude as an f32, exactly.
module logos_i2f(input signed [31:0] n, output reg [31:0] y);
  reg [31:0] mag, mant;
  integer lead, i;
  always @* begin
    mag = n[31] ? -n : n;
    lead = -1;
    for (i = 0; i < 32; i = i + 1) if (mag[i]) lead = i;
    mant = mag << (23 - lead);
    if (lead < 0) y = 32'd0;
    else y = {n[31], 8'd127 + lead[7:0], mant[22:0]};
  end
endmodule

// x·2^k: exact while normal, infinite past the top, a zero of its sign below the bottom.
module logos_ldexp(input [31:0] x, input signed [15:0] k, output reg [31:0] y);
  wire [31:0] f = `LOGOS_FTZ(x);
  wire signed [16:0] e = $signed({9'd0, f[30:23]}) + k;
  always @* begin
    if (f[30:0] == 31'd0 || f[30:23] == 8'hff) y = f;
    else if (e >= 17'sd255) y = {f[31], 8'hff, 23'd0};
    else if (e <= 17'sd0) y = {f[31], 31'd0};
    else y = {f[31], e[7:0], f[22:0]};
  end
endmodule

module logos_exp(input [31:0] x_in, output reg [31:0] y);
  wire [31:0] x = `LOGOS_FTZ(x_in);
  wire nan = (x[30:23] == 8'hff) && (x[22:0] != 23'd0);
  wire above, below;
  logos_flt top(.a(`LOGOS_EXP_TOP), .b(x), .y(above));
  logos_flt bottom(.a(x), .b(`LOGOS_EXP_BOTTOM), .y(below));
  wire [31:0] t;
  logos_fmul m0(.a(x), .b(`LOGOS_INV_LN2_64), .y(t));
  wire signed [31:0] n;
  logos_rne r(.x(t), .n(n));
  wire [31:0] nf;
  logos_i2f c(.n(n), .y(nf));
  wire [31:0] h, l, r1, r0;
  logos_fmul m1(.a(nf), .b(`LOGOS_LN2_64_HI), .y(h));
  logos_fmul m2(.a(nf), .b(`LOGOS_LN2_64_LO), .y(l));
  logos_fsub s1(.a(x), .b(h), .y(r1));
  logos_fsub s2(.a(r1), .b(l), .y(r0));
  wire [31:0] p3, p2, p1, q2, q1, q0, p;
  logos_fmul m3(.a(r0), .b(`LOGOS_ONE_SIXTH), .y(p3));
  logos_fadd a3(.a(32'h3f00_0000), .b(p3), .y(q2));
  logos_fmul m4(.a(r0), .b(q2), .y(p2));
  logos_fadd a2(.a(32'h3f80_0000), .b(p2), .y(q1));
  logos_fmul m5(.a(r0), .b(q1), .y(p1));
  logos_fadd a1(.a(32'h3f80_0000), .b(p1), .y(p));
  wire [5:0] j = n[5:0];
  wire signed [31:0] mexp = (n - $signed({26'd0, j})) >>> 6;
  wire [31:0] tj, tp, z;
  logos_rom_exp2_64 rom(.i({1'b0, j}), .y(tj));
  logos_fmul m6(.a(tj), .b(p), .y(tp));
  logos_ldexp ld(.x(tp), .k(mexp[15:0]), .y(z));
  always @* begin
    if (nan) y = x;
    else if (above) y = 32'h7f80_0000;
    else if (below) y = 32'd0;
    else y = z;
  end
endmodule

module logos_recip(input [31:0] x_in, output reg [31:0] y);
  wire [31:0] x = `LOGOS_FTZ(x_in);
  wire nan = (x[30:23] == 8'hff) && (x[22:0] != 23'd0);
  wire inf = (x[30:0] == 31'h7f80_0000);
  wire zero = (x[30:0] == 31'd0);
  wire signed [15:0] e = $signed({8'd0, x[30:23]}) - 16'sd127;
  wire [31:0] seed_t, seed_m;
  logos_rom_recip_seed rom(.i(x[22:16]), .y(seed_t));
  logos_ldexp ld(.x(seed_t), .k(-e), .y(seed_m));
  wire [31:0] seed = {x[31], seed_m[30:0]};
  wire [31:0] xy0, d0, y1, xy1, d1, y2;
  logos_fmul a0(.a(x), .b(seed), .y(xy0));
  logos_fsub b0(.a(32'h4000_0000), .b(xy0), .y(d0));
  logos_fmul c0(.a(seed), .b(d0), .y(y1));
  logos_fmul a1(.a(x), .b(y1), .y(xy1));
  logos_fsub b1(.a(32'h4000_0000), .b(xy1), .y(d1));
  logos_fmul c1(.a(y1), .b(d1), .y(y2));
  always @* begin
    if (nan) y = x;
    else if (zero) y = {x[31], 8'hff, 23'd0};
    else if (inf) y = {x[31], 31'd0};
    else y = y2;
  end
endmodule

module logos_rsqrt(input [31:0] x_in, output reg [31:0] y);
  wire [31:0] x = `LOGOS_FTZ(x_in);
  wire nan = (x[30:23] == 8'hff) && (x[22:0] != 23'd0);
  wire inf = (x[30:0] == 31'h7f80_0000);
  wire zero = (x[30:0] == 31'd0);
  wire signed [15:0] e = $signed({8'd0, x[30:23]}) - 16'sd127;
  wire parity = e[0];
  wire signed [15:0] half = -((e - $signed({15'd0, parity})) >>> 1);
  wire [31:0] seed_t, seed;
  logos_rom_rsqrt_seed rom(.i({parity, x[22:17]}), .y(seed_t));
  logos_ldexp ld(.x(seed_t), .k(half), .y(seed));
  wire [31:0] xy0, xyy0, h0, d0, y1, xy1, xyy1, h1, d1, y2;
  logos_fmul a0(.a(x), .b(seed), .y(xy0));
  logos_fmul b0(.a(xy0), .b(seed), .y(xyy0));
  logos_fmul c0(.a(32'h3f00_0000), .b(xyy0), .y(h0));
  logos_fsub d0u(.a(32'h3fc0_0000), .b(h0), .y(d0));
  logos_fmul e0(.a(seed), .b(d0), .y(y1));
  logos_fmul a1(.a(x), .b(y1), .y(xy1));
  logos_fmul b1(.a(xy1), .b(y1), .y(xyy1));
  logos_fmul c1(.a(32'h3f00_0000), .b(xyy1), .y(h1));
  logos_fsub d1u(.a(32'h3fc0_0000), .b(h1), .y(d1));
  logos_fmul e1(.a(y1), .b(d1), .y(y2));
  always @* begin
    if (nan || (x[31] && !zero)) y = 32'h7fc0_0000;
    else if (zero) y = 32'h7f80_0000;
    else if (inf) y = 32'd0;
    else y = y2;
  end
endmodule

module logos_ln(input [31:0] x_in, output reg [31:0] y);
  wire [31:0] x = `LOGOS_FTZ(x_in);
  wire nan = (x[30:23] == 8'hff) && (x[22:0] != 23'd0);
  wire inf = (x[30:0] == 31'h7f80_0000);
  wire zero = (x[30:0] == 31'd0);
  // The significand in [1, 2), halved into [3/4, 3/2) when it is 3/2 or more.
  wire big = x[22];
  wire signed [31:0] be = $signed({24'd0, x[30:23]}) - 32'sd127 + (big ? 32'sd1 : 32'sd0);
  wire [31:0] m = {1'b0, big ? 8'd126 : 8'd127, x[22:0]};
  // (m − 3/4)·64 exactly, in units of 2^-23 of the significand: 64·m − 48, then its nearest integer.
  wire [31:0] fixed = big ? ({8'd0, 1'b1, x[22:0]} << 5) : ({8'd0, 1'b1, x[22:0]} << 6);
  wire [31:0] off = fixed - (32'd48 << 23);
  wire [31:0] jw = (off >> 23) + (((off[22:0] > 23'h40_0000) || (off[22:0] == 23'h40_0000 && off[23])) ? 32'd1 : 32'd0);
  wire [6:0] j = jw[6:0];
  wire [31:0] cnum, c;
  logos_i2f ci(.n($signed({25'd0, j}) + 32'sd48), .y(cnum));
  logos_ldexp cl(.x(cnum), .k(-16'sd6), .y(c));
  wire [31:0] invc, lnc, dm, d, d2, t0, t1, t2, t3, t4, poly, ef, a0, a1, a2, a3;
  logos_rom_inv_c r0(.i(j), .y(invc));
  logos_rom_ln_c r1(.i(j), .y(lnc));
  logos_fsub s0(.a(m), .b(c), .y(dm));
  logos_fmul f0(.a(dm), .b(invc), .y(d));
  logos_fmul f1(.a(d), .b(d), .y(d2));
  logos_fmul f2(.a(d), .b(32'h3e80_0000), .y(t0));
  logos_fsub s1(.a(`LOGOS_ONE_THIRD), .b(t0), .y(t1));
  logos_fmul f3(.a(d), .b(t1), .y(t2));
  logos_fsub s2(.a(32'h3f00_0000), .b(t2), .y(t3));
  logos_fmul f4(.a(d2), .b(t3), .y(t4));
  logos_fsub s3(.a(d), .b(t4), .y(poly));
  logos_i2f ei(.n(be), .y(ef));
  logos_fmul f5(.a(ef), .b(`LOGOS_LN2_HI), .y(a0));
  logos_fadd g0(.a(lnc), .b(poly), .y(a1));
  logos_fadd g1(.a(a0), .b(a1), .y(a2));
  logos_fmul f6(.a(ef), .b(`LOGOS_LN2_LO), .y(a3));
  wire [31:0] z;
  logos_fadd g2(.a(a2), .b(a3), .y(z));
  always @* begin
    if (nan || (x[31] && !zero)) y = 32'h7fc0_0000;
    else if (zero) y = 32'hff80_0000;
    else if (inf) y = x;
    else y = z;
  end
endmodule

module logos_ln1p(input [31:0] t_in, output reg [31:0] y);
  wire [31:0] t = `LOGOS_FTZ(t_in);
  wire [31:0] u, lu, um1, err, ru, corr, z;
  logos_fadd a0(.a(32'h3f80_0000), .b(t), .y(u));
  logos_ln l0(.x_in(u), .y(lu));
  logos_fsub s0(.a(u), .b(32'h3f80_0000), .y(um1));
  logos_fsub s1(.a(t), .b(um1), .y(err));
  logos_recip r0(.x_in(u), .y(ru));
  logos_fmul m0(.a(err), .b(ru), .y(corr));
  logos_fadd a1(.a(lu), .b(corr), .y(z));
  always @* y = (u == 32'h3f80_0000) ? t : z;
endmodule

module logos_softplus(input [31:0] u_in, output reg [31:0] y);
  wire [31:0] u = `LOGOS_FTZ(u_in);
  wire above;
  logos_flt top(.a(`LOGOS_SOFTPLUS_TOP), .b(u), .y(above));
  wire [31:0] en, past, ep, near;
  logos_exp e0(.x_in({~u[31], u[30:0]}), .y(en));
  logos_fadd a0(.a(u), .b(en), .y(past));
  logos_exp e1(.x_in(u), .y(ep));
  logos_ln1p l0(.t_in(ep), .y(near));
  always @* y = above ? past : near;
endmodule

module logos_sigmoid(input [31:0] x_in, output [31:0] y);
  wire [31:0] x = `LOGOS_FTZ(x_in);
  wire [31:0] en, d;
  logos_exp e0(.x_in({~x[31], x[30:0]}), .y(en));
  logos_fadd a0(.a(32'h3f80_0000), .b(en), .y(d));
  logos_recip r0(.x_in(d), .y(y));
endmodule

module logos_silu(input [31:0] x, output [31:0] y);
  wire [31:0] s;
  logos_sigmoid g(.x_in(x), .y(s));
  logos_fmul m(.a(x), .b(s), .y(y));
endmodule
"#;
