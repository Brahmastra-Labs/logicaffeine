use super::*;

/// The chain-level diagonal `Δ₀: C → C⊗C` is built independently of the Yoneda comparison map, yet the
/// cup-square it computes — `(dual_x ⊗ dual_x)∘Δ₀`, reading the `((1,gi,1),(1,gi,1))` coefficient on the
/// `C_2` generators — must agree with `yoneda_product(x, x)` for every one-line class `x = h_i`. Two
/// independent constructions of the same cup-square agreeing is the correctness oracle that licenses the
/// diagonal; the cup-`i` refinements built on it then carry the Steenrod operations on Ext.
#[test]
fn the_chain_level_diagonal_reproduces_the_yoneda_cup_squares() {
    let res = minimal_resolution(2, 8);
    let delta = diagonal(&res, 2);
    let h = |i: usize| {
        res[1]
            .iter()
            .position(|g| g.degree == (1 << i))
            .expect("h_i generator")
    };
    for i in 0..=2 {
        let gi = h(i);
        let want = 2 * (1 << i);
        let mut via_diagonal: Vec<usize> = (0..res[2].len())
            .filter(|&k| {
                res[2][k].degree == want && delta[2][&k].contains(&(1, gi, vec![], 1, gi, vec![]))
            })
            .collect();
        via_diagonal.sort_unstable();
        let mut via_yoneda = yoneda_product(&res, 1, gi, 1, gi);
        via_yoneda.sort_unstable();
        assert!(!via_diagonal.is_empty(), "h_{i}² ≠ 0");
        assert_eq!(
            via_diagonal, via_yoneda,
            "Δ₀ reproduces the Yoneda cup-square h_{i}²"
        );
    }
}

/// The first higher coherence — the cup-1 homotopy `Δ₁` — derives the Adams **`Sq⁰` doubling** on the
/// one-line classes: `Sq⁰(h_i) = h_{i+1}`. This is the symmetry the whole differential family rides on
/// (`d₂(h_{i+1}) = h₀h_i²`); here it is computed, not assumed — `Sq⁰` falls straight out of the
/// secondary structure of the chain-level diagonal, exactly the geometric seam under the `h₀` seam.
#[test]
fn the_cup_1_homotopy_derives_the_sq0_doubling_on_the_h_line() {
    let res = minimal_resolution(2, 8);
    let delta = diagonal(&res, 1);
    let delta1 = diagonal_1(&res, 1, &delta);
    let h = |i: usize| {
        res[1]
            .iter()
            .position(|g| g.degree == (1 << i))
            .expect("h_i generator")
    };
    for i in 0..=2 {
        assert_eq!(
            sq0_on_h(&res, &delta1, h(i)),
            vec![h(i + 1)],
            "Sq⁰(h_{i}) = h_(i+1) — the doubling, derived from the cup-1 homotopy"
        );
    }
}

/// The `Sq⁰` doubling is a **ring endomorphism** of Ext — the algebraic Frobenius. Through the cup-2
/// homotopy it acts on the second-line classes, and `Sq⁰(h_i²) = (Sq⁰ h_i)² = h_{i+1}²`: the doubling
/// commutes with the cup product. The left side is computed from `Δ₂` (cup-2 self-pairing of the Ext²
/// class `h_i²`); the right is the ordinary cup-square of `h_{i+1}`. Their agreement is the
/// multiplicativity of the doubling, derived — the same symmetry as `Sq⁰(h_i)=h_{i+1}`, one filtration up.
#[test]
fn the_sq0_doubling_is_a_ring_endomorphism_on_the_squares() {
    let res = minimal_resolution(4, 8);
    let delta = diagonal(&res, 2);
    let delta1 = diagonal_1(&res, 2, &delta);
    let delta2 = cup_homotopy(&res, 2, 2, &delta1);
    let h = |i: usize| {
        res[1]
            .iter()
            .position(|g| g.degree == (1 << i))
            .expect("h_i generator")
    };
    for i in 0..=1 {
        let hi_sq = yoneda_product(&res, 1, h(i), 1, h(i));
        assert_eq!(hi_sq.len(), 1, "h_i² is a single Ext² generator");
        let mut lhs = sq0_on_ext2(&res, &delta2, hi_sq[0]);
        lhs.sort_unstable();
        let mut rhs = yoneda_product(&res, 1, h(i + 1), 1, h(i + 1));
        rhs.sort_unstable();
        assert!(!rhs.is_empty(), "h_(i+1)² ≠ 0");
        assert_eq!(
            lhs, rhs,
            "Sq⁰(h_i²) = h_(i+1)² — the doubling is a ring endomorphism"
        );
    }
}

/// Automating the doubling: build the cup-diagonal tower once, then sweep `Sq⁰` across the low Adams
/// chart and harvest everything it nets in range — the indecomposables `h_i ↦ h_{i+1}` and the products
/// `h_i h_j ↦ h_{i+1} h_{j+1}`, each image cross-checked against the independently-computed ring product.
/// The sweep reaches past the squares to the mixed product `h_0 h_2 ↦ h_1 h_3`. One uniform operator,
/// the whole symmetry applied mechanically.
#[test]
fn the_sq0_doubling_automates_across_the_low_adams_chart() {
    let res = minimal_resolution(4, 10);
    let deltas = cup_diagonals(&res, 2, 2);
    let h = |i: usize| {
        res[1]
            .iter()
            .position(|g| g.degree == (1 << i))
            .expect("h_i generator")
    };
    let mut harvested: Vec<String> = Vec::new();

    for i in 0..=2 {
        assert_eq!(
            sq0(&res, &deltas, 1, h(i)),
            vec![h(i + 1)],
            "Sq⁰(h_i) = h_(i+1)"
        );
        harvested.push(format!("Sq0(h{i})=h{}", i + 1));
    }

    for i in 0..=2 {
        for j in i..=2 {
            let prod = yoneda_product(&res, 1, h(i), 1, h(j));
            if prod.len() != 1 || 2 * res[2][prod[0]].degree > 10 {
                continue;
            }
            let mut got = sq0(&res, &deltas, 2, prod[0]);
            got.sort_unstable();
            let mut want = yoneda_product(&res, 1, h(i + 1), 1, h(j + 1));
            want.sort_unstable();
            assert_eq!(
                got, want,
                "Sq⁰(h_i h_j) = h_(i+1) h_(j+1) — harvested automatically"
            );
            harvested.push(format!("Sq0(h{i}h{j})=h{}h{}", i + 1, j + 1));
        }
    }

    assert!(
        harvested.iter().any(|s| s == "Sq0(h0h2)=h1h3"),
        "sweep reaches the mixed product h_0h_2 ↦ h_1h_3"
    );
    assert!(
        harvested.len() >= 6,
        "doubling harvested ≥6 chart facts automatically: {harvested:?}"
    );
}
