//! The decode engine's arithmetic, bit for bit.
//!
//! Every number the engine produces is defined here as a sequence of exact integer operations and f32 operations,
//! so the hardware is held to these functions exactly and these functions are held to the model's own arithmetic
//! within a measured bar.
//!
//! ★ THE ENGINE NEVER ROUNDS A WEIGHT. A block format stores integer codes and a few scales; every weight is a
//! scale times an integer. Activations are quantized under scales `m·2^e`, the codes the exact nearest integers of
//! `x / scale`, in one of two ways ([`ActScaling`]):
//!   • per block of 32 under a 4-bit significand (`m` in 8..15): a matrix row times an activation row is, within a
//!     group of 256 elements, a sum of products of a few small integers and a binary16 significand with integer dot
//!     products, taken EXACTLY, aligned to the group's least exponent in a wide integer, the group rounded to f32 once;
//!   • per weight block (the matrix format's own block, 32 or 256 elements) under an 8-bit significand (`m` in
//!     128..255), llama.cpp's own blocking: each weight block's two terms — its scale times the block's integer sum,
//!     its second scale times the offset terms' — summed exactly and rounded to f32 once, which is what the array's
//!     block-scale lanes compute at the array's rate.
//! Either way the rounded pieces of a row are added in order.
//!
//! f32 arithmetic is IEEE binary32, round to nearest even, with denormal operands and results flushed to zero
//! keeping their sign ([`ftz`]); multiply and add round separately. Dot products and sums over a vector run in
//! [`LANES`] lanes (element `i` in lane `i mod LANES`, in index order) folded by a balanced pairwise tree. The
//! transcendentals are fixed sequences of those operations over fixed tables.

use serde::Serialize;

use crate::codegen_sva::fp_hypotheses::{Nan, Semantics, Tininess};
use crate::model::quant::{f16_to_f32, GgmlType, KVALUES_IQ4NL};

// ── f32 as the engine computes it ───────────────────────────────────────────────────────────────────

/// `x` with a denormal flushed to a zero of its sign.
pub fn ftz(x: f32) -> f32 {
    if x.is_subnormal() {
        if x.is_sign_negative() {
            -0.0
        } else {
            0.0
        }
    } else {
        x
    }
}

/// The one NaN the engine produces: a quiet NaN of no payload and no sign.
pub const NAN: f32 = f32::from_bits(0x7fc0_0000);

/// `x`, a NaN made the engine's one NaN.
fn canonical(x: f32) -> f32 {
    if x.is_nan() {
        NAN
    } else {
        x
    }
}

pub fn fmul(a: f32, b: f32) -> f32 {
    canonical(ftz(ftz(a) * ftz(b)))
}

pub fn fadd(a: f32, b: f32) -> f32 {
    canonical(ftz(ftz(a) + ftz(b)))
}

pub fn fsub(a: f32, b: f32) -> f32 {
    canonical(ftz(ftz(a) - ftz(b)))
}

/// The multiplier an f32 product is computed by. Its add is the engine's either way: the card's block adds as
/// [`fadd`] does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize)]
pub enum FpUnit {
    /// The engine's own multiply, [`fmul`].
    #[default]
    Soft,
    /// The card's floating-point DSP block ([`FpUnit::CARD`]): it decides a product tiny at 24 bits before the engine
    /// does, so an exact product just below the least normal that the engine rounds up into it can flush to a zero of
    /// its sign; and a NaN takes the operands' signs' exclusive or.
    Hard,
}

impl FpUnit {
    /// The card's multiply, the one class fp-oracle6's receipt names: tininess decided after rounding to 24 bits with
    /// the exponent unbounded, the NaN `0x7fc00000` signed by the operands' signs' exclusive or.
    pub const CARD: Semantics = Semantics { tininess: Tininess::AfterRoundingUnbounded, nan: Nan::XorSigned(0x7fc0_0000), ..Semantics::ENGINE };

    /// `a·b` as this unit computes it.
    pub fn mul(self, a: f32, b: f32) -> f32 {
        match self {
            FpUnit::Soft => fmul(a, b),
            FpUnit::Hard => f32::from_bits(Self::CARD.mul(a.to_bits(), b.to_bits())),
        }
    }
}

/// Lanes a dot product or a sum accumulates in before its tree.
pub const LANES: usize = 16;

/// Folds lane partials by a balanced pairwise tree: adjacent pairs, then pairs of pairs.
pub fn tree(mut lanes: Vec<f32>) -> f32 {
    while lanes.len() > 1 {
        lanes = lanes.chunks(2).map(|p| if p.len() == 2 { fadd(p[0], p[1]) } else { p[0] }).collect();
    }
    lanes.pop().unwrap_or(0.0)
}

/// `Σ a_i·b_i` in [`LANES`] lanes, then the tree.
pub fn fdot(fp: FpUnit, a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "a dot of unequal lengths");
    let mut lanes = vec![0f32; LANES];
    for (i, (&x, &y)) in a.iter().zip(b).enumerate() {
        lanes[i % LANES] = fadd(lanes[i % LANES], fp.mul(x, y));
    }
    tree(lanes)
}

/// `Σ a_i` in [`LANES`] lanes, then the tree.
pub fn fsum(a: &[f32]) -> f32 {
    let mut lanes = vec![0f32; LANES];
    for (i, &x) in a.iter().enumerate() {
        lanes[i % LANES] = fadd(lanes[i % LANES], x);
    }
    tree(lanes)
}

/// The largest of `x`, folded from −∞; a NaN is passed over, and a largest value that is a zero is `+0` whichever
/// zeros it was folded from, so lanes folding in any order agree.
pub fn fmax(x: &[f32]) -> f32 {
    let m = x.iter().fold(f32::NEG_INFINITY, |m, &v| if v > m { v } else { m });
    if m == 0.0 {
        0.0
    } else {
        m
    }
}

/// The largest magnitude in `x`, a denormal read as a zero, folded from `+0`; a NaN is passed over.
pub fn absmax(x: &[f32]) -> f32 {
    x.iter().fold(0f32, |m, &v| {
        let a = ftz(v).abs();
        if a > m {
            a
        } else {
            m
        }
    })
}

/// The index of `x`'s largest element, the first of equals (the two zeros equal), a NaN passed over; `0` when there
/// is none. Lanes each keeping their first largest and folded to the larger, the lower index of equals, agree with it
/// in any order.
pub fn argmax(x: &[f32]) -> usize {
    let mut best: Option<usize> = None;
    for (i, &v) in x.iter().enumerate() {
        if !v.is_nan() && best.is_none_or(|b| v > x[b]) {
            best = Some(i);
        }
    }
    best.unwrap_or(0)
}

/// `x·2^n`, exact while the result is a normal f32: infinite past the top, a zero of its sign below the bottom.
pub fn ldexp(x: f32, n: i32) -> f32 {
    let x = ftz(x);
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    let bits = x.to_bits();
    let e = ((bits >> 23) & 0xff) as i32 + n;
    if e >= 0xff {
        return f32::INFINITY.copysign(x);
    }
    if e <= 0 {
        return 0f32.copysign(x);
    }
    f32::from_bits((bits & 0x807f_ffff) | ((e as u32) << 23))
}

/// `v·2^exp` rounded once to f32, to nearest with ties to even, a denormal flushed to a zero of its sign.
pub fn scaled_to_f32(v: i128, exp: i32) -> f32 {
    if v == 0 {
        return 0.0;
    }
    let neg = v < 0;
    let mut m = v.unsigned_abs();
    let mut e = exp;
    let bits = 128 - m.leading_zeros() as i32;
    if bits > 24 {
        let s = (bits - 24) as u32;
        let rem = m & ((1u128 << s) - 1);
        let half = 1u128 << (s - 1);
        m >>= s;
        e += s as i32;
        if rem > half || (rem == half && m & 1 == 1) {
            m += 1;
            if m == 1 << 24 {
                m >>= 1;
                e += 1;
            }
        }
    }
    let lead = 127 - m.leading_zeros() as i32;
    let unbiased = e + lead;
    if unbiased > 127 {
        return if neg { f32::NEG_INFINITY } else { f32::INFINITY };
    }
    if unbiased < -126 {
        return if neg { -0.0 } else { 0.0 };
    }
    let mantissa = ((m << (23 - lead)) as u32) & 0x7f_ffff;
    f32::from_bits(((neg as u32) << 31) | (((unbiased + 127) as u32) << 23) | mantissa)
}

