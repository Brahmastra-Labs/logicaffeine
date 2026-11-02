//! Binary32 multiply and add under a family of semantics: the hypotheses a hard floating-point block is told apart by.
//!
//! IEEE 754 fixes round-to-nearest-even and leaves an implementation the rest: whether a denormal operand is read as
//! itself or as a zero, whether a result below the normals is kept as a denormal or flushed to zero, where tininess
//! is decided (on the exact result, after rounding to the normals' precision with the exponent unbounded, or, as the
//! engine's numerics do, after rounding at the denormals' precision), the sign a flushed zero carries, and the NaN an
//! operation returns. A [`Semantics`] names one choice of each; [`fmul_verilog`] and [`fadd_verilog`] print the
//! combinational unit computing it, every member of [`family`] from one generator, so two members differ exactly
//! where their semantics do, and [`Semantics::mul`] and [`Semantics::add`] compute it in software, bit for bit what
//! its unit computes. [`Semantics::ENGINE`] is the engine's own arithmetic
//! ([`crate::spatial::decode_engine::numerics`]); [`Semantics::IEEE`] is IEEE 754 with gradual underflow and the
//! payload-propagating NaNs of an x86 core. [`logicaffeine_proof::sweep::distinguish`] tells the family apart from
//! the pairs of [`SPECIALS`]: the members that compute one function, proven, and operand pairs on which every two
//! that differ give different results, the vectors a hard block is measured on to say which member it is.

use std::fmt::Write;

/// How a denormal is read or written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Denormals {
    /// Kept as a denormal.
    Keep,
    /// Read or written as a zero.
    Flush,
}

/// Where a result below the least normal is decided to be one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tininess {
    /// On the exact result: anything below `2^-126` flushes, even what would round up to it.
    BeforeRounding,
    /// After rounding the significand at the denormals' precision: only a result that rounds up into the least
    /// normal survives (the engine's numerics).
    AfterRoundingAtDenormals,
    /// After rounding to 24 bits with the exponent unbounded: what rounds up to `2^-126` survives as the least normal.
    AfterRoundingUnbounded,
}

/// The sign of a result flushed to zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FlushSign {
    /// The sign the result would have had.
    Kept,
    /// Always `+0`.
    Positive,
}

/// The NaN an operation returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Nan {
    /// One encoding for every NaN result.
    Canonical(u32),
    /// One encoding's exponent and significand, its sign the exclusive or of the operands' signs, as the D5005's
    /// floating-point DSP block multiplies.
    XorSigned(u32),
    /// The first NaN operand's payload, quieted; `invalid` for an invalid operation of non-NaN operands.
    Propagate { invalid: u32 },
}

/// One choice of everything IEEE 754 leaves to an implementation of binary32 multiply and add.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Semantics {
    pub denormal_inputs: Denormals,
    pub denormal_results: Denormals,
    /// Read only when results flush.
    pub tininess: Tininess,
    /// Read only when results flush.
    pub flushed_sign: FlushSign,
    pub nan: Nan,
}

impl Semantics {
    /// The engine's arithmetic: denormal operands read as zeros of their sign, a result rounded at the denormals'
    /// precision and flushed unless it reaches the least normal, every NaN `0x7fc00000`.
    pub const ENGINE: Semantics = Semantics {
        denormal_inputs: Denormals::Flush,
        denormal_results: Denormals::Flush,
        tininess: Tininess::AfterRoundingAtDenormals,
        flushed_sign: FlushSign::Kept,
        nan: Nan::Canonical(0x7fc0_0000),
    };

    /// IEEE 754 with gradual underflow and an x86 core's NaNs: the first NaN operand's payload quieted, the invalid
    /// operation's `0xffc00000`.
    pub const IEEE: Semantics = Semantics {
        denormal_inputs: Denormals::Keep,
        denormal_results: Denormals::Keep,
        tininess: Tininess::AfterRoundingAtDenormals,
        flushed_sign: FlushSign::Kept,
        nan: Nan::Propagate { invalid: 0xffc0_0000 },
    };

