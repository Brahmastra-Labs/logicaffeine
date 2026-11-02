//! simd — configurable-precision packed SIMD lanes for the fabric datapath (`FabricTarget::simd`).
//!
//! HiVec's TOPS/W comes from packing several narrow lanes into one datapath. Our analog packs `lanes` values
//! of `width` bits into the existing 64-bit word and operates lane-wise. The one thing that must never happen
//! is a carry bleeding across a lane boundary — the "silent SIMD integer miscompile". So the packed hardware
//! op is defined here as an explicit SWAR (SIMD-within-a-register) form built from the fabric's own `AluOp`
//! tiles, and `simd_tile_lock` proves it BIT-IDENTICAL to a lane-wise reference (N independent scalar ops) for
//! ALL 64-bit inputs via our certified SAT. The reference is the spec; the SWAR form is what silicon runs.

use super::fabric_rtl::{AluOp, FabricOp, Src};

/// A packed lane type: `lanes` values of `width` bits, `lanes * width == 64`.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, serde::Serialize, serde::Deserialize, Ord,
)]
pub enum LaneTy {
    I8x8,
    I16x4,
    I32x2,
}

/// A packed integer SIMD operation carried by a `LoopCell::VectorArith` / `CellKind::Simd` tile. `Add` is the
/// carry-isolated SWAR add (proven ≡ lane-wise by `simd_tile_lock`); the bitwise ops never cross a lane
/// boundary, so they are the native 64-bit op.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, serde::Serialize, serde::Deserialize, Ord,
)]
pub enum VecOp {
    Add,
    And,
    Or,
    Xor,
}

impl VecOp {
    /// Evaluate the packed op lane-wise on two 64-bit operand words — the tile's semantics (= N scalar ops).
    pub fn eval(self, ty: LaneTy, a: u64, b: u64) -> u64 {
        match self {
            VecOp::Add => eval_lanewise_add(ty, a, b),
            VecOp::And => a & b,
            VecOp::Or => a | b,
            VecOp::Xor => a ^ b,
        }
    }

    /// Combinational Verilog for the packed op over two 64-bit operand expressions. `Add` emits the SWAR form
    /// (carry blocked at every lane boundary); the bitwise ops are native (inherently lane-isolated).
    pub fn verilog(self, ty: LaneTy, a: &str, b: &str) -> String {
        match self {
            VecOp::Add => {
                let m = swar_lo_mask(ty);
                let hm = !m;
                format!("((({a}) & 64'h{m:016x}) + (({b}) & 64'h{m:016x})) ^ ((({a}) ^ ({b})) & 64'h{hm:016x})")
            }
            VecOp::And => format!("(({a}) & ({b}))"),
            VecOp::Or => format!("(({a}) | ({b}))"),
            VecOp::Xor => format!("(({a}) ^ ({b}))"),
        }
    }
}

impl LaneTy {
    pub fn lanes(self) -> usize {
        match self {
            LaneTy::I8x8 => 8,
            LaneTy::I16x4 => 4,
            LaneTy::I32x2 => 2,
        }
    }
    pub fn width(self) -> usize {
        64 / self.lanes()
    }
    pub const ALL: &'static [LaneTy] = &[LaneTy::I8x8, LaneTy::I16x4, LaneTy::I32x2];
}

/// The full low-`width` mask replicated across every lane (`0xFFFF…` per lane).
fn lane_full_mask(ty: LaneTy) -> u64 {
    let w = ty.width();
    let lane = if w == 64 { u64::MAX } else { (1u64 << w) - 1 };
    (0..ty.lanes()).fold(0u64, |m, j| m | (lane << (j * w)))
}

/// The SWAR carry-block mask: every lane's low `width-1` bits set (top bit of each lane cleared).
fn swar_lo_mask(ty: LaneTy) -> u64 {
    let w = ty.width();
    let lane = (1u64 << (w - 1)) - 1;
    (0..ty.lanes()).fold(0u64, |m, j| m | (lane << (j * w)))
}

/// The LANE-WISE reference for packed add — the SPEC: each lane independently computes `(a_j + b_j) mod 2^w`,
/// expressed as an `AluOp` residual (shift the lane down, add, mask to `width`, shift back, OR the lanes). This
/// is "N independent scalar adds" made concrete.
pub fn lanewise_add_body(ty: LaneTy) -> (Vec<FabricOp>, usize) {
    let (w, lanes) = (ty.width(), ty.lanes());
    let full = lane_full_mask(ty) & ((1u64 << w) - 1); // low-lane full mask (single lane worth)
    let mut cells: Vec<FabricOp> = Vec::new();
    let mut lane_vals: Vec<Src> = Vec::new();
    for j in 0..lanes {
        let sh = (j * w) as i64;
        let (aj, bj) = if j == 0 {
            (Src::In(0), Src::In(1))
        } else {
            cells.push(FabricOp {
                op: AluOp::UShr,
                a: Src::In(0),
                b: Src::Const(sh),
            });
            let a = Src::Val(cells.len() - 1);
            cells.push(FabricOp {
                op: AluOp::UShr,
                a: Src::In(1),
                b: Src::Const(sh),
            });
            let b = Src::Val(cells.len() - 1);
            (a, b)
        };
        cells.push(FabricOp {
            op: AluOp::Add,
            a: aj,
            b: bj,
        });
        let sum = Src::Val(cells.len() - 1);
        cells.push(FabricOp {
            op: AluOp::And,
            a: sum,
            b: Src::Const(full as i64),
        });
        let masked = Src::Val(cells.len() - 1);
        let laneval = if j == 0 {
            masked
        } else {
            cells.push(FabricOp {
                op: AluOp::Shl,
                a: masked,
                b: Src::Const(sh),
            });
            Src::Val(cells.len() - 1)
        };
        lane_vals.push(laneval);
    }
    // OR the lanes together.
    let mut acc = lane_vals[0];
    for &lv in &lane_vals[1..] {
        cells.push(FabricOp {
            op: AluOp::Or,
            a: acc,
            b: lv,
        });
        acc = Src::Val(cells.len() - 1);
    }
    let result = match acc {
        Src::Val(k) => k,
        _ => cells.len() - 1,
    };
    (cells, result)
}

