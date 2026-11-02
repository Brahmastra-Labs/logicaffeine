use super::*;

#[test]
fn mldsa_montgomery_reduction_is_kernel_certified() {
    assert_eq!(
        MLDSA_QINV * MLDSA_Q - 1,
        MLDSA_MONT_COFACTOR * MLDSA_MONT_R,
        "qinv·q − 1 must be an exact multiple of R = 2³² (qinv = q⁻¹ mod R)"
    );
    assert!(
        mldsa_montgomery_divisibility(),
        "a·qinv·q − a is an exact multiple of R"
    );
    assert!(mldsa_montgomery_congruence(), "redc·R ≡ a (mod q)");
    assert!(
        mldsa_montgomery_reduction_certified(),
        "the combined ML-DSA Montgomery gate"
    );
}

#[test]
fn mldsa_montgomery_certification_is_non_vacuous() {
    // A wrong cofactor must NOT certify — otherwise qinv would be invalid.
    let a = Polynomial::var(0);
    let lhs = a
        .mul(&Polynomial::constant(MLDSA_QINV))
        .mul(&Polynomial::constant(MLDSA_Q))
        .sub(&a);
    let wrong = a
        .mul(&Polynomial::constant(MLDSA_MONT_COFACTOR + 1))
        .mul(&Polynomial::constant(MLDSA_MONT_R));
    assert!(!lhs.canonical_eq(&wrong), "cofactor+1 must be rejected");
}

#[test]
fn montgomery_reduction_is_kernel_certified() {
    assert!(
        montgomery_reduction_divisibility(),
        "qinv'=3327 makes x+(x·qinv')·q an exact multiple of R"
    );
    assert!(montgomery_reduction_congruence(), "redc·R ≡ x (mod q)");
    assert!(
        montgomery_reduction_certified(),
        "the combined Montgomery-reduction gate"
    );
    assert_eq!(
        NEG_QINV_MOD_R * KYBER_Q + 1,
        MONT_COFACTOR * MONT_R,
        "qinv'·q + 1 = 169·R"
    );
}

#[test]
fn montgomery_certification_is_non_vacuous() {
    let x = Polynomial::var(0);
    let lhs = x.add(
        &x.mul(&Polynomial::constant(NEG_QINV_MOD_R))
            .mul(&Polynomial::constant(KYBER_Q)),
    );
    let wrong = x
        .mul(&Polynomial::constant(168))
        .mul(&Polynomial::constant(MONT_R)); // 168 ≠ 169
    assert!(
        !lhs.canonical_eq(&wrong),
        "a wrong cofactor must not be certified — qinv' would be invalid"
    );
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
    assert!(
        !lhs.canonical_eq(&wrong),
        "a wrong rewrite must not be certified equal"
    );
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
    assert!(
        !too_weak,
        "a < 3q does NOT bound a − q below q — the prover must not claim it does"
    );
}