    /// A short name: `din-keep.dout-flush.tiny-before.sign-pos.nan-7fc00000`.
    pub fn name(&self) -> String {
        let d = |x: Denormals| match x {
            Denormals::Keep => "keep",
            Denormals::Flush => "flush",
        };
        let mut n = format!("din-{}.dout-{}", d(self.denormal_inputs), d(self.denormal_results));
        if self.denormal_results == Denormals::Flush {
            n.push_str(match self.tininess {
                Tininess::BeforeRounding => ".tiny-before",
                Tininess::AfterRoundingAtDenormals => ".tiny-after-denormals",
                Tininess::AfterRoundingUnbounded => ".tiny-after-unbounded",
            });
            n.push_str(match self.flushed_sign {
                FlushSign::Kept => ".sign-kept",
                FlushSign::Positive => ".sign-pos",
            });
        }
        match self.nan {
            Nan::Canonical(v) => write!(n, ".nan-{v:08x}").unwrap(),
            Nan::XorSigned(v) => write!(n, ".nan-xor-{:08x}", v & 0x7fff_ffff).unwrap(),
            Nan::Propagate { invalid } => write!(n, ".nan-prop-{invalid:08x}").unwrap(),
        }
        n
    }

    /// `a × b` under these semantics, bit for bit what [`fmul_verilog`]'s unit computes.
    pub fn mul(&self, a: u32, b: u32) -> u32 {
        let (x, y) = (self.operand(a), self.operand(b));
        let s = x.sign ^ y.sign;
        if x.nan || y.nan || (x.inf && y.zero) || (y.inf && x.zero) {
            return self.nan_result(a, b);
        }
        if x.inf || y.inf {
            return s << 31 | 0x7f80_0000;
        }
        if x.zero || y.zero {
            return s << 31;
        }
        let p = u128::from(x.significand) * u128::from(y.significand);
        let lz = match self.denormal_inputs {
            Denormals::Flush => u32::from(p >> 47 & 1 == 0),
            Denormals::Keep if x.denormal || y.denormal => if p == 0 { 48 } else { 47 - (127 - p.leading_zeros()) },
            Denormals::Keep => u32::from(p >> 47 & 1 == 0),
        };
        let pn = (p << lz) & ((1u128 << 48) - 1);
        let e = i32::from(x.exponent) + i32::from(y.exponent) - 126 - lz as i32;
        self.assemble(s, pn, 48, e)
    }

    /// `a + b` under these semantics, bit for bit what [`fadd_verilog`]'s unit computes.
    pub fn add(&self, a: u32, b: u32) -> u32 {
        let (x, y) = (self.operand(a), self.operand(b));
        if x.nan || y.nan || (x.inf && y.inf && x.sign != y.sign) {
            return self.nan_result(a, b);
        }
        if x.inf {
            return x.sign << 31 | 0x7f80_0000;
        }
        if y.inf {
            return y.sign << 31 | 0x7f80_0000;
        }
        if x.zero && y.zero {
            return (x.sign & y.sign) << 31;
        }
        let pass = |v: u32, sign: u32| match (self.denormal_inputs, self.denormal_results) {
            (Denormals::Keep, Denormals::Flush) if v >> 23 & 0xff == 0 => self.flushed(sign),
            _ => v,
        };
        if x.zero {
            return pass(b, y.sign);
        }
        if y.zero {
            return pass(a, x.sign);
        }
        let swap = a & 0x7fff_ffff < b & 0x7fff_ffff;
        let (big, small) = if swap { (y, x) } else { (x, y) };
        let f2 = match self.denormal_inputs {
            Denormals::Flush if small.zero => 0,
            _ => small.significand,
        };
        let d = big.exponent.wrapping_sub(small.exponent);
        let aligned = (u64::from(f2) << 27) >> d.min(50);
        let g2 = (aligned >> 25) << 1 | u64::from(aligned & ((1 << 25) - 1) != 0);
        let g1 = u64::from(big.significand) << 3;
        let sum = if big.sign != small.sign { g1.wrapping_sub(g2) & ((1 << 28) - 1) } else { g1 + g2 };
        if sum == 0 {
            return 0;
        }
        let low = sum & ((1 << 27) - 1);
        let lz = if low == 0 { 27 } else { 26 - (63 - low.leading_zeros()) };
        let carry = sum >> 27 & 1 == 1;
        let e = if carry { i32::from(big.exponent) + 1 } else { i32::from(big.exponent) - lz as i32 };
        let pn = if carry { (sum >> 2) << 1 | u64::from(sum & 3 != 0) } else { (low << lz) & ((1 << 27) - 1) };
        self.assemble(big.sign, u128::from(pn), 27, e)
    }