/// `x` rounded to the nearest whole number, ties to even, as an integer.
pub fn rne(x: f32) -> i64 {
    x.round_ties_even() as i64
}

// ── f16 ─────────────────────────────────────────────────────────────────────────────────────────────

/// `x` rounded to binary16, to nearest with ties to even, subnormals kept, past the top infinite.
pub fn f32_to_f16(x: f32) -> u16 {
    let b = x.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32;
    let mant = b & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if mant != 0 { 0x0200 } else { 0 };
    }
    if exp == 0 {
        // An f32 denormal is below half of binary16's least denormal.
        return sign;
    }
    let e = exp - 127;
    if e > 15 {
        return sign | 0x7c00;
    }
    if e >= -14 {
        let m = mant >> 13;
        let rem = mant & 0x1fff;
        let mut h = (((e + 15) as u32) << 10) | m;
        if rem > 0x1000 || (rem == 0x1000 && m & 1 == 1) {
            h += 1;
        }
        return sign | h as u16;
    }
    // A binary16 denormal counts units of 2^-24: the 24-bit significand times 2^(e+1).
    let s = (-(e + 1)) as u32;
    if s > 24 {
        return sign;
    }
    let sig = 0x80_0000 | mant;
    let n = sig >> s;
    let rem = sig & ((1u32 << s) - 1);
    let half = 1u32 << (s - 1);
    let n = if rem > half || (rem == half && n & 1 == 1) { n + 1 } else { n };
    sign | n as u16
}

/// A scale stored in a block: `sig·2^exp`, exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scale {
    pub sig: i64,
    pub exp: i32,
}

impl Scale {
    pub const ZERO: Scale = Scale { sig: 0, exp: 0 };

    /// A binary16 scale; `None` for an infinity or a NaN.
    pub fn f16(h: u16) -> Option<Scale> {
        let neg = h & 0x8000 != 0;
        let e = ((h >> 10) & 0x1f) as i32;
        let m = (h & 0x3ff) as i64;
        let (sig, exp) = match e {
            0x1f => return None,
            0 => (m, -24),
            _ => (1024 + m, e - 25),
        };
        Some(Scale { sig: if neg { -sig } else { sig }, exp })
    }
}

// ── Activations ─────────────────────────────────────────────────────────────────────────────────────

/// Elements whose products round to f32 together.
pub const GROUP: usize = 256;

/// Elements that share one activation scale: a ggml sub-block, so a block's scale meets a weight sub-block's.
pub const ACT_BLOCK: usize = 32;

/// Words an activation crosses the array in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Words {
    /// One 8-bit word per element: the nearest integer to `x / scale`, at most 127 in magnitude.
    One,
    /// A 16-bit code, the nearest integer to `256·x / scale`, in two words: its high byte signed, its low byte
    /// unsigned, so neither overflows and the low word's pass needs no excess.
    Two,
}

/// How an activation row is scaled, and so how a matrix row's dot with it rounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum ActScaling {
    /// A scale per [`ACT_BLOCK`] elements with a 4-bit significand; a group of [`GROUP`] elements' products summed
    /// exactly and rounded once.
    Sub32,
    /// One scale per weight block with a [`WIDE_BITS`]-bit significand; each block's two terms summed exactly and
    /// rounded once.
    PerBlock,
}

/// Significand bits of a per-weight-block activation scale.
pub const WIDE_BITS: u32 = 8;

/// An activation block's scale `m·2^e`: `m` in 8..15 per [`ActScaling::Sub32`] block, in 128..255 per
/// [`ActScaling::PerBlock`] block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActScale {
    pub m: i64,
    pub e: i32,
}

/// An activation row as the array reads it: a whole-number code per element, worth `code·m·2^e` of its block of
/// `block` elements.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActRow {
    pub codes: Vec<i32>,
    pub scales: Vec<ActScale>,
    pub scaling: ActScaling,
    pub block: usize,
}

impl ActRow {
    /// The value each element stands for.
    pub fn values(&self) -> Vec<f64> {
        self.codes
            .iter()
            .enumerate()
            .map(|(i, &c)| {
                let s = self.scales[i / self.block];
                c as f64 * s.m as f64 * 2f64.powi(s.e)
            })
            .collect()
    }
}

/// `x` as an exact integer significand and exponent, `x = sig·2^exp`.
fn split(x: f32) -> (i64, i32) {
    let b = x.to_bits();
    let e = ((b >> 23) & 0xff) as i32;
    let m = (b & 0x7f_ffff) as i64;
    let sig = if e == 0 { m } else { m | 0x80_0000 };
    let sig = if b >> 31 == 1 { -sig } else { sig };
    (sig, if e == 0 { -149 } else { e - 150 })
}

/// The nearest integer to `x / (m·2^e)`, ties to even, exactly.
pub fn rne_div(x: f32, m: i64, e: i32) -> i64 {
    let (sig, exp) = split(ftz(x));
    if sig == 0 {
        return 0;
    }
    // x / (m·2^e) = sig·2^(exp-e) / m, carried with 64 fraction bits.
    let k = exp - e + 64;
    if k < 0 {
        return 0;
    }
    if k > 60 + 64 {
        return if sig < 0 { i64::MIN } else { i64::MAX };
    }
    let num = (sig.unsigned_abs() as u128) << k as u32;
    let q = num / m as u128;
    let rem = num % m as u128;
    let whole = (q >> 64) as i64;
    let frac = q & u64::MAX as u128;
    let half = 1u128 << 63;
    let up = frac > half || (frac == half && rem > 0) || (frac == half && rem == 0 && whole & 1 == 1);
    let n = whole + up as i64;
    if sig < 0 {
        -n
    } else {
        n
    }
}

/// The least scale `m·2^e` with a `bits`-bit significand (`m` in `2^(bits−1)..2^bits`) and `127·m·2^e ≥ amax`, for a
/// finite positive `amax`.
fn least_scale(amax: f32, bits: u32) -> ActScale {
    let (sig, exp) = split(amax);
    let (lo, hi) = (1i64 << (bits - 1), (1i64 << bits) - 1);
    // 127·m·2^e ≥ sig·2^exp, compared exactly.
    let fits = |m: i64, e: i32| -> bool {
        let d = e - exp;
        if d >= 0 {
            d > 40 || (127 * m) << d >= sig
        } else {
            -d < 60 && 127 * m >= (sig + (1i64 << -d) - 1) >> -d
        }
    };
    let mut e = (amax as f64 / (127.0 * hi as f64)).log2().floor() as i32 - 1;
    while !fits(hi, e) {
        e += 1;
    }
    while fits(hi, e - 1) {
        e -= 1;
    }
    let m = (lo..=hi).find(|&m| fits(m, e)).expect("the least exponent leaves a significand of its width");
    ActScale { m, e }
}

/// The least scale `m·2^e` (`m` in 8..15) with `127·m·2^e ≥ amax`, for a finite positive `amax`.
pub fn block_scale(amax: f32) -> ActScale {
    least_scale(amax, 4)
}

/// The least scale `m·2^e` (`m` in 128..255) with `127·m·2^e ≥ amax`, for a finite positive `amax`.
pub fn wide_scale(amax: f32) -> ActScale {
    least_scale(amax, WIDE_BITS)
}

/// `x` quantized per block of `block` elements, each under `scale_of` its largest magnitude; an all-zero block's codes
/// all zero under `zero`.
fn quantize_with(x: &[f32], words: Words, block: usize, scaling: ActScaling, scale_of: fn(f32) -> ActScale, zero: ActScale) -> ActRow {
    let mut codes = Vec::with_capacity(x.len());
    let mut scales = Vec::with_capacity(x.len() / block);
    for chunk in x.chunks(block) {
        let amax = chunk.iter().fold(0f32, |m, &v| m.max(ftz(v).abs()));
        assert!(amax.is_finite(), "a non-finite activation");
        if amax == 0.0 {
            scales.push(zero);
            codes.extend(std::iter::repeat(0).take(block));
            continue;
        }
        let s = scale_of(amax);
        match words {
            Words::One => {
                scales.push(s);
                codes.extend(chunk.iter().map(|&v| rne_div(v, s.m, s.e).clamp(-127, 127) as i32));
            }
            Words::Two => {
                scales.push(ActScale { m: s.m, e: s.e - 8 });
                codes.extend(chunk.iter().map(|&v| rne_div(v, s.m, s.e - 8) as i32));
            }
        }
    }
    ActRow { codes, scales, scaling, block }
}

