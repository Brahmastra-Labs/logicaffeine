//! IEEE-754 binary64 on integer lanes: the VM's `Float` arithmetic, exact to the bit, for devices whose
//! silicon has no binary64 unit.
//!
//! Every operation unpacks to sign, exponent and a normalized 53-bit significand, computes the exact
//! significand result with the multi-word core (wide products, restoring division, digit-by-digit square
//! root), and rounds once to nearest-even in [`round_pack`], which also produces subnormals, overflow to
//! infinity, and the signed zeros. Special operands (NaN, infinities, zeros) are resolved by selects beside
//! the finite path. A NaN result follows the x86 rule the host runs by: the first NaN operand, quieted, or
//! the default NaN `0xFFF8_0000_0000_0000` for an invalid operation.
//!
//! Values are 64-bit words of this builder's lanes (`int64::n64`); exponents are 64-bit two's complement.

use super::int64::n64;
use super::multiword::{Builder, Mw};

/// The x86 default NaN an invalid operation produces.
pub const DEFAULT_NAN: u64 = 0xFFF8_0000_0000_0000;

/// A binary64 operand taken apart.
pub struct Unpacked {
    pub bits: Mw,
    pub sign: u32,
    pub is_nan: u32,
    pub is_inf: u32,
    pub is_zero: u32,
    /// For a finite nonzero operand: the unbiased exponent of the leading significand bit.
    pub exp: Mw,
    /// For a finite nonzero operand: the significand normalized to `[2^52, 2^53)`.
    pub sig: Mw,
}

fn c64(b: &mut Builder, v: u64) -> Mw {
    let n = n64(b);
    b.mw_const(v as u128, n)
}

/// Take `x` apart. Subnormals are normalized here, so every finite nonzero operand leaves with its leading
/// significand bit at 52.
pub fn unpack(b: &mut Builder, x: &Mw) -> Unpacked {
    let n = x.len();
    let sign = b.mw_top_bit(x);
    let e_field = {
        let s = b.mw_shr(x, 52);
        let m = c64(b, 0x7FF);
        b.mw_and(&s, &m)
    };
    let frac = {
        let m = c64(b, (1u64 << 52) - 1);
        b.mw_and(x, &m)
    };
    let zero = b.mw_zero(n);
    let e_zero = b.mw_eq(&e_field, &zero);
    let e_max_c = c64(b, 0x7FF);
    let e_max = b.mw_eq(&e_field, &e_max_c);
    let f_zero = b.mw_is_zero(&frac);
    let f_nonzero = b.bit_not(f_zero);
    let is_nan = b.and(e_max, f_nonzero);
    let is_inf = b.and(e_max, f_zero);
    let is_zero = b.and(e_zero, f_zero);
    let hidden = c64(b, 1u64 << 52);
    let with_hidden = b.mw_or(&frac, &hidden);
    let raw_sig = b.mw_sel(e_zero, &frac, &with_hidden);
    let one = c64(b, 1);
    let raw_exp_biased = b.mw_sel(e_zero, &one, &e_field);
    let bias = c64(b, 1023);
    let raw_exp = b.mw_minus(&raw_exp_biased, &bias);
    let nc = b.shape().words_for(16);
    let lz = b.mw_clz(&raw_sig, nc);
    let lz64 = b.mw_resize(&lz, n);
    let eleven = c64(b, 11);
    let norm_shift = b.mw_minus(&lz64, &eleven);
    let norm_shift = b.mw_sel(is_zero, &zero, &norm_shift);
    let sig = b.mw_shl_var(&raw_sig, &norm_shift, 7);
    let exp = b.mw_minus(&raw_exp, &norm_shift);
    Unpacked {
        bits: x.clone(),
        sign,
        is_nan,
        is_inf,
        is_zero,
        exp,
        sig,
    }
}

/// `x` with its quiet bit set.
fn quiet(b: &mut Builder, x: &Mw) -> Mw {
    let q = c64(b, 1u64 << 51);
    b.mw_or(x, &q)
}

/// The NaN a two-operand operation returns when an operand is NaN: the first NaN, quieted.
fn propagate(b: &mut Builder, x: &Unpacked, y: &Unpacked) -> Mw {
    let qx = quiet(b, &x.bits);
    let qy = quiet(b, &y.bits);
    b.mw_sel(x.is_nan, &qx, &qy)
}

