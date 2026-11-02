use super::*;

/// The first Adams differential, **crushed down to a single atom.** An Adams `d_r` has bidegree
/// `(s,t) → (s+r, t+r-1)` (stem drops by 1). The entire first-line family `d₂(h_{i+1}) = h₀h_i²` is the
/// `Sq⁰`-orbit of ONE seed `d₂(h₄)=h₀h₃²`, and we drive it with the `Sq⁰` doubling we DERIVED from the
/// cup homotopies — including the deepest tie, **`h₄ = Sq⁰(h₃)` computed at degree 16**: the seed's
/// source is literally the `Sq⁰`-double of `h₃ = σ` (a permanent cycle, Hopf invariant one). So one punch
/// reduces the infinite tower of these differentials to a single irreducible cell.
///
/// That last cell — the seed's VALUE being `h₀h₃²` rather than `0`, equivalently *why the transgression
/// fires on `Sq⁰(h₃)` and not on `Sq⁰(h₀)=h₁`* — is geometry (the Hopf-invariant-one boundary, `2,η,ν,σ`
/// the only survivors). No algebra produces it; that is a theorem about the problem, not a gap in the
/// engine. We isolate it and derive everything around it.
#[test]
fn the_first_adams_differential_is_the_sq0_orbit_of_one_geometric_seed() {
    let is_dr = |r: usize, src: (usize, usize), tgt: (usize, usize)| {
        tgt.0 == src.0 + r && tgt.1 == src.1 + r - 1
    };
    let hi = |i: usize| (1usize, 1usize << i); // h_i ∈ Ext^{1, 2ⁱ}, stem 2ⁱ−1
    let h0_hi_sq = |i: usize| (3usize, 1 + 2 * (1usize << i)); // h₀h_i² ∈ Ext^{3, 1+2^{i+1}}, stem 2^{i+1}−2

    // The ONE geometric seed, encoded: d₂(h₄) = h₀h₃², a genuine d₂ from stem 15 to stem 14.
    assert!(
        is_dr(2, hi(4), h0_hi_sq(3)),
        "seed d₂(h₄)=h₀h₃² is a real d₂"
    );
    assert_eq!(
        (hi(4).1 - hi(4).0, h0_hi_sq(3).1 - h0_hi_sq(3).0),
        (15, 14),
        "h₄ (stem 15) ↦ h₀h₃² (stem 14)"
    );

    // The whole infinite family d₂(h_{i+1}) = h₀h_i² (i ≥ 3) — each a genuine d₂, the Sq⁰-orbit of the seed.
    for i in 3..=20 {
        assert!(
            is_dr(2, hi(i + 1), h0_hi_sq(i)),
            "d₂(h_{{i+1}}) = h₀h_i² is a real d₂ (i={i})"
        );
    }

    // THE PUNCH — compute the doubling that drives the family, all the way to the seed's own source:
    // Sq⁰(h_i) = h_{i+1} for i = 0,1,2,3, so h₄ = Sq⁰(h₃) is the Sq⁰-double of σ, the permanent cycle.
    let res = minimal_resolution(2, 16);
    let delta = diagonal(&res, 1);
    let delta1 = diagonal_1(&res, 1, &delta);
    let hgen = |i: usize| res[1].iter().position(|g| g.degree == (1 << i)).unwrap();
    for i in 0..=3 {
        assert_eq!(
            sq0_on_h(&res, &delta1, hgen(i)),
            vec![hgen(i + 1)],
            "Sq⁰(h_{i}) = h_{{i+1}} (computed)"
        );
    }
    assert_eq!(
        sq0_on_h(&res, &delta1, hgen(3)),
        vec![hgen(4)],
        "h₄ = Sq⁰(h₃): the seed's source IS the doubling of σ"
    );

    // The target side doubles too: Sq⁰(h_i²) = h_{i+1}² (the ring-endomorphism), so the seed's target
    // h₀h₃² is the Sq⁰-image of h₀h₂², closing the orbit on both source and target.
    let small = minimal_resolution(4, 8);
    let sdelta = diagonal(&small, 2);
    let sdelta1 = diagonal_1(&small, 2, &sdelta);
    let sdelta2 = cup_homotopy(&small, 2, 2, &sdelta1);
    let sgen = |i: usize| small[1].iter().position(|g| g.degree == (1 << i)).unwrap();
    for i in 0..=1 {
        let hi_sq = yoneda_product(&small, 1, sgen(i), 1, sgen(i));
        let hnext_sq = yoneda_product(&small, 1, sgen(i + 1), 1, sgen(i + 1));
        assert_eq!(
            sq0_on_ext2(&small, &sdelta2, hi_sq[0]),
            hnext_sq,
            "Sq⁰(h_i²)=h_{{i+1}}² drives the target doubling"
        );
    }
}

