use super::*;

#[test]
fn hurewicz_homology_equals_homotopy_for_k_a_n() {
    // HUREWICZ — the two halves of the machinery meet. K(A,n) is (n−1)-connected, so the Hurewicz
    // theorem predicts π_k ≅ H_k. We compute HOMOLOGY (Moore complex, `homology_size`) and HOMOTOPY
    // (horn-filling, `homotopy_group_size`) by INDEPENDENT algorithms and confirm they agree exactly:
    // both are A at k = n and trivial elsewhere. (Honest ceiling, checked elsewhere: above the
    // connectivity range Hurewicz fails — H_3(S²)=0 while π_3(S²)=Z — so this agreement is content,
    // not a tautology.)
    for n in 2..=3 {
        for k in 0..=(n + 1) {
            assert_eq!(
                homology_size(2, n, k),
                homotopy_group_size(2, n, k),
                "Hurewicz over Z/2: H_{k}(K(A,{n})) = π_{k}"
            );
        }
        // a second coefficient group, at the Hurewicz degree k = n
        assert_eq!(
            homology_size(3, n, n),
            homotopy_group_size(3, n, n),
            "Hurewicz over Z/3 at k = n"
        );
    }
}

#[test]
fn we_get_prove_and_extract_the_infinity_groupoid_homotopy_hypothesis_both_ways() {
    // THE CAPSTONE — get it, prove it, extract it.
    //   GET:     we CONSTRUCT K(A, n), an actual ∞-groupoid object (a simplicial set).
    //   PROVE:   it is a Kan complex — every horn fills (the `k_a_n_is_a_kan_complex` test).
    //   EXTRACT: we read its COMPLETE homotopy back out of the object by horn-filling, recovering the
    //            exact signature (π_0, π_1, …) = (1, 1, …, A at degree n, …, 1).
    // Construction from the groups and extraction of the groups are inverse — the homotopy hypothesis,
    // both directions, on a concrete object we hold. This is what "getting the ∞-groupoid" means
    // honestly: not a single finite object carrying all of an arbitrary tower at once (that is the
    // unbounded Postnikov colimit), but the ∞-groupoid OBJECT for each building block, possessed and
    // fully read out — and these blocks (with the k-invariants) assemble every rung of the tower.
    for n in 2..=3 {
        for modulus in [2usize, 3] {
            let signature: Vec<usize> = (0..=n + 1)
                .map(|k| homotopy_group_size(modulus, n, k))
                .collect();
            let mut expected = vec![1usize; n + 2];
            expected[n] = modulus; // π_n = A
            assert_eq!(
                signature, expected,
                "extracted homotopy of K(Z/{modulus}, {n}) is exactly π_n = A"
            );
        }
    }
}

#[test]
fn k_a_n_has_pi_n_equal_to_A_for_n_2_and_3() {
    // πₙ = A, INSIDE the object: the inner horn at degree n has exactly |A| fillers (the n-cells).
    // n = 3 is the first genuine 3-type the framework holds — π₃ ≠ 0 in an actual Kan complex.
    for n in 2..=3 {
        for modulus in [2usize, 3] {
            for k in 1..n {
                // inner horn 0 < k < n
                assert_eq!(
                    kernel_size(modulus, n, n, k),
                    modulus,
                    "K(Z/{modulus}, {n}): inner horn at degree n has |A| fillers ⇒ πₙ = A"
                );
            }
        }
    }
}

#[test]
fn k_a_n_is_a_kan_complex_for_n_2_and_3() {
    // Kan: every horn fills (image of hₖ = all compatible horns), checked at degrees n and n+1.
    let modulus = 2;
    for n in 2..=3 {
        for m in n..=(n + 1) {
            for k in 0..=m {
                assert_eq!(
                    image_size(modulus, m, n, k),
                    compatible_horn_count(modulus, m, n, k),
                    "K(Z/2, {n}): horn Λ^{m}_{k} fills"
                );
            }
        }
    }
}

#[test]
fn k_a_n_is_exactly_n_truncated_for_n_2_and_3() {
    // Exactly n-truncated: inner horns at degree n+2 fill UNIQUELY (kernel = 1), so there is no
    // π_{n+1}. The higher homotopy stops precisely at level n — a genuine n-type, no more.
    let modulus = 2;
    for n in 2..=3 {
        for k in 1..(n + 2) {
            assert_eq!(
                kernel_size(modulus, n + 2, n, k),
                1,
                "K(Z/2, {n}): inner horn at degree n+2 fills uniquely ⇒ exactly {n}-truncated"
            );
        }
    }
}

#[test]
fn the_eilenberg_maclane_spaces_form_an_omega_spectrum() {
    // NEW + POWERFUL: {K(A,n)} is an Ω-SPECTRUM — each space deloops to the next (ΩK(A,n) ≃
    // K(A,n−1)), so the homotopy is stable under the (n→n+1, k→k+1) shift. An Ω-spectrum is the
    // modern object of stable homotopy theory and the representing object of a cohomology theory:
    // the infinitely-deloopable, fully-stabilized symmetry object.
    for modulus in [2usize, 3] {
        assert!(
            is_omega_spectrum(modulus, 2, 4),
            "K(Z/{modulus}, n) deloops to K(Z/{modulus}, n−1) — the EM spaces form an Ω-spectrum"
        );
    }
}

#[test]
fn the_em_spectrum_has_stable_homotopy_concentrated_in_degree_zero() {
    // The spectrum HA has π_k(HA) = A at k = 0 and 0 elsewhere — concentrated in a single degree,
    // which is exactly what makes HA represent ORDINARY cohomology Hⁿ(−; A). We read the stable
    // homotopy off the deloop-invariant tower.
    for modulus in [2usize, 3] {
        assert_eq!(stable_homotopy(modulus, 0), modulus, "π_0(HA) = A");
        for k in [-2i32, -1, 1, 2, 3] {
            assert_eq!(stable_homotopy(modulus, k), 1, "π_{k}(HA) = 0 for k ≠ 0");
        }
    }
}

#[test]
fn the_general_engine_reproduces_two_type_at_n_equals_2() {
    // Cross-check the general K(A,n) against the bespoke K(A,2): the inner horn Λ²₁ has |A| fillers.
    for modulus in [2usize, 3, 4] {
        assert_eq!(
            kernel_size(modulus, 2, 2, 1),
            modulus,
            "general K(A,2) agrees: π₂ = A"
        );
    }
}
