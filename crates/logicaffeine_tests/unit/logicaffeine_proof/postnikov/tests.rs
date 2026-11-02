use super::*;

#[test]
fn the_cup_square_is_a_cohomology_operation_cocycle_to_cocycle() {
    // A cohomology OPERATION sends closed forms to closed forms. The cup-square of an n-cocycle is a
    // 2n-cocycle: δ(x²) = δx∪x ± x∪δx = 0. Checked on the generator x ∈ H¹(BZ/2): x² is a 2-cocycle.
    let g = FiniteGroup::cyclic(2);
    let x = cup_power_z2(1);
    assert!(is_cocycle(&g, 2, 1, &x), "x is a 1-cocycle");
    let sq = cup_square(&g, 2, 1, &x);
    assert!(
        is_cocycle(&g, 2, 2, &sq),
        "Sq¹(x) = x² is a 2-cocycle — cup-square IS a cohomology operation"
    );
}

#[test]
fn the_top_steenrod_square_doubles_the_power_on_bz2() {
    // On H*(BZ/2; Z/2) = Z/2[x], the top square is Sqⁿ(xᵏ) = x²ᵏ — the cup-square doubles the exponent.
    let g = FiniteGroup::cyclic(2);
    for k in 1..=3 {
        assert_eq!(
            cup_square(&g, 2, k, &cup_power_z2(k)),
            cup_power_z2(2 * k),
            "Sqⁿ doubles the exponent at k={k}"
        );
    }
}

#[test]
fn cup_is_graded_commutative_on_cohomology_giving_steenrod_additivity() {
    // The deep fact under Sq's ADDITIVITY: the cup product is graded-commutative UP TO a coboundary
    // — for COCYCLES, `a∪b + b∪a = δ(a∪₁b)` (the cup-1 product; for non-cocycles extra `δa∪₁b` terms
    // appear, so the identity is genuinely a statement about cohomology classes). Over Z/2 this is
    // exactly what forces `(a+b)² = a²+b²` on cohomology. We confirm `a∪b + b∪a` is a coboundary for
    // every pair of 1-COCYCLES on BZ/2 — homotopy-commutativity, the secondary structure made explicit.
    let g = FiniteGroup::cyclic(2);
    let cocycles: Vec<Vec<usize>> = all_cochains(2, 2, 1)
        .into_iter()
        .filter(|c| is_cocycle(&g, 2, 1, c))
        .collect();
    assert!(
        cocycles.len() >= 2,
        "BZ/2 has the cocycles 0 and x in degree 1"
    );
    for a in &cocycles {
        for b in &cocycles {
            let ab = cup(&g, 2, 1, 1, a, b);
            let ba = cup(&g, 2, 1, 1, b, a);
            let sum: Vec<usize> = ab.iter().zip(&ba).map(|(x, y)| (x + y) % 2).collect();
            assert!(
                is_coboundary(&g, 2, 2, &sum),
                "for cocycles a,b: a∪b + b∪a is a coboundary (graded-commutativity on cohomology ⇒ Sq additive)"
            );
        }
    }
}

#[test]
fn sq1_is_the_bockstein_two_independent_constructions_agree() {
    // Sq¹ = the BOCKSTEIN β — the connecting homomorphism of 0→Z/2→Z/4→Z/2→0. Two INDEPENDENT
    // constructions of Sq¹ — the Cartan/binomial formula (steenrod::sq) and this connecting map —
    // must agree: β(xᵏ) = C(k,1)·x^{k+1} mod 2 on H*(BZ/2). A famous theorem, cross-validated.
    let g = FiniteGroup::cyclic(2);
    for k in 1..=4 {
        let got = bockstein(&g, k, &cup_power_z2(k));
        let coeff = crate::steenrod::sq(1, k) as usize; // C(k,1) mod 2
        let expected: Vec<usize> = cup_power_z2(k + 1)
            .iter()
            .map(|&v| (v * coeff) % 2)
            .collect();
        assert_eq!(got, expected, "β(xᵏ) = Sq¹(xᵏ) = C(k,1)·x^(k+1)");
    }
}