    /// An operand as the datapath reads it: its sign, its class, and (a number) the significand and exponent it is
    /// read with, a flushed denormal a zero, a kept one `0.m × 2^-126`.
    fn operand(&self, v: u32) -> Operand {
        let (sign, exponent, m) = (v >> 31, (v >> 23 & 0xff) as u8, v & 0x7f_ffff);
        let denormal = exponent == 0 && m != 0;
        let zero = match self.denormal_inputs {
            Denormals::Flush => exponent == 0,
            Denormals::Keep => exponent == 0 && m == 0,
        };
        let (significand, exponent) = match self.denormal_inputs {
            Denormals::Keep if denormal => (m, 1),
            _ => (1 << 23 | m, exponent),
        };
        Operand { sign, zero, denormal: denormal && self.denormal_inputs == Denormals::Keep, inf: v & 0x7fff_ffff == 0x7f80_0000, nan: v & 0x7fff_ffff > 0x7f80_0000, significand, exponent }
    }

    /// The NaN an operation of `a` and `b` returns.
    fn nan_result(&self, a: u32, b: u32) -> u32 {
        let nan = |v: u32| v & 0x7fff_ffff > 0x7f80_0000;
        match self.nan {
            Nan::Canonical(v) => v,
            Nan::XorSigned(v) => (a ^ b) & 0x8000_0000 | v & 0x7fff_ffff,
            Nan::Propagate { invalid } if nan(a) => a | 0x40_0000,
            Nan::Propagate { invalid } if nan(b) => b | 0x40_0000,
            Nan::Propagate { invalid } => invalid,
        }
    }

    /// A zero a result is flushed to, under `sign`.
    fn flushed(&self, sign: u32) -> u32 {
        match self.flushed_sign {
            FlushSign::Kept => sign << 31,
            FlushSign::Positive => 0,
        }
    }

    /// The result of `sign` and the significand `pn` normalized with its leading one at bit `width - 1`, its biased
    /// exponent `e`: rounded at the denormals' place to nearest even, overflowing to infinity, and below the least normal
    /// as these semantics underflow.
    fn assemble(&self, sign: u32, pn: u128, width: u32, e: i32) -> u32 {
        let sh = if e < 1 { (1 - e) as u32 } else { 0 }.min(width);
        let wide = (pn << width) >> sh;
        let keep = (wide >> (2 * width - 24)) as u32 & 0xff_ffff;
        let guard = wide >> (2 * width - 25) & 1 == 1;
        let sticky = wide & ((1u128 << (2 * width - 25)) - 1) != 0;
        let r = keep + u32::from(guard && (sticky || keep & 1 == 1));
        let carry = r >> 24 & 1 == 1;
        let ef = if e < 1 { i32::from(r >> 23 & 1 == 1) } else if carry { e + 1 } else { e };
        let mf = if carry { r >> 1 & 0x7f_ffff } else { r & 0x7f_ffff };
        let below = width - 24;
        let cn = (pn >> below) as u32 & 0xff_ffff == 0xff_ffff && pn >> (below - 1) & 1 == 1;
        if ef >= 255 {
            return sign << 31 | 0x7f80_0000;
        }
        match (self.denormal_results, self.tininess) {
            (Denormals::Keep, _) => {}
            (Denormals::Flush, Tininess::AfterRoundingAtDenormals) if ef <= 0 => return self.flushed(sign),
            (Denormals::Flush, Tininess::BeforeRounding) if e < 1 => return self.flushed(sign),
            (Denormals::Flush, Tininess::AfterRoundingUnbounded) if e < 1 => {
                return if e == 0 && cn { sign << 31 | 0x0080_0000 } else { self.flushed(sign) };
            }
            _ => {}
        }
        sign << 31 | (ef as u32 & 0xff) << 23 | mf
    }