/// `±∞`.
fn infinity(b: &mut Builder, sign: u32) -> Mw {
    let pos = c64(b, 0x7FF0_0000_0000_0000);
    let neg = c64(b, 0xFFF0_0000_0000_0000);
    b.mw_sel(sign, &neg, &pos)
}

/// `±0`.
fn signed_zero(b: &mut Builder, sign: u32) -> Mw {
    let pos = c64(b, 0);
    let neg = c64(b, 1u64 << 63);
    b.mw_sel(sign, &neg, &pos)
}

/// Round `sig × 2^(exp − 62)` to binary64, nearest-even, and pack it with `sign`. `sig` has its leading bit
/// at 62 (a nonzero significand), with any lost low bits folded into bit 0 as a sticky bit. The one rounding
/// every operation performs: normal results, gradual underflow to subnormals, and overflow to infinity.
pub fn round_pack(b: &mut Builder, sign: u32, exp: &Mw, sig: &Mw) -> Mw {
    let n = sig.len();
    let bias = c64(b, 1023);
    let be = b.mw_plus(exp, &bias);
    let zero = b.mw_zero(n);
    let one = c64(b, 1);
    let be_pos = {
        let le_zero = b.mw_lts(&be, &one);
        b.bit_not(le_zero)
    };
    let under = b.mw_minus(&one, &be);
    let cap = c64(b, 64);
    let under_capped = {
        let big = b.mw_lts(&cap, &under);
        b.mw_sel(big, &cap, &under)
    };
    let shift = b.mw_sel(be_pos, &zero, &under_capped);
    let (shifted, lost) = b.mw_shr_var_sticky(sig, &shift, 7);
    let lost64 = b.mw_from_word(lost, n);
    let jammed = b.mw_or(&shifted, &lost64);
    let be_minus_1 = b.mw_minus(&be, &one);
    let exp_field = b.mw_sel(be_pos, &be_minus_1, &zero);
    let m = b.mw_shr(&jammed, 10);
    let rmask = c64(b, 0x3FF);
    let r = b.mw_and(&jammed, &rmask);
    let half = c64(b, 0x200);
    let above = b.mw_ltu(&half, &r);
    let at_half = b.mw_eq(&r, &half);
    let lsb = b.mw_bit(&m, 0);
    let tie_up = b.and(at_half, lsb);
    let inc = b.or(above, tie_up);
    let (m_rounded, _) = b.mw_inc(&m, inc);
    let placed_exp = b.mw_shl(&exp_field, 52);
    let magnitude = b.mw_plus(&placed_exp, &m_rounded);
    let sign_bit = {
        let s = b.mw_from_word(sign, n);
        b.mw_shl(&s, 63)
    };
    let packed = b.mw_or(&magnitude, &sign_bit);
    let limit = c64(b, 0x7FF);
    let overflow = {
        let lt = b.mw_lts(&be, &limit);
        b.bit_not(lt)
    };
    let inf = infinity(b, sign);
    b.mw_sel(overflow, &inf, &packed)
}

/// Shift a nonzero `sig` so its leading bit sits at 62, returning it with the amount `exp` rises by (`p − 62`
/// for a leading bit at `p`); the sticky bit rides into bit 0 when bits fall off the bottom.
fn normalize_to_62(b: &mut Builder, sig: &Mw) -> (Mw, Mw) {
    let n = sig.len();
    let nc = b.shape().words_for(16);
    let lz = b.mw_clz(sig, nc);
    let lz64 = b.mw_resize(&lz, n);
    let one = c64(b, 1);
    let top_set = b.mw_top_bit(sig);
    let (down, lost) = b.mw_shr_sticky(sig, 1);
    let lost64 = b.mw_from_word(lost, n);
    let down = b.mw_or(&down, &lost64);
    let left = b.mw_minus(&lz64, &one);
    let up = b.mw_shl_var(sig, &left, 7);
    let normalized = b.mw_sel(top_set, &down, &up);
    let rise = b.mw_minus(&one, &lz64);
    (normalized, rise)
}

