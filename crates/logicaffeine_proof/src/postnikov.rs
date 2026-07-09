//! The **Postnikov `k`-invariant** — the gluing data of a 2-type, and the obstruction to breaking it
//! into a product.
//!
//! `two_type` built `K(A,2)` and `kan_complex` built `K(G,1)`. A general 2-truncated ∞-groupoid is *not*
//! just a `π₁` and a `π₂` sitting side by side: by Mac Lane–Whitehead / Sinh's theorem it is classified
//! by `(π₁ = G, π₂ = A, the G-action, k ∈ H³(G; A))`. The class `k` — the **Postnikov invariant** — is
//! the associator of the corresponding 2-group, and it says how the two levels are *twisted*:
//!
//! - `k = 0` ⟺ the 2-type is the **product** `K(G,1) × K(A,2)` — the levels decouple.
//! - `k ≠ 0` ⟺ a genuinely **entangled** 2-type, provably *not* a product.
//!
//! So `k` is "the single object carrying all `πₙ` *with their interactions*" reduced to one checkable
//! cohomology class. And it is, once more, **a symmetry-breaking obstruction**: splitting the 2-type into
//! a product is the ultimate symmetry break (decoupling `π₁` from `π₂`), and `k ≠ 0` is exactly the
//! obstruction to it — the higher sibling of the deadlock `β₁`. We compute group cohomology directly
//! (`A = Z/modulus`, trivial action) and crush the canonical case `H³(Z/2; Z/2) = Z/2`: the cup-cube
//! cocycle is a 3-cocycle that is *not* a coboundary, so its 2-type is no product.

use crate::two_group::FiniteGroup;

fn idx(order: usize, t: &[usize]) -> usize {
    t.iter().fold(0usize, |a, &x| a * order + x)
}

fn tuples(order: usize, n: usize) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = vec![vec![]];
    for _ in 0..n {
        let mut next = Vec::with_capacity(out.len() * order);
        for t in &out {
            for e in 0..order {
                let mut u = t.clone();
                u.push(e);
                next.push(u);
            }
        }
        out = next;
    }
    out
}

/// Every `n`-cochain `Gⁿ → A` (`A = Z/modulus`): `modulus^{order^n}` of them.
fn all_cochains(order: usize, modulus: usize, n: usize) -> Vec<Vec<usize>> {
    let len = order.pow(n as u32);
    let mut out: Vec<Vec<usize>> = vec![vec![]];
    for _ in 0..len {
        let mut next = Vec::with_capacity(out.len() * modulus);
        for t in &out {
            for v in 0..modulus {
                let mut u = t.clone();
                u.push(v);
                next.push(u);
            }
        }
        out = next;
    }
    out
}

/// The group-cohomology coboundary `δⁿ : Cⁿ(G; A) → Cⁿ⁺¹(G; A)`, with `A = Z/modulus` and **trivial**
/// `G`-action. `f` is a length-`order^n` table; the result is length `order^{n+1}`.
pub fn coboundary(g: &FiniteGroup, modulus: usize, n: usize, f: &[usize]) -> Vec<usize> {
    let order = g.order();
    let mut out = vec![0usize; order.pow((n + 1) as u32)];
    for t in tuples(order, n + 1) {
        // (δf)(g_1,…,g_{n+1}) = f(g_2,…,g_{n+1}) + Σ_{i=1}^n (−1)^i f(…,g_i g_{i+1},…) + (−1)^{n+1} f(g_1,…,g_n)
        let mut val: i64 = f[idx(order, &t[1..])] as i64;
        for i in 1..=n {
            let mut merged = t[..i - 1].to_vec();
            merged.push(g.mul[t[i - 1]][t[i]]);
            merged.extend_from_slice(&t[i + 1..]);
            let sign = if i % 2 == 0 { 1 } else { -1 };
            val += sign * f[idx(order, &merged)] as i64;
        }
        let sign = if (n + 1) % 2 == 0 { 1 } else { -1 };
        val += sign * f[idx(order, &t[..n])] as i64;
        out[idx(order, &t)] = val.rem_euclid(modulus as i64) as usize;
    }
    out
}

/// Is `f` an `n`-cocycle? (`δf = 0`)
pub fn is_cocycle(g: &FiniteGroup, modulus: usize, n: usize, f: &[usize]) -> bool {
    coboundary(g, modulus, n, f).iter().all(|&x| x == 0)
}