    /// The members whose tininess and flushed sign mean nothing (results kept) take the engine's, so no two members
    /// of [`family`] describe one function by construction.
    fn canonical(mut self) -> Semantics {
        if self.denormal_results == Denormals::Keep {
            self.tininess = Tininess::AfterRoundingAtDenormals;
            self.flushed_sign = FlushSign::Kept;
        }
        self
    }
}

/// An operand as a member's datapath reads it.
#[derive(Clone, Copy)]
struct Operand {
    sign: u32,
    zero: bool,
    /// A denormal read as itself (operands kept), `0.m × 2^-126`.
    denormal: bool,
    inf: bool,
    nan: bool,
    significand: u32,
    exponent: u8,
}

/// The operands every pair of which a hard block is first measured on: signed zeros, the denormals' ends, the normals'
/// ends, one, infinities, quiet and signalling NaNs, a mid denormal and one above one.
pub const SPECIALS: [u32; 20] = [
    0x0000_0000, 0x8000_0000, 0x0000_0001, 0x8000_0001, 0x007f_ffff, 0x807f_ffff, 0x0080_0000, 0x8080_0000,
    0x3f80_0000, 0xbf80_0000, 0x7f7f_ffff, 0xff7f_ffff, 0x7f80_0000, 0xff80_0000, 0x7fc0_0000, 0xffc0_0001,
    0x7f80_0001, 0x7fbf_ffff, 0x0040_0000, 0x3400_0001,
];

/// The NaN encodings the family tries: four canonical NaNs, the first NaN operand's payload with either default NaN
/// for an invalid operation (an ARM core's positive one, an x86 core's negative one), and the default quiet NaN signed
/// by the operands' signs' exclusive or (the D5005's floating-point DSP block's multiply).
pub const NANS: [Nan; 7] = [
    Nan::Canonical(0x7fc0_0000),
    Nan::Canonical(0xffc0_0000),
    Nan::Canonical(0x7fff_ffff),
    Nan::Canonical(0xffff_ffff),
    Nan::Propagate { invalid: 0x7fc0_0000 },
    Nan::Propagate { invalid: 0xffc0_0000 },
    Nan::XorSigned(0x7fc0_0000),
];

/// Every member: each choice of operand denormals, result denormals, tininess, flushed sign and NaN, the
/// meaningless combinations folded away; [`Semantics::ENGINE`] first.
pub fn family() -> Vec<Semantics> {
    let mut out = vec![Semantics::ENGINE];
    for denormal_inputs in [Denormals::Flush, Denormals::Keep] {
        for denormal_results in [Denormals::Flush, Denormals::Keep] {
            for tininess in [Tininess::AfterRoundingAtDenormals, Tininess::BeforeRounding, Tininess::AfterRoundingUnbounded] {
                for flushed_sign in [FlushSign::Kept, FlushSign::Positive] {
                    for nan in NANS {
                        let s = Semantics { denormal_inputs, denormal_results, tininess, flushed_sign, nan }.canonical();
                        if !out.contains(&s) {
                            out.push(s);
                        }
                    }
                }
            }
        }
    }
    out
}