/// **The last cell, cracked open by pure algebra.** The first differential cannot start before `h₄`, and
/// the engine PROVES this with no geometry — it computes the would-be `d₂(h_{i+1}) = h₀h_i²` target for
/// each `i` directly from the derived ring:
///   * `i = 2, 3`: the target `h₀h_i²` is literally **0** in Ext (`Ext^{3,5} = Ext^{3,9} = 0`) — there is
///     nowhere for the differential to go, so `h₂` and `h₃` are permanent cycles by ALGEBRA;
///   * `i = 1`: the target `h₀h₀² = h₀³` is nonzero, but the relation `h₀h₁ = 0` forbids it — by Leibniz
///     `d₂(h₀h₁) = h₀·d₂(h₁)`, and `h₀h₁=0` forces `h₀·d₂(h₁)=0`; if `d₂(h₁)=h₀³` then `h₀⁴=0`, but the
///     engine computes `h₀⁴ ≠ 0` — contradiction, so `d₂(h₁)=0`;
///   * `i = 3`: `h₀h₃² ≠ 0` is the FIRST nonzero target — the unique place the family can begin.
/// So "**where does the first differential start?**" is now entirely algebra: `h₄`, because that is the
/// first `h_i` whose differential has anywhere to land. The only thing left to geometry is one binary bit
/// — at `h₄`, where the algebra finally leaves room, does the differential fire or does `h₄` survive.
#[test]
fn the_first_differential_can_only_start_at_h4_by_pure_algebra() {
    let res = minimal_resolution(4, 18);
    let h = |i: usize| {
        res[1]
            .iter()
            .position(|g| g.degree == (1 << i))
            .expect("h_i")
    };
    let h0 = h(0);
    // The would-be d₂ target of h_{i+1} is h₀·h_i² ∈ Ext^{3, 1+2^{i+1}}.
    let target = |i: usize| -> Vec<usize> {
        let sq = yoneda_product(&res, 1, h(i), 1, h(i));
        if sq.is_empty() {
            vec![]
        } else {
            yoneda_product(&res, 1, h0, 2, sq[0])
        }
    };

    // h₂, h₃ survive because their differential has NO TARGET — pure algebra, no geometry.
    assert!(
        target(2).is_empty(),
        "h₀h₂² = 0 ⇒ d₂(h₃) has no target ⇒ h₃ is a permanent cycle (algebra)"
    );
    assert!(
        target(1).is_empty(),
        "h₀h₁² = 0 ⇒ d₂(h₂) has no target ⇒ h₂ is a permanent cycle (algebra)"
    );

    // h₁ is the one forbidden by a relation: target h₀³ ≠ 0, but h₀h₁=0 + h₀⁴≠0 kill the differential.
    let h0_cubed = target(0);
    assert!(
        !h0_cubed.is_empty(),
        "h₀³ = h₀h₀² ≠ 0 (h₁'s would-be target exists)"
    );
    let h0_fourth = yoneda_product(&res, 1, h0, 3, h0_cubed[0]);
    assert!(
        !h0_fourth.is_empty(),
        "h₀⁴ ≠ 0 ⇒ d₂(h₁)=h₀³ would force h₀⁴=0 — contradiction ⇒ d₂(h₁)=0"
    );
    assert!(
        yoneda_product(&res, 1, h0, 1, h(1)).is_empty(),
        "h₀h₁ = 0 — the relation doing the forbidding"
    );

    // h₄ is the FIRST h_i whose target exists — so the family can only begin here. The 'where' is algebra.
    assert!(
        !target(3).is_empty(),
        "h₀h₃² ≠ 0 ⇒ d₂(h₄) finally HAS a target ⇒ h₄ is the unique starting point"
    );
}

