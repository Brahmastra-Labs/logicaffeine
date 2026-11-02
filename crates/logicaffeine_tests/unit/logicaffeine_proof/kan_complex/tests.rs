use super::*;

#[test]
fn the_spectrum_represents_cohomology_nerve_cohomology_is_group_cohomology() {
    // K(A,n) REPRESENTS cohomology: Hⁿ(X;A) = [X, K(A,n)]. Instantiated on the symmetry's classifying
    // space X = BG = K(G,1): Hⁿ(BG;A) = [BG, K(A,n)]. We compute Hⁿ(BG;A) from the NERVE's own face
    // maps and confirm it equals the algebraic group cohomology Hⁿ(G;A) — so what the Eilenberg–
    // MacLane spectrum represents on BG is exactly group cohomology, the home of the k-invariants.
    for n in 2..=3 {
        assert_eq!(
            nerve_cohomology_size(&FiniteGroup::cyclic(2), 2, n),
            crate::postnikov::cohomology_size(&FiniteGroup::cyclic(2), 2, n),
            "Hⁿ(BG; Z/2) from the nerve = group cohomology Hⁿ(Z/2; Z/2)"
        );
    }
    // a second group: H²(BZ/3; Z/2) from the nerve = group cohomology H²(Z/3; Z/2)
    assert_eq!(
        nerve_cohomology_size(&FiniteGroup::cyclic(3), 2, 2),
        crate::postnikov::cohomology_size(&FiniteGroup::cyclic(3), 2, 2),
        "the representing property holds for BZ/3 too"
    );
}

#[test]
fn the_nerve_is_a_kan_complex_so_we_actually_have_an_infinity_groupoid() {
    // We stop computing invariants and build the ∞-GROUPOID OBJECT: the nerve of a group, a
    // simplicial set, and VERIFY its defining property — the Kan horn-filling condition — in degrees
    // 2 and 3, for EVERY horn (inner and outer). A Kan complex IS an ∞-groupoid (homotopy
    // hypothesis). The nerve of Aut(F) is BG = K(Aut(F),1), so this is the very ∞-groupoid the
    // symmetry tower named — now possessed as an object, not merely measured by its Betti numbers.
    for g in [
        FiniteGroup::cyclic(2),
        FiniteGroup::cyclic(3),
        FiniteGroup::symmetric(3),
    ] {
        for n in 2..=3 {
            for k in 0..=n {
                assert!(
                    kan_fills(&g, n, k),
                    "the nerve fills every horn Λ^n_k — it is a Kan complex"
                );
            }
        }
    }
}

#[test]
fn it_is_exactly_one_truncated_so_honestly_its_only_K_G_1_for_now() {
    // HONEST SCOPE — the test that keeps us truthful. The ∞-groupoid we now have is precisely the
    // 1-TRUNCATED one: every inner horn fills UNIQUELY, the hallmark of the nerve of a 1-groupoid
    // (no genuine higher cells, π_{≥2} = 0). A non-truncated ∞-groupoid would have inner horns with
    // MULTIPLE fillers. Assembling the homology ladder's higher πₙ into one coherent Kan complex with
    // its k-invariants — a genuinely 2+-truncated object — is the frontier we do NOT yet hold.
    for g in [
        FiniteGroup::cyclic(2),
        FiniteGroup::cyclic(3),
        FiniteGroup::symmetric(3),
    ] {
        for n in 2..=3 {
            for k in 1..n {
                assert!(
                    inner_horns_fill_uniquely(&g, n, k),
                    "inner horns unique ⇒ 1-truncated = K(G,1)"
                );
            }
        }
    }
}