/// `a + b` (`negate_b` makes it `a − b`).
pub fn add(b: &mut Builder, x: &Mw, y: &Mw, negate_b: bool) -> Mw {
    let n = x.len();
    let ux = unpack(b, x);
    let uy = unpack(b, y);
    let sy = if negate_b {
        b.bit_not(uy.sign)
    } else {
        uy.sign
    };
    let zero = b.mw_zero(n);
    // Order by magnitude: `big` has the larger (exponent, significand).
    let ex_gt = b.mw_lts(&uy.exp, &ux.exp);
    let ex_eq = b.mw_eq(&ux.exp, &uy.exp);
    let sig_ge = {
        let lt = b.mw_ltu(&ux.sig, &uy.sig);
        b.bit_not(lt)
    };
    let tie_ge = b.and(ex_eq, sig_ge);
    let x_big = b.or(ex_gt, tie_ge);
    let big_exp = b.mw_sel(x_big, &ux.exp, &uy.exp);
    let small_exp = b.mw_sel(x_big, &uy.exp, &ux.exp);
    let big_sig = b.mw_sel(x_big, &ux.sig, &uy.sig);
    let small_sig = b.mw_sel(x_big, &uy.sig, &ux.sig);
    let big_sign = b.sel(x_big, ux.sign, sy);
    let small_sign = b.sel(x_big, sy, ux.sign);
    let d = b.mw_minus(&big_exp, &small_exp);
    let cap = c64(b, 64);
    let d_capped = {
        let big = b.mw_lts(&cap, &d);
        b.mw_sel(big, &cap, &d)
    };
    let big9 = b.mw_shl(&big_sig, 9);
    let small9 = b.mw_shl(&small_sig, 9);
    let (aligned, lost) = b.mw_shr_var_sticky(&small9, &d_capped, 7);
    let lost64 = b.mw_from_word(lost, n);
    let aligned = b.mw_or(&aligned, &lost64);
    let same_sign = {
        let differ = b.xor(big_sign, small_sign);
        b.bit_not(differ)
    };
    let sum = b.mw_plus(&big9, &aligned);
    let diff = b.mw_minus(&big9, &aligned);
    let raw = b.mw_sel(same_sign, &sum, &diff);
    let raw_zero = b.mw_is_zero(&raw);
    let (norm, rise) = normalize_to_62(b, &raw);
    // `raw`'s leading bit at 61 stands for 2^big_exp; `rise` counts from 62, so one less.
    let one = c64(b, 1);
    let e1 = b.mw_plus(&big_exp, &rise);
    let e = b.mw_plus(&e1, &one);
    let finite = round_pack(b, big_sign, &e, &norm);
    // An exact zero from opposite signs is +0; from two zeros, −0 only when both are −0.
    let pos_zero = c64(b, 0);
    // One operand zero: the other one exactly.
    let y_signed = {
        let flip = if negate_b {
            c64(b, 1u64 << 63)
        } else {
            b.mw_zero(n)
        };
        b.mw_xor(y, &flip)
    };
    let both_zero = b.and(ux.is_zero, uy.is_zero);
    let zeros_sign = b.and(ux.sign, sy);
    let zz = signed_zero(b, zeros_sign);
    let mut out = b.mw_sel(raw_zero, &pos_zero, &finite);
    out = b.mw_sel(uy.is_zero, x, &out);
    out = b.mw_sel(ux.is_zero, &y_signed, &out);
    out = b.mw_sel(both_zero, &zz, &out);
    // Infinities: ∞ − ∞ is invalid; otherwise the infinite operand.
    let both_inf = b.and(ux.is_inf, uy.is_inf);
    let opposite = b.xor(ux.sign, sy);
    let invalid = b.and(both_inf, opposite);
    let inf_x = infinity(b, ux.sign);
    let inf_y = infinity(b, sy);
    out = b.mw_sel(uy.is_inf, &inf_y, &out);
    out = b.mw_sel(ux.is_inf, &inf_x, &out);
    let dnan = c64(b, DEFAULT_NAN);
    out = b.mw_sel(invalid, &dnan, &out);
    let any_nan = b.or(ux.is_nan, uy.is_nan);
    let nan = propagate(b, &ux, &uy);
    let _ = zero;
    b.mw_sel(any_nan, &nan, &out)
}