/// `x` (a whole number of groups) quantized per block of [`ACT_BLOCK`].
pub fn quantize(x: &[f32], words: Words) -> ActRow {
    assert!(x.len() % GROUP == 0, "{} elements are not a whole number of groups of {GROUP}", x.len());
    quantize_with(x, words, ACT_BLOCK, ActScaling::Sub32, block_scale, ActScale { m: 8, e: 0 })
}

/// `x` (a whole number of `block`s) quantized per weight block of `block` elements.
pub fn quantize_blocks(x: &[f32], words: Words, block: usize) -> ActRow {
    assert!(block > 0 && x.len() % block == 0, "{} elements are not a whole number of blocks of {block}", x.len());
    quantize_with(x, words, block, ActScaling::PerBlock, wide_scale, ActScale { m: 1 << (WIDE_BITS - 1), e: 0 })
}

// ── Matrix rows ─────────────────────────────────────────────────────────────────────────────────────

/// A run of a row's elements under one pair of scales: the elements' coefficients under the first scale (codes
/// times their sub-block scales) and under the second (the offset terms), as stored, exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub start: usize,
    pub len: usize,
    pub a: Scale,
    pub b: Scale,
}

/// A row of a block-format matrix as the engine reads it: every weight is `a·coef + b·offs` of its segment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodedRow {
    pub coef: Vec<i16>,
    pub offs: Vec<i16>,
    pub segments: Vec<Segment>,
}

/// A block a format stores that the engine cannot read: an infinite or NaN scale.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unreadable {
    pub ty: GgmlType,
    pub block: usize,
}

fn half_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

/// ggml's `get_scale_min_k4`: the 6-bit scale and min of sub-block `j` from a K-quant's 12 packed bytes.
fn scale_min_k4(j: usize, q: &[u8]) -> (u8, u8) {
    if j < 4 {
        (q[j] & 63, q[j + 4] & 63)
    } else {
        ((q[j + 4] & 0xf) | ((q[j - 4] >> 6) << 4), (q[j + 4] >> 4) | ((q[j] >> 6) << 4))
    }
}

/// Whether the engine's array reads matrices of `ty`.
pub fn reads(ty: GgmlType) -> bool {
    matches!(
        ty,
        GgmlType::Q4_0
            | GgmlType::Q4_1
            | GgmlType::Q5_0
            | GgmlType::Q5_1
            | GgmlType::Q8_0
            | GgmlType::Q2K
            | GgmlType::Q3K
            | GgmlType::Q4K
            | GgmlType::Q5K
            | GgmlType::Q6K
            | GgmlType::Iq4Nl
            | GgmlType::Iq4Xs
    )
}

impl CodedRow {
    /// The coded form of `bytes` (whole blocks of `ty`), or `None` for a type the array does not read.
    pub fn decode(ty: GgmlType, bytes: &[u8]) -> Option<Result<CodedRow, Unreadable>> {
        if !reads(ty) {
            return None;
        }
        let bb = ty.block_bytes();
        let n = ty.block_len();
        assert!(bytes.len() % bb == 0, "{} bytes are not whole {} blocks", bytes.len(), ty.name());
        let blocks = bytes.len() / bb;
        let mut row = CodedRow { coef: Vec::with_capacity(blocks * n), offs: Vec::with_capacity(blocks * n), segments: Vec::with_capacity(blocks) };
        for (bi, b) in bytes.chunks_exact(bb).enumerate() {
            let bad = || Unreadable { ty, block: bi };
            let start = bi * n;
            let scale = |at: usize| Scale::f16(half_at(b, at)).ok_or_else(bad);
            let (a, s) = match ty {
                GgmlType::Q4_0 => {
                    let qs = &b[2..18];
                    row.coef.extend(qs.iter().map(|&q| (q & 0x0f) as i16 - 8));
                    row.coef.extend(qs.iter().map(|&q| (q >> 4) as i16 - 8));
                    row.offs.extend(std::iter::repeat(0).take(32));
                    (scale(0), Ok(Scale::ZERO))
                }
                GgmlType::Q4_1 => {
                    let qs = &b[4..20];
                    row.coef.extend(qs.iter().map(|&q| (q & 0x0f) as i16));
                    row.coef.extend(qs.iter().map(|&q| (q >> 4) as i16));
                    row.offs.extend(std::iter::repeat(1).take(32));
                    (scale(0), scale(2))
                }
                GgmlType::Q5_0 | GgmlType::Q5_1 => {
                    let one = ty == GgmlType::Q5_1;
                    let (qh_at, qs_at) = if one { (4, 8) } else { (2, 6) };
                    let qh = u32::from_le_bytes([b[qh_at], b[qh_at + 1], b[qh_at + 2], b[qh_at + 3]]);
                    let qs = &b[qs_at..qs_at + 16];
                    let centre = if one { 0 } else { 16 };
                    let lo: Vec<i16> = qs.iter().enumerate().map(|(j, &q)| ((q & 0x0f) | ((((qh >> j) << 4) & 0x10) as u8)) as i16 - centre).collect();
                    let hi: Vec<i16> = qs.iter().enumerate().map(|(j, &q)| ((q >> 4) | (((qh >> (j + 12)) & 0x10) as u8)) as i16 - centre).collect();
                    row.coef.extend(lo);
                    row.coef.extend(hi);
                    row.offs.extend(std::iter::repeat(one as i16).take(32));
                    (scale(0), if one { scale(2) } else { Ok(Scale::ZERO) })
                }
                GgmlType::Q8_0 => {
                    row.coef.extend(b[2..34].iter().map(|&q| (q as i8) as i16));
                    row.offs.extend(std::iter::repeat(0).take(32));
                    (scale(0), Ok(Scale::ZERO))
                }
                GgmlType::Q2K => {
                    let scales = &b[0..16];
                    let qs = &b[16..80];
                    let mut is = 0;
                    for nn in (0..256).step_by(128) {
                        let q = &qs[nn / 4..nn / 4 + 32];
                        let mut shift = 0;
                        for _ in 0..4 {
                            for half_block in [0usize, 16] {
                                let sc = scales[is];
                                is += 1;
                                for l in 0..16 {
                                    row.coef.push((sc & 0xf) as i16 * ((q[l + half_block] >> shift) & 3) as i16);
                                    row.offs.push(-((sc >> 4) as i16));
                                }
                            }
                            shift += 2;
                        }
                    }
                    (scale(80), scale(82))
                }
                GgmlType::Q3K => {
                    let hm = &b[0..32];
                    let qs = &b[32..96];
                    let packed = &b[96..108];
                    let (kmask1, kmask2) = (0x0303_0303u32, 0x0f0f_0f0fu32);
                    let mut aux = [0u32; 4];
                    for (k, a) in aux.iter_mut().take(3).enumerate() {
                        *a = u32::from_le_bytes([packed[4 * k], packed[4 * k + 1], packed[4 * k + 2], packed[4 * k + 3]]);
                    }
                    let tmp = aux[2];
                    aux[2] = ((aux[0] >> 4) & kmask2) | (((tmp >> 4) & kmask1) << 4);
                    aux[3] = ((aux[1] >> 4) & kmask2) | (((tmp >> 6) & kmask1) << 4);
                    aux[0] = (aux[0] & kmask2) | ((tmp & kmask1) << 4);
                    aux[1] = (aux[1] & kmask2) | (((tmp >> 2) & kmask1) << 4);
                    let scales: Vec<i16> = aux.iter().flat_map(|a| a.to_le_bytes()).map(|v| (v as i8) as i16).collect();
                    let mut is = 0;
                    let mut m = 1u8;
                    for nn in (0..256).step_by(128) {
                        let q = &qs[nn / 4..nn / 4 + 32];
                        let mut shift = 0;
                        for _ in 0..4 {
                            for half_block in [0usize, 16] {
                                let sc = scales[is] - 32;
                                is += 1;
                                for l in 0..16 {
                                    let low = ((q[l + half_block] >> shift) & 3) as i16;
                                    let high = if hm[l + half_block] & m != 0 { 0 } else { 4 };
                                    row.coef.push(sc * (low - high));
                                    row.offs.push(0);
                                }
                            }
                            shift += 2;
                            m <<= 1;
                        }
                    }
                    (scale(108), Ok(Scale::ZERO))
                }
                GgmlType::Q4K | GgmlType::Q5K => {
                    let five = ty == GgmlType::Q5K;
                    let sc_bytes = &b[4..16];
                    let (qh, qs) = if five { (&b[16..48], &b[48..176]) } else { (&b[0..0], &b[16..144]) };
                    let (mut u1, mut u2) = (1u8, 2u8);
                    for (pair, j) in (0..256).step_by(64).enumerate() {
                        let q = &qs[j / 2..j / 2 + 32];
                        let (s1, m1) = scale_min_k4(2 * pair, sc_bytes);
                        let (s2, m2) = scale_min_k4(2 * pair + 1, sc_bytes);
                        for l in 0..32 {
                            let hi = if five && qh[l] & u1 != 0 { 16 } else { 0 };
                            row.coef.push(s1 as i16 * ((q[l] & 0xf) as i16 + hi));
                            row.offs.push(-(m1 as i16));
                        }
                        for l in 0..32 {
                            let hi = if five && qh[l] & u2 != 0 { 16 } else { 0 };
                            row.coef.push(s2 as i16 * ((q[l] >> 4) as i16 + hi));
                            row.offs.push(-(m2 as i16));
                        }
                        u1 <<= 2;
                        u2 <<= 2;
                    }
                    (scale(0), scale(2))
                }
                GgmlType::Q6K => {
                    let ql_all = &b[0..128];
                    let qh_all = &b[128..192];
                    let sc_all: Vec<i16> = b[192..208].iter().map(|&v| (v as i8) as i16).collect();
                    let mut block = [0i16; 256];
                    for (part, nn) in (0..256).step_by(128).enumerate() {
                        let ql = &ql_all[part * 64..part * 64 + 64];
                        let qh = &qh_all[part * 32..part * 32 + 32];
                        let sc = &sc_all[part * 8..part * 8 + 8];
                        for l in 0..32 {
                            let is = l / 16;
                            let q1 = ((ql[l] & 0xf) | ((qh[l] & 3) << 4)) as i16 - 32;
                            let q2 = ((ql[l + 32] & 0xf) | (((qh[l] >> 2) & 3) << 4)) as i16 - 32;
                            let q3 = ((ql[l] >> 4) | (((qh[l] >> 4) & 3) << 4)) as i16 - 32;
                            let q4 = ((ql[l + 32] >> 4) | (((qh[l] >> 6) & 3) << 4)) as i16 - 32;
                            block[nn + l] = sc[is] * q1;
                            block[nn + l + 32] = sc[is + 2] * q2;
                            block[nn + l + 64] = sc[is + 4] * q3;
                            block[nn + l + 96] = sc[is + 6] * q4;
                        }
                    }
                    row.coef.extend_from_slice(&block);
                    row.offs.extend(std::iter::repeat(0).take(256));
                    (scale(208), Ok(Scale::ZERO))
                }
                GgmlType::Iq4Nl => {
                    let qs = &b[2..18];
                    row.coef.extend(qs.iter().map(|&q| KVALUES_IQ4NL[(q & 0x0f) as usize] as i16));
                    row.coef.extend(qs.iter().map(|&q| KVALUES_IQ4NL[(q >> 4) as usize] as i16));
                    row.offs.extend(std::iter::repeat(0).take(32));
                    (scale(0), Ok(Scale::ZERO))
                }
                GgmlType::Iq4Xs => {
                    let scales_h = u16::from_le_bytes([b[2], b[3]]);
                    for ib in 0..8 {
                        let ls = ((b[4 + ib / 2] >> (4 * (ib % 2))) & 0xf) as i16 | ((((scales_h >> (2 * ib)) & 3) as i16) << 4);
                        let q = &b[8 + 16 * ib..8 + 16 * ib + 16];
                        row.coef.extend(q.iter().map(|&v| (ls - 32) * KVALUES_IQ4NL[(v & 0x0f) as usize] as i16));
                        row.coef.extend(q.iter().map(|&v| (ls - 32) * KVALUES_IQ4NL[(v >> 4) as usize] as i16));
                    }
                    row.offs.extend(std::iter::repeat(0).take(256));
                    (scale(0), Ok(Scale::ZERO))
                }
                _ => unreachable!("reads() admits only the formats decoded here"),
            };
            match (a, s) {
                (Ok(a), Ok(b)) => row.segments.push(Segment { start, len: n, a, b }),
                _ => return Some(Err(bad())),
            }
        }
        Some(Ok(row))
    }

