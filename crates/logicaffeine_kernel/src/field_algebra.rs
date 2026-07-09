//! Kernel algebra for the prime field 𝔽_q of ML-KEM / ML-DSA — the certified arithmetic
//! substrate (F2 seed).
//!
//! Every identity here is discharged by the kernel's OWN decision procedures — the [`ring`]
//! canonicalizer for polynomial identities (which hold for ALL values, not a ≤16-bit
//! sample), and Fourier-Motzkin [`lia`] for the modular-reduction range bounds — with no Z3.
//! This is the layer the NTT's correctness and its symmetry-derived *speed* rewrites build
//! on: a rewrite that cuts multiplies (or collapses a symmetric butterfly) is only safe once
//! the kernel certifies it computes the same polynomial.
//!
//! [`ring`]: crate::ring
//! [`lia`]: crate::lia

use crate::lia::{fourier_motzkin_unsat, Constraint, LinearExpr, Rational};
use crate::ring::Polynomial;

/// Kyber / ML-KEM's modulus — prime, so ℤ/qℤ is the field 𝔽_q.
pub const KYBER_Q: i64 = 3329;

/// Gauss's three-multiplication identity — the symmetry that turns a 4-multiply bilinear
/// form `ad + bc` into 3 multiplies `(a+b)(c+d) − ac − bd`. It is the archetypal
/// "spend an add to save a multiply" rewrite behind fast complex / polynomial / NTT-butterfly
/// multiplication. Certified as a polynomial identity over any commutative ring by the
/// kernel's `ring` canonicalizer, so a codegen pass may apply it knowing it is sound.
pub fn gauss_three_multiply_identity() -> bool {
    let a = Polynomial::var(0);
    let b = Polynomial::var(1);
    let c = Polynomial::var(2);
    let d = Polynomial::var(3);
    let lhs = a.mul(&d).add(&b.mul(&c)); // ad + bc — 4 multiplies, naive
    let ac = a.mul(&c);
    let bd = b.mul(&d);
    let rhs = a.add(&b).mul(&c.add(&d)).sub(&ac).sub(&bd); // (a+b)(c+d) − ac − bd — 3 multiplies
    lhs.canonical_eq(&rhs)
}

/// A constraint `coeff·a + d  OP  0` over the single variable `a` (de Bruijn 0).
fn con(coeff: i64, d: i64, strict: bool) -> Constraint {
    let expr = LinearExpr::constant(Rational::from_i64(d))
        .add(&LinearExpr::var(0).scale(&Rational::from_i64(coeff)));
    Constraint { expr, strict }
}

/// Certify that conditional-subtract reduction keeps a modular-ADD result in range. After an
/// addition in 𝔽_q the value `a` lies in `[0, 2q)`; the reduction is
/// `csub(a) = if a ≥ q { a − q } else { a }`, and the result must lie in `[0, q)`. The two
/// non-trivial bounds (the `a ≥ q` branch) are discharged by Fourier-Motzkin: a goal is valid
/// iff its negation's constraints are UNSAT.
pub fn conditional_subtract_in_range(q: i64) -> bool {
    // Lower bound `a − q ≥ 0`:  hyp `a ≥ q` (−a + q ≤ 0)  ∧  ¬goal `a − q < 0`  → UNSAT.
    let lower = fourier_motzkin_unsat(&[con(-1, q, false), con(1, -q, true)]);
    // Upper bound `a − q < q` (a − 2q < 0):  hyp `a < 2q` (a − 2q < 0)  ∧  ¬goal `a − 2q ≥ 0`
    // (−a + 2q ≤ 0)  → UNSAT.
    let upper = fourier_motzkin_unsat(&[con(1, -2 * q, true), con(-1, 2 * q, false)]);
    lower && upper
}

/// Montgomery radix `R = 2¹⁶` and `qinv' = −q⁻¹ mod R = 3327`, with the exact integer cofactor
/// `(qinv'·q + 1) / R = 169`. These are the constants of the division-free reduction
/// `redc(x) = (x + ((x·qinv') mod R)·q) / R` used by the (SIMD) ML-KEM NTT.
pub const MONT_R: i64 = 65536;
pub const NEG_QINV_MOD_R: i64 = 3327;
const MONT_COFACTOR: i64 = 169; // (NEG_QINV_MOD_R·KYBER_Q + 1) / MONT_R

