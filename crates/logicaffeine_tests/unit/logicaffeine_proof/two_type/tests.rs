use super::*;

fn trivial_horn(n: usize, k: usize, modulus: usize) -> HashMap<usize, Vec<usize>> {
    // the horn whose given faces are all zero (lives in Γ_{n-1})
    let zero = vec![0usize; surjections(n - 1).len()];
    positions(n, k).iter().map(|&p| (p, zero.clone())).collect()
}

#[test]
fn the_2_type_has_a_nonunique_inner_filler_so_pi2_is_nonzero() {
    // π₂ ≠ 0, INSIDE THE OBJECT. The inner horn Λ²₁ has |A| fillers — the 2-cells — not the unique
    // filler a 1-type forces. This is precisely the structure kan_complex proved K(G,1) lacks:
    // there every inner horn filled uniquely; here a whole copy of A fills the same horn.
    for modulus in [2usize, 3, 4] {
        let fillers = filler_count(modulus, 2, 1, &trivial_horn(2, 1, modulus));
        assert_eq!(
            fillers, modulus,
            "Λ²₁ has |A| = {modulus} fillers — π₂ = A, genuinely 2-dimensional"
        );
        assert!(
            fillers > 1,
            "non-unique inner filler ⇒ NOT 1-truncated (this is what K(G,1) cannot do)"
        );
    }
}

#[test]
fn the_2_type_is_a_kan_complex_every_horn_fills() {
    // It IS a Kan complex (an ∞-groupoid): the face-tuple map hₖ is SURJECTIVE onto the compatible
    // horns in every degree we check (2, 3, 4) and position — so every horn fills. A simplicial
    // abelian group is Kan; we confirm it by counting, not by appeal.
    let modulus = 2;
    for (n, ks) in [(2usize, vec![0, 1, 2]), (3, vec![0, 1, 2, 3])] {
        for k in ks {
            assert_eq!(
                image_size(modulus, n, k),
                compatible_horn_count(modulus, n, k),
                "Λ^{n}_{k} fills: image of hₖ = all compatible horns"
            );
        }
    }
    // degree 4, inner horns: still surjective (Kan)
    for k in 1..4 {
        assert_eq!(
            image_size(modulus, 4, k),
            compatible_horn_count(modulus, 4, k),
            "Λ⁴_{k} fills (Kan)"
        );
    }
}

#[test]
fn the_2_type_realizes_the_crossed_modules_admitted_pi2_admit_equals_have() {
    // SYMMETRY BREAKING, one level up — and the campaign's loop CLOSES. two_group's crossed module
    // A → 1 (g = trivial, h = A, ∂ = 0) ADMITS π₂ = ker∂ = A, algebraically. This K(A,2) Kan complex
    // HAS π₂ = A geometrically: the inner horn Λ²₁ has exactly |A| fillers. They are the SAME A — the
    // 2-group that admits the higher symmetry is realized by the object that carries it: admit = have.
    // And the |A| fillers form an A-TORSOR: breaking that symmetry (choosing the canonical filler) is
    // the very π₀ orbit-collapse this whole campaign began with, now acting on 2-cells. Symmetry
    // breaking runs from π₀ all the way up the tower.
    use crate::two_group::{CrossedModule, FiniteGroup};
    for m in [2usize, 3, 4] {
        let cm = CrossedModule {
            g: FiniteGroup::trivial(),
            h: FiniteGroup::cyclic(m),
            partial: vec![0; m],
        };
        let admitted = cm.pi_two().len(); // algebra: ker ∂ = A
        let had = filler_count(m, 2, 1, &trivial_horn(2, 1, m)); // geometry: # fillers of Λ²₁
        assert_eq!(
            admitted, had,
            "crossed module ADMITS π₂ = A ⟺ K(A,2) HAS π₂ = A (|A| fillers)"
        );
        assert_eq!(admitted, m, "and both equal |A| = {m}");
    }
}

#[test]
fn the_2_type_is_exactly_two_truncated() {
    // EXACTLY 2-truncated. At degree 2 the inner horn fills NON-uniquely (π₂ lives there: kernel = A).
    // By degree 4 the inner horns fill UNIQUELY again (kernel trivial) — there is no π₃, the higher
    // homotopy stops at level 2. So this object is a genuine 2-type: more than K(G,1), and no more.
    let modulus = 2;
    assert_eq!(
        kernel_size(modulus, 2, 1),
        modulus,
        "degree 2 inner: |ker| = |A| ⇒ π₂ = A"
    );
    for k in 1..4 {
        assert_eq!(
            kernel_size(modulus, 4, k),
            1,
            "degree 4 inner Λ⁴_{k}: unique filler ⇒ no π₃ (2-truncated)"
        );
    }
}