    /// The exact value of element `i`: `(sig, exp)` with the value `sig·2^exp`.
    pub fn exact(&self, i: usize) -> f64 {
        let s = &self.segments[i / self.segments[0].len];
        s.a.sig as f64 * 2f64.powi(s.a.exp) * self.coef[i] as f64 + s.b.sig as f64 * 2f64.powi(s.b.exp) * self.offs[i] as f64
    }
}

/// The f32 the engine produces for `row · act`.
///
/// Under [`ActScaling::Sub32`]: per group of [`GROUP`] elements, for every weight segment and every activation block
/// in it, the weight scale's significand times the activation scale's `m` times their exact integer sum, aligned to
/// the group's least exponent and summed exactly, rounded once; the groups added in order.
///
/// Under [`ActScaling::PerBlock`]: per weight block, its first scale times the activation scale times the block's
/// integer sum of coefficients and codes, and its second scale times the activation scale times the integer sum of
/// offset terms and codes, the two summed exactly and rounded once; the blocks added in order.
pub fn dot(row: &CodedRow, act: &ActRow) -> f32 {
    assert_eq!(row.coef.len(), act.codes.len(), "a row and an activation of unequal lengths");
    match act.scaling {
        ActScaling::Sub32 => dot_sub32(row, act),
        ActScaling::PerBlock => dot_per_block(row, act),
    }
}

/// The largest magnitude a coded row's coefficient or offset takes in any format the array reads: Q6_K's `−128·−32`.
pub const COEF_BOUND: u32 = 1 << 12;

/// `Σ c_i·a_i` exactly. Every coefficient lies within [`COEF_BOUND`] and every activation code within `2^15` (a two-word
/// code), so a product lies within `2^27` and eight within `2^30`: the products are summed eight at a time in 32 bits
/// and the eights in 64.
fn int_dot(c: &[i16], a: &[i32]) -> i64 {
    c.chunks(8).zip(a.chunks(8)).map(|(c, a)| c.iter().zip(a).map(|(&c, &a)| c as i32 * a).sum::<i32>() as i64).sum()
}

fn dot_per_block(row: &CodedRow, act: &ActRow) -> f32 {
    let mut acc = 0f32;
    for (b, s) in row.segments.iter().enumerate() {
        assert_eq!(act.block, s.len, "an activation scaled per {} elements against a row of {}-element blocks", act.block, s.len);
        let sc = act.scales[b];
        let range = s.start..s.start + s.len;
        let ia = int_dot(&row.coef[range.clone()], &act.codes[range.clone()]);
        // A block with no second scale has no offset terms to sum.
        let ib = if s.b.sig == 0 { 0 } else { int_dot(&row.offs[range.clone()], &act.codes[range]) };
        let mut terms = [(0i128, 0i32); 2];
        let mut n = 0;
        for (w, sum) in [(s.a, ia), (s.b, ib)] {
            if w.sig != 0 && sum != 0 {
                terms[n] = (w.sig as i128 * sc.m as i128 * sum as i128, w.exp + sc.e);
                n += 1;
            }
        }
        // Every block adds its value: a block whose terms both vanish adds +0.
        acc = fadd(acc, exact_sum(&terms[..n]));
    }
    acc
}

fn dot_sub32(row: &CodedRow, act: &ActRow) -> f32 {
    let seg_len = row.segments.first().map_or(GROUP, |s| s.len);
    let mut acc = 0f32;
    let mut terms: Vec<(i128, i32)> = Vec::with_capacity(32);
    for g in 0..row.coef.len() / GROUP {
        terms.clear();
        for b in g * GROUP / ACT_BLOCK..(g + 1) * GROUP / ACT_BLOCK {
            let s = &row.segments[b * ACT_BLOCK / seg_len];
            let sc = act.scales[b];
            let range = b * ACT_BLOCK..(b + 1) * ACT_BLOCK;
            let ia = int_dot(&row.coef[range.clone()], &act.codes[range.clone()]);
            let ib = if s.b.sig == 0 { 0 } else { int_dot(&row.offs[range.clone()], &act.codes[range]) };
            for (w, sum) in [(s.a, ia), (s.b, ib)] {
                if w.sig != 0 && sum != 0 {
                    terms.push((w.sig as i128 * sc.m as i128 * sum as i128, w.exp + sc.e));
                }
            }
        }
        // Every group adds its value: a group whose terms all vanish adds +0.
        acc = fadd(acc, exact_sum(&terms));
    }
    acc
}