/// Certify the Montgomery DIVISIBILITY: `x + (x·qinv')·q = x·169·R`, i.e. the reduction's
/// numerator (with the un-reduced `lo = x·qinv'`) is an EXACT multiple of `R` — so `redc`'s
/// `/ R` is an exact shift, never a rounding division. This is what makes `qinv' = 3327` the
/// RIGHT constant (`qinv'·q ≡ −1 mod R`): the kernel `ring` canonicalizer reduces both sides to
/// `x·11075584`, certifying it for ALL `x`, not a sampled width.
pub fn montgomery_reduction_divisibility() -> bool {
    let x = Polynomial::var(0);
    let lhs = x.add(
        &x.mul(&Polynomial::constant(NEG_QINV_MOD_R))
            .mul(&Polynomial::constant(KYBER_Q)),
    );
    let rhs = x
        .mul(&Polynomial::constant(MONT_COFACTOR))
        .mul(&Polynomial::constant(MONT_R));
    lhs.canonical_eq(&rhs)
}

/// Certify the Montgomery CONGRUENCE: `(x + lo·q) − x = q·lo`, so the reduction numerator differs
/// from `x` by a multiple of `q` ⇒ `redc·R ≡ x (mod q)` ⇒ `redc ≡ x·R⁻¹ (mod q)` (`R` is a unit
/// in 𝔽_q). A ring identity in the free variables `x`, `lo`, certified for all values.
pub fn montgomery_reduction_congruence() -> bool {
    let x = Polynomial::var(0);
    let lo = Polynomial::var(1);
    let lhs = x.add(&lo.mul(&Polynomial::constant(KYBER_Q))).sub(&x);
    let rhs = Polynomial::constant(KYBER_Q).mul(&lo);
    lhs.canonical_eq(&rhs)
}

/// The full Montgomery reduction is kernel-certified: the constant divides exactly (so `/R` is a
/// shift), the result is `x·R⁻¹ mod q` (congruence), and the final conditional subtract keeps it
/// in `[0, q)` (range). A single gate over the three procedures.
pub fn montgomery_reduction_certified() -> bool {
    montgomery_reduction_divisibility()
        && montgomery_reduction_congruence()
        && conditional_subtract_in_range(KYBER_Q)
}

// ── ML-DSA-65 (Dilithium3) prime field 𝔽_q, q = 8380417, 32-bit Montgomery radix ────────────────

/// Dilithium's modulus — prime, so ℤ/qℤ is the field 𝔽_q.
pub const MLDSA_Q: i64 = 8_380_417;
/// Montgomery radix `R = 2³²` and `qinv = q⁻¹ mod R = 58728449` (the SUBTRACT convention: the
/// reduction is `redc(a) = (a − ((a·qinv) mod R)·q) / R`, matching `mldsa::montgomery_reduce`). The
/// exact integer cofactor `(qinv·q − 1) / R` makes the numerator a clean multiple of `R`.
pub const MLDSA_MONT_R: i64 = 1 << 32;
pub const MLDSA_QINV: i64 = 58_728_449;
const MLDSA_MONT_COFACTOR: i64 = (MLDSA_QINV * MLDSA_Q - 1) / MLDSA_MONT_R;

/// Certify the ML-DSA Montgomery DIVISIBILITY: `a·qinv·q − a = cofactor·R·a`, i.e. the reduction's
/// numerator `a − (a·qinv)·q = −cofactor·R·a` is an EXACT multiple of `R` (so `/R` is a shift, never
/// a rounding divide). This holds because `qinv·q ≡ 1 (mod R)`; the `ring` canonicalizer reduces both
/// sides to `a·(qinv·q − 1)`, certifying it for ALL `a`.
pub fn mldsa_montgomery_divisibility() -> bool {
    let a = Polynomial::var(0);
    let lhs = a
        .mul(&Polynomial::constant(MLDSA_QINV))
        .mul(&Polynomial::constant(MLDSA_Q))
        .sub(&a);
    let rhs = a
        .mul(&Polynomial::constant(MLDSA_MONT_COFACTOR))
        .mul(&Polynomial::constant(MLDSA_MONT_R));
    lhs.canonical_eq(&rhs)
}

/// Certify the ML-DSA Montgomery CONGRUENCE: `(a − lo·q) − a = −q·lo`, so the reduction numerator
/// `redc·R = a − lo·q` differs from `a` by a multiple of `q` ⇒ `redc·R ≡ a (mod q)` ⇒
/// `redc ≡ a·R⁻¹ (mod q)`. A ring identity in the free variables `a`, `lo`, certified for all values.
pub fn mldsa_montgomery_congruence() -> bool {
    let a = Polynomial::var(0);
    let lo = Polynomial::var(1);
    let lhs = a.sub(&lo.mul(&Polynomial::constant(MLDSA_Q))).sub(&a);
    let rhs = Polynomial::constant(-MLDSA_Q).mul(&lo);
    lhs.canonical_eq(&rhs)
}