/// `a × b`.
pub fn mul(b: &mut Builder, x: &Mw, y: &Mw) -> Mw {
    let n = x.len();
    let ux = unpack(b, x);
    let uy = unpack(b, y);
    let sign = b.xor(ux.sign, uy.sign);
    let p = b.mw_mul(&ux.sig, &uy.sig);
    let top = b.mw_bit(&p, 105);
    let (s43, l43) = b.mw_shr_sticky(&p, 43);
    let (s42, l42) = b.mw_shr_sticky(&p, 42);
    let s43 = s43.slice(0, n);
    let s42 = s42.slice(0, n);
    let lost = b.sel(top, l43, l42);
    let chosen = b.mw_sel(top, &s43, &s42);
    let lost64 = b.mw_from_word(lost, n);
    let sig = b.mw_or(&chosen, &lost64);
    let e = b.mw_plus(&ux.exp, &uy.exp);
    let top64 = b.mw_from_word(top, n);
    let e = b.mw_plus(&e, &top64);
    let finite = round_pack(b, sign, &e, &sig);
    let zero = signed_zero(b, sign);
    let inf = infinity(b, sign);
    let any_zero = b.or(ux.is_zero, uy.is_zero);
    let any_inf = b.or(ux.is_inf, uy.is_inf);
    let invalid = b.and(any_zero, any_inf);
    let mut out = b.mw_sel(any_zero, &zero, &finite);
    out = b.mw_sel(any_inf, &inf, &out);
    let dnan = c64(b, DEFAULT_NAN);
    out = b.mw_sel(invalid, &dnan, &out);
    let any_nan = b.or(ux.is_nan, uy.is_nan);
    let nan = propagate(b, &ux, &uy);
    b.mw_sel(any_nan, &nan, &out)
}

/// `a ÷ b`. A zero divisor yields the IEEE result (∞, or NaN for `0 ÷ 0`); the VM's failure there is the
/// cell's divergence, reported beside this value.
pub fn div(b: &mut Builder, x: &Mw, y: &Mw) -> Mw {
    let n = x.len();
    let ux = unpack(b, x);
    let uy = unpack(b, y);
    let sign = b.xor(ux.sign, uy.sign);
    // The numerator `sig_x << 62` over the divisor, as 128-bit values; a zero divisor divides by one.
    let wide = 2 * n;
    let num = b.mw_resize(&ux.sig, wide);
    let num = b.mw_shl(&num, 62);
    let one_w = b.mw_const(1, wide);
    let den = b.mw_resize(&uy.sig, wide);
    let den_zero = b.mw_is_zero(&den);
    let den = b.mw_sel(den_zero, &one_w, &den);
    let (q, r) = b.mw_divrem(&num, &den);
    let r_nonzero = {
        let z = b.mw_is_zero(&r);
        b.bit_not(z)
    };
    let q = q.slice(0, n);
    let sticky = b.mw_from_word(r_nonzero, n);
    let q = b.mw_or(&q, &sticky);
    let (norm, rise) = normalize_to_62(b, &q);
    let e = b.mw_minus(&ux.exp, &uy.exp);
    let e = b.mw_plus(&e, &rise);
    let finite = round_pack(b, sign, &e, &norm);
    let zero = signed_zero(b, sign);
    let inf = infinity(b, sign);
    let both_zero = b.and(ux.is_zero, uy.is_zero);
    let both_inf = b.and(ux.is_inf, uy.is_inf);
    let invalid = b.or(both_zero, both_inf);
    let mut out = b.mw_sel(uy.is_inf, &zero, &finite);
    out = b.mw_sel(ux.is_zero, &zero, &out);
    out = b.mw_sel(uy.is_zero, &inf, &out);
    out = b.mw_sel(ux.is_inf, &inf, &out);
    let dnan = c64(b, DEFAULT_NAN);
    out = b.mw_sel(invalid, &dnan, &out);
    let any_nan = b.or(ux.is_nan, uy.is_nan);
    let nan = propagate(b, &ux, &uy);
    b.mw_sel(any_nan, &nan, &out)
}