/// Binary places an exact sum keeps below its largest term's least place.
pub const SUM_WINDOW: i32 = 64;

/// `Σ v·2^e` over `terms`, rounded once to f32: exact while every term lies within [`SUM_WINDOW`] places of the
/// largest term's least place, a term past the window floored at it, however far past; no terms is +0.
pub fn exact_sum(terms: &[(i128, i32)]) -> f32 {
    let (Some(lo), Some(hi)) = (terms.iter().map(|t| t.1).min(), terms.iter().map(|t| t.1).max()) else {
        return 0.0;
    };
    let least = lo.max(hi - SUM_WINDOW);
    // A term 127 places down is already its floor: −1 or 0.
    let sum: i128 = terms.iter().map(|&(v, e)| if e >= least { v << (e - least) } else { v >> (least - e).min(127) }).sum();
    scaled_to_f32(sum, least)
}

// ── Transcendentals ─────────────────────────────────────────────────────────────────────────────────

/// `2^(j/64)` for `j` in `0..64`, as f32 bit patterns.
pub(crate) const EXP2_64: [u32; 64] = [
    0x3f800000, 0x3f8164d2, 0x3f82cd87, 0x3f843a29, 0x3f85aac3, 0x3f871f62, 0x3f88980f, 0x3f8a14d5,
    0x3f8b95c2, 0x3f8d1adf, 0x3f8ea43a, 0x3f9031dc, 0x3f91c3d3, 0x3f935a2b, 0x3f94f4f0, 0x3f96942d,
    0x3f9837f0, 0x3f99e046, 0x3f9b8d3a, 0x3f9d3eda, 0x3f9ef532, 0x3fa0b051, 0x3fa27043, 0x3fa43516,
    0x3fa5fed7, 0x3fa7cd94, 0x3fa9a15b, 0x3fab7a3a, 0x3fad583f, 0x3faf3b79, 0x3fb123f6, 0x3fb311c4,
    0x3fb504f3, 0x3fb6fd92, 0x3fb8fbaf, 0x3fbaff5b, 0x3fbd08a4, 0x3fbf179a, 0x3fc12c4d, 0x3fc346cd,
    0x3fc5672a, 0x3fc78d75, 0x3fc9b9be, 0x3fcbec15, 0x3fce248c, 0x3fd06334, 0x3fd2a81e, 0x3fd4f35b,
    0x3fd744fd, 0x3fd99d16, 0x3fdbfbb8, 0x3fde60f5, 0x3fe0ccdf, 0x3fe33f89, 0x3fe5b907, 0x3fe8396a,
    0x3feac0c7, 0x3fed4f30, 0x3fefe4ba, 0x3ff28177, 0x3ff5257d, 0x3ff7d0df, 0x3ffa83b3, 0x3ffd3e0c,
];
/// The reciprocal's seed: `1/(1 + (i + ½)/128)` for the top seven mantissa bits `i`.
pub(crate) const RECIP_SEED: [u32; 128] = [
    0x3f7f00ff, 0x3f7d08e5, 0x3f7b1885, 0x3f792fb2, 0x3f774e40, 0x3f757404, 0x3f73a0d5, 0x3f71d48c,
    0x3f700f01, 0x3f6e500f, 0x3f6c9791, 0x3f6ae564, 0x3f693965, 0x3f679373, 0x3f65f36d, 0x3f645933,
    0x3f62c4a7, 0x3f6135aa, 0x3f5fac1f, 0x3f5e27eb, 0x3f5ca8f1, 0x3f5b2f17, 0x3f59ba42, 0x3f584a5a,
    0x3f56df44, 0x3f5578e9, 0x3f541733, 0x3f52ba08, 0x3f516154, 0x3f500d01, 0x3f4ebcf9, 0x3f4d7127,
    0x3f4c2978, 0x3f4ae5d8, 0x3f49a634, 0x3f486a79, 0x3f473294, 0x3f45fe74, 0x3f44ce08, 0x3f43a13e,
    0x3f427806, 0x3f415250, 0x3f40300c, 0x3f3f112b, 0x3f3df59d, 0x3f3cdd53, 0x3f3bc841, 0x3f3ab656,
    0x3f39a786, 0x3f389bc3, 0x3f379301, 0x3f368d31, 0x3f358a48, 0x3f348a3a, 0x3f338cfa, 0x3f32927c,
    0x3f319ab6, 0x3f30a59b, 0x3f2fb322, 0x3f2ec33e, 0x3f2dd5e6, 0x3f2ceb10, 0x3f2c02b0, 0x3f2b1cbe,
    0x3f2a392f, 0x3f2957fb, 0x3f287917, 0x3f279c7b, 0x3f26c21e, 0x3f25e9f7, 0x3f2513fd, 0x3f244029,
    0x3f236e72, 0x3f229ecf, 0x3f21d13a, 0x3f2105a9, 0x3f203c17, 0x3f1f747a, 0x3f1eaecd, 0x3f1deb07,
    0x3f1d2922, 0x3f1c6917, 0x3f1baadf, 0x3f1aee73, 0x3f1a33cd, 0x3f197ae7, 0x3f18c3bb, 0x3f180e41,
    0x3f175a75, 0x3f16a850, 0x3f15f7cc, 0x3f1548e5, 0x3f149b93, 0x3f13efd2, 0x3f13459c, 0x3f129cec,
    0x3f11f5bd, 0x3f115009, 0x3f10abcc, 0x3f100901, 0x3f0f67a2, 0x3f0ec7ab, 0x3f0e2918, 0x3f0d8be3,
    0x3f0cf009, 0x3f0c5584, 0x3f0bbc51, 0x3f0b246b, 0x3f0a8dcd, 0x3f09f874, 0x3f09645c, 0x3f08d181,
    0x3f083fde, 0x3f07af70, 0x3f072033, 0x3f069223, 0x3f06053c, 0x3f05797c, 0x3f04eedd, 0x3f04655e,
    0x3f03dcf9, 0x3f0355ad, 0x3f02cf75, 0x3f024a4e, 0x3f01c636, 0x3f014328, 0x3f00c122, 0x3f004020,
];
/// The reciprocal square root's seed: `1/√(m·2^p)` at the centre of the top six mantissa bits' interval, `p` the
/// exponent's parity (`p·64 + i`).
pub(crate) const RSQRT_SEED: [u32; 128] = [
    0x3f7f017e, 0x3f7d0d3e, 0x3f7b2452, 0x3f79464e, 0x3f7772cb, 0x3f75a969, 0x3f73e9cb, 0x3f723399,
    0x3f708681, 0x3f6ee232, 0x3f6d4660, 0x3f6bb2c5, 0x3f6a271a, 0x3f68a31d, 0x3f672691, 0x3f65b139,
    0x3f6442db, 0x3f62db41, 0x3f617a36, 0x3f601f87, 0x3f5ecb04, 0x3f5d7c7f, 0x3f5c33cc, 0x3f5af0bf,
    0x3f59b330, 0x3f587af7, 0x3f5747ee, 0x3f5619f2, 0x3f54f0df, 0x3f53cc93, 0x3f52acee, 0x3f5191d1,
    0x3f507b1d, 0x3f4f68b6, 0x3f4e5a7f, 0x3f4d505e, 0x3f4c4a38, 0x3f4b47f4, 0x3f4a497a, 0x3f494eb3,
    0x3f485787, 0x3f4763e2, 0x3f4673ac, 0x3f4586d3, 0x3f449d42, 0x3f43b6e6, 0x3f42d3ad, 0x3f41f383,
    0x3f411659, 0x3f403c1c, 0x3f3f64bd, 0x3f3e902c, 0x3f3dbe58, 0x3f3cef34, 0x3f3c22b1, 0x3f3b58c0,
    0x3f3a9154, 0x3f39cc60, 0x3f3909d6, 0x3f3849aa, 0x3f378bd0, 0x3f36d03d, 0x3f3616e4, 0x3f355fba,
    0x3f3450fc, 0x3f32ef41, 0x3f319589, 0x3f304387, 0x3f2ef8f2, 0x3f2db587, 0x3f2c7904, 0x3f2b432a,
    0x3f2a13c0, 0x3f28ea8c, 0x3f27c759, 0x3f26a9f4, 0x3f25922c, 0x3f247fd3, 0x3f2372bd, 0x3f226abe,
    0x3f2167af, 0x3f206968, 0x3f1f6fc4, 0x3f1e7aa0, 0x3f1d89d9, 0x3f1c9d4e, 0x3f1bb4e1, 0x3f1ad073,
    0x3f19efe6, 0x3f191320, 0x3f183a05, 0x3f17647c, 0x3f16926c, 0x3f15c3bc, 0x3f14f857, 0x3f143026,
    0x3f136b13, 0x3f12a90b, 0x3f11e9f9, 0x3f112dca, 0x3f10746d, 0x3f0fbdce, 0x3f0f09dd, 0x3f0e5889,
    0x3f0da9c2, 0x3f0cfd79, 0x3f0c539f, 0x3f0bac25, 0x3f0b06fd, 0x3f0a641a, 0x3f09c36d, 0x3f0924ec,
    0x3f088889, 0x3f07ee37, 0x3f0755ed, 0x3f06bf9e, 0x3f062b40, 0x3f0598c7, 0x3f05082a, 0x3f04795f,
    0x3f03ec5c, 0x3f036117, 0x3f02d788, 0x3f024fa6, 0x3f01c967, 0x3f0144c4, 0x3f00c1b4, 0x3f004030,
];
/// `ln(c_j)` at the table points `c_j = 3/4 + j/64`, `j` in `0..=48`.
pub(crate) const LN_C: [u32; 49] = [
    0xbe934b11, 0xbe88bc74, 0xbe7cc8e3, 0xbe6881c0, 0xbe549f6a, 0xbe411e0b, 0xbe2dfa03, 0xbe1b2fe6,
    0xbe08bc74, 0xbded393b, 0xbdc99af3, 0xbda6988b, 0xbd842cc6, 0xbd44a551, 0xbd020aec, 0xbc8102b3,
    0x00000000, 0x3c7e0546, 0x3cfc14d8, 0x3d3ba2c8, 0x3d785186, 0x3d9a0ebd, 0x3db78694, 0x3dd4936a,
    0x3df1383b, 0x3e06bbf4, 0x3e14aa98, 0x3e22695b, 0x3e2ff984, 0x3e3d5c48, 0x3e4a92d5, 0x3e579e4a,
    0x3e647fbe, 0x3e71383b, 0x3e7dc8c3, 0x3e851927, 0x3e8b3ae5, 0x3e914a10, 0x3e974716, 0x3e9d3263,
    0x3ea30c5e, 0x3ea8d56c, 0x3eae8dee, 0x3eb43641, 0x3eb9cec0, 0x3ebf57c2, 0x3ec4d19c, 0x3eca3ca1,
    0x3ecf991f,
];
/// `1/c_j` at the table points `c_j = 3/4 + j/64`.
pub(crate) const INV_C: [u32; 49] = [
    0x3faaaaab, 0x3fa72f05, 0x3fa3d70a, 0x3fa0a0a1, 0x3f9d89d9, 0x3f9a90e8, 0x3f97b426, 0x3f94f209,
    0x3f924925, 0x3f8fb824, 0x3f8d3dcb, 0x3f8ad8f3, 0x3f888889, 0x3f864b8a, 0x3f842108, 0x3f820821,
    0x3f800000, 0x3f7c0fc1, 0x3f783e10, 0x3f74898d, 0x3f70f0f1, 0x3f6d7304, 0x3f6a0ea1, 0x3f66c2b4,
    0x3f638e39, 0x3f607038, 0x3f5d67c9, 0x3f5a740e, 0x3f579436, 0x3f54c77b, 0x3f520d21, 0x3f4f6475,
    0x3f4ccccd, 0x3f4a4588, 0x3f47ce0c, 0x3f4565c8, 0x3f430c31, 0x3f40c0c1, 0x3f3e82fa, 0x3f3c5264,
    0x3f3a2e8c, 0x3f381703, 0x3f360b61, 0x3f340b41, 0x3f321643, 0x3f302c0b, 0x3f2e4c41, 0x3f2c7692,
    0x3f2aaaab,
];