#[test]
fn the_bockstein_squares_to_zero_sq1_sq1_equals_zero() {
    // β∘β = 0 — a fundamental relation of the Steenrod algebra (Sq¹Sq¹ = 0). The connecting map of a
    // 2-step extension vanishes on iteration; here, on H*(BZ/2), purely from the Z/4 lift.
    let g = FiniteGroup::cyclic(2);
    for k in 1..=3 {
        let bb = bockstein(&g, k + 1, &bockstein(&g, k, &cup_power_z2(k)));
        assert!(bb.iter().all(|&v| v == 0), "β² = 0 (Sq¹Sq¹ = 0)");
    }
}

#[test]
fn the_bockstein_is_a_cohomology_operation_cocycle_to_cocycle() {
    // β sends closed forms to closed forms — it is a cohomology operation. Checked on the generator.
    let g = FiniteGroup::cyclic(2);
    let bx = bockstein(&g, 1, &cup_power_z2(1));
    assert!(is_cocycle(&g, 2, 2, &bx), "β(x) is a 2-cocycle");
}

#[test]
fn the_product_2type_has_a_trivial_k_invariant() {
    // k = 0 is the PRODUCT 2-type K(G,1) × K(A,2): the levels decouple. The zero 3-cochain is a
    // coboundary (δ of zero), so its class vanishes — the un-twisted, splittable case.
    let g = FiniteGroup::cyclic(2);
    let zero = vec![0usize; g.order().pow(3)];
    assert!(is_cocycle(&g, 2, 3, &zero), "the zero cochain is a cocycle");
    assert!(
        is_coboundary(&g, 2, 3, &zero),
        "k = 0 ⇒ the 2-type is the product, splittable"
    );
}

#[test]
fn a_twisted_2type_has_a_nonzero_k_invariant_and_is_not_a_product() {
    // THE CRUSH: a 2-type that is NOT a product. G = Z/2, A = Z/2, and the cup-cube cocycle
    // α(a,b,c) = a·b·c. It IS a 3-cocycle (a valid associator ⇒ a real 2-type), but it is NOT a
    // coboundary — so its Postnikov class k ∈ H³(Z/2;Z/2) is nonzero. π₁ and π₂ are genuinely twisted
    // together; no choice of section unglues them. This is a single object carrying both homotopy
    // groups WITH their interaction — the frontier the homology ladder and K(A,2) could not reach.
    let g = FiniteGroup::cyclic(2);
    let alpha = cup_cube_z2();
    assert!(
        is_cocycle(&g, 2, 3, &alpha),
        "the cup-cube is a 3-cocycle — a valid 2-type associator"
    );
    assert!(
        !is_coboundary(&g, 2, 3, &alpha),
        "k ≠ 0 — the 2-type is genuinely twisted, NOT a product"
    );
}

#[test]
fn h3_of_z2_with_z2_coefficients_is_exactly_Z2() {
    // Rigour: the whole obstruction group, counted. H³(Z/2; Z/2) = Z/2 — exactly two classes, the
    // product (k=0) and the twist (k≠0). So there is, up to equivalence, exactly ONE nontrivial
    // 2-type on these groups, and we hold its invariant.
    let g = FiniteGroup::cyclic(2);
    assert_eq!(
        cohomology_size(&g, 2, 3),
        2,
        "H³(Z/2; Z/2) = Z/2 — product and twist, nothing else"
    );
}

#[test]
fn the_next_obstruction_group_h4_is_also_nonzero() {
    // CLIMBING THE TOWER: the next k-invariant k₄ lives in H⁴, and for Z/2 it too is nonzero —
    // H⁴(Z/2; Z/2) = Z/2. There is a genuine twist available at level 3→4 as well, not just 1→2.
    let g = FiniteGroup::cyclic(2);
    assert_eq!(
        cohomology_size(&g, 2, 4),
        2,
        "H⁴(Z/2; Z/2) = Z/2 — the next obstruction is nonzero too"
    );
}