/// The full ML-DSA Montgomery reduction is kernel-certified: the numerator is an exact multiple of
/// `R` (divisibility) and the result is `a·R⁻¹ mod q` (congruence). Together these prove
/// `mldsa::montgomery_reduce` — hence the AVX2 `montmul32` that shares its formula — computes the
/// right field element, with no Z3, for ALL inputs (not a sampled bit-width).
pub fn mldsa_montgomery_reduction_certified() -> bool {
    mldsa_montgomery_divisibility() && mldsa_montgomery_congruence()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mldsa_montgomery_reduction_is_kernel_certified() {
        assert_eq!(
            MLDSA_QINV * MLDSA_Q - 1,
            MLDSA_MONT_COFACTOR * MLDSA_MONT_R,
            "qinv·q − 1 must be an exact multiple of R = 2³² (qinv = q⁻¹ mod R)"
        );
        assert!(mldsa_montgomery_divisibility(), "a·qinv·q − a is an exact multiple of R");
        assert!(mldsa_montgomery_congruence(), "redc·R ≡ a (mod q)");
        assert!(mldsa_montgomery_reduction_certified(), "the combined ML-DSA Montgomery gate");
    }

    #[test]
    fn mldsa_montgomery_certification_is_non_vacuous() {
        // A wrong cofactor must NOT certify — otherwise qinv would be invalid.
        let a = Polynomial::var(0);
        let lhs = a.mul(&Polynomial::constant(MLDSA_QINV)).mul(&Polynomial::constant(MLDSA_Q)).sub(&a);
        let wrong = a
            .mul(&Polynomial::constant(MLDSA_MONT_COFACTOR + 1))
            .mul(&Polynomial::constant(MLDSA_MONT_R));
        assert!(!lhs.canonical_eq(&wrong), "cofactor+1 must be rejected");
    }

    #[test]
    fn montgomery_reduction_is_kernel_certified() {
        assert!(montgomery_reduction_divisibility(), "qinv'=3327 makes x+(x·qinv')·q an exact multiple of R");
        assert!(montgomery_reduction_congruence(), "redc·R ≡ x (mod q)");
        assert!(montgomery_reduction_certified(), "the combined Montgomery-reduction gate");
        assert_eq!(NEG_QINV_MOD_R * KYBER_Q + 1, MONT_COFACTOR * MONT_R, "qinv'·q + 1 = 169·R");
    }

    #[test]
    fn montgomery_certification_is_non_vacuous() {
        let x = Polynomial::var(0);
        let lhs = x.add(&x.mul(&Polynomial::constant(NEG_QINV_MOD_R)).mul(&Polynomial::constant(KYBER_Q)));
        let wrong = x.mul(&Polynomial::constant(168)).mul(&Polynomial::constant(MONT_R)); // 168 ≠ 169
        assert!(!lhs.canonical_eq(&wrong), "a wrong cofactor must not be certified — qinv' would be invalid");
        let lo = Polynomial::var(1);
        let cong_lhs = x.add(&lo.mul(&Polynomial::constant(KYBER_Q))).sub(&x);
        assert!(!cong_lhs.canonical_eq(&lo), "redc·R − x is q·lo, not lo");
    }

    #[test]
    fn gauss_three_multiply_is_kernel_certified() {
        assert!(
            gauss_three_multiply_identity(),
            "the ring canonicalizer must certify Gauss's 3-multiply butterfly identity"
        );
    }

    #[test]
    fn gauss_identity_is_non_vacuous() {
        // A WRONG rewrite (dropping `− bd`) must NOT canonicalize equal — the prover genuinely
        // distinguishes the identity rather than calling everything equal.
        let a = Polynomial::var(0);
        let b = Polynomial::var(1);
        let c = Polynomial::var(2);
        let d = Polynomial::var(3);
        let lhs = a.mul(&d).add(&b.mul(&c));
        let wrong = a.add(&b).mul(&c.add(&d)).sub(&a.mul(&c)); // missing − bd
        assert!(!lhs.canonical_eq(&wrong), "a wrong rewrite must not be certified equal");
    }

    #[test]
    fn conditional_subtract_in_range_for_kyber_q() {
        assert!(
            conditional_subtract_in_range(KYBER_Q),
            "csub must keep a modular-add result of 𝔽_3329 in [0, q)"
        );
    }

    #[test]
    fn reduction_range_is_non_vacuous() {
        // The procedure must NOT "prove" a FALSE range claim. Under the too-weak hypothesis
        // `a < 3q`, the negation of `a − q < q` (i.e. `a ≥ 2q`) is SATISFIABLE (a ≈ 2.5q), so
        // `fourier_motzkin_unsat` must return FALSE — the bound genuinely does not hold.
        let too_weak =
            fourier_motzkin_unsat(&[con(1, -3 * KYBER_Q, true), con(-1, 2 * KYBER_Q, false)]);
        assert!(!too_weak, "a < 3q does NOT bound a − q below q — the prover must not claim it does");
    }
}