fn table(t: &[u32], i: usize) -> f32 {
    f32::from_bits(t[i])
}

/// `64/ln 2`, and `ln 2/64` split so a whole multiple of its high part below 2^14 is exact.
pub(crate) const INV_LN2_64: f32 = f32::from_bits(0x42b8_aa3b);
pub(crate) const LN2_64_HI: f32 = f32::from_bits(0x3c31_4000);
pub(crate) const LN2_64_LO: f32 = f32::from_bits(0x3748_5fdf);
/// `ln 2` split so a whole multiple of its high part below 2^11 is exact.
pub(crate) const LN2_HI: f32 = f32::from_bits(0x3f31_7000);
pub(crate) const LN2_LO: f32 = f32::from_bits(0x3805_fdf4);
pub(crate) const ONE_SIXTH: f32 = 1.0 / 6.0;
pub(crate) const ONE_THIRD: f32 = 1.0 / 3.0;
/// The bounds past which `exp` is infinite and zero.
pub(crate) const EXP_TOP: f32 = 88.8;
pub(crate) const EXP_BOTTOM: f32 = -104.0;
/// The bound past which `softplus(u)` is `u + e^-u`.
pub(crate) const SOFTPLUS_TOP: f32 = 16.0;

/// `e^x`: `x = n·ln2/64 + r` with `n` the nearest multiple, `e^r` by its cubic, times `2^((n mod 64)/64)` from the
/// table, times `2^(n div 64)`.
pub fn exp(fp: FpUnit, x: f32) -> f32 {
    let x = ftz(x);
    if x.is_nan() {
        return x;
    }
    if x > EXP_TOP {
        return f32::INFINITY;
    }
    if x < EXP_BOTTOM {
        return 0.0;
    }
    let n = rne(fp.mul(x, INV_LN2_64));
    let nf = n as f32;
    let r = fsub(fsub(x, fp.mul(nf, LN2_64_HI)), fp.mul(nf, LN2_64_LO));
    let p = fadd(1.0, fp.mul(r, fadd(1.0, fp.mul(r, fadd(0.5, fp.mul(r, ONE_SIXTH))))));
    let j = n.rem_euclid(64) as usize;
    let m = (n - j as i64) / 64;
    ldexp(fp.mul(table(&EXP2_64, j), p), m as i32)
}

/// `1/x`: the seed from the top seven mantissa bits, then two Newton steps `y·(2 − x·y)`.
pub fn recip(fp: FpUnit, x: f32) -> f32 {
    let x = ftz(x);
    if x.is_nan() {
        return x;
    }
    if x == 0.0 {
        return f32::INFINITY.copysign(x);
    }
    if x.is_infinite() {
        return 0f32.copysign(x);
    }
    let bits = x.to_bits();
    let e = ((bits >> 23) & 0xff) as i32 - 127;
    let i = ((bits >> 16) & 0x7f) as usize;
    let seed = ldexp(table(&RECIP_SEED, i), -e).copysign(x);
    let mut y = seed;
    for _ in 0..2 {
        y = fp.mul(y, fsub(2.0, fp.mul(x, y)));
    }
    y
}

/// `1/√x` for `x ≥ 0`: the seed from the exponent's parity and the top six mantissa bits, then two Newton steps
/// `y·(1.5 − ½·((x·y)·y))`, every intermediate near 1 so none flushes at the ends of the range.
pub fn rsqrt(fp: FpUnit, x: f32) -> f32 {
    let x = ftz(x);
    if x.is_nan() || x < 0.0 {
        return f32::NAN;
    }
    if x == 0.0 {
        return f32::INFINITY;
    }
    if x.is_infinite() {
        return 0.0;
    }
    let bits = x.to_bits();
    let e = ((bits >> 23) & 0xff) as i32 - 127;
    let parity = e.rem_euclid(2);
    let i = ((bits >> 17) & 0x3f) as usize;
    let seed = ldexp(table(&RSQRT_SEED, parity as usize * 64 + i), -(e - parity) / 2);
    let mut y = seed;
    for _ in 0..2 {
        y = fp.mul(y, fsub(1.5, fp.mul(0.5, fp.mul(fp.mul(x, y), y))));
    }
    y
}

