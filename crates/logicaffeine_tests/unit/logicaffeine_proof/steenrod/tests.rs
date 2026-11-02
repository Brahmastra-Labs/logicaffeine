use super::*;

#[test]
fn adem_reduces_known_relations_to_the_admissible_basis() {
    // The Adem reduction is the canonical form in the Steenrod algebra. Sq¹Sq¹ → 0; Sq¹Sq² → Sq³;
    // Sq²Sq² → Sq³Sq¹ (admissible: 3 ≥ 2·1). The defining relations, recovered as rewrites.
    assert!(adem_reduce(&[1, 1]).is_empty(), "Sq¹Sq¹ = 0");
    assert_eq!(
        adem_reduce(&[1, 2]),
        [vec![3]].into_iter().collect(),
        "Sq¹Sq² = Sq³"
    );
    assert_eq!(
        adem_reduce(&[2, 2]),
        [vec![3, 1]].into_iter().collect(),
        "Sq²Sq² = Sq³Sq¹"
    );
    for m in [vec![3, 1], vec![2, 1], vec![5, 2, 1]] {
        for adm in &adem_reduce(&m) {
            assert!(is_admissible(adm), "reduction output is admissible");
        }
    }
}

#[test]
fn two_primary_group_structure_from_the_h0_tower_filtration() {
    // LEAN ON THE h₀ SYMMETRY (read from the resolution boundaries, unused by the dim count): the
    // EXACT 2-local stable groups, distinguishing Z/8 from (Z/2)³. Tower-length multiset [L,…] ⇒ ⊕Z/2^L.
    let res = super::minimal_resolution(8, 18);
    let g = |n| super::stem_2local_tower_lengths(&res, n);
    assert_eq!(g(1), vec![1], "π₁ˢ = Z/2");
    assert_eq!(g(2), vec![1], "π₂ˢ = Z/2");
    assert_eq!(
        g(3),
        vec![3],
        "π₃ˢ: ONE h₀-tower height 3 = Z/8 (not (Z/2)³) ⇒ Z/24"
    );
    assert_eq!(g(6), vec![1], "π₆ˢ = Z/2");
    assert_eq!(g(7), vec![4], "π₇ˢ = Z/16 ⇒ Z/240");
    assert_eq!(g(8), vec![1, 1], "π₈ˢ = (Z/2)² — two separate towers");
    assert_eq!(g(9), vec![1, 1, 1], "π₉ˢ = (Z/2)³");
    assert_eq!(g(11), vec![3], "π₁₁ˢ = Z/8 ⇒ Z/504");
}

#[test]
fn stable_homotopy_groups_in_stems_eight_through_thirteen() {
    // stems 8–13, 2-local orders (E₂ = E∞ here, no differentials yet):
    // π₈=(Z/2)²→2, π₉=(Z/2)³→3, π₁₀=Z/2→1, π₁₁=Z/8→3 (2-local of Z/504), π₁₂=0→0, π₁₃=0→0.
    let e = ext(7, 18);
    let st = |n: usize| -> usize { (0..=7).filter(|&s| n + s <= 18).map(|s| e[s][n + s]).sum() };
    assert_eq!(st(8), 2, "π₈ˢ = (Z/2)²");
    assert_eq!(st(9), 3, "π₉ˢ = (Z/2)³");
    assert_eq!(st(10), 1, "π₁₀ˢ = Z/2 (2-local of Z/6)");
    assert_eq!(st(11), 3, "π₁₁ˢ = Z/8 2-locally (Z/504)");
    assert_eq!(st(12), 0, "π₁₂ˢ = 0");
    assert_eq!(st(13), 0, "π₁₃ˢ = 0 (2-locally; Z/3)");
}

#[test]
fn infinite_h0_tower_and_indecomposable_line() {
    // BREAK FOREVER — the honest infinite part (LIFT AND SHIFT LEFT). Individual stems past ~13 need
    // Adams differentials no one has resolved past degree ~90 — that is open mathematics, not a compute
    // problem, and no engine "breaks past" it by running. But the chart has EXACT recurrences that hold
    // to INFINITY, and the engine confirms them at every level we reach:
    //   • Stem 0:  Ext^{s,s} = 1 for ALL s  (the h₀-tower)  ⇒  π₀ˢ = Z — an infinite tower.
    //   • Line 1:  Ext^{1,t} = 1 ⟺ t is a power of 2  (the h_i = Sq^{2ⁱ}) — for all t, forever.
    // These are the parts of the homotopy of spheres we genuinely break to infinity, by the recurrence.
    let e = ext(8, 8);
    for s in 0..=8 {
        assert_eq!(
            e[s][s], 1,
            "h₀ˢ ≠ 0 — the infinite stem-0 tower (π₀ˢ = Z) at s={s}"
        );
    }
    for t in 1..=16 {
        assert_eq!(
            adams_one_line(t),
            usize::from(t.is_power_of_two()),
            "the h_i line, forever, at t={t}"
        );
    }
}