/// The PACKED SWAR add — one datapath op that blocks carry at every lane boundary:
/// `((a & M) + (b & M)) ^ ((a ^ b) & ~M)`, where `M` clears each lane's top bit. This is what the SIMD tile
/// emits; `simd_tile_lock` certifies it equals [`lanewise_add_body`].
pub fn swar_add_body(ty: LaneTy) -> (Vec<FabricOp>, usize) {
    let m = swar_lo_mask(ty) as i64;
    let hm = !swar_lo_mask(ty) as i64;
    let cells = vec![
        FabricOp {
            op: AluOp::And,
            a: Src::In(0),
            b: Src::Const(m),
        }, // v0 = a & M
        FabricOp {
            op: AluOp::And,
            a: Src::In(1),
            b: Src::Const(m),
        }, // v1 = b & M
        FabricOp {
            op: AluOp::Add,
            a: Src::Val(0),
            b: Src::Val(1),
        }, // v2 = (a&M)+(b&M)
        FabricOp {
            op: AluOp::Xor,
            a: Src::In(0),
            b: Src::In(1),
        }, // v3 = a ^ b
        FabricOp {
            op: AluOp::And,
            a: Src::Val(3),
            b: Src::Const(hm),
        }, // v4 = (a^b) & ~M
        FabricOp {
            op: AluOp::Xor,
            a: Src::Val(2),
            b: Src::Val(4),
        }, // v5 = result
    ];
    (cells, 5)
}

/// A deliberately NAIVE (non-isolated) packed add — a single 64-bit add. Carry bleeds across lane boundaries,
/// so it must NOT equal the lane-wise spec (the non-vacuity witness for the isolation proof).
pub fn naive_add_body() -> (Vec<FabricOp>, usize) {
    (
        vec![FabricOp {
            op: AluOp::Add,
            a: Src::In(0),
            b: Src::In(1),
        }],
        0,
    )
}

/// Packed-float add for **F32×2** (two IEEE-754 `f32` lanes in one 64-bit word, lane 0 low) — the SPEC: each
/// lane an independent scalar `f32` add. A NaN/Inf/subnormal in one lane cannot perturb the other; SAT cannot
/// bitblast IEEE arithmetic, so `simd_tile_lock` certifies this by per-lane bit-exactness + a cross-lane
/// non-contamination battery (the way the scalar float lane is already proven). This is the model the parallel
/// `fp32` FPU lanes realize in silicon.
pub fn eval_lanewise_fadd_f32x2(a: u64, b: u64) -> u64 {
    let lo = f32::from_bits(a as u32) + f32::from_bits(b as u32);
    let hi = f32::from_bits((a >> 32) as u32) + f32::from_bits((b >> 32) as u32);
    ((hi.to_bits() as u64) << 32) | lo.to_bits() as u64
}

/// Packed-float multiply for **F32×2** (see [`eval_lanewise_fadd_f32x2`]).
pub fn eval_lanewise_fmul_f32x2(a: u64, b: u64) -> u64 {
    let lo = f32::from_bits(a as u32) * f32::from_bits(b as u32);
    let hi = f32::from_bits((a >> 32) as u32) * f32::from_bits((b >> 32) as u32);
    ((hi.to_bits() as u64) << 32) | lo.to_bits() as u64
}

/// Pack two `f32` lanes into one 64-bit word (lane 0 low).
pub fn pack_f32x2(lane0: f32, lane1: f32) -> u64 {
    ((lane1.to_bits() as u64) << 32) | lane0.to_bits() as u64
}