/// `ln x` for `x > 0`: `x = m·2^E` with `m` in `[3/4, 3/2)`, so a value near 1 keeps `E = 0`; `m` against the
/// nearest table point `c_j`, and `ln(m/c_j)` by its quartic.
pub fn ln(fp: FpUnit, x: f32) -> f32 {
    let x = ftz(x);
    if x.is_nan() || x < 0.0 {
        return f32::NAN;
    }
    if x == 0.0 {
        return f32::NEG_INFINITY;
    }
    if x.is_infinite() {
        return x;
    }
    let bits = x.to_bits();
    let mut big_e = ((bits >> 23) & 0xff) as i32 - 127;
    let mut m = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000);
    if m >= 1.5 {
        // Halving a significand is exact.
        m *= 0.5;
        big_e += 1;
    }
    // `(m − 3/4)·64` is exact, and so is the table point `3/4 + j/64`.
    let j = rne((m - 0.75) * 64.0) as usize;
    let c = 0.75 + j as f32 / 64.0;
    let d = fp.mul(fsub(m, c), table(&INV_C, j));
    let d2 = fp.mul(d, d);
    let poly = fsub(d, fp.mul(d2, fsub(0.5, fp.mul(d, fsub(ONE_THIRD, fp.mul(d, 0.25))))));
    let ef = big_e as f32;
    fadd(fadd(fp.mul(ef, LN2_HI), fadd(table(&LN_C, j), poly)), fp.mul(ef, LN2_LO))
}

/// `ln(1 + t)` for `t ≥ 0`: `ln u` of `u = 1 + t` rounded, corrected by the rounding's own error
/// `(t − (u − 1))/u`, which is exact.
pub fn ln1p(fp: FpUnit, t: f32) -> f32 {
    let t = ftz(t);
    let u = fadd(1.0, t);
    if u == 1.0 {
        return t;
    }
    fadd(ln(fp, u), fp.mul(fsub(t, fsub(u, 1.0)), recip(fp, u)))
}

/// `ln(1 + e^u)`: `u` past 16 adds `e^-u`.
pub fn softplus(fp: FpUnit, u: f32) -> f32 {
    let u = ftz(u);
    if u > SOFTPLUS_TOP {
        return fadd(u, exp(fp, -u));
    }
    ln1p(fp, exp(fp, u))
}

pub fn sigmoid(fp: FpUnit, x: f32) -> f32 {
    recip(fp, fadd(1.0, exp(fp, -ftz(x))))
}

pub fn silu(fp: FpUnit, x: f32) -> f32 {
    fp.mul(x, sigmoid(fp, x))
}

/// RMSNorm: `x_i · rsqrt(Σx²/n + ε) · w_i`, the product in that order.
pub fn rmsnorm(fp: FpUnit, x: &[f32], w: &[f32], eps: f32) -> Vec<f32> {
    let n = x.len() as f32;
    let r = rsqrt(fp, fadd(fp.mul(fdot(fp, x, x), recip(fp, n)), eps));
    x.iter().zip(w).map(|(&v, &g)| fp.mul(fp.mul(v, r), g)).collect()
}

/// `x / √(Σx² + ε)`.
pub fn l2norm(fp: FpUnit, x: &[f32], eps: f32) -> Vec<f32> {
    let r = rsqrt(fp, fadd(fdot(fp, x, x), eps));
    x.iter().map(|&v| fp.mul(v, r)).collect()
}

/// The Walsh–Hadamard transform of `x` in place, unnormalized: for each stride `h` from 1 doubling to `len/2`, every
/// pair `(x[j], x[j+h])` of each `2h`-block becomes `(x[j] + x[j+h], x[j] − x[j+h])`, in index order. Applied twice
/// it is `len` times the identity, so `len` a power of two makes the inverse exact: the transform, then `2^-log2 len`.
pub fn hadamard(x: &mut [f32]) {
    let n = x.len();
    assert!(n.is_power_of_two(), "a Hadamard transform of {n} elements");
    let mut h = 1;
    while h < n {
        for start in (0..n).step_by(2 * h) {
            for j in start..start + h {
                let (a, b) = (x[j], x[j + h]);
                x[j] = fadd(a, b);
                x[j + h] = fsub(a, b);
            }
        }
        h *= 2;
    }
}

/// A head turned by a rotary table: pair `i` of the first `2·cos.len()` elements — `(i, i + cos.len())` under NEOX,
/// `(2i, 2i + 1)` otherwise — becomes `(x₀·cos_i − x₁·sin_i, x₀·sin_i + x₁·cos_i)`; the rest pass through.
pub fn rope(fp: FpUnit, v: &mut [f32], cos: &[f32], sin: &[f32], neox: bool) {
    let half = cos.len();
    assert!(sin.len() == half && 2 * half <= v.len(), "a rotary table of {half} pairs over {} elements", v.len());
    for i in 0..half {
        let (a, b) = if neox { (i, i + half) } else { (2 * i, 2 * i + 1) };
        let (x0, x1) = (v[a], v[b]);
        v[a] = fsub(fp.mul(x0, cos[i]), fp.mul(x1, sin[i]));
        v[b] = fadd(fp.mul(x0, sin[i]), fp.mul(x1, cos[i]));
    }
}

// ── Attention ───────────────────────────────────────────────────────────────────────────────────────

/// The softmax: the maximum ([`fmax`]), each `exp(s − max)`, their sum in lanes ([`fsum`]), one reciprocal, the
/// products.
pub fn softmax(fp: FpUnit, scores: &[f32]) -> Vec<f32> {
    let m = fmax(scores);
    let e: Vec<f32> = scores.iter().map(|&s| exp(fp, fsub(s, m))).collect();
    let inv = recip(fp, fsum(&e));
    e.iter().map(|&v| fp.mul(v, inv)).collect()
}

/// One query head's attention over its cached rows on the vector units: each position's score the head's dot with
/// the key times `scale`, the [`softmax`] of the scores, and each value element the probabilities' dot ([`fdot`],
/// lanes by position) with that element across the positions.
pub fn attend_vector(fp: FpUnit, q: &[f32], keys: &[&[f32]], values: &[&[f32]], scale: f32) -> Vec<f32> {
    let scores: Vec<f32> = keys.iter().map(|k| fp.mul(fdot(fp, q, k), scale)).collect();
    let p = softmax(fp, &scores);
    let width = values.first().map_or(0, |v| v.len());
    (0..width).map(|e| fdot(fp, &p, &values.iter().map(|v| v[e]).collect::<Vec<_>>())).collect()
}

/// A query head as the array reads it: per 32 elements (the head padded with zeros to a whole group), or the whole
/// head one block.
pub fn quantize_head(q: &[f32], words: Words, scaling: ActScaling) -> ActRow {
    match scaling {
        ActScaling::Sub32 => {
            let mut padded = q.to_vec();
            padded.resize(q.len().div_ceil(GROUP) * GROUP, 0.0);
            quantize(&padded, words)
        }
        ActScaling::PerBlock => quantize_blocks(q, words, q.len()),
    }
}

/// A score as the array computes it against a cached int8 key row (its f16 scale's bits and its codes): per activation
/// block of the query, its scale's `m` times the integer dot of its codes and the key's codes, times the key's scale,
/// summed exactly and rounded once.
pub fn int8_score(q: &ActRow, key_scale: u16, key: &[i8]) -> f32 {
    let scale = Scale::f16(key_scale).expect("a finite cache scale");
    let terms: Vec<(i128, i32)> = key
        .chunks(q.block)
        .enumerate()
        .map(|(b, block)| {
            let sum: i64 = block.iter().enumerate().map(|(i, &c)| q.codes[b * q.block + i] as i64 * c as i64).sum();
            let sc = q.scales[b];
            (scale.sig as i128 * sc.m as i128 * sum as i128, scale.exp + sc.e)
        })
        .filter(|t| t.0 != 0)
        .collect();
    exact_sum(&terms)
}

/// Positions a block of the array's weighted sum spans.
pub const VALUE_BLOCK: usize = 32;

/// The weighted sum on the array over cached int8 value rows (each its f16 scale's bits and its codes): each position's
/// probability times its row's scale, quantized per block of [`VALUE_BLOCK`] positions to 8-bit codes at a power-of-two
/// scale; per value element the block's exact integer sum against the value codes, rounded once a block; the blocks
/// added in order.
pub fn weighted_int8(fp: FpUnit, p: &[f32], values: &[(u16, &[i8])]) -> Vec<f32> {
    let width = values.first().map_or(0, |v| v.1.len());
    let mut out = vec![0f32; width];
    for (b, block) in p.chunks(VALUE_BLOCK).enumerate() {
        let rows = &values[b * VALUE_BLOCK..b * VALUE_BLOCK + block.len()];
        let w: Vec<f32> = block.iter().zip(rows).map(|(&pj, (s, _))| fp.mul(pj, f16_to_f32(*s))).collect();
        let wmax = w.iter().fold(0f32, |m, &v| m.max(v));
        if wmax == 0.0 {
            for o in out.iter_mut() {
                *o = fadd(*o, 0.0);
            }
            continue;
        }
        // The least e with wmax ≤ 255·2^e: the codes are unsigned 8-bit.
        let mut e = (wmax as f64 / 255.0).log2().ceil() as i32;
        while 255.0 * 2f64.powi(e) < wmax as f64 {
            e += 1;
        }
        while 255.0 * 2f64.powi(e - 1) >= wmax as f64 {
            e -= 1;
        }
        let codes: Vec<i64> = w.iter().map(|&v| (v as f64 * 2f64.powi(-e)).round_ties_even() as i64).collect();
        for (el, o) in out.iter_mut().enumerate() {
            let sum: i64 = codes.iter().zip(rows).map(|(&c, (_, v))| c * v[el] as i64).sum();
            *o = fadd(*o, scaled_to_f32(sum as i128, e));
        }
    }
    out
}