#[test]
fn an_explicit_nonzero_obstruction_at_every_checked_level_3_4_5() {
    // The cup-power xⁿ is an explicit nonzero class at each level: a cocycle (valid associator) that
    // is NOT a coboundary (genuinely twisted). We exhibit it at levels 3, 4, 5 — so the Postnikov
    // obstruction does not fizzle out as we climb; a real twist is available at every rung checked.
    let g = FiniteGroup::cyclic(2);
    for n in 3..=5 {
        let x_n = cup_power_z2(n);
        assert!(
            is_cocycle(&g, 2, n, &x_n),
            "xⁿ is an n-cocycle at level {n}"
        );
        assert!(
            !is_coboundary(&g, 2, n, &x_n),
            "xⁿ is NOT a coboundary at level {n} — a real obstruction"
        );
    }
}

#[test]
fn the_obstructions_form_a_graded_ring_cup_product_adds_the_levels() {
    // ONE MORE STRUCTURE: the obstructions don't merely sit in a sequence — they MULTIPLY. The cup
    // product binds the levels: xⁱ ∪ xʲ = xⁱ⁺ʲ, so H*(Z/2; Z/2) = Z/2[x], a polynomial ring with one
    // generator in each degree. The whole infinite ladder of symmetry-breaking obstructions is a
    // single graded algebra — the cohomology ring of BG = K(Z/2,1), the symmetry's classifying space.
    let g = FiniteGroup::cyclic(2);
    for i in 1..=3 {
        for j in 1..=3 {
            let product = cup(&g, 2, i, j, &cup_power_z2(i), &cup_power_z2(j));
            assert_eq!(
                product,
                cup_power_z2(i + j),
                "xⁱ ∪ xʲ = xⁱ⁺ʲ ⇒ the ring is Z/2[x]"
            );
        }
    }
}

#[test]
fn the_postnikov_ladder_of_obstructions_climbs_without_end() {
    // THE ∞, made concrete and honest. The Postnikov tower glues in πₙ by a class kₙ₊₁ ∈ Hⁿ⁺¹, and
    // for Z/2 every one of these obstruction groups is nonzero with an explicit generator. So the
    // ladder of genuinely-twisted higher types never terminates — there is a nonzero
    // symmetry-breaking obstruction at every level we can reach. This does not claim BZ/2 itself has
    // higher homotopy (it is K(Z/2,1)); it shows the RECEPTACLES for higher twists are nonzero at
    // every level, so the ∞-tower of possible twists is genuinely infinite. The obstructions don't
    // stop — that is the ∞ in ∞-groupoid, held as a checkable fact rung by rung, not an article of faith.
    let g = FiniteGroup::cyclic(2);
    for n in 3..=5 {
        assert!(
            is_cocycle(&g, 2, n, &cup_power_z2(n)) && !is_coboundary(&g, 2, n, &cup_power_z2(n)),
            "a nonzero obstruction exists at level {n} — the ladder climbs past it, without end"
        );
    }
}

#[test]
fn the_k_invariant_is_the_obstruction_to_breaking_the_2type_into_a_product() {
    // SYMMETRY BREAKING, the higher sibling of the deadlock β₁. Splitting a 2-type into a product is
    // the ultimate symmetry break — decoupling π₁ from π₂. It succeeds iff the k-invariant is a
    // coboundary (k = 0). The twist's class is NOT, so the split is OBSTRUCTED: k counts exactly how
    // far the higher symmetry fails to break, just as β₁ counted the scheduler obstruction.
    let g = FiniteGroup::cyclic(2);
    let splittable = is_coboundary(&g, 2, 3, &vec![0usize; 8]);
    let twisted_splittable = is_coboundary(&g, 2, 3, &cup_cube_z2());
    assert!(
        splittable,
        "k = 0 ⇒ the symmetry breaks: the 2-type is a product"
    );
    assert!(
        !twisted_splittable,
        "k ≠ 0 ⇒ the break is obstructed — the levels cannot be decoupled"
    );
}
