//! The VM's 64-bit integer operations on narrow lanes: each is the checked semantics of
//! `fabric_rtl::{AluOp, FuncUnit, CmpKind}::eval_checked`, built from the multi-word core. A result that
//! leaves `i64`, a zero divisor or an exponent the VM refuses is reported in the fault word as the
//! divergence's code, never as a wrapped value.

use crate::codegen_sva::fabric_rtl::Divergence;

use super::multiword::{Builder, Mw};

/// The bits of a routine's fault word: 0, or a [`Divergence`] code.
pub const FAULT_BITS: u32 = 8;

/// The words of a 64-bit value on this builder's lanes.
pub fn n64(b: &Builder) -> usize {
    b.shape().words_for(64)
}

/// The fault words of this builder's lanes.
pub fn nfault(b: &Builder) -> usize {
    b.shape().words_for(FAULT_BITS)
}

/// A fault word that is `d`'s code when `cond` is 1, else `otherwise`.
pub fn fault_when(b: &mut Builder, cond: u32, d: Divergence, otherwise: &Mw) -> Mw {
    let n = otherwise.len();
    let code = b.mw_const(d.code() as u128, n);
    b.mw_sel(cond, &code, otherwise)
}

/// No fault.
pub fn no_fault(b: &mut Builder) -> Mw {
    let n = nfault(b);
    b.mw_zero(n)
}

/// `(|a|, sign)` of a two's-complement value; `|MIN|` is `2^63`, exact in the unsigned width.
pub fn magnitude(b: &mut Builder, a: &Mw) -> (Mw, u32) {
    let s = b.mw_top_bit(a);
    let neg = b.mw_neg(a);
    (b.mw_sel(s, &neg, a), s)
}

/// `a + b`, faulting where the VM promotes.
pub fn add(b: &mut Builder, x: &Mw, y: &Mw) -> (Mw, Mw) {
    let z = b.k(0);
    let (s, _) = b.mw_add(x, y, z);
    let sx = b.mw_top_bit(x);
    let sy = b.mw_top_bit(y);
    let ss = b.mw_top_bit(&s);
    let same = b.xor(sx, sy);
    let same = b.bit_not(same);
    let flipped = b.xor(sx, ss);
    let ovf = b.and(same, flipped);
    let ok = no_fault(b);
    (s, fault_when(b, ovf, Divergence::IntOverflow, &ok))
}

/// `a − b`, faulting where the VM promotes.
pub fn sub(b: &mut Builder, x: &Mw, y: &Mw) -> (Mw, Mw) {
    let z = b.k(0);
    let (d, _) = b.mw_sub(x, y, z);
    let sx = b.mw_top_bit(x);
    let sy = b.mw_top_bit(y);
    let sd = b.mw_top_bit(&d);
    let differ = b.xor(sx, sy);
    let flipped = b.xor(sx, sd);
    let ovf = b.and(differ, flipped);
    let ok = no_fault(b);
    (d, fault_when(b, ovf, Divergence::IntOverflow, &ok))
}

/// The signed product of two 64-bit values and 1 when it leaves `i64`.
pub fn mul_checked(b: &mut Builder, x: &Mw, y: &Mw) -> (Mw, u32) {
    let n = x.len();
    let (mx, sx) = magnitude(b, x);
    let (my, sy) = magnitude(b, y);
    let p = b.mw_mul(&mx, &my);
    let lo = p.slice(0, n);
    let hi = p.slice(n, 2 * n);
    let neg = b.xor(sx, sy);
    let hi_clear = b.mw_is_zero(&hi);
    let top = b.mw_top_bit(&lo);
    let min_mag = b.mw_const(1u128 << 63, n);
    let is_min_mag = b.mw_eq(&lo, &min_mag);
    let fits_pos = b.bit_not(top);
    let fits_neg = b.or(fits_pos, is_min_mag);
    let fits_low = b.sel(neg, fits_neg, fits_pos);
    let fits = b.and(hi_clear, fits_low);
    let neg_lo = b.mw_neg(&lo);
    let value = b.mw_sel(neg, &neg_lo, &lo);
    let ovf = b.bit_not(fits);
    (value, ovf)
}