/// Is `f` an `n`-coboundary? (`f = δβ` for some `(n-1)`-cochain `β`)
pub fn is_coboundary(g: &FiniteGroup, modulus: usize, n: usize, f: &[usize]) -> bool {
    all_cochains(g.order(), modulus, n - 1).iter().any(|b| coboundary(g, modulus, n - 1, b) == f)
}

/// `|Hⁿ(G; A)|` — cocycles modulo coboundaries (both are subgroups of `Cⁿ`, so the order is the index).
pub fn cohomology_size(g: &FiniteGroup, modulus: usize, n: usize) -> usize {
    let cocycles = all_cochains(g.order(), modulus, n).into_iter().filter(|f| is_cocycle(g, modulus, n, f)).count();
    let mut coboundaries: std::collections::HashSet<Vec<usize>> = std::collections::HashSet::new();
    for b in all_cochains(g.order(), modulus, n - 1) {
        coboundaries.insert(coboundary(g, modulus, n - 1, &b));
    }
    cocycles / coboundaries.len()
}

/// The **cup product** `Cᵖ × Cᵠ → Cᵖ⁺ᵠ` (Alexander–Whitney; `Z/modulus` coefficients, trivial action,
/// no signs): `(f ∪ h)(a₁,…,a_{p+q}) = f(a₁,…,a_p) · h(a_{p+1},…,a_{p+q})`. It makes the obstruction
/// groups a graded ring `H*(G; A)` — the multiplicative algebra binding all the k-invariant levels.
pub fn cup(g: &FiniteGroup, modulus: usize, p: usize, q: usize, f: &[usize], h: &[usize]) -> Vec<usize> {
    let order = g.order();
    let mut out = vec![0usize; order.pow((p + q) as u32)];
    for t in tuples(order, p + q) {
        out[idx(order, &t)] = (f[idx(order, &t[..p])] * h[idx(order, &t[p..])]) % modulus;
    }
    out
}

/// The **cup-square** cohomology operation `x ↦ x ∪ x : Hⁿ(−; A) → H²ⁿ(−; A)`. Over `Z/2` it is the TOP
/// Steenrod square `Sqⁿ` — the first genuine *cohomology operation* (a natural transformation of the
/// functor `Hⁿ(−;A)`), the secondary structure carried by the cohomology the Eilenberg–MacLane spectrum
/// represents.
pub fn cup_square(g: &FiniteGroup, modulus: usize, n: usize, f: &[usize]) -> Vec<usize> {
    cup(g, modulus, n, n, f, f)
}

/// The **Bockstein** `β : Hⁿ(−; Z/2) → Hⁿ⁺¹(−; Z/2)` — the connecting homomorphism of the short exact
/// sequence `0 → Z/2 →(×2) Z/4 → Z/2 → 0`. By a theorem of Steenrod, `β = Sq¹`. Computed honestly: lift
/// the `Z/2`-cochain to `Z/4`, take `δ` over `Z/4` (which is even on a `Z/2`-cocycle), divide by 2, and
/// reduce mod 2. The lowest genuine Steenrod square, reached as a connecting map rather than a cup-square.
pub fn bockstein(g: &FiniteGroup, n: usize, c: &[usize]) -> Vec<usize> {
    let lifted: Vec<usize> = c.iter().map(|&v| v % 4).collect();
    let d = coboundary(g, 4, n, &lifted); // δ over Z/4
    d.iter().map(|&v| (v / 2) % 2).collect()
}

/// The cup-power `xⁿ` of `Z/2` with `Z/2` coefficients: `α(g_1,…,g_n) = g_1·g_2·⋯·g_n` (1 iff every
/// argument is 1). As the `n`-fold cup product of the nonzero 1-cocycle, it is an `n`-cocycle and the
/// generator of `Hⁿ(Z/2; Z/2) = Z/2` — the explicit nonzero obstruction at level `n`.
pub fn cup_power_z2(n: usize) -> Vec<usize> {
    let len = 2usize.pow(n as u32);
    let mut f = vec![0usize; len];
    f[len - 1] = 1; // only the all-ones tuple (1,1,…,1) has product 1
    f
}