/// One query head's attention over its cached int8 rows on the array: the query quantized as `words` and `scaling`
/// name, each position's [`int8_score`] times `scale`, the [`softmax`], the [`weighted_int8`] sum.
pub fn attend_int8(fp: FpUnit, q: &[f32], keys: &[(u16, &[i8])], values: &[(u16, &[i8])], scale: f32, words: Words, scaling: ActScaling) -> Vec<f32> {
    let qa = quantize_head(q, words, scaling);
    let scores: Vec<f32> = keys.iter().map(|&(s, k)| fp.mul(int8_score(&qa, s, k), scale)).collect();
    weighted_int8(fp, &softmax(fp, &scores), values)
}

// ── The Gated-DeltaNet memory ───────────────────────────────────────────────────────────────────────

/// A Gated-DeltaNet block's causal convolution through SiLU, channel by channel: the ring's rows, oldest first,
/// against the channel's first taps, then the token's row `x` against its last, the products summed in tap order
/// from `+0`. `kernel` holds each channel's `taps` weights together.
pub fn conv(fp: FpUnit, ring: &[Vec<f32>], x: &[f32], kernel: &[f32], taps: usize) -> Vec<f32> {
    assert_eq!(ring.len() + 1, taps, "a ring row for every tap but the token's own");
    (0..x.len())
        .map(|ch| {
            let mut acc = 0f32;
            for (j, row) in ring.iter().enumerate() {
                acc = fadd(acc, fp.mul(row[ch], kernel[ch * taps + j]));
            }
            silu(fp, fadd(acc, fp.mul(x[ch], kernel[ch * taps + taps - 1])))
        })
        .collect()
}

/// One value row of a Gated-DeltaNet memory through one token's rank-one update: `coef = beta·(v − decay·(row·k))`,
/// each element `decay·m + coef·k` written back through `store`; returns `coef`.
pub fn gdn_update(fp: FpUnit, row: &mut [f32], k: &[f32], decay: f32, beta: f32, v: f32, store: impl Fn(f32) -> f32) -> f32 {
    let u = fp.mul(decay, fdot(fp, row, k));
    let coef = fp.mul(beta, fsub(v, u));
    for (m, &kc) in row.iter_mut().zip(k) {
        *m = store(fadd(fp.mul(decay, *m), fp.mul(coef, kc)));
    }
    coef
}

/// One value row of a Gated-DeltaNet memory through one token, in one pass over the row: its dots with the token's
/// key and query, the rank-one update ([`gdn_update`]), and the row's read-out `(decay·(row·q) + coef·(k·q))·scale`
/// of the row as it was. `kq` is the head's `k·q`.
#[allow(clippy::too_many_arguments)]
pub fn gdn_row(fp: FpUnit, row: &mut [f32], k: &[f32], q: &[f32], decay: f32, beta: f32, v: f32, kq: f32, scale: f32, store: impl Fn(f32) -> f32) -> f32 {
    let sq = fdot(fp, row, q);
    let coef = gdn_update(fp, row, k, decay, beta, v, store);
    fp.mul(fadd(fp.mul(decay, sq), fp.mul(coef, kq)), scale)
}

/// One logged token as a lazily flushed head reads it: its key group's key, the head's values, the exponent of its
/// decay and its update gate, as the log stores them.
#[derive(Clone, Copy, Debug)]
pub struct Logged<'a> {
    pub k: &'a [f32],
    pub v: &'a [f32],
    pub log_decay: f32,
    pub beta: f32,
}

/// A query folded back through a lazily flushed head's log, newest token first: per token, its gate times the key's
/// dot with the folded query; that coefficient times the token's values collected into `width` sums; the query decayed
/// past the token's update, `y ← decay·(y − coef·k)`. Returns the folded query and the collected values.
pub fn gdn_fold(fp: FpUnit, q: &[f32], log: &[Logged], width: usize) -> (Vec<f32>, Vec<f32>) {
    fold_on(fp, q.to_vec(), vec![0f32; width], log)
}

/// [`gdn_fold`] over a log whose oldest `older` tokens a trailing flush is replaying, in one walk: first through the
/// newer tokens alone, the fold a row the flush has replayed reads, then on through the older ones, the fold every
/// other row reads. The walk is newest first, so the first is where the second has reached after the newer tokens:
/// each is `gdn_fold` over its own tokens, bit for bit.
pub fn gdn_fold_trailing(fp: FpUnit, q: &[f32], log: &[Logged], width: usize, older: usize) -> [(Vec<f32>, Vec<f32>); 2] {
    let newer = gdn_fold(fp, q, &log[older..], width);
    let all = fold_on(fp, newer.0.clone(), newer.1.clone(), &log[..older]);
    [newer, all]
}

/// A fold carried on from the folded query `y` and the collected values `carried` through `log`, newest token first.
fn fold_on(fp: FpUnit, mut y: Vec<f32>, mut carried: Vec<f32>, log: &[Logged]) -> (Vec<f32>, Vec<f32>) {
    for e in log.iter().rev() {
        let decay = exp(fp, e.log_decay);
        let bc = fp.mul(e.beta, fdot(fp, e.k, &y));
        for (slot, &v) in carried.iter_mut().zip(e.v) {
            *slot = fadd(*slot, fp.mul(bc, v));
        }
        for (yi, &kc) in y.iter_mut().zip(e.k) {
            *yi = fp.mul(decay, fsub(*yi, fp.mul(bc, kc)));
        }
    }
    (y, carried)
}

/// A lazily flushed state row's read-out through its head's folded query: `(row·y + carried)·scale`.
pub fn gdn_read(fp: FpUnit, row: &[f32], y: &[f32], carried: f32, scale: f32) -> f32 {
    fp.mul(fadd(fdot(fp, row, y), carried), scale)
}

/// A lazily flushed state row's flush: every logged update replayed over the row in order ([`gdn_update`] with
/// value `r` of each token), exactly as the token-by-token memory makes them, and the row stored once through `store`.
pub fn gdn_flush(fp: FpUnit, row: &mut [f32], log: &[Logged], r: usize, store: impl Fn(f32) -> f32) {
    let mut s = row.to_vec();
    for e in log {
        gdn_update(fp, &mut s, e.k, exp(fp, e.log_decay), e.beta, e.v[r], |x| x);
    }
    for (m, x) in row.iter_mut().zip(s) {
        *m = store(x);
    }
}

/// `1/127`, the int8 cache's scale per unit of its row's largest magnitude.
pub const INV_127: f32 = f32::from_bits(0x3c01_0204);

/// The int8 codes of `x` under the inverse scale `inv`: each `x·inv` to the nearest integer, ties to even, within
/// ±127, a NaN product zero; every code zero when `inv` is infinite, the reciprocal of a scale that rounded to zero.
pub fn int8_codes(fp: FpUnit, x: &[f32], inv: f32) -> Vec<i8> {
    if inv.is_infinite() {
        return vec![0; x.len()];
    }
    x.iter().map(|&v| rne(fp.mul(v, inv)).clamp(-127, 127) as i8).collect()
}

/// A cache row stored as int8 codes under one f16 scale `amax/127` ([`absmax`]): the scale's bits and the codes under
/// its reciprocal ([`int8_codes`]), all zero when the scale rounds to zero.
pub fn cache_row(fp: FpUnit, x: &[f32]) -> (u16, Vec<i8>) {
    let amax = absmax(x);
    assert!(amax.is_finite(), "a non-finite cache row");
    let s = f32_to_f16(fp.mul(amax, INV_127));
    (s, int8_codes(fp, x, recip(fp, f16_to_f32(s))))
}