/// `a × b`, faulting where the VM promotes.
pub fn mul(b: &mut Builder, x: &Mw, y: &Mw) -> (Mw, Mw) {
    let (v, ovf) = mul_checked(b, x, y);
    let ok = no_fault(b);
    (v, fault_when(b, ovf, Divergence::IntOverflow, &ok))
}

/// The truncated quotient and remainder of two's-complement `x ÷ y`, with the zero-divisor flag and the
/// `MIN ÷ −1` flag. A zero divisor divides by one instead, so nothing downstream sees an undefined word.
fn divmod_signed(b: &mut Builder, x: &Mw, y: &Mw) -> (Mw, Mw, u32, u32) {
    let n = x.len();
    let y_zero = b.mw_is_zero(y);
    let one = b.mw_const(1, n);
    let y_safe = b.mw_sel(y_zero, &one, y);
    let (mx, sx) = magnitude(b, x);
    let (my, sy) = magnitude(b, &y_safe);
    let (q, r) = b.mw_divrem(&mx, &my);
    let qneg = b.xor(sx, sy);
    let nq = b.mw_neg(&q);
    let q = b.mw_sel(qneg, &nq, &q);
    let nr = b.mw_neg(&r);
    let r = b.mw_sel(sx, &nr, &r);
    let min = b.mw_const(1u128 << 63, n);
    let neg_one = b.mw_const(u64::MAX as u128, n);
    let x_min = b.mw_eq(x, &min);
    let y_m1 = b.mw_eq(y, &neg_one);
    let min_by_m1 = b.and(x_min, y_m1);
    (q, r, y_zero, min_by_m1)
}

/// `a / b` truncating, as the VM's `Div` on two `Int`s.
pub fn div(b: &mut Builder, x: &Mw, y: &Mw) -> (Mw, Mw) {
    let (q, _, zero, min_by_m1) = divmod_signed(b, x, y);
    let ok = no_fault(b);
    let f = fault_when(b, min_by_m1, Divergence::IntOverflow, &ok);
    (q, fault_when(b, zero, Divergence::DivideByZero, &f))
}

/// `a % b` taking the dividend's sign, as the VM's `Mod` on two `Int`s.
pub fn rem(b: &mut Builder, x: &Mw, y: &Mw) -> (Mw, Mw) {
    let (_, r, zero, _) = divmod_signed(b, x, y);
    let ok = no_fault(b);
    (r, fault_when(b, zero, Divergence::DivideByZero, &ok))
}

/// `a // b` rounding toward negative infinity, as the VM's `FloorDiv` on two `Int`s.
pub fn floor_div(b: &mut Builder, x: &Mw, y: &Mw) -> (Mw, Mw) {
    let (q, r, zero, min_by_m1) = divmod_signed(b, x, y);
    let sx = b.mw_top_bit(x);
    let sy = b.mw_top_bit(y);
    let differ = b.xor(sx, sy);
    let r_zero = b.mw_is_zero(&r);
    let inexact = b.bit_not(r_zero);
    let down = b.and(differ, inexact);
    let n = q.len();
    let zero_w = b.k(0);
    let dec = b.mw_from_word(down, n);
    let (q2, _) = b.mw_sub(&q, &dec, zero_w);
    let ok = no_fault(b);
    let f = fault_when(b, min_by_m1, Divergence::IntOverflow, &ok);
    (q2, fault_when(b, zero, Divergence::DivideByZero, &f))
}