/// `√a`.
pub fn sqrt(b: &mut Builder, x: &Mw) -> Mw {
    let n = x.len();
    let ux = unpack(b, x);
    // Make the exponent even: an odd exponent moves one bit into the significand.
    let odd = b.mw_bit(&ux.exp, 0);
    let one = c64(b, 1);
    let odd64 = b.mw_from_word(odd, n);
    let e_even = b.mw_minus(&ux.exp, &odd64);
    let sig2 = b.mw_shl(&ux.sig, 1);
    let m = b.mw_sel(odd, &sig2, &ux.sig);
    // √(m · 2^72) has its leading bit at 62 for m in [2^52, 2^54).
    let wide = 2 * n;
    let rad = b.mw_resize(&m, wide);
    let rad = b.mw_shl(&rad, 72);
    let rad_state = b.mw_state(&rad);
    let zero_w = b.mw_zero(wide);
    let rem = b.mw_state(&zero_w);
    let root = b.mw_state(&zero_w);
    b.repeat(64, |s| {
        let rem4 = s.mw_shl(&rem, 2);
        let top2 = s.mw_shr(&rad_state, 126);
        let rem_in = s.mw_or(&rem4, &top2);
        let rad2 = s.mw_shl(&rad_state, 2);
        let root4 = s.mw_shl(&root, 2);
        let one_w = s.mw_const(1, wide);
        let trial = s.mw_or(&root4, &one_w);
        let z = s.k(0);
        let (d, borrow) = s.mw_sub(&rem_in, &trial, z);
        let fits = s.bit_not(borrow);
        let rem2 = s.mw_sel(fits, &d, &rem_in);
        let root2 = s.mw_shl(&root, 1);
        let fits_w = s.mw_from_word(fits, wide);
        let root3 = s.mw_or(&root2, &fits_w);
        s.mw_assign(&rem, &rem2);
        s.mw_assign(&root, &root3);
        s.mw_assign(&rad_state, &rad2);
    });
    let r_nonzero = {
        let z = b.mw_is_zero(&rem);
        b.bit_not(z)
    };
    let sig = root.slice(0, n);
    let sticky = b.mw_from_word(r_nonzero, n);
    let sig = b.mw_or(&sig, &sticky);
    let e = b.mw_sar(&e_even, 1);
    let positive = b.k(0);
    let finite = round_pack(b, positive, &e, &sig);
    let neg_nonzero = {
        let nz = b.bit_not(ux.is_zero);
        b.and(ux.sign, nz)
    };
    let pos_inf = c64(b, f64::INFINITY.to_bits());
    let mut out = b.mw_sel(ux.is_inf, &pos_inf, &finite);
    out = b.mw_sel(ux.is_zero, x, &out);
    let dnan = c64(b, DEFAULT_NAN);
    out = b.mw_sel(neg_nonzero, &dnan, &out);
    let qx = quiet(b, x);
    let _ = one;
    b.mw_sel(ux.is_nan, &qx, &out)
}

/// The IEEE comparisons as 0/1: `(a < b, a ≤ b, a == b)`. NaN compares false; `−0 == +0`.
pub fn compare(b: &mut Builder, x: &Mw, y: &Mw) -> (u32, u32, u32) {
    let ux = unpack(b, x);
    let uy = unpack(b, y);
    let any_nan = b.or(ux.is_nan, uy.is_nan);
    let ordered = b.bit_not(any_nan);
    let both_zero = b.and(ux.is_zero, uy.is_zero);
    let same_bits = b.mw_eq(x, y);
    let eq_raw = b.or(same_bits, both_zero);
    let eq = b.and(ordered, eq_raw);
    let differ = b.xor(ux.sign, uy.sign);
    let mag_x = {
        let m = c64(b, u64::MAX >> 1);
        b.mw_and(x, &m)
    };
    let mag_y = {
        let m = c64(b, u64::MAX >> 1);
        b.mw_and(y, &m)
    };
    let x_lt_mag = b.mw_ltu(&mag_x, &mag_y);
    let y_lt_mag = b.mw_ltu(&mag_y, &mag_x);
    let same_sign_lt = b.sel(ux.sign, y_lt_mag, x_lt_mag);
    let lt_raw = b.sel(differ, ux.sign, same_sign_lt);
    let not_zeros = b.bit_not(both_zero);
    let lt1 = b.and(lt_raw, not_zeros);
    let lt = b.and(lt1, ordered);
    let le = b.or(lt, eq);
    (lt, le, eq)
}