/// The operand classes: zero, infinity, NaN, and (for kept denormals) the significand and exponent a denormal is
/// read with.
fn operands(out: &mut String, s: &Semantics) {
    out.push_str(
        "  wire        sa = a[31], sb = b[31];\n\
         \x20 wire [7:0]  ea = a[30:23], eb = b[30:23];\n\
         \x20 wire [22:0] ma = a[22:0], mb = b[22:0];\n",
    );
    match s.denormal_inputs {
        Denormals::Flush => out.push_str("  wire        za = (ea == 8'd0), zb = (eb == 8'd0);\n"),
        Denormals::Keep => out.push_str("  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);\n"),
    }
    out.push_str(
        "  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);\n\
         \x20 wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);\n",
    );
    match s.denormal_inputs {
        // A flushed denormal is a zero, decided before the datapath: every operand the datapath reads is normal.
        Denormals::Flush => out.push_str(
            "  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};\n\
             \x20 wire [7:0]  xa = ea, xb = eb;\n",
        ),
        // A denormal operand reads as 0.m x 2^-126: no leading one, the least normal's exponent. A zero is decided before
        // the datapath, which reads it as a normal one, so with no denormal operand the datapath is the engine's.
        Denormals::Keep => out.push_str(
            "  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);\n\
             \x20 wire [23:0] fa = {!da, ma}, fb = {!db, mb};\n\
             \x20 wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;\n",
        ),
    }
}

/// The NaN result.
fn nan_result(s: &Semantics) -> String {
    match s.nan {
        Nan::Canonical(v) => format!("32'h{v:08x}"),
        Nan::XorSigned(v) => format!("{{sa ^ sb, 31'h{:08x}}}", v & 0x7fff_ffff),
        Nan::Propagate { invalid } => format!(
            "(na ? {{sa, 8'hff, 1'b1, ma[21:0]}} : nb ? {{sb, 8'hff, 1'b1, mb[21:0]}} : 32'h{invalid:08x})"
        ),
    }
}

/// The rounded result from a significand normalized with its leading one at bit `top` of `pn`, its biased exponent
/// `e` (signed, 12 bits), the sign `s1`: the denormals' place, the rounding, the exponent and significand it lands
/// at (`ef`, `mf`), and for tininess after unbounded rounding whether the 24-bit rounding carries (`cn`).
fn round(out: &mut String, width: usize) {
    let top = width - 1;
    let below = width - 24;
    writeln!(out, "  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;").unwrap();
    writeln!(out, "  wire [6:0]  sh = (sh_raw > 12'd{width}) ? 7'd{width} : sh_raw[6:0];").unwrap();
    writeln!(out, "  wire [{}:0] wide = {{pn, {width}'d0}} >> sh;", 2 * width - 1).unwrap();
    writeln!(out, "  wire [23:0] keep = wide[{}:{}];", 2 * width - 1, 2 * width - 24).unwrap();
    writeln!(out, "  wire        guard = wide[{}];", 2 * width - 25).unwrap();
    writeln!(out, "  wire        sticky = |wide[{}:0];", 2 * width - 26).unwrap();
    out.push_str(
        "  wire        up = guard && (sticky || keep[0]);\n\
         \x20 wire [24:0] r = {1'b0, keep} + {24'd0, up};\n\
         \x20 wire        carry = r[24];\n\
         \x20 wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);\n\
         \x20 wire [22:0] mf = carry ? r[23:1] : r[22:0];\n",
    );
    // All 24 kept bits set and the guard set: the round up carries (the kept bit 0 is set, so ties go up too).
    writeln!(out, "  wire        cn = &pn[{top}:{below}] && pn[{}];", below - 1).unwrap();
}

