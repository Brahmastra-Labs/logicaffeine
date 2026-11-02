use super::*;

#[test]
fn word_ring_laws_are_kernel_certified() {
    assert!(add_associative(), "additive associativity in ℤ/2ⁿ");
    assert!(add_commutative(), "additive commutativity in ℤ/2ⁿ");
    assert!(mul_associative(), "multiplicative associativity in ℤ/2ⁿ");
    assert!(mul_commutative(), "multiplicative commutativity in ℤ/2ⁿ");
    assert!(left_distributive(), "left distributivity in ℤ/2ⁿ");
    assert!(additive_identity(), "additive identity in ℤ/2ⁿ");
    assert!(multiplicative_identity(), "multiplicative identity in ℤ/2ⁿ");
    assert!(karatsuba_expand(), "Karatsuba/gauss expansion in ℤ/2ⁿ");
    assert!(all_word_ring_laws_certified(), "the combined gate");
}

#[test]
fn wrong_word_ring_identities_are_not_certified() {
    // Non-vacuity: the prover must reject FALSE identities, or the proofs above are worthless.
    // a + b ≠ a · b
    assert!(
        !a().add(&b()).canonical_eq(&a().mul(&b())),
        "sum must not be certified equal to product"
    );
    // a · (b + c) ≠ a · b + c  (a dropped factor on the second term)
    assert!(
        !a().mul(&b().add(&c()))
            .canonical_eq(&a().mul(&b()).add(&c())),
        "a broken distributive law must not be certified"
    );
    // (a + b)(c + d) ≠ ac + bd  (the cross terms dropped — the classic Karatsuba bug)
    assert!(
        !a().add(&b())
            .mul(&c().add(&d()))
            .canonical_eq(&a().mul(&c()).add(&b().mul(&d()))),
        "dropping the cross terms must not be certified"
    );
}