/// The canonical nontrivial 3-cocycle (the cup-cube `x³`), generator of `H³(Z/2; Z/2) = Z/2`.
pub fn cup_cube_z2() -> Vec<usize> {
    cup_power_z2(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cup_square_is_a_cohomology_operation_cocycle_to_cocycle() {
        // A cohomology OPERATION sends closed forms to closed forms. The cup-square of an n-cocycle is a
        // 2n-cocycle: δ(x²) = δx∪x ± x∪δx = 0. Checked on the generator x ∈ H¹(BZ/2): x² is a 2-cocycle.
        let g = FiniteGroup::cyclic(2);
        let x = cup_power_z2(1);
        assert!(is_cocycle(&g, 2, 1, &x), "x is a 1-cocycle");
        let sq = cup_square(&g, 2, 1, &x);
        assert!(is_cocycle(&g, 2, 2, &sq), "Sq¹(x) = x² is a 2-cocycle — cup-square IS a cohomology operation");
    }

    #[test]
    fn the_top_steenrod_square_doubles_the_power_on_bz2() {
        // On H*(BZ/2; Z/2) = Z/2[x], the top square is Sqⁿ(xᵏ) = x²ᵏ — the cup-square doubles the exponent.
        let g = FiniteGroup::cyclic(2);
        for k in 1..=3 {
            assert_eq!(cup_square(&g, 2, k, &cup_power_z2(k)), cup_power_z2(2 * k), "Sqⁿ doubles the exponent at k={k}");
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
        let cocycles: Vec<Vec<usize>> = all_cochains(2, 2, 1).into_iter().filter(|c| is_cocycle(&g, 2, 1, c)).collect();
        assert!(cocycles.len() >= 2, "BZ/2 has the cocycles 0 and x in degree 1");
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
            let expected: Vec<usize> = cup_power_z2(k + 1).iter().map(|&v| (v * coeff) % 2).collect();
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
        assert!(is_coboundary(&g, 2, 3, &zero), "k = 0 ⇒ the 2-type is the product, splittable");
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
        assert!(is_cocycle(&g, 2, 3, &alpha), "the cup-cube is a 3-cocycle — a valid 2-type associator");
        assert!(!is_coboundary(&g, 2, 3, &alpha), "k ≠ 0 — the 2-type is genuinely twisted, NOT a product");
    }

    #[test]
    fn h3_of_z2_with_z2_coefficients_is_exactly_Z2() {
        // Rigour: the whole obstruction group, counted. H³(Z/2; Z/2) = Z/2 — exactly two classes, the
        // product (k=0) and the twist (k≠0). So there is, up to equivalence, exactly ONE nontrivial
        // 2-type on these groups, and we hold its invariant.
        let g = FiniteGroup::cyclic(2);
        assert_eq!(cohomology_size(&g, 2, 3), 2, "H³(Z/2; Z/2) = Z/2 — product and twist, nothing else");
    }

    #[test]
    fn the_next_obstruction_group_h4_is_also_nonzero() {
        // CLIMBING THE TOWER: the next k-invariant k₄ lives in H⁴, and for Z/2 it too is nonzero —
        // H⁴(Z/2; Z/2) = Z/2. There is a genuine twist available at level 3→4 as well, not just 1→2.
        let g = FiniteGroup::cyclic(2);
        assert_eq!(cohomology_size(&g, 2, 4), 2, "H⁴(Z/2; Z/2) = Z/2 — the next obstruction is nonzero too");
    }

    #[test]
    fn an_explicit_nonzero_obstruction_at_every_checked_level_3_4_5() {
        // The cup-power xⁿ is an explicit nonzero class at each level: a cocycle (valid associator) that
        // is NOT a coboundary (genuinely twisted). We exhibit it at levels 3, 4, 5 — so the Postnikov
        // obstruction does not fizzle out as we climb; a real twist is available at every rung checked.
        let g = FiniteGroup::cyclic(2);
        for n in 3..=5 {
            let x_n = cup_power_z2(n);
            assert!(is_cocycle(&g, 2, n, &x_n), "xⁿ is an n-cocycle at level {n}");
            assert!(!is_coboundary(&g, 2, n, &x_n), "xⁿ is NOT a coboundary at level {n} — a real obstruction");
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
                assert_eq!(product, cup_power_z2(i + j), "xⁱ ∪ xʲ = xⁱ⁺ʲ ⇒ the ring is Z/2[x]");
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
        assert!(splittable, "k = 0 ⇒ the symmetry breaks: the 2-type is a product");
        assert!(!twisted_splittable, "k ≠ 0 ⇒ the break is obstructed — the levels cannot be decoupled");
    }
}