/// The result's cases after the special ones, each `(condition, value)` in its order: overflow to infinity, the
/// semantics' underflow; and the normal case's value, the last.
fn assemble(cases: &mut Vec<(String, String)>, s: &Semantics, sign: &str) -> String {
    let zero = match s.flushed_sign {
        FlushSign::Kept => format!("{{{sign}, 31'd0}}"),
        FlushSign::Positive => "32'd0".to_string(),
    };
    cases.push(("ef >= 12'sd255".into(), format!("{{{sign}, 8'hff, 23'd0}}")));
    match (s.denormal_results, s.tininess) {
        (Denormals::Keep, _) => {}
        (Denormals::Flush, Tininess::AfterRoundingAtDenormals) => cases.push(("ef <= 12'sd0".into(), zero)),
        (Denormals::Flush, Tininess::BeforeRounding) => cases.push(("e < 12'sd1".into(), zero)),
        (Denormals::Flush, Tininess::AfterRoundingUnbounded) => {
            cases.push(("e < 12'sd1".into(), format!("((e == 12'sd0 && cn) ? {{{sign}, 8'd1, 23'd0}} : {zero})")))
        }
    }
    format!("{{{sign}, ef[7:0], mf}}")
}

/// `y` as the first of `cases` whose condition holds, `last` when none does: one chain of choices, so no bit of it is
/// left undefined by a path that assigns nothing.
fn result(out: &mut String, cases: &[(String, String)], last: &str) {
    out.push_str("  assign y =");
    for (k, (condition, value)) in cases.iter().enumerate() {
        write!(out, "{}({condition}) ? {value}", if k == 0 { " " } else { "\n    : " }).unwrap();
    }
    writeln!(out, "\n    : {last};").unwrap();
}

/// A combinational binary32 multiply under `s`, ports `a`, `b`, `y`.
pub fn fmul_verilog(module: &str, s: &Semantics) -> String {
    let mut out = format!("module {module}(input [31:0] a, input [31:0] b, output [31:0] y);\n");
    operands(&mut out, s);
    // The product written as the engine writes it: Verilog sizes it by its 48-bit context, and two spellings of one
    // product synthesize to two different multipliers.
    match s.denormal_inputs {
        Denormals::Flush => out.push_str("  wire        s = sa ^ sb;\n  wire [47:0] p = {1'b1, ma} * {1'b1, mb};\n"),
        // A denormal operand's significand lacks its leading one: (A - 2^23 da)(B - 2^23 db) = AB - 2^23 (da B + db A)
        // + 2^46 da db over the significands with their leading ones, so the one product AB is the engine's.
        Denormals::Keep => out.push_str(
            "  wire        s = sa ^ sb;\n\
             \x20 wire [47:0] whole = {1'b1, ma} * {1'b1, mb};\n\
             \x20 wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)\n\
             \x20                  + ((da && db) ? 48'd1 << 46 : 48'd0);\n",
        ),
    }
    match s.denormal_inputs {
        // Two normal significands make a product in [2^46, 2^48): its leading one is bit 47 or bit 46.
        Denormals::Flush => out.push_str("  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;\n"),
        // A denormal operand leaves up to 47 leading zeros; two normal ones, as above.
        Denormals::Keep => out.push_str(
            "  reg  [5:0]  lz_d;\n\
             \x20 integer     i;\n\
             \x20 always @* begin\n\
             \x20   lz_d = 6'd48;\n\
             \x20   for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];\n\
             \x20 end\n\
             \x20 wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);\n",
        ),
    }
    out.push_str(
        "  wire [47:0] pn = p << lz;\n\
         \x20 wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});\n",
    );
    round(&mut out, 48);
    let mut cases = vec![
        ("na || nb || (ia && zb) || (ib && za)".to_string(), nan_result(s)),
        ("ia || ib".to_string(), "{s, 8'hff, 23'd0}".to_string()),
        ("za || zb".to_string(), "{s, 31'd0}".to_string()),
    ];
    let last = assemble(&mut cases, s, "s");
    result(&mut out, &cases, &last);
    out.push_str("endmodule\n");
    out
}