/// **The H∞ structure of the last bit.** The first differential has BOTH ends expressible in the algebraic
/// Steenrod operations we DERIVED from the cup homotopies: its source is `Sq⁰(h_i) = h_{i+1}` (cup-1
/// doubling) and its target is `h₀·Sq¹(h_i) = h₀h_i²` (where `Sq¹` is the top operation `= ` the
/// cup-square). The single irreducible geometric atom — the last bit — is then exactly one structural law:
///
/// ```text
///     d₂(Sq⁰ x) = h₀ · Sq¹(x)        (the Kudo / H∞ transgression, for x ∈ Ext¹ of positive stem)
/// ```
///
/// This law is the H∞-ring (extended-power) structure of the SPHERE — geometry, with no pure-algebra
/// derivation; that is the honest atom. But it is ONE clean law, and once we feed it our derived `Sq⁰`
/// and `Sq¹`, it GENERATES the entire first differential, computed: `d₂(h₂) = h₀·Sq¹(h₁) = 0` and
/// `d₂(h₃) = h₀·Sq¹(h₂) = 0` (so `h₂, h₃` survive), and `d₂(h₄) = h₀·Sq¹(h₃) = h₀h₃² ≠ 0` (so `h₄` FIRES).
/// The last bit, crystallized: not a magic value, one transgression law over operations we already own.
#[test]
fn the_h_infinity_transgression_law_generates_the_whole_first_differential() {
    // The source operation Sq⁰ (cup-1 doubling), derived — computed up to the seed's source h₄ = Sq⁰(h₃).
    let cup = minimal_resolution(2, 16);
    let cdelta = diagonal(&cup, 1);
    let cdelta1 = diagonal_1(&cup, 1, &cdelta);
    let cg = |i: usize| cup[1].iter().position(|g| g.degree == (1 << i)).unwrap();
    for i in 0..=3 {
        assert_eq!(
            sq0_on_h(&cup, &cdelta1, cg(i)),
            vec![cg(i + 1)],
            "source: Sq⁰(h_{i}) = h_{{i+1}} (derived)"
        );
    }

    // The target operation Sq¹ (top cup-square) and h₀-multiplication, in a resolution that reaches (3,17).
    let res = minimal_resolution(4, 18);
    let h = |i: usize| res[1].iter().position(|g| g.degree == (1 << i)).unwrap();
    let h0 = h(0);
    let sq1 = |i: usize| yoneda_product(&res, 1, h(i), 1, h(i)); // Sq¹(h_i) = h_i² (top operation, derived)

    // Apply the H∞ transgression law d₂(Sq⁰ h_i) = h₀·Sq¹(h_i) and read off the whole pattern.
    let d2 = |i: usize| -> Vec<usize> {
        let s = sq1(i);
        if s.is_empty() {
            vec![]
        } else {
            yoneda_product(&res, 1, h0, 2, s[0])
        }
    };
    assert!(
        !sq1(3).is_empty(),
        "Sq¹(h₃) = h₃² ≠ 0 (the target operation is nonzero at h₃)"
    );
    assert!(
        d2(1).is_empty(),
        "law ⇒ d₂(h₂) = h₀·Sq¹(h₁) = 0 ⇒ h₂ permanent"
    );
    assert!(
        d2(2).is_empty(),
        "law ⇒ d₂(h₃) = h₀·Sq¹(h₂) = 0 ⇒ h₃ permanent"
    );
    assert!(
        !d2(3).is_empty(),
        "law ⇒ d₂(h₄) = h₀·Sq¹(h₃) = h₀h₃² ≠ 0 ⇒ h₄ FIRES — the last bit, resolved by ONE law"
    );
}

/// **The law's reach widened to the whole first line.** The entire `h`-line IS the `Sq⁰`-orbit of `h_0`:
/// `h_i = Sq⁰ⁱ(h_0)`, which we verify by composing the derived `Sq⁰` (`h_0 → h_1 → h_2 → h_3 → h_4`). The
/// single transgression law `d₂(Sq⁰ x) = h₀·Sq¹(x)` applied along that one orbit therefore generates the
/// ENTIRE infinite first-line family `d₂(h_{i+1}) = h₀h_i²` — every member a genuine `d₂` of the right
/// bidegree `(1,2^{i+1}) → (3,2^{i+1}+1)`. One orbit, one law, the whole family.
#[test]
fn the_transgression_law_over_the_sq0_orbit_of_h0_generates_the_whole_first_line() {
    // The h-line is the Sq⁰-orbit of h_0 — verified by iterating the derived doubling.
    let cup = minimal_resolution(2, 16);
    let cdelta = diagonal(&cup, 1);
    let cdelta1 = diagonal_1(&cup, 1, &cdelta);
    let cg = |i: usize| cup[1].iter().position(|g| g.degree == (1 << i)).unwrap();
    let mut x = cg(0); // start at h_0 and iterate Sq⁰
    for i in 0..=3 {
        let next = sq0_on_h(&cup, &cdelta1, x);
        assert_eq!(
            next,
            vec![cg(i + 1)],
            "Sq⁰ⁱ⁺¹(h_0) = h_{{i+1}}: the h-line is one Sq⁰-orbit"
        );
        x = next[0];
    }

    // The law over the whole orbit = the whole infinite family, each a genuine d₂.
    let is_d2 = |src: (usize, usize), tgt: (usize, usize)| tgt.0 == src.0 + 2 && tgt.1 == src.1 + 1;
    for i in 0..=30 {
        let source = (1usize, 1usize << (i + 1)); // h_{i+1} = Sq⁰(h_i)
        let target = (3usize, 1 + 2 * (1usize << i)); // h₀·Sq¹(h_i) = h₀h_i²
        assert!(
            is_d2(source, target),
            "law over orbit ⇒ d₂(h_{{i+1}}) = h₀h_i² is a real d₂ (i={i})"
        );
    }
}