#[test]
fn stable_homotopy_groups_through_the_seven_stem() {
    // WHAT FALLS OUT, PROVEN. Breaking the Steenrod algebra recursively (the minimal free resolution),
    // the STABLE HOMOTOPY GROUPS OF SPHERES fall out stem by stem. Stems 1–7 are each a single
    // h₀-tower, so the 2-local order is 2^(#classes in the stem) = 2^(Σ_s Ext^{s,n+s}). Verified
    // against the KNOWN groups: π₁..₇ˢ = Z/2, Z/2, Z/24, 0, 0, Z/2, Z/240 — i.e. 2-local orders
    // 2,2,8,1,1,2,16. From a SAT solver's symmetry breaking, recursively, to the homotopy of spheres.
    let e = ext(6, 12);
    let stem_total =
        |n: usize| -> usize { (0..=6).filter(|&s| n + s <= 12).map(|s| e[s][n + s]).sum() };
    assert_eq!(stem_total(1), 1, "π₁ˢ = Z/2");
    assert_eq!(stem_total(2), 1, "π₂ˢ = Z/2");
    assert_eq!(stem_total(3), 3, "π₃ˢ = Z/24 (2-local Z/8)");
    assert_eq!(stem_total(4), 0, "π₄ˢ = 0");
    assert_eq!(stem_total(5), 0, "π₅ˢ = 0");
    assert_eq!(stem_total(6), 1, "π₆ˢ = Z/2");
    assert_eq!(stem_total(7), 4, "π₇ˢ = Z/240 (2-local Z/16)");
}

#[test]
fn minimal_resolution_recovers_ext_through_the_three_stem() {
    // THE RECURSIVE SYMMETRY-BREAKING ENGINE. The minimal free resolution computes Ext^{s,t} for ALL
    // s at once — each homological degree s is ONE MORE cycle of symmetry breaking (break the kernel
    // into irreducible generators, recurse on the relations). We break 5 cycles in one call.
    let e = ext(5, 8);
    for t in 1..=8 {
        assert_eq!(
            e[1][t],
            adams_one_line(t),
            "Ext¹ from the engine matches the 1-line at t={t}"
        );
    }
    for t in 2..=8 {
        assert_eq!(
            e[2][t],
            adams_two_line(t),
            "Ext² from the engine matches the 2-line at t={t}"
        );
    }
    // The h₀-tower in stem 0 (Ext^{s,s} = h₀ˢ) = the 2-local integers, every cycle nonzero.
    for s in 0..=5 {
        assert_eq!(e[s][s], 1, "h₀ˢ tower at filtration {s} (stem 0 = Z₂)");
    }
    // π₃ˢ = Z/24: the h₀-tower over h₂ has height exactly 3, then dies — the famous answer.
    assert_eq!(e[1][4], 1, "h₂");
    assert_eq!(e[2][5], 1, "h₀h₂");
    assert_eq!(e[3][6], 1, "h₀²h₂ — tower height 3");
    assert_eq!(
        e[4][7], 0,
        "h₀³h₂ = 0 ⇒ tower ends ⇒ π₃ˢ = Z/24 (2-locally Z/8)"
    );
}