/// A combinational binary32 add under `s`, ports `a`, `b`, `y`.
pub fn fadd_verilog(module: &str, s: &Semantics) -> String {
    let mut out = format!("module {module}(input [31:0] a, input [31:0] b, output [31:0] y);\n");
    operands(&mut out, s);
    let small = match s.denormal_inputs {
        Denormals::Flush => "z2 ? 24'd0 : (swap ? fa : fb)",
        Denormals::Keep => "swap ? fa : fb",
    };
    writeln!(
        out,
        "  // The larger magnitude first.\n\
         \x20 wire        swap = {{ea, ma}} < {{eb, mb}};\n\
         \x20 wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;\n\
         \x20 wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;\n\
         \x20 wire        z2 = swap ? za : zb;\n\
         \x20 wire [23:0] f1 = swap ? fb : fa;\n\
         \x20 wire [23:0] f2 = {small};\n\
         \x20 wire [7:0]  d = x1 - x2;\n\
         \x20 // Three places below the significand: guard, round, and a sticky of everything shifted past them.\n\
         \x20 wire [50:0] aligned = {{f2, 27'd0}} >> ((d > 8'd50) ? 8'd50 : d);\n\
         \x20 wire [26:0] g2 = {{aligned[50:25], |aligned[24:0]}};\n\
         \x20 wire [26:0] g1 = {{f1, 3'd0}};\n\
         \x20 wire        sub = s1 ^ s2;\n\
         \x20 wire [27:0] sum = sub ? {{1'b0, g1}} - {{1'b0, g2}} : {{1'b0, g1}} + {{1'b0, g2}};\n\
         \x20 reg  [4:0]  lz;\n\
         \x20 integer     i;\n\
         \x20 always @* begin\n\
         \x20   lz = 5'd27;\n\
         \x20   for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];\n\
         \x20 end\n\
         \x20 wire        c = sum[27];\n\
         \x20 wire signed [11:0] e = c ? $signed({{4'd0, x1}}) + 12'sd1 : $signed({{4'd0, x1}}) - $signed({{7'd0, lz}});\n\
         \x20 // Normalized with the leading one at bit 26, the sticky kept through a right shift.\n\
         \x20 wire [26:0] pn = c ? {{sum[27:2], sum[1] | sum[0]}} : (sum[26:0] << lz);"
    )
    .unwrap();
    round(&mut out, 27);
    out.push_str("  wire        zero = (sum == 28'd0);\n");
    let mut cases = vec![
        ("na || nb || (ia && ib && (sa != sb))".to_string(), nan_result(s)),
        ("ia".to_string(), "{sa, 8'hff, 23'd0}".to_string()),
        ("ib".to_string(), "{sb, 8'hff, 23'd0}".to_string()),
        ("za && zb".to_string(), "{sa & sb, 31'd0}".to_string()),
    ];
    // A zero plus the other operand is that operand, flushed when it is a denormal results do not keep.
    let pass = |x: &str, sx: &str, ex: &str| match (s.denormal_inputs, s.denormal_results) {
        (Denormals::Keep, Denormals::Flush) => {
            let zero = match s.flushed_sign {
                FlushSign::Kept => format!("{{{sx}, 31'd0}}"),
                FlushSign::Positive => "32'd0".to_string(),
            };
            format!("({ex} == 8'd0) ? {zero} : {x}")
        }
        _ => x.to_string(),
    };
    cases.push(("za".into(), format!("({})", pass("b", "sb", "eb"))));
    cases.push(("zb".into(), format!("({})", pass("a", "sa", "ea"))));
    cases.push(("zero".into(), "32'd0".into()));
    let last = assemble(&mut cases, s, "s1");
    result(&mut out, &cases, &last);
    out.push_str("endmodule\n");
    out
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/fp_hypotheses/tests.rs"]
mod tests;