/// The synthesizable combinational IEEE-754 `f32` adder — a faithful port of the PROVEN f64 `fp_addsub`
/// (`benchmarks/silicon/fpu.v`): align both significands in a wide 128-bit window (64 bits of alignment room,
/// so every bit survives to sticky), add/sub, leading-zero-count, normalize the leading 1 to bit 26 (24-bit
/// significand at `[26:3]`, guard/round/sticky at `[2:0]`), round-to-nearest-ties-even, and pack (overflow→Inf,
/// underflow→subnormal via `pack_sub32`). `sub=1` computes `a-b`. Proven bit-exact to Rust `f32` under iverilog
/// by `simd_float_rtl_lock`. This module is instantiated once per lane by the packed F32×2 tile.
pub const FP32_ADD_V: &str = r#"module fp32_add(input [31:0] a, input [31:0] b_in, input sub, output reg [31:0] y);
  localparam [31:0] QNAN = 32'h7FC00000;
  reg [31:0] b;
  reg sa, sb;
  reg [7:0] ea, eb;
  reg [22:0] ma, mb;
  reg azero,bzero,ainf,binf,anan,bnan,asub,bsub;
  reg [8:0] xa, xb;
  reg ha, hb;
  reg [23:0] siga, sigb;
  reg [11:0] exp_l, shamt, lead;
  reg [23:0] sig_big, sig_small;
  reg sgn_big, sgn_small, res_sign, op_is_sub;
  reg [127:0] big_ext, small_ext, sum_ext, norm_sig;
  integer i;
  reg signed [11:0] norm_exp;
  reg [24:0] rounded;
  reg guard, roundb, sticky, lsb, round_up;
  reg [23:0] final_sig;

  function [31:0] pack_sub32(input sgn, input [127:0] wsum, input [11:0] wlead, input signed [11:0] bexp);
    reg [11:0] rshift; reg [127:0] win; reg g2,r2,s2,l2,up2; reg [24:0] rr;
    begin
      rshift = 12'd1 - bexp;
      if (wlead >= 12'd26) begin
        win = wsum >> (wlead - 12'd26);
        if (|(wsum & ((128'd1 << (wlead - 12'd26)) - 128'd1))) win[0] = 1'b1;
      end else begin
        win = wsum << (12'd26 - wlead);
      end
      if (rshift >= 12'd128) begin s2 = |win; win = 128'd0; win[0] = s2; end
      else if (rshift != 12'd0) begin
        if (|(win & ((128'd1 << rshift) - 128'd1))) begin win = win >> rshift; win[0] = 1'b1; end
        else win = win >> rshift;
      end
      g2 = win[2]; r2 = win[1]; s2 = win[0]; l2 = win[3];
      up2 = g2 & (r2 | s2 | l2);
      rr = {1'b0, win[26:3]} + (up2 ? 25'd1 : 25'd0);
      if (rr[23]) pack_sub32 = {sgn, 8'd1, rr[22:0]};
      else        pack_sub32 = {sgn, 8'd0, rr[22:0]};
    end
  endfunction

  always @(*) begin
    b  = sub ? {~b_in[31], b_in[30:0]} : b_in;
    sa = a[31]; sb = b[31];
    ea = a[30:23]; eb = b[30:23];
    ma = a[22:0];  mb = b[22:0];
    azero = (ea==8'd0)  && (ma==23'd0); bzero = (eb==8'd0)  && (mb==23'd0);
    ainf  = (ea==8'hFF) && (ma==23'd0); binf  = (eb==8'hFF) && (mb==23'd0);
    anan  = (ea==8'hFF) && (ma!=23'd0); bnan  = (eb==8'hFF) && (mb!=23'd0);
    asub  = (ea==8'd0)  && (ma!=23'd0); bsub  = (eb==8'd0)  && (mb!=23'd0);
    xa = asub ? 9'd1 : {1'b0, ea};
    xb = bsub ? 9'd1 : {1'b0, eb};
    ha = (ea!=8'd0); hb = (eb!=8'd0);
    siga = {ha, ma}; sigb = {hb, mb};
    y = 32'd0;
    // pre-set datapath regs (avoid latches in the combinational always)
    exp_l=12'd0; shamt=12'd0; sig_big=24'd0; sig_small=24'd0; sgn_big=1'b0; sgn_small=1'b0;
    op_is_sub=1'b0; res_sign=1'b0; big_ext=128'd0; small_ext=128'd0; sum_ext=128'd0; lead=12'd0;
    norm_exp=12'sd0; norm_sig=128'd0; rounded=25'd0; final_sig=24'd0;
    guard=1'b0; roundb=1'b0; sticky=1'b0; lsb=1'b0; round_up=1'b0;

    if (anan || bnan) y = QNAN;
    else if (ainf && binf) y = (sa==sb) ? {sa,8'hFF,23'd0} : QNAN;
    else if (ainf) y = {sa,8'hFF,23'd0};
    else if (binf) y = {sb,8'hFF,23'd0};
    else if (azero && bzero) y = (sa==sb) ? {sa,31'd0} : 32'd0;
    else if (azero) y = b;
    else if (bzero) y = a;
    else begin
      if (xa > xb || (xa == xb && siga >= sigb)) begin
        exp_l={3'd0,xa}; sig_big=siga; sgn_big=sa; sig_small=sigb; sgn_small=sb; shamt={3'd0,(xa-xb)};
      end else begin
        exp_l={3'd0,xb}; sig_big=sigb; sgn_big=sb; sig_small=siga; sgn_small=sa; shamt={3'd0,(xb-xa)};
      end
      op_is_sub = (sgn_big != sgn_small);
      res_sign  = sgn_big;
      big_ext = {40'd0, sig_big, 64'd0};
      if (shamt >= 12'd128) begin small_ext = 128'd0; small_ext[0] = |sig_small; end
      else begin
        small_ext = ({40'd0, sig_small, 64'd0}) >> shamt;
        if (shamt != 12'd0 && |(({40'd0, sig_small, 64'd0}) & ((128'd1 << shamt) - 128'd1))) small_ext[0] = 1'b1;
      end
      if (op_is_sub) sum_ext = big_ext - small_ext; else sum_ext = big_ext + small_ext;
      if (sum_ext == 128'd0) y = 32'd0;
      else begin
        lead = 12'd0;
        for (i = 127; i >= 0; i = i - 1) if (sum_ext[i] && lead == 12'd0) lead = i[11:0];
        norm_exp = $signed({3'd0, exp_l[8:0]}) + $signed({3'd0, lead[8:0]}) - 12'sd87;
        if (lead >= 12'd26) begin
          norm_sig = sum_ext >> (lead - 12'd26);
          if (|(sum_ext & ((128'd1 << (lead - 12'd26)) - 128'd1))) norm_sig[0] = 1'b1;
        end else norm_sig = sum_ext << (12'd26 - lead);
        guard=norm_sig[2]; roundb=norm_sig[1]; sticky=norm_sig[0]; lsb=norm_sig[3];
        round_up = guard & (roundb | sticky | lsb);
        rounded = {1'b0, norm_sig[26:3]} + (round_up ? 25'd1 : 25'd0);
        if (rounded[24]) begin final_sig = rounded[24:1]; norm_exp = norm_exp + 12'sd1; end
        else final_sig = rounded[23:0];
        if (norm_exp >= 12'sd255) y = {res_sign, 8'hFF, 23'd0};
        else if (norm_exp <= 12'sd0) y = pack_sub32(res_sign, sum_ext, lead, norm_exp);
        else y = {res_sign, norm_exp[7:0], final_sig[22:0]};
      end
    end
  end
endmodule
"#;

/// The synthesizable combinational IEEE-754 `f32` multiplier — a faithful port of the proven f64 `fp_mul`:
/// pre-normalize each significand (leading 1 to bit 23), multiply (24×24=48-bit, leading at 46/47), normalize
/// the leading 1 to bit 47, round-to-nearest-ties-even, and pack (overflow→Inf, underflow→subnormal, rounded
/// once from the EXACT product to avoid double-rounding). Proven bit-exact to Rust `f32` under iverilog.
pub const FP32_MUL_V: &str = r#"module fp32_mul(input [31:0] a, input [31:0] b, output reg [31:0] y);
  localparam [31:0] QNAN = 32'h7FC00000;
  reg sa, sb, rs;
  reg [7:0] ea, eb;
  reg [22:0] ma, mb;
  reg azero,bzero,ainf,binf,anan,bnan,ha,hb;
  reg [23:0] siga, sigb, sa_sh, sb_sh;
  reg [4:0] lza, lzb;
  reg [47:0] prod, norm;
  reg signed [11:0] exp_sum, e, fe, bexp;
  reg [26:0] mant;
  reg g, r, s, l, up;
  reg [24:0] rnd, rr;
  reg [23:0] fsig;
  reg [11:0] rsh;
  reg [63:0] wide;
  reg g2, r2, s2, l2, up2;

  function [4:0] clz24(input [23:0] v);
    integer j; reg dn;
    begin clz24 = 5'd24; dn = 1'b0;
      for (j = 23; j >= 0; j = j - 1) if (v[j] && !dn) begin clz24 = 5'd23 - j[4:0]; dn = 1'b1; end
    end
  endfunction

  always @(*) begin
    sa = a[31]; sb = b[31]; rs = sa ^ sb;
    ea = a[30:23]; eb = b[30:23];
    ma = a[22:0];  mb = b[22:0];
    azero=(ea==8'd0)&&(ma==23'd0); bzero=(eb==8'd0)&&(mb==23'd0);
    ainf =(ea==8'hFF)&&(ma==23'd0); binf =(eb==8'hFF)&&(mb==23'd0);
    anan =(ea==8'hFF)&&(ma!=23'd0); bnan =(eb==8'hFF)&&(mb!=23'd0);
    ha=(ea!=8'd0); hb=(eb!=8'd0);
    siga={ha,ma}; sigb={hb,mb};
    y=32'd0;
    lza=5'd0; lzb=5'd0; sa_sh=24'd0; sb_sh=24'd0; prod=48'd0; norm=48'd0;
    exp_sum=12'sd0; e=12'sd0; fe=12'sd0; bexp=12'sd0; mant=27'd0; rnd=25'd0; rr=25'd0;
    fsig=24'd0; rsh=12'd0; wide=64'd0; g=0;r=0;s=0;l=0;up=0;g2=0;r2=0;s2=0;l2=0;up2=0;

    if (anan || bnan) y = QNAN;
    else if ((ainf && bzero) || (binf && azero)) y = QNAN;
    else if (ainf || binf) y = {rs, 8'hFF, 23'd0};
    else if (azero || bzero) y = {rs, 31'd0};
    else begin
      lza = clz24(siga); lzb = clz24(sigb);
      exp_sum = (ea!=8'd0 ? $signed({4'd0,ea})-12'sd127 : -12'sd126)
              + (eb!=8'd0 ? $signed({4'd0,eb})-12'sd127 : -12'sd126)
              - $signed({7'd0,lza}) - $signed({7'd0,lzb});
      sa_sh = (siga << lza); sb_sh = (sigb << lzb);
      prod = sa_sh * sb_sh;
      if (prod[47]) begin norm = prod; e = exp_sum + 12'sd1; end
      else          begin norm = prod << 1; e = exp_sum; end

      if (e < -12'sd126) begin
        wide = ({16'd0, norm}) >> 12'd21;
        if (|(({16'd0, norm}) & ((64'd1 << 12'd21) - 64'd1))) wide[0] = 1'b1;
        rsh = (-12'sd126) - e;
        if (rsh >= 12'd64) begin s2 = |wide; wide = 64'd0; wide[0] = s2; end
        else if (rsh != 12'd0) begin
          if (|(wide & ((64'd1 << rsh) - 64'd1))) begin wide = wide >> rsh; wide[0] = 1'b1; end
          else wide = wide >> rsh;
        end
        g2=wide[2]; r2=wide[1]; s2=wide[0]; l2=wide[3];
        up2 = g2 & (r2 | s2 | l2);
        rr = {1'b0, wide[26:3]} + (up2 ? 25'd1 : 25'd0);
        if (rr[23]) y = {rs, 8'd1, rr[22:0]};
        else        y = {rs, 8'd0, rr[22:0]};
      end else begin
        mant = {norm[47:24], norm[23], norm[22], (|norm[21:0])};
        g=mant[2]; r=mant[1]; s=mant[0]; l=mant[3];
        up = g & (r | s | l);
        rnd = {1'b0, mant[26:3]} + (up ? 25'd1 : 25'd0);
        if (rnd[24]) begin fsig = rnd[24:1]; fe = e + 12'sd1; end
        else         begin fsig = rnd[23:0]; fe = e; end
        if (fe > 12'sd127) y = {rs, 8'hFF, 23'd0};
        else if (fe < -12'sd126) y = {rs, 8'd1, fsig[22:0]};
        else begin bexp = fe + 12'sd127; y = {rs, bexp[7:0], fsig[22:0]}; end
      end
    end
  end
endmodule
"#;

/// A self-checking combinational testbench for the scalar `fp32_mul`: drive `n` `(a,b)` vectors, compare `y`
/// against Rust's `f32` multiply oracle in `fp32_mul_o.hex`.
pub fn emit_fp32_scalar_mul_testbench(n: usize) -> String {
    format!(
        r#"`timescale 1ns/1ps
module tb_fp32_mul;
  reg [31:0] a, b; wire [31:0] y;
  reg [31:0] ia [0:{last}]; reg [31:0] ib [0:{last}]; reg [31:0] oo [0:{last}];
  integer i, errors = 0;
  fp32_mul dut(.a(a), .b(b), .y(y));
  initial begin
    $readmemh("fp32_mul_a.hex", ia);
    $readmemh("fp32_mul_b.hex", ib);
    $readmemh("fp32_mul_o.hex", oo);
    for (i = 0; i < {n}; i = i + 1) begin
      a = ia[i]; b = ib[i]; #1;
      if (y !== oo[i]) begin errors = errors + 1; if (errors <= 20) $display("FAIL %0d: a=%h b=%h got=%h exp=%h", i, a, b, y, oo[i]); end
    end
    if (errors == 0) $display("PASS: {n} fp32 muls bit-exact to Rust f32");
    else $display("FAIL: %0d fp32 mul mismatches", errors);
    $finish;
  end
endmodule
"#,
        last = n - 1,
    )
}

/// The synthesizable combinational IEEE-754 `f16` (binary16) adder — the f32 adder narrowed to 5-bit exponent,
/// 10-bit mantissa, bias 15 (window 128-bit, LSB0=64, leading-ref=74, normalize-target=13). Proven bit-exact
/// to the [`f16_add`] reference under iverilog; instantiated once per lane by the packed F16×4 tile.
pub const FP16_ADD_V: &str = r#"module fp16_add(input [15:0] a, input [15:0] b_in, input sub, output reg [15:0] y);
  localparam [15:0] QNAN = 16'h7E00;
  reg [15:0] b;
  reg sa, sb;
  reg [4:0] ea, eb;
  reg [9:0] ma, mb;
  reg azero,bzero,ainf,binf,anan,bnan,asub,bsub;
  reg [5:0] xa, xb;
  reg ha, hb;
  reg [10:0] siga, sigb, sig_big, sig_small, final_sig;
  reg [11:0] exp_l, shamt, lead;
  reg sgn_big, sgn_small, res_sign, op_is_sub;
  reg [127:0] big_ext, small_ext, sum_ext, norm_sig;
  integer i;
  reg signed [11:0] norm_exp;
  reg [11:0] rounded;
  reg guard, roundb, sticky, lsb, round_up;

  function [15:0] pack_sub16(input sgn, input [127:0] wsum, input [11:0] wlead, input signed [11:0] bexp);
    reg [11:0] rshift; reg [127:0] win; reg g2,r2,s2,l2,up2; reg [11:0] rr;
    begin
      rshift = 12'd1 - bexp;
      if (wlead >= 12'd13) begin
        win = wsum >> (wlead - 12'd13);
        if (|(wsum & ((128'd1 << (wlead - 12'd13)) - 128'd1))) win[0] = 1'b1;
      end else begin
        win = wsum << (12'd13 - wlead);
      end
      if (rshift >= 12'd128) begin s2 = |win; win = 128'd0; win[0] = s2; end
      else if (rshift != 12'd0) begin
        if (|(win & ((128'd1 << rshift) - 128'd1))) begin win = win >> rshift; win[0] = 1'b1; end
        else win = win >> rshift;
      end
      g2 = win[2]; r2 = win[1]; s2 = win[0]; l2 = win[3];
      up2 = g2 & (r2 | s2 | l2);
      rr = {1'b0, win[13:3]} + (up2 ? 12'd1 : 12'd0);
      if (rr[10]) pack_sub16 = {sgn, 5'd1, rr[9:0]};
      else        pack_sub16 = {sgn, 5'd0, rr[9:0]};
    end
  endfunction

  always @(*) begin
    b  = sub ? {~b_in[15], b_in[14:0]} : b_in;
    sa = a[15]; sb = b[15];
    ea = a[14:10]; eb = b[14:10];
    ma = a[9:0];   mb = b[9:0];
    azero = (ea==5'd0)  && (ma==10'd0); bzero = (eb==5'd0)  && (mb==10'd0);
    ainf  = (ea==5'h1F) && (ma==10'd0); binf  = (eb==5'h1F) && (mb==10'd0);
    anan  = (ea==5'h1F) && (ma!=10'd0); bnan  = (eb==5'h1F) && (mb!=10'd0);
    asub  = (ea==5'd0)  && (ma!=10'd0); bsub  = (eb==5'd0)  && (mb!=10'd0);
    xa = asub ? 6'd1 : {1'b0, ea};
    xb = bsub ? 6'd1 : {1'b0, eb};
    ha = (ea!=5'd0); hb = (eb!=5'd0);
    siga = {ha, ma}; sigb = {hb, mb};
    y = 16'd0;
    exp_l=12'd0; shamt=12'd0; sig_big=11'd0; sig_small=11'd0; sgn_big=1'b0; sgn_small=1'b0;
    op_is_sub=1'b0; res_sign=1'b0; big_ext=128'd0; small_ext=128'd0; sum_ext=128'd0; lead=12'd0;
    norm_exp=12'sd0; norm_sig=128'd0; rounded=12'd0; final_sig=11'd0;
    guard=1'b0; roundb=1'b0; sticky=1'b0; lsb=1'b0; round_up=1'b0;

    if (anan || bnan) y = QNAN;
    else if (ainf && binf) y = (sa==sb) ? {sa,5'h1F,10'd0} : QNAN;
    else if (ainf) y = {sa,5'h1F,10'd0};
    else if (binf) y = {sb,5'h1F,10'd0};
    else if (azero && bzero) y = (sa==sb) ? {sa,15'd0} : 16'd0;
    else if (azero) y = b;
    else if (bzero) y = a;
    else begin
      if (xa > xb || (xa == xb && siga >= sigb)) begin
        exp_l={6'd0,xa}; sig_big=siga; sgn_big=sa; sig_small=sigb; sgn_small=sb; shamt={6'd0,(xa-xb)};
      end else begin
        exp_l={6'd0,xb}; sig_big=sigb; sgn_big=sb; sig_small=siga; sgn_small=sa; shamt={6'd0,(xb-xa)};
      end
      op_is_sub = (sgn_big != sgn_small);
      res_sign  = sgn_big;
      big_ext = {53'd0, sig_big, 64'd0};
      if (shamt >= 12'd128) begin small_ext = 128'd0; small_ext[0] = |sig_small; end
      else begin
        small_ext = ({53'd0, sig_small, 64'd0}) >> shamt;
        if (shamt != 12'd0 && |(({53'd0, sig_small, 64'd0}) & ((128'd1 << shamt) - 128'd1))) small_ext[0] = 1'b1;
      end
      if (op_is_sub) sum_ext = big_ext - small_ext; else sum_ext = big_ext + small_ext;
      if (sum_ext == 128'd0) y = 16'd0;
      else begin
        lead = 12'd0;
        for (i = 127; i >= 0; i = i - 1) if (sum_ext[i] && lead == 12'd0) lead = i[11:0];
        norm_exp = $signed({6'd0, exp_l[5:0]}) + $signed({3'd0, lead[8:0]}) - 12'sd74;
        if (lead >= 12'd13) begin
          norm_sig = sum_ext >> (lead - 12'd13);
          if (|(sum_ext & ((128'd1 << (lead - 12'd13)) - 128'd1))) norm_sig[0] = 1'b1;
        end else norm_sig = sum_ext << (12'd13 - lead);
        guard=norm_sig[2]; roundb=norm_sig[1]; sticky=norm_sig[0]; lsb=norm_sig[3];
        round_up = guard & (roundb | sticky | lsb);
        rounded = {1'b0, norm_sig[13:3]} + (round_up ? 12'd1 : 12'd0);
        if (rounded[11]) begin final_sig = rounded[11:1]; norm_exp = norm_exp + 12'sd1; end
        else final_sig = rounded[10:0];
        if (norm_exp >= 12'sd31) y = {res_sign, 5'h1F, 10'd0};
        else if (norm_exp <= 12'sd0) y = pack_sub16(res_sign, sum_ext, lead, norm_exp);
        else y = {res_sign, norm_exp[4:0], final_sig[9:0]};
      end
    end
  end
endmodule
"#;

/// The synthesizable combinational IEEE-754 `f16` multiplier — the f32 multiplier narrowed to binary16
/// (11-bit significand product = 22 bits, leading at 20/21). Proven bit-exact to [`f16_mul`] under iverilog.
pub const FP16_MUL_V: &str = r#"module fp16_mul(input [15:0] a, input [15:0] b, output reg [15:0] y);
  localparam [15:0] QNAN = 16'h7E00;
  reg sa, sb, rs;
  reg [4:0] ea, eb;
  reg [9:0] ma, mb;
  reg azero,bzero,ainf,binf,anan,bnan,ha,hb;
  reg [10:0] siga, sigb, sa_sh, sb_sh, fsig;
  reg [4:0] lza, lzb;
  reg [21:0] prod, norm;
  reg signed [11:0] exp_sum, e, fe, bexp;
  reg [13:0] mant;
  reg g, r, s, l, up;
  reg [11:0] rnd, rr, rsh;
  reg [63:0] wide;
  reg g2, r2, s2, l2, up2;

  function [4:0] clz11(input [10:0] v);
    integer j; reg dn;
    begin clz11 = 5'd11; dn = 1'b0;
      for (j = 10; j >= 0; j = j - 1) if (v[j] && !dn) begin clz11 = 5'd10 - j[4:0]; dn = 1'b1; end
    end
  endfunction

  always @(*) begin
    sa = a[15]; sb = b[15]; rs = sa ^ sb;
    ea = a[14:10]; eb = b[14:10];
    ma = a[9:0];   mb = b[9:0];
    azero=(ea==5'd0)&&(ma==10'd0); bzero=(eb==5'd0)&&(mb==10'd0);
    ainf =(ea==5'h1F)&&(ma==10'd0); binf =(eb==5'h1F)&&(mb==10'd0);
    anan =(ea==5'h1F)&&(ma!=10'd0); bnan =(eb==5'h1F)&&(mb!=10'd0);
    ha=(ea!=5'd0); hb=(eb!=5'd0);
    siga={ha,ma}; sigb={hb,mb};
    y=16'd0;
    lza=5'd0; lzb=5'd0; sa_sh=11'd0; sb_sh=11'd0; prod=22'd0; norm=22'd0;
    exp_sum=12'sd0; e=12'sd0; fe=12'sd0; bexp=12'sd0; mant=14'd0; rnd=12'd0; rr=12'd0;
    fsig=11'd0; rsh=12'd0; wide=64'd0; g=0;r=0;s=0;l=0;up=0;g2=0;r2=0;s2=0;l2=0;up2=0;

    if (anan || bnan) y = QNAN;
    else if ((ainf && bzero) || (binf && azero)) y = QNAN;
    else if (ainf || binf) y = {rs, 5'h1F, 10'd0};
    else if (azero || bzero) y = {rs, 15'd0};
    else begin
      lza = clz11(siga); lzb = clz11(sigb);
      exp_sum = (ea!=5'd0 ? $signed({7'd0,ea})-12'sd15 : -12'sd14)
              + (eb!=5'd0 ? $signed({7'd0,eb})-12'sd15 : -12'sd14)
              - $signed({7'd0,lza}) - $signed({7'd0,lzb});
      sa_sh = (siga << lza); sb_sh = (sigb << lzb);
      prod = sa_sh * sb_sh;
      if (prod[21]) begin norm = prod; e = exp_sum + 12'sd1; end
      else          begin norm = prod << 1; e = exp_sum; end

      if (e < -12'sd14) begin
        wide = ({42'd0, norm}) >> 12'd8;
        if (|(({42'd0, norm}) & ((64'd1 << 12'd8) - 64'd1))) wide[0] = 1'b1;
        rsh = (-12'sd14) - e;
        if (rsh >= 12'd64) begin s2 = |wide; wide = 64'd0; wide[0] = s2; end
        else if (rsh != 12'd0) begin
          if (|(wide & ((64'd1 << rsh) - 64'd1))) begin wide = wide >> rsh; wide[0] = 1'b1; end
          else wide = wide >> rsh;
        end
        g2=wide[2]; r2=wide[1]; s2=wide[0]; l2=wide[3];
        up2 = g2 & (r2 | s2 | l2);
        rr = {1'b0, wide[13:3]} + (up2 ? 12'd1 : 12'd0);
        if (rr[10]) y = {rs, 5'd1, rr[9:0]};
        else        y = {rs, 5'd0, rr[9:0]};
      end else begin
        mant = {norm[21:11], norm[10], norm[9], (|norm[8:0])};
        g=mant[2]; r=mant[1]; s=mant[0]; l=mant[3];
        up = g & (r | s | l);
        rnd = {1'b0, mant[13:3]} + (up ? 12'd1 : 12'd0);
        if (rnd[11]) begin fsig = rnd[11:1]; fe = e + 12'sd1; end
        else         begin fsig = rnd[10:0]; fe = e; end
        if (fe > 12'sd15) y = {rs, 5'h1F, 10'd0};
        else if (fe < -12'sd14) y = {rs, 5'd1, fsig[9:0]};
        else begin bexp = fe + 12'sd15; y = {rs, bexp[4:0], fsig[9:0]}; end
      end
    end
  end
endmodule
"#;

/// Emit the packed **F16×4 add** tile: four `fp16_add` lanes over the 64-bit operands. Module `simd_fadd_f16x4`
/// (ports `a,b,r`). Compile alongside [`FP16_ADD_V`]; proven ≡ [`eval_lanewise_fadd_f16x4`] under iverilog.
pub fn emit_simd_fadd_f16x4_verilog() -> String {
    let mut m =
        String::from("module simd_fadd_f16x4(input [63:0] a, input [63:0] b, output [63:0] r);\n");
    for k in 0..4 {
        let (lo, hi) = (k * 16, k * 16 + 15);
        m.push_str(&format!(
            "  wire [15:0] y{k};\n  fp16_add u{k}(.a(a[{hi}:{lo}]), .b_in(b[{hi}:{lo}]), .sub(1'b0), .y(y{k}));\n"
        ));
    }
    m.push_str("  assign r = {y3, y2, y1, y0};\nendmodule\n");
    m
}

/// Emit the packed **F16×4 multiply** tile: four `fp16_mul` lanes. Module `simd_fmul_f16x4`. Compile alongside
/// [`FP16_MUL_V`]; proven ≡ [`eval_lanewise_fmul_f16x4`] under iverilog.
pub fn emit_simd_fmul_f16x4_verilog() -> String {
    let mut m =
        String::from("module simd_fmul_f16x4(input [63:0] a, input [63:0] b, output [63:0] r);\n");
    for k in 0..4 {
        let (lo, hi) = (k * 16, k * 16 + 15);
        m.push_str(&format!(
            "  wire [15:0] y{k};\n  fp16_mul u{k}(.a(a[{hi}:{lo}]), .b(b[{hi}:{lo}]), .y(y{k}));\n"
        ));
    }
    m.push_str("  assign r = {y3, y2, y1, y0};\nendmodule\n");
    m
}

/// A self-checking combinational testbench for the scalar `fp32_add`: drive `n` `(a,b)` 32-bit vectors from
/// `fp32_add_a.hex`/`_b.hex`, compare the settled `y` against Rust's `f32` add oracle in `fp32_add_o.hex`.
pub fn emit_fp32_scalar_add_testbench(n: usize) -> String {
    format!(
        r#"`timescale 1ns/1ps
module tb_fp32_add;
  reg [31:0] a, b; wire [31:0] y;
  reg [31:0] ia [0:{last}]; reg [31:0] ib [0:{last}]; reg [31:0] oo [0:{last}];
  integer i, errors = 0;
  fp32_add dut(.a(a), .b_in(b), .sub(1'b0), .y(y));
  initial begin
    $readmemh("fp32_add_a.hex", ia);
    $readmemh("fp32_add_b.hex", ib);
    $readmemh("fp32_add_o.hex", oo);
    for (i = 0; i < {n}; i = i + 1) begin
      a = ia[i]; b = ib[i]; #1;
      if (y !== oo[i]) begin errors = errors + 1; if (errors <= 20) $display("FAIL %0d: a=%h b=%h got=%h exp=%h", i, a, b, y, oo[i]); end
    end
    if (errors == 0) $display("PASS: {n} fp32 adds bit-exact to Rust f32");
    else $display("FAIL: %0d fp32 add mismatches", errors);
    $finish;
  end
endmodule
"#,
        last = n - 1,
    )
}

/// Emit the packed SWAR add as a COMBINATIONAL Verilog module — the SIMD tile the RTL flow tapes out:
/// `r = ((a & M) + (b & M)) ^ ((a ^ b) & ~M)`, one datapath op, carry blocked at every lane boundary.
/// Proven ≡ the lane-wise spec under iverilog by `simd_rtl_lock` (and ≡ N scalar adds by SAT in `simd_tile_lock`).
pub fn emit_simd_add_verilog(name: &str, ty: LaneTy) -> String {
    let m = swar_lo_mask(ty);
    let hm = !m;
    format!(
        "// simd_{name} — packed {lanes}×{width}-bit SWAR add (carry-isolated, combinational, 1 clock).\n\
         module simd_{name}(input [63:0] a, input [63:0] b, output [63:0] r);\n\
         \x20 assign r = ((a & 64'h{m:016x}) + (b & 64'h{m:016x})) ^ ((a ^ b) & 64'h{hm:016x});\n\
         endmodule\n",
        lanes = ty.lanes(),
        width = ty.width(),
    )
}

/// A self-checking combinational testbench for `simd_{name}`: drive `n` `(a,b)` vectors from
/// `simd_{name}_a.hex`/`_b.hex`, compare the settled `r` against the lane-wise oracle in `_o.hex`, print PASS.
pub fn emit_simd_add_testbench(name: &str, n: usize) -> String {
    format!(
        r#"`timescale 1ns/1ps
module tb_simd_{name};
  reg [63:0] a, b; wire [63:0] r;
  reg [63:0] ia [0:{last}]; reg [63:0] ib [0:{last}]; reg [63:0] oo [0:{last}];
  integer i, errors = 0;
  simd_{name} dut(.a(a), .b(b), .r(r));
  initial begin
    $readmemh("simd_{name}_a.hex", ia);
    $readmemh("simd_{name}_b.hex", ib);
    $readmemh("simd_{name}_o.hex", oo);
    for (i = 0; i < {n}; i = i + 1) begin
      a = ia[i]; b = ib[i]; #1;
      if (r !== oo[i]) begin errors = errors + 1; $display("FAIL vec %0d: got %h exp %h", i, r, oo[i]); end
    end
    if (errors == 0) $display("PASS: {n} SIMD adds match the lane-wise spec");
    else $display("FAIL: %0d SIMD mismatches", errors);
    $finish;
  end
endmodule
"#,
        last = n - 1,
    )
}

/// Emit the packed **F32×2 add** tile: two `fp32_add` lanes (low = bits `[31:0]`, high = `[63:32]`) driven in
/// parallel from the 64-bit operands. Module name `simd_fadd_f32x2` (ports `a,b,r`), so the standard SIMD
/// testbench drives it. Compile alongside [`FP32_ADD_V`]; proven ≡ [`eval_lanewise_fadd_f32x2`] under iverilog.
pub fn emit_simd_fadd_f32x2_verilog() -> String {
    "module simd_fadd_f32x2(input [63:0] a, input [63:0] b, output [63:0] r);\n\
     \x20 wire [31:0] lo, hi;\n\
     \x20 fp32_add u0(.a(a[31:0]),  .b_in(b[31:0]),  .sub(1'b0), .y(lo));\n\
     \x20 fp32_add u1(.a(a[63:32]), .b_in(b[63:32]), .sub(1'b0), .y(hi));\n\
     \x20 assign r = {hi, lo};\n\
     endmodule\n"
        .to_string()
}

/// Emit the packed **F32×2 multiply** tile: two `fp32_mul` lanes in parallel. Module name `simd_fmul_f32x2`.
/// Compile alongside [`FP32_MUL_V`]; proven ≡ [`eval_lanewise_fmul_f32x2`] under iverilog.
pub fn emit_simd_fmul_f32x2_verilog() -> String {
    "module simd_fmul_f32x2(input [63:0] a, input [63:0] b, output [63:0] r);\n\
     \x20 wire [31:0] lo, hi;\n\
     \x20 fp32_mul u0(.a(a[31:0]),  .b(b[31:0]),  .y(lo));\n\
     \x20 fp32_mul u1(.a(a[63:32]), .b(b[63:32]), .y(hi));\n\
     \x20 assign r = {hi, lo};\n\
     endmodule\n"
        .to_string()
}

/// Widen an IEEE-754 binary16 bit pattern to `f32` — EXACT (every f16 value is representable in f32).
pub fn f16_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0f32 } else { 1.0 };
    let exp = ((bits >> 10) & 0x1F) as i32;
    let mant = (bits & 0x3FF) as f32;
    if exp == 0x1F {
        if mant == 0.0 {
            return sign * f32::INFINITY;
        }
        return f32::from_bits(0x7FC0_0000); // canonical qNaN
    }
    if exp == 0 {
        // subnormal (or ±0): value = mant * 2^-24
        return sign * mant * (-24f32).exp2();
    }
    sign * (1.0 + mant / 1024.0) * ((exp - 15) as f32).exp2()
}

/// Round an `f32` to an IEEE-754 binary16 bit pattern, round-to-nearest-ties-even (overflow→Inf,
/// underflow→subnormal/0, NaN→canonical qNaN). The one rounding step in the f16 reference; anchored by
/// known-value assertions in `simd_tile_lock`.
pub fn f32_to_f16(x: f32) -> u16 {
    let bits = x.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xFF) as i32;
    let mant = bits & 0x7F_FFFF; // 23-bit
    if exp == 0xFF {
        return if mant != 0 {
            sign | 0x7E00
        } else {
            sign | 0x7C00
        };
    }
    let e = exp - 127; // unbiased
    if e > 15 {
        return sign | 0x7C00; // overflow → Inf
    }
    if e < -14 {
        // subnormal / underflow: value = full * 2^(e-23), rendered as frac * 2^-24 ⇒ frac = full * 2^(e+1)
        if e < -25 {
            return sign; // strictly less than half the smallest subnormal → ±0
        }
        let full = (1u32 << 23) | mant; // 24-bit significand
        let shift = (-e - 1) as u32; // 14..=24
        let frac = full >> shift;
        let rem = full & ((1u32 << shift) - 1);
        let half = 1u32 << (shift - 1);
        let mut f = frac;
        if rem > half || (rem == half && (frac & 1) == 1) {
            f += 1; // RNE
        }
        if f >= 1024 {
            return sign | 0x0400; // rounded up to the smallest normal
        }
        return sign | f as u16;
    }
    // normal: round the 23-bit mantissa to 10 bits (RNE)
    let frac = mant >> 13;
    let rem = mant & 0x1FFF;
    let half = 0x1000;
    let mut f = frac;
    if rem > half || (rem == half && (frac & 1) == 1) {
        f += 1;
    }
    let mut exp_out = (e + 15) as u16; // 1..=30
    if f >= 1024 {
        f = 0;
        exp_out += 1;
    }
    if exp_out >= 31 {
        return sign | 0x7C00; // overflow → Inf
    }
    sign | (exp_out << 10) | f as u16
}

/// One correctly-rounded IEEE-754 binary16 add: widen (exact), add in f32 (exact — the sum of two f16s fits in
/// f32's 24-bit significand), round back RNE. Equals the native binary16 add bit-for-bit (both correctly rounded).
pub fn f16_add(a: u16, b: u16) -> u16 {
    f32_to_f16(f16_to_f32(a) + f16_to_f32(b))
}

/// One correctly-rounded IEEE-754 binary16 multiply (see [`f16_add`]; the product of two f16s is exact in f32).
pub fn f16_mul(a: u16, b: u16) -> u16 {
    f32_to_f16(f16_to_f32(a) * f16_to_f32(b))
}

/// Pack four f16 lanes into a 64-bit word (lane 0 low).
pub fn pack_f16x4(l0: u16, l1: u16, l2: u16, l3: u16) -> u64 {
    (l0 as u64) | ((l1 as u64) << 16) | ((l2 as u64) << 32) | ((l3 as u64) << 48)
}

/// Packed-float add for **F16×4** — the SPEC: four independent scalar binary16 adds. Lanes are isolated (a
/// NaN/Inf/subnormal in one cannot perturb another); the parallel `fp16` FPU lanes realize it in silicon.
pub fn eval_lanewise_fadd_f16x4(a: u64, b: u64) -> u64 {
    let lane = |sh: u32| f16_add((a >> sh) as u16, (b >> sh) as u16);
    pack_f16x4(lane(0), lane(16), lane(32), lane(48))
}

/// Packed-float multiply for **F16×4** (see [`eval_lanewise_fadd_f16x4`]).
pub fn eval_lanewise_fmul_f16x4(a: u64, b: u64) -> u64 {
    let lane = |sh: u32| f16_mul((a >> sh) as u16, (b >> sh) as u16);
    pack_f16x4(lane(0), lane(16), lane(32), lane(48))
}

/// Evaluate the lane-wise add SPEC concretely (each lane an independent `width`-bit wrapping add).
pub fn eval_lanewise_add(ty: LaneTy, a: u64, b: u64) -> u64 {
    let w = ty.width();
    let full = if w == 64 { u64::MAX } else { (1u64 << w) - 1 };
    let mut out = 0u64;
    for j in 0..ty.lanes() {
        let sh = j * w;
        let aj = (a >> sh) & full;
        let bj = (b >> sh) & full;
        out |= ((aj.wrapping_add(bj)) & full) << sh;
    }
    out
}