/// `a ** b` by square-and-multiply over the exponent's 32 bits, on magnitudes: a factor that overflowed
/// counts only when a later set exponent bit multiplies it in, which is exactly when the VM's exact power
/// leaves `i64`.
pub fn pow(b: &mut Builder, x: &Mw, e: &Mw) -> (Mw, Mw) {
    let n = x.len();
    let (mx, sx) = magnitude(b, x);
    let e_neg = b.mw_top_bit(e);
    let hi32 = b.mw_shr(e, 32);
    let hi_set = b.mw_or_reduce(&hi32);
    let hi_set = b.nonzero(hi_set);
    let not_neg = b.bit_not(e_neg);
    let too_large = b.and(hi_set, not_neg);
    let one = b.mw_const(1, n);
    let acc = b.mw_state(&one);
    let base = b.mw_state(&mx);
    let z = b.k(0);
    let acc_ovf = b.state(z);
    let base_ovf = b.state(z);
    let exp = b.mw_state(e);
    b.repeat(32, |s| {
        let bit = s.mw_bit(&exp, 0);
        let p = s.mw_mul(&acc, &base);
        let p_lo = p.slice(0, n);
        let p_hi = p.slice(n, 2 * n);
        let p_over = s.mw_or_reduce(&p_hi);
        let p_over = s.nonzero(p_over);
        let over_now = s.or(p_over, base_ovf);
        let acc2 = s.mw_sel(bit, &p_lo, &acc);
        let took = s.and(bit, over_now);
        let acc_ovf2 = s.or(acc_ovf, took);
        let sq = s.mw_mul(&base, &base);
        let sq_lo = sq.slice(0, n);
        let sq_hi = sq.slice(n, 2 * n);
        let sq_over = s.mw_or_reduce(&sq_hi);
        let sq_over = s.nonzero(sq_over);
        let base_ovf2 = s.or(base_ovf, sq_over);
        let exp2 = s.mw_shr(&exp, 1);
        s.mw_assign(&acc, &acc2);
        s.assign(acc_ovf, acc_ovf2);
        s.mw_assign(&base, &sq_lo);
        s.assign(base_ovf, base_ovf2);
        s.mw_assign(&exp, &exp2);
    });
    let odd = b.mw_bit(e, 0);
    let neg = b.and(sx, odd);
    let top = b.mw_top_bit(&acc);
    let min_mag = b.mw_const(1u128 << 63, n);
    let is_min_mag = b.mw_eq(&acc, &min_mag);
    let fits_pos = b.bit_not(top);
    let fits_neg = b.or(fits_pos, is_min_mag);
    let fits = b.sel(neg, fits_neg, fits_pos);
    let range_ovf = b.bit_not(fits);
    let ovf = b.or(acc_ovf, range_ovf);
    let nacc = b.mw_neg(&acc);
    let value = b.mw_sel(neg, &nacc, &acc);
    let ok = no_fault(b);
    let f = fault_when(b, ovf, Divergence::IntOverflow, &ok);
    let f = fault_when(b, too_large, Divergence::ExponentTooLarge, &f);
    (
        value,
        fault_when(b, e_neg, Divergence::NegativeExponent, &f),
    )
}

/// The VM's shift count is `b as u32` masked to the word size: the low six bits of `b`, which is exactly the
/// part of the amount a six-stage barrel reads.
const SHIFT_BITS: u32 = 6;

/// `a.wrapping_shl(b as u32)`.
pub fn shl(b: &mut Builder, x: &Mw, y: &Mw) -> Mw {
    b.mw_shl_var(x, y, SHIFT_BITS)
}

/// `a.wrapping_shr(b as u32)` on the signed value (arithmetic).
pub fn shr(b: &mut Builder, x: &Mw, y: &Mw) -> Mw {
    b.mw_sar_var(x, y, SHIFT_BITS)
}

/// `(a as u64).wrapping_shr(b as u32)` (logical).
pub fn ushr(b: &mut Builder, x: &Mw, y: &Mw) -> Mw {
    b.mw_shr_var_sticky(x, y, SHIFT_BITS).0
}

/// The high word of the unsigned 128-bit product.
pub fn mul_hi_u(b: &mut Builder, x: &Mw, y: &Mw) -> Mw {
    let n = x.len();
    let p = b.mw_mul(x, y);
    p.slice(n, 2 * n)
}

/// 1 when the signed product leaves `i64`, as a 64-bit value.
pub fn mul_ovf_s(b: &mut Builder, x: &Mw, y: &Mw) -> Mw {
    let n = x.len();
    let (_, ovf) = mul_checked(b, x, y);
    b.mw_from_word(ovf, n)
}

/// The set bits of the 64-bit pattern, as a 64-bit value.
pub fn popcount(b: &mut Builder, x: &Mw) -> Mw {
    let n = x.len();
    let c = b.mw_popcount(x, b.shape().words_for(8));
    b.mw_resize(&c, n)
}

/// A 0/1 predicate as a 64-bit value.
pub fn bool64(b: &mut Builder, p: u32, n: usize) -> Mw {
    b.mw_from_word(p, n)
}