/// `i64 as f64`, nearest-even.
pub fn from_int(b: &mut Builder, a: &Mw) -> Mw {
    let n = a.len();
    let sign = b.mw_top_bit(a);
    let neg = b.mw_neg(a);
    let mag = b.mw_sel(sign, &neg, a);
    let is_zero = b.mw_is_zero(a);
    let (norm, rise) = normalize_to_62(b, &mag);
    let e62 = c64(b, 62);
    let e = b.mw_plus(&e62, &rise);
    let finite = round_pack(b, sign, &e, &norm);
    let z = b.mw_zero(n);
    b.mw_sel(is_zero, &z, &finite)
}

/// `f.floor() as i64`: NaN is 0, and the conversion saturates at the `i64` bounds.
pub fn floor_to_int(b: &mut Builder, x: &Mw) -> Mw {
    let n = x.len();
    let ux = unpack(b, x);
    let zero = b.mw_zero(n);
    let minus_one = c64(b, u64::MAX);
    let i_max = c64(b, i64::MAX as u64);
    let i_min = c64(b, i64::MIN as u64);
    // |x| < 1: 0 for x ≥ 0 (and −0), −1 for a negative x.
    let e_neg = b.mw_top_bit(&ux.exp);
    let small = b.mw_sel(ux.sign, &minus_one, &zero);
    // 0 ≤ e ≤ 62: the integer part of sig · 2^(e − 52), and whether a fraction was dropped.
    let c52 = c64(b, 52);
    let down_amt = b.mw_minus(&c52, &ux.exp);
    let up_amt = b.mw_minus(&ux.exp, &c52);
    let e_le_52 = {
        let gt = b.mw_lts(&c52, &ux.exp);
        b.bit_not(gt)
    };
    let (down, frac) = b.mw_shr_var_sticky(&ux.sig, &down_amt, 6);
    let up = b.mw_shl_var(&ux.sig, &up_amt, 4);
    let int_part = b.mw_sel(e_le_52, &down, &up);
    let had_frac = b.and(e_le_52, frac);
    let neg_int = b.mw_neg(&int_part);
    let (neg_floor, _) = {
        let z = b.k(0);
        let dec = b.mw_from_word(had_frac, n);
        b.mw_sub(&neg_int, &dec, z)
    };
    let mid = b.mw_sel(ux.sign, &neg_floor, &int_part);
    // e ≥ 63: beyond i64 (−2^63 itself saturates to MIN too).
    let c63 = c64(b, 63);
    let huge = {
        let lt = b.mw_lts(&ux.exp, &c63);
        b.bit_not(lt)
    };
    let sat = b.mw_sel(ux.sign, &i_min, &i_max);
    let mut out = b.mw_sel(e_neg, &small, &mid);
    out = b.mw_sel(huge, &sat, &out);
    out = b.mw_sel(ux.is_inf, &sat, &out);
    out = b.mw_sel(ux.is_zero, &zero, &out);
    b.mw_sel(ux.is_nan, &zero, &out)
}

/// `|x|`.
pub fn abs(b: &mut Builder, x: &Mw) -> Mw {
    let m = c64(b, u64::MAX >> 1);
    b.mw_and(x, &m)
}

/// `f64::max(x, y)` for operands that are not NaN: `y` when `x < y`, else `x`.
fn max_ordered(b: &mut Builder, x: &Mw, y: &Mw) -> Mw {
    let (lt, _, _) = compare(b, x, y);
    b.mw_sel(lt, y, x)
}

/// The VM's `is approximately` on two integers: both converted to `f64`, equal, or apart by at most
/// `max(1e-9 · max(|a|, |b|), 1e-12)`.
pub fn approx_eq_ints(b: &mut Builder, x: &Mw, y: &Mw) -> u32 {
    let fx = from_int(b, x);
    let fy = from_int(b, y);
    let (_, _, eq) = compare(b, &fx, &fy);
    let d = add(b, &fx, &fy, true);
    let diff = abs(b, &d);
    let ax = abs(b, &fx);
    let ay = abs(b, &fy);
    let m = max_ordered(b, &ax, &ay);
    let tiny = c64(b, 1e-9f64.to_bits());
    let scaled = mul(b, &tiny, &m);
    let floor = c64(b, 1e-12f64.to_bits());
    let bound = max_ordered(b, &scaled, &floor);
    let (_, within, _) = compare(b, &diff, &bound);
    b.or(eq, within)
}