#[test]
fn low_stem_homotopy_from_the_e2_lines() {
    // GET THE ANSWER: the first stable homotopy groups of SPHERES, read off the Adams E₂ chart we
    // built from the Steenrod algebra. Stem 1 (t−s=1): the only class is h₁ — Ext^{1,2}=1, while
    // Ext^{2,3}=h₀h₁=0 kills the h₀-tower above it — so π₁ˢ = Z/2, the Hopf map η. Stem 2: h₁²
    // (Ext^{2,4}=1), and the tower above dies (Ext^{3,5}=0), so π₂ˢ = Z/2 (η²). These are the
    // CORRECT stable stems (π₁ˢ = π₂ˢ = Z/2). Symmetry breaking, from SAT, computed homotopy of spheres.
    assert_eq!(adams_one_line(2), 1, "h₁ generates stem 1");
    assert_eq!(
        adams_two_line(3),
        0,
        "h₀h₁ = 0 — the stem-1 h₀-tower is dead, so π₁ˢ = Z/2"
    );
    assert_eq!(adams_two_line(4), 1, "h₁² generates stem 2 ⇒ π₂ˢ = Z/2");
}

#[test]
fn the_adams_e2_two_line_matches_the_h_i_h_j_chart() {
    // Ext^{2,t} via the MINIMAL FREE RESOLUTION of Z/2 over the Steenrod algebra — the second line of
    // the Adams E₂ page. Verified against the known chart: a basis {h_i h_j : i ≤ j, j ≠ i+1}, with
    // the adjacency relation h_i h_{i+1} = 0. So Ext²₃ = 0 (h₀h₁), Ext²₂ = 1 (h₀²), Ext²₄ = 1 (h₁²),
    // Ext²₅ = 1 (h₀h₂), Ext²₆ = 0 (h₁h₂), … We computed the relations among the Sq^{2ⁱ} from scratch.
    for t in 2..=16 {
        assert_eq!(
            adams_two_line(t),
            known_ext2(t),
            "dim Ext^{{2,t}} = #(h_i h_j) at t={t}"
        );
    }
}

#[test]
fn the_adams_e2_one_line_is_exactly_the_h_i_indecomposables() {
    // INTO THE ADAMS SPECTRAL SEQUENCE. Ext^{1,t}_𝒜(Z/2,Z/2) — the first line of the Adams E₂ page —
    // is the indecomposables of the Steenrod algebra in degree t. Computed as dim 𝒜_t − dim(decomposables_t),
    // it is 1 exactly when t is a power of 2 (the classes h₀, h₁, h₂, … = Sq^{2ⁱ}) and 0 otherwise.
    // h_i sits in stem 2ⁱ−1; h₀,h₁,h₂,h₃ (stems 0,1,3,7) are the Hopf-invariant-one elements. This is
    // the bottom edge of the chart whose far reaches are the unsolved stable stems.
    for t in 1..=16 {
        assert_eq!(
            adams_one_line(t),
            usize::from(t.is_power_of_two()),
            "dim Ext^{{1,t}} = 1 iff t is a power of 2 (the h_i) at t={t}"
        );
    }
}

#[test]
fn sq_n_is_decomposable_iff_n_is_not_a_power_of_two_hopf_invariant_one() {
    // THE CRACK AT THE FRONTIER. Sqⁿ is DECOMPOSABLE (a sum of products of lower squares) iff n is
    // NOT a power of 2 — so the indecomposables of the Steenrod algebra are exactly the Sq^{2ⁱ}. This
    // is the algebraic heart of the HOPF INVARIANT ONE theorem and the reason the real/complex/
    // quaternion/octonion division algebras exist ONLY in dimensions 1, 2, 4, 8. Computed purely from
    // the Adem relations — symmetry breaking of cohomology operations reaching a landmark of topology.
    for n in 2..=17 {
        assert_eq!(
            is_decomposable_sq(n),
            !n.is_power_of_two(),
            "Sqⁿ decomposable ⟺ n not a power of 2 (indecomposables = Sq^{{2ⁱ}}) at n={n}"
        );
    }
}

#[test]
fn adem_reduction_preserves_the_action_on_bz2() {
    // THE CRACK'S CORRECTNESS GATE: rewriting a monomial to admissibles via Adem must not change the
    // operation it represents. For every monomial, its action on H*(BZ/2) equals the (XOR) action of
    // its admissible reduction — verified across Z/2[x]. The algebra computation and the geometry agree.
    let monomials = [
        vec![1, 1],
        vec![1, 2],
        vec![2, 2],
        vec![3, 2],
        vec![2, 3],
        vec![1, 2, 1],
        vec![4, 2],
    ];
    for m in monomials {
        let reduced = adem_reduce(&m);
        for k in 0..=12 {
            let xk = monomial(k);
            let direct = act_monomial(&m, &xk);
            let mut via_basis = vec![0u8];
            for adm in &reduced {
                via_basis = {
                    let a = act_monomial(adm, &xk);
                    let n = via_basis.len().max(a.len());
                    (0..n)
                        .map(|i| {
                            via_basis.get(i).copied().unwrap_or(0) ^ a.get(i).copied().unwrap_or(0)
                        })
                        .collect()
                };
            }
            assert_eq!(
                direct,
                trim(via_basis),
                "monomial {m:?} acts as its admissible reduction at k={k}"
            );
        }
    }
}

