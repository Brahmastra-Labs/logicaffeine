use super::*;

/// The multiplicative structure of the Adams E₂ page — the cohomology ring of the Steenrod algebra —
/// DERIVED from the minimal resolution, not asserted. The product is the Yoneda composition: lift the
/// cocycle dual to a generator to a comparison chain map `φ_•` of the resolution (the diagonal
/// `X → X×X` dualized) and pair. The structural relations of the one-line classes `h_i ∈ Ext^{1,2ⁱ}`
/// then fall out of the algebra:
///
///   * each cup-square `h_i²` is nonzero (the indecomposables square nontrivially),
///   * the non-adjacent products `h_0h_2`, `h_0h_3`, `h_1h_3` survive,
///   * the Adem-adjacency relations `h_i h_{i+1} = 0` hold,
///   * the product is commutative (the Steenrod algebra is cocommutative),
///   * and the triple relation `h_1³ = h_0² h_2` — the first genuinely non-dimensional fact about the
///     ring, the generator of Ext^{3,6} — is recovered as an equality of computed products.
#[test]
fn the_h_i_products_obey_the_known_ext_ring_relations() {
    let res = minimal_resolution(4, 18);
    let h = |i: usize| {
        res[1]
            .iter()
            .position(|g| g.degree == (1 << i))
            .expect("h_i generator")
    };
    let prod = |a_s, a, b_s, b| yoneda_product(&res, a_s, a, b_s, b);

    for i in 0..=3 {
        assert!(
            !prod(1, h(i), 1, h(i)).is_empty(),
            "cup-square h_{i}² is nonzero"
        );
    }
    assert_eq!(
        prod(1, h(0), 1, h(0)).len(),
        1,
        "h_0² spans the 1-dim Ext^{{2,2}}"
    );

    assert!(!prod(1, h(0), 1, h(2)).is_empty(), "h_0 h_2 ≠ 0");
    assert!(!prod(1, h(0), 1, h(3)).is_empty(), "h_0 h_3 ≠ 0");
    assert!(!prod(1, h(1), 1, h(3)).is_empty(), "h_1 h_3 ≠ 0");

    assert!(
        prod(1, h(0), 1, h(1)).is_empty(),
        "h_0 h_1 = 0 (Adem adjacency)"
    );
    assert!(
        prod(1, h(1), 1, h(2)).is_empty(),
        "h_1 h_2 = 0 (Adem adjacency)"
    );
    assert!(
        prod(1, h(2), 1, h(3)).is_empty(),
        "h_2 h_3 = 0 (Adem adjacency)"
    );

    assert_eq!(
        prod(1, h(0), 1, h(2)),
        prod(1, h(2), 1, h(0)),
        "commutativity h_0 h_2 = h_2 h_0"
    );
    assert_eq!(
        prod(1, h(1), 1, h(3)),
        prod(1, h(3), 1, h(1)),
        "commutativity h_1 h_3 = h_3 h_1"
    );

    // The triple relation h_1³ = h_0² h_2 in Ext^{3,6}. Each intermediate square is a single generator,
    // so the cube and h_0²·h_2 are well-defined classes; both must equal the lone Ext^{3,6} generator.
    let h1sq = prod(1, h(1), 1, h(1));
    let h0sq = prod(1, h(0), 1, h(0));
    assert_eq!(h1sq.len(), 1, "h_1² spans Ext^{{2,4}}");
    assert_eq!(h0sq.len(), 1, "h_0² spans Ext^{{2,2}}");
    let h1_cubed = yoneda_product(&res, 2, h1sq[0], 1, h(1));
    let h0sq_h2 = yoneda_product(&res, 2, h0sq[0], 1, h(2));
    assert!(!h1_cubed.is_empty(), "h_1³ ≠ 0");
    assert_eq!(h1_cubed, h0sq_h2, "h_1³ = h_0² h_2 — derived, not assumed");
}