#[test]
fn adem_relations_hold_on_bz2() {
    // ADEM RELATIONS — the defining relations of the Steenrod algebra, verified on H*(BZ/2) = Z/2[x]
    // by COMPOSING the operations. Sq¹Sq¹ = 0 was the first; here Sq¹Sq² = Sq³ and Sq²Sq² = Sq³Sq¹.
    // The relations are the structural skeleton of the whole algebra.
    for k in 0..=12 {
        let xk = monomial(k);
        assert_eq!(
            apply_sq(1, &apply_sq(1, &xk)),
            vec![0u8],
            "Adem Sq¹Sq¹ = 0 at k={k}"
        );
        assert_eq!(
            apply_sq(1, &apply_sq(2, &xk)),
            apply_sq(3, &xk),
            "Adem Sq¹Sq² = Sq³ at k={k}"
        );
        assert_eq!(
            apply_sq(2, &apply_sq(2, &xk)),
            apply_sq(3, &apply_sq(1, &xk)),
            "Adem Sq²Sq² = Sq³Sq¹ at k={k}"
        );
    }
}

#[test]
fn the_steenrod_action_on_bz2_is_binomial_coefficients_mod_2() {
    // THE DEEPEST TRUTH: the Steenrod squares on H*(BZ/2; Z/2) = Z/2[x] — determined entirely by the
    // Cartan formula and Sq(x) = x + x² — are EXACTLY the binomial coefficients mod 2:
    // Sqⁱ(xᵏ) = C(k,i)·x^{k+i}. Cohomology operations are governed by a number-theoretic law.
    for k in 0..=10 {
        for i in 0..=k {
            assert_eq!(sq(i, k), (binom(k, i) % 2) as u8, "Sqⁱ(xᵏ) = C(k,i) mod 2");
        }
    }
}

#[test]
fn binomial_mod_2_is_lucas_theorem_bitwise() {
    // …and C(k,i) mod 2 = Lucas' theorem: nonzero iff i's binary 1-bits are a subset of k's
    // (`i & k == i`). The Steenrod action is the bit-containment pattern of the exponents.
    for k in 0..=15usize {
        for i in 0..=k {
            let lucas = u8::from((i & k) == i);
            assert_eq!(
                (binom(k, i) % 2) as u8,
                lucas,
                "C(k,i) mod 2 = [i AND k == i]"
            );
        }
    }
}

#[test]
fn steenrod_axioms_identity_top_square_and_instability() {
    // The defining axioms, recovered: Sq⁰ = identity; Sqᵏ(xᵏ) = x^{2k} (the TOP square = the
    // cup-square from postnikov); Sqⁱ = 0 for i > k (instability — no operation raises degree by more
    // than the class's own degree).
    for k in 1..=8 {
        assert_eq!(sq(0, k), 1, "Sq⁰(xᵏ) = xᵏ");
        assert_eq!(
            sq(k, k),
            1,
            "Sqᵏ(xᵏ) = x^2k — the top square is the cup-square"
        );
        assert_eq!(sq(k + 1, k), 0, "Sqⁱ(xᵏ) = 0 for i > k (instability)");
    }
}

#[test]
fn the_total_square_obeys_the_cartan_formula_ring_homomorphism() {
    // CARTAN: Sq(a·b) = Sq(a)·Sq(b) — the total square is a RING HOMOMORPHISM, the structural heart of
    // the Steenrod algebra. Verified on every product of monomials up to degree 5.
    for i in 0..=5 {
        for j in 0..=5 {
            let lhs = total_square(&monomial(i + j));
            let rhs = trim(mul(
                &total_square(&monomial(i)),
                &total_square(&monomial(j)),
            ));
            assert_eq!(lhs, rhs, "Cartan: Sq(xⁱ⁺ʲ) = Sq(xⁱ)·Sq(xʲ)");
        }
    }
}
